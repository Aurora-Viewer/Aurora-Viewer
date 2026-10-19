//! Native movement decisions. The bridge receives bounded motor settings;
//! AgentAnimation broadcasts strokes just like AOEngine (Firestorm aoengine.cpp).
pub mod crouch_toggle;
pub mod movelock;
pub mod passage;
pub mod run;
pub mod strokes;
pub mod swim;
#[cfg(test)]
mod tests;
pub mod water_float;
pub mod water_jump;
pub mod water_walk;
use crate::agent::MoveInput;
use aurora_net::bridge::Motion;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

// Default_Avatar_Movement_Speeds (Second Life Wiki), level-terrain running.
// Only used to migrate old absolute-speed preferences, never measured velocity.
const STANDARD_RUN_SPEED: f32 = 5.13;
// https://wiki.secondlife.com/wiki/Default_Avatar_Movement_Speeds
const STANDARD_FLY_SPEED: f32 = 16.0;
const STANDARD_FLY_DOWN_SPEED: f32 = 22.87;

/// Match bridge.lsl's horizontal camera frame for shore probes and passage.
/// Camera pitch must not turn either check into a ground/sky ray.
pub fn horizontal_heading(camera_forward: Vec3, body_forward: Vec3) -> Vec3 {
    let horizontal = camera_forward.with_z(0.0);
    if horizontal.is_finite() && horizontal.length_squared() >= 1.0e-6 {
        horizontal.normalize()
    } else {
        body_forward.with_z(0.0).normalize_or(Vec3::X)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Breaststroke,
    Backstroke,
    Crawl,
}
impl Style {
    pub const ALL: [Self; 3] = [Self::Breaststroke, Self::Backstroke, Self::Crawl];
    pub fn name(self) -> &'static str {
        match self {
            Self::Breaststroke => "Aurora Swim Breaststroke",
            Self::Backstroke => "Aurora Swim Backstroke",
            Self::Crawl => "Aurora Swim Crawl",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    #[serde(default, rename = "enabled", skip_serializing)]
    legacy_enabled: Option<bool>,
    pub running: bool,
    pub fly_assist: bool,
    pub swimming: bool,
    pub water_walk: bool,
    pub water_jumps: bool,
    pub pass_wall: bool,
    pub jump_obstacles: bool,
    pub run_multiplier: f32,
    pub swim_speed: f32,
    pub fly_multiplier: f32,
    #[serde(default, rename = "fly_speed", skip_serializing)]
    legacy_fly_speed: Option<f32>,
    pub float_height: f32,
    #[serde(default, rename = "run_speed", skip_serializing)]
    legacy_run_speed: Option<f32>,
    pub style: Style,
    /// Inventory items, not asset UUIDs: links and missing items are resolved at use.
    pub strokes: [Uuid; 3],
    pub discover_strokes: [bool; 3],
    pub water_clips: [Uuid; 3],
    #[serde(default)]
    controller_version: u8,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            legacy_enabled: None,
            running: false,
            fly_assist: false,
            swimming: false,
            water_walk: false,
            water_jumps: true,
            pass_wall: false,
            jump_obstacles: true,
            run_multiplier: 1.0,
            swim_speed: 5.0,
            fly_multiplier: 1.0,
            legacy_fly_speed: None,
            float_height: 0.0,
            legacy_run_speed: None,
            style: Style::default(),
            strokes: [Uuid::nil(); 3],
            discover_strokes: [true; 3],
            water_clips: [Uuid::nil(); 3],
            controller_version: 6,
        }
    }
}
impl Preferences {
    /// Selecting one water mode never implicitly enables the other on release.
    pub fn set_water_mode(&mut self, walking: bool, enabled: bool, swim_ready: bool) {
        if !walking && enabled && !swim_ready {
            return;
        }
        if walking {
            self.water_walk = enabled;
            if enabled {
                self.swimming = false;
            }
        } else {
            self.swimming = enabled;
            if enabled {
                self.water_walk = false;
            }
        }
    }
    pub fn any_enabled(&self) -> bool {
        self.running || self.fly_assist || self.swimming || self.water_walk || self.pass_wall
    }
    pub fn sanitize(&mut self) {
        // Older versions allowed both; keep the explicit water-walk choice.
        if self.water_walk {
            self.swimming = false;
        }
        let old_running_default = self.controller_version < 3;
        if self.controller_version == 0 {
            self.controller_version = 1;
        }
        if self.controller_version == 1 {
            self.controller_version = 2;
        }
        if self.controller_version == 2 {
            if let Some(speed) = self.legacy_run_speed.take() {
                // Old defaults become neutral x1; preserve custom boosts,
                // but never migrate a slowdown into the new enhancement.
                self.run_multiplier = if speed == 4.5 || speed == 8.0 {
                    1.0
                } else {
                    speed / STANDARD_RUN_SPEED
                };
            }
            self.controller_version = 3;
        }
        if let Some(enabled) = self.legacy_enabled.take() {
            if !enabled {
                self.running = false;
                self.fly_assist = false;
                self.swimming = false;
            } else if old_running_default {
                self.running = true;
            }
        }
        if self.controller_version < 5 && self.swim_speed == 2.0 {
            self.swim_speed = 5.0;
        }
        if self.controller_version < 6
            && let Some(speed) = self.legacy_fly_speed.take()
        {
            // Neutralize former defaults; preserve custom boosts without slowdowns.
            self.fly_multiplier = if speed == 10.0 || speed == 25.0 {
                1.0
            } else {
                speed / STANDARD_FLY_SPEED
            };
        }
        self.legacy_fly_speed = None;
        self.controller_version = 6;
        let bounded = |v: f32, min, max, fallback| if v.is_finite() { v.clamp(min, max) } else { fallback };
        self.run_multiplier = bounded(self.run_multiplier, 1.0, 5.0, 1.0);
        self.swim_speed = bounded(self.swim_speed, 0.5, 8.0, 5.0);
        self.fly_multiplier = bounded(self.fly_multiplier, 1.0, 5.0, 1.0);
        self.float_height = bounded(self.float_height, -0.5, 0.5, 0.0);
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Ground,
    Flight,
    Surface,
    Submerged,
    Leaving,
    WaterWalking,
}
impl State {
    pub fn label(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Ground => "movement-ground",
            Self::Flight => "movement-flight",
            Self::Surface => "movement-surface",
            Self::Submerged => "movement-submerged",
            Self::Leaving => "movement-leaving",
            Self::WaterWalking => "movement-water-walk",
        })
    }
    pub fn swimming(self) -> bool {
        matches!(self, Self::Surface | Self::Submerged)
    }
}
pub struct Frame {
    pub position: Vec3,
    pub velocity: Vec3,
    pub height: f32,
    pub water: Option<f32>,
    /// Lowest displayed body contact, when the posed skeleton is available.
    pub water_contact: Option<Vec3>,
    pub ground: Option<f32>,
    pub flying: bool,
    pub can_fly: bool,
    pub seated: bool,
    pub suspended: bool,
    pub locked: bool,
    pub bridge_ready: bool,
    pub running: bool,
    pub obstacle: bool,
    pub landing: Option<Vec3>,
    pub on_ground: bool,
    pub jumping: bool,
}
#[derive(Default)]
pub struct Runtime {
    pub state: State,
    pub water_jump: water_jump::Runtime,
    pub water_walk: water_walk::Runtime,
    /// Only restore a flight flag that this feature took over itself.
    previous_flight: Option<bool>,
    blocked_since: Option<Instant>,
    jump_until: Option<Instant>,
    shore_jump: Option<(Motion, Instant, Vec3)>,
    /// Own landing cleanup beyond the short motor pulse, including delayed flags.
    water_exit_until: Option<Instant>,
    water_exit_jump: bool,
    water_controls: swim::Controls,
    /// An ascent begun in water cannot become AutomaticFly after reaching land.
    water_ascent_held: bool,
    /// Water-relative depth/surface target; released after arrival.
    water_target: Option<(u8, f32)>,
    water_target_until: Option<Instant>,
    /// Last meaningful remaining distance; movement renews a surface return's budget.
    surface_progress: Option<f32>,
    exit_flight: bool,
    last_jump: Option<Instant>,
    swim_signal: Option<crate::world::PlayingAnimation>,
    swim_pose: Option<crate::ao::State>,
    /// Keep animation context when a free camera/focus loss releases physics.
    swim_inspection: Option<State>,
    entry_fall: bool,
    flight_exit_pose: bool,
    water_since: Option<Instant>,
    /// Entry inertia must not select idle depth before buoyancy reaches the surface.
    surface_on_entry: bool,
    /// Keep the old dive threshold while a higher surface target is settling.
    settled_float_height: f32,
    passage: Option<(Motion, Instant)>,
    pub passage_feedback: passage::Feedback,
    run: run::Guard,
    pub run_start: Option<Instant>,
    pub native_running: bool,
}
#[derive(Default)]
pub struct Step {
    pub motion: Motion,
    pub flying: Option<bool>,
}
impl Runtime {
    pub fn allows_idle_anchor(&self) -> bool {
        !self.checking_water_landing() && !self.water_jump.busy() && !self.water_walk.active() && self.passage.is_none()
    }
    pub fn checking_water_landing(&self) -> bool {
        self.water_exit_until.is_some() || self.state.swimming() || self.state == State::Leaving
    }
    pub fn cancel_water_exit(&mut self) {
        self.water_exit_until = None;
        self.shore_jump = None;
        self.jump_until = None;
        self.water_target = None;
        self.exit_flight = false;
        self.previous_flight = None;
    }
    pub fn native_fly_allowed(&self) -> bool {
        !self.water_ascent_held && !matches!(self.state, State::Surface | State::Submerged | State::Leaving | State::WaterWalking)
    }
    /// Shared render/AO intent without advancing animation continuity clocks.
    pub fn swim_intent(&self, input: &MoveInput) -> Option<crate::ao::State> {
        use crate::ao::State as AoState;
        let state = self.swim_inspection.unwrap_or(self.state);
        if self.passage.is_some() || self.water_jump.outro() {
            return None;
        }
        self.water_jump.pose().or_else(|| {
            (state.swimming() || state == State::Leaving || self.water_target.is_some_and(|(mode, _)| mode == 10)).then_some(
                if self.entry_fall {
                    AoState::WaterEntry
                } else if state == State::Leaving {
                    AoState::WaterShoreExit
                } else if self.flight_exit_pose {
                    AoState::WaterFlightExit
                } else if self.water_target.is_some_and(|(mode, _)| mode == 7) {
                    AoState::SwimmingDown
                } else if self.water_target.is_some_and(|(mode, _)| mode == 8 || mode == 10) {
                    AoState::SwimmingUp
                } else if input.down && state == State::Submerged {
                    AoState::SwimmingDown
                } else if input.up && state == State::Submerged {
                    AoState::SwimmingUp
                } else if input.forward || input.back || input.strafe_left || input.strafe_right {
                    if state == State::Submerged {
                        AoState::UnderwaterSwimming
                    } else {
                        AoState::SwimmingForward
                    }
                } else if state == State::Submerged {
                    AoState::UnderwaterFloating
                } else {
                    AoState::Floating
                },
            )
        })
    }
    /// A stable AO state signal. Physics and animations have separate owners.
    pub fn swim_signal(&mut self, now: Instant, input: &MoveInput) -> Option<(crate::ao::State, crate::world::PlayingAnimation)> {
        let state = self.swim_intent(input);
        self.swim_signal = state.map(|state| {
            crate::world::PlayingAnimation::from_signal(
                state.motion(),
                0,
                now,
                self.swim_signal.as_ref().filter(|_| self.swim_pose == Some(state)),
            )
        });
        self.swim_pose = state;
        state.zip(self.swim_signal)
    }
    pub fn cancel_water_gesture(&mut self) {
        self.water_controls = Default::default();
    }
    pub fn start_passage(&mut self, offset: Vec3, now: Instant) -> Option<Motion> {
        let motion = Motion::passage(offset.to_array())?;
        self.water_jump = Default::default();
        self.entry_fall = false;
        self.flight_exit_pose = false;
        // The script owns its 1.2 s deadline. Leave room for command delivery
        // before releasing the mode; retransmissions retain the same nonce.
        self.passage = Some((motion, now + Duration::from_millis(1500)));
        self.passage_feedback.start(motion.action, now);
        self.water_controls = Default::default();
        self.water_target = None;
        self.water_target_until = None;
        self.surface_progress = None;
        self.surface_on_entry = false;
        self.shore_jump = None;
        self.water_exit_until = None;
        self.jump_until = None;
        self.exit_flight = false;
        Some(motion)
    }
    pub fn update(&mut self, prefs: &Preferences, frame: &Frame, input: &MoveInput, now: Instant) -> Step {
        self.water_ascent_held = input.up && (self.water_ascent_held || !self.native_fly_allowed());
        self.swim_inspection = if frame.suspended && !frame.seated && prefs.swimming {
            self.swim_inspection
                .or(self.state.swimming().then_some(self.state))
                .filter(|state| {
                    frame.water.is_some_and(|water| {
                        water.is_finite()
                            && frame.position.is_finite()
                            && frame.position.z < water + frame.height * 0.5 + 0.5
                            && (*state == State::Submerged || frame.position.z > water - frame.height * 0.5 - 0.5)
                    })
                })
        } else {
            None
        };
        self.native_running = false;
        let mut step = Step::default();
        if let Some((motion, until)) = self.passage {
            if prefs.pass_wall && now < until && frame.bridge_ready && !frame.seated && !frame.suspended && !frame.locked {
                self.run_start = self.run.update(false, frame, now);
                step.motion = motion;
                return step;
            }
            self.passage = None;
        }
        let active = prefs.any_enabled() && frame.bridge_ready && !frame.seated && !frame.suspended;
        let ground_ascent = input.up && !self.water_ascent_held && frame.on_ground && supported_above_water(frame);
        if !active || !prefs.swimming || frame.locked || ground_ascent || self.water_exit_until.is_some_and(|until| now >= until) {
            self.water_exit_until = None;
        }
        if self.water_exit_until.is_some() && self.water_exit_jump {
            step.flying = Some(false);
        }
        let landed = self.water_exit_until.is_some() && frame.on_ground && frame.velocity.z.abs() < 1.5 && supported_above_water(frame);
        if landed {
            self.water_target = None;
            self.exit_flight = false;
            self.previous_flight = Some(false);
            self.shore_jump = None;
            self.jump_until = None;
            step.flying = Some(false);
        }
        let was_water_walking = self.water_walk.active();
        if let Some(motion) = self
            .water_walk
            .update(prefs.water_walk && !self.water_jump.busy(), frame, input, now)
        {
            self.state = if motion.mode == 4 { State::Leaving } else { State::WaterWalking };
            self.previous_flight = None;
            self.water_target = None;
            self.water_target_until = None;
            self.surface_on_entry = false;
            self.surface_progress = None;
            self.water_controls = Default::default();
            self.entry_fall = false;
            self.flight_exit_pose = false;
            self.shore_jump = None;
            self.jump_until = None;
            self.run_start = self.run.update(false, frame, now);
            self.native_running = false;
            step.motion = motion;
            step.flying = Some(false);
            return step;
        }
        let was_swimming = self.state.swimming();
        let water = frame.water.filter(|v| v.is_finite());
        if self.state != State::Surface
            || water.is_some_and(|w| {
                let surface = w - (frame.height * 0.5 - 0.4).clamp(0.25, 0.8) + prefs.float_height;
                (frame.position.z - surface).abs() < 0.1
            })
        {
            self.settled_float_height = prefs.float_height;
        }
        let dive_height = if self.state == State::Surface {
            prefs.float_height.min(self.settled_float_height)
        } else {
            prefs.float_height
        };
        let flotation_depth = (frame.height * 0.5 - 0.4).clamp(0.25, 0.8);
        let surface_offset = -flotation_depth + prefs.float_height;
        let dive_offset = -flotation_depth + dive_height;
        let deep = water.zip(frame.ground).is_some_and(|(w, g)| w - g > frame.height * 0.55);
        if let Some((8, offset)) = self.water_target
            && let Some(water) = water
        {
            let distance = (water + offset - frame.position.z).abs();
            if self.surface_progress.is_some_and(|previous| (distance - previous).abs() >= 0.05) {
                // Falling can deepen the destination after entry. Keep the
                // same budget as a fresh surface tap at the observed depth.
                self.water_target_until = Some(Self::water_deadline(8, distance, prefs.swim_speed, now));
                self.surface_progress = Some(distance);
            }
        }
        if self.water_target_until.is_some_and(|until| now >= until) || (self.exit_flight && !frame.can_fly) {
            self.water_target = None;
            self.exit_flight = false;
        }
        if !deep && !self.exit_flight {
            self.water_target = None;
        }
        if !active || !prefs.swimming {
            self.jump_until = None;
            self.blocked_since = None;
            self.shore_jump = None;
            self.water_target = None;
            self.exit_flight = false;
        }
        if input.down {
            if self.exit_flight || self.water_target.is_some_and(|(mode, _)| mode == 8) {
                self.water_target = None;
            }
            self.exit_flight = false;
            self.shore_jump = None;
        }
        if self.exit_flight && water.is_some_and(|w| frame.position.z > w + frame.height * 0.5 + 1.15) {
            self.exit_flight = false;
        }
        if self
            .shore_jump
            .is_some_and(|(_, until, target)| now >= until || frame.position.distance(target) < 0.2)
        {
            self.shore_jump = None;
        }
        let leaving = self.jump_until.is_some_and(|until| now < until) || self.shore_jump.is_some();
        let wet = active
            && prefs.swimming
            && deep
            && !landed
            && !leaving
            && !self.exit_flight
            && water.is_some_and(|w| frame.position.z < w + if was_swimming { 0.45 + surface_offset.max(0.0) } else { -0.05 });
        if wet && !was_swimming {
            self.entry_fall = !frame.flying && frame.velocity.z < -0.5;
        }
        if !wet || frame.velocity.z >= -0.5 || input.up || input.down {
            self.entry_fall = false;
        }
        if wet && !was_swimming && (self.previous_flight.is_none() || self.state == State::Leaving) {
            self.surface_on_entry = true;
        }
        if !wet || input.down || frame.locked {
            self.surface_on_entry = false;
        }
        let return_to_surface = self.surface_on_entry
            && !Self::surface_reached(frame, surface_offset)
            && water.is_some_and(|w| frame.position.z < w + surface_offset - 0.12 || frame.velocity.z < -0.5 || !frame.on_ground);
        if Self::surface_reached(frame, surface_offset) {
            self.surface_on_entry = false;
        }
        self.state = if wet {
            if self.water_target.is_some_and(|(mode, _)| mode == 7) {
                State::Submerged
            } else if let Some((8, offset)) = self.water_target {
                if Self::surface_reached(frame, offset) {
                    State::Surface
                } else {
                    State::Submerged
                }
            } else if water.is_some_and(|w| frame.position.z < w + dive_offset - if self.state == State::Submerged { 0.18 } else { 0.42 }) {
                State::Submerged
            } else {
                State::Surface
            }
        } else if active && leaving {
            State::Leaving
        } else if frame.flying {
            State::Flight
        } else {
            State::Ground
        };
        if wet && !was_swimming {
            self.water_since = Some(now);
            self.previous_flight.get_or_insert(frame.flying);
            // Swimming is a damped physical motor, not native flight.
            step.flying = Some(false);
        } else if !wet
            && !leaving
            && !frame.suspended
            && let Some(previous) = self.previous_flight.take()
            && !frame.seated
        {
            step.flying = Some(previous && !frame.on_ground);
        }
        if let Some((mode, offset)) = self.water_target
            && water.is_none_or(|w| {
                if mode == 7 {
                    frame.position.z <= w + offset + 0.12
                } else if mode == 10 {
                    frame.position.z >= w + offset - 0.12
                } else {
                    Self::surface_reached(frame, offset)
                }
            })
        {
            self.water_target = None;
            if mode == 10 {
                self.exit_flight = false;
            }
        }
        let action = self
            .water_controls
            .update(
                active && prefs.swimming,
                if was_water_walking { State::Surface } else { self.state },
                return_to_surface || self.water_target.is_some_and(|(mode, _)| mode == 8),
                frame,
                input,
                now,
            )
            .or(return_to_surface.then_some(swim::Action::Surface));
        if action.is_some() {
            self.surface_on_entry = false;
        }
        match action {
            Some(swim::Action::Dive) => {
                self.water_target = Some((7, -(frame.height * 0.5 + 1.1).max(1.8)));
                self.state = State::Submerged;
                self.exit_flight = false;
                step.flying = Some(false);
            }
            Some(swim::Action::Surface) => {
                self.water_target = Some((8, surface_offset));
                if wet && !Self::surface_reached(frame, surface_offset) {
                    self.state = State::Submerged;
                }
            }
            Some(swim::Action::Jump) => {
                self.water_target = None;
                self.previous_flight = Some(false);
                self.state = State::Leaving;
                step.flying = Some(false);
                self.water_exit_until = Some(now + Duration::from_secs(5));
                self.water_exit_jump = true;
                if let Some(target) = frame.landing.filter(|_| frame.obstacle)
                    && let Some(motion) = Motion::shore_jump((target - frame.position).to_array())
                {
                    self.shore_jump = Some((motion, now + Duration::from_millis(2400), target));
                } else {
                    self.jump_until = Some(now + Duration::from_millis(650));
                }
            }
            Some(swim::Action::Fly) => {
                self.water_target = Some((10, frame.height * 0.5 + 1.25));
                self.water_exit_until = Some(now + Duration::from_secs(5));
                self.water_exit_jump = false;
                self.exit_flight = true;
                self.previous_flight = None;
                self.shore_jump = None;
                self.jump_until = None;
                self.state = State::Flight;
                step.flying = Some(true);
            }
            None => {}
        }
        if matches!(action, Some(swim::Action::Dive | swim::Action::Surface | swim::Action::Fly))
            && let Some((mode, offset)) = self.water_target
            && let Some(water) = water
        {
            // Give long ascents enough time, but never keep an unreachable
            // destination (a ceiling or the seabed) owning the motor forever.
            let distance = (water + offset - frame.position.z).abs();
            self.water_target_until = Some(Self::water_deadline(mode, distance, prefs.swim_speed, now));
            self.surface_progress = (mode == 8).then_some(distance);
        } else if self.water_target.is_none() {
            self.water_target_until = None;
            self.surface_progress = None;
        }
        let local = Vec3::new(
            input.forward as u8 as f32 - input.back as u8 as f32,
            input.strafe_left as u8 as f32 - input.strafe_right as u8 as f32,
            input.up as u8 as f32 - input.down as u8 as f32,
        );
        let translating = local.x != 0.0 || local.y != 0.0;
        let stuck = wet
            && prefs.jump_obstacles
            && self.water_target.is_none()
            && self.state == State::Surface
            && local.x > 0.0
            && frame.velocity.truncate().length() < 0.4
            && frame.velocity.z.abs() < 0.3
            && self
                .water_since
                .is_some_and(|since| now.duration_since(since) >= Duration::from_secs(1))
            && frame.obstacle
            && !frame.locked;
        if stuck {
            let since = *self.blocked_since.get_or_insert(now);
            if now.duration_since(since) >= Duration::from_millis(800)
                && self.last_jump.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(3))
            {
                self.jump_until = now.checked_add(Duration::from_millis(650));
                if let Some(target) = frame.landing
                    && let Some(motion) = Motion::shore_jump((target - frame.position).to_array())
                {
                    self.shore_jump = Some((motion, now + Duration::from_millis(2400), target));
                }
                self.water_exit_until = Some(now + Duration::from_secs(5));
                self.water_exit_jump = true;
                self.last_jump = Some(now);
                self.state = State::Leaving;
                self.previous_flight = Some(false);
                step.flying = Some(false);
            }
        } else {
            self.blocked_since = None;
        }
        let flying = step.flying.unwrap_or(frame.flying);
        self.flight_exit_pose = self.exit_flight && water.is_some_and(|w| frame.position.z >= w - frame.height * 0.5);
        if !self.state.swimming() && self.state != State::Leaving {
            self.state = if flying { State::Flight } else { State::Ground };
        }
        // Arm LSL's control capture even while Movelock holds us. Only the
        // script decides when native control events release that hold.
        if active
            && (!frame.locked
                || self.state.swimming()
                || (flying && prefs.fly_assist)
                || self.water_target.is_some_and(|(mode, _)| mode == 10))
        {
            let speed = if self.state.swimming() || self.state == State::Leaving || self.water_target.is_some() {
                prefs.swim_speed
            } else if flying {
                let normal = if input.down && !input.up {
                    STANDARD_FLY_DOWN_SPEED
                } else {
                    STANDARD_FLY_SPEED
                };
                normal * prefs.fly_multiplier
            } else {
                prefs.run_multiplier
            };
            let mode = match self.state {
                State::Leaving => 4,
                State::Surface => self.water_target.map_or(3, |(mode, _)| mode),
                State::Submerged => self.water_target.map_or(6, |(mode, _)| mode),
                _ if self.water_target.is_some_and(|(mode, _)| mode == 10) => 10,
                _ if flying && prefs.fly_assist && prefs.fly_multiplier > 1.0 && frame.can_fly => 2,
                _ if !flying
                    && prefs.running
                    && prefs.run_multiplier > 1.0
                    && frame.running
                    && translating
                    && !input.down
                    && !input.up
                    && !frame.jumping =>
                {
                    1
                }
                _ => 0,
            };
            if mode != 0 {
                step.motion = Motion::motor(mode, speed, frame.height);
                if mode == 4 {
                    step.motion.parameters[2] = 4.5;
                } else if let Some((_, offset)) = self.water_target {
                    step.motion.parameters[2] = offset;
                } else if mode == 3 || mode == 6 {
                    step.motion.parameters[2] = prefs.float_height;
                }
            }
        }
        if let Some((motion, _, _)) = self.shore_jump {
            step.motion = motion;
            self.state = State::Leaving;
            step.flying = Some(false);
        }
        if self.water_exit_until.is_some() && self.state == State::Leaving {
            step.flying = Some(false);
        }
        self.run_start = self.run.update(step.motion.mode == 1 && !input.up && !frame.jumping, frame, now);
        self.native_running = frame.running
            && translating
            && frame.on_ground
            && frame.velocity.z.abs() <= 1.5
            && !frame.jumping
            && !frame.locked
            && !input.up
            && !input.down
            && !frame.flying
            && !frame.seated
            && !frame.suspended
            && self.state == State::Ground;
        step
    }
    fn surface_reached(frame: &Frame, offset: f32) -> bool {
        // Passing the waterline during a fast fall is not a completed ascent.
        frame.water.is_some_and(|w| (frame.position.z - w - offset).abs() <= 0.18) && (-0.5..=1.0).contains(&frame.velocity.z)
    }
    pub fn cancel_for_cannonball(&mut self) {
        self.water_exit_until = None;
        self.passage = None;
        self.water_target = None;
        self.water_target_until = None;
        self.shore_jump = None;
        self.jump_until = None;
        self.exit_flight = false;
        self.previous_flight = None;
        self.surface_on_entry = false;
        self.entry_fall = false;
        self.flight_exit_pose = false;
        self.run_start = None;
    }
    fn water_deadline(mode: u8, distance: f32, speed: f32, now: Instant) -> Instant {
        let minimum = if mode == 8 { 5.0 } else { 2.0 };
        let seconds = (distance / speed * 2.0 + 2.0).clamp(minimum, 60.0);
        now + Duration::try_from_secs_f32(seconds).unwrap_or(Duration::from_secs(5))
    }
}

fn supported_above_water(frame: &Frame) -> bool {
    frame
        .water
        .is_some_and(|water| frame.position.z - frame.height * 0.5 >= water - 0.1)
}

/// Keep simulator jump and fast-flight controls out of the swim motor.
/// Movelock never masks input: the script observes controls with pass-through.
pub fn controls(mut flags: u32, state: State, captured: bool) -> u32 {
    use aurora_net::control as c;
    if state.swimming() || matches!(state, State::Leaving | State::WaterWalking) || captured {
        flags &= !c::FAST_UP;
        // Once LSL has acknowledged its capture, it must receive up/down
        // too so Movelock can detect a voluntary dive without an HTTP relay.
        if !captured {
            flags &= !(c::UP_POS | c::UP_NEG);
        }
    }
    flags
}
