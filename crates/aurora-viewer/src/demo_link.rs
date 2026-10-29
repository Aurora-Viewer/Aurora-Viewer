//! AURORA_DEMO_LINK: a simulated link between the offline demo and a
//! stand-in simulator for our avatar, to try Aurora's movement (own_motion.rs)
//! against Firestorm's without a grid.
//!
//! Without it, the demo moves our avatar at once (`AgentState::simulate_locally`
//! on the drawn avatar). With it, the keys reach the stand-in after half the
//! ping; it moves its own copy of the avatar with the same rules, and its
//! reports (ImprovedTerseObjectUpdate of our avatar, its AvatarAnimation)
//! come back after the other half and go through `World::apply_received`,
//! like the network's. The viewer then runs the grid's path: extrapolation,
//! smoothing and, with Aurora's movement, the prediction.
//!
//! `AURORA_DEMO_LINK=<ping ms>[,jitter=<ms>][,aurora|firestorm][,vehicle]`:
//! `jitter` delays each message by up to that much more (order kept);
//! `aurora` / `firestorm` force the movement for the session without
//! touching the saved choice; `vehicle` seats our avatar on a vehicle that
//! drives back and forth, then stands it up (`vehicle_events`); `crash`
//! does the same faster, under a scripted camera, hitting a wall at each end.
//! `native-flight` opts into full-speed flight with inertia for navigation
//! regressions; ordinary ground/vehicle fixtures retain their default model.

use crate::agent::{AgentState, MoveInput};
use crate::own_motion::{ANIM_CROUCHWALK, ANIM_FLY, ANIM_HOVER, ANIM_HOVER_DOWN, ANIM_HOVER_UP, ANIM_RUN, ANIM_STAND, ANIM_WALK};
use aurora_net::NetEvent;
use aurora_net::objects::{ObjectUpdate, TerseUpdate};
use glam::{Quat, Vec3};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Round trip (s).
    pub ping: f32,
    /// Most extra delay of a message (s).
    pub jitter: f32,
    /// Aurora's movement forced on or off for the session.
    pub prediction: Option<bool>,
    pub vehicle: bool,
    /// The vehicle scenario with a scripted camera and two collisions.
    pub crash: bool,
    /// One `demo link trace` line per frame (`trace`).
    pub trace: bool,
    /// `stoplag=<ms>`: how long the stand-in keeps its walk signal once
    /// stopped, as a busy simulator is late to say it (s).
    pub stop_lag: f32,
    /// Full native flight speed and a 0.5 s velocity ramp, including coast.
    pub native_flight: bool,
}

impl Config {
    pub fn from_env() -> Option<Config> {
        Config::parse(&std::env::var("AURORA_DEMO_LINK").ok()?)
    }

    fn parse(value: &str) -> Option<Config> {
        let mut parts = value.split(',').map(str::trim);
        let ping = parts.next()?.parse::<f32>().ok().filter(|p| p.is_finite() && *p >= 0.0)? / 1000.0;
        let mut cfg = Config {
            ping: ping.min(2.0),
            jitter: 0.0,
            prediction: None,
            vehicle: false,
            crash: false,
            trace: false,
            stop_lag: 0.0,
            native_flight: false,
        };
        for part in parts {
            match part {
                "aurora" => cfg.prediction = Some(true),
                "firestorm" => cfg.prediction = Some(false),
                "vehicle" => cfg.vehicle = true,
                "crash" => (cfg.vehicle, cfg.crash) = (true, true),
                "trace" => cfg.trace = true,
                "native-flight" => cfg.native_flight = true,
                _ => {
                    let ms = |name: &str| {
                        part.strip_prefix(name)
                            .and_then(|ms| ms.parse::<f32>().ok())
                            .filter(|ms| ms.is_finite() && *ms >= 0.0)
                    };
                    if let Some(ms) = ms("jitter=") {
                        cfg.jitter = (ms / 1000.0).min(1.0);
                    } else if let Some(ms) = ms("stoplag=") {
                        cfg.stop_lag = (ms / 1000.0).min(5.0);
                    } else {
                        log::warn!("AURORA_DEMO_LINK: unknown part {part:?}");
                    }
                }
            }
        }
        Some(cfg)
    }
}

/// What the viewer sends this frame.
#[derive(Debug, Clone, Copy)]
pub struct Keys {
    pub input: MoveInput,
    pub yaw: f32,
    pub flying: bool,
    pub running: bool,
    pub mouselook: bool,
    pub automatic_land: bool,
}

/// What reaches the viewer: an event and when the "network" read it, or
/// the end of a flight the stand-in decided (a landing).
pub enum Delivery {
    Event(Box<NetEvent>, Instant),
    Landed,
}

/// `AgentState::simulate_locally`'s centre above the land.
const SIMULATED_STAND: f32 = 0.84;

fn plausible_half_height(half_height: f32) -> bool {
    half_height.is_finite() && (0.3..3.0).contains(&half_height)
}

/// Our avatar's local id in the demo region.
const AGENT_LOCAL_ID: u32 = 9000;
const VEHICLE_LOCAL_ID: u32 = 961;

pub struct DemoLink {
    pub cfg: Config,
    sim: AgentState,
    to_sim: VecDeque<(Instant, Keys)>,
    to_viewer: VecDeque<(Instant, Delivery)>,
    keys: Option<Keys>,
    flying: bool,
    last_report: Option<Instant>,
    reported_velocity: Vec3,
    animation: Option<Uuid>,
    /// Since when the stand-in stands while its walk signal is kept
    /// (`stoplag`).
    stopped_at: Option<Instant>,
    sequence: i32,
    last_arrival: Instant,
    seed: u64,
    start: Instant,
    /// The vehicle scenario: seconds since the start already played.
    vehicle_step: u32,
    probe: Probe,
    /// Height of our avatar's centre above the land as the viewer keeps
    /// it (half its size), against `simulate_locally`'s 0.84 m: the
    /// reports are raised by the difference, or each would pull the
    /// avatar down.
    stand_offset: f32,
    /// The viewer's avatar was seated last frame (`follow_viewer`).
    viewer_seated: bool,
    /// Sits and stand ups of `reply`, when they reach the stand-in: from
    /// then on it reports nothing, or stands where it put the avatar.
    seat_changes: VecDeque<(Instant, bool, Vec3)>,
}

/// What the log tells of each press and release of a walking key: how
/// long the drawn avatar took to start and to stop, the number to compare
/// between Aurora's movement and Firestorm's.
#[derive(Default)]
struct Probe {
    keys: bool,
    mark: Option<(Instant, Vec3, bool)>,
    last: Option<(Instant, Vec3)>,
    /// Distance travelled frame by frame since the mark.
    path: f32,
}

impl DemoLink {
    pub fn new(cfg: Config, position: Vec3, yaw: f32, half_height: f32, now: Instant) -> Self {
        let mut sim = AgentState::default();
        sim.position = position;
        sim.server_pos = position;
        sim.yaw = yaw;
        sim.reset_motion();
        log::info!(
            "demo link: ping {:.0} ms, jitter {:.0} ms, movement {}, vehicle {}",
            cfg.ping * 1000.0,
            cfg.jitter * 1000.0,
            cfg.prediction.map_or("as saved", |on| if on { "aurora" } else { "firestorm" }),
            cfg.vehicle
        );
        DemoLink {
            cfg,
            sim,
            to_sim: VecDeque::new(),
            to_viewer: VecDeque::new(),
            keys: None,
            flying: false,
            last_report: None,
            reported_velocity: Vec3::ZERO,
            animation: None,
            stopped_at: None,
            sequence: 0,
            last_arrival: now,
            seed: 0x9E37_79B9_7F4A_7C15,
            start: now,
            vehicle_step: 0,
            probe: Probe::default(),
            viewer_seated: false,
            seat_changes: VecDeque::new(),
            stand_offset: if plausible_half_height(half_height) {
                half_height - SIMULATED_STAND
            } else {
                0.0
            },
        }
    }

    /// Seconds since the link started.
    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// The answer of the simulator to a command the viewer sent at `now`
    /// (a sit, a stand up): the command reaches it after half the ping and
    /// its answer comes back after the other half.
    pub fn reply(&mut self, now: Instant, events: Vec<NetEvent>) {
        let at_simulator = now + self.one_way();
        for ev in &events {
            if let NetEvent::ObjectUpdates { objects, .. } = ev
                && let Some(me) = objects.iter().find(|o| o.full_id == super::DEMO_AGENT)
            {
                self.seat_changes.push_back((at_simulator, me.parent_id != 0, me.position));
            }
        }
        for ev in events {
            self.send(at_simulator, Delivery::Event(Box::new(ev), at_simulator));
        }
    }

    /// The viewer's avatar sat down or stood up (the answer of `reply`
    /// arrived): the stand-in follows, seated without reports, or standing
    /// where the viewer was put.
    pub fn follow_viewer(&mut self, seated: bool, position: Vec3) {
        if seated == self.viewer_seated {
            return;
        }
        self.viewer_seated = seated;
        if seated {
            self.sim.seated = true;
        } else if self.sim.seated {
            let p = position - Vec3::Z * self.stand_offset;
            self.sim.position = p;
            self.sim.server_pos = p;
            self.sim.seated = false;
            self.sim.reset_motion();
            self.animation = None;
        }
    }

    /// Where the viewer drew our avatar last frame, with the keys held.
    pub fn probe(&mut self, now: Instant, keys: &MoveInput, drawn: Vec3) {
        let p = &mut self.probe;
        let held = keys.forward || keys.back || keys.strafe_left || keys.strafe_right;
        if held != p.keys {
            p.keys = held;
            p.mark = Some((now, drawn, held));
            p.path = 0.0;
        }
        let speed = p.last.map_or(0.0, |(t, at)| {
            let dt = now.saturating_duration_since(t).as_secs_f32();
            p.path += drawn.distance(at);
            if dt > 0.0 { drawn.distance(at) / dt } else { 0.0 }
        });
        p.last = Some((now, drawn));
        let Some((at, from, pressed)) = p.mark else {
            return;
        };
        let ms = now.saturating_duration_since(at).as_secs_f32() * 1000.0;
        if pressed && drawn.distance(from) > 0.05 {
            log::info!("demo link: the drawn avatar moved {ms:.0} ms after the press");
            p.mark = None;
        } else if !pressed && speed < 0.1 {
            log::info!(
                "demo link: the drawn avatar stopped {ms:.0} ms after the release, {:.2} m on ({:.2} m travelled)",
                drawn.distance(from),
                p.path
            );
            p.mark = None;
        } else if ms > 3000.0 {
            p.mark = None;
        }
    }

    /// Half the ping, and up to `jitter` more (xorshift: the same run each time).
    fn one_way(&mut self) -> Duration {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        let r = (self.seed >> 11) as f32 / (1u64 << 53) as f32;
        Duration::from_secs_f32(0.5 * self.cfg.ping + r * self.cfg.jitter)
    }

    fn send(&mut self, now: Instant, delivery: Delivery) {
        // a later message never overtakes an earlier one (one circuit)
        let at = (now + self.one_way()).max(self.last_arrival);
        self.last_arrival = at;
        self.to_viewer.push_back((at, delivery));
    }

    /// One frame: the keys leave for the stand-in, which moves its avatar
    /// with the keys that reached it; what reached the viewer is returned.
    pub fn frame(&mut self, now: Instant, keys: Keys, dt: f32) -> Vec<Delivery> {
        let up = now + self.one_way();
        self.to_sim.push_back((up, keys));
        while let Some((at, k)) = self.to_sim.front().copied() {
            if at > now {
                break;
            }
            self.to_sim.pop_front();
            self.keys = Some(k);
        }
        if self.cfg.vehicle {
            self.vehicle(now);
        }
        while self.seat_changes.front().is_some_and(|(at, _, _)| *at <= now) {
            if let Some((_, seated, position)) = self.seat_changes.pop_front() {
                self.sim.seated = seated;
                if !seated {
                    let p = position - Vec3::Z * self.stand_offset;
                    self.sim.position = p;
                    self.sim.server_pos = p;
                    self.sim.reset_motion();
                    self.animation = None;
                }
            }
        }
        if let Some(k) = self.keys
            && !self.sim.seated
        {
            if k.flying != self.flying {
                self.flying = k.flying;
                self.sim.flying = k.flying;
            }
            self.sim.yaw = k.yaw;
            let mut input = k.input;
            input.run = k.running;
            let p = self.sim.server_pos;
            let was_flying = self.sim.flying;
            if self.cfg.native_flight {
                self.sim
                    .simulate_native_flight(&input, k.mouselook, Some(super::floor_at(p.x, p.y)), dt, k.automatic_land);
            } else {
                self.sim
                    .simulate_locally(&input, k.mouselook, Some(super::floor_at(p.x, p.y)), dt, k.automatic_land);
            }
            if was_flying && !self.sim.flying {
                self.flying = false;
                self.send(now, Delivery::Landed);
            }
            self.report(now, &input);
        }
        let mut out = Vec::new();
        while self.to_viewer.front().is_some_and(|(at, _)| *at <= now) {
            if let Some((at, d)) = self.to_viewer.pop_front() {
                out.push(match d {
                    Delivery::Event(ev, _) => Delivery::Event(ev, at),
                    Delivery::Landed => Delivery::Landed,
                });
            }
        }
        out
    }

    /// The stand-in's reports: a terse update when the velocity changes,
    /// a few a second while moving, one a second at rest; the animation
    /// when it changes.
    fn report(&mut self, now: Instant, input: &MoveInput) {
        let velocity = self.sim.velocity;
        let since = self
            .last_report
            .map_or(f32::MAX, |t| now.saturating_duration_since(t).as_secs_f32());
        let moving = velocity.length() > 0.05;
        if velocity.distance(self.reported_velocity) > 0.05 || (moving && since > 0.25) || since > 1.0 {
            self.last_report = Some(now);
            self.reported_velocity = velocity;
            let update = TerseUpdate {
                local_id: AGENT_LOCAL_ID,
                state: 0,
                is_avatar: true,
                foot_plane: None,
                position: self.sim.server_pos + Vec3::Z * self.stand_offset,
                velocity,
                acceleration: Vec3::ZERO,
                rotation: Quat::from_rotation_z(self.sim.yaw),
                angular_velocity: Vec3::ZERO,
                texture_entry: None,
            };
            let ev = NetEvent::TerseUpdates {
                handle: super::HANDLE,
                updates: vec![update],
            };
            self.send(now, Delivery::Event(Box::new(ev), now));
        }
        let horizontal = velocity.truncate().length() > 0.1;
        let animation = if self.sim.flying {
            if horizontal {
                ANIM_FLY
            } else if input.up {
                ANIM_HOVER_UP
            } else if input.down {
                ANIM_HOVER_DOWN
            } else {
                ANIM_HOVER
            }
        } else if horizontal {
            if input.down {
                ANIM_CROUCHWALK
            } else if input.run {
                ANIM_RUN
            } else {
                ANIM_WALK
            }
        } else {
            ANIM_STAND
        };
        // (`stoplag`: the walk signal kept a while once stopped)
        if animation != ANIM_STAND {
            self.stopped_at = None;
        }
        let animation = if animation == ANIM_STAND && self.animation.is_some_and(|a| a != ANIM_STAND && !self.sim.flying) {
            let since = *self.stopped_at.get_or_insert(now);
            if now.saturating_duration_since(since).as_secs_f32() < self.cfg.stop_lag {
                self.animation.unwrap_or(animation)
            } else {
                animation
            }
        } else {
            animation
        };
        if self.animation != Some(animation) {
            self.animation = Some(animation);
            self.signal(now, animation);
        }
    }

    fn signal(&mut self, now: Instant, animation: Uuid) {
        self.sequence += 1;
        let ev = NetEvent::AvatarAnimations {
            avatar: super::DEMO_AGENT,
            anims: vec![(animation, self.sequence)],
            sources: Vec::new(),
        };
        self.send(now, Delivery::Event(Box::new(ev), now));
    }

    /// `vehicle`: at 2 s a car appears beside our avatar, at 3 s the avatar
    /// sits on it, from 4 s to 16 s it drives back and forth at 4 m/s with a
    /// report every quarter of a second (extrapolated and smoothed like any
    /// object, world/motion.rs), at 17 s the avatar stands up beside it.
    /// The keys held meanwhile must not move the avatar. `crash`: see
    /// [`crash_motion`].
    fn vehicle(&mut self, now: Instant) {
        let quarter = (now.saturating_duration_since(self.start).as_secs_f32() * 4.0) as u32;
        while self.vehicle_step < quarter {
            self.vehicle_step += 1;
            let t = self.vehicle_step as f32 / 4.0;
            for ev in vehicle_events(t, VEHICLE_ORIGIN, self.cfg.crash) {
                self.send(now, Delivery::Event(Box::new(ev), now));
            }
            if t == 3.0 {
                self.sim.seated = true;
                self.signal(now, super::SEAT_ANIM);
                self.animation = None;
            }
            if t == 17.0 {
                let (pos, _) = vehicle_path(17.0, VEHICLE_ORIGIN);
                self.sim.position = pos + STAND_OFFSET;
                self.sim.server_pos = pos + STAND_OFFSET;
                self.sim.seated = false;
                self.sim.reset_motion();
            }
        }
    }
}

/// Where the car starts, in front of the demo's arrival point, and where
/// our avatar stands up from it.
const VEHICLE_ORIGIN: Vec3 = Vec3::new(126.0, 118.0, 0.0);
const STAND_OFFSET: Vec3 = Vec3::new(0.0, 1.6, 0.6);

/// The car's place and velocity `t` seconds into the scenario: 12 m east
/// in 3 s, back in 3 s, twice, from 4 s to 16 s.
fn vehicle_path(t: f32, origin: Vec3) -> (Vec3, Vec3) {
    const SPEED: f32 = 4.0;
    let ground = |x: f32, y: f32| super::floor_at(x, y) + 0.4;
    let driven = (t - 4.0).clamp(0.0, 12.0);
    let leg = (driven / 3.0) as u32;
    let into = driven - leg as f32 * 3.0;
    let (x, v) = if leg.is_multiple_of(2) {
        (into * SPEED, SPEED)
    } else {
        ((3.0 - into) * SPEED, -SPEED)
    };
    let moving = t > 4.0 && t < 16.0;
    let at = Vec3::new(origin.x + x, origin.y, 0.0);
    (
        Vec3::new(at.x, at.y, ground(at.x, at.y)),
        if moving { Vec3::X * v } else { Vec3::ZERO },
    )
}

/// The reports of the `crash` scenario at `t`: position, velocity, rotation
/// and spin. 17 m east at 6 m/s, a wall at 7 s, 17 m back, a wall at 11 s,
/// then still. Each impact bounces the car back with a yaw and roll kick
/// that the next two reports undo, as a simulator reports a collision: a
/// few abrupt reports whose spin does not match the next one.
fn crash_motion(t: f32, origin: Vec3) -> (Vec3, Vec3, Quat, Vec3) {
    const SPEED: f32 = 6.0;
    let kick = |yaw: f32, roll: f32| Quat::from_rotation_z(yaw) * Quat::from_rotation_x(roll);
    // (in quarters of a second, the step of the reports)
    let (x, v, rotation, spin) = match (t * 4.0).round() as i32 {
        ..=16 => (0.0, 0.0, Quat::IDENTITY, Vec3::ZERO),
        17..=27 => ((t - 4.0) * SPEED, SPEED, Quat::IDENTITY, Vec3::ZERO),
        28 => (17.6, -2.0, kick(0.3, 0.12), Vec3::new(-0.8, 0.0, -2.5)),
        29 => (17.2, -0.8, kick(-0.12, -0.04), Vec3::new(0.3, 0.0, 0.8)),
        30..=31 => (17.0, 0.0, Quat::IDENTITY, Vec3::ZERO),
        32..=43 => (17.0 - (t - 8.0) * SPEED, -SPEED, Quat::IDENTITY, Vec3::ZERO),
        44 => (-0.6, 2.0, kick(-0.3, -0.12), Vec3::new(0.8, 0.0, 2.5)),
        45 => (-0.2, 0.8, kick(0.12, 0.04), Vec3::new(-0.3, 0.0, -0.8)),
        _ => (0.0, 0.0, Quat::IDENTITY, Vec3::ZERO),
    };
    let (x, y) = (origin.x + x, origin.y);
    (Vec3::new(x, y, super::floor_at(x, y) + 0.4), Vec3::X * v, rotation, spin)
}

fn vehicle_prim(t: f32, origin: Vec3, crash: bool) -> ObjectUpdate {
    use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
    let (position, velocity, rotation, spin) = if crash {
        crash_motion(t, origin)
    } else {
        let (position, velocity) = vehicle_path(t, origin);
        (position, velocity, Quat::IDENTITY, Vec3::ZERO)
    };
    let mut o = super::prim(
        VEHICLE_LOCAL_ID,
        position,
        rotation,
        Vec3::new(2.4, 1.3, 0.6),
        super::shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        super::te([0.85, 0.32, 0.25, 1.0], 0, false, 0.0),
        aurora_prim::ExtraParams::default(),
        "",
    );
    o.velocity = velocity;
    o.angular_velocity = spin;
    o
}

fn seated_agent(parent: u32, position: Vec3) -> ObjectUpdate {
    use aurora_prim::params::{LL_PCODE_LEGACY_AVATAR, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
    let mut o = super::prim(
        AGENT_LOCAL_ID,
        position,
        Quat::IDENTITY,
        Vec3::new(0.45, 0.6, 1.9),
        super::shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        super::te([0.91, 0.93, 0.98, 1.0], 0, false, 0.0),
        aurora_prim::ExtraParams::default(),
        "",
    );
    o.pcode = LL_PCODE_LEGACY_AVATAR;
    o.full_id = super::DEMO_AGENT;
    o.parent_id = parent;
    o.name_values = "FirstName STRING RW SV Aurora\nLastName STRING RW SV Demo\nTitle STRING RW SV Aurora".into();
    o
}

/// The vehicle scenario's messages at `t` seconds (a quarter-second step).
fn vehicle_events(t: f32, origin: Vec3, crash: bool) -> Vec<NetEvent> {
    let update = |objects: Vec<ObjectUpdate>| NetEvent::ObjectUpdates {
        handle: super::HANDLE,
        objects,
    };
    if t == 2.0 || (t > 4.0 && t <= 16.25) {
        if t == 2.0 {
            log::info!("demo link: the vehicle appears");
        }
        return vec![update(vec![vehicle_prim(t, origin, crash)])];
    }
    if crash && t == 3.25 {
        // a racing script's camera: rigid behind the car, no lag
        // (llSetCameraParams; ids of llfollowcamparams.h)
        log::info!("demo link: the vehicle takes the camera");
        return vec![NetEvent::SetFollowCam {
            handle: super::HANDLE,
            object: vehicle_prim(t, origin, crash).full_id,
            properties: vec![
                (12, 1.0),
                (7, 6.0),
                (0, 12.0),
                (4, 1.0),
                (5, 0.0),
                (6, 0.0),
                (8, 0.0),
                (9, 0.0),
                (10, 0.0),
                (11, 0.0),
            ],
        }];
    }
    if t == 3.0 {
        log::info!("demo link: our avatar sits on the vehicle");
        return vec![
            NetEvent::SitResponse {
                object: vehicle_prim(t, origin, crash).full_id,
                camera_eye: Vec3::new(-6.0, 0.0, 2.5),
                camera_at: Vec3::new(0.0, 0.0, 0.8),
                force_mouselook: false,
            },
            update(vec![seated_agent(VEHICLE_LOCAL_ID, Vec3::new(0.0, 0.0, 0.9))]),
        ];
    }
    if t == 17.0 {
        log::info!("demo link: our avatar stands up");
        let (pos, _) = vehicle_path(t, origin);
        return vec![update(vec![seated_agent(0, pos + STAND_OFFSET)])];
    }
    Vec::new()
}

/// Stand-ins for the simulator's walk, run, crouch, fly, hover and stand
/// (the demo has no asset server for Linden's): legs and arms swinging at
/// the pace of each, so a start or a stop shows.
pub fn animations() -> Vec<(Uuid, aurora_assets::Animation)> {
    vec![
        (ANIM_STAND, super::idle_animation()),
        (ANIM_WALK, cycle(1.0, 0.45, 0.0)),
        (ANIM_RUN, cycle(0.65, 0.8, 0.15)),
        (ANIM_CROUCHWALK, cycle(1.3, 0.3, 0.5)),
        (ANIM_FLY, pose(-1.2, 0.0)),
        (ANIM_HOVER, pose(0.0, 0.3)),
        (ANIM_HOVER_UP, pose(0.0, -0.4)),
        (ANIM_HOVER_DOWN, pose(0.0, 0.8)),
    ]
}

fn joint(name: &str, keys: Vec<(f32, Quat)>) -> aurora_assets::anim::JointMotion {
    aurora_assets::anim::JointMotion {
        joint_name: name.into(),
        priority: -1,
        rot_keys: keys
            .into_iter()
            .map(|(time, rotation)| aurora_assets::anim::RotKey { time, rotation })
            .collect(),
        pos_keys: Vec::new(),
    }
}

/// A gait: one stride of `period` seconds, legs swinging by `swing`
/// radians, the body bent forward by `lean`.
fn cycle(period: f32, swing: f32, lean: f32) -> aurora_assets::Animation {
    let half = period / 2.0;
    let leg = |sign: f32| {
        vec![
            (0.0, Quat::from_rotation_y(-swing * sign)),
            (half, Quat::from_rotation_y(swing * sign)),
            (period, Quat::from_rotation_y(-swing * sign)),
        ]
    };
    let knee = |phase: f32| {
        vec![
            (0.0, Quat::from_rotation_y(0.6 * swing * phase.max(0.0))),
            (half, Quat::from_rotation_y(0.6 * swing * (-phase).max(0.0))),
            (period, Quat::from_rotation_y(0.6 * swing * phase.max(0.0))),
        ]
    };
    let down = 1.3f32;
    let arm = |side: f32| {
        vec![
            (0.0, Quat::from_rotation_x(-down * side) * Quat::from_rotation_y(swing * 0.6 * side)),
            (
                half,
                Quat::from_rotation_x(-down * side) * Quat::from_rotation_y(-swing * 0.6 * side),
            ),
            (
                period,
                Quat::from_rotation_x(-down * side) * Quat::from_rotation_y(swing * 0.6 * side),
            ),
        ]
    };
    aurora_assets::Animation {
        base_priority: 3,
        duration: period,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: period,
        looping: true,
        ease_in: 0.2,
        ease_out: 0.2,
        hand_pose: 0,
        joints: vec![
            joint("mHipLeft", leg(1.0)),
            joint("mHipRight", leg(-1.0)),
            joint("mKneeLeft", knee(1.0)),
            joint("mKneeRight", knee(-1.0)),
            joint("mShoulderLeft", arm(1.0)),
            joint("mShoulderRight", arm(-1.0)),
            joint("mTorso", vec![(0.0, Quat::from_rotation_y(lean))]),
        ],
    }
}

/// A held pose: the body tilted by `tilt` (forward for the flight), the
/// legs bent by `legs`.
fn pose(tilt: f32, legs: f32) -> aurora_assets::Animation {
    aurora_assets::Animation {
        base_priority: 3,
        joints: vec![
            joint("mPelvis", vec![(0.0, Quat::from_rotation_y(tilt))]),
            joint("mHipLeft", vec![(0.0, Quat::from_rotation_y(-legs))]),
            joint("mHipRight", vec![(0.0, Quat::from_rotation_y(-legs))]),
            joint("mKneeLeft", vec![(0.0, Quat::from_rotation_y(legs * 1.5))]),
            joint("mKneeRight", vec![(0.0, Quat::from_rotation_y(legs * 1.5))]),
        ],
        ..super::idle_animation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_reads_the_ping_and_its_options() {
        let c = Config::parse("200").expect("ping");
        assert_eq!((c.ping, c.jitter, c.prediction, c.vehicle), (0.2, 0.0, None, false));
        assert_eq!(Config::parse("100,stoplag=800").map(|c| c.stop_lag), Some(0.8));
        let c = Config::parse("150, jitter=30, firestorm, vehicle").expect("all");
        assert_eq!((c.ping, c.jitter, c.prediction, c.vehicle), (0.15, 0.03, Some(false), true));
        assert_eq!(Config::parse("aurora"), None);
        assert_eq!(Config::parse("100,aurora").and_then(|c| c.prediction), Some(true));
        assert!(!c.native_flight);
        assert!(Config::parse("100,native-flight").unwrap().native_flight);
        let c = Config::parse("100,crash").expect("crash");
        assert!(c.vehicle && c.crash);
    }

    #[test]
    fn the_crash_hits_a_wall_at_each_end_then_stops_where_it_started() {
        let o = VEHICLE_ORIGIN;
        let at = |t: f32| crash_motion(t, o);
        assert_eq!(at(5.0).1, Vec3::X * 6.0);
        // the impacts: thrown back, turned and spinning, undone after
        for t in [7.0, 11.0] {
            let (_, velocity, rotation, spin) = at(t);
            assert!(velocity.x * at(t - 0.25).1.x < 0.0, "bounced at {t}");
            assert!(rotation.angle_between(Quat::IDENTITY) > 0.2 && spin.length() > 2.0);
            assert_eq!(at(t + 0.5).2, Quat::IDENTITY);
        }
        assert_eq!(at(9.0).1, Vec3::X * -6.0);
        // standing up beside it as in the plain scenario
        assert!((at(17.0).0.x - vehicle_path(17.0, o).0.x).abs() < 1e-4);
        assert_eq!(at(17.0).1, Vec3::ZERO);
    }

    #[test]
    fn native_flight_fixture_keeps_nudge_speed_and_coasts_without_changing_ground_physics() {
        let mut agent = AgentState::default();
        agent.server_pos = Vec3::Z * 100.0;
        agent.reset_motion();
        agent.flying = true;
        let input = MoveInput {
            forward: true,
            nudge_fb: true,
            ..Default::default()
        };
        for _ in 0..60 {
            agent.simulate_native_flight(&input, false, None, 1.0 / 60.0, true);
        }
        assert!(agent.velocity.x > 13.0, "NUDGE must not hide native flight speed");
        let before = agent.server_pos;
        agent.simulate_native_flight(&MoveInput::default(), false, None, 1.0 / 60.0, true);
        assert!(agent.server_pos.x > before.x && agent.velocity.x > 10.0, "release must coast");
        for _ in 0..300 {
            agent.simulate_native_flight(&MoveInput::default(), false, None, 1.0 / 60.0, true);
        }
        assert!(agent.velocity.length() < 0.01);
        let mut ordinary = AgentState::default();
        let mut native = ordinary.clone();
        ordinary.simulate_locally(&input, false, Some(0.0), 0.1, true);
        native.simulate_native_flight(&input, false, Some(0.0), 0.1, true);
        assert_eq!(native.server_pos, ordinary.server_pos);
        assert_eq!(native.velocity, ordinary.velocity);
    }

    /// The keys reach the stand-in after half the ping and its report
    /// reaches the viewer after the other half.
    #[test]
    fn a_key_comes_back_as_a_report_one_round_trip_later() {
        let cfg = Config::parse("200").expect("ping");
        let start = Instant::now();
        let mut link = DemoLink::new(cfg, Vec3::new(128.0, 128.0, 25.0), 0.0, 0.84, start);
        let mut keys = Keys {
            input: MoveInput::default(),
            yaw: 0.0,
            flying: false,
            running: false,
            mouselook: false,
            automatic_land: true,
        };
        keys.input.forward = true;
        let mut first_moving = None;
        for n in 1..=40 {
            let now = start + Duration::from_millis(n * 10);
            for d in link.frame(now, keys, 0.01) {
                if let Delivery::Event(ev, at) = d
                    && let NetEvent::TerseUpdates { updates, .. } = *ev
                    && updates[0].velocity.length() > 1.0
                    && first_moving.is_none()
                {
                    first_moving = Some(at.duration_since(start).as_secs_f32());
                }
            }
        }
        let t = first_moving.expect("the stand-in walked");
        assert!((0.19..=0.22).contains(&t), "{t}");
    }

    /// A sit answered through the link comes back one round trip after
    /// the request; the stand-in then stays seated and silent, and stands
    /// where the viewer was put.
    #[test]
    fn a_sit_and_a_stand_up_come_back_one_round_trip_later() {
        let cfg = Config::parse("200").expect("ping");
        let start = Instant::now();
        let mut link = DemoLink::new(cfg, Vec3::new(128.0, 128.0, 25.0), 0.0, 0.84, start);
        let keys = Keys {
            input: MoveInput::default(),
            yaw: 0.0,
            flying: false,
            running: false,
            mouselook: false,
            automatic_land: true,
        };
        let seat = super::super::action_id(970);
        link.reply(start, super::super::sit_on_prim(970, seat, Vec3::ONE));
        link.frame(start + Duration::from_millis(105), keys, 0.105);
        assert!(link.sim.seated, "seated when the request reached it");
        let mut sat_at = None;
        for n in 1..=40 {
            let now = start + Duration::from_millis(n * 10);
            for d in link.frame(now, keys, 0.01) {
                if let Delivery::Event(ev, at) = d
                    && matches!(*ev, NetEvent::SitResponse { .. })
                {
                    sat_at = Some(at.duration_since(start).as_secs_f32());
                }
            }
        }
        let t = sat_at.expect("the sit came back");
        assert!((0.19..=0.21).contains(&t), "{t}");
        link.follow_viewer(true, Vec3::ZERO);
        assert!(link.sim.seated);
        let stood = Vec3::new(130.0, 127.0, 26.0);
        link.follow_viewer(false, stood);
        assert!(!link.sim.seated);
        assert_eq!(link.sim.server_pos, stood);
    }

    #[test]
    fn the_vehicle_drives_back_and_forth_then_stops() {
        let o = VEHICLE_ORIGIN;
        assert_eq!(vehicle_path(3.0, o).1, Vec3::ZERO);
        assert_eq!(vehicle_path(5.0, o).1, Vec3::X * 4.0);
        assert_eq!(vehicle_path(8.0, o).1, Vec3::X * -4.0);
        assert!((vehicle_path(7.0, o).0.x - (o.x + 12.0)).abs() < 1e-4);
        assert!((vehicle_path(16.0, o).0.x - o.x).abs() < 1e-4);
        assert_eq!(vehicle_path(16.5, o).1, Vec3::ZERO);
    }
}
