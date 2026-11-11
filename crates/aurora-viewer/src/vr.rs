//! Pure controller locomotion and turning rules for native OpenXR VR.
//!
//! Conversion to `MoveInput` stays in `app.rs`, so controller acquisition,
//! user preferences and avatar/network control remain separate concerns.

use glam::{Vec2, Vec3};
use std::time::{Duration, Instant};

/// Removes a radial stick dead zone and rescales the remaining range.
pub fn dead_zone(stick: Vec2, threshold: f32) -> Vec2 {
    let threshold = threshold.clamp(0.0, 0.99);
    let length = stick.length();
    if !length.is_finite() || length <= threshold {
        return Vec2::ZERO;
    }
    stick / length * ((length - threshold) / (1.0 - threshold)).min(1.0)
}

/// Converts a left-stick sample to a world movement vector. Head pitch is
/// deliberately ignored, so looking up or down never starts flight.
pub fn movement_vector(stick: Vec2, head_forward: Vec3, dead_zone_threshold: f32) -> Vec3 {
    let stick = dead_zone(stick, dead_zone_threshold);
    let forward = head_forward.with_z(0.0).normalize_or(Vec3::X);
    // Aurora's horizontal axes are opposite the OpenXR stick's conventional
    // lateral sign: forward × up is the physical left direction here.
    let left = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
    (forward * stick.y + left * stick.x).clamp_length_max(1.0)
}

/// Directional digital controls consumed by Aurora's normal `AgentUpdate`
/// route. The server remains authoritative for actual avatar speed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MovementIntent {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
}

pub fn movement_intent(stick: Vec2) -> MovementIntent {
    MovementIntent {
        forward: stick.y > 0.15,
        back: stick.y < -0.15,
        left: stick.x < -0.15,
        right: stick.x > 0.15,
    }
}

/// State for snap-turn edge detection and its comfort cooldown.
#[derive(Debug, Clone, Copy, Default)]
pub struct SnapTurn {
    armed: bool,
    last: Option<Instant>,
}

impl SnapTurn {
    pub fn step(&mut self, stick_x: f32, angle: f32, cooldown: Duration, now: Instant) -> f32 {
        const ACTIVATE: f32 = 0.7;
        const RELEASE: f32 = 0.3;
        if stick_x.abs() <= RELEASE {
            self.armed = true;
            return 0.0;
        }
        if !self.armed || stick_x.abs() < ACTIVATE || self.last.is_some_and(|last| now.duration_since(last) < cooldown) {
            return 0.0;
        }
        self.armed = false;
        self.last = Some(now);
        angle.copysign(stick_x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_dead_zone_is_neutral_then_rescaled() {
        assert_eq!(dead_zone(Vec2::new(0.1, 0.1), 0.2), Vec2::ZERO);
        assert!((dead_zone(Vec2::Y * 0.6, 0.2).y - 0.5).abs() < 1e-6);
    }

    #[test]
    fn movement_follows_head_yaw_not_pitch() {
        let forward = movement_vector(Vec2::Y, Vec3::new(0.0, 0.5, 0.8), 0.1);
        assert!(forward.y > 0.99);
        assert!(forward.z.abs() < 1e-6);
    }

    #[test]
    fn snap_turn_requires_release_and_respects_cooldown() {
        let now = Instant::now();
        let mut turn = SnapTurn::default();
        assert_eq!(turn.step(0.8, 0.5, Duration::from_millis(250), now), 0.5);
        assert_eq!(turn.step(0.8, 0.5, Duration::from_millis(250), now + Duration::from_secs(1)), 0.0);
        assert_eq!(turn.step(0.0, 0.5, Duration::from_millis(250), now + Duration::from_secs(1)), 0.0);
        assert_eq!(turn.step(-0.8, 0.5, Duration::from_millis(250), now + Duration::from_secs(1)), -0.5);
    }
}
