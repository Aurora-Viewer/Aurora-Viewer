//! Text selection across the lines of a conversation, like Firestorm's
//! transcript (FSChatHistory, a read-only LLTextEditor: a drag selects across
//! the lines without moving the window, Ctrl+A selects the whole
//! conversation, Ctrl+C copies it).
//!
//! The lines are laid out only where they are in sight (`view.rs`), so the
//! selection belongs to the conversation, not to egui's labels (whose
//! selection ends when one of its ends scrolls away): its two ends are a line
//! (by its number, `LineClock`) and a character of the text the line shows.
//!
//! - While a line is drawn (`row`), the pieces of its text (runs of text,
//!   links, names, emoji pictures) go through `label` / `atom`: they are
//!   recorded with where they stand, and the part of them that is selected
//!   is painted as in a text field.
//! - The pieces recorded this frame find the character under the pointer
//!   (`hit`).
//! - The copy takes, for a line selected from its start, its time and its
//!   sender ("[12:34] Name: text", Firestorm's transcript lines), then the
//!   selected text; the lines out of sight give the text they would show.

use crate::ui::bidi::{self, Visual};
use egui::{Pos2, Rect};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

/// One end of a selection: a line (its number) and a character of its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub seq: u64,
    /// Characters before it in the line's text (`usize::MAX`: its end).
    pub at: usize,
}

impl Point {
    pub fn start(seq: u64) -> Point {
        Point { seq, at: 0 }
    }

    pub fn end(seq: u64) -> Point {
        Point { seq, at: usize::MAX }
    }
}

/// The selection of a conversation.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Where the drag started (or the first line for Ctrl+A).
    pub anchor: Option<Point>,
    /// Where it is now.
    pub head: Option<Point>,
    /// The text of the lines its ends are in, as laid out when they were
    /// set (a partly selected line is copied from it).
    texts: HashMap<u64, String>,
    /// Set by a double click: the copy is the word alone, never the line's
    /// "[time] Name: " prefix, even for a word at the start of its line.
    word: bool,
}

impl Selection {
    /// Both ends, in the order of the conversation; None when nothing is
    /// selected.
    pub fn bounds(&self) -> Option<(Point, Point)> {
        let (a, h) = (self.anchor?, self.head?);
        let (s, e) = if a <= h { (a, h) } else { (h, a) };
        (s != e).then_some((s, e))
    }

    pub fn is_empty(&self) -> bool {
        self.bounds().is_none()
    }

    pub fn clear(&mut self) {
        *self = Selection::default();
    }

    /// Start a selection at a point (a press on the lines).
    pub fn start_at(&mut self, p: Point, row_text: &str) {
        self.word = false;
        self.texts.clear();
        self.texts.insert(p.seq, row_text.to_owned());
        self.anchor = Some(p);
        self.head = Some(p);
    }

    /// Move its free end (the drag goes on).
    pub fn extend_to(&mut self, p: Point, row_text: &str) {
        let anchor = self.anchor.map(|a| a.seq);
        self.texts.retain(|&seq, _| Some(seq) == anchor);
        self.texts.insert(p.seq, row_text.to_owned());
        self.head = Some(p);
    }

    /// Characters `range` of one line (a double click on a word).
    pub fn select_in_row(&mut self, seq: u64, range: Range<usize>, row_text: &str) {
        self.word = true;
        self.texts.clear();
        self.texts.insert(seq, row_text.to_owned());
        self.anchor = Some(Point { seq, at: range.start });
        self.head = Some(Point { seq, at: range.end });
    }

    /// Every line, from the first to the last one shown.
    pub fn select_all(&mut self, first: u64, last: u64) {
        self.word = false;
        self.texts.clear();
        self.anchor = Some(Point::start(first));
        self.head = Some(Point::end(last));
    }

    /// The characters of a line that are selected.
    pub fn of_row(&self, seq: u64) -> Option<Range<usize>> {
        let (s, e) = self.bounds()?;
        if seq < s.seq || seq > e.seq {
            return None;
        }
        let from = if seq == s.seq { s.at } else { 0 };
        let to = if seq == e.seq { e.at } else { usize::MAX };
        (from < to).then_some(from..to)
    }

    /// The selection as plain text, one line of the conversation per line:
    /// `rows` are the numbers of the lines shown, in order; `row` gives a
    /// line's prefix ("[12:34] Name: ") and the text it shows.
    pub fn copy_text(&self, rows: impl IntoIterator<Item = u64>, row: impl Fn(u64) -> (String, String)) -> String {
        let mut out: Vec<String> = Vec::new();
        for seq in rows {
            let Some(range) = self.of_row(seq) else {
                continue;
            };
            let (prefix, shown) = row(seq);
            // a line whose end was set by the pointer: the text laid out then
            let text = self.texts.get(&seq).cloned().unwrap_or(shown);
            let chars = text.chars().count();
            let (from, to) = (range.start.min(chars), range.end.min(chars));
            let part: String = text.chars().skip(from).take(to - from).collect();
            if from == 0 && !self.word {
                out.push(format!("{prefix}{part}"));
            } else if !part.is_empty() {
                out.push(part);
            }
        }
        out.join("\n")
    }
}

/// Where a piece of a line's text stands this frame.
#[derive(Clone)]
enum Shape {
    Galley {
        pos: Pos2,
        visual: Visual,
    },
    /// A link, a name, an emoji: selected whole.
    Atom(Rect),
}

#[derive(Clone)]
struct Piece {
    /// Its first character in the line's text.
    start: usize,
    len: usize,
    shape: Shape,
}

impl Piece {
    fn rect(&self) -> Rect {
        match &self.shape {
            Shape::Galley { pos, visual } => visual.galley.rect.translate(pos.to_vec2()),
            Shape::Atom(r) => *r,
        }
    }

    /// The character of the piece nearest to `pos`.
    fn at(&self, pos: Pos2) -> usize {
        match &self.shape {
            Shape::Galley { pos: origin, visual } => self.start + visual.cursor_at(pos - *origin).min(self.len),
            Shape::Atom(r) => self.start + if pos.x < r.center().x { 0 } else { self.len },
        }
    }
}

/// A line drawn this frame: where it stands, its text and its pieces.
#[derive(Clone)]
pub struct Row {
    pub seq: u64,
    pub rect: Rect,
    pub text: String,
    pieces: Vec<Piece>,
}

/// The line being drawn.
struct Current {
    seq: u64,
    text: String,
    chars: usize,
    selected: Option<Range<usize>>,
    /// Selection colours of the pieces (background, text): the theme's,
    /// else what a bubble asks for (`colors`).
    colors: Option<(egui::Color32, egui::Color32)>,
    pieces: Vec<Piece>,
}

thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
    static DRAWN: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
}

/// Draw a line whose text can be selected: `selected` is its part of the
/// selection; `painted` false when it is only measured.
pub fn row<R>(ui: &mut egui::Ui, seq: u64, selected: Option<Range<usize>>, painted: bool, draw: impl FnOnce(&mut egui::Ui) -> R) -> R {
    CURRENT.with_borrow_mut(|c| {
        *c = Some(Current {
            seq,
            text: String::new(),
            chars: 0,
            selected,
            colors: None,
            pieces: Vec::new(),
        });
    });
    let out = draw(ui);
    if let Some(c) = CURRENT.with_borrow_mut(Option::take)
        && painted
    {
        let rect = ui.min_rect();
        DRAWN.with_borrow_mut(|d| {
            d.push(Row {
                seq: c.seq,
                rect,
                text: c.text,
                pieces: c.pieces,
            });
        });
    }
    out
}

/// The stable number of the message whose sender is being drawn.
pub(super) fn current_seq() -> Option<u64> {
    CURRENT.with_borrow(|c| c.as_ref().map(|c| c.seq))
}

/// The lines drawn since the last call (this frame's).
pub fn take_drawn() -> Vec<Row> {
    DRAWN.with_borrow_mut(std::mem::take)
}

/// The selection colours of the next pieces of the line being drawn (the
/// text of my bubbles is on the accent, the theme's selection would vanish
/// on it).
pub fn colors(background: egui::Color32, text: egui::Color32) {
    CURRENT.with_borrow_mut(|c| {
        if let Some(c) = c {
            c.colors = Some((background, text));
        }
    });
}

/// A run of text: in a line of a conversation it is recorded and its
/// selected part painted; elsewhere the usual selectable label.
pub fn label(ui: &mut egui::Ui, text: egui::RichText) -> egui::Response {
    if CURRENT.with_borrow(Option::is_none) {
        return ui.add(egui::Label::new(text).wrap().selectable(true));
    }
    let shown = text.text().to_owned();
    let (pos, visual, resp) = if bidi::has_rtl(&shown) {
        bidi_label(ui, text)
    } else {
        let (pos, galley, resp) = egui::Label::new(text).wrap().selectable(false).layout_in_ui(ui);
        (pos, Visual::plain(galley), resp)
    };
    let len = shown.chars().count();
    let (start, selected, colors) = CURRENT.with_borrow_mut(|c| {
        let c = c.as_mut().expect("a line is being drawn");
        let start = c.chars;
        c.chars += len;
        c.text.push_str(&shown);
        let selected = c
            .selected
            .clone()
            .map(|s| s.start.max(start).saturating_sub(start)..s.end.min(start + len).saturating_sub(start))
            .filter(|r| r.start < r.end);
        (start, selected, c.colors)
    });
    let mut galley = visual.galley.clone();
    if ui.is_rect_visible(resp.rect) {
        if let Some(r) = selected.clone().filter(|_| visual.reordered()) {
            // the selection of a reordered text may be in several places of
            // a row: its background, under the text, then its letters in the
            // selected text colour
            let (bg, fg) = colors.unwrap_or((ui.visuals().selection.bg_fill, ui.visuals().selection.stroke.color));
            for rect in visual.range_rects(pos, r.clone()) {
                ui.painter().add(egui::Shape::rect_filled(rect, 0.0, bg));
            }
            visual.tint(&mut galley, r, fg);
        } else if let Some(r) = selected {
            let mut visuals = ui.visuals().clone();
            if let Some((bg, fg)) = colors {
                visuals.selection.bg_fill = bg;
                visuals.selection.stroke.color = fg;
            }
            let range = egui::text::CCursorRange::two(egui::text::CCursor::new(r.start), egui::text::CCursor::new(r.end));
            egui::text_selection::visuals::paint_text_selection(&mut galley, &visuals, &range, None);
        }
        ui.painter()
            .add(egui::epaint::TextShape::new(pos, galley.clone(), ui.visuals().text_color()));
    }
    CURRENT.with_borrow_mut(|c| {
        if let Some(c) = c {
            c.pieces.push(Piece {
                start,
                len,
                shape: Shape::Galley { pos, visual },
            });
        }
    });
    resp
}

/// `Label::layout_in_ui` for a text with right-to-left characters, in
/// display order (`bidi`): in a wrapping row of widgets it starts after the
/// previous one and goes on on the rows below, as a label does.
fn bidi_label(ui: &mut egui::Ui, text: egui::RichText) -> (Pos2, Visual, egui::Response) {
    let job = egui::WidgetText::from(text).into_layout_job(ui.style(), egui::FontSelection::Default, ui.text_valign());
    let mut job = Arc::unwrap_or_clone(job);
    let sense = egui::Sense::hover();
    let available = ui.available_width();
    if ui.layout().main_dir() == egui::Direction::LeftToRight && ui.layout().main_wrap() && available.is_finite() {
        // (Label::layout_in_ui's own rules for a wrapping layout)
        job.wrap.max_width = available;
        job.first_row_min_height = ui.cursor().height();
        job.halign = egui::Align::Min;
        job.justify = false;
        if let Some(first) = job.sections.first_mut() {
            first.leading_space = available - ui.available_size_before_wrap().x;
        }
        let visual = ui.fonts_mut(|f| bidi::layout(f, job, bidi::Base::Auto));
        let pos = egui::pos2(ui.max_rect().left(), ui.cursor().top());
        let mut rows = visual.galley.rows.iter();
        let first = rows.next().map_or(Rect::NOTHING, |r| r.rect_without_leading_space());
        let mut response = ui.allocate_rect(first.translate(pos.to_vec2()), sense);
        for row in rows {
            response |= ui.allocate_rect(row.rect().translate(pos.to_vec2()), sense);
        }
        return (pos, visual, response);
    }
    job.wrap.max_width = available;
    job.halign = ui.layout().horizontal_placement();
    let visual = ui.fonts_mut(|f| bidi::layout(f, job, bidi::Base::Auto));
    let (rect, response) = ui.allocate_exact_size(visual.galley.size(), egui::Sense::hover());
    let pos = match visual.galley.job.halign {
        egui::Align::LEFT => rect.left_top(),
        egui::Align::Center => rect.center_top(),
        egui::Align::RIGHT => rect.right_top(),
    };
    (pos, visual, response)
}

/// Before a piece selected whole (a link, a name, an emoji): its place
/// under the text for the selection's background, when a line is drawn.
pub fn atom_begin(ui: &egui::Ui) -> Option<egui::layers::ShapeIdx> {
    CURRENT.with_borrow(Option::is_some).then(|| ui.painter().add(egui::Shape::Noop))
}

/// After it: `text` is what it shows, `rect` where.
pub fn atom_end(ui: &egui::Ui, behind: Option<egui::layers::ShapeIdx>, text: &str, rect: Rect) {
    let Some(behind) = behind else {
        return;
    };
    let len = text.chars().count();
    let selected = CURRENT.with_borrow_mut(|c| {
        let c = c.as_mut()?;
        let start = c.chars;
        c.chars += len;
        c.text.push_str(text);
        c.pieces.push(Piece {
            start,
            len,
            shape: Shape::Atom(rect),
        });
        let s = c.selected.clone()?;
        (s.start < start + len && s.end > start).then(|| c.colors.map_or(ui.visuals().selection.bg_fill, |(bg, _)| bg))
    });
    if let Some(bg) = selected {
        ui.painter().set(behind, egui::Shape::rect_filled(rect, 0.0, bg));
    }
}

/// The character under `pos` among the lines drawn: above them the start of
/// the first, under them the end of the last.
pub fn hit(rows: &[Row], pos: Pos2) -> Option<(Point, &str)> {
    let first = rows.first()?;
    let last = rows.last()?;
    if pos.y < first.rect.top() {
        return Some((Point::start(first.seq), &first.text));
    }
    if pos.y >= last.rect.bottom() {
        return Some((Point::end(last.seq), &last.text));
    }
    let row = rows.iter().find(|r| pos.y < r.rect.bottom()).unwrap_or(last);
    let at = match nearest_piece(row, pos) {
        Some(p) => p.at(pos),
        // a line without text (a card): its start or its end
        None if pos.y < row.rect.center().y => 0,
        None => usize::MAX,
    };
    Some((Point { seq: row.seq, at }, &row.text))
}

/// What a double click at `pos` selects, like LLTextEditor::handleDoubleClick
/// (indra/llui/lltexteditor.cpp): a link, a name or an emoji whole, else the
/// word under the pointer (`word_range`). None off the lines or on a space.
pub fn word_at(rows: &[Row], pos: Pos2) -> Option<(u64, Range<usize>, &str)> {
    let row = rows.iter().find(|r| r.rect.top() <= pos.y && pos.y < r.rect.bottom())?;
    let piece = nearest_piece(row, pos)?;
    let range = match piece.shape {
        Shape::Atom(_) => piece.start..piece.start + piece.len,
        Shape::Galley { .. } => word_range(&row.text, piece.at(pos))?,
    };
    Some((row.seq, range, &row.text))
}

/// LLWStringUtil::isPartOfWord: a letter, a digit or '_'.
fn part_of_word(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

/// The characters a double click at caret `at` of `text` selects: the run of
/// word characters it is on, else the one character that is not a space;
/// nothing on a space. The caret falls between two characters: the one after
/// it counts first, then the one before (the pointer on the right half of a
/// word's last letter).
fn word_range(text: &str, at: usize) -> Option<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let word = [Some(at), at.checked_sub(1)]
        .into_iter()
        .flatten()
        .find(|&i| chars.get(i).copied().is_some_and(part_of_word));
    let Some(i) = word else {
        return chars.get(at).is_some_and(|c| !c.is_whitespace()).then(|| at..at + 1);
    };
    let start = chars[..i].iter().rposition(|c| !part_of_word(*c)).map_or(0, |p| p + 1);
    let end = chars[i..].iter().position(|c| !part_of_word(*c)).map_or(chars.len(), |p| i + p);
    Some(start..end)
}

/// The piece on the pointer's row of text, else the nearest one above or
/// below it.
fn nearest_piece(row: &Row, pos: Pos2) -> Option<&Piece> {
    let distance = |p: &Piece| {
        let r = p.rect();
        let dy = if pos.y < r.top() {
            r.top() - pos.y
        } else {
            (pos.y - r.bottom()).max(0.0)
        };
        let dx = if pos.x < r.left() {
            r.left() - pos.x
        } else {
            (pos.x - r.right()).max(0.0)
        };
        (dy, dx)
    };
    row.pieces
        .iter()
        .min_by(|a, b| distance(a).partial_cmp(&distance(b)).unwrap_or(std::cmp::Ordering::Equal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(sel: &Selection, lines: &[(u64, &str, &str)]) -> String {
        sel.copy_text(lines.iter().map(|l| l.0), |seq| {
            let l = lines.iter().find(|l| l.0 == seq).expect("a line shown");
            (l.1.to_owned(), l.2.to_owned())
        })
    }

    const LINES: [(u64, &str, &str); 4] = [
        (10, "[12:00] Jane: ", "hello there"),
        (11, "[12:01] Bob: ", "hi Jane"),
        (12, "[12:02] ", "Jane waves"),
        (13, "[12:03] Bob: ", "bye"),
    ];

    #[test]
    fn the_ends_come_in_any_order() {
        let mut sel = Selection::default();
        assert!(sel.is_empty());
        sel.start_at(Point { seq: 12, at: 4 }, "Jane waves");
        assert!(sel.is_empty(), "a press alone selects nothing");
        sel.extend_to(Point { seq: 10, at: 6 }, "hello there");
        assert_eq!(sel.bounds(), Some((Point { seq: 10, at: 6 }, Point { seq: 12, at: 4 })));
        assert_eq!(sel.of_row(10), Some(6..usize::MAX));
        assert_eq!(sel.of_row(11), Some(0..usize::MAX));
        assert_eq!(sel.of_row(12), Some(0..4));
        assert_eq!(sel.of_row(13), None);
        assert_eq!(sel.of_row(9), None);
    }

    #[test]
    fn a_copy_names_the_lines_taken_from_their_start() {
        let mut sel = Selection::default();
        sel.start_at(Point { seq: 10, at: 6 }, "hello there");
        sel.extend_to(Point { seq: 12, at: 4 }, "Jane waves");
        assert_eq!(copy(&sel, &LINES), "there\n[12:01] Bob: hi Jane\n[12:02] Jane");
        // within one line
        sel.start_at(Point { seq: 11, at: 3 }, "hi Jane");
        sel.extend_to(Point { seq: 11, at: 7 }, "hi Jane");
        assert_eq!(copy(&sel, &LINES), "Jane");
        // the whole conversation, out of sight or not
        sel.select_all(10, 13);
        assert_eq!(
            copy(&sel, &LINES),
            "[12:00] Jane: hello there\n[12:01] Bob: hi Jane\n[12:02] Jane waves\n[12:03] Bob: bye"
        );
        // lines hidden by the filter are not in `rows`: not copied
        assert_eq!(copy(&sel, &[LINES[0], LINES[3]]), "[12:00] Jane: hello there\n[12:03] Bob: bye");
        sel.clear();
        assert_eq!(copy(&sel, &LINES), "");
    }

    #[test]
    fn a_partly_selected_line_is_copied_as_it_was_laid_out() {
        // the end was set on a line whose text shows a name the copy of a
        // line out of sight would not know yet
        let mut sel = Selection::default();
        sel.start_at(Point { seq: 10, at: 0 }, "hello there");
        sel.extend_to(Point { seq: 11, at: 7 }, "hi @Jane Doe");
        assert_eq!(copy(&sel, &LINES), "[12:00] Jane: hello there\n[12:01] Bob: hi @Jan");
    }

    /// LLTextEditor::handleDoubleClick: the word under the caret (letters,
    /// digits, '_'), else the one character that is not a space.
    #[test]
    fn a_double_click_takes_the_word_or_the_character_under_it() {
        let text = "hi, l'été_2 !";
        // on a word, from either side of a letter
        assert_eq!(word_range(text, 0), Some(0..2));
        assert_eq!(word_range(text, 2), Some(0..2), "right half of its last letter");
        // accents and digits are part of a word, the apostrophe is not
        assert_eq!(word_range(text, 7), Some(6..11));
        assert_eq!(word_range(text, 5), Some(4..5));
        // punctuation alone, nothing on a space or past the end
        assert_eq!(word_range(text, 12), Some(12..13));
        assert_eq!(word_range("a , b", 2), Some(2..3));
        assert_eq!(word_range("a  b", 2), None);
        assert_eq!(word_range(text, usize::MAX), None);
        // a message of one word
        assert_eq!(word_range("hello", 3), Some(0..5));
    }

    #[test]
    fn the_pointer_finds_its_line_and_character() {
        let ctx = egui::Context::default();
        let mut drawn = Vec::new();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            },
            |ui| {
                for (seq, text) in [(5, "first line"), (6, "second line")] {
                    row(ui, seq, None, true, |ui| {
                        label(ui, egui::RichText::new(text).size(13.0));
                    });
                }
                drawn = take_drawn();
            },
        );
        // no renderer here (the font atlas would trip a debug assertion)
        out.textures_delta.clear();
        assert_eq!(drawn.len(), 2);
        let (a, b) = (&drawn[0], &drawn[1]);
        assert_eq!((a.text.as_str(), b.text.as_str()), ("first line", "second line"));
        // the start of the second line, its end, between two letters
        let left = Pos2::new(b.rect.left() - 5.0, b.rect.center().y);
        assert_eq!(hit(&drawn, left).map(|(p, _)| p), Some(Point { seq: 6, at: 0 }));
        let right = Pos2::new(b.rect.right() + 50.0, b.rect.center().y);
        assert_eq!(hit(&drawn, right).map(|(p, _)| p), Some(Point { seq: 6, at: 11 }));
        let inside = hit(&drawn, Pos2::new(b.rect.left() + b.rect.width() * 0.5, b.rect.center().y)).map(|(p, _)| p.at);
        assert!(inside.is_some_and(|at| (3..9).contains(&at)), "{inside:?}");
        // above and below the lines
        assert_eq!(
            hit(&drawn, Pos2::new(10.0, a.rect.top() - 20.0)).map(|(p, _)| p),
            Some(Point::start(5))
        );
        assert_eq!(
            hit(&drawn, Pos2::new(10.0, b.rect.bottom() + 20.0)).map(|(p, _)| p),
            Some(Point::end(6))
        );
        // outside a line nothing is recorded: the usual label
        assert!(CURRENT.with_borrow(Option::is_none));
    }
}
