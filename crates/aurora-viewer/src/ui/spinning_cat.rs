//! What the spinning cat (`crate::oiia`) shows of itself: the warning about
//! flashing lights that comes before it, the note that stands for it when
//! its video cannot play, and the Stop button while it plays.

use super::widgets::{self, ActionKind};
use crate::theme::Palette;
use aurora_i18n::tr;
use egui::{Id, Vec2};

/// The warning: Some(true) to start the show, Some(false) to leave. Cancel
/// comes first and is what Escape answers: nothing starts by accident. It
/// says the music plays from YouTube, since starting reaches that site.
pub fn warning(ctx: &egui::Context, p: &Palette) -> Option<bool> {
    let mut answer = None;
    let width = (ctx.content_rect().width() - 80.0).clamp(260.0, 460.0);
    let response = widgets::Modal::new(Id::unique("spinning_cat_warning")).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
        ui.horizontal(|ui| {
            widgets::glyph(ui, "alert-triangle-filled", p.warn, 22.0);
            ui.label(
                egui::RichText::new(tr!("ui-fun-cat-warning-title"))
                    .size(17.0)
                    .strong()
                    .color(p.ink),
            );
        });
        ui.label(egui::RichText::new(tr!("ui-fun-cat-warning-body")).size(13.0).color(p.muted));
        ui.label(egui::RichText::new(tr!("ui-fun-cat-warning-calm")).size(13.0).color(p.muted));
        ui.label(egui::RichText::new(tr!("ui-fun-cat-warning-video")).size(13.0).color(p.muted));
        ui.label(egui::RichText::new(tr!("ui-fun-cat-warning-ask")).size(13.0).strong().color(p.ink));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let size = Vec2::new((width - 8.0) / 2.0, 32.0);
            if widgets::action_button(ui, p, "x", tr!("common-cancel"), ActionKind::Primary, size).clicked() {
                answer = Some(false);
            }
            if widgets::action_button(
                ui,
                p,
                "alert-triangle-filled",
                tr!("ui-fun-cat-warning-confirm"),
                ActionKind::Caution,
                size,
            )
            .clicked()
            {
                answer = Some(true);
            }
        });
    });
    if answer.is_none() && response.is_top_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        answer = Some(false);
    }
    answer
}

/// The show's video cannot play here (no media plugins, or YouTube's
/// previews hidden): the code brings this note instead. True once closed.
pub fn unavailable(ctx: &egui::Context, p: &Palette) -> bool {
    let mut closed = false;
    let width = (ctx.content_rect().width() - 80.0).clamp(260.0, 420.0);
    let response = widgets::Modal::new(Id::unique("spinning_cat_unavailable")).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
        ui.label(egui::RichText::new(tr!("ui-fun-cat-unavailable")).size(13.0).color(p.ink));
        ui.add_space(4.0);
        closed = widgets::action_button(ui, p, "x", tr!("common-close"), ActionKind::Primary, Vec2::new(width, 32.0)).clicked();
    });
    closed || response.is_top_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
}

/// The Stop button, at the bottom of the view while the show plays. True
/// when pressed.
pub fn stop(ctx: &egui::Context, p: &Palette) -> bool {
    let room = super::layout::work_area(ctx);
    let mut pressed = false;
    egui::Area::new(Id::unique("spinning_cat_stop"))
        .anchor(
            egui::Align2::CENTER_BOTTOM,
            Vec2::new(0.0, room.bottom() - ctx.content_rect().bottom() - 16.0),
        )
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                pressed = widgets::action_button(
                    ui,
                    p,
                    "player-stop-filled",
                    tr!("common-stop"),
                    ActionKind::Danger,
                    Vec2::new(0.0, 30.0),
                )
                .on_hover_text(tr!("ui-fun-cat-stop-hint"))
                .clicked();
            });
        });
    pressed
}
