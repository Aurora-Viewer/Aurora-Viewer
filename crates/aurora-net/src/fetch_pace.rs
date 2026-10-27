//! How many asset requests the fetcher keeps under way at once
//! (`fetch.rs`): a limit that follows the link instead of a fixed number.
//!
//! The Legacy Viewer gives each kind of asset its own connections and keeps
//! up to 5 pipelined requests on each (`LLAppCoreHttp::init_data`,
//! `_httppolicy.cpp`), about 40 textures, 40 meshes and 60 other assets in
//! flight; Aurora's client does not pipeline (HTTP/1.1, one request per
//! connection) and had 24 for everything. In cold-cache arrivals at KB on
//! 2026-10-27, on a fast link, those 24 were the limit: about 450 requests
//! a second (24 over ~50 ms each) with up to 7,400 waiting while the link
//! was less than half used; 64 and 128 at once loaded the region in 50 and
//! 40 s instead of 80, with no refusal from the asset hosts. On a slow link
//! the opposite holds (2026-10-18, 75 Mb/s at night): more requests only
//! wait in the network, the first byte comes 3 to 4 times later. Hence a
//! limit that adapts, a deliberate difference from the Legacy Viewer's
//! fixed connections (only how many GETs are under way; the requests, their
//! order and their retries are unchanged):
//!
//! - it grows while requests wait for a slot (the limit was reached) and
//!   the asset hosts still answer about as fast as when idle;
//! - it shrinks when the time to the first byte doubles (requests waiting
//!   in the network, not at the host) and halves on a refusal (429, 503) or
//!   a transport error;
//! - never below the 24 of before, never above `MAX`.
//!
//! `AURORA_FETCH_SLOTS=<n>` fixes it for a launch, to compare.

use std::time::{Duration, Instant};

/// The fixed number Aurora used before, and the floor of the limit.
pub const MIN: usize = 24;
/// The ceiling: past 128 at once the arrivals at KB gained little.
pub const MAX: usize = 192;
/// A window ends after this many answers (at least) or this long.
const WINDOW_ANSWERS: usize = 16;
const WINDOW_TIME: Duration = Duration::from_secs(1);
/// Growth per window while saturated and fast: a quarter, 4 at least.
const GROW: f32 = 0.25;
/// Slow answers: the median time to the first byte past twice the
/// baseline, and past it by this much at least (a 20 ms baseline going to
/// 45 ms is still a fast host).
const SLOW_FACTOR: f32 = 2.0;
const SLOW_MARGIN_MS: f32 = 60.0;
const SHRINK_SLOW: f32 = 0.8;
const SHRINK_REFUSED: f32 = 0.5;
/// Growth only while the answers are within this of the baseline.
const FAST_FACTOR: f32 = 1.5;
/// The baseline follows a path that became slower by this much a window.
const BASELINE_DRIFT: f32 = 1.05;

/// What one answer tells the pace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Answer {
    /// Headers came after this long.
    Headers(Duration),
    /// Refused for load (429, 503) or no answer at all (transport error).
    Refused,
}

#[derive(Debug)]
pub struct Pace {
    limit: f32,
    min: f32,
    max: f32,
    /// Median time to the first byte when the hosts are not loaded, in ms.
    baseline: Option<f32>,
    window: Vec<f32>,
    refused: bool,
    /// The limit was reached with requests waiting during the window.
    saturated: bool,
    started: Instant,
}

impl Pace {
    /// Adaptive, or fixed at `fixed` (`AURORA_FETCH_SLOTS`).
    pub fn new(fixed: Option<usize>, now: Instant) -> Pace {
        let (min, max) = match fixed {
            Some(n) => (n, n),
            None => (MIN, MAX),
        };
        Pace {
            limit: min as f32,
            min: min as f32,
            max: max as f32,
            baseline: None,
            window: Vec::new(),
            refused: false,
            saturated: false,
            started: now,
        }
    }

    pub fn limit(&self) -> usize {
        self.limit as usize
    }

    /// The fetcher had requests waiting and no room under the limit.
    pub fn note_saturated(&mut self) {
        self.saturated = true;
    }

    /// An answer; true when the limit changed at the end of a window.
    pub fn answer(&mut self, a: Answer, now: Instant) -> bool {
        match a {
            Answer::Headers(t) => self.window.push(t.as_secs_f32() * 1000.0),
            Answer::Refused => self.refused = true,
        }
        let long = now.duration_since(self.started) >= WINDOW_TIME;
        let full = self.window.len() >= WINDOW_ANSWERS.max(self.limit() / 2);
        if (long && !self.window.is_empty()) || full || (self.refused && long) {
            return self.adjust(now);
        }
        false
    }

    fn adjust(&mut self, now: Instant) -> bool {
        let before = self.limit();
        let median = median(&mut self.window);
        if let Some(m) = median {
            self.baseline = Some(self.baseline.map_or(m, |b| (b * BASELINE_DRIFT).min(m)));
        }
        let baseline = self.baseline.unwrap_or(0.0);
        if self.refused {
            self.limit = (self.limit * SHRINK_REFUSED).max(self.min);
        } else if let Some(m) = median {
            if m > baseline * SLOW_FACTOR && m > baseline + SLOW_MARGIN_MS {
                self.limit = (self.limit * SHRINK_SLOW).max(self.min);
            } else if self.saturated && m <= baseline * FAST_FACTOR {
                self.limit = (self.limit + (self.limit * GROW).max(4.0)).min(self.max);
            }
        }
        self.window.clear();
        self.refused = false;
        self.saturated = false;
        self.started = now;
        self.limit() != before
    }

    /// For the log: the baseline in ms.
    pub fn baseline_ms(&self) -> Option<f32> {
        self.baseline
    }
}

fn median(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mid = v.len() / 2;
    let (_, m, _) = v.select_nth_unstable_by(mid, f32::total_cmp);
    Some(*m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Answer {
        Answer::Headers(Duration::from_millis(n))
    }

    /// One full window of answers of `t` ms, saturated or not.
    fn window(p: &mut Pace, now: &mut Instant, t: u64, saturated: bool) {
        if saturated {
            p.note_saturated();
        }
        for _ in 0..WINDOW_ANSWERS.max(p.limit() / 2) {
            *now += Duration::from_millis(1);
            p.answer(ms(t), *now);
        }
    }

    #[test]
    fn grows_while_saturated_and_fast_up_to_the_ceiling() {
        let mut now = Instant::now();
        let mut p = Pace::new(None, now);
        assert_eq!(p.limit(), MIN);
        window(&mut p, &mut now, 50, true);
        assert_eq!(p.limit(), 30);
        for _ in 0..40 {
            window(&mut p, &mut now, 55, true);
        }
        assert_eq!(p.limit(), MAX);
    }

    #[test]
    fn holds_when_nothing_waits() {
        let mut now = Instant::now();
        let mut p = Pace::new(None, now);
        for _ in 0..10 {
            window(&mut p, &mut now, 50, false);
        }
        assert_eq!(p.limit(), MIN);
    }

    #[test]
    fn shrinks_when_answers_slow_down_and_halves_on_a_refusal() {
        let mut now = Instant::now();
        let mut p = Pace::new(None, now);
        for _ in 0..10 {
            window(&mut p, &mut now, 50, true);
        }
        let grown = p.limit();
        assert!(grown > 80, "{grown}");
        // requests waiting in the network: 50 ms became 300 ms
        window(&mut p, &mut now, 300, true);
        assert_eq!(p.limit(), (grown as f32 * SHRINK_SLOW) as usize);
        let slowed = p.limit();
        // a 503
        p.answer(Answer::Refused, now);
        now += WINDOW_TIME;
        assert!(p.answer(ms(50), now));
        assert_eq!(p.limit(), ((slowed as f32 * SHRINK_REFUSED) as usize).max(MIN));
        // never below the floor
        for _ in 0..10 {
            p.answer(Answer::Refused, now);
            now += WINDOW_TIME;
            p.answer(ms(50), now);
        }
        assert_eq!(p.limit(), MIN);
    }

    #[test]
    fn a_fast_host_getting_a_little_slower_is_not_slow() {
        let mut now = Instant::now();
        let mut p = Pace::new(None, now);
        window(&mut p, &mut now, 20, true);
        let l = p.limit();
        // 20 → 45 ms: more than twice, but within the margin
        window(&mut p, &mut now, 45, true);
        assert!(p.limit() >= l);
    }

    #[test]
    fn fixed_never_moves() {
        let mut now = Instant::now();
        let mut p = Pace::new(Some(64), now);
        for _ in 0..10 {
            window(&mut p, &mut now, 50, true);
        }
        window(&mut p, &mut now, 900, true);
        assert_eq!(p.limit(), 64);
    }
}
