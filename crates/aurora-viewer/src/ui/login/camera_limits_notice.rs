//! The login screen's card after version 33 of the settings turned the
//! camera's limits back on (`Settings::camera_limits_restored`): the
//! camera keeps within the draw distance and in front of walls again, as
//! with the Legacy Viewer's defaults, and the card says where to turn that
//! off. Shown once, like `photo_quality_notice.rs`, on a plain card: nothing
//! went wrong.
use crate::theme::Palette;
use crate::ui::{
    icons::Icons,
    widgets::{self, ActionKind},
};
use egui::{RichText, Sense, Vec2};

/// Draws the card; `Some(true)` to open the camera preferences,
/// `Some(false)` when it was only read.
pub fn card(ui: &mut egui::Ui, p: &Palette, icons: &Icons, width: f32) -> Option<bool> {
    let mut answer = None;
    super::card(ui, p, 16, false, |ui| {
        ui.set_width(width - 32.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            if let Some(icon) = icons.get("camera-filled") {
                egui::Image::new(icon).tint(p.violet_light).paint_at(ui, r);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("login-camera-limits-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                )
                .wrap(),
            );
        });
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("login-camera-limits")).size(12.5).color(p.ink)).wrap());
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let size = Vec2::new(0.0, 30.0);
            if widgets::action_button(ui, p, "check", aurora_i18n::tr!("login-camera-limits-ok"), ActionKind::Flat, size).clicked() {
                answer = Some(false);
            }
            let open = aurora_i18n::tr!("login-camera-limits-preferences");
            if widgets::action_button(ui, p, "camera-filled", open, ActionKind::Flat, size).clicked() {
                answer = Some(true);
            }
        });
    });
    answer
}
