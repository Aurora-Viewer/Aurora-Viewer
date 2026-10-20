//! Texture uploads, following LLFloaterImagePreview::loadImage and
//! LLImageRaw::biasedDimToPowerOfTwo (Firestorm, originally LGPL 2.1).
//! This module runs on background workers; it keeps source pixels so changes
//! of maximum size or alpha policy never compound a previous resampling.

use crate::{AssetError, DecodedImage};
use image::{ImageReader, imageops::FilterType};
use std::io::Cursor;

pub const MAX_SOURCE_PIXELS: u64 = 8096 * 8096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub max_width: u32,
    pub max_height: u32,
    pub lossless_small: bool,
    pub strip_alpha: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_width: 1024,
            max_height: 1024,
            lossless_small: true,
            strip_alpha: true,
        }
    }
}

/// Firestorm's 1.75 threshold biases sizes down, unlike nearest-power rounding.
pub fn biased_dimension(size: u32, maximum: u32) -> u32 {
    let maximum = maximum.clamp(4, 2048).next_power_of_two().min(2048);
    let mut smaller = maximum;
    let mut larger = maximum;
    while smaller > size && smaller > 4 {
        larger = smaller;
        smaller >>= 1;
    }
    if size as f64 / smaller as f64 > 1.75 { larger } else { smaller }
}

pub fn dimensions(width: u32, height: u32, options: Options) -> (u32, u32) {
    let max_w = options.max_width.clamp(4, 2048);
    let max_h = options.max_height.clamp(4, 2048);
    if width > max_w || height > max_h {
        let scale = (max_w as f64 / width as f64).min(max_h as f64 / height as f64);
        let contract = |n: f64, max: u32| {
            let n = (n.round() as u32).clamp(4, max);
            1 << n.ilog2()
        };
        (contract(width as f64 * scale, max_w), contract(height as f64 * scale, max_h))
    } else {
        (biased_dimension(width, 2048), biased_dimension(height, 2048))
    }
}

pub fn decode(bytes: &[u8]) -> Result<DecodedImage, AssetError> {
    decode_inner(bytes, false)
}

/// Material maps may use luminance PNGs; convert their channels explicitly.
pub fn decode_map(bytes: &[u8]) -> Result<DecodedImage, AssetError> {
    decode_inner(bytes, true)
}
fn decode_inner(bytes: &[u8], maps: bool) -> Result<DecodedImage, AssetError> {
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(AssetError::TooLarge("texture file"));
    }
    if bytes.starts_with(&[0xff, 0x4f]) || bytes.get(4..12) == Some(b"jP  \r\n\x87\n") {
        let mut image = crate::decode_j2k(bytes, 0)?;
        if maps && matches!(image.components, 1 | 2) {
            let alpha = image.components == 2;
            let mut data = Vec::with_capacity(image.width as usize * image.height as usize * if alpha { 4 } else { 3 });
            for pixel in image.data.chunks_exact(image.components as usize) {
                data.extend_from_slice(&[pixel[0]; 3]);
                if alpha {
                    data.push(pixel[1]);
                }
            }
            image.data = data;
            image.components = if alpha { 4 } else { 3 };
        }
        validate(&image)?;
        return Ok(image);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| AssetError::invalid(e.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(8096 * 8096 / 4);
    limits.max_image_height = Some(8096 * 8096 / 4);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| AssetError::invalid(e.to_string()))?;
    use image::ImageDecoder;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > MAX_SOURCE_PIXELS {
        return Err(AssetError::TooLarge("texture pixel area"));
    }
    if !matches!(decoder.color_type(), image::ColorType::Rgb8 | image::ColorType::Rgba8)
        && !(maps && matches!(decoder.color_type(), image::ColorType::L8 | image::ColorType::La8))
    {
        return Err(AssetError::invalid("texture must have RGB or RGBA channels"));
    }
    let mut source = image::DynamicImage::from_decoder(decoder).map_err(|e| AssetError::invalid(e.to_string()))?;
    if maps && source.color() == image::ColorType::L8 {
        source = image::DynamicImage::ImageRgb8(source.to_rgb8());
    }
    if maps && source.color() == image::ColorType::La8 {
        source = image::DynamicImage::ImageRgba8(source.to_rgba8());
    }
    let components = source.color().channel_count();
    let image = DecodedImage {
        width: w,
        height: h,
        components,
        data: source.into_bytes(),
        discard: 0,
    };
    validate(&image)?;
    Ok(image)
}

pub(crate) fn validate(image: &DecodedImage) -> Result<(), AssetError> {
    if image.width == 0 || image.height == 0 || u64::from(image.width) * u64::from(image.height) > MAX_SOURCE_PIXELS {
        return Err(AssetError::TooLarge("texture dimensions"));
    }
    if !matches!(image.components, 3 | 4)
        || image.data.len() != image.width as usize * image.height as usize * usize::from(image.components)
    {
        return Err(AssetError::invalid("texture channels or pixels"));
    }
    Ok(())
}

pub fn empty_alpha(image: &DecodedImage) -> bool {
    image.components == 4
        && image.data.as_chunks::<4>().0.iter().filter(|p| p[3] > 253).count() as f64 > image.width as f64 * image.height as f64 * 0.999
}

pub fn prepare(source: &DecodedImage, options: Options) -> Result<DecodedImage, AssetError> {
    validate(source)?;
    let rgba: Vec<u8> = if source.components == 4 {
        source.data.clone()
    } else {
        source
            .data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect()
    };
    let pixels = image::RgbaImage::from_raw(source.width, source.height, rgba).ok_or_else(|| AssetError::invalid("texture pixels"))?;
    let (width, height) = dimensions(source.width, source.height, options);
    let pixels = if (width, height) == (source.width, source.height) {
        pixels
    } else {
        image::imageops::resize(&pixels, width, height, FilterType::Triangle)
    };
    let strip = source.components == 3 || (options.strip_alpha && empty_alpha(source));
    let data = if strip {
        pixels.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect()
    } else {
        pixels.into_raw()
    };
    Ok(DecodedImage {
        width,
        height,
        components: if strip { 3 } else { 4 },
        data,
        discard: 0,
    })
}

pub fn encode(pixels: &DecodedImage, options: Options) -> Result<Vec<u8>, AssetError> {
    validate(pixels)?;
    // JPEG2KEncode::setImage (indra/llimagej2coj/llimagej2coj.cpp) converts
    // bottom-up LLImageRaw rows to top-down codestream rows. DecodedImage
    // already uses that order, so pass its pixels directly to OpenJPEG.
    crate::j2k::encode_texture_j2k(
        pixels.width,
        pixels.height,
        pixels.components,
        &pixels.data,
        options.lossless_small && pixels.width * pixels.height <= 128 * 128,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scaling_matches_firestorm_threshold_and_preserves_large_source_ratio() {
        assert_eq!(biased_dimension(224, 2048), 128);
        assert_eq!(biased_dimension(225, 2048), 256);
        assert_eq!(dimensions(4000, 2000, Options::default()), (1024, 512));
        assert_eq!(dimensions(16, 1024, Options::default()), (16, 1024));
    }
    #[test]
    fn alpha_policy_and_upload_row_order_keep_preview_pixels_intact() {
        let source = DecodedImage {
            width: 4,
            height: 4,
            components: 4,
            data: (0..16).flat_map(|i| [i * 10, 20, 30, 255]).collect(),
            discard: 0,
        };
        let options = Options::default();
        assert!(empty_alpha(&source));
        let pixels = prepare(&source, options).expect("prepare");
        assert_eq!(pixels.components, 3);
        let uploaded = crate::decode_j2k(&encode(&pixels, options).expect("encode"), 0).expect("decode");
        assert_eq!(uploaded, pixels);
        assert_eq!(pixels.data[0], 0);
        let mut transparent = source;
        transparent.data[3] = 200;
        assert!(!empty_alpha(&transparent));
        assert_eq!(prepare(&transparent, options).expect("prepare").components, 4);
    }
    #[test]
    fn imported_images_preserve_orientation_after_upload() {
        for components in [3, 4] {
            let rgba = image::RgbaImage::from_fn(8, 4, |x, y| {
                image::Rgba([
                    (x * 25) as u8,
                    (y * 60) as u8,
                    (x * 10 + y * 15) as u8,
                    (80 + x * 10 + y * 20) as u8,
                ])
            });
            let source = if components == 4 {
                image::DynamicImage::ImageRgba8(rgba)
            } else {
                image::DynamicImage::ImageRgb8(image::DynamicImage::ImageRgba8(rgba).into_rgb8())
            };
            for format in [image::ImageFormat::Png, image::ImageFormat::Bmp] {
                let mut file = Cursor::new(Vec::new());
                source.write_to(&mut file, format).expect("source file");
                let decoded = decode(file.get_ref()).expect("import image");
                let options = Options::default();
                let pixels = prepare(&decoded, options).expect("prepare image");
                let original = pixels.clone();
                let uploaded = encode(&pixels, options).expect("upload image");
                assert_eq!(pixels, original, "encoding must preserve the preview");
                let downloaded = crate::decode_j2k(&uploaded, 0).expect("download image");
                assert_eq!(downloaded, pixels, "{format:?}, {components} channels");
                assert_eq!(decode(&uploaded).expect("reimport image"), pixels);
                assert_eq!(decode_map(&uploaded).expect("reimport material map"), pixels);
            }
        }
    }
    #[test]
    fn large_progressive_uploads_decode_at_every_resolution() {
        let pixels = DecodedImage {
            width: 2048,
            height: 32,
            components: 3,
            data: vec![150; 2048 * 32 * 3],
            discard: 0,
        };
        let bytes = encode(&pixels, Options::default()).expect("encode");
        for discard in 0..=4 {
            let image = crate::decode_j2k(&bytes, discard).expect("decode progressive");
            assert_eq!(image.width, 2048 >> discard);
            assert_eq!(image.height, 32 >> discard);
        }
    }
}
