//! A blocked resident's lines in the nearby chat history, when Preferences ›
//! Chat keeps them (`settings::BlockedChat`, the Legacy Viewer's
//! FSShowMutedChatHistory): the "n messages hidden · Show" row that stands
//! for a folded run (`view::blocked_rows`), and the greying of the lines
//! drawn.

use crate::theme::Palette;
use crate::world::ChatLine;
use egui::{RichText, Vec2};

/// How much of its colour a blocked resident's line keeps, drawn in the
/// history; its text is in the blocked colour already
/// (`colors::chat_color`).
const GREYED: f32 = 0.55;

/// The opacity a line is drawn with in the history.
pub(super) fn alpha(line: &ChatLine) -> f32 {
    if line.blocked { GREYED } else { 1.0 }
}

/// The row of a folded run of `count` lines, under the day's marker when
/// it opens a day. True when Show is clicked.
pub(super) fn fold_row(ui: &mut egui::Ui, p: &Palette, look: super::view::Look, count: usize) -> bool {
    if let Some((year, month, day)) = look.marker {
        super::date_marker(ui, p, year, month, day);
    }
    let size = super::super::fonts::chat_size(ui.ctx(), 12.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
        super::thread::paint_icon(ui.painter(), "eye-off", r, p.muted_dim);
        ui.label(
            RichText::new(format!("{} ·", aurora_i18n::tr!("ui-chat-blocked-hidden", count = count)))
                .size(size)
                .color(p.muted),
        );
        let show = ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("ui-chat-preview-show"))
                    .size(size)
                    .color(p.violet_light),
            )
            .sense(egui::Sense::click()),
        );
        show.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(aurora_i18n::tr!("ui-chat-blocked-show"))
            .clicked()
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{ChatKind, Origin};

    #[test]
    fn only_a_blocked_line_is_greyed() {
        let mut line = ChatLine {
            time: std::time::SystemTime::now(),
            from: "Violet Wolf".into(),
            text: "hello".into(),
            kind: ChatKind::Local(aurora_net::ChatType::Normal),
            source: uuid::Uuid::from_u128(7),
            origin: Origin::Live,
            blocked: false,
            beyond_range: false,
            speaker: Default::default(),
        };
        assert_eq!(alpha(&line), 1.0);
        line.blocked = true;
        assert!(alpha(&line) < 1.0);
    }
}
