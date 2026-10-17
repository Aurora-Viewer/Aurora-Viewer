//! LLModel::writeModel / writeModelToStream and skin/decomposition LLSD.
//! Per-section compression and every size/count are bounded before upload.

use super::*;
use aurora_llsd::{Llsd, llsd_map};
use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub skin: bool,
    pub joints: bool,
    pub lock_scale: bool,
    pub pelvis_offset: f32,
    pub secondary: Option<u32>,
}

fn floats(values: impl IntoIterator<Item = f32>) -> Llsd {
    Llsd::Array(values.into_iter().map(|v| Llsd::Real(f64::from(v))).collect())
}
fn matrix(value: Mat4) -> Llsd {
    floats(value.to_cols_array())
}
fn quantize(value: f32, min: f32, max: f32) -> u16 {
    if max == min {
        0
    } else {
        ((value - min) / (max - min) * 65535.0).clamp(0.0, 65535.0).floor() as u16
    }
}
fn compressed(value: &Llsd) -> Result<Vec<u8>, AssetError> {
    let bytes = aurora_llsd::to_binary(value);
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(AssetError::TooLarge("mesh section"));
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&bytes).map_err(|e| AssetError::invalid(e.to_string()))?;
    encoder.finish().map_err(|e| AssetError::invalid(e.to_string()))
}

pub fn lod(model: &Model, include_skin: bool) -> Result<Llsd, AssetError> {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for face in &model.faces {
        face.validate()?;
        for vertex in &face.vertices {
            let point = Vec3::from_array(vertex.position);
            min = min.min(point);
            max = max.max(point);
        }
    }
    let mut faces = Vec::new();
    for face in &model.faces {
        if face.vertices.len() < 3 || face.indices.is_empty() {
            faces.push(llsd_map! {"NoGeometry" => true});
            continue;
        }
        let mut uv_min = [f32::MAX; 2];
        let mut uv_max = [f32::MIN; 2];
        for v in &face.vertices {
            for axis in 0..2 {
                uv_min[axis] = uv_min[axis].min(v.uv[axis]);
                uv_max[axis] = uv_max[axis].max(v.uv[axis]);
            }
        }
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv = Vec::new();
        let mut indices = Vec::new();
        let mut weights = Vec::new();
        for vertex in &face.vertices {
            for axis in 0..3 {
                positions.extend(quantize(vertex.position[axis], min[axis], max[axis]).to_le_bytes());
                normals.extend(quantize(vertex.normal[axis], -1.0, 1.0).to_le_bytes());
            }
            for axis in 0..2 {
                uv.extend(quantize(vertex.uv[axis], uv_min[axis], uv_max[axis]).to_le_bytes());
            }
            if include_skin {
                let mut count = 0;
                for (joint, weight) in vertex.joints.iter().zip(vertex.weights) {
                    if weight > 0.0 {
                        if *joint == 255 {
                            return Err(AssetError::invalid("skin joint terminator index"));
                        }
                        weights.push(*joint);
                        weights.extend(quantize(weight, 0.0, 1.0).max(1).to_le_bytes());
                        count += 1;
                    }
                }
                if count < 4 {
                    weights.push(255);
                }
            }
        }
        for index in &face.indices {
            indices.extend((*index as u16).to_le_bytes());
        }
        let mut data = llsd_map! { "Position" => Llsd::Binary(positions), "PositionDomain" => llsd_map! { "Min" => floats(min.to_array()), "Max" => floats(max.to_array()) },
        "Normal" => Llsd::Binary(normals), "TexCoord0" => Llsd::Binary(uv), "TexCoord0Domain" => llsd_map! {"Min" => floats(uv_min), "Max" => floats(uv_max)}, "TriangleList" => Llsd::Binary(indices) };
        if include_skin {
            data.insert("Weights", Llsd::Binary(weights));
        }
        // the size taken out by the normalization, which the tangents are
        // found in (LLModel::writeModel, LLVolumeFace::cacheOptimize)
        if model.size.is_finite() && model.size.min_element() > 0.0 {
            data.insert("NormalizedScale", floats(model.size.to_array()));
        }
        faces.push(data);
    }
    Ok(Llsd::Array(faces))
}

fn skin(skin: &Skin, options: Options) -> Result<Llsd, AssetError> {
    if skin.names.is_empty()
        || skin.names.len() > 110
        || skin.names.len() != skin.inverse_bind.len()
        || !skin.bind_shape.is_finite()
        || !options.pelvis_offset.is_finite()
        || skin.inverse_bind.iter().any(|matrix| !matrix.is_finite())
    {
        return Err(AssetError::invalid("skin upload parameters"));
    }
    let mut data = llsd_map! { "joint_names" => Llsd::Array(skin.names.iter().map(|name| Llsd::String(name.clone())).collect()),
    "inverse_bind_matrix" => Llsd::Array(skin.inverse_bind.iter().map(|m| matrix(*m)).collect()), "bind_shape_matrix" => matrix(skin.bind_shape) };
    if options.joints {
        if skin.alternate_bind.len() != skin.names.len() || skin.alternate_bind.iter().any(|m| !m.is_finite()) {
            return Err(AssetError::invalid("alternate skin binds"));
        }
        data.insert(
            "alt_inverse_bind_matrix",
            Llsd::Array(skin.alternate_bind.iter().map(|m| matrix(*m)).collect()),
        );
        data.insert("pelvis_offset", Llsd::Real(f64::from(options.pelvis_offset)));
        if options.lock_scale {
            data.insert("lock_scale_if_joint_position", Llsd::Boolean(true));
        }
    }
    Ok(data)
}

fn convex(physics: &process::Physics) -> Result<Llsd, AssetError> {
    if physics.hulls.len() > 256
        || physics.hulls.iter().any(|h| !(3..=256).contains(&h.vertices.len()))
        || physics.base.as_ref().is_some_and(|h| h.vertices.len() > 256)
    {
        return Err(AssetError::TooLarge("convex physics limits"));
    }
    let all = physics.hulls.iter().chain(physics.base.iter()).flat_map(|h| h.vertices.iter());
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for point in all {
        let p = Vec3::from_array(*point);
        if !p.is_finite() {
            return Err(AssetError::invalid("nonfinite hull point"));
        }
        min = min.min(p);
        max = max.max(p);
    }
    if min.x == f32::MAX {
        return Ok(Llsd::new_map());
    }
    let positions = |points: &[[f32; 3]]| -> Vec<u8> {
        points
            .iter()
            .flat_map(|p| (0..3).flat_map(|axis| quantize(p[axis], min[axis], max[axis]).to_le_bytes()))
            .collect()
    };
    let mut data = llsd_map! { "Min" => floats(min.to_array()), "Max" => floats(max.to_array()) };
    if !physics.hulls.is_empty() {
        data.insert(
            "HullList",
            Llsd::Binary(physics.hulls.iter().map(|h| h.vertices.len() as u8).collect()),
        );
        data.insert(
            "Position",
            Llsd::Binary(physics.hulls.iter().flat_map(|h| positions(&h.vertices)).collect()),
        );
    }
    if let Some(base) = &physics.base {
        data.insert("BoundingVerts", Llsd::Binary(positions(&base.vertices)));
    }
    Ok(data)
}

pub fn asset(
    lods: &[Option<Model>; 4],
    physics: Option<&Model>,
    decomposition: &process::Physics,
    options: Options,
) -> Result<Vec<u8>, AssetError> {
    let high = lods[3].as_ref().ok_or_else(|| AssetError::invalid("mesh has no high LOD"))?;
    if high.faces.len() > 8 {
        return Err(AssetError::TooLarge("mesh textureable faces"));
    }
    let mut sections = Vec::<(&str, Vec<u8>)>::new();
    if options.skin
        && let Some(value) = &high.skin
    {
        sections.push(("skin", compressed(&skin(value, options)?)?));
    }
    let empty = process::Physics::default();
    let decomposition = if options.secondary.is_some() { &empty } else { decomposition };
    sections.push(("physics_convex", compressed(&convex(decomposition)?)?));
    for (index, key) in ["lowest_lod", "low_lod", "medium_lod", "high_lod"].into_iter().enumerate() {
        if let Some(model) = &lods[index] {
            if model.faces.len() != high.faces.len() {
                return Err(AssetError::invalid("LOD textureable face counts disagree"));
            }
            if options.skin
                && let Some(skin) = &high.skin
                && model.faces.iter().flat_map(|f| &f.vertices).any(|v| {
                    v.weights.iter().sum::<f32>() <= 0.0
                        || v.weights.iter().sum::<f32>() > 1.001
                        || v.joints
                            .iter()
                            .zip(v.weights)
                            .any(|(j, w)| w > 0.0 && *j as usize >= skin.names.len())
                })
            {
                return Err(AssetError::invalid("missing or invalid LOD skin weights"));
            }
            sections.push((key, compressed(&lod(model, options.skin && high.skin.is_some())?)?));
        }
    }
    if options.secondary.is_none()
        && decomposition.hulls.is_empty()
        && let Some(model) = physics
    {
        sections.push(("physics_mesh", compressed(&lod(model, false)?)?));
    }
    let mut header = llsd_map! { "version" => 1 };
    if let Some(index) = options.secondary {
        header.insert("secondary", Llsd::Boolean(true));
        header.insert("submodel_id", Llsd::Integer(index as i32));
    }
    let mut offset = 0;
    for (key, bytes) in &sections {
        header.insert(*key, llsd_map! { "offset" => offset, "size" => bytes.len() as i32 });
        offset += bytes.len() as i32;
    }
    let mut output = aurora_llsd::to_binary(&header);
    for (_, bytes) in sections {
        output.extend(bytes);
    }
    if output.len() > 64 * 1024 * 1024 {
        return Err(AssetError::TooLarge("mesh asset"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn triangle() -> Model {
        Model {
            name: "Test".into(),
            submodel: 0,
            faces: vec![Face {
                material: 0,
                indices: vec![0, 1, 2],
                vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
                    .into_iter()
                    .map(|position| Vertex {
                        position,
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.0; 2],
                        joints: [0; 4],
                        weights: [0.0; 4],
                    })
                    .collect(),
            }],
            skin: None,
            center: Vec3::ZERO,
            size: Vec3::ONE,
        }
    }
    #[test]
    fn serialized_mesh_reads_with_the_production_decoder_including_flat_domains() {
        let model = triangle();
        let lods = [Some(model.clone()), Some(model.clone()), Some(model.clone()), Some(model)];
        let encoded = asset(&lods, None, &process::Physics::default(), Options::default()).expect("encode");
        let header = crate::parse_mesh_header(&encoded).expect("header");
        for section in header.lods.into_iter().flatten() {
            let faces = crate::decode_mesh_lod(&encoded[section.0..section.0 + section.1]).expect("decode");
            assert_eq!(faces[0].indices, [0, 1, 2]);
            assert_eq!(faces[0].positions[2], [0.0, 1.0, 0.0]);
        }
    }
}
