//! Port of LLFolderView::handleKeyHere (indra/llui/llfolderview.cpp,
//! originally LGPL 2.1): arrows follow displayed rows; Shift grows/shrinks
//! sibling selections without selecting an open folder's children as well.
//! Right opens the current folder; Left closes it, or from an item or a
//! closed folder selects the parent row (a second Left then closes it).
//! Typing on the focused list is LLFolderView::handleUnicodeCharHere and
//! LLFolderView::search: the nearest row whose name starts with the typed
//! text is selected in place, without filtering the inventory.

use super::*;

/// The prefix typed on the focused list (LLFolderView::mSearchString) and when
/// its last character came in. It is per window, like the open folders.
#[derive(Default)]
pub(super) struct TypeAhead {
    pub(super) text: String,
    at: f64,
}

/// `TypeAheadTimeout` (indra/newview/app_settings/settings.xml): the typed
/// prefix is forgotten after this many seconds of silence.
const TYPE_AHEAD_TIMEOUT: f64 = 1.5;
/// The cap of LLFolderView::handleUnicodeCharHere.
const TYPE_AHEAD_MAX: usize = 128;

/// Run after rebuilding the displayed order and before laying out its viewport,
/// including while Aurora's explorer has swapped its flat tree into `plain`.
pub(super) fn navigate(ui: &egui::Ui, inv: &Inventory, st: &mut InventoryUi, filtered: bool) -> bool {
    // The explorer lists one folder flat (the Legacy Viewer's single folder
    // mode): its folders never open in place, so Left and Right stay unused.
    let explorer = st.aurora.is_some_and(|frame| frame.explorer);
    st.shown_order = if filtered {
        ShownOrder::Results
    } else if explorer {
        ShownOrder::Flat
    } else {
        ShownOrder::Plain
    };
    let selected_after_delete = delete::reconcile_selection(inv, st);
    // A click starts a new prefix, as LLFolderView::handleMouseDown clears
    // mSearchString, and so do Escape, F2, Home/End, Page Up/Down and any
    // arrow once handled (LLFolderView::handleKeyHere), even when the click
    // gives the keyboard to another field.
    let cleared = ui.input(|i| {
        i.pointer.any_pressed()
            || i.key_pressed(egui::Key::Escape)
            || i.key_pressed(egui::Key::F2)
            || i.key_pressed(egui::Key::Home)
            || i.key_pressed(egui::Key::End)
            || i.key_pressed(egui::Key::PageUp)
            || i.key_pressed(egui::Key::PageDown)
    });
    if cleared {
        st.type_ahead.text.clear();
    }
    if !clipboard::list_has_focus(ui, st) || st.rename.is_some() {
        return selected_after_delete;
    }
    let typed = type_ahead(ui, inv, st, filtered);
    let mut keys = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
                && !modifiers.alt
                && !modifiers.ctrl
                && !modifiers.command
            {
                let arrow = match key {
                    egui::Key::ArrowUp => Arrow::Up,
                    egui::Key::ArrowDown => Arrow::Down,
                    egui::Key::ArrowLeft if !explorer => Arrow::Left,
                    egui::Key::ArrowRight if !explorer => Arrow::Right,
                    _ => return true,
                };
                keys.push((arrow, modifiers.shift));
                false
            } else {
                true
            }
        });
    });
    let navigated = !keys.is_empty();
    for (arrow, shift) in keys {
        match arrow {
            Arrow::Up | Arrow::Down => step(inv, st, filtered, arrow == Arrow::Down, shift),
            Arrow::Left | Arrow::Right => fold(inv, st, filtered, arrow == Arrow::Right),
        }
    }
    if navigated {
        // LLFolderView::handleKeyHere clears the prefix after moving the
        // selection with an arrow.
        st.type_ahead.text.clear();
    }
    typed || navigated || selected_after_delete
}

/// LLFolderView::handleUnicodeCharHere and LLFolderView::search: append the
/// typed text to the window's prefix, forget it after the timeout or a click,
/// and select the nearest visible row that starts with it.
fn type_ahead(ui: &egui::Ui, inv: &Inventory, st: &mut InventoryUi, filtered: bool) -> bool {
    let mut typed = String::new();
    ui.input_mut(|input| {
        input.events.retain(|event| match event {
            egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty() => {
                typed.push_str(text);
                false
            }
            _ => true,
        });
    });
    if typed.is_empty() {
        return false;
    }
    let now = ui.input(|i| i.time);
    if now - st.type_ahead.at > TYPE_AHEAD_TIMEOUT {
        st.type_ahead.text.clear();
    }
    st.type_ahead.at = now;
    let room = TYPE_AHEAD_MAX.saturating_sub(st.type_ahead.text.chars().count());
    st.type_ahead.text.extend(typed.chars().take(room));
    search(inv, st, filtered)
}

/// The nearest match from the selected row, wrapping once, as
/// LLFolderView::search walks the open nodes from `getCurSelectedItem`.
fn search(inv: &Inventory, st: &mut InventoryUi, filtered: bool) -> bool {
    let typed = st.type_ahead.text.clone();
    if typed.is_empty() {
        return false;
    }
    let order = if filtered { &st.result_tree.order } else { &st.plain.order };
    if order.is_empty() {
        return false;
    }
    let start = st.selected.and_then(|id| order.iter().position(|row| *row == id)).unwrap_or(0);
    let found = (0..order.len())
        .map(|offset| order[(start + offset) % order.len()])
        .find(|id| matches(row_label(inv, *id).as_ref(), &typed));
    let Some(id) = found else {
        return false;
    };
    st.selection.clear();
    st.selection.insert(id);
    st.anchor = Some(id);
    st.selected = Some(id);
    st.keyboard_selection = true;
    true
}

/// The name a row shows: a folder's label (the special folders included) or an
/// item's stored name, borrowed so a keystroke does not copy every row.
fn row_label<'a>(inv: &'a Inventory, id: Uuid) -> std::borrow::Cow<'a, str> {
    if inv.folders.contains_key(&id) {
        super::folder_label(inv, id)
    } else {
        std::borrow::Cow::Borrowed(rules::name_ref(inv, id).unwrap_or_default())
    }
}

/// LLFolderView::search compares the name and the typed text up to the shorter
/// of the two (so typing past the end of a matching name keeps it selected),
/// case-insensitively. ASCII takes the fast byte path; a name or a typed letter
/// outside ASCII is lowercased character by character instead.
fn matches(name: &str, typed: &str) -> bool {
    if name.is_empty() || typed.is_empty() {
        return false;
    }
    if name.is_ascii() && typed.is_ascii() {
        let len = name.len().min(typed.len());
        return name.as_bytes()[..len].eq_ignore_ascii_case(&typed.as_bytes()[..len]);
    }
    let mut name = name.chars().flat_map(char::to_lowercase);
    let mut typed = typed.chars().flat_map(char::to_lowercase);
    loop {
        match (name.next(), typed.next()) {
            (Some(a), Some(b)) if a == b => {}
            (None, _) | (_, None) => return true,
            _ => return false,
        }
    }
}

/// The rows the list drew last: the plain tree, Aurora's explorer folder or
/// the result tree of a search, a filter or another tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ShownOrder {
    #[default]
    Plain,
    Flat,
    Results,
}

/// The actual displayed order, including when the explorer temporarily swaps
/// its rows into `plain`. Deletion without a confirmation can start there.
pub(super) fn shown_order(st: &InventoryUi) -> &[Uuid] {
    match st.shown_order {
        ShownOrder::Plain => &st.plain.order,
        ShownOrder::Flat if st.aurora.is_some_and(|frame| frame.explorer) => &st.plain.order,
        ShownOrder::Flat => &st.flat.order,
        ShownOrder::Results => &st.result_tree.order,
    }
}

/// LLInventoryPanel::handleKeyHere (indra/newview/llinventorypanel.cpp): Enter
/// with no modifier opens the selected rows, `LLInventoryAction::doToSelected`
/// with "open". An item follows its double-click (`LLItemBridge::openItem`:
/// a wearable is put on or taken off, a texture previews, a landmark
/// teleports); a folder folds or unfolds, as `LLFolderBridge::performAction`.
pub(super) fn open_shortcut(
    ui: &mut egui::Ui,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let pressed = ui.input_mut(|input| {
        let mut pressed = false;
        input.events.retain(|event| {
            if matches!(event, egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none())
            {
                pressed = true;
                false
            } else {
                true
            }
        });
        pressed
    });
    if pressed {
        open_selected(inv, st, prefs, facts, actions);
        // An opened item changes the rows (a worn tag, a new window): keep the
        // list's keyboard so a second Enter reaches it again.
        clipboard::request_focus(ui, st);
    }
}

fn open_selected(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts, actions: &mut Vec<InvAction>) {
    let mut ids: Vec<_> = st.selection.iter().copied().collect();
    ids.sort_unstable();
    // The Legacy Viewer checks the whole selection for Marketplace wear before opening anything.
    if ids.iter().any(|id| context::listing_wear(inv, *id, st)) {
        return;
    }
    let mut opened = HashSet::new();
    for &id in &ids {
        if let Some(original) = rules::original(inv, id) {
            // A selected item and its links all describe the same action. Prefer its own row.
            if (id != original && ids.binary_search(&original).is_ok()) || !opened.insert(original) {
                continue;
            }
        }
        if inv.folders.contains_key(&id) {
            // The explorer steps into a folder; anywhere else Enter folds it.
            if st.aurora.is_some_and(|frame| frame.explorer) {
                st.browse = id;
                st.selection.clear();
                st.selected = None;
            } else if st.result_view() {
                st.result_tree.toggle(id, st.tab);
            } else {
                st.plain.set_open(id, !st.plain.is_open(id));
            }
        } else {
            context::double_click(inv, id, st, prefs, facts, actions);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

/// KEY_RIGHT / KEY_LEFT of LLFolderView::handleKeyHere, on the current row
/// only: never more than that one folder opens or closes.
fn fold(inv: &Inventory, st: &mut InventoryUi, filtered: bool, right: bool) {
    let Some(current) = st.selected.filter(|id| st.selection.contains(id)) else {
        return;
    };
    let folder = inv.folders.contains_key(&current);
    let open = folder
        && if filtered {
            st.result_tree.is_open(current, st.tab)
        } else {
            st.plain.is_open(current)
        };
    if right || open {
        if folder {
            if filtered {
                st.result_tree.set_open(current, st.tab, right);
            } else {
                st.plain.set_open(current, right);
            }
        }
        return;
    }
    // The parent only when it is a row: the top folders under a hidden
    // root stay put, like those under the Legacy Viewer's own root.
    let order = if filtered { &st.result_tree.order } else { &st.plain.order };
    if let Some(parent) = rules::parent(inv, current).filter(|parent| order.contains(parent)) {
        st.selection.clear();
        st.selection.insert(parent);
        st.anchor = Some(parent);
        st.selected = Some(parent);
        st.keyboard_selection = true;
    }
}

/// LLFolderView::scrollToShowSelection: leave visible rows in place and move
/// only enough to bring an offscreen row fully into the virtualized viewport.
pub(super) fn scroll_offset(st: &InventoryUi, index: usize, row_height: f32, pitch: f32, height: f32) -> f32 {
    let (offset, viewport) = st.list_viewport.unwrap_or((0.0, height));
    let height = viewport.min(height).max(row_height);
    let top = index as f32 * pitch;
    offset.min(top).max(top + row_height - height).max(0.0)
}

fn step(inv: &Inventory, st: &mut InventoryUi, filtered: bool, down: bool, shift: bool) {
    let order = if filtered { &st.result_tree.order } else { &st.plain.order };
    let Some(current) = st.selected.filter(|id| st.selection.contains(id)) else {
        return;
    };
    let Some(index) = order.iter().position(|id| *id == current) else {
        return;
    };
    let parent = rules::parent(inv, current);
    let eligible = |id: &&Uuid| !shift || rules::parent(inv, **id) == parent;
    let next = if down {
        order[index + 1..].iter().find(eligible)
    } else {
        order[..index].iter().rev().find(eligible)
    }
    .copied();
    // The first arrow after a mouse selection starts from the clicked row,
    // even when Ctrl-click previously selected unrelated rows.
    if !st.keyboard_selection {
        st.selection.clear();
        st.selection.insert(current);
        st.anchor = Some(current);
        st.keyboard_selection = true;
    }
    if let Some(next) = next {
        if shift {
            if st.selection.contains(&next) {
                st.selection.remove(&current);
            } else {
                st.selection.insert(next);
            }
        } else {
            st.selection.clear();
            st.selection.insert(next);
            st.anchor = Some(next);
        }
        st.selected = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_skips_descendants_and_stops_at_sibling_boundaries() {
        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.created_folders(
            [(10, 1), (11, 10), (20, 1), (30, 1)]
                .map(|(n, parent)| aurora_net::inventory::InvFolder {
                    id: id(n),
                    parent: id(parent),
                    ..Default::default()
                })
                .to_vec(),
        );
        let mut st = InventoryUi {
            selected: Some(id(10)),
            selection: HashSet::from([id(10)]),
            ..Default::default()
        };
        st.plain.order = vec![id(10), id(11), id(20), id(30)];
        step(&inv, &mut st, false, true, true);
        assert_eq!(st.selection, HashSet::from([id(10), id(20)]));
        step(&inv, &mut st, false, true, true);
        step(&inv, &mut st, false, true, true);
        assert_eq!(st.selection, HashSet::from([id(10), id(20), id(30)]));
        assert_eq!(st.selected, Some(id(30)));
        for _ in 0..3 {
            step(&inv, &mut st, false, false, true);
        }
        assert_eq!(st.selection, HashSet::from([id(10)]));
        assert_eq!(st.selected, Some(id(10)));
        step(&inv, &mut st, false, true, false);
        assert_eq!(st.selection, HashSet::from([id(11)]), "plain Down enters the open folder");
        step(&inv, &mut st, false, true, true);
        assert_eq!(st.selection, HashSet::from([id(11)]), "Shift cannot leave its parent");
        st.result_tree.order = vec![id(30), id(20), id(10)];
        st.selected = Some(id(30));
        st.selection = HashSet::from([id(30)]);
        step(&inv, &mut st, true, true, true);
        assert_eq!(
            st.selection,
            HashSet::from([id(30), id(20)]),
            "follow the actual filtered/sorted order"
        );
    }

    #[test]
    fn left_and_right_open_close_and_climb_one_folder_at_a_time() {
        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.created_folders(
            [(10, 1), (11, 10), (20, 1)]
                .map(|(n, parent)| aurora_net::inventory::InvFolder {
                    id: id(n),
                    parent: id(parent),
                    ..Default::default()
                })
                .to_vec(),
        );
        inv.items.insert(
            id(12),
            aurora_net::inventory::InvItem {
                id: id(12),
                parent: id(11),
                ..Default::default()
            },
        );
        let select = |st: &mut InventoryUi, n| {
            st.selected = Some(id(n));
            st.selection = HashSet::from([id(n)]);
        };
        let mut st = InventoryUi::default();
        // The root (1) is hidden: 10 and 20 are top rows.
        st.plain.order = vec![id(10), id(20)];
        select(&mut st, 10);
        fold(&inv, &mut st, false, true);
        assert!(st.plain.is_open(id(10)));
        assert!(!st.plain.is_open(id(20)), "only the current folder opens");
        fold(&inv, &mut st, false, true);
        assert!(st.plain.is_open(id(10)), "Right again keeps it open");
        st.plain.order = vec![id(10), id(11), id(20)];
        select(&mut st, 11);
        fold(&inv, &mut st, false, true);
        st.plain.order = vec![id(10), id(11), id(12), id(20)];
        select(&mut st, 12);
        fold(&inv, &mut st, false, true);
        assert_eq!(st.selected, Some(id(12)), "Right on an item does nothing");
        fold(&inv, &mut st, false, false);
        assert_eq!(st.selection, HashSet::from([id(11)]), "Left on an item selects its folder");
        assert!(st.plain.is_open(id(11)), "and leaves that folder open");
        fold(&inv, &mut st, false, false);
        assert!(!st.plain.is_open(id(11)), "a second Left closes it");
        assert_eq!(st.selected, Some(id(11)));
        assert!(st.plain.is_open(id(10)), "its parent stays open");
        fold(&inv, &mut st, false, false);
        assert_eq!(st.selection, HashSet::from([id(10)]), "Left on a closed folder climbs");
        fold(&inv, &mut st, false, false);
        assert!(!st.plain.is_open(id(10)));
        fold(&inv, &mut st, false, false);
        assert_eq!(st.selected, Some(id(10)), "no row above a top folder of a hidden root");
        assert!(!st.plain.is_open(id(1)));

        // A search's result tree keeps its own open state per tab.
        st.result_tree.order = vec![id(10), id(11), id(12)];
        select(&mut st, 11);
        assert!(st.result_tree.is_open(id(11), st.tab));
        fold(&inv, &mut st, true, false);
        assert!(!st.result_tree.is_open(id(11), st.tab));
        assert!(st.result_tree.is_open(id(10), st.tab));
        fold(&inv, &mut st, true, false);
        assert_eq!(st.selected, Some(id(10)));
        fold(&inv, &mut st, true, true);
        fold(&inv, &mut st, true, false);
        assert!(!st.result_tree.is_open(id(10), st.tab));
    }

    #[test]
    fn hidden_or_deselected_active_rows_do_not_change_selection() {
        let id = Uuid::from_u128;
        let inv = Inventory::default();
        let mut st = InventoryUi {
            selected: Some(id(10)),
            selection: HashSet::from([id(10)]),
            ..Default::default()
        };
        st.result_tree.order = vec![id(20)];
        step(&inv, &mut st, true, true, true);
        assert_eq!(st.selection, HashSet::from([id(10)]));
        st.plain.order = vec![id(10), id(20)];
        st.selection.clear();
        step(&inv, &mut st, false, true, true);
        assert!(st.selection.is_empty());
    }

    #[test]
    fn enter_puts_on_or_takes_off_the_selected_wearable() {
        use crate::world::appearance::Action as Wear;
        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.add_items(vec![aurora_net::inventory::InvItem {
            id: id(100),
            parent: id(1),
            asset_type: 6,
            ..Default::default()
        }]);
        let prefs = InventoryPreferences::default();
        let mut facts = Facts::default();
        let mut st = InventoryUi {
            selected: Some(id(100)),
            selection: HashSet::from([id(100)]),
            ..Default::default()
        };
        let mut actions = Vec::new();
        open_selected(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(
            matches!(&actions[..], [InvAction::Appearance(Wear::WearItem { item, replace: true, point: 0 })] if *item == id(100)),
            "not worn: put it on"
        );
        facts.worn.insert(id(100));
        let mut actions = Vec::new();
        open_selected(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(
            matches!(&actions[..], [InvAction::Appearance(Wear::Remove(item))] if *item == id(100)),
            "worn: take it off"
        );
    }

    #[test]
    fn enter_opens_each_link_target_once_and_reveals_a_linked_folder() {
        use crate::world::appearance::Action as Wear;
        use aurora_net::inventory::{InvFolder, InvItem};

        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.created_folders(vec![InvFolder {
            id: id(30),
            ..Default::default()
        }]);
        inv.add_items(vec![
            InvItem {
                id: id(20),
                asset_type: 6,
                ..Default::default()
            },
            InvItem {
                id: id(10),
                asset_type: 24,
                asset_id: id(20),
                ..Default::default()
            },
            InvItem {
                id: id(40),
                asset_type: 24,
                asset_id: id(20),
                ..Default::default()
            },
            InvItem {
                id: id(50),
                asset_type: 25,
                asset_id: id(30),
                ..Default::default()
            },
        ]);
        let mut st = InventoryUi::default();
        for rows in [vec![id(10), id(20), id(40)], vec![id(10), id(40)]] {
            st.selection = rows.into_iter().collect();
            let mut actions = Vec::new();
            open_selected(&inv, &mut st, &Default::default(), &Default::default(), &mut actions);
            assert!(matches!(&actions[..], [InvAction::Appearance(Wear::WearItem { item, .. })] if *item == id(20)));
        }

        st.selection = HashSet::from([id(50)]);
        open_selected(&inv, &mut st, &Default::default(), &Default::default(), &mut Vec::new());
        assert_eq!(st.reveal, Some(id(30)));
        assert_eq!(st.selection, HashSet::from([id(30)]));
    }

    #[test]
    fn enter_rejects_the_whole_selection_when_a_marketplace_link_cannot_be_worn() {
        use aurora_net::inventory::{InvFolder, InvItem};
        let id = Uuid::from_u128;
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.created_folders(vec![
            InvFolder {
                id: inv.root,
                type_default: 8,
                ..Default::default()
            },
            InvFolder {
                id: id(30),
                parent: inv.root,
                type_default: crate::world::marketplace::LISTINGS,
                ..Default::default()
            },
        ]);
        inv.add_items(vec![
            InvItem {
                id: id(10),
                parent: inv.root,
                asset_type: 6,
                ..Default::default()
            },
            InvItem {
                id: id(20),
                parent: id(30),
                asset_type: 24,
                asset_id: id(10),
                ..Default::default()
            },
        ]);
        let mut st = InventoryUi {
            selection: HashSet::from([id(10), id(20)]),
            ..Default::default()
        };
        let mut actions = Vec::new();
        open_selected(&inv, &mut st, &Default::default(), &Default::default(), &mut actions);
        assert!(actions.is_empty());
        assert_eq!(st.message, aurora_i18n::tr!("marketplace-alert-cannot-wear"));
    }

    #[test]
    fn enter_folds_a_folder_and_opens_every_selected_item() {
        use crate::world::appearance::Action as Wear;
        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.created_folders(vec![aurora_net::inventory::InvFolder {
            id: id(10),
            parent: id(1),
            ..Default::default()
        }]);
        inv.add_items(vec![aurora_net::inventory::InvItem {
            id: id(100),
            parent: id(10),
            asset_type: 6,
            ..Default::default()
        }]);
        let prefs = InventoryPreferences::default();
        let facts = Facts::default();
        let mut st = InventoryUi {
            selected: Some(id(10)),
            selection: HashSet::from([id(10), id(100)]),
            ..Default::default()
        };
        let mut actions = Vec::new();
        open_selected(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(st.plain.is_open(id(10)), "the folder folds open");
        assert!(
            matches!(&actions[..], [InvAction::Appearance(Wear::WearItem { item, .. })] if *item == id(100)),
            "the item under it is put on"
        );
        let mut actions = Vec::new();
        open_selected(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(!st.plain.is_open(id(10)), "a second Enter folds it closed");
    }

    #[test]
    fn type_ahead_matches_a_prefix_case_insensitively_and_past_the_name() {
        assert!(matches("Tarte", "tar"));
        assert!(matches("Tarte", "TAR"));
        assert!(matches("Tarte", "tart"));
        assert!(matches("Tarte", "tartes"), "typing past the name keeps it");
        assert!(!matches("Tarte", "tarz"));
        assert!(!matches("Tarte", ""));
        assert!(!matches("", "tar"));
        // The Legacy Viewer ignores non-ASCII typed text; Aurora matches it.
        assert!(matches("Élodie", "él"));
        assert!(matches("élodie", "ÉL"));
        assert!(!matches("Élodie", "el"));
    }

    #[test]
    fn type_ahead_selects_items_and_folders_from_the_selection_and_wraps() {
        use aurora_net::inventory::{InvFolder, InvItem};
        let id = Uuid::from_u128;
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.created_folders(vec![
            InvFolder {
                id: id(1),
                type_default: 8,
                ..Default::default()
            },
            InvFolder {
                id: id(10),
                parent: id(1),
                name: "Objects".into(),
                ..Default::default()
            },
            InvFolder {
                id: id(20),
                parent: id(1),
                name: "Scripts".into(),
                ..Default::default()
            },
            InvFolder {
                id: id(30),
                parent: id(1),
                name: "Textures".into(),
                ..Default::default()
            },
            InvFolder {
                id: id(40),
                parent: id(1),
                name: "Tarte".into(),
                ..Default::default()
            },
        ]);
        inv.add_items(vec![InvItem {
            id: id(100),
            parent: id(1),
            name: "Tartan".into(),
            ..Default::default()
        }]);
        let mut st = InventoryUi {
            selected: Some(id(40)),
            selection: HashSet::from([id(40)]),
            ..Default::default()
        };
        st.plain.order = vec![id(10), id(20), id(30), id(40), id(100)];

        st.type_ahead.text = "scr".into();
        assert!(search(&inv, &mut st, false), "a folder after the selection");
        assert_eq!(st.selected, Some(id(20)));
        assert_eq!(st.selection, HashSet::from([id(20)]));

        st.type_ahead.text = "tex".into();
        assert!(search(&inv, &mut st, false), "the search wraps around the list");
        assert_eq!(st.selected, Some(id(30)));

        st.type_ahead.text = "tar".into();
        assert!(search(&inv, &mut st, false), "items match as well as folders");
        assert_eq!(st.selected, Some(id(40)), "the nearest match from the selection");

        st.type_ahead.text = "tart".into();
        assert!(search(&inv, &mut st, false));
        assert_eq!(st.selected, Some(id(40)), "Tarte still starts with 'tart'");

        let before = st.selected;
        st.type_ahead.text = "zz".into();
        assert!(!search(&inv, &mut st, false));
        assert_eq!(st.selected, before, "no match leaves the selection alone");
    }

    #[test]
    fn type_ahead_searches_the_result_tree_when_filtered() {
        use aurora_net::inventory::InvItem;
        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.add_items(vec![
            InvItem {
                id: id(100),
                name: "Tarte".into(),
                ..Default::default()
            },
            InvItem {
                id: id(101),
                name: "Tartan".into(),
                ..Default::default()
            },
        ]);
        let mut st = InventoryUi::default();
        st.plain.order = vec![id(100)];
        st.result_tree.order = vec![id(100), id(101)];
        st.selected = Some(id(100));
        st.type_ahead.text = "tartan".into();
        assert!(search(&inv, &mut st, true));
        assert_eq!(st.selected, Some(id(101)), "the result tree, not the plain one");
        let mut plain = InventoryUi::default();
        plain.plain.order = vec![id(100)];
        plain.type_ahead.text = "tartan".into();
        assert!(!search(&inv, &mut plain, false), "the plain tree has no Tartan row");
    }
}
