//! Snapshot only the explicitly supported public fields, never Settings or
//! World wholesale (both contain private state). Background work lives in
//! bug_report::files; the frame just hands it these small display strings.

use super::*;
use crate::bug_report::Field;

impl App {
    pub(super) fn open_bug_report(&mut self, renderer: &Renderer) {
        let gpu = &renderer.info;
        let s = &self.settings;
        let mut fields = vec![
            Field::new("bug-version", "Viewer version", aurora_version::version(), true),
            Field::new("bug-commit", "Build commit", aurora_version::commit(), true),
            Field::new("bug-build", "Build profile", env!("AURORA_BUILD_PROFILE"), true),
            Field::new("bug-gpu", "Graphics card", format!("{} ({})", gpu.name, gpu.backend), true),
            Field::new("bug-driver", "Graphics driver", &gpu.driver, true),
            Field::new(
                "bug-vram",
                "Video memory",
                gpu.vram_mb.map(|mb| format!("{mb} MiB")).unwrap_or_else(|| "Unavailable".into()),
                true,
            ),
            Field::new(
                "bug-resolution",
                "Window resolution",
                format!(
                    "{} × {}",
                    (self.egui_ctx.content_rect().width() * self.egui_ctx.pixels_per_point()) as u32,
                    (self.egui_ctx.content_rect().height() * self.egui_ctx.pixels_per_point()) as u32
                ),
                true,
            ),
            Field::new(
                "bug-graphics",
                "Graphics settings",
                format!(
                    "Draw distance: {} m; LOD: {}; shadows: {} (quality {}); antialiasing: {}; SSAO: {}; reflections: {}; texture budget: {} MiB; max avatars: {}; vsync: {}",
                    s.draw_distance,
                    s.lod_factor,
                    s.shadows,
                    s.shadow_quality,
                    s.antialiasing,
                    s.ssao,
                    s.water_reflections,
                    s.texture_budget(gpu.vram_mb, gpu.integrated),
                    s.max_avatars,
                    s.vsync
                ),
                true,
            ),
            Field::new(
                "bug-performance",
                "Frame timing",
                format!("{:.1} FPS; {:.2} ms/frame", self.perf.fps, self.perf.frames.avg),
                true,
            ),
            Field::new("bug-language", "Viewer language", aurora_i18n::language(), true),
            Field::new(
                "bug-duration",
                "Session duration",
                format!("{} s", self.start.elapsed().as_secs()),
                true,
            ),
        ];
        if matches!(self.screen, Screen::World) {
            fields.insert(
                3,
                Field::new(
                    "bug-location",
                    "Location",
                    crate::slurl::make(&self.world.region_name(), self.world.agent.position),
                    true,
                ),
            );
            if let Some(name) = self.world.social.avatar_names.get(&self.world.agent_id) {
                fields.push(Field::new("bug-avatar", "Avatar name", &name.display_name, false));
            }
        }
        self.bug_report.begin(fields, self.demo);
    }
}
