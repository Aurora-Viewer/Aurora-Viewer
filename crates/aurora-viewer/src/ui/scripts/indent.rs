//! LLTextEditor::autoIndent (Firestorm indra/llui/lltexteditor.cpp, originally
//! LGPL 2.1): copy the current line's indentation and add four spaces after
//! an opening brace. Aurora also preserves literal tabs in tab-indented code.

use egui::{Event, Id, TextBuffer, text::CharIndex};
use std::ops::Range;

fn newline(source: &str, at: CharIndex) -> String {
    let byte = source.byte_index_from_char_index(at).0;
    let prefix = &source[..byte];
    let line = prefix.rsplit('\n').next().unwrap_or_default();
    let indent = &line[..line.find(|c| c != ' ' && c != '\t').unwrap_or(line.len())];
    let mut result = format!("\n{indent}");
    if line.trim_end().ends_with('{') {
        result.push_str(if indent.contains('\t') { "\t" } else { "    " });
    }
    result
}

struct Buffer<'a> {
    source: &'a mut String,
    enter: &'a str,
}

impl TextBuffer for Buffer<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Buffer<'static>>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        self.source
    }
    fn insert_text(&mut self, text: &str, at: CharIndex) -> usize {
        if text == self.enter {
            let insertion = newline(self.source, at);
            self.source.insert_text(&insertion, at)
        } else {
            self.source.insert_text(text, at)
        }
    }
    fn delete_char_range(&mut self, range: Range<CharIndex>) {
        self.source.delete_char_range(range);
    }
    fn replace_with(&mut self, text: &str) {
        self.source.replace_with(text);
    }
}

pub(super) fn edit<R>(ui: &mut egui::Ui, id: Id, source: &mut String, show: impl FnOnce(&mut egui::Ui, &mut dyn TextBuffer) -> R) -> R {
    let has_enter = ui.memory(|m| m.has_focus(id)) && ui.input(|i| i.events.iter().any(is_enter));
    if !has_enter {
        return show(ui, source);
    }
    // egui 0.36 has no insertion callback for Return. Route just those keys
    // through a private per-frame token, expanded at the actual cursor after
    // any preceding events / selection deletion. Native cursor movement and
    // undo then include the whole insertion atomically. Real paste is untouched.
    let token = format!("\0aurora-return-{}\0", uuid::Uuid::new_v4());
    let mut keys = ui.input_mut(|i| {
        i.events
            .iter_mut()
            .filter_map(|event| is_enter(event).then(|| std::mem::replace(event, Event::Paste(token.clone()))))
            .collect::<Vec<_>>()
            .into_iter()
    });
    let result = show(ui, &mut Buffer { source, enter: &token });
    ui.input_mut(|i| {
        for event in &mut i.events {
            if matches!(event, Event::Paste(text) if text == &token)
                && let Some(key) = keys.next()
            {
                *event = key;
            }
        }
    });
    result
}

fn is_enter(event: &Event) -> bool {
    matches!(event, Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. }
        if modifiers.matches_logically(egui::Modifiers::NONE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{
        Key, Modifiers, RawInput,
        text::{CCursor, CCursorRange},
    };

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn frame(ctx: &egui::Context, source: &mut String, events: Vec<Event>, time: f64) -> CCursorRange {
        let mut cursor = None;
        let _ = ctx.run_ui(
            RawInput {
                events,
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let output = edit(ui, Id::new("code"), source, |ui, source| {
                    egui::TextEdit::multiline(source).id(Id::new("code")).code_editor().show(ui)
                });
                cursor = output.cursor_range;
            },
        );
        cursor.unwrap_or_default()
    }

    fn setup(ctx: &egui::Context, source: &mut String, start: usize, end: usize) {
        frame(ctx, source, vec![], 0.0);
        let mut state = egui::TextEdit::load_state(ctx, Id::new("code")).expect("state");
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(start), CCursor::new(end))));
        state.store(ctx, Id::new("code"));
        ctx.memory_mut(|m| m.request_focus(Id::new("code")));
    }

    #[test]
    fn newline_copies_spaces_tabs_and_adds_one_level_after_a_brace() {
        for (source, expected) in [
            ("    llSay();", "\n    "),
            ("\t\tllSay();", "\n\t\t"),
            ("    {", "\n        "),
            ("\t{ ", "\n\t\t"),
            ("    ", "\n    "),
            ("default", "\n"),
        ] {
            assert_eq!(newline(source, CharIndex(source.chars().count())), expected);
        }
        assert_eq!(newline("    éあsuite", CharIndex(6)), "\n    ");
    }

    #[test]
    fn enter_and_typing_in_one_frame_use_the_actual_cursor_and_paste_stays_exact() {
        let ctx = egui::Context::default();
        let mut source = "\téあ".to_owned();
        setup(&ctx, &mut source, 3, 3);
        let cursor = frame(
            &ctx,
            &mut source,
            vec![
                Event::Text("{ type".into()),
                key(Key::Backspace, Modifiers::NONE),
                key(Key::Backspace, Modifiers::NONE),
                key(Key::Backspace, Modifiers::NONE),
                key(Key::Backspace, Modifiers::NONE),
                key(Key::Enter, Modifiers::NONE),
                Event::Text("code".into()),
            ],
            0.1,
        );
        assert_eq!(source, "\téあ{ \n\t\tcode");
        assert_eq!(cursor.primary.index.0, source.chars().count());
        frame(&ctx, &mut source, vec![Event::Paste("\n".into())], 0.2);
        assert!(source.ends_with("code\n"));
    }

    #[test]
    fn selection_replacement_and_undo_redo_include_the_indentation() {
        let ctx = egui::Context::default();
        let mut source = "    éあ rest".to_owned();
        setup(&ctx, &mut source, 6, 11);
        let cursor = frame(&ctx, &mut source, vec![key(Key::Enter, Modifiers::NONE)], 0.1);
        assert_eq!(source, "    éあ\n    ");
        assert_eq!(cursor.primary.index.0, 11);
        frame(&ctx, &mut source, vec![key(Key::Z, Modifiers::COMMAND)], 0.2);
        assert_eq!(source, "    éあ rest");
        frame(&ctx, &mut source, vec![key(Key::Y, Modifiers::COMMAND)], 0.3);
        assert_eq!(source, "    éあ\n    ");
    }
}
