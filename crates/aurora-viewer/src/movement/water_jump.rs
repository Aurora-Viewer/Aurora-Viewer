//! Predictive presentation of native water jumps. The observer bridge mode
//! leaves native jump controls/gravity alone; the existing swim motor owns entry.
use super::Frame;
use crate::{agent::MoveInput, ao::State};
use aurora_net::bridge::Motion;
use glam::Vec3;
use std::time::{Duration, Instant};

const DIVE_WINDOW: Duration = Duration::from_millis(450);

#[derive(Clone, Copy, Debug)]
pub struct Landing {
    pub position: Vec3,
    pub seconds: f32,
}

/// Conservative ballistic look-ahead, refined by actual velocity after takeoff.
/// Unknown terrain, shallow water and an intervening landfall reject the action.
pub fn predict(frame: &Frame, horizontal: Vec3, mut ground: impl FnMut(Vec3) -> Option<f32>) -> Option<Landing> {
    let water = frame.water?;
    if !frame.position.is_finite() || !frame.velocity.is_finite() || !horizontal.is_finite() || !water.is_finite() {
        return None;
    }
    let above = frame.position.z - frame.height * 0.35 - water;
    if !(0.05..=12.0).contains(&above) {
        return None;
    }
    let up = if frame.on_ground { 4.5 } else { frame.velocity.z };
    let seconds = (up + (up * up + 19.62 * above).sqrt()) / 9.81;
    if !(0.04..=2.5).contains(&seconds) {
        return None;
    }
    let landing = (frame.position + horizontal.with_z(0.0) * seconds).with_z(water);
    if water - ground(landing)? < frame.height.max(1.0) * 0.7 {
        return None;
    }
    for i in 1..=8 {
        let t = seconds * i as f32 / 8.0;
        let p = frame.position + horizontal.with_z(0.0) * t + Vec3::Z * (up * t - 4.905 * t * t);
        if p.z - frame.height * 0.35 < ground(p)? + 0.08 {
            return None;
        }
    }
    Some(Landing {
        position: landing,
        seconds,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dive,
    Cannonball,
}

#[derive(Clone, Copy, Debug)]
pub struct Impact {
    pub position: Vec3,
    pub speed: f32,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug)]
pub struct Clips {
    pub dive: bool,
    pub cannonball: bool,
    pub dive_outro: f32,
    pub cannonball_recovery: Recovery,
}

impl Default for Clips {
    fn default() -> Self {
        Self {
            dive: false,
            cannonball: false,
            dive_outro: 2.4,
            cannonball_recovery: Recovery {
                duration: 2.8,
                loop_in: 0.6,
                loop_out: 1.2,
                ease_out: 0.25,
                looping: true,
            },
        }
    }
}

/// Native stop timing, matching LLKeyframeMotion::setStopTime and the scene
/// controller. A late tuck stopped before loop-in must finish its intro too.
#[derive(Clone, Copy, Debug)]
pub struct Recovery {
    pub duration: f32,
    pub loop_in: f32,
    pub loop_out: f32,
    pub ease_out: f32,
    pub looping: bool,
}
impl Recovery {
    pub fn from_animation(a: &aurora_assets::Animation) -> Self {
        Self {
            duration: a.duration,
            loop_in: a.loop_in,
            loop_out: a.loop_out,
            ease_out: a.ease_out,
            looping: a.looping,
        }
    }
    fn seconds_after_stop(self, elapsed: f32) -> f32 {
        let delay = if !self.looping || self.loop_out == self.duration {
            0.0
        } else {
            let span = self.loop_out - self.loop_in;
            let fraction = if span > 0.0 { (elapsed - self.loop_in) % span } else { 0.0 };
            (self.duration - self.loop_in - self.ease_out - fraction).max(0.0)
        };
        delay + self.ease_out + 0.03
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Air {
        kind: Kind,
        started: Instant,
        pose_started: Instant,
        water: f32,
        action: u32,
        descended: bool,
        until: Instant,
        flight_released: bool,
    },
    Entry {
        started: Instant,
        outro: f32,
    },
    Recovery {
        until: Instant,
    },
}

#[derive(Default)]
pub struct Runtime {
    phase: Option<Phase>,
    up: bool,
    down: bool,
    jumping: bool,
    recent_jump: Option<Instant>,
    last_position: Option<Vec3>,
    last_velocity: Vec3,
    last_sample: Option<Instant>,
    impact: Option<Impact>,
}

impl Runtime {
    pub fn descent_edge(&self, input: &MoveInput) -> bool {
        input.down && !self.down
    }
    pub fn busy(&self) -> bool {
        self.phase.is_some()
    }
    pub fn cannonball(&self) -> bool {
        matches!(
            self.phase,
            Some(Phase::Air {
                kind: Kind::Cannonball,
                ..
            })
        )
    }
    pub fn pose(&self) -> Option<State> {
        match self.phase {
            Some(Phase::Air { kind: Kind::Dive, .. }) => Some(State::WaterDiveJump),
            Some(Phase::Air {
                kind: Kind::Cannonball, ..
            }) => Some(State::WaterCannonball),
            Some(Phase::Entry { .. }) => Some(State::WaterDiveEntry),
            _ => None,
        }
    }

    pub fn outro(&self) -> bool {
        matches!(self.phase, Some(Phase::Recovery { .. }))
    }

    pub fn take_impact(&mut self) -> Option<Impact> {
        self.impact.take()
    }

    pub fn observer(&self, height: f32) -> Option<Motion> {
        if let Some(Phase::Air { kind, action, .. }) = self.phase {
            Motion::water_jump(kind == Kind::Cannonball, height, action)
        } else {
            None
        }
    }

    pub fn update(&mut self, enabled: bool, clips: Clips, frame: &Frame, input: &MoveInput, landing: Option<Landing>, now: Instant) {
        let up_edge = input.up && !self.up;
        let down_edge = input.down && !self.down;
        let jump_edge = frame.jumping && !self.jumping;
        (self.up, self.down, self.jumping) = (input.up, input.down, frame.jumping);
        let last_velocity = std::mem::replace(&mut self.last_velocity, frame.velocity);
        let dt = self
            .last_sample
            .replace(now)
            .map_or(0.0, |last| now.saturating_duration_since(last).as_secs_f32().min(1.0));
        let allowance = 8.0 + frame.velocity.length().max(last_velocity.length()).min(200.0) * dt;
        let jumped = self
            .last_position
            .replace(frame.position)
            .is_some_and(|p| p.distance(frame.position) > allowance);
        if !enabled
            || !frame.bridge_ready
            || frame.seated
            || frame.suspended
            || frame.locked
            || jumped
            || !frame.position.is_finite()
            || !frame.velocity.is_finite()
            || !frame.height.is_finite()
        {
            self.phase = None;
            self.impact = None;
            self.recent_jump = None;
            return;
        }
        if self
            .recent_jump
            .is_some_and(|started| now.saturating_duration_since(started) > DIVE_WINDOW)
        {
            self.recent_jump = None;
        }
        if frame.flying {
            self.recent_jump = None;
        } else if jump_edge || (up_edge && frame.on_ground) {
            self.recent_jump = Some(now);
        }
        if frame.flying
            && self.phase.is_some_and(|p| {
                !matches!(
                    p,
                    Phase::Air {
                        kind: Kind::Cannonball,
                        ..
                    }
                )
            })
        {
            self.phase = None;
        }
        let Some(water) = frame.water.filter(|w| w.is_finite()) else {
            self.phase = None;
            self.impact = None;
            return;
        };
        if self.phase.is_some_and(|phase| match phase {
            Phase::Air { kind: Kind::Dive, .. } | Phase::Entry { .. } => !clips.dive,
            Phase::Air {
                kind: Kind::Cannonball, ..
            } => !clips.cannonball,
            Phase::Recovery { .. } => false,
        }) {
            self.phase = None;
            self.impact = None;
            return;
        }
        let contact = frame
            .water_contact
            .filter(|p| p.is_finite())
            .unwrap_or(frame.position - Vec3::Z * (frame.height * 0.35));
        let above = contact.z > water + 0.03;
        let water_below = frame.ground.is_none_or(|ground| ground.is_finite() && water - ground > 0.06);
        let was_armed = self.phase.is_some();
        if !was_armed && above && down_edge && (!frame.on_ground || frame.jumping || frame.flying) {
            let kind = if !frame.flying && self.recent_jump.is_some() && landing.is_some() && clips.dive {
                Some(Kind::Dive)
            } else if water_below && clips.cannonball {
                Some(Kind::Cannonball)
            } else {
                None
            };
            if let Some(kind) = kind {
                let action = ((uuid::Uuid::new_v4().as_u128() as u32) & i32::MAX as u32).max(1);
                self.phase = Some(Phase::Air {
                    kind,
                    started: now,
                    pose_started: now,
                    water,
                    action,
                    descended: false,
                    until: air_deadline(kind, frame, now),
                    flight_released: !frame.flying,
                });
            }
        }
        match self.phase {
            Some(Phase::Air {
                mut kind,
                started,
                mut pose_started,
                water: origin_water,
                action,
                mut descended,
                mut until,
                mut flight_released,
            }) => {
                if now >= until || (water - origin_water).abs() > 0.5 || (frame.flying && (kind == Kind::Dive || flight_released)) {
                    self.phase = None;
                    return;
                }
                if was_armed && down_edge && above && !frame.on_ground && clips.cannonball && kind != Kind::Cannonball {
                    kind = Kind::Cannonball;
                    pose_started = now;
                    until = air_deadline(kind, frame, now);
                }
                flight_released |= !frame.flying;
                descended |= frame.velocity.z < -0.4;
                if !above && descended && frame.ground.is_some_and(|g| water - g > frame.height * 0.55) && !frame.on_ground {
                    let speed = (-frame.velocity.z).max(-last_velocity.z).clamp(0.0, 30.0);
                    self.impact = Some(Impact {
                        position: contact.with_z(water),
                        speed,
                        kind,
                    });
                    self.phase = Some(match kind {
                        // Let the simulator broadcast the P5 held pose before
                        // its normal stop releases the authored recovery tail.
                        Kind::Dive => Phase::Entry {
                            started: now,
                            outro: clips.dive_outro,
                        },
                        Kind::Cannonball => Phase::Recovery {
                            until: deadline(
                                now,
                                clips
                                    .cannonball_recovery
                                    .seconds_after_stop(now.saturating_duration_since(pose_started).as_secs_f32()),
                            ),
                        },
                    });
                } else if frame.on_ground
                    && above
                    && (descended || (!frame.jumping && now.saturating_duration_since(started) > Duration::from_millis(850)))
                {
                    self.phase = None;
                } else {
                    self.phase = Some(Phase::Air {
                        kind,
                        started,
                        pose_started,
                        water: origin_water,
                        action,
                        descended,
                        until,
                        flight_released,
                    });
                }
            }
            Some(Phase::Entry { started, outro }) if now.saturating_duration_since(started) >= Duration::from_millis(180) => {
                self.phase = Some(Phase::Recovery {
                    until: deadline(now, outro),
                });
            }
            Some(Phase::Recovery { until }) if now >= until => self.phase = None,
            Some(Phase::Recovery { .. }) if frame.on_ground && above => self.phase = None,
            _ => {}
        }
    }
}

fn air_deadline(kind: Kind, frame: &Frame, now: Instant) -> Instant {
    let seconds = if kind == Kind::Dive {
        5.0
    } else {
        let height = (frame.position.z - frame.water.unwrap_or(frame.position.z)).max(0.0);
        let up = frame.velocity.z.clamp(-200.0, 200.0);
        ((up + (up * up + 19.62 * height).sqrt()) / 9.81 * 1.5 + 3.0).clamp(5.0, 120.0)
    };
    now + Duration::from_secs_f32(seconds)
}

fn deadline(now: Instant, seconds: f32) -> Instant {
    let seconds = if seconds.is_finite() { seconds.clamp(0.1, 6.0) } else { 2.4 };
    now + Duration::from_secs_f32(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame {
            position: Vec3::new(0.0, 0.0, 24.0),
            velocity: Vec3::new(3.0, 0.0, 0.0),
            height: 1.8,
            water: Some(20.0),
            water_contact: None,
            ground: Some(16.0),
            flying: false,
            can_fly: true,
            seated: false,
            suspended: false,
            locked: false,
            bridge_ready: true,
            running: false,
            obstacle: false,
            landing: None,
            on_ground: true,
            jumping: false,
        }
    }
    fn clips() -> Clips {
        Clips {
            dive: true,
            cannonball: true,
            ..Default::default()
        }
    }
    fn landing() -> Option<Landing> {
        Some(Landing {
            position: Vec3::new(5.0, 0.0, 20.0),
            seconds: 1.5,
        })
    }

    #[test]
    fn a_late_cannonball_finishes_its_pre_loop_tuck_and_full_recovery() {
        let now = Instant::now();
        let mut runtime = Runtime::default();
        let mut f = frame();
        let mut input = MoveInput {
            up: true,
            ..Default::default()
        };
        runtime.update(true, clips(), &f, &input, landing(), now);
        f.on_ground = false;
        f.velocity.z = -8.0;
        input.up = false;
        input.down = true;
        runtime.update(true, clips(), &f, &input, landing(), now + Duration::from_millis(1200));
        assert_eq!(runtime.pose(), Some(State::WaterCannonball));
        f.water_contact = Some(Vec3::new(0.0, 0.0, 19.99));
        runtime.update(true, clips(), &f, &input, None, now + Duration::from_millis(1250));
        assert_eq!(runtime.take_impact().unwrap().kind, Kind::Cannonball);
        runtime.update(true, clips(), &f, &input, None, now + Duration::from_millis(3950));
        assert!(runtime.outro(), "the 50 ms intro cannot cut its remaining 2.75 seconds");
        runtime.update(true, clips(), &f, &input, None, now + Duration::from_millis(4050));
        assert!(!runtime.outro());
        let c = clips().cannonball_recovery;
        assert!(
            (c.seconds_after_stop(0.9) - 1.93).abs() < 0.001,
            "a held tuck uses its actual loop phase"
        );
    }

    #[test]
    fn prediction_requires_deep_known_water_and_a_clear_terrain_arc() {
        let f = frame();
        let found = predict(&f, Vec3::X * 3.0, |_| Some(16.0)).expect("deep water ahead");
        assert!(found.position.x > 3.0 && found.seconds < 2.5);
        assert!(predict(&f, Vec3::X * 3.0, |_| None).is_none());
        assert!(predict(&f, Vec3::X * 3.0, |_| Some(19.5)).is_none());
        assert!(
            predict(&f, Vec3::ZERO, |_| Some(16.0)).is_some(),
            "a vertical cannonball is valid too"
        );
        assert!(predict(&f, Vec3::X * 3.0, |p| Some(if (1.0..3.0).contains(&p.x) { 30.0 } else { 16.0 })).is_none());
    }

    #[test]
    fn airborne_down_edge_tucks_once_and_actual_body_contact_makes_one_impact() {
        let now = Instant::now();
        let mut r = Runtime::default();
        let mut f = frame();
        let mut input = MoveInput {
            up: true,
            ..Default::default()
        };
        r.update(true, clips(), &f, &input, landing(), now);
        assert!(r.pose().is_none(), "jumping alone must retain the ordinary jump");
        f.on_ground = false;
        f.jumping = true;
        f.velocity.z = 2.0;
        input.up = false;
        input.down = true;
        r.update(true, clips(), &f, &input, landing(), now + Duration::from_millis(100));
        assert_eq!(r.pose(), Some(State::WaterDiveJump));
        let observer = r.observer(f.height).expect("passive bridge observer");
        assert_eq!(observer.mode, 11);
        assert_eq!(observer.parameters, [0.0, 1.8, 0.0]);
        input.down = false;
        r.update(true, clips(), &f, &input, landing(), now + Duration::from_millis(150));
        input.down = true;
        r.update(true, clips(), &f, &input, landing(), now + Duration::from_millis(200));
        assert_eq!(r.pose(), Some(State::WaterCannonball));
        assert_eq!(
            r.observer(f.height).unwrap().action,
            observer.action,
            "one action survives the pose change"
        );
        f.velocity.z = -4.0;
        f.water_contact = Some(Vec3::new(0.8, 0.1, 19.99));
        r.update(true, clips(), &f, &input, None, now + Duration::from_millis(500));
        let impact = r.take_impact().expect("body touches water even with root still above it");
        assert_eq!(impact.kind, Kind::Cannonball);
        assert_eq!(impact.position, Vec3::new(0.8, 0.1, 20.0));
        assert_eq!(impact.speed, 4.0);
        assert!(r.take_impact().is_none());
        assert!(r.observer(f.height).is_none());
        assert!(r.outro());
        r.update(true, clips(), &f, &input, None, now + Duration::from_millis(600));
        assert!(
            r.take_impact().is_none(),
            "a held key/continued immersion cannot retrigger the burst"
        );
    }

    #[test]
    fn a_held_ground_descend_key_is_not_a_cannonball_gesture() {
        let now = Instant::now();
        let mut r = Runtime::default();
        let mut f = frame();
        let c = Clips {
            dive: false,
            cannonball: true,
            ..Default::default()
        };
        let mut input = MoveInput {
            down: true,
            ..Default::default()
        };
        r.update(true, c, &f, &input, landing(), now);
        f.on_ground = false;
        f.jumping = true;
        f.velocity.z = 2.0;
        r.update(true, c, &f, &input, landing(), now + Duration::from_millis(100));
        assert!(r.pose().is_none());
        input.down = false;
        r.update(true, c, &f, &input, landing(), now + Duration::from_millis(150));
        input.down = true;
        r.update(true, c, &f, &input, landing(), now + Duration::from_millis(200));
        assert_eq!(r.pose(), Some(State::WaterCannonball));
    }

    #[test]
    fn cannonball_starts_from_flight_without_a_jump_or_predicted_landing() {
        let now = Instant::now();
        let mut r = Runtime::default();
        let mut f = frame();
        f.position.z = 150.0;
        f.on_ground = false;
        f.flying = true;
        let c = Clips { dive: false, ..clips() };
        let input = MoveInput {
            down: true,
            ..Default::default()
        };
        r.update(true, c, &f, &input, None, now);
        assert_eq!(r.pose(), Some(State::WaterCannonball));
        assert_eq!(r.observer(f.height).unwrap().mode, 12);
        let action = r.observer(f.height).unwrap().action;
        // An initial server flight echo is allowed; genuine flight resumption
        // after the viewer's release cancels the action.
        r.update(true, c, &f, &input, None, now + Duration::from_millis(100));
        assert!(r.cannonball());
        f.flying = false;
        f.velocity.z = -20.0;
        for second in 1..=6 {
            f.position.z -= 20.0;
            r.update(true, c, &f, &MoveInput::default(), None, now + Duration::from_secs(second));
            assert!(r.cannonball(), "a long fall must survive the former five-second timeout");
            assert_eq!(r.observer(f.height).unwrap().action, action);
        }
        f.flying = true;
        r.update(true, c, &f, &input, None, now + Duration::from_millis(6100));
        assert!(!r.busy() && r.take_impact().is_none());
    }

    #[test]
    fn standalone_cannonball_accepts_unknown_terrain_but_rejects_known_dry_ground() {
        let now = Instant::now();
        for ground in [None, Some(16.0), Some(19.8), Some(21.0), Some(f32::NAN)] {
            let mut r = Runtime::default();
            let mut f = frame();
            f.on_ground = false;
            f.ground = ground;
            r.update(
                true,
                clips(),
                &f,
                &MoveInput {
                    down: true,
                    ..Default::default()
                },
                None,
                now,
            );
            assert_eq!(r.cannonball(), ground.is_none_or(|g| g < 20.0));
        }
    }

    #[test]
    fn a_dive_hands_off_the_p5_hold_then_releases_its_full_outro() {
        let now = Instant::now();
        let mut r = Runtime::default();
        let mut f = frame();
        r.update(
            true,
            clips(),
            &f,
            &MoveInput {
                up: true,
                ..Default::default()
            },
            landing(),
            now,
        );
        f.on_ground = false;
        f.jumping = true;
        f.velocity.z = 2.0;
        r.update(
            true,
            clips(),
            &f,
            &MoveInput {
                down: true,
                ..Default::default()
            },
            landing(),
            now + Duration::from_millis(100),
        );
        assert_eq!(r.pose(), Some(State::WaterDiveJump));
        f.velocity.z = -5.0;
        f.water_contact = Some(Vec3::new(0.0, 0.0, 19.9));
        let input = MoveInput::default();
        r.update(true, clips(), &f, &input, None, now + Duration::from_secs(1));
        assert_eq!(r.pose(), Some(State::WaterDiveEntry));
        assert!(!r.outro());
        r.update(true, clips(), &f, &input, None, now + Duration::from_millis(1100));
        assert_eq!(r.pose(), Some(State::WaterDiveEntry));
        r.update(true, clips(), &f, &input, None, now + Duration::from_millis(1200));
        assert!(r.outro());
        assert!(r.pose().is_none());
        r.update(true, clips(), &f, &input, None, now + Duration::from_millis(3500));
        assert!(r.outro());
        r.update(true, clips(), &f, &input, None, now + Duration::from_millis(3601));
        assert!(!r.outro());
    }

    #[test]
    fn unavailable_contexts_and_position_jumps_cancel_without_a_splash() {
        for reason in 0..7 {
            let now = Instant::now();
            let mut r = Runtime::default();
            let mut f = frame();
            let input = MoveInput {
                up: true,
                ..Default::default()
            };
            r.update(true, clips(), &f, &input, landing(), now);
            f.on_ground = false;
            f.jumping = true;
            r.update(
                true,
                clips(),
                &f,
                &MoveInput {
                    down: true,
                    ..Default::default()
                },
                landing(),
                now + Duration::from_millis(10),
            );
            assert_eq!(r.pose(), Some(State::WaterDiveJump));
            match reason {
                0 => f.seated = true,
                1 => f.locked = true,
                2 => f.suspended = true,
                3 => f.flying = true,
                4 => f.bridge_ready = false,
                5 => f.position.x += 30.0,
                _ => f.water = None,
            }
            r.update(true, clips(), &f, &input, landing(), now + Duration::from_millis(50));
            assert!(r.pose().is_none() && r.observer(f.height).is_none() && r.take_impact().is_none());
        }
    }

    #[test]
    fn a_dive_requires_a_fresh_early_down_edge_and_an_available_safe_dive() {
        let now = Instant::now();
        for (delay, flying, predicted, dive, expected) in [
            (100, false, true, true, State::WaterDiveJump),
            (450, false, true, true, State::WaterDiveJump),
            (451, false, true, true, State::WaterCannonball),
            (100, true, true, true, State::WaterCannonball),
            (100, false, false, true, State::WaterCannonball),
            (100, false, true, false, State::WaterCannonball),
        ] {
            let mut r = Runtime::default();
            let mut f = frame();
            let clips = Clips { dive, ..clips() };
            r.update(
                true,
                clips,
                &f,
                &MoveInput {
                    up: true,
                    ..Default::default()
                },
                landing(),
                now,
            );
            assert!(!r.busy());
            f.on_ground = false;
            f.velocity.z = 2.0;
            f.flying = flying;
            r.update(true, clips, &f, &MoveInput::default(), landing(), now + Duration::from_millis(50));
            assert!(!r.busy(), "a predicted landing cannot start an animation on its own");
            let input = MoveInput {
                down: true,
                ..Default::default()
            };
            r.update(
                true,
                clips,
                &f,
                &input,
                if predicted { landing() } else { None },
                now + Duration::from_millis(delay),
            );
            assert_eq!(
                r.pose(),
                Some(expected),
                "delay {delay}, flight {flying}, predicted {predicted}, clip {dive}"
            );
            r.update(true, clips, &f, &input, landing(), now + Duration::from_millis(delay + 20));
            assert_eq!(r.pose(), Some(expected), "holding Down must not count as a second tap");
        }
    }

    #[test]
    fn a_delayed_native_jump_starts_its_gesture_window_when_signalled() {
        let now = Instant::now();
        let mut r = Runtime::default();
        let mut f = frame();
        r.update(
            true,
            clips(),
            &f,
            &MoveInput {
                up: true,
                ..Default::default()
            },
            landing(),
            now,
        );
        f.on_ground = false;
        f.jumping = true;
        f.velocity.z = 2.0;
        r.update(
            true,
            clips(),
            &f,
            &MoveInput::default(),
            landing(),
            now + Duration::from_millis(700),
        );
        assert!(!r.busy());
        r.update(
            true,
            clips(),
            &f,
            &MoveInput {
                down: true,
                ..Default::default()
            },
            landing(),
            now + Duration::from_millis(900),
        );
        assert_eq!(r.pose(), Some(State::WaterDiveJump));
    }

    #[test]
    fn authored_cannonball_is_a_native_body_only_loop_with_a_recovery_tail() {
        let a = aurora_assets::parse_animation(include_bytes!("../../assets/animations/swimming/cannonball.anim"))
            .expect("native authored clip");
        assert_eq!(a.base_priority, 6);
        assert!(a.looping);
        assert!(a.loop_in > 0.0 && a.loop_out > a.loop_in && a.duration - a.loop_out > 1.5);
        assert_eq!(a.joints.len(), 21);
        for joint in &a.joints {
            assert_eq!(joint.priority, 6);
            assert!(joint.pos_keys.is_empty() || joint.joint_name == "mPelvis");
            assert!(joint.rot_keys.iter().all(|key| key.rotation.is_finite()));
            assert!(joint.pos_keys.iter().all(|key| key.position.abs().max_element() < 0.04));
            if let (Some(a), Some(b)) = (joint.rotation_at(a.loop_in), joint.rotation_at(a.loop_out)) {
                assert!(a.dot(b).abs() > 0.99999, "the held tuck must join exactly");
            }
        }
    }
}
