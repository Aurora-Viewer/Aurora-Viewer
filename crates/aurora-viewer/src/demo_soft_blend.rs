//! AURORA_DEMO_SOFT_BLEND=1: blended faces whose opacity crosses one half
//! within the face, under things that occlude them.
//!
//! A set 2 m along the start view and 3 m to its left, against a wall:
//!
//! - on the left a glass desk top (face alpha 0.72, a texture whose alpha
//!   wanders around 0.75: the opacity runs from 0.38 to 0.70) with a screen
//!   standing on it, and a box on a tray of the same texture at face alpha
//!   1 (0.53 to 0.97, across the opacity from which a face is opaque to the
//!   eye);
//! - on the right two shelves with balls, behind a pane of streaked glass
//!   (face alpha 0.66, streaks at texture alpha 1 over 0.55: the opacity is
//!   0.66 in the streaks and 0.36 between them).
//!
//! The screen-space occlusion and the contact shadows must fade with the
//! opacity of such a face, never stop along a line where it crosses a
//! limit: with `AURORA_DEMO_DEBUG=ao`, no ragged border on the desk top, on
//! the tray nor along the streaks, and what stands behind the glass keeps
//! its own occlusion.
//!
//! From the start position, or AURORA_DEMO_POS="x,y".

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::params::{LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PROFILE_SQUARE};
use aurora_prim::{ExtraParams, TextureEntry};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const FIRST_ID: u32 = 86_500;
/// Alpha wandering around 0.75, with the grain of a compressed texture.
const TEX_SMUDGE: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0042);
/// Slanted streaks at alpha 1 over a ground at 0.55.
const TEX_STREAKS: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0043);
const DESK_ALPHA: f32 = 0.72;
const PANE_ALPHA: f32 = 0.66;

fn enabled() -> bool {
    std::env::var("AURORA_DEMO_SOFT_BLEND").is_ok_and(|v| v.trim() == "1")
}

fn glass(texture: Uuid, alpha: f32) -> Arc<TextureEntry> {
    let mut t = TextureEntry::default();
    for f in t.faces.iter_mut() {
        f.texture = texture;
        f.color = [0.8, 0.9, 0.95, alpha];
    }
    Arc::new(t)
}

pub fn events() -> Vec<NetEvent> {
    if !enabled() {
        return Vec::new();
    }
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let across = Vec3::new(-look.y, look.x, 0.0);
    let facing = Quat::from_rotation_z(look.y.atan2(look.x));
    let eye = std::env::var("AURORA_DEMO_POS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
        .filter(|p| p.len() >= 2)
        .map_or(Vec3::new(132.0, 126.0, 0.0), |p| Vec3::new(p[0], p[1], 0.0));
    let centre = eye + look * 2.0 + across * 3.0;
    let ground = height(centre.x, centre.y);
    // `ahead` metres along the view, `left` across it, `up` above the ground
    let at = |ahead: f32, left: f32, up: f32| centre + look * ahead + across * left + Vec3::Z * (ground + up);
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let ball = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let wood = te([0.75, 0.7, 0.62, 1.0], 0, false, 0.0);
    let dark = te([0.2, 0.2, 0.22, 1.0], 0, false, 0.0);
    let red = te([0.8, 0.3, 0.25, 1.0], 0, false, 0.0);
    // (scales: along the view, across it, up)
    let mut parts = vec![
        // the wall
        (at(0.7, 0.0, 2.2), Vec3::new(0.1, 4.6, 2.4), boxp, wood.clone()),
        // the desk top, a box on its tray and a screen on its foot
        (at(0.15, 1.0, 1.6), Vec3::new(1.0, 2.2, 0.04), boxp, glass(TEX_SMUDGE, DESK_ALPHA)),
        (at(0.0, 1.6, 1.63), Vec3::new(0.6, 0.6, 0.02), boxp, glass(TEX_SMUDGE, 1.0)),
        (at(0.0, 1.6, 1.79), Vec3::new(0.3, 0.3, 0.3), boxp, red.clone()),
        (at(0.3, 0.7, 1.67), Vec3::new(0.25, 0.4, 0.1), boxp, dark.clone()),
        (at(0.3, 0.7, 2.05), Vec3::new(0.06, 1.1, 0.6), boxp, dark),
        // the pane, 25 cm in front of the wall
        (at(0.4, -1.3, 2.2), Vec3::new(0.01, 1.4, 1.6), boxp, glass(TEX_STREAKS, PANE_ALPHA)),
    ];
    for up in [1.75, 2.45] {
        parts.push((at(0.52, -1.3, up), Vec3::new(0.26, 1.4, 0.04), boxp, wood.clone()));
        for left in [-1.7, -1.3, -0.9] {
            parts.push((at(0.52, left, up + 0.15), Vec3::splat(0.26), ball, red.clone()));
        }
    }
    let objects: Vec<ObjectUpdate> = parts
        .into_iter()
        .enumerate()
        .map(|(k, (pos, scale, volume, entry))| prim(FIRST_ID + k as u32, pos, facing, scale, volume, entry, ExtraParams::default(), ""))
        .collect();
    log::info!("demo soft blend: {} objects", objects.len());
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// A value in 0..1 that changes from one texel block to the next.
fn grain(x: u32, y: u32) -> f32 {
    let h = (x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35);
    (h >> 24) as f32 / 255.0
}

/// Alpha of the desk top's texel, 0..1.
fn smudge_alpha(x: u32, y: u32) -> f32 {
    let wave = (x as f32 * 0.071 + 1.0).sin() * (y as f32 * 0.053).cos();
    (0.75 + 0.19 * wave + 0.04 * (grain(x / 3, y / 3) - 0.5)).clamp(0.0, 1.0)
}

/// Alpha of the pane's texel, 0..1.
fn streak_alpha(x: u32, y: u32) -> f32 {
    let t = ((x as f32 + 0.6 * y as f32) / 48.0).fract();
    let ramp = |a: f32, b: f32| ((t - a) / (b - a)).clamp(0.0, 1.0);
    let band = ramp(0.3, 0.4) - ramp(0.6, 0.7);
    (0.55 + 0.45 * band + 0.04 * (grain(x, y) - 0.5)).clamp(0.0, 1.0)
}

/// Pixels of this scenario's textures (RGBA8, 128×128).
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    const N: u32 = 128;
    let alpha: fn(u32, u32) -> f32 = match *id {
        TEX_SMUDGE => smudge_alpha,
        TEX_STREAKS => streak_alpha,
        _ => return None,
    };
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            px.extend_from_slice(&[235, 240, 245, (alpha(x, y) * 255.0).round() as u8]);
        }
    }
    Some((px, N, N))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both faces hold texels on each side of one half: the line the
    /// occlusion used to stop at runs through them.
    #[test]
    fn the_opacity_of_both_faces_crosses_one_half() {
        for (id, face) in [(TEX_SMUDGE, DESK_ALPHA), (TEX_STREAKS, PANE_ALPHA)] {
            let (px, w, h) = texture(&id).expect("a texture of the scenario");
            assert_eq!(px.len(), (w * h * 4) as usize);
            let opacity: Vec<f32> = px.chunks(4).map(|p| f32::from(p[3]) / 255.0 * face).collect();
            let over = opacity.iter().filter(|a| **a >= 0.5).count();
            assert!(
                over * 5 > opacity.len() && over * 5 < opacity.len() * 4,
                "{id}: {over} of {}",
                opacity.len()
            );
        }
    }
}
