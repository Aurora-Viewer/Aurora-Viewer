//! L$ received rising into the balance (#2675): a `+100 L$` under the
//! balance, right-aligned with it, that waits, flies up into the number,
//! which then counts up to its total. Several payments stack downwards and
//! leave one at a time. Aurora's own: the Legacy Viewer only has its toast.
//!
//! The number is the real balance minus what is still on its way, so it
//! always ends on the real balance, and a purchase meanwhile shows at once.
//! An amount is only held back once the balance has risen by it: the reply
//! naming the transaction comes before the balance it brought.
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
/// More rows would fill the screen: the last one adds up the next payments.
const MAX_ROWS: usize = 6;
/// From one row to the next, in points.
const ROW_HEIGHT: f32 = 18.0;

struct Row {
    id: u64,
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
}

#[derive(Default)]
pub struct BalanceGains {
    rows: VecDeque<Row>,
    next: u64,
    /// L$ of the rows the balance has not brought yet.
    unmatched: i64,
    /// L$ of the rows the balance holds, kept out of the number.
    held: i64,
    last: Option<i32>,
    /// The first row flies since then.
    flying: Option<Instant>,
    /// The rows move up one step since then.
    step: Option<Instant>,
    count: Option<Count>,
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
    pub fn push(&mut self, amount: i32, now: Instant) {
        let amount = i64::from(amount);
        self.unmatched += amount;
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

    /// The first row has landed: its L$ start counting up.
    fn land(&mut self, still: bool, now: Instant) {
        let Some(row) = self.rows.pop_front() else {
            return;
        };
        let got = row.amount.min(self.held);
        self.held -= got;
        self.unmatched = (self.unmatched - (row.amount - got)).max(0);
        let left = self.left(now);
        self.count = (!still && left + got > 0).then_some(Count {
            total: left + got,
            start: now,
        });
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
    /// screen, `still` the "Reduce motion" setting.
    pub fn update(&mut self, balance: Option<i32>, shown: bool, still: bool, now: Instant) -> Frame {
        let Some(real) = balance.filter(|_| shown) else {
            let next = self.next;
            *self = Self { next, ..Self::default() };
            return Frame { balance, rows: Vec::new() };
        };
        if let Some(last) = self.last
            && real > last
        {
            let matched = (i64::from(real) - i64::from(last)).min(self.unmatched);
            self.unmatched -= matched;
            self.held += matched;
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
        if self.flying.is_none()
            && self.step.is_none()
            && let Some(top) = self.rows.front_mut()
        {
            let on_top = *top.on_top.get_or_insert(now);
            if now >= top.arrived + HOLD && now >= on_top + ON_TOP {
                self.flying = Some(now);
            }
        }
        let number = i64::from(real) - self.held - self.left(now);
        Frame {
            balance: Some(i32::try_from(number.clamp(i64::from(real.min(0)), i64::from(real))).unwrap_or(real)),
            rows: self.rows(still, now),
        }
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
        let text = format!("+{} {}", aurora_i18n::integer(row.amount), aurora_i18n::currency());
        let y = if row.slot >= 0.0 {
            anchor.top + row.slot * ROW_HEIGHT
        } else {
            anchor.top + -row.slot * (anchor.text_top - anchor.top)
        };
        // the colour of the balance it joins
        let galley = painter.layout_no_wrap(text, font.clone(), p.ink.gamma_multiply(row.alpha));
        let pos = egui::pos2(anchor.right - galley.size().x, y);
        painter.galley(pos, galley, p.ink);
    }
    ctx.request_repaint();
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

        /// A payment as the session gives it: the transaction, then the balance.
        fn paid(&mut self, amount: i32) {
            self.gains.push(amount, self.now);
            self.balance += amount;
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

    #[test]
    fn a_payment_waits_as_a_row_then_lands_exactly_on_the_balance() {
        let mut run = Run::new(1000);
        run.paid(100);
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
        assert_eq!(
            frame,
            Frame {
                balance: Some(1100),
                rows: Vec::new()
            }
        );
    }

    #[test]
    fn rows_stack_downwards_and_leave_one_at_a_time() {
        let mut run = Run::new(0);
        run.paid(10);
        run.wait(AFTER);
        run.paid(20);
        run.wait(AFTER);
        run.paid(30);
        let frame = run.wait(AFTER);
        let amounts = |f: &Frame| f.rows.iter().map(|r| r.amount).collect::<Vec<_>>();
        assert_eq!(amounts(&frame), [10, 20, 30]);
        assert_eq!(frame.rows.iter().map(|r| r.slot).collect::<Vec<_>>(), [0.0, 1.0, 2.0]);
        // the first leaves, the others stay where they are until it has landed
        let frame = run.wait(HOLD + AFTER * 4);
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
        let frame = run.wait(ON_TOP + FLY + STEP + ON_TOP + FLY + COUNT);
        assert_eq!(
            frame,
            Frame {
                balance: Some(60),
                rows: Vec::new()
            }
        );
    }

    #[test]
    fn a_purchase_meanwhile_lowers_the_number_at_once() {
        let mut run = Run::new(1000);
        run.paid(100);
        run.wait(AFTER);
        run.balance -= 300;
        let frame = run.wait(AFTER);
        assert_eq!(frame.balance, Some(700));
        let frame = run.wait(HOLD + FLY + COUNT + STEP);
        assert_eq!(
            frame,
            Frame {
                balance: Some(800),
                rows: Vec::new()
            }
        );
    }

    #[test]
    fn the_transaction_before_its_balance_does_not_dip_the_number() {
        let mut run = Run::new(1000);
        run.gains.push(100, run.now);
        assert_eq!(run.wait(AFTER * 3).balance, Some(1000));
        run.balance += 100;
        assert_eq!(run.wait(AFTER * 3).balance, Some(1000));
    }

    #[test]
    fn a_balance_that_never_came_ends_on_the_real_one() {
        let mut run = Run::new(1000);
        run.gains.push(100, run.now);
        let frame = run.wait(HOLD + FLY + COUNT + STEP);
        assert_eq!(
            frame,
            Frame {
                balance: Some(1000),
                rows: Vec::new()
            }
        );
        // and a later rise is not taken for it
        run.balance += 50;
        assert_eq!(run.wait(AFTER).balance, Some(1050));
    }

    /// The balance comes from the grid: any two values of it.
    #[test]
    fn extreme_balances_do_not_overflow() {
        let mut run = Run::new(-2_000_000_000);
        run.gains.push(100, run.now);
        run.balance = 2_000_000_000;
        assert_eq!(run.wait(AFTER).balance, Some(1_999_999_900));
        let frame = run.wait(HOLD + FLY + COUNT + STEP);
        assert_eq!(frame, Frame { balance: Some(2_000_000_000), rows: Vec::new() });
    }

    #[test]
    fn past_six_rows_the_last_one_adds_up() {
        let mut run = Run::new(0);
        for _ in 0..9 {
            run.paid(5);
        }
        let frame = run.wait(AFTER);
        assert_eq!(frame.rows.len(), MAX_ROWS);
        assert_eq!(frame.rows[MAX_ROWS - 1].amount, 20);
        let frame = run.wait((HOLD + ON_TOP + FLY + STEP) * 8);
        assert_eq!(
            frame,
            Frame {
                balance: Some(45),
                rows: Vec::new()
            }
        );
    }

    #[test]
    fn reduced_motion_fades_in_place_and_jumps_to_the_total() {
        let mut run = Run::new(1000);
        run.still = true;
        run.paid(100);
        run.paid(10);
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
    fn a_hidden_balance_has_no_rows_and_forgets_them() {
        let mut run = Run::new(1000);
        run.paid(100);
        run.wait(AFTER);
        let frame = run.gains.update(Some(run.balance), false, false, run.now);
        assert_eq!(
            frame,
            Frame {
                balance: Some(1100),
                rows: Vec::new()
            }
        );
        assert_eq!(
            run.wait(AFTER),
            Frame {
                balance: Some(1100),
                rows: Vec::new()
            }
        );
        let frame = run.gains.update(None, true, false, run.now);
        assert_eq!(
            frame,
            Frame {
                balance: None,
                rows: Vec::new()
            }
        );
    }
}
