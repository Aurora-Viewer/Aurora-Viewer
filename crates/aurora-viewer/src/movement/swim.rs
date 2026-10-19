//! Water-key gestures use resolved movement actions, never physical key names.
//! Unlike agent_jump (llviewerinput.cpp), one surface ascent press immediately
//! requests a shore jump; a second press requests flight. These are extensions
//! requested by the maintainer; the Legacy AO only supplies swim animations.
use super::{Frame, State};
use crate::agent::MoveInput;
use std::time::{Duration, Instant};

const DOUBLE_ASCENT: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Dive,
    Surface,
    Jump,
    Fly,
}

#[derive(Default)]
pub struct Controls {
    up: bool,
    down: bool,
    surface_press: Option<Instant>,
}

impl Controls {
    pub fn update(
        &mut self,
        enabled: bool,
        state: State,
        surfacing: bool,
        frame: &Frame,
        input: &MoveInput,
        now: Instant,
    ) -> Option<Action> {
        let down_edge = input.down && !self.down;
        let up_edge = input.up && !self.up;
        self.down = input.down;
        self.up = input.up;
        if self
            .surface_press
            .is_some_and(|first| now.saturating_duration_since(first) > DOUBLE_ASCENT)
            || !matches!(state, State::Surface | State::Leaving)
        {
            self.surface_press = None;
        }
        if !enabled || frame.seated || frame.suspended {
            self.surface_press = None;
            return None;
        }
        if input.up && input.down {
            self.surface_press = None;
            return None;
        }
        if input.down {
            self.surface_press = None;
        }
        if down_edge && state == State::Surface {
            return Some(Action::Dive);
        }
        if up_edge {
            // A held underwater press cannot become a second action on arrival.
            if state == State::Submerged || (surfacing && state.swimming()) {
                self.surface_press = None;
                return Some(Action::Surface);
            }
            if self.surface_press.take().is_some() {
                return frame.can_fly.then_some(Action::Fly);
            }
            if state == State::Surface {
                self.surface_press = Some(now);
                return Some(Action::Jump);
            }
        }
        None
    }
}
