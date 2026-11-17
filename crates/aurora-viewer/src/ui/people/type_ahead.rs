//! Type-ahead of the lists of residents, as the Legacy Viewer's
//! `LLScrollListCtrl::handleUnicodeCharHere` (indra/llui/llscrolllistctrl.cpp):
//! a click gives the list the keyboard, then typing jumps to the first row
//! whose name starts with what was typed. The buffer empties after
//! `TypeAheadTimeout` (1.5 s), Backspace removes a character, and repeating
//! one character cycles through the rows that start with it.
//!
//! The Legacy Viewer searches the first text column of its list (the username
//! of the friend list). Aurora's rows show the chosen name, so every form the
//! row knows is matched. When a display name and another person's username
//! share a prefix, the first matching row in the current list order wins.

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

/// The prefix kept between frames while the list owns the keyboard.
#[derive(Default, Clone)]
struct Search {
    text: String,
    reset_at: f64,
}

/// The list's focus and the match to reveal while drawing its rows this frame.
#[derive(Clone, Copy)]
pub(crate) struct Navigation {
    focus: Id,
    pub chosen: Option<Uuid>,
}

impl Navigation {
    pub fn new<'a>(ui: &Ui, focus: Id, selected: Option<Uuid>, entries: impl FnOnce() -> Vec<Entry<'a>>) -> Self {
        let background = ui.interact(ui.available_rect_before_wrap(), focus, egui::Sense::click());
        if background.clicked() {
            background.request_focus();
        }
        let key = focus.with("type-ahead");
        let has_focus = background.has_focus();
        if !has_focus || background.gained_focus() {
            ui.ctx().data_mut(|data| data.remove::<Search>(key));
        }
        if has_focus {
            // Offer native composition at the list, as TextEdit does at its caret.
            let to_global = ui.ctx().layer_transform_to_global(ui.layer_id()).unwrap_or_default();
            let rect = background.interact_rect;
            let cursor = Rect::from_min_size(rect.min, egui::vec2(1.0, ui.spacing().interact_size.y));
            ui.output_mut(|out| {
                out.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect: to_global * rect,
                    cursor_rect: to_global * cursor,
                    should_interrupt_composition: false,
                });
            });
        }
        let chosen = if has_focus && ui.input(|input| input.events.iter().any(is_search_event)) {
            keys(ui, key, &entries(), selected)
        } else {
            None
        };
        Self { focus, chosen }
    }

    /// Call before skipping clipped rows so an off-screen match can scroll into view.
    pub fn row(self, ui: &Ui, response: &egui::Response, rect: Rect, id: Uuid) {
        if self.chosen == Some(id) {
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
        if response.clicked() || response.secondary_clicked() {
            ui.memory_mut(|memory| memory.request_focus(self.focus));
        }
    }
}

fn is_search_event(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Text(_)
            | egui::Event::Ime(egui::ImeEvent::Commit(_))
            | egui::Event::Key {
                key: Key::Backspace,
                pressed: true,
                ..
            }
    )
}

fn keys(ui: &Ui, key: Id, entries: &[Entry<'_>], selected: Option<Uuid>) -> Option<Uuid> {
    let now = ui.input(|i| i.time);
    let mut search = ui.ctx().data(|d| d.get_temp::<Search>(key)).unwrap_or_default();
    expire(&mut search, now);
    let mut chosen = None;
    ui.input(|i| {
        for event in &i.events {
            match event {
                egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    for c in text.chars() {
                        if let Some(id) = typed(&mut search, entries, chosen.or(selected), c, now) {
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
                        chosen = Some(entries[index].id);
                    }
                }
                _ => {}
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(key, search));
    chosen
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
            let mut folded = name.chars().flat_map(char::to_lowercase);
            target.chars().all(|c| folded.next() == Some(c))
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
            .enumerate()
            .map(|(index, (name, username))| Entry {
                id: Uuid::from_u128(1 + index as u128),
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
    fn batched_letters_cycle_from_the_last_match_and_focus_resets_the_prefix() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let friends = entries(&[("Alice", ""), ("Amber", ""), ("Aria", ""), ("Bob", "")]);
        let focus = Id::unique("list");
        let mut built = 0;
        let mut frame = |events: Vec<egui::Event>, owner: Id, selected: Option<Uuid>| {
            let mut chosen = None;
            frames.run(events, |ui| {
                ui.memory_mut(|memory| memory.request_focus(owner));
                chosen = Navigation::new(ui, focus, selected, || {
                    built += 1;
                    friends.clone()
                })
                .chosen;
            });
            chosen
        };
        assert_eq!(frame(Vec::new(), focus, None), None);
        assert_eq!(
            frame(
                vec![egui::Event::Text("aa".into()), egui::Event::Text("a".into())],
                focus,
                Some(friends[3].id)
            ),
            Some(friends[2].id)
        );
        // This frame belongs to another widget; the old "a" must not prefix "b".
        assert_eq!(
            frame(vec![egui::Event::Text("ignored".into())], Id::unique("elsewhere"), None),
            None
        );
        assert_eq!(frame(vec![egui::Event::Text("b".into())], focus, None), Some(friends[3].id));
        assert_eq!(built, 2, "idle and unfocused lists never build search entries");
    }

    #[test]
    fn composing_a_name_jumps_only_when_the_ime_commits_to_the_focused_list() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let friends = entries(&[("山田", ""), ("山本", ""), ("山本A", "")]);
        let focus = Id::unique("composed-name-list");
        let preedit = |text: &str| {
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: text.into(),
                active_range_chars: None,
            })
        };
        let commit = |text: &str| egui::Event::Ime(egui::ImeEvent::Commit(text.into()));
        let mut frame = |events, owner, selected| {
            let mut chosen = None;
            let (_, output) = frames.run_with_output(events, |ui| {
                if !ui.memory(|mem| mem.has_focus(owner)) {
                    ui.memory_mut(|mem| mem.request_focus(owner));
                }
                chosen = Navigation::new(ui, focus, selected, || friends.clone()).chosen;
            });
            (chosen, output.ime.is_some())
        };
        assert_eq!(frame(vec![], focus, None), (None, true));
        assert_eq!(frame(vec![preedit("山")], focus, None), (None, true));
        assert_eq!(frame(vec![preedit(""), commit("山")], focus, None), (Some(friends[0].id), true));
        assert_eq!(
            frame(vec![commit("本"), egui::Event::Text("A".into())], focus, Some(friends[0].id)),
            (Some(friends[2].id), true)
        );
        assert_eq!(frame(vec![commit("山")], Id::unique("another-widget"), None), (None, false));
        assert_eq!(frame(vec![commit("山本A")], focus, None), (Some(friends[2].id), true));
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
