use super::*;
use crate::scene::anim::{AnimStreamer, BoundAnim};
use glam::{Mat4, Vec3};

fn entry() -> Entry {
    let animation = demo::tail_animation();
    Entry::new(
        "tail.anim".into(),
        Ok(Source::Animation {
            bytes: Arc::new(aurora_assets::import_animation::encode(&animation).expect("animation bytes")),
            animation: Arc::new(animation),
            bvh: None,
        }),
        Default::default(),
    )
}

fn model(avatars: &AvatarLibrary) -> Arc<Model> {
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")).expect("skeleton");
    let scene = aurora_assets::import_mesh::collada::parse(demo::tail_dae().as_bytes(), &skeleton).expect("tail model");
    prepare("tail.dae".into(), mesh::Draft::new(scene, HashMap::new()), avatars).expect("rigged preview")
}

#[test]
fn custom_animation_model_moves_bento_vertices_and_keeps_the_uploaded_animation_unchanged() {
    let avatars = AvatarLibrary::load();
    let model = model(&avatars);
    let mut entry = entry();
    let source = entry.source.as_ref().expect("animation");
    let before = files::encode(source, entry.options, entry.sound.options).expect("upload bytes");
    let Source::Animation { animation, .. } = source else {
        panic!("animation")
    };
    entry.player.bound = Some(Arc::new(BoundAnim::bind(animation.as_ref().clone(), &avatars.rig)));
    entry.animation_model.generation = 1;
    complete(&mut entry, 1, Some(Ok(model.clone())));
    let mut streamer = AnimStreamer::new(PathBuf::new());
    let part = model.geometry.parts.iter().find(|part| part.skin.is_some()).expect("weighted part");
    let skin = part.skin.as_ref().expect("skin");
    assert_eq!(skin.joints, vec![avatars.rig.names["mTail1"]]);
    let point = Vec3::from_array(part.vertices[0].position);
    let mut sample = |time| {
        entry.player.seek(time);
        entry.player.tick(entry.id, &avatars.rig, &mut streamer, Some(&model.base));
        let pose = entry.player.pose.as_ref().expect("sampled pose");
        let matrix = Mat4::from_cols_array_2d(&pose[skin.joints[0]]) * skin.inverse_bind[0];
        for vertex in part.vertices.iter() {
            assert!(
                matrix
                    .transform_point3(Vec3::from_array(vertex.position))
                    .distance(model.geometry.center)
                    <= model.geometry.radius + 0.0001,
                "the camera's model bounds must keep a rotating tail entirely in view"
            );
        }
        matrix.transform_point3(point)
    };
    let start = sample(0.0);
    let turned = sample(0.5);
    sample(1.5);
    assert!(start.distance(turned) > 0.01, "a Bento tail vertex must move with its animation");
    assert!(model.geometry.center.is_finite() && model.geometry.radius > 0.0);
    assert_eq!(
        files::encode(entry.source.as_ref().expect("source"), entry.options, entry.sound.options).expect("upload bytes"),
        before
    );
    reset(&mut entry);
    assert!(entry.animation_model.current.is_none());
    assert!(entry.player.pose.is_none(), "returning to standard must invalidate the paused pose");
    assert_eq!(
        files::encode(entry.source.as_ref().expect("source"), entry.options, entry.sound.options).expect("upload bytes"),
        before
    );
}

#[test]
fn custom_animation_model_preserves_imported_joint_positions_in_geometry_and_paused_poses() {
    let avatars = AvatarLibrary::load();
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")).expect("skeleton");
    let scene = aurora_assets::import_mesh::collada::parse(demo::tail_dae().as_bytes(), &skeleton).expect("tail");
    let mut draft = mesh::Draft::new(scene, HashMap::new());
    let offset = Vec3::new(-0.4, 0.1, 0.3);
    Arc::make_mut(&mut draft.scene).models[0]
        .skin
        .as_mut()
        .expect("skin")
        .alternate_bind[0] = Mat4::from_translation(offset);
    let model = prepare("offset-tail.dae".into(), draft, &avatars).expect("preview");
    let joint = avatars.rig.names["mTail1"];
    assert_eq!(model.base.local_pos[joint], offset);
    let mut entry = entry();
    let Source::Animation { animation, .. } = entry.source.as_ref().expect("source") else {
        panic!("animation")
    };
    entry.player.bound = Some(Arc::new(BoundAnim::bind(animation.as_ref().clone(), &avatars.rig)));
    entry.player.seek(0.0);
    entry
        .player
        .tick(entry.id, &avatars.rig, &mut AnimStreamer::new(PathBuf::new()), Some(&model.base));
    let pose = Mat4::from_cols_array_2d(&entry.player.pose.as_ref().expect("paused pose")[joint]);
    let skin = model.geometry.parts[0].skin.as_ref().expect("skin");
    assert!(skin.rest[0].abs_diff_eq(pose * skin.inverse_bind[0], 0.0001));
}

#[test]
fn custom_animation_model_cancel_errors_and_late_results_keep_the_current_selection() {
    let avatars = AvatarLibrary::load();
    let current = model(&avatars);
    let mut entry = entry();
    entry.animation_model.generation = 1;
    complete(&mut entry, 1, Some(Ok(current.clone())));
    let generation = entry.preview_generation;
    for result in [None, Some(Err("invalid model".into()))] {
        entry.animation_model.pending = true;
        complete(&mut entry, 1, result);
        assert!(!entry.animation_model.pending);
        assert!(Arc::ptr_eq(
            entry.animation_model.current.as_ref().expect("current model"),
            &current
        ));
        assert_eq!(entry.preview_generation, generation);
    }
    reset(&mut entry);
    complete(&mut entry, 1, Some(Ok(current)));
    assert!(entry.animation_model.current.is_none(), "a late file load must not undo Standard");
    assert!(prepare("static.dae".into(), super::super::tests::mesh_fixture(), &avatars).is_err());
}

#[test]
fn custom_animation_model_loads_a_local_collada_file_without_adding_it_to_the_import_queue() {
    let root = std::env::temp_dir().join(format!("aurora-animation-model-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("fixture directory");
    let path = root.join("tail.dae");
    let bytes = demo::tail_dae();
    std::fs::write(&path, &bytes).expect("local file");
    let avatars = Arc::new(AvatarLibrary::load());
    let model = load(path.clone(), 768, &avatars).expect("load local model");
    assert_eq!(std::fs::read_to_string(&path).expect("unchanged file"), bytes);
    let mut entry = entry();
    entry.animation_model.generation = 1;
    let id = entry.id;
    let mut imports = Imports {
        entries: vec![entry],
        ..Default::default()
    };
    let world = crate::world::World::new(avatars.clone());
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    let ctx = egui::Context::default();
    imports
        .tx
        .send(Worker::AnimationModel(id, 1, Some(Ok(model.clone()))))
        .expect("worker result");
    imports.poll(&ctx, &jobs, &world, &Default::default());
    assert_eq!(imports.entries.len(), 1);
    assert!(matches!(imports.entries[0].source, Some(Source::Animation { .. })));
    imports.entries[0].preview_not_before = None;
    imports.request_model(id, &jobs, &ctx, avatars);
    assert!(Arc::ptr_eq(imports.entries[0].model.as_ref().expect("preview"), &model.geometry));
    assert!(!imports.entries[0].preview_pending);
    std::fs::remove_file(path).expect("remove fixture");
    std::fs::remove_dir(root).expect("remove fixture directory");
}

#[test]
fn custom_animation_model_loads_gltf_and_glb_with_bento_weights() {
    let root = std::env::temp_dir().join(format!("aurora-animation-gltf-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("fixture directory");
    let mut binary = Vec::new();
    for value in [0.0f32, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.2, 0.0] {
        binary.extend(value.to_le_bytes());
    }
    for _ in 0..12 {
        binary.extend(0u16.to_le_bytes());
    }
    for _ in 0..3 {
        for value in [1.0f32, 0.0, 0.0, 0.0] {
            binary.extend(value.to_le_bytes());
        }
    }
    for index in [0u16, 1, 2, 0] {
        binary.extend(index.to_le_bytes());
    }
    let mut document = serde_json::json!({
        "asset": {"version": "2.0"},
        "buffers": [{"uri": "tail.bin", "byteLength": binary.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 24},
            {"buffer": 0, "byteOffset": 60, "byteLength": 48},
            {"buffer": 0, "byteOffset": 108, "byteLength": 6}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "VEC4"},
            {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"},
            {"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}, "indices": 3}]}],
        "nodes": [{"mesh": 0, "skin": 0}, {"name": "mTail1"}],
        "skins": [{"joints": [1]}],
        "scenes": [{"nodes": [0, 1]}], "scene": 0
    });
    std::fs::write(root.join("tail.bin"), &binary).expect("external buffer");
    std::fs::write(root.join("tail.gltf"), document.to_string()).expect("glTF");
    document["buffers"][0].as_object_mut().expect("buffer").remove("uri");
    let mut json = document.to_string().into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend(((12 + 8 + json.len() + 8 + binary.len()) as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(json);
    glb.extend((binary.len() as u32).to_le_bytes());
    glb.extend(b"BIN\0");
    glb.extend(binary);
    std::fs::write(root.join("tail.glb"), glb).expect("GLB");
    let avatars = AvatarLibrary::load();
    for name in ["tail.gltf", "tail.glb"] {
        let model = load(root.join(name), 768, &avatars).expect("Bento preview model");
        let part = model.geometry.parts.iter().find(|part| part.skin.is_some()).expect("skin");
        assert_eq!(part.skin.as_ref().expect("skin").joints, vec![avatars.rig.names["mTail1"]]);
        assert!(part.vertices.iter().any(|vertex| vertex.weights[0] > 0.0));
    }
    for name in ["tail.gltf", "tail.glb", "tail.bin"] {
        std::fs::remove_file(root.join(name)).expect("remove fixture");
    }
    std::fs::remove_dir(root).expect("remove fixture directory");
}

#[test]
fn saved_animation_models_round_trip_and_reuse_the_loaded_model_across_animations() {
    let avatars = Arc::new(AvatarLibrary::load());
    let first = entry();
    let second = entry();
    let (first_id, second_id) = (first.id, second.id);
    let mut imports = Imports {
        entries: vec![first, second],
        ..Default::default()
    };
    let model = model(&avatars);
    imports.complete_animation_model(first_id, 0, Some(Ok(model.clone())));
    let saved = imports.entries[0].animation_model.saved.expect("saved model");
    assert!(imports.animation_models_dirty);
    assert_eq!(imports.load_preferences.animation_models.len(), 1);
    let json = serde_json::to_string(&imports.load_preferences).expect("save preferences");
    let restored: Preferences = serde_json::from_str(&json).expect("reload preferences");
    assert_eq!(restored.animation_models, imports.load_preferences.animation_models);
    assert_eq!(
        serde_json::from_str::<Preferences>("{}")
            .expect("old preferences")
            .animation_models
            .len(),
        0
    );
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    imports.select_animation_model(second_id, saved, &jobs, &egui::Context::default(), avatars);
    assert_eq!(imports.entries[1].animation_model.saved, Some(saved));
    assert!(!imports.entries[1].animation_model.pending);
    assert!(Arc::ptr_eq(
        imports.entries[1].animation_model.current.as_ref().expect("shared model"),
        &model
    ));
    imports.complete_animation_model(first_id, 0, Some(Ok(model)));
    assert_eq!(
        imports.load_preferences.animation_models.len(),
        1,
        "adding the same file never duplicates it"
    );
    reset(&mut imports.entries[0]);
    assert_eq!(imports.entries[1].animation_model.saved, Some(saved), "Standard is per animation");
}

#[test]
fn removing_a_saved_animation_model_keeps_its_file_and_rejects_pending_results() {
    let root = std::env::temp_dir().join(format!("aurora-model-list-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("fixture directory");
    let path = root.join("tail.dae");
    std::fs::write(&path, demo::tail_dae()).expect("model file");
    let avatars = Arc::new(AvatarLibrary::load());
    let current = load(path.clone(), 768, &avatars).expect("model");
    let entry = entry();
    let id = entry.id;
    let mut imports = Imports {
        entries: vec![entry],
        ..Default::default()
    };
    imports.complete_animation_model(id, 0, Some(Ok(current.clone())));
    let saved = imports.entries[0].animation_model.saved.expect("saved model");
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    imports.entries[0].animation_model.requested = Some(saved);
    imports.remove_animation_model(saved, &jobs);
    assert!(path.is_file(), "removing a reference must never delete the user's file");
    assert!(imports.load_preferences.animation_models.is_empty());
    assert!(imports.animation_model_cache.is_empty());
    assert!(imports.entries[0].animation_model.current.is_none());
    imports.complete_animation_model(id, 0, Some(Ok(current)));
    assert!(
        imports.load_preferences.animation_models.is_empty(),
        "late loading cannot re-add a removed model"
    );
    std::fs::remove_file(path).expect("remove fixture");
    std::fs::remove_dir(root).expect("remove fixture directory");
}

#[test]
fn an_unavailable_saved_animation_model_keeps_the_list_and_the_previous_preview() {
    let avatars = Arc::new(AvatarLibrary::load());
    let current = model(&avatars);
    let entry = entry();
    let id = entry.id;
    let mut imports = Imports {
        entries: vec![entry],
        ..Default::default()
    };
    imports.complete_animation_model(id, 0, Some(Ok(current.clone())));
    let missing = Saved::remember(&mut imports.load_preferences.animation_models, "missing.dae".into());
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    imports.select_animation_model(id, missing, &jobs, &egui::Context::default(), avatars);
    let generation = imports.entries[0].animation_model.generation;
    imports.complete_animation_model(id, generation, Some(Err("missing file".into())));
    assert_eq!(
        imports.entries[0].animation_model.saved,
        Some(missing),
        "an unavailable entry stays selected so it can be removed"
    );
    assert!(Arc::ptr_eq(
        imports.entries[0].animation_model.current.as_ref().expect("retained preview"),
        &current
    ));
    assert_eq!(imports.load_preferences.animation_models.len(), 2);
    assert!(imports.entries[0].animation_model.error.is_some());
    imports.remove_animation_model(missing, &jobs);
    assert_eq!(imports.load_preferences.animation_models.len(), 1);
    assert!(imports.entries[0].animation_model.current.is_none());
}
