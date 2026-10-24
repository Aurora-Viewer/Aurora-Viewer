use super::*;
use glam::Mat4;
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

fn triangle() -> serde_json::Value {
    let vertices: Vec<_> = [[0.0,0.0,0.0], [0.2,0.0,0.0], [0.0,0.2,0.0]].into_iter().map(|position| {
        serde_json::json!({"position":position,"normal":[0.0,0.0,1.0],"uv":[0.0,0.0],"joints":[0,0,0,0],"weights":[2.0,0.0,0.0,0.0]})
    }).collect();
    serde_json::json!({"protocol":1,"revision":1,"meshes":[{
        "name":"Test garment","joints":["mPelvis"],"inverse_bind":[Mat4::IDENTITY.to_cols_array()],
        "faces":[{"color":[0.3,0.2,0.8,1.0],"vertices":vertices,"indices":[0,1,2]}]
    }]})
}
fn supported() -> HashSet<String> {
    HashSet::from(["mPelvis".into()])
}

fn static_triangle() -> serde_json::Value {
    let mut value = triangle();
    value["meshes"][0]["joints"] = serde_json::json!([]);
    value["meshes"][0]["inverse_bind"] = serde_json::json!([]);
    for vertex in value["meshes"][0]["faces"][0]["vertices"].as_array_mut().expect("vertices") {
        vertex["weights"] = serde_json::json!([0, 0, 0, 0]);
    }
    value
}

#[test]
fn blender_live_static_sources_have_normalized_geometry_without_a_skin() {
    let mut value = static_triangle();
    let source = Uuid::new_v4();
    value["meshes"][0]["source_id"] = serde_json::json!(source);
    // More than eight surfaces must remain one editable source.
    value["meshes"][0]["faces"] = serde_json::Value::Array(vec![value["meshes"][0]["faces"][0].clone(); 9]);
    let prepared = prepare(&value).expect("static source");
    assert_eq!(prepared.meshes.len(), 2);
    assert_eq!(prepared.sources.len(), 1);
    assert!(!prepared.sources[0].rigged());
    assert!(prepared.sources[0].scene.models.iter().all(|model| model.skin.is_none()));
    assert!(
        prepared
            .meshes
            .iter()
            .all(|mesh| mesh.source == source && mesh.skin.is_none() && mesh.header.skin.is_none())
    );
    assert_eq!(prepared.meshes[0].size, prepared.meshes[1].size);
    let mut live = Live::default();
    live.receive(prepared);
    let placement = objects::Placement {
        parent: None,
        offset: glam::Vec3::ZERO,
        location: objects::Location::Detached,
        position: glam::Vec3::ZERO,
        rotation: glam::Quat::IDENTITY,
        scale: glam::Vec3::ONE,
        size: glam::Vec3::ONE,
        center: glam::Vec3::ZERO,
    };
    live.placements.insert(source, placement);
    assert!(live.visible_snapshot().expect("snapshot").meshes.is_empty());
    value["revision"] = 2.into();
    value["meshes"][0]["name"] = "Renamed object".into();
    live.receive(prepare(&value).expect("new revision"));
    assert_eq!(live.placements[&source].location, objects::Location::Detached);
    assert!(
        live.visible_snapshot().expect("snapshot").meshes.is_empty(),
        "updates never rez a derezzed source"
    );
    live.receive(prepare(&serde_json::json!({"protocol":1,"revision":3,"meshes":[]})).expect("paused source"));
    live.receive(prepare(&value).expect("resumed source"));
    assert_eq!(
        live.placements[&source].location,
        objects::Location::Detached,
        "pause/resume retains the local placement"
    );
}

#[test]
fn blender_live_static_sources_reject_accidental_skin_data() {
    for field in ["weights", "joints"] {
        let mut value = static_triangle();
        value["meshes"][0]["faces"][0]["vertices"][0][field] = serde_json::json!([1, 0, 0, 0]);
        assert!(prepare(&value).is_err());
    }
    let mut mixed = static_triangle();
    mixed["meshes"]
        .as_array_mut()
        .expect("meshes")
        .push(triangle()["meshes"][0].clone());
    let prepared = prepare(&mixed).expect("mixed scene");
    assert!(!prepared.sources[0].rigged());
    assert!(prepared.sources[1].rigged());
}
fn prepare(value: &serde_json::Value) -> Result<protocol::Prepared, String> {
    protocol::prepare(&serde_json::to_vec(value).expect("JSON"), &supported())
}

fn linked_objects() -> serde_json::Value {
    let mut value = static_triangle();
    let mut child = value["meshes"][0].clone();
    value["meshes"][0]["source_id"] = serde_json::json!(Uuid::from_u128(1));
    value["meshes"][0]["name"] = "Parent mesh".into();
    child["source_id"] = serde_json::json!(Uuid::from_u128(2));
    child["name"] = "Child mesh".into();
    child["link"] = serde_json::json!({"parent":Uuid::from_u128(1),"offset":[1.0,0.0,-1.0]});
    child["faces"] = serde_json::json!(vec![child["faces"][0].clone(); 9]);
    value["meshes"].as_array_mut().expect("meshes").insert(0, child);
    value
}

#[test]
fn blender_live_linked_sources_validate_parentage_and_install_roots_before_children() {
    let value = linked_objects();
    let snapshot = prepare(&value).expect("linked meshes");
    assert_eq!(snapshot.sources.len(), 2);
    assert_eq!(snapshot.meshes.len(), 3, "child batches remain one source");
    assert_eq!(snapshot.sources[0].id, Uuid::from_u128(1));
    assert_eq!(snapshot.sources[1].link.expect("child").parent, snapshot.sources[0].id);
    assert_eq!(snapshot.meshes[0].source, snapshot.sources[0].id);
    for parent in [Uuid::nil(), Uuid::from_u128(2), Uuid::from_u128(3)] {
        let mut bad = value.clone();
        bad["meshes"][0]["link"]["parent"] = serde_json::json!(parent);
        assert!(prepare(&bad).is_err());
    }
    let mut cycle = value.clone();
    cycle["meshes"][1]["link"] = serde_json::json!({"parent":Uuid::from_u128(2),"offset":[0,0,0]});
    assert!(prepare(&cycle).is_err());
    let mut excessive = value.clone();
    excessive["meshes"][0]["link"]["offset"] = serde_json::json!([0, 0, 65537]);
    assert!(prepare(&excessive).is_err());
    let mut rigged = value;
    rigged["meshes"][1] = triangle()["meshes"][0].clone();
    rigged["meshes"][1]["source_id"] = serde_json::json!(Uuid::from_u128(1));
    assert!(prepare(&rigged).is_err(), "rigged sources cannot be static roots");
}

#[test]
fn blender_live_objects_are_grouped_as_linksets_with_stable_native_prim_numbers() {
    let mut value = linked_objects();
    let mut separate = triangle()["meshes"][0].clone();
    separate["source_id"] = serde_json::json!(Uuid::from_u128(3));
    value["meshes"].as_array_mut().expect("meshes").insert(1, separate);
    let mut live = Live::default();
    live.receive(prepare(&value).expect("snapshot"));
    let rows = live.linkset_rows();
    assert_eq!(
        rows.iter().map(|r| r.source.id).collect::<Vec<_>>(),
        // Keep the original root order; children follow their own root.
        [Uuid::from_u128(3), Uuid::from_u128(1), Uuid::from_u128(2)]
    );
    assert_eq!(
        rows.iter().map(|r| (r.number, r.prims)).collect::<Vec<_>>(),
        [(0, 1), (1, 3), (2, 2)]
    );
}

#[test]
fn blender_live_linked_meshes_are_individually_editable_and_follow_the_root() {
    use crate::{
        build::{BuildSettings, BuildTool},
        scene::Scene,
        world::World,
    };
    use glam::{Quat, Vec3};
    let root = Uuid::from_u128(1);
    let child = Uuid::from_u128(2);
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let avatar = world
        .objects
        .get(world.objects.index_of_uuid(&world.agent_id).expect("avatar"))
        .expect("avatar")
        .key;
    let mut live = Live::default();
    let mut value = linked_objects();
    let snapshot = prepare(&value).expect("snapshot");
    live.receive(snapshot.clone());
    live.prepare_placements(&snapshot, &world);
    assert_eq!(live.placements[&child].position, Vec3::new(1.0, 0.0, -1.0));
    let mut roots = HashMap::new();
    let mut parts = HashMap::<Uuid, usize>::new();
    for (i, mesh) in snapshot.meshes.iter().enumerate() {
        let part = parts.entry(mesh.source).or_default();
        let p = live.placements[&mesh.source];
        let mut object = crate::demo::hud::object(31, false);
        object.full_id = Uuid::from_u128(100 + i as u128);
        object.local_id = 1 + i as u32;
        object.state = 0;
        object.scale = p.size * p.scale;
        (object.position, object.rotation) = live.preview_transform(mesh.source, *part).expect("preview transform");
        object.update_flags |= crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY;
        let id = object.full_id;
        let index = world
            .objects
            .upsert_preview(live.preview_parent(mesh.source, *part, false, avatar, &roots), object);
        if *part == 0 {
            roots.insert(mesh.source, world.objects.get(index).expect("object").key);
        }
        live.instances.insert((mesh.source, *part), id);
        *part += 1;
    }
    let ri = world.objects.index_of(&roots[&root]).expect("root");
    let ci = world.objects.index_of(&roots[&child]).expect("child");
    assert_eq!(crate::build::root_of(&world, ci), ri);
    assert_eq!(live.edit_target(child, false), live.root(root));
    assert_eq!(live.edit_target(child, true), live.root(child));
    assert_eq!(
        world.objects.children_of(&roots[&root]).len(),
        2,
        "every linked prim has the same single root"
    );
    assert!(
        world.objects.children_of(&roots[&child]).is_empty(),
        "linked prims are not nested linksets"
    );
    let batch = world.objects.index_of_uuid(&live.instances[&(child, 1)]).expect("child batch");
    assert_eq!(world.objects.get(batch).expect("batch").position, live.placements[&child].position);
    let mut build = BuildTool::default();
    let mut settings = BuildSettings::default();
    build.click_select(&mut world, &settings, Some(ci), false);
    assert_eq!(build.selection, [roots[&root]], "normal click selects the linked set root");
    settings.edit_linked = true;
    build.click_select(&mut world, &settings, Some(ci), false);
    assert_eq!(build.selection, [roots[&child]]);
    assert_eq!(build.selected_link_number(&world), Some(2));
    let now = std::time::Instant::now();
    let initial = Scene::object_transform(&world, ci, now, 0).expect("child transform").0;
    build.set_transform(&mut world, &settings, Some(initial + Vec3::Y), None, None, true);
    live.capture_placements(&mut world);
    assert!(live.placements[&child].position.abs_diff_eq(Vec3::new(1.0, 1.0, -1.0), 1e-5));
    assert_eq!(world.objects.get(batch).expect("batch").position, live.placements[&child].position);
    let root_placement = live.placements[&root];
    live.place_object(
        root,
        objects::Action::Transform {
            position: root_placement.position,
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: Vec3::splat(2.0),
        },
        &world,
    );
    live.apply_transform(root, &mut world);
    let p = Scene::object_transform(&world, ci, now, 0).expect("rotated child").0;
    assert!(p.abs_diff_eq(root_placement.position + Vec3::new(-2.0, 2.0, -2.0), 1e-4));
    assert!(
        Scene::object_transform(&world, batch, now, 0)
            .expect("batch")
            .0
            .abs_diff_eq(p, 1e-4)
    );
    value["revision"] = 2.into();
    value["meshes"][0]["name"] = "Renamed child".into();
    let update = prepare(&value).expect("update");
    live.receive(update.clone());
    live.prepare_placements(&update, &world);
    assert!(live.placements[&child].position.abs_diff_eq(Vec3::new(2.0, 2.0, -2.0), 1e-5));
    live.place_object(child, objects::Action::Attach(31), &world);
    assert_eq!(live.placements[&root].location, objects::Location::Attachment(31));
    assert_eq!(live.preview_parent(root, 0, false, avatar, &roots), Some(avatar));
    assert_eq!(live.preview_parent(child, 0, false, avatar, &roots), Some(roots[&root]));
    live.place_object(child, objects::Action::Detach, &world);
    assert!(live.visible_snapshot().expect("snapshot").meshes.is_empty());
    live.receive(update.clone());
    assert!(
        live.visible_snapshot().expect("snapshot").meshes.is_empty(),
        "a revision cannot rez a detached family"
    );
    live.place_object(child, objects::Action::Rez, &world);
    assert_eq!(live.visible_snapshot().expect("snapshot").meshes.len(), 3);
    live.set_state(root, SourceState::Hidden);
    assert!(live.visible_snapshot().expect("snapshot").meshes.is_empty());
}

#[test]
fn blender_live_unparenting_preserves_world_placement_and_drop_supports_the_complete_family() {
    use glam::{Quat, Vec3};
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let mut value = linked_objects();
    let mut snapshot = prepare(&value).expect("snapshot");
    // Unit cubes make the complete-family vertical bounds explicit.
    for mesh in &mut snapshot.meshes {
        mesh.size = Vec3::ONE;
    }
    let root = Uuid::from_u128(1);
    let child = Uuid::from_u128(2);
    let mut live = Live::default();
    live.receive(snapshot.clone());
    live.prepare_placements(&snapshot, &world);
    assert!((live.floor_offset(root) - 1.5).abs() < 1e-5);
    let p = live.placements[&root];
    live.place_object(
        root,
        objects::Action::Transform {
            position: p.position,
            rotation: Quat::from_rotation_z(0.5),
            scale: Vec3::splat(2.0),
        },
        &world,
    );
    assert!((live.floor_offset(root) - 3.0).abs() < 1e-5);
    let root_before = live.placements[&root];
    let child_before = live.placements[&child];
    value["meshes"][0].as_object_mut().expect("mesh").remove("link");
    let mut update = prepare(&value).expect("unparented");
    for mesh in &mut update.meshes {
        mesh.size = Vec3::ONE;
    }
    live.receive(update.clone());
    live.prepare_placements(&update, &world);
    let after = live.placements[&child];
    assert_eq!(after.parent, None);
    assert!(
        after
            .position
            .abs_diff_eq(root_before.position + root_before.rotation * child_before.position, 1e-5)
    );
    assert!(after.rotation.abs_diff_eq(root_before.rotation * child_before.rotation, 1e-5));
    assert_eq!(live.linked_root(child), child);
    // Re-link, detach the family, then unparent while it is absent. Metadata
    // updates must not revive either source, including hidden members.
    let linked = prepare(&linked_objects()).expect("relinked");
    live.receive(linked.clone());
    live.prepare_placements(&linked, &world);
    live.place_object(root, objects::Action::Detach, &world);
    live.receive(update.clone());
    live.prepare_placements(&update, &world);
    assert!(live.visible_snapshot().expect("snapshot").meshes.is_empty());
}

#[test]
fn blender_live_link_geometry_updates_keep_child_edits_and_compensate_root_center_changes() {
    use glam::Vec3;
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let root = Uuid::from_u128(1);
    let child = Uuid::from_u128(2);
    let mut snapshot = prepare(&linked_objects()).expect("snapshot");
    let mut live = Live::default();
    live.receive(snapshot.clone());
    live.prepare_placements(&snapshot, &world);
    live.placements.get_mut(&child).expect("child").position += Vec3::Y * 2.0;
    let before = live.placements[&root].position + live.placements[&child].position;
    snapshot
        .sources
        .iter_mut()
        .find(|s| s.id == child)
        .expect("source")
        .link
        .as_mut()
        .expect("link")
        .offset[1] += 1.0;
    for mesh in &mut snapshot.meshes {
        mesh.center += if mesh.source == root { Vec3::Z } else { Vec3::X };
    }
    live.receive(snapshot.clone());
    live.prepare_placements(&snapshot, &world);
    let after = live.placements[&root].position + live.placements[&child].position;
    assert!(after.abs_diff_eq(before + Vec3::X + Vec3::Y, 1e-5));
    live.prepare_placements(&snapshot, &world);
    assert_eq!(
        live.placements[&root].position + live.placements[&child].position,
        after,
        "no idle drift"
    );
}

#[test]
fn blender_live_round_trip_keeps_skin_and_materials() {
    let prepared = prepare(&triangle()).expect("valid mesh");
    assert_eq!(prepared.revision, 1);
    assert_eq!(prepared.triangles, 1);
    assert_eq!(prepared.meshes[0].skin.as_ref().expect("skin").joint_names, ["mPelvis"]);
    assert_eq!(prepared.meshes[0].colors, [[0.3, 0.2, 0.8, 1.0]]);
    let mesh = &prepared.meshes[0];
    let range = mesh.header.lods[3].expect("high LOD");
    let section = aurora_assets::mesh::section_bytes(&mesh.data, range).expect("section");
    let decoded = aurora_assets::decode_mesh_lod(section).expect("LOD");
    assert!(!decoded.is_empty());
}

#[test]
fn blender_live_embedded_pbr_keeps_factors_maps_transforms_and_content_identity() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut value = triangle();
    value["images"] = serde_json::json!([{"width":2,"height":1,"rgba":STANDARD.encode([255,0,0,255,0,255,0,128])}]);
    value["meshes"][0]["faces"][0]["material"] = serde_json::json!({
        "metallic":0.7,"roughness":0.2,"emissive":[0.1,0.2,0.3],"double_sided":true,
        "alpha_mode":"BLEND","textures":[0,null,null,0],
        "transforms":[[0.1,0.2,2.0,3.0,0.4],[0,0,1,1,0],[0,0,1,1,0],[0,0,1,1,0]]
    });
    let first = prepare(&value).expect("PBR snapshot");
    let second = prepare(&value).expect("repeated snapshot");
    assert_eq!(
        first.images[0].id, second.images[0].id,
        "unchanged pixels reuse the local texture slot"
    );
    assert!(first.images[0].has_alpha);
    let (_, material) = first.meshes[0].materials[0].as_ref().expect("PBR material");
    assert_eq!(material.base_color_factor, [0.3, 0.2, 0.8, 1.0]);
    assert_eq!(material.metallic_factor, 0.7);
    assert_eq!(material.roughness_factor, 0.2);
    assert_eq!(material.base_color_texture, Some(first.images[0].id));
    assert_eq!(material.emissive_texture, material.base_color_texture);
    assert_eq!(material.alpha_mode, aurora_assets::material::AlphaMode::Blend);
    assert_eq!(material.transforms[0].scale, [2.0, 3.0]);
    value["images"][0]["rgba"] = STANDARD.encode([0, 0, 255, 255, 255, 255, 0, 255]).into();
    assert_ne!(first.images[0].id, prepare(&value).expect("painted image").images[0].id);
}

#[test]
fn blender_live_rejects_invalid_materials_images_and_references() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut base = triangle();
    base["images"] = serde_json::json!([{"width":1,"height":1,"rgba":STANDARD.encode([255;4])}]);
    base["meshes"][0]["faces"][0]["material"] = serde_json::json!({
        "metallic":0.0,"roughness":0.5,"emissive":[0,0,0],"double_sided":true,"alpha_mode":"OPAQUE","textures":[0,null,null,null]
    });
    for case in 0..8 {
        let mut value = base.clone();
        match case {
            0 => value["images"][0]["width"] = 1025.into(),
            1 => value["images"][0]["rgba"] = "not base64!".into(),
            2 => value["images"][0]["height"] = 0.into(),
            3 => value["meshes"][0]["faces"][0]["material"]["textures"][0] = 1.into(),
            4 => value["meshes"][0]["faces"][0]["material"]["metallic"] = 2.into(),
            5 => value["meshes"][0]["faces"][0]["material"]["alpha_mode"] = "UNKNOWN".into(),
            6 => value["meshes"][0]["faces"][0]["material"]["emissive"][0] = (-1).into(),
            _ => value["images"] = serde_json::Value::Array(vec![value["images"][0].clone(); 17]),
        }
        assert!(prepare(&value).is_err(), "invalid appearance case {case}");
    }
}

#[test]
fn blender_live_rejects_bad_skin_geometry_and_protocol_without_replacing_good_state() {
    for change in 0..8 {
        let mut value = triangle();
        match change {
            0 => value["protocol"] = 2.into(),
            1 => value["meshes"][0]["joints"][0] = "unknown".into(),
            2 => value["meshes"][0]["faces"][0]["indices"][2] = 99.into(),
            3 => value["meshes"][0]["faces"][0]["vertices"][0]["weights"] = serde_json::json!([0, 0, 0, 0]),
            4 => value["meshes"][0]["faces"][0]["vertices"][0]["joints"][0] = 2.into(),
            5 => value["meshes"][0]["inverse_bind"][0] = serde_json::to_value([0.0; 16]).expect("matrix"),
            6 => value["meshes"][0]["faces"][0]["vertices"][0]["weights"] = serde_json::json!([2, -1, 0, 0]),
            _ => value["meshes"][0]["faces"][0]["color"][0] = (-1).into(),
        }
        assert!(prepare(&value).is_err(), "invalid case {change}");
    }
    assert!(protocol::prepare(b"{}", &supported()).is_err());
    let empty = prepare(&serde_json::json!({"protocol":1,"revision":2,"meshes":[]})).expect("clear snapshot");
    assert!(empty.meshes.is_empty());
}

#[test]
fn blender_live_splits_many_materials_without_discarding_triangles() {
    let mut value = triangle();
    let face = value["meshes"][0]["faces"][0].clone();
    value["meshes"][0]["faces"] = serde_json::Value::Array(vec![face; 9]);
    let prepared = prepare(&value).expect("nine surfaces");
    assert_eq!(prepared.meshes.len(), 2);
    assert_eq!(prepared.triangles, 9);
    assert_eq!(prepared.sources.len(), 1, "one source object, regardless of draw batches");
    let scene = &prepared.sources[0].scene;
    assert_eq!(scene.models.len(), 2);
    assert_eq!(scene.instances.len(), 2);
    assert_eq!(scene.models[1].submodel, 1);
    assert_eq!(scene.models[0].center, scene.models[1].center);
    assert_eq!(scene.models[0].size, scene.models[1].size);
    assert!(prepared.meshes.iter().all(|m| m.source == prepared.sources[0].id));
}

#[test]
fn blender_live_visibility_and_removal_survive_updates_reordering_and_renaming() {
    let mut value = triangle();
    let mut other = value["meshes"][0].clone();
    value["meshes"][0]["source_id"] = Uuid::from_u128(1).to_string().into();
    other["source_id"] = Uuid::from_u128(2).to_string().into();
    value["meshes"].as_array_mut().expect("meshes").push(other);
    let mut live = Live::default();
    live.receive(prepare(&value).expect("two sources"));
    live.set_state(Uuid::from_u128(1), SourceState::Hidden);
    live.set_state(Uuid::from_u128(2), SourceState::Removed);
    assert!(live.visible_snapshot().expect("snapshot").meshes.is_empty());
    value["revision"] = 2.into();
    value["meshes"][0]["name"] = "Renamed garment".into();
    value["meshes"].as_array_mut().expect("meshes").reverse();
    live.receive(prepare(&value).expect("updated sources"));
    assert_eq!(live.state(Uuid::from_u128(1)), SourceState::Hidden);
    assert_eq!(live.state(Uuid::from_u128(2)), SourceState::Removed);
    live.set_state(Uuid::from_u128(1), SourceState::Visible);
    let visible = live.visible_snapshot().expect("visible object");
    assert_eq!(visible.sources.len(), 1);
    assert_eq!(visible.sources[0].name, "Renamed garment");
    assert_eq!(visible.meshes.len(), 1);
    assert_eq!(visible.triangles, 1);
    live.set_state(Uuid::from_u128(2), SourceState::Visible);
    assert_eq!(live.visible_snapshot().expect("restored").sources.len(), 2);
    value["meshes"] = serde_json::json!([]);
    live.receive(prepare(&value).expect("empty source list"));
    assert!(live.states.is_empty(), "departed sources do not accumulate in memory");
}

#[test]
fn blender_live_legacy_identity_survives_reordering_and_rejects_duplicate_explicit_ids() {
    let mut value = triangle();
    let mut other = value["meshes"][0].clone();
    other["name"] = "Other garment".into();
    value["meshes"].as_array_mut().expect("meshes").push(other);
    let first = prepare(&value).expect("legacy");
    value["meshes"].as_array_mut().expect("meshes").reverse();
    let reordered = prepare(&value).expect("reordered legacy");
    assert_eq!(first.sources[0].id, reordered.sources[1].id);
    assert_eq!(first.sources[1].id, reordered.sources[0].id);
    for mesh in value["meshes"].as_array_mut().expect("meshes") {
        mesh["source_id"] = Uuid::from_u128(1).to_string().into();
    }
    assert!(prepare(&value).is_err());
    value["meshes"] = serde_json::json!([value["meshes"][0]]);
    value["meshes"][0]["source_id"] = Uuid::nil().to_string().into();
    assert!(prepare(&value).is_err());
}

#[test]
fn blender_live_row_buttons_import_their_own_source_only_when_available() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
    let palette = theme.palette();
    let mut live = Live {
        open: true,
        ..Default::default()
    };
    let mut value = triangle();
    let mut other = value["meshes"][0].clone();
    other["name"] = "Other garment".into();
    value["meshes"].as_array_mut().expect("meshes").push(other);
    live.receive(prepare(&value).expect("sources"));
    let sources: Vec<_> = live.snapshot.as_ref().expect("snapshot").sources.iter().map(|s| s.id).collect();
    let mut time = 0.0;
    let mut frame = |live: &mut Live, can_upload, events| {
        time += 1.0 / 60.0;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 900.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                super::ui::show(live, ui.ctx(), &palette, can_upload);
            },
        );
        out.textures_delta.clear();
        out
    };
    let positions = |out: &egui::FullOutput, label: &str| {
        out.shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Text(text) = &s.shape
                    && text.galley.text() == label
                {
                    Some(text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    for _ in 0..6 {
        frame(&mut live, true, vec![]);
    }
    let out = frame(&mut live, true, vec![]);
    assert!(positions(&out, aurora_i18n::tr!("common-remove")).is_empty());
    let click = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let clear = positions(&out, aurora_i18n::tr!("blender-live-clear"))[0];
    frame(&mut live, true, click(clear, true));
    frame(&mut live, true, click(clear, false));
    assert!(sources.iter().all(|id| live.state(*id) == SourceState::Removed));
    let out = frame(&mut live, true, vec![]);
    assert!(positions(&out, aurora_i18n::tr!("blender-live-upload")).is_empty());
    assert!(positions(&out, aurora_i18n::tr!("blender-live-restore")).is_empty());
    for id in &sources {
        live.set_state(*id, SourceState::Visible);
    }
    assert!(sources.iter().all(|id| live.state(*id) == SourceState::Visible));
    let out = frame(&mut live, false, vec![]);
    let imports = positions(&out, aurora_i18n::tr!("blender-live-upload"));
    assert_eq!(imports.len(), 2, "each source has its own row action");
    for import in imports {
        frame(&mut live, false, click(import, true));
        frame(&mut live, false, click(import, false));
    }
    assert!(live.upload_requested.is_none(), "busy importer cannot receive another draft");
    live.set_state(sources[0], SourceState::Hidden);
    live.selected = Some(sources[1]);
    let out = frame(&mut live, true, vec![]);
    let imports = positions(&out, aurora_i18n::tr!("blender-live-upload"));
    for (index, name) in ["Test garment", "Other garment"].into_iter().enumerate() {
        let name_pos = positions(&out, name)[0];
        assert!((name_pos.y - imports[index].y).abs() < 3.0, "import stays on its object's row");
        frame(&mut live, true, click(imports[index], true));
        frame(&mut live, true, click(imports[index], false));
        assert_eq!(
            live.upload_requested.take(),
            Some(sources[index]),
            "row import ignores the previous selection"
        );
    }
}

#[test]
fn blender_live_static_controls_request_derez_rez_and_hud_attachment_for_the_selected_source() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
    let palette = theme.palette();
    let mut live = Live {
        open: true,
        ..Default::default()
    };
    live.receive(prepare(&static_triangle()).expect("source"));
    let source = live.selected.expect("selected");
    live.placements.insert(
        source,
        objects::Placement {
            parent: None,
            offset: glam::Vec3::ZERO,
            location: objects::Location::World,
            position: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::ONE,
            size: glam::Vec3::ONE,
            center: glam::Vec3::ZERO,
        },
    );
    live.instances.insert((source, 0), Uuid::new_v4());
    live.attach_choices = vec![(6, false, "Right hand".into()), (31, true, "HUD center".into())];
    let mut time = 0.0;
    let mut frame = |live: &mut Live, events| {
        time += 1.0 / 60.0;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1200.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                super::ui::show(live, ui.ctx(), &palette, true);
            },
        );
        out.textures_delta.clear();
        out
    };
    let position = |out: &egui::FullOutput, label: &str| {
        out.shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text() == label
                {
                    Some(text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| panic!("missing control {label}"))
    };
    let click = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    for _ in 0..6 {
        frame(&mut live, vec![]);
    }
    let out = frame(&mut live, vec![]);
    let at = position(&out, aurora_i18n::tr!("blender-object-derez"));
    frame(&mut live, click(at, true));
    frame(&mut live, click(at, false));
    assert!(matches!(live.object_action.take(), Some((id, objects::Action::Derez)) if id == source));
    live.placements.get_mut(&source).expect("placement").location = objects::Location::Detached;
    let out = frame(&mut live, vec![]);
    let at = position(&out, aurora_i18n::tr!("blender-object-rez"));
    frame(&mut live, click(at, true));
    frame(&mut live, click(at, false));
    assert!(matches!(live.object_action.take(), Some((id, objects::Action::Rez)) if id == source));
    let out = frame(&mut live, vec![]);
    let at = position(&out, aurora_i18n::tr!("common-attach-to-hud"));
    frame(&mut live, click(at, true));
    frame(&mut live, click(at, false));
    let out = frame(&mut live, vec![]);
    let at = position(&out, "HUD center");
    frame(&mut live, click(at, true));
    frame(&mut live, click(at, false));
    assert!(matches!(live.object_action.take(), Some((id, objects::Action::Attach(31))) if id == source));
}

#[test]
fn blender_live_attachment_namespace_survives_rekey_without_colliding_with_simulator_objects() {
    use crate::world::objects::{ObjectStore, PREVIEW_REGION};
    let mut store = ObjectStore::default();
    let mut avatar = crate::demo::action_avatar(false);
    let avatar_id = avatar.full_id;
    let avatar_index = store.upsert(0, avatar.clone());
    let parent = store.get(avatar_index).expect("avatar").key;
    let mut preview = avatar.clone();
    preview.local_id = 1;
    preview.full_id = Uuid::new_v4();
    preview.pcode = aurora_prim::params::LL_PCODE_VOLUME;
    let preview_id = preview.full_id;
    let preview_index = store.insert_preview_attachment(parent, preview.clone());
    assert_eq!(store.get(preview_index).expect("preview").key.region, PREVIEW_REGION);
    assert_eq!(store.parent_of(store.get(preview_index).expect("preview")), Some(avatar_index));
    preview.full_id = Uuid::new_v4();
    preview.parent_id = 0;
    let server_index = store.upsert(0, preview);
    assert_ne!(server_index, preview_index);
    avatar.local_id = 900;
    store.upsert(256, avatar);
    let parent = store.get(avatar_index).expect("moved avatar");
    assert_eq!(parent.full_id, avatar_id);
    assert_eq!(store.get(preview_index).expect("preview").parent_key(), Some(parent.key));
    store.remove_idx(avatar_index);
    assert!(store.index_of_uuid(&preview_id).is_none());
    assert!(store.get(server_index).is_some());
}

#[test]
fn blender_live_transport_handshake_errors_reconnect_and_latest_slot() {
    let ctx = egui::Context::default();
    let probe = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("free port");
    let port = probe.local_addr().expect("address").port();
    let shared = transport::start_listener(probe, supported(), Default::default(), ctx, None).expect("start");
    let owner = Server { shared: shared.clone() };
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    stream.write_all(b"AURBLD1\n").expect("magic");
    let mut reader = BufReader::new(stream.try_clone().expect("reader"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("hello");
    assert!(line.contains("mPelvis"));
    assert!(line.contains("pbr-materials"));
    for revision in 1..=3 {
        let mut value = triangle();
        value["revision"] = revision.into();
        let bytes = serde_json::to_vec(&value).expect("snapshot");
        stream.write_all(&(bytes.len() as u32).to_le_bytes()).expect("length");
        stream.write_all(&bytes).expect("data");
        line.clear();
        reader.read_line(&mut line).expect("ack");
        assert!(line.contains("\"ok\":true"));
    }
    assert_eq!(shared.latest.lock().as_ref().expect("latest").revision, 3);
    stream.write_all(&2u32.to_le_bytes()).expect("bad length");
    stream.write_all(b"{}").expect("bad data");
    line.clear();
    reader.read_line(&mut line).expect("rejection");
    assert!(line.contains("\"ok\":false"));
    assert_eq!(shared.latest.lock().as_ref().expect("unchanged").revision, 3);
    drop(owner);
}

#[test]
fn blender_live_shape_transport_requires_negotiation_and_confirms_import_separately() {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("port");
    let port = listener.local_addr().expect("address").port();
    let shared = transport::start_listener(listener, supported(), Default::default(), egui::Context::default(), None).expect("server");
    let owner = Server { shared: shared.clone() };
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    stream.write_all(b"AURBLD1\n").expect("hello");
    let mut reader = BufReader::new(stream.try_clone().expect("reader"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("hello");
    let id = Uuid::from_u128(1234);
    *shared.shape.lock() = Some(shape::Outgoing {
        id,
        data: Arc::new(serde_json::json!({"id": id, "name": "Test Shape"})),
    });
    stream.write_all(&0u32.to_le_bytes()).expect("old client heartbeat");
    line.clear();
    reader.read_line(&mut line).expect("reply");
    assert!(!line.contains("shape"), "old plug-ins receive no shape data");
    assert!(shared.shape.lock().is_some());
    for message in [
        serde_json::json!({"type":"features","features":["shape-transfer"]}),
        serde_json::json!({"type":"shape-result","id":id,"error":null}),
    ] {
        let data = serde_json::to_vec(&message).expect("control");
        stream.write_all(&(data.len() as u32).to_le_bytes()).expect("length");
        stream.write_all(&data).expect("control");
        line.clear();
        reader.read_line(&mut line).expect("reply");
        let reply: serde_json::Value = serde_json::from_str(&line).expect("reply JSON");
        if message["type"] == "features" {
            assert_eq!(reply["shape"]["id"], id.to_string());
        } else {
            assert!(reply.get("shape").is_none(), "no duplicate import on the acknowledgement");
        }
    }
    assert_eq!(shared.shape_ack.lock().take(), Some((id, None)));
    assert!(shared.latest.lock().is_none(), "shape controls never replace live mesh revisions");
    *shared.shape.lock() = Some(shape::Outgoing {
        id,
        data: Arc::new(serde_json::json!({"action":"apply"})),
    });
    stream.write_all(&0u32.to_le_bytes()).expect("0.9 heartbeat");
    line.clear();
    reader.read_line(&mut line).expect("reply");
    assert!(!line.contains("shape"), "0.9 must never interpret Apply as Create");
    assert!(shared.shape.lock().is_some());
    let data = br#"{"type":"features","features":["shape-transfer","shape-apply"]}"#;
    stream.write_all(&(data.len() as u32).to_le_bytes()).expect("length");
    stream.write_all(data).expect("features");
    line.clear();
    reader.read_line(&mut line).expect("apply delivery");
    let reply: serde_json::Value = serde_json::from_str(&line).expect("JSON");
    assert_eq!(reply["shape"]["action"], "apply");
    assert!(shared.shape.lock().is_none());
    drop(owner);
}

#[test]
fn blender_lods_transport_negotiates_ignores_stale_results_and_keeps_live_revision() {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("port");
    let port = listener.local_addr().expect("address").port();
    let shared = transport::start_listener(listener, supported(), Default::default(), egui::Context::default(), None).expect("server");
    let owner = Server { shared: shared.clone() };
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    stream.write_all(b"AURBLD1\n").expect("hello");
    let mut reader = BufReader::new(stream.try_clone().expect("reader"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("hello");
    assert!(line.contains("import-lods"));
    let request = import::Request {
        id: Uuid::from_u128(123),
        source: Uuid::from_u128(456),
    };
    *shared.import_request.lock() = Some(request);
    *shared.import_expected.lock() = Some(request);
    let mut exchange = |bytes: &[u8]| {
        stream.write_all(&(bytes.len() as u32).to_le_bytes()).expect("length");
        stream.write_all(bytes).expect("data");
        line.clear();
        reader.read_line(&mut line).expect("reply");
        serde_json::from_str::<serde_json::Value>(&line).expect("JSON")
    };
    assert!(exchange(&[]).get("prepare_import").is_none());
    assert!(shared.import_request.lock().is_some(), "old clients receive no request");
    let reply = exchange(br#"{"type":"features","features":["import-lods"]}"#);
    assert_eq!(reply["prepare_import"]["id"], request.id.to_string());
    assert!(shared.import_request.lock().is_none());
    assert_eq!(exchange(&serde_json::to_vec(&triangle()).expect("snapshot"))["revision"], 1);
    for id in [Uuid::from_u128(124), request.id] {
        let mut bytes = b"LODS".to_vec();
        bytes.extend(serde_json::to_vec(&serde_json::json!({"id":id,"source":request.source,"error":"Missing LOD"})).expect("result"));
        assert_eq!(exchange(&bytes)["ok"], true);
        assert_eq!(shared.import_result.lock().is_some(), id == request.id);
        assert_eq!(shared.import_expected.lock().is_none(), id == request.id);
        assert_eq!(shared.latest.lock().as_ref().expect("latest").revision, 1);
    }
    let (id, result) = shared.import_result.lock().take().expect("result");
    assert_eq!(id, request.id);
    assert!(result.is_err_and(|message| message == "Missing LOD"));
    let mut next = triangle();
    next["revision"] = 2.into();
    assert_eq!(exchange(&serde_json::to_vec(&next).expect("snapshot"))["revision"], 2);
    assert_eq!(shared.status.lock().received, 2);
    drop(owner);
}
