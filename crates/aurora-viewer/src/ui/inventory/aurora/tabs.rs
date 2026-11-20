//! The explorer's tabs, as Windows Explorer's and Dolphin's: several
//! folders open in one pane, each with its own place, Back / Forward
//! history and selection. Ctrl+T opens the folder shown in a new tab,
//! Ctrl+W closes the tab, Ctrl+Tab / Ctrl+Shift+Tab go from one to the next
//! (all rebindable; Cmd+T / Cmd+W on a Mac); a middle click on a folder, or
//! its menu's "Open in new tab", opens it in a tab behind. The strip shows
//! from two tabs on, as Dolphin's: a click shows a tab, a middle click or
//! its cross closes it, cargo dropped on a tab goes into its folder and
//! cargo held over one shows it. Aurora difference: the Legacy Viewer's
//! inventory has no tabs (its All Items / Recent / Worn tabs are the
//! column's shortcuts here).
//!
//! The tab shown lives in the pane's own fields (`InventoryUi::browse`,
//! `history`, `selection`…), so the rest of the explorer never knows about
//! tabs: switching swaps them with the stored tab's. The tabs last the
//! session, as the history does.

use super::super::shortcuts::{self, Command};
use super::super::*;
use super::history::{self, Place};
use crate::ui::widgets;
use egui::{Pos2, Rect, Sense, Vec2};

/// The most tabs a pane opens.
pub(crate) const LIMIT: usize = 20;
/// The strip's height.
const STRIP: f32 = 30.0;
/// How long cargo is held over a tab before it shows (Explorer's delay).
const HOVER_SHOWS: f64 = 0.6;

/// A tab not shown: what the pane's fields hold while it is.
#[derive(Debug, Default)]
pub(crate) struct Tab {
    place: Place,
    history: history::History,
    selection: HashSet<Uuid>,
    selected: Option<Uuid>,
    anchor: Option<Uuid>,
}

/// A pane's tabs. With fewer than two the pane has none (`list` empty);
/// otherwise `list[active]` is an empty stand-in for the tab shown.
#[derive(Debug, Default)]
pub(crate) struct Tabs {
    list: Vec<Tab>,
    active: usize,
    /// Cargo held over a tab since then (its index, the time).
    hover: Option<(usize, f64)>,
}

impl Tabs {
    pub(crate) fn len(&self) -> usize {
        self.list.len().max(1)
    }
}

/// The tab shown, taken out of the pane's fields.
fn take(st: &mut InventoryUi) -> Tab {
    Tab {
        place: Place {
            tab: st.tab,
            folder: st.browse,
        },
        history: std::mem::take(&mut st.history),
        selection: std::mem::take(&mut st.selection),
        selected: st.selected.take(),
        anchor: st.anchor.take(),
    }
}

/// `tab` shown in the pane's fields.
fn put(st: &mut InventoryUi, prefs: &InventoryPreferences, tab: Tab) {
    set_tab(st, prefs, tab.place.tab);
    if !tab.place.folder.is_nil() {
        st.browse = tab.place.folder;
    }
    st.history = tab.history;
    st.selection = tab.selection;
    st.selected = tab.selected;
    st.anchor = tab.anchor;
    // the keyboard's place, a typed path, a rename or a menu belong to the
    // tab left
    st.focus = None;
    st.path_edit = None;
    st.rename = None;
    st.row_menu = None;
    st.background_menu = false;
    st.reveal = st.selected;
}

/// The place of tab `index`: the pane's own for the one shown.
fn place(st: &InventoryUi, index: usize) -> Place {
    if index == st.tabs.active || st.tabs.list.is_empty() {
        Place {
            tab: st.tab,
            folder: st.browse,
        }
    } else {
        st.tabs.list[index].place
    }
}

/// Opens `folder` in a new tab at the end, as Explorer does; `show` goes
/// there, otherwise it waits behind (a middle click). False at the limit.
pub(crate) fn open(st: &mut InventoryUi, prefs: &InventoryPreferences, folder: Uuid, show: bool) -> bool {
    if st.tabs.list.len() >= LIMIT {
        st.message = aurora_i18n::tr!("inventory-tabs-limit", count = LIMIT).to_string();
        return false;
    }
    if st.tabs.list.is_empty() {
        st.tabs.list.push(Tab::default());
        st.tabs.active = 0;
    }
    let at = st.tabs.list.len();
    st.tabs.list.push(Tab {
        place: Place { tab: 0, folder },
        ..Tab::default()
    });
    if show {
        switch(st, prefs, at);
    }
    true
}

/// Shows tab `index`.
pub(crate) fn switch(st: &mut InventoryUi, prefs: &InventoryPreferences, index: usize) {
    if index == st.tabs.active || index >= st.tabs.list.len() {
        return;
    }
    let shown = take(st);
    // the tab left takes the stand-in's place, the stand-in the next one's
    let next = std::mem::take(&mut st.tabs.list[index]);
    st.tabs.list[st.tabs.active] = shown;
    st.tabs.active = index;
    put(st, prefs, next);
}

/// Closes tab `index`; the last tab stays.
pub(crate) fn close(st: &mut InventoryUi, prefs: &InventoryPreferences, index: usize) {
    let count = st.tabs.list.len();
    if count < 2 || index >= count {
        return;
    }
    if index == st.tabs.active {
        // the tab after it shows, or the one before for the last
        switch(st, prefs, if index + 1 < count { index + 1 } else { index - 1 });
    }
    st.tabs.list.remove(index);
    if index < st.tabs.active {
        st.tabs.active -= 1;
    }
    if st.tabs.list.len() < 2 {
        st.tabs.list.clear();
        st.tabs.active = 0;
    }
}

/// The next tab (`forward`) or the one before, wrapping.
pub(crate) fn cycle(st: &mut InventoryUi, prefs: &InventoryPreferences, forward: bool) {
    let count = st.tabs.list.len();
    if count < 2 {
        return;
    }
    let next = if forward {
        (st.tabs.active + 1) % count
    } else {
        (st.tabs.active + count - 1) % count
    };
    switch(st, prefs, next);
}

/// Forgets the tabs whose folder is gone, as the history forgets places.
pub(crate) fn retain(st: &mut InventoryUi, prefs: &InventoryPreferences, inv: &Inventory) {
    let gone: Vec<usize> = (0..st.tabs.list.len())
        .filter(|index| *index != st.tabs.active)
        .filter(|index| {
            let place = st.tabs.list[*index].place;
            place.tab == 0 && !inv.folders.contains_key(&place.folder)
        })
        .collect();
    for index in gone.into_iter().rev() {
        close(st, prefs, index);
    }
}

/// The tab keys while the list has the keyboard.
pub(crate) fn input(ui: &egui::Ui, st: &mut InventoryUi, prefs: &InventoryPreferences) {
    let commands = shortcuts::take(ui, |c| {
        matches!(c, Command::NewTab | Command::CloseTab | Command::NextTab | Command::PreviousTab)
    });
    for command in commands {
        match command {
            Command::NewTab => {
                let folder = if st.tab == 0 { st.browse } else { Uuid::nil() };
                if !folder.is_nil() {
                    open(st, prefs, folder, true);
                }
            }
            Command::CloseTab => close(st, prefs, st.tabs.active),
            Command::NextTab => cycle(st, prefs, true),
            Command::PreviousTab => cycle(st, prefs, false),
            _ => {}
        }
    }
}

/// The strip above the explorer's toolbar, from two tabs on.
#[allow(clippy::too_many_arguments)]
pub(crate) fn strip(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let count = st.tabs.list.len();
    if count < 2 {
        st.tabs.hover = None;
        return;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STRIP), Sense::hover());
    let plus = Rect::from_min_size(Pos2::new(rect.right() - STRIP, rect.top()), Vec2::splat(STRIP));
    let room = (plus.left() - rect.left() - 4.0).max(0.0);
    let width = ((room - 2.0 * (count as f32 - 1.0)) / count as f32).clamp(48.0, 220.0);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let now = ui.input(|i| i.time);
    let mut wanted = None;
    let mut closing = None;
    let mut held = None;
    for index in 0..count {
        let cell = Rect::from_min_size(
            Pos2::new(rect.left() + index as f32 * (width + 2.0), rect.top()),
            Vec2::new(width, STRIP),
        );
        if cell.left() >= plus.left() {
            break;
        }
        let place = place(st, index);
        let name = history::place_name(inv, place);
        let response = ui.interact(cell, ui.scope_id().with(("inventory_folder_tab", index)), Sense::click());
        let shown = index == st.tabs.active;
        let hot = response.hovered();
        if shown || hot {
            let fill = if shown { p.raised } else { p.raised.gamma_multiply(0.6) };
            ui.painter().rect_filled(cell, radius, fill);
        }
        let ink = if shown { p.ink } else { p.muted };
        let close_rect = Rect::from_center_size(Pos2::new(cell.right() - 13.0, cell.center().y), Vec2::splat(18.0));
        let icon = match place.tab {
            0 => "folder-filled",
            2 => "clock-filled",
            3 => "hanger-2-filled",
            4 => "star-filled",
            _ => "inbox",
        };
        widgets::paint_glyph(ui.painter(), icon, Pos2::new(cell.left() + 14.0, cell.center().y), 14.0, ink);
        let galley = ui.painter().layout_job({
            let mut job =
                egui::text::LayoutJob::single_section(name.clone(), egui::TextFormat::simple(egui::FontId::proportional(12.5), ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width((cell.width() - 28.0 - 22.0).max(8.0));
            job
        });
        widgets::paint_row_text(ui.painter(), galley, cell.left() + 26.0, cell.center().y, ink);
        // its cross, on the tab shown or under the pointer
        let close = ui.interact(
            close_rect,
            ui.scope_id().with(("inventory_folder_tab_close", index)),
            Sense::click(),
        );
        if shown || hot || close.hovered() {
            if close.hovered() {
                ui.painter().rect_filled(close_rect, radius, p.raised.lerp_to_gamma(p.ink, 0.16));
            }
            widgets::paint_glyph(
                ui.painter(),
                "x",
                close_rect.center(),
                12.0,
                if close.hovered() { p.ink } else { p.muted },
            );
        }
        let close = close.on_hover_text(shortcuts::tip(ui.ctx(), aurora_i18n::tr!("inventory-close-tab"), Command::CloseTab));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_folder_tab", st.ui_id, index)), cell));
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, shown, &name));
        if close.clicked() || response.middle_clicked() {
            closing = Some(index);
        } else if response.clicked() {
            wanted = Some(index);
        }
        let response = response.on_hover_text(&name);
        // cargo dropped on a tab goes into its folder; held there, the tab
        // shows
        if place.tab == 0 && inv.folders.contains_key(&place.folder) {
            if !shown && response.contains_pointer() && egui::DragAndDrop::has_any_payload(ui.ctx()) {
                held = Some(index);
            }
            drag::target(ui, p, &response, inv, place.folder, st, prefs, facts, actions);
        }
    }
    let tip = shortcuts::tip(ui.ctx(), aurora_i18n::tr!("inventory-new-tab"), Command::NewTab);
    let new_tab = ui
        .scope_builder(egui::UiBuilder::new().max_rect(plus), |ui| {
            history::button(ui, p, "plus", tip, st.tab == 0)
        })
        .inner;
    if new_tab.clicked() && st.tab == 0 {
        let folder = st.browse;
        open(st, prefs, folder, true);
    }
    match (held, st.tabs.hover) {
        (Some(index), Some((was, since))) if was == index => {
            if now - since >= HOVER_SHOWS {
                wanted = Some(index);
                st.tabs.hover = None;
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        (Some(index), _) => {
            st.tabs.hover = Some((index, now));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        (None, _) => st.tabs.hover = None,
    }
    if let Some(index) = closing {
        close(st, prefs, index);
    } else if let Some(index) = wanted {
        switch(st, prefs, index);
    }
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn at(st: &InventoryUi) -> Uuid {
        st.browse
    }

    #[test]
    fn tabs_keep_their_own_folder_history_and_selection() {
        let prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            browse: id(1),
            ..Default::default()
        };
        st.history.observe(Place { tab: 0, folder: id(1) });
        st.selection.insert(id(11));
        st.selected = Some(id(11));
        assert_eq!(st.tabs.len(), 1);

        // a tab behind: the pane stays
        assert!(open(&mut st, &prefs, id(2), false));
        assert_eq!((st.tabs.len(), st.tabs.active, at(&st)), (2, 0, id(1)));
        // a tab shown
        assert!(open(&mut st, &prefs, id(3), true));
        assert_eq!((st.tabs.len(), st.tabs.active, at(&st)), (3, 2, id(3)));
        assert!(st.selection.is_empty() && !st.history.can_back());
        st.history.observe(Place { tab: 0, folder: id(3) });
        st.browse = id(4);
        st.history.observe(Place { tab: 0, folder: id(4) });

        // back to the first: its selection and history come back
        switch(&mut st, &prefs, 0);
        assert_eq!(at(&st), id(1));
        assert_eq!(st.selected, Some(id(11)));
        assert!(st.selection.contains(&id(11)));
        assert!(!st.history.can_back());
        // in their order of opening: the one behind, the one shown (now at 4)
        assert_eq!(place(&st, 1).folder, id(2));
        assert_eq!(place(&st, 2).folder, id(4));
        cycle(&mut st, &prefs, true);
        cycle(&mut st, &prefs, true);
        assert_eq!(at(&st), id(4));
        assert!(st.history.can_back(), "its own history");
        cycle(&mut st, &prefs, true);
        assert_eq!(at(&st), id(1), "wraps");
        cycle(&mut st, &prefs, false);
        assert_eq!(at(&st), id(4), "wraps back");
    }

    #[test]
    fn closing_shows_the_next_tab_and_the_last_one_stays() {
        let prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            browse: id(1),
            ..Default::default()
        };
        open(&mut st, &prefs, id(2), false);
        open(&mut st, &prefs, id(3), false);
        // 1 (shown), 2, 3
        close(&mut st, &prefs, 0);
        assert_eq!((at(&st), st.tabs.active, st.tabs.len()), (id(2), 0, 2));
        close(&mut st, &prefs, 1);
        assert_eq!((at(&st), st.tabs.len()), (id(2), 1));
        assert!(st.tabs.list.is_empty(), "one tab is no tabs");
        close(&mut st, &prefs, 0);
        assert_eq!(at(&st), id(2), "the last tab stays");
        // the last tab closed shows the one before
        open(&mut st, &prefs, id(5), true);
        close(&mut st, &prefs, 1);
        assert_eq!(at(&st), id(2));
    }

    #[test]
    fn at_most_twenty_tabs_and_gone_folders_close() {
        let prefs = InventoryPreferences::default();
        let mut st = InventoryUi {
            browse: id(1),
            ..Default::default()
        };
        for n in 0..LIMIT as u128 + 3 {
            open(&mut st, &prefs, id(100 + n), false);
        }
        assert_eq!(st.tabs.len(), LIMIT);
        assert!(!st.message.is_empty());
        let inv = Inventory::default();
        retain(&mut st, &prefs, &inv);
        assert_eq!(st.tabs.len(), 1, "every folder behind is gone");
        assert_eq!(at(&st), id(1), "the tab shown stays");
    }
}
