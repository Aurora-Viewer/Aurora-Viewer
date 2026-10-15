//! The lines of a conversation, laid out only where they are in sight.
//!
//! A conversation holds up to a few thousand lines and its window shows
//! twenty: laying all of them out every frame (wrapping, emoji, links) cost
//! in proportion to the whole conversation. Here every shown line is a row
//! with a height, the scroll area gets their sum, and only the rows that
//! cross the viewport are drawn.
//!
//! - A row's height is the one measured the last time it was laid out at
//!   this width; until then it is an estimate. Rows not yet measured are
//!   measured without being painted, a few per frame, the ones nearest the
//!   viewport first, so that scrolling meets measured rows.
//! - When a row above the viewport changes height, what is in sight must
//!   not move: the frame draws with the old scroll origin and the offset is
//!   corrected for the next one (`show` returns that correction).
//! - Rows are numbered by `LineClock`, so heights survive lines added at
//!   either end and lines dropped by the bound.

use crate::world::ChatLine;
use crate::world::chat_log::format::Slt;
use crate::world::transcripts::LineClock;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Space between two rows (the item spacing of the conversation).
const GAP: f32 = 2.0;
/// Time spent per frame measuring rows out of sight.
const MEASURE_BUDGET: Duration = Duration::from_micros(300);

/// What precedes a line's text: the day when it changes, the sender's
/// header when the sender or the moment does. `tail`: the last line of its
/// sender's group, which the bubbles end with the picture and the time.
/// `first`: the first row of the list, against its top edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    pub marker: Option<(i64, i64, i64)>,
    pub header: bool,
    pub tail: bool,
    pub first: bool,
}

/// Mark the last line of each group: the one before a new header, a day
/// marker, a system line, or the end.
fn mark_tails(looks: &mut [Look], system: impl Fn(usize) -> bool) {
    for i in 0..looks.len() {
        looks[i].tail = match looks.get(i + 1) {
            None => true,
            Some(next) => next.header || next.marker.is_some() || system(i + 1),
        };
    }
}

/// One shown line.
#[derive(Debug, Clone, Copy)]
pub struct Row {
    /// Index of its line in the conversation.
    pub line: usize,
    pub look: Look,
    /// It matches the find bar's text.
    pub hit: bool,
    /// Top, from the top of the content.
    y: f32,
    h: f32,
    measured: bool,
}

/// What the rows are built from.
pub struct Source<'a> {
    pub lines: &'a VecDeque<ChatLine>,
    pub clock: LineClock,
    /// Only the lines matching this are shown (lowercase; empty = all).
    pub filter: &'a str,
    /// The find bar's text (lowercase; empty = none).
    pub find: &'a str,
}

#[derive(Default)]
pub struct View {
    token: u64,
    revision: u64,
    len: usize,
    width: f32,
    filter: String,
    find: String,
    today: (i64, i64, i64),
    rows: Vec<Row>,
    /// Measured height and look of the lines, by number, for this width
    /// and these texts.
    heights: HashMap<u64, (f32, Look)>,
    /// Heights of another width, used as estimates.
    estimates: HashMap<u64, f32>,
    total: f32,
    unmeasured: usize,
    /// Rows matching the find text, as indices of `rows`.
    hits: Vec<usize>,
    /// Row to bring in sight at the next `show`.
    scroll_to: Option<usize>,
}

/// Case-insensitive search of `needle` (already lowercase) in `hay`: the
/// byte ranges of `hay` that match, without overlap.
pub fn find_ci(hay: &str, needle: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut start = 0;
    while let Some(c) = hay[start..].chars().next() {
        match match_at(&hay[start..], needle) {
            Some(len) => {
                out.push(start..start + len);
                start += len;
            }
            None => start += c.len_utf8(),
        }
    }
    out
}

/// Bytes at the start of `hay` whose lowercase is `needle`, if any.
fn match_at(hay: &str, needle: &str) -> Option<usize> {
    let mut want = needle.chars();
    for (i, c) in hay.char_indices() {
        for l in c.to_lowercase() {
            if want.next() != Some(l) {
                return None;
            }
        }
        if want.as_str().is_empty() {
            return Some(i + c.len_utf8());
        }
    }
    None
}

/// A line matches a text when its message or its sender contains it.
pub fn matches(line: &ChatLine, needle: &str) -> bool {
    !find_ci(&line.text, needle).is_empty() || !find_ci(&line.from, needle).is_empty()
}

/// Messages from the same sender within this many seconds share one header.
const GROUP_SECONDS: u64 = 300;

/// Works out the look of each line from the one shown before it.
pub struct Grouping<'a> {
    prev: Option<&'a ChatLine>,
    prev_day: Option<Slt>,
    today: Slt,
}

impl<'a> Grouping<'a> {
    pub fn new(today: Slt) -> Grouping<'a> {
        Grouping {
            prev: None,
            prev_day: None,
            today,
        }
    }

    pub fn look(&mut self, line: &'a ChatLine) -> Look {
        let mut look = Look::default();
        if line.time != std::time::UNIX_EPOCH {
            let day = Slt::of(line.time);
            if !day.same_day(self.prev_day.as_ref().unwrap_or(&self.today)) {
                look.marker = Some((day.year, day.month, day.day));
                self.prev = None;
            }
            self.prev_day = Some(day);
        }
        if line.kind.is_notice() {
            self.prev = None;
            return look;
        }
        look.header = match self.prev {
            Some(p) => {
                p.source != line.source
                    || (p.kind == crate::world::ChatKind::Own) != (line.kind == crate::world::ChatKind::Own)
                    || p.origin != line.origin
                    || p.from != line.from
                    || line
                        .time
                        .duration_since(p.time)
                        .map(|d| d.as_secs() > GROUP_SECONDS)
                        .unwrap_or(true)
            }
            None => true,
        };
        self.prev = Some(line);
        look
    }
}

/// Height of a row before it is measured.
fn estimate(line: &ChatLine, look: Look, width: f32) -> f32 {
    let per_row = (width / 7.0).max(8.0);
    let rows = line
        .text
        .split('\n')
        .map(|l| (l.chars().count() as f32 / per_row).ceil().max(1.0))
        .sum::<f32>();
    rows * 18.0 + if look.header { 36.0 } else { 0.0 } + if look.marker.is_some() { 22.0 } else { 0.0 } + GAP
}

impl View {
    /// Rows matching the find text (indices for `row` and `scroll_to`).
    pub fn hits(&self) -> &[usize] {
        &self.hits
    }

    pub fn row(&self, i: usize) -> Option<&Row> {
        self.rows.get(i)
    }

    /// Bring a row in sight at the next `show`.
    pub fn scroll_to(&mut self, row: usize) {
        self.scroll_to = Some(row);
    }

    /// Bring the rows up to date with the conversation. True when the set
    /// of rows matching the find text may have changed.
    pub fn sync(&mut self, s: &Source, width: f32) -> bool {
        let today = Slt::of(std::time::SystemTime::now());
        let today_key = (today.year, today.month, today.day);
        // heights hold for a width and for the texts that change the looks
        // (the filter) or split the labels (the find highlight)
        let layout_changed = self.token != s.clock.token || self.width != width || self.filter != s.filter || self.find != s.find;
        if !layout_changed && self.revision == s.clock.revision && self.len == s.lines.len() && self.today == today_key {
            return false;
        }
        if layout_changed {
            let same_lines = self.token == s.clock.token;
            let old = std::mem::take(&mut self.heights);
            self.estimates = if same_lines {
                old.into_iter().map(|(k, (h, _))| (k, h)).collect()
            } else {
                HashMap::new()
            };
        }
        self.token = s.clock.token;
        self.revision = s.clock.revision;
        self.len = s.lines.len();
        self.width = width;
        s.filter.clone_into(&mut self.filter);
        s.find.clone_into(&mut self.find);
        self.today = today_key;

        let first = s.clock.first;
        self.rows.clear();
        self.hits.clear();
        self.unmeasured = 0;
        let mut grouping = Grouping::new(today);
        let shown: Vec<usize> = (0..s.lines.len())
            .filter(|&i| s.filter.is_empty() || matches(&s.lines[i], s.filter))
            .collect();
        let mut looks: Vec<Look> = shown.iter().map(|&i| grouping.look(&s.lines[i])).collect();
        mark_tails(&mut looks, |k| s.lines[shown[k]].kind.is_notice());
        if let Some(look) = looks.first_mut() {
            look.first = true;
        }
        let mut y = 0.0;
        for (&i, look) in shown.iter().zip(looks) {
            let line = &s.lines[i];
            let seq = first + i as u64;
            let (h, measured) = match self.heights.get(&seq) {
                Some(&(h, l)) if l == look => (h, true),
                _ => (
                    self.estimates.get(&seq).copied().unwrap_or_else(|| estimate(line, look, width)),
                    false,
                ),
            };
            self.unmeasured += usize::from(!measured);
            let hit = !s.find.is_empty() && matches(line, s.find);
            if hit {
                self.hits.push(self.rows.len());
            }
            self.rows.push(Row {
                line: i,
                look,
                hit,
                y,
                h,
                measured,
            });
            y += h;
        }
        self.total = y;
        // forget the lines the bound dropped
        if self.heights.len() > self.rows.len() * 2 + 64 {
            self.heights.retain(|&seq, _| seq >= first);
        }
        if self.estimates.len() > self.rows.len() * 2 + 64 {
            self.estimates.retain(|&seq, _| seq >= first);
        }
        true
    }

    fn seq(&self, first: u64, row: usize) -> u64 {
        first + self.rows[row].line as u64
    }

    /// Record the measured height of a row; returns the change.
    fn set_height(&mut self, first: u64, row: usize, h: f32) -> f32 {
        let seq = self.seq(first, row);
        let r = &mut self.rows[row];
        let delta = h - r.h;
        if !r.measured {
            r.measured = true;
            self.unmeasured -= 1;
        }
        r.h = h;
        self.heights.insert(seq, (h, r.look));
        delta
    }

    fn reflow(&mut self, from: usize) {
        let mut y = match from.checked_sub(1).and_then(|i| self.rows.get(i)) {
            Some(r) => r.y + r.h,
            None => 0.0,
        };
        for r in &mut self.rows[from..] {
            r.y = y;
            y += r.h;
        }
        self.total = y;
    }

    /// Lay out a row without painting it, when its height is not known.
    /// Returns how much its height changed.
    fn measure(&mut self, ui: &mut egui::Ui, first: u64, i: usize, draw: &mut dyn FnMut(&mut egui::Ui, &Row, bool)) -> f32 {
        let row = self.rows[i];
        if row.measured {
            return 0.0;
        }
        // anywhere: nothing is painted
        let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(self.width, row.h));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("conv_measure", first + row.line as u64))
                .max_rect(rect)
                .invisible(),
        );
        draw(&mut child, &row, false);
        let h = child.min_rect().height() + GAP;
        self.set_height(first, i, h)
    }

    /// Lay out the rows in sight in a scroll area's content `ui`, whose
    /// visible part is `viewport` (content coordinates). `draw` draws one
    /// row in the Ui it is given (`painted` false: measuring only).
    ///
    /// Returns how much the scroll offset must grow so that the next frame
    /// shows the same rows at the same place (0 at the end of the
    /// conversation, which the scroll area follows by itself), and the
    /// response of the content under the rows: it takes the clicks and the
    /// drags (a text selection, `select.rs`), which then never move the
    /// window.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        viewport: egui::Rect,
        first: u64,
        draw: &mut dyn FnMut(&mut egui::Ui, &Row, bool),
    ) -> (f32, egui::Response) {
        let width = self.width;
        // at the end of the conversation the scroll area follows its growth
        // by itself (stick to bottom): no correction to add then
        let at_end = viewport.bottom() >= self.total - 1.0;
        let target = self.scroll_to.take().filter(|&t| t < self.rows.len());

        // ---- heights that must be exact before anything is placed: the
        // rows in sight (laid out this frame anyway) and, around a row to
        // bring in sight, a viewport's height on each side. Then the others,
        // outward from the viewport, within the frame's budget.
        let mut shift_above = 0.0;
        if self.unmeasured > 0 {
            let started = Instant::now();
            let top = self.rows.partition_point(|r| r.y + r.h <= viewport.top());
            let bottom = self.rows.partition_point(|r| r.y < viewport.bottom());
            let mut dirty_from = usize::MAX;
            let mut note = |i: usize, delta: f32| {
                if delta != 0.0 {
                    dirty_from = dirty_from.min(i);
                    if i < top {
                        shift_above += delta;
                    }
                }
            };
            let around = match target {
                Some(t) => {
                    let y = self.rows[t].y;
                    let reach = viewport.height();
                    self.rows.partition_point(|r| r.y < y - reach)..self.rows.partition_point(|r| r.y <= y + reach)
                }
                None => 0..0,
            };
            for i in (top..bottom).chain(around) {
                let delta = self.measure(ui, first, i, draw);
                note(i, delta);
            }
            let (mut up, mut down) = (top, bottom);
            while self.unmeasured > 0 && (up > 0 || down < self.rows.len()) {
                let mut measured = false;
                if up > 0 {
                    up -= 1;
                    measured |= !self.rows[up].measured;
                    let delta = self.measure(ui, first, up, draw);
                    note(up, delta);
                }
                if down < self.rows.len() {
                    measured |= !self.rows[down].measured;
                    let delta = self.measure(ui, first, down, draw);
                    note(down, delta);
                    down += 1;
                }
                if measured && started.elapsed() > MEASURE_BUDGET {
                    break;
                }
            }
            if dirty_from != usize::MAX {
                self.reflow(dirty_from);
            }
            if self.unmeasured > 0 {
                ui.ctx().request_repaint();
            }
        }

        // ---- the content takes its whole height; rows are placed in it.
        // What changed above the viewport is not applied to this frame's
        // positions: the scroll offset follows at the next one.
        let (rect, content) = ui.allocate_exact_size(egui::vec2(width, self.total), egui::Sense::click_and_drag());
        let base = rect.top() - shift_above;
        let (view_top, view_bottom) = (viewport.top() + shift_above, viewport.bottom() + shift_above);
        if let Some(r) = target.and_then(|t| self.rows.get(t)) {
            // at once: an animated scroll would run over the corrections of
            // the offset made while rows are measured
            let to = egui::Rect::from_min_size(egui::pos2(rect.left(), base + r.y), egui::vec2(width, r.h));
            ui.scroll_to_rect_animation(to, Some(egui::Align::Center), egui::style::ScrollAnimation::none());
        }
        let mut i = self.rows.partition_point(|r| r.y + r.h <= view_top);
        let mut changed_from = usize::MAX;
        while i < self.rows.len() && self.rows[i].y < view_bottom {
            let row = self.rows[i];
            let row_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), base + row.y), egui::vec2(width, row.h));
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("conv_row", first + row.line as u64))
                    .max_rect(row_rect),
            );
            draw(&mut child, &row, true);
            let h = child.min_rect().height() + GAP;
            if (h - row.h).abs() > 0.25 || !row.measured {
                self.set_height(first, i, h);
                changed_from = changed_from.min(i);
            }
            i += 1;
        }
        if changed_from != usize::MAX {
            // a row in sight changed (a name arrived, an emoji loaded): the
            // rows under it take their place at the next frame
            self.reflow(changed_from);
            ui.ctx().request_repaint();
        }
        (if at_end { 0.0 } else { shift_above }, content)
    }

    /// The lines shown, in order: their index in the conversation.
    pub fn lines(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().map(|r| r.line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{ChatKind, Origin};
    use std::time::UNIX_EPOCH;

    fn line(from: &str, text: &str, secs: u64) -> ChatLine {
        ChatLine {
            time: UNIX_EPOCH + Duration::from_secs(1_800_000_000 + secs),
            from: from.into(),
            text: text.into(),
            kind: ChatKind::Im,
            source: uuid::Uuid::from_u128(from.len() as u128),
            origin: Origin::Live,
        }
    }

    #[test]
    fn case_insensitive_search() {
        assert_eq!(find_ci("Hello hello HELLO", "hello"), [0..5, 6..11, 12..17]);
        assert_eq!(find_ci("Ça va, ÇA VA ?", "ça va"), [0..6, 8..14]);
        assert_eq!(find_ci("aaa", "aa"), vec![0..2]);
        assert!(find_ci("abc", "").is_empty());
        assert!(find_ci("abc", "abcd").is_empty());
        assert_eq!(find_ci("猫と犬", "と"), vec![3..6]);
        // a letter whose lowercase is longer than itself
        assert_eq!(find_ci("İstanbul", "i\u{307}st"), vec![0..4]);
        assert!(find_ci("x", "y").is_empty());
    }

    #[test]
    fn headers_and_day_markers() {
        let day = 86_400;
        let lines = [
            line("Jane", "one", 0),
            line("Jane", "two", 60),
            line("Jane", "much later", 60 + GROUP_SECONDS + 1),
            line("Bob", "hi", 60 + GROUP_SECONDS + 2),
            line("Bob", "next day", 3 * day),
        ];
        let today = Slt::of(lines[4].time);
        let mut g = Grouping::new(today);
        let looks: Vec<Look> = lines.iter().map(|l| g.look(l)).collect();
        // the first line is of another day than today: a marker
        assert!(looks[0].marker.is_some() && looks[0].header);
        assert_eq!(looks[1], Look::default());
        assert!(looks[2].header && looks[2].marker.is_none());
        assert!(looks[3].header);
        // today's lines get a marker when they follow another day's
        assert!(looks[4].marker.is_some() && looks[4].header);
        // a conversation of today only has no marker
        let mut g = Grouping::new(today);
        assert_eq!(g.look(&lines[4]).marker, None);
        // history and new lines of the same sender do not share a header
        let mut old = line("Jane", "old", 0);
        old.origin = Origin::Log;
        let new = line("Jane", "new", 10);
        let mut g = Grouping::new(Slt::of(old.time));
        assert!(g.look(&old).header);
        assert!(g.look(&new).header);
    }

    #[test]
    fn a_group_ends_before_a_header_a_day_or_a_system_line() {
        let day = 86_400;
        let mut sys = line("", "notice", 70);
        sys.kind = ChatKind::System;
        let lines = [
            line("Jane", "one", 0),
            line("Jane", "two", 60),
            sys,
            line("Jane", "three", 80),
            line("Bob", "hi", 90),
            line("Bob", "next day", day),
        ];
        let mut g = Grouping::new(Slt::of(lines[5].time));
        let mut looks: Vec<Look> = lines.iter().map(|l| g.look(l)).collect();
        mark_tails(&mut looks, |k| lines[k].kind == ChatKind::System);
        let tails: Vec<bool> = looks.iter().map(|l| l.tail).collect();
        assert_eq!(tails, [false, true, true, true, true, true]);
        // after the system line the same sender starts a new group
        assert!(looks[3].header);
    }

    #[test]
    fn rows_follow_the_conversation_and_keep_their_heights() {
        let mut lines: VecDeque<ChatLine> = (0..100).map(|i| line("Jane", &format!("line {i}"), i)).collect();
        let mut clock = LineClock::default();
        let mut view = View::default();
        let sync = |view: &mut View, lines: &VecDeque<ChatLine>, clock: LineClock, filter: &str, find: &str| {
            view.sync(
                &Source {
                    lines,
                    clock,
                    filter,
                    find,
                },
                300.0,
            )
        };
        assert!(sync(&mut view, &lines, clock, "", ""));
        assert_eq!(view.rows.len(), 100);
        assert_eq!(view.unmeasured, 100);
        // nothing changed: nothing is rebuilt
        assert!(!sync(&mut view, &lines, clock, "", ""));
        // measure them all
        for i in 0..100 {
            view.set_height(clock.first, i, 20.0);
        }
        view.reflow(0);
        assert_eq!(view.unmeasured, 0);
        assert_eq!(view.total, 2000.0);
        // a line arrives, one is dropped by the bound: the others keep
        // their measured heights
        lines.push_back(line("Jane", "new", 200));
        clock.pushed_back();
        lines.pop_front();
        clock.popped_front();
        assert!(sync(&mut view, &lines, clock, "", ""));
        assert_eq!(view.rows.len(), 100);
        // the new first line gets a marker and a header it did not have, the
        // line before the new one is no longer the end of its group
        assert_eq!(view.unmeasured, 3);
        assert!(view.rows[1..98].iter().all(|r| r.measured && r.h == 20.0));
        // history put before the first line
        for i in 0..3 {
            let mut l = line("Jane", &format!("old {i}"), 0);
            l.origin = Origin::Log;
            lines.push_front(l);
        }
        clock.pushed_front(3);
        assert!(sync(&mut view, &lines, clock, "", ""));
        assert_eq!(view.rows.len(), 103);
        // (the last two still wait for their first measure)
        assert!(view.rows[4..101].iter().all(|r| r.measured));
        // the find text marks its rows, the filter keeps only its lines
        assert!(sync(&mut view, &lines, clock, "", "line 4"));
        assert_eq!(view.hits().len(), 11);
        assert!(view.hits().iter().all(|&i| view.rows[i].hit));
        assert!(sync(&mut view, &lines, clock, "old", ""));
        assert_eq!(view.rows.len(), 3);
        assert_eq!(view.rows[2].line, 2);
        // another width: every row is to measure again, from estimates
        view.width = 0.0;
        assert!(sync(&mut view, &lines, clock, "", ""));
        assert_eq!(view.unmeasured, 103);
    }
}
