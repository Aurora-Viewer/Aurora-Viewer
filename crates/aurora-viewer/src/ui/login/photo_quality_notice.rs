//! The login screen's card after the last session crashed or was stopped
//! with the photo window's "Photo quality" on (`app/snapshot.rs`,
//! `photo_quality_at_start`): the switch is off now, and the card says so,
//! with a way to turn it back on. Same caution card as `safe_graphics_notice.rs`.
use crate::theme::Palette;
use crate::ui::{
    icons::Icons,
    widgets::{self, ActionKind},
};
use egui::{RichText, Sense, Vec2};

/// Draws the card; `Some(true)` to turn the photo quality back on,
/// `Some(false)` when it was only read.
pub fn card(ui: &mut egui::Ui, p: &Palette, icons: &Icons, width: f32) -> Option<bool> {
    let mut answer = None;
    super::card_caution(ui, p, 16, |ui| {
        ui.set_width(width - 32.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            if let Some(icon) = icons.get("alert-triangle-filled") {
                egui::Image::new(icon).tint(p.amber).paint_at(ui, r);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("login-photo-quality-off-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                )
                .wrap(),
            );
        });
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("login-photo-quality-off")).size(12.5).color(p.ink)).wrap());
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let size = Vec2::new(0.0, 30.0);
            if widgets::action_button(
                ui,
                p,
                "check",
                aurora_i18n::tr!("login-photo-quality-off-ok"),
                ActionKind::Caution,
                size,
            )
            .clicked()
            {
                answer = Some(false);
            }
            let back_on = aurora_i18n::tr!("login-photo-quality-off-back-on");
            if widgets::action_button(ui, p, "sparkles-filled", back_on, ActionKind::Flat, size).clicked() {
                answer = Some(true);
            }
        });
    });
    answer
}
