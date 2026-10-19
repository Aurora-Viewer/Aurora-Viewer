//! Walking our avatar to a point by itself, and the double click on the
//! ground that starts it (Preferences › Controls › "Double-click on the
//! ground", the wolf menu › Me).
//!
//! Port of LLAgent::startAutoPilotGlobal / autoPilot / stopAutoPilot
//! (indra/newview/llagent.cpp), LLToolPie::walkToClickedLocation and
//! teleportToClickedLocation (indra/newview/lltoolpie.cpp), originally LGPL
//! 2.1. Like Firestorm, the viewer only turns the body and sends the walk
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
/// without getting nearer, counted over the whole walk as Firestorm does.
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

fn angle_between(a: Vec3, b: Vec3) -> f32 {
    let (a, b) = (a.normalize_or_zero(), b.normalize_or_zero());
    a.dot(b).clamp(-1.0, 1.0).acos()
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

    /// Ended where it was going: within the stop distance of the target,
    /// on the ground plane (Aurora's walk does not fly up to a target
    /// above the avatar, as startAutoPilotGlobal may).
    pub fn reached(&self, position: Vec3) -> bool {
        (self.target - position).truncate().length() < self.stop_distance
    }

    /// One frame of LLAgent::autoPilot from the avatar's place and heading,
    /// `dt` seconds after the last (1 / gFPSClamped).
    pub fn step(&mut self, position: Vec3, yaw: f32, dt: f32) -> Step {
        let at = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        let mut direction = self.target - position;
        let target_dist = direction.length();
        if target_dist >= self.target_dist {
            self.no_progress += dt;
            if self.no_progress > MAX_TIME_NO_PROGRESS {
                return Step {
                    yaw: 0.0,
                    flags: control::STOP,
                    finish: Some(Finish::NoProgress),
                };
            }
        }
        self.target_dist = target_dist;
        // a two-dimensional solution
        direction.z = 0.0;
        let xy_distance = direction.length();
        let direction = direction.normalize_or_zero();
        let mut flags = 0;
        let mut yaw = if self.target_dist > self.stop_distance {
            angle_between(at, direction)
        } else {
            0.0
        };
        yaw *= 4.0 * dt;
        if at.cross(direction).z > 0.0 {
            flags |= control::YAW_POS;
        } else {
            yaw = -yaw;
            flags |= control::YAW_NEG;
        }
        let slow_distance = (self.stop_distance + 2.0).max(3.0);
        // (Firestorm compares the scaled, signed turn: kept as it is)
        if xy_distance > slow_distance && yaw < std::f32::consts::PI / 10.0 {
            flags |= control::FAST_AT | control::AT_POS;
        } else if self.target_dist > self.stop_distance {
            let along = at.dot(direction);
            if along > 0.9 {
                flags |= control::AT_POS;
            } else if along < -0.9 {
                flags |= control::AT_NEG;
            }
        }
        let finish = (self.target_dist < self.stop_distance).then(|| {
            flags |= control::STOP;
            Finish::Arrived
        });
        Step { yaw, flags, finish }
    }
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
