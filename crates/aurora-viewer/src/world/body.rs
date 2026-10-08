//! Rendered body orientation of avatars, port of
//! `LLVOAvatar::updateOrientation` (indra/newview/llvoavatar.cpp, Linden
//! Research and Firestorm, originally LGPL 2.1).
//!
//! The rotation an avatar sends (its agent frame, the camera direction) is
//! not what is drawn: the body turns toward the direction of travel once it
//! moves, holds its heading when standing until the agent frame is more than
//! ~45° away, and always follows with a lag. Our own avatar's turn sets
//! AGENT_CONTROL_TURN_LEFT / RIGHT so the simulator plays the turn animations.

use glam::{Mat3, Quat, Vec3};
use uuid::Uuid;

/// sit_ground, sit_ground_constrained, standup (AGENT_NO_ROTATE_ANIMS).
pub const NO_ROTATE_ANIMS: [Uuid; 3] = [
    Uuid::from_u128(0x1c7600d6_661f_b87b_efe2_d7421eb93c86),
    Uuid::from_u128(0x1a2bd58e_87ff_0df8_0b4c_53e047b0bb6e),
    Uuid::from_u128(0x3da1d753_028a_5446_24f3_9c9b856d9422),
];

const PELVIS_LAG_FLYING: f32 = 0.22;
const PELVIS_LAG_WALKING: f32 = 0.4;
const PELVIS_LAG_MOUSELOOK: f32 = 0.15;
const MOUSELOOK_PELVIS_FOLLOW_FACTOR: f32 = 0.5;
/// AvatarRotateThresholdSlow / Fast (degrees).
const ROTATE_THRESHOLD_SLOW: f32 = 60.0;
const ROTATE_THRESHOLD_FAST: f32 = 2.0;

fn clamp_rescale(x: f32, in0: f32, in1: f32, out0: f32, out1: f32) -> f32 {
    let t = ((x - in0) / (in1 - in0)).clamp(0.0, 1.0);
    out0 + (out1 - out0) * t
}

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, 0.0).normalize_or_zero()
}

/// What updateOrientation needs to know about one avatar this frame.
pub struct Input {
    /// Forward axis of the avatar's rotation (our agent frame for us).
    pub prim_dir: Vec3,
    pub velocity: Vec3,
    pub in_air: bool,
    /// Our avatar in mouselook: the camera's look direction.
    pub mouselook_at: Option<Vec3>,
    pub flying: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Body {
    pub rotation: Quat,
    turning: bool,
    speed_accum: f32,
}

impl Body {
    pub fn new(prim_dir: Vec3) -> Body {
        let fwd = match flat(prim_dir) {
            v if v == Vec3::ZERO => Vec3::X,
            v => v,
        };
        Body {
            rotation: Quat::from_rotation_z(fwd.y.atan2(fwd.x)),
            turning: false,
            speed_accum: 0.0,
        }
    }

    /// Advance one frame. Returns the turn direction while turning
    /// (+1 left, -1 right), for our own control flags.
    pub fn update(&mut self, i: &Input, dt: f32) -> i8 {
        let dt = dt.clamp(0.01, 0.2);
        let speed = Vec3::new(i.velocity.x, i.velocity.y, 0.0).length();
        self.speed_accum = self.speed_accum * 0.95 + speed * 0.05;
        let up = Vec3::Z;
        let prim = match flat(i.prim_dir) {
            v if v == Vec3::ZERO => self.rotation * Vec3::X,
            v => v,
        };
        let vel_dir = i.velocity.normalize_or_zero();
        // Firestorm default (FSDisableTurningAroundWhenWalkingBackwards off):
        // walking backwards turns the avatar around to face where it goes
        let mut fwd = prim.lerp(vel_dir, clamp_rescale(speed, 0.5, 2.0, 0.0, 1.0));
        if let Some(at) = i.mouselook_at {
            if i.flying {
                fwd = at;
            } else {
                let at = flat(at);
                let dot = fwd.dot(at);
                if dot < 0.0 {
                    fwd = (fwd - 2.0 * at * dot).normalize_or_zero();
                }
            }
        }
        let pelvis = self.rotation * Vec3::X;
        let mut threshold = clamp_rescale(speed, 0.1, 1.0, ROTATE_THRESHOLD_SLOW, ROTATE_THRESHOLD_FAST);
        if i.mouselook_at.is_some() {
            threshold *= MOUSELOOK_PELVIS_FOLLOW_FACTOR;
        }
        threshold = threshold.to_radians();
        let angle = if fwd.length_squared() > 1e-8 {
            pelvis.angle_between(fwd)
        } else {
            0.0
        };
        if !self.turning && angle > threshold * 0.75 {
            self.turning = true;
        }
        if self.turning {
            // tighter threshold when turning, scaled for frames under 16 ms
            threshold *= 0.4;
            if dt < 0.016 {
                threshold *= dt / 0.016;
            }
        }
        if angle < threshold {
            self.turning = false;
        }
        fwd += (pelvis - fwd) * clamp_rescale(angle, threshold * 0.75, threshold, 1.0, 0.0);
        let left = up.cross(fwd).normalize_or_zero();
        if left == Vec3::ZERO {
            return 0;
        }
        let fwd = left.cross(up);
        let target = Quat::from_mat3(&Mat3::from_cols(fwd, left, up));
        let turn = if self.turning {
            if fwd.cross(pelvis).dot(up) > 0.0 { -1 } else { 1 }
        } else {
            0
        };
        let lag = if i.mouselook_at.is_some() {
            PELVIS_LAG_MOUSELOOK
        } else if i.in_air {
            PELVIS_LAG_FLYING * clamp_rescale(self.speed_accum, 0.0, 15.0, 3.0, 1.0)
        } else {
            PELVIS_LAG_WALKING
        };
        let u = (dt / lag).clamp(0.0, 1.0);
        self.rotation = self.rotation.slerp(target, u).normalize();
        turn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaw(q: Quat) -> f32 {
        let f = q * Vec3::X;
        f.y.atan2(f.x).to_degrees()
    }

    #[test]
    fn standing_holds_then_turns_and_walking_faces_travel() {
        let still = |deg: f32| Input {
            prim_dir: Quat::from_rotation_z(deg.to_radians()) * Vec3::X,
            velocity: Vec3::ZERO,
            in_air: false,
            mouselook_at: None,
            flying: false,
        };
        let mut b = Body::new(Vec3::X);
        // standing: a 30° camera turn does not move the body
        for _ in 0..120 {
            assert_eq!(b.update(&still(30.0), 1.0 / 60.0), 0);
        }
        assert!(yaw(b.rotation).abs() < 0.5);
        // past ~45°: the body turns (to the left) until it is within the
        // turning threshold (60° x 0.4 = 24°), the head does the rest
        let mut turned = false;
        for _ in 0..240 {
            turned |= b.update(&still(80.0), 1.0 / 60.0) == 1;
        }
        assert!(turned);
        let y = yaw(b.rotation);
        assert!(y > 50.0 && y < 62.0, "{y}");
        // walking backwards (3 m/s toward -X while facing +X): turns around
        let mut b = Body::new(Vec3::X);
        let back = Input {
            prim_dir: Vec3::X,
            velocity: Vec3::new(-3.0, 0.0, 0.0),
            in_air: false,
            mouselook_at: None,
            flying: false,
        };
        for _ in 0..240 {
            b.update(&back, 1.0 / 60.0);
        }
        assert!(yaw(b.rotation).abs() > 170.0, "{}", yaw(b.rotation));
        // strafing left: faces left
        let mut b = Body::new(Vec3::X);
        let side = Input {
            velocity: Vec3::new(0.0, 3.0, 0.0),
            ..back
        };
        for _ in 0..240 {
            b.update(&side, 1.0 / 60.0);
        }
        assert!((yaw(b.rotation) - 90.0).abs() < 3.0, "{}", yaw(b.rotation));
    }
}
