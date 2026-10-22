//! LLTextEditor::autoIndent, removeCharOrTab and handleSpecialKey (Firestorm
//! indra/llui/lltexteditor.cpp, originally LGPL 2.1): four-column tab stops.
//! Insert spaces for new tabs; preserve imported indentation and pasted text.
//! Closing braces follow unindentLineBeforeCloseBrace, but match their opener
//! instead of blindly removing one stop (also supports imported literal tabs).

use egui::{
    Event, Id, TextBuffer,
    text::{CCursor, CharIndex},
};
use std::ops::Range;

const TAB_SIZE: usize = 4;

fn line_before(source: &str, at: CharIndex) -> &str {
    let byte = source.byte_index_from_char_index(at).0;
    source[..byte].rsplit('\n').next().unwrap_or_default()
}

fn column(line: &str) -> usize {
    line.chars()
        .fold(0, |column, c| if c == '\t' { column + next_stop(column) } else { column + 1 })
}

fn next_stop(column: usize) -> usize {
    TAB_SIZE - column % TAB_SIZE
}

fn newline(source: &str, at: CharIndex) -> String {
    let line = line_before(source, at);
    let indent = &line[..line.find(|c| c != ' ' && c != '\t').unwrap_or(line.len())];
    let mut result = format!("\n{indent}");
    if line.trim_end().ends_with('{') {
        result.push_str(&" ".repeat(next_stop(column(indent))));
    }
    result
}

struct Buffer<'a> {
    source: &'a mut String,
    enter: Option<&'a str>,
    tab: Option<&'a str>,
    close: Option<&'a str>,
    scope: &'a dyn Fn(&str, usize) -> Option<Range<usize>>,
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
        if Some(text) == self.enter {
            let insertion = if (self.scope)(self.source, self.source.byte_index_from_char_index(at).0).is_some() {
                newline(self.source, at)
            } else {
                "\n".into()
            };
            self.source.insert_text(&insertion, at)
        } else if Some(text) == self.tab {
            let insertion = " ".repeat(next_stop(column(line_before(self.source, at))));
            self.source.insert_text(&insertion, at)
        } else {
            self.source.insert_text(text, at)
        }
    }
    fn insert_text_at(&mut self, cursor: &mut CCursor, text: &str, limit: usize) {
        if Some(text) == self.close {
            let at = self.source.byte_index_from_char_index(cursor.index).0;
            let line = line_before(self.source, cursor.index);
            let start = at - line.len();
            let whitespace = line.chars().all(|c| c == ' ' || c == '\t');
            let scope = (self.scope)(self.source, at);
            cursor.index += self.source.insert_text("}", cursor.index);
            if whitespace
                && let Some(scope) = scope
                && let Some(open) = super::syntax::matching_open_brace(&self.source[scope.start..at + 1], at - scope.start)
            {
                let open = scope.start + open;
                let opener = self.source[..open].rsplit('\n').next().unwrap_or_default();
                let indent = &opener[..opener.find(|c| c != ' ' && c != '\t').unwrap_or(opener.len())];
                if column(&self.source[start..at]) > column(indent) {
                    let indent = indent.to_owned();
                    let removed = self.source[start..at].chars().count();
                    self.source.replace_range(start..at, &indent);
                    cursor.index = CharIndex(cursor.index.0 - removed + indent.chars().count());
                }
            }
        } else {
            let cutoff = limit.saturating_sub(self.source.chars().count());
            let text = text.char_indices().nth(cutoff).map_or(text, |(i, _)| &text[..i]);
            cursor.index += self.insert_text(text, cursor.index);
        }
    }
    fn delete_char_range(&mut self, range: Range<CharIndex>) {
        self.source.delete_char_range(range);
    }
    fn delete_previous_char(&mut self, cursor: CCursor) -> CCursor {
        if cursor.index == CharIndex::ZERO {
            return cursor;
        }
        let line = line_before(self.source, cursor.index);
        let spaces = line.bytes().rev().take_while(|c| *c == b' ').count();
        let remainder = column(line) % TAB_SIZE;
        let step = if remainder == 0 { TAB_SIZE } else { remainder };
        // Like Firestorm, only remove a complete pseudo-tab: never eat code
        // or join lines just to reach a stop. Literal tabs delete atomically.
        let count = if spaces >= step { step } else { 1 };
        self.source.delete_selected_ccursor_range([cursor - count, cursor])
    }
    fn replace_with(&mut self, text: &str) {
        self.source.replace_with(text);
    }
}

pub(super) fn edit<R>(ui: &mut egui::Ui, id: Id, source: &mut String, show: impl FnOnce(&mut egui::Ui, &mut dyn TextBuffer) -> R) -> R {
    edit_scoped(ui, id, source, &|source, _| Some(0..source.len()), show).0
}

/// Tab stops and block indentation also serve Markdown source editors. Only
/// a language scope enables script comments, auto-indent and brace matching.
pub(crate) fn edit_scoped<R>(
    ui: &mut egui::Ui,
    id: Id,
    source: &mut String,
    scope: &dyn Fn(&str, usize) -> Option<Range<usize>>,
    show: impl FnOnce(&mut egui::Ui, &mut dyn TextBuffer) -> R,
) -> (R, bool) {
    let changed = super::editing::keyboard(ui, id, source, scope);
    let active = ui.memory(|m| m.has_focus(id))
        && ui.input(|i| {
            i.events.iter().any(|event| {
                insertion(event).is_some()
                    || matches!(
                        event,
                        Event::Key {
                            key: egui::Key::Backspace,
                            pressed: true,
                            ..
                        }
                    )
            })
        });
    if !active {
        return (show(ui, source), changed);
    }
    // egui 0.36 has no insertion callback for Return / Tab. Route those keys
    // through a private per-frame token, expanded at the actual cursor after
    // any preceding events / selection deletion. Native cursor movement and
    // undo then include the whole insertion atomically. Real paste is untouched.
    let tokens = ui.input(|i| i.events.iter().any(|e| insertion(e).is_some())).then(|| {
        let id = uuid::Uuid::new_v4();
        [
            format!("\0aurora-return-{id}\0"),
            format!("\0aurora-tab-{id}\0"),
            format!("\0aurora-close-{id}\0"),
        ]
    });
    let mut keys = ui.input_mut(|i| {
        i.events
            .iter_mut()
            .filter_map(|event| {
                let [enter, tab, close] = tokens.as_ref()?;
                let token = match insertion(event)? {
                    Insertion::Newline => enter,
                    Insertion::Tab => tab,
                    Insertion::CloseBrace => close,
                };
                Some(std::mem::replace(event, Event::Paste(token.clone())))
            })
            .collect::<Vec<_>>()
            .into_iter()
    });
    let result = show(
        ui,
        &mut Buffer {
            source,
            enter: tokens.as_ref().map(|t| t[0].as_str()),
            tab: tokens.as_ref().map(|t| t[1].as_str()),
            close: tokens.as_ref().map(|t| t[2].as_str()),
            scope,
        },
    );
    ui.input_mut(|i| {
        for event in &mut i.events {
            if matches!(event, Event::Paste(text) if tokens.as_ref().is_some_and(|tokens| tokens.contains(text)))
                && let Some(key) = keys.next()
            {
                *event = key;
            }
        }
    });
    (result, changed)
}

enum Insertion {
    Newline,
    Tab,
    CloseBrace,
}

fn insertion(event: &Event) -> Option<Insertion> {
    match event {
        Event::Text(text) if text == "}" => Some(Insertion::CloseBrace),
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if modifiers.matches_logically(egui::Modifiers::NONE) => match key {
            egui::Key::Enter => Some(Insertion::Newline),
            egui::Key::Tab if !modifiers.shift => Some(Insertion::Tab),
            _ => None,
        },
        _ => None,
    }
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
        let mut frame_output = ctx.run_ui(
            RawInput {
                events,
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let output = edit(ui, Id::unique("code"), source, |ui, source| {
                    egui::TextEdit::multiline(source).id(Id::unique("code")).code_editor().show(ui)
                });
                cursor = output.cursor_range;
            },
        );
        // Headless tests inspect editing state without uploading GPU textures.
        frame_output.textures_delta.clear();
        cursor.unwrap_or_default()
    }

    fn setup(ctx: &egui::Context, source: &mut String, start: usize, end: usize) {
        frame(ctx, source, vec![], 0.0);
        let mut state = egui::TextEdit::load_state(ctx, Id::unique("code")).expect("state");
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(start), CCursor::new(end))));
        state.store(ctx, Id::unique("code"));
        ctx.memory_mut(|m| m.request_focus(Id::unique("code")));
        // Let code_editor register its focus filter before sending Tab, as it
        // does on the real mouse-focus frame in the viewer.
        frame(ctx, source, vec![], 0.01);
    }

    #[test]
    fn real_keyboard_multiline_tab_comments_and_undo_preserve_the_selection() {
        let ctx = egui::Context::default();
        let mut source = "éあ\n  second\nthird".to_owned();
        setup(&ctx, &mut source, 0, 12);
        frame(&ctx, &mut source, vec![key(Key::Tab, Modifiers::NONE)], 0.1);
        assert_eq!(source, "    éあ\n      second\nthird");
        frame(&ctx, &mut source, vec![key(Key::Tab, Modifiers::SHIFT)], 0.2);
        assert_eq!(source, "éあ\n  second\nthird");
        frame(&ctx, &mut source, vec![key(Key::K, Modifiers::COMMAND)], 0.3);
        assert_eq!(source, "// éあ\n  // second\nthird");
        frame(&ctx, &mut source, vec![key(Key::K, Modifiers::COMMAND)], 0.4);
        assert_eq!(source, "éあ\n  second\nthird");
        frame(&ctx, &mut source, vec![key(Key::K, Modifiers::COMMAND | Modifiers::SHIFT)], 0.5);
        assert_eq!(source, "/*éあ\n  second\n*/third");
        frame(&ctx, &mut source, vec![key(Key::Z, Modifiers::COMMAND)], 0.6);
        assert_eq!(source, "éあ\n  second\nthird");
    }

    #[test]
    fn commands_after_native_selection_changes_wait_for_the_actual_cursor() {
        let ctx = egui::Context::default();
        let mut source = "first\nsecond\n".into();
        setup(&ctx, &mut source, 0, 0);
        frame(
            &ctx,
            &mut source,
            vec![key(Key::A, Modifiers::COMMAND), key(Key::Tab, Modifiers::NONE)],
            0.1,
        );
        frame(&ctx, &mut source, vec![], 0.2);
        assert_eq!(source, "    first\n    second\n");
    }

    #[test]
    fn close_brace_matches_nested_openers_tabs_and_unicode_and_undo_is_atomic() {
        for (original, expected) in [
            ("default {\n    event() {\n        ", "default {\n    event() {\n    }"),
            ("\t{\r\n\t\t\t", "\t{\r\n\t}"),
            ("   if (TRUE) {\n        ", "   if (TRUE) {\n   }"),
            (
                "// éあ\ndefault {\n    llSay(0, \"{\\\"}\"); // {\n    /* { } */\n        ",
                "// éあ\ndefault {\n    llSay(0, \"{\\\"}\"); // {\n    /* { } */\n}",
            ),
        ] {
            let ctx = egui::Context::default();
            let mut source = original.to_owned();
            let end = source.chars().count();
            setup(&ctx, &mut source, end, end);
            let cursor = frame(&ctx, &mut source, vec![Event::Text("}".into()), Event::Text(" // fin".into())], 0.1);
            assert_eq!(source, format!("{expected} // fin"));
            assert_eq!(cursor.primary.index.0, source.chars().count());
            frame(&ctx, &mut source, vec![key(Key::Z, Modifiers::COMMAND)], 0.2);
            assert_eq!(source, original);
            frame(&ctx, &mut source, vec![key(Key::Y, Modifiers::COMMAND)], 0.3);
            assert_eq!(source, format!("{expected} // fin"));
        }
    }

    #[test]
    fn close_brace_keeps_inline_code_paste_unmatched_and_non_code_contexts() {
        for original in ["    ", "{ code ", "    {\n  ", "{\n    /*\n        ", "{\n    \"line\n        "] {
            let ctx = egui::Context::default();
            let mut source = original.to_owned();
            let end = source.chars().count();
            setup(&ctx, &mut source, end, end);
            frame(&ctx, &mut source, vec![Event::Text("}".into())], 0.1);
            assert_eq!(source, format!("{original}}}"));
        }
        let ctx = egui::Context::default();
        let mut source = "{\n    ".to_owned();
        setup(&ctx, &mut source, 6, 6);
        frame(&ctx, &mut source, vec![Event::Paste("}".into())], 0.1);
        assert_eq!(source, "{\n    }");
    }

    #[test]
    fn enter_tab_selection_and_multiple_closers_use_the_current_buffer() {
        let ctx = egui::Context::default();
        let mut source = "{\n    {\n        selected".to_owned();
        setup(&ctx, &mut source, 16, 24);
        let cursor = frame(
            &ctx,
            &mut source,
            vec![
                Event::Text("}".into()),
                key(Key::Enter, Modifiers::NONE),
                key(Key::Tab, Modifiers::NONE),
                Event::Text("}".into()),
            ],
            0.1,
        );
        assert_eq!(source, "{\n    {\n    }\n}");
        assert_eq!(cursor.primary.index.0, source.chars().count());
    }

    #[test]
    fn newline_copies_spaces_tabs_and_adds_one_level_after_a_brace() {
        for (source, expected) in [
            ("    llSay();", "\n    "),
            ("\t\tllSay();", "\n\t\t"),
            ("    {", "\n        "),
            ("\t{ ", "\n\t    "),
            ("   {", "\n    "),
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
        assert_eq!(source, "\téあ{ \n\t    code");
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

    #[test]
    fn tab_uses_spaces_and_advances_to_the_next_stop_from_every_offset() {
        for spaces in 0..13 {
            let ctx = egui::Context::default();
            let mut source = format!("{}code", " ".repeat(spaces));
            setup(&ctx, &mut source, spaces, spaces);
            let cursor = frame(&ctx, &mut source, vec![key(Key::Tab, Modifiers::NONE)], 0.1);
            let expected = spaces + next_stop(spaces);
            assert_eq!(source, format!("{}code", " ".repeat(expected)), "offset {spaces}");
            assert_eq!(cursor.primary.index.0, expected);
            assert!(!source.contains('\t'));
        }
    }

    #[test]
    fn backspace_removes_full_or_incomplete_groups_without_touching_the_suffix() {
        for spaces in 1..14 {
            let ctx = egui::Context::default();
            let mut source = format!("previous\r\n{}éあ", " ".repeat(spaces));
            let at = 10 + spaces;
            setup(&ctx, &mut source, at, at);
            let cursor = frame(&ctx, &mut source, vec![key(Key::Backspace, Modifiers::NONE)], 0.1);
            let expected = (spaces - 1) / TAB_SIZE * TAB_SIZE;
            assert_eq!(source, format!("previous\r\n{}éあ", " ".repeat(expected)), "offset {spaces}");
            assert_eq!(cursor.primary.index.0, 10 + expected);
        }
    }

    #[test]
    fn selection_and_code_backspace_keep_native_deletion_rules() {
        for (original, start, end, expected, at) in [
            ("    code", 1, 3, "  code", 1),
            ("x   ", 4, 4, "x  ", 3),
            ("    éあ", 6, 6, "    é", 5),
            ("    \n", 5, 5, "    ", 4),
            ("code", 0, 0, "code", 0),
        ] {
            let ctx = egui::Context::default();
            let mut source = original.to_owned();
            setup(&ctx, &mut source, start, end);
            let cursor = frame(&ctx, &mut source, vec![key(Key::Backspace, Modifiers::NONE)], 0.1);
            assert_eq!(source, expected);
            assert_eq!(cursor.primary.index.0, at);
        }
    }

    #[test]
    fn tabs_then_backspace_in_one_frame_use_the_current_column_and_undo_atomically() {
        let ctx = egui::Context::default();
        let mut source = "\t  code".to_owned();
        setup(&ctx, &mut source, 3, 3);
        let cursor = frame(
            &ctx,
            &mut source,
            vec![
                key(Key::Tab, Modifiers::NONE),
                key(Key::Tab, Modifiers::NONE),
                key(Key::Backspace, Modifiers::NONE),
                Event::Text("éあ".into()),
                Event::Paste("\t   ".into()),
            ],
            0.1,
        );
        assert_eq!(source, "\t    éあ\t   code");
        assert_eq!(cursor.primary.index.0, 11);
        frame(&ctx, &mut source, vec![key(Key::Z, Modifiers::COMMAND)], 0.2);
        assert_eq!(source, "\t  code");
        frame(&ctx, &mut source, vec![key(Key::Y, Modifiers::COMMAND)], 0.3);
        assert_eq!(source, "\t    éあ\t   code");
    }

    #[test]
    fn tab_replaces_selection_and_backspace_deletes_an_imported_tab_as_one_unit() {
        let ctx = egui::Context::default();
        let mut source = "\ttext".to_owned();
        setup(&ctx, &mut source, 1, 1);
        let cursor = frame(&ctx, &mut source, vec![key(Key::Backspace, Modifiers::NONE)], 0.1);
        assert_eq!(source, "text");
        assert_eq!(cursor.primary.index.0, 0);
        setup(&ctx, &mut source, 0, 4);
        let cursor = frame(&ctx, &mut source, vec![key(Key::Tab, Modifiers::NONE)], 0.2);
        assert_eq!(source, "    ");
        assert_eq!(cursor.primary.index.0, 4);
    }
}
