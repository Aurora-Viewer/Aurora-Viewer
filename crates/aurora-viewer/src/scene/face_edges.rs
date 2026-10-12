//! Material-face borders, using the position welding of Firestorm's
//! LLVolume::generateSilhouetteVertices (LGPL 2.1). Prepared on the geometry
//! worker, cached as line indices beside triangles, posed by the same GPU
//! vertex shader. Texture seams and triangle diagonals are not borders.
use super::jobs::FaceGeom;
use glam::Vec3;
use std::collections::HashMap;

pub fn boundary(face: &FaceGeom) -> Vec<u16> {
    let mut ids = HashMap::new();
    let weld: Vec<_> = face
        .vertices
        .iter()
        .enumerate()
        .map(|(i, vertex)| {
            let position = vertex.pos.map(|v| (v * 1e4).round() as i32);
            let skin = face.skin.as_ref().and_then(|skin| skin.get(i)).map(|s| (s.joints, s.weights));
            let next = ids.len() as u32;
            *ids.entry((position, skin)).or_insert(next)
        })
        .collect();
    let mut edges: HashMap<(u32, u32), (u32, u16, u16)> = HashMap::new();
    for tri in face.indices.as_chunks::<3>().0 {
        if tri.iter().any(|&i| i as usize >= face.vertices.len()) {
            continue;
        }
        let p = tri.map(|i| Vec3::from_array(face.vertices[i as usize].pos));
        if (p[1] - p[0]).cross(p[2] - p[0]).length_squared() < 1e-14 {
            continue;
        }
        for side in 0..3 {
            let (a, b) = (tri[side], tri[(side + 1) % 3]);
            let (wa, wb) = (weld[a as usize], weld[b as usize]);
            if wa == wb {
                continue;
            }
            edges.entry((wa.min(wb), wa.max(wb))).or_insert((0, a, b)).0 += 1;
        }
    }
    let mut border: Vec<_> = edges.into_iter().filter(|(_, (count, _, _))| *count == 1).collect();
    border.sort_unstable_by_key(|(edge, _)| *edge);
    border.into_iter().flat_map(|(_, (_, a, b))| [a, b]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_render::Vertex;

    fn face(positions: &[[f32; 3]], indices: &[u16]) -> FaceGeom {
        FaceGeom {
            vertices: positions.iter().map(|&pos| Vertex { pos, ..Default::default() }).collect(),
            indices: indices.to_vec(),
            skin: None,
        }
    }

    #[test]
    fn quad_border_excludes_its_diagonal_and_welds_a_texture_seam() {
        let quad = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let edges = boundary(&face(&quad, &[0, 1, 2, 0, 2, 3]));
        assert_eq!(edges.len(), 8);
        assert!(!edges.as_chunks::<2>().0.iter().any(|e| *e == [0, 2] || *e == [2, 0]));
        let seam = [quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]];
        assert_eq!(boundary(&face(&seam, &[0, 1, 2, 3, 4, 5])).len(), 8);
    }

    #[test]
    fn closed_faces_have_no_border_and_invalid_triangles_are_ignored() {
        let tetra = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert!(boundary(&face(&tetra, &[0, 1, 2, 0, 3, 1, 0, 2, 3, 1, 3, 2])).is_empty());
        assert!(boundary(&face(&tetra, &[0, 0, 1, 0, 1, 99])).is_empty());
    }

    #[test]
    fn different_skin_bindings_are_not_welded_together() {
        let mut face = face(
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            &[0, 1, 2, 3, 4, 5],
        );
        face.skin = Some(
            (0..6)
                .map(|i| aurora_render::SkinVertex {
                    joints: [u8::from(i >= 3), 0, 0, 0],
                    weights: [255, 0, 0, 0],
                })
                .collect(),
        );
        assert_eq!(boundary(&face).len(), 12);
    }

    #[test]
    fn preparation_retains_only_triangles_for_picking_and_reuses_vertices_for_borders() {
        let quad = || {
            face(
                &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
                &[0, 1, 2, 0, 2, 3],
            )
        };
        let prepared = super::super::jobs::PreparedGeom::new(vec![Some(quad())], Vec3::ZERO, Vec3::ONE, true, None);
        assert_eq!(prepared.outlines.len(), 1);
        assert_eq!(prepared.outlines[0], 6..14);
        assert_eq!(prepared.pick_faces[0].as_ref().unwrap().indices.len(), 6);
        let Some(super::super::jobs::FaceUpload::Memory(face)) = &prepared.faces[0] else {
            panic!("memory geometry");
        };
        assert_eq!(face.vertices.len(), 4);
        assert_eq!(face.indices.len(), 14);
        let update = super::super::jobs::PreparedGeom::new_vertices(vec![Some(quad())], Vec3::ZERO, Vec3::ONE);
        assert_eq!(update.outlines.len(), 1);
        assert_eq!(update.outlines[0], 6..6);
        let Some(super::super::jobs::FaceUpload::Memory(face)) = &update.faces[0] else {
            panic!("vertex update");
        };
        assert_eq!(face.indices.len(), 6);
    }
}
