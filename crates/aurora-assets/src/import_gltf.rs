//! Validated glTF/GLB resources shared by PBR and mesh imports. Material
//! extraction follows LLMaterialEditor::loadMaterialFromFile / setFromGltfModel
//! (Firestorm indra/newview, originally LGPL 2.1).

use crate::{AssetError, PbrMaterial};
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// TinyGLTF::GetMaxExternalFileSize, used by LLTinyGLTFHelper::loadModel.
/// This is per external resource, not the main document or its decoded pixels.
pub const MAX_EXTERNAL_BYTES: u64 = i32::MAX as u64;

pub fn check_external_size(bytes: u64) -> Result<(), AssetError> {
    if bytes > MAX_EXTERNAL_BYTES {
        return Err(AssetError::GltfResourceSize {
            bytes,
            limit: MAX_EXTERNAL_BYTES,
        });
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Document {
    pub json: Value,
    pub buffers: Vec<Arc<[u8]>>,
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
        Self::parse_inner(data, &mut load)
    }

    /// Mesh and material imports share the Legacy Viewer's file acceptance.
    pub fn parse_mesh(data: &[u8], mut load: impl FnMut(&str) -> Result<Vec<u8>, AssetError>) -> Result<Self, AssetError> {
        Self::parse_inner(data, &mut load)
    }

    fn parse_inner(data: &[u8], load: &mut impl FnMut(&str) -> Result<Vec<u8>, AssetError>) -> Result<Self, AssetError> {
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
        let mut resources = HashMap::<String, Arc<[u8]>>::new();
        for (index, buffer) in json["buffers"].as_array().into_iter().flatten().enumerate() {
            let bytes = if let Some(uri) = buffer["uri"].as_str() {
                if let Some(cached) = resources.get(uri) {
                    cached.clone()
                } else {
                    let bytes: Arc<[u8]> = resource(uri, load)?.into();
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
            buffers.push(bytes);
        }
        Ok(Self { json, buffers })
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
            return Ok(Image::Bytes(resource(uri, &mut load)?));
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

fn resource(uri: &str, load: &mut impl FnMut(&str) -> Result<Vec<u8>, AssetError>) -> Result<Vec<u8>, AssetError> {
    if let Some(data) = uri.strip_prefix("data:") {
        let (header, data) = data.split_once(',').ok_or_else(|| AssetError::invalid("data URI"))?;
        if !header.ends_with(";base64") {
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
    check_external_size(bytes.len() as u64)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_glb_can_exceed_256_mib() {
        let mut json = br#"{"asset":{"version":"2.0"},"materials":[{"name":"Large document"}]}"#.to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        // Unknown chunks are legal GLB padding and do not allocate decoded
        // images. Exercise the old whole-file cap without a huge fixture.
        let padding = 256 * 1024 * 1024;
        let length = 12 + 8 + json.len() + 8 + padding;
        let mut glb = Vec::with_capacity(length);
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(length as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4e4f534au32.to_le_bytes());
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(padding as u32).to_le_bytes());
        glb.extend_from_slice(&0x12345678u32.to_le_bytes());
        glb.resize(length, 0);
        let doc = Document::parse(&glb, |_| panic!("no external resource")).expect("large GLB");
        assert_eq!(doc.materials().unwrap()[0].name, "Large document");
    }

    #[test]
    fn material_documents_keep_all_materials_and_shared_buffers_above_4096() {
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "materials": vec![serde_json::json!({}); 4097],
            "buffers": vec![serde_json::json!({"byteLength": 4, "uri": "shared.bin"}); 4097],
        });
        let mut reads = 0;
        let doc = Document::parse(&serde_json::to_vec(&json).unwrap(), |_| {
            reads += 1;
            Ok(vec![0; 4])
        })
        .expect("all materials and buffers");
        assert_eq!(doc.materials().unwrap().len(), 4097);
        assert_eq!(doc.buffers.len(), 4097);
        assert_eq!(reads, 1);
        assert!(Arc::ptr_eq(&doc.buffers[0], &doc.buffers[4096]));
    }

    #[test]
    fn external_resource_limit_matches_tinygltf_and_reports_both_sizes() {
        assert!(check_external_size(256 * 1024 * 1024 + 1).is_ok());
        assert!(check_external_size(i32::MAX as u64).is_ok());
        let error = check_external_size(i32::MAX as u64 + 1).unwrap_err();
        assert!(matches!(
            error,
            AssetError::GltfResourceSize {
                bytes: 2147483648,
                limit: 2147483647
            }
        ));
        assert!(error.to_string().contains("2147483648"));
        assert!(error.to_string().contains("2147483647"));
    }

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
