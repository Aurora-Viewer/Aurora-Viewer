//! Source-only Markdown actions. Character selections and native undo history
//! remain attached to the original notecard editor, including embedded items.

use std::ops::Range;

use egui::{
    Id, TextBuffer,
    text::{CCursor, CCursorRange, CharIndex},
};

use crate::{
    theme::Palette,
    ui::{menu, widgets},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Heading,
    Bullet,
    Ordered,
    Quote,
    CodeBlock,
    LslBlock,
}

const ACTIONS: [Action; 11] = [
    Action::Bold,
    Action::Italic,
    Action::Strike,
    Action::Code,
    Action::Link,
    Action::Heading,
    Action::Bullet,
    Action::Ordered,
    Action::Quote,
    Action::CodeBlock,
    Action::LslBlock,
];

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Bold => aurora_i18n::tr!("notecard-format-bold"),
            Self::Italic => aurora_i18n::tr!("notecard-format-italic"),
            Self::Strike => aurora_i18n::tr!("notecard-format-strike"),
            Self::Code => aurora_i18n::tr!("notecard-format-code"),
            Self::Link => aurora_i18n::tr!("notecard-format-link"),
            Self::Heading => aurora_i18n::tr!("notecard-format-heading"),
            Self::Bullet => aurora_i18n::tr!("notecard-format-bullet"),
            Self::Ordered => aurora_i18n::tr!("notecard-format-ordered"),
            Self::Quote => aurora_i18n::tr!("notecard-format-quote"),
            Self::CodeBlock => aurora_i18n::tr!("notecard-format-code-block"),
            Self::LslBlock => aurora_i18n::tr!("notecard-format-lsl"),
        }
    }

    fn example(self) -> String {
        let text = aurora_i18n::tr!("notecard-format-placeholder");
        match self {
            Self::Bold => format!("**{text}**"),
            Self::Italic => format!("*{text}*"),
            Self::Strike => format!("~~{text}~~"),
            Self::Code => format!("`{text}`"),
            Self::Link => format!("[{text}](https://example.com)"),
            Self::Heading => format!("## {text}"),
            Self::Bullet => format!("- {text}"),
            Self::Ordered => format!("1. {text}"),
            Self::Quote => format!("> {text}"),
            Self::CodeBlock => format!("```\n{text}\n```"),
            Self::LslBlock => "```lsl\nllOwnerSay(\"Hello!\");\n```".into(),
        }
    }
}

/// The caller applies the command after the source editor consumes its pointer
/// events, so losing focus to a toolbar click cannot override the new selection.
pub(super) fn toolbar(ui: &mut egui::Ui, p: &Palette, id: Id, editable: bool) -> Option<Action> {
    let mut command = None;
    ui.push_id(id.with("formatting"), |ui| {
        ui.horizontal_wrapped(|ui| {
            for action in ACTIONS {
                let response = widgets::action_button_if(
                    ui,
                    p,
                    "",
                    action.label(),
                    widgets::ActionKind::Flat,
                    egui::vec2(0.0, ui.spacing().interact_size.y),
                    editable,
                )
                .on_hover_text(action.example());
                if action == Action::Bold {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(id.with("demo_format_bold"), response.rect));
                }
                if response.clicked() {
                    command = Some(action);
                }
            }
            let help = menu::button(ui, p, egui::Button::new(aurora_i18n::tr!("notecard-markdown-help")), |ui| {
                ui.set_max_width(420.0);
                ui.label(aurora_i18n::tr!("notecard-markdown-help-intro"));
                egui::Grid::new("syntax").spacing(egui::vec2(16.0, 6.0)).show(ui, |ui| {
                    for action in ACTIONS {
                        ui.label(action.label());
                        ui.label(egui::RichText::new(action.example()).monospace());
                        ui.end_row();
                    }
                });
                ui.label(aurora_i18n::tr!("notecard-markdown-help-extra"));
                ui.label(aurora_i18n::tr!("notecard-markdown-help-code"));
            });
            ui.ctx().data_mut(|data| data.insert_temp(id.with("demo_format_help"), help.rect));
        });
        ui.label(egui::RichText::new(aurora_i18n::tr!("notecard-format-hint")).small().color(p.muted));
    });
    command
}

pub(super) fn apply(ctx: &egui::Context, id: Id, text: &mut String, action: Action) -> bool {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let before = state.cursor.char_range().unwrap_or_default();
    let range = before.as_sorted_char_range();
    let (updated, selection) = transform(text, range.start.0..range.end.0, action);
    if updated == *text {
        return false;
    }
    let mut after = CCursorRange::two(CCursor::new(selection.start), CCursor::new(selection.end));
    if before.primary.index < before.secondary.index {
        std::mem::swap(&mut after.primary, &mut after.secondary);
    }
    let mut undo = state.undoer();
    undo.add_undo(&(before, text.clone()));
    *text = updated;
    state.cursor.set_char_range(Some(after));
    undo.add_undo(&(after, text.clone()));
    state.set_undoer(undo);
    state.store(ctx, id);
    ctx.memory_mut(|memory| memory.request_focus(id));
    ctx.request_repaint();
    true
}

pub(super) fn history_key(ui: &mut egui::Ui, id: Id, editable: bool) -> Option<bool> {
    if !editable || !ui.memory(|memory| memory.has_focus(id)) {
        return None;
    }
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z)
            || input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
        {
            Some(true)
        } else if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
            Some(false)
        } else {
            None
        }
    })
}

/// egui's undo snapshots compare both selection and text. Losing IME ownership
/// collapses a selection, so first align with the latest equal-text checkpoint
/// and skip selection-only snapshots. Otherwise Undo appears to do nothing and
/// Redo can discard its history merely because another pane received focus.
pub(super) fn history(ctx: &egui::Context, id: Id, text: &mut String, redo: bool) -> bool {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let mut current = (state.cursor.char_range().unwrap_or_default(), text.clone());
    let mut undo = state.undoer();
    if !undo.has_redo(&current)
        && let Some(top) = undo.clone().undo(&current).filter(|(_, source)| source == text)
    {
        current = top.clone();
    }
    let mut changed = false;
    loop {
        let next = if redo { undo.redo(&current) } else { undo.undo(&current) }.cloned();
        let Some(next) = next else { break };
        if next.1 != *text {
            text.clone_from(&next.1);
            state.cursor.set_char_range(Some(next.0));
            changed = true;
            break;
        }
        current = next;
    }
    state.set_undoer(undo);
    state.store(ctx, id);
    ctx.memory_mut(|memory| memory.request_focus(id));
    if changed {
        ctx.request_repaint();
    }
    changed
}

fn replace(text: &str, range: Range<usize>, replacement: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let start = text[..range.start].chars().count();
    let mut updated = text.to_owned();
    updated.replace_range(range, replacement);
    (updated, start + selection.start..start + selection.end)
}

fn run(text: &str, marker: char, reverse: bool) -> usize {
    if reverse {
        text.chars().rev().take_while(|c| *c == marker).count()
    } else {
        text.chars().take_while(|c| *c == marker).count()
    }
}

fn fence(text: &str, minimum: usize) -> String {
    "`".repeat(
        text.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
            .max(minimum),
    )
}

fn transform(text: &str, selection: Range<usize>, action: Action) -> (String, Range<usize>) {
    let count = text.chars().count();
    let mut range = text.byte_index_from_char_index(CharIndex(selection.start.min(count))).0
        ..text.byte_index_from_char_index(CharIndex(selection.end.min(count))).0;
    if matches!(
        action,
        Action::Heading | Action::Bullet | Action::Ordered | Action::Quote | Action::CodeBlock | Action::LslBlock
    ) {
        range.start = text[..range.start].rfind('\n').map_or(0, |at| at + 1);
        if range.end > range.start && text[..range.end].ends_with('\n') {
            range.end -= 1;
        }
        range.end = text[range.end..].find('\n').map_or(text.len(), |at| range.end + at);
        return block(text, range, action);
    }
    // Keep whitespace outside inline delimiters, otherwise CommonMark does
    // not recognise emphasis (e.g. selecting a word and its trailing space).
    range.start += text[range.clone()].len() - text[range.clone()].trim_start().len();
    range.end = range.start + text[range.clone()].trim_end().len();
    let selected = &text[range.clone()];
    let content = if selected.is_empty() {
        aurora_i18n::tr!("notecard-format-placeholder")
    } else {
        selected
    };
    if action == Action::Link {
        let prefix = format!("[{content}](");
        let start = prefix.chars().count();
        return replace(text, range, &format!("{prefix}https://)"), start..start + 8);
    }
    let marker = match action {
        Action::Bold => "**".into(),
        Action::Italic => "*".into(),
        Action::Strike => "~~".into(),
        Action::Code => fence(content, 1),
        _ => unreachable!("block actions handled above"),
    };
    if action == Action::Code {
        let n = run(selected, '`', false);
        if n > 0 && n == run(selected, '`', true) && selected.len() > n * 2 {
            let inner = &selected[n..selected.len() - n];
            let inner = if inner.starts_with(' ') && inner.ends_with(' ') && !inner.trim().is_empty() {
                &inner[1..inner.len() - 1]
            } else {
                inner
            };
            return replace(text, range, inner, 0..inner.chars().count());
        }
    }
    let eligible = |s: &str, reverse| action != Action::Italic || run(s, '*', reverse) % 2 == 1;
    if selected.len() >= marker.len() * 2
        && selected.starts_with(&marker)
        && selected.ends_with(&marker)
        && eligible(selected, false)
        && eligible(selected, true)
    {
        let inner = &selected[marker.len()..selected.len() - marker.len()];
        return replace(text, range, inner, 0..inner.chars().count());
    }
    let before = &text[..range.start];
    let after = &text[range.end..];
    let outside = if action == Action::Code {
        let n = run(before, '`', true);
        if n > 0 && n == run(after, '`', false) {
            Some("`".repeat(n))
        } else if let (Some(before), Some(after)) = (before.strip_suffix(' '), after.strip_prefix(' ')) {
            let n = run(before, '`', true);
            if n > 0 && n == run(after, '`', false) {
                return replace(text, range.start - n - 1..range.end + n + 1, selected, 0..selected.chars().count());
            }
            None
        } else {
            None
        }
    } else {
        (before.ends_with(&marker) && after.starts_with(&marker) && eligible(before, true) && eligible(after, false))
            .then(|| marker.clone())
    };
    if let Some(outside) = outside {
        return replace(
            text,
            range.start - outside.len()..range.end + outside.len(),
            selected,
            0..selected.chars().count(),
        );
    }
    let padding = if action == Action::Code && (content.starts_with('`') || content.ends_with('`')) {
        " "
    } else {
        ""
    };
    let start = marker.len() + padding.len();
    replace(
        text,
        range,
        &format!("{marker}{padding}{content}{padding}{marker}"),
        start..start + content.chars().count(),
    )
}

fn prefix(line: &str, action: Action) -> usize {
    match action {
        Action::Heading => {
            let n = run(line, '#', false);
            if (1..=6).contains(&n) && line[n..].starts_with(' ') {
                n + 1
            } else {
                0
            }
        }
        Action::Bullet => {
            if ["- ", "* ", "+ "].iter().any(|p| line.starts_with(p)) {
                2
            } else {
                0
            }
        }
        Action::Ordered => {
            let digits = line.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && line[digits..].starts_with(". ") {
                digits + 2
            } else {
                0
            }
        }
        Action::Quote if line.starts_with("> ") => 2,
        _ => 0,
    }
}

fn block(text: &str, range: Range<usize>, action: Action) -> (String, Range<usize>) {
    let selected = &text[range.clone()];
    if matches!(action, Action::CodeBlock | Action::LslBlock) {
        let fence_line = |line: &str| {
            let count = run(line, '`', false);
            count >= 3 && (action == Action::CodeBlock || line[count..].trim().eq_ignore_ascii_case("lsl"))
        };
        if let Some(before) = text[..range.start].strip_suffix('\n') {
            let opening = before.rsplit('\n').next().unwrap_or_default();
            let closing = format!("\n{}", "`".repeat(run(opening, '`', false)));
            if fence_line(opening)
                && text[range.end..].starts_with(&closing)
                && text[range.end + closing.len()..].chars().next().is_none_or(|c| c == '\n')
            {
                return replace(
                    text,
                    range.start - opening.len() - 1..range.end + closing.len(),
                    selected,
                    0..selected.chars().count(),
                );
            }
        }
        let first = selected.lines().next().unwrap_or_default();
        let last = format!("\n{}", "`".repeat(run(first, '`', false)));
        if fence_line(first) && selected.ends_with(&last) {
            let inner = selected.get(first.len() + 1..selected.len() - last.len()).unwrap_or_default();
            return replace(text, range, inner, 0..inner.chars().count());
        }
        let content = if selected.is_empty() && action == Action::LslBlock {
            "default\n{\n    state_entry()\n    {\n        llOwnerSay(\"Hello!\");\n    }\n}"
        } else if selected.is_empty() {
            aurora_i18n::tr!("notecard-format-placeholder")
        } else {
            selected
        };
        let marker = fence(content, 3);
        let language = if action == Action::LslBlock { "lsl" } else { "" };
        let start = marker.len() + language.len() + 1;
        return replace(
            text,
            range,
            &format!("{marker}{language}\n{content}\n{marker}"),
            start..start + content.chars().count(),
        );
    }
    let remove = selected.lines().filter(|line| !line.trim().is_empty()).all(|line| {
        let line = line.trim_start();
        if action == Action::Heading {
            line.starts_with("## ")
        } else {
            prefix(line, action) > 0
        }
    }) && !selected.trim().is_empty();
    let mut number = 0;
    let updated = selected
        .split('\n')
        .map(|line| {
            if line.trim().is_empty() && !selected.is_empty() {
                return line.to_owned();
            }
            let indent = line.len() - line.trim_start().len();
            let body = &line[indent..];
            let n = prefix(body, action);
            let body = &body[n..];
            let marker = if remove {
                String::new()
            } else {
                match action {
                    Action::Heading => "## ".into(),
                    Action::Bullet => "- ".into(),
                    Action::Ordered => {
                        number += 1;
                        format!("{number}. ")
                    }
                    Action::Quote => "> ".into(),
                    _ => unreachable!("code block handled above"),
                }
            };
            let body = if body.is_empty() && !remove {
                aurora_i18n::tr!("notecard-format-placeholder")
            } else {
                body
            };
            format!("{}{marker}{body}", &line[..indent])
        })
        .collect::<Vec<_>>()
        .join("\n");
    let count = updated.chars().count();
    let start = if selected.is_empty() {
        count - aurora_i18n::tr!("notecard-format-placeholder").chars().count()
    } else {
        0
    };
    replace(text, range, &updated, start..count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_toggles_with_unicode_attachments_whitespace_and_nested_styles() {
        for action in [Action::Bold, Action::Italic, Action::Strike, Action::Code] {
            let source = "Before Été 🐺\u{100000} after";
            let (formatted, range) = transform(source, 7..14, action);
            assert_eq!(formatted.chars().filter(|c| *c == '\u{100000}').count(), 1);
            let (restored, _) = transform(&formatted, range, action);
            assert_eq!(restored, source);
        }
        assert_eq!(transform("word ", 0..5, Action::Bold).0, "**word** ");
        let (italic, range) = transform("**word**", 2..6, Action::Italic);
        assert_eq!(italic, "***word***");
        assert_eq!(transform(&italic, range, Action::Italic).0, "**word**");
        assert_eq!(transform("**word**", 0..8, Action::Bold).0, "word");
    }

    #[test]
    fn multiline_actions_toggle_and_exclude_the_next_unselected_line() {
        for action in [
            Action::Heading,
            Action::Bullet,
            Action::Ordered,
            Action::Quote,
            Action::CodeBlock,
            Action::LslBlock,
        ] {
            let source = "Été\nsecond\nuntouched";
            let (formatted, range) = transform(source, 1..11, action);
            assert!(formatted.ends_with("\nuntouched"));
            assert_eq!(transform(&formatted, range, action).0, source);
        }
        assert_eq!(transform("# One\n### Two", 0..13, Action::Heading).0, "## One\n## Two");
        assert_eq!(transform("one\ntwo", 0..7, Action::Ordered).0, "1. one\n2. two");
    }

    #[test]
    fn templates_select_editable_content_and_code_delimiters_survive_backticks() {
        for action in ACTIONS {
            let (text, range) = transform("", 0..0, action);
            assert!(!text.is_empty());
            assert!(range.start < range.end && range.end <= text.chars().count());
        }
        let (link, range) = transform("Été", 0..3, Action::Link);
        assert_eq!(link, "[Été](https://)");
        assert_eq!(link.chars().skip(range.start).take(range.len()).collect::<String>(), "https://");
        assert_eq!(transform("a`b", 0..3, Action::Code).0, "``a`b``");
        let (code, range) = transform("```", 0..3, Action::CodeBlock);
        assert_eq!(code, "````\n```\n````");
        assert_eq!(transform(&code, range, Action::CodeBlock).0, "```");
        let (code, range) = transform("llSay(0, \"Été 🐺\");", 0..18, Action::LslBlock);
        assert!(code.starts_with("```lsl\n"));
        assert_eq!(transform(&code, range, Action::LslBlock).0, "llSay(0, \"Été 🐺\");");
    }
}
