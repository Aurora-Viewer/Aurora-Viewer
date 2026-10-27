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
