//! Relative lens reset gestures, independent of input polling and frame rate.

use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct ResetGesture {
    ready: bool,
    down: bool,
    started: Option<Instant>,
    tap: Option<Instant>,
    blocked: bool,
}

impl ResetGesture {
    /// A tap must be short, with a release before another actuation.
    /// Keep a reset chord/second press from immediately zooming again.
    pub fn sample(&mut self, value: f32, pressed: bool, chord: bool, taps: bool, now: Instant) -> (bool, bool) {
        const WINDOW: Duration = Duration::from_millis(350);
        const TAP: Duration = Duration::from_millis(250);
        let neutral = value <= 0.15 && !pressed && !chord;
        let down = value > if self.down { 0.15 } else { 0.5 };
        if !self.ready {
            self.ready = neutral;
            self.down = down;
            return (false, false);
        }
        if self.blocked {
            if neutral {
                self.blocked = false;
            }
            self.down = down;
            return (false, true);
        }
        if self.tap.is_some_and(|at| now.saturating_duration_since(at) > WINDOW) {
            self.tap = None;
        }
        // A fresh native button counter proves an intervening release,
        // even if both frame samples happened while the button was held.
        if pressed && self.down {
            self.tap = self.started.filter(|at| now.saturating_duration_since(*at) <= TAP);
        }
        let press = pressed || (down && !self.down);
        let reset = chord || (taps && press && self.tap.is_some());
        if reset {
            self.blocked = true;
            self.started = None;
            self.tap = None;
        } else if taps {
            if press {
                self.started = Some(now);
            }
            if !down && (self.down || pressed) {
                self.tap = self.started.take().filter(|at| now.saturating_duration_since(*at) <= TAP);
            }
        }
        self.down = down;
        (reset, reset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joystick_zoom_reset_requires_short_taps_and_release_and_suppresses_the_second_hold() {
        let origin = Instant::now();
        let mut gesture = ResetGesture::default();
        let mut sample = |ms, value, pressed| gesture.sample(value, pressed, false, true, origin + Duration::from_millis(ms));
        assert_eq!(sample(0, 0.8, false), (false, false), "ignore an initially held axis");
        sample(20, 0.0, false);
        sample(40, 0.8, false);
        sample(60, 0.3, false);
        assert_eq!(sample(70, 0.8, false), (false, false), "jitter is not a release");
        sample(100, 0.0, false);
        assert_eq!(sample(180, 0.8, false), (true, true));
        assert_eq!(sample(200, 0.8, false), (false, true));
        assert_eq!(sample(220, 0.0, false), (false, true));
        assert_eq!(sample(240, 0.8, false), (false, false), "a new gesture can zoom again");
        sample(600, 0.0, false);
        assert_eq!(sample(620, 0.8, false), (false, false), "a long hold is not a tap");
        sample(640, 0.0, false);
        assert_eq!(sample(1000, 0.8, false), (false, false), "expired taps cannot reset");
    }

    #[test]
    fn joystick_zoom_reset_accepts_short_button_edges_and_latches_a_chord_until_both_release() {
        let origin = Instant::now();
        let mut gesture = ResetGesture::default();
        gesture.sample(0.0, false, false, true, origin);
        gesture.sample(0.0, true, false, true, origin + Duration::from_millis(20));
        assert_eq!(
            gesture.sample(0.0, true, false, true, origin + Duration::from_millis(120)),
            (true, true)
        );
        let mut counters = ResetGesture::default();
        counters.sample(0.0, false, false, true, origin);
        counters.sample(1.0, true, false, true, origin + Duration::from_millis(20));
        assert_eq!(
            counters.sample(1.0, true, false, true, origin + Duration::from_millis(120)),
            (true, true)
        );
        let mut chord = ResetGesture::default();
        chord.sample(0.0, false, false, false, origin);
        assert_eq!(chord.sample(1.0, true, true, false, origin), (true, true));
        assert_eq!(chord.sample(1.0, false, false, false, origin), (false, true));
        assert_eq!(chord.sample(0.0, false, false, false, origin), (false, true));
        assert_eq!(chord.sample(1.0, true, false, false, origin), (false, false));
    }
}
