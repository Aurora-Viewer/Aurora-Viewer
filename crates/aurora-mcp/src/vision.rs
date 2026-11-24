//! Off-thread Viewport PNG Encoding, Downscaling, Orbit Manifests, and Visual Image Diffing.

use crate::asset_inspect::validate_local_file_path;
use crate::bridge::{CaptureBufferType, McpError, RawCapturedFrame};
use base64::Engine;
use image::{ImageBuffer, Rgba, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use uuid::Uuid;

const ALLOWED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tga", "bmp", "webp", "gif"];
const MAX_COMPARE_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CAPTURE_FILES: usize = 200;
const MAX_CAPTURE_DIR_BYTES: u64 = 256 * 1024 * 1024;
const PRUNE_EVERY_N_WRITES: u64 = 16;

static CAPTURE_WRITE_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedCapture {
    pub path: String,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub base64_png: String,
    pub buffer_type: String,
    pub near_clip: f32,
    pub far_clip: f32,
}

pub fn capture_output_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("aurora-mcp-captures");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Periodically prunes oldest files in `aurora-mcp-captures` to bound disk usage.
pub(crate) fn maybe_prune_capture_dir(dir: &Path) {
    let count = CAPTURE_WRITE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if count.is_multiple_of(PRUNE_EVERY_N_WRITES) {
        prune_capture_dir_in(dir, MAX_CAPTURE_FILES, MAX_CAPTURE_DIR_BYTES);
    }
}

fn prune_capture_dir_in(dir: &Path, max_files: usize, max_bytes: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    let mut total_bytes: u64 = 0;
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let len = meta.len();
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        total_bytes = total_bytes.saturating_add(len);
        files.push((entry.path(), mtime, len));
    }
    if files.len() <= max_files && total_bytes <= max_bytes {
        return;
    }
    files.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut remaining_files = files.len();
    for (path, _, len) in files {
        if remaining_files <= max_files && total_bytes <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            remaining_files = remaining_files.saturating_sub(1);
            total_bytes = total_bytes.saturating_sub(len);
        }
    }
}

/// Read a `capture_orbit` manifest back from disk (`capture_manifest_read`).
///
/// Only `manifest-*.json` files inside [`capture_output_dir`] are read: a bare
/// manifest id names `manifest-<id>.json` there, and any other path must
/// resolve (symlinks followed) into that directory. Arbitrary files on disk
/// are refused.
pub fn read_capture_manifest(key: &str) -> Result<serde_json::Value, McpError> {
    const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
    let key = key.trim();
    if key.is_empty() {
        return Err(McpError::InvalidParams("manifest_path or manifest_id is required".into()));
    }
    let dir = capture_output_dir();
    let candidate = if key.contains(['/', '\\']) {
        PathBuf::from(key)
    } else if key.starts_with("manifest-") && key.ends_with(".json") {
        dir.join(key)
    } else {
        dir.join(format!("manifest-{key}.json"))
    };
    let refuse = || McpError::PermissionDenied(format!("'{key}' is not a capture manifest in {}", dir.display()));
    let canon_dir = dir.canonicalize().map_err(|_| refuse())?;
    let canon = candidate
        .canonicalize()
        .map_err(|e| McpError::InvalidParams(format!("cannot read manifest '{key}': {e}")))?;
    let name_ok = canon
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("manifest-") && n.ends_with(".json"));
    if !canon.starts_with(&canon_dir) || !name_ok {
        return Err(refuse());
    }
    let meta = std::fs::metadata(&canon).map_err(|e| McpError::InvalidParams(format!("cannot read manifest '{key}': {e}")))?;
    if !meta.is_file() || meta.len() > MAX_MANIFEST_BYTES {
        return Err(refuse());
    }
    let text = std::fs::read_to_string(&canon).map_err(|e| McpError::InvalidParams(format!("cannot read manifest '{key}': {e}")))?;
    serde_json::from_str(&text).map_err(|e| McpError::InvalidParams(format!("invalid manifest JSON: {e}")))
}

/// Downscales (if needed) and encodes a raw RGBA8 frame into a PNG file and
/// Base64 payload on a background worker thread.
pub fn encode_captured_frame(mut raw: RawCapturedFrame, max_dimensions: (u32, u32), label: &str) -> Result<EncodedCapture, McpError> {
    let expected_len = (raw.width as usize).saturating_mul(raw.height as usize).saturating_mul(4);
    if raw.rgba_bytes.len() < expected_len || raw.width == 0 || raw.height == 0 {
        return Err(McpError::Internal(format!(
            "invalid raw frame buffer ({} bytes for {}x{})",
            raw.rgba_bytes.len(),
            raw.width,
            raw.height
        )));
    }

    raw.rgba_bytes.truncate(expected_len);
    let img: RgbaImage = ImageBuffer::from_raw(raw.width, raw.height, raw.rgba_bytes)
        .ok_or_else(|| McpError::Internal("failed to construct RgbaImage".into()))?;

    let (max_w, max_h) = (max_dimensions.0.max(32), max_dimensions.1.max(32));
    let final_img = if raw.width > max_w || raw.height > max_h {
        let scale = (max_w as f32 / raw.width as f32).min(max_h as f32 / raw.height as f32);
        let nw = ((raw.width as f32 * scale).round() as u32).max(1);
        let nh = ((raw.height as f32 * scale).round() as u32).max(1);
        image::imageops::resize(&img, nw, nh, FilterType::Triangle)
    } else {
        img
    };

    let (width, height) = (final_img.width(), final_img.height());
    let rgba = final_img.as_raw();
    let mut rgb_bytes = Vec::with_capacity((width as usize).saturating_mul(height as usize).saturating_mul(3));
    for px in rgba.chunks_exact(4) {
        rgb_bytes.extend_from_slice(&px[..3]);
    }
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|e| McpError::Internal(format!("PNG header error: {e}")))?;
        writer
            .write_image_data(&rgb_bytes)
            .map_err(|e| McpError::Internal(format!("PNG encode error: {e}")))?;
    }

    let mut hasher = Sha256::new();
    hasher.update(&png_bytes);
    let sha256: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();

    let safe_label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let file_name = format!(
        "{}-{}-{}.png",
        if safe_label.is_empty() { "capture" } else { &safe_label },
        match raw.buffer_type {
            CaptureBufferType::ColorRgb => "color",
            CaptureBufferType::DepthNormalized => "depth",
        },
        &sha256[..12]
    );
    let out_dir = capture_output_dir();
    let path = out_dir.join(file_name);
    std::fs::write(&path, &png_bytes).map_err(|e| McpError::Internal(format!("failed to write capture PNG: {e}")))?;
    maybe_prune_capture_dir(&out_dir);

    let base64_png = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Ok(EncodedCapture {
        path: path.display().to_string(),
        sha256,
        width,
        height,
        base64_png,
        buffer_type: match raw.buffer_type {
            CaptureBufferType::ColorRgb => "COLOR".into(),
            CaptureBufferType::DepthNormalized => "DEPTH".into(),
        },
        near_clip: raw.near_clip,
        far_clip: raw.far_clip,
    })
}

/// Compare two images on disk (`image_compare`), computing RMSE, MAE, max channel
/// difference, changed pixel percentage, and writing a visual diff PNG.
pub fn compare_images(reference_path: &str, observed_path: &str) -> Result<Value, McpError> {
    let ref_path = validate_local_file_path(reference_path, ALLOWED_IMAGE_EXTENSIONS, Some(MAX_COMPARE_IMAGE_BYTES))?;
    let obs_path = validate_local_file_path(observed_path, ALLOWED_IMAGE_EXTENSIONS, Some(MAX_COMPARE_IMAGE_BYTES))?;
    let ref_img = image::open(&ref_path)
        .map_err(|e| McpError::InvalidParams(format!("cannot open reference '{reference_path}': {e}")))?
        .to_rgba8();
    let obs_img = image::open(&obs_path)
        .map_err(|e| McpError::InvalidParams(format!("cannot open observed '{observed_path}': {e}")))?
        .to_rgba8();

    if ref_img.dimensions() != obs_img.dimensions() {
        return Err(McpError::InvalidParams(format!(
            "image dimensions differ: reference is {}x{}, observed is {}x{}",
            ref_img.width(),
            ref_img.height(),
            obs_img.width(),
            obs_img.height()
        )));
    }

    let (w, h) = ref_img.dimensions();
    let total_pixels = (w as u64 * h as u64).max(1);
    let mut diff_img = RgbaImage::new(w, h);
    let mut sum_sq_err = 0.0f64;
    let mut sum_abs_err = 0.0f64;
    let mut max_channel_diff = 0u8;
    let mut changed_pixels = 0u64;

    for (x, y, p_ref) in ref_img.enumerate_pixels() {
        let p_obs = obs_img.get_pixel(x, y);
        let dr = p_ref[0].abs_diff(p_obs[0]);
        let dg = p_ref[1].abs_diff(p_obs[1]);
        let db = p_ref[2].abs_diff(p_obs[2]);
        let max_c = dr.max(dg).max(db);
        if max_c > 0 {
            changed_pixels += 1;
        }
        max_channel_diff = max_channel_diff.max(max_c);
        for d in [dr, dg, db] {
            let df = d as f64 / 255.0;
            sum_abs_err += df;
            sum_sq_err += df * df;
        }
        // Amplify small visual differences 4x so the diff PNG is easy to read.
        diff_img.put_pixel(x, y, Rgba([dr.saturating_mul(4), dg.saturating_mul(4), db.saturating_mul(4), 255]));
    }

    let channels = (total_pixels * 3) as f64;
    let mae = sum_abs_err / channels;
    let rmse = (sum_sq_err / channels).sqrt();
    let changed_ratio = changed_pixels as f64 / total_pixels as f64;

    let out_dir = capture_output_dir();
    let diff_path = out_dir.join(format!("diff-{}.png", Uuid::new_v4()));
    diff_img
        .save(&diff_path)
        .map_err(|e| McpError::Internal(format!("failed to save diff image: {e}")))?;
    maybe_prune_capture_dir(&out_dir);

    Ok(json!({
        "reference": reference_path,
        "observed": observed_path,
        "diff_image_path": diff_path.display().to_string(),
        "width": w,
        "height": h,
        "rmse": (rmse * 1_000_000.0).round() / 1_000_000.0,
        "mae": (mae * 1_000_000.0).round() / 1_000_000.0,
        "max_channel_diff": max_channel_diff,
        "changed_pixels": changed_pixels,
        "changed_pixel_ratio": (changed_ratio * 1_000_000.0).round() / 1_000_000.0,
        "identical": changed_pixels == 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_encode_and_compare_frames_roundtrip() {
        let raw_a = RawCapturedFrame {
            width: 64,
            height: 64,
            buffer_type: CaptureBufferType::ColorRgb,
            rgba_bytes: vec![120u8; 64 * 64 * 4],
            near_clip: 0.1,
            far_clip: 256.0,
            camera_position: [128.0, 128.0, 25.0],
            camera_focus: [128.0, 130.0, 25.0],
            in_demo_mode: true,
        };
        let mut raw_b = raw_a.clone();
        raw_b.rgba_bytes[0] = 200;

        let enc_a = encode_captured_frame(raw_a, (64, 64), "test_a").expect("encode a");
        let enc_b = encode_captured_frame(raw_b, (64, 64), "test_b").expect("encode b");

        let same = compare_images(&enc_a.path, &enc_a.path).expect("compare identical");
        assert_eq!(same["identical"], true);
        assert_eq!(same["rmse"], 0.0);

        let diff = compare_images(&enc_a.path, &enc_b.path).expect("compare modified");
        assert_eq!(diff["identical"], false);
        assert_eq!(diff["changed_pixels"], 1);
    }

    #[test]
    fn manifest_read_is_confined_to_the_capture_dir() {
        assert!(matches!(read_capture_manifest("/etc/passwd"), Err(McpError::PermissionDenied(_))));
        let outside = std::env::temp_dir().join(format!("manifest-outside-{}.json", uuid::Uuid::new_v4().simple()));
        std::fs::write(&outside, "{}").expect("write outside");
        assert!(matches!(
            read_capture_manifest(&outside.display().to_string()),
            Err(McpError::PermissionDenied(_))
        ));
        let _ = std::fs::remove_file(&outside);
        let id = uuid::Uuid::new_v4().to_string();
        let inside = capture_output_dir().join(format!("manifest-{id}.json"));
        std::fs::write(&inside, r#"{"manifest_id":"x"}"#).expect("write inside");
        assert_eq!(read_capture_manifest(&id).expect("by id")["manifest_id"], "x");
        assert_eq!(
            read_capture_manifest(&inside.display().to_string()).expect("by path")["manifest_id"],
            "x"
        );
        let _ = std::fs::remove_file(&inside);
    }

    #[test]
    fn prune_capture_dir_evicts_oldest_files_over_limits() {
        let dir = std::env::temp_dir().join(format!("aurora-prune-test-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).expect("create test dir");

        let base = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        for (i, size) in [(0, 40usize), (1, 40), (2, 40), (3, 40)] {
            let p = dir.join(format!("cap-{i}.png"));
            let f = std::fs::File::create(&p).expect("create file");
            use std::io::Write;
            (&f).write_all(&vec![b'x'; size]).expect("write bytes");
            let _ = f.set_modified(base + std::time::Duration::from_secs(i));
        }

        // Enforce max 2 files and max 90 bytes: cap-0 and cap-1 (oldest) must be removed.
        prune_capture_dir_in(&dir, 2, 90);
        assert!(!dir.join("cap-0.png").exists());
        assert!(!dir.join("cap-1.png").exists());
        assert!(dir.join("cap-2.png").exists());
        assert!(dir.join("cap-3.png").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
