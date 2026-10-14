//! Photos to the inventory (LLPanelSnapshotInventory and
//! LLSnapshotLivePreview::saveTexture, indra/newview, originally LGPL 2.1):
//! the photo is taken at the chosen size, scaled to powers of two of at
//! most 2048 (LLImageRaw::biasedScaleToPowerOfTwo with MAX_TEXTURE_SIZE),
//! encoded in JPEG 2000 and uploaded as a texture of inventory type
//! "snapshot" into the Photo Album, named after the place it was taken.

use super::{Aspect, MIN_SIDE};
use serde::{Deserialize, Serialize};

/// MAX_TEXTURE_SIZE: the largest side of an uploaded texture.
pub const MAX_TEXTURE_SIDE: u32 = 2048;
/// FT_SNAPSHOT_CATEGORY, the Photo Album.
pub const PHOTO_ALBUM: i32 = 15;
/// Inventory names and descriptions the server keeps (DB_INV_ITEM_NAME_STR_LEN,
/// DB_INV_ITEM_DESC_STR_LEN).
const NAME_BYTES: usize = 63;
const DESCRIPTION_BYTES: usize = 127;

/// The sizes of panel_snapshot_inventory.xml's combo: Small (128) to Huge
/// (2048), or the window's (LastSnapshotToInventoryResolution, Large by
/// default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureSize {
    S128,
    S256,
    S512,
    S1024,
    S2048,
    Window,
}

impl TextureSize {
    pub const ALL: [TextureSize; 6] = [
        TextureSize::S128,
        TextureSize::S256,
        TextureSize::S512,
        TextureSize::S1024,
        TextureSize::S2048,
        TextureSize::Window,
    ];

    /// The long side of a preset.
    pub fn side(self) -> Option<u32> {
        match self {
            TextureSize::S128 => Some(128),
            TextureSize::S256 => Some(256),
            TextureSize::S512 => Some(512),
            TextureSize::S1024 => Some(1024),
            TextureSize::S2048 => Some(2048),
            TextureSize::Window => None,
        }
    }
}

/// The size the photo is taken at: the preset as its long side with the
/// chosen proportions, or the window's long side (Firestorm's "Current
/// Window"), never above MAX_TEXTURE_SIDE since it is scaled down anyway.
pub fn capture_size(aspect: Aspect, size: TextureSize, window: (u32, u32)) -> (u32, u32) {
    let window = (window.0.max(1), window.1.max(1));
    let ratio = aspect.ratio(window);
    let long = size.side().unwrap_or(window.0.max(window.1)).min(MAX_TEXTURE_SIDE);
    let (w, h) = if ratio >= 1.0 {
        (long, (long as f32 / ratio).round() as u32)
    } else {
        ((long as f32 * ratio).round() as u32, long)
    };
    (w.max(MIN_SIDE), h.max(MIN_SIDE))
}

/// The texture a photo of `capture` becomes: each side to a power of two,
/// rounded down unless it is over 1.75 times the lower one.
pub fn texture_size(capture: (u32, u32)) -> (u32, u32) {
    use aurora_assets::import_texture::biased_dimension;
    (
        biased_dimension(capture.0, MAX_TEXTURE_SIDE),
        biased_dimension(capture.1, MAX_TEXTURE_SIDE),
    )
}

/// LLAgentUI::buildLocationString(LOCATION_FORMAT_FULL):
/// "Parcel, Region (x, y, z) - Maturity", without the parcel when it has no
/// name, without the maturity when it is unknown.
pub fn location(parcel: &str, region: &str, position: glam::Vec3, maturity: &str) -> String {
    let mut s = String::new();
    if !parcel.trim().is_empty() {
        s.push_str(parcel.trim());
        s.push_str(", ");
    }
    s.push_str(&format!(
        "{} ({}, {}, {})",
        region.trim(),
        position.x.round() as i32,
        position.y.round() as i32,
        position.z.round() as i32
    ));
    if !maturity.is_empty() {
        s.push_str(" - ");
        s.push_str(maturity);
    }
    s
}

/// "Snapshot : <place>", as saveTexture names it (in English: the item is
/// seen by others in their own language's viewer).
pub fn item_name(location: &str) -> String {
    clip(&format!("Snapshot : {location}"), NAME_BYTES)
}

/// "Taken by <who> at <place>".
pub fn item_description(who: &str, location: &str) -> String {
    clip(&format!("Taken by {who} at {location}"), DESCRIPTION_BYTES)
}

/// At most `bytes` bytes, cut on a character.
fn clip(s: &str, bytes: usize) -> String {
    let mut end = s.len().min(bytes);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// The photo (RGBA, rows top to bottom) as the texture to upload: scaled to
/// `texture`, its alpha dropped, encoded lossy like saveTexture
/// (LLImageJ2C::encode with no reversible option). Runs on a worker.
pub fn encode(width: u32, height: u32, rgba: &[u8], texture: (u32, u32)) -> Result<Vec<u8>, String> {
    use aurora_assets::{DecodedImage, import_texture};
    let pixels = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or("photo pixels")?;
    let scaled = if (width, height) == texture {
        pixels
    } else {
        image::imageops::resize(&pixels, texture.0, texture.1, image::imageops::FilterType::Triangle)
    };
    let data: Vec<u8> = scaled.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let image = DecodedImage {
        width: texture.0,
        height: texture.1,
        components: 3,
        data,
        discard: 0,
    };
    let options = import_texture::Options {
        max_width: MAX_TEXTURE_SIDE,
        max_height: MAX_TEXTURE_SIDE,
        lossless_small: false,
        strip_alpha: true,
    };
    import_texture::encode(&image, options).map_err(|e| e.to_string())
}
