//! Explicit physical support at the native waterline. The existing bridge
//! owns collision-aware targets; the ordinary AO owns all animation requests.
use super::Frame;
use crate::{agent::MoveInput, ao::State, world::PlayingAnimation};
use aurora_net::bridge::Motion;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Runtime {
    active: bool,
    up: bool,
    jump: Option<Jump>,
    pose: Option<State>,
    signal: Option<PlayingAnimation>,
}

struct Jump {
    motion: Motion,
    started: Instant,
    risen: bool,
}

impl Runtime {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn pose(&self) -> Option<State> {
        self.pose
    }

    pub fn signal(&mut self, now: Instant) -> Option<(State, PlayingAnimation)> {
        let state = self.pose?;
        let signal = PlayingAnimation::from_signal(state.motion(), 0, now, self.signal.as_ref().filter(|a| a.id == state.motion()));
        self.signal = Some(signal);
        Some((state, signal))
    }

    pub fn update(&mut self, enabled: bool, frame: &Frame, input: &MoveInput, now: Instant) -> Option<Motion> {
        let up_edge = input.up && !self.up;
        self.up = input.up;
        let supported = frame
            .water
            .zip(frame.ground)
            .is_some_and(|(water, ground)| water.is_finite() && ground.is_finite() && water - ground > 0.06);
        if !enabled
            || !frame.bridge_ready
            || frame.seated
            || frame.suspended
            || frame.flying
            || !frame.position.is_finite()
            || !frame.height.is_finite()
            || !supported
            || (frame.locked && !self.active)
        {
            self.active = false;
            self.pose = None;
            self.signal = None;
            self.jump = None;
            return None;
        }
        let target = frame.water? + frame.height * 0.5;
        let running = frame.running || input.run;
        let speed = if running { 5.13 } else { 3.2 };
        // LLAgent::moveUp starts a ground jump on ascent. Native translation
        // is captured over water, so keep the jump in the support motor with
        // one action ID; repeated commands must never renew its trajectory.
        if up_edge && self.active && self.jump.is_none() && !frame.locked && (frame.position.z - target).abs() < 0.3 {
            self.jump = Some(Jump {
                motion: Motion::water_walk_jump(speed, frame.height),
                started: now,
                risen: false,
            });
        }
        if let Some(jump) = &mut self.jump {
            jump.risen |= frame.position.z > target + 0.2;
            let landed = jump.risen && frame.position.z <= target + 0.12 && frame.velocity.z <= 0.5;
            if !frame.locked && !landed && now.duration_since(jump.started) < Duration::from_secs(3) {
                // Keep the same jump animation clock through descent. This
                // supported arc returns to water, so a native fall signal
                // must not interrupt the pose before actual contact.
                self.pose = Some(State::Jumping);
                return Some(jump.motion);
            }
            self.jump = None;
        }
        if frame.position.z > target + 0.45 {
            self.active = false;
            self.pose = None;
            self.signal = None;
            return None;
        }
        self.active = true;
        let moving = input.forward || input.back || input.strafe_left || input.strafe_right || frame.velocity.truncate().length() > 0.25;
        // Match ground crouching intent (LLAgent::moveUp and AOSet's separate
        // crouch states in indra/newview/llagent.cpp and aoset.cpp). Descend
        // keeps the water support; it no longer means leaving this mode.
        self.pose = Some(if input.down {
            if moving { State::CrouchWalk } else { State::Crouching }
        } else if !moving {
            State::Standing
        } else if running {
            State::Running
        } else {
            State::Walking
        });
        let speed = if input.down {
            1.2
        } else if running {
            5.13
        } else {
            3.2
        };
        Some(Motion::motor(13, speed, frame.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn frame() -> Frame {
        Frame {
            position: Vec3::new(40.0, 124.0, 20.9),
            velocity: Vec3::ZERO,
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
            on_ground: false,
            jumping: false,
        }
    }

    #[test]
    fn water_walk_selects_ground_states_and_keeps_their_animation_clock() {
        let now = Instant::now();
        let mut r = Runtime::default();
        let f = frame();
        let mut input = MoveInput::default();
        assert_eq!(r.update(true, &f, &input, now).unwrap().mode, 13);
        assert_eq!(r.signal(now).unwrap().0, State::Standing);
        input.forward = true;
        assert_eq!(r.update(true, &f, &input, now).unwrap().parameters, [3.2, 1.8, 0.0]);
        let (state, first) = r.signal(now).unwrap();
        assert_eq!(state, State::Walking);
        let later = now + Duration::from_millis(100);
        r.update(true, &f, &input, later);
        assert_eq!(r.signal(later).unwrap().1.continuous_start, first.continuous_start);
        input.run = true;
        assert_eq!(r.update(true, &f, &input, later).unwrap().parameters[0], 5.13);
        assert_eq!(r.signal(later).unwrap().0, State::Running);
        input.down = true;
        assert_eq!(r.update(true, &f, &input, later).unwrap().parameters[0], 1.2);
        assert!(r.active());
        assert_eq!(r.signal(later).unwrap().0, State::CrouchWalk);
    }

    #[test]
    fn crouching_supports_all_ground_directions_coasting_and_release() {
        let now = Instant::now();
        for direction in 0..6 {
            let mut r = Runtime::default();
            let mut f = frame();
            f.running = true;
            let mut input = MoveInput {
                down: true,
                run: true,
                ..Default::default()
            };
            match direction {
                0 => input.forward = true,
                1 => input.back = true,
                2 => input.strafe_left = true,
                3 => input.strafe_right = true,
                4 => f.velocity.x = 0.4,
                _ => {}
            }
            let state = if direction == 5 { State::Crouching } else { State::CrouchWalk };
            let motion = r.update(true, &f, &input, now).unwrap();
            assert_eq!(motion.mode, 13);
            assert_eq!(motion.parameters, [1.2, f.height, 0.0]);
            assert!(motion.valid());
            let (pose, first) = r.signal(now).unwrap();
            assert_eq!(pose, state);
            let later = now + Duration::from_millis(250);
            r.update(true, &f, &input, later);
            assert_eq!(r.signal(later).unwrap().1.continuous_start, first.continuous_start);
            input.down = false;
            assert_eq!(r.update(true, &f, &input, later).unwrap().parameters[0], 5.13);
            assert_eq!(r.pose(), Some(if direction == 5 { State::Standing } else { State::Running }));
            assert!(r.active());
        }
    }

    #[test]
    fn water_walk_releases_unavailable_contexts_and_never_pulls_a_flyer_down() {
        let now = Instant::now();
        for reason in 0..8 {
            let mut r = Runtime::default();
            let mut f = frame();
            assert!(r.update(true, &f, &MoveInput::default(), now).is_some());
            match reason {
                0 => f.flying = true,
                1 => f.seated = true,
                2 => f.suspended = true,
                3 => f.bridge_ready = false,
                4 => f.ground = Some(20.0),
                5 => f.water = None,
                6 => f.position.z += 5.0,
                _ => f.position.x = f32::NAN,
            }
            assert!(r.update(true, &f, &MoveInput::default(), now).is_none());
            assert!(!r.active() && r.signal(now).is_none());
        }
    }

    #[test]
    fn water_walk_jump_keeps_one_action_and_animation_through_descent_until_landing() {
        let now = Instant::now();
        let mut r = Runtime::default();
        let mut f = frame();
        f.position.z = 18.0;
        assert_eq!(r.update(true, &f, &MoveInput::default(), now).unwrap().mode, 13);
        f.position.z = 20.9;
        let input = MoveInput {
            up: true,
            ..Default::default()
        };
        let jump = r.update(true, &f, &input, now).unwrap();
        assert_eq!(jump.mode, 13);
        assert_eq!(jump.parameters, [3.2, f.height, 2.5]);
        assert!(jump.valid() && jump.action > 0);
        assert!(r.active());
        let (state, started) = r.signal(now).unwrap();
        assert_eq!(state, State::Jumping);
        // A delayed simulator update must not release the launch command.
        assert_eq!(r.update(true, &f, &input, now + Duration::from_millis(400)), Some(jump));
        f.position.z += 0.7;
        f.velocity.z = 1.5;
        r.update(true, &f, &MoveInput::default(), now + Duration::from_millis(600));
        let later = now + Duration::from_millis(800);
        assert_eq!(
            r.update(true, &f, &input, later),
            Some(jump),
            "another airborne press cannot rearm the jump"
        );
        assert_eq!(r.signal(later).unwrap().0, State::Jumping);
        f.velocity.z = -2.0;
        assert_eq!(r.update(true, &f, &input, later), Some(jump));
        let (state, descending) = r.signal(later).unwrap();
        assert_eq!(state, State::Jumping);
        assert_eq!(descending.continuous_start, started.continuous_start);
        f.position.z = 20.95;
        let landed = now + Duration::from_millis(1400);
        let support = r.update(true, &f, &input, landed).unwrap();
        assert_eq!(support, Motion::motor(13, 3.2, f.height));
        assert_eq!(r.signal(landed).unwrap().0, State::Standing);
        assert_eq!(
            r.update(true, &f, &input, now + Duration::from_secs(2)),
            Some(support),
            "a held key never repeats"
        );
        r.update(true, &f, &MoveInput::default(), now + Duration::from_millis(2100));
        let next = r.update(true, &f, &input, now + Duration::from_millis(2200)).unwrap();
        assert_ne!(next.action, jump.action);
        assert!(next.valid());
    }

    #[test]
    fn water_walk_jump_works_in_each_direction_without_flight_and_preserves_run_speed() {
        let now = Instant::now();
        for direction in 0..5 {
            for run in [false, true] {
                let mut r = Runtime::default();
                let mut f = frame();
                f.can_fly = false;
                let mut input = MoveInput {
                    run,
                    down: true,
                    ..Default::default()
                };
                match direction {
                    1 => input.forward = true,
                    2 => input.back = true,
                    3 => input.strafe_left = true,
                    4 => input.strafe_right = true,
                    _ => {}
                }
                r.update(true, &f, &input, now);
                input.up = true;
                let jump = r.update(true, &f, &input, now).unwrap();
                assert_eq!(jump.parameters[0], if run { 5.13 } else { 3.2 });
                assert!(jump.valid() && jump.action > 0);
                assert_eq!(r.pose(), Some(State::Jumping), "ascent leaves the crouch pose");
            }
        }
    }

    #[test]
    fn water_walk_jump_requires_surface_support_and_cancels_on_context_loss_or_timeout() {
        let now = Instant::now();
        for reason in 0..9 {
            let mut r = Runtime::default();
            let mut f = frame();
            let up = MoveInput {
                up: true,
                ..Default::default()
            };
            r.update(true, &f, &MoveInput::default(), now);
            assert!(r.update(true, &f, &up, now).unwrap().action > 0);
            match reason {
                0 => f.flying = true,
                1 => f.seated = true,
                2 => f.suspended = true,
                3 => f.bridge_ready = false,
                4 => f.water = None,
                5 => f.ground = Some(20.0),
                6 => f.locked = true,
                _ => {}
            }
            let elapsed = if reason == 8 {
                Duration::from_secs(3)
            } else {
                Duration::from_millis(100)
            };
            let result = r.update(reason != 7, &f, &up, now + elapsed);
            assert!(r.jump.is_none());
            assert!(result.is_none_or(|motion| motion.action == 0), "no surviving jump: {reason}");
        }
        for z in [18.0, 22.0] {
            let mut r = Runtime::default();
            let mut f = frame();
            f.position.z = z;
            r.update(true, &f, &MoveInput::default(), now);
            let result = r.update(
                true,
                &f,
                &MoveInput {
                    up: true,
                    ..Default::default()
                },
                now,
            );
            assert!(
                result.is_none_or(|motion| motion.action == 0),
                "no jump before reaching the surface"
            );
        }
    }
}
