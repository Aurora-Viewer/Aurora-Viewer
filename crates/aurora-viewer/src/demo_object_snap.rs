//! AURORA_DEMO_OBJECT_SNAP: boxes to snap together with the build tools' "Snap to
//! objects" (build/object_snap.rs), 6 m along the start view.
//!
//! - a 4 x 4 m slab on the ground (`object:87700`);
//! - a blue 1 m cube floating 0.3 m above it (`object:87701`): drag its Z
//!   arrow down to sit it on the slab, its X arrow to touch the orange cube;
//! - an orange 1 m cube on the slab, 0.8 m to its +X (`object:87702`);
//! - a green box turned 30° about Z (`object:87703`), a purple plank tilted
//!   20° about Y (`object:87704`, the cube's corner rests on its slope) and
//!   a 2 m pillar (`object:87705`, flush tops and sides);
//! - to the left, a hollow room (`object:87706`, walls only) with a yellow
//!   cube inside its box (`object:87707`): it stops at the inner walls, as on
//!   the floor of a mesh house; and a cylinder (`object:87708`) met on its
//!   round side, not its box's corners.
//!
//! `1` adds the boxes; `on` also chooses Snap › Objects when
//! AURORA_DEMO_BUILD opens the tools (`build::demo::apply_object_snap`).
//! From the start position, or AURORA_DEMO_POS="x,y" (open land: "80,100").

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PROFILE_SQUARE};
use glam::{Quat, Vec3};

pub const FIRST_ID: u32 = 87_700;

pub fn enabled() -> bool {
    std::env::var("AURORA_DEMO_OBJECT_SNAP").is_ok_and(|v| matches!(v.trim(), "1" | "on"))
}

pub fn events() -> Vec<NetEvent> {
    if !enabled() {
        return Vec::new();
    }
    let eye = std::env::var("AURORA_DEMO_POS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
        .filter(|p| p.len() >= 2)
        .map_or(Vec3::new(132.0, 126.0, 0.0), |p| Vec3::new(p[0], p[1], 0.0));
    events_at(eye)
}

/// The boxes, seen from `eye` (its height is ignored).
pub fn events_at(eye: Vec3) -> Vec<NetEvent> {
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let at = eye + look * 6.0;
    let ground = height(at.x, at.y);
    // the slab's top
    let top = ground + 0.2;
    let place = |dx: f32, dy: f32, z: f32| Vec3::new(at.x + dx, at.y + dy, z);
    let boxes = [
        (
            place(0.0, 0.0, ground + 0.1),
            Quat::IDENTITY,
            Vec3::new(4.0, 4.0, 0.2),
            [0.55, 0.56, 0.6, 1.0],
        ),
        (place(-1.2, -0.8, top + 0.8), Quat::IDENTITY, Vec3::ONE, [0.25, 0.45, 0.95, 1.0]),
        (place(0.6, -0.8, top + 0.5), Quat::IDENTITY, Vec3::ONE, [0.95, 0.55, 0.2, 1.0]),
        (
            place(0.4, 1.0, top + 0.4),
            Quat::from_rotation_z(30f32.to_radians()),
            Vec3::new(1.0, 0.6, 0.8),
            [0.3, 0.8, 0.45, 1.0],
        ),
        (
            place(-1.1, 1.1, top + 0.6),
            Quat::from_rotation_y(20f32.to_radians()),
            Vec3::new(1.6, 0.5, 0.1),
            [0.65, 0.4, 0.9, 1.0],
        ),
        (
            place(1.5, 0.3, top + 1.0),
            Quat::IDENTITY,
            Vec3::new(0.4, 0.4, 2.0),
            [0.9, 0.85, 0.75, 1.0],
        ),
    ];
    let square = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut objects: Vec<_> = boxes
        .into_iter()
        .zip(FIRST_ID..)
        .map(|((pos, rot, scale, color), id)| prim(id, pos, rot, scale, square, te(color, 0, false, 0.0), Default::default(), ""))
        .collect();
    // to the left: a room (a box prim hollowed at 90 %, walls only) with a
    // cube inside its box, and a cylinder; their surfaces, not their boxes
    let left = |dx: f32, dy: f32, z: f32| Vec3::new(at.x + dx - 1.6, at.y + dy + 3.4, z);
    objects.push(prim(
        FIRST_ID + 6,
        left(0.0, 0.0, ground + 1.0),
        Quat::IDENTITY,
        Vec3::new(3.0, 3.0, 2.0),
        // profile_hollow in 1/50000ths: 90 %
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 45000, 0),
        te([0.8, 0.75, 0.65, 1.0], 0, false, 0.0),
        Default::default(),
        "",
    ));
    objects.push(prim(
        FIRST_ID + 7,
        left(-0.3, 0.2, ground + 0.4),
        Quat::IDENTITY,
        Vec3::splat(0.8),
        square,
        te([0.95, 0.85, 0.2, 1.0], 0, false, 0.0),
        Default::default(),
        "",
    ));
    objects.push(prim(
        FIRST_ID + 8,
        left(2.6, -1.2, ground + 0.6),
        Quat::IDENTITY,
        Vec3::new(1.0, 1.0, 1.2),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
        te([0.4, 0.75, 0.85, 1.0], 0, false, 0.0),
        Default::default(),
        "",
    ));
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}
