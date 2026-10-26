//! Spelling in a text field, called right after the field is shown: the
//! auto-replace of the word typed (LLLineEditor::handleUnicodeCharHere's
//! mAutoreplaceCallback, FIRE-11373) and the red squiggles under the
//! misspelled words (LLLineEditor::draw, LLTextBase::drawMisspelled,
//! indra/llui, originally LGPL 2.1).
//!
//! The Legacy Viewer checks only the fields that ask for it
//! (`spellcheck="true"`: the chat bars, conversations, notecards, group
//! notices, landmarks, descriptions…); here those fields call [`field`].
//! The words found are kept for the field's id, where the text menu
//! (`ui/text_menu.rs`) reads them for its suggestions. The text is checked
//! again only when it, the dictionary or the user's words change.
//!
//! As in the Legacy Viewer, the word being typed gets no squiggle until
//! the keyboard has rested half a second (SPELLCHECK_DELAY).

use crate::theme::Palette;
use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditOutput;
use egui::{Id, Pos2, Stroke};
use std::hash::BuildHasher as _;
use std::ops::Range;
use std::sync::Arc;

/// SPELLCHECK_DELAY of lllineeditor.cpp / lltexteditor.cpp.
const TYPING_DELAY: f64 = 0.5;

/// The misspelled words of a field: their character ranges and themselves.
pub type Marked = Arc<Vec<(Range<usize>, String)>>;

#[derive(Clone, Default)]
struct Cache {
    hash: u64,
    revision: u64,
    marked: Marked,
    /// When the text last changed (egui's input time).
    edited: f64,
    /// The pass that drew the field last.
    pass: u64,
}

fn cache_id(field: Id) -> Id {
    field.with("aurora_spelling")
}

/// The misspelled words of `field`, if it was drawn checked at the last
/// pass or this one (for the text menu).
pub fn marked(ctx: &egui::Context, field: Id) -> Option<Marked> {
    let cache: Cache = ctx.data(|d| d.get_temp(cache_id(field)))?;
    (ctx.cumulative_pass_nr() <= cache.pass + 1).then_some(cache.marked)
}

/// After an editable field: `replace` asks for the auto-replace (the
/// Legacy chat bars, conversations and notecards), the squiggles come with
/// the spell checker on.
pub fn field(ui: &egui::Ui, p: &Palette, output: &mut TextEditOutput, text: &mut String, replace: bool) {
    if replace && auto_replace(ui, output, text) {
        // the galley shows the old text: the squiggles wait for the next frame
        return;
    }
    if !crate::spelling::active() {
        return;
    }
    let ctx = ui.ctx();
    let id = output.response.id;
    let now = ui.input(|i| i.time);
    let hash = foldhash::fast::FixedState::default().hash_one(text.as_str());
    let revision = crate::spelling::revision();
    let mut cache: Cache = ctx.data(|d| d.get_temp(cache_id(id))).unwrap_or_default();
    if cache.hash != hash || cache.revision != revision {
        if cache.hash != hash && cache.pass != 0 {
            cache.edited = now;
        }
        cache.hash = hash;
        cache.revision = revision;
        cache.marked = Arc::new(crate::spelling::misspelled(text));
    }
    cache.pass = ctx.cumulative_pass_nr();
    let marked = cache.marked.clone();
    let edited = cache.edited;
    ctx.data_mut(|d| d.insert_temp(cache_id(id), cache));
    if marked.is_empty() {
        return;
    }
    // the word under the cursor while typing
    let typing = output.response.has_focus() && now - edited < TYPING_DELAY;
    let cursor = output.cursor_range.map(|r| r.primary.index.0);
    if typing {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(TYPING_DELAY - (now - edited)));
    }
    let painter = ui.painter().with_clip_rect(output.text_clip_rect);
    let stroke = Stroke::new(1.0, p.danger);
    for (range, _) in marked.iter() {
        if typing && cursor.is_some_and(|c| range.start <= c && c <= range.end) {
            continue;
        }
        for (left, right, bottom) in segments(output, range.clone()) {
            squiggle(&painter, left, right, bottom, stroke);
        }
    }
}

/// The pieces of a word on screen, one per row it spans: (left, right,
/// baseline bottom).
fn segments(output: &TextEditOutput, range: Range<usize>) -> Vec<(f32, f32, f32)> {
    let galley = &output.galley;
    let origin = output.galley_pos.to_vec2();
    let mut out = Vec::new();
    let mut start = galley.pos_from_cursor(CCursor::new(range.start));
    let mut prev = start;
    for i in range.start + 1..=range.end {
        let at = galley.pos_from_cursor(CCursor::new(i));
        // a new row: the piece of the previous one ends there
        if (at.min.y - prev.min.y).abs() > 0.5 {
            out.push((start.min.x + origin.x, prev.min.x + origin.x, prev.max.y + origin.y));
            let mut next_row = CCursor::new(i - 1);
            next_row.prefer_next_row = true;
            start = galley.pos_from_cursor(next_row);
        }
        prev = at;
    }
    out.push((start.min.x + origin.x, prev.min.x + origin.x, prev.max.y + origin.y));
    out.retain(|(left, right, _)| right - left > 1.0);
    out
}

/// The Legacy zigzag (lllineeditor.cpp: 2 px up, 2 px down), at the
/// bottom of the row.
fn squiggle(painter: &egui::Painter, left: f32, right: f32, bottom: f32, stroke: Stroke) {
    let (low, high) = (bottom - 0.5, bottom - 2.5);
    let mut points = Vec::new();
    let mut x = left;
    let mut up = false;
    while x < right {
        points.push(Pos2::new(x, if up { high } else { low }));
        up = !up;
        x += 2.0;
    }
    points.push(Pos2::new(right, if up { high } else { low }));
    painter.add(egui::Shape::line(points, stroke));
}

/// The auto-replace after a typed space (LLAutoReplace::autoreplaceCallback);
/// true when the text changed.
fn auto_replace(ui: &egui::Ui, output: &mut TextEditOutput, text: &mut String) -> bool {
    if !crate::autoreplace::enabled() || !output.response.changed() {
        return false;
    }
    let typed_space = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(s) if s.ends_with(' '))));
    let Some(at) = output.cursor_range.filter(|r| r.is_empty()).map(|r| r.primary.index.0) else {
        return false;
    };
    if !typed_space {
        return false;
    }
    let lists = crate::autoreplace::lists();
    let Some((replaced, cursor)) = crate::autoreplace::after_space(&lists, text, at) else {
        return false;
    };
    *text = replaced;
    let range = CCursorRange::one(CCursor::new(cursor));
    output.cursor_range = Some(range);
    output.state.cursor.set_char_range(Some(range));
    output.state.clone().store(ui.ctx(), output.response.id);
    output.response.mark_changed();
    ui.ctx().request_repaint();
    true
}

#[cfg(test)]
mod tests;
