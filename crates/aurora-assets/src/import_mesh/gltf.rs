//! glTF accessors and mesh scene hierarchy. All offsets/counts/strides are
//! checked before allocation or reads; sparse and normalized attributes are
//! supported. Geometry is converted from glTF Y-up to SL Z-up once.

use super::*;
use crate::import_gltf::Document;
use glam::Quat;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn integer(value: &Value, field: &'static str) -> Result<usize, AssetError> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| AssetError::invalid(field))
}
fn component(bytes: &[u8], ty: usize, normalized: bool) -> Result<f32, AssetError> {
    let n = match ty {
        5120 => {
            let n = bytes[0] as i8;
            if normalized { (n as f32 / 127.0).max(-1.0) } else { n as f32 }
        }
        5121 => {
            if normalized {
                bytes[0] as f32 / 255.0
            } else {
                bytes[0] as f32
            }
        }
        5122 => {
            let n = i16::from_le_bytes([bytes[0], bytes[1]]);
            if normalized { (n as f32 / 32767.0).max(-1.0) } else { n as f32 }
        }
        5123 => {
            let n = u16::from_le_bytes([bytes[0], bytes[1]]);
            if normalized { n as f32 / 65535.0 } else { n as f32 }
        }
        5125 => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
        5126 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        _ => return Err(AssetError::Unsupported("glTF component type".into())),
    };
    if !n.is_finite() {
        return Err(AssetError::invalid("nonfinite glTF component"));
    }
    Ok(n)
}
pub fn attribute<const N: usize>(doc: &Document, index: usize) -> Result<Vec<[f32; N]>, AssetError> {
    let accessor = doc.json["accessors"]
        .get(index)
        .ok_or_else(|| AssetError::invalid("glTF accessor index"))?;
    let expected = match N {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        16 => "MAT4",
        _ => return Err(AssetError::invalid("accessor width")),
    };
    if accessor["type"].as_str() != Some(expected) {
        return Err(AssetError::invalid("accessor shape"));
    }
    let count = integer(&accessor["count"], "accessor count")?;
    if count > 4 * 1024 * 1024 || count.checked_mul(N).is_none_or(|n| n > 16 * 1024 * 1024) {
        return Err(AssetError::TooLarge("glTF accessor"));
    }
    let ty = integer(&accessor["componentType"], "accessor component type")?;
    let width = match ty {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        _ => return Err(AssetError::Unsupported("glTF component type".into())),
    };
    let normalized = accessor["normalized"].as_bool().unwrap_or(false);
    if normalized && matches!(ty, 5125 | 5126) {
        return Err(AssetError::invalid("normalized integer/float accessor"));
    }
    let mut values = vec![[0.0; N]; count];
    let read = |data: &[u8], offset: usize, stride: usize, values: &mut [[f32; N]]| -> Result<(), AssetError> {
        if stride < N * width || !stride.is_multiple_of(width) {
            return Err(AssetError::invalid("accessor stride"));
        }
        for (row, value) in values.iter_mut().enumerate() {
            let start = row
                .checked_mul(stride)
                .and_then(|n| offset.checked_add(n))
                .ok_or(AssetError::Truncated("accessor offset"))?;
            for (column, dst) in value.iter_mut().enumerate() {
                let start = start
                    .checked_add(column * width)
                    .ok_or(AssetError::Truncated("accessor component"))?;
                *dst = component(
                    data.get(start..start + width).ok_or(AssetError::Truncated("accessor values"))?,
                    ty,
                    normalized,
                )?;
            }
        }
        Ok(())
    };
    if accessor.get("bufferView").is_some() {
        let view = integer(&accessor["bufferView"], "accessor view")?;
        let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = doc.json["bufferViews"][view]["byteStride"].as_u64().unwrap_or((N * width) as u64) as usize;
        read(doc.view(view)?, offset, stride, &mut values)?;
    } else if accessor.get("sparse").is_none() {
        return Err(AssetError::invalid("accessor has neither view nor sparse data"));
    }
    if let Some(sparse) = accessor.get("sparse") {
        let count_sparse = integer(&sparse["count"], "sparse count")?;
        if count_sparse > count {
            return Err(AssetError::invalid("sparse count exceeds accessor"));
        }
        let indices = &sparse["indices"];
        let index_ty = integer(&indices["componentType"], "sparse index type")?;
        let index_width = match index_ty {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => return Err(AssetError::invalid("sparse index component type")),
        };
        let index_data = doc.view(integer(&indices["bufferView"], "sparse index view")?)?;
        let index_offset = indices["byteOffset"].as_u64().unwrap_or(0) as usize;
        let mut patches = vec![[0.0; N]; count_sparse];
        let source = &sparse["values"];
        read(
            doc.view(integer(&source["bufferView"], "sparse values view")?)?,
            source["byteOffset"].as_u64().unwrap_or(0) as usize,
            N * width,
            &mut patches,
        )?;
        let mut previous = None;
        for (row, patch) in patches.into_iter().enumerate() {
            let start = row
                .checked_mul(index_width)
                .and_then(|n| n.checked_add(index_offset))
                .ok_or(AssetError::Truncated("sparse indices"))?;
            let index = component(
                index_data
                    .get(start..start + index_width)
                    .ok_or(AssetError::Truncated("sparse indices"))?,
                index_ty,
                false,
            )? as usize;
            if index >= count || previous.is_some_and(|previous| index <= previous) {
                return Err(AssetError::invalid("sparse indices must be ordered and in range"));
            }
            previous = Some(index);
            values[index] = patch;
        }
    }
    Ok(values)
}

fn transform(node: &Value) -> Result<Mat4, AssetError> {
    let array = |key: &str, defaults: &[f32]| -> Result<Vec<f32>, AssetError> {
        if let Some(values) = node[key].as_array() {
            if values.len() != defaults.len() {
                return Err(AssetError::invalid("glTF transform shape"));
            }
            values
                .iter()
                .map(|v| {
                    v.as_f64()
                        .map(|n| n as f32)
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| AssetError::invalid("glTF transform number"))
                })
                .collect()
        } else {
            Ok(defaults.to_vec())
        }
    };
    if node.get("matrix").is_some() {
        let numbers = array("matrix", &Mat4::IDENTITY.to_cols_array())?;
        let matrix: [f32; 16] = numbers.try_into().map_err(|_| AssetError::invalid("node matrix"))?;
        return Ok(Mat4::from_cols_array(&matrix));
    }
    let t = array("translation", &[0.0; 3])?;
    let s = array("scale", &[1.0; 3])?;
    let r = array("rotation", &[0.0, 0.0, 0.0, 1.0])?;
    if s.iter().any(|n| *n <= 0.0) {
        return Err(AssetError::invalid("negative or zero node scale"));
    }
    let rotation = Quat::from_xyzw(r[0], r[1], r[2], r[3]);
    if rotation.length_squared() < 0.000001 {
        return Err(AssetError::invalid("zero node quaternion"));
    }
    Ok(Mat4::from_scale_rotation_translation(
        Vec3::new(s[0], s[1], s[2]),
        rotation.normalize(),
        Vec3::new(t[0], t[1], t[2]),
    ))
}

pub fn parse(doc: &Document, skeleton: &crate::Skeleton) -> Result<Scene, AssetError> {
    let nodes = doc.json["nodes"]
        .as_array()
        .ok_or_else(|| AssetError::invalid("glTF has no nodes"))?;
    if nodes.len() > 8192 {
        return Err(AssetError::TooLarge("glTF nodes"));
    }
    let basis = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let mut globals = vec![Mat4::IDENTITY; nodes.len()];
    let mut instances = Vec::new();
    let scene_index = doc.json["scene"].as_u64().unwrap_or(0) as usize;
    let roots = doc.json["scenes"][scene_index]["nodes"]
        .as_array()
        .ok_or_else(|| AssetError::invalid("glTF has no selected scene"))?;
    fn visit(
        index: usize,
        parent: Mat4,
        nodes: &[Value],
        globals: &mut [Mat4],
        instances: &mut Vec<usize>,
        visiting: &mut HashSet<usize>,
        depth: usize,
    ) -> Result<(), AssetError> {
        if depth >= 128 || !visiting.insert(index) {
            return Err(AssetError::invalid("cyclic or too deep glTF hierarchy"));
        }
        let node = nodes.get(index).ok_or_else(|| AssetError::invalid("glTF node index"))?;
        let global = parent * transform(node)?;
        if !global.is_finite() || global.determinant() <= 0.0 {
            return Err(AssetError::invalid("node transform is mirrored or singular"));
        }
        globals[index] = global;
        if node.get("mesh").is_some() {
            instances.push(index);
        }
        for child in node["children"].as_array().into_iter().flatten() {
            visit(
                integer(child, "child node index")?,
                global,
                nodes,
                globals,
                instances,
                visiting,
                depth + 1,
            )?;
        }
        visiting.remove(&index);
        Ok(())
    }
    for root in roots {
        visit(
            integer(root, "root node index")?,
            Mat4::IDENTITY,
            nodes,
            &mut globals,
            &mut instances,
            &mut HashSet::new(),
            0,
        )?;
    }
    if instances.is_empty() || instances.len() > 256 {
        return Err(AssetError::invalid("mesh scene must contain between one and 256 instances"));
    }
    let mut scene = Scene::default();
    if doc.json["materials"].is_array() {
        scene.materials = doc
            .materials()?
            .into_iter()
            .map(|material| Material {
                name: material.name,
                value: material.value,
                images: material.images,
            })
            .collect();
    }
    let fallback_material = scene.materials.len();
    scene.materials.push(Material {
        name: "Default".into(),
        value: PbrMaterial::default(),
        images: [None; 4],
    });
    let mut mesh_models = HashMap::<(usize, Option<usize>), Vec<usize>>::new();
    for node_index in instances {
        let node = &nodes[node_index];
        let mesh_index = integer(&node["mesh"], "mesh index")?;
        let skin_index = node["skin"].as_u64().map(|n| n as usize);
        let key = (mesh_index, skin_index);
        if let std::collections::hash_map::Entry::Vacant(e) = mesh_models.entry(key) {
            let mesh = doc.json["meshes"]
                .get(mesh_index)
                .ok_or_else(|| AssetError::invalid("mesh index"))?;
            let skin = if let Some(index) = skin_index {
                Some(read_skin(doc, index, nodes, basis, skeleton, &mut scene.warnings)?)
            } else {
                None
            };
            let mut faces = Vec::new();
            let primitives = mesh["primitives"]
                .as_array()
                .ok_or_else(|| AssetError::invalid("mesh has no primitives"))?;
            for primitive in primitives {
                let attributes = &primitive["attributes"];
                let positions = attribute::<3>(doc, integer(&attributes["POSITION"], "mesh positions")?)?;
                let normals = attributes["NORMAL"].as_u64().map(|i| attribute::<3>(doc, i as usize)).transpose()?;
                let uvs = attributes["TEXCOORD_0"]
                    .as_u64()
                    .map(|i| attribute::<2>(doc, i as usize))
                    .transpose()?;
                let joints = attributes["JOINTS_0"]
                    .as_u64()
                    .map(|i| attribute::<4>(doc, i as usize))
                    .transpose()?;
                let weights = attributes["WEIGHTS_0"]
                    .as_u64()
                    .map(|i| attribute::<4>(doc, i as usize))
                    .transpose()?;
                if normals.as_ref().is_some_and(|v| v.len() != positions.len())
                    || uvs.as_ref().is_some_and(|v| v.len() != positions.len())
                    || joints.as_ref().is_some_and(|v| v.len() != positions.len())
                    || weights.as_ref().is_some_and(|v| v.len() != positions.len())
                    || joints.is_some() != weights.is_some()
                {
                    return Err(AssetError::invalid("primitive attribute counts disagree"));
                }
                let mut vertices = Vec::with_capacity(positions.len());
                for (i, position) in positions.into_iter().enumerate() {
                    let mut ids = [0; 4];
                    let mut influence = weights.as_ref().map(|w| w[i]).unwrap_or([0.0; 4]);
                    for slot in 0..4 {
                        let index = joints.as_ref().map(|j| j[i][slot]).unwrap_or(0.0);
                        if index < 0.0
                            || index > 254.0
                            || index.fract() != 0.0
                            || skin
                                .as_ref()
                                .is_some_and(|s| influence[slot] > 0.0 && index as usize >= s.names.len())
                        {
                            return Err(AssetError::invalid("joint index is outside skin"));
                        }
                        ids[slot] = index as u8;
                    }
                    let sum: f32 = influence.iter().sum();
                    if sum > 0.0 {
                        for w in &mut influence {
                            *w /= sum;
                        }
                    }
                    let normal = normals.as_ref().map(|n| n[i]).unwrap_or([0.0; 3]);
                    let uv = uvs.as_ref().map(|u| u[i]).unwrap_or([0.0; 2]);
                    vertices.push(Vertex {
                        position: basis.transform_point3(Vec3::from_array(position)).to_array(),
                        normal: basis.transform_vector3(Vec3::from_array(normal)).normalize_or_zero().to_array(),
                        uv: [uv[0], 1.0 - uv[1]],
                        joints: ids,
                        weights: influence,
                    });
                }
                let mut indices = if let Some(index) = primitive["indices"].as_u64() {
                    let accessor = &doc.json["accessors"][index as usize];
                    if !matches!(accessor["componentType"].as_u64(), Some(5121 | 5123 | 5125)) {
                        return Err(AssetError::invalid("indices must be unsigned integers"));
                    }
                    attribute::<1>(doc, index as usize)?
                        .into_iter()
                        .map(|v| v[0] as u32)
                        .collect::<Vec<_>>()
                } else {
                    (0..vertices.len() as u32).collect()
                };
                match primitive["mode"].as_u64().unwrap_or(4) {
                    4 => {}
                    5 => {
                        let old = indices;
                        indices = old
                            .windows(3)
                            .enumerate()
                            .flat_map(|(i, t)| if i % 2 == 0 { [t[0], t[1], t[2]] } else { [t[1], t[0], t[2]] })
                            .collect();
                    }
                    6 => {
                        let old = indices;
                        indices = old
                            .get(1..)
                            .unwrap_or_default()
                            .windows(2)
                            .flat_map(|t| [old[0], t[0], t[1]])
                            .collect();
                    }
                    _ => return Err(AssetError::Unsupported("mesh primitive must contain triangles".into())),
                }
                let material = primitive["material"].as_u64().map_or(fallback_material, |i| i as usize);
                if material >= scene.materials.len() {
                    return Err(AssetError::invalid("mesh material index"));
                }
                let mut split = split_face(vertices, indices, material)?;
                if normals.is_none() {
                    for face in &mut split {
                        process::normals(face, 180.0)?;
                    }
                }
                faces.extend(split);
            }
            let mut models = Vec::new();
            let (center, size) = normalization(&faces)?;
            for (submodel, group) in faces.chunks(8).enumerate() {
                let name = mesh["name"].as_str().unwrap_or("Mesh").to_owned();
                let mut model = Model {
                    name,
                    submodel: submodel as u32,
                    faces: group.to_vec(),
                    skin: skin.clone(),
                    center: Vec3::ZERO,
                    size: Vec3::ONE,
                };
                model.normalize_to(center, size)?;
                models.push(scene.models.len());
                scene.models.push(model);
            }
            e.insert(models);
        }
        for model in &mesh_models[&key] {
            let source = &scene.models[*model];
            let normalized_to_source = Mat4::from_scale_rotation_translation(source.size, Quat::IDENTITY, source.center);
            scene.instances.push(Instance {
                model: *model,
                transform: basis * globals[node_index] * basis.inverse() * normalized_to_source,
                name: node["name"].as_str().unwrap_or(&source.name).into(),
                materials: source.faces.iter().map(|face| face.material).collect(),
            });
        }
    }
    if scene.models.is_empty() || scene.instances.len() > 256 {
        return Err(AssetError::TooLarge("split mesh instances"));
    }
    Ok(scene)
}

fn read_skin(
    doc: &Document,
    index: usize,
    nodes: &[Value],
    basis: Mat4,
    skeleton: &crate::Skeleton,
    warnings: &mut Vec<String>,
) -> Result<Skin, AssetError> {
    let skin = doc.json["skins"].get(index).ok_or_else(|| AssetError::invalid("skin index"))?;
    let joints = skin["joints"].as_array().ok_or_else(|| AssetError::invalid("skin has no joints"))?;
    if joints.is_empty() || joints.len() > 254 {
        return Err(AssetError::TooLarge("skin joints"));
    }
    let inverse = skin["inverseBindMatrices"]
        .as_u64()
        .map(|i| attribute::<16>(doc, i as usize))
        .transpose()?
        .unwrap_or(vec![Mat4::IDENTITY.to_cols_array(); joints.len()]);
    if inverse.len() != joints.len() {
        return Err(AssetError::invalid("inverse bind matrix count"));
    }
    let mut names = Vec::new();
    let mut inverse_bind = Vec::new();
    let mut alternate_bind = Vec::new();
    for (joint, inverse) in joints.iter().zip(inverse) {
        let index = integer(joint, "skin joint node")?;
        let node = nodes.get(index).ok_or_else(|| AssetError::invalid("skin joint node index"))?;
        let name = node["name"]
            .as_str()
            .ok_or_else(|| AssetError::invalid("skin joint needs a name"))?;
        let canonical = skeleton.find(name).map(|i| skeleton.joints[i].name.clone()).unwrap_or_else(|| {
            warnings.push(format!("Unknown joint: {name}"));
            name.into()
        });
        names.push(canonical);
        let matrix = Mat4::from_cols_array(&inverse);
        if !matrix.is_finite() || matrix.determinant().abs() < 0.000001 {
            return Err(AssetError::invalid("singular skin inverse bind"));
        }
        let inverse = basis * matrix * basis.inverse();
        inverse_bind.push(inverse);
        // Alternate binds preserve inverse rotation and carry the imported
        // local joint offset (LLDAELoader's mAlternateBindMatrix convention).
        let mut alternate = inverse;
        alternate.w_axis = basis.transform_vector3(transform(node)?.w_axis.truncate()).extend(1.0);
        alternate_bind.push(alternate);
    }
    Ok(Skin {
        names,
        inverse_bind,
        alternate_bind,
        bind_shape: Mat4::IDENTITY,
        pelvis_offset: 0.0,
    })
}
