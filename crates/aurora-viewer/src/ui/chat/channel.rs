//! The nearby-chat channel selector shared by the Conversations, Legacy
//! Viewer-style chat window and bottom-bar input fields. As the Legacy
//! Viewer's `ChatChannel` spinner (floater_fs_nearby_chat.xml, FSShowChatChannel),
//! its channel stays outside the text and is kept from one message to the
//! next; a `/n` typed before the text still wins (FSFloaterNearbyChat::sendChat).

use super::parse_channel;
use crate::theme::Palette;
use crate::ui::menu;
use egui::RichText;

/// Keep a 4-point gap and the 30-point selector inside the input row.
pub(super) const RESERVED_WIDTH: f32 = 34.0;

/// A click opens the channel's number; `channel` is the selector's own
/// value, shared by every nearby chat field. Returns whether its menu is open.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, channel: &mut i32, id: egui::Id) -> bool {
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
        if *channel == 0 { p.muted } else { p.amber },
    );
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-chat-channel-tip"));
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(170.0);
        ui.label(RichText::new(aurora_i18n::tr!("ui-chat-channel")).size(11.5).color(p.muted));
        ui.add(egui::DragValue::new(channel).range(-2_147_483_648..=2_147_483_647).speed(0.2));
        if *channel != 0 && menu::item(ui, p, "x", aurora_i18n::tr!("ui-chat-channel-reset")) {
            *channel = 0;
            ui.close();
        }
    })
}

/// The channel and message of a nearby line, read after the OOC wrap as the
/// Legacy Viewer does: a `/n` opening the text, else the selector's channel
/// when it is shown (`selector`), else 0.
pub(super) fn resolve(text: &str, selector: Option<i32>) -> (i32, String) {
    let (channel, message) = parse_channel(text);
    // parse_channel gives the text back unchanged when it has no `/n`
    match selector {
        Some(chosen) if message == text => (chosen, message),
        _ => (channel, message),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn the_selector_channel_applies_unless_the_text_names_one() {
        assert_eq!(resolve("hi", Some(5)), (5, "hi".into()));
        assert_eq!(resolve("/7 hi", Some(5)), (7, "hi".into()));
        // "/0" typed is a channel set, as stripChannelNumber's is_set
        assert_eq!(resolve("/0 hi", Some(5)), (0, "hi".into()));
        assert_eq!(resolve("/me waves", Some(5)), (5, "/me waves".into()));
        // hidden selector: the text alone decides
        assert_eq!(resolve("hi", None), (0, "hi".into()));
        // Alt+Enter wraps first, so the selector's channel carries the OOC line
        assert_eq!(resolve(&super::super::ooc("brb"), Some(5)), (5, "(( brb ))".into()));
    }
}
