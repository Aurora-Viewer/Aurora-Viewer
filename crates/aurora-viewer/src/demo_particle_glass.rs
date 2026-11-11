//! AURORA_DEMO_PARTICLE_GLASS: particles sorted in with blended faces, to see
//! what is drawn over what (`scene::particle_order`), and what it costs.
//!
//! - `1`: along the start view, 9 m ahead, three 3 m glass panes side by
//!   side at face alpha 0.45: an ordinary one on the left, a fullbright one
//!   in the middle, a glowing one (glow 0.4) on the right. An emitter (the
//!   plaza fountain's) stands 2 m behind each pane, one 2 m in front of each,
//!   and one in the left pane's plane, so that its arc crosses it. Behind a
//!   pane the particles are tinted by the glass, in front they are clear;
//!   the arc through the pane is cut exactly at it; the glowing pane keeps
//!   its glow over the particles behind it, and the fullbright one sorts
//!   with them like the others (they are all blended faces). The emitters
//!   show in red: the particle-source beacon highlight, a debug overlay.
//! - `sparse`: what a busy place looks like more than `dense` does: 60 thin
//!   panes spread over 8 to 68 m ahead and 22 m across, and 6 emitters in
//!   it, each far from most of the panes on the screen. By depth alone the
//!   plumes are cut by every pane in their depth band; by where the panes are,
//!   by few.
//! - `dense`: 300 thin panes along the view, 8 to 80 m ahead, each beside
//!   the line of sight, and 36 emitters spread over the same depths, for the
//!   cost of the runs at the cap (`AURORA_PROFILE`; 256 runs are drawn at
//!   most, `AURORA_PARTICLE_INTERLEAVE=0` to compare).
//!
//! From the start position, or AURORA_DEMO_POS="x,y".

use super::{HANDLE, fountain_particles, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::objects::{ObjectUpdate, ParticleUpdate};
use aurora_prim::ExtraParams;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
use glam::{Quat, Vec3};

const FIRST_ID: u32 = 87_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Glass,
    Sparse,
    Dense,
}

fn mode() -> Option<Mode> {
    match std::env::var("AURORA_DEMO_PARTICLE_GLASS").ok()?.trim() {
        "1" => Some(Mode::Glass),
        "sparse" => Some(Mode::Sparse),
        "dense" => Some(Mode::Dense),
        _ => None,
    }
}

fn pane(id: u32, pos: Vec3, rot: Quat, size: f32, alpha: f32, fullbright: bool, glow: f32) -> ObjectUpdate {
    prim(
        id,
        pos,
        rot,
        Vec3::new(0.04, size, size),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.55, 0.75, 1.0, alpha], 0, fullbright, glow),
        ExtraParams::default(),
        "",
    )
}

/// A small dark block with the plaza fountain's particle system on top.
fn emitter(id: u32, pos: Vec3) -> ObjectUpdate {
    let mut o = prim(
        id,
        pos,
        Quat::IDENTITY,
        Vec3::splat(0.25),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.1, 0.1, 0.12, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    o.particles = ParticleUpdate::Set(fountain_particles());
    o
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
    // `along` m ahead and `side` m to the left of the view, `up` m over the ground
    let at = |along: f32, side: f32, up: f32| {
        let p = eye + look * along + across * side;
        Vec3::new(p.x, p.y, height(p.x, p.y) + up)
    };
    let mut objects = Vec::new();
    match mode {
        Mode::Glass => {
            // the panes' centres 1.9 m up: they stand on the ground
            let (left, middle, right) = (3.4, 0.0, -3.4);
            objects.push(pane(FIRST_ID, at(9.0, left, 1.9), upright, 3.0, 0.45, false, 0.0));
            objects.push(pane(FIRST_ID + 1, at(9.0, middle, 1.9), upright, 3.0, 0.45, true, 0.0));
            objects.push(pane(FIRST_ID + 2, at(9.0, right, 1.9), upright, 3.0, 0.45, false, 0.4));
            let spots = [
                (11.0, left),
                (11.0, middle),
                (11.0, right),
                (7.0, left),
                (7.0, middle),
                (7.0, right),
                (9.0, left + 1.0),
            ];
            for (id, (along, side)) in (FIRST_ID + 3..).zip(spots) {
                objects.push(emitter(id, at(along, side, 0.3)));
            }
        }
        Mode::Sparse => {
            for k in 0..60u32 {
                let along = 8.0 + k as f32;
                // spread over 22 m across, the order of the sides scrambled
                let side = ((k * 7) % 13) as f32 * 1.8 - 10.8;
                objects.push(pane(FIRST_ID + k, at(along, side, 1.9), upright, 3.0, 0.3, false, 0.0));
            }
            let spots = [(14.0, -9.0), (24.0, 6.0), (34.0, -3.0), (44.0, 9.0), (54.0, -7.0), (64.0, 2.0)];
            for (id, (along, side)) in (FIRST_ID + 300..).zip(spots) {
                objects.push(emitter(id, at(along, side, 0.3)));
            }
        }
        Mode::Dense => {
            for k in 0..300u32 {
                // 8 to 80 m, alternating sides of the line of sight
                let along = 8.0 + k as f32 * 0.24;
                let side = if k % 2 == 0 { -1.6 } else { 1.6 } + (((k * 7) % 5) as f32 - 2.0) * 0.3;
                objects.push(pane(FIRST_ID + k, at(along, side, 1.9), upright, 3.0, 0.3, false, 0.0));
            }
            for k in 0..36u32 {
                let along = 9.0 + k as f32 * 2.0;
                let side = ((k * 5) % 7) as f32 - 3.0;
                objects.push(emitter(FIRST_ID + 300 + k, at(along, side, 0.3)));
            }
        }
    }
    log::info!("demo particle glass: {mode:?}, {} objects", objects.len());
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}
