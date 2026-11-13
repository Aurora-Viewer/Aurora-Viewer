//! Type-ahead of the lists of residents, as the Legacy Viewer's
//! `LLScrollListCtrl::handleUnicodeCharHere` (indra/llui/llscrolllistctrl.cpp):
//! a click gives the list the keyboard, then typing jumps to the first row
//! whose name starts with what was typed. The buffer empties after
//! `TypeAheadTimeout` (1.5 s), Backspace removes a character, and repeating
//! one character cycles through the rows that start with it.
//!
//! The Legacy Viewer searches the first text column of its list (the username
//! of the friend list). Aurora's rows show the chosen name, so every form the
//! row knows is matched: the visible name first, then the username and the
//! full name. A superset of the Legacy Viewer, never a different row when the
//! username is the one typed.

use egui::{Align, Id, Key, Rect, Ui};
use uuid::Uuid;

/// TypeAheadTimeout, in seconds (indra/newview/app_settings/settings.xml).
const TIMEOUT: f64 = 1.5;

/// One row of a list: its id and the names a typed prefix may match, the
/// first one being the name the row shows.
#[derive(Clone, Copy)]
pub(crate) struct Entry<'a> {
    pub id: Uuid,
    pub names: [&'a str; 3],
}

/// The buffer and the row a match asked to bring into view, kept between
/// frames under the list's focus id.
#[derive(Default, Clone)]
struct Search {
    text: String,
    reset_at: f64,
    reveal: Option<Uuid>,
}

/// Takes the characters typed while `focus` owns the keyboard; returns the
/// row to select. `selected` is the row the keyboard starts from when one
/// character is repeated.
pub(crate) fn keys(ui: &Ui, focus: Id, entries: &[Entry<'_>], selected: Option<Uuid>) -> Option<Uuid> {
    if !ui.memory(|m| m.has_focus(focus)) {
        return None;
    }
    let now = ui.input(|i| i.time);
    let key = focus.with("type-ahead");
    let mut search = ui.ctx().data(|d| d.get_temp::<Search>(key)).unwrap_or_default();
    expire(&mut search, now);
    let mut chosen = None;
    ui.input(|i| {
        for event in &i.events {
            match event {
                egui::Event::Text(text) => {
                    for c in text.chars() {
                        if let Some(id) = typed(&mut search, entries, selected.or(chosen), c, now) {
                            chosen = Some(id);
                        }
                    }
                }
                egui::Event::Key {
                    key: Key::Backspace,
                    pressed: true,
                    ..
                } => {
                    search.reset_at = now + TIMEOUT;
                    search.text.pop();
                    if !search.text.is_empty()
                        && let Some(index) = prefix(entries, &search.text)
                    {
                        let id = entries[index].id;
                        search.reveal = Some(id);
                        chosen = Some(id);
                    }
                }
                _ => {}
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, search));
    chosen
}

/// Brings the row `id` into view when it is the one a name typed asked for.
/// Call it right after the row's rectangle is known, even when the row is
/// outside the viewport.
pub(crate) fn reveal(ui: &Ui, focus: Id, rect: Rect, id: Uuid) {
    let key = focus.with("type-ahead");
    let Some(mut search) = ui.ctx().data(|d| d.get_temp::<Search>(key)) else {
        return;
    };
    if search.reveal != Some(id) {
        return;
    }
    ui.scroll_to_rect(rect, Some(Align::Center));
    search.reveal = None;
    ui.ctx().data_mut(|d| d.insert_temp(key, search));
}

/// A pause longer than TypeAheadTimeout starts the search afresh.
fn expire(search: &mut Search, now: f64) {
    if now > search.reset_at {
        search.text.clear();
    }
}

/// One character typed: a prefix of a name selects the first match, else
/// repeating one character steps to the next name that starts with it.
fn typed(search: &mut Search, entries: &[Entry<'_>], selected: Option<Uuid>, c: char, now: f64) -> Option<Uuid> {
    let mut lower = String::new();
    lower.extend(c.to_lowercase());
    let mut target = search.text.clone();
    target.push_str(&lower);
    if let Some(index) = prefix(entries, &target) {
        search.text = target;
        search.reset_at = now + TIMEOUT;
        search.reveal = Some(entries[index].id);
        return Some(entries[index].id);
    }
    if repeated(&target) && !entries.is_empty() {
        let from = selected
            .and_then(|id| entries.iter().position(|e| e.id == id))
            .map_or(0, |index| (index + 1) % entries.len());
        for step in 0..entries.len() {
            let index = (from + step) % entries.len();
            if entries[index]
                .names
                .iter()
                .any(|name| name.chars().next().is_some_and(|first| starts(first, &lower)))
            {
                search.reset_at = now + TIMEOUT;
                search.reveal = Some(entries[index].id);
                return Some(entries[index].id);
            }
        }
    }
    None
}

/// The first row one of whose names starts with `target` (case-insensitive).
fn prefix(entries: &[Entry<'_>], target: &str) -> Option<usize> {
    entries.iter().position(|entry| {
        entry.names.iter().any(|name| {
            let mut first = String::new();
            first.extend(name.chars().flat_map(char::to_lowercase));
            first.starts_with(target)
        })
    })
}

/// Every character the same (LLScrollListCtrl::isRepeatedChars).
fn repeated(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    chars.all(|c| c == first)
}

/// Whether `first` is `lower` (a lowercased character, as stored in the
/// buffer).
fn starts(first: char, lower: &str) -> bool {
    let mut folded = String::new();
    folded.extend(first.to_lowercase());
    folded.starts_with(lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries<'a>(names: &'a [(&'a str, &'a str)]) -> Vec<Entry<'a>> {
        names
            .iter()
            .map(|(name, username)| Entry {
                id: Uuid::from_u128(name.len() as u128),
                names: [name, username, ""],
            })
            .collect()
    }

    #[test]
    fn a_prefix_selects_the_first_row_case_insensitively() {
        let e = entries(&[("Aaron", "aaron.resident"), ("Bob", "bob.resident")]);
        let mut s = Search::default();
        assert_eq!(typed(&mut s, &e, None, 'b', 0.0), Some(e[1].id));
        assert_eq!(s.text, "b");
        assert_eq!(typed(&mut s, &e, None, 'O', 0.0), Some(e[1].id));
        assert_eq!(s.text, "bo");
    }

    #[test]
    fn a_username_matches_too() {
        let e = entries(&[("Zoe", "alice.resident"), ("Bob", "bob.resident")]);
        let mut s = Search::default();
        assert_eq!(typed(&mut s, &e, None, 'a', 0.0), Some(e[0].id));
    }

    #[test]
    fn a_character_outside_a_prefix_keeps_the_buffer() {
        let e = entries(&[("Aaron", "aaron.resident"), ("Bob", "bob.resident")]);
        let mut s = Search::default();
        typed(&mut s, &e, None, 'a', 0.0);
        assert_eq!(typed(&mut s, &e, None, 'z', 0.0), None);
        assert_eq!(s.text, "a");
    }

    #[test]
    fn repeating_a_character_steps_through_the_names_that_start_with_it() {
        let e = entries(&[("Alice", "alice.resident"), ("Amber", "amber.resident"), ("Bob", "bob.resident")]);
        let mut s = Search::default();
        let first = typed(&mut s, &e, None, 'a', 0.0);
        assert_eq!(first, Some(e[0].id));
        assert_eq!(typed(&mut s, &e, first, 'a', 0.0), Some(e[1].id));
        assert_eq!(typed(&mut s, &e, Some(e[1].id), 'a', 0.0), Some(e[0].id));
        assert_eq!(s.text, "a");
    }

    #[test]
    fn the_buffer_empties_after_the_timeout() {
        let e = entries(&[("Aaron", "aaron.resident"), ("Bob", "bob.resident")]);
        let mut s = Search::default();
        typed(&mut s, &e, None, 'a', 0.0);
        expire(&mut s, TIMEOUT - 0.1);
        assert_eq!(s.text, "a");
        expire(&mut s, TIMEOUT + 0.1);
        assert!(s.text.is_empty());
    }
}
