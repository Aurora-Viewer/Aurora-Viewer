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

/// The held walking keys (or Controls window buttons, or a joystick's
/// digital walk) as a stick, so they move in VR the way the left stick does.
pub fn keys_stick(forward: bool, back: bool, left: bool, right: bool) -> Vec2 {
    let axis = |plus: bool, minus: bool| plus as i8 as f32 - minus as i8 as f32;
    Vec2::new(axis(right, left), axis(forward, back)).normalize_or_zero()
}

/// The teleport arc's sample points: thrown from `origin` along `dir` at
/// 8 m/s under gravity, `count` points 0.06 s apart (Z up).
pub fn arc_points(origin: Vec3, dir: Vec3, count: usize) -> impl Iterator<Item = Vec3> {
    const SPEED: f32 = 8.0;
    const STEP: f32 = 0.06;
    let velocity = dir.normalize_or(Vec3::X) * SPEED;
    (0..count).map(move |i| {
        let t = i as f32 * STEP;
        origin + velocity * t + Vec3::Z * (-0.5 * 9.81 * t * t)
    })
}

/// Teleport aiming on the left stick: pushed forward past 0.6 aims,
/// let back under 0.3 lands.
pub fn teleport_aiming(was: bool, stick_y: f32) -> bool {
    if was { stick_y > 0.3 } else { stick_y > 0.6 }
}

/// The avatar's body turning after the head while standing: once the head
/// is more than 50° away it turns (at most 3 rad/s) until within 5°.
#[derive(Debug, Clone, Copy, Default)]
pub struct BodyFollow {
    turning: bool,
}

impl BodyFollow {
    const START: f32 = 50.0 * std::f32::consts::PI / 180.0;
    const STOP: f32 = 5.0 * std::f32::consts::PI / 180.0;
    const RATE: f32 = 3.0;

    /// The body's new yaw from its yaw and the head's, radians.
    pub fn step(&mut self, body: f32, head: f32, dt: f32) -> f32 {
        let diff = (head - body + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        if diff.abs() > Self::START {
            self.turning = true;
        }
        if !self.turning || !diff.is_finite() {
            return body;
        }
        if diff.abs() < Self::STOP {
            self.turning = false;
            return body;
        }
        body + diff.signum() * (Self::RATE * dt.max(0.0)).min(diff.abs())
    }

    pub fn reset(&mut self) {
        self.turning = false;
    }
}

/// How far the avatar's eyes lead its centre along its facing (the
/// first-person camera's standing eye, `camera::mouselook_eye`).
pub const EYE_LEAD: f32 = 0.12;

/// The world shift of the VR view: `forward` metres along the avatar's
/// level facing, `height` metres up.
pub fn view_offset(facing: Vec3, forward: f32, height: f32) -> Vec3 {
    facing.with_z(0.0).normalize_or_zero() * forward + Vec3::Z * height
}

/// A button held long enough, for actions that must not fire on a brush
/// (recentring on the thumbstick click).
#[derive(Debug, Clone, Copy, Default)]
pub struct Hold {
    since: Option<Instant>,
    fired: bool,
}

impl Hold {
    /// True once, when `down` has been held for `hold`.
    pub fn step(&mut self, down: bool, hold: Duration, now: Instant) -> bool {
        if !down {
            *self = Self::default();
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if !self.fired && now.duration_since(since) >= hold {
            self.fired = true;
            return true;
        }
        false
    }
}

/// The comfort vignette eased toward `target`: closing in about 0.1 s,
/// opening in about 0.25 s.
pub fn ease_vignette(current: f32, target: f32, dt: f32) -> f32 {
    let rate = if target > current { 10.0 } else { 4.0 };
    let t = (dt.max(0.0) * rate).min(1.0);
    (current + (target - current) * t).clamp(0.0, 1.0)
}

/// Trigger hysteresis: pressed past 0.7, released only under 0.35.
pub fn trigger_held(was_held: bool, value: f32) -> bool {
    if was_held { value > 0.35 } else { value >= 0.7 }
}

/// State for snap-turn edge detection and its comfort cooldown.
#[derive(Debug, Clone, Copy)]
pub struct SnapTurn {
    armed: bool,
    last: Option<Instant>,
}

/// Armed from the start: the first flick turns, without first centring the
/// stick.
impl Default for SnapTurn {
    fn default() -> Self {
        Self { armed: true, last: None }
    }
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
    fn keys_make_a_unit_stick_and_opposites_cancel() {
        assert_eq!(keys_stick(true, false, false, false), Vec2::Y);
        assert_eq!(keys_stick(true, true, false, false), Vec2::ZERO);
        let diagonal = keys_stick(true, false, true, false);
        assert!((diagonal.length() - 1.0).abs() < 1e-6 && diagonal.x < 0.0 && diagonal.y > 0.0);
        // a left key moves the same way as the stick pushed left
        let left = movement_vector(keys_stick(false, false, true, false), Vec3::X, 0.0);
        let stick = movement_vector(Vec2::new(-1.0, 0.0), Vec3::X, 0.0);
        assert!(left.distance(stick) < 1e-6);
    }

    #[test]
    fn view_offset_turns_with_the_facing() {
        let east = view_offset(Vec3::X, 0.1, 0.2);
        assert!(east.distance(Vec3::new(0.1, 0.0, 0.2)) < 1e-6);
        // a pitched facing still moves the view level
        let north = view_offset(Vec3::new(0.0, 0.5, 0.8), 0.1, 0.0);
        assert!(north.distance(Vec3::new(0.0, 0.1, 0.0)) < 1e-6);
    }

    #[test]
    fn vignette_closes_faster_than_it_opens_and_stays_in_range() {
        let closing = ease_vignette(0.0, 1.0, 0.05);
        let opening = 1.0 - ease_vignette(1.0, 0.0, 0.05);
        assert!(closing > opening && closing <= 1.0);
        assert_eq!(ease_vignette(0.3, 0.0, 10.0), 0.0);
    }

    #[test]
    fn arc_rises_then_falls_forward() {
        let points: Vec<_> = arc_points(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0), 24).collect();
        assert_eq!(points.len(), 24);
        assert_eq!(points[0], Vec3::ZERO);
        let top = points.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(top > 0.5 && points[23].z < top);
        assert!(points.windows(2).all(|w| w[1].x > w[0].x));
    }

    #[test]
    fn teleport_aim_has_hysteresis() {
        assert!(!teleport_aiming(false, 0.5));
        assert!(teleport_aiming(false, 0.7));
        assert!(teleport_aiming(true, 0.5));
        assert!(!teleport_aiming(true, 0.2));
    }

    #[test]
    fn body_turns_after_the_head_past_50_degrees_then_settles() {
        let mut follow = BodyFollow::default();
        // 30° away: the body stays
        assert_eq!(follow.step(0.0, 30f32.to_radians(), 0.1), 0.0);
        // 90° away: it turns toward the head, no further than the rate
        let mut body = follow.step(0.0, 90f32.to_radians(), 0.1);
        assert!((body - 0.3).abs() < 1e-5);
        for _ in 0..20 {
            body = follow.step(body, 90f32.to_radians(), 0.1);
        }
        assert!((body - 90f32.to_radians()).abs() < 5f32.to_radians());
        // across the ±180° seam it takes the short way
        let mut follow = BodyFollow::default();
        let body = follow.step(170f32.to_radians(), -110f32.to_radians(), 0.1);
        assert!(body > 170f32.to_radians());
    }

    #[test]
    fn hold_fires_once_after_its_time() {
        let now = Instant::now();
        let hold = Duration::from_millis(600);
        let mut h = Hold::default();
        assert!(!h.step(true, hold, now));
        assert!(!h.step(true, hold, now + Duration::from_millis(300)));
        assert!(h.step(true, hold, now + Duration::from_millis(700)));
        assert!(!h.step(true, hold, now + Duration::from_millis(900)));
        assert!(!h.step(false, hold, now + Duration::from_secs(1)));
        assert!(!h.step(true, hold, now + Duration::from_secs(2)));
    }

    #[test]
    fn trigger_clicks_once_through_jitter() {
        let mut held = false;
        let mut clicks = 0;
        for value in [0.0, 0.72, 0.68, 0.71, 0.5, 0.69, 0.3, 0.75] {
            let now = trigger_held(held, value);
            clicks += (now && !held) as u32;
            held = now;
        }
        assert_eq!(clicks, 2);
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
