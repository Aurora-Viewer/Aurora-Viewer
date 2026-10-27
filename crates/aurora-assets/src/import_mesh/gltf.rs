//! glTF accessors and mesh scene hierarchy. All offsets/counts/strides are
//! checked before allocation or reads; sparse and normalized attributes are
//! supported. Geometry is converted from glTF Y-up to SL Z-up once.

use super::*;
use crate::import_gltf::Document;
use glam::Quat;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(super) fn integer(value: &Value, field: &'static str) -> Result<usize, AssetError> {
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
    if count > u32::MAX as usize
        || count
            .checked_mul(N * std::mem::size_of::<f32>())
            .is_none_or(|n| n > isize::MAX as usize)
    {
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
    if let Some(view) = accessor["bufferView"].as_u64() {
        let data = doc.view(view as usize)?;
        let stride = doc.json["bufferViews"][view as usize]["byteStride"]
            .as_u64()
            .unwrap_or((N * width) as u64) as usize;
        let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
        let end = count
            .saturating_sub(1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(offset))
            .and_then(|n| n.checked_add(N * width));
        if count > 0 && end.is_none_or(|n| n > data.len()) {
            return Err(AssetError::Truncated("accessor values"));
        }
    }
    let mut values = super::allocate(count, [0.0; N], "glTF accessor memory")?;
    let read = |data: &[u8], offset: usize, stride: usize, values: &mut [[f32; N]]| -> Result<(), AssetError> {
        if stride < N * width || !stride.is_multiple_of(width) {
            return Err(AssetError::invalid("accessor stride"));
        }
        let end = values
            .len()
            .saturating_sub(1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(offset))
            .and_then(|n| n.checked_add(N * width))
            .ok_or(AssetError::Truncated("accessor values"))?;
        if !values.is_empty() && end > data.len() {
            return Err(AssetError::Truncated("accessor values"));
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
        let mut patches = super::allocate(count_sparse, [0.0; N], "sparse accessor memory")?;
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
            let index = unsigned(
                index_data
                    .get(start..start + index_width)
                    .ok_or(AssetError::Truncated("sparse indices"))?,
            ) as usize;
            if index >= count || previous.is_some_and(|previous| index <= previous) {
                return Err(AssetError::invalid("sparse indices must be ordered and in range"));
            }
            previous = Some(index);
            values[index] = patch;
        }
    }
    Ok(values)
}

fn unsigned(bytes: &[u8]) -> u32 {
    match bytes.len() {
        1 => u32::from(bytes[0]),
        2 => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        _ => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    }
}

fn source_bytes(data: &[u8], start: usize, length: usize) -> Result<&[u8], AssetError> {
    let end = start.checked_add(length).ok_or(AssetError::Truncated("glTF accessor bytes"))?;
    data.get(start..end).ok_or(AssetError::Truncated("glTF accessor bytes"))
}

pub(super) fn indices(doc: &Document, index: usize) -> Result<Vec<u32>, AssetError> {
    let accessor = &doc.json["accessors"][index];
    let ty = accessor["componentType"].as_u64();
    if !matches!(ty, Some(5121 | 5123 | 5125)) || accessor["normalized"].as_bool().unwrap_or(false) {
        return Err(AssetError::invalid("indices must be unsigned integers"));
    }
    if ty != Some(5125) {
        return Ok(attribute::<1>(doc, index)?.into_iter().map(|v| v[0] as u32).collect());
    }
    if accessor["type"].as_str() != Some("SCALAR") {
        return Err(AssetError::invalid("index accessor shape"));
    }
    let count = integer(&accessor["count"], "index accessor count")?;
    let source = if let Some(view) = accessor["bufferView"].as_u64() {
        let bytes = doc.view(view as usize)?;
        let stride = doc.json["bufferViews"][view as usize]["byteStride"].as_u64().unwrap_or(4) as usize;
        let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
        let end = count
            .saturating_sub(1)
            .checked_mul(stride)
            .and_then(|n| n.checked_add(offset))
            .and_then(|n| n.checked_add(4));
        if stride < 4 || !stride.is_multiple_of(4) || (count > 0 && end.is_none_or(|end| end > bytes.len())) {
            return Err(AssetError::Truncated("index accessor"));
        }
        Some((bytes, offset, stride))
    } else {
        None
    };
    let mut values = super::allocate(count, 0u32, "index accessor memory")?;
    if let Some((bytes, offset, stride)) = source {
        for (row, value) in values.iter_mut().enumerate() {
            *value = unsigned(source_bytes(bytes, offset + row * stride, 4)?);
        }
    } else if accessor.get("sparse").is_none() {
        return Err(AssetError::invalid("index accessor has no data"));
    }
    if let Some(sparse) = accessor.get("sparse") {
        let count_sparse = integer(&sparse["count"], "sparse count")?;
        if count_sparse > count {
            return Err(AssetError::invalid("sparse count exceeds accessor"));
        }
        let patches = &sparse["indices"];
        let width = match patches["componentType"].as_u64() {
            Some(5121) => 1,
            Some(5123) => 2,
            Some(5125) => 4,
            _ => return Err(AssetError::invalid("sparse index type")),
        };
        let patch_indices = doc.view(integer(&patches["bufferView"], "sparse index view")?)?;
        let index_offset = patches["byteOffset"].as_u64().unwrap_or(0) as usize;
        let patch_values = doc.view(integer(&sparse["values"]["bufferView"], "sparse value view")?)?;
        let value_offset = sparse["values"]["byteOffset"].as_u64().unwrap_or(0) as usize;
        let mut previous = None;
        for row in 0..count_sparse {
            let start = row
                .checked_mul(width)
                .and_then(|n| n.checked_add(index_offset))
                .ok_or(AssetError::Truncated("sparse indices"))?;
            let index = unsigned(source_bytes(patch_indices, start, width)?) as usize;
            if index >= count || previous.is_some_and(|p| index <= p) {
                return Err(AssetError::invalid("sparse indices must be ordered and in range"));
            }
            previous = Some(index);
            let start = row
                .checked_mul(4)
                .and_then(|n| n.checked_add(value_offset))
                .ok_or(AssetError::Truncated("sparse values"))?;
            values[index] = unsigned(source_bytes(patch_values, start, 4)?);
        }
    }
    Ok(values)
}

pub(super) fn transform(node: &Value) -> Result<Mat4, AssetError> {
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
    if s.contains(&0.0) {
        return Err(AssetError::invalid("zero node scale"));
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
    parse_with_options(doc, skeleton, DEFAULT_MODEL_LIMIT)
}

pub fn parse_with_options(doc: &Document, skeleton: &crate::Skeleton, model_limit: u32) -> Result<Scene, AssetError> {
    let nodes = doc.json["nodes"]
        .as_array()
        .ok_or_else(|| AssetError::invalid("glTF has no nodes"))?;
    let mut instances = Vec::new();
    let scene_index = doc.json["scene"].as_u64().unwrap_or(0) as usize;
    let roots = doc.json["scenes"][scene_index]["nodes"]
        .as_array()
        .ok_or_else(|| AssetError::invalid("glTF has no selected scene"))?;
    let mut pending = roots
        .iter()
        .rev()
        .map(|r| integer(r, "root node index"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut visited = HashSet::new();
    while let Some(index) = pending.pop() {
        let node = nodes.get(index).ok_or_else(|| AssetError::invalid("glTF node index"))?;
        if !visited.insert(index) {
            continue;
        }
        if node.get("mesh").is_some() {
            instances.push(index);
        }
        for child in node["children"].as_array().into_iter().flatten().rev() {
            pending.push(integer(child, "child node index")?);
        }
    }
    if instances.is_empty() {
        return Err(AssetError::invalid("mesh scene has no instances"));
    }
    let mut required: Vec<_> = visited.into_iter().collect();
    for skin in doc.json["skins"].as_array().into_iter().flatten() {
        for joint in skin["joints"].as_array().into_iter().flatten() {
            required.push(integer(joint, "skin joint index")?);
        }
    }
    let globals = super::gltf_skin::globals_for(nodes, &required)?;
    let basis = super::gltf_skin::basis(doc, nodes, &globals, skeleton)?;
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
    let mesh_count = doc.json["meshes"].as_array().map_or(1, |m| m.len().max(1));
    let face_limit = (model_limit as usize / mesh_count + 1).saturating_mul(8);
    let mut mesh_models = HashMap::<(usize, Option<usize>, usize), Vec<usize>>::new();
    let mut labels = HashMap::<String, usize>::new();
    for node_index in instances {
        let node = &nodes[node_index];
        let mesh_index = integer(&node["mesh"], "mesh index")?;
        let skin_index = node["skin"].as_u64().map(|n| n as usize);
        let mirrored = globals[node_index].determinant() < 0.0;
        // processNodeHierarchy keeps node identities even when they share a
        // glTF mesh. Their authored lower LODs may differ independently.
        let key = (mesh_index, skin_index, node_index);
        let reflection = if mirrored {
            Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))
        } else {
            Mat4::IDENTITY
        };
        let vertex_transform = basis * if skin_index.is_some() { globals[node_index] } else { reflection };
        let normal_transform = vertex_transform.inverse().transpose();
        if let std::collections::hash_map::Entry::Vacant(e) = mesh_models.entry(key) {
            let mesh = doc.json["meshes"]
                .get(mesh_index)
                .ok_or_else(|| AssetError::invalid("mesh index"))?;
            let mut name = node["name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .or_else(|| mesh["name"].as_str().filter(|n| !n.is_empty()))
                .map_or_else(|| format!("node_{node_index}"), str::to_owned);
            let copies = labels.entry(name.clone()).or_default();
            if *copies > 0 {
                name.push_str(&format!("_copy_{copies}"));
            }
            *copies += 1;
            let mut skin = if let Some(index) = skin_index {
                Some(super::gltf_skin::read(
                    doc,
                    index,
                    nodes,
                    &globals,
                    basis,
                    skeleton,
                    &mut scene.warnings,
                )?)
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
                            || index >= u16::MAX as f32
                            || index.fract() != 0.0
                            || skin
                                .as_ref()
                                .is_some_and(|s| influence[slot] > 0.0 && index as usize >= s.names.len())
                        {
                            return Err(AssetError::invalid("joint index is outside skin"));
                        }
                        ids[slot] = index as u16;
                    }
                    let sum: f32 = influence.iter().sum();
                    if sum > 0.0 {
                        for w in &mut influence {
                            *w /= sum;
                        }
                    }
                    // populateModelFromMesh uses the source +Z default when
                    // the exporter omitted normals; explicit regeneration
                    // remains a separate importer option.
                    let normal = normals.as_ref().map(|n| n[i]).unwrap_or([0.0, 0.0, 1.0]);
                    let uv = uvs.as_ref().map(|u| u[i]).unwrap_or([0.0; 2]);
                    vertices.push(Vertex {
                        position: vertex_transform.transform_point3(Vec3::from_array(position)).to_array(),
                        normal: normal_transform
                            .transform_vector3(Vec3::from_array(normal))
                            .normalize_or_zero()
                            .to_array(),
                        uv: [uv[0], 1.0 - uv[1]],
                        joints: ids,
                        weights: influence,
                    });
                }
                let mut indices = if let Some(index) = primitive["indices"].as_u64() {
                    indices(doc, index as usize)?
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
                if mirrored {
                    for triangle in indices.as_chunks_mut::<3>().0 {
                        triangle.swap(1, 2);
                    }
                }
                faces.extend(split_face(vertices, indices, material)?);
            }
            if let Some(skin) = &mut skin {
                super::skin::prune_gltf(&mut faces, skin, skeleton)?;
            }
            faces.sort_by(|a, b| scene.materials[a.material].name.cmp(&scene.materials[b.material].name));
            if faces.len() > face_limit {
                scene.warnings.push(format!("Generated model limit reached for mesh {mesh_index}"));
                faces.truncate(face_limit);
            }
            let mut models = Vec::new();
            let (center, size) = normalization(&faces)?;
            if skin.is_some() {
                super::skin::share_weights(&mut faces, center, size);
                super::skin::complete_weights(&mut faces, size);
            }
            for (submodel, group) in faces.chunks(8).enumerate() {
                let mut model = Model {
                    name: name.clone(),
                    submodel: submodel as u32,
                    faces: group.to_vec(),
                    skin: skin.clone(),
                    center: Vec3::ZERO,
                    size: Vec3::ONE,
                };
                model.normalize_to(center, size)?;
                scene.normalized_bounds.insert(scene.models.len(), model.normalized_bounds());
                scene.material_bindings.insert(
                    scene.models.len(),
                    model.faces.iter().map(|f| scene.materials[f.material].name.clone()).collect(),
                );
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
                transform: if skin_index.is_some() {
                    normalized_to_source
                } else {
                    basis * globals[node_index] * reflection * basis.inverse() * normalized_to_source
                },
                name: node["name"].as_str().unwrap_or(&source.name).into(),
                materials: source.faces.iter().map(|face| face.material).collect(),
            });
        }
    }
    if scene.models.is_empty() {
        return Err(AssetError::TooLarge("split mesh instances"));
    }
    Ok(scene)
}
