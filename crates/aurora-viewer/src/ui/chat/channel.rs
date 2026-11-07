//! The nearby-chat channel selector shared by the Conversations, Legacy
//! Viewer-style chat window and bottom-bar input fields.

use super::parse_channel;
use crate::theme::Palette;
use crate::ui::menu;
use egui::RichText;

/// Keep a 4-point gap and the 30-point selector inside the input row.
pub(super) const RESERVED_WIDTH: f32 = 34.0;

/// A click opens the current channel; editing it rewrites the draft's `/n`.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, draft: &mut String, id: egui::Id) {
    let (channel, _) = parse_channel(draft);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 22.0), egui::Sense::hover());
    let resp = ui.interact(rect, id, egui::Sense::click());
    ui.painter().rect_filled(
        rect,
        ui.visuals().widgets.inactive.corner_radius,
        if resp.hovered() { p.field.gamma_multiply(1.3) } else { p.field },
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("/{channel}"),
        egui::FontId::proportional(11.0),
        if channel == 0 { p.muted } else { p.amber },
    );
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-chat-channel-tip"));
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(170.0);
        ui.label(RichText::new(aurora_i18n::tr!("ui-chat-channel")).size(11.5).color(p.muted));
        let mut n = channel;
        ui.add(egui::DragValue::new(&mut n).range(-2_147_483_648..=2_147_483_647).speed(0.2));
        if n != channel {
            let (_, message) = parse_channel(draft);
            *draft = if n == 0 { message } else { format!("/{n} {message}") };
        }
        if channel != 0 && menu::item(ui, p, "x", aurora_i18n::tr!("ui-chat-channel-reset")) {
            let (_, message) = parse_channel(draft);
            *draft = message;
            ui.close();
        }
    });
}
