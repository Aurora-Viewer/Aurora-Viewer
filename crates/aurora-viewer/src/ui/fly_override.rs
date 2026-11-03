//! The fly override's first-use caution: the Legacy Viewer's
//! FirstUseFlyOverride notification (skins/default/xui/en/notifications.xml,
//! raised by LLAgent::updateFSAlwaysFly in indra/newview/llagent.cpp), an
//! alert with a single OK, shown once ever.

use crate::theme::Palette;
use egui::{Id, Vec2};

/// True once the caution is dismissed (OK or Escape).
pub fn caution(ctx: &egui::Context, p: &Palette) -> bool {
    let mut done = false;
    let width = (ctx.content_rect().width() - 80.0).clamp(240.0, 400.0);
    let response = super::widgets::Modal::new(Id::unique("fly_override_caution")).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
        ui.horizontal(|ui| {
            super::widgets::glyph(ui, "plane-departure-filled", p.violet_light, 20.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-bars-fly-override"))
                    .size(17.0)
                    .strong()
                    .color(p.ink),
            );
        });
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-bars-fly-override-caution"))
                .size(13.0)
                .color(p.muted),
        );
        ui.add_space(4.0);
        if super::widgets::action_button(
            ui,
            p,
            "check",
            aurora_i18n::tr!("common-ok"),
            super::widgets::ActionKind::Primary,
            Vec2::new(width, 32.0),
        )
        .clicked()
        {
            done = true;
        }
    });
    if !done && response.is_top_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        done = true;
    }
    done
}
