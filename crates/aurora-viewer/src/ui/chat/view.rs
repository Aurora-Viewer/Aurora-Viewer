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
//!   corrected for the next one (`show` returns that correction). At the
//!   end of the conversation the rows stand against the bottom in the same
//!   frame: the scroll area follows the end only a frame later, and a
//!   resize, which wraps every row again, made the messages jump for one
//!   frame at each line a row gained or lost (the human's report of
//!   2026-10-16).
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
/// `first`: the first row of the list, against its top edge. `new`: the
/// first line that was unread when the conversation was opened ("New").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    pub marker: Option<(i64, i64, i64)>,
    pub header: bool,
    pub tail: bool,
    pub first: bool,
    pub new: bool,
}

/// How the rows of a conversation are drawn, for their grouping and their
/// heights: the window of tabs (bubbles, Firestorm's lines), or Aurora's
/// workspace with its messages grouped by author (`ui/chat/thread.rs`),
/// roomy or compact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Classic,
    Aurora,
    Compact,
}

impl Layout {
    pub fn workspace(self) -> bool {
        self != Layout::Classic
    }
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
    pub layout: Layout,
    /// The number of the first line unread when the conversation was
    /// opened, which carries the "New" marker.
    pub first_unread: Option<u64>,
}

#[derive(Default)]
pub struct View {
    token: u64,
    revision: u64,
    len: usize,
    /// Next sequence number at the last sync (independent of filtering).
    next: u64,
    width: f32,
    filter: String,
    find: String,
    layout: Layout,
    first_unread: Option<u64>,
    today: (i64, i64, i64),
    rows: Vec<Row>,
    /// Measured height and look of the lines, by number, for this width
    /// and these texts.
    heights: HashMap<u64, (f32, Look)>,
    /// Heights of another width, used as estimates.
    estimates: HashMap<u64, f32>,
    total: f32,
    /// The content's height at the last `show`: the scroll area's, whose
    /// end it sticks to.
    shown_height: f32,
    /// Where the viewport's top stood at the last `show` when the rows were
    /// placed against the end: the scroll area is still there when the
    /// window got shorter (its bottom then short of the end) and nobody
    /// scrolled.
    anchored_top: Option<f32>,
    /// How far the viewport's bottom stood above the end at the last `show`
    /// (0 against the end).
    below: f32,
    /// The number after the last line when the reader left the end: the
    /// lines from it on came in since (`latest.rs` counts them).
    left_end: Option<u64>,
    /// Bottom alignment last used by a short workspace conversation.
    lift: f32,
    /// A menu holds this alignment while new rows continue below it.
    paused: Option<Pause>,
    /// Closing a menu at the live end places rows there in the same frame;
    /// the scroll area receives the finite content height after layout.
    resume_following: bool,
    /// Visible row positions relative to the viewport's top, for holding
    /// the selected message when an older New marker moves away.
    positions: Vec<(u64, f32)>,
    unmeasured: usize,
    /// Rows matching the find text, as indices of `rows`.
    hits: Vec<usize>,
    /// Row to bring in sight at the next `show`.
    scroll_to: Option<usize>,
}

struct Pause {
    lift: f32,
    following: bool,
    anchor: Option<(u64, f32)>,
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
/// The same in Aurora's workspace, where an author's messages group over
/// seven minutes.
const WORKSPACE_GROUP_SECONDS: u64 = 420;

/// Works out the look of each line from the one shown before it.
pub struct Grouping<'a> {
    prev: Option<&'a ChatLine>,
    prev_day: Option<Slt>,
    today: Slt,
    layout: Layout,
}

/// An emote ("/me waves") is said by its sender's name: in Aurora's
/// workspace it stands alone, with its own head.
fn is_emote(line: &ChatLine) -> bool {
    line.text.starts_with("/me ") || line.text.starts_with("/me'")
}

impl<'a> Grouping<'a> {
    #[cfg(test)]
    pub fn new(today: Slt) -> Grouping<'a> {
        Grouping::with(today, Layout::Classic)
    }

    pub fn with(today: Slt, layout: Layout) -> Grouping<'a> {
        Grouping {
            prev: None,
            prev_day: None,
            today,
            layout,
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
        let (gap, emotes_alone) = match self.layout {
            Layout::Classic => (GROUP_SECONDS, false),
            Layout::Aurora | Layout::Compact => (WORKSPACE_GROUP_SECONDS, true),
        };
        let emote = emotes_alone && is_emote(line);
        look.header = emote
            || match self.prev {
                Some(p) => {
                    p.source != line.source
                        || (p.kind == crate::world::ChatKind::Own) != (line.kind == crate::world::ChatKind::Own)
                        // an object's script errors under their own warning
                        // sign, apart from what it says (#1263)
                        || super::object_row::script_error(p) != super::object_row::script_error(line)
                        || p.origin != line.origin
                        || p.from != line.from
                        || line.time.duration_since(p.time).map(|d| d.as_secs() > gap).unwrap_or(true)
                }
                None => true,
            };
        // the line after an emote opens a group of its own too
        self.prev = (!emote).then_some(line);
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

    /// How far the end of the conversation was under the viewport at the
    /// last `show`.
    pub fn below(&self) -> f32 {
        self.below
    }

    pub(super) fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    pub(super) fn next(&self) -> u64 {
        self.next
    }

    /// Hold the viewport, including a short conversation's bottom padding.
    /// Resume following only if the reader was at the end before the menu.
    pub(super) fn pause(&mut self, paused: bool, selected: Option<u64>) -> bool {
        if paused {
            if self.paused.is_none() {
                self.paused = Some(Pause {
                    lift: self.lift,
                    following: self.below <= 1.0,
                    anchor: self
                        .positions
                        .iter()
                        .find(|(seq, _)| Some(*seq) == selected)
                        .or_else(|| self.positions.first())
                        .copied(),
                });
                self.left_end.get_or_insert(self.next);
                self.anchored_top = None;
            }
            false
        } else {
            let resume = self.paused.take().is_some_and(|p| p.following);
            self.resume_following |= resume;
            self.resume_following
        }
    }

    /// The rows that came in since the reader left the end, among those
    /// `keep` takes (by line index).
    pub fn arrived(&self, first: u64, keep: impl Fn(usize) -> bool) -> usize {
        let Some(from) = self.left_end else {
            return 0;
        };
        let start = self.rows.partition_point(|r| first + (r.line as u64) < from);
        self.rows[start..].iter().filter(|r| keep(r.line)).count()
    }

    /// Bring the rows up to date with the conversation. True when the set
    /// of rows matching the find text may have changed.
    pub fn sync(&mut self, s: &Source, width: f32) -> bool {
        let today = Slt::of(std::time::SystemTime::now());
        let today_key = (today.year, today.month, today.day);
        // heights hold for a width and for the texts that change the looks
        // (the filter) or split the labels (the find highlight)
        let layout_changed =
            self.token != s.clock.token || self.width != width || self.filter != s.filter || self.find != s.find || self.layout != s.layout;
        if !layout_changed
            && self.revision == s.clock.revision
            && self.len == s.lines.len()
            && self.today == today_key
            && self.first_unread == s.first_unread
        {
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
        self.next = s.clock.first + self.len as u64;
        self.width = width;
        s.filter.clone_into(&mut self.filter);
        s.find.clone_into(&mut self.find);
        self.layout = s.layout;
        self.first_unread = s.first_unread;
        self.today = today_key;

        let first = s.clock.first;
        self.rows.clear();
        self.hits.clear();
        self.unmeasured = 0;
        let mut grouping = Grouping::with(today, s.layout);
        let shown: Vec<usize> = (0..s.lines.len())
            .filter(|&i| s.filter.is_empty() || matches(&s.lines[i], s.filter))
            .collect();
        let mut looks: Vec<Look> = shown.iter().map(|&i| grouping.look(&s.lines[i])).collect();
        mark_tails(&mut looks, |k| s.lines[shown[k]].kind.is_notice());
        if let Some(look) = looks.first_mut() {
            look.first = true;
        }
        if let Some(seq) = s.first_unread
            && let Some(k) = shown.iter().position(|&i| first + i as u64 == seq)
            && k > 0
        {
            // (above the very first line it would say nothing)
            looks[k].new = true;
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
        // by itself (stick to bottom), a frame late: the rows are placed
        // against the bottom here instead, no correction to add
        let at_end = self.resume_following
            || (self.paused.is_none()
                && (viewport.bottom() >= self.shown_height - 1.0 || self.anchored_top.is_some_and(|t| (viewport.top() - t).abs() < 0.5)));
        let target = self.scroll_to.take().filter(|&t| t < self.rows.len());
        let anchored = at_end && target.is_none();

        // ---- heights that must be exact before anything is placed: the
        // rows in sight (laid out this frame anyway) and, around a row to
        // bring in sight, a viewport's height on each side. Then the others,
        // outward from the viewport, within the frame's budget.
        let mut shift_above = 0.0;
        if anchored && self.unmeasured > 0 {
            // the rows a viewport's height up from the end: exact, as the
            // rows above them no longer place them
            let (mut i, mut up, mut dirty_from) = (self.rows.len(), 0.0, usize::MAX);
            while i > 0 && up < viewport.height() {
                i -= 1;
                if self.measure(ui, first, i, draw) != 0.0 {
                    dirty_from = i;
                }
                up += self.rows[i].h;
            }
            if dirty_from != usize::MAX {
                self.reflow(dirty_from);
            }
        }
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
        // at least the height in sight: a short conversation's empty dark
        // area under its lines selects too, never moves the window
        let height = (self.total + self.paused.as_ref().map_or(0.0, |p| p.lift)).max(viewport.height());
        // Measuring a variable number of rows advances egui's auto ids.
        // The menu and selection must keep the same owner on every frame.
        let (_, rect) = ui.allocate_space(egui::vec2(width, height));
        let content = ui.interact(rect, ui.scope_id().with("conv_content"), egui::Sense::click_and_drag());
        // Aurora's workspace keeps a short conversation against the field,
        // as a chat does: its lines start from the bottom
        let lift = self.paused.as_ref().map_or_else(
            || {
                if self.layout.workspace() {
                    (height - self.total).max(0.0)
                } else {
                    0.0
                }
            },
            |p| p.lift,
        );
        if !ui.is_sizing_pass() {
            self.lift = lift;
        }
        self.shown_height = height;
        // (not a window's sizing pass: it scrolls an area of its own, at its
        // top)
        if !ui.is_sizing_pass() {
            if anchored {
                (self.below, self.left_end) = (0.0, None);
            } else {
                self.below = (height - viewport.bottom()).max(0.0);
                self.left_end.get_or_insert(first + self.len as u64);
            }
        }
        // Moving the New marker or measuring an earlier row must not move
        // the menu's target. Keep a surviving visible row at its old screen
        // position; never retain the transcript itself after eviction.
        let correction = self
            .paused
            .as_ref()
            .and_then(|p| p.anchor)
            .and_then(|(seq, y)| {
                self.rows
                    .iter()
                    .find(|r| first + r.line as u64 == seq)
                    .map(|r| r.y + lift - y - viewport.top())
            })
            .unwrap_or(shift_above);
        let (base, view_top) = if anchored {
            // the viewport's top once at the end, where the scroll area
            // will be at the next frame
            let top = height - viewport.height();
            self.anchored_top = Some(top);
            (rect.top() + lift + viewport.top() - top, self.total - viewport.height())
        } else {
            self.anchored_top = None;
            (rect.top() + lift - correction, viewport.top() + correction)
        };
        let view_bottom = view_top + viewport.height();
        if let Some(r) = target.and_then(|t| self.rows.get(t)) {
            // at once: an animated scroll would run over the corrections of
            // the offset made while rows are measured
            let to = egui::Rect::from_min_size(egui::pos2(rect.left(), base + r.y), egui::vec2(width, r.h));
            ui.scroll_to_rect_animation(to, Some(egui::Align::Center), egui::style::ScrollAnimation::none());
        }
        let mut i = self.rows.partition_point(|r| r.y + r.h <= view_top);
        let mut changed_from = usize::MAX;
        let real_pass = !ui.is_sizing_pass();
        let mut positions = if real_pass {
            std::mem::take(&mut self.positions)
        } else {
            Vec::new()
        };
        positions.clear();
        while i < self.rows.len() && self.rows[i].y < view_bottom {
            let row = self.rows[i];
            let row_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), base + row.y), egui::vec2(width, row.h));
            if real_pass && row_rect.intersects(ui.clip_rect()) {
                positions.push((first + row.line as u64, row_rect.top() - rect.top() - viewport.top()));
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .scope_id(ui.scope_id().with(("conv_row", first + row.line as u64)))
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
        if real_pass {
            self.positions = positions;
            self.resume_following = false;
        }
        (if at_end { 0.0 } else { correction }, content)
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
        // an object's script errors open a group of their own, under their
        // warning sign, and what it says next opens the next (#1263)
        let object = |text: &str, secs, kind| ChatLine {
            kind: ChatKind::Object(kind),
            ..line("Demo HUD", text, secs)
        };
        let said = [
            object("ready", 0, aurora_net::ChatType::OwnerSay),
            object("error", 1, aurora_net::ChatType::Debug),
            object("error again", 2, aurora_net::ChatType::Debug),
            object("ready", 3, aurora_net::ChatType::OwnerSay),
        ];
        let mut g = Grouping::new(Slt::of(said[0].time));
        let headers: Vec<bool> = said.iter().map(|l| g.look(l).header).collect();
        assert_eq!(headers, [true, true, false, true]);
    }

    /// Aurora's workspace groups an author's messages within seven
    /// minutes, and an emote stands alone.
    #[test]
    fn the_workspace_groups_by_author_within_seven_minutes() {
        let lines = [
            line("Jane", "one", 0),
            line("Jane", "six minutes later", 360),
            line("Jane", "/me waves", 370),
            line("Jane", "after the emote", 380),
            line("Jane", "eight minutes later", 380 + 480),
        ];
        let today = Slt::of(lines[0].time);
        let looks: Vec<bool> = {
            let mut g = Grouping::with(today, Layout::Aurora);
            lines.iter().map(|l| g.look(l).header).collect()
        };
        assert_eq!(looks, [true, false, true, true, true]);
        // the tabs keep five minutes and emotes in their group
        let mut g = Grouping::new(today);
        let classic: Vec<bool> = lines.iter().map(|l| g.look(l).header).collect();
        assert_eq!(classic, [true, true, false, false, true]);
    }

    /// The first line unread when the conversation opened carries "New",
    /// unless it is the very first one.
    #[test]
    fn the_first_unread_line_is_marked_new() {
        let lines: VecDeque<ChatLine> = (0..5).map(|i| line("Jane", &format!("line {i}"), i * 10)).collect();
        let clock = LineClock::default();
        let mut view = View::default();
        let mut sync = |first_unread| {
            view.sync(
                &Source {
                    lines: &lines,
                    clock,
                    filter: "",
                    find: "",
                    layout: Layout::Aurora,
                    first_unread,
                },
                300.0,
            );
            view.rows.iter().map(|r| r.look.new).collect::<Vec<_>>()
        };
        assert_eq!(sync(Some(clock.first + 3)), [false, false, false, true, false]);
        assert_eq!(sync(Some(clock.first)), [false; 5]);
        assert_eq!(sync(None), [false; 5]);
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
                    layout: Layout::Classic,
                    first_unread: None,
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

    /// At the end of a conversation, a resize that wraps a message in sight
    /// again leaves the last one against the bottom on that very frame: the
    /// scroll area follows the end a frame late, the messages jumped for
    /// one frame at each line a row gained or lost (the human's report of
    /// 2026-10-16).
    #[test]
    fn a_resize_keeps_the_end_in_place() {
        let lines: VecDeque<ChatLine> = (0..40).map(|i| line("Jane", &format!("line {i}"), i * 600)).collect();
        let clock = LineClock::default();
        let mut view = View::default();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let mut bottoms = Vec::new();
        for width in [400.0, 400.0, 400.0, 300.0, 300.0, 400.0, 300.0, 250.0, 400.0, 400.0] {
            let mut last = None;
            frames.run(Vec::new(), |ui| {
                ui.set_max_width(width);
                let out = crate::ui::widgets::scroll_offset::show(ui, egui::ScrollArea::vertical(), "resize")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height(300.0)
                    .min_scrolled_height(300.0)
                    .show_viewport(ui, |ui, viewport| {
                        let source = Source {
                            lines: &lines,
                            clock,
                            filter: "",
                            find: "",
                            layout: Layout::Aurora,
                            first_unread: None,
                        };
                        view.sync(&source, width);
                        let (shift, _) = view.show(ui, viewport, clock.first, &mut |ui, row, painted| {
                            // a message in sight wraps on two lines when narrow
                            let narrow = ui.max_rect().width() < 350.0;
                            let h = if row.line == 37 && narrow { 40.0 } else { 20.0 };
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.max_rect().width(), h), egui::Sense::hover());
                            if painted && row.line == 39 {
                                last = Some(rect.bottom());
                            }
                        });
                        shift
                    });
                if out.inner != 0.0 {
                    let offset = out.state.unclamped_offset() + egui::vec2(0.0, out.inner);
                    crate::ui::widgets::scroll_offset::set(ui.ctx(), out.id, offset);
                }
            });
            bottoms.push(last.expect("the last message is drawn"));
        }
        let first = bottoms[1];
        assert!(bottoms[1..].iter().all(|b| (b - first).abs() < 0.5), "{bottoms:?}");
    }

    /// Away from the end, the view says how far it is and counts the lines
    /// that came in since; the offset `latest.rs` sets past the end brings
    /// it back, where it follows new lines again (#558).
    #[test]
    fn leaving_the_end_counts_the_new_lines_until_the_jump_back() {
        let mut lines: VecDeque<ChatLine> = (0..40).map(|i| line("Jane", &format!("line {i}"), i * 600)).collect();
        let mut clock = LineClock::default();
        let mut view = View::default();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let mut frame = |view: &mut View, lines: &VecDeque<ChatLine>, clock: LineClock, jump: bool| {
            frames.run(Vec::new(), |ui| {
                ui.set_max_width(400.0);
                let out = crate::ui::widgets::scroll_offset::show(ui, egui::ScrollArea::vertical(), "latest")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height(300.0)
                    .min_scrolled_height(300.0)
                    .show_viewport(ui, |ui, viewport| {
                        let source = Source {
                            lines,
                            clock,
                            filter: "",
                            find: "",
                            layout: Layout::Aurora,
                            first_unread: None,
                        };
                        view.sync(&source, 400.0);
                        view.show(ui, viewport, clock.first, &mut |ui, _, _| {
                            ui.allocate_exact_size(egui::vec2(ui.max_rect().width(), 20.0), egui::Sense::hover());
                        })
                        .0
                    });
                let mut offset = out.state.unclamped_offset() + egui::vec2(0.0, out.inner);
                if jump {
                    offset.y = out.content_size.y;
                }
                crate::ui::widgets::scroll_offset::set(ui.ctx(), out.id, offset);
            });
        };
        for _ in 0..3 {
            frame(&mut view, &lines, clock, false);
        }
        assert_eq!(view.below(), 0.0);
        // to the top: the end far below, nothing new yet
        view.scroll_to(0);
        for _ in 0..3 {
            frame(&mut view, &lines, clock, false);
        }
        assert!(view.below() > 300.0, "{}", view.below());
        assert_eq!(view.arrived(clock.first, |_| true), 0);
        // two lines come in, one of them kept
        for i in 0..2 {
            lines.push_back(line("Bob", &format!("new {i}"), 40 * 600 + i));
            clock.pushed_back();
        }
        frame(&mut view, &lines, clock, false);
        assert!(view.below() > 300.0);
        assert_eq!(view.arrived(clock.first, |_| true), 2);
        assert_eq!(view.arrived(clock.first, |i| i == 41), 1);
        // the jump: at the end, and following the next line there
        frame(&mut view, &lines, clock, true);
        for _ in 0..2 {
            frame(&mut view, &lines, clock, false);
        }
        assert_eq!(view.below(), 0.0);
        assert_eq!(view.arrived(clock.first, |_| true), 0);
        lines.push_back(line("Bob", "after", 50 * 600));
        clock.pushed_back();
        for _ in 0..3 {
            frame(&mut view, &lines, clock, false);
        }
        assert_eq!(view.below(), 0.0);
        assert_eq!(view.arrived(clock.first, |_| true), 0);
    }
}
