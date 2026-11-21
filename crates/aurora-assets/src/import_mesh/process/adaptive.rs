//! Adaptive default collision hulls. Unlike LLMeshUploadThread::generateHulls
//! (indra/newview/llmeshrepository.cpp), the caller chooses Lowest first and
//! bounds the additional simplification error instead of imposing a budget.

use super::{AssetError, Hull, Model, Vector, has_volume, try_convex_hull};
use parry3d::{query::PointQuery, shape::Triangle};
use std::sync::atomic::{AtomicBool, Ordering};

const RELATIVE_ERROR: f32 = 0.0025;

/// Simplify the existing bounded base hull, retaining its axis extents and
/// support-plane extremes. `scales` are the normalized-to-instance scales,
/// including the model dimensions; uniform upload scale cancels from the
/// relative error. Custom collision geometry continues to use `base_hull`.
pub fn base_hull(model: &Model, scales: &[glam::Vec3], cancel: &AtomicBool) -> Result<Hull, AssetError> {
    cancelled(cancel)?;
    let source = super::base_hull(model)?;
    cancelled(cancel)?;
    if source.vertices.len() <= 8 {
        return Ok(source);
    }
    let points: Vec<_> = source.vertices.iter().copied().map(Vector::from_array).collect();
    let (min, max) = bounds(&points);
    let extent = max - min;
    let mut tolerance = extent.length() * RELATIVE_ERROR;
    for scale in scales.iter().copied().chain(std::iter::once(model.size)) {
        let scale = Vector::from_array(scale.abs().to_array());
        if !scale.is_finite() || scale.min_element() <= 0.0 {
            return Err(AssetError::invalid("collision instance scale"));
        }
        // |S d| <= max(S) |d|: this conservative bound covers every
        // nonuniformly scaled instance without repeating distance queries.
        tolerance = tolerance.min((extent * scale).length() / scale.max_element() * RELATIVE_ERROR);
    }
    // BoundingVerts floors each coordinate to 16 bits. Reserve its largest
    // possible displacement so the decoded hull still meets the tolerance.
    let wire_error = extent.length() / 65535.0;
    let tolerance = tolerance - wire_error;
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Ok(source);
    }
    let mut chosen = seed(&points, min, max);
    loop {
        cancelled(cancel)?;
        if chosen.len() >= points.len() {
            return Ok(source);
        }
        let selected: Vec<_> = chosen.iter().map(|&index| points[index]).collect();
        let Ok((vertices, indices)) = try_convex_hull(&selected) else {
            return Ok(source);
        };
        if !has_volume(&vertices, &indices) {
            return Ok(source);
        }
        let Some(surface) = Surface::new(&vertices, &indices) else {
            return Ok(source);
        };
        let mut worst = (0, 0.0f32);
        for (index, point) in points.iter().enumerate() {
            let distance = surface.distance_squared(*point);
            if !distance.is_finite() {
                return Ok(source);
            }
            if distance > worst.1 {
                worst = (index, distance);
            }
        }
        if worst.1 <= tolerance * tolerance {
            let (candidate_min, candidate_max) = bounds(&vertices);
            let epsilon = extent.max_element() * 1e-6;
            if (candidate_min - min).abs().max_element() > epsilon || (candidate_max - max).abs().max_element() > epsilon {
                return Ok(source);
            }
            // The server rebuilds the convex hull after quantization; reject
            // a collapsed result rather than weakening the flat-part guard.
            let wire: Vec<_> = vertices
                .iter()
                .map(|p| min + ((*p - min) / extent * 65535.0).floor() / 65535.0 * extent)
                .collect();
            if !try_convex_hull(&wire).is_ok_and(|(p, t)| has_volume(&p, &t)) {
                return Ok(source);
            }
            return Ok(Hull {
                vertices: vertices.into_iter().map(|p| p.to_array()).collect(),
                indices,
            });
        }
        if chosen.contains(&worst.0) {
            return Ok(source);
        }
        chosen.push(worst.0);
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<(), AssetError> {
    if cancel.load(Ordering::Relaxed) {
        Err(AssetError::invalid("cancelled"))
    } else {
        Ok(())
    }
}

fn bounds(points: &[Vector]) -> (Vector, Vector) {
    points
        .iter()
        .fold((Vector::splat(f32::MAX), Vector::splat(f32::MIN)), |(min, max), p| {
            (min.min(*p), max.max(*p))
        })
}

fn seed(points: &[Vector], min: Vector, max: Vector) -> Vec<usize> {
    let mut chosen = Vec::new();
    for axis in 0..3 {
        for side in [min[axis], max[axis]] {
            // Retain the span of each support plane, not just one arbitrary
            // corner of a flat bottom. Curved support outlines may simplify
            // within the same distance bound as the rest of the envelope.
            let face: Vec<_> = points.iter().enumerate().filter(|(_, p)| p[axis] == side).map(|(i, _)| i).collect();
            for tangent in (0..3).filter(|&i| i != axis) {
                for index in [
                    face.iter().min_by(|&&a, &&b| points[a][tangent].total_cmp(&points[b][tangent])),
                    face.iter().max_by(|&&a, &&b| points[a][tangent].total_cmp(&points[b][tangent])),
                ]
                .into_iter()
                .flatten()
                {
                    if !chosen.contains(index) {
                        chosen.push(*index);
                    }
                }
            }
        }
    }
    chosen
}

struct Surface {
    faces: Vec<(Triangle, Vector)>,
}
impl Surface {
    fn new(points: &[Vector], indices: &[[u32; 3]]) -> Option<Self> {
        let center = points.iter().copied().sum::<Vector>() / points.len() as f32;
        let faces = indices
            .iter()
            .map(|t| {
                let triangle = Triangle::new(
                    *points.get(t[0] as usize)?,
                    *points.get(t[1] as usize)?,
                    *points.get(t[2] as usize)?,
                );
                let mut normal = triangle.normal()?;
                if normal.dot(triangle.a - center) < 0.0 {
                    normal = -normal;
                }
                Some((triangle, normal))
            })
            .collect::<Option<Vec<_>>>()?;
        (!faces.is_empty()).then_some(Self { faces })
    }
    fn distance_squared(&self, point: Vector) -> f32 {
        if self.faces.iter().all(|(triangle, normal)| normal.dot(point - triangle.a) <= 0.0) {
            return 0.0;
        }
        self.faces
            .iter()
            .map(|(triangle, _)| point.distance_squared(triangle.project_local_point(point, false).point))
            .fold(f32::MAX, f32::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_mesh::{Face, Vertex};

    fn cylinder() -> Model {
        let points: Vec<_> = (0..256)
            .map(|i| {
                let angle = (i % 128) as f32 * std::f32::consts::TAU / 128.0;
                Vector::new(angle.cos() * 0.5, angle.sin() * 0.5, if i < 128 { -0.5 } else { 0.5 })
            })
            .collect();
        let (points, triangles) = try_convex_hull(&points).expect("cylinder hull");
        Model {
            name: "Cylinder".into(),
            submodel: 0,
            center: glam::Vec3::ZERO,
            size: glam::Vec3::ONE,
            skin: None,
            faces: vec![Face {
                material: 0,
                vertices: points
                    .iter()
                    .map(|p| Vertex {
                        position: p.to_array(),
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.0; 2],
                        joints: [0; 4],
                        weights: [0.0; 4],
                    })
                    .collect(),
                indices: triangles.into_iter().flatten().collect(),
            }],
        }
    }

    #[test]
    fn adaptive_hull_reduces_vertices_preserves_support_and_bounds_decoded_error() {
        let model = cylinder();
        let source = super::super::base_hull(&model).expect("base hull");
        let scales = [glam::Vec3::ONE, glam::Vec3::new(0.02, 3.0, 0.1)];
        let result = base_hull(&model, &scales, &AtomicBool::new(false)).expect("adaptive hull");
        assert!(result.vertices.len() < source.vertices.len() / 2);
        let points: Vec<_> = result.vertices.iter().copied().map(Vector::from_array).collect();
        assert_eq!(bounds(&points), (Vector::splat(-0.5), Vector::splat(0.5)));
        for side in [-0.5, 0.5] {
            assert!(points.iter().filter(|p| p.z == side).count() >= 4);
        }
        let asset = crate::import_mesh::encode::asset(
            &[None, None, None, Some(model.clone())],
            None,
            &super::super::Physics {
                base: Some(result),
                hulls: Vec::new(),
            },
            Default::default(),
        )
        .expect("encoded asset");
        let header = crate::parse_mesh_header(&asset).expect("mesh header");
        let (offset, size) = header.physics_convex.expect("physics section");
        let decoded = crate::mesh_physics::decode(&asset[offset..offset + size])
            .expect("production physics decoder")
            .base
            .expect("base hull");
        for scale in scales {
            let scale = Vector::from_array(scale.to_array());
            let physical: Vec<_> = decoded.positions.iter().map(|p| Vector::from_array(*p) * scale).collect();
            let (vertices, indices) = try_convex_hull(&physical).expect("decoded hull");
            let surface = Surface::new(&vertices, &indices).expect("decoded surface");
            for p in &source.vertices {
                assert!(surface.distance_squared(Vector::from_array(*p) * scale).sqrt() <= RELATIVE_ERROR * scale.length() + 1e-6);
            }
        }
    }

    #[test]
    fn adaptive_hull_distance_is_euclidean_at_corners_and_cancellation_is_observed() {
        let points = [Vector::ZERO, Vector::X, Vector::Y, Vector::Z];
        let (points, indices) = try_convex_hull(&points).expect("tetrahedron");
        let surface = Surface::new(&points, &indices).expect("surface");
        assert_eq!(surface.distance_squared(Vector::splat(0.1)), 0.0);
        assert!((surface.distance_squared(Vector::splat(-1.0)) - 3.0).abs() < 1e-6);
        assert!(base_hull(&cylinder(), &[glam::Vec3::ONE], &AtomicBool::new(true)).is_err());
    }
}
