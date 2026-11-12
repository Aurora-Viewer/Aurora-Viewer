//! Offline reproduction of #2138 using raw sphere parameters, without assets.

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::params::*;
use glam::{Quat, Vec3};

pub fn events() -> Vec<NetEvent> {
    if std::env::var("AURORA_DEMO_TORTURED_PRIMS").as_deref() != Ok("1") {
        return Vec::new();
    }
    let origin = Vec3::new(100.0, 100.0, height(100.0, 100.0) + 2.0);
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let across = Vec3::new(-look.y, look.x, 0.0);
    let raw = RawShape {
        path_curve: LL_PCODE_PATH_CIRCLE,
        profile_curve: LL_PCODE_PROFILE_CIRCLE_HALF,
        path_scale_x: 100,
        path_scale_y: 0,
        path_twist: -50,
        path_twist_begin: -50,
        profile_hollow: 30000,
        ..Default::default()
    };
    let lantern = raw.to_params();
    let oval = RawShape { path_scale_y: 100, ..raw }.to_params();
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let torus = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, 175, 0, 0);
    // Left to right: the old clamped oval, the decoded lantern, an ordinary
    // sphere and a torus. The first two use the report's scale proportions,
    // enlarged three times for inspection; local X is the vertical axis.
    let objects = [
        (oval, [0.55, 0.55, 0.6, 1.0]),
        (lantern, [0.95, 0.3, 0.1, 1.0]),
        (sphere, [0.25, 0.65, 0.9, 1.0]),
        (torus, [0.3, 0.8, 0.5, 1.0]),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (volume, color))| {
        prim(
            97_380 + index as u32,
            origin + look * 6.0 - across * (index as f32 - 1.5) * 3.0,
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::new(0.56313, 0.26975, 0.26975) * 3.0,
            volume,
            te(color, 0, false, 0.0),
            Default::default(),
            "",
        )
    })
    .collect();
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}
