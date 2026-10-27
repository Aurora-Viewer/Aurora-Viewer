use super::*;
use crate::import_gltf::Document;
use serde_json::json;

fn skeleton() -> crate::Skeleton {
    crate::Skeleton::parse(include_bytes!("../../../aurora-viewer/assets/character/avatar_skeleton.xml")).expect("SL skeleton")
}

fn triangle() -> Face {
    Face {
        vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0.0, 0.0, 1.0],
                uv: [0.2, 0.7],
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            })
            .collect(),
        indices: vec![0, 1, 2],
        material: 0,
    }
}

fn simple_gltf(nodes: serde_json::Value, roots: serde_json::Value) -> Document {
    let bytes: Vec<_> = triangle()
        .vertices
        .iter()
        .flat_map(|v| v.position.into_iter().flat_map(f32::to_le_bytes))
        .collect();
    let json = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bytes.len()}], "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
        "meshes":[{"name":"Part","primitives":[{"attributes":{"POSITION":0}}]}],"nodes":nodes,"scenes":[{"nodes":roots}]});
    Document::parse_mesh(&serde_json::to_vec(&json).expect("JSON"), |_| Ok(bytes.clone())).expect("document")
}

#[test]
fn mesh_import_parity_large_dae_and_embedded_glb_are_accepted() {
    {
        let mut bytes = super::tests::DAE.to_vec();
        bytes.extend_from_slice(b"<!--");
        bytes.resize(64 * 1024 * 1024 + 1, b' ');
        bytes.extend_from_slice(b"-->");
        assert_eq!(
            collada::parse(&bytes, &crate::Skeleton::default())
                .expect("DAE above 64 MiB")
                .models
                .len(),
            1
        );
    }
    // Before the fix the BIN chunk was counted twice, rejecting this valid
    // GLB although its whole file fits inside the material resource budget.
    let binary_length = 129 * 1024 * 1024;
    let mut json = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":binary_length}]})
        .to_string()
        .into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let length = 12 + 8 + json.len() + 8 + binary_length;
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend((length as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(0x4e4f534au32.to_le_bytes());
    glb.extend(json);
    glb.extend((binary_length as u32).to_le_bytes());
    glb.extend(0x004e4942u32.to_le_bytes());
    glb.resize(length, 0);
    let parsed = Document::parse(&glb, |_| panic!("embedded buffer")).expect("GLB buffer counted once");
    assert_eq!(parsed.buffers[0].len(), binary_length);
}

#[test]
fn mesh_import_parity_dense_faces_and_duplicate_attributes_do_not_fail() {
    let face = triangle();
    let indices = [0, 1, 2].repeat(1024 * 1024 + 1);
    let parts = split_face(face.vertices.clone(), indices, 0).expect("dense indexed face");
    assert_eq!(parts.iter().map(|face| face.indices.len() / 3).sum::<usize>(), 1024 * 1024 + 1);
    let vertices = (0..10).flat_map(|_| face.vertices.iter().cloned()).collect();
    let parts = split_face(vertices, (0..30).collect(), 0).expect("redundant source vertices");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].vertices.len(), 3);
    assert_eq!(parts[0].indices.len(), 30);
}

#[test]
fn mesh_import_parity_32_bit_indices_remain_exact_above_float_integer_precision() {
    let expected = [16_777_217u32, 16_777_219, 16_777_223];
    let bytes: Vec<_> = expected.into_iter().flat_map(u32::to_le_bytes).collect();
    let json = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"indices.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bytes.len()}],"accessors":[{"bufferView":0,"componentType":5125,"count":3,"type":"SCALAR"}]});
    let doc = Document::parse_mesh(json.to_string().as_bytes(), |_| Ok(bytes.clone())).expect("document");
    assert_eq!(gltf::indices(&doc, 0).expect("exact indices"), expected);
}

#[test]
fn mesh_import_parity_gltf_mirrored_and_ordinary_instances_keep_their_geometry() {
    let doc = simple_gltf(
        json!([{"mesh":0,"scale":[-2.0,1.0,1.0]},{"mesh":0,"translation":[3.0,0.0,0.0]}]),
        json!([0, 1]),
    );
    let scene = gltf::parse(&doc, &crate::Skeleton::default()).expect("mirrored mesh");
    for (index, instance) in scene.instances.iter().enumerate() {
        let model = &scene.models[instance.model];
        let points: Vec<_> = model.faces[0]
            .vertices
            .iter()
            .map(|v| instance.transform.transform_point3(Vec3::from_array(v.position)))
            .collect();
        assert!(instance.transform.determinant() > 0.0);
        let expected = if index == 0 {
            Vec3::new(-2.0, 0.0, 0.0)
        } else {
            Vec3::new(4.0, 0.0, 0.0)
        };
        assert!(points.iter().any(|p| p.distance(expected) < 0.00001));
    }
}

#[test]
fn mesh_import_parity_unused_gltf_node_transforms_do_not_reject_the_selected_scene() {
    let doc = simple_gltf(json!([{"mesh":0},{"scale":[0.0,0.0,0.0]}]), json!([0]));
    assert!(gltf::parse(&doc, &crate::Skeleton::default()).is_ok());
}

#[test]
fn mesh_import_parity_gltf_node_labels_keep_shared_mesh_lods_independent() {
    let doc = simple_gltf(json!([{"mesh":0,"name":"PartA"},{"mesh":0,"name":"PartB"}]), json!([0, 1]));
    let scene = gltf::parse(&doc, &crate::Skeleton::default()).expect("node identities");
    assert_eq!(scene.models.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["PartA", "PartB"]);
    assert_ne!(scene.instances[0].model, scene.instances[1].model);
}

#[test]
fn mesh_import_parity_more_than_256_instances_and_deep_gltf_hierarchy() {
    let mut nodes: Vec<_> = (0..140).map(|i| json!({"children":[i+1]})).collect();
    nodes.extend((0..300).map(|_| json!({"mesh":0})));
    let roots: Vec<_> = std::iter::once(0).chain(141..440).collect();
    let document = simple_gltf(json!(nodes), json!(roots));
    let scene = gltf::parse(&document, &crate::Skeleton::default()).expect("deep hierarchy and many instances");
    assert_eq!(scene.instances.len(), 300);
    let visual = (0..300).map(|_| "<node><instance_geometry url=\"#a\"/></node>").collect::<String>();
    let dae = super::tests::collada_objects(&visual, false);
    let scene = collada::parse(dae.as_bytes(), &crate::Skeleton::default()).expect("many DAE instances");
    assert_eq!(scene.instances.len(), 300);
}

#[test]
fn mesh_import_parity_model_limit_is_generated_per_source_not_a_scene_cap() {
    let primitives = ["<triangles><input semantic=\"VERTEX\" source=\"#vertices\"/><p>0 1 2</p></triangles>"; 24].join("");
    let dae = String::from_utf8_lossy(super::tests::DAE).replace(
        "<triangles count=\"1\"><input semantic=\"VERTEX\" source=\"#vertices\" offset=\"0\"/><p>0 1 2</p></triangles>",
        &primitives,
    );
    let scene = collada::parse_with_options(dae.as_bytes(), &crate::Skeleton::default(), false, 0).expect("limited import");
    assert_eq!(scene.models.len(), 1);
    assert!(!scene.warnings.is_empty());
    let scene = collada::parse_with_options(dae.as_bytes(), &crate::Skeleton::default(), false, 16).expect("expanded limit");
    assert_eq!(scene.models.len(), 3);
}

#[test]
fn mesh_import_parity_creases_split_vertices_and_smoothing_is_area_weighted() {
    let mut face = triangle();
    let mut extra = face.vertices[0].clone();
    extra.position = [0.0, 0.0, 4.0];
    face.vertices.push(extra);
    face.indices.extend([0, 3, 1]);
    let mut smooth = face.clone();
    process::normals(&mut face, 45.0).expect("crease normals");
    assert_eq!(face.vertices.len(), 6);
    let a = Vec3::from_array(face.vertices[face.indices[0] as usize].normal);
    let b = Vec3::from_array(face.vertices[face.indices[3] as usize].normal);
    assert!(a.dot(b).abs() < 0.0001);
    assert!(face.vertices.iter().all(|v| v.uv == [0.2, 0.7] && v.weights[0] == 1.0));
    process::normals(&mut smooth, 180.0).expect("smooth normals");
    assert_eq!(smooth.vertices.len(), 4);
    let expected = Vec3::new(0.0, 4.0, 1.0).normalize();
    assert!(Vec3::from_array(smooth.vertices[0].normal).distance(expected) < 0.0001);
    let mut dense = triangle();
    dense.indices = [0, 1, 2].repeat(21846);
    let original: Vec<_> = dense.vertices.iter().map(|v| v.normal).collect();
    for angle in [45.0, 180.0] {
        process::normals(&mut dense, angle).expect("dense face is skipped");
        assert_eq!(dense.vertices.iter().map(|v| v.normal).collect::<Vec<_>>(), original);
    }
}

#[test]
fn mesh_import_parity_missing_normals_follow_each_format_without_regeneration() {
    let mut face = triangle();
    let mut vertex = face.vertices[0].clone();
    vertex.position = [0.0, 0.0, 4.0];
    face.vertices.push(vertex);
    face.indices.extend([0, 3, 1]);
    process::missing_normals(&mut face).expect("DAE fallback");
    assert!(Vec3::from_array(face.vertices[0].normal).distance(Vec3::new(0.0, 1.0, 1.0).normalize()) < 0.0001);
    let mut doc = simple_gltf(json!([{"mesh":0}]), json!([0]));
    let bytes: Vec<_> = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    doc.buffers[0] = bytes.into();
    let scene = gltf::parse(&doc, &crate::Skeleton::default()).expect("glTF fallback");
    assert!(
        scene.models[0].faces[0]
            .vertices
            .iter()
            .all(|v| Vec3::from_array(v.normal).distance(-Vec3::Y) < 0.0001)
    );
}

#[test]
fn mesh_import_parity_flat_axes_keep_a_unit_normalization_scale_and_real_bounds() {
    let mut model = Model {
        name: "Plane".into(),
        submodel: 0,
        faces: vec![triangle()],
        skin: None,
        center: Vec3::ZERO,
        size: Vec3::ONE,
    };
    model.normalize().expect("flat geometry");
    assert_eq!(model.size, Vec3::ONE);
    let bounds = model.normalized_bounds();
    assert_eq!(bounds[0].z, 0.0);
    assert_eq!(bounds[1].z, 0.0);
    let scene = collada::parse(super::tests::DAE, &crate::Skeleton::default()).expect("flat DAE");
    assert!(
        (scene.models[0].size.z - 0.01).abs() < 0.000001,
        "the unit axis follows the DAE's centimetre units"
    );
    assert_eq!(scene.normalized_bounds[&0][0].z, 0.0);
}

#[test]
fn mesh_import_parity_rigged_dae_keeps_authored_binds_and_bone_offsets_for_each_up_axis() {
    let rig = skeleton();
    for axis in ["Z_UP", "Y_UP", "X_UP"] {
        let dae = super::tests::collada_objects(
            "<node sid=\"mPelvis\" type=\"JOINT\"><translate sid=\"location\">1 2 3</translate></node>",
            true,
        )
        .replace("Z_UP", axis);
        let scene = collada::parse(dae.as_bytes(), &rig).expect("rigged DAE");
        let model = &scene.models[0];
        let skin = model.skin.as_ref().expect("skin");
        assert!(skin.inverse_bind[0].abs_diff_eq(Mat4::IDENTITY, 0.00001));
        assert_eq!(skin.alternate_bind[0].w_axis.truncate(), Vec3::new(1.0, 2.0, 3.0));
        let vertex = model.faces[0].vertices[1].position;
        let raw = skin.bind_shape.transform_point3(Vec3::from_array(vertex));
        assert!(raw.distance(Vec3::X) < 0.00001, "authored rig frame for {axis}: {raw:?}");
    }
}

#[test]
fn mesh_import_parity_gltf_detects_only_the_two_shoulder_xy_case() {
    let rig = skeleton();
    let permutation = Mat4::from_cols(Vec3::Y.extend(0.0), Vec3::Z.extend(0.0), Vec3::X.extend(0.0), glam::Vec4::W);
    let bytes: Vec<_> = [permutation, permutation]
        .into_iter()
        .flat_map(|m| m.to_cols_array().into_iter().flat_map(f32::to_le_bytes))
        .collect();
    let json = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"bind.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bytes.len()}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"MAT4"}],
        "nodes":[{"name":"mShoulderLeft"},{"name":"mShoulderRight"}],"skins":[{"joints":[0,1],"inverseBindMatrices":0}]});
    let mut doc = Document::parse_mesh(json.to_string().as_bytes(), |_| Ok(bytes.clone())).expect("document");
    let nodes = doc.json["nodes"].as_array().expect("nodes");
    let globals = gltf_skin::globals(nodes).expect("globals");
    let expected = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2) * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    assert!(
        gltf_skin::basis(&doc, nodes, &globals, &rig)
            .expect("basis")
            .abs_diff_eq(expected, 0.00001)
    );
    doc.json["nodes"][1]["name"] = json!("mElbowRight");
    let nodes = doc.json["nodes"].as_array().expect("nodes");
    assert!(
        gltf_skin::basis(&doc, nodes, &globals, &rig)
            .expect("ordinary basis")
            .abs_diff_eq(Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2), 0.00001)
    );
}

#[test]
fn mesh_import_parity_gltf_binds_use_parent_rest_and_missing_sl_bones() {
    let rig = skeleton();
    let basis = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let pelvis = rig.find("mPelvis").expect("pelvis");
    let torso = rig.find("mTorso").expect("torso");
    let pelvis_global = basis.inverse() * rig.joints[pelvis].world;
    let torso_global = basis.inverse() * rig.joints[torso].world;
    let armature = Mat4::from_rotation_y(0.6) * Mat4::from_translation(Vec3::new(3.0, 4.0, 5.0));
    let bytes: Vec<_> = [pelvis_global.inverse(), torso_global.inverse()]
        .into_iter()
        .flat_map(|m| m.to_cols_array().into_iter().flat_map(f32::to_le_bytes))
        .collect();
    let json = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"bind.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bytes.len()}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"MAT4"}],
        "nodes":[{"name":"Armature","matrix":armature.to_cols_array(),"children":[1]},
            {"name":"mPelvis","matrix":(armature.inverse()*pelvis_global).to_cols_array(),"children":[2]},
            {"name":"mTorso","matrix":(pelvis_global.inverse()*torso_global).to_cols_array()}],
        "skins":[{"joints":[1,2],"inverseBindMatrices":0}]});
    let doc = Document::parse_mesh(json.to_string().as_bytes(), |_| Ok(bytes.clone())).expect("document");
    let nodes = doc.json["nodes"].as_array().expect("nodes");
    let globals = gltf_skin::globals(nodes).expect("global rests");
    let skin = gltf_skin::read(&doc, 0, nodes, &globals, basis, &rig, &mut Vec::new()).expect("converted skin");
    assert!(
        skin.alternate_bind[1].w_axis.truncate().distance(rig.joints[torso].pos) < 0.0001,
        "omitted spine bones must not lengthen torso"
    );
    assert!(skin.inverse_bind[0].abs_diff_eq(rig.joints[pelvis].world.inverse(), 0.0001));
    assert!(skin.inverse_bind[1].abs_diff_eq(rig.joints[torso].world.inverse(), 0.0001));
}

#[test]
fn mesh_import_parity_gltf_large_source_palette_prunes_after_usage() {
    let rig = skeleton();
    let mut doc = simple_gltf(json!([{"mesh":0,"skin":0}]), json!([0]));
    let nodes = doc.json["nodes"].as_array_mut().expect("nodes");
    nodes.extend((0..299).map(|i| json!({"name":format!("Unused{i}")})));
    nodes.push(json!({"name":"mPelvis"}));
    doc.json["skins"] = json!([{"joints":(1..=300).collect::<Vec<_>>()}]);
    // Add exact u16 joint indices and normalized float weights.
    let mut bytes = doc.buffers[0].to_vec();
    let joint_start = bytes.len();
    for _ in 0..3 {
        for id in [299u16, 0, 0, 0] {
            bytes.extend(id.to_le_bytes());
        }
    }
    let weight_start = bytes.len();
    for _ in 0..3 {
        for weight in [1.0f32, 0.0, 0.0, 0.0] {
            bytes.extend(weight.to_le_bytes());
        }
    }
    doc.buffers[0] = bytes.into();
    doc.json["bufferViews"].as_array_mut().expect("views").extend([
        json!({"buffer":0,"byteOffset":joint_start,"byteLength":24}),
        json!({"buffer":0,"byteOffset":weight_start,"byteLength":48}),
    ]);
    doc.json["accessors"].as_array_mut().expect("accessors").extend([
        json!({"bufferView":1,"componentType":5123,"count":3,"type":"VEC4"}),
        json!({"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"}),
    ]);
    doc.json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(1);
    doc.json["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(2);
    let scene = gltf::parse(&doc, &rig).expect("large source palette");
    let model = &scene.models[0];
    assert_eq!(model.skin.as_ref().expect("pruned skin").names, ["mPelvis"]);
    assert!(model.faces[0].vertices.iter().all(|v| v.joints[0] == 0 && v.weights[0] == 1.0));
}

#[test]
fn mesh_import_parity_missing_weights_search_other_faces_in_normalized_space() {
    let mut source = triangle();
    source.vertices.truncate(2);
    source.indices.clear();
    source.vertices[0].position = [10.0, 0.0, 2.0];
    source.vertices[0].joints = [1, 0, 0, 0];
    source.vertices[1].position = [0.0, 2.0, 2.0];
    let mut missing = triangle();
    for vertex in &mut missing.vertices {
        vertex.position = [0.0, 0.0, 2.0];
        vertex.weights = [0.0; 4];
    }
    let mut faces = vec![source, missing];
    let geometry: Vec<_> = faces
        .iter()
        .flat_map(|face| face.vertices.iter().map(|v| (v.position, v.normal, v.uv)))
        .collect();
    skin::complete_weights(&mut faces, Vec3::new(100.0, 1.0, 1.0));
    assert!(
        faces[1]
            .vertices
            .iter()
            .all(|v| v.joints[0] == 1 && v.weights == [1.0, 0.0, 0.0, 0.0])
    );
    assert_eq!(faces[0].vertices[1].joints[0], 0, "existing influences remain authored");
    assert_eq!(
        geometry,
        faces
            .iter()
            .flat_map(|face| face.vertices.iter().map(|v| (v.position, v.normal, v.uv)))
            .collect::<Vec<_>>()
    );
}

#[test]
fn mesh_import_parity_dae_coincident_layers_share_the_last_source_weights() {
    // Synthetic onion layer: the same triangle twice in one geometry, each
    // copy weighted to its own joint. LLDAELoader keys the weights by
    // position, so both copies take the later source vertices' list.
    let document = r##"<COLLADA><asset><unit meter="1"/><up_axis>Z_UP</up_axis></asset>
<library_geometries><geometry id="g" name="layers"><mesh>
<source id="g-pos"><float_array count="18">0 0 0 1 0 0 0 1 0 0 0 0 1 0 0 0 1 0</float_array><technique_common><accessor count="6" stride="3"/></technique_common></source>
<vertices id="g-verts"><input semantic="POSITION" source="#g-pos"/></vertices>
<triangles count="2"><input semantic="VERTEX" source="#g-verts"/><p>0 1 2 3 4 5</p></triangles></mesh></geometry></library_geometries>
<library_controllers><controller id="g-skin"><skin source="#g">
<source id="g-joints"><Name_array count="2">mPelvis mHead</Name_array></source>
<source id="g-bind"><float_array count="32">1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</float_array><technique_common><accessor count="2" stride="16"/></technique_common></source>
<source id="g-weights"><float_array count="1">1</float_array><technique_common><accessor count="1" stride="1"/></technique_common></source>
<joints><input semantic="JOINT" source="#g-joints"/><input semantic="INV_BIND_MATRIX" source="#g-bind"/></joints>
<vertex_weights count="6"><input semantic="JOINT" source="#g-joints" offset="0"/><input semantic="WEIGHT" source="#g-weights" offset="1"/><vcount>1 1 1 1 1 1</vcount><v>0 0 0 0 0 0 1 0 1 0 1 0</v></vertex_weights>
</skin></controller></library_controllers>
<library_visual_scenes><visual_scene id="scene"><node name="body"><instance_controller url="#g-skin"/></node></visual_scene></library_visual_scenes><scene><instance_visual_scene url="#scene"/></scene></COLLADA>"##;
    let scene = collada::parse(document.as_bytes(), &skeleton()).expect("layered DAE");
    let model = &scene.models[0];
    assert_eq!(model.skin.as_ref().expect("skin").names, ["mPelvis", "mHead"]);
    let vertices: Vec<_> = model.faces.iter().flat_map(|face| &face.vertices).collect();
    assert!(!vertices.is_empty());
    assert!(vertices.iter().all(|v| v.joints[0] == 1 && v.weights == [1.0, 0.0, 0.0, 0.0]));
}

#[test]
fn mesh_import_parity_near_positions_take_the_first_weights_within_epsilon() {
    let vertex = |position: [f32; 3], joint: u16, weight: f32| Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv: [0.0; 2],
        joints: [joint, 0, 0, 0],
        weights: [weight, 0.0, 0.0, 0.0],
    };
    let mut faces = vec![Face {
        vertices: vec![
            vertex([0.1 + 4e-6, 0.0, 0.0], 3, 1.0),
            vertex([0.1, 0.0, 0.0], 2, 1.0),
            vertex([0.1, 0.0, 2e-5], 4, 1.0),
            vertex([0.1, 5e-6, 0.0], 0, 0.0),
            vertex([0.4, 0.0, 0.0], 0, 0.0),
        ],
        indices: vec![0, 1, 2, 2, 3, 4],
        material: 0,
    }];
    skin::share_weights(&mut faces, Vec3::ZERO, Vec3::ONE);
    let joints: Vec<_> = faces[0].vertices.iter().map(|v| (v.joints[0], v.weights[0])).collect();
    // Lowest LLVector3 within 1e-5 wins; beyond it each keeps its own,
    // and an isolated unweighted vertex is left to complete_weights.
    assert_eq!(joints, [(2, 1.0), (2, 1.0), (4, 1.0), (2, 1.0), (0, 0.0)]);
}

#[test]
fn mesh_import_parity_gltf_high_repairs_vertices_whose_only_joint_is_unsupported() {
    // Entirely synthetic: a triangle, one standard joint and one helper.
    let rig = skeleton();
    let mut doc = simple_gltf(
        json!([{"mesh":0,"skin":0},{"name":"mHead","translation":[0.0,2.0,0.0]},{"name":"ExporterHelper"}]),
        json!([0, 1, 2]),
    );
    doc.json["skins"] = json!([{"joints":[1,2]}]);
    let mut bytes = doc.buffers[0].to_vec();
    let joint_start = bytes.len();
    for id in [0u16, 0, 1] {
        for joint in [id, 0, 0, 0] {
            bytes.extend(joint.to_le_bytes());
        }
    }
    let weight_start = bytes.len();
    for _ in 0..3 {
        for weight in [1.0f32, 0.0, 0.0, 0.0] {
            bytes.extend(weight.to_le_bytes());
        }
    }
    doc.buffers[0] = bytes.into();
    doc.json["bufferViews"].as_array_mut().expect("views").extend([
        json!({"buffer":0,"byteOffset":joint_start,"byteLength":24}),
        json!({"buffer":0,"byteOffset":weight_start,"byteLength":48}),
    ]);
    doc.json["accessors"].as_array_mut().expect("accessors").extend([
        json!({"bufferView":1,"componentType":5123,"count":3,"type":"VEC4"}),
        json!({"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"}),
    ]);
    doc.json["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(1);
    doc.json["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(2);
    let scene = gltf::parse(&doc, &rig).expect("synthetic rig");
    let high = &scene.models[0];
    assert_eq!(high.skin.as_ref().expect("skin").names, ["mHead"]);
    assert!(high.faces[0].vertices.iter().all(|v| v.joints[0] == 0 && v.weights[0] == 1.0));
    let encoded = encode::lod(high, true).expect("High weights");
    let weights = encoded[0]["Weights"].as_binary();
    assert_eq!(weights.len(), high.faces[0].vertices.len() * 4);
    assert!(weights.as_chunks::<4>().0.iter().all(|weight| *weight == [0, 255, 255, 255]));
}
