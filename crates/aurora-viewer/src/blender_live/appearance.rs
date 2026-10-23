//! Embedded PBR images are decoded, bounded and staged on the socket worker.
use aurora_assets::material::{AlphaMode, PbrMaterial, TextureTransform};
use aurora_render::{StagedTexture, StagingPool};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;
pub const STAGING_BUSY: &str = "AURORA_BLENDER_STAGING_BUSY";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: String,
}

#[derive(Clone)]
pub struct PreparedImage {
    pub id: Uuid,
    pub pixels: Arc<aurora_assets::DecodedImage>,
    pub staged: Option<Arc<StagedTexture>>,
    pub alpha: crate::scene::jobs::AlphaKind,
    pub has_alpha: bool,
}

fn transforms() -> [[f32; 5]; 4] {
    [[0.0, 0.0, 1.0, 1.0, 0.0]; 4]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub double_sided: bool,
    pub alpha_mode: String,
    pub textures: [Option<usize>; 4],
    #[serde(default = "transforms")]
    pub transforms: [[f32; 5]; 4],
}

fn invalid() -> String {
    aurora_i18n::tr!("blender-live-invalid-material").into()
}

pub fn images(images: Vec<Image>, pool: Option<&StagingPool>, cache: &HashMap<Uuid, PreparedImage>) -> Result<Vec<PreparedImage>, String> {
    if images.len() > 16 {
        return Err(invalid());
    }
    let mut total = 0;
    images
        .into_iter()
        .map(|image| {
            if !(1..=1024).contains(&image.width) || !(1..=1024).contains(&image.height) {
                return Err(invalid());
            }
            let size = image.width as usize * image.height as usize * 4;
            total += size;
            if total > 16 * 1024 * 1024 || image.rgba.len() != size.div_ceil(3) * 4 {
                return Err(invalid());
            }
            let rgba = STANDARD.decode(image.rgba).map_err(|_| invalid())?;
            if rgba.len() != size {
                return Err(invalid());
            }
            let mut hash = Sha256::new();
            hash.update(b"Aurora Blender Live image v1");
            hash.update(image.width.to_le_bytes());
            hash.update(image.height.to_le_bytes());
            hash.update(&rgba);
            let digest = hash.finalize();
            let mut id = [0; 16];
            id.copy_from_slice(&digest[..16]);
            let id = Uuid::from_bytes(id);
            if let Some(cached) = cache.get(&id)
                && (pool.is_none() || cached.staged.is_some())
            {
                return Ok(cached.clone());
            }
            let alpha = crate::scene::jobs::classify_alpha(&rgba);
            let has_alpha = rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255);
            let pixels = Arc::new(aurora_assets::DecodedImage {
                width: image.width,
                height: image.height,
                components: 4,
                data: rgba,
                discard: 0,
            });
            let levels = aurora_render::build_mips(image.width, image.height, pixels.data.clone(), 12);
            let staged = if let Some(pool) = pool {
                let refs: Vec<_> = levels.iter().map(|(w, h, bytes)| (*w, *h, bytes.as_slice())).collect();
                Some(Arc::new(StagedTexture::new(pool, &refs).ok_or_else(|| STAGING_BUSY.to_owned())?))
            } else {
                None
            };
            Ok(PreparedImage {
                id,
                pixels,
                staged,
                alpha,
                has_alpha,
            })
        })
        .collect()
}

impl Material {
    pub fn prepare(self, color: [f32; 4], images: &[PreparedImage]) -> Result<PbrMaterial, String> {
        if [self.metallic, self.roughness]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.emissive.iter().any(|v| !v.is_finite() || !(0.0..=64.0).contains(v))
            || self.transforms.iter().flatten().any(|v| !v.is_finite() || v.abs() > 1e6)
            || self.textures.iter().flatten().any(|index| *index >= images.len())
        {
            return Err(invalid());
        }
        let ids = self.textures.map(|index| index.map(|index| images[index].id));
        let alpha_mode = match self.alpha_mode.as_str() {
            "OPAQUE" => AlphaMode::Opaque,
            "BLEND" => AlphaMode::Blend,
            "MASK" => AlphaMode::Mask,
            _ => return Err(invalid()),
        };
        Ok(PbrMaterial {
            base_color_factor: color,
            base_color_texture: ids[0],
            normal_texture: ids[1],
            metallic_roughness_texture: ids[2],
            emissive_texture: ids[3],
            metallic_factor: self.metallic,
            roughness_factor: self.roughness,
            emissive_factor: self.emissive,
            alpha_mode,
            double_sided: self.double_sided,
            transforms: self.transforms.map(|v| TextureTransform {
                offset: [v[0], v[1]],
                scale: [v[2], v[3]],
                rotation: v[4],
            }),
            ..Default::default()
        })
    }
}
