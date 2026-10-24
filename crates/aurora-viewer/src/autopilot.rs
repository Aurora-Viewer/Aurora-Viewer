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
    pub flying: bool,
    waiting: bool,
    previous_position: Vec3,
}

/// The finish callback of a walk: run when it ends within its stop
/// distance and was not cancelled (LLAgent::stopAutoPilot's `success`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrival {
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

impl AutoPilot {
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
            flying: false,
            waiting: false,
            previous_position: from,
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
            flying,
            ..Self::start(from, target, false, now)
        }
    }

    pub fn shift(&mut self, delta: Vec3) {
        self.target += delta;
        self.previous_position += delta;
    }

    fn distance(&self, delta: Vec3) -> f32 {
        if self.flying { delta.length() } else { delta.truncate().length() }
    }

    /// Ended where it was going: within the stop distance of the target,
    /// on the ground plane (Aurora's walk does not fly up to a target
    /// above the avatar, as startAutoPilotGlobal may).
    pub fn reached(&self, position: Vec3) -> bool {
        self.distance(self.target - position) <= self.stop_distance
    }

    /// One frame of LLAgent::autoPilot from the avatar's place and heading,
    /// `dt` seconds after the last (1 / gFPSClamped).
    pub fn step(&mut self, position: Vec3, yaw: f32, dt: f32) -> Step {
        let stop = |finish| Step {
            yaw: 0.0,
            flags: control::STOP,
            finish,
        };
        if !position.is_finite() || !self.target.is_finite() || !yaw.is_finite() || !dt.is_finite() {
            return stop(Some(Finish::NoProgress));
        }
        let dt = dt.clamp(0.0, 0.1);
        let at = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        let delta = self.target - position;
        let target_dist = self.distance(delta);
        if self.reached(position) {
            self.no_progress = 0.0;
            self.previous_position = position;
            self.waiting = self.leader.is_some();
            return stop(self.leader.is_none().then_some(Finish::Arrived));
        }
        if self.waiting {
            self.previous_position = position;
            self.no_progress = 0.0;
            self.target_dist = target_dist;
            if target_dist <= self.stop_distance + 1.0 {
                return stop(None);
            }
            self.waiting = false;
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
        if self.no_progress > MAX_TIME_NO_PROGRESS {
            return stop(Some(Finish::NoProgress));
        }
        let direction = Vec3::new(delta.x, delta.y, 0.0).normalize_or_zero();
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
        if at.dot(direction) > 0.9 && delta.truncate().length() > if self.flying { 0.5 } else { 0.0 } {
            flags |= if self.flying && target_dist <= slow_distance {
                control::NUDGE_AT_POS
            } else {
                control::AT_POS
            };
            if target_dist > slow_distance {
                flags |= control::FAST_AT;
            }
        }
        if self.flying && delta.z.abs() > 0.5 {
            flags |= if delta.z > 0.0 { control::UP_POS } else { control::UP_NEG };
            if delta.z.abs() > 6.0 {
                flags |= control::FAST_UP;
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
    fn flight_uses_height_and_nudges_for_the_final_approach() {
        let mut ap = AutoPilot::avatar(Vec3::ZERO, Vec3::Z * 10.0, None, true, Instant::now());
        let s = ap.step(Vec3::ZERO, 0.0, FRAME);
        assert_ne!(s.flags & control::UP_POS, 0);
        assert_eq!(s.yaw, 0.0);
        assert!(!ap.reached(Vec3::ZERO));
        assert_eq!(ap.step(Vec3::Z * 8.0, 0.0, FRAME).finish, Some(Finish::Arrived));
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
