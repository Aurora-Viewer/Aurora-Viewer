//! AURORA_DEMO_SOFT_SHADOWS: things standing in the sun, to see how the
//! penumbra of their shadows widens with the distance of the caster (soft
//! shadows, aurora-render `soft_shadows.rs`).
//!
//! - `1` or `near` (the same at 0.4 of the distance): along the start view, left to right: a pergola (a 6 m by 3 m roof
//!   3 m up on four posts), a tall pole 14 m high, a table (a top 0.75 m up
//!   on four legs) and a sphere on the ground (the acne check). The sun is
//!   fixed 35 degrees up (AURORA_DEMO_SUN overrides it) and the soft shadows
//!   are on at the Preferences' softness (AURORA_SOFT_SHADOWS=0 and
//!   AURORA_SHADOW_SOFTNESS=<x> compare). Add AURORA_DEMO_CROWD=1 for a
//!   standing avatar.
//!
//! From the start position, or AURORA_DEMO_POS="x,y".

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::{
    LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PROFILE_SQUARE,
};
use glam::{Quat, Vec3};

const FIRST_ID: u32 = 87_000;

/// Whether the scenario is asked for.
pub fn enabled() -> bool {
    std::env::var("AURORA_DEMO_SOFT_SHADOWS").is_ok_and(|v| matches!(v.trim(), "1" | "near"))
}

/// The sun the scenario stands in, 35 degrees up, in front of the start view so that the shadows fall towards it
/// (AURORA_DEMO_SUN overrides it).
pub fn sun() -> Option<Vec3> {
    enabled().then(|| {
        let (azimuth, elevation) = (60.0_f32.to_radians(), 35.0_f32.to_radians());
        Vec3::new(elevation.cos() * azimuth.cos(), elevation.cos() * azimuth.sin(), elevation.sin())
    })
}

/// A box standing on `base` (its middle, at the height of its bottom).
fn block(id: u32, base: Vec3, size: Vec3, color: [f32; 4]) -> ObjectUpdate {
    prim(
        id,
        base + Vec3::Z * (size.z * 0.5),
        Quat::IDENTITY,
        size,
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te(color, 0, false, 0.0),
        ExtraParams::default(),
        "",
    )
}

pub fn events() -> Vec<NetEvent> {
    if !enabled() {
        return Vec::new();
    }
    // `near`: the same things at a third of the distance, for the contact
    // under the table and at the foot of the pole
    let near = std::env::var("AURORA_DEMO_SOFT_SHADOWS").is_ok_and(|v| v.trim() == "near");
    let scale = if near { 0.4 } else { 1.0 };
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let across = Vec3::new(-look.y, look.x, 0.0);
    let eye = std::env::var("AURORA_DEMO_POS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
        .filter(|p| p.len() >= 2)
        .map_or(Vec3::new(132.0, 126.0, 0.0), |p| Vec3::new(p[0], p[1], 0.0));
    // a pale plate over the ground to receive the shadows, level whatever
    // the terrain does, everything standing on it
    let centre = eye + (look * 10.0 - across * 2.0) * scale;
    let level = height(centre.x, centre.y) + 0.6;
    let ground = |p: Vec3| Vec3::new(p.x, p.y, level);
    let wood = [0.62, 0.45, 0.3, 1.0];
    let white = [0.9, 0.9, 0.88, 1.0];
    let mut objects = Vec::new();
    let mut id = FIRST_ID;
    let mut next = || {
        id += 1;
        id
    };
    objects.push(prim(
        next(),
        Vec3::new(centre.x, centre.y, level - 0.6),
        Quat::from_rotation_z(look.y.atan2(look.x)),
        Vec3::new(40.0, 40.0, 1.2),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.45, 0.45, 0.43, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    // the pergola: the roof 3 m up, its shadow on the ground beside it
    let pergola = eye + (look * 14.0 + across * 9.0) * scale;
    let top = ground(pergola);
    for (dx, dy) in [(-2.8, -1.3), (2.8, -1.3), (-2.8, 1.3), (2.8, 1.3)] {
        objects.push(block(
            next(),
            ground(pergola + across * dx + look * dy),
            Vec3::new(0.2, 0.2, 3.0),
            wood,
        ));
    }
    objects.push(prim(
        next(),
        top + Vec3::Z * 3.075,
        Quat::from_rotation_z(look.y.atan2(look.x)),
        Vec3::new(3.0, 6.0, 0.15),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te(wood, 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    // a tall pole: its shadow blurs towards the tip
    let pole = eye + (look * 14.0 + across * 2.0) * scale;
    objects.push(prim(
        next(),
        ground(pole) + Vec3::Z * 7.0,
        Quat::IDENTITY,
        Vec3::new(0.3, 0.3, 14.0),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
        te(white, 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    // a table: the top 0.75 m up on four legs
    let table = eye + (look * 12.0 - across * 3.0) * scale;
    for (dx, dy) in [(-0.55, -0.35), (0.55, -0.35), (-0.55, 0.35), (0.55, 0.35)] {
        objects.push(block(
            next(),
            ground(table + across * dx + look * dy),
            Vec3::new(0.06, 0.06, 0.72),
            wood,
        ));
    }
    objects.push(prim(
        next(),
        ground(table) + Vec3::Z * 0.75,
        Quat::from_rotation_z(look.y.atan2(look.x)),
        Vec3::new(0.8, 1.2, 0.06),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te(white, 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    // a sphere resting on the ground: a smooth surface at the terminator
    let sphere = eye + (look * 10.0 - across * 7.0) * scale;
    let mut ball = prim(
        next(),
        ground(sphere) + Vec3::Z * 0.75,
        Quat::IDENTITY,
        Vec3::splat(1.5),
        shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0),
        te(white, 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    ball.material = 3;
    objects.push(ball);
    log::info!("demo soft shadows: {} prims", objects.len());
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}
