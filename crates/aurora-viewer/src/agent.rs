//! Our own avatar: simulator-driven motion and `AgentUpdate` controls.
//!
//! Port of LLViewerObject::interpolateLinearMotion (indra/newview/llviewerobject.cpp),
//! LLDrawable::updateXform (indra/newview/lldrawable.cpp) and
//! LLSmoothInterpolation::calcInterpolant (indra/llcommon/llcriticaldamp.cpp),
//! originally LGPL 2.1. Like Firestorm, keyboard input only controls orientation
//! and the flags sent to the simulator; it does not predict avatar physics.
//! Aurora's movement (Preferences › Aurora, off by default) draws the avatar
//! ahead of the simulator's position by what the keys still on their way
//! will add (`own_motion.rs`) and fades corrections of 0.5 to 3 m slowly.

use crate::own_motion::{self, Intent, Prediction, Step};
use aurora_net::AgentControls;
use aurora_net::control;
use glam::{Quat, Vec3};
use std::time::Instant;
use uuid::Uuid;

// indra/llcharacter/llanimationstates.cpp (originally LGPL 2.1).
const PRE_JUMP: Uuid = uuid::uuid!("7a4e87fe-de39-6fcb-6223-024b00893244");
const LAND: Uuid = uuid::uuid!("7a17b059-12b2-41b1-570a-186368b6aa6f");
const MEDIUM_LAND: Uuid = uuid::uuid!("f4f00d6e-b9fe-9292-f4cb-0ae06ea58d57");
const STANDUP: Uuid = uuid::uuid!("3da1d753-028a-5446-24f3-9c9b856d9422");

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
    /// Native joystick angular rates (radians/second), already filtered.
    pub joystick_yaw: f32,
    pub joystick_pitch: f32,
    pub joystick_mouselook_yaw: bool,
}

impl MoveInput {
    /// LLAgent::moveAt/moveLeft/moveUp/moveYaw (indra/newview/llagent.cpp):
    /// movement returns the camera to the avatar, except latched crouching.
    pub fn resets_camera(&self, crouch_latched: bool) -> bool {
        self.forward
            || self.back
            || self.turn_left
            || self.turn_right
            || self.strafe_left
            || self.strafe_right
            || self.up
            || (self.down && !crouch_latched)
            || self.joystick_yaw != 0.0
            || self.joystick_pitch != 0.0
    }
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
    last_jump_input: Option<Instant>,
    corrections: Corrections,
    /// Aurora's movement prediction (own_motion.rs): on while
    /// `set_predicting` says so.
    pub prediction: Prediction,
    predicting: bool,
    /// The lead to draw this frame, as the world constrained it (land,
    /// edges of the regions), and the one `position` holds.
    lead: Vec3,
    drawn_lead: Vec3,
    /// A correction of 0.5 to 3 m is fading, with the prediction's
    /// longer half-life.
    slow_blend: bool,
    /// With the prediction, what is left of the last correction: it fades
    /// riding on the extrapolation instead of the drawable chasing it, so a
    /// moving avatar is not drawn late (world/motion.rs does the same for
    /// the other objects).
    offset: Vec3,
    /// With the prediction, where the simulator's position, the correction
    /// and the lead put the avatar last frame (`position` glides to it),
    /// and the velocity of that glide.
    aim: Vec3,
    glide: Vec3,
}

/// Simulator reports that put our avatar away from where it was
/// extrapolated: what a lagging simulator shows as rubber-banding.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Corrections {
    /// Reports farther than `CORRECTION_MIN` from the extrapolated position.
    pub count: u32,
    /// Largest distance of a report from the extrapolated position (m).
    pub largest: f32,
}

/// Distance counted as a correction: Firestorm's own-avatar snap statistic
/// (AGENT_POSITION_SNAP in LLViewerObject::processUpdateMessage, 0.5 m).
const CORRECTION_MIN: f32 = 0.5;
/// Farther than the drawable's snap distance the report is a move
/// (teleport within the region, sit target), not a correction.
const CORRECTION_MAX: f32 = 10.0;

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
            last_jump_input: None,
            corrections: Corrections::default(),
            prediction: Prediction::default(),
            predicting: false,
            lead: Vec3::ZERO,
            drawn_lead: Vec3::ZERO,
            slow_blend: false,
            offset: Vec3::ZERO,
            aim: Vec3::new(128.0, 128.0, 25.0),
            glide: Vec3::ZERO,
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
    /// LLAgent::moveUp records every upward input while not flying.
    pub fn record_jump_input(&mut self, now: Instant) {
        if !self.flying {
            self.last_jump_input = Some(now);
        }
    }

    /// Port of LLAgent::onAnimStop (indra/newview/llagent.cpp, LGPL 2.1).
    /// The simulator waits for FINISH_ANIM after a quick jump tap. A landing
    /// must not finish a newer pre-jump (FIRE-34049 / FIRE-34273); use the
    /// default RecentJumpThresholdSecs (1 s), but never suppress pre-jump.
    pub fn animation_stop_flags(&self, id: Uuid, up: bool, now: Instant) -> u32 {
        let landing = id == LAND || id == MEDIUM_LAND;
        let recent_jump = self
            .last_jump_input
            .is_some_and(|t| now.saturating_duration_since(t).as_secs_f32() < 1.0);
        if id == STANDUP || (!up && (id == PRE_JUMP || (landing && !recent_jump))) {
            control::FINISH_ANIM
        } else {
            0
        }
    }

    /// Port of LLAgent::getControlFlags (indra/newview/llagent.cpp, LGPL 2.1).
    /// Quickjump (FSIgnoreFinishAnimation) puts FINISH_ANIM in every
    /// AgentUpdate: the simulator no longer holds the jump until the
    /// pre-jump animation ends, nor the next move until the landing one does.
    pub fn quickjump_flags(flags: u32, quickjump: bool) -> u32 {
        if quickjump { flags | control::FINISH_ANIM } else { flags }
    }

    /// LLAgent::isSitting: on an object or on the ground.
    pub fn is_sitting(&self) -> bool {
        self.seated || self.ground_sit
    }

    pub fn has_local_control(&self) -> bool {
        self.has_server && !self.seated
    }

    /// The region positions are relative to changed (LLAgent::setRegion):
    /// the same place and motion, in the new region's coordinates.
    pub fn shift(&mut self, delta: Vec3) {
        self.position += delta;
        self.server_pos += delta;
        self.predicted_pos += delta;
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
        self.drop_prediction();
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

    /// Off the seat without a server report (LLVOAvatar::getOffObject ->
    /// sitDown(false), indra/newview/llvoavatar.cpp): standing where the
    /// avatar was drawn, `pos` in main-region coordinates, nothing of the
    /// seated state carried over.
    pub fn get_off(&mut self, pos: Vec3) {
        self.drop_prediction();
        self.seated = false;
        self.position = pos;
        self.aim = pos;
        self.server_pos = pos;
        self.predicted_pos = pos;
        self.velocity = Vec3::ZERO;
        self.server_vel = Vec3::ZERO;
        self.acceleration = Vec3::ZERO;
    }

    /// A server report replaces the extrapolated object state immediately;
    /// drawable damping in `update` handles the visible correction.
    #[cfg(test)]
    pub fn on_server_update(&mut self, pos: Vec3, vel: Vec3, acceleration: Vec3, seated: bool, in_main_region: bool) {
        self.on_server_update_at(pos, vel, acceleration, seated, in_main_region, Instant::now());
    }

    /// `on_server_update` of a report the network read at `now`.
    pub fn on_server_update_at(&mut self, pos: Vec3, vel: Vec3, acceleration: Vec3, seated: bool, in_main_region: bool, now: Instant) {
        if !in_main_region || !pos.is_finite() || !vel.is_finite() || !acceleration.is_finite() {
            return;
        }
        aurora_net::transition::step_once_arrived("own update", || {
            format!(
                "first update of our avatar from the region ({:.1} m from where it was drawn{})",
                pos.distance(self.position),
                if seated { ", seated" } else { "" }
            )
        });
        let was_seated = self.seated;
        self.seated = seated;
        if seated || was_seated {
            // a change of parent: nothing of the standing motion carries
            // over, the first frame standing starts from the report
            self.drop_prediction();
        }
        if seated {
            // LLVOAvatar::sitOnObject: sitting ends the flight
            if !was_seated {
                self.flying = false;
            }
            // Parent-relative coordinates must never enter standing prediction.
            self.velocity = Vec3::ZERO;
            self.acceleration = Vec3::ZERO;
            self.last_prediction = now;
            return;
        }
        if !self.has_server || was_seated {
            self.position = pos;
            self.aim = pos;
            self.glide = Vec3::ZERO;
        } else {
            let gap = pos.distance(self.predicted_pos);
            if gap <= CORRECTION_MAX {
                self.corrections.count += u32::from(gap > CORRECTION_MIN);
                self.corrections.largest = self.corrections.largest.max(gap);
            }
            if self.predicting && own_motion::slow_correction(gap) {
                self.slow_blend = true;
            } else if gap > 3.0 {
                self.slow_blend = false;
            }
            if self.predicting {
                // (the frame that applies it: what the lead must cover)
                self.prediction.on_report(vel, now, now.max(Instant::now()));
                // where the avatar was aimed and where the report puts it,
                // compared at the later of the report and the last frame:
                // a report that changes the velocity (a start, a stop) says
                // nothing of the motion before it
                let aimed = self.aim - self.drawn_lead;
                let (was, is) = if now >= self.last_prediction {
                    let ahead = now.duration_since(self.last_prediction).as_secs_f32();
                    (aimed + self.velocity * ahead, pos)
                } else {
                    let behind = self.last_prediction.duration_since(now).as_secs_f32();
                    (aimed, pos + vel * behind)
                };
                let offset = was - is;
                self.offset = if offset.is_finite() && offset.length() <= CORRECTION_MAX {
                    offset
                } else {
                    Vec3::ZERO
                };
            }
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

    /// Seated, the agent is where its seat carries it: the avatar object's
    /// place in the world, computed through the seat every frame (sitting
    /// branch of LLAgent::setPositionAgent, indra/newview/llagent.cpp).
    /// Standing up starts from there.
    pub fn follow_seat(&mut self, position: Vec3) {
        if !self.seated || !position.is_finite() {
            return;
        }
        self.drop_prediction();
        self.position = position;
        self.server_pos = position;
        self.predicted_pos = position;
    }

    pub fn predicted_position(&self) -> Vec3 {
        self.predicted_pos
    }

    /// Corrections since the previous call.
    pub fn take_corrections(&mut self) -> Corrections {
        std::mem::take(&mut self.corrections)
    }

    /// Aurora's movement on or off for this frame: the setting, on a grid
    /// (or the demo's simulated link), outside a teleport. Turned off, the
    /// avatar is drawn where the simulator puts it, as in Firestorm.
    pub fn set_predicting(&mut self, on: bool) {
        if !on && self.predicting {
            self.drop_prediction();
        }
        if on && !self.predicting {
            // from where the drawable was: no jump when it is turned on
            let offset = self.position - self.predicted_pos;
            self.offset = if offset.length() <= CORRECTION_MAX { offset } else { Vec3::ZERO };
            self.aim = self.position;
            self.glide = Vec3::ZERO;
        }
        self.predicting = on;
    }

    pub fn predicting(&self) -> bool {
        self.predicting
    }

    /// What is left of the last correction with the prediction, and the
    /// lead drawn this frame (`AURORA_DEMO_LINK`'s trace).
    pub fn drawn_offsets(&self) -> (Vec3, Vec3) {
        (self.offset, self.drawn_lead)
    }

    /// Back to the server's motion alone (a seat, a teleport, the setting).
    /// Where the avatar was drawn stays: standing, the drawable fades from
    /// there to the server's position.
    fn drop_prediction(&mut self) {
        self.prediction.reset();
        self.lead = Vec3::ZERO;
        self.drawn_lead = Vec3::ZERO;
        self.slow_blend = false;
        self.offset = Vec3::ZERO;
        self.glide = Vec3::ZERO;
    }

    /// One frame of the prediction, from what the keys sent ask for.
    /// Standing only: seated on an object, a vehicle or the ground, the
    /// keys never move the drawn avatar (a vehicle's script moves it on
    /// the simulator, `follow_seat` draws it).
    pub fn step_prediction(&mut self, intent: Option<Intent>, rtt: f32, dt: f32, now: Instant) {
        if !self.predicting || !self.has_local_control() || self.ground_sit {
            self.drop_prediction();
            return;
        }
        self.prediction.step(Step {
            intent,
            server_velocity: self.velocity,
            rtt,
            flying: self.flying,
            dt,
            now,
        });
        // the simulator stopped a little short of the lead: the avatar
        // stays where it is drawn (a correction that waits at rest)
        let left = self.prediction.settle(self.velocity);
        if left != Vec3::ZERO {
            self.offset += left;
            // (what a coast had left at its landing fades slowly)
            self.slow_blend |= own_motion::slow_correction(self.offset.length());
        }
        self.lead = self.prediction.lead();
    }

    /// The lead the prediction asks for, before the land and the regions.
    pub fn wanted_lead(&self) -> Vec3 {
        if self.predicting { self.prediction.lead() } else { Vec3::ZERO }
    }

    /// The lead to draw, kept on the land and in the regions by the world.
    pub fn set_lead(&mut self, lead: Vec3) {
        if self.predicting && lead.is_finite() {
            self.lead = lead;
        }
    }

    /// The velocity of the drawn avatar: with the prediction, that of its
    /// glide.
    pub fn drawn_velocity(&self) -> Vec3 {
        if self.predicting && self.has_local_control() {
            self.glide
        } else {
            self.velocity
        }
    }

    /// Motion left in visual smoothing after a simulator stop. The legacy
    /// path's exponential convergence still moves us even with zero velocity.
    pub fn settling_velocity(&self) -> Vec3 {
        if self.predicting {
            self.glide
        } else {
            (self.predicted_pos - self.position) * (std::f32::consts::LN_2 / OBJECT_DAMPING_TIME)
        }
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
    pub fn simulate_locally(&mut self, input: &MoveInput, mouselook: bool, ground: Option<f32>, dt: f32, automatic_land: bool) {
        self.simulate_flight(input, mouselook, ground, dt, false, automatic_land);
    }

    /// Optional native-flight stand-in for offline navigation tests. Native
    /// flight retains velocity after release; NUDGE is not a speed limiter.
    pub fn simulate_native_flight(&mut self, input: &MoveInput, mouselook: bool, ground: Option<f32>, dt: f32, automatic_land: bool) {
        self.simulate_flight(input, mouselook, ground, dt, true, automatic_land);
    }

    fn simulate_flight(
        &mut self,
        input: &MoveInput,
        mouselook: bool,
        ground: Option<f32>,
        dt: f32,
        native_flight: bool,
        automatic_land: bool,
    ) {
        // object center above the floor: half the default body height (SL
        // places the pelvis 0.13 m above it, see Scene::skeleton_of)
        const STAND: f32 = 0.84;
        if self.seated {
            // the seat places the avatar (`follow_seat`), not the keys
            return;
        }
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
            if native_flight { 16.0 } else { 10.0 }
        } else if input.down {
            1.2 // crouch-walk
        } else if input.run {
            5.13
        } else {
            3.2
        };
        let nudge = if (input.nudge_fb || input.nudge_lr) && !(native_flight && self.flying) {
            0.4
        } else {
            1.0
        };
        let mut horiz = dir.normalize_or_zero() * speed * nudge;
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
        if native_flight && self.flying {
            let blend = 1.0 - (-dt / 0.5).exp();
            let velocity = self.server_vel.lerp(Vec3::new(horiz.x, horiz.y, vz), blend);
            horiz = velocity.with_z(0.0);
            vz = velocity.z;
        }
        pos += Vec3::new(horiz.x, horiz.y, vz) * dt;
        if let Some(f) = floor
            && pos.z < f
        {
            pos.z = f;
            vz = vz.max(0.0);
            if self.flying && input.down && automatic_land {
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
        self.yaw += input.joystick_yaw * dt.clamp(0.0, 0.2);
        self.pitch = (self.pitch + input.joystick_pitch * dt.clamp(0.0, 0.2)).clamp(-1.55, 1.55);
        if !mouselook && input.turn_left != input.turn_right {
            let step = TURN_RATE * yaw_rate(input.turn_held) * dt;
            self.yaw += if input.turn_left { step } else { -step };
        }
        self.yaw = (self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        if !self.has_local_control() {
            return;
        }
        let min_distance = 0.001_f32.powi(2) * camera_distance.powi(2);
        if self.predicting {
            // Aurora: the simulator's extrapolated position, what is left of
            // the last correction, and the lead of the keys over it
            let half_life = if self.slow_blend {
                own_motion::SLOW_CORRECTION_HALF_LIFE
            } else {
                OBJECT_DAMPING_TIME
            };
            // a few centimetres left at a stop wait for the next move; not
            // the height (a landing would leave the feet in the ground)
            let resting = !self.prediction.keys_held()
                && self.lead == Vec3::ZERO
                && !self.slow_blend
                && self.offset.length() < own_motion::REST_OFFSET;
            let fade = 0.5_f32.powf(dt / half_life);
            if resting {
                self.offset.z *= fade;
            } else {
                self.offset *= fade;
            }
            let left = self.offset.length_squared();
            if left < min_distance || !self.offset.is_finite() {
                self.offset = Vec3::ZERO;
            }
            if left < 0.05_f32.powi(2) {
                self.slow_blend = false;
            }
            self.drawn_lead = self.lead;
            let aim = self.predicted_pos + self.offset + self.drawn_lead;
            self.aim = aim;
            // the drawn avatar glides to it, so a stop eases out instead of
            // halting in one frame, and a frame of the simulator's motion
            // more or less than the lead does not show as a step
            if !aim.is_finite() || self.position.distance(aim) > CORRECTION_MAX {
                self.position = aim;
                self.glide = Vec3::ZERO;
            } else {
                let glided = own_motion::glide(self.position, self.glide, aim, dt);
                (self.position, self.glide) = if self.prediction.keys_held() {
                    glided
                } else {
                    own_motion::ease_out(self.position, self.glide, glided, aim, dt)
                };
            }
            return;
        }
        // LLDrawable::updateXform: 0.06 s is a half-life, not an exponential
        // time constant. Snap tiny or very large residual corrections.
        let k = (1.0 - 2.0_f32.powf(-dt / OBJECT_DAMPING_TIME)).clamp(0.0, 1.0);
        // (`lerp_monotonic`: an avatar at rest stays where it is, see Camera::update)
        let damped = self.position.lerp_monotonic(self.predicted_pos, k);
        let residual = damped.distance_squared(self.predicted_pos);
        self.position = if residual >= min_distance && residual <= 10.0_f32.powi(2) {
            damped
        } else {
            self.predicted_pos
        };
    }

    /// Control flags for this frame.
    pub fn control_flags(&self, input: &MoveInput, mouselook: bool) -> u32 {
        let mut f = 0;
        if input.joystick_yaw > 0.0 && (!mouselook || input.joystick_mouselook_yaw) {
            f |= control::YAW_POS;
        }
        if input.joystick_yaw < 0.0 && (!mouselook || input.joystick_mouselook_yaw) {
            f |= control::YAW_NEG;
        }
        if input.joystick_pitch > 0.0 {
            f |= control::PITCH_POS;
        }
        if input.joystick_pitch < 0.0 {
            f |= control::PITCH_NEG;
        }
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
        let left = cam.view_up().cross(at).normalize_or(Vec3::Y);
        let cam_up = at.cross(left).normalize_or(Vec3::Z);
        AgentControls {
            control_flags: self.control_flags(input, cam.mouselook()),
            body_rotation: self.body_rotation(),
            head_rotation: cam.head_rotation(self),
            camera_center: cam.position,
            camera_at: at,
            camera_left: left,
            camera_up: cam_up,
            far,
            state: 0,
            flags: 0,
            alt_or_ctrl: false,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn automatic_landing_preference_keeps_demo_descent_and_ascent_available() {
        let down = MoveInput {
            down: true,
            ..Default::default()
        };
        let up = MoveInput {
            up: true,
            ..Default::default()
        };
        for native in [false, true] {
            for automatic_land in [false, true] {
                let mut agent = AgentState {
                    flying: true,
                    server_pos: Vec3::Z * 0.84,
                    ..Default::default()
                };
                for _ in 0..10 {
                    agent.simulate_flight(&down, false, Some(0.0), 0.1, native, automatic_land);
                }
                assert_eq!(agent.flying, !automatic_land);
                assert_eq!(agent.server_pos.z, 0.84);
                let flags = agent.control_flags(&down, false);
                assert_ne!(flags & control::UP_NEG, 0, "descent remains held");
                assert_eq!(flags & control::FLY != 0, !automatic_land);
                agent.simulate_flight(&up, false, Some(0.0), 0.1, native, automatic_land);
                assert!(agent.server_pos.z > 0.84, "jump or ascend remains available");
            }
        }
    }

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
    fn quick_jump_tap_finishes_pre_jump_but_not_recent_landing() {
        let mut agent = AgentState::default();
        let now = Instant::now();
        agent.record_jump_input(now);
        let released = at(now, 0.3);
        assert_eq!(agent.animation_stop_flags(PRE_JUMP, false, released), control::FINISH_ANIM);
        for landing in [LAND, MEDIUM_LAND] {
            assert_eq!(agent.animation_stop_flags(landing, false, released), 0);
            assert_eq!(agent.animation_stop_flags(landing, false, at(now, 1.0)), control::FINISH_ANIM);
        }
        assert_eq!(agent.animation_stop_flags(Uuid::from_u128(1), false, released), 0);
    }

    #[test]
    fn quickjump_finishes_animations_in_every_update() {
        let walking = control::AT_POS | control::FAST_AT;
        assert_eq!(AgentState::quickjump_flags(walking, false), walking);
        assert_eq!(AgentState::quickjump_flags(walking, true), walking | control::FINISH_ANIM);
        assert_eq!(AgentState::quickjump_flags(0, true), control::FINISH_ANIM);
    }

    /// With automatic takeoff off the agent never flies from a held Up, and
    /// only Quickjump puts FINISH_ANIM in the AgentUpdate: the simulator plays
    /// the pre-jump and repeats the jump while Up stays held, as with the
    /// Legacy Viewer's AutomaticFly off (agent_jump only calls moveUp).
    #[test]
    fn held_ground_jump_keeps_the_pre_jump_unless_quickjump() {
        let mut agent = AgentState::default();
        let now = Instant::now();
        let input = MoveInput {
            up: true,
            forward: true,
            ..Default::default()
        };
        agent.record_jump_input(now);
        let held = agent.control_flags(&input, false);
        assert_ne!(held & control::UP_POS, 0, "the simulator keeps the repeated jump input");
        assert_eq!(held & (control::FLY | control::FINISH_ANIM), 0);
        assert_eq!(AgentState::quickjump_flags(held, false), held, "pre-jump kept");
        assert_eq!(AgentState::quickjump_flags(held, true), held | control::FINISH_ANIM);
        // LLAgent::onAnimStop: no finish while Up is held, so the next jump
        // keeps its pre-jump; a released tap finishes it and cannot stall.
        assert_eq!(agent.animation_stop_flags(PRE_JUMP, true, at(now, 0.4)), 0);
        assert_eq!(agent.animation_stop_flags(PRE_JUMP, false, at(now, 0.4)), control::FINISH_ANIM);
        // a landing right after a jump never finishes the next pre-jump
        assert_eq!(agent.animation_stop_flags(LAND, false, at(now, 0.8)), 0);
    }

    #[test]
    fn held_up_suppresses_jump_and_landing_finish_but_not_standup() {
        let agent = AgentState::default();
        let now = Instant::now();
        for anim in [PRE_JUMP, LAND, MEDIUM_LAND] {
            assert_eq!(agent.animation_stop_flags(anim, true, now), 0);
            assert_eq!(agent.animation_stop_flags(anim, false, now), control::FINISH_ANIM);
        }
        assert_eq!(agent.animation_stop_flags(STANDUP, true, now), control::FINISH_ANIM);
    }

    #[test]
    fn repeated_jump_input_refreshes_landing_guard_and_flight_does_not() {
        let mut agent = AgentState::default();
        let now = Instant::now();
        agent.record_jump_input(now);
        agent.record_jump_input(at(now, 0.9));
        assert_eq!(agent.animation_stop_flags(LAND, false, at(now, 1.1)), 0);
        assert_eq!(agent.animation_stop_flags(PRE_JUMP, false, at(now, 1.1)), control::FINISH_ANIM);
        agent.flying = true;
        agent.record_jump_input(at(now, 1.8));
        assert_eq!(agent.animation_stop_flags(LAND, false, at(now, 2.0)), control::FINISH_ANIM);
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
    fn corrections_count_reports_away_from_the_extrapolation() {
        let (mut agent, now) = moving(Vec3::X * 3.0, Vec3::ZERO);
        // where it was expected: no correction, a small largest distance
        agent.predict_at(at(now, 0.2), 1.0, 0.0);
        agent.on_server_update_at(Vec3::X * 0.7, Vec3::X * 3.0, Vec3::ZERO, false, true, at(now, 0.2));
        let c = agent.take_corrections();
        assert_eq!(c.count, 0);
        assert!((c.largest - 0.1).abs() < 1e-4, "{c:?}");
        // pulled back 2 m by a lagging simulator: one correction
        agent.predict_at(at(now, 1.2), 1.0, 0.0);
        agent.on_server_update_at(Vec3::X * 1.7, Vec3::ZERO, Vec3::ZERO, false, true, at(now, 1.2));
        let c = agent.take_corrections();
        assert_eq!(c.count, 1);
        assert!((c.largest - 2.0).abs() < 1e-3, "{c:?}");
        // a move across the region is not one, and taking empties the count
        agent.on_server_update_at(Vec3::X * 100.0, Vec3::ZERO, Vec3::ZERO, false, true, at(now, 1.3));
        assert_eq!(agent.take_corrections(), Corrections::default());
        // standing up from a seat places the avatar: not a correction either
        agent.on_server_update_at(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, true, true, at(now, 1.4));
        agent.on_server_update_at(Vec3::Y * 3.0, Vec3::ZERO, Vec3::ZERO, false, true, at(now, 1.5));
        assert_eq!(agent.take_corrections(), Corrections::default());
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
    fn a_seated_agent_is_where_its_seat_carries_it() {
        let (mut agent, now) = moving(Vec3::X, Vec3::ZERO);
        // standing, the seat has no say
        agent.follow_seat(Vec3::splat(50.0));
        near(agent.position, Vec3::ZERO);
        // seated: the flight ends, the report is seat-relative and the seat
        // gives the place
        agent.flying = true;
        agent.on_server_update_at(Vec3::Z * 0.4, Vec3::ZERO, Vec3::ZERO, true, true, now);
        assert!(!agent.flying);
        let carried = Vec3::new(200.0, 30.0, 45.0);
        agent.follow_seat(carried);
        near(agent.position, carried);
        near(agent.predicted_position(), carried);
        agent.follow_seat(Vec3::splat(f32::NAN));
        near(agent.position, carried);
        // the keys do not move a seated avatar (demo), and standing up 1 m
        // from the seat is neither a jump nor a correction
        let forward = MoveInput {
            forward: true,
            ..MoveInput::default()
        };
        agent.simulate_locally(&forward, false, Some(0.0), 0.1, true);
        agent.update(&forward, false, 0.1, 5.0);
        near(agent.position, carried);
        agent.on_server_update_at(carried + Vec3::X, Vec3::ZERO, Vec3::ZERO, false, true, at(now, 1.0));
        near(agent.position, carried + Vec3::X);
        assert_eq!(agent.take_corrections(), Corrections::default());
    }

    #[test]
    fn invalid_motion_does_not_replace_valid_server_state() {
        let (mut agent, now) = moving(Vec3::X, Vec3::ZERO);
        agent.on_server_update_at(Vec3::splat(f32::NAN), Vec3::ZERO, Vec3::ZERO, true, true, now);
        assert!(agent.has_local_control());
        agent.predict_at(at(now, 0.1), 1.0, 0.0);
        near(agent.predicted_position(), Vec3::X * 0.1);
    }

    fn walk() -> Option<Intent> {
        Intent::from_flags(control::AT_POS | control::FAST_AT, 0.0, false)
    }

    /// Aurora's movement: the drawn avatar moves with the key, ahead of
    /// the simulator's position, which itself does not move before its
    /// report.
    #[test]
    fn with_the_prediction_the_key_moves_the_drawn_avatar_at_once() {
        let (mut agent, now) = moving(Vec3::ZERO, Vec3::ZERO);
        agent.set_predicting(true);
        for n in 1..=10 {
            agent.step_prediction(walk(), 0.2, 0.01, at(now, n as f32 * 0.01));
            agent.update(&MoveInput::default(), false, 0.01, 5.0);
        }
        near(agent.predicted_position(), Vec3::ZERO);
        let ahead = agent.position.x;
        assert!(ahead > 0.03, "{:?}", agent.position);
        assert!(agent.drawn_velocity().x > 1.0, "{:?}", agent.drawn_velocity());
        // turned off: back to the simulator's position, smoothly
        agent.set_predicting(false);
        agent.update(&MoveInput::default(), false, 0.06, 5.0);
        assert!(agent.position.x > 0.0 && agent.position.x < ahead, "{:?}", agent.position);
        for _ in 0..20 {
            agent.update(&MoveInput::default(), false, 0.06, 5.0);
        }
        near(agent.position, Vec3::ZERO);
    }

    /// Seated, as a passenger or as the driver of a vehicle (its script
    /// moves it on the simulator), or on a prim at rest: the keys never
    /// move the drawn avatar, which is where its seat carries it
    /// (Firestorm's sitting key bindings orbit the camera or pass the keys
    /// to the vehicle's script: key_bindings.xml <sitting>,
    /// camera_move_forward_sitting in llviewerinput.cpp).
    #[test]
    fn seated_the_keys_never_move_the_drawn_avatar() {
        let (mut agent, now) = moving(Vec3::ZERO, Vec3::ZERO);
        agent.set_predicting(true);
        agent.step_prediction(walk(), 0.2, 0.1, at(now, 0.1));
        assert!(agent.wanted_lead().x > 0.0);
        // sitting down: the lead and the prediction's motion are dropped
        agent.on_server_update_at(Vec3::Z * 0.9, Vec3::ZERO, Vec3::ZERO, true, true, at(now, 0.2));
        assert_eq!(agent.wanted_lead(), Vec3::ZERO);
        for (n, seat) in [Vec3::new(10.0, 0.0, 1.0), Vec3::new(11.0, 0.0, 1.0), Vec3::new(12.0, 0.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            let t = 0.3 + n as f32 * 0.25;
            agent.step_prediction(walk(), 0.2, 0.25, at(now, t));
            agent.follow_seat(seat);
            agent.update(
                &MoveInput {
                    forward: true,
                    ..MoveInput::default()
                },
                false,
                0.25,
                5.0,
            );
            near(agent.position, seat);
            assert_eq!(agent.wanted_lead(), Vec3::ZERO);
            assert_eq!(agent.drawn_velocity(), Vec3::ZERO);
        }
        // a vehicle's keys still leave for the simulator, its script uses them
        assert_ne!(
            agent.control_flags(
                &MoveInput {
                    forward: true,
                    ..MoveInput::default()
                },
                false
            ) & control::AT_POS,
            0
        );
        // on the ground (sit_ground_constrained), no lead either
        let (mut agent, now) = moving(Vec3::ZERO, Vec3::ZERO);
        agent.set_predicting(true);
        agent.ground_sit = true;
        agent.step_prediction(walk(), 0.2, 0.1, at(now, 0.1));
        agent.update(&MoveInput::default(), false, 0.1, 5.0);
        assert_eq!(agent.wanted_lead(), Vec3::ZERO);
        near(agent.position, Vec3::ZERO);
    }

    /// Standing up is a change of parent: no fade from the seat and no
    /// lead carried over; the first frame standing starts from the report.
    #[test]
    fn standing_up_starts_from_the_simulators_position() {
        let (mut agent, now) = moving(Vec3::ZERO, Vec3::ZERO);
        agent.set_predicting(true);
        agent.on_server_update_at(Vec3::Z, Vec3::ZERO, Vec3::ZERO, true, true, now);
        agent.follow_seat(Vec3::new(50.0, 50.0, 20.0));
        let stood = Vec3::new(51.5, 50.0, 20.0);
        agent.on_server_update_at(stood, Vec3::ZERO, Vec3::ZERO, false, true, at(now, 1.0));
        near(agent.position, stood);
        agent.step_prediction(walk(), 0.2, 0.01, at(now, 1.01));
        agent.update(&MoveInput::default(), false, 0.01, 5.0);
        // one frame of the key's motion from there, not a fade from the seat
        assert!(agent.position.distance(stood) < 0.05, "{:?}", agent.position);
        assert_eq!(agent.take_corrections(), Corrections::default());
    }

    /// Our avatar walking with Aurora's movement against a simulator one
    /// round trip away (0.2 s) that starts and stops at once, keys held
    /// `hold` seconds out of 3: the drawn position frame by frame, and
    /// where the simulator ends.
    fn walk_and_stop(fps: f32, hold: f32) -> (Vec<(f32, f32)>, f32) {
        let start = Instant::now();
        let at = |t: f32| start + Duration::from_secs_f32(t);
        let keys = |t: f32| t >= 0.0 && t.rem_euclid(3.0) < hold;
        let mut agent = AgentState::default();
        agent.on_server_update_at(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, false, true, start);
        agent.set_predicting(true);
        let (mut server, mut velocity) = (0.0_f32, 0.0_f32);
        let mut sent: Option<(f32, f32)> = None;
        let mut on_the_way: std::collections::VecDeque<(f32, f32, f32)> = Default::default();
        let dt = 1.0 / fps;
        let mut drawn = Vec::new();
        let mut t = 0.0;
        while t < 6.0 {
            t += dt;
            // the simulator plays the keys half a round trip late, and its
            // report comes back after the other half
            let v = if keys(t - 0.1) { 3.2 } else { 0.0 };
            server += velocity * dt;
            velocity = v;
            if sent.is_none_or(|(last_v, last_t)| last_v != v || t - last_t >= 0.25) {
                sent = Some((v, t));
                on_the_way.push_back((t + 0.1, server, v));
            }
            while on_the_way.front().is_some_and(|(arrives, _, _)| *arrives <= t) {
                let (arrives, x, v) = on_the_way.pop_front().expect("report");
                agent.on_server_update_at(Vec3::X * x, Vec3::X * v, Vec3::ZERO, false, true, at(arrives));
            }
            agent.predict_at(at(t), 1.0, 0.0);
            let flags = if keys(t) { control::AT_POS | control::FAST_AT } else { 0 };
            agent.step_prediction(Intent::from_flags(flags, 0.0, false), 0.2, dt, at(t));
            agent.set_lead(agent.wanted_lead());
            agent.update(&MoveInput::default(), false, dt, 5.0);
            drawn.push((t, agent.position.x));
        }
        (drawn, server)
    }

    /// At a release the drawn avatar eases to its stop: its speed falls to
    /// zero without a step (bounded change per frame), never rises again,
    /// and it does not come back. Before the calibration has seen a stop
    /// (the first release) and after (the second), at 60 and 500 frames a
    /// second.
    #[test]
    fn a_stop_eases_out_without_a_step_or_a_pull_back() {
        for fps in [60.0, 500.0] {
            let (drawn, _) = walk_and_stop(fps, 1.5);
            for (n, release) in [1.5_f32, 4.5].into_iter().enumerate() {
                let window: Vec<_> = drawn
                    .windows(2)
                    .filter(|w| w[1].0 > release - 0.2 && w[1].0 < release + 1.0)
                    .collect();
                let speeds: Vec<(f32, f32)> = window.iter().map(|w| (w[1].0, (w[1].1 - w[0].1) / (w[1].0 - w[0].0))).collect();
                let max_step = speeds
                    .windows(2)
                    .map(|s| (s[1].1 - s[0].1).abs() / (s[1].0 - s[0].0))
                    .fold(0.0, f32::max);
                let rise = speeds
                    .windows(2)
                    .filter(|s| s[0].0 >= release + 0.02)
                    .map(|s| s[1].1 - s[0].1)
                    .fold(0.0, f32::max);
                let ahead = window.iter().map(|w| w[1].1).fold(f32::MIN, f32::max);
                let end = window.last().map_or(0.0, |w| w[1].1);
                let report = format!(
                    "fps {fps}, release {n}: accel max {max_step:.1} m/s², rise {rise:.3} m/s, pull back {:.3} m",
                    ahead - end
                );
                assert!(max_step < 40.0, "{report}");
                assert!(rise < 0.05, "{report}");
                assert!(ahead - end < 0.01, "{report}");
                assert!(speeds.last().is_some_and(|s| s.1.abs() < 0.01), "{report}: still moving");
            }
        }
    }

    /// A tap shorter than the round trip: the simulator moves after the
    /// release; the drawn avatar neither counts that motion twice nor
    /// comes back, and ends where the simulator does.
    #[test]
    fn a_tap_shorter_than_the_round_trip_ends_where_the_simulator_does() {
        for fps in [60.0, 500.0] {
            let (drawn, server) = walk_and_stop(fps, 0.1);
            let mut ahead = f32::MIN;
            for &(_, x) in &drawn {
                assert!(x > ahead - 0.01, "fps {fps}: came back from {ahead} to {x}");
                ahead = ahead.max(x);
            }
            let end = drawn.last().map_or(0.0, |d| d.1);
            assert!((end - server).abs() < own_motion::REST_OFFSET, "fps {fps}: {end} for {server}");
        }
    }

    /// The end of a flight on Agni (2026-10-16): hovering at rest, no key,
    /// the simulator drops our avatar 5.8 m to the ground. The drawn avatar
    /// falls with it instead of hanging in the air until the next key, and
    /// comes down where the simulator lands.
    #[test]
    fn a_fall_without_keys_is_followed() {
        for fps in [60.0, 500.0] {
            let start = Instant::now();
            let at = |t: f32| start + Duration::from_secs_f32(t);
            let mut agent = AgentState::default();
            let hover = Vec3::new(78.4, 154.1, 35.3);
            agent.on_server_update_at(hover, Vec3::ZERO, Vec3::ZERO, false, true, start);
            agent.set_predicting(true);
            // a few centimetres left of the last correction and a glide
            // nearly at rest, as on the grid
            agent.offset = Vec3::new(0.0, 0.0, 0.03);
            agent.glide = Vec3::new(0.0, 0.0, -1e-4);
            let dt = 1.0 / fps;
            let (mut t, mut next_report) = (0.0_f32, 0.0_f32);
            let mut lag = 0.0_f32;
            while t < 2.5 {
                t += dt;
                if t >= next_report {
                    next_report += 0.05;
                    // free fall from t = 0.2 s, on the ground (29.5) after 1.1 s
                    let fall = (t - 0.2).max(0.0);
                    let (z, vz) = if fall < 1.09 {
                        (35.3 - 4.9 * fall * fall, -9.8 * fall)
                    } else {
                        (29.5, 0.0)
                    };
                    let z = z.max(29.5);
                    agent.on_server_update_at(Vec3::new(hover.x, hover.y, z), Vec3::Z * vz, Vec3::ZERO, false, true, at(t));
                }
                agent.predict_at(at(t), 1.0, 0.0);
                agent.step_prediction(None, 0.2, dt, at(t));
                agent.set_lead(agent.wanted_lead());
                agent.update(&MoveInput::default(), false, dt, 5.0);
                lag = lag.max(agent.position.z - agent.predicted_pos.z);
            }
            // (the glide trails a fall at 10 m/s by about 0.8 m, as
            // Firestorm's damping does)
            assert!(lag < 1.2, "fps {fps}: hung {lag:.2} m above the simulator's avatar");
            assert!((agent.position.z - 29.5).abs() < 0.1, "fps {fps}: ended at {:.2}", agent.position.z);
        }
    }

    /// Firestorm's movement stays as it was: a scripted walk, a 0.5 m
    /// pull-back, a stop and a last correction give, every 0.1 s, the
    /// positions the code of main gave (recorded on 2026-10-16 from
    /// origin/main e12ff851, `agent.rs` before Aurora's movement).
    #[test]
    fn firestorm_movement_draws_what_main_drew() {
        const MAIN: [f32; 20] = [
            10.0, 10.0, 10.14793, 10.41374, 10.71666, 11.03129, 11.26202, 11.29897, 11.52981, 11.82173, 12.13288, 12.45009, 12.62128,
            12.67521, 12.67468, 12.62352, 12.60741, 12.6, 12.6, 12.6,
        ];
        let start = Instant::now();
        let at = |t: f32| start + Duration::from_secs_f32(t);
        let mut agent = AgentState::default();
        agent.on_server_update_at(Vec3::new(10.0, 10.0, 20.0), Vec3::ZERO, Vec3::ZERO, false, true, at(0.0));
        let reports = [
            (0.2, 10.0, 3.2),
            (0.45, 10.8, 3.2),
            (0.7, 11.1, 3.2),
            (0.95, 11.9, 3.2),
            (1.2, 12.7, 0.0),
            (1.5, 12.6, 0.0),
        ];
        let dt = 1.0 / 60.0;
        let mut next = 0;
        let mut drawn = Vec::new();
        for n in 1..=120 {
            let t = n as f32 * dt;
            while next < reports.len() && reports[next].0 <= t + 1e-6 {
                let (rt, x, v) = reports[next];
                agent.on_server_update_at(Vec3::new(x, 10.0, 20.0), Vec3::X * v, Vec3::ZERO, false, true, at(rt));
                next += 1;
            }
            // what the app does each frame with the setting on Firestorm
            agent.set_predicting(false);
            agent.predict_at(at(t), 1.0, 0.0);
            let flags = if t < 1.0 { control::AT_POS | control::FAST_AT } else { 0 };
            agent.step_prediction(Intent::from_flags(flags, 0.0, false), 0.2, dt, at(t));
            agent.set_lead(agent.wanted_lead());
            let input = MoveInput {
                forward: t < 1.0,
                ..Default::default()
            };
            agent.update(&input, false, dt, 5.0);
            assert_eq!(agent.drawn_velocity(), agent.velocity);
            if n % 6 == 0 {
                drawn.push(agent.position.x);
            }
        }
        for (i, (now, main)) in drawn.iter().zip(MAIN).enumerate() {
            assert!(
                (now - main).abs() < 1e-4,
                "at {:.1} s: {now} instead of main's {main}",
                (i + 1) as f32 * 0.1
            );
        }
        assert_eq!(drawn.len(), MAIN.len());
    }

    /// A pull-back of 2 m fades with the prediction's 0.25 s half-life,
    /// Firestorm's 0.06 s without it.
    #[test]
    fn with_the_prediction_corrections_of_two_metres_fade_slowly() {
        for (predicting, half_life) in [(true, own_motion::SLOW_CORRECTION_HALF_LIFE), (false, OBJECT_DAMPING_TIME)] {
            let (mut agent, now) = moving(Vec3::ZERO, Vec3::ZERO);
            agent.set_predicting(predicting);
            agent.on_server_update_at(Vec3::X * 2.0, Vec3::ZERO, Vec3::ZERO, false, true, at(now, 0.1));
            agent.update(&MoveInput::default(), false, half_life, 5.0);
            if predicting {
                // (the drawn avatar glides after it)
                near(agent.drawn_offsets().0, -Vec3::X);
            } else {
                near(agent.position, Vec3::X);
            }
            assert_eq!(agent.take_corrections().count, 1);
        }
    }
}
