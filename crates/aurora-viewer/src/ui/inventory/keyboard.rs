//! Port of LLFolderView::handleKeyHere (indra/llui/llfolderview.cpp,
//! originally LGPL 2.1): arrows follow displayed rows; Shift grows/shrinks
//! sibling selections without selecting an open folder's children as well.
//! Right opens the current folder; Left closes it, or from an item or a
//! closed folder selects the parent row (a second Left then closes it).
//! Letters typed into the list jump to the first row whose name starts with
//! them (handleUnicodeCharHere and search, same file).

use super::*;

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
    if !clipboard::list_has_focus(ui, st) || st.rename.is_some() {
        return selected_after_delete;
    }
    let mut keys = Vec::new();
    let mut typed = String::new();
    let now = ui.input(|i| i.time);
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Text(text) = event {
                typed.extend(text.chars().filter(|c| !c.is_control()));
                return false;
            }
            if let egui::Event::Key {
                key: egui::Key::Escape | egui::Key::Home | egui::Key::End | egui::Key::PageUp | egui::Key::PageDown,
                pressed: true,
                ..
            } = event
            {
                st.type_ahead.clear();
            }
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
    let mut navigated = !keys.is_empty();
    if navigated {
        st.type_ahead.clear();
    }
    for (arrow, shift) in keys {
        match arrow {
            Arrow::Up | Arrow::Down => step(inv, st, filtered, arrow == Arrow::Down, shift),
            Arrow::Left | Arrow::Right => fold(inv, st, filtered, arrow == Arrow::Right),
        }
    }
    if !typed.is_empty() {
        st.type_ahead.push(&typed, now);
        navigated |= type_ahead(inv, st);
    }
    navigated || selected_after_delete
}

/// The Legacy Viewer's `TypeAheadTimeout` default (settings.xml), in seconds.
const TYPE_AHEAD_TIMEOUT: f64 = 1.5;
/// LLFolderView::handleUnicodeCharHere stops growing its string at 128.
const TYPE_AHEAD_MAX: usize = 128;

/// LLFolderView's `mSearchString` and `mSearchTimer`: the letters typed
/// while the list has the keyboard, emptied after a pause, a click, an
/// arrow, Escape, Home, End, Page Up or Page Down, or a rename.
#[derive(Debug, Default)]
pub(super) struct TypeAhead {
    text: String,
    last: f64,
}

impl TypeAhead {
    pub(super) fn clear(&mut self) {
        self.text.clear();
    }

    fn push(&mut self, typed: &str, now: f64) {
        if now - self.last > TYPE_AHEAD_TIMEOUT {
            self.text.clear();
        }
        self.last = now;
        let room = TYPE_AHEAD_MAX.saturating_sub(self.text.chars().count());
        self.text.extend(typed.chars().take(room));
    }
}

/// The name a row shows, as LLFolderViewModelItem::getSearchableName gives it.
fn row_name(inv: &Inventory, id: Uuid) -> String {
    match inv.items.get(&id) {
        Some(item) => shown_name(item).to_owned(),
        None => folder_name(inv, id),
    }
}

/// Port of LLFolderView::search (indra/llui/llfolderview.cpp, originally
/// LGPL 2.1), called from handleUnicodeCharHere: from the current row
/// itself, the first shown row whose name starts with the typed letters,
/// ignoring case and wrapping once, becomes the only selection. The Legacy
/// Viewer ignores letters past ASCII; any letter matches here, so Japanese
/// and accented names can be reached too.
fn type_ahead(inv: &Inventory, st: &mut InventoryUi) -> bool {
    let typed = st.type_ahead.text.to_lowercase();
    let order = shown_order(st);
    let start = st
        .selected
        .filter(|id| st.selection.contains(id))
        .and_then(|current| order.iter().position(|id| *id == current))
        .unwrap_or(0);
    let Some(found) = order[start..]
        .iter()
        .chain(&order[..start])
        .find(|id| row_name(inv, **id).to_lowercase().starts_with(&typed))
        .copied()
    else {
        return false;
    };
    st.selection.clear();
    st.selection.insert(found);
    st.anchor = Some(found);
    st.selected = Some(found);
    st.keyboard_selection = true;
    true
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
    fn type_ahead_grows_until_a_pause_and_stops_at_the_legacy_limit() {
        let mut ahead = TypeAhead::default();
        ahead.push("b", 10.0);
        ahead.push("o", 11.0);
        assert_eq!(ahead.text, "bo");
        ahead.push("x", 12.6);
        assert_eq!(ahead.text, "x", "more than 1.5 s after the last letter starts again");
        ahead.clear();
        assert!(ahead.text.is_empty());
        ahead.push(&"a".repeat(200), 13.0);
        assert_eq!(ahead.text.chars().count(), TYPE_AHEAD_MAX);
    }

    #[test]
    fn type_ahead_selects_the_next_shown_match_from_the_current_row() {
        let id = Uuid::from_u128;
        let mut inv = Inventory::default();
        inv.created_folders(vec![aurora_net::inventory::InvFolder {
            id: id(10),
            parent: id(1),
            name: "Body Parts".into(),
            ..Default::default()
        }]);
        for (n, name) in [(11, "Apple"), (12, "banana"), (13, "Berry"), (14, "Été")] {
            inv.items.insert(
                id(n),
                aurora_net::inventory::InvItem {
                    id: id(n),
                    parent: id(1),
                    name: name.into(),
                    ..Default::default()
                },
            );
        }
        let mut st = InventoryUi::default();
        st.plain.order = [11, 10, 12, 13, 14].map(id).to_vec();
        let typed = |st: &mut InventoryUi, text: &str| {
            st.type_ahead.clear();
            st.type_ahead.push(text, 0.0);
            type_ahead(&inv, st)
        };

        assert!(typed(&mut st, "b"), "nothing selected: search from the top");
        assert_eq!(st.selected, Some(id(10)), "folders match by their shown name");
        assert_eq!(st.selection, HashSet::from([id(10)]));
        assert_eq!(st.anchor, Some(id(10)));
        assert!(st.keyboard_selection);
        assert!(typed(&mut st, "B"));
        assert_eq!(st.selected, Some(id(10)), "the current row itself still matches");
        assert!(typed(&mut st, "ber"));
        assert_eq!(st.selected, Some(id(13)), "a longer prefix moves on, case ignored");
        assert!(typed(&mut st, "a"));
        assert_eq!(st.selected, Some(id(11)), "wraps to the top");
        assert!(typed(&mut st, "ét"));
        assert_eq!(st.selected, Some(id(14)), "letters past ASCII match too");

        st.selection = HashSet::from([id(11), id(12)]);
        assert!(!typed(&mut st, "z"), "no match leaves the selection alone");
        assert_eq!(st.selection, HashSet::from([id(11), id(12)]));

        st.result_tree.order = [13, 12].map(id).to_vec();
        st.shown_order = ShownOrder::Results;
        assert!(typed(&mut st, "b"));
        assert_eq!(st.selected, Some(id(13)), "a search's rows in their own order");
        assert!(!typed(&mut st, "body"), "rows hidden by the search are skipped");
    }
}
