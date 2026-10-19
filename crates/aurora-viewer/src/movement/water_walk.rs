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
    jump_until: Option<Instant>,
    resume_after: Option<Instant>,
    pose: Option<State>,
    signal: Option<PlayingAnimation>,
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
            self.jump_until = None;
            self.resume_after = None;
            return None;
        }
        if up_edge && self.active {
            self.jump_until = Some(now + Duration::from_millis(350));
            self.resume_after = Some(now + Duration::from_millis(750));
        }
        if self.jump_until.is_some_and(|until| now < until) {
            self.active = false;
            self.pose = Some(State::Jumping);
            let mut motion = Motion::motor(4, 3.2, frame.height);
            motion.parameters[2] = 2.5;
            return Some(motion);
        }
        let target = frame.water? + frame.height * 0.5;
        if self.resume_after.is_some_and(|until| now < until) || frame.position.z > target + 0.45 {
            self.active = false;
            self.pose = None;
            self.signal = None;
            return None;
        }
        self.active = true;
        let moving = input.forward || input.back || input.strafe_left || input.strafe_right || frame.velocity.truncate().length() > 0.25;
        let running = frame.running || input.run;
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
    fn water_walk_can_rise_from_depth_and_jump_without_restarting_the_pulse() {
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
        assert_eq!(r.update(true, &f, &input, now).unwrap().mode, 4);
        assert_eq!(r.signal(now).unwrap().0, State::Jumping);
        assert!(r.update(true, &f, &input, now + Duration::from_millis(400)).is_none());
        assert_eq!(r.update(true, &f, &input, now + Duration::from_millis(800)).unwrap().mode, 13);
        assert_eq!(r.signal(now).unwrap().0, State::Standing);
    }
}
