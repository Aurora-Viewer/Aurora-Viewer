//! LLToolPipette-style transient input for the construction color dialog.

use super::App;
use crate::ui::widgets::color::pipette as shared;

impl App {
    /// An eyedropper is armed: the one of the palette dialog (construction,
    /// script editors) or the one the compact pickers share.
    pub(super) fn color_pipette(&self) -> bool {
        self.build.ui.color_picker.pipette || self.script_editors.color_pipette() || shared::armed(&self.egui_ctx)
    }

    /// The mouse button that sampled is released: the eyedropper is put down.
    pub(super) fn release_color_pipette(&mut self) {
        if shared::armed(&self.egui_ctx) {
            shared::release(&self.egui_ctx);
        } else {
            let picker = self.active_color_picker();
            picker.pipette = false;
            picker.sampled = None;
        }
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
        let compact = shared::armed(&self.egui_ctx);
        let fresh = if compact {
            shared::needs_sample(&self.egui_ctx, cursor)
        } else {
            self.active_color_picker().needs_sample(cursor)
        };
        if !fresh {
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
        if compact {
            shared::sample(&self.egui_ctx, color.map(|(_, rgb)| rgb));
        } else {
            self.active_color_picker().sample(color);
        }
    }
}
