//! A cube with the ten lines of hover text of issue #2284, close enough to
//! the start position to measure their spacing (AURORA_DEMO_HOVER_LINES).

use super::{HANDLE, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};

const TEXT: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10";

pub fn scenario() -> bool {
    std::env::var("AURORA_DEMO_HOVER_LINES").is_ok()
}

/// The cube, left of the view axis of the demo's start position (see
/// `events`), clear of the windows open at the start.
pub fn events() -> Vec<NetEvent> {
    let (start, ahead) = (Vec3::new(132.0, 126.0, 0.0), Vec3::new(0.8, 0.4, 0.0).normalize());
    let right = Vec3::new(ahead.y, -ahead.x, 0.0);
    let p = start + ahead * 5.0 - right * 3.2;
    let object = prim(
        120_200,
        Vec3::new(p.x, p.y, floor_at(p.x, p.y) + 0.25),
        Quat::IDENTITY,
        Vec3::splat(0.5),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.55, 0.36, 0.96, 1.0], 1, false, 0.0),
        ExtraParams::default(),
        TEXT,
    );
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object],
    }]
}
