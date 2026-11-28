//! Port of LLFolderView::handleKeyHere (indra/llui/llfolderview.cpp,
//! originally LGPL 2.1): arrows follow displayed rows; Shift grows/shrinks
//! sibling selections without selecting an open folder's children as well.
//! Right opens the current folder; Left closes it, or from an item or a
//! closed folder selects the parent row (a second Left then closes it).

use super::shortcuts;
use super::*;

/// Run after rebuilding the displayed order and before laying out its viewport,
/// including while Aurora's explorer has swapped its flat tree into `plain`.
pub(super) fn navigate(ui: &egui::Ui, inv: &Inventory, st: &mut InventoryUi, filtered: bool) -> bool {
    // The explorer and the Legacy-style single folder window list one folder
    // flat (the Legacy Viewer's single folder mode): its folders never open
    // in place, so Left and Right stay unused.
    let explorer = st.lists_flat();
    st.shown_order = order_kind(st, filtered);
    let selected_after_delete = delete::reconcile_selection(inv, st);
    // A click starts a new prefix, as LLFolderView::handleMouseDown does for a
    // press on the folder view. Aurora also clears it when the click gives the
    // keyboard to another field (harmless: the prefix only counts while the
    // list has the keyboard).
    if ui.input(|i| i.pointer.any_pressed()) {
        st.type_ahead.clear();
    }
    if !clipboard::list_has_focus(ui, st) || st.rename.is_some() {
        return selected_after_delete;
    }
    let selected_by_key = select_keys::handle(ui, inv, st, explorer);
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
    if navigated {
        select_keys::adopt_focus(st);
    }
    for (arrow, shift) in keys {
        match arrow {
            Arrow::Up | Arrow::Down => step(inv, st, filtered, arrow == Arrow::Down, shift),
            Arrow::Left | Arrow::Right => fold(inv, st, filtered, arrow == Arrow::Right),
        }
    }
    navigated || selected_after_delete || selected_by_key
}

/// The list's file-manager commands that act on the selection: a new
/// folder (Ctrl+Shift+N, then its name typed as `created_selection` does
/// for the New menu), its properties (Alt+Enter), Shift+Delete, and the
/// history's Ctrl+Z / Ctrl+Y.
///
/// Shift+Delete purges (PurgeSelectedItems, with its confirmation that has
/// no "don't ask again") only what is already in the Trash: Second Life
/// only purges from there (LLInvFVBridge::isItemRemovable /
/// "Purge Item"), so anywhere else it moves to the Trash as Delete does.
/// Windows Explorer's Shift+Delete deletes anywhere; the Second Life rule
/// wins.
pub(super) fn commands(
    ui: &mut egui::Ui,
    inv: &Inventory,
    root: Uuid,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let wanted = |c: shortcuts::Command| {
        matches!(
            c,
            shortcuts::Command::NewFolder
                | shortcuts::Command::Properties
                | shortcuts::Command::Purge
                | shortcuts::Command::ContextMenu
                | shortcuts::Command::Refresh
                | shortcuts::Command::Undo
                | shortcuts::Command::Redo
        )
    };
    for command in shortcuts::take(ui, wanted) {
        match command {
            shortcuts::Command::NewFolder => {
                let (to, in_all) = st.new_folder_target(inv, root, st.aurora.map(|frame| frame.layout));
                if st.pending.is_some() || !rules::destination(inv, to) {
                    continue;
                }
                context::new_folder(st, actions, to, in_all);
            }
            // Explorer's Alt+Enter: the item's properties in a window of
            // its own, as many as asked for, the list free behind them
            shortcuts::Command::Properties => {
                if let Some(id) = st.selected.filter(|id| st.selection.contains(id)) {
                    context_catalog::properties_windows(st, vec![id]);
                }
            }
            // Shift+F10: the selection's menu at its row, else the menu of
            // the explorer's empty space (Windows' keyboard menu)
            shortcuts::Command::ContextMenu if st.aurora.is_some() => {
                let explorer = st.aurora.is_some_and(|frame| frame.layout != aurora::Layout::Compact);
                match st.selected.filter(|id| st.selection.contains(id)) {
                    Some(id) => {
                        let at = st.selected_rect.map_or_else(|| ui.max_rect().center(), |rect| rect.left_bottom());
                        st.row_menu = Some((id, at));
                        st.background_menu = false;
                    }
                    None if explorer && st.tab == 0 => {
                        st.row_menu = Some((root, ui.max_rect().center()));
                        st.background_menu = true;
                    }
                    None => {}
                }
            }
            // F5 in the tree and the Legacy-style window: "Reload folder" of
            // the selected folder (the explorer's own F5 reloads the folder
            // shown, `aurora::history`)
            shortcuts::Command::Refresh => {
                let folder = st.selected.filter(|id| inv.folders.contains_key(id)).unwrap_or(root);
                if folder != inv.root && folder != inv.lib_root && inv.folders.contains_key(&folder) {
                    st.message.clear();
                    actions.push(InvAction::Preview(folder, Some(clipboard::focus_id(st))));
                }
            }
            shortcuts::Command::Purge => purge(inv, st, prefs, facts, actions),
            // the history is the session's, not this list's: the app takes
            // its next step (`world::inventory::undo`)
            // while a change is on its way the app says to wait
            shortcuts::Command::Undo | shortcuts::Command::Redo => {
                actions.push(InvAction::History {
                    undo: command == shortcuts::Command::Undo,
                    named: false,
                });
            }
            _ => {}
        }
    }
}

/// Shift+Delete: what is in the Trash is purged after its confirmation,
/// anything else goes to the Trash as Delete does (see `commands`).
pub(super) fn purge(inv: &Inventory, st: &mut InventoryUi, prefs: &InventoryPreferences, facts: &Facts, actions: &mut Vec<InvAction>) {
    let ids: Vec<Uuid> = st.selection.iter().copied().collect();
    if ids.is_empty() || st.pending.is_some() || st.pending_delete.is_some() {
        return;
    }
    let in_trash = ids.iter().all(|id| rules::in_type(inv, *id, 14));
    if in_trash && delete::allowed(inv, &ids, prefs, facts) {
        st.dialog = Some(EditDialog::Delete(ids, true));
    } else {
        delete::request(inv, st, prefs, facts, actions);
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

/// Which list holds the rows shown. Only Aurora's explorer keeps its rows in
/// `flat` (swapped in while it draws); the Legacy-style single folder window
/// lists its folder in `plain`, like the tree, so that a delete there finds
/// the next row (LLFolderView::getNextUnselectedItem).
pub(super) fn order_kind(st: &InventoryUi, filtered: bool) -> ShownOrder {
    if filtered {
        ShownOrder::Results
    } else if st.aurora.is_some_and(|frame| frame.explorer) {
        ShownOrder::Flat
    } else {
        ShownOrder::Plain
    }
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
            // A single folder list steps into a folder; anywhere else Enter
            // folds it.
            if st.lists_flat() {
                st.browse = id;
                st.selection.clear();
                st.selected = None;
            } else if st.result_view() {
                // a folder without a caret stays shut (`rows::branches`)
                if st.result_tree.folds(id) {
                    st.result_tree.toggle(id, st.tab);
                }
            } else if st.plain.folds(id) {
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
    // A folder without a caret neither opens nor counts as open: Left goes
    // to its parent (`rows::branches`).
    let folder = inv.folders.contains_key(&current)
        && if filtered {
            st.result_tree.folds(current)
        } else {
            st.plain.folds(current)
        };
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
        st.view.prepare(&inv, view::Sort::default());
        // The root (1) is hidden: 10 and 20 are top rows.
        let list = |st: &mut InventoryUi| {
            st.plain.prepare(&st.view, &inv, id(1), false, true);
            st.plain.order.clone()
        };
        assert_eq!(list(&mut st), [id(10), id(20)]);
        select(&mut st, 10);
        fold(&inv, &mut st, false, true);
        assert!(st.plain.is_open(id(10)));
        assert!(!st.plain.is_open(id(20)), "only the current folder opens");
        fold(&inv, &mut st, false, true);
        assert!(st.plain.is_open(id(10)), "Right again keeps it open");
        assert_eq!(list(&mut st), [id(10), id(11), id(20)]);
        select(&mut st, 11);
        fold(&inv, &mut st, false, true);
        assert_eq!(list(&mut st), [id(10), id(11), id(12), id(20)]);
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

        // An empty folder has no caret: Right leaves it shut, Left climbs.
        list(&mut st);
        select(&mut st, 20);
        fold(&inv, &mut st, false, true);
        assert!(!st.plain.is_open(id(20)), "Right does not open an empty folder");
        open_selected(&inv, &mut st, &InventoryPreferences::default(), &Facts::default(), &mut Vec::new());
        assert!(!st.plain.is_open(id(20)), "nor does Enter");

        // A search's result tree keeps its own open state per tab.
        st.result_tree.order = vec![id(10), id(11), id(12)];
        st.result_tree.rows = [(10, true), (11, true), (12, false)]
            .map(|(n, folder)| view::TreeRow {
                id: id(n),
                depth: 0,
                folder,
                open: folder,
                branches: folder,
            })
            .to_vec();
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
        st.view.prepare(&inv, view::Sort::default());
        st.plain.prepare(&st.view, &inv, id(1), false, true);
        let mut actions = Vec::new();
        open_selected(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(st.plain.is_open(id(10)), "the folder folds open");
        st.plain.prepare(&st.view, &inv, id(1), false, true);
        assert!(
            matches!(&actions[..], [InvAction::Appearance(Wear::WearItem { item, .. })] if *item == id(100)),
            "the item under it is put on"
        );
        let mut actions = Vec::new();
        open_selected(&inv, &mut st, &prefs, &facts, &mut actions);
        assert!(!st.plain.is_open(id(10)), "a second Enter folds it closed");
    }
}
