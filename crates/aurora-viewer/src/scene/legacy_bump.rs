//! Pure Legacy Viewer bump-preset resolution and height-to-normal conversion.
//!
//! Preset identifiers follow `LLDrawPoolBump::bindBumpMap` and the order of
//! `app_settings/std_bump.ini` in the Legacy Viewer. Height wrapping and the
//! normal construction follow `LLBumpImageList::generateNormalMapFromAlpha`
//! and `normgenF.glsl` (originally LGPL 2.1).

use glam::Vec3;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BumpSource {
    None,
    Brightness,
    Darkness,
    Standard(Uuid),
}

const STANDARD: [Uuid; 15] = [
    Uuid::from_u128(0x058c75c0_a0d5_f2f8_43f3_e9699a89c2fc), // woodgrain
    Uuid::from_u128(0x6c9fa78a_1c69_2168_325b_3e03ffa348ce), // bark
    Uuid::from_u128(0xb8eed5f0_64b7_6e12_b67f_43fa8e773440), // bricks
    Uuid::from_u128(0x9deab416_9c63_78d6_d558_9a156f12044c), // checker
    Uuid::from_u128(0xdb9d39ec_a896_c287_1ced_64566217021e), // concrete
    Uuid::from_u128(0xf2d7b6f6_4200_1e9a_fd5b_96459e950f94), // crustytile
    Uuid::from_u128(0xd9258671_868f_7511_c321_7baef9e948a4), // cutstone
    Uuid::from_u128(0xd21e44ca_ff1c_a96e_b2ef_c0753426b7d9), // discs
    Uuid::from_u128(0x4726f13e_bd07_f2fb_feb0_bfa2ac58ab61), // gravel
    Uuid::from_u128(0xe569711a_27c2_aad4_9246_0c910239a179), // petridish
    Uuid::from_u128(0x073c9723_540c_5449_cdd4_0e87fdc159e3), // siding
    Uuid::from_u128(0xae874d1a_93ef_54fb_5fd3_eb0cb156afc0), // stonetile
    Uuid::from_u128(0x92e66e00_f56f_598a_7997_048aa64cde18), // stucco
    Uuid::from_u128(0x83b77fc6_10b4_63ec_4de7_f40629f238c5), // suction
    Uuid::from_u128(0x735198cf_6ea0_2550_e222_21d3c6a341ae), // weave
];

/// Resolve a texture-entry bump code. Codes beyond the Legacy Viewer table
/// are reserved and intentionally do not become texture indices.
pub fn bump_source(code: u8) -> BumpSource {
    match code {
        0 => BumpSource::None,
        1 => BumpSource::Brightness,
        2 => BumpSource::Darkness,
        3..=17 => BumpSource::Standard(STANDARD[(code - 3) as usize]),
        _ => BumpSource::None,
    }
}

/// A PBR material owns its normal slot, as does any explicit legacy normal.
pub fn should_apply(code: u8, pbr: bool, explicit_normal: bool) -> bool {
    !pbr && !explicit_normal && bump_source(code) != BumpSource::None
}

/// Convert sampled RGB values to the pseudo-height used by the Legacy
/// Viewer brightness/darkness normal-map shader. `rgb` is the normalized
/// source RGB8/RGBA8 sample, without sRGB conversion (LLImageGL::createGLTexture).
pub fn brightness_height(rgb: [f32; 3], darkness: bool) -> f32 {
    let luminance = rgb[0] * 0.2995 + rgb[1] * 0.5875 + rgb[2] * 0.1145;
    if darkness { 1.0 - luminance } else { luminance }
}

/// Make an RGBA tangent-space normal image from a single-channel heightfield.
/// Height values are bytes, edge sampling wraps, and output alpha retains the
/// source height, matching the Legacy Viewer standard-bump callback. Returns
/// `None` for malformed dimensions/data or a non-finite/non-positive scale.
pub fn normal_from_height(width: usize, height: usize, heights: &[u8], scale: f32) -> Option<Vec<[u8; 4]>> {
    normal_from_height_scaled(width, height, heights, scale, 1.0)
}

/// Variant used by the Legacy Viewer's brightness/darkness shader, whose
/// luminance differences are multiplied by 512 rather than stored as bytes
/// (512 / 255, rounded to the matching two-byte slope).
pub fn normal_from_height_scaled(width: usize, height: usize, heights: &[u8], scale: f32, height_scale: f32) -> Option<Vec<[u8; 4]>> {
    let heights: Vec<_> = heights.iter().map(|&h| h as f32).collect();
    normal_from_values(width, height, &heights, scale, height_scale, false, 1.0)
}

/// normgenF.glsl retains float luminance until writing its normalized RGBA8
/// target. Quantizing height first would erase relief in dark gradients.
pub fn normal_from_brightness(width: usize, height: usize, rgba: &[u8], darkness: bool) -> Option<Vec<[u8; 4]>> {
    if rgba.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let heights: Vec<_> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            brightness_height(
                [pixel[0] as f32 / 255.0, pixel[1] as f32 / 255.0, pixel[2] as f32 / 255.0],
                darkness,
            )
        })
        .collect();
    normal_from_values(width, height, &heights, 64.0, 512.0, true, 255.0)
}

fn normal_from_values(
    width: usize,
    height: usize,
    heights: &[f32],
    scale: f32,
    height_scale: f32,
    rounded: bool,
    alpha_scale: f32,
) -> Option<Vec<[u8; 4]>> {
    let len = width.checked_mul(height)?;
    if width == 0
        || height == 0
        || heights.len() != len
        || !scale.is_finite()
        || scale <= 0.0
        || !height_scale.is_finite()
        || height_scale <= 0.0
    {
        return None;
    }

    let mut out = Vec::with_capacity(len);
    for y in 0..height {
        for x in 0..width {
            let r = (x + 1) % width;
            let l = (x + width - 1) % width;
            let down_y = (y + 1) % height;
            let up_y = (y + height - 1) % height;
            let center = heights[y * width + x];
            let right = Vec3::new(scale, 0.0, (heights[y * width + r] - center) * height_scale);
            let left = Vec3::new(-scale, 0.0, (heights[y * width + l] - center) * height_scale);
            let up = Vec3::new(0.0, -scale, (heights[up_y * width + x] - center) * height_scale);
            let down = Vec3::new(0.0, scale, (heights[down_y * width + x] - center) * height_scale);
            let normal = (right.cross(down) + down.cross(left) + left.cross(up) + up.cross(right)).normalize_or_zero();
            // Top-down source rows reverse the reference raw image's Y axis.
            // Flip before quantization: 255 - floor(x) differs by one byte.
            let normal = Vec3::new(normal.x, -normal.y, normal.z);
            let encoded = (normal * 0.5 + Vec3::splat(0.5)) * 255.0;
            let byte = |value: f32| if rounded { value.round() as u8 } else { value as u8 };
            out.push([byte(encoded.x), byte(encoded.y), byte(encoded.z), byte(center * alpha_scale)]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_every_legacy_bump_code_and_rejects_reserved_codes() {
        assert_eq!(bump_source(0), BumpSource::None);
        assert_eq!(bump_source(1), BumpSource::Brightness);
        assert_eq!(bump_source(2), BumpSource::Darkness);
        assert_ne!(bump_source(14), BumpSource::None); // Stone Tiles
        assert_ne!(bump_source(9), BumpSource::None); // Cut Stone
        for code in 18..32 {
            assert_eq!(bump_source(code), BumpSource::None);
        }
        let ids: Vec<_> = (3..=17)
            .map(|code| match bump_source(code) {
                BumpSource::Standard(id) => id,
                other => panic!("unexpected source for {code}: {other:?}"),
            })
            .collect();
        assert_eq!(ids, STANDARD);
    }

    #[test]
    fn explicit_normal_and_pbr_materials_keep_precedence() {
        assert!(should_apply(9, false, false));
        assert!(!should_apply(9, true, false));
        assert!(!should_apply(9, false, true));
        assert!(!should_apply(0, false, false));
        assert!(!should_apply(18, false, false));
    }

    #[test]
    fn flat_and_periodic_heightfields_produce_finite_upward_normals() {
        let flat = normal_from_height(2, 2, &[100; 4], 64.0).unwrap();
        assert!(
            flat.iter()
                .all(|p| (127..=128).contains(&p[0]) && (127..=128).contains(&p[1]) && p[2] == 255 && p[3] == 100)
        );
        let periodic = normal_from_height(2, 2, &[0, 255, 255, 0], 64.0).unwrap();
        assert_eq!(periodic.len(), 4);
        assert!(periodic.iter().all(|p| p[2] > 127));
    }

    #[test]
    fn ramp_tilts_and_wrapping_samples_opposite_edges() {
        let image = normal_from_height(3, 1, &[0, 100, 200], 64.0).unwrap();
        assert_eq!(image[1][3], 100);
        assert_ne!(image[1][0], 127);
        // At x=0, the left neighbor is x=2 due to wrapped sampling.
        assert_ne!(image[0][0], image[1][0]);
    }

    #[test]
    fn top_down_vertical_height_ramp_has_bottom_up_legacy_green_orientation() {
        let heights = [0, 0, 0, 100, 100, 100, 200, 200, 200];
        let image = normal_from_height(3, 3, &heights, 64.0).unwrap();
        assert!(image[4][1] > 128, "the middle row's height increases toward the image bottom");
        assert!(image[4][2] > 128);
    }

    #[test]
    fn brightness_and_darkness_are_complementary_legacy_heights() {
        let rgb = [0.2, 0.4, 0.8];
        let bright = brightness_height(rgb, false);
        let dark = brightness_height(rgb, true);
        assert!((bright + dark - 1.0).abs() < f32::EPSILON);
        assert!(brightness_height([1.0; 3], false) > brightness_height([0.0; 3], false));
        assert!(brightness_height([1.0; 3], true) < brightness_height([0.0; 3], true));
    }

    #[test]
    fn brightness_uses_normalized_source_bytes_and_float_shader_heights() {
        let gray = [128, 128, 128, 255].repeat(9);
        let map = normal_from_brightness(3, 3, &gray, false).unwrap();
        assert!(map.iter().all(|p| p[3] == 128), "RGB8 samples are not linearized to height 55");
        // One red byte has sub-byte luminance: quantizing height before the
        // shader's 512 multiplier would lose or exaggerate this shallow bump.
        let mut red = vec![0; 3 * 3 * 4];
        red[8] = 1;
        let bright = normal_from_brightness(3, 3, &red, false).unwrap();
        let dark = normal_from_brightness(3, 3, &red, true).unwrap();
        assert_ne!(bright[1][0], 128);
        assert!((i16::from(bright[1][0]) + i16::from(dark[1][0]) - 255).abs() <= 1);
    }

    #[test]
    fn standard_normal_quantization_matches_the_reference_unsigned_byte_cast() {
        let flat = normal_from_height(1, 1, &[100], 64.0).unwrap();
        assert_eq!(flat, [[127, 127, 255, 100]]);
        assert_eq!(normal_from_height(3, 1, &[0, 100, 200], 64.0).unwrap()[1], [20, 127, 196, 100]);
    }

    #[test]
    fn brightness_shader_height_scale_matches_512_over_byte_range() {
        let heights = [0, 100, 255];
        let standard = normal_from_height(3, 1, &heights, 64.0).unwrap();
        let brightness = normal_from_height_scaled(3, 1, &heights, 64.0, 512.0 / 255.0).unwrap();
        assert_ne!(standard, brightness);
        assert!(brightness[1][2] < standard[1][2]);
    }

    #[test]
    fn normal_scale_changes_slope_and_invalid_inputs_are_rejected() {
        let heights = [0, 100, 255];
        let strong = normal_from_height(3, 1, &heights, 32.0).unwrap();
        let mild = normal_from_height(3, 1, &heights, 128.0).unwrap();
        assert_ne!(strong[1][0], mild[1][0]);
        assert!(normal_from_height(0, 2, &[], 64.0).is_none());
        assert!(normal_from_height(2, 2, &[1], 64.0).is_none());
        assert!(normal_from_height(1, 1, &[1], f32::NAN).is_none());
    }
}
