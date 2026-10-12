//! LLToolPipette-style transient input for the construction color dialog.

use super::App;

impl App {
    pub(super) fn color_pipette(&self) -> bool {
        self.build.ui.color_picker.pipette || self.script_editors.color_pipette()
    }

    pub(super) fn active_color_picker(&mut self) -> &mut crate::ui::color_picker::ColorPicker {
        if self.script_editors.color_pipette()
            && let Some(picker) = self.script_editors.color_picker()
        {
            picker
        } else {
            &mut self.build.ui.color_picker
        }
    }

    pub(super) fn sample_build_color(&mut self) {
        let cursor = self.cursor_pos;
        if !self.active_color_picker().needs_sample(cursor) {
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
        self.active_color_picker().sample(color);
    }
}
