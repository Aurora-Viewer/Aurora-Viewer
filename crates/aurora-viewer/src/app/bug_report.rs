//! Snapshot only the explicitly supported public fields, never Settings or
//! World wholesale (both contain private state). Background work lives in
//! bug_report::files; the frame just hands it these small display strings.

use super::*;
use crate::bug_report::Field;

impl App {
    pub(super) fn open_bug_report(&mut self, renderer: &Renderer) {
        let gpu = &renderer.info;
        let s = &self.settings;
        // What paces the frames besides the scene (issue #120): the user
        // cap, the mode the swapchain presents with and the current rate of
        // the screen the window is on.
        let frame_limit = if s.fps_cap {
            format!("{} FPS", s.fps_limit)
        } else {
            "false".into()
        };
        let present_mode = present_mode_name(renderer.present_mode());
        let screen_rate = format!("{:.0} Hz", self.screen_rate.hz());
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
                    "Draw distance: {} m; LOD: {}; shadows: {} (quality {}); antialiasing: {}; SSAO: {}; reflections: {}; texture budget: {} MiB; max avatars: {}; vsync: {}; frame limit: {}; present mode: {present_mode}; screen: {screen_rate}",
                    s.draw_distance,
                    s.lod_factor,
                    s.shadows,
                    s.shadow_quality,
                    s.antialiasing,
                    s.ssao,
                    s.water_reflections,
                    s.texture_budget(gpu.vram_mb, gpu.integrated),
                    s.max_avatars,
                    s.vsync,
                    if s.fps_cap { frame_limit.as_str() } else { "off" }
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
        if let Some(graphics) = fields.iter_mut().find(|field| field.label == "bug-graphics") {
            graphics.details = vec![
                ("common-draw-distance", format!("{} m", s.draw_distance)),
                ("bug-lod", s.lod_factor.to_string()),
                ("bug-shadows", s.shadows.to_string()),
                ("bug-shadow-quality", s.shadow_quality.to_string()),
                ("bug-antialiasing", s.antialiasing.to_string()),
                ("bug-ssao", s.ssao.to_string()),
                ("bug-reflections", s.water_reflections.to_string()),
                (
                    "bug-texture-budget",
                    format!("{} MiB", s.texture_budget(gpu.vram_mb, gpu.integrated)),
                ),
                ("bug-max-avatars", s.max_avatars.to_string()),
                ("bug-vsync", s.vsync.to_string()),
                ("bug-frame-limit", frame_limit),
                ("bug-present-mode", present_mode.to_string()),
                ("bug-screen-rate", screen_rate),
            ];
        }
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
