//! LLScriptEdCore::performAction, LLTextEditor selection / search (Firestorm,
//! LGPL 2.1). Commands share egui's cursor and undo history with keyboard edits.

use egui::{
    Id, TextBuffer,
    text::{CCursor, CCursorRange, CharIndex},
};
use std::ops::Range;

pub(super) fn cursor(ctx: &egui::Context, id: Id) -> CCursorRange {
    egui::TextEdit::load_state(ctx, id)
        .and_then(|s| s.cursor.char_range())
        .unwrap_or_default()
}

pub(super) fn select(ctx: &egui::Context, id: Id, range: Range<usize>, focus: bool) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end))));
    state.store(ctx, id);
    if focus {
        ctx.memory_mut(|m| m.request_focus(id));
    }
}

pub(super) fn bytes(source: &str, range: CCursorRange) -> Range<usize> {
    let [start, end] = range.sorted_cursors();
    source.byte_index_from_char_index(start.index).0..source.byte_index_from_char_index(end.index).0
}

pub(super) fn replace(ctx: &egui::Context, id: Id, source: &mut String, range: Range<usize>, text: &str) -> bool {
    let start = source.byte_index_from_char_index(CharIndex(range.start)).0;
    let end = source.byte_index_from_char_index(CharIndex(range.end)).0;
    if source.len() - (end - start) + text.len() > aurora_net::scripts::MAX_SOURCE {
        return false;
    }
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let mut undo = state.undoer();
    undo.add_undo(&(cursor(ctx, id), source.clone()));
    source.replace_range(start..end, text);
    let at = CCursorRange::one(CCursor::new(range.start + text.chars().count()));
    state.cursor.set_char_range(Some(at));
    undo.add_undo(&(at, source.clone()));
    state.set_undoer(undo);
    state.store(ctx, id);
    ctx.data_mut(|d| d.insert_temp(Id::new((id, "history_dirty")), true));
    true
}

pub(super) fn undo(ctx: &egui::Context, id: Id, source: &mut String, redo: bool) {
    let Some(mut state) = egui::TextEdit::load_state(ctx, id) else {
        return;
    };
    let current = (cursor(ctx, id), source.clone());
    let mut undo = state.undoer();
    let next = if redo { undo.redo(&current) } else { undo.undo(&current) };
    if let Some((range, text)) = next {
        source.clone_from(text);
        state.cursor.set_char_range(Some(*range));
    }
    state.set_undoer(undo);
    state.store(ctx, id);
    ctx.data_mut(|d| d.insert_temp(Id::new((id, "history_dirty")), true));
}

pub(super) fn position(source: &str, at: usize) -> (usize, usize) {
    let byte = source.byte_index_from_char_index(CharIndex(at)).0;
    let prefix = &source[..byte];
    (
        prefix.bytes().filter(|c| *c == b'\n').count(),
        prefix.rsplit('\n').next().unwrap_or_default().chars().count(),
    )
}

pub(super) fn line_cursor(source: &str, line: usize, column: usize) -> usize {
    let mut at = 0;
    for (n, text) in source.split('\n').enumerate() {
        if n == line {
            return at + column.min(text.trim_end_matches('\r').chars().count());
        }
        at += text.chars().count() + 1;
    }
    source.chars().count()
}

pub(super) fn error_location(error: &str) -> Option<(usize, usize)> {
    let text = error.trim().strip_prefix('(')?.split_once(')')?.0;
    let (line, column) = text.split_once(',')?;
    Some((line.trim().parse().ok()?, column.trim().parse().ok()?))
}

pub(super) fn word(source: &str, at: usize) -> &str {
    let byte = source.byte_index_from_char_index(CharIndex(at)).0;
    let is_word = |c: char| c == '_' || c.is_alphanumeric();
    let start = source[..byte]
        .char_indices()
        .rev()
        .find(|(_, c)| !is_word(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = source[byte..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(source.len(), |(i, _)| byte + i);
    &source[start..end]
}

/// Unicode lowercase may change byte length: keep original character ranges.
pub(super) fn matches(source: &str, needle: &str, insensitive: bool) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    if !insensitive {
        let mut previous = 0;
        let mut chars = 0;
        return source
            .match_indices(needle)
            .map(|(byte, text)| {
                chars += source[previous..byte].chars().count();
                let start = chars;
                chars += text.chars().count();
                previous = byte + text.len();
                start..chars
            })
            .collect();
    }
    let mut folded = String::new();
    let mut boundaries = Vec::new();
    for (n, c) in source.chars().enumerate() {
        boundaries.push((folded.len(), n));
        folded.extend(c.to_lowercase());
    }
    boundaries.push((folded.len(), source.chars().count()));
    let needle = needle.to_lowercase();
    folded
        .match_indices(&needle)
        .filter_map(|(byte, text)| {
            let start = boundaries.binary_search_by_key(&byte, |b| b.0).ok()?;
            let end = boundaries.binary_search_by_key(&(byte + text.len()), |b| b.0).ok()?;
            Some(boundaries[start].1..boundaries[end].1)
        })
        .collect()
}

pub(super) fn next_match(ranges: &[Range<usize>], selection: Range<usize>, up: bool) -> Option<Range<usize>> {
    if up {
        ranges
            .iter()
            .rev()
            .find(|r| r.end <= selection.start)
            .or_else(|| ranges.last())
            .cloned()
    } else {
        ranges
            .iter()
            .find(|r| r.start >= selection.end && **r != selection)
            .or_else(|| ranges.first())
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_edits_share_native_undo_and_redo_and_reject_oversized_input() {
        let ctx = egui::Context::default();
        let id = Id::new("command_edit");
        let mut source = "éあ start".to_owned();
        select(&ctx, id, 2..8, false);
        assert!(replace(&ctx, id, &mut source, 2..8, " finish"));
        assert_eq!(source, "éあ finish");
        undo(&ctx, id, &mut source, false);
        assert_eq!(source, "éあ start");
        assert_eq!(
            cursor(&ctx, id).as_sorted_char_range(),
            egui::text::CharIndex(2)..egui::text::CharIndex(8)
        );
        undo(&ctx, id, &mut source, true);
        assert_eq!(source, "éあ finish");
        let before = source.clone();
        assert!(!replace(&ctx, id, &mut source, 0..0, &"x".repeat(aurora_net::scripts::MAX_SOURCE)));
        assert_eq!(source, before);
    }
    #[test]
    fn search_ranges_wrap_and_handle_unicode_case_expansion() {
        let ranges = matches("Été été ÉTÉ", "été", true);
        assert_eq!(ranges, vec![0..3, 4..7, 8..11]);
        assert_eq!(next_match(&ranges, 8..11, false), Some(0..3));
        assert_eq!(next_match(&ranges, 0..3, true), Some(8..11));
        assert!(matches("İ", "i", true).is_empty());
        assert_eq!(matches("İ", "i\u{307}", true), vec![0..1]);
        assert!(matches("abc", "", true).is_empty());
    }
    #[test]
    fn navigation_clamps_lines_and_columns_and_parses_diagnostics() {
        assert_eq!(position("é\r\nあx", 4), (1, 1));
        assert_eq!(line_cursor("é\r\nあx", 1, 1), 4);
        assert_eq!(line_cursor("é\r\nあx", 0, 99), 1);
        assert_eq!(line_cursor("é\r\nあx", 99, 0), 5);
        assert_eq!(error_location("(3, 7) : ERROR : Syntax error"), Some((3, 7)));
        assert_eq!(error_location("network error"), None);
        assert_eq!(word("é llSay(0)", 4), "llSay");
    }
}
