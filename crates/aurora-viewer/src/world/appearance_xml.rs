//! Self-only LLVOAvatar::dumpArchetypeXML / dump_visual_param export
//! (indra/newview/llvoavatar.cpp, originally LGPL 2.1). No asset export.
//!
//! Stricter than the Legacy Viewer, whose dump checks no permission: the
//! parameters of a wearable type are written only when the agent may modify
//! every worn item of that type (`actions::appearance_xml_export_allowed`,
//! the rule of the editors' Export XML button); the others are left out and
//! counted, so a no-modify shape or skin is not written to a file.
use super::World;
use crate::scene::avatar::AvatarLibrary;
use aurora_llsd::xml::{self, Element};
use glam::{Mat4, Vec3};
use std::{collections::BTreeMap, fmt::Write, sync::Arc};

pub fn ready(world: &World) -> bool {
    !world.agent_id.is_nil()
        && world.avatar_lib.skeleton.is_some()
        && world.server_appearance().is_some()
        && world
            .visual_params(&world.agent_id)
            .is_some_and(|values| !values.is_empty() && values.len() >= world.avatar_lib.shape_params.transmitted_count())
}

/// Captured on click, before the native dialog. Never resolves a selected
/// avatar, downloads assets or changes the currently worn outfit.
pub struct Snapshot {
    library: Arc<AvatarLibrary>,
    values: Vec<u8>,
    posed: Vec<Mat4>,
    /// Wearable types (LLWearableType) left out: a worn item of the type
    /// that the agent may not modify.
    refused: std::collections::BTreeSet<u8>,
    /// How many worn items made them left out.
    left_out: usize,
}

/// The wearable types whose parameters may not be written, and how many worn
/// items refuse it: each worn body part or clothing item of the outfit is
/// checked with `actions::appearance_xml_export_allowed`.
fn refused_types(world: &World) -> (std::collections::BTreeSet<u8>, usize) {
    let mut refused = std::collections::BTreeSet::new();
    let mut count = 0;
    let links = super::appearance::cof(&world.inventory)
        .into_iter()
        .flat_map(|cof| super::appearance::folder_links(&world.inventory, cof));
    for link in links.filter(|link| !link.folder) {
        let Some(item) = world.inventory.items.get(&link.target) else {
            continue;
        };
        if !matches!(item.asset_type, 5 | 13) {
            continue;
        }
        if !super::inventory::actions::appearance_xml_export_allowed(item, world.agent_id) {
            refused.insert((item.flags & 0xff) as u8);
            count += 1;
        }
    }
    (refused, count)
}
impl Snapshot {
    pub fn own(world: &World) -> Option<Self> {
        let (refused, left_out) = refused_types(world);
        ready(world).then(|| Self {
            library: world.avatar_lib.clone(),
            values: world.visual_params(&world.agent_id).unwrap_or_default().to_vec(),
            posed: world.avatar_poses.get(&world.agent_id).cloned().unwrap_or_default(),
            refused,
            left_out,
        })
    }

    /// How many worn items were left out for want of modify rights.
    pub fn left_out(&self) -> usize {
        self.left_out
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        let bytes = self.bytes()?;
        crate::atomic_file::write(path, &bytes).map_err(|e| e.to_string())
    }

    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        let lib = &self.library;
        let sk = lib.skeleton.as_ref().ok_or("missing skeleton")?;
        let lad = AvatarLibrary::file("avatar_lad.xml").ok_or("missing appearance definitions")?;
        let root = xml::parse(lad).map_err(|e| format!("appearance definitions: {e}"))?;
        let weights = lib.shape_params.weights(&self.values);
        let mut params = BTreeMap::new();
        collect_params(&root, &mut params);
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<linden_genepool version=\"1.0\">\n\t<archetype name=\"???\">\n",
        );
        for (id, param) in params {
            let refused = param
                .attr("wearable")
                .and_then(crate::world::inventory::wearable::kind_of)
                .is_some_and(|kind| self.refused.contains(&kind));
            if refused {
                continue;
            }
            let Some(value) = weights.get(&id).copied() else {
                continue;
            };
            if !value.is_finite() {
                return Err("non-finite appearance parameter".into());
            }
            let min = number(param, "value_min", 0.0);
            let max = number(param, "value_max", 1.0);
            let byte = if max > min {
                (((value - min) / (max - min)).clamp(0.0, 1.0) * 255.0).floor() as u8
            } else {
                0
            };
            let kind = param
                .elements()
                .find(|e| {
                    matches!(
                        e.name.as_str(),
                        "param_alpha" | "param_color" | "param_driver" | "param_morph" | "param_skeleton"
                    )
                })
                .map_or("unknown", |e| e.name.as_str());
            let name = param.attr("name").unwrap_or("");
            let display = param.attr("label").unwrap_or(name);
            let _ = writeln!(
                out,
                "\t\t<param id=\"{id}\" name=\"{}\" display=\"{}\" value=\"{value:.3}\" u8=\"{byte}\" type=\"{kind}\" wearable=\"{}\" group=\"{}\"/>",
                escaped(&name.to_lowercase()),
                escaped(display),
                escaped(param.attr("wearable").unwrap_or("invalid")),
                escaped(param.attr("group").unwrap_or("0"))
            );
        }
        // The ordinary Legacy Viewer export writes null UUIDs, including
        // baked slots. This is appearance data, never texture extraction.
        for te in 0..45 {
            let _ = writeln!(out, "\t\t<texture te=\"{te}\" uuid=\"00000000-0000-0000-0000-000000000000\"/>");
        }
        // Keep the root avatar-local: world position is not needed to load
        // a shape and should not turn a portable file into a location log.
        joint(&mut out, "root", "mRoot", Vec3::ZERO, Vec3::ONE)?;
        let shape = lib.shape_params.shape(&self.values, &lib.rig);
        let posed: Option<Vec<_>> = (self.posed.len() == lib.rig.len()).then(|| {
            self.posed
                .iter()
                .zip(&lib.rig.default_world_inv)
                .map(|(pose, inv)| *pose * inv.inverse())
                .collect()
        });
        let mut joints: Vec<_> = sk.joints.iter().enumerate().collect();
        joints.sort_by_key(|(_, j)| (j.is_collision_volume, &j.name));
        for (index, definition) in joints {
            let mut pos = shape.local_pos[index];
            let mut scale = shape.scale[index];
            if let Some(posed) = &posed {
                let world = posed[index];
                // Matrix decomposition can move a reflection onto another axis.
                // Keep the shaped skeleton's scale signs on their own axes.
                scale = Vec3::new(
                    world.x_axis.truncate().length(),
                    world.y_axis.truncate().length(),
                    world.z_axis.truncate().length(),
                ) * shape.scale[index].signum();
                pos = if let Some(parent) = lib.rig.parent[index] {
                    posed[parent].inverse().transform_point3(world.w_axis.truncate())
                } else {
                    world.w_axis.truncate()
                };
            }
            joint(
                &mut out,
                if definition.is_collision_volume {
                    "collision_volume"
                } else {
                    "bone"
                },
                &definition.name,
                pos,
                scale,
            )?;
        }
        let mut points: Vec<_> = lib.attach_points.iter().collect();
        points.sort_by_key(|(id, _)| **id);
        for (id, point) in points {
            let name = lib.attach_names.get(id).map(String::as_str).unwrap_or("");
            joint(&mut out, "attachment_point", name, point.local_position, Vec3::ONE)?;
        }
        out.push_str("\t</archetype>\n</linden_genepool>\n");
        Ok(out.into_bytes())
    }
}
fn collect_params<'a>(el: &'a Element, params: &mut BTreeMap<i32, &'a Element>) {
    if el.name == "param"
        && let Some(id) = el.attr("id").and_then(|s| s.trim().parse().ok())
    {
        // Shared templates keep the first definition, as LLCharacter does.
        params.entry(id).or_insert(el);
    }
    for child in el.elements() {
        collect_params(child, params);
    }
}
fn number(el: &Element, key: &str, default: f32) -> f32 {
    el.attr(key)
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|n| n.is_finite())
        .unwrap_or(default)
}
fn escaped(value: &str) -> String {
    let mut out = String::new();
    xml::escape(value, &mut out);
    out
}
fn joint(out: &mut String, tag: &str, name: &str, pos: Vec3, scale: Vec3) -> Result<(), String> {
    if !pos.is_finite() || !scale.is_finite() {
        return Err("non-finite appearance joint".into());
    }
    let _ = writeln!(
        out,
        "\t\t<{tag} name=\"{}\" position=\"{:.6} {:.6} {:.6}\" scale=\"{:.6} {:.6} {:.6}\"/>",
        escaped(name),
        pos.x,
        pos.y,
        pos.z,
        scale.x,
        scale.y,
        scale.z
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{AvatarAppearance, NetEvent};
    use uuid::Uuid;
    fn world() -> World {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        let appearance = AvatarAppearance {
            avatar_id: world.agent_id,
            visual_params: world.avatar_lib.shape_params.values_with(&[(33, 0.8), (80, 1.0)]),
            texture_entry: None,
            cof_version: 0,
            appearance_version: 1,
            hover_height: 0.0,
            attachments: Vec::new(),
        };
        world.apply(NetEvent::Appearance(appearance));
        world
    }
    #[test]
    fn appearance_xml_exports_only_self_with_legacy_parameters_and_no_asset_ids() {
        let world = world();
        let snapshot = Snapshot::own(&world).unwrap();
        let bytes = snapshot.bytes().unwrap();
        let root = xml::parse(&bytes).unwrap();
        assert_eq!(root.name, "linden_genepool");
        let archetype = root.elements().next().unwrap();
        let params: Vec<_> = archetype.elements().filter(|e| e.name == "param").collect();
        assert!(params.len() > 200);
        assert_eq!(params.iter().filter(|e| e.attr("id") == Some("33")).count(), 1);
        let male = params.iter().find(|e| e.attr("id") == Some("80")).unwrap();
        assert_eq!(male.attr("value"), Some("1.000"));
        assert_eq!(male.attr("wearable"), Some("shape"));
        assert_eq!(archetype.elements().filter(|e| e.name == "texture").count(), 45);
        assert!(
            archetype
                .elements()
                .filter(|e| e.name == "texture")
                .all(|e| e.attr("uuid") == Some("00000000-0000-0000-0000-000000000000"))
        );
        assert!(archetype.elements().any(|e| e.name == "bone" && e.attr("name") == Some("mPelvis")));
        assert!(archetype.elements().any(|e| e.name == "collision_volume"));
        assert!(archetype.elements().any(|e| e.name == "attachment_point"));
        assert!(!String::from_utf8(bytes).unwrap().contains(&world.agent_id.to_string()));
    }
    #[test]
    fn appearance_xml_requires_complete_own_data_and_freezes_the_clicked_appearance() {
        let mut world = world();
        let snapshot = Snapshot::own(&world).unwrap();
        let original = snapshot.bytes().unwrap();
        world.preview_shape(Some(world.avatar_lib.shape_params.values_with(&[(80, 0.0)])));
        assert_ne!(Snapshot::own(&world).unwrap().bytes().unwrap(), original);
        assert_eq!(snapshot.bytes().unwrap(), original);
        world.agent_id = Uuid::from_u128(2);
        assert!(Snapshot::own(&world).is_none());
        world.preview_shape(Some(vec![0]));
        assert!(Snapshot::own(&world).is_none());
        world.agent_id = Uuid::nil();
        assert!(!ready(&world));
    }
    #[test]
    fn appearance_xml_uses_the_current_own_joint_pose_in_local_coordinates() {
        let mut world = world();
        let lib = &world.avatar_lib;
        let mut shape = lib.shape_params.shape(world.server_visual_params(), &lib.rig);
        let chest = lib.rig.names["mChest"];
        shape.local_pos[chest] += Vec3::new(0.01, 0.02, 0.03);
        shape.scale[chest] = Vec3::new(1.1, 0.9, 1.2);
        let posed = lib.rig.world_with(&lib.rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
        world.avatar_poses.insert(
            world.agent_id,
            posed.iter().zip(&lib.rig.default_world_inv).map(|(p, inv)| *p * *inv).collect(),
        );
        let bytes = Snapshot::own(&world).unwrap().bytes().unwrap();
        let root = xml::parse(&bytes).unwrap();
        let chest = root
            .elements()
            .next()
            .unwrap()
            .elements()
            .find(|e| e.name == "bone" && e.attr("name") == Some("mChest"))
            .unwrap();
        let parse = |key| {
            let values: Vec<f32> = chest.attr(key).unwrap().split_whitespace().map(|v| v.parse().unwrap()).collect();
            Vec3::new(values[0], values[1], values[2])
        };
        assert!(parse("position").abs_diff_eq(shape.local_pos[world.avatar_lib.rig.names["mChest"]], 1e-5));
        assert!(parse("scale").abs_diff_eq(Vec3::new(1.1, 0.9, 1.2), 1e-5));
    }

    #[test]
    fn appearance_xml_round_trips_the_shape_parameters_read_by_the_legacy_importer() {
        let world = world();
        let snapshot = Snapshot::own(&world).unwrap();
        let bytes = snapshot.bytes().unwrap();
        let root = xml::parse(&bytes).unwrap();
        assert_eq!(root.attr("version"), Some("1.0"));
        // LLPanelEditWearable::onClickedImportBtnCallback reads only id/value.
        let values: Vec<_> = root
            .elements()
            .next()
            .unwrap()
            .elements()
            .filter(|e| e.name == "param" && e.attr("wearable") == Some("shape"))
            .map(|e| {
                (
                    e.attr("id").unwrap().parse::<i32>().unwrap(),
                    e.attr("value").unwrap().parse::<f32>().unwrap(),
                )
            })
            .collect();
        let weights = world.avatar_lib.shape_params.weights(&snapshot.values);
        assert!(values.len() > 50);
        for (id, value) in values {
            assert!((value - weights[&id]).abs() <= 0.00051, "parameter {id}");
        }
    }

    #[test]
    fn appearance_xml_failed_validation_preserves_the_existing_file_and_a_valid_save_replaces_it() {
        let world = world();
        let mut snapshot = Snapshot::own(&world).unwrap();
        let path = std::env::temp_dir().join(format!("aurora-appearance-{}.xml", Uuid::new_v4()));
        std::fs::write(&path, b"old appearance").unwrap();
        snapshot.posed = vec![Mat4::NAN; snapshot.library.rig.len()];
        assert!(snapshot.write(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old appearance");
        snapshot.posed.clear();
        snapshot.write(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), snapshot.bytes().unwrap());
        std::fs::remove_file(&path).unwrap();
        assert!(snapshot.write(&path.join("missing.xml")).is_err());
    }

    /// The wearable types of the file's parameters.
    fn types(snapshot: &Snapshot) -> std::collections::BTreeSet<String> {
        let bytes = snapshot.bytes().expect("export");
        let root = xml::parse(&bytes).expect("xml");
        root.elements()
            .next()
            .expect("archetype")
            .elements()
            .filter(|e| e.name == "param")
            .filter_map(|e| e.attr("wearable").map(str::to_owned))
            .collect()
    }

    #[test]
    fn appearance_xml_leaves_out_the_worn_items_the_agent_may_not_modify() {
        use crate::world::inventory::actions::{COPY, FULL, MODIFY, TRANSFER};
        let mut world = world();
        crate::world::appearance::seed_demo(&mut world.inventory, world.agent_id);
        crate::world::appearance::seed_demo_wearables(&mut world.inventory, world.agent_id);
        let shape = crate::shape_editor::worn_shape(&world).expect("worn shape");
        let all = Snapshot::own(&world).expect("snapshot");
        assert_eq!(all.left_out(), 0, "every demo wearable is the agent's, full permissions");
        assert!(types(&all).contains("shape") && types(&all).contains("skin"));
        let other = Uuid::from_u128(99);
        // The cases of actions::appearance_xml_export_allowed that refuse.
        for (what, owner, group_owned, owner_mask) in [
            ("no modify", world.agent_id, false, COPY | TRANSFER),
            ("someone else's", other, false, FULL),
            ("group owned", world.agent_id, true, FULL),
        ] {
            let item = world.inventory.items.get_mut(&shape).expect("shape item");
            item.owner = owner;
            item.group_owned = group_owned;
            item.owner_mask = owner_mask;
            item.everyone_mask = COPY;
            let snapshot = Snapshot::own(&world).expect("snapshot");
            assert_eq!(snapshot.left_out(), 1, "{what}");
            let kinds = types(&snapshot);
            assert!(!kinds.contains("shape"), "{what}: no shape parameter");
            assert!(kinds.contains("skin"), "{what}: the other wearables stay");
        }
        // No copy or no transfer still allows it, as the editors' button.
        for owner_mask in [MODIFY | TRANSFER, COPY | MODIFY] {
            let item = world.inventory.items.get_mut(&shape).expect("shape item");
            item.owner = world.agent_id;
            item.group_owned = false;
            item.owner_mask = owner_mask;
            let snapshot = Snapshot::own(&world).expect("snapshot");
            assert_eq!(snapshot.left_out(), 0);
            assert!(types(&snapshot).contains("shape"));
        }
    }

    #[test]
    fn appearance_xml_escapes_attributes_and_rejects_invalid_joint_values() {
        let mut text = String::new();
        joint(&mut text, "bone", "A\"<&", Vec3::ZERO, Vec3::ONE).unwrap();
        let root = xml::parse(text.as_bytes()).unwrap();
        assert_eq!(root.attr("name"), Some("A\"<&"));
        assert!(joint(&mut text, "bone", "bad", Vec3::NAN, Vec3::ONE).is_err());
    }
}
