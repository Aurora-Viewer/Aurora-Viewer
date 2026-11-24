//! Find and replace in the text of a notecard: the Legacy Viewer's
//! LLFloaterSearchReplace (indra/newview/llfloatersearchreplace.cpp,
//! Copyright (c) 2010-2013 Kitty Barnett, originally LGPL 2.1), opened by
//! Ctrl+F (LLPreviewNotecard::handleKeyHere) or the window's Search button
//! (LLPreviewNotecard::onSearchButtonClicked). Same fields, options and
//! defaults: the text to find, its replacement, "Case insensitive" and
//! "Search upwards" both off (LSLFindCaseInsensitivity, LSLFindDirection),
//! Find, Replace and Replace all; every match of the text is highlighted
//! while it is open (LLTextEditor::setHighlightWord). The matching and the
//! one-step undo are the script editor's (`scripts::editing`).
//!
//! Aurora draws it as a bar at the top of the notecard window, not as a
//! separate floater: it belongs to the text it searches and never covers it.
//! An embedded inventory item is one marker character of the text: it is
//! never part of a search or a replacement, so a replacement never removes
//! an item nor makes one up.

use std::{ops::Range, sync::Arc};

use crate::{
    theme::Palette,
    ui::{scripts::editing, widgets},
};
use aurora_net::inventory::notecard::FIRST_EMBEDDED_CHAR;
use egui::{Id, RichText};

/// Width of the two text fields.
const FIELD_WIDTH: f32 = 200.0;

#[derive(Default)]
pub(super) struct Find {
    pub open: bool,
    query: String,
    replacement: String,
    insensitive: bool,
    whole_words: bool,
    up: bool,
    /// Put the keyboard in the text to find, its text selected.
    focus: bool,
    status: String,
    /// The match Find last went to (characters of the text).
    current: Option<Range<usize>>,
    /// The matches of `query` in `cached`, while neither changes.
    cached: Option<Cached>,
}

struct Cached {
    text: String,
    query: String,
    insensitive: bool,
    whole_words: bool,
    ranges: Arc<Vec<Range<usize>>>,
}

/// What the editor paints for the open bar, as it was set on this frame or
/// the last one.
#[derive(Clone)]
pub(super) struct Highlights {
    /// The character ranges of every match, in order and apart.
    pub ranges: Arc<Vec<Range<usize>>>,
    /// The match the bar last went to, when it is still selected.
    pub current: Option<Range<usize>>,
    frame: u64,
}

impl Highlights {
    /// The matches overlapping the characters `chars`.
    pub fn within(&self, chars: Range<usize>) -> &[Range<usize>] {
        let start = self.ranges.partition_point(|r| r.end <= chars.start);
        let end = start + self.ranges[start..].partition_point(|r| r.start < chars.end);
        &self.ranges[start..end]
    }
}

fn highlights_id(editor: Id) -> Id {
    editor.with("find_highlights")
}

fn scroll_id(editor: Id) -> Id {
    editor.with("find_scroll")
}

/// The open bar's matches in the editor `editor`, if any.
pub(super) fn highlights(ctx: &egui::Context, editor: Id) -> Option<Highlights> {
    let now = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data(|d| d.get_temp::<Highlights>(highlights_id(editor)))
        .filter(|h| h.frame + 1 >= now)
}

/// The bar went to a match: the editor scrolls it into view once.
pub(super) fn take_scroll(ctx: &egui::Context, editor: Id) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(scroll_id(editor))).unwrap_or(false)
}

/// The ids of the bar's two fields in the window of the editor `editor`.
pub(super) fn field_ids(editor: Id) -> [Id; 2] {
    [editor.with("find_query"), editor.with("find_replacement")]
}

/// `text` without the markers of embedded items.
fn plain(text: &str) -> String {
    text.chars().filter(|c| (*c as u32) < FIRST_EMBEDDED_CHAR).collect()
}

impl Find {
    /// Open the bar on `editor`, showing `text`: its selection becomes the
    /// text to find, and the keyboard goes to that field (onOpen).
    pub fn begin(&mut self, ctx: &egui::Context, editor: Id, text: &str) {
        let selection = editing::bytes(text, editing::cursor(ctx, editor));
        let selected = plain(&text[selection]);
        if !selected.is_empty() && !selected.contains('\n') {
            self.query = selected;
        }
        self.open = true;
        self.focus = true;
        self.status.clear();
        // The selected match stays the current one (LLFloaterSearchReplace
        // keeps the editor's selection), so Replace changes that one even
        // after the bar takes the keyboard and egui shrinks the selection.
        // Without one, the match the bar went to stays current while the
        // caret egui left on it has not moved (Ctrl+F again after Find).
        let ranges = self.matches(text);
        let selected = Self::selected(ctx, editor);
        self.current = if ranges.contains(&selected) {
            Some(selected)
        } else {
            self.current(ctx, editor, &ranges)
        };
    }

    /// Close the bar and give the keyboard back to the text (onClose).
    pub fn close(&mut self, ctx: &egui::Context, editor: Id) {
        self.hide();
        ctx.memory_mut(|m| m.request_focus(editor));
    }

    /// Close the bar with its document; the text to find and the options
    /// stay for the next one, as the Legacy floater keeps them.
    pub fn hide(&mut self) {
        self.open = false;
        self.status.clear();
        self.current = None;
        self.cached = None;
    }

    /// The bar over `text`, the content of the editor `editor`; Replace and
    /// Replace all only when `editable`. True when it changed `text`.
    pub fn bar(&mut self, ui: &mut egui::Ui, p: &Palette, editor: Id, text: &mut String, editable: bool) -> bool {
        if !self.open {
            return false;
        }
        let ctx = ui.ctx().clone();
        let [query_id, replacement_id] = field_ids(editor);
        // before the fields: egui's own Escape takes the keyboard from them
        let in_bar = ctx.memory(|m| m.focused()).is_some_and(|id| id == query_id || id == replacement_id);
        let mut changed = false;
        let mut close = false;
        ui.horizontal(|ui| {
            let response = widgets::search_text_edit(ui, p, &mut self.query, |edit| {
                edit.id(query_id)
                    .hint_text(aurora_i18n::tr!("script-search-query"))
                    .desired_width(FIELD_WIDTH)
            })
            .on_hover_text(aurora_i18n::tr!("script-search-query"));
            if std::mem::take(&mut self.focus) {
                editing::select(&ctx, query_id, 0..self.query.chars().count(), true);
            }
            if response.changed() {
                self.status.clear();
            }
            if (response.has_focus() || response.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                self.find(&ctx, editor, text);
                response.request_focus();
            }
            if widgets::checkbox(ui, p, &mut self.insensitive, aurora_i18n::tr!("script-search-insensitive")).changed() {
                self.status.clear();
            }
            if widgets::checkbox(ui, p, &mut self.whole_words, aurora_i18n::tr!("script-search-whole-words")).changed() {
                self.current = None;
                self.status.clear();
            }
            widgets::checkbox(ui, p, &mut self.up, aurora_i18n::tr!("script-search-up"));
            if widgets::flat_button(ui, p, aurora_i18n::tr!("script-search-find")).clicked() {
                self.find(&ctx, editor, text);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true);
                if !self.status.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&self.status).color(p.muted)).truncate())
                        .on_hover_text(&self.status);
                }
            });
        });
        ui.horizontal(|ui| {
            widgets::text_field(ui, editable, &mut self.replacement, |buffer, _| {
                egui::TextEdit::singleline(buffer)
                    .id(replacement_id)
                    .hint_text(aurora_i18n::tr!("script-search-replacement"))
                    .desired_width(FIELD_WIDTH)
            })
            .on_hover_text(aurora_i18n::tr!("script-search-replacement"));
            ui.add_enabled_ui(editable && !self.query.is_empty(), |ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("script-search-replace")).clicked() {
                    changed |= self.replace(&ctx, editor, text);
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("script-search-replace-all")).clicked() {
                    changed |= self.replace_all(&ctx, editor, text);
                }
            });
        });
        // Escape in one of the bar's fields closes it.
        if close || (in_bar && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))) {
            self.close(&ctx, editor);
            return changed;
        }
        self.publish(&ctx, editor, text);
        changed
    }

    /// The matches of the text to find in `text`, computed again only when
    /// one of them changed.
    fn matches(&mut self, text: &str) -> Arc<Vec<Range<usize>>> {
        let query = plain(&self.query);
        if let Some(cached) = &self.cached
            && cached.insensitive == self.insensitive
            && cached.whole_words == self.whole_words
            && cached.query == query
            && cached.text == text
        {
            return cached.ranges.clone();
        }
        let ranges = Arc::new(editing::matches_filtered(text, &query, self.insensitive, self.whole_words));
        self.cached = Some(Cached {
            text: text.to_owned(),
            query,
            insensitive: self.insensitive,
            whole_words: self.whole_words,
            ranges: ranges.clone(),
        });
        ranges
    }

    /// The match the bar went to, while it is still one and the editor's
    /// cursor has not moved since. egui keeps a selection only in the field
    /// with the keyboard: elsewhere it shrinks to its moving end, the start
    /// of a match selected right to left, else its end.
    fn current(&self, ctx: &egui::Context, editor: Id, ranges: &[Range<usize>]) -> Option<Range<usize>> {
        let current = self.current.clone()?;
        let selected = Self::selected(ctx, editor);
        let caret_at_edge = selected.is_empty() && (selected.start == current.start || selected.start == current.end);
        (ranges.contains(&current) && (selected == current || caret_at_edge)).then_some(current)
    }

    fn selected(ctx: &egui::Context, editor: Id) -> Range<usize> {
        let [start, end] = editing::cursor(ctx, editor).sorted_cursors();
        start.index.0..end.index.0
    }

    /// Where Find, Replace and Search upwards start from: the current
    /// match, else the editor's selection (LLTextEditor's selection).
    fn selection(&self, ctx: &egui::Context, editor: Id, ranges: &[Range<usize>]) -> Range<usize> {
        self.current(ctx, editor, ranges).unwrap_or_else(|| Self::selected(ctx, editor))
    }

    /// Hand the matches to the editor, which paints them.
    fn publish(&mut self, ctx: &egui::Context, editor: Id, text: &str) {
        let ranges = self.matches(text);
        let highlights = Highlights {
            current: self.current(ctx, editor, &ranges),
            ranges,
            frame: ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT),
        };
        ctx.data_mut(|d| d.insert_temp(highlights_id(editor), highlights));
    }

    /// Select the next match after the selection (before it with "Search
    /// upwards"), wrapping around: LLTextEditor::selectNext.
    pub fn find(&mut self, ctx: &egui::Context, editor: Id, text: &str) {
        let ranges = self.matches(text);
        let from = self.selection(ctx, editor, &ranges);
        self.current = editing::next_match(&ranges, from, self.up);
        if let Some(found) = self.current.clone() {
            editing::select(ctx, editor, found, false);
            ctx.data_mut(|d| d.insert_temp(scroll_id(editor), true));
            self.status.clear();
        } else {
            self.status = aurora_i18n::tr!("script-search-not-found").into();
        }
    }

    /// Replace the selection when it is a match, then go to the next one:
    /// LLTextEditor::replaceText.
    fn replace(&mut self, ctx: &egui::Context, editor: Id, text: &mut String) -> bool {
        let ranges = self.matches(text);
        let selected = self.selection(ctx, editor, &ranges);
        let changed = ranges.contains(&selected);
        if changed {
            editing::replace_text(ctx, editor, text, selected, &plain(&self.replacement));
        }
        self.find(ctx, editor, text);
        changed
    }

    /// What the demo scenarios type in the two fields.
    pub fn demo_texts(&mut self, query: &str, replacement: &str) {
        query.clone_into(&mut self.query);
        replacement.clone_into(&mut self.replacement);
    }

    /// Replace every match, as one undo step: LLTextEditor::replaceTextAll.
    pub fn replace_all(&mut self, ctx: &egui::Context, editor: Id, text: &mut String) -> bool {
        let ranges = self.matches(text);
        if ranges.is_empty() {
            self.status = aurora_i18n::tr!("script-search-not-found").into();
            return false;
        }
        self.current = None;
        let output = editing::replaced_all(text, &ranges, &plain(&self.replacement));
        editing::replace_text(ctx, editor, text, 0..text.chars().count(), &output);
        self.status = aurora_i18n::tr!("script-search-replaced", count = ranges.len() as i64);
        true
    }
}

/// The rectangles, in galley coordinates, of the matches of `highlights`
/// on the rows between `top` and `bottom`, the current one apart: one walk
/// over the rows, whatever the number of matches.
pub(super) fn rects(galley: &egui::Galley, highlights: &Highlights, top: f32, bottom: f32) -> Vec<(egui::Rect, bool)> {
    let mut rects = Vec::new();
    let mut row_start = 0;
    for row in &galley.rows {
        let count = row.char_count_including_newline().0;
        let chars = row_start..row_start + row.row.char_count_excluding_newline().0;
        row_start += count;
        if row.max_y() < top {
            continue;
        }
        if row.min_y() > bottom {
            break;
        }
        for range in highlights.within(chars.clone()) {
            let from = range.start.max(chars.start) - chars.start;
            let to = range.end.min(chars.end) - chars.start;
            let left = row.pos.x + row.row.x_offset(egui::text::CharIndex(from));
            let right = row.pos.x + row.row.x_offset(egui::text::CharIndex(to));
            let rect = egui::Rect::from_x_y_ranges(left..=right, row.min_y()..=row.max_y());
            rects.push((rect, highlights.current.as_ref() == Some(range)));
        }
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "pose1 <0,0,1>\nPose2 <0,0,2>\n\u{100000} POSE3";

    fn find(query: &str, insensitive: bool) -> Find {
        Find {
            open: true,
            query: query.into(),
            insensitive,
            ..Default::default()
        }
    }

    fn selected(ctx: &egui::Context, id: Id, text: &str) -> String {
        text[editing::bytes(text, editing::cursor(ctx, id))].to_owned()
    }

    #[test]
    fn whole_words_filters_matches_and_refreshes_cache() {
        let ctx = egui::Context::default();
        let id = Id::unique("note_whole_words");
        let mut bar = find("hi", false);
        let mut text = "hi high hi_there (hi)".to_owned();

        // Without the option, partial-word matches are included.
        let all = bar.matches(&text);
        assert_eq!(all.len(), 4);

        // Changing the option must invalidate cached results.
        bar.whole_words = true;
        let whole = bar.matches(&text);
        assert_eq!(whole.len(), 2);
        assert!(!Arc::ptr_eq(&all, &whole));

        // Highlighting and replacement use the same filtered matches.
        bar.publish(&ctx, id, &text);
        assert_eq!(highlights(&ctx, id).expect("published").ranges.len(), 2);

        bar.replacement = "HELLO".into();
        assert!(bar.replace_all(&ctx, id, &mut text));
        assert_eq!(text, "HELLO high hi_there (HELLO)");

        bar.whole_words = false;
        assert_eq!(bar.matches(&text).len(), 2);
    }
    #[test]
    fn find_is_case_sensitive_by_default_and_wraps_both_ways() {
        let ctx = egui::Context::default();
        let id = Id::unique("note_find");
        let mut bar = find("pose", false);
        bar.find(&ctx, id, NOTE);
        assert_eq!(
            editing::cursor(&ctx, id).as_sorted_char_range(),
            egui::text::CharIndex(0)..egui::text::CharIndex(4)
        );
        bar.find(&ctx, id, NOTE);
        // the only match: it wraps around to itself
        assert_eq!(
            editing::cursor(&ctx, id).as_sorted_char_range(),
            egui::text::CharIndex(0)..egui::text::CharIndex(4)
        );
        let mut bar = find("pose", true);
        bar.up = true;
        bar.find(&ctx, id, NOTE);
        assert_eq!(selected(&ctx, id, NOTE), "POSE");
        bar.find(&ctx, id, NOTE);
        assert_eq!(selected(&ctx, id, NOTE), "Pose");
        assert!(take_scroll(&ctx, id), "going to a match scrolls the editor once");
        assert!(!take_scroll(&ctx, id));
        let mut bar = find("absent", true);
        bar.find(&ctx, id, NOTE);
        assert_eq!(bar.status, aurora_i18n::tr!("script-search-not-found"));
    }

    #[test]
    fn replace_takes_the_selected_match_only_then_goes_to_the_next() {
        let ctx = egui::Context::default();
        let id = Id::unique("note_replace");
        let mut text = NOTE.to_owned();
        let mut bar = find("pose", true);
        bar.replacement = "anim".into();
        // nothing selected yet: Replace only goes to the first match
        assert!(!bar.replace(&ctx, id, &mut text));
        assert_eq!(text, NOTE);
        assert!(bar.replace(&ctx, id, &mut text));
        assert!(text.starts_with("anim1 <0,0,1>\nPose2"));
        assert_eq!(selected(&ctx, id, &text), "Pose");
    }

    #[test]
    fn the_current_match_survives_egui_shrinking_the_selection() {
        let ctx = egui::Context::default();
        let id = Id::unique("note_shrunk");
        let mut text = NOTE.to_owned();
        let mut bar = find("pose", true);
        bar.replacement = "anim".into();
        bar.find(&ctx, id, &text);
        bar.find(&ctx, id, &text);
        assert_eq!(bar.current, Some(14..18));
        // what egui does to an editor without the keyboard
        editing::select(&ctx, id, 18..18, false);
        bar.publish(&ctx, id, &text);
        assert_eq!(highlights(&ctx, id).expect("published").current, Some(14..18));
        bar.up = true;
        bar.find(&ctx, id, &text);
        assert_eq!(bar.current, Some(0..4), "upwards from the match, not from its end");
        editing::select(&ctx, id, 4..4, false);
        assert!(bar.replace(&ctx, id, &mut text), "Replace takes the shrunk match");
        assert!(text.starts_with("anim1 <0,0,1>\nPose2"));
        // the cursor moved by hand: the search starts from there
        editing::select(&ctx, id, 20..20, false);
        bar.up = false;
        bar.find(&ctx, id, &text);
        assert_eq!(selected(&ctx, id, &text), "POSE");
    }

    #[test]
    fn replace_all_is_one_undo_step_and_keeps_embedded_items() {
        let ctx = egui::Context::default();
        let id = Id::unique("note_replace_all");
        let mut text = NOTE.to_owned();
        let mut bar = find("pose", true);
        // a marker pasted in either field never reaches the text
        bar.query.push('\u{100001}');
        bar.replacement = "sit\u{100002}".into();
        assert!(bar.replace_all(&ctx, id, &mut text));
        assert_eq!(text, "sit1 <0,0,1>\nsit2 <0,0,2>\n\u{100000} sit3");
        assert_eq!(bar.status, aurora_i18n::tr!("script-search-replaced", count = 3_i64));
        editing::undo(&ctx, id, &mut text, false);
        assert_eq!(text, NOTE);
        editing::undo(&ctx, id, &mut text, true);
        assert_eq!(text, "sit1 <0,0,1>\nsit2 <0,0,2>\n\u{100000} sit3");
        let mut none = find("absent", false);
        assert!(!none.replace_all(&ctx, id, &mut text));
        assert_eq!(none.status, aurora_i18n::tr!("script-search-not-found"));
    }

    #[test]
    fn begin_takes_a_one_line_selection_without_its_items() {
        let ctx = egui::Context::default();
        let id = Id::unique("note_begin");
        let mut bar = Find {
            query: "kept".into(),
            ..Default::default()
        };
        // no selection: the last text stays
        bar.begin(&ctx, id, NOTE);
        assert!(bar.open && bar.focus);
        assert_eq!(bar.query, "kept");
        let start = NOTE.chars().position(|c| c == '\u{100000}').expect("item");
        editing::select(&ctx, id, start..start + 3, false);
        bar.begin(&ctx, id, NOTE);
        assert_eq!(bar.query, " P");
        // a selection over several lines is no text to find
        editing::select(&ctx, id, 0..20, false);
        bar.begin(&ctx, id, NOTE);
        assert_eq!(bar.query, " P");
    }

    #[test]
    fn replace_after_begin_changes_the_selected_word_whichever_way_it_was_selected() {
        // egui shrinks the editor's selection to its moving end once the bar
        // takes the keyboard: the start of a word selected right to left.
        for (anchor, caret) in [(7, 12), (12, 7)] {
            let ctx = egui::Context::default();
            let id = Id::unique(("note_replace_selected", anchor));
            let mut text = "count, count, count_max".to_owned();
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(anchor),
                egui::text::CCursor::new(caret),
            )));
            state.store(&ctx, id);
            let mut bar = Find {
                replacement: "total".into(),
                ..Default::default()
            };
            bar.begin(&ctx, id, &text);
            assert_eq!(bar.query, "count");
            editing::select(&ctx, id, caret..caret, false);
            assert!(bar.replace(&ctx, id, &mut text), "{anchor}..{caret}");
            assert_eq!(text, "count, total, count_max", "{anchor}..{caret}");
        }
    }

    #[test]
    fn highlights_list_the_matches_of_a_span_and_follow_the_selection() {
        let ctx = egui::Context::default();
        let id = Id::unique("note_highlights");
        let mut bar = find("pose", true);
        bar.publish(&ctx, id, NOTE);
        let shown = highlights(&ctx, id).expect("published");
        assert_eq!(shown.ranges.len(), 3);
        assert!(shown.current.is_none());
        let first = shown.ranges[0].clone();
        assert_eq!(first, 0..4);
        assert_eq!(shown.within(0..3), std::slice::from_ref(&first));
        assert_eq!(shown.within(3..14), std::slice::from_ref(&first));
        assert!(shown.within(4..14).is_empty());
        assert_eq!(shown.within(14..15), std::slice::from_ref(&shown.ranges[1]));
        assert_eq!(shown.ranges[1], 14..18);
        assert_eq!(shown.within(0..usize::MAX).len(), 3);
        bar.find(&ctx, id, NOTE);
        bar.publish(&ctx, id, NOTE);
        assert_eq!(highlights(&ctx, id).expect("published").current, Some(0..4));
        // the cache follows a changed text
        bar.publish(&ctx, id, "pose");
        assert_eq!(highlights(&ctx, id).expect("published").ranges.len(), 1);
        bar.close(&ctx, id);
        assert!(bar.cached.is_none());
    }
}
