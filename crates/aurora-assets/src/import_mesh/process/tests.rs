use super::*;

const METHODS: [Method; 3] = [Method::Legacy, Method::Reliable, Method::Fast];

fn vertex(position: [f32; 3], material: usize) -> Vertex {
    Vertex {
        position,
        normal: [0.0, 0.0, 1.0],
        uv: [position[0] + material as f32, position[1]],
        joints: [material as u16 + 1, material as u16 + 2, 0, 0],
        weights: [0.75, 0.25, 0.0, 0.0],
    }
}

fn model(faces: Vec<Face>) -> Model {
    Model {
        name: "Material floor".into(),
        submodel: 0,
        faces,
        skin: None,
        center: Vec3::ZERO,
        size: Vec3::ONE,
    }
}

fn grid(material: usize, cells: u32, offset: f32, seams: bool) -> Face {
    let mut face = Face {
        material,
        vertices: (0..=cells)
            .flat_map(|y| (0..=cells).map(move |x| vertex([offset + x as f32 / cells as f32, y as f32 / cells as f32, 0.0], material)))
            .collect(),
        indices: (0..cells)
            .flat_map(|y| {
                (0..cells).flat_map(move |x| {
                    let i = y * (cells + 1) + x;
                    [i, i + 1, i + cells + 1, i + 1, i + cells + 2, i + cells + 1]
                })
            })
            .collect(),
    };
    if seams {
        face.vertices = face
            .indices
            .iter()
            .enumerate()
            .map(|(corner, index)| {
                let mut vertex = face.vertices[*index as usize].clone();
                vertex.uv[0] += corner as f32;
                vertex
            })
            .collect();
        face.indices = (0..face.vertices.len() as u32).collect();
    }
    face
}

fn triangle(material: usize, offset: f32) -> Face {
    Face {
        material,
        vertices: [[offset, 0.0, 0.0], [offset + 0.5, 0.0, 0.0], [offset, 0.5, 0.0]]
            .map(|position| vertex(position, material))
            .to_vec(),
        indices: vec![0, 1, 2],
    }
}

fn options(method: Method, triangles: usize) -> LodOptions {
    LodOptions {
        triangles,
        error: 1.0,
        method,
        crease_angle: None,
        error_only: false,
    }
}

fn assert_materials_survive(source: &Model, reduced: &Model) {
    assert_eq!(reduced.faces.len(), source.faces.len(), "material slots retain their order");
    for (original, face) in source.faces.iter().zip(&reduced.faces) {
        assert_eq!(face.material, original.material);
        face.validate().expect("valid reduced material face");
        if original.indices.is_empty() {
            assert!(face.indices.is_empty(), "an unused material slot gains no geometry");
            continue;
        }
        assert!(face.indices.len() >= 3, "material {} disappeared", original.material);
        assert!(
            face.indices.as_chunks::<3>().0.iter().any(|triangle| {
                let points = triangle.map(|index| Vec3::from_array(face.vertices[index as usize].position));
                !degenerate(points)
            }),
            "material {} needs a real triangle, not the Legacy Viewer's collapsed placeholder",
            original.material
        );
    }
}

fn assert_original_attributes(source: &Model, reduced: &Model) {
    for (original, face) in source.faces.iter().zip(&reduced.faces) {
        for vertex in &face.vertices {
            assert!(
                original.vertices.iter().any(|candidate| {
                    candidate.position == vertex.position
                        && candidate.normal == vertex.normal
                        && candidate.uv == vertex.uv
                        && candidate.joints == vertex.joints
                        && candidate.weights == vertex.weights
                }),
                "material {} borrowed or lost a source vertex attribute: {vertex:?}",
                original.material
            );
        }
    }
}

#[test]
fn every_lod_method_preserves_small_materials_even_when_their_budget_rounds_to_zero() {
    let source = model(vec![
        grid(3, 8, 0.0, true),
        triangle(7, 2.0),
        Face {
            material: 11,
            vertices: Vec::new(),
            indices: Vec::new(),
        },
    ]);
    let original = format!("{source:?}");
    for method in METHODS {
        for target in [0, 1, 2, 3, 17] {
            let reduced = lod(&source, options(method, target), &AtomicBool::new(false)).expect("LOD with a tiny material budget");
            assert_materials_survive(&source, &reduced);
            assert_original_attributes(&source, &reduced);
            if method == Method::Legacy && target <= 2 {
                assert_eq!(reduced.triangles(), 2, "empty material slots do not raise the minimum");
            }
        }
    }
    assert_eq!(
        format!("{source:?}"),
        original,
        "LOD generation leaves its high-detail source intact"
    );
}

#[test]
fn material_floor_also_applies_to_error_threshold_generation() {
    let source = model(vec![grid(3, 5, 0.0, true), triangle(7, 2.0)]);
    for method in METHODS {
        for error in [0.0, 1.0] {
            let reduced = lod(
                &source,
                LodOptions {
                    error,
                    error_only: true,
                    ..options(method, 0)
                },
                &AtomicBool::new(false),
            )
            .expect("error-controlled LOD");
            assert_materials_survive(&source, &reduced);
        }
    }
}

#[test]
fn legacy_automatic_reaches_one_real_triangle_per_material_even_with_coincident_seams() {
    // Welded positions must not let the first material consume the second
    // one's slot, normals, texture coordinates or joint weights.
    let mut second = grid(7, 6, 0.0, true);
    for vertex in &mut second.vertices {
        vertex.normal = [0.0, 0.0, -1.0];
    }
    let source = model(vec![grid(3, 6, 0.0, true), second, triangle(11, 2.0)]);
    for target in [0, 1, source.faces.len()] {
        let reduced = lod(&source, options(Method::Legacy, target), &AtomicBool::new(false)).expect("minimum automatic LOD");
        assert_materials_survive(&source, &reduced);
        assert_original_attributes(&source, &reduced);
        assert!(reduced.faces.iter().all(|face| face.indices.len() == 3));
        assert_eq!(reduced.triangles(), 3, "the minimum is one triangle per used material");
    }
}

#[test]
fn legacy_automatic_can_reduce_a_closed_mesh_to_one_triangle() {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let source = model(vec![Face {
        material: 3,
        vertices: positions.map(|position| vertex(position, 3)).to_vec(),
        indices: vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5,
        ],
    }]);
    let reduced = lod(&source, options(Method::Legacy, 1), &AtomicBool::new(false)).expect("one-triangle closed mesh LOD");
    assert_materials_survive(&source, &reduced);
    assert_original_attributes(&source, &reduced);
    assert_eq!(
        reduced.triangles(),
        1,
        "a closed surface must not force restoration of the full source"
    );
}

#[test]
fn preserving_a_material_skips_a_collapsed_source_triangle_when_a_real_one_exists() {
    let mut face = triangle(3, 0.0);
    face.indices = vec![0, 0, 0, 0, 1, 2];
    let source = model(vec![face]);
    for method in METHODS {
        let reduced = lod(&source, options(method, 0), &AtomicBool::new(false)).expect("material floor with a collapsed first triangle");
        assert_materials_survive(&source, &reduced);
        assert_original_attributes(&source, &reduced);
    }
}

#[test]
fn minimum_material_generation_still_honors_cancellation() {
    let source = model(vec![grid(3, 4, 0.0, true), triangle(7, 2.0)]);
    for method in METHODS {
        assert!(lod(&source, options(method, 0), &AtomicBool::new(true)).is_err());
    }
}
