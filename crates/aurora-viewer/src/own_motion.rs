//! Aurora's movement prediction of our own avatar (Preferences › Aurora ›
//! Movement; Firestorm's behaviour stays the default until it has been
//! tried on the grid).
//!
//! Firestorm moves our avatar only when the simulator's update comes back
//! (LLViewerObject::processUpdateMessage, indra/newview/llviewerobject.cpp;
//! `agent.rs` keeps that path): at 200 ms of ping the avatar starts and stops
//! a fifth of a second after the key. Here the drawn avatar is the
//! simulator's position (extrapolated and smoothed as in Firestorm) plus a
//! *lead*: what our inputs still on their way will add to it.
//!
//! - The lead is the motion the keys asked for over the last delay (the
//!   round trip and the simulator's step): what the simulator has not
//!   played yet, since it plays our inputs that much later. It comes from
//!   the keys alone, so it cannot drift: once they are released it drains
//!   in one delay while the simulator plays the end of the motion. It is
//!   about one round trip of movement (0.7 m walking, 1.2 m running at
//!   200 ms of ping).
//! - When the simulator does not follow (a wall, a freeze, a push) the
//!   lead fades and the server's motion is drawn alone until the keys
//!   change. Three such failures within 10 s suspend the prediction for 5 s.
//! - Speeds, ramps and the delay are learned from the simulator's own
//!   reports (steady speed per mode, time to reach it and to stop, time
//!   from a press to the first report that moves), seeded with values
//!   still to be confirmed on the grid; [`Calibration::learned`] is logged
//!   for that.
//! - The walk, run, crouch and fly animations start and stop with the keys
//!   ([`Locomotion`]) instead of waiting for the simulator's AvatarAnimation
//!   (process_avatar_animation, indra/newview/llviewermessage.cpp); the
//!   simulator's signals take over as they come.
//!
//! Never seated (on an object, a vehicle or the ground), during a teleport
//! or before the arrival in a region: the caller resets it there and the
//! server's motion is all that is drawn.

use crate::world::PlayingAnimation;
use aurora_net::control;
use glam::Vec3;
use std::collections::VecDeque;
use std::time::Instant;
use uuid::Uuid;

/// How the simulator moves an avatar for a set of keys: each has its own
/// speed and ramp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Walk,
    Run,
    /// Walking with the down key held (ANIM_AGENT_CROUCHWALK).
    Crouch,
    /// The first moments of a press (AGENT_CONTROL_NUDGE_*, LLAgent::moveAtNudge).
    Nudge,
    Fly,
}

impl Mode {
    const ALL: [Mode; 5] = [Mode::Walk, Mode::Run, Mode::Crouch, Mode::Nudge, Mode::Fly];

    fn index(self) -> usize {
        self as usize
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Walk => "walk",
            Mode::Run => "run",
            Mode::Crouch => "crouch",
            Mode::Nudge => "nudge",
            Mode::Fly => "fly",
        }
    }
}

/// What the keys sent to the simulator ask of our avatar this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intent {
    pub mode: Mode,
    /// Horizontal unit direction in region coordinates; zero when flying
    /// straight up or down.
    pub direction: Vec3,
    /// +1 up, -1 down, 0: flying only.
    pub vertical: f32,
}

impl Intent {
    /// From the control flags of the AgentUpdate (`AgentState::control_flags`
    /// and the autopilot's), the body's heading and the run state
    /// (SetAlwaysRun). None when nothing moves the avatar: no key, a jump
    /// or a crouch on the spot (the simulator alone knows where a jump
    /// lands).
    pub fn from_flags(flags: u32, yaw: f32, running: bool) -> Option<Intent> {
        let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.0);
        let left = Vec3::new(-yaw.sin(), yaw.cos(), 0.0);
        let mut direction = Vec3::ZERO;
        let (mut full, mut nudge) = (false, false);
        for (flag, towards, is_full) in [
            (control::AT_POS, forward, true),
            (control::AT_NEG, -forward, true),
            (control::LEFT_POS, left, true),
            (control::LEFT_NEG, -left, true),
            (control::NUDGE_AT_POS, forward, false),
            (control::NUDGE_AT_NEG, -forward, false),
            (control::NUDGE_LEFT_POS, left, false),
            (control::NUDGE_LEFT_NEG, -left, false),
        ] {
            if flags & flag != 0 {
                direction += towards;
                full |= is_full;
                nudge |= !is_full;
            }
        }
        let direction = direction.normalize_or_zero();
        let up = flags & control::UP_POS != 0;
        let down = flags & control::UP_NEG != 0;
        if flags & control::FLY != 0 {
            let vertical = f32::from(u8::from(up)) - f32::from(u8::from(down));
            return (direction != Vec3::ZERO || vertical != 0.0).then_some(Intent {
                mode: Mode::Fly,
                direction,
                vertical,
            });
        }
        if direction == Vec3::ZERO {
            return None;
        }
        let mode = if down {
            Mode::Crouch
        } else if nudge && !full {
            Mode::Nudge
        } else if running {
            Mode::Run
        } else {
            Mode::Walk
        };
        Some(Intent {
            mode,
            direction,
            vertical: 0.0,
        })
    }

    /// The same request for the simulator: same mode and vertical key, a
    /// heading within about 25°. A steady turn while walking changes it
    /// only every 25°.
    fn same(a: Option<Intent>, b: Option<Intent>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.mode == b.mode && a.vertical == b.vertical && a.direction.dot(b.direction) > 0.9,
            _ => false,
        }
    }
}

/// Starting values, until the simulator's reports give the real ones.
/// Walk and run: the speeds the offline demo already used for SL (3.2 and
/// 5.13 m/s); fly: 16 m/s, measured in the log of the border-crossing
/// trial on Agni (2026-10-13). Crouch, nudge and climb are guesses. No
/// ramp on the land until one is seen: a lead that stops short of a
/// simulator that slows down is caught up going on (`ease_out`), one that
/// runs past a simulator that stops at once would have to come back. A
/// flight gets going over half a second (Agni, 2026-10-16: from a hover,
/// 90 % of its speed 0.55 s after the first report that moved); with no
/// ramp a flight of a moment while walking (the jump key held) led by
/// 2.9 m, then the drawn avatar stood still while the simulator caught up.
const SEED_SPEED: [f32; 5] = [3.2, 5.13, 1.2, 1.0, 16.0];
const SEED_CLIMB: f32 = 4.0;
const SEED_RAMP: [f32; 5] = [0.0, 0.0, 0.0, 0.0, 0.5];
/// The simulator plays an input this long after the round trip: one of
/// its physics steps (45 a second), to be confirmed on the grid.
const SEED_DELAY: f32 = 1.0 / 45.0;
/// Seconds after the round trip before a report counts as steady motion.
const SETTLE: f32 = 0.35;
/// Weight of a new sample once a value has been learned.
const LEARN_RATE: f32 = 0.3;

/// Speeds and ramps of the simulator, learned from its reports.
#[derive(Debug, Clone)]
pub struct Calibration {
    speed: [f32; 5],
    speed_samples: [u32; 5],
    /// Climbing and descending speed while flying.
    climb: [f32; 2],
    climb_samples: [u32; 2],
    /// Seconds from the first report that moves to the first at 90 % of
    /// the speed.
    ramp: [f32; 5],
    ramp_samples: [u32; 5],
    /// The previous report of a steady motion: two in a row within 3 %
    /// make a sample.
    last: Option<(Mode, f32)>,
    /// A press from rest: its time, and the first report that moved.
    ramp_watch: Option<(Mode, Instant, Option<Instant>)>,
    /// Seconds beyond the round trip from a press to the first report
    /// that moves.
    delay: f32,
    delay_samples: u32,
    /// The same on a release: from the first report slower than the
    /// speed to the first at rest.
    stop_watch: Option<(Mode, Option<Instant>)>,
    changed: bool,
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            speed: SEED_SPEED,
            speed_samples: [0; 5],
            climb: [SEED_CLIMB; 2],
            climb_samples: [0; 2],
            ramp: SEED_RAMP,
            ramp_samples: [0; 5],
            last: None,
            ramp_watch: None,
            delay: SEED_DELAY,
            delay_samples: 0,
            stop_watch: None,
            changed: false,
        }
    }
}

fn learn(value: &mut f32, samples: &mut u32, observed: f32) -> bool {
    let before = *value;
    *value = if *samples == 0 {
        observed
    } else {
        before + LEARN_RATE * (observed - before)
    };
    *samples = samples.saturating_add(1);
    (*value - before).abs() > 0.01 * before.abs().max(0.01)
}

impl Calibration {
    pub fn speed(&self, mode: Mode) -> f32 {
        self.speed[mode.index()]
    }

    pub fn ramp(&self, mode: Mode) -> f32 {
        self.ramp[mode.index()]
    }

    fn climb(&self, vertical: f32) -> f32 {
        self.climb[usize::from(vertical < 0.0)]
    }

    /// The velocity the keys ask for.
    fn target(&self, intent: &Intent) -> Vec3 {
        intent.direction * self.speed(intent.mode) + Vec3::Z * intent.vertical * self.climb(intent.vertical)
    }

    /// The time from an input to the simulator's report of its effect.
    pub fn delay(&self, rtt: f32) -> f32 {
        rtt + self.delay
    }

    /// A press from rest at `pressed`: time how long the simulator takes to
    /// get going.
    fn watch_ramp(&mut self, mode: Mode, pressed: Instant) {
        self.ramp_watch = Some((mode, pressed, None));
    }

    /// A release: time how long the simulator takes to stop. The ramp
    /// serves both ways.
    fn watch_stop(&mut self, mode: Mode) {
        self.stop_watch = Some((mode, None));
    }

    /// A report of the simulator: `held` seconds after the keys last
    /// changed, `rtt` the round trip.
    fn observe(&mut self, intent: Option<Intent>, held: f32, rtt: f32, velocity: Vec3, now: Instant, applied: Instant) {
        let horizontal = velocity.truncate().length();
        let Some(intent) = intent else {
            self.last = None;
            self.ramp_watch = None;
            if let Some((mode, first)) = self.stop_watch {
                let speed = self.speed(mode);
                let i = mode.index();
                match first {
                    // at rest in its first report after the release
                    None if horizontal <= 0.1 * speed => {
                        self.changed |= learn(&mut self.ramp[i], &mut self.ramp_samples[i], 0.0);
                        self.stop_watch = None;
                    }
                    None if horizontal < 0.9 * speed => self.stop_watch = Some((mode, Some(now))),
                    Some(first) if horizontal <= 0.1 * speed => {
                        let ramp = now.saturating_duration_since(first).as_secs_f32();
                        if ramp <= 1.0 {
                            self.changed |= learn(&mut self.ramp[i], &mut self.ramp_samples[i], ramp);
                        }
                        self.stop_watch = None;
                    }
                    _ => {}
                }
            }
            return;
        };
        self.stop_watch = None;
        if let Some((mode, pressed, first)) = self.ramp_watch
            && mode == intent.mode
            && intent.direction != Vec3::ZERO
        {
            let speed = self.speed(mode);
            if first.is_none() && horizontal > 0.1 * speed {
                // the first report that moves: the delay of the simulator
                let extra = applied.saturating_duration_since(pressed).as_secs_f32() - rtt;
                if (-0.05..=0.5).contains(&extra) {
                    self.changed |= learn(&mut self.delay, &mut self.delay_samples, extra.max(0.0));
                }
            }
            match first {
                // already at speed in its first report: quicker than the
                // reports are spaced
                None if horizontal >= 0.9 * speed => {
                    self.changed |= learn(&mut self.ramp[mode.index()], &mut self.ramp_samples[mode.index()], 0.0);
                    self.ramp_watch = None;
                }
                None if horizontal > 0.1 * speed => self.ramp_watch = Some((mode, pressed, Some(now))),
                Some(first) if horizontal >= 0.9 * speed => {
                    let ramp = now.saturating_duration_since(first).as_secs_f32();
                    if ramp <= 1.0 {
                        self.changed |= learn(&mut self.ramp[mode.index()], &mut self.ramp_samples[mode.index()], ramp);
                    }
                    self.ramp_watch = None;
                }
                _ => {}
            }
        }
        if held < rtt + SETTLE {
            self.last = None;
            return;
        }
        let (observed, slot) = if intent.direction != Vec3::ZERO {
            let along = velocity.truncate().dot(intent.direction.truncate());
            if horizontal <= 0.3 || along < 0.95 * horizontal {
                self.last = None;
                return;
            }
            // a speed of the land twice its usual one is not the keys' (a
            // push, a slide, momentum the airborne test missed)
            if intent.mode != Mode::Fly && horizontal > 2.0 * SEED_SPEED[intent.mode.index()] {
                self.last = None;
                return;
            }
            (horizontal, None)
        } else {
            if velocity.z * intent.vertical <= 0.3 {
                self.last = None;
                return;
            }
            (velocity.z.abs(), Some(usize::from(intent.vertical < 0.0)))
        };
        if let Some((mode, previous)) = self.last
            && mode == intent.mode
            && (observed - previous).abs() <= 0.03 * observed
        {
            self.changed |= match slot {
                None => learn(
                    &mut self.speed[intent.mode.index()],
                    &mut self.speed_samples[intent.mode.index()],
                    observed,
                ),
                Some(i) => learn(&mut self.climb[i], &mut self.climb_samples[i], observed),
            };
        }
        self.last = Some((intent.mode, observed));
    }

    /// Something was learned since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// The values in use, for the log: `walk=3.20/4 …` (speed in m/s and
    /// the number of samples it comes from, 0: still the starting value),
    /// then the climb speeds and the ramps (s).
    pub fn learned(&self) -> String {
        let mut out = String::new();
        for mode in Mode::ALL {
            let i = mode.index();
            out.push_str(&format!("{}={:.2}/{} ", mode.name(), self.speed[i], self.speed_samples[i]));
        }
        out.push_str(&format!(
            "up={:.2}/{} down={:.2}/{} ramp",
            self.climb[0], self.climb_samples[0], self.climb[1], self.climb_samples[1]
        ));
        for mode in Mode::ALL {
            let i = mode.index();
            out.push_str(&format!(" {}={:.2}/{}", mode.name(), self.ramp[i], self.ramp_samples[i]));
        }
        out.push_str(&format!(" delay=+{:.3}/{}", self.delay, self.delay_samples));
        out
    }
}

/// Half-life of a lead the keys no longer drive (the simulator did not
/// follow, the prediction is suspended).
const FADE_HALF_LIFE: f32 = 0.1;
/// Seconds the simulator may move otherwise than asked, once it should
/// have followed, before the prediction gives up.
const FOLLOW_GRACE: f32 = 0.15;
/// Seconds beyond the delay and the ramp before the simulator's velocity
/// is compared with the keys'.
const FOLLOW_SETTLE: f32 = 0.15;
const LEAD_CAP: f32 = 3.0;
const FAILURE_WINDOW: f32 = 10.0;
const FAILURES_TO_SUSPEND: usize = 3;
const SUSPEND: f32 = 5.0;
/// The keys' velocities kept: more than the longest delay.
const HISTORY: f32 = 3.0;
/// At the end of a flight the simulator keeps the flight's speed until the
/// avatar lands (Firestorm only clears the FLY flag): the avatar coasts
/// while the simulator moves it this much faster (m/s) than the keys ask.
const COAST_MARGIN: f32 = 1.0;
/// While it coasts, the lead goes from the flight's to the keys' at most
/// this fast (m/s): the drawn avatar slows down by that much, instead of
/// dropping to a walk's lead in one delay and catching up after.
const COAST_DRAIN: f32 = 3.0;
/// Vertical speed (m/s) of the simulator's reports beyond which our
/// avatar is in the air: no speed of the land is learned from them.
const AIRBORNE: f32 = 1.0;

/// One frame of the prediction.
#[derive(Debug, Clone, Copy)]
pub struct Step {
    pub intent: Option<Intent>,
    /// The simulator's velocity, as extrapolated.
    pub server_velocity: Vec3,
    /// Round trip to the simulator (s).
    pub rtt: f32,
    pub flying: bool,
    pub dt: f32,
    pub now: Instant,
}

/// The lead of the drawn avatar over the simulator's.
#[derive(Debug, Clone)]
pub struct Prediction {
    pub calibration: Calibration,
    lead: Vec3,
    /// The velocity the keys give the drawn avatar.
    velocity: Vec3,
    /// That velocity frame by frame (end of the frame, its length): the
    /// lead is what of it the simulator has not played yet.
    history: VecDeque<(Instant, f32, Vec3)>,
    intent: Option<Intent>,
    /// The mode of the last intent: a release slows down at its ramp.
    last_mode: Option<Mode>,
    /// When the intent last changed.
    since: Option<Instant>,
    /// Seconds the simulator has been moving otherwise than asked.
    mismatch: f32,
    /// The simulator did not follow: the server's motion alone until the
    /// keys change.
    given_up: bool,
    failures: Vec<Instant>,
    suspended_until: Option<Instant>,
    flying: bool,
    /// Largest lead since the last `take_lead_max`.
    lead_max: f32,
    rtt: f32,
    /// Since the keys were released, the simulator was seen moving: its
    /// stop is the end of the motion (`settle`).
    moved_since_release: bool,
    /// The flight ended and the simulator carries the avatar on at its
    /// speed (`COAST_MARGIN`): the lead drains at `COAST_DRAIN`.
    coasting: bool,
    /// What the lead had left over the keys' when the coast ended (the
    /// landing): handed to the caller as a correction (`settle`).
    landed: Vec3,
}

impl Default for Prediction {
    fn default() -> Self {
        Prediction {
            calibration: Calibration::default(),
            lead: Vec3::ZERO,
            velocity: Vec3::ZERO,
            history: VecDeque::new(),
            intent: None,
            last_mode: None,
            since: None,
            mismatch: 0.0,
            given_up: false,
            failures: Vec::new(),
            suspended_until: None,
            flying: false,
            lead_max: 0.0,
            rtt: 0.0,
            moved_since_release: false,
            coasting: false,
            landed: Vec3::ZERO,
        }
    }
}

fn horizontal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, 0.0)
}

impl Prediction {
    pub fn lead(&self) -> Vec3 {
        self.lead
    }

    /// Back to the server's motion alone, keeping what was learned: a
    /// seat, a teleport, the setting turned off.
    pub fn reset(&mut self) {
        *self = Prediction {
            calibration: std::mem::take(&mut self.calibration),
            failures: std::mem::take(&mut self.failures),
            suspended_until: self.suspended_until,
            lead_max: self.lead_max,
            ..Prediction::default()
        };
    }

    fn driving(&self, now: Instant) -> bool {
        let suspended = self.suspended_until.is_some_and(|t| now < t);
        !suspended && !self.given_up
    }

    /// What the keys asked for over the last `delay` seconds.
    fn window(&self, now: Instant, delay: f32) -> Vec3 {
        let mut sum = Vec3::ZERO;
        for &(end, dt, velocity) in self.history.iter().rev() {
            // the frame spans [age + dt, age] seconds before now
            let age = now.saturating_duration_since(end).as_secs_f32();
            if age >= delay {
                break;
            }
            sum += velocity * ((age + dt).min(delay) - age);
        }
        sum
    }

    pub fn step(&mut self, s: Step) {
        self.rtt = s.rtt.clamp(0.0, 2.0);
        let was_flying = std::mem::replace(&mut self.flying, s.flying);
        let server = if s.flying {
            s.server_velocity
        } else {
            horizontal(s.server_velocity)
        };
        if !Intent::same(s.intent, self.intent) || self.since.is_none() {
            match (self.intent, s.intent) {
                (None, Some(i)) if server.length() < 0.1 => self.calibration.watch_ramp(i.mode, s.now),
                (Some(i), None) if i.direction != Vec3::ZERO && i.mode != Mode::Nudge => self.calibration.watch_stop(i.mode),
                _ => {}
            }
            self.since = Some(s.now);
            self.given_up = false;
            self.mismatch = 0.0;
            self.moved_since_release = false;
        }
        if s.intent.is_none() && server.length() > 0.1 {
            self.moved_since_release = true;
        }
        // (a heading that drifts within the same request follows the body)
        self.intent = s.intent;
        if let Some(i) = s.intent {
            self.last_mode = Some(i.mode);
        }
        self.failures
            .retain(|t| s.now.saturating_duration_since(*t).as_secs_f32() < FAILURE_WINDOW);
        let ramp = self.last_mode.map_or(0.0, |m| self.calibration.ramp(m));
        let target = self.intent.map_or(Vec3::ZERO, |i| self.calibration.target(&i));
        // the end of a flight at speed: the simulator carries the avatar on
        // until it lands, whatever the keys ask
        let (carried, asked) = (horizontal(server).length(), horizontal(target).length());
        if was_flying && !s.flying && carried > asked + COAST_MARGIN {
            self.coasting = true;
        }
        let landed = self.coasting && (s.flying || carried <= asked + COAST_MARGIN);
        if landed {
            self.coasting = false;
        }
        // the simulator's ramp, as a first order reaching 90 % in `ramp`
        let tau = ramp / std::f32::consts::LN_10;
        let k = if tau > 1e-3 { 1.0 - (-s.dt / tau).exp() } else { 1.0 };
        self.velocity += (target - self.velocity) * k.clamp(0.0, 1.0);
        if !s.flying {
            self.velocity.z = 0.0;
        }
        let driving = self.driving(s.now);
        self.history
            .push_back((s.now, s.dt.max(0.0), if driving { self.velocity } else { Vec3::ZERO }));
        while self
            .history
            .front()
            .is_some_and(|(end, _, _)| s.now.saturating_duration_since(*end).as_secs_f32() > HISTORY)
        {
            self.history.pop_front();
        }
        let delay = self.calibration.delay(self.rtt);
        if driving {
            let keyed = self.window(s.now, delay);
            if self.coasting {
                let gap = keyed - self.lead;
                let most = COAST_DRAIN * s.dt.max(0.0);
                self.lead += if gap.length() > most { gap.normalize_or_zero() * most } else { gap };
            } else {
                if landed {
                    // what is left stays where it is drawn and fades
                    self.landed += horizontal(self.lead - keyed);
                }
                self.lead = keyed;
                self.check_follow(server, delay + ramp, s);
            }
        } else {
            self.lead *= 0.5_f32.powf(s.dt / FADE_HALF_LIFE);
            if self.lead.length_squared() < 1e-6 {
                self.lead = Vec3::ZERO;
            }
        }
        if self.lead.length() > LEAD_CAP {
            self.lead = self.lead.normalize_or_zero() * LEAD_CAP;
        }
        if !s.flying {
            self.lead.z = 0.0;
        }
        self.lead_max = self.lead_max.max(self.lead.length());
    }

    /// Once the simulator should be moving as asked (`settle` seconds after
    /// the keys changed), a velocity that is not (a wall, a freeze, a
    /// script) for `FOLLOW_GRACE` ends the lead.
    fn check_follow(&mut self, server: Vec3, settle: f32, s: Step) {
        let held = self.since.map_or(0.0, |t| s.now.saturating_duration_since(t).as_secs_f32());
        let asked = self.velocity.length();
        if self.intent.is_none() || held < settle + FOLLOW_SETTLE || asked < 0.5 {
            self.mismatch = 0.0;
            return;
        }
        let along = server.dot(self.velocity) / asked;
        if along < 0.5 * asked {
            self.mismatch += s.dt;
        } else {
            self.mismatch = 0.0;
        }
        if self.mismatch > FOLLOW_GRACE {
            self.given_up = true;
            self.mismatch = 0.0;
            self.failures.push(s.now);
            log::info!(
                "own motion: the simulator does not follow ({:.2} m/s of {:.2} asked), lead {:.2} m dropped",
                along.max(0.0),
                asked,
                self.lead.length()
            );
            if self.failures.len() >= FAILURES_TO_SUSPEND {
                self.failures.clear();
                self.suspended_until = Some(s.now + std::time::Duration::from_secs_f32(SUSPEND));
                log::info!("own motion: suspended for {SUSPEND} s after {FAILURES_TO_SUSPEND} failures");
            }
        }
    }

    /// A report of the simulator, for the calibration: read at `received`,
    /// applied by the frame at `applied` (the delay is counted to that frame:
    /// until then the drawn simulator's position moves as before).
    pub fn on_report(&mut self, velocity: Vec3, received: Instant, applied: Instant) {
        // in the air (a fall, a jump, the end of a flight) the simulator's
        // speed is not the keys': a fall with a flight's momentum, forward
        // held, taught a walk of 12 m/s on Agni
        if self.coasting || (!self.flying && velocity.z.abs() > AIRBORNE) {
            return;
        }
        let held = self.since.map_or(0.0, |t| received.saturating_duration_since(t).as_secs_f32());
        self.calibration.observe(self.intent, held, self.rtt, velocity, received, applied);
    }

    /// Keys released and the simulator stopped after moving: what is left
    /// of the lead is a motion it will not make (the release reached it a
    /// physics step sooner than the lead expected). Dropped and returned,
    /// for the caller to keep where the avatar is drawn instead of walking
    /// it back.
    /// The same for what the lead of a coast had left at the landing.
    pub fn settle(&mut self, server_velocity: Vec3) -> Vec3 {
        let landed = std::mem::take(&mut self.landed);
        let server = if self.flying {
            server_velocity
        } else {
            horizontal(server_velocity)
        };
        if self.intent.is_some() || !self.moved_since_release || server.length() > 0.05 || self.lead == Vec3::ZERO {
            return landed;
        }
        self.history.clear();
        self.moved_since_release = false;
        landed + std::mem::take(&mut self.lead)
    }

    /// A key that moves the avatar is held.
    pub fn keys_held(&self) -> bool {
        self.intent.is_some()
    }

    /// Largest lead since the previous call (m).
    pub fn take_lead_max(&mut self) -> f32 {
        std::mem::take(&mut self.lead_max)
    }
}

/// Smoothing time of the drawn avatar's glide to where the prediction puts
/// it: about Firestorm's drawable damping (0.06 s half-life), so a stop
/// eases out over a quarter of a second like Firestorm's.
pub const GLIDE_TIME: f32 = 0.08;

/// One frame of the glide from `current` (moving at `velocity`) to
/// `target`: a critically damped follow (the "SmoothDamp" of Game
/// Programming Gems 4, 1.10), which never overshoots a target that stops
/// and whose acceleration starts from zero. Returns the new position and
/// velocity.
pub fn glide(current: Vec3, velocity: Vec3, target: Vec3, dt: f32) -> (Vec3, Vec3) {
    let omega = 2.0 / GLIDE_TIME;
    let x = omega * dt.max(0.0);
    let decay = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
    let change = current - target;
    let temp = (velocity + omega * change) * dt.max(0.0);
    (target + (change + temp) * decay, (velocity - omega * temp) * decay)
}

/// Deceleration (m/s²) of the last of a stop that `ease_out` stretches.
const EASE_DECEL: f32 = 6.0;

/// Beyond this the aim is not the end of our stop but a motion of the
/// simulator's own (a push, a slide): the glide follows it.
const EASE_RANGE: f32 = 0.5;

/// After a release the drawn avatar only slows down. When the simulator
/// stops a frame later than the lead expected (the release reached it a
/// physics step later), the aim moves on a few centimetres after the glide
/// had nearly stopped: rather than speeding up again for them, the avatar
/// goes on at the speed it had and slows down as it reaches the aim.
/// Horizontally and within `EASE_RANGE` only: the height is the glide's,
/// so a fall the simulator makes alone (the end of a flight, a ledge) or
/// a take-off is followed (an avatar nearly at rest kept its nearly zero
/// speed and hung in the air until the next key). `glided` is the glide's
/// answer from `current` at `velocity`.
pub fn ease_out(current: Vec3, velocity: Vec3, glided: (Vec3, Vec3), aim: Vec3, dt: f32) -> (Vec3, Vec3) {
    let ahead = horizontal(aim - current);
    let velocity = horizontal(velocity);
    let before = velocity.length();
    let distance = ahead.length();
    if horizontal(glided.1).length() <= before || ahead.dot(velocity) <= 0.0 || distance > EASE_RANGE {
        return glided;
    }
    let speed = before.min((2.0 * EASE_DECEL * distance).sqrt());
    let step = speed * dt.max(0.0);
    let (x, y) = if step >= distance {
        (aim.x, aim.y)
    } else {
        let moved = current + ahead / distance * step;
        (moved.x, moved.y)
    };
    let flat = if step >= distance { Vec3::ZERO } else { ahead / distance * speed };
    (Vec3::new(x, y, glided.0.z), Vec3::new(flat.x, flat.y, glided.1.z))
}

/// A correction smaller than this, at rest after a release, waits for the
/// next move to fade (`AgentState::update`): the avatar stopped a few
/// centimetres from where the simulator did, which is not worth walking
/// back on the spot.
pub const REST_OFFSET: f32 = 0.15;

/// Half-life with which a correction of our avatar fades: Firestorm's
/// 0.06 s (OBJECT_DAMPING_TIME_CONSTANT, LLDrawable::updateXform,
/// indra/newview/lldrawable.cpp), and with the prediction a longer one for
/// the gaps of 0.5 to 3 m a lagging simulator opens, so they are not seen
/// almost at full speed. Beyond 10 m the drawable snaps, as before.
pub const SLOW_CORRECTION_HALF_LIFE: f32 = 0.25;

/// A correction of this size fades slowly with the prediction.
pub fn slow_correction(gap: f32) -> bool {
    (0.5..=3.0).contains(&gap)
}

// indra/llcharacter/llanimationstates.cpp (originally LGPL 2.1).
pub const ANIM_WALK: Uuid = uuid::uuid!("6ed24bd8-91aa-4b12-ccc7-c97c857ab4e0");
pub const ANIM_RUN: Uuid = uuid::uuid!("05ddbff8-aaa9-92a1-2b74-8fe77a29b445");
pub const ANIM_CROUCHWALK: Uuid = uuid::uuid!("47f5f6fb-22e5-ae44-f871-73aaaf4a6022");
pub const ANIM_FLY: Uuid = uuid::uuid!("aec4610c-757f-bc4e-c092-c6e9caf18daf");
pub const ANIM_FLYSLOW: Uuid = uuid::uuid!("2b5a38b2-5e00-3a97-a495-4c826bc443e6");
pub const ANIM_HOVER: Uuid = uuid::uuid!("4ae8016b-31b9-03bb-c401-b1ea941db41d");
pub const ANIM_HOVER_UP: Uuid = uuid::uuid!("62c5de58-cb33-5743-3d07-9e4cd4352864");
pub const ANIM_HOVER_DOWN: Uuid = uuid::uuid!("20f063ea-8306-2562-0b07-5c853b37b31e");
pub const ANIM_STAND: Uuid = uuid::uuid!("2408fe9e-df1d-1d7d-f4ff-1384fa7b350f");
/// AGENT_STAND_ANIMS: the stand and its four variants.
const STANDS: [Uuid; 5] = [
    ANIM_STAND,
    uuid::uuid!("15468e00-3400-bb66-cecc-646d7c14458e"),
    uuid::uuid!("370f3a20-6ca6-9971-848c-9a01bc42ae3c"),
    uuid::uuid!("42b46214-4b44-79ae-deb8-0df61424ff4b"),
    uuid::uuid!("f22fed8b-a5ed-2c93-64d5-bdd8b93c889f"),
];
/// The simulator's motions that carry the avatar somewhere.
const MOVING: [Uuid; 7] = [
    ANIM_WALK,
    ANIM_RUN,
    ANIM_CROUCHWALK,
    ANIM_FLY,
    ANIM_FLYSLOW,
    ANIM_HOVER_UP,
    ANIM_HOVER_DOWN,
];
/// Sequence of a motion started locally, before the simulator numbers it.
const LOCAL_SEQUENCE: i32 = i32::MIN;

/// The motion the simulator will play for an intent.
pub fn locomotion_animation(intent: Option<Intent>) -> Option<Uuid> {
    let i = intent?;
    Some(match i.mode {
        Mode::Walk | Mode::Nudge => ANIM_WALK,
        Mode::Run => ANIM_RUN,
        Mode::Crouch => ANIM_CROUCHWALK,
        Mode::Fly if i.direction != Vec3::ZERO => ANIM_FLY,
        Mode::Fly if i.vertical > 0.0 => ANIM_HOVER_UP,
        Mode::Fly => ANIM_HOVER_DOWN,
    })
}

fn rest_animation(id: &Uuid) -> bool {
    STANDS.contains(id) || *id == ANIM_HOVER
}

/// The longest a local start or stop waits for the simulator's own while
/// what is drawn and reported says it is only late (`Locomotion::overlay`).
const HOLD: f32 = 3.0;

/// What says whether the simulator's signal is late or refuses a local
/// start or stop (`Locomotion::overlay`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Moving {
    pub flying: bool,
    /// The simulator's last report moves our avatar.
    pub simulator: bool,
    /// The drawn avatar moves (its glide).
    pub drawn: bool,
}

/// Our walk, run, crouch and fly animations started and stopped with the
/// keys, over the simulator's signals.
#[derive(Debug, Clone, Default)]
pub struct Locomotion {
    wanted: Option<Uuid>,
    /// When `wanted` last changed, which is also the start of its local
    /// motion.
    since: Option<Instant>,
    released: Option<Instant>,
    /// The local motion drawn last frame.
    shown: Option<(Uuid, Instant)>,
    /// The simulator's signal that took over a local motion, and the start
    /// it keeps from it so the cycle does not jump.
    takeover: Option<(Uuid, i32, Instant)>,
}

impl Locomotion {
    pub fn reset(&mut self) {
        *self = Locomotion::default();
    }

    /// The motion the keys asked for last.
    pub fn wanted(&self) -> Option<Uuid> {
        self.wanted
    }

    /// The list to draw instead of the simulator's `base`, or None to draw
    /// it as it is. `window`: how long a local start or stop waits for the
    /// simulator to agree. Past it, a local stop is kept while the
    /// simulator does not report our avatar moving, and a local start
    /// while the drawn avatar moves (`moving`): the simulator's signal is
    /// only late (a busy simulator, a slow link), and giving up then
    /// brought the walk back for a moment after a stop, and the AO's with
    /// it, or stopped it under a moving avatar; `HOLD` at most.
    pub fn overlay(
        &mut self,
        base: &[PlayingAnimation],
        wanted: Option<Uuid>,
        moving: Moving,
        window: f32,
        now: Instant,
    ) -> Option<Vec<PlayingAnimation>> {
        let flying = moving.flying;
        if wanted != self.wanted || self.since.is_none() {
            if self.wanted.is_some() && wanted.is_none() {
                self.released = Some(now);
            }
            if wanted.is_some() {
                self.released = None;
            }
            self.wanted = wanted;
            self.since = Some(now);
        }
        let start = self.since.unwrap_or(now);
        let recent = |t: Instant| now.saturating_duration_since(t).as_secs_f32() < window;
        let within_hold = |t: Instant| now.saturating_duration_since(t).as_secs_f32() < HOLD;
        let stop_held = |t: Instant| recent(t) || (!moving.simulator && within_hold(t));
        let start_held = |t: Instant| recent(t) || (moving.drawn && within_hold(t));
        if let Some(a) = wanted
            && let Some(signal) = base.iter().find(|p| p.id == a)
            && let Some((shown, at)) = self.shown
            && shown == a
        {
            self.takeover = Some((a, signal.sequence, at));
        }
        let mut drawn = base.to_vec();
        let mut changed = false;
        if let Some((id, sequence, at)) = self.takeover {
            match drawn.iter_mut().find(|p| p.id == id && p.sequence == sequence) {
                Some(p) => {
                    p.sequence_start = at;
                    p.continuous_start = at;
                    changed = true;
                }
                None => self.takeover = None,
            }
        }
        self.shown = None;
        match wanted {
            Some(a) if !base.iter().any(|p| p.id == a) && start_held(start) => {
                drawn.retain(|p| !MOVING.contains(&p.id) && !rest_animation(&p.id));
                drawn.push(PlayingAnimation {
                    id: a,
                    sequence: LOCAL_SEQUENCE,
                    sequence_start: start,
                    continuous_start: start,
                });
                self.shown = Some((a, start));
                changed = true;
            }
            None if self.released.is_some_and(stop_held) && base.iter().any(|p| MOVING.contains(&p.id)) => {
                drawn.retain(|p| !MOVING.contains(&p.id));
                if !drawn.iter().any(|p| rest_animation(&p.id)) {
                    let at = self.released.unwrap_or(now);
                    drawn.push(PlayingAnimation {
                        id: if flying { ANIM_HOVER } else { ANIM_STAND },
                        sequence: LOCAL_SEQUENCE,
                        sequence_start: at,
                        continuous_start: at,
                    });
                }
                changed = true;
            }
            _ => {}
        }
        changed.then_some(drawn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(start: Instant, seconds: f32) -> Instant {
        start + Duration::from_secs_f32(seconds)
    }

    fn walk(direction: Vec3) -> Option<Intent> {
        Some(Intent {
            mode: Mode::Walk,
            direction,
            vertical: 0.0,
        })
    }

    #[test]
    fn intents_follow_the_flags_sent() {
        let i = Intent::from_flags(control::AT_POS | control::FAST_AT, 0.0, false).expect("walk");
        assert_eq!(i.mode, Mode::Walk);
        assert!(i.direction.distance(Vec3::X) < 1e-6);
        let i = Intent::from_flags(control::AT_POS | control::LEFT_POS, std::f32::consts::FRAC_PI_2, true).expect("run");
        assert_eq!(i.mode, Mode::Run);
        assert!(i.direction.distance(Vec3::new(-1.0, 1.0, 0.0).normalize()) < 1e-6);
        assert_eq!(
            Intent::from_flags(control::NUDGE_AT_POS, 0.0, false).map(|i| i.mode),
            Some(Mode::Nudge)
        );
        assert_eq!(
            Intent::from_flags(control::AT_POS | control::UP_NEG, 0.0, true).map(|i| i.mode),
            Some(Mode::Crouch)
        );
        // a jump, a crouch on the spot, opposite keys: nothing to predict
        assert_eq!(Intent::from_flags(control::UP_POS, 0.0, false), None);
        assert_eq!(Intent::from_flags(control::UP_NEG, 0.0, false), None);
        assert_eq!(Intent::from_flags(control::AT_POS | control::AT_NEG, 0.0, false), None);
        let up = Intent::from_flags(control::FLY | control::UP_POS, 0.0, false).expect("climb");
        assert_eq!((up.mode, up.direction, up.vertical), (Mode::Fly, Vec3::ZERO, 1.0));
        assert_eq!(Intent::from_flags(control::FLY, 0.0, false), None);
    }

    /// The end of a flight at 16 m/s, forward held (Agni, 2026-10-16): the
    /// simulator carries the avatar on at the flight's speed until it lands
    /// 0.5 s later. The drawn avatar slows down by `COAST_DRAIN` at most
    /// (it dropped to 4.9 m/s, then rushed at 27 when the lead fell to a
    /// walk's at once), the simulator is not taken for one that does not
    /// follow, no walk of 15 m/s is learned, and what the lead has left at
    /// the landing is handed over as a correction.
    #[test]
    fn the_end_of_a_flight_coasts_without_a_dip() {
        let mut p = Prediction::default();
        let now = Instant::now();
        let (rtt, dt) = (0.2, 1.0 / 100.0);
        let fly = Some(Intent {
            mode: Mode::Fly,
            direction: Vec3::X,
            vertical: 0.0,
        });
        let (mut server, mut previous_drawn, mut previous_speed) = (0.0_f32, None::<f32>, None::<f32>);
        let mut worst_change = 0.0_f32;
        let mut handed = Vec3::ZERO;
        for n in 0..400 {
            let t = n as f32 * dt;
            let flying = t < 2.0;
            // the simulator flies, then falls on with the flight's speed and
            // walks from the landing (forward still held)
            let v = if t < 2.5 { 16.0 } else { 3.2 };
            p.step(Step {
                intent: if flying { fly } else { walk(Vec3::X) },
                server_velocity: Vec3::new(v, 0.0, if flying || t >= 2.5 { 0.0 } else { -5.0 }),
                rtt,
                flying,
                dt,
                now: at(now, t),
            });
            if !flying && t < 2.5 {
                p.on_report(Vec3::new(v, 0.0, -5.0), at(now, t), at(now, t));
            }
            handed += p.settle(Vec3::X * v);
            server += v * dt;
            let drawn = server + p.lead().x + handed.x;
            if let Some(previous) = previous_drawn {
                let speed = (drawn - previous) / dt;
                if (1.0..2.49).contains(&t) {
                    if let Some(before) = previous_speed {
                        worst_change = worst_change.max((speed - before).abs());
                    }
                    assert!(speed > 16.0 - COAST_DRAIN - 0.5, "t {t:.2}: drawn at {speed:.1} m/s");
                }
                previous_speed = Some(speed);
            }
            previous_drawn = Some(drawn);
        }
        // (the coast's slowing starts at once: the agent's glide smooths it)
        assert!(
            worst_change <= COAST_DRAIN + 0.1,
            "the drawn speed jumped by {worst_change:.2} m/s in a frame"
        );
        assert!(p.failures.is_empty(), "taken for a simulator that does not follow");
        assert_eq!(p.calibration.speed(Mode::Walk), SEED_SPEED[Mode::Walk.index()]);
        assert!(handed.x > 0.0, "nothing handed over at the landing");
    }

    /// A press, the simulator following one round trip later, then a
    /// release: the drawn avatar starts at once, leads by about speed ×
    /// round trip, and ends where the simulator ends.
    #[test]
    fn the_lead_covers_the_round_trip_and_returns_to_zero() {
        let mut p = Prediction::default();
        p.calibration.ramp = [0.0; 5];
        let now = Instant::now();
        let rtt = 0.2;
        let dt = 1.0 / 100.0;
        let mut drawn = 0.0_f32;
        let mut server = 0.0_f32;
        for n in 0..300 {
            let t = n as f32 * dt;
            let keys = t < 1.5;
            let server_moving = t >= rtt && t < 1.5 + rtt;
            let v = if server_moving { 3.2 } else { 0.0 };
            p.step(Step {
                intent: if keys { walk(Vec3::X) } else { None },
                server_velocity: Vec3::X * v,
                rtt,
                flying: false,
                dt,
                now: at(now, t),
            });
            server += v * dt;
            drawn = server + p.lead().x;
            if n == 10 {
                assert!(drawn > 0.25, "moves from the first frames: {drawn}");
            }
            if (100..150).contains(&n) {
                assert!((p.lead().x - 3.2 * (rtt + SEED_DELAY)).abs() < 0.02, "steady lead {}", p.lead().x);
            }
        }
        assert!(p.lead().length() < 1e-3, "{:?}", p.lead());
        assert!((drawn - 3.2 * 1.5).abs() < 0.05, "{drawn}");
    }

    #[test]
    fn a_simulator_that_does_not_follow_ends_the_lead_and_three_suspend_it() {
        let mut p = Prediction::default();
        let now = Instant::now();
        let mut t = 0.0;
        let run = |p: &mut Prediction, intent: Option<Intent>, seconds: f32, t: &mut f32| {
            let end = *t + seconds;
            while *t < end {
                *t += 0.01;
                p.step(Step {
                    intent,
                    server_velocity: Vec3::ZERO,
                    rtt: 0.2,
                    flying: false,
                    dt: 0.01,
                    now: at(now, *t),
                });
            }
        };
        // into a wall: the lead stays bounded, then goes
        run(&mut p, walk(Vec3::X), 0.5, &mut t);
        assert!(p.lead().x > 0.5 && p.lead().x <= 3.2 * (0.2 + SEED_DELAY) + 1e-3, "{:?}", p.lead());
        run(&mut p, walk(Vec3::X), 1.0, &mut t);
        assert!(p.given_up);
        assert!(p.lead().length() < 0.01);
        // twice more, each after the keys changed: suspended
        for _ in 0..2 {
            run(&mut p, None, 0.1, &mut t);
            run(&mut p, walk(Vec3::X), 1.5, &mut t);
        }
        assert!(p.suspended_until.is_some());
        run(&mut p, None, 0.1, &mut t);
        run(&mut p, walk(Vec3::Y), 0.3, &mut t);
        assert_eq!(p.lead(), Vec3::ZERO, "suspended: the server's motion alone");
    }

    #[test]
    fn a_push_without_keys_never_leads() {
        let mut p = Prediction::default();
        let now = Instant::now();
        for n in 1..100 {
            p.step(Step {
                intent: None,
                server_velocity: Vec3::X * 6.0,
                rtt: 0.2,
                flying: false,
                dt: 0.01,
                now: at(now, n as f32 * 0.01),
            });
        }
        assert_eq!(p.lead(), Vec3::ZERO);
    }

    #[test]
    fn reset_drops_the_lead_and_keeps_what_was_learned() {
        let mut p = Prediction::default();
        p.calibration.speed[0] = 3.0;
        let now = Instant::now();
        p.step(Step {
            intent: walk(Vec3::X),
            server_velocity: Vec3::ZERO,
            rtt: 0.2,
            flying: false,
            dt: 0.1,
            now,
        });
        assert!(p.lead().x > 0.0);
        p.reset();
        assert_eq!(p.lead(), Vec3::ZERO);
        assert_eq!(p.calibration.speed(Mode::Walk), 3.0);
    }

    #[test]
    fn steady_reports_teach_the_speed_and_a_start_the_ramp() {
        let mut c = Calibration::default();
        let now = Instant::now();
        c.watch_ramp(Mode::Run, at(now, 0.0));
        let run = Some(Intent {
            mode: Mode::Run,
            direction: Vec3::X,
            vertical: 0.0,
        });
        c.observe(run, 0.25, 0.2, Vec3::X * 1.0, at(now, 0.25), at(now, 0.25));
        c.observe(run, 0.40, 0.2, Vec3::X * 4.8, at(now, 0.40), at(now, 0.40));
        assert!((c.ramp(Mode::Run) - 0.15).abs() < 1e-3, "{}", c.ramp(Mode::Run));
        // its first report that moves came 50 ms after the round trip
        assert!((c.delay(0.2) - 0.25).abs() < 1e-3, "{}", c.delay(0.2));
        // not settled yet: no speed sample
        c.observe(run, 0.45, 0.2, Vec3::X * 4.9, at(now, 0.45), at(now, 0.45));
        assert_eq!(c.speed(Mode::Run), 5.13);
        c.observe(run, 0.6, 0.2, Vec3::X * 4.9, at(now, 0.6), at(now, 0.6));
        c.observe(run, 0.7, 0.2, Vec3::X * 4.92, at(now, 0.7), at(now, 0.7));
        assert!((c.speed(Mode::Run) - 4.92).abs() < 1e-4);
        assert!(c.take_changed());
        // sideways against a wall, or a jitter of 10 %: no sample
        c.observe(run, 0.8, 0.2, Vec3::Y * 4.9, at(now, 0.8), at(now, 0.8));
        c.observe(run, 0.9, 0.2, Vec3::X * 4.4, at(now, 0.9), at(now, 0.9));
        c.observe(run, 1.0, 0.2, Vec3::X * 4.9, at(now, 1.0), at(now, 1.0));
        assert!((c.speed(Mode::Run) - 4.92).abs() < 1e-4);
        assert!(c.learned().starts_with("walk=3.20/0 run=4.92/1 "), "{}", c.learned());
        // a release: from the first slower report to the first at rest
        c.watch_stop(Mode::Run);
        c.observe(None, 0.0, 0.2, Vec3::X * 4.9, at(now, 2.0), at(now, 2.0));
        c.observe(None, 0.0, 0.2, Vec3::X * 2.0, at(now, 2.2), at(now, 2.2));
        c.observe(None, 0.0, 0.2, Vec3::ZERO, at(now, 2.3), at(now, 2.3));
        let ramp = 0.15 + LEARN_RATE * (0.1 - 0.15);
        assert!((c.ramp(Mode::Run) - ramp).abs() < 1e-4, "{}", c.ramp(Mode::Run));
    }

    #[test]
    fn corrections_of_half_a_metre_to_three_fade_slowly() {
        assert!(!slow_correction(0.3));
        assert!(slow_correction(0.5) && slow_correction(2.0) && slow_correction(3.0));
        assert!(!slow_correction(5.0));
    }

    fn signal(id: Uuid, sequence: i32, now: Instant) -> PlayingAnimation {
        PlayingAnimation::from_signal(id, sequence, now, None)
    }

    fn moving(flying: bool, simulator: bool, drawn: bool) -> Moving {
        Moving { flying, simulator, drawn }
    }

    #[test]
    fn the_walk_starts_with_the_key_and_the_simulator_keeps_its_cycle() {
        let mut l = Locomotion::default();
        let now = Instant::now();
        let stand = [signal(ANIM_STAND, 1, now)];
        assert_eq!(l.overlay(&stand, None, moving(false, false, false), 0.6, now), None);
        let pressed = at(now, 1.0);
        let drawn = l
            .overlay(&stand, Some(ANIM_WALK), moving(false, false, false), 0.6, pressed)
            .expect("local walk");
        assert_eq!(drawn.len(), 1);
        assert_eq!((drawn[0].id, drawn[0].sequence_start), (ANIM_WALK, pressed));
        // the simulator's walk arrives: its cycle keeps the local start
        let server = [signal(ANIM_WALK, 2, at(now, 1.25))];
        let drawn = l
            .overlay(&server, Some(ANIM_WALK), moving(false, false, false), 0.6, at(now, 1.25))
            .expect("kept start");
        assert_eq!((drawn[0].id, drawn[0].sequence, drawn[0].sequence_start), (ANIM_WALK, 2, pressed));
        // and the simulator alone after that, still with that start
        let drawn = l
            .overlay(&server, Some(ANIM_WALK), moving(false, false, false), 0.6, at(now, 3.0))
            .expect("kept start");
        assert_eq!(drawn[0].continuous_start, pressed);
        // the simulator never agrees: the local walk gives up
        let mut l = Locomotion::default();
        assert!(l.overlay(&stand, Some(ANIM_RUN), moving(false, false, false), 0.6, now).is_some());
        assert_eq!(
            l.overlay(&stand, Some(ANIM_RUN), moving(false, false, false), 0.6, at(now, 0.7)),
            None
        );
        // unless the drawn avatar is moving: its walk waits for the
        // simulator's late signal, 3 s at most (no sliding feet)
        let mut l = Locomotion::default();
        assert!(l.overlay(&stand, Some(ANIM_RUN), moving(false, false, true), 0.6, now).is_some());
        assert!(
            l.overlay(&stand, Some(ANIM_RUN), moving(false, false, true), 0.6, at(now, 2.9))
                .is_some()
        );
        assert_eq!(
            l.overlay(&stand, Some(ANIM_RUN), moving(false, false, true), 0.6, at(now, 3.1)),
            None
        );
    }

    #[test]
    fn the_walk_stops_with_the_key_until_the_simulator_stops_it() {
        let mut l = Locomotion::default();
        let now = Instant::now();
        let walking = [signal(ANIM_WALK, 2, now)];
        l.overlay(&walking, Some(ANIM_WALK), moving(false, false, false), 0.6, now);
        let released = at(now, 1.0);
        let drawn = l
            .overlay(&walking, None, moving(false, false, false), 0.6, released)
            .expect("stopped");
        assert_eq!(drawn.iter().map(|p| p.id).collect::<Vec<_>>(), vec![ANIM_STAND]);
        // flying, the rest is the hover
        let mut l = Locomotion::default();
        let flying = [signal(ANIM_FLY, 3, now)];
        l.overlay(&flying, Some(ANIM_FLY), moving(true, false, false), 0.6, now);
        let drawn = l.overlay(&flying, None, moving(true, false, false), 0.6, released).expect("hover");
        assert_eq!(drawn.iter().map(|p| p.id).collect::<Vec<_>>(), vec![ANIM_HOVER]);
        // the simulator's own stop, or its report that the avatar still
        // moves once the wait is over: its list as it is
        let stand = [signal(ANIM_STAND, 4, at(now, 1.2))];
        assert_eq!(l.overlay(&stand, None, moving(true, false, false), 0.6, at(now, 1.2)), None);
        assert_eq!(l.overlay(&flying, None, moving(true, true, false), 0.6, at(now, 1.7)), None);
    }

    #[test]
    fn a_late_stop_of_the_simulator_does_not_bring_the_walk_back() {
        let mut l = Locomotion::default();
        let now = Instant::now();
        let walking = [signal(ANIM_WALK, 2, now)];
        l.overlay(&walking, Some(ANIM_WALK), moving(false, true, false), 0.6, now);
        let released = at(now, 1.0);
        assert!(l.overlay(&walking, None, moving(false, true, false), 0.6, released).is_some());
        // past the wait, its walk signal still there but the avatar still
        // in its reports: the signal is only late, the stop is kept
        let drawn = l
            .overlay(&walking, None, moving(false, false, false), 0.6, at(now, 2.5))
            .expect("still stopped");
        assert_eq!(drawn.iter().map(|p| p.id).collect::<Vec<_>>(), vec![ANIM_STAND]);
        // the avatar moves again in its reports (pushed, a key lost): its walk
        assert_eq!(l.overlay(&walking, None, moving(false, true, false), 0.6, at(now, 2.6)), None);
        // and never longer than the hold, whatever it reports
        let mut l = Locomotion::default();
        l.overlay(&walking, Some(ANIM_WALK), moving(false, true, false), 0.6, now);
        l.overlay(&walking, None, moving(false, false, false), 0.6, released);
        assert!(l.overlay(&walking, None, moving(false, false, false), 0.6, at(now, 3.9)).is_some());
        assert_eq!(l.overlay(&walking, None, moving(false, false, false), 0.6, at(now, 4.1)), None);
    }

    #[test]
    fn other_animations_are_left_alone() {
        let mut l = Locomotion::default();
        let now = Instant::now();
        let typing = Uuid::from_u128(7);
        let base = [signal(ANIM_STAND, 1, now), signal(typing, 1, now)];
        let drawn = l
            .overlay(&base, Some(ANIM_WALK), moving(false, false, false), 0.6, now)
            .expect("walk");
        assert_eq!(drawn.iter().map(|p| p.id).collect::<Vec<_>>(), vec![typing, ANIM_WALK]);
        assert_eq!(
            locomotion_animation(Intent::from_flags(control::FLY | control::UP_NEG, 0.0, false)),
            Some(ANIM_HOVER_DOWN)
        );
    }
}
