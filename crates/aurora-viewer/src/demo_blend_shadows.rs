//! AURORA_DEMO_BLEND_SHADOWS: alpha-blended prims standing in the sun, to
//! see their shadows (fs_shadow) and what they cost.
//!
//! - `1`: six 2 m panels lying 1.3 m above the ground in a row 14 m along
//!   the start view, left to right: a greenhouse pane (opaque frame, panes at texture alpha 0.5);
//!   the same at face alpha 0.6; an opaque texture at face alpha 0.5, then
//!   0.03, then 0.95; a texture at alpha 0.75 everywhere. As in the Legacy
//!   Viewer, the shadows are: the frame alone, solid; the frame alone,
//!   dithered; dithered; none; solid; dithered.
//! - `dense`: 400 greenhouse panes over a 60 m square ahead, for the cost of
//!   the blended casters (AURORA_PROFILE, AURORA_BLEND_SHADOWS=0 to compare).
//!
//! From the start position, or AURORA_DEMO_POS="x,y".

use super::{BLANK_TEXTURE, HANDLE, height, prim, shape};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
use aurora_prim::{ExtraParams, TextureEntry};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const FIRST_ID: u32 = 86_000;
/// Opaque frame bars around four panes at alpha 128.
const TEX_GREENHOUSE: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0040);
/// Alpha 192 (0.75) everywhere: over the 0.598 cutoff, under 0.88.
const TEX_ALPHA_75: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0041);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Row,
    Dense,
}

fn mode() -> Option<Mode> {
    match std::env::var("AURORA_DEMO_BLEND_SHADOWS").ok()?.trim() {
        "1" => Some(Mode::Row),
        "dense" => Some(Mode::Dense),
        _ => None,
    }
}

fn panel(id: u32, pos: Vec3, rot: Quat, texture: Uuid, alpha: f32) -> ObjectUpdate {
    let mut t = TextureEntry::default();
    for f in t.faces.iter_mut() {
        f.texture = texture;
        f.color = [0.85, 0.95, 0.9, alpha];
    }
    prim(
        id,
        pos,
        rot,
        Vec3::new(0.04, 2.0, 2.0),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        Arc::new(t),
        ExtraParams::default(),
        "",
    )
}

pub fn events() -> Vec<NetEvent> {
    let Some(mode) = mode() else {
        return Vec::new();
    };
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let across = Vec3::new(-look.y, look.x, 0.0);
    let yaw = look.y.atan2(look.x);
    let eye = std::env::var("AURORA_DEMO_POS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
        .filter(|p| p.len() >= 2)
        .map_or(Vec3::new(132.0, 126.0, 0.0), |p| Vec3::new(p[0], p[1], 0.0));
    let upright = Quat::from_rotation_z(yaw);
    // the row: roofs lying 1.3 m above the ground, their shadows on it
    let flat = upright * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let above = |p: Vec3, h: f32| Vec3::new(p.x, p.y, height(p.x, p.y) + h);
    let mut objects = Vec::new();
    match mode {
        Mode::Row => {
            let row = [
                (TEX_GREENHOUSE, 1.0),
                (TEX_GREENHOUSE, 0.6),
                (BLANK_TEXTURE, 0.5),
                (BLANK_TEXTURE, 0.03),
                (BLANK_TEXTURE, 0.95),
                (TEX_ALPHA_75, 1.0),
            ];
            for (k, (texture, alpha)) in row.into_iter().enumerate() {
                // (right of the view: clear of the first-launch windows)
                let p = eye + look * 14.0 - across * (4.0 + (k as f32 - 2.5) * 2.4);
                objects.push(panel(FIRST_ID + k as u32, above(p, 1.3), flat, texture, alpha));
            }
        }
        Mode::Dense => {
            for gy in 0..20 {
                for gx in 0..20 {
                    let k = gy * 20 + gx;
                    let p = eye + look * (10.0 + gx as f32 * 3.0) + across * ((gy as f32 - 9.5) * 3.0);
                    objects.push(panel(FIRST_ID + k, above(p, 1.2), upright, TEX_GREENHOUSE, 1.0));
                }
            }
        }
    }
    log::info!("demo blend shadows: {mode:?}, {} panels", objects.len());
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// Pixels of this scenario's textures (RGBA8, 64×64).
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    const N: u32 = 64;
    match *id {
        TEX_GREENHOUSE => {
            let mut px = Vec::with_capacity((N * N * 4) as usize);
            for y in 0..N {
                for x in 0..N {
                    // bars 4 texels wide on the edges and across the middle
                    let bar = |v: u32| !(4..60).contains(&v) || (30..34).contains(&v);
                    if bar(x) || bar(y) {
                        px.extend_from_slice(&[70, 82, 76, 255]);
                    } else {
                        px.extend_from_slice(&[200, 230, 220, 128]);
                    }
                }
            }
            Some((px, N, N))
        }
        TEX_ALPHA_75 => Some(([220, 200, 160, 192].repeat((N * N) as usize), N, N)),
        _ => None,
    }
}
