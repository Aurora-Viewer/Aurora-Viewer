//! What a crash report says the viewer was doing (crash/watchdog.rs): every
//! two seconds a few numbers already measured for the status bar and the
//! texture budget, and fixed names. A few atomics and one video memory
//! query, nothing measured for it alone.

use std::collections::VecDeque;

use super::*;

const EVERY: Duration = Duration::from_secs(2);

/// The span the swapchain recreations are counted over.
const SWAPCHAIN_SPAN: Duration = Duration::from_secs(10);

/// Swapchain recreations in the last [`SWAPCHAIN_SPAN`], from the running
/// total read at each note. Frozen at the first note after the GPU was
/// lost: a report then tells whether the window was being resized just
/// before the loss (#1701), not in the seconds the viewer took to close.
#[derive(Debug, Default)]
pub(super) struct SwapchainRecent {
    /// (when, running total), oldest first, over the span.
    samples: VecDeque<(Instant, u64)>,
    frozen: Option<u64>,
}

impl SwapchainRecent {
    fn note(&mut self, now: Instant, total: u64, device_lost: bool) -> u64 {
        if let Some(count) = self.frozen {
            return count;
        }
        while self.samples.len() > 1 && self.samples.get(1).is_some_and(|(at, _)| now.duration_since(*at) >= SWAPCHAIN_SPAN) {
            self.samples.pop_front();
        }
        let count = self.samples.front().map_or(0, |(_, oldest)| total.saturating_sub(*oldest));
        self.samples.push_back((now, total));
        if device_lost {
            self.frozen = Some(count);
        }
        count
    }
}

impl App {
    pub(super) fn crash_state(&mut self, renderer: &aurora_render::Renderer, window: &winit::window::Window) {
        let now = Instant::now();
        if self.crash_state_at.is_some_and(|at| now.duration_since(at) < EVERY) {
            return;
        }
        self.crash_state_at = Some(now);
        let video = renderer.video_memory.read(&renderer.device);
        let size = window.inner_size();
        let swapchain_10s = self
            .crash_swapchain
            .note(now, aurora_render::present::recreations(), renderer.device_lost());
        crate::crash::note_state(crate::crash::State {
            screen: match self.screen {
                Screen::Login => "login",
                Screen::Loading { .. } => "loading",
                Screen::World => "world",
            },
            fps: self.perf.fps,
            frame_ms: self.perf.frames.avg,
            window: (size.width, size.height),
            preset: match crate::graphics_presets::Preset::matching(&self.settings) {
                Some(crate::graphics_presets::Preset::Potato) => "potato",
                Some(crate::graphics_presets::Preset::Low) => "low",
                Some(crate::graphics_presets::Preset::Medium) => "medium",
                Some(crate::graphics_presets::Preset::High) => "high",
                Some(crate::graphics_presets::Preset::Deluxe) => "deluxe",
                None => "custom",
            },
            vram_used_mb: video.map(|v| v.usage >> 20),
            vram_budget_mb: video.map(|v| v.budget >> 20),
            texture_pressure: (
                u32::from(self.scene.textures.stats.pressure),
                u32::from(crate::scene::texture_budget::MAX_PRESSURE),
            ),
            objects: self.scene.stats.objects,
            avatars: self.scene.stats.avatars.total,
            regions: self.world.regions.len(),
            swapchain_10s,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn swapchain_recreations_are_counted_over_the_last_ten_seconds() {
        let mut recent = SwapchainRecent::default();
        let t = Instant::now();
        // the first note has nothing to compare with
        assert_eq!(recent.note(t, 3, false), 0);
        // a drag: 20 recreations a second, noted every two seconds
        assert_eq!(recent.note(t + s(2), 43, false), 40);
        assert_eq!(recent.note(t + s(4), 83, false), 80);
        assert_eq!(recent.note(t + s(10), 203, false), 200);
        // past ten seconds the oldest notes drop out
        assert_eq!(recent.note(t + s(12), 243, false), 200);
        // the drag ends: back to nothing within ten seconds
        assert_eq!(recent.note(t + s(14), 250, false), 167);
        assert_eq!(recent.note(t + s(24), 250, false), 0);
    }

    #[test]
    fn the_count_stays_as_it_was_when_the_gpu_was_lost() {
        let mut recent = SwapchainRecent::default();
        let t = Instant::now();
        assert_eq!(recent.note(t, 0, false), 0);
        assert_eq!(recent.note(t + s(2), 30, false), 30);
        // lost: this note still counts what came before the loss
        assert_eq!(recent.note(t + s(4), 70, true), 70);
        // the viewer closing after it does not change the report
        assert_eq!(recent.note(t + s(20), 71, true), 70);
        assert_eq!(recent.note(t + s(30), 71, false), 70);
    }
}
