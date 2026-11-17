//! Gaming mode: a toolbar button with which the viewer steps back for a game
//! or another heavy program and stays logged in. The world stops drawing at
//! once (the pause of a window behind on a busy card, busy_pause.rs, without
//! waiting for slow frames: the user said why), the loop runs at
//! `VISIBLE_FPS` while the window can be seen and `HIDDEN_FPS` once it is
//! minimised or covered, and the in-world media stop drawing (Hidden
//! priority, media/update.rs). The network, the chat, the sounds and the
//! notifications go on: with fewer frames, each one may apply more of the
//! network's events (`event_budget`). Back in front, it ends by itself
//! (`gaming_mode_leave_on_focus`), or with the button again. A headset that
//! presents ends it too: the OpenXR frame waiter paces VR, and a world that
//! stops drawing in front of the eyes is not a pause (busy_pause.rs leaves
//! VR alone the same way). Not in the Legacy Viewer.

use super::*;

/// Frames per second while the window is in sight: the interface over the
/// blurred world still answers a click, slowly.
const VISIBLE_FPS: f32 = 5.0;
/// Frames per second minimised or covered: enough to read the network's
/// news and raise the notifications, nothing is seen.
const HIDDEN_FPS: f32 = 2.0;
/// Time a frame may spend applying the network's events (8 ms otherwise,
/// `App::pump_network_events`). At `HIDDEN_FPS` that is 120 ms a second,
/// what the 15 fps background cap gives at 8 ms a frame: a busy region's
/// object updates do not pile up behind the slow loop.
const EVENT_BUDGET: Duration = Duration::from_millis(60);

#[derive(Debug, Default)]
pub(super) struct GamingMode {
    pub(super) on: bool,
    /// AURORA_DEMO_GAMING_MODE: the scenario turned it on once.
    demo_done: bool,
    /// When the focus coming back ended it (`gaming_mode_leave_on_focus`).
    left_on_focus: Option<Instant>,
}

/// A press of the button this soon after the focus came back is the click
/// that brought the window to front: the focus already ended gaming mode,
/// and the press must not turn it on again. Seen in demo: off then on 17 ms
/// later, a second click needed. The press reaches the toolbar at the next
/// frame, up to 500 ms away at `HIDDEN_FPS`.
const FOCUS_CLICK: Duration = Duration::from_secs(1);

/// Whether a press of the button at `now` is the click that brought the
/// window to front, gaming mode ended by the focus at `left`.
fn focus_click(left: Option<Instant>, now: Instant) -> bool {
    left.is_some_and(|left| now.saturating_duration_since(left) < FOCUS_CLICK)
}

/// The frame limiter's cap (0 = none) under gaming mode: `hidden` the
/// window is minimised or covered.
pub(super) fn cap(cap: f32, on: bool, hidden: bool) -> f32 {
    if !on {
        return cap;
    }
    let low = if hidden { HIDDEN_FPS } else { VISIBLE_FPS };
    if cap >= 1.0 { cap.min(low) } else { low }
}

/// The time a frame may spend applying the network's events, gaming mode
/// `on` or not.
pub(super) fn event_budget(on: bool) -> Duration {
    if on { EVENT_BUDGET } else { Duration::from_millis(8) }
}

/// `AURORA_DEMO_GAMING_MODE=<frame>`: gaming mode turns on at that frame,
/// as from its button, to see the pause, its notice and its title.
fn demo_from() -> Option<u64> {
    static FROM: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *FROM.get_or_init(|| std::env::var("AURORA_DEMO_GAMING_MODE").ok()?.trim().parse().ok())
}

impl App {
    /// The toolbar's Gaming mode button.
    pub(super) fn toggle_gaming_mode(&mut self) {
        let left = self.gaming_mode.left_on_focus.take();
        if !self.gaming_mode.on && focus_click(left, Instant::now()) {
            return;
        }
        self.set_gaming_mode(!self.gaming_mode.on);
    }

    fn set_gaming_mode(&mut self, on: bool) {
        if self.gaming_mode.on == on {
            return;
        }
        self.gaming_mode.on = on;
        log::info!("gaming mode {}", if on { "on" } else { "off" });
    }

    /// The window has the focus again.
    pub(super) fn gaming_mode_focused(&mut self) {
        if self.settings.gaming_mode_leave_on_focus && self.gaming_mode.on {
            self.set_gaming_mode(false);
            self.gaming_mode.left_on_focus = Some(Instant::now());
        }
    }

    /// The 3D drawing pauses for gaming mode; a photo being taken is
    /// drawn all the same.
    pub(super) fn gaming_mode_pauses(&self) -> bool {
        self.gaming_mode.on && !self.snapshots.drawing()
    }

    /// Every frame, after the render: a headset presenting ends gaming mode
    /// (`vr`), then the demo scenario.
    pub(super) fn gaming_mode_frame(&mut self, vr: bool) {
        if vr && self.gaming_mode.on {
            log::info!("gaming mode ends: a headset presents");
            self.set_gaming_mode(false);
            return;
        }
        if self.demo && !self.gaming_mode.demo_done && demo_from().is_some_and(|from| self.frame_count >= from) {
            self.gaming_mode.demo_done = true;
            self.set_gaming_mode(true);
        }
    }

    /// The notice over the paused world (busy_pause.rs draws it).
    pub(super) fn gaming_mode_notice(&self) -> &'static str {
        if self.settings.gaming_mode_leave_on_focus {
            aurora_i18n::tr!("gaming-mode-notice")
        } else {
            aurora_i18n::tr!("gaming-mode-notice-button")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_leaves_the_cap_as_it_was() {
        assert_eq!(cap(0.0, false, true), 0.0);
        assert_eq!(cap(15.0, false, false), 15.0);
    }

    #[test]
    fn on_crawls_and_slower_out_of_sight() {
        assert_eq!(cap(0.0, true, false), VISIBLE_FPS, "no cap at all becomes the visible one");
        assert_eq!(cap(120.0, true, false), VISIBLE_FPS);
        assert_eq!(cap(120.0, true, true), HIDDEN_FPS);
        assert_eq!(cap(1.0, true, false), 1.0, "a lower cap of the user stays");
    }

    #[test]
    fn the_click_that_brings_the_window_back_does_not_turn_it_on_again() {
        let left = Instant::now();
        assert!(focus_click(Some(left), left + Duration::from_millis(17)));
        assert!(focus_click(Some(left), left + Duration::from_millis(500)), "next frame at 2 fps");
        assert!(!focus_click(Some(left), left + FOCUS_CLICK), "a later press is the user's");
        assert!(!focus_click(None, left), "not ended by the focus");
    }

    #[test]
    fn the_slow_loop_applies_as_many_events_a_second_as_the_background_cap() {
        assert_eq!(event_budget(false), Duration::from_millis(8));
        let per_second = |budget: Duration, fps: f32| budget.as_secs_f32() * fps;
        let background = per_second(event_budget(false), 15.0);
        assert!(per_second(event_budget(true), HIDDEN_FPS) >= background - 1e-6);
        assert!(per_second(event_budget(true), VISIBLE_FPS) >= background);
    }
}
