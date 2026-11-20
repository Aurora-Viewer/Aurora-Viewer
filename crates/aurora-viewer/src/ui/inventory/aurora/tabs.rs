//! The explorer's tabs, as Windows Explorer's and Dolphin's: several
//! folders open in one pane, each with its own place, Back / Forward
//! history and selection. Ctrl+T opens the folder shown in a new tab,
//! Ctrl+W closes the tab, Ctrl+Tab / Ctrl+Shift+Tab go from one to the next
//! (all rebindable; Cmd+T / Cmd+W on a Mac); a middle click on a folder, or
//! its menu's "Open in new tab", opens it in a tab behind. The strip is
//! always there above the explorer's toolbar, as Explorer's: a click shows
//! a tab, a middle click or its cross closes it, cargo dropped on a tab goes
//! into its folder and cargo held over one shows it. A tab dragged along
//! the strip moves there; dragged onto another window's strip it moves to
//! that window (a window whose last tab leaves closes, as Explorer's and
//! the browsers' windows merge); dragged away from any strip it becomes a
//! window of its own (`settle`). Aurora difference: the Legacy Viewer's
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

/// Takes tab `index` out of the pane. Its last tab leaves the pane empty
/// (`tabs_emptied`): only a window that may close gives it (`loose`).
fn detach(st: &mut InventoryUi, prefs: &InventoryPreferences, index: usize) -> Option<Tab> {
    let count = st.tabs.len();
    if index >= count {
        return None;
    }
    if count == 1 {
        st.tabs_emptied = true;
        return Some(take(st));
    }
    if index == st.tabs.active {
        switch(st, prefs, if index + 1 < count { index + 1 } else { index - 1 });
    }
    let tab = st.tabs.list.remove(index);
    if index < st.tabs.active {
        st.tabs.active -= 1;
    }
    if st.tabs.list.len() < 2 {
        st.tabs.list.clear();
        st.tabs.active = 0;
    }
    Some(tab)
}

/// `tab` comes in before tab `at`, shown.
fn attach(st: &mut InventoryUi, prefs: &InventoryPreferences, tab: Tab, at: usize) {
    if st.tabs.list.is_empty() {
        st.tabs.list.push(Tab::default());
        st.tabs.active = 0;
    }
    let at = at.min(st.tabs.list.len());
    st.tabs.list.insert(at, tab);
    if at <= st.tabs.active {
        st.tabs.active += 1;
    }
    switch(st, prefs, at);
}

/// Tab `from` moves to before tab `to` (counted before the move).
fn reorder(st: &mut InventoryUi, from: usize, to: usize) {
    let count = st.tabs.list.len();
    if count < 2 || from >= count {
        return;
    }
    let to = if to > from { to - 1 } else { to }.min(count - 1);
    if to == from {
        return;
    }
    let tab = st.tabs.list.remove(from);
    st.tabs.list.insert(to, tab);
    let active = st.tabs.active;
    st.tabs.active = if active == from {
        to
    } else {
        let mut active = active;
        if from < active {
            active -= 1;
        }
        if to <= active {
            active += 1;
        }
        active
    };
}

/// Whether the pane's window may lose its last tab and close: the
/// additional window and the windows made from tabs, not the main window
/// or its right pane.
fn loose(st: &InventoryUi) -> bool {
    st.torn_off || st.ui_id == ADDITIONAL_WINDOW
}

/// Whether the pane takes tabs from elsewhere: the whole inventory's, not a
/// folder window's (its tabs stay under its folder).
fn open_to_tabs(st: &InventoryUi) -> bool {
    st.scope.is_nil()
}

/// A tab being dragged: its pane and place in the strip.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TabCargo {
    pane: Uuid,
    index: usize,
}

/// The dragged tab was let go at `at`, away from the strip `strip`.
#[derive(Clone, Copy, Debug)]
struct Ended {
    pane: Uuid,
    index: usize,
    at: Pos2,
    strip: Rect,
}

impl Default for Ended {
    fn default() -> Self {
        Ended {
            pane: Uuid::nil(),
            index: 0,
            at: Pos2::ZERO,
            strip: Rect::NOTHING,
        }
    }
}

/// The dragged tab was let go on the strip of `pane`, before its tab `at`.
#[derive(Clone, Copy, Debug, Default)]
struct Landing {
    pane: Uuid,
    at: usize,
}

fn ended_id() -> egui::Id {
    egui::Id::unique("inventory_tab_drag_ended")
}

fn landing_id() -> egui::Id {
    egui::Id::unique("inventory_tab_drag_landing")
}

/// Once every pane and window has drawn: the tab let go this frame moves
/// along its strip, to the strip it landed on, or out to a window of its
/// own (pushed to `spawned`, opening where it was let go).
pub(crate) fn settle(
    ctx: &egui::Context,
    panes: &mut [&mut InventoryUi],
    prefs: &InventoryPreferences,
    spawned: &mut Vec<InventoryWindow>,
) {
    let (ended, landing) = ctx.data_mut(|d| (d.remove_temp::<Ended>(ended_id()), d.remove_temp::<Landing>(landing_id())));
    let Some(ended) = ended else { return };
    let Some(from) = panes.iter().position(|st| st.ui_id == ended.pane) else {
        return;
    };
    match landing {
        Some(landing) if landing.pane == ended.pane => reorder(panes[from], ended.index, landing.at),
        Some(landing) => {
            let Some(to) = panes.iter().position(|st| st.ui_id == landing.pane) else {
                return;
            };
            let gives = (panes[from].tabs.len() > 1 || loose(panes[from])) && open_to_tabs(panes[from]);
            if !gives || !open_to_tabs(panes[to]) || panes[to].tabs.list.len() >= LIMIT {
                return;
            }
            if let Some(tab) = detach(panes[from], prefs, ended.index) {
                attach(panes[to], prefs, tab, landing.at);
            }
        }
        // away from the strips: a window of its own, unless the tab is its
        // window's only one (dragging that is moving the window)
        None => {
            let away = !ended.strip.expand2(Vec2::new(0.0, 24.0)).contains(ended.at);
            if !away || panes[from].tabs.len() < 2 || !open_to_tabs(panes[from]) {
                return;
            }
            if let Some(tab) = detach(panes[from], prefs, ended.index) {
                let mut state = InventoryUi {
                    torn_off: true,
                    spawn_at: Some(ended.at - Vec2::new(60.0, 16.0)),
                    ..Default::default()
                };
                put(&mut state, prefs, tab);
                spawned.push(InventoryWindow {
                    id: Uuid::new_v4(),
                    root: Uuid::nil(),
                    additional: false,
                    state: Box::new(state),
                    open: true,
                });
            }
        }
    }
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

/// The strip above the explorer's toolbar.
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
    let count = st.tabs.len();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STRIP), Sense::hover());
    let plus = Rect::from_min_size(Pos2::new(rect.right() - STRIP, rect.top()), Vec2::splat(STRIP));
    let room = (plus.left() - rect.left() - 4.0).max(0.0);
    let width = ((room - 2.0 * (count as f32 - 1.0)) / count as f32).clamp(48.0, 220.0);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let now = ui.input(|i| i.time);
    let dragged_tab = egui::DragAndDrop::payload::<TabCargo>(ui.ctx()).map(|cargo| *cargo);
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
        let response = ui.interact(cell, ui.scope_id().with(("inventory_folder_tab", index)), Sense::click_and_drag());
        let shown = index == st.tabs.active || st.tabs.list.is_empty();
        let hot = response.hovered();
        let moving = dragged_tab.is_some_and(|cargo| cargo.pane == st.ui_id && cargo.index == index);
        if (shown || hot) && !moving {
            let fill = if shown { p.raised } else { p.raised.gamma_multiply(0.6) };
            ui.painter().rect_filled(cell, radius, fill);
        }
        let ink = if moving {
            p.muted_dim
        } else if shown {
            p.ink
        } else {
            p.muted
        };
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
        // its cross, on the tab shown or under the pointer; the last tab
        // has none
        if count > 1 {
            let close = ui.interact(
                close_rect,
                ui.scope_id().with(("inventory_folder_tab_close", index)),
                Sense::click(),
            );
            if (shown || hot || close.hovered()) && !moving {
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
            if close.clicked() {
                closing = Some(index);
            }
        }
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_folder_tab", st.ui_id, index)), cell));
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, shown, &name));
        if response.middle_clicked() && count > 1 {
            closing = Some(index);
        } else if response.clicked() {
            wanted = Some(index);
        }
        // the tab itself dragged: along the strip, to another window's, or
        // away to a window of its own (`settle`)
        if response.drag_started() && open_to_tabs(st) {
            egui::DragAndDrop::set_payload(ui.ctx(), TabCargo { pane: st.ui_id, index });
        }
        if moving {
            ghost(ui, p, &name, icon);
        }
        if response.drag_stopped()
            && moving
            && let Some(at) = ui.input(|i| i.pointer.interact_pos())
        {
            let ended = Ended {
                pane: st.ui_id,
                index,
                at,
                strip: rect,
            };
            ui.ctx().data_mut(|d| d.insert_temp(ended_id(), ended));
        }
        let response = response.on_hover_text(&name);
        // cargo dropped on a tab goes into its folder; held there, the tab
        // shows
        if dragged_tab.is_none() && place.tab == 0 && inv.folders.contains_key(&place.folder) {
            if !shown && response.contains_pointer() && egui::DragAndDrop::has_any_payload(ui.ctx()) {
                held = Some(index);
            }
            drag::target(ui, p, &response, inv, place.folder, st, prefs, facts, actions);
        }
    }
    // a tab let go over this strip lands before the tab under it
    if dragged_tab.is_some()
        && open_to_tabs(st)
        && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        && rect.expand2(Vec2::new(0.0, 6.0)).contains(pointer)
    {
        let at = (((pointer.x - rect.left()) / (width + 2.0)) + 0.5).floor().clamp(0.0, count as f32) as usize;
        let x = (rect.left() + at as f32 * (width + 2.0) - 1.0).min(plus.left() - 2.0);
        ui.painter().vline(x, rect.y_range(), egui::Stroke::new(2.0, p.violet));
        if ui.input(|i| i.pointer.primary_released()) {
            ui.ctx().data_mut(|d| d.insert_temp(landing_id(), Landing { pane: st.ui_id, at }));
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

/// The tab under the pointer while it is dragged.
fn ghost(ui: &egui::Ui, p: &Palette, name: &str, icon: &str) {
    let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("inventory_tab_ghost")));
    let galley = painter.layout_no_wrap(name.to_owned(), egui::FontId::proportional(12.5), p.ink);
    let rect = Rect::from_min_size(
        pointer + Vec2::new(12.0, 8.0),
        Vec2::new((galley.size().x + 40.0).min(260.0), STRIP),
    );
    let radius = ui.visuals().widgets.inactive.corner_radius;
    painter.rect_filled(rect, radius, p.raised);
    painter.rect_stroke(rect, radius, egui::Stroke::new(1.0, p.violet), egui::StrokeKind::Inside);
    widgets::paint_glyph(&painter, icon, Pos2::new(rect.left() + 14.0, rect.center().y), 14.0, p.ink);
    painter.with_clip_rect(rect.shrink(4.0)).galley(
        Pos2::new(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0),
        galley,
        p.ink,
    );
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

    fn pane(n: u128, folder: u128) -> InventoryUi {
        InventoryUi {
            ui_id: id(n),
            browse: id(folder),
            ..Default::default()
        }
    }

    fn folders(st: &InventoryUi) -> Vec<Uuid> {
        (0..st.tabs.len()).map(|index| place(st, index).folder).collect()
    }

    #[test]
    fn a_tab_dragged_along_its_strip_moves_there() {
        let prefs = InventoryPreferences::default();
        let mut st = pane(0, 1);
        open(&mut st, &prefs, id(2), false);
        open(&mut st, &prefs, id(3), false);
        // the first (shown) to the end
        reorder(&mut st, 0, 3);
        assert_eq!(folders(&st), [id(2), id(3), id(1)]);
        assert_eq!((st.tabs.active, at(&st)), (2, id(1)), "still shown");
        // the last to the front
        reorder(&mut st, 2, 0);
        assert_eq!(folders(&st), [id(1), id(2), id(3)]);
        assert_eq!(st.tabs.active, 0);
        reorder(&mut st, 1, 3);
        assert_eq!(folders(&st), [id(1), id(3), id(2)]);
        assert_eq!(at(&st), id(1));
    }

    fn let_go(ctx: &egui::Context, pane: u128, index: usize, landing: Option<(u128, usize)>, at: Pos2) {
        ctx.data_mut(|d| {
            d.insert_temp(
                ended_id(),
                Ended {
                    pane: id(pane),
                    index,
                    at,
                    strip: Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, STRIP)),
                },
            );
            if let Some((pane, at)) = landing {
                d.insert_temp(landing_id(), Landing { pane: id(pane), at });
            }
        });
    }

    #[test]
    fn a_tab_moves_to_another_window_and_a_window_left_empty_closes() {
        let prefs = InventoryPreferences::default();
        let ctx = egui::Context::default();
        let mut main = pane(0, 1);
        open(&mut main, &prefs, id(2), false);
        let mut other = InventoryUi {
            torn_off: true,
            ..pane(7, 5)
        };
        let mut spawned = Vec::new();
        // main's second tab onto the other window's strip, at its front
        let_go(&ctx, 0, 1, Some((7, 0)), Pos2::new(10.0, 10.0));
        settle(&ctx, &mut [&mut main, &mut other], &prefs, &mut spawned);
        assert_eq!(folders(&main), [id(1)]);
        assert_eq!(folders(&other), [id(2), id(5)]);
        assert_eq!(at(&other), id(2), "shown where it lands");
        // the main window keeps its last tab
        let_go(&ctx, 0, 0, Some((7, 2)), Pos2::new(10.0, 10.0));
        settle(&ctx, &mut [&mut main, &mut other], &prefs, &mut spawned);
        assert_eq!(folders(&main), [id(1)]);
        assert!(!main.tabs_emptied);
        // a made window gives its tabs and closes with the last
        let_go(&ctx, 7, 0, Some((0, 1)), Pos2::new(10.0, 10.0));
        settle(&ctx, &mut [&mut main, &mut other], &prefs, &mut spawned);
        let_go(&ctx, 7, 0, Some((0, 2)), Pos2::new(10.0, 10.0));
        settle(&ctx, &mut [&mut main, &mut other], &prefs, &mut spawned);
        assert_eq!(folders(&main), [id(1), id(2), id(5)]);
        assert!(other.tabs_emptied, "its window closes");
        assert!(spawned.is_empty());
    }

    #[test]
    fn a_tab_let_go_away_from_the_strips_becomes_a_window() {
        let prefs = InventoryPreferences::default();
        let ctx = egui::Context::default();
        let mut main = pane(0, 1);
        let mut spawned = Vec::new();
        // a pane's only tab stays
        let_go(&ctx, 0, 0, None, Pos2::new(300.0, 300.0));
        settle(&ctx, &mut [&mut main], &prefs, &mut spawned);
        assert!(spawned.is_empty());
        open(&mut main, &prefs, id(2), false);
        // let go on the strip itself: nothing
        let_go(&ctx, 0, 1, None, Pos2::new(300.0, 10.0));
        settle(&ctx, &mut [&mut main], &prefs, &mut spawned);
        assert!(spawned.is_empty());
        // away from it: a window of its own where it was let go
        let_go(&ctx, 0, 1, None, Pos2::new(300.0, 300.0));
        settle(&ctx, &mut [&mut main], &prefs, &mut spawned);
        assert_eq!(folders(&main), [id(1)]);
        let window = spawned.pop().expect("a window");
        assert!(window.open && window.root.is_nil() && window.state.torn_off);
        assert_eq!(window.state.browse, id(2));
        assert!(window.state.spawn_at.is_some_and(|at| at.y < 300.0));
        // a folder window keeps its tabs under its folder
        let mut folder = InventoryUi {
            scope: id(9),
            ..pane(8, 9)
        };
        open(&mut folder, &prefs, id(10), false);
        let_go(&ctx, 8, 1, Some((0, 0)), Pos2::new(10.0, 10.0));
        settle(&ctx, &mut [&mut main, &mut folder], &prefs, &mut spawned);
        assert_eq!(folders(&folder), [id(9), id(10)]);
        let_go(&ctx, 0, 0, Some((8, 0)), Pos2::new(10.0, 10.0));
        settle(&ctx, &mut [&mut main, &mut folder], &prefs, &mut spawned);
        assert_eq!(folders(&folder), [id(9), id(10)], "takes none");
    }
}
