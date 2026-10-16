//! Water-key gestures use resolved movement actions, never physical key names.
//! The 0.5 s hold follows Firestorm's agent_jump (llviewerinput.cpp).
//! Tap-to-dive/surface and collision-aware shore jumps are Aurora extensions
//! requested by the maintainer; Firestorm's AO only supplies swim animations.
use super::{Frame, State};
use crate::agent::MoveInput;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Dive,
    Surface,
    Jump,
    Fly,
    CancelSurface,
}

struct AscentPress {
    since: Instant,
    origin: State,
    fired: bool,
    canceled_surface: bool,
}

#[derive(Default)]
pub struct Controls {
    up: Option<AscentPress>,
    down: bool,
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
        self.down = input.down;
        if !enabled || frame.seated || frame.suspended {
            self.up = None;
            return None;
        }
        if input.up && input.down {
            self.up = None;
            return None;
        }
        if down_edge && state == State::Surface {
            self.up = None;
            return Some(Action::Dive);
        }
        if input.up {
            if let Some(press) = &mut self.up {
                if !press.fired && frame.can_fly && now.saturating_duration_since(press.since) >= Duration::from_millis(500) {
                    press.fired = true;
                    return Some(Action::Fly);
                }
            } else if state.swimming() || state == State::Leaving {
                self.up = Some(AscentPress {
                    since: now,
                    origin: state,
                    fired: false,
                    canceled_surface: surfacing,
                });
                if surfacing {
                    return Some(Action::CancelSurface);
                }
                if state == State::Submerged {
                    return Some(Action::Surface);
                }
            }
        } else if let Some(press) = self.up.take()
            && !press.fired
            && !press.canceled_surface
        {
            return match press.origin {
                State::Submerged => Some(Action::Surface),
                State::Surface if frame.obstacle => Some(Action::Jump),
                State::Surface if frame.can_fly => Some(Action::Fly),
                _ => None,
            };
        }
        None
    }
}
