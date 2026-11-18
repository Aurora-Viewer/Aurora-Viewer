//! The flycam a game controller flies. Port of LLFlycam
//! (indra/newview/llflycam.cpp) and of the input assembly of
//! LLAgentCamera::updateFlycam (indra/newview/llagentcamera.cpp), LL viewer
//! 26.5, originally LGPL 2.1.
//!
//! It moves the viewer's shared flycam (`joystick::flycam::Flycam`, local
//! +X forward, +Y left, +Z up, like LL's frame) with LL's own rates: 10 m/s,
//! turns scaled by the field of view, a zoom that crosses the whole range in
//! 5 s. The 3D mouse keeps its own rules (`joystick::flycam`).

use crate::joystick::flycam::Flycam;
use glam::{Quat, Vec3};

/// MAX_FLYCAM_SPEED (m/s at full deflection).
const MAX_SPEED: f32 = 10.0;
/// MIN_FLYCAM_SPEED_FACTOR, MAX_FLYCAM_SPEED_FACTOR.
const MIN_SPEED_FACTOR: f32 = 0.5;
const MAX_SPEED_FACTOR: f32 = 2.0;
const PITCH_RATE: f32 = 0.75;
const YAW_RATE: f32 = 0.90;
const ROLL_RATE: f32 = 0.90;
/// MIN_FIELD_OF_VIEW, MAX_FIELD_OF_VIEW, DEFAULT_FIELD_OF_VIEW (llcamera.h).
const MIN_VIEW: f32 = 5.0 * std::f32::consts::PI / 180.0;
const MAX_VIEW: f32 = 175.0 * std::f32::consts::PI / 180.0;
const DEFAULT_VIEW: f32 = 60.0 * std::f32::consts::PI / 180.0;
/// FULL_ZOOM_PERIOD: seconds from one end of the zoom to the other.
const FULL_ZOOM_PERIOD: f32 = 5.0;
/// MAX_DELTA_TIME: a slow frame does not jump.
const MAX_DT: f32 = 0.2;
const MIN_ORBIT_DISTANCE: f32 = 0.1;
/// The unroll's glide (startReset's duration in updateFlycam).
const UNROLL_SECONDS: f32 = 1.0;
/// MIN_UNROLL_SIN: ~0.6° off vertical, no level to find.
const MIN_UNROLL_SIN: f32 = 0.01;
/// The orbit's focal point when the ray hits nothing, and the ray's reach.
pub const ORBIT_DEFAULT_DISTANCE: f32 = 20.0;
pub const ORBIT_RAY_LENGTH: f32 = 512.0;

/// FlycamChannel.
pub const TRUCK: usize = 0;
pub const DOLLY: usize = 1;
pub const PAN: usize = 2;
pub const TILT: usize = 3;
pub const BOOM: usize = 4;
pub const ROLL: usize = 5;
pub const ZOOM: usize = 6;

#[derive(Debug, Clone, Copy)]
struct Reset {
    from: (Vec3, Quat),
    to: (Vec3, Quat),
    duration: f32,
    remaining: f32,
}

/// What LLFlycam keeps between frames beyond the camera itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct State {
    orbit: Option<Vec3>,
    reset: Option<Reset>,
}

/// One frame's request, already summed and clamped to -1..1.
#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub channels: [f32; 7],
    pub unroll: bool,
    /// The orbit modifier is held; on its press, the focal distance the
    /// viewer's ray found (`ORBIT_DEFAULT_DISTANCE` when nothing).
    pub orbit: Option<f32>,
    pub speed_factor: f32,
    pub allow_roll: bool,
}

impl State {
    pub fn orbiting(&self) -> bool {
        self.orbit.is_some()
    }

    /// Steps 4 to 6 of updateFlycam, then LLFlycam::integrate.
    pub fn step(&mut self, cam: &mut Flycam, input: Input, dt: f32) {
        let dt = dt.clamp(0.0, MAX_DT);
        let mut ch = input.channels;
        if input.unroll && input.allow_roll {
            self.start_unroll(cam);
        }
        match (input.orbit, self.orbit) {
            (Some(distance), None) => self.orbit = Some(cam.position + cam.forward() * distance),
            (None, Some(_)) => self.orbit = None,
            _ => {}
        }
        let mut radial = 0.0;
        if self.orbit.is_some() {
            // truck and dolly turn around the focal point and change the
            // distance to it (LLAgentCamera::updateFlycam)
            ch[TRUCK] = -ch[TRUCK];
            ch[DOLLY] = -ch[DOLLY];
            ch[PAN] = (ch[PAN] + ch[TRUCK]).clamp(-1.0, 1.0);
            ch[TRUCK] = 0.0;
            radial = ch[DOLLY];
            ch[DOLLY] = 0.0;
        }
        let speed = input.speed_factor.clamp(MIN_SPEED_FACTOR, MAX_SPEED_FACTOR);
        if let Some(r) = self.reset.as_mut() {
            r.remaining = (r.remaining - dt).max(0.0);
            let t = 1.0 - r.remaining / r.duration;
            cam.position = r.from.0.lerp(r.to.0, t);
            cam.rotation = nlerp(r.from.1, r.to.1, t);
            if r.remaining <= 0.0 {
                self.reset = None;
            }
            return;
        }
        let velocity = Vec3::new(ch[DOLLY], ch[TRUCK], ch[BOOM]) * MAX_SPEED * speed;
        let pitch_rate = ch[TILT] * speed * PITCH_RATE;
        let yaw_rate = ch[PAN] * speed * YAW_RATE;
        let roll_rate = if input.allow_roll { ch[ROLL] * speed * ROLL_RATE } else { 0.0 };
        let zoom_rate = ch[ZOOM] * (MAX_VIEW - MIN_VIEW) / FULL_ZOOM_PERIOD;
        let view = cam.fov;
        let view_ratio = view / DEFAULT_VIEW;

        let forward_up = (cam.rotation * Vec3::X).z.clamp(-1.0, 1.0);
        let sin_pitch = (1.0 - forward_up * forward_up).max(0.0).sqrt();
        let mut renormalize = false;
        let yaw = dt * yaw_rate * view_ratio * sin_pitch.max(0.1);
        if yaw != 0.0 {
            // a body-frame turn (LL's `dQ * mRotation`)
            cam.rotation *= Quat::from_rotation_z(yaw);
            if !input.allow_roll {
                level(cam);
            }
            renormalize = true;
        }
        let pitch = dt * pitch_rate * view_ratio;
        let to_vertical = 1.0 - forward_up.abs();
        if pitch != 0.0 && !(forward_up > 0.0 && pitch < -to_vertical) && !(forward_up < 0.0 && pitch > to_vertical) {
            cam.rotation *= Quat::from_rotation_y(pitch);
            renormalize = true;
        }
        let roll = dt * roll_rate * view_ratio;
        if roll != 0.0 {
            cam.rotation *= Quat::from_rotation_x(roll);
            renormalize = true;
        }
        if let Some(focal) = self.orbit {
            let distance = (cam.position - focal).length();
            let distance = (distance * (1.0 + dt * radial)).max(MIN_ORBIT_DISTANCE);
            cam.position = focal - (cam.rotation * Vec3::X) * distance;
        }
        if velocity.length_squared() > 0.0 {
            let translation = cam.rotation * (velocity * dt);
            cam.position += translation;
            if let Some(focal) = self.orbit.as_mut() {
                *focal += translation;
            }
        }
        if zoom_rate != 0.0 {
            cam.fov = (view - dt * zoom_rate).clamp(MIN_VIEW, MAX_VIEW);
        }
        if renormalize {
            cam.rotation = cam.rotation.normalize();
        }
    }

    /// The Unroll of updateFlycam: same place, same forward, up taken from
    /// the world's, over one second.
    fn start_unroll(&mut self, cam: &Flycam) {
        let forward = (cam.rotation * Vec3::X).normalize_or(Vec3::X);
        let up = Vec3::Z - forward * Vec3::Z.dot(forward);
        if up.length_squared() <= MIN_UNROLL_SIN * MIN_UNROLL_SIN {
            return;
        }
        let up = up.normalize();
        let left = up.cross(forward);
        let target = Quat::from_mat3(&glam::Mat3::from_cols(forward, left, up)).normalize();
        self.orbit = None;
        self.reset = Some(Reset {
            from: (cam.position, cam.rotation),
            to: (cam.position, target),
            duration: UNROLL_SECONDS,
            remaining: UNROLL_SECONDS,
        });
    }
}

/// The un-roll after a yaw: keep forward, rebuild up in the plane of forward
/// and the world's up (LLFlycam::integrate, !mAllowRoll).
fn level(cam: &mut Flycam) {
    let forward = (cam.rotation * Vec3::X).normalize_or(Vec3::X);
    let up = Vec3::Z - forward * Vec3::Z.dot(forward);
    if up.length_squared() > MIN_UNROLL_SIN * MIN_UNROLL_SIN {
        let up = up.normalize();
        let left = up.cross(forward);
        cam.rotation = Quat::from_mat3(&glam::Mat3::from_cols(forward, left, up)).normalize();
    }
}

/// LL's nlerp: the shorter way, normalised.
fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    (a * (1.0 - t) + b * t).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Flycam {
        Flycam::from_direction(Vec3::new(128.0, 128.0, 30.0), Vec3::X, Vec3::Z, DEFAULT_VIEW)
    }

    fn input(channel: usize, value: f32) -> Input {
        let mut i = Input {
            speed_factor: 1.0,
            ..Default::default()
        };
        i.channels[channel] = value;
        i
    }

    #[test]
    fn full_dolly_moves_ten_metres_a_second() {
        let (mut c, mut s) = (cam(), State::default());
        s.step(&mut c, input(DOLLY, 1.0), 0.1);
        assert!((c.position.x - 129.0).abs() < 1e-4, "{:?}", c.position);
        let mut i = input(TRUCK, 1.0);
        i.speed_factor = 2.0;
        s.step(&mut c, i, 0.1);
        assert!(
            (c.position.y - 130.0).abs() < 1e-4,
            "truck goes left, twice as fast: {:?}",
            c.position
        );
    }

    #[test]
    fn a_slow_frame_is_capped() {
        let (mut c, mut s) = (cam(), State::default());
        s.step(&mut c, input(BOOM, 1.0), 1.0);
        assert!((c.position.z - 32.0).abs() < 1e-4, "0.2 s at most: {:?}", c.position);
    }

    #[test]
    fn pan_turns_left_by_the_view_ratio() {
        let (mut c, mut s) = (cam(), State::default());
        s.step(&mut c, input(PAN, 1.0), 0.1);
        let f = c.forward();
        let angle = f.y.atan2(f.x);
        assert!((angle - 0.09).abs() < 1e-4, "0.9 rad/s at 60°: {angle}");
        c.set_fov(DEFAULT_VIEW / 2.0);
        let before = angle;
        s.step(&mut c, input(PAN, 1.0), 0.1);
        let f = c.forward();
        assert!((f.y.atan2(f.x) - before - 0.045).abs() < 1e-4, "half the view, half the turn");
    }

    #[test]
    fn roll_needs_allow_roll() {
        let (mut c, mut s) = (cam(), State::default());
        s.step(&mut c, input(ROLL, 1.0), 0.1);
        assert!(c.up().y.abs() < 1e-5, "no roll without AllowRoll");
        let mut i = input(ROLL, 1.0);
        i.allow_roll = true;
        s.step(&mut c, i, 0.1);
        assert!(c.up().y.abs() > 0.05, "rolled: {:?}", c.up());
    }

    #[test]
    fn zoom_in_narrows_without_the_speed_factor() {
        let (mut c, mut s) = (cam(), State::default());
        let mut i = input(ZOOM, 1.0);
        i.speed_factor = 2.0;
        s.step(&mut c, i, 0.1);
        let expected = DEFAULT_VIEW - 0.1 * (MAX_VIEW - MIN_VIEW) / 5.0;
        assert!((c.fov - expected).abs() < 1e-5, "{} vs {expected}", c.fov);
    }

    #[test]
    fn orbit_keeps_facing_its_focal_point() {
        let (mut c, mut s) = (cam(), State::default());
        let mut i = input(TRUCK, 1.0);
        i.orbit = Some(ORBIT_DEFAULT_DISTANCE);
        for _ in 0..10 {
            s.step(&mut c, i, 0.1);
        }
        let focal = Vec3::new(148.0, 128.0, 30.0);
        assert!(((c.position - focal).length() - 20.0).abs() < 1e-2, "same distance");
        assert!(c.forward().dot((focal - c.position).normalize()) > 0.999, "faces it");
        i.orbit = None;
        s.step(&mut c, i, 0.1);
        assert!(!s.orbiting());
    }

    #[test]
    fn unroll_glides_level_in_one_second() {
        let (mut c, mut s) = (cam(), State::default());
        c.rotation *= Quat::from_rotation_x(0.5);
        let mut i = Input {
            speed_factor: 1.0,
            unroll: true,
            allow_roll: true,
            ..Default::default()
        };
        s.step(&mut c, i, 0.0);
        i.unroll = false;
        // (in frames of 0.1 s: a frame counts for 0.2 s at most)
        for _ in 0..5 {
            s.step(&mut c, i, 0.1);
        }
        assert!(c.up().y.abs() > 0.01, "half way");
        for _ in 0..5 {
            s.step(&mut c, i, 0.1);
        }
        assert!(c.up().y.abs() < 1e-4, "level: {:?}", c.up());
        let mut i = Input {
            speed_factor: 1.0,
            unroll: true,
            ..Default::default()
        };
        c.rotation *= Quat::from_rotation_x(0.5);
        i.allow_roll = false;
        s.step(&mut c, i, 0.5);
        assert!(c.up().y.abs() > 0.1, "no unroll without AllowRoll");
    }
}
