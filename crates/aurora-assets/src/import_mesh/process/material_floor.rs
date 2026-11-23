//! Material survival after LOD simplification. LLModelPreview's fallback in
//! genMeshOptimizerPerModel can leave a [0, 0, 0] placeholder for a lost face
//! (indra/newview/llmodelpreview.cpp). Aurora keeps a real source triangle
//! instead, and exposes one triangle per occupied material as Automatic's
//! explicit minimum. Error-limited reduction never forces that minimum.

use super::{AssetError, Face, LegacyPass, LodOptions, Method, Model, Vec3, legacy_per_model, minimum_lod_triangles};
use std::sync::atomic::{AtomicBool, Ordering};

fn largest_triangle(face: &Face) -> Option<([u32; 3], f64)> {
    face.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| {
            let [a, b, c] = triangle.map(|index| Vec3::from_array(face.vertices[index as usize].position).as_dvec3());
            (*triangle, (b - a).cross(c - a).length_squared())
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

fn keep_triangle(face: &mut Face, triangle: [u32; 3]) {
    face.indices = triangle.to_vec();
    face.compact();
}

/// At the explicitly requested floor, try the same Legacy passes within
/// each material so a welded neighbour cannot take its last triangle or UVs.
/// Keep the largest surviving triangle as a deterministic representative.
fn minimum_face(source: &Face, cancel: &AtomicBool) -> Result<Face, AssetError> {
    let single = Model {
        name: String::new(),
        submodel: 0,
        faces: vec![source.clone()],
        skin: None,
        center: Vec3::ZERO,
        size: Vec3::ONE,
    };
    let mut chosen = source.clone();
    let decimator = single.triangles() as f32;
    for pass in [LegacyPass::Full, LegacyPass::NoNormals, LegacyPass::NoUvs, LegacyPass::NoTopology] {
        if cancel.load(Ordering::Relaxed) {
            return Err(AssetError::invalid("cancelled"));
        }
        if let Some((mut candidate, _)) = legacy_per_model(&single, decimator, 1.0, pass) {
            let face = candidate.faces.remove(0);
            if face.indices.len() < chosen.indices.len() && largest_triangle(&face).is_some_and(|(_, area)| area > 0.0) {
                chosen = face;
                if chosen.indices.len() == 3 {
                    break;
                }
            }
        }
    }
    if let Some((triangle, _)) = largest_triangle(&chosen) {
        keep_triangle(&mut chosen, triangle);
    }
    Ok(chosen)
}

pub(super) fn enforce(source: &Model, reduced: &mut Model, options: LodOptions, cancel: &AtomicBool) -> Result<(), AssetError> {
    let exact_minimum = options.method == Method::Legacy && !options.error_only && options.triangles <= minimum_lod_triangles(source);
    for (original, face) in source.faces.iter().zip(&mut reduced.faces) {
        if cancel.load(Ordering::Relaxed) {
            return Err(AssetError::invalid("cancelled"));
        }
        if original.indices.is_empty() {
            face.indices.clear();
            continue;
        }
        if exact_minimum {
            *face = minimum_face(original, cancel)?;
        } else if !largest_triangle(face).is_some_and(|(_, area)| area > 0.0) {
            // Restore this slot's own attributes, including skin weights,
            // without borrowing a vertex from a neighbouring material.
            *face = original.clone();
            // An error limit is not permission to discard the rest of the
            // surface: without a usable result, restore that face in full.
            if !options.error_only
                && let Some((triangle, _)) = largest_triangle(face)
            {
                keep_triangle(face, triangle);
            }
        }
    }
    Ok(())
}
