//! AURORA_DEMO_VEILS=<n>[,<strings>[,<gap>]]: a hall hung with lace, as a wedding
//! venue is: the order and the cost of many blended layers one behind the
//! other, crossing one another, with faintly glowing strings among them.
//!
//! - `n` (40 for any value that is not a number) curtains 14 m wide and 8 m
//!   tall, 1 m apart from 2.5 m ahead of the start view: lace whose holes
//!   are clear, whose net is 38 % opaque and whose threads are opaque, in
//!   four tints. Three in four hang across the view; the others hang along
//!   it or at a slant and cross those: no order of the curtains is right
//!   for both sides of a crossing.
//! - `strings` (as many as curtains when left out): glowing strings hanging
//!   between the curtains, 80 % opaque.
//! - `gap`: centimeters from one curtain to the next (100 when left out).
//!   Close together, all of them fill the view: `40,0,25` is 40 layers on
//!   most pixels, most of them hidden behind the first ones; `4,0` is 4,
//!   all of them showing.
//!
//! What to look for: a curtain keeps its tint from one side of a crossing to
//! the other, and the nearer of two is the one seen over the other on both
//! sides; the strings behind several curtains glow less than those in front.
//! From the start position, or AURORA_DEMO_POS="x,y".

use super::{HANDLE, height, prim, shape};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
use aurora_prim::{ExtraParams, TextureEntry};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const FIRST_ID: u32 = 88_000;
/// Lace: clear holes, a net at alpha 96, opaque threads.
const TEX_LACE: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0050);
/// Opaque white, for the strings.
const TEX_STRING: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0051);

const TINTS: [[f32; 3]; 4] = [[1.0, 0.96, 0.9], [1.0, 0.82, 0.86], [0.84, 0.9, 1.0], [0.88, 1.0, 0.86]];

/// Curtains and strings asked for, and the meters between two curtains.
fn counts() -> Option<(u32, u32, f32)> {
    let v = std::env::var("AURORA_DEMO_VEILS").ok()?;
    let mut parts = v.split(',').map(|p| p.trim().parse::<u32>().ok());
    let curtains = parts.next().flatten().unwrap_or(40).clamp(1, 400);
    let strings = parts.next().flatten().unwrap_or(curtains).min(2000);
    let gap = parts.next().flatten().unwrap_or(100).clamp(5, 1000) as f32 / 100.0;
    Some((curtains, strings, gap))
}

fn hung(id: u32, pos: Vec3, rot: Quat, scale: Vec3, texture: Uuid, color: [f32; 4], glow: f32) -> ObjectUpdate {
    let mut t = TextureEntry::default();
    for f in t.faces.iter_mut() {
        f.texture = texture;
        // a cell of lace every 15 cm or so
        f.scale_s = (scale.y * 1.6).max(1.0).round();
        f.scale_t = (scale.z * 1.6).max(1.0).round();
        f.color = color;
        f.glow = glow;
    }
    prim(
        id,
        pos,
        rot,
        scale,
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        Arc::new(t),
        ExtraParams::default(),
        "",
    )
}

pub fn events() -> Vec<NetEvent> {
    let Some((curtains, strings, gap)) = counts() else {
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
    let ground = height(eye.x, eye.y);
    let at = |ahead: f32, side: f32, up: f32| {
        let p = eye + look * ahead + across * side;
        Vec3::new(p.x, p.y, ground + up)
    };
    let mut objects = Vec::new();
    for k in 0..curtains {
        let ahead = 2.5 + k as f32 * gap;
        let tint = TINTS[k as usize % TINTS.len()];
        // three in four across the view, to one side then the other; the
        // fourth along it or at a slant, through its neighbours
        let (side, turn) = match k % 8 {
            3 => (-3.0, std::f32::consts::FRAC_PI_2),
            7 => (3.0, 0.6),
            _ => (if k % 2 == 0 { -1.5 } else { 1.5 }, 0.0),
        };
        objects.push(hung(
            FIRST_ID + k,
            at(ahead, side, 4.0),
            Quat::from_rotation_z(yaw + turn),
            Vec3::new(0.02, 14.0, 8.0),
            TEX_LACE,
            [tint[0], tint[1], tint[2], 1.0],
            0.0,
        ));
    }
    let depth = curtains as f32 * gap;
    for k in 0..strings {
        // spread over the hall, between the curtains
        let ahead = 3.0 + (k as f32 * 0.618_034).fract() * depth;
        let side = ((k as f32 * 0.754_877).fract() - 0.5) * 12.0;
        objects.push(hung(
            FIRST_ID + 1000 + k,
            at(ahead, side, 4.0),
            Quat::IDENTITY,
            Vec3::new(0.04, 0.04, 7.6),
            TEX_STRING,
            [1.0, 0.9, 0.6, 0.8],
            0.3,
        ));
    }
    log::info!("demo veils: {curtains} curtains {gap} m apart, {strings} strings");
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// Pixels of this scenario's textures (RGBA8).
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    const N: u32 = 128;
    match *id {
        TEX_LACE => {
            let mut px = Vec::with_capacity((N * N * 4) as usize);
            for y in 0..N {
                for x in 0..N {
                    // a hole in each cell of 32 texels, a thread around it
                    let (cx, cy) = ((x % 32) as f32 - 15.5, (y % 32) as f32 - 15.5);
                    let r = (cx * cx + cy * cy).sqrt();
                    // (and a hem of thread along the cell's edges)
                    let hem = x % 32 < 2 || y % 32 < 2;
                    let alpha = if r < 8.0 {
                        0
                    } else if r < 10.5 || hem {
                        255
                    } else {
                        96
                    };
                    px.extend_from_slice(&[240, 236, 228, alpha]);
                }
            }
            Some((px, N, N))
        }
        TEX_STRING => Some(([255, 255, 255, 255].repeat(64), 8, 8)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lace_has_holes_a_net_and_threads() {
        let (px, w, h) = texture(&TEX_LACE).expect("the lace");
        assert_eq!(px.len() as u32, w * h * 4);
        let count = |alpha: u8| px.as_chunks::<4>().0.iter().filter(|p| p[3] == alpha).count();
        assert!(count(0) > 0 && count(96) > count(255) && count(255) > 0);
        assert_eq!(count(0) + count(96) + count(255), (w * h) as usize);
    }
}
