//! The picture's shape: crop, quarter turns, flips and a new size, with the
//! `image` crate (Lanczos3 for the size, as the inventory's thumbnails).

use super::{Edit, Picture};
use image::{RgbaImage, imageops};

/// The largest side a resize may give (the GPU's and the files' limit).
pub const MAX_SIDE: u32 = 16_384;

pub fn apply(pic: &Picture, edit: Edit) -> Picture {
    let Some(img) = RgbaImage::from_raw(pic.w, pic.h, pic.rgba.to_vec()) else {
        return pic.clone();
    };
    let out = match edit {
        Edit::Crop(crop) => {
            let (x, y, w, h) = crop.pixels(pic.w, pic.h);
            imageops::crop_imm(&img, x, y, w, h).to_image()
        }
        Edit::RotateRight => imageops::rotate90(&img),
        Edit::RotateLeft => imageops::rotate270(&img),
        Edit::FlipHorizontal => imageops::flip_horizontal(&img),
        Edit::FlipVertical => imageops::flip_vertical(&img),
        Edit::Resize { sx, sy } => {
            let (w, h) = resized(pic.w, pic.h, sx, sy);
            if (w, h) == (pic.w, pic.h) {
                return pic.clone();
            }
            imageops::resize(&img, w, h, imageops::FilterType::Lanczos3)
        }
    };
    let (w, h) = out.dimensions();
    Picture::new(w, h, out.into_raw())
}

/// The size a resize by `sx`×`sy` gives a `w`×`h` picture: at least one
/// pixel, at most `MAX_SIDE`.
pub fn resized(w: u32, h: u32, sx: f32, sy: f32) -> (u32, u32) {
    let side = |n: u32, s: f32| {
        let s = if s.is_finite() && s > 0.0 { s } else { 1.0 };
        ((n as f32 * s).round() as u32).clamp(1, MAX_SIDE)
    };
    (side(w, sx), side(h, sy))
}

/// The size the picture has once `edits` are played on a `w`×`h` original,
/// without touching its pixels (the window shows the full size it will save).
pub fn size_after(mut w: u32, mut h: u32, edits: &[Edit]) -> (u32, u32) {
    for edit in edits {
        (w, h) = match *edit {
            Edit::Crop(crop) => {
                let (_, _, cw, ch) = crop.pixels(w, h);
                (cw, ch)
            }
            Edit::RotateRight | Edit::RotateLeft => (h, w),
            Edit::FlipHorizontal | Edit::FlipVertical => (w, h),
            Edit::Resize { sx, sy } => resized(w, h, sx, sy),
        };
    }
    (w, h)
}
