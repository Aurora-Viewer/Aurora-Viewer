//! Collision hulls from `LLModel::Decomposition::fromLLSD`
//! (indra/llprimitive/llmodel.cpp, originally LGPL 2.1).

use crate::{AssetError, MeshFace};
use glam::Vec3;
use parry3d::{math::Vector, transformation::try_convex_hull};

#[derive(Default)]
pub struct Decomposition {
    pub hulls: Vec<MeshFace>,
    pub base: Option<MeshFace>,
}

/// Triangulate an exact hull without the uploader's vertex simplification.
/// Call on a worker, including for convexified parametric prims.
pub fn convex_hull(points: impl IntoIterator<Item = [f32; 3]>) -> Result<MeshFace, AssetError> {
    let points: Vec<_> = points.into_iter().map(Vector::from_array).collect();
    if points.len() < 3 || points.len() > 65536 || points.iter().any(|p| !p.is_finite()) {
        return Err(AssetError::invalid("physics hull points"));
    }
    let (points, triangles) = try_convex_hull(&points).map_err(|_| AssetError::invalid("degenerate physics hull"))?;
    let indices = triangles
        .into_iter()
        .flatten()
        .map(u16::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AssetError::TooLarge("physics hull indices"))?;
    Ok(MeshFace {
        positions: points.into_iter().map(|p| [p.x, p.y, p.z]).collect(),
        indices,
        ..Default::default()
    })
}

/// Whether a mesh has a physics shape of its own, which the "Prim" physics
/// shape type uses: a `physics_mesh` block, or a convex decomposition with
/// at least one hull. Port of `LLMeshRepository::hasPhysicsShape`
/// (indra/newview/llmeshrepository.cpp, originally LGPL 2.1), which also
/// reads the hulls of `LLModel::Decomposition::fromLLSD` ("HullList" with
/// "Positions"). `convex` is the `physics_convex` block once held; `None`
/// while that block is still to be fetched, when the answer is not known yet.
pub fn has_physics_shape(header: &crate::MeshHeader, convex: Option<&[u8]>) -> Option<bool> {
    if header.physics_mesh.is_some() {
        return Some(true);
    }
    if header.physics_convex.is_none() {
        return Some(false);
    }
    // A damaged block counts as no hulls, as fromLLSD then leaves none.
    let data = crate::mesh::unzip_llsd(convex?).unwrap_or_default();
    Some(data.is_map() && data.has("Positions") && !data["HullList"].as_binary().is_empty())
}

pub fn decode(section: &[u8]) -> Result<Decomposition, AssetError> {
    let data = crate::mesh::unzip_llsd(section)?;
    // Secondary mesh models can carry an undefined decomposition.
    if matches!(data, aurora_llsd::Llsd::Undef) {
        return Ok(Decomposition::default());
    }
    if !data.is_map() {
        return Err(AssetError::invalid("physics decomposition is not a map"));
    }
    let (min, max) = if data.has("Min") {
        (Vec3::from_array(data["Min"].as_vec3()), Vec3::from_array(data["Max"].as_vec3()))
    } else {
        (Vec3::splat(-0.5), Vec3::splat(0.5))
    };
    if !min.is_finite() || !max.is_finite() || min.cmpgt(max).any() {
        return Err(AssetError::invalid("physics hull domain"));
    }
    let points = |bytes: &[u8]| -> Result<Vec<[f32; 3]>, AssetError> {
        if !bytes.len().is_multiple_of(6) {
            return Err(AssetError::Truncated("physics hull positions"));
        }
        Ok(bytes
            .as_chunks::<6>()
            .0
            .iter()
            .map(|p| {
                let q = Vec3::new(
                    u16::from_le_bytes([p[0], p[1]]) as f32,
                    u16::from_le_bytes([p[2], p[3]]) as f32,
                    u16::from_le_bytes([p[4], p[5]]) as f32,
                );
                (min + (max - min) * (q / 65535.0)).to_array()
            })
            .collect())
    };
    let counts = data["HullList"].as_binary();
    if counts.len() > 256 {
        return Err(AssetError::TooLarge("physics hull count"));
    }
    let mut positions = data["Positions"].as_binary();
    let mut hulls = Vec::with_capacity(counts.len());
    for count in counts {
        let count = if *count == 0 { 256 } else { *count as usize };
        let size = count * 6;
        let Some(bytes) = positions.get(..size) else {
            return Err(AssetError::Truncated("physics hull positions"));
        };
        hulls.push(convex_hull(points(bytes)?)?);
        positions = &positions[size..];
    }
    if !positions.is_empty() {
        return Err(AssetError::invalid("unclaimed physics hull positions"));
    }
    let base = if data.has("BoundingVerts") {
        Some(convex_hull(points(data["BoundingVerts"].as_binary())?)?)
    } else {
        None
    };
    Ok(Decomposition { hulls, base })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::{Llsd, llsd_map};
    use std::io::Write;

    fn compressed(data: Llsd) -> Vec<u8> {
        let mut out = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        out.write_all(&aurora_llsd::to_binary(&data)).unwrap();
        out.finish().unwrap()
    }

    #[test]
    fn physics_hulls_decode_domains_base_and_decomposition_independently() {
        let corners: Vec<u8> = (0..8)
            .flat_map(|i| (0..3).flat_map(move |axis| (if i & (1 << axis) == 0 { 0u16 } else { u16::MAX }).to_le_bytes()))
            .collect();
        let domain = |x: f64| Llsd::Array(vec![Llsd::Real(x); 3]);
        let block = compressed(llsd_map! {
            "Min" => domain(-2.0), "Max" => domain(3.0),
            "HullList" => Llsd::Binary(vec![8, 8]),
            "Positions" => Llsd::Binary([corners.clone(), corners.clone()].concat()),
            "BoundingVerts" => Llsd::Binary(corners),
        });
        let decoded = decode(&block).unwrap();
        assert_eq!(decoded.hulls.len(), 2);
        for face in decoded.hulls.iter().chain(decoded.base.iter()) {
            assert_eq!(face.indices.len(), 36);
            assert!(face.positions.contains(&[-2.0; 3]));
            assert!(face.positions.contains(&[3.0; 3]));
        }
    }

    #[test]
    fn a_mesh_has_a_physics_shape_with_a_physics_mesh_or_analysed_hulls() {
        let header = |physics_mesh: bool, physics_convex: bool| crate::MeshHeader {
            header_size: 0,
            lods: [None, None, None, Some((0, 1))],
            skin: None,
            physics_convex: physics_convex.then_some((10, 20)),
            physics_mesh: physics_mesh.then_some((30, 40)),
            version: 1,
            creator: uuid::Uuid::nil(),
        };
        let hulls = compressed(llsd_map! {
            "HullList" => Llsd::Binary(vec![8]),
            "Positions" => Llsd::Binary(vec![0; 48]),
            "BoundingVerts" => Llsd::Binary(vec![0; 48]),
        });
        // only the base hull the uploader always writes
        let base_only = compressed(llsd_map! { "BoundingVerts" => Llsd::Binary(vec![0; 48]) });
        assert_eq!(has_physics_shape(&header(true, false), None), Some(true), "a physics mesh");
        assert_eq!(has_physics_shape(&header(true, true), None), Some(true), "no need for the hulls");
        assert_eq!(has_physics_shape(&header(false, false), None), Some(false), "no physics block");
        assert_eq!(has_physics_shape(&header(false, true), None), None, "hulls still to fetch");
        assert_eq!(has_physics_shape(&header(false, true), Some(&hulls)), Some(true), "analysed hulls");
        assert_eq!(has_physics_shape(&header(false, true), Some(&base_only)), Some(false));
        assert_eq!(
            has_physics_shape(&header(false, true), Some(&[1, 2, 3])),
            Some(false),
            "damaged block"
        );
    }

    #[test]
    fn physics_hulls_reject_truncated_or_nonfinite_data() {
        for data in [
            llsd_map! { "HullList" => Llsd::Binary(vec![0]), "Positions" => Llsd::Binary(vec![0; 12]) },
            llsd_map! { "BoundingVerts" => Llsd::Binary(vec![0; 7]) },
            llsd_map! { "Min" => Llsd::Array(vec![Llsd::Real(f64::NAN); 3]) },
        ] {
            assert!(decode(&compressed(data)).is_err());
        }
    }
}
