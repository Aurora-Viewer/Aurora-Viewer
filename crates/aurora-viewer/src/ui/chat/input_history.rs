//! Sent-message recall, following LLChatEntry::updateHistory and
//! handleSpecialKey (indra/llui/llchatentry.cpp, originally LGPL 2.1):
//! Ctrl+Up/Down, no count limit, consecutive duplicates skipped, draft
//! restored after the newest entry. Deliberate improvement: nearby chat's
//! two editors share the entries, with independent browsing and drafts.

use std::collections::HashMap;

#[derive(Default)]
pub(super) struct History {
    entries: Vec<String>,
    browsing: HashMap<egui::Id, Browse>,
}

struct Browse {
    position: usize,
    draft: String,
}

impl History {
    /// Keep the original input, including channel prefixes and gestures,
    /// before the send path transforms it. Nothing is written to disk.
    pub(super) fn record(&mut self, field: egui::Id, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        if self.entries.last().is_none_or(|last| last != text) {
            self.entries.push(text.to_owned());
        }
        self.browsing.remove(&field);
    }

    fn step(&mut self, field: egui::Id, text: &mut String, older: bool) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        if older {
            let browse = self.browsing.entry(field).or_insert_with(|| Browse {
                position: self.entries.len(),
                draft: text.clone(),
            });
            if browse.position == 0 {
                return false;
            }
            browse.position -= 1;
            text.clone_from(&self.entries[browse.position]);
        } else {
            let Some(browse) = self.browsing.get_mut(&field) else {
                return false;
            };
            browse.position += 1;
            if browse.position < self.entries.len() {
                text.clone_from(&self.entries[browse.position]);
            } else {
                text.clone_from(&browse.draft);
                self.browsing.remove(&field);
            }
        }
        true
    }

    /// Before TextEdit and emoji navigation: take only Ctrl+Up/Down from
    /// the focused field, including repeats, so egui does not move its
    /// cursor first. Plain arrows and the other modifiers keep their uses.
    pub(super) fn recall(&mut self, ui: &egui::Ui, field: egui::Id, text: &mut String) -> bool {
        if !ui.memory(|m| m.has_focus(field)) {
            return false;
        }
        let mut changed = false;
        ui.input_mut(|i| {
            i.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && modifiers.ctrl
                    && !modifiers.shift
                    && !modifiers.alt
                    && !modifiers.mac_cmd
                    && matches!(key, egui::Key::ArrowUp | egui::Key::ArrowDown)
                {
                    changed |= self.step(field, text, *key == egui::Key::ArrowUp);
                    false
                } else {
                    true
                }
            });
        });
        if changed {
            super::cursor_to_end(ui.ctx(), field, text);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_walks_both_ways_and_restores_the_unsent_draft() {
        let mut history = History::default();
        let field = egui::Id::unique("field");
        history.record(field, "first");
        history.record(field, "/42 café 🙂");
        let mut text = "unfinished draft".to_owned();
        assert!(!history.step(field, &mut text, false));
        assert!(history.step(field, &mut text, true));
        assert_eq!(text, "/42 café 🙂");
        assert!(history.step(field, &mut text, true));
        assert_eq!(text, "first");
        assert!(!history.step(field, &mut text, true));
        assert_eq!(text, "first");
        assert!(history.step(field, &mut text, false));
        assert_eq!(text, "/42 café 🙂");
        assert!(history.step(field, &mut text, false));
        assert_eq!(text, "unfinished draft");
        assert!(!history.step(field, &mut text, false));
    }

    #[test]
    fn empty_sends_and_consecutive_duplicates_are_skipped_without_a_count_limit() {
        let mut history = History::default();
        let field = egui::Id::unique("field");
        history.record(field, " \n\t");
        let mut text = String::new();
        assert!(!history.step(field, &mut text, true));
        for n in 0..2000 {
            history.record(field, &n.to_string());
            history.record(field, &n.to_string());
        }
        for n in (0..2000).rev() {
            assert!(history.step(field, &mut text, true));
            assert_eq!(text, n.to_string());
        }
        assert!(!history.step(field, &mut text, true));
        history.record(field, "1999");
        assert!(history.step(field, &mut text, true));
        assert_eq!(text, "1999");
        history.record(field, "0");
        assert!(history.step(field, &mut text, true));
        assert_eq!(text, "0");
        assert!(history.step(field, &mut text, true));
        assert_eq!(text, "1999");
    }

    #[test]
    fn shared_entries_keep_each_local_fields_draft_and_browsing() {
        let mut history = History::default();
        let bar = super::super::bar_input_id();
        let window = egui::Id::unique(("conv_input", None::<uuid::Uuid>));
        history.record(bar, "from the bar");
        history.record(window, "from the window");
        let mut bar_text = "bar draft".to_owned();
        let mut window_text = "window draft".to_owned();
        history.step(bar, &mut bar_text, true);
        history.step(window, &mut window_text, true);
        assert_eq!(bar_text, "from the window");
        assert_eq!(window_text, "from the window");
        history.step(bar, &mut bar_text, true);
        assert_eq!(bar_text, "from the bar");
        assert_eq!(window_text, "from the window");
        window_text.push_str(" edited");
        history.record(window, &window_text);
        history.step(bar, &mut bar_text, false);
        assert_eq!(bar_text, "from the window");
        history.step(bar, &mut bar_text, false);
        assert_eq!(bar_text, "from the window edited");
        history.step(bar, &mut bar_text, false);
        assert_eq!(bar_text, "bar draft");
        window_text.clear();
        history.step(window, &mut window_text, true);
        assert_eq!(window_text, "from the window edited");
        history.step(window, &mut window_text, false);
        assert!(window_text.is_empty());
    }
}
