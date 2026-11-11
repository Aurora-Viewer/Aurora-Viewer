//! Shared texture/material eyedropper input, independent of the opening window.

use super::App;
use crate::ui::texture_picker::pipette;

impl App {
    pub(super) fn texture_pipette(&self) -> bool {
        pipette::armed(&self.egui_ctx)
    }

    pub(super) fn sample_texture(&mut self) {
        let Some(materials) = pipette::needs_sample(&self.egui_ctx, self.cursor_pos) else {
            return;
        };
        while self.color_pipette() {
            self.release_color_pipette();
        }
        let hud = self
            .settings
            .show_huds
            .then_some(self.scene.lists.hud_view)
            .flatten()
            .and_then(|view| {
                self.scene
                    .pipette_face_at_ray(&self.world, view.ray(self.cursor_pos.0, self.cursor_pos.1), true, f32::INFINITY)
            });
        let face = hud.or_else(|| {
            self.gfx
                .as_ref()
                .and_then(|gfx| gfx.renderer.cursor_ray(self.cursor_pos.0, self.cursor_pos.1))
                .and_then(|ray| self.scene.pipette_face_at_ray(&self.world, ray, false, self.settings.draw_distance))
        });
        let hit = face.and_then(|(idx, face)| {
            let object = self.world.objects.get(idx)?;
            let asset = if materials {
                // The base asset only, never per-face PBR overrides.
                object.visual_material(face).unwrap_or_default()
            } else {
                object.visual_texture_face(face).texture
            };
            Some((object.full_id, face, asset))
        });
        pipette::sample(&self.egui_ctx, hit);
    }
}
