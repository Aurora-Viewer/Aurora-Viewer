use super::*;
use crate::import_gltf::Document;
use serde_json::json;

#[test]
fn gltf_interleaved_vertices_instance_transforms_and_sparse_positions() {
    let mut bytes = Vec::new();
    for position in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for value in position {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend([0; 4]);
    }
    let doc = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":bytes.len(),"byteStride":16}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0,"translation":[2.0,3.0,4.0]}],"scenes":[{"nodes":[0]}]});
    let document = Document::parse(&serde_json::to_vec(&doc).expect("JSON"), |_| Ok(bytes.clone())).expect("document");
    let scene = gltf::parse(&document, &crate::Skeleton::default()).expect("mesh scene");
    assert_eq!(scene.instances.len(), 1);
    let model = &scene.models[0];
    let point = Vec3::from_array(model.faces[0].vertices[0].position);
    let world = scene.instances[0].transform.transform_point3(point);
    assert!((world - Vec3::new(2.0, -4.0, 3.0)).length() < 0.0001);
    assert!(model.faces[0].vertices.iter().all(|v| Vec3::from_array(v.normal).length() > 0.99));
    let mut sparse_bytes = vec![0, 1, 2, 0];
    for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        sparse_bytes.extend(value.to_le_bytes());
    }
    let doc = json!({"asset":{"version":"2.0"},"buffers":[{"uri":"mesh.bin","byteLength":sparse_bytes.len()}],
        "bufferViews":[{"buffer":0,"byteLength":3},{"buffer":0,"byteOffset":4,"byteLength":36}],
        "accessors":[{"componentType":5126,"count":3,"type":"VEC3","sparse":{"count":3,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":1}}}]});
    let doc = Document::parse(&serde_json::to_vec(&doc).expect("JSON"), |_| Ok(sparse_bytes.clone())).expect("sparse document");
    assert_eq!(gltf::attribute::<3>(&doc, 0).expect("sparse accessor")[2], [0.0, 1.0, 0.0]);
}

pub(super) const DAE: &[u8] = br##"<COLLADA><asset><unit meter="0.01"/><up_axis>Z_UP</up_axis></asset><library_geometries><geometry id="geo"><mesh>
<source id="positions"><float_array count="9">0 0 0 100 0 0 0 100 0</float_array><technique_common><accessor count="3" stride="3"/></technique_common></source>
<vertices id="vertices"><input semantic="POSITION" source="#positions"/></vertices><triangles count="1"><input semantic="VERTEX" source="#vertices" offset="0"/><p>0 1 2</p></triangles>
</mesh></geometry></library_geometries><library_visual_scenes><visual_scene id="scene"><node id="node"><matrix>1 0 0 2 0 1 0 3 0 0 1 4 0 0 0 1</matrix><instance_geometry url="#geo"/></node></visual_scene></library_visual_scenes><scene><instance_visual_scene url="#scene"/></scene></COLLADA>"##;

#[test]
fn collada_units_row_major_transform_and_normalized_geometry_preserve_positions() {
    let scene = collada::parse(DAE, &crate::Skeleton::default()).expect("Collada");
    let model = &scene.models[0];
    let world = scene.instances[0]
        .transform
        .transform_point3(Vec3::from_array(model.faces[0].vertices[1].position));
    assert!((world - Vec3::new(1.02, 0.03, 0.04)).length() < 0.00001);
    assert_eq!(model.faces[0].indices, [0, 1, 2]);
    let meters = collada::parse_with_meters(DAE, &crate::Skeleton::default(), true).expect("forced meters");
    let model = &meters.models[0];
    let point = meters.instances[0]
        .transform
        .transform_point3(Vec3::from_array(model.faces[0].vertices[1].position));
    assert!((point - Vec3::new(102.0, 3.0, 4.0)).length() < 0.0001);
    let malformed = String::from_utf8_lossy(DAE).replace("0 1 2</p>", "0 1 9</p>");
    assert!(collada::parse(malformed.as_bytes(), &crate::Skeleton::default()).is_err());
}

#[test]
fn gltf_cycles_missing_accessor_data_and_singular_transform_are_rejected() {
    let document = Document::parse(
        br#"{"asset":{"version":"2.0"},"nodes":[{"children":[0]}],"scenes":[{"nodes":[0]}]}"#,
        |_| Ok(vec![]),
    )
    .expect("document");
    assert!(gltf::parse(&document, &crate::Skeleton::default()).is_err());
    let document = Document::parse(
        br#"{"asset":{"version":"2.0"},"accessors":[{"componentType":5126,"count":3,"type":"VEC3"}]}"#,
        |_| Ok(vec![]),
    )
    .expect("document");
    assert!(gltf::attribute::<3>(&document, 0).is_err());
}

pub(super) fn collada_objects(visual: &str, rigged: bool) -> String {
    let mut geometries = String::new();
    let mut controllers = String::new();
    for id in ["a", "b"] {
        geometries.push_str(&format!(r##"<geometry id="{id}" name="{id}"><mesh>
<source id="{id}-pos"><float_array count="9">0 0 0 1 0 0 0 1 0</float_array><technique_common><accessor count="3" stride="3"/></technique_common></source>
<vertices id="{id}-verts"><input semantic="POSITION" source="#{id}-pos"/></vertices>
<triangles count="1"><input semantic="VERTEX" source="#{id}-verts"/><p>0 1 2</p></triangles></mesh></geometry>"##));
        if rigged {
            controllers.push_str(&format!(r##"<controller id="{id}-skin"><skin source="#{id}">
<source id="{id}-joints"><Name_array count="1">mPelvis</Name_array></source>
<source id="{id}-bind"><float_array count="16">1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</float_array><technique_common><accessor count="1" stride="16"/></technique_common></source>
<source id="{id}-weights"><float_array count="1">1</float_array><technique_common><accessor count="1" stride="1"/></technique_common></source>
<joints><input semantic="JOINT" source="#{id}-joints"/><input semantic="INV_BIND_MATRIX" source="#{id}-bind"/></joints>
<vertex_weights count="3"><input semantic="JOINT" source="#{id}-joints" offset="0"/><input semantic="WEIGHT" source="#{id}-weights" offset="1"/><vcount>1 1 1</vcount><v>0 0 0 0 0 0</v></vertex_weights>
</skin></controller>"##));
        }
    }
    format!(
        r##"<COLLADA><asset><unit meter="1"/><up_axis>Z_UP</up_axis></asset>
<library_geometries>{geometries}</library_geometries><library_controllers>{controllers}</library_controllers>
<library_visual_scenes><visual_scene id="scene">{visual}</visual_scene></library_visual_scenes><scene><instance_visual_scene url="#scene"/></scene></COLLADA>"##
    )
}

#[test]
fn collada_multiple_objects_keep_geometry_instances_and_nested_transforms() {
    let document = collada_objects(
        r##"<node name="parent"><translate>2 3 4</translate>
<node name="first"><instance_geometry url="#a"/></node>
<node name="second"><translate>5 0 0</translate><instance_geometry url="#b"/></node>
<node name="repeated"><translate>0 6 0</translate><instance_geometry url="#a"/></node></node>"##,
        false,
    );
    let scene = collada::parse(document.as_bytes(), &crate::Skeleton::default()).expect("multi-object scene");
    assert_eq!(scene.models.len(), 2);
    assert_eq!(scene.instances.len(), 3);
    assert_eq!(scene.instances.iter().map(|i| i.model).collect::<Vec<_>>(), [0, 1, 0]);
    for (instance, expected) in scene.instances.iter().zip([[2.0, 3.0, 4.0], [7.0, 3.0, 4.0], [2.0, 9.0, 4.0]]) {
        let model = &scene.models[instance.model];
        let position = instance
            .transform
            .transform_point3(Vec3::from_array(model.faces[0].vertices[0].position));
        assert!((position - Vec3::from_array(expected)).length() < 0.00001);
    }
}

#[test]
fn collada_skin_libraries_load_without_scene_instances_and_do_not_duplicate_explicit_controllers() {
    let skeleton = crate::Skeleton::parse(include_bytes!("../../../aurora-viewer/assets/character/avatar_skeleton.xml")).expect("skeleton");
    for visual in [
        "<node id=\"mPelvis\" type=\"JOINT\"/>",
        r##"<node id="mPelvis" type="JOINT"/><node><translate>2 3 4</translate><instance_controller url="#a-skin"/></node>"##,
    ] {
        let document = collada_objects(visual, true);
        let scene = collada::parse(document.as_bytes(), &skeleton).expect("library-only skin controllers");
        assert_eq!(scene.models.len(), 2);
        assert_eq!(scene.instances.len(), 2);
        assert!(scene.models.iter().all(|m| m.skin.as_ref().is_some_and(|s| s.names == ["mPelvis"])));
        assert!(
            scene
                .models
                .iter()
                .flat_map(|m| &m.faces)
                .flat_map(|f| &f.vertices)
                .all(|v| v.weights == [1.0, 0.0, 0.0, 0.0])
        );
        assert!(scene.warnings.is_empty());
        let first = &scene.models[scene.instances[0].model];
        let point = scene.instances[0]
            .transform
            .transform_point3(Vec3::from_array(first.faces[0].vertices[0].position));
        assert!(
            (point
                - if visual.contains("instance_controller") {
                    Vec3::new(2.0, 3.0, 4.0)
                } else {
                    Vec3::ZERO
                })
            .length()
                < 0.00001
        );
    }
}

fn cube() -> Model {
    let vertices = [-0.5f32, 0.5]
        .into_iter()
        .flat_map(|x| {
            [-0.5f32, 0.5].into_iter().flat_map(move |y| {
                [-0.5f32, 0.5].into_iter().map(move |z| Vertex {
                    position: [x, y, z],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0; 2],
                    joints: [0; 4],
                    weights: [0.0; 4],
                })
            })
        })
        .collect();
    Model {
        name: "Cube".into(),
        submodel: 0,
        center: Vec3::ZERO,
        size: Vec3::ONE,
        skin: None,
        faces: vec![Face {
            material: 0,
            vertices,
            indices: vec![
                0, 2, 3, 0, 3, 1, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 4, 6, 0, 6, 2, 1, 3, 7, 1, 7, 5,
            ],
        }],
    }
}

#[test]
fn uploads_carry_the_normalized_scale_their_tangents_are_found_in() {
    // LLModel::writeModel writes it; LLVolumeFace::cacheOptimize reads it
    let mut model = cube();
    for v in &mut model.faces[0].vertices {
        v.position[0] *= 4.0;
    }
    model.normalize().expect("normalized");
    let data = encode::asset(
        &[None, None, None, Some(model)],
        None,
        &process::Physics::default(),
        encode::Options::default(),
    )
    .expect("encoded");
    let header = crate::parse_mesh_header(&data).expect("header");
    let section = crate::mesh::section_bytes(&data, header.lods[3].expect("high lod")).expect("section");
    let faces = crate::decode_mesh_lod(section).expect("faces");
    assert_eq!(faces[0].normalized_scale, Some([4.0, 1.0, 1.0]));
}

#[test]
fn automatic_lods_reduce_disconnected_seams_without_mixing_materials_or_mutating_the_source() {
    let mut source = cube();
    let face = &mut source.faces[0];
    let prototype = face.vertices[0].clone();
    face.vertices = (0..=10)
        .flat_map(|y| {
            let prototype = prototype.clone();
            (0..=10).map(move |x| Vertex {
                position: [x as f32 / 10.0, y as f32 / 10.0, 0.0],
                ..prototype.clone()
            })
        })
        .collect();
    face.indices = (0..10)
        .flat_map(|y| {
            (0..10).flat_map(move |x| {
                let i = y * 11 + x;
                [i, i + 1, i + 11, i + 1, i + 12, i + 11]
            })
        })
        .collect();
    let mut second = source.faces[0].clone();
    second.material = 1;
    for vertex in &mut second.vertices {
        vertex.position[0] += 2.0;
    }
    source.faces.push(second);
    for face in &mut source.faces {
        face.vertices = face
            .indices
            .iter()
            .enumerate()
            .map(|(corner, index)| {
                let mut vertex = face.vertices[*index as usize].clone();
                vertex.uv = [corner as f32 / 600.0, face.material as f32];
                vertex
            })
            .collect();
        face.indices = (0..face.vertices.len() as u32).collect();
    }
    let options = process::LodOptions {
        triangles: 100,
        error: 1.0,
        method: process::Method::Auto,
        crease_angle: None,
        error_only: false,
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let precise = process::lod(
        &source,
        process::LodOptions {
            method: process::Method::Precise,
            ..options
        },
        &cancel,
    )
    .expect("precise LOD");
    assert!(
        precise.triangles() > 180,
        "fragmented seams prevent precise reduction from reaching the budget"
    );
    let lod = process::lod(&source, options, &cancel).expect("automatic LOD");
    assert!(
        lod.triangles() > 0 && lod.triangles() <= 180,
        "AUTO should reach the reference's 1.8x budget tolerance: {} triangles",
        lod.triangles()
    );
    for (face, original) in lod.faces.iter().zip(&source.faces) {
        face.validate().expect("valid reduced face");
        assert_eq!(face.material, original.material);
        for vertex in &face.vertices {
            assert!(original.vertices.iter().any(|v| v.position == vertex.position && v.uv == vertex.uv));
        }
    }
    assert_eq!(source.triangles(), 400);
    assert!(source.faces.iter().all(|face| face.vertices.len() == 600));
    let error_lod = process::lod(
        &source,
        process::LodOptions {
            error_only: true,
            error: 0.0,
            ..options
        },
        &cancel,
    )
    .expect("zero error LOD");
    assert!(
        error_lod.triangles() > 180,
        "error mode must retain the explicit threshold without welding away seams"
    );
}

#[test]
fn convex_physics_is_bounded_and_cancelled_processing_preserves_the_source() {
    let source = cube();
    let hull = process::base_hull(&source).expect("hull");
    assert_eq!(hull.vertices.len(), 8);
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    assert!(
        process::lod(
            &source,
            process::LodOptions {
                triangles: 4,
                error: 0.1,
                method: process::Method::Reliable,
                crease_angle: None,
                error_only: false,
            },
            &cancelled
        )
        .is_err()
    );
    let lod = process::lod(
        &source,
        process::LodOptions {
            triangles: 4,
            error: 1.0,
            method: process::Method::Precise,
            crease_angle: None,
            error_only: false,
        },
        &std::sync::atomic::AtomicBool::new(false),
    )
    .expect("LOD");
    assert!(lod.triangles() <= source.triangles());
    assert_eq!(source.triangles(), 12);
    for fill_mode in 0..3 {
        let physics = process::analyze(
            &source,
            process::PhysicsOptions {
                resolution: 1000,
                max_hulls: 8,
                max_vertices: 64,
                fill_mode,
                ..Default::default()
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("analyze cube");
        assert!(!physics.hulls.is_empty());
        assert!(physics.hulls.iter().all(|h| h.vertices.len() <= 64));
    }
}

#[test]
fn error_mode_welds_identical_exported_corners_and_respects_the_geometric_threshold() {
    let mut source = cube();
    let face = &mut source.faces[0];
    face.vertices = face.indices.iter().map(|index| face.vertices[*index as usize].clone()).collect();
    face.indices = (0..face.vertices.len() as u32).collect();
    let options = process::LodOptions {
        triangles: 0,
        error: 0.0,
        method: process::Method::Auto,
        crease_angle: None,
        error_only: true,
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let faithful = process::lod(&source, options, &cancel).expect("zero error");
    let loose = process::lod(&source, process::LodOptions { error: 1.0, ..options }, &cancel).expect("large error");
    assert_eq!(faithful.triangles(), 12);
    assert!(
        loose.triangles() < faithful.triangles(),
        "the error threshold changes the actual geometry"
    );
    assert_eq!(source.faces[0].vertices.len(), 36);
    assert_eq!(source.triangles(), 12);
}

#[test]
fn lod_skin_names_are_remapped_and_unweighted_vertices_use_nearest_source() {
    let mut high = cube();
    high.skin = Some(Skin {
        names: vec!["mPelvis".into(), "mTorso".into()],
        inverse_bind: vec![Mat4::IDENTITY; 2],
        alternate_bind: vec![Mat4::IDENTITY; 2],
        bind_shape: Mat4::IDENTITY,
        pelvis_offset: 0.0,
    });
    for vertex in &mut high.faces[0].vertices {
        vertex.joints = [u16::from(vertex.position[0] > 0.0), 0, 0, 0];
        vertex.weights = [1.0, 0.0, 0.0, 0.0];
    }
    let mut low = high.clone();
    low.skin.as_mut().expect("skin").names.reverse();
    low.faces[0].vertices[0].joints = [1, 0, 0, 0];
    low.faces[0].vertices[1].position = [0.48, 0.49, 0.47];
    low.faces[0].vertices[1].weights = [0.0; 4];
    skin::transfer_weights(&mut low, &high).expect("weights");
    assert_eq!(low.faces[0].vertices[0].joints[0], 0);
    assert_eq!(low.faces[0].vertices[1].joints[0], 1);
    assert_eq!(low.faces[0].vertices[1].weights, [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(low.skin.as_ref().expect("skin").names, high.skin.as_ref().expect("skin").names);
}

#[test]
fn gltf_palette_pruning_preserves_weighted_joints_and_active_groups() {
    let skeleton = crate::Skeleton::parse(include_bytes!("../../../aurora-viewer/assets/character/avatar_skeleton.xml")).expect("skeleton");
    let names: Vec<_> = skeleton
        .joints
        .iter()
        .filter(|j| !j.is_collision_volume)
        .map(|j| j.name.clone())
        .collect();
    assert!(names.len() > 110);
    let source = names.iter().position(|name| name == "mHandRing3Left").expect("weighted joint");
    let mut skin = Skin {
        inverse_bind: vec![Mat4::IDENTITY; names.len()],
        alternate_bind: vec![Mat4::IDENTITY; names.len()],
        names,
        bind_shape: Mat4::IDENTITY,
        pelvis_offset: 0.0,
    };
    let mut model = cube();
    for vertex in &mut model.faces[0].vertices {
        vertex.joints = [source as u16, 0, 0, 0];
        vertex.weights = [0.7, 0.0, 0.0, 0.0];
    }
    skin::prune_gltf(&mut model.faces, &mut skin, &skeleton).expect("prune");
    assert!(skin.names.len() <= 110);
    assert_eq!(skin.names[0], "mHandRing3Left");
    assert!(skin.names.iter().any(|name| name == "mPelvis"));
    assert_eq!(model.faces[0].vertices[0].joints[0], 0);
    assert_eq!(model.faces[0].vertices[0].weights, [1.0, 0.0, 0.0, 0.0]);
    skin.names = vec!["Unsupported".into(), "mPelvis".into()];
    skin.inverse_bind = vec![Mat4::IDENTITY; 2];
    skin.alternate_bind = skin.inverse_bind.clone();
    for vertex in &mut model.faces[0].vertices {
        vertex.joints = [0, 1, 0, 0];
        vertex.weights = [0.8, 0.2, 0.0, 0.0];
    }
    skin::prune_gltf(&mut model.faces, &mut skin, &skeleton).expect("unsupported pruning");
    assert_eq!(skin.names, ["mPelvis"]);
    assert_eq!(model.faces[0].vertices[0].weights, [0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn physics_degeneracy_distinguishes_small_balanced_and_thin_triangles() {
    assert!(!process::degenerate([Vec3::ZERO, Vec3::X * 0.005, Vec3::Y * 0.005]));
    assert!(process::degenerate([Vec3::ZERO, Vec3::X, Vec3::Y * 0.00001]));
    assert!(process::degenerate([Vec3::ZERO, Vec3::X, Vec3::X * 2.0]));
    assert!(!process::degenerate([Vec3::ZERO, Vec3::X, Vec3::Y]));
}
