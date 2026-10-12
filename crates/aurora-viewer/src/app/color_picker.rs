//! LLToolPipette-style transient input for the construction color dialog.

use super::App;

impl App {
    pub(super) fn sample_build_color(&mut self) {
        if !self.build.ui.color_picker.needs_sample(self.cursor_pos) {
            return;
        }
        let hud = self
            .settings
            .show_huds
            .then_some(self.scene.lists.hud_view)
            .flatten()
            .and_then(|view| {
                self.scene
                    .color_at_ray(&self.world, view.ray(self.cursor_pos.0, self.cursor_pos.1), true, f32::INFINITY)
            });
        let color = hud.or_else(|| {
            self.gfx
                .as_ref()
                .and_then(|gfx| gfx.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1))
                .and_then(|ray| self.scene.color_at_ray(&self.world, ray, false, self.settings.draw_distance))
        });
        self.build.ui.color_picker.sample(color);
    }
}
