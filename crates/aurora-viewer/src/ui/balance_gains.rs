//! The L$ balance changing (#2675): a `+100 L$` in green or a `-100 L$` in
//! red under the balance, right-aligned with it, that waits and flies up
//! into the number. Several changes stack downwards and leave one at a
//! time. Aurora's own: the Legacy Viewer only has its toast.
//!
//! The rows come from the balance itself, whatever moved it: a sale, a
//! payment, L$ bought, a fee, a refund. A change is kept out of the number
//! until its row lands. The number then flashes, green for L$ gained, red
//! for L$ spent, and ticks to its new value, so it always ends on the real
//! balance. L$ received and L$ spent have a switch each (Preferences ›
//! Layout › Bars, the balance's right click): a change whose rows are off
//! goes into the number at once, without a row or a flash.
//!
//! Reduce motion: a row shows, waits and goes, and the number takes its new
//! value at once; no fade, flight, slide, count or flash.
//!
//! Frames: at full rate only while something moves, else one when the top
//! row's wait ends (`repaint_after`). The state takes the time as an
//! argument, the tests need no clock.

use crate::theme::Palette;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a row stays before it leaves.
const HOLD: Duration = Duration::from_secs(3);
/// How long a row stays on top before it leaves, however long it waited.
const ON_TOP: Duration = Duration::from_secs(1);
const FADE_IN: Duration = Duration::from_millis(200);
const FLY: Duration = Duration::from_millis(400);
const STEP: Duration = Duration::from_millis(250);
const COUNT: Duration = Duration::from_millis(600);
/// How long the number takes to fade back from its flash.
const FLASH: Duration = Duration::from_millis(600);
/// More rows would fill the screen: past them, the last waiting row of the
/// same sign adds up the next changes (a row of its own when none has it,
/// so at most one more).
const MAX_ROWS: usize = 6;
/// From one row to the next, in points.
const ROW_HEIGHT: f32 = 18.0;

/// Which rows show under the balance: L$ received (or bought), L$ spent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub received: bool,
    pub spent: bool,
}

impl Shown {
    pub const BOTH: Self = Self {
        received: true,
        spent: true,
    };

    fn any(self) -> bool {
        self.received || self.spent
    }

    /// Whether a change of `amount` L$ (spent when negative) has its row.
    fn of(self, amount: i64) -> bool {
        if amount > 0 { self.received } else { self.spent }
    }
}

struct Row {
    id: u64,
    /// L$ gained, or spent when negative; never 0.
    amount: i64,
    arrived: Instant,
    on_top: Option<Instant>,
}

/// L$ landing in the number, spent when negative: `total` was left when
/// `start` began.
struct Count {
    total: i64,
    start: Instant,
}

/// A row as this frame draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct GainRow {
    pub id: u64,
    /// L$ gained, or spent when negative.
    pub amount: i64,
    /// Rows under the balance: 0 the first, -1 in the number.
    pub slot: f32,
    pub alpha: f32,
}

/// What a frame shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub balance: Option<i32>,
    pub rows: Vec<GainRow>,
    /// The number's flash: towards 1 for L$ gained, -1 for L$ spent.
    pub flash: f32,
}

/// The balance's colour for a `Frame::flash`.
pub fn ink(p: &Palette, flash: f32) -> egui::Color32 {
    let towards = if flash < 0.0 { p.danger } else { p.success };
    p.ink.lerp_to_gamma(towards, flash.abs())
}

#[derive(Default)]
pub struct BalanceGains {
    rows: VecDeque<Row>,
    next: u64,
    last: Option<i32>,
    /// The switches the rows follow.
    shown: Shown,
    /// The first row flies since then.
    flying: Option<Instant>,
    /// The rows move up one step since then.
    step: Option<Instant>,
    count: Option<Count>,
    /// A row landed in the number then: whether it was L$ gained.
    flash: Option<(Instant, bool)>,
}

/// Fraction of `length` gone since `start`, 0 to 1.
fn progress(now: Instant, start: Instant, length: Duration) -> f32 {
    (now.saturating_duration_since(start).as_secs_f32() / length.as_secs_f32()).min(1.0)
}

/// Fast start, soft landing.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

impl BalanceGains {
    /// The next balance is a starting point, not a change (a demo's own).
    pub fn resync(&mut self) {
        self.last = None;
    }

    fn change(&mut self, amount: i64, now: Instant) {
        // its rows are off: it goes into the number at once
        if !self.shown.of(amount) {
            return;
        }
        // a flying row is already on its way into the number; a row of the
        // other sign would show "+0" or turn round
        let flying = usize::from(self.flying.is_some());
        if self.rows.len() >= MAX_ROWS
            && let Some(row) = self
                .rows
                .iter_mut()
                .skip(flying)
                .rev()
                .find(|row| row.amount.signum() == amount.signum())
        {
            row.amount += amount;
            return;
        }
        self.next += 1;
        self.rows.push_back(Row {
            id: self.next,
            amount,
            arrived: now,
            on_top: None,
        });
    }

    /// A switch was turned: the rows it now hides go into the number at
    /// once, and so do their count and flash.
    fn hide(&mut self, shown: Shown) {
        self.shown = shown;
        let front = self.rows.front().map(|row| row.id);
        let before = self.rows.len();
        self.rows.retain(|row| shown.of(row.amount));
        if self.rows.len() != before {
            self.step = None;
            if self.rows.front().map(|row| row.id) != front {
                self.flying = None;
            }
        }
        if self.count.as_ref().is_some_and(|count| !shown.of(count.total)) {
            self.count = None;
        }
        if self.flash.is_some_and(|(_, gained)| !shown.of(if gained { 1 } else { -1 })) {
            self.flash = None;
        }
    }

    /// The changes still on their way into the number.
    fn held(&self) -> i64 {
        self.rows.iter().map(|row| row.amount).sum()
    }

    /// The first row has landed: the number flashes and starts ticking, or
    /// takes its value at once with reduced motion.
    fn land(&mut self, still: bool, now: Instant) {
        let Some(row) = self.rows.pop_front() else {
            return;
        };
        if still {
            self.count = None;
            self.flash = None;
            self.step = None;
            return;
        }
        let left = self.left(now) + row.amount;
        self.flash = Some((now, row.amount > 0));
        self.count = (left != 0).then_some(Count { total: left, start: now });
        self.step = (!self.rows.is_empty()).then_some(now);
    }

    /// What is left of the count, rounded away from zero: it ends on 0. In
    /// f64, exact for any two balances, where an f32 is not past 2^24.
    fn left(&self, now: Instant) -> i64 {
        self.count.as_ref().map_or(0, |c| {
            let rest = (1.0 - f64::from(progress(now, c.start, COUNT))).powi(2);
            (c.total.abs() as f64 * rest).ceil() as i64 * c.total.signum()
        })
    }

    /// One frame: `balance` is the real one, `shown` the rows wanted (none
    /// while the balance is hidden), `still` the "Reduce motion" setting.
    pub fn update(&mut self, balance: Option<i32>, shown: Shown, still: bool, now: Instant) -> Frame {
        let Some(real) = balance.filter(|_| shown.any()) else {
            // forgotten once, then nothing to do while it stays so
            if self.last.is_some() {
                let next = self.next;
                *self = Self { next, ..Self::default() };
            }
            return Frame {
                balance,
                ..Frame::default()
            };
        };
        if shown != self.shown {
            self.hide(shown);
        }
        if let Some(last) = self.last
            && real != last
        {
            self.change(i64::from(real) - i64::from(last), now);
        }
        self.last = Some(real);
        if self.flying.is_some_and(|start| now >= start + FLY) {
            self.flying = None;
            self.land(still, now);
        }
        if self.step.is_some_and(|start| now >= start + STEP) {
            self.step = None;
        }
        if self.count.as_ref().is_some_and(|c| now >= c.start + COUNT) {
            self.count = None;
        }
        if self.flash.is_some_and(|(start, _)| now >= start + FLASH) {
            self.flash = None;
        }
        if self.flying.is_none()
            && self.step.is_none()
            && let Some(top) = self.rows.front_mut()
        {
            let on_top = *top.on_top.get_or_insert(now);
            if now >= top.arrived + HOLD && now >= on_top + ON_TOP {
                if still {
                    // no flight: it goes, and the next one is on top
                    self.land(true, now);
                    if let Some(next) = self.rows.front_mut() {
                        next.on_top = Some(now);
                    }
                } else {
                    self.flying = Some(now);
                }
            }
        }
        let number = i64::from(real) - self.held() - self.left(now);
        Frame {
            balance: Some(i32::try_from(number).unwrap_or(real)),
            rows: self.rows(still, now),
            flash: self.flash.map_or(0.0, |(start, gained)| {
                let left = 1.0 - progress(now, start, FLASH);
                if gained { left } else { -left }
            }),
        }
    }

    /// When the next frame is due after `update`: at once while something
    /// moves (a fade, a flight, a slide, the count, the flash), when the
    /// top row's wait ends while the rows stand still, never with nothing
    /// to show.
    pub fn repaint_after(&self, still: bool, now: Instant) -> Option<Duration> {
        let fading_in = !still && self.rows.iter().any(|row| now < row.arrived + FADE_IN);
        if fading_in || self.flying.is_some() || self.step.is_some() || self.count.is_some() || self.flash.is_some() {
            return Some(Duration::ZERO);
        }
        let top = self.rows.front()?;
        let leaves = (top.arrived + HOLD).max(top.on_top.unwrap_or(now) + ON_TOP);
        Some(leaves.saturating_duration_since(now))
    }

    fn rows(&self, still: bool, now: Instant) -> Vec<GainRow> {
        let moved = self.step.map_or(0.0, |start| 1.0 - ease_out(progress(now, start, STEP)));
        self.rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let mut slot = i as f32 + moved;
                let mut alpha = if still { 1.0 } else { progress(now, row.arrived, FADE_IN) };
                if i == 0
                    && let Some(start) = self.flying
                {
                    let t = progress(now, start, FLY);
                    if still {
                        // Reduce motion turned on during its flight
                        alpha = 1.0 - t;
                    } else {
                        slot = -ease_out(t);
                        // it melts into the number at the end of its flight
                        alpha = ((1.0 - t) / 0.4).min(1.0);
                    }
                }
                GainRow {
                    id: row.id,
                    amount: row.amount,
                    slot,
                    alpha,
                }
            })
            .collect()
    }
}

/// The parts of a shown balance: each digit alone, what lies between them
/// as one run.
fn parts(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(|c: char| c.is_ascii_digit()).flat_map(|part| {
        // a run ending in its digit: the run, then the digit
        let digit = part.len() - part.chars().last().filter(char::is_ascii_digit).map_or(0, char::len_utf8);
        [&part[..digit], &part[digit..]].into_iter().filter(|part| !part.is_empty())
    })
}

fn is_digit(part: &str) -> bool {
    part.starts_with(|c: char| c.is_ascii_digit())
}

/// The cell of every digit in `font`: as wide as its widest digit, so that
/// the number and what holds it keep their width while it ticks. Measured
/// once per font and scale, and again once new fonts are in use
/// (`fonts::installed_at`), not ten layouts per digit and per frame.
fn digit_cell(painter: &egui::Painter, font: &egui::FontId) -> f32 {
    let ctx = painter.ctx();
    let id = egui::Id::unique(("balance_digit_cell", font, painter.pixels_per_point().to_bits()));
    let installed = crate::ui::fonts::installed_at(ctx);
    if let Some((measured, width)) = ctx.data(|d| d.get_temp::<(u64, f32)>(id))
        && installed.is_none_or(|at| measured > at)
    {
        return width;
    }
    let width = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
        .into_iter()
        .map(|digit| {
            painter
                .layout_no_wrap(digit.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(id, (frame, width)));
    width
}

/// How wide `part` stands: a digit its `cell`, the rest its own width.
fn part_width(painter: &egui::Painter, part: &str, font: &egui::FontId, cell: f32) -> f32 {
    if is_digit(part) {
        cell
    } else {
        painter
            .layout_no_wrap(part.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    }
}

/// The width of a shown balance, for its digits' cells.
pub fn number_width(painter: &egui::Painter, text: &str, font: &egui::FontId) -> f32 {
    let cell = digit_cell(painter, font);
    parts(text).map(|part| part_width(painter, part, font, cell)).sum()
}

/// A shown balance from `left_center`, each digit centred in its cell.
pub fn paint_number(painter: &egui::Painter, text: &str, font: &egui::FontId, left_center: egui::Pos2, color: egui::Color32) {
    let digit = digit_cell(painter, font);
    let mut x = left_center.x;
    for part in parts(text) {
        // `paint_ink_left` starts at the ink: a part's leading space stays
        let ink = part.trim_start();
        if ink.is_empty() {
            x += part_width(painter, part, font, digit);
            continue;
        }
        let galley = painter.layout_no_wrap(ink.to_owned(), font.clone(), color);
        let (width, cell) = if is_digit(part) {
            (digit, digit)
        } else if ink.len() == part.len() {
            (galley.size().x, galley.size().x)
        } else {
            (part_width(painter, part, font, digit), galley.size().x)
        };
        let left = x + width - cell + (cell - galley.size().x) / 2.0;
        crate::ui::widgets::paint_ink_left(painter, galley, egui::pos2(left, left_center.y), color);
        x += width;
    }
}

/// Where the rows of a balance go: `right` the end of its text, `text_top`
/// where the text starts and `top` where the first row stands.
pub struct Anchor {
    pub right: f32,
    pub text_top: f32,
    pub top: f32,
}

/// The rows over the world, under the balance: L$ received in the success
/// colour, L$ spent in the danger colour and semibold. They take no input:
/// the balance and what lies under them stay clickable.
pub fn paint(ctx: &egui::Context, p: &Palette, rows: &[GainRow], anchor: &Anchor, font: egui::FontId) {
    if rows.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("balance_gains")));
    for row in rows {
        let spent = row.amount < 0;
        let sign = if spent { '-' } else { '+' };
        let text = format!("{sign}{} {}", aurora_i18n::integer(row.amount.abs()), aurora_i18n::currency());
        let y = if row.slot >= 0.0 {
            anchor.top + row.slot * ROW_HEIGHT
        } else {
            anchor.top + -row.slot * (anchor.text_top - anchor.top)
        };
        let (color, font) = if spent {
            (p.danger, egui::FontId::new(font.size, crate::ui::fonts::semibold(ctx)))
        } else {
            (p.success, font.clone())
        };
        let color = color.gamma_multiply(row.alpha);
        let galley = painter.layout_no_wrap(text, font, color);
        painter.galley(egui::pos2(anchor.right - galley.size().x, y), galley, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFTER: Duration = Duration::from_millis(16);

    struct Run {
        gains: BalanceGains,
        now: Instant,
        balance: i32,
        shown: Shown,
        still: bool,
    }

    impl Run {
        fn new(balance: i32) -> Self {
            let mut run = Run {
                gains: BalanceGains::default(),
                now: Instant::now(),
                balance,
                shown: Shown::BOTH,
                still: false,
            };
            run.frame();
            run
        }

        fn frame(&mut self) -> Frame {
            self.gains.update(Some(self.balance), self.shown, self.still, self.now)
        }

        /// Frames for `length`, the last one returned.
        fn wait(&mut self, length: Duration) -> Frame {
            let end = self.now + length;
            let mut frame = self.frame();
            while self.now < end {
                self.now += AFTER;
                frame = self.frame();
            }
            frame
        }

        fn repaint(&self) -> Option<Duration> {
            self.gains.repaint_after(self.still, self.now)
        }
    }

    const ALL_OF_IT: Duration = Duration::from_secs(60);

    fn amounts(frame: &Frame) -> Vec<i64> {
        frame.rows.iter().map(|r| r.amount).collect()
    }

    #[test]
    fn a_number_splits_into_its_digits_and_what_lies_between() {
        let parts = |text| parts(text).collect::<Vec<_>>();
        assert_eq!(parts("12,480 L$"), ["1", "2", ",", "4", "8", "0", " L$"]);
        assert_eq!(parts("7 L$"), ["7", " L$"]);
        assert_eq!(parts("1 234 €"), ["1", " ", "2", "3", "4", " €"]);
        assert_eq!(parts("L$"), ["L$"]);
        assert_eq!(parts(""), Vec::<&str>::new());
    }

    fn settled(balance: i32) -> Frame {
        Frame {
            balance: Some(balance),
            ..Frame::default()
        }
    }

    #[test]
    fn a_gain_waits_as_a_row_then_lands_exactly_on_the_balance() {
        let mut run = Run::new(1000);
        run.balance += 100;
        let frame = run.wait(AFTER);
        assert_eq!(frame.balance, Some(1000), "held back while the row waits");
        assert_eq!(frame.rows.len(), 1);
        assert_eq!((frame.rows[0].amount, frame.rows[0].slot), (100, 0.0));
        let frame = run.wait(HOLD - AFTER * 2);
        assert_eq!(frame.balance, Some(1000));
        // the flight: up into the number, the number still waiting
        let frame = run.wait(AFTER * 8);
        assert!(frame.rows[0].slot < 0.0, "{frame:?}");
        assert_eq!(frame.balance, Some(1000));
        // the count-up rises and never passes the total
        let mut last = 1000;
        for _ in 0..80 {
            let frame = run.wait(AFTER);
            let number = frame.balance.expect("a balance");
            assert!((last..=1100).contains(&number), "{number} after {last}");
            last = number;
        }
        let frame = run.wait(COUNT);
        assert_eq!(frame, settled(1100));
    }

    #[test]
    fn whatever_raises_the_balance_shows_a_row() {
        // a transaction names nothing here: L$ bought come the same way
        let mut run = Run::new(0);
        run.balance = 5000;
        assert_eq!(run.wait(AFTER).rows[0].amount, 5000);
    }

    #[test]
    fn a_purchase_waits_as_a_row_then_ticks_down_exactly_to_the_balance() {
        let mut run = Run::new(1000);
        run.balance -= 300;
        let frame = run.wait(AFTER);
        assert_eq!(frame.balance, Some(1000), "held back while the row waits");
        assert_eq!(frame.rows[0].amount, -300);
        let frame = run.wait(HOLD + AFTER * 4);
        assert!(frame.rows[0].slot < 0.0, "{frame:?}");
        assert_eq!(frame.balance, Some(1000));
        // it lands: a red flash, and the number never leaves the two values
        let frame = run.wait(FLY);
        assert_eq!(frame.rows.len(), 0);
        assert!(frame.flash < -0.5, "{frame:?}");
        let mut last = frame.balance.expect("a balance");
        for _ in 0..60 {
            let number = run.wait(AFTER).balance.expect("a balance");
            assert!((700..=last).contains(&number), "{number} after {last}");
            last = number;
        }
        assert_eq!(run.wait(ALL_OF_IT), settled(700));
    }

    #[test]
    fn a_gain_flashes_green_where_a_purchase_flashes_red() {
        let mut run = Run::new(1000);
        run.balance += 100;
        run.wait(HOLD + FLY + AFTER * 4);
        let frame = run.frame();
        assert!(frame.flash > 0.5, "{frame:?}");
        let p = crate::theme::Theme::default().palette();
        assert_ne!(ink(&p, 1.0), ink(&p, -1.0));
        assert_eq!(ink(&p, 0.0), p.ink);
        assert_eq!(ink(&p, 1.0), p.success);
        assert_eq!(ink(&p, -1.0), p.danger);
    }

    #[test]
    fn no_flash_with_reduced_motion() {
        let mut run = Run::new(1000);
        run.still = true;
        run.balance -= 100;
        run.wait(HOLD + FLY + AFTER * 4);
        let frame = run.frame();
        assert_eq!((frame.flash, frame.balance), (0.0, Some(900)));
        assert_eq!(run.repaint(), None);
    }

    #[test]
    fn changes_land_in_turn_on_the_real_balance() {
        let mut run = Run::new(1000);
        run.balance += 100;
        run.wait(AFTER);
        run.balance -= 300;
        let frame = run.wait(AFTER);
        assert_eq!(frame.balance, Some(1000), "nothing moves before its row");
        assert_eq!(amounts(&frame), [100, -300]);
        assert_eq!(run.wait(ALL_OF_IT), settled(800));
    }

    #[test]
    fn rows_stack_downwards_and_leave_one_at_a_time() {
        let mut run = Run::new(0);
        for amount in [10, 20, 30] {
            run.balance += amount;
            run.wait(AFTER);
        }
        let frame = run.wait(AFTER);
        assert_eq!(amounts(&frame), [10, 20, 30]);
        assert_eq!(frame.rows.iter().map(|r| r.slot).collect::<Vec<_>>(), [0.0, 1.0, 2.0]);
        // the first leaves, the others stay where they are until it has landed
        let frame = run.wait(HOLD + AFTER * 2);
        assert_eq!(amounts(&frame), [10, 20, 30]);
        assert!(frame.rows[0].slot < 0.0);
        assert_eq!(frame.rows[1].slot, 1.0);
        // landed: one step up, in a slide
        let frame = run.wait(FLY);
        assert_eq!(amounts(&frame), [20, 30]);
        assert!(frame.rows[0].slot > 0.0 && frame.rows[0].slot < 1.0, "{frame:?}");
        let frame = run.wait(STEP);
        assert_eq!(frame.rows.iter().map(|r| r.slot).collect::<Vec<_>>(), [0.0, 1.0]);
        // it has waited long enough, yet stays on top for a moment
        let frame = run.wait(ON_TOP / 2);
        assert_eq!(frame.rows[0].slot, 0.0, "{frame:?}");
        assert_eq!(run.wait(ALL_OF_IT), settled(60));
    }

    /// The balance comes from the grid: any two values of it.
    #[test]
    fn extreme_balances_do_not_overflow() {
        let mut run = Run::new(-2_000_000_000);
        run.balance = 2_000_000_000;
        assert_eq!(run.wait(AFTER).rows[0].amount, 4_000_000_000);
        assert_eq!(run.wait(ALL_OF_IT), settled(2_000_000_000));
    }

    #[test]
    fn a_count_past_two_to_the_24_never_passes_its_total() {
        let mut run = Run::new(0);
        run.balance = 16_777_217;
        run.wait(HOLD + FLY + AFTER * 2);
        let mut last = run.frame().balance.expect("a balance");
        for _ in 0..60 {
            let number = run.wait(AFTER).balance.expect("a balance");
            assert!((last..=16_777_217).contains(&number), "{number} after {last}");
            last = number;
        }
        assert_eq!(run.wait(ALL_OF_IT), settled(16_777_217));
    }

    #[test]
    fn past_six_rows_the_last_one_adds_up() {
        let mut run = Run::new(0);
        for _ in 0..9 {
            run.balance += 5;
            run.frame();
        }
        let frame = run.wait(AFTER);
        assert_eq!(frame.rows.len(), MAX_ROWS);
        assert_eq!(frame.rows[MAX_ROWS - 1].amount, 20);
        let frame = run.wait((HOLD + ON_TOP + FLY + STEP) * 8);
        assert_eq!(frame, settled(45));
    }

    #[test]
    fn past_six_rows_a_change_adds_up_with_its_own_sign_only() {
        let mut run = Run::new(1000);
        for _ in 0..MAX_ROWS {
            run.balance += 5;
            run.frame();
        }
        // the other way: never "+0 L$" nor a row that turns round
        run.balance -= 5;
        let frame = run.frame();
        assert_eq!(amounts(&frame), [5, 5, 5, 5, 5, 5, -5]);
        run.balance -= 7;
        run.balance += 3;
        let frame = run.frame();
        assert_eq!(amounts(&frame), [5, 5, 5, 5, 5, 5, -9]);
        run.balance += 2;
        let frame = run.frame();
        assert_eq!(amounts(&frame), [5, 5, 5, 5, 5, 7, -9]);
        assert!(frame.rows.iter().all(|r| r.amount != 0));
        assert_eq!(run.wait(ALL_OF_IT), settled(1023));
    }

    #[test]
    fn each_switch_shows_its_own_rows_only() {
        let mut run = Run::new(1000);
        run.shown = Shown {
            received: false,
            spent: true,
        };
        run.balance += 100;
        let frame = run.wait(AFTER);
        assert_eq!(frame, settled(1100), "into the number at once, no flash");
        run.balance -= 40;
        let frame = run.wait(AFTER);
        assert_eq!((frame.balance, amounts(&frame)), (Some(1100), vec![-40]));
        run.shown = Shown {
            received: true,
            spent: false,
        };
        run.balance += 10;
        let frame = run.wait(AFTER);
        // the spent row went into the number when its switch was turned off
        assert_eq!((frame.balance, amounts(&frame)), (Some(1060), vec![10]));
        let frame = run.wait(HOLD + FLY + AFTER * 2);
        assert!(frame.flash > 0.0, "{frame:?}");
        // and the flash of what lands goes with its switch
        run.shown = Shown {
            received: false,
            spent: true,
        };
        assert_eq!(run.wait(AFTER), settled(1070));
    }

    #[test]
    fn reduced_motion_shows_and_hides_without_motion() {
        let mut run = Run::new(1000);
        run.still = true;
        run.balance += 100;
        run.frame();
        run.balance += 10;
        let frame = run.frame();
        assert!(frame.rows.iter().all(|r| r.alpha == 1.0), "no fade in: {frame:?}");
        let frame = run.wait(HOLD - AFTER * 2);
        assert_eq!(frame.balance, Some(1000));
        let frame = run.wait(AFTER * 4);
        assert_eq!(frame.balance, Some(1100), "no flight nor count-up");
        assert_eq!(
            frame.rows.iter().map(|r| (r.amount, r.slot, r.alpha)).collect::<Vec<_>>(),
            [(10, 0.0, 1.0)]
        );
        assert_eq!(run.wait(ON_TOP), settled(1110));
    }

    #[test]
    fn frames_at_full_rate_only_while_something_moves() {
        let mut run = Run::new(1000);
        assert_eq!(run.repaint(), None, "nothing to show");
        run.balance += 100;
        run.frame();
        assert_eq!(run.repaint(), Some(Duration::ZERO), "it fades in");
        run.wait(FADE_IN + AFTER);
        let wait = run.repaint().expect("a row waits");
        assert!(wait > Duration::from_secs(2) && wait <= HOLD, "{wait:?}");
        // its wait over: the flight, the count and the flash move
        run.now += wait;
        run.frame();
        assert_eq!(run.repaint(), Some(Duration::ZERO));
        run.wait(ALL_OF_IT);
        assert_eq!(run.repaint(), None);
        // with reduced motion, never at full rate
        run.still = true;
        run.balance += 5;
        run.frame();
        assert!(run.repaint().is_some_and(|wait| wait > Duration::ZERO));
        run.wait(ALL_OF_IT);
        assert_eq!(run.repaint(), None);
    }

    #[test]
    fn a_hidden_balance_or_rows_have_none_and_forget_them() {
        let mut run = Run::new(1000);
        run.balance += 100;
        run.wait(AFTER);
        let frame = run.gains.update(Some(run.balance), Shown::default(), false, run.now);
        assert_eq!(frame, settled(1100));
        run.shown = Shown::default();
        assert_eq!(run.wait(AFTER), settled(1100));
        assert_eq!(run.repaint(), None);
        // what changed meanwhile is no change once they show again
        run.balance += 50;
        run.frame();
        run.shown = Shown::BOTH;
        assert_eq!(run.wait(AFTER), settled(1150));
        let frame = run.gains.update(None, Shown::BOTH, false, run.now);
        assert_eq!(frame, Frame::default());
    }

    #[test]
    fn the_first_balance_is_no_change() {
        let mut gains = BalanceGains::default();
        let frame = gains.update(Some(1000), Shown::BOTH, false, Instant::now());
        assert_eq!(frame, settled(1000));
        gains.resync();
        let frame = gains.update(Some(9000), Shown::BOTH, false, Instant::now());
        assert_eq!(frame.rows, Vec::new());
    }
}
