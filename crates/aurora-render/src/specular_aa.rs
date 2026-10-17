//! The math of the geometric specular anti-aliasing (common.wgsl
//! `specular_aa_roughness`, `specular_aa_gloss`, `toksvig_variance`),
//! mirrored here to test it: the shaders must keep these in step.

const VARIANCE: f32 = 0.15;
const THRESHOLD: f32 = 0.2;
const FLOOR: f32 = 0.001;
const TOKSVIG_FROM: f32 = 0.985;

/// The variance of a normal turning `per_pixel` radians per pixel along
/// both screen axes.
fn variance_of_turn(per_pixel: f32) -> f32 {
    VARIANCE * 2.0 * per_pixel * per_pixel
}

fn toksvig_variance(map_len: f32) -> f32 {
    let len = map_len.clamp(1e-3, 1.0);
    (TOKSVIG_FROM - len).max(0.0) / len
}

fn roughness(perceptual: f32, variance: f32) -> f32 {
    let kernel = (2.0 * (variance - FLOOR).max(0.0)).min(THRESHOLD);
    let a = perceptual * perceptual;
    (a * a + kernel).clamp(0.0, 1.0).sqrt().sqrt()
}

fn gloss(gloss: f32, variance: f32) -> f32 {
    let e = gloss * gloss * 368.0;
    let alpha = (2.0 / (e + 2.0)).sqrt();
    let p = roughness(alpha.sqrt(), variance);
    let a2 = (p * p).max(1e-4);
    let e2 = (2.0 / (a2 * a2) - 2.0).max(0.0);
    (e2 / 368.0).sqrt().clamp(0.0, 1.0)
}

#[test]
fn a_normal_a_pixel_can_resolve_keeps_every_roughness_and_glossiness() {
    // still, and a ball about 110 pixels across (0.018 rad per pixel)
    for v in [0.0, variance_of_turn(0.018)] {
        for r in [0.03_f32, 0.1, 0.3, 0.6, 1.0] {
            assert!((roughness(r, v) - r).abs() < 1e-5, "roughness {r}");
        }
        for g in [0.05_f32, 0.2, 0.5, 0.8, 1.0] {
            assert!((gloss(g, v) - g).abs() < 1e-3, "glossiness {g}");
        }
    }
}

#[test]
fn a_normal_turning_faster_than_a_pixel_widens_highlights_up_to_the_cap() {
    let small_object = variance_of_turn(0.2);
    let relief_afar = variance_of_turn(0.6);
    // a mirror gets rougher, the more the normal turns per pixel
    assert!(roughness(0.03, small_object) > 0.2);
    assert!(roughness(0.03, relief_afar) > roughness(0.03, small_object));
    // capped: never rougher than alpha² = threshold for a mirror
    assert!((roughness(0.03, 100.0) - THRESHOLD.sqrt().sqrt()).abs() < 1e-3);
    // a rough surface hardly changes, even at the cap (0.9 → 0.96)
    assert!(roughness(0.9, relief_afar) - 0.9 < 0.07);
    // the same for a legacy glossiness: it can only go down
    assert!(gloss(1.0, relief_afar) < 0.5);
    assert!(gloss(0.3, small_object) <= 0.3 + 1e-4);
}

#[test]
fn only_normals_shorter_than_quantization_count_as_mip_variance() {
    assert_eq!(toksvig_variance(1.0), 0.0);
    assert_eq!(toksvig_variance(0.99), 0.0);
    assert!(toksvig_variance(0.9) > 0.09 && toksvig_variance(0.9) < 0.1);
    assert!(toksvig_variance(0.0) > 100.0);
}
