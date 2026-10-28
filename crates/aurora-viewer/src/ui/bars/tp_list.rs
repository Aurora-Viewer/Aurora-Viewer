//! The session's places under the back / forward arrows (#1322): an arrow
//! held down half a second, dragged off while pressed or right clicked
//! lists every place of this session's navigation history, the newest at
//! the top, with the time we reached it; the place we are in is lit, a
//! click on another one teleports there, and releasing the held button
//! over a place goes there too. Port of LLNavigationBar::
//! onBackOrForwardButtonHeldDown, showTeleportHistoryMenu and
//! rebuildTeleportHistoryMenu (indra/newview/llnavigationbar.cpp,
//! originally LGPL 2.1). The arrival time, the right click and the link to
//! the full Teleport history are Aurora's: the Legacy Viewer's list has
//! the names alone.

use super::{BarAction, StatusInfo};
use crate::theme::Palette;
use crate::ui::menu;
use std::cmp::Ordering;

/// LLButton's held_down_delay (widgets/button.xml), in seconds.
const HOLD: f64 = 0.5;
/// Past this height the places scroll (the Legacy Viewer's menu is
/// scrollable too).
const LIST_MAX_H: f32 = 420.0;
/// A longer title ends with "…": the list keeps a menu's width.
const TITLE_MAX: usize = 48;

/// A press on an arrow: when it started, and whether it opened the list.
#[derive(Clone, Copy)]
struct Press {
    since: f64,
    opened: bool,
}

/// Whether the arrows have a list to show: the place we are in and at
/// least another one.
pub(super) fn has_list(st: &StatusInfo) -> bool {
    st.tp_session.items.len() > 1
}

/// The list of the arrow `resp`, opened by a hold, a drag off it or a right
/// click, at `place` beside it. Returns whether the arrow was clicked (a
/// press that opened the list is no click) and the place chosen.
pub(super) fn arrow(
    ui: &mut egui::Ui,
    p: &Palette,
    resp: &egui::Response,
    st: &StatusInfo,
    place: Option<(egui::RectAlign, f32)>,
) -> (bool, Option<BarAction>) {
    let ctx = ui.ctx().clone();
    let id = resp.id.with("tp_list_press");
    let now = ui.input(|i| i.time);
    let list = has_list(st);
    let down = resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
    let mut open = list && resp.secondary_clicked();
    let mut released_held = false;
    if down {
        let mut press = ctx.data(|d| d.get_temp::<Press>(id)).unwrap_or(Press { since: now, opened: false });
        // setClickDraggingCallback: leaving the button while pressed opens
        // the list at once
        let off = ui.input(|i| i.pointer.interact_pos()).is_some_and(|at| !resp.rect.contains(at));
        if !press.opened && list && (now - press.since >= HOLD || off) {
            press.opened = true;
            open = true;
        } else if !press.opened {
            // a still pointer sends no event: wake the frame when the hold is due
            ctx.request_repaint_after_secs((HOLD - (now - press.since)).max(0.0) as f32);
        }
        ctx.data_mut(|d| d.insert_temp(id, press));
    } else if let Some(press) = ctx.data(|d| d.get_temp::<Press>(id)) {
        released_held = press.opened;
        ctx.data_mut(|d| d.remove::<Press>(id));
    }
    let mut action = None;
    menu::dropdown_held(resp, p, open, place, |ui| action = body(ui, p, st, released_held));
    (resp.clicked() && !released_held, action)
}

/// The places, newest first (rebuildTeleportHistoryMenu: "in the reverse
/// order, just like in Firefox"), then the full Teleport history.
/// `released_held`: the held button was released this frame, over the
/// place to go to if any.
fn body(ui: &mut egui::Ui, p: &Palette, st: &StatusInfo, released_held: bool) -> Option<BarAction> {
    let session = st.tp_session;
    let mut action = None;
    menu::heading(ui, p, aurora_i18n::tr!("ui-bars-tp-list-heading"));
    let release = released_held && ui.input(|i| i.pointer.any_released());
    egui::ScrollArea::vertical()
        .id_salt("bars_tp_list")
        .max_height(LIST_MAX_H)
        .show(ui, |ui| {
            for (i, e) in session.items.iter().enumerate().rev() {
                // TYPE_BACKWARD / TYPE_CURRENT / TYPE_FORWARD
                let icon = match i.cmp(&session.current) {
                    Ordering::Less => "arrow-left",
                    Ordering::Equal => "map-pin-filled",
                    Ordering::Greater => "arrow-right",
                };
                let title = e.title();
                let title = if title.is_empty() {
                    aurora_i18n::tr!("ui-bars-tp-list-unknown").to_owned()
                } else {
                    short(&title)
                };
                let time = super::clock::time_of(st.system_island, e.at);
                let here = i == session.current;
                let row = menu::current_row(ui, p, icon, &title, &time, here);
                if here {
                    row.on_hover_text(aurora_i18n::tr!("ui-bars-tp-list-here"));
                } else if row.clicked() || (release && ui.rect_contains_pointer(row.rect)) {
                    action = Some(BarAction::TeleportHistoryItem(i));
                }
            }
        });
    menu::separator(ui, p);
    if menu::item_keys(ui, p, "history", aurora_i18n::tr!("common-teleport-history"), "Alt+H") {
        action = Some(BarAction::TeleportHistory);
    }
    if action.is_some() {
        ui.close();
    }
    action
}

/// `title` cut to [`TITLE_MAX`] characters, "…" at its end when it was.
fn short(title: &str) -> String {
    if title.chars().count() <= TITLE_MAX {
        return title.to_owned();
    }
    let mut s: String = title.chars().take(TITLE_MAX - 1).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_titles_end_with_an_ellipsis() {
        assert_eq!(short("Aurora Lagoon, Boreal Lagoon"), "Aurora Lagoon, Boreal Lagoon");
        let long = "Ä".repeat(60);
        let cut = short(&long);
        assert_eq!(cut.chars().count(), TITLE_MAX);
        assert!(cut.ends_with('…'));
    }
}
