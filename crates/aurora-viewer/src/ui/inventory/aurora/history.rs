//! Where the explorer has been: Back, Forward and Up, as in a desktop file
//! manager. Port of LLInventorySingleFolderPanel's history
//! (mBackwardFolders / mForwardFolders, onBackwardFolder, onForwardFolder,
//! onUpFolder; indra/newview/llinventorypanel.cpp, originally LGPL 2.1),
//! the back_btn / forward_btn / up_btn of panel_main_inventory.xml.
//!
//! Aurora differences: the history is read from where the explorer is
//! each frame rather than kept by each caller, so every way of going
//! somewhere (a double-click, the column, the breadcrumb, Enter, a
//! revealed item, a shortcut such as Recent) is a step; it holds at most
//! `LIMIT` places; the keys of Windows Explorer and Dolphin (Alt+←/→/↑,
//! the mouse's back and forward buttons, Backspace; all rebindable) and a
//! right-click on Back or Forward listing the places, as in Explorer; Up
//! and Back select the folder they come out of, as Explorer and Dolphin do;
//! a new place forgets what was ahead, as in Explorer (`changeFolderRoot`
//! keeps `mForwardFolders`).

use super::super::shortcuts::{self, Command};
use super::super::*;
use crate::ui::menu;
use egui::{Sense, Vec2};

/// The most places Back keeps (Forward as many).
const LIMIT: usize = 50;
/// The most places a right-click on Back or Forward lists.
const LISTED: usize = 12;

/// A place the explorer shows: a folder, or one of the shortcuts (Recent,
/// Worn, Favorites, Received) whatever folder was last browsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Place {
    pub tab: usize,
    pub folder: Uuid,
}

impl Place {
    pub(crate) fn of(st: &InventoryUi) -> Self {
        Self {
            tab: st.tab,
            // a shortcut is the same place from any folder
            folder: if st.tab == 0 { st.browse } else { Uuid::nil() },
        }
    }
}

/// The places behind and ahead of the explorer.
#[derive(Debug, Default)]
pub(crate) struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
    current: Option<Place>,
}

impl History {
    /// The explorer is at `place` this frame: a new place pushes the last
    /// one behind and forgets what was ahead. True when the place changed.
    pub(crate) fn observe(&mut self, place: Place) -> bool {
        match self.current {
            Some(current) if current == place => false,
            Some(current) => {
                push(&mut self.back, current);
                self.forward.clear();
                self.current = Some(place);
                true
            }
            None => {
                self.current = Some(place);
                true
            }
        }
    }

    pub(crate) fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub(crate) fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Goes `steps` places back (1: the last one); the places passed go
    /// ahead.
    pub(crate) fn back(&mut self, steps: usize) -> Option<Place> {
        step(&mut self.back, &mut self.forward, &mut self.current, steps)
    }

    pub(crate) fn forward(&mut self, steps: usize) -> Option<Place> {
        step(&mut self.forward, &mut self.back, &mut self.current, steps)
    }

    /// The places behind, the last one first.
    pub(crate) fn behind(&self) -> impl Iterator<Item = Place> + '_ {
        self.back.iter().rev().copied()
    }

    /// The places ahead, the next one first.
    pub(crate) fn ahead(&self) -> impl Iterator<Item = Place> + '_ {
        self.forward.iter().rev().copied()
    }

    /// Forgets the folders that are gone (deleted, purged): Back never
    /// lands on nothing. Two equal places brought together become one.
    pub(crate) fn retain(&mut self, exists: impl Fn(Uuid) -> bool) {
        let keep = |place: &Place| place.tab != 0 || exists(place.folder);
        for list in [&mut self.back, &mut self.forward] {
            list.retain(keep);
            list.dedup();
        }
        if self.back.last() == self.current.as_ref() {
            self.back.pop();
        }
        if self.forward.last() == self.current.as_ref() {
            self.forward.pop();
        }
    }
}

fn push(list: &mut Vec<Place>, place: Place) {
    if list.last() != Some(&place) {
        list.push(place);
    }
    if list.len() > LIMIT {
        list.remove(0);
    }
}

fn step(from: &mut Vec<Place>, to: &mut Vec<Place>, current: &mut Option<Place>, steps: usize) -> Option<Place> {
    if steps == 0 || steps > from.len() {
        return None;
    }
    for _ in 0..steps {
        let next = from.pop()?;
        if let Some(here) = current.replace(next) {
            push(to, here);
        }
    }
    *current
}

/// The folder above the explorer's, within its window (a folder window
/// stops at its folder).
pub(crate) fn parent(inv: &Inventory, st: &InventoryUi) -> Option<Uuid> {
    if st.tab != 0 || st.browse == super::base(inv, st) {
        return None;
    }
    rules::parent(inv, st.browse).filter(|id| inv.folders.contains_key(id))
}

/// Shows `place`; the folder left is selected when it is in sight there.
fn go(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, place: Place) {
    let left = st.browse;
    set_tab(st, prefs, place.tab);
    if place.tab == 0 && inv.folders.contains_key(&place.folder) {
        st.search.clear();
        st.browse = place.folder;
    }
    st.selection.clear();
    st.focus = None;
    st.selected = None;
    if place.tab == 0 && rules::parent(inv, left) == Some(st.browse) {
        st.selection.insert(left);
        st.selected = Some(left);
        st.anchor = Some(left);
        st.reveal = Some(left);
    }
}

/// Back or Forward: the history has already stepped, and stands where the
/// explorer now is (it was not a new place).
fn go_stepped(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, place: Place) {
    go(inv, st, prefs, place);
    st.history.current = Some(Place::of(st));
}

/// Up or the top: a new place like any other, so the folder left goes
/// behind and Forward is forgotten (`LLInventorySingleFolderPanel::
/// changeFolderRoot` pushes the folder left to `mBackwardFolders`).
fn go_new(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, place: Place) {
    go(inv, st, prefs, place);
    st.history.observe(Place::of(st));
}

pub(crate) fn go_back(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, steps: usize) {
    if let Some(place) = st.history.back(steps) {
        go_stepped(inv, st, prefs, place);
    }
}

pub(crate) fn go_forward(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, steps: usize) {
    if let Some(place) = st.history.forward(steps) {
        go_stepped(inv, st, prefs, place);
    }
}

pub(crate) fn go_up(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences) {
    if let Some(folder) = parent(inv, st) {
        go_new(inv, st, prefs, Place { tab: 0, folder });
    }
}

pub(crate) fn go_root(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences) {
    let folder = super::base(inv, st);
    go_new(inv, st, prefs, Place { tab: 0, folder });
}

/// Asks the server for the explorer's folder again (F5), as "Reload
/// folder" of its menu does.
pub(crate) fn refresh(inv: &Inventory, st: &mut InventoryUi, actions: &mut Vec<InvAction>) {
    if st.tab == 0 && st.browse != inv.root && st.browse != inv.lib_root && inv.folders.contains_key(&st.browse) {
        st.message.clear();
        actions.push(InvAction::Preview(st.browse, Some(clipboard::focus_id(st))));
    }
}

/// The mouse's side buttons over the explorer, and its keys while its list
/// has the keyboard and no text is being typed.
pub(crate) fn input(
    ui: &egui::Ui,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    actions: &mut Vec<InvAction>,
    area: egui::Rect,
) {
    if st.dialog.is_some() || st.rename.is_some() || ui.ctx().text_edit_focused() || egui::Popup::is_any_open(ui.ctx()) {
        return;
    }
    // the mouse's side buttons over the window, as in a file manager
    if ui.rect_contains_pointer(area) {
        // the mouse's bound buttons (back and forward by default)
        for command in shortcuts::mouse(ui, |c| matches!(c, Command::Back | Command::Forward | Command::Up | Command::Root)) {
            match command {
                Command::Back => go_back(inv, st, prefs, 1),
                Command::Forward => go_forward(inv, st, prefs, 1),
                Command::Up => go_up(inv, st, prefs),
                Command::Root => go_root(inv, st, prefs),
                _ => {}
            }
        }
    }
    // the keys only while the list has the keyboard: elsewhere Alt+arrows
    // move the camera and F-keys play gestures
    if !clipboard::list_has_focus(ui, st) {
        return;
    }
    let navigation = |c: Command| {
        matches!(
            c,
            Command::Back | Command::Forward | Command::Up | Command::Root | Command::Refresh | Command::EditPath | Command::Search
        )
    };
    for command in shortcuts::take(ui, navigation) {
        match command {
            Command::Back => go_back(inv, st, prefs, 1),
            Command::Forward => go_forward(inv, st, prefs, 1),
            Command::Up => go_up(inv, st, prefs),
            Command::Root => go_root(inv, st, prefs),
            Command::Refresh => refresh(inv, st, actions),
            Command::EditPath => super::breadcrumb::edit(inv, st),
            Command::Search => st.focus_search = true,
            _ => {}
        }
    }
}

/// The name of a place in the lists of Back and Forward.
pub(super) fn place_name(inv: &Inventory, place: Place) -> String {
    if place.tab == 0 {
        folder_name(inv, place.folder)
    } else {
        super::toolbar::tab_name(place.tab).to_owned()
    }
}

/// A button of the three, greyed when it leads nowhere.
pub(super) fn button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: String, enabled: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(super::toolbar::BAR),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_tool_button", icon)), rect));
    let hot = enabled && (response.hovered() || response.has_focus());
    if hot {
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            p.raised.lerp_to_gamma(p.ink, 0.16),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let ink = if !enabled {
        p.muted_dim
    } else if hot {
        p.ink
    } else {
        p.muted
    };
    crate::ui::widgets::paint_glyph(ui.painter(), icon, rect.center(), 16.0, ink);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &tip));
    response.on_hover_text(tip)
}

/// Back, Forward and Up at the head of the explorer's toolbar; a
/// right-click on Back or Forward lists where they lead.
pub(crate) fn buttons(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences) {
    let ctx = ui.ctx().clone();
    let back = button(
        ui,
        p,
        "arrow-left",
        shortcuts::tip(&ctx, aurora_i18n::tr!("inventory-back"), Command::Back),
        st.history.can_back(),
    );
    let forward = button(
        ui,
        p,
        "arrow-right",
        shortcuts::tip(&ctx, aurora_i18n::tr!("inventory-forward"), Command::Forward),
        st.history.can_forward(),
    );
    let up = button(
        ui,
        p,
        "arrow-up",
        shortcuts::tip(&ctx, aurora_i18n::tr!("inventory-up"), Command::Up),
        parent(inv, st).is_some(),
    );
    if back.clicked() {
        go_back(inv, st, prefs, 1);
    }
    if forward.clicked() {
        go_forward(inv, st, prefs, 1);
    }
    if up.clicked() {
        go_up(inv, st, prefs);
    }
    for (response, behind) in [(&back, true), (&forward, false)] {
        let places: Vec<Place> = if behind {
            st.history.behind().take(LISTED).collect()
        } else {
            st.history.ahead().take(LISTED).collect()
        };
        if places.is_empty() {
            continue;
        }
        let mut chosen = None;
        menu::context_menu(response, p, |ui| {
            for (n, place) in places.iter().enumerate() {
                let icon = if place.tab == 0 { "folder-filled" } else { "history" };
                if menu::item(ui, p, icon, &place_name(inv, *place)) {
                    chosen = Some(n + 1);
                }
            }
        });
        if let Some(steps) = chosen {
            if behind {
                go_back(inv, st, prefs, steps);
            } else {
                go_forward(inv, st, prefs, steps);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(n: u128) -> Place {
        Place {
            tab: 0,
            folder: Uuid::from_u128(n),
        }
    }

    #[test]
    fn going_somewhere_new_forgets_what_was_ahead() {
        let mut h = History::default();
        for n in 1..=3 {
            h.observe(at(n));
        }
        assert_eq!(h.back(1), Some(at(2)));
        assert_eq!(h.back(1), Some(at(1)));
        assert!(!h.can_back() && h.can_forward());
        assert_eq!(h.forward(1), Some(at(2)));
        h.observe(at(9));
        assert!(!h.can_forward());
        assert_eq!(h.behind().collect::<Vec<_>>(), vec![at(2), at(1)]);
    }

    #[test]
    fn the_same_place_seen_again_is_not_a_step() {
        let mut h = History::default();
        h.observe(at(1));
        h.observe(at(1));
        assert!(!h.can_back());
        h.observe(at(2));
        assert_eq!(h.behind().collect::<Vec<_>>(), vec![at(1)]);
    }

    #[test]
    fn a_listed_place_is_reached_in_one_jump() {
        let mut h = History::default();
        for n in 1..=5 {
            h.observe(at(n));
        }
        assert_eq!(h.back(3), Some(at(2)));
        assert_eq!(h.ahead().collect::<Vec<_>>(), vec![at(3), at(4), at(5)]);
        assert_eq!(h.forward(3), Some(at(5)));
        assert_eq!(h.back(9), None, "past the oldest place");
        assert_eq!(h.current, Some(at(5)));
    }

    #[test]
    fn the_history_is_bounded() {
        let mut h = History::default();
        for n in 0..(LIMIT as u128 + 20) {
            h.observe(at(n));
        }
        assert_eq!(h.behind().count(), LIMIT);
        assert_eq!(h.behind().last(), Some(at(19)));
    }

    #[test]
    fn deleted_folders_leave_the_history() {
        let mut h = History::default();
        for n in [1, 2, 1, 3] {
            h.observe(at(n));
        }
        h.retain(|id| id != Uuid::from_u128(2));
        // 1, 2, 1 → 1 once
        assert_eq!(h.behind().collect::<Vec<_>>(), vec![at(1)]);
        assert_eq!(h.back(1), Some(at(1)));
    }

    fn tree() -> Inventory {
        use crate::world::inventory::{FetchState, Folder};
        let mut inv = Inventory::default();
        for (id, parent) in [(1u128, 0u128), (2, 1), (3, 2)] {
            let (id, parent) = (Uuid::from_u128(id), Uuid::from_u128(parent));
            inv.folders.insert(
                id,
                Folder {
                    info: aurora_net::inventory::InvFolder {
                        id,
                        parent,
                        name: format!("{id}"),
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Fetched,
                    library: false,
                },
            );
            if let Some(p) = inv.folders.get_mut(&parent) {
                p.children.push(id);
            }
        }
        inv.root = Uuid::from_u128(1);
        inv
    }

    /// Up and the top are steps like any other: Back returns to the folder
    /// left (`onUpFolderClicked` → `changeFolderRoot`).
    #[test]
    fn up_and_the_top_are_steps_back_returns_from() {
        let inv = tree();
        let prefs = InventoryPreferences::default();
        let mut st = InventoryUi::default();
        for n in 1..=3 {
            st.browse = Uuid::from_u128(n);
            st.history.observe(Place::of(&st));
        }
        go_up(&inv, &mut st, &prefs);
        assert_eq!(st.browse, Uuid::from_u128(2));
        // the next frame sees no new place
        assert!(!st.history.observe(Place::of(&st)));
        go_back(&inv, &mut st, &prefs, 1);
        assert_eq!(st.browse, Uuid::from_u128(3), "Back returns to B");
        go_root(&inv, &mut st, &prefs);
        assert_eq!(st.browse, Uuid::from_u128(1));
        assert!(!st.history.can_forward(), "a new place forgets what was ahead");
        go_back(&inv, &mut st, &prefs, 1);
        assert_eq!(st.browse, Uuid::from_u128(3));
        go_back(&inv, &mut st, &prefs, 1);
        assert_eq!(st.browse, Uuid::from_u128(2));
    }

    #[test]
    fn a_shortcut_is_one_place_whatever_folder_was_browsed() {
        let mut st = InventoryUi {
            tab: 2,
            browse: Uuid::from_u128(7),
            ..Default::default()
        };
        let recent = Place::of(&st);
        st.browse = Uuid::from_u128(8);
        assert_eq!(Place::of(&st), recent);
        assert!(recent.folder.is_nil());
    }
}
