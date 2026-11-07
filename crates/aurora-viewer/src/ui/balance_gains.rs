//! The L$ balance changing (#2675): a `+100 L$` or, in the danger colour, a
//! `-100 L$` under the balance, right-aligned with it, that waits and flies
//! up into the number. Several changes stack downwards and leave one at a
//! time. Aurora's own: the Legacy Viewer only has its toast.
//!
//! The rows come from the balance itself, whatever moved it: a sale, a
//! payment, L$ bought, a fee, a refund. L$ gained are kept out of the number
//! until they land, then it counts up, so it always ends on the real balance.
//! L$ spent are never held back: the number drops at once and stays exact,
//! so that nobody counts on money already gone; when their row lands, the
//! number flashes in its colour.
//!
//! The state takes the time as an argument, the tests need no clock.

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
/// How long the number takes to fade back from the colour of L$ spent.
const FLASH: Duration = Duration::from_millis(600);
/// More rows would fill the screen: the last one adds up the next changes.
const MAX_ROWS: usize = 6;
/// From one row to the next, in points.
const ROW_HEIGHT: f32 = 18.0;

struct Row {
    id: u64,
    /// L$ gained, or spent when negative.
    amount: i64,
    arrived: Instant,
    on_top: Option<Instant>,
}

/// L$ landing in the number: `total` was left when `start` began.
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
    /// The colour of L$ spent on the number, 0 to 1.
    pub flash: f32,
}

/// The balance's colour, `flash` towards that of L$ spent.
pub fn ink(p: &Palette, flash: f32) -> egui::Color32 {
    p.ink.lerp_to_gamma(p.danger, flash)
}

#[derive(Default)]
pub struct BalanceGains {
    rows: VecDeque<Row>,
    next: u64,
    last: Option<i32>,
    /// The first row flies since then.
    flying: Option<Instant>,
    /// The rows move up one step since then.
    step: Option<Instant>,
    count: Option<Count>,
    /// L$ spent landed in the number then.
    flash: Option<Instant>,
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
        if self.rows.len() >= MAX_ROWS
            && let Some(last) = self.rows.back_mut()
        {
            last.amount += amount;
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

    /// L$ gained still on their way into the number.
    fn held(&self) -> i64 {
        self.rows.iter().map(|row| row.amount.max(0)).sum()
    }

    /// The first row has landed: L$ gained start counting up.
    fn land(&mut self, still: bool, now: Instant) {
        let Some(row) = self.rows.pop_front() else {
            return;
        };
        if row.amount < 0 && !still {
            self.flash = Some(now);
        }
        let left = self.left(now) + row.amount.max(0);
        self.count = (!still && left > 0).then_some(Count { total: left, start: now });
        self.step = (!still && !self.rows.is_empty()).then_some(now);
    }

    /// What is left of the count-up.
    fn left(&self, now: Instant) -> i64 {
        self.count.as_ref().map_or(0, |c| {
            let t = progress(now, c.start, COUNT);
            (c.total as f32 * (1.0 - ease_out(t))).ceil() as i64
        })
    }

    /// One frame: `balance` is the real one, `shown` whether it is on
    /// screen with its highlight, `still` the "Reduce motion" setting.
    pub fn update(&mut self, balance: Option<i32>, shown: bool, still: bool, now: Instant) -> Frame {
        let Some(real) = balance.filter(|_| shown) else {
            let next = self.next;
            *self = Self { next, ..Self::default() };
            return Frame {
                balance,
                rows: Vec::new(),
                flash: 0.0,
            };
        };
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
        if self.flash.is_some_and(|start| now >= start + FLASH) {
            self.flash = None;
        }
        if self.flying.is_none()
            && self.step.is_none()
            && let Some(top) = self.rows.front_mut()
        {
            let on_top = *top.on_top.get_or_insert(now);
            if now >= top.arrived + HOLD && now >= on_top + ON_TOP {
                self.flying = Some(now);
            }
        }
        let number = i64::from(real) - self.held() - self.left(now);
        Frame {
            balance: Some(i32::try_from(number.clamp(i64::from(real.min(0)), i64::from(real))).unwrap_or(real)),
            rows: self.rows(still, now),
            flash: self.flash.map_or(0.0, |start| 1.0 - progress(now, start, FLASH)),
        }
    }

    /// Something moves: the frames must keep coming.
    pub fn animating(&self) -> bool {
        !self.rows.is_empty() || self.count.is_some() || self.flash.is_some()
    }

    fn rows(&self, still: bool, now: Instant) -> Vec<GainRow> {
        let moved = self.step.map_or(0.0, |start| 1.0 - ease_out(progress(now, start, STEP)));
        self.rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let mut slot = i as f32 + moved;
                let mut alpha = progress(now, row.arrived, FADE_IN);
                if i == 0
                    && let Some(start) = self.flying
                {
                    let t = progress(now, start, FLY);
                    if still {
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

/// Where the rows of a balance go: `right` the end of its text, `text_top`
/// where the text starts and `top` where the first row stands.
pub struct Anchor {
    pub right: f32,
    pub text_top: f32,
    pub top: f32,
}

/// The rows over the world, under the balance. They take no input: the
/// balance and what lies under them stay clickable.
pub fn paint(ctx: &egui::Context, p: &Palette, rows: &[GainRow], anchor: &Anchor, font: egui::FontId) {
    if rows.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("balance_gains")));
    for row in rows {
        let sign = if row.amount < 0 { '-' } else { '+' };
        let text = format!("{sign}{} {}", aurora_i18n::integer(row.amount.abs()), aurora_i18n::currency());
        let y = if row.slot >= 0.0 {
            anchor.top + row.slot * ROW_HEIGHT
        } else {
            anchor.top + -row.slot * (anchor.text_top - anchor.top)
        };
        // the colour of the balance it joins; what left it stands out
        let spent = row.amount < 0;
        let color = if spent { p.danger } else { p.ink };
        let font = if spent {
            egui::FontId::new(font.size, crate::ui::fonts::semibold(ctx))
        } else {
            font.clone()
        };
        let galley = painter.layout_no_wrap(text, font, color.gamma_multiply(row.alpha));
        let pos = egui::pos2(anchor.right - galley.size().x, y);
        if spent {
            // heavier than the semibold, the heaviest face there is
            painter.galley(pos + egui::vec2(0.6, 0.0), galley.clone(), color);
        }
        painter.galley(pos, galley, color);
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
        still: bool,
    }

    impl Run {
        fn new(balance: i32) -> Self {
            let mut run = Run {
                gains: BalanceGains::default(),
                now: Instant::now(),
                balance,
                still: false,
            };
            run.frame();
            run
        }

        fn frame(&mut self) -> Frame {
            self.gains.update(Some(self.balance), true, self.still, self.now)
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
    }

    const ALL_OF_IT: Duration = Duration::from_secs(60);

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
    fn a_purchase_drops_the_number_at_once_and_shows_a_negative_row() {
        let mut run = Run::new(1000);
        run.balance -= 300;
        let frame = run.wait(AFTER);
        assert_eq!(frame.balance, Some(700), "never held back");
        assert_eq!(frame.rows[0].amount, -300);
        // it leaves like the others, and nothing counts
        let frame = run.wait(HOLD + AFTER * 4);
        assert!(frame.rows[0].slot < 0.0, "{frame:?}");
        assert_eq!(frame.balance, Some(700));
        // it lands: the number flashes, fades back and stays
        let frame = run.wait(FLY);
        assert_eq!((frame.balance, frame.rows.len()), (Some(700), 0));
        assert!(frame.flash > 0.5, "{frame:?}");
        let later = run.wait(FLASH / 2);
        assert!(later.flash < frame.flash, "{later:?}");
        assert_eq!(later.balance, Some(700));
        assert_eq!(run.wait(FLASH), settled(700));
    }

    #[test]
    fn only_spending_flashes_and_never_with_reduced_motion() {
        let mut run = Run::new(1000);
        run.balance += 100;
        run.wait(HOLD + FLY + AFTER * 4);
        assert_eq!(run.frame().flash, 0.0, "a gain counts up instead");
        let mut run = Run::new(1000);
        run.still = true;
        run.balance -= 100;
        run.wait(HOLD + FLY + AFTER * 4);
        assert_eq!(run.frame().flash, 0.0);
        assert_eq!(run.wait(ALL_OF_IT), settled(900));
        assert!(!run.gains.animating());
    }

    #[test]
    fn a_purchase_while_a_gain_waits_lowers_the_number_too() {
        let mut run = Run::new(1000);
        run.balance += 100;
        run.wait(AFTER);
        run.balance -= 300;
        let frame = run.wait(AFTER);
        assert_eq!(frame.balance, Some(700));
        assert_eq!(frame.rows.iter().map(|r| r.amount).collect::<Vec<_>>(), [100, -300]);
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
        let amounts = |f: &Frame| f.rows.iter().map(|r| r.amount).collect::<Vec<_>>();
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
    fn reduced_motion_fades_in_place_and_jumps_to_the_total() {
        let mut run = Run::new(1000);
        run.still = true;
        run.balance += 100;
        run.frame();
        run.balance += 10;
        run.wait(HOLD + AFTER * 4);
        let frame = run.frame();
        assert_eq!(frame.rows[0].slot, 0.0, "no flight");
        assert!(frame.rows[0].alpha < 1.0, "it fades");
        assert_eq!(frame.balance, Some(1000));
        let frame = run.wait(FLY);
        assert_eq!(frame.balance, Some(1100), "no count-up");
        assert_eq!(frame.rows.iter().map(|r| r.slot).collect::<Vec<_>>(), [0.0], "no slide");
    }

    #[test]
    fn a_hidden_balance_or_highlight_has_no_rows_and_forgets_them() {
        let mut run = Run::new(1000);
        run.balance += 100;
        run.wait(AFTER);
        let frame = run.gains.update(Some(run.balance), false, false, run.now);
        assert_eq!(frame, settled(1100));
        assert_eq!(run.wait(AFTER), settled(1100));
        // what changed meanwhile is no change once it shows again
        run.gains.update(Some(run.balance + 50), false, false, run.now);
        run.balance += 50;
        assert_eq!(run.wait(AFTER).rows, Vec::new());
        let frame = run.gains.update(None, true, false, run.now);
        assert_eq!(frame, Frame::default());
    }

    #[test]
    fn the_first_balance_is_no_change() {
        let mut gains = BalanceGains::default();
        let frame = gains.update(Some(1000), true, false, Instant::now());
        assert_eq!(frame, settled(1000));
        gains.resync();
        let frame = gains.update(Some(9000), true, false, Instant::now());
        assert_eq!(frame.rows, Vec::new());
    }
}
