//! Port of LLFolderView::handleKeyHere (indra/llui/llfolderview.cpp,
//! originally LGPL 2.1): arrows follow displayed rows; Shift grows/shrinks
//! sibling selections without selecting an open folder's children as well.

use super::*;

/// Run after rebuilding the displayed order and before laying out its viewport,
/// including while Aurora's explorer has swapped its flat tree into `plain`.
pub(super) fn navigate(ui: &egui::Ui, inv: &Inventory, st: &mut InventoryUi, filtered: bool) -> bool {
    if !clipboard::list_has_focus(ui, st) || st.rename.is_some() {
        return false;
    }
    let mut keys = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Key {
                key: key @ (egui::Key::ArrowUp | egui::Key::ArrowDown),
                pressed: true,
                modifiers,
                ..
            } = event
                && !modifiers.alt
                && !modifiers.ctrl
                && !modifiers.command
            {
                keys.push((*key == egui::Key::ArrowDown, modifiers.shift));
                false
            } else {
                true
            }
        });
    });
    let navigated = !keys.is_empty();
    for (down, shift) in keys {
        step(inv, st, filtered, down, shift);
    }
    navigated
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
}
