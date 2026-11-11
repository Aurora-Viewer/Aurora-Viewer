//! AURORA_DEMO_WET_LAYER=1: the "wet" and "shine" layers worn over mesh
//! bodies (#1420). Three skin-coloured spheres ahead of the start position,
//! left of the avatar, each under a slightly larger shell, from the avatar
//! outwards:
//! - a transparent texture (alpha 0) with a blended legacy material whose
//!   specular map holds wet streaks, as a body's wet layer;
//! - the built-in transparent texture with the same material, as the shine
//!   layers of mesh heads and eyes;
//! - the same transparent texture with no material, the control: nothing
//!   shows.
//!
//! The Legacy Viewer draws the blended shells with max(texture alpha, glare)
//! (class3/deferred/materialF.glsl): the streaks reflect the sky, the rest of
//! the shell stays invisible.

use super::{floor_at, prim, shape, te};
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

pub const SKIN: Uuid = Uuid::from_u128(0xde19_3e70_0000_4000_8000_0000_0000_0001);
pub const CLEAR: Uuid = Uuid::from_u128(0xde19_3e70_0000_4000_8000_0000_0000_0002);
pub const STREAKS: Uuid = Uuid::from_u128(0xde19_3e70_0000_4000_8000_0000_0000_0003);
pub const MATERIAL: Uuid = Uuid::from_u128(0xde19_3e70_0000_4000_8000_0000_0000_0004);
/// Local ids taken by `objects`.
pub const COUNT: u32 = 6;

pub fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var("AURORA_DEMO").is_ok_and(|v| v.trim() == "1") && std::env::var("AURORA_DEMO_WET_LAYER").is_ok_and(|v| v.trim() == "1")
    });
    *ENABLED
}

pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if !enabled() {
        return None;
    }
    match *id {
        SKIN => Some(solid([224, 172, 140, 255])),
        CLEAR => Some(solid([255, 255, 255, 0])),
        STREAKS => Some(streaks()),
        _ => None,
    }
}

fn solid(rgba: [u8; 4]) -> (Vec<u8>, u32, u32) {
    const N: usize = 16;
    (rgba.repeat(N * N), N as u32, N as u32)
}

/// Drips running down: white where wet, black elsewhere, the alpha (the
/// environment mask) the same.
fn streaks() -> (Vec<u8>, u32, u32) {
    const N: usize = 128;
    let mut rgba = vec![0u8; N * N * 4];
    for column in (3..N).step_by(9) {
        let x = (column + (column * 7) % 5) % N;
        let start = (column * 37) % 48;
        let length = 40 + (column * 53) % 70;
        for y in start..(start + length).min(N) {
            let wobble = ((y as f32 * 0.15 + column as f32).sin() * 1.5).round() as isize;
            for w in 0..2isize {
                let px = (x as isize + wobble + w).rem_euclid(N as isize) as usize;
                let i = (y * N + px) * 4;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    (rgba, N as u32, N as u32)
}

pub fn material() -> crate::scene::legacy_mat::LegacyMaterial {
    crate::scene::legacy_mat::LegacyMaterial {
        specular_map: STREAKS,
        specular_st: [1.0, 1.0, 0.0, 0.0],
        specular_color: [255, 255, 255, 255],
        specular_exp: 200,
        env_intensity: 200,
        diffuse_alpha_mode: crate::scene::legacy_mat::alpha_mode::BLEND,
        ..Default::default()
    }
}

fn layer(texture: Uuid, material: Option<Uuid>) -> Arc<aurora_prim::te::TextureEntry> {
    let mut entry = (*te([1.0; 4], 0, false, 0.0)).clone();
    for face in &mut entry.faces {
        face.texture = texture;
        if let Some(material) = material {
            face.material_id = material;
        }
    }
    Arc::new(entry)
}

/// The spheres and their shells, from local id `first`, 2.5 m ahead of the
/// start position `(sx, sy)` along the start look-at (0.8, 0.4).
pub fn objects(first: u32, sx: f32, sy: f32) -> Vec<ObjectUpdate> {
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let (dx, dy) = (0.894_427_2, 0.447_213_6);
    // left of the avatar, as AURORA_DEMO_BLEND_AO: in view of the start
    // camera, clear of the windows on the left of the screen
    let shells = [
        (0.0, layer(CLEAR, Some(MATERIAL))),
        (1.0, layer(aurora_prim::te::TRANSPARENT_TEXTURE, Some(MATERIAL))),
        (2.0, layer(CLEAR, None)),
    ];
    let mut objects = Vec::with_capacity(COUNT as usize);
    for (i, (side, shell)) in shells.into_iter().enumerate() {
        // `side` metres to the left of the view's axis
        let (x, y) = (sx + dx * 2.5 - dy * side, sy + dy * 2.5 + dx * side);
        let at = Vec3::new(x, y, floor_at(x, y) + 1.6);
        let id = first + 2 * i as u32;
        objects.push(prim(
            id,
            at,
            Quat::IDENTITY,
            Vec3::splat(0.8),
            sphere.clone(),
            layer(SKIN, None),
            ExtraParams::default(),
            "",
        ));
        objects.push(prim(
            id + 1,
            at,
            Quat::IDENTITY,
            Vec3::splat(0.82),
            sphere.clone(),
            shell,
            ExtraParams::default(),
            "",
        ));
    }
    objects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wet_layer_fixture_is_a_transparent_shell_with_a_blended_shiny_material() {
        let (clear, _, _) = solid([255, 255, 255, 0]);
        assert!(clear.chunks(4).all(|p| p[3] == 0));
        let (streaks, w, h) = streaks();
        assert_eq!(streaks.len(), (w * h * 4) as usize);
        assert!(streaks.chunks(4).any(|p| p[3] == 255) && streaks.chunks(4).any(|p| p[3] == 0));
        let m = material();
        assert_eq!(m.diffuse_alpha_mode, crate::scene::legacy_mat::alpha_mode::BLEND);
        assert!(m.env_intensity > 0 && m.specular_exp > 0);
        let objects = objects(1, 100.0, 100.0);
        assert_eq!(objects.len(), COUNT as usize);
    }
}
