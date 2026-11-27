//! Opening a picture of the computer and choosing how it is saved. The
//! decoding refuses pictures too large to work with (a header that claims a
//! huge size is not trusted); the saving itself is the photo window's
//! (`snapshot::save`), a file written beside its name then renamed.

use super::Picture;
use crate::snapshot::FileFormat;
use std::path::{Path, PathBuf};

/// The largest side a picture may have to be opened.
pub const MAX_SIDE: u32 = 16_384;
/// What the decoder may allocate (a 16k × 8k picture, with room).
const MAX_ALLOC: u64 = 1 << 30;

/// The file types the window opens.
pub const OPEN_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "tga", "gif", "webp"];

pub fn load(path: &Path) -> Result<Picture, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return Err("empty picture".into());
    }
    Ok(Picture::new(w, h, rgba.into_raw()))
}

/// A copy of `pic` no larger than `max_side` (the one the window shows and
/// edits live), or the picture itself when it is small enough.
pub fn working_copy(pic: &Picture, max_side: u32) -> Picture {
    if pic.w.max(pic.h) <= max_side {
        return pic.clone();
    }
    let (w, h, rgba) = crate::snapshot::save::downscale(pic.w, pic.h, &pic.rgba, max_side);
    Picture::new(w, h, rgba)
}

/// The file type a saved name asks for: JPEG for .jpg/.jpeg, BMP for .bmp,
/// PNG otherwise (and the name then ends in .png).
pub fn save_target(path: &Path) -> (PathBuf, FileFormat) {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("jpg" | "jpeg") => (path.to_path_buf(), FileFormat::Jpeg),
        Some("bmp") => (path.to_path_buf(), FileFormat::Bmp),
        Some("png") => (path.to_path_buf(), FileFormat::Png),
        _ => (path.with_extension("png"), FileFormat::Png),
    }
}

/// The name offered to save an edit of `original`: `<name>-edited.png`
/// beside it.
pub fn suggested_name(original: &Path) -> String {
    let stem = original.file_stem().and_then(|s| s.to_str()).unwrap_or("picture");
    format!("{stem}-edited.png")
}

/// Whether two paths name the same file (the original would be overwritten).
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
