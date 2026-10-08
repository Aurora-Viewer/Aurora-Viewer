//! Our own avatar: simulator-driven motion and `AgentUpdate` controls.
//!
//! Port of LLViewerObject::interpolateLinearMotion (indra/newview/llviewerobject.cpp),
//! LLDrawable::updateXform (indra/newview/lldrawable.cpp) and
//! LLSmoothInterpolation::calcInterpolant (indra/llcommon/llcriticaldamp.cpp),
//! originally LGPL 2.1. Like Firestorm, keyboard input only controls orientation
//! and the flags sent to the simulator; it does not predict avatar physics.

use aurora_net::AgentControls;
use aurora_net::control;
use glam::{Quat, Vec3};
use std::time::Instant;

#[derive(Debug, Clone, Copy, Default)]
pub struct MoveInput {
    pub forward: bool,
    pub back: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub up: bool,
    pub down: bool,
    pub run: bool,
    /// First moments of a key press: small "nudge" steps (LLAgent::moveAtNudge).
    pub nudge_fb: bool,
    pub nudge_lr: bool,
    /// Seconds the turn key has been held (LLFloaterMove::getYawRate).
    pub turn_held: f32,
}

#[derive(Debug, Clone)]
pub struct AgentState {
    /// Render-space position (main-region local).
    pub position: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
    pub seated: bool,
    /// Sitting on the ground: ANIM_AGENT_SIT_GROUND_CONSTRAINED plays on us
    /// (LLVOAvatar::processSingleAnimationStateChange -> sitDown). No
    /// parent, so positions stay region-relative unlike `seated`.
    pub ground_sit: bool,
    /// The drawn body is turning (+1 left, -1 right): TURN_LEFT / RIGHT.
    pub body_turn: i8,
    pub server_pos: Vec3,
    server_vel: Vec3,
    acceleration: Vec3,
    /// Undamped object position, separate from the drawable position above.
    predicted_pos: Vec3,
    last_server: Instant,
    last_prediction: Instant,
    has_server: bool,
}

impl Default for AgentState {
    fn default() -> Self {
        let now = Instant::now();
        AgentState {
            position: Vec3::new(128.0, 128.0, 25.0),
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            flying: false,
            seated: false,
            ground_sit: false,
            body_turn: 0,
            server_pos: Vec3::new(128.0, 128.0, 25.0),
            server_vel: Vec3::ZERO,
            acceleration: Vec3::ZERO,
            predicted_pos: Vec3::new(128.0, 128.0, 25.0),
            last_server: now,
            last_prediction: now,
            has_server: false,
        }
    }
}

/// Keyboard turn: 90°/s (LLAgent::propagate), ramping up from 5 % over the
/// first 0.25 s of the press (LLFloaterMove::getYawRate).
const TURN_RATE: f32 = std::f32::consts::FRAC_PI_2;
// Firestorm's physics timestep, dead-circuit phase-out and drawable damping.
const PHYSICS_TIMESTEP: f32 = 1.0 / 45.0;
const PHASE_OUT_TIME: f32 = 2.0;
const MAX_INTERPOLATION_TIME: f32 = 3.0;
const OBJECT_DAMPING_TIME: f32 = 0.06;

fn yaw_rate(held: f32) -> f32 {
    const NUDGE_TIME: f32 = 0.25;
    const YAW_NUDGE_RATE: f32 = 0.05;
    if held < NUDGE_TIME {
        YAW_NUDGE_RATE + held * (1.0 - YAW_NUDGE_RATE) / NUDGE_TIME
    } else {
        1.0
    }
}

impl AgentState {
    /// LLAgent::isSitting: on an object or on the ground.
    pub fn is_sitting(&self) -> bool {
        self.seated || self.ground_sit
    }

    pub fn has_local_control(&self) -> bool {
        self.has_server && !self.seated
    }

    pub fn body_rotation(&self) -> Quat {
        Quat::from_rotation_z(self.yaw)
    }

    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), self.yaw.sin(), 0.0)
    }

    pub fn set_look_at(&mut self, look: Vec3) {
        if look.truncate().length_squared() > 1e-6 {
            self.yaw = look.y.atan2(look.x);
        }
    }

    /// Seconds since the simulator last reported our position.
    pub fn server_age(&self) -> f32 {
        self.last_server.elapsed().as_secs_f32()
    }

    pub fn reset_motion(&mut self) {
        self.server_pos = self.position;
        self.server_vel = Vec3::ZERO;
        self.acceleration = Vec3::ZERO;
        self.velocity = Vec3::ZERO;
        self.predicted_pos = self.position;
        self.seated = false;
        self.has_server = true;
        self.last_server = Instant::now();
        self.last_prediction = self.last_server;
    }

    /// A server report replaces the extrapolated object state immediately;
    /// drawable damping in `update` handles the visible correction.
    pub fn on_server_update(&mut self, pos: Vec3, vel: Vec3, acceleration: Vec3, seated: bool, in_main_region: bool) {
        self.on_server_update_at(pos, vel, acceleration, seated, in_main_region, Instant::now());
    }

    fn on_server_update_at(&mut self, pos: Vec3, vel: Vec3, acceleration: Vec3, seated: bool, in_main_region: bool, now: Instant) {
        if !in_main_region || !pos.is_finite() || !vel.is_finite() || !acceleration.is_finite() {
            return;
        }
        let was_seated = self.seated;
        self.seated = seated;
        if seated {
            // Parent-relative coordinates must never enter standing prediction.
            self.velocity = Vec3::ZERO;
            self.acceleration = Vec3::ZERO;
            self.last_prediction = now;
            return;
        }
        if !self.has_server || was_seated {
            self.position = pos;
        }
        if self.has_server && (pos.z - self.server_pos.z).abs() > 0.5 {
            log::info!(
                "agent z {:.2} -> {:.2} (vel {:.2}, flying {}, seated {seated})",
                self.server_pos.z,
                pos.z,
                vel.z,
                self.flying
            );
        }
        self.server_pos = pos;
        self.server_vel = vel;
        self.predicted_pos = pos;
        self.velocity = vel;
        self.acceleration = acceleration;
        self.last_server = now;
        self.last_prediction = now;
        self.has_server = true;
    }

    pub fn predicted_position(&self) -> Vec3 {
        self.predicted_pos
    }

    /// Move from server velocity/acceleration, never from the keyboard. The
    /// simulator can omit avatar updates while its trajectory remains valid:
    /// only a silent circuit enables the 2 s / 3 s phase-out, as in Firestorm.
    pub fn predict(&mut self, time_dilation: f32, packet_age: f32) {
        self.predict_at(Instant::now(), time_dilation, packet_age);
    }

    fn predict_at(&mut self, now: Instant, time_dilation: f32, packet_age: f32) {
        let previous_age = self.last_prediction.saturating_duration_since(self.last_server).as_secs_f32();
        let raw_dt = now.saturating_duration_since(self.last_prediction).as_secs_f32();
        if !self.has_local_control() || !time_dilation.is_finite() || time_dilation <= 0.0 || raw_dt <= 0.0 {
            return;
        }
        self.last_prediction = now;
        let age = now.saturating_duration_since(self.last_server).as_secs_f32();
        let dt = time_dilation * raw_dt;
        let mut displacement = (self.velocity + 0.5 * (dt - PHYSICS_TIMESTEP) * self.acceleration) * dt;
        let mut delta_velocity = self.acceleration * dt;
        if age > PHASE_OUT_TIME && packet_age > PHASE_OUT_TIME {
            // Preserve Firestorm's incremental phase-out, including its
            // previous-interpolation branch (not a timeout on avatar reports).
            let phase = if age > MAX_INTERPOLATION_TIME {
                0.0
            } else if previous_age > PHASE_OUT_TIME {
                (MAX_INTERPOLATION_TIME - age) / (MAX_INTERPOLATION_TIME - raw_dt)
            } else {
                (MAX_INTERPOLATION_TIME - age) / (MAX_INTERPOLATION_TIME - PHASE_OUT_TIME)
            }
            .clamp(0.0, 1.0);
            displacement *= phase;
            delta_velocity *= phase;
        }
        let position = self.predicted_pos + displacement;
        let velocity = self.velocity + delta_velocity;
        if position.is_finite() && velocity.is_finite() {
            self.predicted_pos = position;
            self.velocity = velocity;
        }
    }

    /// Terrain/region constraints operate on the extrapolated object, not the
    /// last authoritative position. A missing neighboring region stops motion.
    pub fn constrain_prediction(&mut self, position: Vec3, stop: bool) {
        self.predicted_pos = position;
        if stop {
            self.velocity = Vec3::ZERO;
            self.acceleration = Vec3::ZERO;
        }
    }

    /// Offline demo only: stand in for the simulator's avatar physics so the
    /// controls can be tried without a grid (walk 3.2 m/s, run 5.1 m/s, fly,
    /// jump, terrain following). On a grid the simulator does this.
    pub fn simulate_locally(&mut self, input: &MoveInput, mouselook: bool, ground: Option<f32>, dt: f32) {
        // object center above the floor: half the default body height (SL
        // places the pelvis 0.13 m above it, see Scene::skeleton_of)
        const STAND: f32 = 0.84;
        let dt = dt.min(0.1);
        let fwd = self.forward();
        let left = Vec3::new(-fwd.y, fwd.x, 0.0);
        let mut dir = Vec3::ZERO;
        if input.forward {
            dir += fwd;
        }
        if input.back {
            dir -= fwd;
        }
        if input.strafe_left || (mouselook && input.turn_left) {
            dir += left;
        }
        if input.strafe_right || (mouselook && input.turn_right) {
            dir -= left;
        }
        let speed = if self.flying {
            10.0
        } else if input.down {
            1.2 // crouch-walk
        } else if input.run {
            5.13
        } else {
            3.2
        };
        let nudge = if input.nudge_fb || input.nudge_lr { 0.4 } else { 1.0 };
        let horiz = dir.normalize_or_zero() * speed * nudge;
        let floor = ground.map(|g| g + STAND);
        let mut pos = self.server_pos;
        let mut vz = self.server_vel.z;
        let grounded = floor.is_some_and(|f| pos.z <= f + 0.05);
        if self.flying {
            vz = if input.up {
                4.0
            } else if input.down {
                -4.0
            } else {
                0.0
            };
        } else if grounded {
            vz = if input.up { 5.0 } else { 0.0 };
        } else {
            vz -= 9.8 * dt;
        }
        pos += Vec3::new(horiz.x, horiz.y, vz) * dt;
        if let Some(f) = floor
            && pos.z < f
        {
            pos.z = f;
            vz = vz.max(0.0);
            if self.flying && input.down {
                self.flying = false; // landing
            }
        }
        self.server_pos = pos;
        self.server_vel = Vec3::new(horiz.x, horiz.y, vz);
        self.predicted_pos = pos;
        self.velocity = self.server_vel;
        self.acceleration = Vec3::ZERO;
        self.last_server = Instant::now();
        self.last_prediction = self.last_server;
        self.has_server = true;
    }

    /// Advance smoothing and turning.
    pub fn update(&mut self, input: &MoveInput, mouselook: bool, dt: f32, camera_distance: f32) {
        if !mouselook && input.turn_left != input.turn_right {
            let step = TURN_RATE * yaw_rate(input.turn_held) * dt;
            self.yaw += if input.turn_left { step } else { -step };
        }
        self.yaw = (self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        if !self.has_local_control() {
            return;
        }
        // LLDrawable::updateXform: 0.06 s is a half-life, not an exponential
        // time constant. Snap tiny or very large residual corrections.
        let k = (1.0 - 2.0_f32.powf(-dt / OBJECT_DAMPING_TIME)).clamp(0.0, 1.0);
        let damped = self.position.lerp(self.predicted_pos, k);
        let residual = damped.distance_squared(self.predicted_pos);
        let min_distance = 0.001_f32.powi(2) * camera_distance.powi(2);
        self.position = if residual >= min_distance && residual <= 10.0_f32.powi(2) {
            damped
        } else {
            self.predicted_pos
        };
    }

    /// Control flags for this frame.
    pub fn control_flags(&self, input: &MoveInput, mouselook: bool) -> u32 {
        let mut f = 0;
        // a held key always sends FAST_* (LLAgent::moveAt / moveLeft /
        // moveUp); running is SetAlwaysRun
        if input.forward {
            f |= if input.nudge_fb {
                control::NUDGE_AT_POS
            } else {
                control::AT_POS | control::FAST_AT
            };
        }
        if input.back {
            f |= if input.nudge_fb {
                control::NUDGE_AT_NEG
            } else {
                control::AT_NEG | control::FAST_AT
            };
        }
        let strafe_l = input.strafe_left || (mouselook && input.turn_left);
        let strafe_r = input.strafe_right || (mouselook && input.turn_right);
        if strafe_l {
            f |= if input.nudge_lr {
                control::NUDGE_LEFT_POS
            } else {
                control::LEFT_POS | control::FAST_LEFT
            };
        }
        if strafe_r {
            f |= if input.nudge_lr {
                control::NUDGE_LEFT_NEG
            } else {
                control::LEFT_NEG | control::FAST_LEFT
            };
        }
        if !mouselook {
            if input.turn_left {
                f |= control::YAW_POS;
            }
            if input.turn_right {
                f |= control::YAW_NEG;
            }
        }
        // the turn animations follow the drawn body, not the keys
        // (LLVOAvatar::updateOrientation)
        match self.body_turn {
            1 => f |= control::TURN_LEFT,
            -1 => f |= control::TURN_RIGHT,
            _ => {}
        }
        if input.up {
            f |= control::UP_POS | control::FAST_UP;
        }
        if input.down {
            f |= control::UP_NEG | control::FAST_UP;
        }
        if self.flying {
            f |= control::FLY;
        }
        if mouselook {
            f |= control::MOUSELOOK;
        }
        f
    }

    pub fn controls(&self, input: &MoveInput, cam: &crate::camera::Camera, far: f32) -> AgentControls {
        let at = cam.forward();
        let up = Vec3::Z;
        let left = up.cross(at).normalize_or(Vec3::Y);
        let cam_up = at.cross(left).normalize_or(Vec3::Z);
        AgentControls {
            control_flags: self.control_flags(input, cam.mouselook()),
            body_rotation: self.body_rotation(),
            head_rotation: Quat::from_rotation_y(-self.pitch * 0.5),
            camera_center: cam.position,
            camera_at: at,
            camera_left: left,
            camera_up: cam_up,
            far,
            state: 0,
            flags: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn moving(velocity: Vec3, acceleration: Vec3) -> (AgentState, Instant) {
        let now = Instant::now();
        let mut agent = AgentState::default();
        agent.on_server_update_at(Vec3::ZERO, velocity, acceleration, false, true, now);
        (agent, now)
    }

    fn at(start: Instant, seconds: f32) -> Instant {
        start + Duration::from_secs_f32(seconds)
    }

    fn near(actual: Vec3, expected: Vec3) {
        assert!(actual.distance(expected) < 0.0001, "{actual:?} != {expected:?}");
    }

    #[test]
    fn keys_only_send_controls_without_simulating_physics() {
        let (mut agent, now) = moving(Vec3::ZERO, Vec3::ZERO);
        agent.flying = true;
        let input = MoveInput {
            forward: true,
            strafe_left: true,
            up: true,
            run: true,
            ..Default::default()
        };
        agent.predict_at(at(now, 1.0), 1.0, 0.0);
        agent.update(&input, false, 0.1, 5.0);
        near(agent.position, Vec3::ZERO);
        near(agent.velocity, Vec3::ZERO);
        let flags = agent.control_flags(&input, false);
        assert_ne!(flags & control::AT_POS, 0);
        assert_ne!(flags & control::LEFT_POS, 0);
        assert_ne!(flags & control::UP_POS, 0);
        assert_ne!(flags & control::FLY, 0);
    }

    #[test]
    fn live_circuit_keeps_extrapolating_without_avatar_reports() {
        let (mut agent, now) = moving(Vec3::X * 3.2, Vec3::ZERO);
        for t in [0.2, 0.7, 4.0] {
            agent.predict_at(at(now, t), 1.0, 0.1);
            near(agent.predicted_position(), Vec3::X * 3.2 * t);
        }
        near(agent.server_pos, Vec3::ZERO);
    }

    #[test]
    fn acceleration_uses_simulator_time_and_average_physics_velocity() {
        let (mut agent, now) = moving(Vec3::X * 2.0, Vec3::X * 2.0);
        agent.predict_at(at(now, 0.4), 0.5, 0.0);
        near(agent.predicted_position(), Vec3::X * 0.435_555_55);
        near(agent.velocity, Vec3::X * 2.4);
        // With a complete 45 Hz physics second, the first reported average
        // velocity contributes no acceleration displacement, then 44 steps do.
        let (mut agent, now) = moving(Vec3::ZERO, Vec3::X);
        for tick in 1..=45 {
            agent.predict_at(at(now, tick as f32 / 45.0), 1.0, 0.0);
        }
        near(agent.predicted_position(), Vec3::X * (22.0 / 45.0));
        near(agent.velocity, Vec3::X);
    }

    #[test]
    fn silent_circuit_phases_out_and_stops_then_resumes() {
        let (mut agent, now) = moving(Vec3::X * 2.0, Vec3::ZERO);
        agent.predict_at(at(now, 2.5), 1.0, 2.5);
        near(agent.predicted_position(), Vec3::X * 2.5);
        agent.predict_at(at(now, 2.7), 1.0, 2.7);
        near(agent.predicted_position(), Vec3::X * 2.542_857_2);
        agent.predict_at(at(now, 3.5), 1.0, 3.5);
        near(agent.predicted_position(), Vec3::X * 2.542_857_2);
        agent.predict_at(at(now, 4.0), 1.0, 0.1);
        near(agent.predicted_position(), Vec3::X * 3.542_857_2);
    }

    #[test]
    fn correction_replaces_motion_and_damps_the_drawable() {
        let (mut agent, now) = moving(Vec3::X * 3.0, Vec3::X);
        agent.predict_at(at(now, 0.2), 1.0, 0.0);
        agent.position = Vec3::ZERO;
        agent.on_server_update_at(Vec3::X, Vec3::ZERO, Vec3::ZERO, false, true, at(now, 0.2));
        near(agent.predicted_position(), Vec3::X);
        agent.predict_at(at(now, 0.5), 1.0, 0.0);
        near(agent.predicted_position(), Vec3::X);
        near(agent.velocity, Vec3::ZERO);
        agent.update(&MoveInput::default(), false, 0.06, 5.0);
        near(agent.position, Vec3::X * 0.5);
        agent.update(&MoveInput::default(), false, 0.06, 5.0);
        near(agent.position, Vec3::X * 0.75);
        for _ in 0..10 {
            agent.update(&MoveInput::default(), false, 0.06, 5.0);
        }
        near(agent.position, Vec3::X);
    }

    #[test]
    fn large_corrections_snap_and_teleport_clears_old_motion() {
        let (mut agent, now) = moving(Vec3::X, Vec3::Z);
        let destination = Vec3::new(10.0, 10.0, 1000.0);
        agent.on_server_update_at(destination, Vec3::ZERO, Vec3::ZERO, false, true, now);
        agent.update(&MoveInput::default(), false, 0.06, 5.0);
        near(agent.position, destination);
        agent.seated = true;
        agent.position = destination + Vec3::X;
        agent.reset_motion();
        assert!(agent.has_local_control());
        near(agent.predicted_position(), destination + Vec3::X);
        agent.predict_at(agent.last_prediction + Duration::from_secs(1), 1.0, 0.0);
        near(agent.predicted_position(), destination + Vec3::X);
        near(agent.velocity, Vec3::ZERO);
    }

    #[test]
    fn seated_and_neighbor_reports_cannot_pollute_standing_motion() {
        let (mut agent, now) = moving(Vec3::X, Vec3::ZERO);
        agent.on_server_update_at(Vec3::splat(100.0), Vec3::ZERO, Vec3::ZERO, true, false, now);
        assert!(!agent.seated);
        near(agent.predicted_position(), Vec3::ZERO);
        agent.on_server_update_at(Vec3::splat(100.0), Vec3::ZERO, Vec3::ZERO, true, true, now);
        assert!(!agent.has_local_control());
        agent.predict_at(at(now, 1.0), 1.0, 0.0);
        near(agent.predicted_position(), Vec3::ZERO);
        let standing = Vec3::new(40.0, 50.0, 60.0);
        agent.on_server_update_at(standing, Vec3::Y, Vec3::ZERO, false, true, at(now, 1.0));
        assert!(agent.has_local_control());
        near(agent.position, standing);
        agent.predict_at(at(now, 1.1), 1.0, 0.0);
        near(agent.predicted_position(), standing + Vec3::Y * 0.1);
    }

    #[test]
    fn invalid_motion_does_not_replace_valid_server_state() {
        let (mut agent, now) = moving(Vec3::X, Vec3::ZERO);
        agent.on_server_update_at(Vec3::splat(f32::NAN), Vec3::ZERO, Vec3::ZERO, true, true, now);
        assert!(agent.has_local_control());
        agent.predict_at(at(now, 0.1), 1.0, 0.0);
        near(agent.predicted_position(), Vec3::X * 0.1);
    }
}
