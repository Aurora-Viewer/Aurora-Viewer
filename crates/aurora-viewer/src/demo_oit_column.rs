//! AURORA_DEMO_OIT_COLUMN=<tubes>[,<alpha>]: a tall translucent column with a
//! fine repeating texture beside the start position, for the anti-aliasing
//! modes of the order-independent transparency (oit_stochastic.rs): nested
//! hollow tubes (each one an outer and an inner wall, both sides seen
//! through the others), so a curved surface of many blended layers fills
//! much of the view at a slant. A wrong order of the layers shows as bands
//! across it; the stochastic method's grain should look the same in every
//! anti-aliasing mode.
use super::{HANDLE, height, prim, shape};
use aurora_net::NetEvent;
use aurora_prim::params::*;
use aurora_prim::{ExtraParams, TextureEntry};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const TEX_WEAVE: Uuid = Uuid::from_u128(0xDE40_C01E_0000_0000_0000_0000_0000_0001);

/// The tubes (1 to 5, 3 when the value is not a number) and the face
/// colour's alpha (0.05 to 1, 1 by default: the texture's own 0.3 to 0.6).
fn spec(value: &str) -> (u32, f32) {
    let mut parts = value.split(',').map(str::trim);
    let tubes = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(3).clamp(1, 5);
    let alpha = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0).clamp(0.05, 1.0);
    (tubes, alpha)
}

pub fn events() -> Vec<NetEvent> {
    let Ok(value) = std::env::var("AURORA_DEMO_OIT_COLUMN") else {
        return Vec::new();
    };
    let (tubes, alpha) = spec(&value);
    let (sx, sy) = std::env::var("AURORA_DEMO_POS")
        .ok()
        .and_then(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            (p.len() >= 2).then(|| (p[0], p[1]))
        })
        .unwrap_or((132.0, 126.0));
    // the start look-at (0.8, 0.4); the column 1.5 m ahead and 2 m to the
    // left of our avatar (the left of the view, clear of the windows)
    let (dx, dy) = (0.894_427_2, 0.447_213_6);
    let (px, py) = (sx + dx * 1.5 - dy * 2.0, sy + dy * 1.5 + dx * 2.0);
    let ground = height(px, py);
    let mut objects = Vec::new();
    for k in 0..tubes {
        let mut te = TextureEntry::default();
        for f in &mut te.faces {
            f.texture = TEX_WEAVE;
            f.color = [0.95, 0.85 - 0.08 * k as f32, 0.7, alpha];
            // a fine weave, its texels a few pixels wide at the start view
            // and finer towards the column's edges and top
            f.scale_s = 12.0;
            f.scale_t = 24.0;
            f.offset_s = k as f32 * 0.17;
        }
        let d = 2.2 - 0.4 * k as f32;
        objects.push(prim(
            98_100 + k,
            Vec3::new(px, py, ground + 4.0),
            Quat::IDENTITY,
            Vec3::new(d, d, 8.0),
            // hollow 60 %: an outer and an inner wall
            shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 30000, 0),
            Arc::new(te),
            ExtraParams::default(),
            "",
        ));
    }
    log::info!("oit column: {} tubes, alpha {alpha}", objects.len());
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// A 64 x 64 weave: thin stripes of two shades both ways, the alpha 0.3 to
/// 0.6 with a texel-scale grain (a fixed hash), like a woven or grained
/// translucent material.
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if *id != TEX_WEAVE {
        return None;
    }
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for y in 0u32..64 {
        for x in 0u32..64 {
            let h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 24;
            let warp = x % 4 < 2;
            let weft = y % 4 < 2;
            let shade: u8 = match (warp, weft) {
                (true, true) => 230,
                (true, false) | (false, true) => 170,
                (false, false) => 110,
            };
            let alpha = 77 + ((h * 76) >> 8) as u8;
            rgba.extend_from_slice(&[shade, shade.saturating_sub(20), shade.saturating_sub(50), alpha]);
        }
    }
    Some((rgba, 64, 64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_reads_its_tubes_and_alpha() {
        assert_eq!(spec("on"), (3, 1.0));
        assert_eq!(spec("5,0.5"), (5, 0.5));
        assert_eq!(spec("40,0"), (5, 0.05));
    }

    #[test]
    fn the_weave_alpha_stays_between_0_3_and_0_6() {
        let (rgba, w, h) = texture(&TEX_WEAVE).expect("the weave");
        assert_eq!((w, h), (64, 64));
        let alphas: Vec<u8> = rgba.chunks(4).map(|p| p[3]).collect();
        assert!(alphas.iter().all(|&a| (76..=154).contains(&a)));
        assert!(alphas.iter().any(|&a| a < 90) && alphas.iter().any(|&a| a > 140));
    }
}
