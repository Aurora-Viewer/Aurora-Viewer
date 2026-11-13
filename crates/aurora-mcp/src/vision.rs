//! Off-thread Viewport PNG Encoding, Downscaling, Orbit Manifests, and Visual Image Diffing.

use crate::asset_inspect::validate_local_file_path;
use crate::bridge::{CaptureBufferType, McpError, RawCapturedFrame};
use base64::Engine;
use image::{ ImageBuffer, Rgba, RgbaImage, imageops::FilterType};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;

const ALLOWED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "tga", "bmp", "webp", "gif"];
const MAX_COMPARE_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

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

/// Downscales (if needed) and encodes a raw RGBA8 frame into a PNG file and
/// Base64 payload on a background worker thread.
pub fn encode_captured_frame(
    raw: RawCapturedFrame,
    max_dimensions: (u32, u32),
    label: &str,
) -> Result<EncodedCapture, McpError> {
    let expected_len = (raw.width as usize)
        .saturating_mul(raw.height as usize)
        .saturating_mul(4);
    if raw.rgba_bytes.len() < expected_len || raw.width == 0 || raw.height == 0 {
        return Err(McpError::Internal(format!(
            "invalid raw frame buffer ({} bytes for {}x{})",
            raw.rgba_bytes.len(),
            raw.width,
            raw.height
        )));
    }

    let img: RgbaImage = ImageBuffer::from_raw(
        raw.width,
        raw.height,
        raw.rgba_bytes[..expected_len].to_vec(),
    )
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
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| McpError::Internal(format!("PNG header error: {e}")))?;
        writer
            .write_image_data(final_img.as_raw())
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
        if safe_label.is_empty() {
            "capture"
        } else {
            &safe_label
        },
        match raw.buffer_type {
            CaptureBufferType::ColorRgb => "color",
            CaptureBufferType::DepthNormalized => "depth",
        },
        &sha256[..12]
    );
    let path = capture_output_dir().join(file_name);
    std::fs::write(&path, &png_bytes)
        .map_err(|e| McpError::Internal(format!("failed to write capture PNG: {e}")))?;

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
    let ref_path = validate_local_file_path(
        reference_path,
        ALLOWED_IMAGE_EXTENSIONS,
        Some(MAX_COMPARE_IMAGE_BYTES),
    )?;
    let obs_path = validate_local_file_path(
        observed_path,
        ALLOWED_IMAGE_EXTENSIONS,
        Some(MAX_COMPARE_IMAGE_BYTES),
    )?;
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
        diff_img.put_pixel(
            x,
            y,
            Rgba([
                dr.saturating_mul(4),
                dg.saturating_mul(4),
                db.saturating_mul(4),
                255,
            ]),
        );
    }

    let channels = (total_pixels * 3) as f64;
    let mae = sum_abs_err / channels;
    let rmse = (sum_sq_err / channels).sqrt();
    let changed_ratio = changed_pixels as f64 / total_pixels as f64;

    let diff_path = capture_output_dir().join(format!("diff-{}.png", Uuid::new_v4()));
    diff_img
        .save(&diff_path)
        .map_err(|e| McpError::Internal(format!("failed to save diff image: {e}")))?;

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
}
