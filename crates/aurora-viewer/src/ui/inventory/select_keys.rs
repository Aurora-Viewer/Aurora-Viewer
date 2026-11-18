//! The list's selection keys beyond the arrows: Home, End, Page Up, Page
//! Down (Shift extends from the anchor), select all, invert, Esc to select
//! nothing, type-ahead, and in the explorer Ctrl+↑/↓ to move the keyboard's
//! place without selecting and Ctrl+Space to select or unselect there.
//!
//! Type-ahead is a port of LLFolderView::handleUnicodeCharHere and
//! LLFolderView::search (indra/llui/llfolderview.cpp, originally LGPL 2.1):
//! the letters typed within `TypeAheadTimeout` (1.5 s) select the next row
//! whose name starts with them, from the selected row, wrapping at the end.
//! Aurora differences: any character (the Legacy Viewer stops at
//! non-ASCII), and the same letter typed again goes on to the next row
//! that starts with it, as in Windows Explorer and Dolphin. Home, End,
//! Page Up and Page Down only scroll the Legacy Viewer's list
//! (LLFolderView::handleKeyHere); here they select, as in a file manager.
//! Select all, invert (Dolphin's Ctrl+Shift+A), Esc and the keyboard's
//! place apart from the selection (Explorer's Ctrl+arrows, Ctrl+Space) are
//! Aurora's.

use super::shortcuts::{self, Command};
use super::*;

/// LLFolderView's `TypeAheadTimeout` (settings.xml), in seconds.
const TYPE_AHEAD_TIMEOUT: f64 = 1.5;
/// The most characters type-ahead keeps (LLFolderView keeps 128).
const TYPE_AHEAD_MAX: usize = 128;
/// The height a page counts rows by when the list's own is unknown.
const ROW_PITCH: f32 = 26.0;

/// The letters typed so far and when the last one came.
#[derive(Debug, Default)]
pub(super) struct TypeAhead {
    text: String,
    at: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Move {
    First,
    Last,
    PageUp,
    PageDown,
    /// The keyboard's place, one row up or down, the selection kept.
    Focus(bool),
    /// Ctrl+Space: the keyboard's place in or out of the selection.
    Toggle,
    /// Esc: nothing selected.
    Clear,
}

/// The row `typed` leads to among `names` (in shown order), from
/// `current`. `again`: the same single letter typed once more, which goes
/// past the current row; otherwise the current row itself may match.
pub(super) fn type_ahead_target(names: &[(Uuid, String)], current: Option<Uuid>, typed: &str, again: bool) -> Option<Uuid> {
    if typed.is_empty() || names.is_empty() {
        return None;
    }
    let typed = typed.to_lowercase();
    let start = current.and_then(|id| names.iter().position(|(row, _)| *row == id)).unwrap_or(0);
    let skip = usize::from(again && current.is_some());
    (0..names.len())
        .map(|n| (start + skip + n) % names.len())
        .find(|&i| names[i].1.to_lowercase().starts_with(&typed))
        .map(|i| names[i].0)
}

/// The rows Home, End and the page keys lead to: `index` among `len`
/// rows, `page` rows to a page.
fn moved(index: usize, len: usize, page: usize, step: Move) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match step {
        Move::First => 0,
        Move::Last => last,
        Move::PageUp => index.saturating_sub(page),
        Move::PageDown => (index + page).min(last),
        Move::Focus(down) => {
            if down {
                (index + 1).min(last)
            } else {
                index.saturating_sub(1)
            }
        }
        Move::Toggle | Move::Clear => index,
    })
}

/// The selection from `anchor` to `to` in `order`, both ends in.
fn range(order: &[Uuid], anchor: usize, to: usize) -> impl Iterator<Item = Uuid> + '_ {
    order[anchor.min(to)..=anchor.max(to)].iter().copied()
}

fn row_name(inv: &Inventory, id: Uuid) -> String {
    if inv.folders.contains_key(&id) {
        folder_name(inv, id)
    } else {
        rules::name(inv, id)
    }
}

/// The keyboard's place moves with an arrow from where Ctrl+↑/↓ left it:
/// that row becomes the selection the arrow starts from.
pub(super) fn adopt_focus(st: &mut InventoryUi) {
    if let Some(focus) = st.focus.take() {
        st.selection.clear();
        st.selection.insert(focus);
        st.selected = Some(focus);
        st.anchor = Some(focus);
        st.keyboard_selection = true;
    }
}

/// Handles this frame's selection keys and typed letters for the focused
/// list; true when the selected row changed (the list then scrolls to it).
pub(super) fn handle(ui: &egui::Ui, inv: &Inventory, st: &mut InventoryUi, explorer: bool) -> bool {
    let commands = shortcuts::take(ui, false, |c| matches!(c, Command::SelectAll | Command::InvertSelection));
    let cut = st.cut;
    let anything = !st.selection.is_empty() || st.focus.is_some();
    let mut moves = Vec::new();
    let mut typed = String::new();
    ui.input_mut(|input| {
        let held = input.modifiers;
        input.events.retain(|event| match event {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let plain = !modifiers.alt && !modifiers.command && !modifiers.ctrl;
                let ctrl = (modifiers.command || modifiers.ctrl) && !modifiers.alt && !modifiers.shift;
                let step = match key {
                    egui::Key::Home if plain => Some(Move::First),
                    egui::Key::End if plain => Some(Move::Last),
                    egui::Key::PageUp if plain => Some(Move::PageUp),
                    egui::Key::PageDown if plain => Some(Move::PageDown),
                    egui::Key::ArrowUp if explorer && ctrl => Some(Move::Focus(false)),
                    egui::Key::ArrowDown if explorer && ctrl => Some(Move::Focus(true)),
                    egui::Key::Space if explorer && ctrl => Some(Move::Toggle),
                    // a cut waits for its paste: Esc cancels the cut first (clipboard.rs)
                    egui::Key::Escape if modifiers.is_none() && !cut && anything => Some(Move::Clear),
                    _ => None,
                };
                match step {
                    Some(step) => {
                        moves.push((step, modifiers.shift));
                        false
                    }
                    None => true,
                }
            }
            egui::Event::Text(text) if !held.command && !held.ctrl && !held.alt => {
                typed.push_str(text);
                false
            }
            _ => true,
        });
    });
    let order: Vec<Uuid> = keyboard::shown_order(st).to_vec();
    let mut changed = false;
    for command in commands {
        match command {
            Command::SelectAll => {
                st.selection.extend(order.iter().copied());
                if st.selected.is_none() {
                    st.selected = order.first().copied();
                }
            }
            Command::InvertSelection => {
                let before = std::mem::take(&mut st.selection);
                st.selection.extend(order.iter().filter(|id| !before.contains(id)));
                st.selected = st
                    .selected
                    .filter(|id| st.selection.contains(id))
                    .or_else(|| order.iter().find(|id| st.selection.contains(id)).copied());
            }
            _ => {}
        }
        st.focus = None;
        st.keyboard_selection = true;
    }
    let page = st
        .list_viewport
        .map_or(10, |(_, height)| ((height / ROW_PITCH).floor() as usize).saturating_sub(1).max(1));
    for (step, shift) in moves {
        st.type_ahead.text.clear();
        if step == Move::Clear {
            st.selection.clear();
            st.selected = None;
            st.focus = None;
            continue;
        }
        let here = st.focus.or(st.selected).and_then(|id| order.iter().position(|row| *row == id));
        if step == Move::Toggle {
            if let Some(id) = here.map(|i| order[i]) {
                if !st.selection.remove(&id) {
                    st.selection.insert(id);
                }
                st.selected = Some(id);
                st.anchor = Some(id);
            }
            continue;
        }
        let Some(to) = moved(here.unwrap_or(0), order.len(), page, step) else {
            continue;
        };
        let target = order[to];
        if let Move::Focus(_) = step {
            st.focus = Some(target);
            continue;
        }
        st.focus = None;
        if shift {
            let anchor = st
                .anchor
                .and_then(|a| order.iter().position(|row| *row == a))
                .unwrap_or(here.unwrap_or(to));
            st.selection.clear();
            st.selection.extend(range(&order, anchor, to));
        } else {
            st.selection.clear();
            st.selection.insert(target);
            st.anchor = Some(target);
        }
        st.selected = Some(target);
        st.keyboard_selection = true;
        changed = true;
    }
    if !typed.is_empty() {
        let now = ui.input(|i| i.time);
        if now - st.type_ahead.at > TYPE_AHEAD_TIMEOUT {
            st.type_ahead.text.clear();
        }
        st.type_ahead.at = now;
        // a space only counts within a name being typed
        if !(st.type_ahead.text.is_empty() && typed.trim().is_empty()) {
            let before = st.type_ahead.text.clone();
            if st.type_ahead.text.chars().count() < TYPE_AHEAD_MAX {
                st.type_ahead.text.push_str(&typed);
            }
            // "a" then "a": Explorer's next row starting with "a"
            let mut letters = st.type_ahead.text.chars();
            let first = letters.next();
            let repeated = st.type_ahead.text.chars().count() > 1 && letters.all(|c| Some(c) == first);
            let (wanted, again) = if repeated {
                (first.map(String::from).unwrap_or_default(), true)
            } else {
                (st.type_ahead.text.clone(), false)
            };
            let names: Vec<(Uuid, String)> = order.iter().map(|id| (*id, row_name(inv, *id))).collect();
            let current = st.selected.filter(|id| st.selection.contains(id));
            if let Some(found) = type_ahead_target(&names, current, &wanted, again) {
                st.selection.clear();
                st.selection.insert(found);
                st.selected = Some(found);
                st.anchor = Some(found);
                st.focus = None;
                st.keyboard_selection = true;
                changed = true;
            } else if !before.is_empty() {
                // nothing starts so: keep what was found, as the Legacy Viewer
                st.type_ahead.text = before;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<(Uuid, String)> {
        list.iter()
            .enumerate()
            .map(|(n, name)| (Uuid::from_u128(n as u128 + 1), (*name).to_owned()))
            .collect()
    }

    #[test]
    fn typed_letters_find_the_next_name_from_the_selection_and_wrap() {
        let rows = names(&["Apple", "banana", "Berry", "cherry", "Bread"]);
        let id = |n: u128| Some(Uuid::from_u128(n));
        assert_eq!(type_ahead_target(&rows, None, "b", false), id(2));
        // from the selected row itself, as LLFolderView::search
        assert_eq!(type_ahead_target(&rows, id(3), "be", false), id(3));
        assert_eq!(type_ahead_target(&rows, id(4), "B", false), id(5));
        assert_eq!(type_ahead_target(&rows, id(5), "app", false), id(1), "wraps at the end");
        assert_eq!(type_ahead_target(&rows, id(1), "zz", false), None);
    }

    #[test]
    fn the_same_letter_again_goes_on_to_the_next_row() {
        let rows = names(&["Bag", "Belt", "Coat", "Boots"]);
        let id = |n: u128| Some(Uuid::from_u128(n));
        assert_eq!(type_ahead_target(&rows, id(1), "b", true), id(2));
        assert_eq!(type_ahead_target(&rows, id(2), "b", true), id(4));
        assert_eq!(type_ahead_target(&rows, id(4), "b", true), id(1));
    }

    #[test]
    fn names_in_any_script_are_found() {
        let rows = names(&["靴", "Été", "été 2"]);
        assert_eq!(type_ahead_target(&rows, None, "靴", false), Some(Uuid::from_u128(1)));
        assert_eq!(type_ahead_target(&rows, None, "ÉTÉ ", false), Some(Uuid::from_u128(3)));
    }

    #[test]
    fn home_end_and_pages_stop_at_the_ends() {
        assert_eq!(moved(5, 10, 3, Move::First), Some(0));
        assert_eq!(moved(5, 10, 3, Move::Last), Some(9));
        assert_eq!(moved(5, 10, 3, Move::PageUp), Some(2));
        assert_eq!(moved(1, 10, 3, Move::PageUp), Some(0));
        assert_eq!(moved(8, 10, 3, Move::PageDown), Some(9));
        assert_eq!(moved(0, 10, 3, Move::Focus(false)), Some(0));
        assert_eq!(moved(0, 0, 3, Move::Last), None);
    }

    #[test]
    fn a_range_runs_either_way_from_the_anchor() {
        let order: Vec<Uuid> = (1..=6).map(Uuid::from_u128).collect();
        assert_eq!(range(&order, 4, 1).count(), 4);
        assert_eq!(range(&order, 1, 4).collect::<Vec<_>>(), order[1..=4].to_vec());
    }
}
