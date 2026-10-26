//! Walking our avatar to a point by itself, and the double click on the
//! ground that starts it (Preferences › Controls › "Double-click on the
//! ground", the wolf menu › Me).
//!
//! Port of LLAgent::startAutoPilotGlobal / autoPilot / stopAutoPilot
//! (indra/newview/llagent.cpp), LLToolPie::walkToClickedLocation and
//! teleportToClickedLocation (indra/newview/lltoolpie.cpp), originally LGPL
//! 2.1. Like the Legacy Viewer, the viewer only turns the body and sends the walk
//! flags of `AgentUpdate`; the simulator moves the avatar. Firestorm offers
//! no action, walk or teleport; "run" is the walk with running on
//! (SetAlwaysRun), Aurora's own.

use aurora_net::control;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// What a double click on the ground does: the `walk_to` / `teleport_to`
/// double-click bindings of Firestorm's third-person mode
/// (LLFloaterPreference::updateClickActionControls), no action by default
/// as in its key_bindings.xml.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum DoubleClick {
    #[default]
    Nothing,
    Walk,
    Run,
    Teleport,
}

impl DoubleClick {
    pub const ALL: [DoubleClick; 4] = [DoubleClick::Nothing, DoubleClick::Walk, DoubleClick::Run, DoubleClick::Teleport];

    pub fn label(self) -> &'static str {
        match self {
            DoubleClick::Nothing => aurora_i18n::tr!("ui-options-double-click-nothing"),
            DoubleClick::Walk => aurora_i18n::tr!("ui-options-double-click-walk"),
            DoubleClick::Run => aurora_i18n::tr!("ui-options-double-click-run"),
            DoubleClick::Teleport => aurora_i18n::tr!("ui-options-double-click-teleport"),
        }
    }

    /// The Tabler icon of the choice in the wolf menu.
    pub fn icon(self) -> &'static str {
        match self {
            DoubleClick::Nothing => "hand-stop",
            DoubleClick::Walk => "walk",
            DoubleClick::Run => "run",
            DoubleClick::Teleport => "map-pin-filled",
        }
    }
}

/// Two presses of the left button make a double click within the system's
/// double-click time and box, as the window messages Firestorm reads.
#[derive(Default)]
pub struct DoubleClickDetector {
    last: Option<(Instant, (f32, f32))>,
}

/// GetDoubleClickTime / SM_CXDOUBLECLK on Windows; their defaults elsewhere.
fn system_double_click() -> (Duration, f32) {
    #[cfg(windows)]
    {
        // SAFETY: two reads of system settings, without arguments to check.
        let (ms, px) = unsafe {
            (
                windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime(),
                windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows_sys::Win32::UI::WindowsAndMessaging::SM_CXDOUBLECLK),
            )
        };
        if ms > 0 && px > 0 {
            return (Duration::from_millis(ms as u64), px as f32);
        }
    }
    (Duration::from_millis(500), 4.0)
}

impl DoubleClickDetector {
    /// A press of the left button on the world at `at` (window pixels):
    /// the second of a double click? A third press starts a new pair.
    pub fn press(&mut self, now: Instant, at: (f32, f32)) -> bool {
        let (time, size) = system_double_click();
        self.press_within(now, at, time, size)
    }

    fn press_within(&mut self, now: Instant, at: (f32, f32), time: Duration, size: f32) -> bool {
        let double = self.last.is_some_and(|(t, (x, y))| {
            now.saturating_duration_since(t) <= time && (at.0 - x).abs() <= size / 2.0 && (at.1 - y).abs() <= size / 2.0
        });
        self.last = if double { None } else { Some((now, at)) };
        double
    }

    /// A press elsewhere (a window, a HUD): no double click across it.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// AUTOPILOT_MAX_TIME_NO_PROGRESS_WALK: the walk gives up after this long
/// without progress. Successful movement resets this consecutive stall timer.
const MAX_TIME_NO_PROGRESS: f32 = 1.5;

/// What a press of the left button on the world was, once the HUDs, the
/// media faces, the click actions and the windows have had theirs.
#[derive(Debug, Clone, Copy, Default)]
pub struct Press {
    pub double: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub build: bool,
    pub on_avatar: bool,
}

/// The double click on the ground takes this press: the second of a
/// double click, without a modifier (Alt orbits the camera: the bindings
/// of `walk_to` / `teleport_to` are MASK_NONE), outside the build tools
/// and away from our own avatar (teleportToClickedLocation refuses it).
pub fn takes_ground_double_click(p: Press) -> bool {
    p.double && !p.alt && !p.ctrl && !p.shift && !p.build && !p.on_avatar
}

/// AU_FLAGS_CLIENT_AUTOPILOT of `AgentUpdate` (send_agent_update).
pub const AU_FLAGS_CLIENT_AUTOPILOT: u8 = 0x02;
/// AU_FLAGS_HIDETITLE: the simulator hides our group title from everyone
/// (RenderHideGroupTitle).
pub const AU_FLAGS_HIDETITLE: u8 = 0x01;

/// How long the destination stays marked (the HUD blob of
/// walkToClickedLocation lasts 3 s).
pub const MARK_DURATION: Duration = Duration::from_secs(3);

/// Aurora's follow controls. The Legacy Viewer's startFollowPilot accepts
/// a stop distance but has no live controls or teleport recovery.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FollowSettings {
    pub distance: f32,
    pub teleport_if_stuck: bool,
}

impl Default for FollowSettings {
    fn default() -> Self {
        Self {
            distance: 2.0,
            teleport_if_stuck: false,
        }
    }
}

impl FollowSettings {
    pub const MIN_DISTANCE: f32 = 0.5;
    pub const MAX_DISTANCE: f32 = 20.0;

    pub fn bounded_distance(self) -> f32 {
        if self.distance.is_finite() {
            self.distance.clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE)
        } else {
            Self::default().distance
        }
    }
}

/// A walk under way.
#[derive(Debug, Clone)]
pub struct AutoPilot {
    /// Where to: the clicked point, raised to the avatar's centre
    /// (setAutoPilotTargetGlobal keeps the target `PelvisToFoot` above the
    /// ground).
    pub target: Vec3,
    stop_distance: f32,
    target_dist: f32,
    no_progress: f32,
    /// Running while walking there.
    pub run: bool,
    pub started: Instant,
    /// What to do once there (the finish callback of startAutoPilotGlobal).
    pub on_arrival: Option<Arrival>,
    /// A persistent follow keeps its pilot while resting near the leader.
    pub leader: Option<uuid::Uuid>,
    /// Also retained by Go to, so a late animation packet can enable flight
    /// without turning its fixed destination into a persistent follow.
    pub subject: Option<uuid::Uuid>,
    pub flying: bool,
    pub avatar_flight: Option<FlightTarget>,
    waiting: bool,
    previous_position: Vec3,
    landing_offset: Vec3,
    target_velocity: Vec3,
    flight_destination: Option<Vec3>,
    horizontal_thrust: FlightAxis,
    vertical_thrust: FlightAxis,
    /// A second blocked arrival must stop, rather than teleport in a loop.
    /// Genuine travel of at least a metre permits a later recovery.
    recovery_origin: Option<Vec3>,
}

/// The finish callback of a walk: run when it ends within its stop
/// distance and was not cancelled (LLAgent::stopAutoPilot's `success`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrival {
    /// Land.Sit / near_sit_down_point: sit only after a successful approach.
    SitGround,
    /// "Put on" of a rezzed object (LLObjectAttachToAvatar::onNearAttachObject).
    Attach { object: uuid::Uuid, point: u8 },
}

/// Why a walk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Arrived,
    /// No nearer for too long (an obstacle).
    NoProgress,
}

/// What a frame of the walk asks for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    /// Turn of the body this frame (radians, + to the left).
    pub yaw: f32,
    pub flags: u32,
    pub finish: Option<Finish>,
}

/// A known locomotion mode and the altitude at which our physical centre
/// would rest on the target's support. Missing animation signals retain the
/// last known mode; a jump or an unloaded AO is not a landing command.
#[derive(Debug, Clone, Copy)]
pub struct FlightTarget {
    pub flying: Option<bool>,
    pub landing_height: f32,
}

pub struct AvatarTarget {
    pub position: Vec3,
    pub flight: FlightTarget,
    pub velocity: Vec3,
    pub running: bool,
}

/// Land beside the precise leader, at our own standing height on its
/// support. Coarse radar positions never authorize an automatic teleport.
pub fn follow_recovery_position(from: Vec3, target: &AvatarTarget, settings: FollowSettings) -> Option<Vec3> {
    if !from.is_finite() || !target.position.is_finite() || !target.flight.landing_height.is_finite() {
        return None;
    }
    let away = (from - target.position).with_z(0.0).normalize_or(Vec3::NEG_X);
    let position = target.position + away * (settings.bounded_distance() - 0.1).max(0.4);
    Some(if target.flight.flying == Some(false) {
        position.with_z(target.flight.landing_height)
    } else {
        position
    })
}

/// Native flight retains inertia and NUDGE does not bound its speed. Plan
/// one continuous burn, then keep that axis released until the target moves.
/// The burn budget also bounds short trips before velocity reports arrive.
#[derive(Debug, Clone, Default)]
struct FlightAxis {
    remaining: Option<f32>,
}

impl FlightAxis {
    fn advance(&mut self, gap: f32, closing: f32, speed: f32, response: FlightResponse, dt: f32) -> bool {
        let remaining = self
            .remaining
            .get_or_insert_with(|| ((gap - closing * response.coast) / speed).max(0.0));
        if gap <= 0.0 || *remaining <= 0.0 || (closing > 0.1 && closing * response.coast + speed * response.delay >= gap) {
            *remaining = 0.0;
            return false;
        }
        *remaining = (*remaining - dt).max(0.0);
        true
    }

    fn released(&self) -> bool {
        self.remaining == Some(0.0)
    }
}

/// Motion still to happen after the latest physical report: coasting and
/// the input in transit. Keep report age out of drawable prediction.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlightResponse {
    coast: f32,
    delay: f32,
    speed: f32,
}

impl AutoPilot {
    pub fn set_follow_distance(&mut self, distance: f32, position: Vec3) {
        let distance = FollowSettings {
            distance,
            ..Default::default()
        }
        .bounded_distance();
        if self.leader.is_some() && self.stop_distance != distance {
            self.stop_distance = distance;
            if self.landing_offset != Vec3::ZERO {
                self.landing_offset = self.landing_offset.normalize_or(Vec3::NEG_X) * (distance - 0.2).max(1.0);
            }
            self.reset_progress(position);
        }
    }

    pub fn recovered(&mut self, position: Vec3) {
        self.recovery_origin = Some(position);
    }

    pub fn can_recover(&self) -> bool {
        self.leader.is_some() && self.recovery_origin.is_none()
    }

    /// startAutoPilotGlobal without a stop distance: the square root of the
    /// distance, 0.5 m at least. Never flying (walkToClickedLocation).
    pub fn start(from: Vec3, target: Vec3, run: bool, now: Instant) -> AutoPilot {
        let distance = from.distance(target);
        AutoPilot {
            target,
            stop_distance: distance.sqrt().max(0.5),
            target_dist: distance,
            no_progress: 0.0,
            run,
            started: now,
            on_arrival: None,
            leader: None,
            subject: None,
            flying: false,
            avatar_flight: None,
            waiting: false,
            previous_position: from,
            landing_offset: Vec3::ZERO,
            target_velocity: Vec3::ZERO,
            flight_destination: None,
            horizontal_thrust: FlightAxis::default(),
            vertical_thrust: FlightAxis::default(),
            recovery_origin: None,
        }
    }

    /// startAutoPilotGlobal with a stop distance and a finish callback.
    pub fn start_to(from: Vec3, target: Vec3, stop_distance: f32, arrival: Arrival, now: Instant) -> AutoPilot {
        AutoPilot {
            stop_distance,
            on_arrival: Some(arrival),
            ..AutoPilot::start(from, target, false, now)
        }
    }

    /// Avatar approach, using LLAgent::startFollowPilot's target identity.
    /// Unlike the Legacy Viewer, arrival rests at 2 m and following resumes
    /// only beyond 3 m. Ground travel compares XY, so different pelvis heights
    /// cannot keep the avatar turning or walking through its destination.
    pub fn avatar(from: Vec3, target: Vec3, leader: Option<uuid::Uuid>, flying: bool, now: Instant) -> Self {
        Self {
            stop_distance: 2.0,
            leader,
            subject: leader,
            flying,
            avatar_flight: Some(FlightTarget {
                flying: None,
                landing_height: target.z,
            }),
            ..Self::start(from, target, false, now)
        }
    }

    pub fn shift(&mut self, delta: Vec3) {
        self.target += delta;
        self.previous_position += delta;
        if let Some(destination) = &mut self.flight_destination {
            *destination += delta;
        }
        if let Some(origin) = &mut self.recovery_origin {
            *origin += delta;
        }
        if let Some(flight) = &mut self.avatar_flight {
            flight.landing_height += delta.z;
        }
    }

    pub fn observe_flight(&mut self, observed: FlightTarget, position: Vec3) {
        let Some(flight) = &mut self.avatar_flight else { return };
        let changed = observed.flying.is_some() && observed.flying != flight.flying;
        if observed.flying.is_some() {
            flight.flying = observed.flying;
        }
        flight.landing_height = observed.landing_height;
        if changed {
            self.landing_offset = Vec3::ZERO;
            if observed.flying == Some(false) && self.flying {
                let away = (position - self.target).with_z(0.0);
                if away.length_squared() < 1.0 {
                    // A leader can move directly under us. Move beside them
                    // before descending rather than landing on their head.
                    self.landing_offset = away.normalize_or(Vec3::NEG_X) * (self.stop_distance - 0.2).max(1.0);
                }
            }
            self.reset_progress(position);
        }
    }

    pub fn observe_avatar(&mut self, observed: AvatarTarget, position: Vec3) {
        if self.leader.is_some() {
            self.target = observed.position;
            self.target_velocity = observed.velocity;
            let run_distance = self.stop_distance + if self.run { 0.5 } else { 1.0 };
            self.run = observed.running
                && !self.flying
                && observed.flight.flying != Some(true)
                && (self.target - position).truncate().length() > run_distance;
        }
        self.observe_flight(observed.flight, position);
    }

    pub fn wants_flight(&self) -> bool {
        self.avatar_flight.is_some_and(|f| f.flying == Some(true))
    }

    pub fn landing(&self) -> bool {
        self.flying && self.avatar_flight.is_some_and(|f| f.flying == Some(false))
    }

    pub fn set_flying(&mut self, flying: bool, position: Vec3) {
        if self.flying != flying {
            self.flying = flying;
            self.reset_progress(position);
        }
    }

    fn reset_progress(&mut self, position: Vec3) {
        self.waiting = false;
        self.no_progress = 0.0;
        self.previous_position = position;
        self.target_dist = self.distance(self.goal() - position);
        self.flight_destination = None;
        self.horizontal_thrust = FlightAxis::default();
        self.vertical_thrust = FlightAxis::default();
    }

    fn goal(&self) -> Vec3 {
        if self.landing() {
            (self.target + self.landing_offset).with_z(self.avatar_flight.map_or(self.target.z, |f| f.landing_height))
        } else {
            self.target
        }
    }

    fn landing_radius(&self) -> f32 {
        if self.landing_offset == Vec3::ZERO {
            self.stop_distance
        } else {
            0.5
        }
    }

    fn distance(&self, delta: Vec3) -> f32 {
        if self.flying { delta.length() } else { delta.truncate().length() }
    }

    /// Ground arrival uses XY; hovering also requires matching height.
    /// A landing must finish on
    /// confirmed support before proximity alone can end the approach.
    pub fn reached(&self, position: Vec3) -> bool {
        let awaiting_flight =
            self.avatar_flight.is_some_and(|f| f.flying.is_none()) && (self.target.z - position.z).abs() > self.stop_distance;
        !awaiting_flight
            && !self.landing()
            && if self.flying && self.avatar_flight.is_some() {
                (self.target - position).truncate().length() <= self.stop_distance && (self.target.z - position.z).abs() <= 0.2
            } else {
                self.distance(self.target - position) <= self.stop_distance
            }
    }

    /// One frame of LLAgent::autoPilot from the avatar's place and heading,
    /// `dt` seconds after the last (1 / gFPSClamped).
    #[cfg(test)]
    pub fn step(&mut self, position: Vec3, yaw: f32, dt: f32) -> Step {
        self.step_moving(position, Vec3::ZERO, yaw, dt, FlightResponse::default())
    }

    pub fn step_moving(&mut self, position: Vec3, velocity: Vec3, yaw: f32, dt: f32, response: FlightResponse) -> Step {
        let stop = |finish| Step {
            yaw: 0.0,
            flags: control::STOP,
            finish,
        };
        if !position.is_finite() || !self.target.is_finite() || !self.goal().is_finite() || !yaw.is_finite() || !dt.is_finite() {
            return stop(Some(Finish::NoProgress));
        }
        if self.recovery_origin.is_some_and(|origin| position.distance(origin) >= 1.0) {
            self.recovery_origin = None;
        }
        let dt = dt.clamp(0.0, 0.1);
        let at = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        let landing = self.landing();
        let braking = self.flying && self.avatar_flight.is_some() && response.speed > 0.0;
        if braking && self.flight_destination.is_none_or(|goal| goal.distance(self.goal()) > 1.0) {
            self.flight_destination = Some(self.goal());
            self.horizontal_thrust = FlightAxis::default();
            self.vertical_thrust = FlightAxis::default();
        }
        let delta = self.goal() - position;
        let xy_distance = delta.truncate().length();
        let target_dist = self.distance(delta);
        let settled = velocity.is_finite() && velocity.length() < 0.1;
        let coasted_nearby = braking
            && !landing
            && self.horizontal_thrust.released()
            && self.vertical_thrust.released()
            && xy_distance <= self.stop_distance + 0.75
            && delta.z.abs() <= 0.5;
        if (self.reached(position) || coasted_nearby) && (!braking || settled) {
            self.no_progress = 0.0;
            self.previous_position = position;
            self.waiting = self.leader.is_some();
            return stop(self.leader.is_none().then_some(Finish::Arrived));
        }
        if self.waiting {
            self.previous_position = position;
            self.no_progress = 0.0;
            self.target_dist = target_dist;
            if xy_distance <= self.stop_distance + 1.0 && (!self.flying || delta.z.abs() <= 1.0) {
                return stop(None);
            }
            self.reset_progress(position);
            self.flight_destination = Some(self.goal());
        }
        // Follow must measure our own progress: a leader walking away can
        // increase the remaining distance even while we are moving correctly.
        let travelled = (position - self.previous_position).dot(delta.normalize_or_zero());
        let progress = if self.leader.is_some() {
            travelled > 0.001
        } else {
            target_dist < self.target_dist - 0.001
        };
        self.no_progress = if progress { 0.0 } else { self.no_progress + dt };
        self.previous_position = position;
        self.target_dist = target_dist;
        let stall_limit = if braking {
            MAX_TIME_NO_PROGRESS.max((response.coast + response.delay) * 4.0 + 1.0)
        } else {
            MAX_TIME_NO_PROGRESS
        };
        if self.no_progress > stall_limit {
            return stop(Some(Finish::NoProgress));
        }
        if !self.flying
            && self.avatar_flight.is_some_and(|f| f.flying.is_none())
            && delta.z.abs() > self.stop_distance
            && xy_distance <= self.stop_distance
        {
            // Do not complete Go to underneath an airborne target while its
            // animation packet is still in flight. Wait without walking loops;
            // the normal stall deadline still bounds missing information.
            return stop(None);
        }
        let horizontal_stop = if landing && self.landing_offset != Vec3::ZERO {
            0.5
        } else if self.flying && self.avatar_flight.is_some() {
            (self.stop_distance - 0.1).max(0.5)
        } else if self.flying {
            0.5
        } else {
            0.0
        };
        let direction = if self.flying && xy_distance <= horizontal_stop {
            Vec3::ZERO
        } else {
            Vec3::new(delta.x, delta.y, 0.0).normalize_or_zero()
        };
        let angle = if direction == Vec3::ZERO {
            0.0
        } else {
            at.cross(direction).z.atan2(at.dot(direction))
        };
        let turn = angle * (4.0 * dt).min(1.0);
        let mut flags = if angle > 0.001 {
            control::YAW_POS
        } else if angle < -0.001 {
            control::YAW_NEG
        } else {
            0
        };
        let slow_distance = (self.stop_distance + if self.flying { 5.0 } else { 2.0 }).max(3.0);
        if at.dot(direction) > 0.9 && xy_distance > horizontal_stop {
            flags |= if self.flying && xy_distance <= slow_distance {
                control::NUDGE_AT_POS
            } else {
                control::AT_POS
            };
            if xy_distance > slow_distance {
                flags |= control::FAST_AT;
            }
        }
        let vertical_tolerance = if landing || self.avatar_flight.is_some() { 0.1 } else { 0.5 };
        // Descend only near the landed leader. Otherwise crossing a lower
        // roof or the edge of a raised platform could end flight too soon.
        let descend_radius = if self.landing_offset == Vec3::ZERO {
            self.landing_radius() + 1.0
        } else {
            self.landing_radius()
        };
        if self.flying && delta.z.abs() > vertical_tolerance && (delta.z > 0.0 || !landing || xy_distance <= descend_radius) {
            flags |= if delta.z > 0.0 { control::UP_POS } else { control::UP_NEG };
            if delta.z.abs() > 6.0 {
                flags |= control::FAST_UP;
            }
        }
        if landing && xy_distance <= self.landing_radius() && delta.z <= 0.5 {
            // Keep descending through the final arrival radius until the
            // simulator reports our own contact. Never dive below the
            // target's support if the nearby platform does not extend here.
            // Allow the final quarter metre for body-size/contact tolerance;
            // stopping above it could prevent the support report entirely.
            flags = if delta.z <= 0.25 { control::UP_NEG } else { control::STOP };
        }
        let mut turn = turn;
        if braking {
            let relative = if velocity.is_finite() {
                velocity - self.target_velocity
            } else {
                Vec3::ZERO
            };
            let closing = relative.truncate().dot(delta.truncate().normalize_or_zero()).max(0.0);
            let horizontal = control::AT_POS | control::NUDGE_AT_POS | control::FAST_AT;
            // A turn is not part of the burn. Once released, do not turn
            // back toward a target whose stopping envelope we just entered.
            let advancing = flags & horizontal != 0;
            if (advancing || self.horizontal_thrust.released())
                && !self
                    .horizontal_thrust
                    .advance(xy_distance - horizontal_stop, closing, response.speed, response, dt)
            {
                flags &= !horizontal;
                if closing > 0.1 || xy_distance <= horizontal_stop {
                    flags &= !(control::YAW_POS | control::YAW_NEG);
                    turn = 0.0;
                }
            }
            if xy_distance <= horizontal_stop {
                self.horizontal_thrust.remaining = Some(0.0);
            }
            // Hover at the target's height, rather than stopping at the
            // edge of a 2 m sphere below it. Landing keeps descending to
            // physical contact through World::auto_land.
            if !landing
                && !self
                    .vertical_thrust
                    .advance(delta.z.abs(), (relative.z * delta.z.signum()).max(0.0), 4.0, response, dt)
            {
                flags &= !(control::UP_POS | control::UP_NEG | control::FAST_UP);
            }
            if flags == 0 {
                flags = control::STOP;
            }
        }
        Step {
            yaw: turn,
            flags,
            finish: None,
        }
    }
}

/// Precise main-region position, including a seated avatar's parent chain.
/// Coarse map locations are deliberately excluded: their quantization and
/// stale entries cannot drive a stable approach or establish target presence.
pub fn avatar_position(world: &crate::world::World, id: uuid::Uuid) -> Option<Vec3> {
    if id == world.agent_id || id.is_nil() {
        return None;
    }
    let idx = world.objects.index_of_uuid(&id)?;
    world.objects.get(idx).filter(|o| o.is_avatar())?;
    crate::scene::Scene::object_transform_unshifted(world, idx, Instant::now())
        .map(|(p, _, _)| p)
        .filter(|p| p.is_finite())
}

/// Flight signals are the simulator's locomotion states from
/// llanimationstates.cpp / LLVOAvatar::processAnimationStateChanges. Read
/// the received signals, independently of local AO animation substitution.
pub(crate) fn flight_from_signals(seated: bool, signals: impl Iterator<Item = uuid::Uuid>) -> Option<bool> {
    use crate::ao::State;
    if seated {
        return Some(false);
    }
    let mut grounded = false;
    let mut airborne = false;
    for id in signals {
        match State::from_motion(id) {
            Some(State::Flying | State::FlyingSlow | State::FlyingUp | State::FlyingDown | State::Hovering) => return Some(true),
            Some(
                State::Standing
                | State::Walking
                | State::Running
                | State::Crouching
                | State::CrouchWalk
                | State::Landing
                | State::SoftLanding
                | State::StandingUp
                | State::Sitting
                | State::GroundSit,
            ) => grounded = true,
            Some(State::Jumping | State::PreJumping | State::Falling) => airborne = true,
            _ => {}
        }
    }
    if grounded && !airborne { Some(false) } else { None }
}

pub fn avatar_target(world: &crate::world::World, id: uuid::Uuid, own_height: f32) -> Option<AvatarTarget> {
    let position = avatar_position(world, id)?;
    let o = world.objects.get(world.objects.index_of_uuid(&id)?)?;
    if !o.scale.z.is_finite() || o.scale.z <= 0.0 || !own_height.is_finite() || own_height <= 0.0 {
        return None;
    }
    let feet = position.z - o.scale.z * 0.5;
    let local = position - world.region_offset(o.key.region)?;
    let contact = o
        .foot_plane
        .filter(|p| p.is_finite() && p.z > 0.1 && o.parent_id == 0)
        .map(|p| (p.w - p.x * local.x - p.y * local.y - 0.05) / p.z)
        .filter(|z| z.is_finite() && (feet - z).abs() <= 0.5);
    let terrain = world.ground_height(position).filter(|z| z.is_finite() && (feet - z).abs() <= 0.5);
    let support = match (contact, terrain) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    let flying =
        flight_from_signals(o.parent_id != 0, world.animations.get(&id).into_iter().flatten().map(|a| a.id)).or(support.map(|_| false));
    Some(AvatarTarget {
        position,
        velocity: if o.velocity.is_finite() { o.velocity } else { Vec3::ZERO },
        running: world
            .animations
            .get(&id)
            .into_iter()
            .flatten()
            .any(|a| crate::ao::State::from_motion(a.id) == Some(crate::ao::State::Running))
            || (o.velocity.is_finite() && o.velocity.truncate().length() > 4.0),
        flight: FlightTarget {
            flying,
            landing_height: support.unwrap_or(feet) + own_height * 0.5 + 0.05,
        },
    })
}

/// Calibration measures time to 90% speed, whereas coasting distance is
/// velocity times the exponential time constant. The round trip covers
/// the report's downlink and the release's uplink, once each.
pub fn flight_response(calibration: &crate::own_motion::Calibration, rtt: f32, report_age: f32) -> FlightResponse {
    FlightResponse {
        coast: (calibration.ramp(crate::own_motion::Mode::Fly) / std::f32::consts::LN_10).clamp(0.0, 1.0),
        delay: calibration.delay(rtt.clamp(0.0, 2.0)) + report_age.clamp(0.0, 0.5),
        speed: calibration.speed(crate::own_motion::Mode::Fly).max(1.0),
    }
}

pub fn can_takeoff(world: &crate::world::World) -> bool {
    can_takeoff_with_unknown_policy(world, false)
}

/// Try a requested join without waiting on policy packets. Known restrictions
/// still apply, and an existing flight may cross a no-takeoff parcel.
pub(crate) fn can_fly_for_join(world: &crate::world::World) -> bool {
    if world.agent.flying {
        !world.agent.is_sitting()
            && !world.rlva.has("fly")
            && world.main_region.is_some()
            && world
                .main()
                .and_then(|r| r.info.as_ref())
                .is_none_or(|r| r.region_flags & aurora_net::region_flags::BLOCK_FLY == 0)
    } else {
        world.main_region.is_some() && can_takeoff_with_unknown_policy(world, true)
    }
}

fn can_takeoff_with_unknown_policy(world: &crate::world::World, allow_unknown: bool) -> bool {
    use crate::world::land::{AgentRights, owned_by_agent, powers};
    // LLAgent::canFly: parcel owners and the owning group's flight managers
    // may fly despite ALLOW_FLY, but region and restraint rules still apply.
    let rights = AgentRights {
        id: world.agent_id,
        groups: &world.groups.groups,
    };
    !world.agent.is_sitting()
        && !world.rlva.has("fly")
        && world
            .main()
            .and_then(|r| r.info.as_ref())
            .map_or(allow_unknown, |r| r.region_flags & aurora_net::region_flags::BLOCK_FLY == 0)
        && world.parcel.as_ref().map_or(allow_unknown, |p| {
            p.flags & aurora_net::parcel_flags::ALLOW_FLY != 0
                || (!world.agent_id.is_nil() && owned_by_agent(p, &rights, powers::LAND_ALLOW_FLY))
        })
}

/// LLAgent::propagate / World::auto_land use physical support, not the
/// prediction's lead. The target landing does not turn off our FLY flag
/// in midair, and our arrival radius does not leave us hovering forever.
pub fn land_near_target(world: &mut crate::world::World, pilot: &mut AutoPilot) -> bool {
    if !pilot.landing() || (pilot.goal() - world.agent.server_pos).truncate().length() > pilot.landing_radius() {
        return false;
    }
    world.auto_land(&crate::agent::MoveInput {
        down: true,
        ..Default::default()
    });
    if !world.agent.flying {
        pilot.set_flying(false, world.agent.position);
        return true;
    }
    false
}

/// Where a teleport by double click goes (teleportToClickedLocation then
/// doTeleportViaLocationLookAt): the clicked point raised by
/// `pelvis_to_foot`, in the region it lies in. A point of the current
/// region (`size` metres wide, its south-west corner at `origin`) stays in
/// it; a point beyond goes to the 256 m slot under it, which a variable
/// region's simulator accepts as its own.
pub fn teleport_destination(origin: (u32, u32), size: f32, point: Vec3, pelvis_to_foot: f32) -> ((u32, u32), Vec3) {
    let p = point + Vec3::Z * pelvis_to_foot;
    if (0.0..size).contains(&p.x) && (0.0..size).contains(&p.y) {
        return (origin, p);
    }
    let gx = (origin.0 as f64 + p.x as f64).max(0.0);
    let gy = (origin.1 as f64 + p.y as f64).max(0.0);
    let slot = ((gx / 256.0).floor() as u32 * 256, (gy / 256.0).floor() as u32 * 256);
    (slot, Vec3::new((gx - slot.0 as f64) as f32, (gy - slot.1 as f64) as f32, p.z))
}

/// AURORA_DEMO_DOUBLECLICK=walk|run|tp: the double-click action of the
/// session, without saving it (`menu` keeps the saved one).
pub fn demo_action() -> Option<DoubleClick> {
    static ACTION: std::sync::OnceLock<Option<DoubleClick>> = std::sync::OnceLock::new();
    *ACTION.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_DOUBLECLICK").ok()?;
        match v.split(',').next().unwrap_or("").trim() {
            "walk" => Some(DoubleClick::Walk),
            "run" => Some(DoubleClick::Run),
            "tp" | "teleport" => Some(DoubleClick::Teleport),
            "menu" => None,
            _ => Some(DoubleClick::Nothing),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    #[test]
    fn two_presses_close_in_time_and_place_make_one_double_click() {
        let mut d = DoubleClickDetector::default();
        let t = Instant::now();
        let (time, size) = (Duration::from_millis(500), 4.0);
        assert!(!d.press_within(t, (100.0, 100.0), time, size));
        assert!(d.press_within(t + Duration::from_millis(300), (101.0, 99.0), time, size));
        // a third press starts a new pair
        assert!(!d.press_within(t + Duration::from_millis(400), (101.0, 99.0), time, size));
        // too late
        assert!(!d.press_within(t + Duration::from_millis(1000), (101.0, 99.0), time, size));
        // too far
        assert!(!d.press_within(t + Duration::from_millis(1100), (110.0, 99.0), time, size));
        // a press elsewhere in between
        d.reset();
        assert!(!d.press_within(t + Duration::from_millis(1200), (110.0, 99.0), time, size));
    }

    #[test]
    fn only_a_plain_second_press_off_our_avatar_takes_the_double_click() {
        let plain = Press {
            double: true,
            ..Default::default()
        };
        assert!(takes_ground_double_click(plain));
        assert!(!takes_ground_double_click(Press { double: false, ..plain }));
        assert!(!takes_ground_double_click(Press { alt: true, ..plain }));
        assert!(!takes_ground_double_click(Press { ctrl: true, ..plain }));
        assert!(!takes_ground_double_click(Press { shift: true, ..plain }));
        assert!(!takes_ground_double_click(Press { build: true, ..plain }));
        assert!(!takes_ground_double_click(Press { on_avatar: true, ..plain }));
    }

    #[test]
    fn the_stop_distance_grows_with_the_distance() {
        let t = Instant::now();
        assert_eq!(AutoPilot::start(Vec3::ZERO, Vec3::new(0.1, 0.0, 0.0), false, t).stop_distance, 0.5);
        let far = AutoPilot::start(Vec3::ZERO, Vec3::new(16.0, 0.0, 0.0), false, t);
        assert!((far.stop_distance - 4.0).abs() < 1e-5);
    }

    #[test]
    fn a_far_target_ahead_walks_fast_and_a_near_one_slowly() {
        let t = Instant::now();
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(30.0, 0.0, 0.0), false, t);
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert_eq!(s.flags & (control::FAST_AT | control::AT_POS), control::FAST_AT | control::AT_POS);
        assert_eq!(s.finish, None);
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(3.0, 0.0, 0.0), false, t);
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert_ne!(s.flags & control::AT_POS, 0);
        assert_eq!(s.flags & control::FAST_AT, 0);
    }

    #[test]
    fn the_body_turns_toward_the_target_on_the_right_side() {
        let t = Instant::now();
        // target to the left (+Y while facing +X)
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0), false, t);
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert!(s.yaw > 0.0);
        assert_ne!(s.flags & control::YAW_POS, 0);
        // to the right
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(0.0, -10.0, 0.0), false, t);
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert!(s.yaw < 0.0);
        assert_ne!(s.flags & control::YAW_NEG, 0);
        // sideways and close: no step forward until it faces the target
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0), false, t);
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert_eq!(s.flags & (control::AT_POS | control::AT_NEG), 0);
    }

    #[test]
    fn the_walk_stops_within_its_stop_distance() {
        let t = Instant::now();
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(9.0, 0.0, 0.0), false, t);
        // stop distance 3 m
        let s = ap.step(Vec3::new(6.5, 0.0, 0.0), 0.0, FRAME);
        assert_eq!(s.finish, Some(Finish::Arrived));
        assert_ne!(s.flags & control::STOP, 0);
    }

    #[test]
    fn a_walk_that_gets_no_nearer_gives_up() {
        let t = Instant::now();
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0), false, t);
        let mut finish = None;
        for _ in 0..200 {
            let s = ap.step(Vec3::ZERO, 0.0, FRAME);
            if s.finish.is_some() {
                finish = s.finish;
                break;
            }
        }
        assert_eq!(finish, Some(Finish::NoProgress));
        // walking toward it never gives up early
        let mut ap = AutoPilot::start(Vec3::ZERO, Vec3::new(20.0, 0.0, 0.0), false, t);
        for i in 1..90 {
            let s = ap.step(Vec3::new(i as f32 * 0.05, 0.0, 0.0), 0.0, FRAME);
            assert_eq!(s.finish, None);
        }
    }

    #[test]
    fn avatar_arrival_ignores_ground_height_and_never_turns_or_moves_at_rest() {
        for z in [-20.0, 0.0, 20.0] {
            let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::new(2.0, 0.0, z), None, false, Instant::now());
            ap.observe_flight(
                FlightTarget {
                    flying: Some(false),
                    landing_height: z,
                },
                Vec3::ZERO,
            );
            assert_eq!(
                ap.step(Vec3::ZERO, 2.0, FRAME),
                Step {
                    yaw: 0.0,
                    flags: control::STOP,
                    finish: Some(Finish::Arrived)
                }
            );
            assert!(ap.reached(Vec3::ZERO));
        }
    }

    #[test]
    fn far_targets_on_either_side_wait_for_the_body_to_turn() {
        for y in [-40.0, 40.0] {
            let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::new(-20.0, y, 0.0), None, false, Instant::now());
            let s = ap.step(Vec3::ZERO, 0.0, FRAME);
            assert_eq!(s.flags & (control::AT_POS | control::AT_NEG | control::FAST_AT), 0);
            assert_eq!(s.yaw.signum(), y.signum());
        }
    }

    #[test]
    fn following_rests_indefinitely_and_resumes_only_outside_the_restart_margin() {
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 2.0, Some(uuid::Uuid::from_u128(7)), false, Instant::now());
        for i in 0..1200 {
            ap.target.x = if i % 2 == 0 { 2.0 } else { 2.9 };
            assert_eq!(
                ap.step(Vec3::ZERO, 0.0, FRAME),
                Step {
                    yaw: 0.0,
                    flags: control::STOP,
                    finish: None
                }
            );
        }
        ap.target.x = 3.1;
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert_ne!(s.flags & control::AT_POS, 0);
        assert_eq!(s.finish, None);
        assert_eq!(ap.step(Vec3::X * 1.2, 0.0, FRAME).flags, control::STOP);
        assert_ne!(ap.step(Vec3::X * -0.2, 0.0, FRAME).flags & control::AT_POS, 0);
    }

    #[test]
    fn follow_distance_changes_apply_to_resting_and_stalled_pilots_without_affecting_go_to() {
        let now = Instant::now();
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 6.0, Some(uuid::Uuid::from_u128(7)), false, now);
        ap.set_follow_distance(6.0, Vec3::ZERO);
        assert_eq!(ap.step(Vec3::ZERO, 0.0, FRAME).flags, control::STOP);
        ap.set_follow_distance(1.0, Vec3::ZERO);
        assert_ne!(ap.step(Vec3::ZERO, 0.0, FRAME).flags & control::AT_POS, 0);
        for _ in 0..60 {
            assert_eq!(ap.step(Vec3::ZERO, 0.0, FRAME).finish, None);
        }
        ap.set_follow_distance(3.0, Vec3::ZERO);
        for _ in 0..60 {
            assert_eq!(ap.step(Vec3::ZERO, 0.0, FRAME).finish, None);
        }
        ap.set_follow_distance(7.0, Vec3::ZERO);
        assert_eq!(ap.step(Vec3::ZERO, 0.0, FRAME).flags, control::STOP);
        ap.target.x = 7.9;
        assert_eq!(ap.step(Vec3::ZERO, 0.0, FRAME).flags, control::STOP);
        ap.target.x = 8.1;
        assert_ne!(ap.step(Vec3::ZERO, 0.0, FRAME).flags & control::AT_POS, 0);
        let mut go = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 6.0, None, false, now);
        go.set_follow_distance(10.0, Vec3::ZERO);
        assert_eq!(go.stop_distance, 2.0);
    }

    #[test]
    fn follow_settings_preserve_old_defaults_bound_external_values_and_round_trip() {
        let old: crate::settings::Settings = serde_json::from_str("{}").expect("old settings");
        assert_eq!(old.follow, FollowSettings::default());
        let chosen = FollowSettings {
            distance: 6.4,
            teleport_if_stuck: true,
        };
        let json = serde_json::to_string(&chosen).expect("follow settings");
        assert_eq!(serde_json::from_str::<FollowSettings>(&json).expect("saved settings"), chosen);
        for (value, expected) in [(f32::NAN, 2.0), (f32::INFINITY, 2.0), (-10.0, 0.5), (100.0, 20.0)] {
            assert_eq!(FollowSettings { distance: value, ..chosen }.bounded_distance(), expected);
        }
    }

    #[test]
    fn follow_distance_change_replans_flight_and_keeps_a_landing_beside_the_leader() {
        let position = Vec3::Z * 8.0;
        let mut ap = AutoPilot::avatar(position, Vec3::ZERO, Some(uuid::Uuid::from_u128(7)), true, Instant::now());
        ap.observe_flight(
            FlightTarget {
                flying: Some(false),
                landing_height: 0.0,
            },
            position,
        );
        assert_ne!(ap.landing_offset, Vec3::ZERO);
        ap.flight_destination = Some(Vec3::ZERO);
        ap.horizontal_thrust.remaining = Some(0.0);
        ap.set_follow_distance(6.0, position);
        assert!((ap.landing_offset.length() - 5.8).abs() < 0.001);
        assert_ne!(ap.goal().x, ap.target.x);
        assert!(ap.flight_destination.is_none());
        assert!(ap.horizontal_thrust.remaining.is_none());
    }

    #[test]
    fn follow_recovery_lands_beside_the_target_at_the_correct_height_and_rejects_invalid_data() {
        let mut target = AvatarTarget {
            position: Vec3::new(260.0, 20.0, 50.0),
            flight: FlightTarget {
                flying: Some(false),
                landing_height: 49.5,
            },
            velocity: Vec3::ZERO,
            running: false,
        };
        let settings = FollowSettings {
            distance: 6.0,
            teleport_if_stuck: true,
        };
        let p = follow_recovery_position(Vec3::new(240.0, 20.0, 1.0), &target, settings).expect("precise target");
        assert!(p.abs_diff_eq(Vec3::new(254.1, 20.0, 49.5), 0.001));
        let (slot, local) = teleport_destination((256_000, 256_000), 256.0, p, 0.0);
        assert_eq!(slot, (256_000, 256_000));
        assert_eq!(local.z, 49.5, "the centre is not raised twice");
        target.flight.flying = Some(true);
        assert_eq!(follow_recovery_position(target.position, &target, settings).expect("hover").z, 50.0);
        assert!(follow_recovery_position(Vec3::NAN, &target, settings).is_none());
        target.position.z = f32::INFINITY;
        assert!(follow_recovery_position(Vec3::ZERO, &target, settings).is_none());
    }

    #[test]
    fn follow_recovery_cannot_loop_at_a_blocked_arrival_but_real_travel_rearms_it() {
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 10.0, Some(uuid::Uuid::from_u128(7)), false, Instant::now());
        assert!(ap.can_recover());
        ap.recovered(Vec3::ZERO);
        for _ in 0..120 {
            ap.step(Vec3::ZERO, 0.0, FRAME);
            assert!(!ap.can_recover());
        }
        ap.set_follow_distance(3.0, Vec3::ZERO);
        assert!(!ap.can_recover(), "changing the slider cannot rearm a blocked arrival");
        ap.shift(Vec3::X * -256.0);
        ap.step(Vec3::X * -256.0, 0.0, FRAME);
        assert!(!ap.can_recover(), "a region-origin shift is not actual travel");
        ap.step(Vec3::X * -255.0, 0.0, FRAME);
        assert!(ap.can_recover());
        assert!(!AutoPilot::avatar(Vec3::ZERO, Vec3::X, None, false, Instant::now()).can_recover());
    }

    #[test]
    fn a_moving_leader_does_not_count_as_an_obstacle_but_a_stationary_follower_does() {
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 10.0, Some(uuid::Uuid::from_u128(7)), false, Instant::now());
        for i in 0..600 {
            ap.target.x = 10.0 + i as f32 * 0.1;
            assert_eq!(ap.step(Vec3::X * (i as f32 * 0.05), 0.0, FRAME).finish, None);
        }
        let mut end = None;
        for _ in 0..120 {
            end = ap.step(Vec3::X * 30.0, 0.0, FRAME).finish;
            if end.is_some() {
                break;
            }
        }
        assert_eq!(end, Some(Finish::NoProgress));
    }

    #[test]
    fn progress_resets_the_stall_timer_and_region_shifts_preserve_the_destination() {
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 10.0, None, false, Instant::now());
        for _ in 0..60 {
            assert_eq!(ap.step(Vec3::ZERO, 0.0, FRAME).finish, None);
        }
        assert_eq!(ap.step(Vec3::X, 0.0, FRAME).finish, None);
        for _ in 0..60 {
            assert_eq!(ap.step(Vec3::X, 0.0, FRAME).finish, None);
        }
        ap.shift(Vec3::X * -256.0);
        assert_eq!(ap.target, Vec3::X * -246.0);
        assert!(!ap.reached(Vec3::X * -255.0));
        assert!(ap.reached(Vec3::X * -248.0));
    }

    #[test]
    fn flight_matches_height_and_uses_slow_forward_flags_nearby() {
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::Z * 10.0, None, true, Instant::now());
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert_ne!(s.flags & control::UP_POS, 0);
        assert_eq!(s.yaw, 0.0);
        assert!(!ap.reached(Vec3::ZERO));
        assert_eq!(
            ap.step(Vec3::Z * 8.0, 0.0, FRAME).finish,
            None,
            "keep climbing to the target's height"
        );
        assert_eq!(ap.step(Vec3::Z * 10.0, 0.0, FRAME).finish, Some(Finish::Arrived));
        ap.target = Vec3::X * 5.0;
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert_ne!(s.flags & control::NUDGE_AT_POS, 0);
        assert_eq!(s.flags & (control::AT_POS | control::FAST_AT), 0);
    }

    #[test]
    fn avatar_approach_arrives_without_overshoot_at_different_frame_rates() {
        for fps in [30.0, 60.0, 144.0] {
            let dt = 1.0 / fps;
            let mut position = Vec3::ZERO;
            let mut yaw = 0.0;
            let mut ap = AutoPilot::avatar(position, Vec3::new(-8.0, -6.0, 5.0), None, false, Instant::now());
            ap.observe_flight(
                FlightTarget {
                    flying: Some(false),
                    landing_height: 5.0,
                },
                position,
            );
            let mut arrived = false;
            for _ in 0..(fps as usize * 20) {
                let s = ap.step(position, yaw, dt);
                yaw += s.yaw;
                if let Some(end) = s.finish {
                    assert_eq!(end, Finish::Arrived);
                    arrived = true;
                    break;
                }
                if s.flags & control::AT_POS != 0 {
                    position += Vec3::new(yaw.cos(), yaw.sin(), 0.0) * 3.2 * dt;
                }
            }
            assert!(arrived, "fps={fps}, position={position:?}");
            assert!((ap.target - position).truncate().length() > 1.8);
        }
    }

    #[test]
    fn invalid_motion_data_stops_without_emitting_nan_turns() {
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 10.0, None, false, Instant::now());
        for (p, yaw, dt) in [(Vec3::NAN, 0.0, FRAME), (Vec3::ZERO, f32::NAN, FRAME), (Vec3::ZERO, 0.0, f32::NAN)] {
            assert_eq!(
                ap.step(p, yaw, dt),
                Step {
                    yaw: 0.0,
                    flags: control::STOP,
                    finish: Some(Finish::NoProgress)
                }
            );
        }
    }

    #[test]
    fn flight_observation_includes_stationary_hover_and_does_not_mistake_jumps_for_flight() {
        use crate::ao::State;
        for state in [
            State::Flying,
            State::FlyingSlow,
            State::FlyingUp,
            State::FlyingDown,
            State::Hovering,
        ] {
            assert_eq!(flight_from_signals(false, [state.motion()].into_iter()), Some(true));
            assert_eq!(flight_from_signals(true, [state.motion()].into_iter()), Some(false), "seated wins");
            assert_eq!(
                flight_from_signals(false, [State::Standing.motion(), state.motion()].into_iter()),
                Some(true)
            );
        }
        for state in [
            State::Standing,
            State::Walking,
            State::Landing,
            State::SoftLanding,
            State::GroundSit,
        ] {
            assert_eq!(flight_from_signals(false, [state.motion()].into_iter()), Some(false));
        }
        for state in [State::Jumping, State::PreJumping, State::Falling] {
            assert_eq!(flight_from_signals(false, [state.motion()].into_iter()), None);
        }
        assert_eq!(flight_from_signals(false, [uuid::Uuid::from_u128(123)].into_iter()), None);
        assert_eq!(flight_from_signals(false, std::iter::empty()), None);
    }

    #[test]
    fn one_shot_approach_accepts_late_flight_without_changing_its_destination() {
        let destination = Vec3::new(1.0, 0.0, 6.0);
        let mut pilot = AutoPilot::avatar(Vec3::ZERO, destination, None, false, Instant::now());
        pilot.subject = Some(uuid::Uuid::from_u128(7));
        assert!(!pilot.reached(Vec3::ZERO));
        assert_eq!(pilot.step(Vec3::ZERO, 0.0, FRAME).flags, control::STOP);
        assert_eq!(pilot.step(Vec3::ZERO, 0.0, FRAME).finish, None);
        pilot.observe_avatar(
            AvatarTarget {
                position: Vec3::new(20.0, 0.0, 6.0),
                flight: FlightTarget {
                    flying: Some(true),
                    landing_height: 6.0,
                },
                velocity: Vec3::X * 5.0,
                running: false,
            },
            Vec3::ZERO,
        );
        assert!(pilot.wants_flight());
        assert_eq!(pilot.target, destination);
        assert!(pilot.leader.is_none());
    }

    #[test]
    fn ground_follow_matches_running_but_slows_near_the_leader_and_during_flight() {
        let mut pilot = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 10.0, Some(uuid::Uuid::from_u128(7)), false, Instant::now());
        let target = |running, flying| AvatarTarget {
            position: Vec3::X * 10.0,
            flight: FlightTarget {
                flying: Some(flying),
                landing_height: 0.0,
            },
            velocity: Vec3::X * 5.0,
            running,
        };
        pilot.observe_avatar(target(true, false), Vec3::ZERO);
        assert!(pilot.run);
        pilot.observe_avatar(target(true, false), Vec3::X * 8.0);
        assert!(!pilot.run, "slow the last metres");
        pilot.observe_avatar(target(false, false), Vec3::ZERO);
        assert!(!pilot.run, "walking target restores walking");
        pilot.observe_avatar(target(true, true), Vec3::ZERO);
        assert!(!pilot.run, "flight does not keep a temporary run");
    }

    #[test]
    fn flight_arrival_uses_one_burn_per_axis_and_matches_the_targets_height() {
        use std::collections::VecDeque;
        for fps in [30, 60, 144] {
            for (ping, initial_yaw, destination) in [
                (0.1, 0.0, Vec3::new(35.0, 8.0, 6.0)),
                (0.3, 0.0, Vec3::new(35.0, 8.0, 6.0)),
                (0.6, 0.0, Vec3::new(35.0, 8.0, 6.0)),
                (0.3, std::f32::consts::PI, Vec3::new(35.0, 8.0, 6.0)),
                (0.6, 0.0, Vec3::new(3.0, 0.0, 1.0)),
                (0.3, 0.0, Vec3::new(0.0, 0.0, 6.0)),
                (0.3, 0.0, Vec3::new(12.0, 0.0, -6.0)),
            ] {
                let dt = 1.0 / fps as f32;
                let mut pilot = AutoPilot::avatar(Vec3::ZERO, destination, Some(uuid::Uuid::from_u128(7)), true, Instant::now());
                pilot.observe_flight(
                    FlightTarget {
                        flying: Some(true),
                        landing_height: 6.0,
                    },
                    Vec3::ZERO,
                );
                let (mut position, mut velocity, mut yaw) = (Vec3::ZERO, Vec3::ZERO, initial_yaw);
                let mut commands = VecDeque::new();
                let mut reports = VecDeque::new();
                let mut received = (0.0, Vec3::ZERO, Vec3::ZERO);
                let mut commanded = Vec3::ZERO;
                let mut last_send = -1.0;
                let mut arrived_at = None;
                let mut last_motion = 0.0;
                let mut previous_side = -1.0_f32;
                let mut crossings = 0;
                let mut burns = [0; 2];
                let mut thrusting = [false; 2];
                for frame in 0..fps * 60 {
                    let now = frame as f32 * dt;
                    while reports.front().is_some_and(|(at, _)| *at <= now) {
                        received = reports.pop_front().unwrap().1;
                    }
                    let mut response = flight_response(&crate::own_motion::Calibration::default(), ping, 0.0);
                    // Raw reports, as in the app: their age plus the uplink.
                    response.delay = now - received.0 + ping * 0.5;
                    let step = pilot.step_moving(received.1, received.2, yaw, dt, response);
                    assert!(step.finish.is_none(), "fps={fps}, ping={ping}, frame={frame}, {step:?}");
                    assert_eq!(step.flags & control::AT_NEG, 0);
                    yaw += step.yaw;
                    for (axis, mask) in [control::AT_POS | control::NUDGE_AT_POS, control::UP_POS | control::UP_NEG]
                        .into_iter()
                        .enumerate()
                    {
                        let on = step.flags & mask != 0;
                        if on && !thrusting[axis] {
                            burns[axis] += 1;
                        }
                        thrusting[axis] = on;
                    }
                    // Twenty AgentUpdates/s; NUDGE deliberately has full native
                    // flight speed. The old immediate demo hid this inertia.
                    if now - last_send >= 0.05 - 1e-4 {
                        let horizontal = if step.flags & (control::AT_POS | control::NUDGE_AT_POS) != 0 {
                            Vec3::new(yaw.cos(), yaw.sin(), 0.0) * 16.0
                        } else {
                            Vec3::ZERO
                        };
                        let vertical = if step.flags & control::UP_POS != 0 {
                            4.0
                        } else if step.flags & control::UP_NEG != 0 {
                            -4.0
                        } else {
                            0.0
                        };
                        commands.push_back((now + ping * 0.5, horizontal + Vec3::Z * vertical));
                        last_send = now;
                    }
                    while commands.front().is_some_and(|(at, _)| *at <= now) {
                        commanded = commands.pop_front().unwrap().1;
                    }
                    velocity += (commanded - velocity) * (1.0 - (-dt / 0.5).exp());
                    position += velocity * dt;
                    let side = (position - destination).dot(destination);
                    if side * previous_side < 0.0 {
                        crossings += 1;
                    }
                    previous_side = side;
                    reports.push_back((now + ping * 0.5, (now, position, velocity)));
                    if (position - destination).truncate().length() <= 2.75
                        && (position.z - destination.z).abs() <= 0.5
                        && arrived_at.is_none()
                    {
                        arrived_at = Some(now);
                    }
                    if commanded.length() > 0.01 {
                        last_motion = now;
                    }
                }
                assert!(arrived_at.is_some(), "never arrived: fps={fps}, ping={ping}, position={position:?}");
                assert!(
                    crossings <= 1,
                    "repeated passes through the target: fps={fps}, ping={ping}, crossings={crossings}"
                );
                assert!(
                    (position - destination).truncate().length() <= 2.75 && (position.z - destination.z).abs() <= 0.5,
                    "fps={fps}, ping={ping}, position={position:?}"
                );
                assert!(
                    velocity.length() < 0.01 && last_motion < 50.0,
                    "kept correcting: fps={fps}, ping={ping}, last_motion={last_motion}"
                );
                assert!(
                    burns.into_iter().all(|count| count <= 1),
                    "restarted thrust: fps={fps}, ping={ping}, burns={burns:?}"
                );
            }
        }
    }

    #[test]
    fn flight_coasting_does_not_disable_the_obstacle_timeout() {
        let mut pilot = AutoPilot::avatar(Vec3::ZERO, Vec3::X * 5.0, Some(uuid::Uuid::from_u128(7)), true, Instant::now());
        let mut finish = None;
        for _ in 0..600 {
            let response = flight_response(&crate::own_motion::Calibration::default(), 0.3, 0.0);
            finish = pilot.step_moving(Vec3::ZERO, Vec3::ZERO, 0.0, FRAME, response).finish;
            if finish.is_some() {
                break;
            }
        }
        assert_eq!(finish, Some(Finish::NoProgress));
    }

    #[test]
    fn hovering_requires_matching_height_and_restarts_when_the_leader_moves() {
        let target = Vec3::new(1.5, 0.0, 1.0);
        let mut pilot = AutoPilot::avatar(Vec3::ZERO, target, Some(uuid::Uuid::from_u128(7)), true, Instant::now());
        assert!(!pilot.reached(Vec3::ZERO), "a sphere would accept this position below the leader");
        let response = flight_response(&crate::own_motion::Calibration::default(), 0.3, 0.0);
        let rest = target.with_x(0.0);
        assert_eq!(pilot.step_moving(rest, Vec3::ZERO, 0.0, FRAME, response).flags, control::STOP);
        pilot.target += Vec3::new(0.1, 0.0, 0.05);
        assert_eq!(
            pilot.step_moving(rest, Vec3::ZERO, 0.0, FRAME, response).flags,
            control::STOP,
            "ignore idle report jitter"
        );
        pilot.target += Vec3::new(6.0, 0.0, 3.0);
        let step = pilot.step_moving(rest, Vec3::ZERO, 0.0, FRAME, response);
        assert_ne!(step.flags & (control::AT_POS | control::NUDGE_AT_POS), 0);
        assert_ne!(step.flags & control::UP_POS, 0);
    }

    #[test]
    fn flight_changes_restart_a_resting_follow_and_unknown_signals_keep_its_mode() {
        let mut pilot = AutoPilot::avatar(Vec3::ZERO, Vec3::X, Some(uuid::Uuid::from_u128(7)), false, Instant::now());
        pilot.observe_flight(
            FlightTarget {
                flying: Some(false),
                landing_height: 0.0,
            },
            Vec3::ZERO,
        );
        assert_eq!(pilot.step(Vec3::ZERO, 0.0, FRAME).flags, control::STOP);
        pilot.target.z = 6.0;
        pilot.observe_flight(
            FlightTarget {
                flying: Some(true),
                landing_height: 6.0,
            },
            Vec3::ZERO,
        );
        assert!(pilot.wants_flight());
        pilot.set_flying(true, Vec3::ZERO);
        assert_ne!(pilot.step(Vec3::ZERO, 0.0, FRAME).flags & control::UP_POS, 0);
        pilot.observe_flight(
            FlightTarget {
                flying: None,
                landing_height: 6.0,
            },
            Vec3::ZERO,
        );
        assert!(pilot.wants_flight(), "missing signals cannot command a landing");
        pilot.observe_flight(
            FlightTarget {
                flying: Some(false),
                landing_height: 0.0,
            },
            Vec3::Z,
        );
        assert!(pilot.landing());
        let step = pilot.step(Vec3::Z, 0.0, FRAME);
        assert_eq!(step.finish, None, "proximity cannot finish a landing while still flying");
        assert_eq!(step.flags, control::UP_NEG);
        pilot.set_flying(false, Vec3::ZERO);
        assert_eq!(pilot.step(Vec3::ZERO, 0.0, FRAME).flags, control::STOP);
        pilot.observe_flight(
            FlightTarget {
                flying: Some(true),
                landing_height: 6.0,
            },
            Vec3::ZERO,
        );
        assert!(pilot.wants_flight(), "follow survives landing and a second takeoff");
    }

    #[test]
    fn landing_approaches_before_descending_and_does_not_dive_below_missing_support() {
        let from = Vec3::Z * 10.0;
        let mut pilot = AutoPilot::avatar(from, Vec3::X * 10.0, None, true, Instant::now());
        pilot.observe_flight(
            FlightTarget {
                flying: Some(false),
                landing_height: 1.0,
            },
            from,
        );
        let step = pilot.step(from, 0.0, FRAME);
        assert_eq!(step.flags & control::UP_NEG, 0, "do not land far from the target");
        assert_ne!(step.flags & control::AT_POS, 0);
        let near = Vec3::new(8.5, 0.0, 10.0);
        assert_eq!(pilot.step(near, 0.0, FRAME).flags, control::UP_NEG);
        let at_floor = near.with_z(1.05);
        assert_eq!(pilot.step(at_floor, 0.0, FRAME).flags, control::UP_NEG);
        assert_eq!(pilot.step(at_floor, 0.0, FRAME).finish, None);
        assert_eq!(pilot.step(near.with_z(0.7), 0.0, FRAME).flags, control::STOP);
    }

    #[test]
    fn flight_preserves_lateral_clearance_and_landing_moves_beside_an_overhead_leader() {
        let mut pilot = AutoPilot::avatar(Vec3::ZERO, Vec3::new(1.85, 0.0, 6.0), None, true, Instant::now());
        pilot.observe_flight(
            FlightTarget {
                flying: Some(true),
                landing_height: 6.0,
            },
            Vec3::ZERO,
        );
        let step = pilot.step(Vec3::ZERO, 0.0, FRAME);
        assert_eq!(
            step.flags,
            control::UP_POS,
            "climb without moving into the leader's horizontal space"
        );
        pilot.target = Vec3::ZERO;
        let above = Vec3::Z * 10.0;
        pilot.observe_flight(
            FlightTarget {
                flying: Some(false),
                landing_height: 1.0,
            },
            above,
        );
        assert!(pilot.landing_offset.truncate().length() >= 1.0);
        let step = pilot.step(above, std::f32::consts::PI, FRAME);
        assert_eq!(step.flags & control::UP_NEG, 0, "move aside before descending");
        assert_ne!(step.flags & (control::AT_POS | control::NUDGE_AT_POS), 0);
        assert_eq!(
            step.flags & control::FAST_AT,
            0,
            "vertical separation cannot speed up a short lateral approach"
        );
        let beside = pilot.goal().with_z(10.0);
        assert_eq!(pilot.step(beside, std::f32::consts::PI, FRAME).flags, control::UP_NEG);
    }

    fn landing_fixture() -> (crate::world::World, AutoPilot) {
        use crate::{demo, scene::avatar::AvatarLibrary, world::World};
        use aurora_net::NetEvent;
        let handle = aurora_net::origin_to_handle(256000, 256000);
        let mut world = World::new(std::sync::Arc::new(AvatarLibrary::load()));
        world.agent_id = demo::DEMO_AGENT;
        world.main_region = Some(handle);
        let mut own = demo::action_avatar(false);
        own.position = Vec3::new(10.0, 10.0, 101.0);
        own.scale.z = 1.8;
        own.foot_plane = Some(glam::Vec4::new(0.0, 0.0, 1.0, 100.0));
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![own.clone()],
        });
        world.apply(NetEvent::AgentMovementComplete {
            handle,
            position: own.position,
            look_at: Vec3::X,
        });
        world.agent.flying = true;
        let mut pilot = AutoPilot::avatar(own.position, own.position + Vec3::X, Some(demo::DEMO_LOUP), true, Instant::now());
        pilot.observe_flight(
            FlightTarget {
                flying: Some(false),
                landing_height: 100.95,
            },
            own.position,
        );
        (world, pilot)
    }

    #[test]
    fn landing_waits_for_our_physical_contact_near_the_leader_including_on_a_platform() {
        let (mut world, mut pilot) = landing_fixture();
        let base = world.agent.server_pos;
        // Rendering/prediction can appear to have arrived before physics.
        world.agent.position = base;
        world.agent.server_pos = base + Vec3::Z * 6.0;
        assert!(!land_near_target(&mut world, &mut pilot));
        assert!(world.agent.flying);
        world.agent.server_pos = base + Vec3::X * 10.0;
        assert!(
            !land_near_target(&mut world, &mut pilot),
            "physical contact far away is insufficient"
        );
        world.agent.server_pos = base;
        world.agent.position = base + Vec3::Z * 3.0;
        assert!(land_near_target(&mut world, &mut pilot));
        assert!(!world.agent.flying && !pilot.flying);
        assert!(pilot.leader.is_some());
        assert_eq!(world.agent.server_pos, base);
    }

    #[test]
    fn target_height_accounts_for_different_avatar_sizes_and_preserves_platform_contact() {
        use crate::demo;
        use aurora_net::NetEvent;
        let (mut world, _) = landing_fixture();
        let mut target = demo::action_avatar(false);
        target.local_id = 9001;
        target.full_id = demo::DEMO_LOUP;
        target.position = Vec3::new(11.0, 10.0, 100.5);
        target.scale.z = 1.0;
        target.foot_plane = Some(glam::Vec4::new(0.0, 0.0, 1.0, 100.0));
        world.apply(NetEvent::ObjectUpdates {
            handle: world.main_region.unwrap(),
            objects: vec![target],
        });
        let observed = avatar_target(&world, demo::DEMO_LOUP, 2.0).unwrap();
        assert_eq!(observed.flight.flying, Some(false));
        assert!((observed.flight.landing_height - 101.0).abs() < 0.001);
        world.apply(NetEvent::AvatarAnimations {
            avatar: demo::DEMO_LOUP,
            anims: vec![(crate::own_motion::ANIM_HOVER, 1)],
            sources: Vec::new(),
        });
        assert_eq!(avatar_target(&world, demo::DEMO_LOUP, 2.0).unwrap().flight.flying, Some(true));
    }

    #[test]
    fn automatic_takeoff_obeys_region_parcel_sitting_and_fly_restrictions() {
        use crate::{demo, scene::avatar::AvatarLibrary, world::World};
        use aurora_net::NetEvent;
        let mut world = World::new(std::sync::Arc::new(AvatarLibrary::load()));
        assert!(!can_takeoff(&world), "wait for region flight policy");
        world.agent_id = demo::DEMO_AGENT;
        let info = demo::events()
            .into_iter()
            .find_map(|e| match e {
                NetEvent::RegionHandshake(info) => Some(info),
                _ => None,
            })
            .unwrap();
        world.main_region = Some(info.handle);
        world.apply(NetEvent::RegionHandshake(info.clone()));
        assert!(!can_takeoff(&world), "wait for parcel flight policy");
        world.parcel = Some(std::sync::Arc::new(aurora_net::ParcelInfo {
            flags: aurora_net::parcel_flags::ALLOW_FLY,
            ..Default::default()
        }));
        assert!(can_takeoff(&world));
        world.agent.seated = true;
        assert!(!can_takeoff(&world));
        world.agent.seated = false;
        let mut blocked = (*info).clone();
        blocked.region_flags |= aurora_net::region_flags::BLOCK_FLY;
        world.apply(NetEvent::RegionHandshake(std::sync::Arc::new(blocked)));
        assert!(!can_takeoff(&world));
        world.apply(NetEvent::RegionHandshake(info.clone()));
        world.parcel = Some(std::sync::Arc::new(aurora_net::ParcelInfo::default()));
        assert!(!can_takeoff(&world));
        std::sync::Arc::make_mut(world.parcel.as_mut().unwrap()).owner_id = world.agent_id;
        assert!(can_takeoff(&world), "parcel owner may fly");
        let group = uuid::Uuid::from_u128(746);
        std::sync::Arc::make_mut(world.parcel.as_mut().unwrap()).owner_id = group;
        world.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Flight managers".into(),
            insignia: uuid::Uuid::nil(),
            powers: 0,
            accept_notices: false,
            list_in_profile: false,
            contribution: 0,
        });
        assert!(!can_takeoff(&world), "membership alone does not grant flight");
        world.groups.groups[0].powers = crate::world::land::powers::LAND_ALLOW_FLY;
        assert!(can_takeoff(&world), "owning group's flight power allows flight");
        let mut blocked = (*info).clone();
        blocked.region_flags |= aurora_net::region_flags::BLOCK_FLY;
        world.apply(NetEvent::RegionHandshake(std::sync::Arc::new(blocked)));
        assert!(!can_takeoff(&world), "region restriction takes precedence over group powers");
        world.apply(NetEvent::RegionHandshake(info));
        world.groups.groups.clear();
        std::sync::Arc::make_mut(world.parcel.as_mut().unwrap()).flags = aurora_net::parcel_flags::ALLOW_FLY;
        assert!(can_takeoff(&world));
        world.rlva.configure(true);
        world.rlva_console_input("@fly=n");
        assert!(!can_takeoff(&world));
    }

    #[test]
    fn avatar_target_uses_seat_and_region_transforms_and_rejects_coarse_or_missing_objects() {
        use crate::{demo, scene::avatar::AvatarLibrary, world::World};
        use aurora_net::NetEvent;
        use std::sync::Arc;
        let (main, neighbour) = (
            aurora_net::origin_to_handle(256000, 256000),
            aurora_net::origin_to_handle(256256, 256000),
        );
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = demo::DEMO_AGENT;
        world.main_region = Some(main);
        let id = demo::DEMO_LOUP;
        world.coarse.insert(main, vec![(id, Vec3::splat(12.0))]);
        assert_eq!(avatar_position(&world, id), None);
        let mut seat = demo::action_avatar(false);
        seat.local_id = 970;
        seat.full_id = uuid::Uuid::from_u128(970);
        seat.pcode = 9;
        seat.position = Vec3::new(10.0, 20.0, 30.0);
        seat.rotation = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mut subject = demo::action_avatar(true);
        subject.full_id = id;
        subject.position = Vec3::X;
        world.apply(NetEvent::ObjectUpdates {
            handle: neighbour,
            objects: vec![seat, subject],
        });
        let position = avatar_position(&world, id).expect("seated target");
        assert!(position.abs_diff_eq(Vec3::new(266.0, 21.0, 30.0), 0.001), "{position:?}");
        assert_eq!(avatar_position(&world, world.agent_id), None);
        let idx = world.objects.index_of_uuid(&id).expect("subject");
        world.objects.remove_idx(idx);
        assert_eq!(
            avatar_position(&world, id),
            None,
            "stale coarse position cannot keep a follow alive"
        );
    }

    #[test]
    fn a_teleport_stays_in_the_region_or_goes_to_the_slot_under_the_point() {
        // in the region: the same point, raised to the avatar's centre
        let (slot, p) = teleport_destination((256_000, 256_512), 256.0, Vec3::new(10.0, 20.0, 30.0), 1.0);
        assert_eq!(slot, (256_000, 256_512));
        assert_eq!(p, Vec3::new(10.0, 20.0, 31.0));
        // east of it: the neighbour's slot
        let (slot, p) = teleport_destination((256_000, 256_512), 256.0, Vec3::new(260.0, 20.0, 30.0), 1.0);
        assert_eq!(slot, (256_256, 256_512));
        assert!((p.x - 4.0).abs() < 1e-3 && (p.y - 20.0).abs() < 1e-3);
        // a variable region keeps a point of its own
        let (slot, _) = teleport_destination((256_000, 256_512), 512.0, Vec3::new(300.0, 20.0, 30.0), 1.0);
        assert_eq!(slot, (256_000, 256_512));
    }
}
