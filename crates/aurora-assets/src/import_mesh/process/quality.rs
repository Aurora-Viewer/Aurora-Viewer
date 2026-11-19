//! Quality LOD generation. Unlike the fallback passes of
//! LLModelPreview::genMeshOptimizerLODs (indra/newview/llmodelpreview.cpp),
//! this path never discards UV/normal seams to reach the triangle target.

use super::{AssetError, LodOptions, Model, Vertex, shadow_indices};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
};

// meshoptimizer's error combines geometry and weighted attributes. This is
// an approximate simplification metric, not a pointwise UV/pixel guarantee.
// See meshoptimizer 1.3 README, "Attribute-aware simplification": UV weights
// of 10-100 and normal weights around 1. Start at the conservative low end
// for UVs; tiled UVs naturally carry a larger penalty than a single tile.
const ATTRIBUTE_WEIGHTS: [f32; 5] = [10.0, 10.0, 0.5, 0.5, 0.5];
const QUALITY_ERROR: f32 = 0.01;

pub(super) fn lod(model: &Model, options: LodOptions, cancel: &AtomicBool) -> Result<Model, AssetError> {
    cancelled(cancel)?;
    let mut vertices = Vec::new();
    let mut owners = Vec::new();
    let mut ranges = Vec::new();
    for (face_id, face) in model.faces.iter().enumerate() {
        face.validate()?;
        let start = vertices.len() as u32;
        vertices.extend_from_slice(&face.vertices);
        owners.resize(vertices.len(), face_id);
        ranges.push((start, vertices.len() as u32));
    }
    // Weld only identical attributes within the same material slot. Compact
    // unused/exporter-duplicated corners before detecting discontinuities.
    let shadow = shadow_indices(model, &ranges, 2);
    let mut remap = vec![u32::MAX; vertices.len()];
    let mut compact = Vec::<Vertex>::new();
    let mut compact_owners = Vec::new();
    let indices: Vec<_> = shadow
        .into_iter()
        .map(|index| {
            let mapped = &mut remap[index as usize];
            if *mapped == u32::MAX {
                *mapped = compact.len() as u32;
                compact.push(vertices[index as usize].clone());
                compact_owners.push(owners[index as usize]);
            }
            *mapped
        })
        .collect();
    if indices.is_empty() {
        return Ok(model.clone());
    }
    cancelled(cancel)?;
    // Remaining coincident vertices differ in UVs, normals, skin attributes
    // or material slot. Lock every side of the seam, including its first
    // representative, so simplification cannot shift its shape or mapping.
    let mut first = HashMap::new();
    let mut locked = vec![false; compact.len()];
    for (index, vertex) in compact.iter().enumerate() {
        let key = vertex.position.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
        if let Some(&other) = first.get(&key) {
            locked[index] = true;
            locked[other] = true;
        } else {
            first.insert(key, index);
        }
    }
    let attributes: Vec<_> = compact.iter().flat_map(|v| v.uv.into_iter().chain(v.normal)).collect();
    let target = if options.error_only {
        3
    } else {
        options.triangles.saturating_mul(3).max(3)
    }
    .min(indices.len());
    let error = if options.error_only { options.error } else { QUALITY_ERROR };
    let output = meshopt::simplify_with_attributes_and_locks_decoder(
        &indices,
        &compact,
        &attributes,
        &ATTRIBUTE_WEIGHTS,
        std::mem::size_of::<[f32; 5]>(),
        &locked,
        target,
        error,
        meshopt::SimplifyOptions::None,
        None,
    );
    cancelled(cancel)?;
    // No permissive/sloppy fallback and no component pruning: retaining
    // detail is preferable to satisfying a budget by dropping the part.
    if output.is_empty() {
        return Ok(model.clone());
    }
    let mut reduced = model.clone();
    for face in &mut reduced.faces {
        face.vertices.clear();
        face.indices.clear();
    }
    let mut remap = vec![u32::MAX; compact.len()];
    for triangle in output.as_chunks::<3>().0 {
        let owner = compact_owners[triangle[0] as usize];
        if triangle.iter().any(|i| compact_owners[*i as usize] != owner) {
            return Err(AssetError::invalid("quality LOD crossed a material boundary"));
        }
        let face = &mut reduced.faces[owner];
        for &index in triangle {
            let mapped = &mut remap[index as usize];
            if *mapped == u32::MAX {
                *mapped = face.vertices.len() as u32;
                face.vertices.push(compact[index as usize].clone());
            }
            face.indices.push(*mapped);
        }
    }
    for face in &mut reduced.faces {
        if let Some(angle) = options.crease_angle {
            super::normals(face, angle)?;
        }
        face.validate()?;
    }
    Ok(reduced)
}

fn cancelled(cancel: &AtomicBool) -> Result<(), AssetError> {
    if cancel.load(Ordering::Relaxed) {
        Err(AssetError::invalid("cancelled"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_mesh::Face;

    fn grid(warp: bool) -> Model {
        let vertices = (0..=16)
            .flat_map(|y| {
                (0..=16).map(move |x| {
                    let (x, y) = (x as f32 / 16.0, y as f32 / 16.0);
                    let bump = if warp {
                        0.2 * (x * std::f32::consts::PI).sin() * (y * std::f32::consts::PI).sin()
                    } else {
                        0.0
                    };
                    Vertex {
                        position: [x, y, 0.0],
                        uv: [x + bump, y],
                        normal: [0.0, 0.0, 1.0],
                        joints: [0; 4],
                        weights: [0.0; 4],
                    }
                })
            })
            .collect();
        let indices = (0..16)
            .flat_map(|y| {
                (0..16).flat_map(move |x| {
                    let i = y * 17 + x;
                    [i, i + 1, i + 17, i + 1, i + 18, i + 17]
                })
            })
            .collect();
        Model {
            name: "UV grid".into(),
            submodel: 0,
            center: glam::Vec3::ZERO,
            size: glam::Vec3::ONE,
            skin: None,
            faces: vec![Face {
                vertices,
                indices,
                material: 0,
            }],
        }
    }
    fn options() -> LodOptions {
        LodOptions {
            triangles: 2,
            error: 1.0,
            method: super::super::Method::Quality,
            crease_angle: None,
            error_only: false,
        }
    }
    fn uv_error(source: &Model, reduced: &Model) -> f32 {
        let face = &reduced.faces[0];
        source.faces[0]
            .vertices
            .iter()
            .map(|p| {
                face.indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .filter_map(|t| {
                        let [a, b, c] = t.map(|i| &face.vertices[i as usize]);
                        let xy = |v: &Vertex| glam::Vec2::new(v.position[0], v.position[1]);
                        let (ab, ac, ap) = (xy(b) - xy(a), xy(c) - xy(a), xy(p) - xy(a));
                        let area = ab.perp_dot(ac);
                        let (u, v) = (ap.perp_dot(ac) / area, ab.perp_dot(ap) / area);
                        if u < -1e-5 || v < -1e-5 || u + v > 1.00001 {
                            return None;
                        }
                        let uv = glam::Vec2::from_array(a.uv) * (1.0 - u - v)
                            + glam::Vec2::from_array(b.uv) * u
                            + glam::Vec2::from_array(c.uv) * v;
                        Some(uv.distance(glam::Vec2::from_array(p.uv)))
                    })
                    .reduce(f32::min)
                    .expect("source sample covered by LOD")
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn quality_lod_protects_texture_mapping_inside_an_island_and_keeps_a_soft_budget() {
        let cancel = AtomicBool::new(false);
        let flat = lod(&grid(false), options(), &cancel).expect("affine UV LOD");
        assert_eq!(flat.triangles(), 2, "affine UVs do not prevent useful reduction");
        let source = grid(true);
        let quality = lod(&source, options(), &cancel).expect("quality LOD");
        let geometry = super::super::lod(
            &source,
            LodOptions {
                method: super::super::Method::Precise,
                ..options()
            },
            &cancel,
        )
        .expect("geometry LOD");
        assert!(quality.triangles() > options().triangles && quality.triangles() < source.triangles());
        assert!(uv_error(&source, &quality) < 0.02, "preserve the curved texture mapping");
        assert!(uv_error(&source, &geometry) > 0.15, "the fixture detects geometry-only reduction");
        assert!(lod(&source, options(), &AtomicBool::new(true)).is_err());
    }

    #[test]
    fn quality_lod_keeps_both_sides_of_uv_normal_and_material_boundaries() {
        for boundary in ["uv", "normal", "material"] {
            let mut source = grid(false);
            let mut second = source.faces[0].clone();
            for v in &mut second.vertices {
                v.position[0] += 1.0;
                if boundary == "uv" {
                    v.uv[0] += 2.0;
                } else {
                    v.uv[0] += 1.0;
                }
                if boundary == "normal" {
                    v.normal = [0.0, 1.0, 0.0];
                }
            }
            if boundary == "material" {
                second.material = 1;
                source.faces.push(second);
            } else {
                let face = &mut source.faces[0];
                let start = face.vertices.len() as u32;
                face.indices.extend(second.indices.iter().map(|i| start + i));
                face.vertices.extend(second.vertices);
            }
            let reduced = lod(&source, options(), &AtomicBool::new(false)).expect("protected LOD");
            for (original, output) in source.faces.iter().zip(&reduced.faces) {
                assert_eq!(original.material, output.material);
                for v in original.vertices.iter().filter(|v| v.position[0] == 1.0) {
                    assert!(
                        output
                            .vertices
                            .iter()
                            .any(|r| r.position == v.position && r.uv == v.uv && r.normal == v.normal),
                        "lost {boundary} seam vertex"
                    );
                }
            }
            assert!(reduced.triangles() > 2);
            assert!(reduced.triangles() < source.triangles());
        }
    }
}
