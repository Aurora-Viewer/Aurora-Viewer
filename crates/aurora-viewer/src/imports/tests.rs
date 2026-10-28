use super::*;

#[test]
fn authored_native_animation_opens_the_normal_inspector_without_starting_or_charging_an_upload() {
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let animation = Animation {
        base_priority: 3,
        duration: 1.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 1.0,
        looping: true,
        ease_in: 0.2,
        ease_out: 0.3,
        hand_pose: 0,
        joints: vec![aurora_assets::anim::JointMotion {
            joint_name: "mHead".into(),
            priority: 5,
            rot_keys: vec![aurora_assets::anim::RotKey {
                time: 0.0,
                rotation: glam::Quat::IDENTITY,
            }],
            pos_keys: Vec::new(),
        }],
    };
    let bytes = Arc::new(aurora_assets::import_animation::encode(&animation).unwrap());
    let animation = Arc::new(aurora_assets::anim::parse_animation_upload(&bytes).unwrap());
    let mut imports = Imports::default();
    imports.authored_animation(
        &world,
        &crate::ui::inventory::InventoryPreferences::default(),
        "Authored".into(),
        bytes.clone(),
        animation,
    );
    assert!(imports.open);
    assert_eq!(imports.inspector, Some(Choice::Animation));
    assert_eq!(imports.entries.len(), 1);
    let entry = &imports.entries[0];
    assert_eq!(imports.active, Some(entry.id));
    assert!(matches!(entry.status, Status::Ready));
    let Some(Source::Animation {
        bytes: stored,
        animation,
        bvh: None,
    }) = &entry.source
    else {
        panic!("native source")
    };
    assert_eq!(**stored, *bytes);
    assert_eq!(animation.joints[0].priority, 5);
    assert!(imports.queue.is_empty());
    assert!(imports.current.is_none());
    assert!(!imports.confirm);
    assert_eq!(imports.spent, 0);
}
use std::time::{Duration, Instant};

#[test]
fn texture_size_limit_defaults_off_and_new_rows_capture_the_latest_persisted_choice() {
    let mut prefs: Preferences = serde_json::from_str(r#"{"max_width":1024,"max_height":1024}"#).expect("old settings");
    assert!(
        !prefs.limit_texture_size,
        "the old implicit size limit is not an explicit new choice"
    );
    let ctx = egui::Context::default();
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    let mut imports = Imports::default();
    for limit in [false, true, false] {
        let previous: Vec<_> = imports.entries.iter().map(|entry| entry.options).collect();
        imports.load_preferences = prefs.clone();
        let request = Uuid::new_v4();
        imports.pending_files = Some(request);
        let loaded = vec![
            Entry::new("large.png".into(), Ok(Source::Texture(image(2048, 1024))), Default::default()),
            Entry::new("small.png".into(), Ok(Source::Texture(image(240, 180))), Default::default()),
            Entry::new("rounded.png".into(), Ok(Source::Texture(image(1100, 900))), Default::default()),
            Entry::new(
                "material.gltf".into(),
                Ok(Source::Material {
                    value: PbrMaterial::default(),
                    images: Box::new([None, None, None, None]),
                }),
                Default::default(),
            ),
        ];
        imports.tx.send(Worker::Loaded(request, loaded)).expect("loaded files");
        // The user can change the choice while the file worker is running.
        imports.load_preferences.limit_texture_size = limit;
        assert!(imports.poll(&ctx, &jobs, &world, &Default::default()).is_empty());
        for (entry, old) in imports.entries.iter().zip(&previous) {
            assert_eq!(&entry.options, old, "existing files keep their own limit");
        }
        let added = &imports.entries[previous.len()..];
        let maximum = if limit { 1024 } else { 2048 };
        assert_eq!((added[0].options.max_width, added[0].options.max_height), (maximum, maximum));
        for entry in &added[1..3] {
            assert_eq!(
                (entry.options.max_width, entry.options.max_height),
                (2048, 2048),
                "no unnecessary reduction for small uploads"
            );
        }
        assert_eq!(
            added[2].source.as_ref().expect("rounded image").dimensions(added[2].options),
            (1024, 1024)
        );
        assert_eq!(
            added[3].options,
            import_texture::Options::default(),
            "material-map defaults stay independent"
        );
        prefs.limit_texture_size = limit;
        prefs = serde_json::from_slice(&serde_json::to_vec(&prefs).expect("save preferences")).expect("restore preferences");
        assert_eq!(prefs.limit_texture_size, limit);
    }
}

#[test]
fn per_type_permissions_preserve_old_settings_and_transfer_without_copy() {
    use aurora_net::build::perm;
    use permissions::NextOwner;
    let mut prefs: Preferences =
        serde_json::from_str(r#"{"next_modify":true,"next_copy":true,"next_transfer":false}"#).expect("old settings");
    let old = prefs.permissions();
    for kind in [Kind::Texture, Kind::Sound, Kind::Animation, Kind::Mesh, Kind::Material] {
        assert_eq!(
            prefs.permissions_for(kind),
            old,
            "old common rights migrate without changing any type"
        );
    }
    prefs.permissions_by_type.set(
        Kind::Texture,
        NextOwner {
            modify: false,
            copy: true,
            transfer: false,
            share_group: None,
            everyone_copy: None,
        },
    );
    prefs.permissions_by_type.set(
        Kind::Mesh,
        NextOwner {
            modify: true,
            copy: false,
            transfer: false,
            share_group: None,
            everyone_copy: None,
        },
    );
    assert_eq!(prefs.permissions_for(Kind::Texture).next_owner, perm::MOVE | perm::COPY);
    assert_eq!(
        prefs.permissions_for(Kind::Mesh).next_owner,
        perm::MOVE | perm::MODIFY | perm::TRANSFER
    );
    assert_eq!(prefs.permissions_for(Kind::Material), old);
    let saved = serde_json::to_vec(&prefs).expect("settings");
    let restored: Preferences = serde_json::from_slice(&saved).expect("saved settings");
    assert_eq!(restored, prefs);
}

#[test]
fn legacy_sharing_migrates_to_each_type_and_new_rows_remain_independent() {
    use aurora_net::build::perm;
    let mut prefs: Preferences = serde_json::from_str(
        r#"{"share_group":true,"everyone_copy":true,"permissions_by_type":{"texture":{"modify":false,"copy":true,"transfer":false}}}"#,
    )
    .expect("legacy sharing");
    for kind in [Kind::Texture, Kind::Sound, Kind::Animation, Kind::Mesh, Kind::Material] {
        assert_eq!(prefs.permissions_for(kind).group, perm::COPY | perm::MOVE | perm::MODIFY);
        assert_eq!(prefs.permissions_for(kind).everyone, perm::COPY);
    }
    let mut sound = prefs.next_owner_for(Kind::Sound);
    sound.share_group = Some(false);
    sound.everyone_copy = Some(false);
    prefs.permissions_by_type.set(Kind::Sound, sound);
    assert_eq!(prefs.permissions_for(Kind::Sound).group, 0);
    assert_eq!(prefs.permissions_for(Kind::Sound).everyone, 0);
    assert_ne!(prefs.permissions_for(Kind::Texture).group, 0);
    assert_ne!(prefs.permissions_for(Kind::Mesh).everyone, 0);
    let restored: Preferences = serde_json::from_slice(&serde_json::to_vec(&prefs).expect("save")).expect("restore");
    assert_eq!(restored, prefs);
}

#[test]
fn mixed_uploads_freeze_each_type_and_material_textures_use_texture_permissions() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::{
        build::perm,
        inventory::{InvFolder, mesh_upload::Quote},
    };
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.balance = Some(1000);
    let folder = Uuid::from_u128(700);
    world.inventory.root = folder;
    world.inventory.folders.insert(
        folder,
        Folder {
            info: InvFolder {
                id: folder,
                type_default: 8,
                name: "Inventory".into(),
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    let map = files::local_image((*image(4, 4)).clone(), "material map".into());
    let mut mesh = mesh_fixture();
    mesh.quote = Some(Quote {
        token: Uuid::new_v4(),
        cost: 20,
        data: aurora_llsd::Llsd::Undef,
    });
    let mut imports = Imports {
        choice: Choice::Bulk,
        ..Default::default()
    };
    for (name, source) in [
        ("Texture", Source::Texture(image(4, 4))),
        (
            "Sound",
            Source::Sound(Arc::new(Sound {
                source_channels: 1,
                source_bits: 16,
                mono: vec![0.8; 44100],
            })),
        ),
        (
            "Material",
            Source::Material {
                value: Default::default(),
                images: Box::new([Some(map), None, None, None]),
            },
        ),
        ("Mesh", Source::Mesh(Box::new(mesh))),
    ] {
        let mut entry = Entry::new(name.into(), Ok(source), Default::default());
        entry.folder = folder;
        if matches!(entry.source, Some(Source::Sound(_))) {
            entry.sound.options = sound::Options {
                gain: 0.5,
                start: 0.25,
                end: Some(0.75),
                ..Default::default()
            };
        }
        imports.entries.push(entry);
    }
    let mut prefs = Preferences::default();
    prefs.permissions_by_type.set(
        Kind::Texture,
        permissions::NextOwner {
            modify: false,
            copy: true,
            transfer: false,
            share_group: None,
            everyone_copy: None,
        },
    );
    prefs.permissions_by_type.set(
        Kind::Material,
        permissions::NextOwner {
            modify: true,
            copy: true,
            transfer: false,
            share_group: Some(false),
            everyone_copy: Some(false),
        },
    );
    prefs.share_group = true;
    prefs.everyone_copy = true;
    imports.start(&world, &Default::default(), &prefs, &prices());
    assert_eq!(
        imports.queue.len(),
        5,
        "four assets and their material's texture: {}",
        imports.message
    );
    prefs.permissions_by_type.set(
        Kind::Texture,
        permissions::NextOwner {
            modify: false,
            copy: false,
            transfer: true,
            share_group: None,
            everyone_copy: None,
        },
    );
    prefs.share_group = false;
    prefs.everyone_copy = false;
    for entry in &mut imports.entries {
        entry.sound.options = Default::default();
    }
    for step in &imports.queue {
        assert_eq!(
            step.permissions.group,
            if step.kind == Kind::Material {
                0
            } else {
                perm::COPY | perm::MOVE | perm::MODIFY
            }
        );
        assert_eq!(step.permissions.everyone, if step.kind == Kind::Material { 0 } else { perm::COPY });
        if let Source::Sound(source) = &step.source {
            let edited = step.sound_options.render(source).expect("frozen sound edit");
            assert_eq!(edited.duration(), 0.5);
            assert_eq!(edited.mono[0], 0.4);
            assert_eq!(source.mono.len(), 44100);
        }
        let expected = match step.kind {
            Kind::Texture => perm::MOVE | perm::COPY,
            Kind::Material => perm::MOVE | perm::COPY | perm::MODIFY,
            Kind::Mesh | Kind::Sound => perm::MOVE | perm::TRANSFER,
            _ => panic!("unexpected fixture"),
        };
        assert_eq!(
            step.permissions.next_owner, expected,
            "the confirmed queue retains the rights for its type"
        );
    }
}

#[test]
fn sound_split_cost_names_snapshots_and_partial_retry_match_the_real_queue() {
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::InvFolder;
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.balance = Some(1000);
    let folder = Uuid::from_u128(730);
    world.inventory.root = folder;
    world.inventory.folders.insert(
        folder,
        Folder {
            info: InvFolder {
                id: folder,
                type_default: 8,
                name: "Inventory".into(),
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    let source = Arc::new(Sound {
        source_channels: 1,
        source_bits: 16,
        mono: vec![0.3; 44100 * 7],
    });
    let mut entry = Entry::new("Track.wav".into(), Ok(Source::Sound(source.clone())), Default::default());
    entry.folder = folder;
    entry.sound.options = sound::Options {
        gain: 2.0,
        start: 0.5,
        end: Some(6.0),
        segment: Some(2.0),
        ..Default::default()
    };
    let edit = entry.sound.options;
    let mut imports = Imports {
        choice: Choice::Sound,
        entries: vec![entry],
        ..Default::default()
    };
    assert_eq!(imports.total(&prices()), Some(30));
    imports.start(&world, &Default::default(), &Default::default(), &prices());
    assert_eq!(
        imports.queue.iter().map(|step| step.name.as_str()).collect::<Vec<_>>(),
        ["Track_01", "Track_02", "Track_03"]
    );
    imports.entries[0].sound.options.gain = 1.0;
    for step in &imports.queue {
        let Source::Sound(sound) = &step.source else { panic!("sound") };
        assert!(
            Arc::ptr_eq(sound, &source),
            "pieces share the original instead of copying it on the UI thread"
        );
        let rendered = step.sound_options.render(sound).expect("piece");
        assert!(rendered.duration() <= 2.0);
        assert_eq!(rendered.mono[0], 0.6, "confirmed gain remains frozen");
    }
    imports.entries[0].sound.options = edit;
    let request = Uuid::new_v4();
    let first = imports.queue.pop_front().expect("first piece");
    imports.current = Some((request, first));
    assert!(
        imports
            .uploaded(
                request,
                Ok(Receipt {
                    item: aurora_net::inventory::InvItem::default(),
                    cost: 10
                })
            )
            .is_some()
    );
    assert!(
        matches!(imports.entries[0].status, Status::Preparing),
        "a partial success keeps its parent draft"
    );
    let request = Uuid::new_v4();
    let second = imports.queue.pop_front().expect("second piece");
    imports.current = Some((request, second));
    imports.uploaded(request, Err(Error::Verification));
    assert!(!imports.busy());
    imports.restore_draft(imports.entries[0].id);
    assert_eq!(imports.total(&prices()), Some(20), "acknowledged pieces are not charged again");
    imports.start(&world, &Default::default(), &Default::default(), &prices());
    assert_eq!(
        imports.queue.iter().map(|step| step.name.as_str()).collect::<Vec<_>>(),
        ["Track_02", "Track_03"]
    );
    assert_eq!(source.mono[0], 0.3);
}

#[test]
fn enlarged_maps_keep_resolution_and_reject_replaced_or_closed_previews() {
    let ctx = egui::Context::default();
    let (jobs, _) = crate::scene::jobs::Jobs::new();
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let map = files::local_image((*image(400, 200)).clone(), "rectangular map".into());
    let entry = Entry::new(
        "material.gltf".into(),
        Ok(Source::Material {
            value: Default::default(),
            images: Box::new([Some(map.clone()), None, None, None]),
        }),
        Default::default(),
    );
    let id = entry.id;
    let mut imports = Imports {
        entries: vec![entry],
        preview_zoom: Some((id, Some(0))),
        ..Default::default()
    };
    imports.request_large_map(id, 0, &jobs, &ctx);
    let deadline = Instant::now() + Duration::from_secs(10);
    while imports.entries[0].material_large_pending[0] {
        assert!(Instant::now() < deadline, "large-map worker completed");
        imports.poll(&ctx, &jobs, &world, &Default::default());
        std::thread::sleep(Duration::from_millis(2));
    }
    let cached = imports.entries[0].material_large_previews[0].take().expect("large map prepared");
    assert_eq!(
        cached.2.size(),
        [400, 200],
        "the loupe uses original pixels rather than the 96-pixel thumbnail"
    );
    imports.entries[0].preview_generation = 1;
    let reply = || {
        Worker::MaterialLargePreview(
            id,
            0,
            0,
            map.key,
            Some(egui::ColorImage::filled([2, 1], egui::Color32::TRANSPARENT)),
        )
    };
    imports.tx.send(reply()).expect("worker response");
    imports.poll(&ctx, &jobs, &world, &Default::default());
    assert!(
        imports.entries[0].material_large_previews[0].is_none(),
        "an obsolete response cannot replace the current map"
    );
    imports.entries[0].preview_generation = 0;
    imports.preview_zoom = None;
    imports.tx.send(reply()).expect("worker response");
    imports.poll(&ctx, &jobs, &world, &Default::default());
    assert!(
        imports.entries[0].material_large_previews[0].is_none(),
        "closed previews do not retain a large GPU texture"
    );
}

#[test]
fn batch_mesh_quotes_are_required_and_added_to_the_total() {
    let mut imports = Imports {
        choice: Choice::Bulk,
        ..Default::default()
    };
    imports.entries.push(Entry::new(
        "texture.png".into(),
        Ok(Source::Texture(image(16, 16))),
        Default::default(),
    ));
    for name in ["first.dae", "second.glb"] {
        let entry = Entry::new(name.into(), Ok(Source::Mesh(Box::new(mesh_fixture()))), Default::default());
        assert!(Choice::Bulk.accepts(&entry));
        imports.entries.push(entry);
    }
    assert_eq!(imports.total(&prices()), None);
    for (index, entry) in imports.entries[1..].iter_mut().enumerate() {
        let Some(Source::Mesh(draft)) = &mut entry.source else {
            panic!("mesh")
        };
        draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
            token: Uuid::new_v4(),
            cost: 20 + index as i32,
            data: aurora_llsd::Llsd::Undef,
        });
    }
    assert_eq!(imports.total(&prices()), Some(51));
    imports.entries[2].selected = false;
    assert_eq!(imports.total(&prices()), Some(30));
    imports.entries[1].local = true;
    assert!(!Choice::Bulk.accepts(&imports.entries[1]));
    assert_eq!(imports.total(&prices()), Some(10));
}

pub(super) fn mesh_fixture() -> mesh::Draft {
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml")).expect("skeleton");
    let scene = aurora_assets::import_mesh::collada::parse(demo::DAE.as_bytes(), &skeleton).expect("synthetic mesh");
    mesh::Draft::new(scene, HashMap::new())
}

#[test]
fn mesh_multi_object_file_preserves_instances_through_preparation_and_fee_payload() {
    let scene = aurora_assets::import_mesh::collada::parse(demo::multiple_dae().as_bytes(), &aurora_assets::Skeleton::default())
        .expect("multi-object file");
    let draft = mesh::Draft::new(scene, HashMap::new());
    let prepared = mesh::prepare(&draft).expect("prepare all objects");
    let request = mesh::request(
        &draft,
        &prepared,
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "Assembly",
        "Three objects",
        Permissions::default(),
        Default::default(),
    )
    .expect("encode multi-object upload");
    let query = aurora_llsd::from_xml(&request.query).expect("fee query");
    let resources = aurora_llsd::from_xml(&request.resources).expect("upload resources");
    assert_eq!(query["asset_resources"]["instance_list"], resources["instance_list"]);
    assert_eq!(resources["mesh_list"].as_array().len(), 1, "instances share the uploaded geometry");
    assert_eq!(resources["instance_list"].as_array().len(), 3);
    for (instance, (name, x)) in resources["instance_list"]
        .as_array()
        .iter()
        .zip([("Left", -1.0), ("Middle", 1.0), ("Right", 3.0)])
    {
        assert_eq!(instance["mesh"].as_i32(), 0);
        assert_eq!(instance["mesh_name"].as_str(), name);
        assert_eq!(
            instance["position"].as_array().iter().map(|v| v.as_f64()).collect::<Vec<_>>(),
            [x, 2.0, 0.0]
        );
    }
}

#[test]
fn mesh_upload_eleven_materials_keep_valid_collision_blocks_in_both_payloads() {
    use aurora_llsd::Llsd;
    use std::io::Read;

    let faces = (0..11)
        .map(|i| {
            format!(
                r##"<triangles count="12" material="surface-{i}"><input semantic="VERTEX" source="#vertices" offset="0"/><p>0 2 1 0 3 2 4 5 6 4 6 7 0 1 5 0 5 4 3 7 6 3 6 2 0 4 7 0 7 3 1 2 6 1 6 5</p></triangles>"##
            )
        })
        .collect::<String>();
    let start = demo::DAE.find("<triangles").expect("geometry start");
    let end = demo::DAE.find("</triangles>").expect("geometry end") + "</triangles>".len();
    let dae = format!("{}{}{}", &demo::DAE[..start], faces, &demo::DAE[end..]);
    let scene =
        aurora_assets::import_mesh::collada::parse(dae.as_bytes(), &aurora_assets::Skeleton::default()).expect("eleven material slots");
    assert_eq!(scene.models.iter().map(|m| m.faces.len()).collect::<Vec<_>>(), [8, 3]);
    assert_eq!(scene.models.iter().map(|m| m.submodel).collect::<Vec<_>>(), [0, 1]);
    let draft = mesh::Draft::new(scene, HashMap::new());
    let prepared = mesh::prepare(&draft).expect("prepare both models");
    let request = mesh::request(
        &draft,
        &prepared,
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "Eleven surfaces",
        "",
        Permissions::default(),
        Default::default(),
    )
    .expect("mesh request");
    let query = aurora_llsd::from_xml(&request.query).expect("fee request");
    let resources = aurora_llsd::from_xml(&request.resources).expect("final resources");
    assert_eq!(query["asset_resources"]["mesh_list"], resources["mesh_list"]);
    assert_eq!(query["asset_resources"]["instance_list"], resources["instance_list"]);
    assert_eq!(resources["mesh_list"].as_array().len(), 2);
    assert_eq!(resources["instance_list"].as_array().len(), 2);
    assert_eq!(resources["instance_list"][0]["physics_shape_type"].as_i32(), 2);
    assert_eq!(resources["instance_list"][1]["physics_shape_type"].as_i32(), 1);
    for (i, encoded) in resources["mesh_list"].as_array().iter().enumerate() {
        let encoded = encoded.as_binary();
        let (header, start) = aurora_llsd::from_binary(encoded).expect("mesh header");
        let range = &header["physics_convex"];
        assert!(range.is_map(), "model {i} needs its default collision block");
        let offset = start + range["offset"].as_i32() as usize;
        let size = range["size"].as_i32() as usize;
        let mut bytes = Vec::new();
        flate2::read::ZlibDecoder::new(&encoded[offset..offset + size])
            .read_to_end(&mut bytes)
            .expect("collision section");
        let (convex, _) = aurora_llsd::from_binary(&bytes).expect("convex LLSD");
        assert!(matches!(convex["BoundingVerts"], Llsd::Binary(_)), "model {i} has a real base hull");
        assert!(convex["BoundingVerts"].as_binary().len() >= 4 * 6);
        assert!(
            !header.has("submodel_id"),
            "a model with a real base hull needs no placeholder marker"
        );
        assert!(!header.has("physics_mesh"));
        let header = aurora_assets::parse_mesh_header(encoded).expect("production mesh header");
        for (level, section) in header.lods.into_iter().enumerate() {
            let section = section.expect("all four LODs are included");
            let decoded = aurora_assets::decode_mesh_lod(&encoded[section.0..section.0 + section.1]).expect("LOD geometry");
            assert_eq!(decoded.len(), if i == 0 { 8 } else { 3 });
            if level == 3 {
                assert_eq!(
                    decoded.iter().map(|f| f.indices.len() / 3).sum::<usize>(),
                    if i == 0 { 96 } else { 36 }
                );
            }
        }
    }
}

#[test]
fn mesh_edits_keep_the_last_preview_debounce_new_jobs_and_replace_only_the_current_generation() {
    let mut entry = Entry::new("mesh.dae".into(), Ok(Source::Mesh(Box::new(mesh_fixture()))), Default::default());
    let model = Arc::new(aurora_render::preview::Model {
        parts: Vec::new(),
        center: glam::Vec3::ZERO,
        radius: 1.0,
    });
    entry.model = Some(model.clone());
    entry.model_generation = Some(0);
    entry.invalidate_preview();
    entry.preview_not_before = Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
    assert!(Arc::ptr_eq(entry.model.as_ref().expect("previous model visible"), &model));
    let id = entry.id;
    let mut imports = Imports {
        entries: vec![entry],
        ..Default::default()
    };
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let world = crate::world::World::new(avatars.clone());
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    let ctx = egui::Context::default();
    imports.request_model(id, &jobs, &ctx, avatars);
    assert!(!imports.entries[0].preview_pending, "editing has not settled: no worker started");
    let replacement = Arc::new(aurora_render::preview::Model {
        parts: Vec::new(),
        center: glam::Vec3::ONE,
        radius: 2.0,
    });
    let obsolete = Arc::new(aurora_render::preview::Model {
        parts: Vec::new(),
        center: glam::Vec3::ONE,
        radius: 9.0,
    });
    imports.tx.send(Worker::Model(id, 0, obsolete, None)).expect("obsolete result");
    imports.poll(&ctx, &jobs, &world, &Default::default());
    assert!(Arc::ptr_eq(
        imports.entries[0].model.as_ref().expect("previous model retained"),
        &model
    ));
    imports
        .tx
        .send(Worker::Model(id, 1, replacement.clone(), None))
        .expect("current result");
    imports.poll(&ctx, &jobs, &world, &Default::default());
    assert!(Arc::ptr_eq(imports.entries[0].model.as_ref().expect("replacement"), &replacement));
    assert_eq!(imports.entries[0].model_generation, Some(1));
    assert!(imports.entries[0].preview_not_before.is_none());
    imports.entries[0].preview_pending = true;
    imports.entries[0].preview_not_before = Some(std::time::Instant::now());
    imports
        .tx
        .send(Worker::PreviewFailed(id, 1, "invalid physics".into()))
        .expect("preparation failure");
    imports.poll(&ctx, &jobs, &world, &Default::default());
    assert!(!imports.entries[0].preview_pending);
    assert!(
        imports.entries[0].preview_not_before.is_none(),
        "failed work must stop the loading indicator"
    );
    assert!(Arc::ptr_eq(
        imports.entries[0].model.as_ref().expect("previous model retained after failure"),
        &replacement
    ));
}

#[test]
fn avatar_preview_preference_round_trips_on_and_off_and_defaults_off() {
    assert!(!serde_json::from_str::<Preferences>("{}").expect("old settings").preview_on_avatar);
    for enabled in [true, false] {
        let prefs = Preferences {
            preview_on_avatar: enabled,
            ..Default::default()
        };
        let stored = serde_json::to_vec(&prefs).expect("save");
        assert_eq!(
            serde_json::from_slice::<Preferences>(&stored).expect("reload").preview_on_avatar,
            enabled
        );
    }
}

#[test]
fn triangle_budget_ignores_the_inactive_error_threshold_and_reduces_lods() {
    let mut draft = mesh_fixture();
    for lod in &mut draft.lods[..3] {
        lod.error = 0.0;
    }
    let prepared = mesh::prepare(&draft).expect("prepare");
    let high = prepared.lods[0][3].as_ref().expect("high").triangles();
    assert_eq!(high, 12);
    for lod in &prepared.lods[0][..3] {
        assert!(
            lod.as_ref().expect("generated LOD").triangles() < high,
            "triangle mode must reduce this closed model even with a saved zero error threshold"
        );
    }
    draft.lods[2].error_mode = true;
    let by_error = mesh::prepare(&draft).expect("error mode");
    assert_eq!(by_error.lods[0][2].as_ref().expect("medium").triangles(), high);
}

#[test]
fn loading_a_valid_lod_recovers_from_a_previous_preview_failure_and_ignores_stale_errors() {
    let mut draft = mesh_fixture();
    draft.lods[2].source = mesh::LodSource::File;
    let replacement = draft.scene.clone();
    let mut entry = Entry::new("base.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
    let error = "invalid asset data: LOD file missing".to_owned();
    entry.preview_error = Some(error.clone());
    let id = entry.id;
    let mut imports = Imports {
        entries: vec![entry],
        message: error.clone(),
        ..Default::default()
    };
    imports
        .tx
        .send(Worker::MeshFile(id, 2, Ok((replacement, HashMap::new(), "replacement.dae".into()))))
        .expect("file result");
    imports.tx.send(Worker::PreviewFailed(id, 0, error)).expect("stale result");
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    assert!(
        imports
            .poll(
                &egui::Context::default(),
                &jobs,
                &world,
                &crate::ui::inventory::InventoryPreferences::default()
            )
            .is_empty()
    );
    let entry = &imports.entries[0];
    assert!(entry.preview_error.is_none());
    assert!(imports.message.is_empty());
    assert_eq!(entry.preview_generation, 1);
    let Some(Source::Mesh(draft)) = &entry.source else { panic!("mesh") };
    assert!(draft.lods[2].file.is_some());
    assert!(mesh::prepare(draft).is_ok());
}

fn prices() -> Prices {
    Prices {
        texture: Some(10),
        large_texture: Some(40),
        sound: Some(10),
        animation: Some(10),
    }
}

#[test]
fn mesh_import_factor_obeys_prim_size_and_linkset_span() {
    use aurora_assets::import_mesh::{Instance, Scene};
    use glam::{Mat4, Vec3};
    let instance = |x| Instance {
        model: 0,
        name: "Part".into(),
        materials: Vec::new(),
        transform: Mat4::from_scale_rotation_translation(Vec3::splat(2.0), glam::Quat::IDENTITY, Vec3::X * x),
    };
    let mut draft = mesh::Draft::new(
        Scene {
            instances: vec![instance(0.0)],
            ..Default::default()
        },
        HashMap::new(),
    );
    draft.options.scale = 2.0;
    assert_eq!(draft.dimensions_and_limit(), (Vec3::splat(4.0), 32.0));
    Arc::make_mut(&mut draft.scene).instances.push(instance(239.0));
    let (dimensions, limit) = draft.dimensions_and_limit();
    assert_eq!(dimensions, Vec3::new(482.0, 4.0, 4.0));
    assert!((limit - 240.0 / 241.0).abs() < 0.00001);
}

#[test]
fn instance_edits_retire_the_quote_and_keep_prepared_geometry() {
    let mut draft = mesh::Draft::new(Default::default(), HashMap::new());
    let prepared = Arc::new(mesh::Prepared {
        lods: Vec::new(),
        physics: Vec::new(),
        hulls: Vec::new(),
        degenerate_triangles: 0,
    });
    draft.prepared = Some(prepared.clone());
    let token = Uuid::new_v4();
    draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
        token,
        cost: 10,
        data: aurora_llsd::Llsd::Undef,
    });
    let before = draft.options.clone();
    draft.options.scale = 0.5;
    draft.options.pelvis = 0.2;
    assert!(draft.finish_edit(&before, false));
    assert!(Arc::ptr_eq(draft.prepared.as_ref().expect("geometry"), &prepared));
    assert!(draft.quote.is_none());
    assert_eq!(draft.discarded_quotes, [token]);
    let before = draft.options.clone();
    draft.options.normals = true;
    assert!(draft.finish_edit(&before, false));
    assert!(draft.prepared.is_none());
}

#[test]
fn restoring_a_failed_draft_keeps_material_edits_without_sending_or_clearing_the_warning() {
    let mut imports = Imports::default();
    let material = PbrMaterial {
        roughness_factor: 0.123,
        base_color_factor: [0.2, 0.4, 0.6, 1.0],
        ..Default::default()
    };
    let mut entry = Entry::new(
        "material.gltf".into(),
        Ok(Source::Material {
            value: material.clone(),
            images: Box::new([None, None, None, None]),
        }),
        Default::default(),
    );
    entry.status = Status::Error("Unknown result".into());
    let id = entry.id;
    imports.entries.push(entry);
    imports.message = "Verify inventory before retrying".into();
    imports.restore_draft(id);
    let Some(Source::Material { value, .. }) = &imports.entries[0].source else {
        panic!("material")
    };
    assert_eq!(*value, material);
    assert!(matches!(imports.entries[0].status, Status::Ready));
    assert!(imports.current.is_none() && imports.queue.is_empty());
    assert_eq!(imports.message, "Verify inventory before retrying");
}
pub(super) fn image(w: u32, h: u32) -> Arc<DecodedImage> {
    Arc::new(DecodedImage {
        width: w,
        height: h,
        components: 3,
        data: vec![128; w as usize * h as usize * 3],
        discard: 0,
    })
}
#[test]
fn shared_material_maps_are_charged_once_with_equivalent_effective_encoding() {
    let pixels = image(4, 4);
    let map = LocalImage {
        path: None,
        key: Uuid::new_v4(),
        name: "Shared".into(),
        pixels,
        asset: None,
        empty_alpha: false,
    };
    let source = Source::Material {
        value: PbrMaterial::default(),
        images: Box::new([Some(map), None, None, None]),
    };
    let mut imports = Imports {
        choice: Choice::Material,
        ..Default::default()
    };
    imports
        .entries
        .push(Entry::new("a.gltf".into(), Ok(source.clone()), Default::default()));
    let mut second = Entry::new("b.gltf".into(), Ok(source), Default::default());
    second.options.max_width = 2048;
    second.options.max_height = 2048;
    imports.entries.push(second);
    assert_eq!(imports.total(&prices()), Some(10));
    if let Some(Source::Material { images, .. }) = &mut imports.entries[1].source {
        images[0].as_mut().expect("map").asset = Some(Uuid::new_v4());
    }
    imports.entries[0].selected = false;
    assert_eq!(imports.total(&prices()), Some(0));
}
#[test]
fn bulk_scaling_uses_area_pricing_and_failed_selected_rows_block_payment() {
    let mut imports = Imports {
        choice: Choice::Bulk,
        ..Default::default()
    };
    let options = import_texture::Options {
        max_width: 2048,
        max_height: 2048,
        ..Default::default()
    };
    imports
        .entries
        .push(Entry::new("large.png".into(), Ok(Source::Texture(image(2048, 1024))), options));
    assert_eq!(imports.total(&prices()), Some(40));
    imports.scale_bulk_1k = true;
    assert_eq!(imports.total(&prices()), Some(10));
    let mut failed = Entry::new("broken.png".into(), Err("Invalid".into()), options);
    failed.selected = true;
    imports.entries.push(failed);
    assert_eq!(imports.total(&prices()), None);
}
#[test]
fn cancelling_future_uploads_retains_the_in_flight_receipt() {
    let mut imports = Imports::default();
    let entry = Entry::new("texture.png".into(), Ok(Source::Texture(image(4, 4))), Default::default());
    let step = Step {
        key: entry.id,
        entry: Some(entry.id),
        kind: Kind::Texture,
        folder: Uuid::new_v4(),
        name: entry.name.clone(),
        description: String::new(),
        source: entry.source.clone().expect("source"),
        options: entry.options,
        sound_options: Default::default(),
        cost: 10,
        permissions: Permissions::default(),
    };
    let request = Uuid::new_v4();
    imports.current = Some((request, step));
    imports.entries.push(entry);
    imports.entries[0].status = Status::Uploading;
    imports.cancel_remaining();
    let item = aurora_net::inventory::InvItem {
        id: Uuid::new_v4(),
        asset_id: Uuid::new_v4(),
        ..Default::default()
    };
    assert!(
        imports
            .uploaded(
                Uuid::new_v4(),
                Ok(Receipt {
                    item: item.clone(),
                    cost: 10
                })
            )
            .is_none()
    );
    assert!(imports.current.is_some());
    assert_eq!(
        imports
            .uploaded(
                request,
                Ok(Receipt {
                    item: item.clone(),
                    cost: 10
                })
            )
            .expect("receipt")
            .id,
        item.id
    );
    assert!(matches!(imports.entries[0].status, Status::Done(_)));
    assert_eq!(imports.spent, 10);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    imports.poll(&egui::Context::default(), &jobs, &world, &Default::default());
    imports.open(Choice::Bulk);
    assert!(imports.entries.is_empty());
    assert!(imports.active.is_none());
}

#[test]
fn acknowledged_uploads_leave_local_previews_and_unfinished_drafts_untouched() {
    let ctx = egui::Context::default();
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    let mut imports = Imports {
        choice: Choice::Bulk,
        ..Default::default()
    };
    let mut local = Entry::new("local.png".into(), Ok(Source::Texture(image(4, 4))), Default::default());
    local.local = true;
    local.local_published = true;
    local.preview = Some(ctx.load_texture(
        "local_preview",
        egui::ColorImage::filled([4, 4], egui::Color32::TRANSPARENT),
        Default::default(),
    ));
    let local_id = local.id;
    let local_texture = local.preview.as_ref().expect("local preview").id();
    let slot = Uuid::new_v4();
    local.local_slots.insert(slot, 42);
    imports.entries.push(local);
    let mut unselected = Entry::new("later.png".into(), Ok(Source::Texture(image(4, 4))), Default::default());
    unselected.selected = false;
    let unselected_id = unselected.id;
    imports.entries.push(unselected);
    let mut cancelled = Entry::new("cancelled.png".into(), Ok(Source::Texture(image(4, 4))), Default::default());
    cancelled.status = Status::Cancelled;
    let cancelled_id = cancelled.id;
    imports.entries.push(cancelled);
    let mut failed = Entry::new("failed.png".into(), Ok(Source::Texture(image(4, 4))), Default::default());
    failed.status = Status::Uploading;
    let failed_id = failed.id;
    imports.entries.push(failed);
    let mut done = Entry::new("done.png".into(), Ok(Source::Texture(image(4, 4))), Default::default());
    done.status = Status::Uploading;
    let done_id = done.id;
    imports.entries.push(done);
    imports.active = Some(done_id);
    let begin = |imports: &mut Imports, id| {
        let entry = imports.entries.iter().find(|entry| entry.id == id).expect("draft");
        let request = Uuid::new_v4();
        imports.current = Some((
            request,
            Step {
                key: id,
                entry: Some(id),
                kind: Kind::Texture,
                folder: entry.folder,
                name: entry.name.clone(),
                description: entry.description.clone(),
                source: entry.source.clone().expect("source"),
                options: entry.options,
                sound_options: Default::default(),
                cost: 10,
                permissions: Default::default(),
            },
        ));
        request
    };
    let request = begin(&mut imports, done_id);
    imports.uploaded(
        request,
        Ok(Receipt {
            item: aurora_net::inventory::InvItem::default(),
            cost: 10,
        }),
    );
    let request = begin(&mut imports, failed_id);
    imports.uploaded(request, Err(Error::Verification));
    imports.poll(&ctx, &jobs, &world, &Default::default());
    assert_eq!(
        imports.entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        [local_id, unselected_id, cancelled_id, failed_id]
    );
    assert_eq!(imports.active, Some(unselected_id));
    assert!(matches!(imports.entries[2].status, Status::Cancelled));
    assert!(matches!(imports.entries[3].status, Status::Error(_)));
    assert!(!imports.message.is_empty());
    assert_eq!(imports.spent, 10);
    imports.open(Choice::Local);
    assert_eq!(imports.active, Some(local_id));
    let local = &imports.entries[0];
    assert!(local.local_published);
    assert_eq!(local.preview.as_ref().expect("retained local preview").id(), local_texture);
    assert_eq!(local.local_slots[&slot], 42);
    assert!(imports.retired_local_slots.is_empty());
}

#[test]
fn retiring_material_rows_keeps_texture_receipts_for_the_rest_of_the_batch() {
    let ctx = egui::Context::default();
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    let map = files::local_image((*image(4, 4)).clone(), "shared.png".into());
    let key = map.upload_key(Default::default());
    let source = Source::Material {
        value: PbrMaterial::default(),
        images: Box::new([Some(map.clone()), None, None, None]),
    };
    let mut imports = Imports {
        choice: Choice::Bulk,
        ..Default::default()
    };
    for name in ["first.gltf", "second.gltf"] {
        let mut entry = Entry::new(name.into(), Ok(source.clone()), Default::default());
        entry.status = Status::Preparing;
        imports.queue.push_back(Step {
            key: entry.id,
            entry: Some(entry.id),
            kind: Kind::Material,
            folder: Uuid::new_v4(),
            name: entry.name.clone(),
            description: String::new(),
            source: source.clone(),
            options: entry.options,
            sound_options: Default::default(),
            cost: 0,
            permissions: Default::default(),
        });
        imports.entries.push(entry);
    }
    imports.active = Some(imports.entries[0].id);
    let texture_request = Uuid::new_v4();
    imports.current = Some((
        texture_request,
        Step {
            key,
            entry: None,
            kind: Kind::Texture,
            folder: Uuid::new_v4(),
            name: map.name.clone(),
            description: String::new(),
            source: Source::Texture(map.pixels.clone()),
            options: Default::default(),
            sound_options: Default::default(),
            cost: 10,
            permissions: Default::default(),
        },
    ));
    let asset = Uuid::new_v4();
    imports.uploaded(
        texture_request,
        Ok(Receipt {
            item: aurora_net::inventory::InvItem {
                id: Uuid::new_v4(),
                asset_id: asset,
                ..Default::default()
            },
            cost: 10,
        }),
    );
    for remaining in [2, 1] {
        imports.poll(&ctx, &jobs, &world, &Default::default());
        assert_eq!(imports.entries.len(), remaining);
        assert_eq!(imports.active, Some(imports.entries[0].id));
        let (request, step) = imports.current.as_ref().expect("next material");
        let Source::Material { value, .. } = &step.source else {
            panic!("material upload")
        };
        assert_eq!(value.base_color_texture, Some(asset));
        imports.uploaded(
            *request,
            Ok(Receipt {
                item: aurora_net::inventory::InvItem {
                    id: Uuid::new_v4(),
                    ..Default::default()
                },
                cost: 0,
            }),
        );
    }
    imports.poll(&ctx, &jobs, &world, &Default::default());
    imports.open(Choice::Bulk);
    assert!(imports.entries.is_empty());
    assert!(imports.active.is_none());
    assert_eq!(imports.spent, 10);
    assert_eq!(imports.total(&prices()), Some(0));
    assert_eq!(imports.uploaded[&key].item.asset_id, asset);
}
