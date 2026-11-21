use super::*;

fn draft() -> Draft {
    let mut draft = crate::imports::tests::mesh_fixture();
    for lod in &mut draft.lods[..3] {
        lod.source = LodSource::Above;
    }
    draft
}

fn request(draft: &Draft) -> aurora_net::inventory::mesh_upload::Request {
    super::request(
        draft,
        &prepare(draft).expect("prepare"),
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        "Parity",
        "",
        Permissions::default(),
        Default::default(),
    )
    .expect("encode request")
}

#[test]
fn mesh_physics_convex_conversion_repairs_collision_only_and_keeps_the_upload_guard() {
    let mut draft = crate::imports::tests::invalid_physics_fixture();
    let original = prepare(&draft).expect("visual and physical geometry");
    assert_eq!(original.degenerate_triangles, 1);
    assert!(original.physics_error().is_some());
    assert!(
        super::request(
            &draft,
            &original,
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "Physics",
            "",
            Permissions::default(),
            Default::default()
        )
        .is_err()
    );
    let visual = original.lods[0]
        .each_ref()
        .map(|lod| encode::lod(lod.as_ref().expect("visual LOD"), false).expect("visual geometry"));
    draft.options.decompose = true;
    let repaired = prepare(&draft).expect("convex collision geometry");
    assert!(repaired.physics_error().is_none());
    assert!(!repaired.hulls[0].hulls.is_empty());
    assert_eq!(
        original.physics[0].as_ref().unwrap().triangles(),
        repaired.physics[0].as_ref().unwrap().triangles()
    );
    for (lod, original) in repaired.lods[0].iter().zip(&visual) {
        assert_eq!(
            encode::lod(lod.as_ref().expect("visual LOD"), false).expect("visual geometry"),
            *original
        );
    }
    assert!(request(&draft).validate().is_ok());
    draft.options.decompose = false;
    assert_eq!(prepare(&draft).expect("original physics").degenerate_triangles, 1);
}

#[test]
fn mesh_import_parity_authored_lod_subsets_reorder_pad_and_match_suffixes() {
    let mut draft = draft();
    let scene = Arc::make_mut(&mut draft.scene);
    scene.models[0].name = "Part".into();
    let mut second = scene.models[0].faces[0].clone();
    for vertex in &mut second.vertices {
        vertex.position[2] += 0.1;
    }
    scene.models[0].faces.push(second);
    scene.material_bindings.insert(0, vec!["Red".into(), "Blue".into()]);
    let mut low = scene.clone();
    low.models[0].name = "Part_LOD1".into();
    low.models[0].faces.remove(0);
    low.material_bindings.insert(0, vec!["Blue".into()]);
    draft.lods[2].source = LodSource::File;
    draft.lods[2].file = Some(Arc::new(low));
    let prepared = prepare(&draft).expect("authored subset");
    let lower = prepared.lods[0][2].as_ref().expect("medium");
    assert_eq!(lower.faces.len(), 2);
    assert!(lower.faces[0].indices.is_empty());
    assert!(!lower.faces[1].indices.is_empty());
    let payload = encode::lod(lower, false).expect("LOD LLSD");
    assert!(payload[0]["NoGeometry"].as_bool());
    assert!(!payload[1]["TriangleList"].as_binary().is_empty());
    let low = Arc::make_mut(draft.lods[2].file.as_mut().expect("file"));
    low.material_bindings.insert(0, vec!["Unexpected".into()]);
    assert!(prepare(&draft).is_err(), "a lower LOD still cannot introduce materials");
}

#[test]
fn mesh_import_parity_unused_missing_textures_are_neither_required_nor_uploaded() {
    let mut draft = draft();
    Arc::make_mut(&mut draft.scene).materials.push(codec::Material {
        name: "Unused".into(),
        value: Default::default(),
        images: [Some(99), None, None, None],
    });
    draft.options.textures = true;
    let resources = aurora_llsd::from_xml(&request(&draft).resources).expect("resources");
    assert!(resources["texture_list"].as_array().is_empty());
    Arc::make_mut(&mut draft.scene).instances[0].materials[0] = 1;
    assert!(
        super::request(
            &draft,
            &prepare(&draft).expect("geometry"),
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            "Parity",
            "",
            Permissions::default(),
            Default::default()
        )
        .is_err(),
        "a used missing texture must still fail"
    );
}

#[test]
fn mesh_import_parity_many_instances_and_transfer_settings_survive_fee_encoding() {
    let mut draft = draft();
    let scene = Arc::make_mut(&mut draft.scene);
    scene.instances = vec![scene.instances[0].clone(); 300];
    assert_eq!(draft.upload_timeout, 600);
    draft.upload_timeout = 900;
    let upload = request(&draft);
    assert_eq!(upload.timeout_secs, 900);
    let query = aurora_llsd::from_xml(&upload.query).expect("query");
    assert_eq!(query["asset_resources"]["instance_list"].as_array().len(), 300);
    assert_eq!(query["asset_resources"]["mesh_list"].as_array().len(), 1);
    let resources = aurora_llsd::from_xml(&upload.resources).expect("upload resources");
    assert_eq!(
        query["asset_resources"]["mesh_list"], resources["mesh_list"],
        "the fee and upload use the same compressed bytes"
    );
    assert!(upload.validate().is_ok());
    let prefs = Preferences::default();
    assert_eq!(prefs.mesh_model_limit, 768);
    assert_eq!(prefs.mesh_upload_timeout, 600);
    let saved = serde_json::to_vec(&prefs).expect("preferences");
    assert_eq!(serde_json::from_slice::<Preferences>(&saved).expect("restored"), prefs);
}

#[test]
fn mesh_import_parity_oversized_scene_scale_is_clamped_on_load() {
    let original = draft();
    let mut scene = original.scene.as_ref().clone();
    scene.instances[0].transform *= glam::Mat4::from_scale(glam::Vec3::splat(100.0));
    let mut loaded = Draft::new(scene, HashMap::new());
    for lod in &mut loaded.lods[..3] {
        lod.source = LodSource::Above;
    }
    assert!((loaded.options.scale - 0.64).abs() < 0.00001);
    assert!(prepare(&loaded).is_ok());
}

#[test]
fn mesh_import_parity_flat_source_uploads_a_unit_axis_without_inflating_preview_dimensions() {
    let mut draft = draft();
    let scene = Arc::make_mut(&mut draft.scene);
    scene.models[0].size.z = 1.0;
    for vertex in scene.models[0].faces.iter_mut().flat_map(|face| &mut face.vertices) {
        vertex.position[2] = 0.0;
    }
    scene.models[0].faces[0].indices = vec![0, 2, 1, 0, 3, 2];
    scene.models[0].faces[0].compact();
    scene.instances[0].transform = glam::Mat4::IDENTITY;
    scene.normalized_bounds.insert(0, scene.models[0].normalized_bounds());
    assert_eq!(draft.dimensions_and_limit().0.z, 0.0);
    let upload = request(&draft);
    let resources = aurora_llsd::from_xml(&upload.resources).expect("resources");
    assert_eq!(resources["instance_list"][0]["scale"][2].as_f64(), 1.0);
}

#[test]
fn mesh_import_parity_dae_palettes_over_110_declared_joints_remain_uploadable() {
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")).expect("rig");
    let names: Vec<_> = skeleton.joints.iter().take(120).map(|j| j.name.clone()).collect();
    let mut draft = draft();
    let scene = Arc::make_mut(&mut draft.scene);
    let model = &mut scene.models[0];
    model.skin = Some(codec::Skin {
        inverse_bind: vec![glam::Mat4::IDENTITY; names.len()],
        alternate_bind: vec![glam::Mat4::IDENTITY; names.len()],
        names,
        bind_shape: glam::Mat4::IDENTITY,
        pelvis_offset: 0.0,
    });
    for vertex in model.faces.iter_mut().flat_map(|f| &mut f.vertices) {
        vertex.weights = [1.0, 0.0, 0.0, 0.0];
    }
    assert!(inspector::Avatar::new(scene).valid());
    draft.options.skin = true;
    assert!(request(&draft).validate().is_ok());
}

#[test]
fn mesh_base_hull_keeps_custom_collision_first_then_uses_the_smallest_nonempty_lod() {
    let draft = draft();
    let high = &draft.scene.models[0];
    let named = |name: &str| Model {
        name: name.into(),
        ..high.clone()
    };
    let physics = named("physics");
    let mut lods = [
        Some(named("lowest")),
        Some(named("low")),
        Some(named("medium")),
        Some(named("high")),
    ];
    assert_eq!(hull_source(Some(&physics), &lods, high).name, "physics");
    assert_eq!(hull_source(None, &lods, high).name, "lowest");
    lods[0] = None;
    assert_eq!(hull_source(None, &lods, high).name, "low");
    lods[1] = None;
    assert_eq!(hull_source(None, &lods, high).name, "medium");
    let mut empty = named("empty collision");
    for face in &mut empty.faces {
        face.indices.clear();
    }
    assert_eq!(
        hull_source(Some(&empty), &lods, high).name,
        "medium",
        "a model with no triangle is passed over"
    );
    lods[2] = None;
    assert_eq!(hull_source(None, &lods, high).name, high.name);
}

#[test]
fn mesh_default_lods_use_legacy_generation_and_one_third_targets() {
    // genMeshOptimizerLODs truncates base / 3^(3 - level).
    assert_eq!(default_lod_targets(3000, 1), [111, 333, 1000, 3000]);
    // #3433's 1,400 triangles: 466, 155 and 51 below the High LOD.
    assert_eq!(default_lod_targets(1400, 1), [51, 155, 466, 1400]);
    // 10 / 27 truncates to 0 there; the target stays at least 1.
    assert_eq!(default_lod_targets(10, 1), [1, 1, 3, 10]);
    assert_eq!(default_lod_targets(10, 6), [6, 6, 6, 10]);
    assert_eq!(default_lod_targets(0, 0), [0; 4]);
    let mut scene = Scene::default();
    scene.models.push(Model {
        name: "Counted".into(),
        submodel: 0,
        skin: None,
        center: glam::Vec3::ZERO,
        size: glam::Vec3::ONE,
        faces: vec![codec::Face {
            vertices: Vec::new(),
            indices: vec![0; 3000 * 3],
            material: 0,
        }],
    });
    let draft = Draft::from_shared(Arc::new(scene), HashMap::new());
    let sources = |draft: &Draft| draft.lods.each_ref().map(|lod| lod.source);
    let targets = |draft: &Draft| draft.lods.each_ref().map(|lod| lod.triangles);
    assert_eq!(targets(&draft), [111, 333, 1000, 3000]);
    assert_eq!(
        sources(&draft),
        [LodSource::Legacy, LodSource::Legacy, LodSource::Legacy, LodSource::File]
    );
}

#[test]
fn mesh_saved_generation_modes_migrate_to_the_automatic_chain() {
    for name in ["Legacy", "Auto", "Combine", "Fast"] {
        let saved = format!("\"{name}\"");
        assert_eq!(serde_json::from_str::<LodSource>(&saved).expect("saved mode"), LodSource::Legacy);
    }
    assert_eq!(serde_json::to_string(&LodSource::Legacy).expect("automatic mode"), "\"Legacy\"");
    assert_eq!(
        serde_json::from_str::<LodSource>("\"Reliable\"").expect("reliable mode"),
        LodSource::Reliable
    );
}

#[test]
fn mesh_lod_model_budgets_preserve_each_nonempty_material_face() {
    let mut scene = crate::imports::tests::mesh_fixture().scene.as_ref().clone();
    let first = &mut scene.models[0];
    let face = first.faces[0].clone();
    first.faces = vec![face.clone(); 3];
    let mut second = first.clone();
    second.faces = vec![face.clone(), face];
    for face in &mut second.faces {
        face.indices.truncate(3);
    }
    let mut empty = second.faces[0].clone();
    empty.indices.clear();
    second.faces.push(empty);
    scene.models.push(second);
    assert_eq!(minimum_lod_triangles(&scene), 5);
    let loaded = Draft::new(scene.clone(), HashMap::new());
    assert_eq!(loaded.lods[0].triangles, 5, "the default cannot hide a material face");
    for target in [0, 4, 5, 6, 17, 38, 100] {
        let targets = model_lod_targets(&scene.models, target);
        assert!(targets[0] >= 3);
        assert_eq!(targets[1], 2, "single-triangle faces retain their only triangle");
        assert_eq!(targets.iter().sum::<usize>(), target.clamp(5, 38));
    }
}

#[test]
fn mesh_adaptive_default_uses_lowest_but_preserves_explicit_collision() {
    let mut draft = crate::imports::tests::mesh_fixture();
    let mut lowest = draft.scene.as_ref().clone();
    lowest.models[0].faces[0].vertices[0].position = [4.0, 4.0, 4.0];
    draft.lods[0].source = LodSource::File;
    draft.lods[0].file = Some(Arc::new(lowest));
    let prepared = prepare(&draft).expect("prepare");
    let hull = prepared.hulls[0].base.as_ref().expect("base hull");
    assert!(hull.vertices.contains(&[4.0; 3]), "the Lowest file defines the default collision");
    assert!(prepared.physics[0].is_none(), "no collision model was chosen");
    draft.options.physics_source = 4;
    let prepared = prepare(&draft).expect("explicit bounding box");
    assert!(prepared.physics[0].is_some());
    assert!(
        prepared.hulls[0]
            .base
            .as_ref()
            .expect("custom hull")
            .vertices
            .iter()
            .all(|p| p.iter().all(|v| v.abs() <= 0.5))
    );
}
