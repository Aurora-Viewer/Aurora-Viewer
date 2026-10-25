//! The login screen's warning about an out of date graphics driver
//! (`gpu_driver.rs`): a card in the top left corner, edged in the caution
//! colour, with the vendor's download page and a way to leave it be for
//! this driver.
use crate::gpu_driver::{Notice, Reason};
use crate::theme::Palette;
use crate::ui::{
    icons::Icons,
    widgets::{self, ActionKind},
};
use egui::{Context, Id, RichText, Sense, Vec2};

/// Width of the login screen's warning cards (this one, `safe_graphics_notice.rs`);
/// less in a narrow window, where they keep clear of the logo in the middle.
const WIDTH: f32 = 380.0;

/// The top left corner's warnings, one under the other; the closure draws
/// them, at the width it is given.
pub fn column<R>(ctx: &Context, body: impl FnOnce(&mut egui::Ui, f32) -> R) -> R {
    let width = WIDTH.min(ctx.content_rect().width() / 2.0 - 120.0).max(260.0);
    egui::Area::new(Id::unique("login_warnings"))
        .anchor(egui::Align2::LEFT_TOP, Vec2::new(25.0, 20.0))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 12.0;
            body(ui, width)
        })
        .inner
}

/// Draws the card; true when the user dismissed it.
pub fn card(ui: &mut egui::Ui, p: &Palette, icons: &Icons, notice: &Notice, width: f32) -> bool {
    let mut dismissed = false;
    super::card_caution(ui, p, 16, |ui| {
        ui.set_width(width - 32.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            if let Some(icon) = icons.get("alert-triangle-filled") {
                egui::Image::new(icon).tint(p.amber).paint_at(ui, r);
            }
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("login-driver-old-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                )
                .wrap(),
            );
        });
        ui.add_space(6.0);
        let detail = match &notice.reason {
            Reason::Version { found, minimum } => aurora_i18n::tr!(
                "login-driver-old-version",
                gpu = notice.gpu.clone(),
                found = found.clone(),
                minimum = minimum.clone()
            ),
            Reason::Age { date, .. } => aurora_i18n::tr!(
                "login-driver-old-age",
                gpu = notice.gpu.clone(),
                date = format!("{}-{:02}-{:02}", date.0, date.1, date.2)
            ),
        };
        ui.add(egui::Label::new(RichText::new(detail).size(12.5).color(p.ink)).wrap());
        ui.add_space(2.0);
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("login-driver-old-why")).size(12.5).color(p.muted)).wrap());
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let size = Vec2::new(0.0, 30.0);
            let download = aurora_i18n::tr!("login-driver-download", vendor = notice.vendor.name().to_string());
            if widgets::action_button(ui, p, "download-filled", &download, ActionKind::Caution, size)
                .on_hover_text(notice.vendor.download_url())
                .clicked()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(notice.vendor.download_url()));
            }
            if widgets::action_button(ui, p, "eye-off", aurora_i18n::tr!("login-driver-dismiss"), ActionKind::Flat, size).clicked() {
                dismissed = true;
            }
        });
    });
    dismissed
}
