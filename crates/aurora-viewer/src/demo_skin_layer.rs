//! AURORA_DEMO_BLEND_AO=layer: the soft-edged layers worn over mesh bodies
//! (a tattoo, a blush, a second skin). Two skin-coloured spheres ahead of
//! the start position, left of the avatar, each in a niche that occludes
//! it (a wall behind, one on each side, a floor and a brim):
//! - the nearer one under a shell 4 mm larger, of the same colour, whose
//!   blended texture fades in and out in soft patches;
//! - the other bare, the control.
//!
//! The shell is the colour of the sphere: where the patches fade nothing
//! should show, with the ambient occlusion on as with it off.

use super::{floor_at, prim, shape, te};
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

pub const SKIN: Uuid = Uuid::from_u128(0xde19_3e71_0000_4000_8000_0000_0000_0001);
pub const PATCHES: Uuid = Uuid::from_u128(0xde19_3e71_0000_4000_8000_0000_0000_0002);
/// Local ids taken by `objects`.
pub const COUNT: u32 = 13;

const SKIN_RGB: [u8; 3] = [224, 172, 140];

pub fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var("AURORA_DEMO").is_ok_and(|v| v.trim() == "1")
            && std::env::var("AURORA_DEMO_BLEND_AO").is_ok_and(|v| v.trim() == "layer")
    });
    *ENABLED
}

pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if !enabled() {
        return None;
    }
    match *id {
        SKIN => {
            const N: usize = 16;
            Some(([SKIN_RGB[0], SKIN_RGB[1], SKIN_RGB[2], 255].repeat(N * N), N as u32, N as u32))
        }
        PATCHES => Some(patches()),
        _ => None,
    }
}

/// Skin all over, the alpha in soft round patches: from clear to opaque
/// and back over an eighth of the texture, never a hard step.
fn patches() -> (Vec<u8>, u32, u32) {
    const N: usize = 256;
    let mut rgba = Vec::with_capacity(N * N * 4);
    for y in 0..N {
        for x in 0..N {
            let wave = |at: usize| (at as f32 / N as f32 * 4.0 * std::f32::consts::TAU).sin();
            let alpha = (0.5 + 0.5 * wave(x) * wave(y)).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[SKIN_RGB[0], SKIN_RGB[1], SKIN_RGB[2], (alpha * 255.0).round() as u8]);
        }
    }
    (rgba, N as u32, N as u32)
}

fn textured(texture: Uuid) -> Arc<aurora_prim::te::TextureEntry> {
    let mut entry = (*te([1.0; 4], 0, false, 0.0)).clone();
    for face in &mut entry.faces {
        face.texture = texture;
    }
    Arc::new(entry)
}

/// The two niches, from local id `first`, 2.5 m ahead of the start
/// position `(sx, sy)` along the start look-at (0.8, 0.4).
pub fn objects(first: u32, sx: f32, sy: f32) -> Vec<ObjectUpdate> {
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let (dx, dy) = (0.894_427_2, 0.447_213_6);
    let ahead = Vec3::new(dx, dy, 0.0);
    let facing = Quat::from_rotation_z(dy.atan2(dx));
    let side_axis = Vec3::new(-dy, dx, 0.0);
    let grey = te([0.55, 0.55, 0.58, 1.0], 0, false, 0.0);
    let mut objects = Vec::with_capacity(COUNT as usize);
    let mut id = first;
    let mut add = |at: Vec3, rotation: Quat, scale: Vec3, volume, entry| {
        objects.push(prim(id, at, rotation, scale, volume, entry, ExtraParams::default(), ""));
        id += 1;
    };
    for (side, layered) in [(0.6, true), (1.4, false)] {
        // `side` metres to the left of the view's axis
        let (x, y) = (sx + dx * 2.5 - dy * side, sy + dy * 2.5 + dx * side);
        let at = Vec3::new(x, y, floor_at(x, y) + 1.6);
        add(at, Quat::IDENTITY, Vec3::splat(0.4), sphere, textured(SKIN));
        if layered {
            add(at, Quat::IDENTITY, Vec3::splat(0.408), sphere, textured(PATCHES));
        }
        // the niche around it: a wall behind, one on each side, a floor
        // and a brim, open towards the camera
        add(at + ahead * 0.22, facing, Vec3::new(0.04, 0.6, 0.6), boxp, grey.clone());
        for s in [-1.0, 1.0] {
            add(at + side_axis * (s * 0.28), facing, Vec3::new(0.6, 0.04, 0.6), boxp, grey.clone());
            add(at + Vec3::Z * (s * 0.28), facing, Vec3::new(0.6, 0.6, 0.04), boxp, grey.clone());
        }
    }
    objects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::jobs::{AlphaKind, classify_alpha};

    #[test]
    fn the_layer_fades_without_a_step_and_is_blended() {
        let (px, w, h) = patches();
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert_eq!(classify_alpha(&px), AlphaKind::Blend);
        // clear and opaque patches, and no texel far from its neighbour
        let alpha = |x: u32, y: u32| i32::from(px[((y * w + x) * 4 + 3) as usize]);
        assert!(px.chunks(4).any(|p| p[3] < 8) && px.chunks(4).any(|p| p[3] > 247));
        for y in 0..h {
            for x in 1..w {
                assert!((alpha(x, y) - alpha(x - 1, y)).abs() <= 10, "{x} {y}");
            }
        }
        assert_eq!(objects(1, 100.0, 100.0).len(), COUNT as usize);
    }
}
