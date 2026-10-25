//! "Crouch as a toggle" (the Legacy Viewer's FSCrouchToggle: agent_push_down
//! in llviewerinput.cpp and the latch of LLAppViewer::idle, originally LGPL
//! 2.1): a press of the Down key crouches until the next press; jumping or
//! flying lets go. In flight the key descends as usual, and seated it does
//! nothing more than before. Unlike the Legacy Viewer's per-account
//! FSCrouchToggleStatus, the latch does not outlive the session.

#[derive(Debug, Default)]
pub struct CrouchToggle {
    latched: bool,
    /// The Down key was held on the last frame (presses are edges).
    was_down: bool,
}

impl CrouchToggle {
    pub fn latched(&self) -> bool {
        self.latched
    }

    /// The Down input to use this frame, from the keys' `down` and `up`.
    pub fn apply(&mut self, on: bool, flying: bool, sitting: bool, down: bool, up: bool) -> bool {
        let pressed = down && !self.was_down;
        self.was_down = down;
        if !on || flying || up {
            self.latched = false;
            return down;
        }
        if sitting {
            return down;
        }
        if pressed {
            self.latched = !self.latched;
        }
        self.latched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::MoveInput;

    #[test]
    fn latched_crouch_keeps_camera_reset_for_other_movement_axes() {
        let mut crouch = CrouchToggle::default();
        assert!(crouch.apply(true, false, false, true, false));
        for axis in 0..9 {
            let mut input = MoveInput {
                down: crouch.apply(true, false, false, false, false),
                ..Default::default()
            };
            match axis {
                0 => input.forward = true,
                1 => input.back = true,
                2 => input.turn_left = true,
                3 => input.turn_right = true,
                4 => input.strafe_left = true,
                5 => input.strafe_right = true,
                6 => input.up = true,
                7 => input.joystick_yaw = 0.5,
                _ => input.joystick_pitch = -0.5,
            }
            assert!(input.resets_camera(crouch.latched()), "movement axis {axis}");
        }
    }

    #[test]
    fn held_crouch_jump_and_flight_keep_camera_reset() {
        for (on, flying, sitting, down, up) in [
            (false, false, false, true, false),
            (true, true, false, true, false),
            (true, false, true, true, false),
            (true, false, false, false, true),
        ] {
            let mut crouch = CrouchToggle::default();
            let input = MoveInput {
                down: crouch.apply(on, flying, sitting, down, up),
                up,
                ..Default::default()
            };
            assert!(input.resets_camera(crouch.latched()));
        }
    }

    #[test]
    fn a_press_crouches_until_the_next_one() {
        let mut c = CrouchToggle::default();
        assert!(c.apply(true, false, false, true, false), "pressed");
        assert!(c.apply(true, false, false, true, false), "held: no new press");
        assert!(c.apply(true, false, false, false, false), "released: still crouched");
        assert!(!c.apply(true, false, false, true, false), "second press");
        assert!(!c.apply(true, false, false, true, false), "held");
        assert!(!c.apply(true, false, false, false, false));
    }

    #[test]
    fn jumping_or_flying_lets_go_and_flight_descends_as_held() {
        let mut c = CrouchToggle::default();
        c.apply(true, false, false, true, false);
        c.apply(true, false, false, false, false);
        assert!(!c.apply(true, false, false, false, true), "jump");
        assert!(!c.apply(true, false, false, false, false));
        c.apply(true, false, false, true, false);
        c.apply(true, false, false, false, false);
        assert!(!c.apply(true, true, false, false, false), "flying");
        assert!(c.apply(true, true, false, true, false), "held in flight");
        assert!(!c.apply(true, true, false, false, false));
    }

    #[test]
    fn off_the_key_is_held_as_usual() {
        let mut c = CrouchToggle::default();
        assert!(c.apply(false, false, false, true, false));
        assert!(!c.apply(false, false, false, false, false));
    }
}
