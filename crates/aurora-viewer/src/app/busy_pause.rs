//! A window behind on a card another program keeps busy stops drawing its
//! 3D until it has the focus again (#2355). Two viewers on one card, the
//! other in front: Windows gives the card to the window in front first, and
//! this one's frames, about 10 ms alone, took 364 to 611 ms on an RTX 4060
//! before one ran past the driver's time limit (TDR, 2 s) and Windows reset
//! the card ("GPU device lost", with a hang). The pause is the one of a
//! window behind past the driver's memory budget (texture_budget::Background,
//! #1808): empty draw lists and the 3D targets at `PAUSED_RESOLUTION_DIVISOR`,
//! the chat, the network, the sounds and the interface going on. Unlike that
//! one, the textures stay: the card is short of time, not of memory, and the
//! world comes back sharp at once. What is left of the world is blurred
//! and darkened; a notice over it and the window title say so. Not in the Legacy Viewer.

use super::*;

/// A GPU frame at least this long is slow: ten times the background cap's
/// 66 ms would allow the card to keep, and well under the 2 s of the
/// driver's limit (the frames before the loss of #2355: 364 and 611 ms).
const SLOW_MS: f32 = 150.0;
/// Slow frames in a row before the pause: one long frame (a burst of
/// uploads, a pipeline build) is not a busy card.
const SLOW_FRAMES: u8 = 2;

/// Whether the 3D drawing is paused, see the module.
#[derive(Debug, Default)]
pub(super) struct BusyPause {
    /// Slow GPU frames in a row while behind.
    slow: u8,
    paused: bool,
    /// The last GPU time counted: the renderer gives its last measure
    /// again each frame until a new one comes, and one slow frame seen
    /// twice is not two.
    last_ms: Option<f32>,
    /// AURORA_DEMO_BUSY_PAUSE: a click in the window while paused, the
    /// demo's busy card is over (its window often has the focus already).
    pub(super) demo_released: bool,
    /// Dev Toolbox R13: the same pause on demand, in front or behind, with
    /// or without the setting, until R13 is pressed again (a click in the
    /// window does not end it: the button itself is one).
    pub(super) forced: bool,
    /// The setting follows the newest change made in any Aurora window
    /// (instances.rs): the one that pauses is the window behind.
    pub(super) follow: crate::instances::SwitchFollow,
}

impl BusyPause {
    /// One frame: `behind` the window has not the focus, `allowed` the
    /// setting is on and nothing needs the 3D drawn (a photo, a headset, a
    /// timed run), `gpu_ms` the GPU time the renderer measured last (the
    /// same again, or `None`, while no new measure came). Once paused it stays so until the window
    /// is in front again: its own frames are then short and say nothing of
    /// the card. Returns whether the state changed.
    pub(super) fn update(&mut self, behind: bool, allowed: bool, gpu_ms: Option<f32>) -> bool {
        let was = self.paused;
        let new = gpu_ms.filter(|&ms| self.last_ms != Some(ms));
        if gpu_ms.is_some() {
            self.last_ms = gpu_ms;
        }
        if !behind || !allowed {
            self.slow = 0;
            self.paused = false;
        } else if !self.paused
            && let Some(ms) = new
        {
            self.slow = if ms >= SLOW_MS { self.slow.saturating_add(1) } else { 0 };
            self.paused = self.slow >= SLOW_FRAMES;
        }
        self.paused != was
    }

    pub(super) fn paused(&self) -> bool {
        self.paused || self.forced
    }
}

/// `AURORA_DEMO_BUSY_PAUSE=<frame>`: from that frame on, the window counts
/// as behind with slow GPU frames, until it gets the focus (a click), to
/// see the pause, its notice and its title without a second program.
fn demo_from() -> Option<u64> {
    static FROM: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *FROM.get_or_init(|| std::env::var("AURORA_DEMO_BUSY_PAUSE").ok()?.trim().parse().ok())
}

impl App {
    /// The 3D drawing pauses: behind past the driver's memory budget
    /// (texture_budget::Background), or behind on a busy card (this module).
    pub(super) fn drawing_paused(&self) -> bool {
        self.scene.textures.drawing_paused() || self.busy_pause.paused()
    }

    /// Every frame, after the render: the pause follows the GPU time.
    pub(super) fn busy_pause_frame(&mut self, vr: bool) {
        let mut timed = self.capture.is_some() || self.frame_profile.enabled();
        let (mut behind, mut gpu_ms) = (!self.focused, self.last_render.gpu_ms);
        if self.demo && !self.busy_pause.demo_released && demo_from().is_some_and(|from| self.frame_count >= from) {
            // (a new measure each frame)
            let ms = SLOW_MS * 2.0 + (self.frame_count % 2) as f32;
            (behind, gpu_ms, timed) = (true, Some(ms), false);
        }
        let allowed = self.settings.background_gpu_pause && !timed && !vr && !self.snapshots.drawing();
        if self.busy_pause.update(behind, allowed, gpu_ms) {
            if self.busy_pause.paused() {
                log::warn!(
                    "render paused behind: the GPU frames take {:.0} ms, the card is busy with another program",
                    gpu_ms.unwrap_or_default()
                );
            } else {
                log::info!("render resumed");
            }
        }
    }

    /// The notice over the paused world.
    pub(super) fn busy_pause_notice(&self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.busy_pause.paused() {
            return;
        }
        // The paused world, blurred and darkened (the modals' blur, ui_blur.rs:
        // the scene under the boundary, what is painted after it sharp), then
        // the notice in amber; both under the windows, so the chat watched
        // on a second screen stays whole. Asked by the human (2026-11-05).
        // (one layer: the boundary and the shade first, the notice's area
        // painted after them in it, sharp)
        let id = egui::Id::unique("busy-pause-notice");
        let layer = egui::LayerId::new(egui::Order::Background, id);
        aurora_render::blurred_backdrop(ctx, layer);
        ctx.layer_painter(layer)
            .rect_filled(ctx.content_rect(), 0.0, p.navy.gamma_multiply(0.55));
        let room = crate::ui::layout::work_area(ctx);
        egui::Area::new(id)
            .order(egui::Order::Background)
            .pivot(egui::Align2::CENTER_CENTER)
            .fixed_pos(room.center())
            .interactable(false)
            .show(ctx, |ui| {
                crate::ui::widgets::overlay(ui, p, egui::Margin::symmetric(16, 10), |ui| {
                    let text = if self.busy_pause.forced {
                        aurora_i18n::tr!("busy-pause-forced-notice")
                    } else {
                        aurora_i18n::tr!("busy-pause-notice")
                    };
                    ui.label(egui::RichText::new(text).color(p.amber));
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_slow_frames_behind_pause_until_the_focus_comes_back() {
        let mut b = BusyPause::default();
        assert!(!b.update(true, true, Some(400.0)));
        assert!(!b.paused(), "one slow frame is not a busy card");
        assert!(!b.update(true, true, Some(400.0)), "nor the same measure given again");
        assert!(b.update(true, true, Some(410.0)));
        assert!(b.paused());
        // its own frames are short while paused: it stays paused
        assert!(!b.update(true, true, Some(0.3)));
        assert!(!b.update(true, true, None));
        assert!(b.paused());
        assert!(b.update(false, true, Some(0.3)), "in front: drawn again");
        assert!(!b.paused());
    }

    #[test]
    fn a_fast_frame_between_two_slow_ones_starts_the_count_again() {
        let mut b = BusyPause::default();
        b.update(true, true, Some(SLOW_MS));
        b.update(true, true, Some(20.0));
        b.update(true, true, Some(SLOW_MS + 1.0));
        assert!(!b.paused());
        // no new measure leaves the count as it was
        b.update(true, true, None);
        b.update(true, true, Some(SLOW_MS + 1.0));
        assert!(!b.paused());
        b.update(true, true, Some(SLOW_MS + 2.0));
        assert!(b.paused());
    }

    #[test]
    fn in_front_or_not_allowed_never_pauses() {
        let mut b = BusyPause::default();
        for _ in 0..5 {
            b.update(false, true, Some(900.0));
        }
        assert!(!b.paused(), "slow frames in front are the user's own scene");
        for _ in 0..5 {
            b.update(true, false, Some(900.0));
        }
        assert!(!b.paused(), "setting off, a photo, a headset or a timed run");
        b.update(true, true, Some(901.0));
        b.update(true, true, Some(902.0));
        assert!(b.paused());
        assert!(b.update(true, false, None), "a photo started behind: drawn again");
    }
}
