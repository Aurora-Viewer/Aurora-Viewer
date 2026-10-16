//! Timeline of a teleport or a region crossing (AURORA_PROFILE): every
//! step from the request (or the CrossedRegion event) to the arrival, with
//! its time since the start, written as one `teleport timeline` or
//! `crossing timeline` line when it ends, fails or times out, with what the
//! link lost meanwhile. The session and the viewer add the steps they see
//! (messages, circuits, capabilities, the loading screen…); the
//! connections to the neighbour regions are written as they happen
//! (`region link` lines). Off without the variable: each step point reads
//! one flag. docs/TESTING.md#profiling lists the steps.

use crate::probe;
use crate::stats::NetStats;
use parking_lot::Mutex;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Teleport,
    Crossing,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Teleport => "teleport",
            Kind::Crossing => "crossing",
        }
    }

    /// Given up after this long without an end.
    fn deadline(self) -> Duration {
        match self {
            Kind::Teleport => Duration::from_secs(90),
            Kind::Crossing => Duration::from_secs(30),
        }
    }
}

/// The link counters a timeline reports the change of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkCounts {
    pub gaps: u64,
    pub late: u64,
    pub resent: u64,
    pub given_up: u64,
    pub duplicates: u64,
    pub eq_errors: u64,
}

impl LinkCounts {
    pub fn read(stats: &NetStats) -> Self {
        let s = stats.snapshot();
        LinkCounts {
            gaps: s.seq_gaps,
            late: s.seq_late,
            resent: s.resent,
            given_up: s.dropped_reliable,
            duplicates: s.duplicates,
            eq_errors: s.eq_errors,
        }
    }
}

/// One teleport or crossing under way.
#[derive(Debug)]
pub struct Timeline {
    kind: Kind,
    start: Instant,
    steps: Vec<(Duration, String)>,
    /// Steps that are only noted the first time (first object update…).
    once: Vec<&'static str>,
    at_start: LinkCounts,
    /// After the arrival, the timeline still collects what follows until
    /// then (corrections, first moves), and ends.
    settle_until: Option<Instant>,
}

impl Timeline {
    pub fn new(kind: Kind, first: String, at: Instant, link: LinkCounts) -> Self {
        Timeline {
            kind,
            start: at,
            steps: vec![(Duration::ZERO, first)],
            once: Vec::new(),
            at_start: link,
            settle_until: None,
        }
    }

    pub fn step(&mut self, at: Instant, text: String) {
        self.steps.push((at.saturating_duration_since(self.start), text));
    }

    /// False when that step was already noted.
    pub fn first(&mut self, key: &'static str) -> bool {
        if self.once.contains(&key) {
            return false;
        }
        self.once.push(key);
        true
    }

    /// The arrival: the timeline ends `settle` later at the latest.
    pub fn arrived(&mut self, at: Instant, settle: Duration) {
        self.settle_until.get_or_insert(at + settle);
    }

    /// Why it ends now, if it does: settled after its arrival, or too long.
    pub fn due(&self, now: Instant) -> Option<&'static str> {
        if self.settle_until.is_some_and(|t| now >= t) {
            Some("arrived")
        } else if now.saturating_duration_since(self.start) >= self.kind.deadline() {
            Some("timed out")
        } else {
            None
        }
    }

    /// The log line of the timeline ending at `at` for `outcome`.
    pub fn line(&self, outcome: &str, at: Instant, link: LinkCounts) -> String {
        let since = |a: u64, b: u64| a.saturating_sub(b);
        let s = &self.at_start;
        let lost = since(link.gaps, s.gaps).saturating_sub(since(link.late, s.late));
        let mut out = format!(
            "{} timeline: {outcome} after {:.0} ms",
            self.kind.name(),
            at.saturating_duration_since(self.start).as_secs_f32() * 1000.0
        );
        for (t, text) in &self.steps {
            let _ = write!(out, " | +{:.0} {text}", t.as_secs_f32() * 1000.0);
        }
        let _ = write!(
            out,
            " | link: lost={lost} gaps={} late={} resent={} given_up={} dup={} event_queue_errors={}",
            since(link.gaps, s.gaps),
            since(link.late, s.late),
            since(link.resent, s.resent),
            since(link.given_up, s.given_up),
            since(link.duplicates, s.duplicates),
            since(link.eq_errors, s.eq_errors),
        );
        out
    }
}

static CURRENT: Mutex<Option<Timeline>> = Mutex::new(None);
/// A timeline is under way (the steps of busy places, like the object
/// updates, stop at this flag otherwise).
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// A timeline is under way (and the probes are on).
#[inline]
pub fn running() -> bool {
    probe::enabled() && ACTIVE.load(Relaxed)
}

/// The kind of the timeline under way.
pub fn current() -> Option<Kind> {
    if !running() {
        return None;
    }
    CURRENT.lock().as_ref().map(|t| t.kind)
}

/// A teleport or crossing starts with `first`; one still under way ends
/// there (`interrupted`).
pub fn begin(kind: Kind, first: impl FnOnce() -> String, stats: &NetStats) {
    if !probe::enabled() {
        return;
    }
    let now = Instant::now();
    let link = LinkCounts::read(stats);
    let mut current = CURRENT.lock();
    if let Some(old) = current.take() {
        log::info!("{}", old.line("interrupted by the next one", now, link));
    }
    *current = Some(Timeline::new(kind, first(), now, link));
    ACTIVE.store(true, Relaxed);
}

/// A teleport starts unless one is already under way (TeleportStart of a
/// teleport the simulator began: a script, a god).
pub fn begin_teleport_if_none(first: impl FnOnce() -> String, stats: &NetStats) {
    if current() != Some(Kind::Teleport) {
        begin(Kind::Teleport, first, stats);
    }
}

/// A step of the timeline under way (nothing without one).
pub fn step(text: impl FnOnce() -> String) {
    if !running() {
        return;
    }
    if let Some(t) = CURRENT.lock().as_mut() {
        t.step(Instant::now(), text());
    }
}

/// A step noted only the first time its `key` comes in a timeline.
pub fn step_once(key: &'static str, text: impl FnOnce() -> String) {
    if !running() {
        return;
    }
    if let Some(t) = CURRENT.lock().as_mut()
        && t.first(key)
    {
        t.step(Instant::now(), text());
    }
}

/// [`step_once`] after the arrival only (what came before is the region
/// left behind).
pub fn step_once_arrived(key: &'static str, text: impl FnOnce() -> String) {
    if !running() {
        return;
    }
    if let Some(t) = CURRENT.lock().as_mut()
        && t.settle_until.is_some()
        && t.first(key)
    {
        t.step(Instant::now(), text());
    }
}

/// The arrival of the timeline under way: it ends `settle` from now at the
/// latest.
pub fn arrived(settle: Duration) {
    if !running() {
        return;
    }
    if let Some(t) = CURRENT.lock().as_mut() {
        t.arrived(Instant::now(), settle);
    }
}

/// The timeline under way ends now for `outcome` (only one of `kind`
/// when given).
pub fn end(kind: Option<Kind>, outcome: &str, stats: &NetStats) {
    if !running() {
        return;
    }
    let mut current = CURRENT.lock();
    if current.as_ref().is_some_and(|t| kind.is_none_or(|k| k == t.kind))
        && let Some(t) = current.take()
    {
        ACTIVE.store(false, Relaxed);
        log::info!("{}", t.line(outcome, Instant::now(), LinkCounts::read(stats)));
    }
}

/// The session's tick: a timeline settled after its arrival, or that took
/// too long, ends.
pub fn tick(stats: &NetStats) {
    if !running() {
        return;
    }
    let now = Instant::now();
    let mut current = CURRENT.lock();
    if let Some(why) = current.as_ref().and_then(|t| t.due(now))
        && let Some(t) = current.take()
    {
        ACTIVE.store(false, Relaxed);
        log::info!("{}", t.line(why, now, LinkCounts::read(stats)));
    }
}

/// A connection to a region changed (`region link` line, and a step of the
/// timeline under way).
pub fn link(text: impl FnOnce() -> String) {
    if !probe::enabled() {
        return;
    }
    let text = text();
    log::info!("region link: {text}");
    step(|| text);
}

/// A region handle as grid coordinates in regions (`1000,1003`).
pub fn grid(handle: u64) -> String {
    format!("{},{}", (handle >> 32) / 256, (handle & 0xFFFF_FFFF) / 256)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn a_teleport_line_lists_its_steps_and_what_the_link_lost() {
        let t0 = Instant::now();
        let start = LinkCounts {
            gaps: 10,
            late: 2,
            resent: 5,
            ..LinkCounts::default()
        };
        let mut t = Timeline::new(Kind::Teleport, "TeleportLocationRequest sent".into(), t0, start);
        t.step(t0 + ms(80), "TeleportStart".into());
        assert!(t.first("first object update"));
        t.step(t0 + ms(2400), "first object update".into());
        assert!(!t.first("first object update"));
        assert_eq!(t.due(t0 + ms(3000)), None);
        t.arrived(t0 + ms(2000), ms(5000));
        // a later arrival does not move the end
        t.arrived(t0 + ms(4000), ms(5000));
        assert_eq!(t.due(t0 + ms(6999)), None);
        assert_eq!(t.due(t0 + ms(7000)), Some("arrived"));
        let end = LinkCounts {
            gaps: 17,
            late: 3,
            resent: 9,
            given_up: 1,
            duplicates: 2,
            eq_errors: 1,
        };
        assert_eq!(
            t.line("arrived", t0 + ms(3100), end),
            "teleport timeline: arrived after 3100 ms | +0 TeleportLocationRequest sent | +80 TeleportStart \
             | +2400 first object update | link: lost=6 gaps=7 late=1 resent=4 given_up=1 dup=2 event_queue_errors=1"
        );
    }

    #[test]
    fn a_crossing_without_an_arrival_times_out() {
        let t0 = Instant::now();
        let t = Timeline::new(Kind::Crossing, "CrossedRegion".into(), t0, LinkCounts::default());
        assert_eq!(t.due(t0 + ms(29_000)), None);
        assert_eq!(t.due(t0 + ms(30_000)), Some("timed out"));
        assert!(
            t.line("timed out", t0 + ms(30_000), LinkCounts::default())
                .starts_with("crossing timeline: timed out after 30000 ms | +0 CrossedRegion |")
        );
    }

    #[test]
    fn handles_read_as_grid_coordinates() {
        assert_eq!(grid((256_000u64 << 32) | 256_768), "1000,1003");
    }
}
