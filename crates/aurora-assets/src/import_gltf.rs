//! Bounded glTF/GLB resources shared by PBR and mesh imports. Material
//! extraction follows LLMaterialEditor::loadMaterialFromFile / setFromGltfModel
//! (Firestorm indra/newview, originally LGPL 2.1).

use crate::{AssetError, PbrMaterial};
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Document {
    pub json: Value,
    pub buffers: Vec<Arc<[u8]>>,
    mesh: bool,
}
#[derive(Debug, Clone)]
pub enum Image {
    Existing(Uuid),
    Bytes(Vec<u8>),
}
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub value: PbrMaterial,
    /// Local image indices. Existing UUID references are already in `value`.
    pub images: [Option<usize>; 4],
    pub occlusion: Option<usize>,
}

impl Document {
    pub fn parse(data: &[u8], mut load: impl FnMut(&str) -> Result<Vec<u8>, AssetError>) -> Result<Self, AssetError> {
        Self::parse_inner(data, &mut load, false)
    }

    /// Mesh imports follow LLGLTFLoader's file acceptance; material-only
    /// imports keep their existing bounded resource policy.
    pub fn parse_mesh(data: &[u8], mut load: impl FnMut(&str) -> Result<Vec<u8>, AssetError>) -> Result<Self, AssetError> {
        Self::parse_inner(data, &mut load, true)
    }

    fn parse_inner(data: &[u8], load: &mut impl FnMut(&str) -> Result<Vec<u8>, AssetError>, mesh: bool) -> Result<Self, AssetError> {
        if !mesh && data.len() > MAX_BYTES {
            return Err(AssetError::TooLarge("glTF"));
        }
        let mut binary = None;
        let json = if data.starts_with(b"glTF") {
            let number = |offset| {
                data.get(offset..offset + 4)
                    .map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]) as usize)
                    .ok_or(AssetError::Truncated("GLB"))
            };
            if number(4)? != 2 || number(8)? != data.len() {
                return Err(AssetError::invalid("GLB version or length"));
            }
            let mut pos: usize = 12;
            let mut json = None;
            while pos < data.len() {
                let length = number(pos)?;
                let kind = number(pos + 4)?;
                let end = (pos + 8)
                    .checked_add(length)
                    .filter(|n| *n <= data.len())
                    .ok_or(AssetError::Truncated("GLB chunk"))?;
                if length % 4 != 0 {
                    return Err(AssetError::invalid("unaligned GLB chunk"));
                }
                match kind {
                    0x4e4f534a if pos == 12 && json.is_none() => json = Some(serde_json::from_slice::<Value>(&data[pos + 8..end])?),
                    0x004e4942 if json.is_some() && binary.is_none() => binary = Some(data[pos + 8..end].to_vec()),
                    0x4e4f534a | 0x004e4942 => return Err(AssetError::invalid("duplicate or unordered GLB chunk")),
                    _ => {}
                }
                pos = end;
            }
            json.ok_or_else(|| AssetError::invalid("GLB has no JSON chunk"))?
        } else {
            serde_json::from_slice::<Value>(data)?
        };
        if json["asset"]["version"].as_str() != Some("2.0") {
            return Err(AssetError::Unsupported("glTF version".into()));
        }
        let mut buffers = Vec::new();
        // A GLB's embedded buffer is already included in the file length.
        let mut total = data.len();
        let mut resources = HashMap::<String, Arc<[u8]>>::new();
        for (index, buffer) in json["buffers"].as_array().into_iter().flatten().enumerate() {
            let embedded = index == 0 && buffer.get("uri").is_none() && binary.is_some();
            if !mesh && buffers.len() >= 4096 {
                return Err(AssetError::TooLarge("glTF buffers"));
            }
            let bytes = if let Some(uri) = buffer["uri"].as_str() {
                if let Some(cached) = resources.get(uri) {
                    cached.clone()
                } else {
                    let bytes: Arc<[u8]> = resource(uri, load, mesh)?.into();
                    resources.insert(uri.into(), bytes.clone());
                    bytes
                }
            } else if index == 0 {
                binary.take().ok_or_else(|| AssetError::invalid("missing GLB buffer"))?.into()
            } else {
                return Err(AssetError::invalid("buffer has no URI"));
            };
            let expected = buffer["byteLength"].as_u64().ok_or_else(|| AssetError::invalid("buffer length"))?;
            if expected > bytes.len() as u64 || bytes.len() as u64 > expected + 3 {
                return Err(AssetError::invalid("buffer length mismatch"));
            }
            total = total
                .checked_add(if embedded { 0 } else { bytes.len() })
                .filter(|n| mesh || *n <= MAX_BYTES)
                .ok_or(AssetError::TooLarge("glTF resources"))?;
            buffers.push(bytes);
        }
        Ok(Self { json, buffers, mesh })
    }

    pub fn view(&self, index: usize) -> Result<&[u8], AssetError> {
        let view = self.json["bufferViews"]
            .get(index)
            .ok_or_else(|| AssetError::invalid("buffer view index"))?;
        let buffer = view["buffer"]
            .as_u64()
            .and_then(|i| self.buffers.get(i as usize))
            .ok_or_else(|| AssetError::invalid("buffer index"))?;
        let offset = view["byteOffset"].as_u64().unwrap_or(0);
        let length = view["byteLength"]
            .as_u64()
            .ok_or_else(|| AssetError::invalid("buffer view length"))?;
        let end = offset
            .checked_add(length)
            .filter(|n| *n <= buffer.len() as u64)
            .ok_or(AssetError::Truncated("buffer view"))?;
        Ok(&buffer[offset as usize..end as usize])
    }

    pub fn image(&self, index: usize, mut load: impl FnMut(&str) -> Result<Vec<u8>, AssetError>) -> Result<Image, AssetError> {
        let image = self.json["images"].get(index).ok_or_else(|| AssetError::invalid("image index"))?;
        if let Some(uri) = image["uri"].as_str() {
            if let Ok(id) = Uuid::parse_str(uri) {
                return Ok(Image::Existing(id));
            }
            return Ok(Image::Bytes(resource(uri, &mut load, self.mesh)?));
        }
        let index = image["bufferView"]
            .as_u64()
            .ok_or_else(|| AssetError::invalid("image has no URI or buffer view"))?;
        Ok(Image::Bytes(self.view(index as usize)?.to_vec()))
    }

    pub fn materials(&self) -> Result<Vec<Material>, AssetError> {
        let materials = self.json["materials"]
            .as_array()
            .ok_or_else(|| AssetError::invalid("glTF has no materials"))?;
        if !self.mesh && materials.len() > 4096 {
            return Err(AssetError::TooLarge("material count"));
        }
        materials
            .iter()
            .enumerate()
            .map(|(index, material)| {
                let mut images = [None; 4];
                for (slot, info) in [
                    &material["pbrMetallicRoughness"]["baseColorTexture"],
                    &material["normalTexture"],
                    &material["pbrMetallicRoughness"]["metallicRoughnessTexture"],
                    &material["emissiveTexture"],
                ]
                .into_iter()
                .enumerate()
                {
                    if info.is_null() {
                        continue;
                    }
                    let texture = info["index"]
                        .as_u64()
                        .and_then(|i| self.json["textures"].get(i as usize))
                        .ok_or_else(|| AssetError::invalid("material texture index"))?;
                    let source = texture["source"].as_u64().ok_or_else(|| AssetError::invalid("texture source"))? as usize;
                    let image = self.json["images"]
                        .get(source)
                        .ok_or_else(|| AssetError::invalid("texture image index"))?;
                    if image["uri"].as_str().and_then(|uri| Uuid::parse_str(uri).ok()).is_none() {
                        images[slot] = Some(source);
                    }
                }
                let name = material["name"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Material {}", index + 1));
                let value = crate::material::material_from_gltf_at(&self.json, index);
                // Validate f32 conversion and all transforms before using them.
                value.encode_asset()?;
                let occlusion = if material["occlusionTexture"].is_null() {
                    None
                } else {
                    let source = material["occlusionTexture"]["index"]
                        .as_u64()
                        .and_then(|i| self.json["textures"].get(i as usize))
                        .and_then(|t| t["source"].as_u64())
                        .filter(|i| self.json["images"].get(*i as usize).is_some())
                        .ok_or_else(|| AssetError::invalid("occlusion texture index"))?;
                    Some(source as usize)
                };
                Ok(Material {
                    name,
                    value,
                    images,
                    occlusion,
                })
            })
            .collect()
    }
}

fn resource(uri: &str, load: &mut impl FnMut(&str) -> Result<Vec<u8>, AssetError>, mesh: bool) -> Result<Vec<u8>, AssetError> {
    if let Some(data) = uri.strip_prefix("data:") {
        let (header, data) = data.split_once(',').ok_or_else(|| AssetError::invalid("data URI"))?;
        if !header.ends_with(";base64") || (!mesh && data.len() > MAX_BYTES) {
            return Err(AssetError::Unsupported("data URI encoding".into()));
        }
        return base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| AssetError::invalid("base64 resource"));
    }
    if uri.contains("://") || uri.starts_with("//") {
        return Err(AssetError::Unsupported("remote glTF resource".into()));
    }
    let bytes = load(uri)?;
    if !mesh && bytes.len() > MAX_BYTES {
        return Err(AssetError::TooLarge("glTF resource"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_materials_share_image_indices_and_keep_existing_uuid() {
        let data = br#"{"asset":{"version":"2.0"},"images":[{"uri":"local.png"},{"uri":"11111111-2222-3333-4444-555555555555"}],"textures":[{"source":0},{"source":1}],"materials":[{"name":"A","pbrMetallicRoughness":{"baseColorTexture":{"index":0}}},{"name":"B","normalTexture":{"index":0},"emissiveTexture":{"index":1},"alphaMode":"MASK"}]}"#;
        let document = Document::parse(data, |_| panic!("no buffer reads")).expect("glTF");
        let materials = document.materials().expect("materials");
        assert_eq!(materials[0].images[0], Some(0));
        assert_eq!(materials[1].images[1], Some(0));
        assert!(materials[1].images[3].is_none());
        assert!(materials[1].value.emissive_texture.is_some());
        let reread = crate::parse_material_asset(&materials[1].value.encode_asset().expect("encode")).expect("decode");
        assert_eq!(reread, materials[1].value);
    }
    #[test]
    fn rejects_missing_views_truncated_glb_and_remote_resources() {
        assert!(Document::parse(b"glTF", |_| Ok(vec![])).is_err());
        let doc = Document::parse(br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"data:application/octet-stream;base64,AAAAAA=="}],"bufferViews":[{"buffer":0,"byteLength":8}]}"#, |_| panic!("embedded")).expect("document");
        assert!(doc.view(0).is_err());
        assert!(
            Document::parse(
                br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"https://example.test/file"}]}"#,
                |_| panic!("no network")
            )
            .is_err()
        );
    }
}
