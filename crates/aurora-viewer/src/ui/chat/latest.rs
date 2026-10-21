//! The button back to the latest messages (#558): it rises over the bottom
//! of a conversation once the reader has scrolled away from its end, says
//! how many messages came in since, and brings the end back in sight. The
//! Legacy Viewer has none: its history only follows the end while it is
//! there (FSChatHistory::appendMessage, mScrollToBottom).

use crate::theme::Palette;
use crate::ui::widgets;

/// Further from the end than this share of the height in sight, the button
/// shows even when nothing came in.
const AWAY: f32 = 0.5;
const HEIGHT: f32 = 28.0;
/// Its gap above the bottom of the lines.
const MARGIN: f32 = 10.0;
/// How far it rises while it fades in.
const RISE: f32 = 6.0;

/// The button over the bottom of `area`, the lines in sight, when the end is
/// `below` points under them or `arrived` messages came in since the reader
/// left it. True when it was clicked.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, id: egui::Id, area: egui::Rect, below: f32, arrived: usize) -> bool {
    // away for two frames running: a conversation just shown stands off its
    // end for one frame, until the scroll area sticks to it
    let away = arrived > 0 || below > area.height() * AWAY;
    let away_id = id.with("away");
    let frames = if away {
        ui.data(|d| d.get_temp::<u8>(away_id)).unwrap_or(0).saturating_add(1)
    } else {
        0
    };
    ui.data_mut(|d| d.insert_temp(away_id, frames));
    let wanted = frames >= 2;
    let t = ui.ctx().animate_bool_with_time(id, wanted, 0.15);
    // while it fades out, the words it had (the count is gone at once)
    let label_id = id.with("label");
    let label = if wanted {
        let label = if arrived > 0 {
            aurora_i18n::tr!("ui-chat-latest-new", count = arrived)
        } else {
            aurora_i18n::tr!("ui-chat-latest").to_owned()
        };
        ui.data_mut(|d| d.insert_temp(label_id, label.clone()));
        label
    } else {
        ui.data(|d| d.get_temp::<String>(label_id)).unwrap_or_default()
    };
    if t == 0.0 {
        return false;
    }
    let width = widgets::action_button_fit(ui, "arrow-down", &label).min(area.width() - 2.0 * MARGIN);
    let bottom = area.bottom() - MARGIN + (1.0 - t) * RISE;
    let rect = egui::Rect::from_min_size(
        egui::pos2(area.center().x - width / 2.0, bottom - HEIGHT),
        egui::vec2(width, HEIGHT),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    child.multiply_opacity(t);
    let resp = widgets::action_button(&mut child, p, "arrow-down", &label, widgets::ActionKind::Flat, rect.size());
    wanted && resp.clicked()
}
