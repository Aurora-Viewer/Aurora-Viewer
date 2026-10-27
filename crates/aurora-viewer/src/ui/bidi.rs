//! Right-to-left text (Arabic, Hebrew…) in the order it is read: the Unicode
//! Bidirectional Algorithm (UAX #9, `unicode-bidi`) on top of egui's layout.
//!
//! egui 0.37 lays every row out from left to right. Its shaper (harfrust)
//! writes a run of one font from right to left when the run's first letter
//! is a right-to-left one, so an Arabic word comes out joined and in order,
//! but the words of a sentence (and the numbers among them) stay ordered
//! from left to right, and Arabic-Indic digits come out reversed. Here a
//! text that holds such characters is broken into rows first, each row is
//! put in display order (rules L1 and L2, brackets mirrored by rule L4), and
//! egui gets that order with each right-to-left word in its own section, in
//! reading order: the shaper reverses and joins the word, and nothing else
//! (a section never shares a run with another).
//!
//! The text egui draws is then no longer the text in reading order: callers
//! that need the place of a character (links, selection) ask [`Visual`],
//! which keeps the box of each character of the original text.
//!
//! The Legacy Viewer has neither shaping nor bidi (no HarfBuzz nor FriBidi in
//! its sources, LLFontGL draws characters in memory order): this goes beyond
//! it.

use egui::epaint::text::{ByteIndex, FontsView, Galley, LayoutJob, LayoutSection, TextWrapping};
use egui::{Pos2, Rect, Vec2};
use std::ops::Range;
use std::sync::Arc;
use unicode_bidi::{BidiClass, Level, ParagraphBidiInfo, bidi_class};

/// The direction of a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base {
    /// That of its first letter (rules P2 and P3): an Arabic sentence reads
    /// from the right, an English one with an Arabic word from the left.
    Auto,
    /// Left to right, the characters `range` (a sender's name, its picture
    /// before it) isolated as between FSI and PDI (rules X5c, X6a): the name
    /// stays first on the left in its own direction, and an Arabic message
    /// after it is not reordered with it.
    Sender(Range<usize>),
}

/// Whether egui would misplace part of `text`: a right-to-left letter or an
/// Arabic-Indic digit. The other texts are laid out as they are.
pub fn has_rtl(text: &str) -> bool {
    text.chars()
        .any(|c| c >= '\u{0590}' && matches!(bidi_class(c), BidiClass::R | BidiClass::AL | BidiClass::AN))
}

/// Whether a paragraph of `text` reads from right to left: its first
/// letter is a right-to-left one (rules P2 and P3).
pub fn rtl_paragraph(text: &str) -> bool {
    has_rtl(text) && unicode_bidi::get_base_direction(text) == unicode_bidi::Direction::Rtl
}

/// Lay `job` out like `fonts.layout_job`, in display order.
pub fn layout(fonts: &mut FontsView<'_>, job: LayoutJob, base: Base) -> Visual {
    lay_out(fonts, job, base, true)
}

/// [`layout`] for a caller that only paints the text.
pub fn galley(ctx: &egui::Context, job: LayoutJob, base: Base) -> Arc<Galley> {
    ctx.fonts_mut(|f| lay_out(f, job, base, false)).galley
}

/// `with_map`: also keep where each character went (its box), which a
/// caller that only paints the text (every frame: name tags, floating
/// text) does without.
fn lay_out(fonts: &mut FontsView<'_>, job: LayoutJob, base: Base, with_map: bool) -> Visual {
    if !has_rtl(&job.text) {
        return Visual {
            galley: fonts.layout_job(job),
            map: None,
        };
    }
    let mut measure = |text: &str, section: &LayoutSection| piece(fonts, text, section).width;
    let rows = visual_rows(&job, base, &mut measure);
    let (visual_job, pieces) = visual_job(&job, &rows);
    let mut galley = fonts.layout_job(visual_job);
    // cut at its overflow character: egui laid out a text already cut, the
    // caller (a tooltip on an elided label) asks the galley
    if rows.iter().any(|r| r.segments.iter().any(|s| s.at.contains(&usize::MAX))) {
        Arc::make_mut(&mut galley).elided = true;
    }
    let map = with_map.then(|| {
        Arc::new(Map {
            boxes: boxes(fonts, &job, &rows, &pieces, &galley),
            len: job.text.chars().count(),
        })
    });
    Visual { galley, map }
}

/// `FontsView::layout_no_wrap` in display order, each paragraph in its own
/// direction: a name, a title, an object's floating text.
pub fn no_wrap(ctx: &egui::Context, text: String, font_id: egui::FontId, color: egui::Color32) -> Arc<Galley> {
    galley(ctx, LayoutJob::simple(text, font_id, color, f32::INFINITY), Base::Auto)
}

/// A text laid out in display order, and where each of its characters went.
#[derive(Clone)]
pub struct Visual {
    pub galley: Arc<Galley>,
    /// None: the text was laid out as it is, the galley's characters are
    /// those of the text.
    map: Option<Arc<Map>>,
}

struct Map {
    /// By character of the original text, relative to the galley: a
    /// character that is not drawn (the space a row ends on, one elided)
    /// has a box of no width at the end of its row, or none.
    boxes: Vec<CharBox>,
    len: usize,
}

#[derive(Clone, Copy)]
struct CharBox {
    rect: Rect,
    /// Read from right to left: the start of the character is its right
    /// edge.
    rtl: bool,
}

impl CharBox {
    const NONE: CharBox = CharBox {
        rect: Rect::NOTHING,
        rtl: false,
    };
}

impl Visual {
    /// A galley laid out as it is (its characters are those of the text).
    pub fn plain(galley: Arc<Galley>) -> Visual {
        Visual { galley, map: None }
    }

    /// The cursor before the character `at` of the text: a rect of no
    /// width on its row, relative to the galley.
    pub fn cursor_rect(&self, at: usize) -> Rect {
        let b = self.map.as_ref().and_then(|m| m.boxes.get(at)).filter(|b| b.rect.is_finite());
        match b {
            Some(b) => {
                let x = if b.rtl { b.rect.max.x } else { b.rect.min.x };
                Rect::from_x_y_ranges(x..=x, b.rect.y_range())
            }
            None => self.galley.pos_from_cursor(egui::text::CCursor::new(at)),
        }
    }

    /// The left edge of the characters `range` of the text on the row of
    /// its first one, whatever their direction: where a picture before a
    /// name goes. A rect of no width, relative to the galley.
    pub fn left_edge(&self, range: Range<usize>) -> Rect {
        let Some(map) = &self.map else {
            return self.cursor_rect(range.start);
        };
        let drawn: Vec<&CharBox> = map
            .boxes
            .get(range.start.min(map.len)..range.end.min(map.len))
            .unwrap_or_default()
            .iter()
            .filter(|b| b.rect.is_positive())
            .collect();
        let Some(first) = drawn.first() else {
            return self.cursor_rect(range.start);
        };
        let x = drawn
            .iter()
            .filter(|b| (b.rect.min.y - first.rect.min.y).abs() < 0.5)
            .map(|b| b.rect.min.x)
            .fold(f32::INFINITY, f32::min);
        Rect::from_x_y_ranges(x..=x, first.rect.y_range())
    }

    /// Paint the glyphs of the characters `range` of the text in `color`
    /// (selected text), in `galley`, a copy of this one's.
    pub fn tint(&self, galley: &mut Arc<Galley>, range: Range<usize>, color: egui::Color32) {
        let rects = self.range_rects(Pos2::ZERO, range);
        if rects.is_empty() {
            return;
        }
        let galley = Arc::make_mut(galley);
        for placed in galley.rows.iter_mut() {
            let origin = placed.pos;
            let row = Arc::make_mut(&mut placed.row);
            let end = row.visuals.glyph_vertex_range.end as u32;
            let starts: Vec<u32> = row.glyphs.iter().map(|g| g.first_vertex).collect();
            for (i, g) in row.glyphs.iter().enumerate() {
                let center = origin + Vec2::new(g.pos.x + g.advance_width * 0.5, 0.0);
                if !rects
                    .iter()
                    .any(|r| r.x_range().contains(center.x) && r.y_range().contains(origin.y + row.size.y * 0.5))
                {
                    continue;
                }
                let to = starts.get(i + 1).copied().unwrap_or(end);
                for v in row
                    .visuals
                    .mesh
                    .vertices
                    .get_mut(g.first_vertex as usize..to as usize)
                    .unwrap_or_default()
                {
                    v.color = color;
                }
            }
        }
    }

    /// Whether the text was reordered (the galley's character indices are
    /// not those of the text).
    pub fn reordered(&self) -> bool {
        self.map.is_some()
    }

    /// The rects that cover the characters `range` of the text, one per run
    /// of neighbouring characters of a row, at `origin`.
    pub fn range_rects(&self, origin: Pos2, range: Range<usize>) -> Vec<Rect> {
        let Some(map) = &self.map else {
            return super::chat::range_rects(&self.galley, origin, range);
        };
        let mut out: Vec<Rect> = Vec::new();
        for b in map.boxes.get(range.start.min(map.len)..range.end.min(map.len)).unwrap_or_default() {
            if !b.rect.is_positive() {
                continue;
            }
            let r = b.rect.translate(origin.to_vec2());
            let touching = out
                .iter_mut()
                .find(|o| (o.min.y - r.min.y).abs() < 0.5 && ((o.max.x - r.min.x).abs() < 1.0 || (r.max.x - o.min.x).abs() < 1.0));
            match touching {
                Some(o) => *o = o.union(r),
                None => out.push(r),
            }
        }
        out
    }

    /// The cursor (characters of the text before it) nearest to `pos`,
    /// relative to the galley.
    pub fn cursor_at(&self, pos: Vec2) -> usize {
        let Some(map) = &self.map else {
            return self.galley.cursor_from_pos(pos).index.0;
        };
        // the row under the pointer, else the nearest one
        let row_dist = |b: &CharBox| {
            if b.rect.y_range().contains(pos.y) {
                0.0
            } else {
                (b.rect.center().y - pos.y).abs()
            }
        };
        let drawn = || {
            map.boxes
                .iter()
                .enumerate()
                .filter(|(_, b)| b.rect.is_finite() && b.rect.height() > 0.0)
        };
        let Some(best_row) = drawn().map(|(_, b)| row_dist(b)).min_by(f32::total_cmp) else {
            return 0;
        };
        let x_dist = |b: &CharBox| {
            if b.rect.x_range().contains(pos.x) {
                0.0
            } else {
                (b.rect.min.x - pos.x).abs().min((b.rect.max.x - pos.x).abs())
            }
        };
        let Some((i, b)) = drawn()
            .filter(|(_, b)| row_dist(b) <= best_row + 0.5)
            .min_by(|a, b| x_dist(a.1).total_cmp(&x_dist(b.1)))
        else {
            return 0;
        };
        // before the character when the pointer is on the half it starts with
        let first_half = pos.x < b.rect.center().x;
        if first_half != b.rtl { i } else { i + 1 }
    }
}

/// A character of a row, in reading order.
#[derive(Debug, Clone, Copy)]
struct RowChar {
    c: char,
    /// Its index in the text.
    at: usize,
    section: usize,
    level: Level,
}

/// A piece of a row in display order: a section of the job given to egui.
#[derive(Debug, Clone, PartialEq)]
struct Segment {
    /// What egui is given (a right-to-left word in reading order, which the
    /// shaper reverses; brackets of a right-to-left run mirrored).
    text: String,
    /// The index in the original text of each character of `text`, in its
    /// order (`usize::MAX`: the overflow character).
    at: Vec<usize>,
    /// By character of `text`: read from right to left (an odd level),
    /// where its start is its right edge.
    rtl: Vec<bool>,
    /// The section of the original job whose format it takes.
    section: usize,
    /// A right-to-left word: the shaper draws it reversed.
    word: bool,
    /// Holds one right-to-left character only: nothing may join it.
    alone: bool,
}

/// A row of the text: its segments in display order, and the characters of
/// the text it holds without drawing them (trailing spaces of a wrapped row,
/// elided ones).
#[derive(Debug, Clone, Default, PartialEq)]
struct VisualRow {
    segments: Vec<Segment>,
    hidden: Vec<usize>,
    /// The paragraph reads from right to left.
    rtl: bool,
}

/// Characters the shaper would write from right to left if a run started
/// with them: those of the right-to-left scripts. Outside a word, each one
/// goes in its own section.
fn rtl_script(c: char) -> bool {
    matches!(c as u32,
        0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

/// Characters of a right-to-left word: letters, their marks and the joiners.
fn word_char(c: char) -> bool {
    matches!(bidi_class(c), BidiClass::R | BidiClass::AL | BidiClass::NSM | BidiClass::BN)
}

/// Rule L4: a bracket in a right-to-left run is drawn as its mirror.
fn mirror(c: char) -> char {
    unicode_bidi_mirroring::get_mirrored(c).unwrap_or(c)
}

/// The segments of a row (its characters in reading order, levels resolved
/// for the row), in display order.
fn segments(row: &[RowChar]) -> Vec<Segment> {
    let levels: Vec<Level> = row.iter().map(|c| c.level).collect();
    let order = ParagraphBidiInfo::reorder_visual(&levels);
    let mut out: Vec<Segment> = Vec::new();
    let mut k = 0;
    while k < order.len() {
        let ch = row[order[k]];
        if ch.level.is_rtl() && word_char(ch.c) {
            // the characters drawn after it that come before it in the text
            let mut end = k + 1;
            while end < order.len()
                && order[end] + 1 == order[end - 1]
                && row[order[end]].level == ch.level
                && row[order[end]].section == ch.section
                && word_char(row[order[end]].c)
            {
                end += 1;
            }
            let word = &row[order[end - 1]..=order[k]];
            out.push(Segment {
                text: word.iter().map(|c| c.c).collect(),
                at: word.iter().map(|c| c.at).collect(),
                rtl: word.iter().map(|c| c.level.is_rtl()).collect(),
                section: ch.section,
                word: true,
                alone: false,
            });
            k = end;
            continue;
        }
        let shown = if ch.level.is_rtl() { mirror(ch.c) } else { ch.c };
        let alone = rtl_script(ch.c);
        match out.last_mut() {
            Some(last) if !alone && !last.word && !last.alone && last.section == ch.section => {
                last.text.push(shown);
                last.at.push(ch.at);
                last.rtl.push(ch.level.is_rtl());
            }
            _ => out.push(Segment {
                text: shown.to_string(),
                at: vec![ch.at],
                rtl: vec![ch.level.is_rtl()],
                section: ch.section,
                word: false,
                alone,
            }),
        }
        k += 1;
    }
    out
}

/// The rows of `job` in display order. `measure` gives the width of a text
/// in the format of a section (no wrapping).
fn visual_rows(job: &LayoutJob, base: Base, measure: &mut dyn FnMut(&str, &LayoutSection) -> f32) -> Vec<VisualRow> {
    let text = job.text.as_str();
    // the section of each byte's character
    let section_of = |byte: usize| {
        job.sections
            .iter()
            .position(|s| s.byte_range.start.0 <= byte && byte < s.byte_range.end.0)
            .unwrap_or(0)
    };
    let wrap = &job.wrap;
    let mut rows: Vec<VisualRow> = Vec::new();
    let mut char_at = 0;
    let paragraphs: Vec<&str> = if job.break_on_newline {
        text.split('\n').collect()
    } else {
        vec![text]
    };
    for (p, para) in paragraphs.iter().enumerate() {
        let para_start = para.as_ptr() as usize - text.as_ptr() as usize;
        let chars: Vec<(usize, char)> = para.char_indices().collect();
        let level = match base {
            Base::Auto => None,
            Base::Sender(_) => Some(Level::ltr()),
        };
        // the sender's name, in characters of this paragraph: the algorithm
        // is given the paragraph with FSI and PDI around it (3 bytes each)
        let isolated = match &base {
            Base::Sender(r) if r.start >= char_at && r.start < r.end && r.start - char_at < chars.len() => {
                Some(r.start - char_at..(r.end - char_at).min(chars.len()))
            }
            _ => None,
        };
        let shift = |k: usize| {
            isolated
                .as_ref()
                .map_or(0, |r| 3 * (usize::from(k >= r.start) + usize::from(k >= r.end)))
        };
        let marked: String;
        let analysed = match &isolated {
            Some(r) => {
                let mut m = String::with_capacity(para.len() + 6);
                for (k, &(_, c)) in chars.iter().enumerate() {
                    if k == r.start {
                        m.push('\u{2068}');
                    }
                    if k == r.end {
                        m.push('\u{2069}');
                    }
                    m.push(c);
                }
                if r.end == chars.len() {
                    m.push('\u{2069}');
                }
                marked = m;
                marked.as_str()
            }
            None => para,
        };
        let info = ParagraphBidiInfo::new(analysed, level);
        let rtl = info.paragraph_level.is_rtl();
        // the width of a run of the paragraph's characters, section by
        // section (a section is shaped on its own)
        let mut width = |range: Range<usize>| -> f32 {
            let mut w = 0.0;
            let mut k = range.start;
            while k < range.end {
                let s = section_of(para_start + chars[k].0);
                let mut e = k + 1;
                while e < range.end && section_of(para_start + chars[e].0) == s {
                    e += 1;
                }
                let from = chars[k].0;
                let to = chars.get(e).map_or(para.len(), |c| c.0);
                let section = &job.sections[s];
                let lead = if section.byte_range.start.0 == para_start + from {
                    section.leading_space
                } else {
                    0.0
                };
                w += lead + measure(&para[from..to], section);
                k = e;
            }
            w
        };
        let ranges = break_rows(&chars, wrap, &mut width);
        let last_para = p + 1 == paragraphs.len();
        for (r, range) in ranges.iter().enumerate() {
            let soft = r + 1 < ranges.len();
            // a wrapped row does not draw the spaces it ends on
            let mut shown = range.clone();
            while soft && shown.end > shown.start && chars[shown.end - 1].1.is_whitespace() {
                shown.end -= 1;
            }
            // in the analysed text (the name's marks counted)
            let byte = |k: usize| chars.get(k).map_or(para.len(), |c| c.0) + shift(k);
            let levels = if shown.is_empty() {
                Vec::new()
            } else {
                info.reordered_levels(byte(shown.start)..byte(shown.end))
            };
            let row: Vec<RowChar> = (shown.clone())
                .map(|k| RowChar {
                    c: chars[k].1,
                    at: char_at + k,
                    section: section_of(para_start + chars[k].0),
                    level: levels[byte(k)],
                })
                .collect();
            rows.push(VisualRow {
                segments: segments(&row),
                hidden: (shown.end..range.end).map(|k| char_at + k).collect(),
                rtl,
            });
        }
        if rows.len() > wrap.max_rows {
            break;
        }
        char_at += chars.len() + usize::from(!last_para);
    }
    elide(job, &mut rows, measure);
    rows
}

/// Rows of a paragraph (ranges of `chars`): a row breaks after the spaces
/// that end a word, at `wrap.max_width`; a word too long for a row overruns
/// it, unless `wrap.break_anywhere`.
fn break_rows(chars: &[(usize, char)], wrap: &TextWrapping, width: &mut dyn FnMut(Range<usize>) -> f32) -> Vec<Range<usize>> {
    if !wrap.max_width.is_finite() || chars.is_empty() {
        return std::iter::once(0..chars.len()).collect();
    }
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0.0;
    let mut k = 0;
    while k < chars.len() {
        // a word and the spaces after it
        let mut end = k;
        while end < chars.len() && !chars[end].1.is_whitespace() {
            end += 1;
        }
        let word_end = end;
        while end < chars.len() && chars[end].1.is_whitespace() {
            end += 1;
        }
        let w = width(k..word_end);
        if k > start && used + w > wrap.max_width {
            rows.push(start..k);
            start = k;
            used = 0.0;
        }
        if wrap.break_anywhere && used + w > wrap.max_width {
            // the word alone does not fit: cut it where it overflows
            let mut cut = k + 1;
            while cut < word_end && used + width(k..cut + 1) <= wrap.max_width {
                cut += 1;
            }
            rows.push(start..cut);
            start = cut;
            used = 0.0;
            k = cut;
            continue;
        }
        used += w + if end > word_end { width(word_end..end) } else { 0.0 };
        k = end;
    }
    rows.push(start..chars.len());
    rows
}

/// Rule of `wrap.max_rows`: the rows beyond it go, and the last one kept
/// ends with the overflow character, cut until it fits. As egui does, the
/// end of the text in reading order is what is cut: the left end of a
/// right-to-left row.
fn elide(job: &LayoutJob, rows: &mut Vec<VisualRow>, measure: &mut dyn FnMut(&str, &LayoutSection) -> f32) {
    let wrap = &job.wrap;
    let too_wide = |rows: &[VisualRow], measure: &mut dyn FnMut(&str, &LayoutSection) -> f32| {
        wrap.max_width.is_finite() && wrap.break_anywhere && rows.last().is_some_and(|r| row_width(job, r, measure) > wrap.max_width)
    };
    let cut_rows = rows.len() > wrap.max_rows;
    if !cut_rows && (wrap.max_rows == usize::MAX || !too_wide(rows, measure)) {
        return;
    }
    let mut dropped: Vec<usize> = Vec::new();
    // too wide but not too many rows: none goes, the last one is cut
    let keep = wrap.max_rows.max(1).min(rows.len());
    for row in rows.drain(keep..) {
        dropped.extend(row.segments.iter().flat_map(|s| s.at.iter().copied()));
        dropped.extend(row.hidden);
    }
    let Some(last) = rows.last_mut() else {
        return;
    };
    last.hidden.extend(dropped);
    let Some(overflow) = wrap.overflow_character else {
        return;
    };
    let section = last.segments.first().map_or(0, |s| s.section);
    let fits = |row: &VisualRow, measure: &mut dyn FnMut(&str, &LayoutSection) -> f32| {
        !wrap.max_width.is_finite() || row_width(job, row, measure) <= wrap.max_width
    };
    let mut row = with_overflow(last, overflow, section);
    // the character read last goes first
    while !fits(&row, measure) {
        let Some(&at) = last.segments.iter().flat_map(|s| s.at.iter()).max() else {
            break;
        };
        for s in last.segments.iter_mut() {
            if let Some(i) = s.at.iter().position(|&a| a == at) {
                s.at.remove(i);
                s.rtl.remove(i);
                let mut text: Vec<char> = s.text.chars().collect();
                text.remove(i);
                s.text = text.into_iter().collect();
            }
        }
        last.segments.retain(|s| !s.at.is_empty());
        last.hidden.push(at);
        row = with_overflow(last, overflow, section);
    }
    *last = row;
}

/// `row` with the overflow character at the end of its reading order.
fn with_overflow(row: &VisualRow, overflow: char, section: usize) -> VisualRow {
    let mut out = row.clone();
    let seg = Segment {
        text: overflow.to_string(),
        at: vec![usize::MAX],
        rtl: vec![row.rtl],
        section,
        word: false,
        alone: true,
    };
    if row.rtl {
        out.segments.insert(0, seg);
    } else {
        out.segments.push(seg);
    }
    out
}

fn row_width(job: &LayoutJob, row: &VisualRow, measure: &mut dyn FnMut(&str, &LayoutSection) -> f32) -> f32 {
    row.segments.iter().map(|s| measure(&s.text, &job.sections[s.section])).sum()
}

/// The job egui lays out: the rows one after the other, each segment a
/// section, a new line between the rows (no wrapping left to do). Gives
/// the byte range of each segment, row by row.
fn visual_job(job: &LayoutJob, rows: &[VisualRow]) -> (LayoutJob, Vec<Vec<Range<usize>>>) {
    let mut out = LayoutJob {
        text: String::new(),
        sections: Vec::new(),
        wrap: TextWrapping::no_max_width(),
        first_row_min_height: job.first_row_min_height,
        break_on_newline: true,
        halign: job.halign,
        justify: false,
        round_output_to_gui: job.round_output_to_gui,
        // a space at the right end of a reordered row is often the one read
        // first (before a word, after a link): it keeps its room when the
        // row is aligned to the right (a wrapped row already lost the
        // spaces it ends on)
        keep_trailing_whitespace: true,
    };
    let first_char = |s: &LayoutSection| job.text[s.byte_range.start.0..].chars().next().map(|_| s.byte_range.start.0);
    // the lead of the text (a label's first row indentation, the room for a
    // picture) stays at the left of the first row, whatever is drawn there
    let indent = job
        .sections
        .first()
        .filter(|s| s.byte_range.start.0 == 0)
        .map_or(0.0, |s| s.leading_space);
    let char_starts: Vec<usize> = job.text.char_indices().map(|(b, _)| b).collect();
    let mut pieces = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let mut ranges = Vec::with_capacity(row.segments.len());
        // another section's lead (the room for a picture before a name) goes
        // on the row of its first character, at the left of the section's
        // leftmost segment there: in display order a right-to-left word's
        // first character is on its right
        let starts_here = |section: usize| {
            let first = first_char(&job.sections[section]);
            row.segments
                .iter()
                .filter(|s| s.section == section)
                .any(|s| s.at.iter().any(|&at| char_starts.get(at).copied() == first))
        };
        for (i, seg) in row.segments.iter().enumerate() {
            let source = &job.sections[seg.section];
            let leftmost = row.segments.iter().position(|s| s.section == seg.section) == Some(i);
            let own = if seg.section != 0 && leftmost && starts_here(seg.section) {
                source.leading_space
            } else {
                0.0
            };
            let lead = own + if (r, i) == (0, 0) { indent } else { 0.0 };
            let start = out.text.len();
            out.text.push_str(&seg.text);
            out.sections.push(LayoutSection {
                leading_space: lead,
                byte_range: ByteIndex(start)..ByteIndex(out.text.len()),
                format: source.format.clone(),
            });
            ranges.push(start..out.text.len());
        }
        if r + 1 < rows.len() {
            // the new line belongs to the row's last section (an empty row
            // takes the format of the first section of the text)
            let start = out.text.len();
            out.text.push('\n');
            match out.sections.last_mut() {
                Some(last) if !row.segments.is_empty() => last.byte_range.end = ByteIndex(out.text.len()),
                _ => out.sections.push(LayoutSection {
                    leading_space: 0.0,
                    byte_range: ByteIndex(start)..ByteIndex(out.text.len()),
                    format: job.sections.first().map(|s| s.format.clone()).unwrap_or_default(),
                }),
            }
        }
        pieces.push(ranges);
    }
    (out, pieces)
}

/// A text laid out alone in a section's format.
struct Piece {
    width: f32,
    /// The left edge of each character, when egui gave one glyph per
    /// character.
    xs: Option<Vec<f32>>,
}

fn piece(fonts: &mut FontsView<'_>, text: &str, section: &LayoutSection) -> Piece {
    let job = LayoutJob::single_section(text.to_owned(), section.format.clone());
    let galley = fonts.layout_job(job);
    let glyphs: Vec<f32> = galley.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.pos.x)).collect();
    Piece {
        width: galley.size().x,
        xs: (glyphs.len() == text.chars().count()).then_some(glyphs),
    }
}

/// The box of each character of the text in the laid-out galley.
fn boxes(fonts: &mut FontsView<'_>, job: &LayoutJob, rows: &[VisualRow], pieces: &[Vec<Range<usize>>], galley: &Galley) -> Vec<CharBox> {
    let len = job.text.chars().count();
    let mut boxes = vec![CharBox::NONE; len];
    for (r, row) in rows.iter().enumerate() {
        let Some(placed) = galley.rows.get(r) else {
            continue;
        };
        let rect = placed.rect();
        let (top, bottom) = (rect.min.y, rect.max.y);
        let mut x = placed.pos.x;
        for (seg, range) in row.segments.iter().zip(&pieces[r]) {
            let section = &galley.job.sections.iter().find(|s| s.byte_range.start.0 == range.start);
            let lead = section.map_or(0.0, |s| s.leading_space);
            let source = &job.sections[seg.section];
            let p = piece(fonts, &seg.text, source);
            x += lead;
            let n = seg.at.len();
            for (k, &at) in seg.at.iter().enumerate() {
                if at >= len {
                    continue;
                }
                // a word is drawn reversed: its first character is on the
                // right; its letters share the width
                let (min, max) = match (&p.xs, seg.word) {
                    (Some(xs), false) => (xs[k], xs.get(k + 1).copied().unwrap_or(p.width)),
                    (_, true) => {
                        let step = p.width / n as f32;
                        (p.width - step * (k + 1) as f32, p.width - step * k as f32)
                    }
                    (None, false) => {
                        let step = p.width / n as f32;
                        (step * k as f32, step * (k + 1) as f32)
                    }
                };
                boxes[at] = CharBox {
                    rect: Rect::from_min_max(Pos2::new(x + min, top), Pos2::new(x + max, bottom)),
                    rtl: seg.rtl.get(k).copied().unwrap_or(row.rtl),
                };
            }
            x += p.width;
        }
        // what the row holds without drawing it sits at its end
        let end = if row.rtl { placed.pos.x } else { x };
        for &at in &row.hidden {
            if let Some(b) = boxes.get_mut(at) {
                *b = CharBox {
                    rect: Rect::from_min_max(Pos2::new(end, top), Pos2::new(end, bottom)),
                    rtl: row.rtl,
                };
            }
        }
    }
    boxes
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::text::TextFormat;

    fn job(text: &str) -> LayoutJob {
        LayoutJob::single_section(text.to_owned(), TextFormat::default())
    }

    /// Each character one unit wide.
    fn units(text: &str, _: &LayoutSection) -> f32 {
        text.chars().count() as f32
    }

    /// A context with its fonts loaded by a first pass, whose font atlas
    /// upload is dropped on purpose (epaint asserts that a `TexturesDelta`
    /// is never dropped unapplied).
    fn fonts_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    /// The rows as egui is given them: each segment between brackets, a
    /// word marked with `*`.
    fn shown(job: &LayoutJob, base: Base) -> Vec<String> {
        visual_rows(job, base, &mut units)
            .iter()
            .map(|r| {
                r.segments
                    .iter()
                    .map(|s| format!("[{}{}]", if s.word { "*" } else { "" }, s.text))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn plain_text_is_left_alone() {
        assert!(!has_rtl("Hello, world! Ça va ? 123"));
        assert!(has_rtl("مرحبا"));
        assert!(has_rtl("שלום"));
        assert!(has_rtl("١٢٣"), "Arabic-Indic digits are reversed by the shaper");
    }

    #[test]
    fn the_words_of_an_arabic_sentence_go_from_right_to_left() {
        // each word stays in reading order: the shaper reverses and joins it
        assert_eq!(shown(&job("مرحبا بالعالم"), Base::Auto), ["[*بالعالم][ ][*مرحبا]"]);
    }

    #[test]
    fn numbers_and_latin_words_keep_their_order_in_arabic() {
        assert_eq!(shown(&job("عدد 123 مرحبا"), Base::Auto), ["[*مرحبا][ 123 ][*عدد]"]);
        assert_eq!(shown(&job("انا Aurora Viewer"), Base::Auto), ["[Aurora Viewer ][*انا]"]);
        // Arabic-Indic digits: one section each, so that none is reversed
        assert_eq!(shown(&job("١٢٣"), Base::Auto), ["[١][٢][٣]"]);
    }

    #[test]
    fn an_english_sentence_keeps_its_direction() {
        assert_eq!(shown(&job("Hi مرحبا بالعالم!"), Base::Auto), ["[Hi ][*بالعالم][ ][*مرحبا][!]"]);
        // a sender's name first: left to right even when the body is Arabic
        assert_eq!(shown(&job("مرحبا!"), Base::Sender(0..5)), ["[*مرحبا][!]"]);
        assert_eq!(shown(&job("مرحبا!"), Base::Auto), ["[!][*مرحبا]"]);
    }

    #[test]
    fn brackets_are_mirrored_in_right_to_left_text() {
        assert_eq!(shown(&job("(مرحبا)"), Base::Auto), ["[(][*مرحبا][)]"]);
    }

    #[test]
    fn a_word_does_not_cross_sections() {
        let mut j = LayoutJob::default();
        j.append("مر", 0.0, TextFormat::default());
        j.append(
            "حبا",
            0.0,
            TextFormat {
                italics: true,
                ..Default::default()
            },
        );
        assert_eq!(shown(&j, Base::Auto), ["[*حبا][*مر]"]);
    }

    #[test]
    fn rows_break_before_reordering() {
        let mut j = job("واحد اثنان ثلاثة");
        j.wrap = TextWrapping::wrap_at_width(11.0);
        // the first words read go on the first row, from the right
        assert_eq!(shown(&j, Base::Auto), ["[*اثنان][ ][*واحد]", "[*ثلاثة]"]);
        let rows = visual_rows(&j, Base::Auto, &mut units);
        // the space the first row ends on is held, not drawn
        assert_eq!(rows[0].hidden, vec![10]);
    }

    #[test]
    fn paragraphs_keep_their_own_direction() {
        assert_eq!(shown(&job("Hello\nمرحبا بك"), Base::Auto), ["[Hello]", "[*بك][ ][*مرحبا]"]);
    }

    #[test]
    fn a_truncated_right_to_left_row_ends_on_its_left() {
        let mut j = job("واحد اثنان ثلاثة");
        j.wrap = TextWrapping::truncate_at_width(8.0);
        let rows = shown(&j, Base::Auto);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with("[…]"), "{rows:?}");
        let width: f32 = visual_rows(&j, Base::Auto, &mut units)[0]
            .segments
            .iter()
            .map(|s| s.text.chars().count() as f32)
            .sum();
        assert!(width <= 8.0, "{rows:?}");
    }

    #[test]
    fn every_character_is_placed_once() {
        for text in ["مرحبا بالعالم", "Hi (مرحبا) 123 ١٢٣!", "a\n\nب ج", "שלום עולם"] {
            let mut j = job(text);
            j.wrap = TextWrapping::wrap_at_width(6.0);
            let mut seen: Vec<usize> = visual_rows(&j, Base::Auto, &mut units)
                .iter()
                .flat_map(|r| r.segments.iter().flat_map(|s| s.at.clone()).chain(r.hidden.clone()))
                .collect();
            seen.sort_unstable();
            let newlines = text.matches('\n').count();
            assert_eq!(seen.len() + newlines, text.chars().count(), "{text:?}");
            seen.dedup();
            assert_eq!(seen.len() + newlines, text.chars().count(), "{text:?}");
        }
    }

    #[test]
    fn a_row_too_wide_with_fewer_rows_than_allowed_is_cut_without_panic() {
        // one row, fewer than max_rows, too wide with the spaces it ends on
        // (the last row keeps them): cut, not drained past its end
        let mut j = job("ا    ");
        j.wrap = TextWrapping {
            max_width: 2.0,
            max_rows: 5,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let rows = visual_rows(&j, Base::Auto, &mut units);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(row_width(&j, &rows[0], &mut units) <= 2.0, "{rows:?}");
    }

    #[test]
    fn latin_letters_in_a_right_to_left_row_are_read_from_the_left() {
        let rows = visual_rows(&job("שלום Aurora"), Base::Auto, &mut units);
        let latin = rows[0]
            .segments
            .iter()
            .find(|s| s.text.starts_with("Aurora"))
            .expect("the Latin run");
        // the space after it is read from the right (level 1), the letters
        // from the left (level 2)
        assert_eq!(latin.rtl[..6], [false; 6]);
        assert!(rows[0].segments.iter().filter(|s| s.word).all(|s| s.rtl.iter().all(|&r| r)));
    }

    /// A chat line: "x ", then a two-word right-to-left name with the room
    /// for a picture before it, then its body.
    fn named_line() -> (LayoutJob, Range<usize>) {
        let mut j = LayoutJob::default();
        j.append("x ", 0.0, TextFormat::default());
        j.append(
            "ذئب بنفسجي",
            10.0,
            TextFormat {
                italics: true,
                ..Default::default()
            },
        );
        j.append(": hi", 0.0, TextFormat::default());
        (j, 2..12)
    }

    #[test]
    fn an_isolated_sender_stays_first_before_an_arabic_message() {
        let text = "ذئب بنفسجي: مرحبا بكم";
        // without isolation (no name) the name, the colon and the message
        // are one right-to-left run: the name goes to the right end
        assert_eq!(shown(&job(text), Base::Sender(0..0)), ["[*بكم][ ][*مرحبا][ :][*بنفسجي][ ][*ذئب]"]);
        // isolated, the name comes first, in its own reading order
        assert_eq!(shown(&job(text), Base::Sender(0..10)), ["[*بنفسجي][ ][*ذئب][: ][*بكم][ ][*مرحبا]"]);
        // a Latin name and its Arabic message
        assert_eq!(shown(&job("Bob: مرحبا"), Base::Sender(0..3)), ["[Bob: ][*مرحبا]"]);
    }

    #[test]
    fn the_room_before_a_right_to_left_name_is_at_its_left() {
        let (j, name) = named_line();
        let rows = visual_rows(&j, Base::Sender(name), &mut units);
        let (out, _) = visual_job(&j, &rows);
        let leads: Vec<(String, f32)> = out
            .sections
            .iter()
            .map(|s| (out.text[s.byte_range.start.0..s.byte_range.end.0].to_owned(), s.leading_space))
            .collect();
        // the name's second word is drawn first: the room goes before it,
        // not between the two words
        assert_eq!(leads[1], ("بنفسجي".to_owned(), 10.0), "{leads:?}");
        assert!(leads.iter().enumerate().all(|(i, l)| i == 1 || l.1 == 0.0), "{leads:?}");
    }

    #[test]
    fn a_picture_goes_at_the_left_of_a_right_to_left_name() {
        let ctx = fonts_ctx();
        let (j, name) = named_line();
        let visual = ctx.fonts_mut(|f| layout(f, j, Base::Sender(name.clone())));
        let left = visual.left_edge(name.clone()).min.x;
        let map = visual.map.as_ref().expect("reordered");
        // the name starts after "x " and the room left for the picture
        let after_x = map.boxes[1].rect.max.x;
        assert!((left - after_x - 10.0).abs() < 0.5, "{left} after {after_x}");
        assert!(map.boxes[name].iter().all(|b| b.rect.min.x >= left - 0.01));
        // its first character, read first, is on its right
        assert!(visual.cursor_rect(2).min.x > left + 1.0);
    }

    #[test]
    fn the_cursor_before_a_latin_word_in_a_hebrew_line_is_on_its_left() {
        let ctx = fonts_ctx();
        let visual = ctx.fonts_mut(|f| layout(f, job("שלום Aurora"), Base::Auto));
        let map = visual.map.as_ref().expect("reordered");
        // "A" (5) is read from the left: the cursor before it is on its left
        // edge, the one after "a" (11, the end) on the right edge of "a"
        assert_eq!(visual.cursor_rect(5).min.x, map.boxes[5].rect.min.x);
        assert_eq!(visual.cursor_rect(10).min.x, map.boxes[10].rect.min.x);
        // "ש" (0) is read from the right
        assert_eq!(visual.cursor_rect(0).min.x, map.boxes[0].rect.max.x);
        // a click on the left half of "A" puts the cursor before it
        let a = map.boxes[5].rect;
        assert_eq!(visual.cursor_at(Vec2::new(a.min.x + 0.2, a.center().y)), 5);
    }

    #[test]
    fn selected_right_to_left_text_takes_the_selected_color() {
        let ctx = fonts_ctx();
        let visual = ctx.fonts_mut(|f| layout(f, job("שלום Aurora"), Base::Auto));
        let mut galley = visual.galley.clone();
        let red = egui::Color32::RED;
        visual.tint(&mut galley, 5..11, red);
        let colored = |g: &Galley| {
            g.rows
                .iter()
                .flat_map(|r| r.row.visuals.mesh.vertices.iter())
                .filter(|v| v.color == red)
                .count()
        };
        assert_eq!(colored(&visual.galley), 0, "the shared galley is left alone");
        // the six letters of "Aurora", four vertices each, nothing more
        assert_eq!(colored(&galley), 6 * 4);
    }

    #[test]
    fn a_paint_only_galley_keeps_no_boxes() {
        let ctx = fonts_ctx();
        let painted = ctx.fonts_mut(|f| lay_out(f, job("مرحبا بك"), Base::Auto, false));
        assert!(painted.map.is_none());
        let full = ctx.fonts_mut(|f| layout(f, job("مرحبا بك"), Base::Auto));
        assert_eq!(painted.galley.size(), full.galley.size());
    }
}
