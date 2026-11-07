//! Demo of Face nearest avatar and the Compass, without a grid:
//! `AURORA_DEMO_AVATAR_ALIGN=<mode>`. Every turn logs its bearing before
//! and after (`avatar align: bearing …° -> …°`).
//!
//! The steps come after the loading fade (frame 620).
//!
//! - `nearest`: the agent faces east at frame 650, then the toolbar's
//!   `face_nearest_avatar` runs at frame 800 (a capture at 750 is the
//!   before, at 1000 the after).
//! - `full` (or `1`): the Compass opens at frame 650, then turns to the
//!   east by its ring (750), 45° to the left (810), to the nearest cardinal
//!   (870, nothing to do) and to the nearest avatar by its button (930).
//! - `mini`: the compact Compass opens at frame 650 (the layout is saved,
//!   as its "Full" button would), then turns 1° to the right at 750.

use crate::ui::avatar_align::Action;
use std::sync::OnceLock;

/// The frame the Compass opens (`full`, `mini`).
pub const OPEN_FRAME: u64 = 650;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// A toolbar command, as its button runs it.
    Toolbar(&'static str),
    /// A button or a click of the Compass.
    Window(Action),
    /// The agent's yaw set, to start from a known heading.
    Yaw(f32),
}

pub fn mode() -> Option<String> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| {
        let mode = std::env::var("AURORA_DEMO_AVATAR_ALIGN").ok()?;
        match mode.trim() {
            "" | "0" => None,
            "1" => Some("full".into()),
            other => Some(other.to_owned()),
        }
    })
    .clone()
}

/// Whether the Compass opens in this mode.
pub fn opens(mode: &str) -> bool {
    matches!(mode, "full" | "mini")
}

/// What runs at `frame` in `mode`.
pub fn step(mode: &str, frame: u64) -> Option<Step> {
    match (mode, frame) {
        ("nearest", 650) => Some(Step::Yaw(0.0)),
        ("nearest", 800) => Some(Step::Toolbar("face_nearest_avatar")),
        ("full", 750) => Some(Step::Window(Action::Bearing(90.0))),
        ("full", 810) => Some(Step::Window(Action::Rotate(-45.0))),
        ("full", 870) => Some(Step::Window(Action::NearestCardinal)),
        ("full", 930) => Some(Step::Window(Action::FaceNearest)),
        ("mini", 750) => Some(Step::Window(Action::Rotate(1.0))),
        _ => None,
    }
}
