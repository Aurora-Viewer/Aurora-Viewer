//! The photo window's free camera: the view leaves the avatar, which stays
//! where it is, and flies with the movement keys, the Controls window's
//! joysticks and the right button. Firestorm's nearest feature is the 3D
//! mouse flycam (LLViewerJoystick::moveFlycam, indra/newview, originally
//! LGPL 2.1): it starts from where the camera is, moves in the camera's own
//! frame and caps a long frame at 0.2 s, as here. Firestorm has no keyboard
//! free camera; speed, the slow / fast modifiers and the optional collisions
//! are Aurora's.

use glam::Vec3;
use std::ops::RangeInclusive;

/// Metres a second at speed 1 of the window (validated by the human).
pub const DEFAULT_SPEED: f32 = 2.0;
pub const SPEED_RANGE: RangeInclusive<f32> = 0.5..=20.0;
/// Shift held: this much faster; Ctrl held: this much slower.
pub const FAST: f32 = 4.0;
pub const SLOW: f32 = 0.25;
/// A frame longer than this moves as if it lasted this long
/// (moveFlycam's "ridiculously big movements").
pub const MAX_STEP_TIME: f32 = 0.2;
/// Never quite vertical: the view keeps a heading.
const PITCH_LIMIT: f32 = 1.55;
/// The look joystick held to its edge: radians a second (the orbit keys'
/// rate, LLAgentCamera ORBIT_RATE).
pub const LOOK_RATE: f32 = std::f32::consts::FRAC_PI_2;
/// How far from a wall or the ground a colliding camera stops.
pub const CLEARANCE: f32 = 0.3;

/// The photo window's switch and the flight's settings: for this window
/// only, never saved (the switch is off each time the window opens).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeSettings {
    pub on: bool,
    /// Metres a second.
    pub speed: f32,
    /// Through walls and the ground (on by default).
    pub no_collisions: bool,
}

impl Default for FreeSettings {
    fn default() -> Self {
        FreeSettings {
            on: false,
            speed: DEFAULT_SPEED,
            no_collisions: true,
        }
    }
}

/// Where the free camera is and where it looks (render space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeCamera {
    pub position: Vec3,
    /// Heading, radians, 0 along +X, counter-clockwise.
    pub yaw: f32,
    /// Radians, positive looking up.
    pub pitch: f32,
}

/// What moves the free camera this frame: each axis from -1 to 1 (keys at
/// 1, a joystick anywhere between).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FreeMove {
    pub forward: f32,
    pub left: f32,
    pub up: f32,
    pub fast: bool,
    pub slow: bool,
}

impl FreeMove {
    pub fn is_still(&self) -> bool {
        self.forward == 0.0 && self.left == 0.0 && self.up == 0.0
    }
}

impl FreeCamera {
    /// The camera where it is, looking where it looks (moveFlycam(reset)).
    pub fn from_view(position: Vec3, target: Vec3) -> FreeCamera {
        let d = (target - position).normalize_or(Vec3::X);
        FreeCamera {
            position,
            yaw: d.y.atan2(d.x),
            pitch: d.z.clamp(-1.0, 1.0).asin().clamp(-PITCH_LIMIT, PITCH_LIMIT),
        }
    }

    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cy * cp, sy * cp, sp)
    }

    /// The point looked at, one metre ahead.
    pub fn target(&self) -> Vec3 {
        self.position + self.forward()
    }

    /// How far it goes this frame: forward where it looks (pitch included,
    /// like the flycam), sideways level, up and down along the vertical.
    pub fn step(&self, m: FreeMove, speed: f32, dt: f32) -> Vec3 {
        let mut rate = speed.clamp(*SPEED_RANGE.start(), *SPEED_RANGE.end());
        if m.fast {
            rate *= FAST;
        }
        if m.slow {
            rate *= SLOW;
        }
        let left = Vec3::new(-self.yaw.sin(), self.yaw.cos(), 0.0);
        let dir = self.forward() * m.forward.clamp(-1.0, 1.0) + left * m.left.clamp(-1.0, 1.0) + Vec3::Z * m.up.clamp(-1.0, 1.0);
        // a diagonal goes no faster than a straight line
        let dir = if dir.length_squared() > 1.0 { dir.normalize() } else { dir };
        dir * rate * dt.clamp(0.0, MAX_STEP_TIME)
    }

    /// Turn the view: `yaw` to the left, `pitch` up, in radians.
    pub fn turn(&mut self, yaw: f32, pitch: f32) {
        self.yaw = (self.yaw + yaw).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// The right button dragged `dx`, `dy` pixels: the view follows the
    /// mouse as the right-drag orbit does.
    pub fn drag(&mut self, dx: f32, dy: f32, sensitivity: f32) {
        let k = super::DRAG_RATE * sensitivity;
        self.turn(-dx * k, -dy * k);
    }
}

/// The part of `delta` a colliding camera may go: each axis on its own, so
/// that it slides along a wall or the ground instead of stopping dead.
/// `free(from, dir, length)` is the distance clear of any obstacle along
/// `dir` (unit) from `from`, at most `length`.
pub fn collide(from: Vec3, delta: Vec3, mut free: impl FnMut(Vec3, Vec3, f32) -> f32) -> Vec3 {
    let mut at = from;
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        let d = delta.dot(axis);
        if d == 0.0 {
            continue;
        }
        let dir = axis * d.signum();
        let room = (free(at, dir, d.abs() + CLEARANCE) - CLEARANCE).max(0.0);
        at += dir * d.abs().min(room);
    }
    at - from
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn it_starts_where_the_camera_looks() {
        let c = FreeCamera::from_view(Vec3::new(10.0, 10.0, 30.0), Vec3::new(10.0, 20.0, 30.0));
        assert!(close(c.forward(), Vec3::Y));
        assert!(close(c.target(), Vec3::new(10.0, 11.0, 30.0)));
        let down = FreeCamera::from_view(Vec3::ZERO, Vec3::new(1.0, 0.0, -1.0));
        assert!((down.pitch + std::f32::consts::FRAC_PI_4).abs() < 1e-4);
    }

    #[test]
    fn keys_move_it_at_its_speed() {
        let c = FreeCamera::from_view(Vec3::ZERO, Vec3::X);
        let forward = FreeMove {
            forward: 1.0,
            ..Default::default()
        };
        assert!(close(c.step(forward, DEFAULT_SPEED, 0.1), Vec3::new(0.2, 0.0, 0.0)));
        // Shift four times faster, Ctrl four times slower
        let fast = FreeMove { fast: true, ..forward };
        assert!(close(c.step(fast, DEFAULT_SPEED, 0.1), Vec3::new(0.8, 0.0, 0.0)));
        let slow = FreeMove { slow: true, ..forward };
        assert!(close(c.step(slow, DEFAULT_SPEED, 0.1), Vec3::new(0.05, 0.0, 0.0)));
        // A / D slide level, E / C go straight up and down
        let left = FreeMove {
            left: 1.0,
            ..Default::default()
        };
        assert!(close(c.step(left, 1.0, 0.1), Vec3::Y * 0.1));
        let up = FreeMove {
            up: -1.0,
            ..Default::default()
        };
        assert!(close(c.step(up, 1.0, 0.1), -Vec3::Z * 0.1));
        // a diagonal no faster; the speed kept in its range; a long frame capped
        let both = FreeMove {
            forward: 1.0,
            left: 1.0,
            ..Default::default()
        };
        assert!((c.step(both, 1.0, 0.1).length() - 0.1).abs() < 1e-4);
        assert!((c.step(forward, 100.0, 0.1).length() - 2.0).abs() < 1e-4);
        assert!((c.step(forward, 1.0, 3.0).length() - MAX_STEP_TIME).abs() < 1e-4);
    }

    #[test]
    fn forward_follows_the_pitch_but_a_slide_stays_level() {
        let mut c = FreeCamera::from_view(Vec3::ZERO, Vec3::X);
        c.turn(0.0, 0.5);
        let forward = c.step(
            FreeMove {
                forward: 1.0,
                ..Default::default()
            },
            1.0,
            0.1,
        );
        assert!((forward.normalize().z - 0.5f32.sin()).abs() < 1e-4);
        let left = c.step(
            FreeMove {
                left: 1.0,
                ..Default::default()
            },
            1.0,
            0.1,
        );
        assert!(left.z.abs() < 1e-6);
        // never straight up
        c.turn(0.0, 10.0);
        assert!(c.pitch < std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn a_colliding_camera_slides_along_the_wall() {
        // a wall at x = 1: the camera stops short of it but still goes up
        let wall = |from: Vec3, dir: Vec3, length: f32| {
            if dir.x > 0.0 { (1.0 - from.x).min(length) } else { length }
        };
        let moved = collide(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.5), wall);
        assert!(close(moved, Vec3::new(1.0 - CLEARANCE, 0.0, 0.5)));
        // nothing in the way: the whole move
        let moved = collide(Vec3::ZERO, Vec3::new(-2.0, 1.0, 0.0), wall);
        assert!(close(moved, Vec3::new(-2.0, 1.0, 0.0)));
        // already against it: no step into it
        let moved = collide(Vec3::new(1.0 - CLEARANCE, 0.0, 0.0), Vec3::new(0.1, 0.0, 0.0), wall);
        assert!(close(moved, Vec3::ZERO));
    }
}
