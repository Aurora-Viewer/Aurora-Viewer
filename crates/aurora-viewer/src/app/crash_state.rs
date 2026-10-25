//! What a crash report says the viewer was doing (crash/watchdog.rs): every
//! two seconds a few numbers already measured for the status bar and the
//! texture budget, and fixed names. A few atomics and one video memory
//! query, nothing measured for it alone.

use super::*;

const EVERY: Duration = Duration::from_secs(2);

impl App {
    pub(super) fn crash_state(&mut self, renderer: &aurora_render::Renderer, window: &winit::window::Window) {
        let now = Instant::now();
        if self.crash_state_at.is_some_and(|at| now.duration_since(at) < EVERY) {
            return;
        }
        self.crash_state_at = Some(now);
        let video = renderer.video_memory.read(&renderer.device);
        let size = window.inner_size();
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
        });
    }
}
