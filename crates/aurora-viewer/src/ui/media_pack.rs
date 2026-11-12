//! The card that asks before Aurora's media plugins pack is downloaded
//! (`media/pack.rs`): shown once, the first time web pages or media need
//! the plugins, over the login screen or the world. "Not now" leaves the
//! Download buttons of Preferences › Sound and voice › Media, of the pages
//! and of the media controls (`web_view.rs`). The same dialog as the
//! update's (`update.rs`), in both interface styles. The Legacy Viewer
//! ships its plugins inside its installer and has no such card.

use super::icons::Icons;
use super::widgets::{self, flat_button_sized, primary_button_sized};
use crate::theme::Palette;
use egui::{RichText, Sense, Vec2};

const ID: &str = "media_pack_consent";

/// Draws the card; `Some(true)` for Download, `Some(false)` for Not now
/// (Escape and a click beside the card count as Not now: nothing is
/// downloaded without a yes).
pub fn card(ctx: &egui::Context, p: &Palette, icons: &Icons) -> Option<bool> {
    let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 480.0);
    let mut answer = None;
    aurora_render::blurred_backdrop(ctx, egui::Modal::default_area(egui::Id::unique(ID)).layer());
    let response = widgets::Modal::new(egui::Id::unique(ID)).transparent_backdrop().show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(10.0, 9.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(48.0), Sense::hover());
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
            if let Some(texture) = icons.get("download-filled") {
                egui::Image::new(texture).tint(p.violet_light).paint_at(ui, rect.shrink(10.0));
            }
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("media-plugins-consent-title"))
                        .size(19.0)
                        .strong()
                        .color(p.ink),
                )
                .wrap(),
            );
        });
        let size = super::web_view::pack_megabytes();
        ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("media-plugins-consent", size = size))
                    .size(13.0)
                    .color(p.ink),
            )
            .wrap(),
        );
        ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("media-plugins-consent-later-hint"))
                    .size(12.0)
                    .color(p.muted),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let size = Vec2::new((width - 10.0) / 2.0, 34.0);
            if flat_button_sized(ui, p, aurora_i18n::tr!("media-plugins-consent-later"), size).clicked() {
                answer = Some(false);
            }
            if primary_button_sized(ui, p, aurora_i18n::tr!("media-plugins-download"), size).clicked() {
                answer = Some(true);
            }
        });
    });
    if answer.is_none() && response.should_close() {
        answer = Some(false);
    }
    answer
}
