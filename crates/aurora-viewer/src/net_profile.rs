//! Network profile: what the link and the session did over each second.
//!
//! Counted every second whether or not anything reads it: the Performance
//! window shows the last second ([`NetPeriod`]) and the totals of the session
//! ([`NetTotals`]). With `AURORA_PROFILE=1` each second of a grid session is
//! also one "net summary" line in the log, next to "perf summary"
//! (frame_profile.rs); docs/TESTING.md lists its fields.

use crate::agent::Corrections;
use aurora_net::{NetStats, NetStatsSnapshot};
use std::time::{Duration, Instant};

/// Time covered by one summary.
const PERIOD: Duration = Duration::from_secs(1);

/// One period of the session, as the log line prints it. Times are in
/// milliseconds, distances in metres, rates per second.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NetPeriod {
    /// Last and smoothed round trip to the agent's simulator.
    pub ping_ms: u32,
    pub ping_avg_ms: u32,
    pub packets_in: f32,
    pub packets_out: f32,
    pub kbps_in: f32,
    pub kbps_out: f32,
    /// Reliable packets sent again, and given up on.
    pub resent: u64,
    pub given_up: u64,
    pub duplicates: u64,
    pub unacked: u32,
    /// Sequence numbers skipped by the simulators, and packets that came
    /// after a later one: their difference is what was lost for good.
    pub gaps: u64,
    pub late: u64,
    /// Longest time the session loop spent on one wake-up, during which no
    /// circuit was read, acknowledged or answered.
    pub loop_busy_ms: f32,
    /// Events applied by the main thread, the most left waiting at the end
    /// of a frame, and the longest wait of an applied one.
    pub events: u32,
    pub events_left: usize,
    pub event_wait_ms: f32,
    pub agent_updates: u64,
    /// Longest wait of a key press or release before the AgentUpdate that
    /// carried it; None when no such change was sent.
    pub input_delay_ms: Option<f32>,
    pub corrections: Corrections,
    pub http_active: u32,
    pub http_queued: u32,
    pub http_done: u64,
    pub http_failed: u64,
    pub http_retries: u64,
    pub http_kbps: f32,
}

/// What the Performance window keeps beyond the last period.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NetTotals {
    /// The last input delay measured (periods without a key change keep it).
    pub input_delay_ms: Option<f32>,
    /// Corrections of our avatar since the login, and the largest one.
    pub corrections: Corrections,
}

pub struct NetProfile {
    log: bool,
    start: Instant,
    prev: Option<NetStatsSnapshot>,
    events: u32,
    events_left: usize,
    event_wait: Duration,
    corrections: Corrections,
    pub last: NetPeriod,
    pub totals: NetTotals,
}

impl Default for NetProfile {
    fn default() -> Self {
        NetProfile::new(std::env::var_os("AURORA_PROFILE").is_some())
    }
}

impl NetProfile {
    pub fn new(log: bool) -> Self {
        NetProfile {
            log,
            start: Instant::now(),
            prev: None,
            events: 0,
            events_left: 0,
            event_wait: Duration::ZERO,
            corrections: Corrections::default(),
            last: NetPeriod::default(),
            totals: NetTotals::default(),
        }
    }

    /// The main thread applied an event the network sent at `sent`.
    pub fn event_applied(&mut self, sent: Instant, now: Instant) {
        self.events += 1;
        self.event_wait = self.event_wait.max(now.saturating_duration_since(sent));
    }

    /// Events still in the channel once the frame stopped applying them.
    pub fn events_left(&mut self, left: usize) {
        self.events_left = self.events_left.max(left);
    }

    /// Corrections of our avatar noted by the world this frame.
    pub fn corrections(&mut self, c: Corrections) {
        self.corrections.count += c.count;
        self.corrections.largest = self.corrections.largest.max(c.largest);
    }

    /// A session ended: its totals do not carry over to the next login.
    pub fn reset(&mut self) {
        *self = NetProfile::new(self.log);
    }

    /// End of a frame: closes the period once a second and logs its line
    /// when asked to, during a session.
    pub fn end_frame(&mut self, stats: &NetStats) {
        if let Some(line) = self.end_frame_at(stats, Instant::now())
            && self.log
        {
            log::info!("net summary: {line}");
        }
    }

    fn end_frame_at(&mut self, stats: &NetStats, now: Instant) -> Option<String> {
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed < PERIOD {
            return None;
        }
        let snap = stats.snapshot();
        let peaks = stats.take_peaks();
        let prev = self.prev.replace(snap).unwrap_or(snap);
        let secs = elapsed.as_secs_f32();
        let rate = |now: u64, before: u64| now.saturating_sub(before) as f32 / secs;
        let kbps = |now: u64, before: u64| rate(now, before) * 8.0 / 1000.0;
        let since = |now: u64, before: u64| now.saturating_sub(before);
        self.last = NetPeriod {
            ping_ms: snap.ping_ms,
            ping_avg_ms: snap.ping_avg_ms,
            packets_in: rate(snap.packets_in, prev.packets_in),
            packets_out: rate(snap.packets_out, prev.packets_out),
            kbps_in: kbps(snap.bytes_in, prev.bytes_in),
            kbps_out: kbps(snap.bytes_out, prev.bytes_out),
            resent: since(snap.resent, prev.resent),
            given_up: since(snap.dropped_reliable, prev.dropped_reliable),
            duplicates: since(snap.duplicates, prev.duplicates),
            unacked: snap.unacked,
            gaps: since(snap.seq_gaps, prev.seq_gaps),
            late: since(snap.seq_late, prev.seq_late),
            loop_busy_ms: peaks.loop_busy_ms,
            events: self.events,
            events_left: self.events_left,
            event_wait_ms: self.event_wait.as_secs_f32() * 1000.0,
            agent_updates: since(snap.agent_updates, prev.agent_updates),
            input_delay_ms: peaks.input_delay_ms,
            corrections: self.corrections,
            http_active: snap.http_in_flight,
            http_queued: snap.http_queued,
            http_done: since(snap.http_completed, prev.http_completed),
            http_failed: since(snap.http_failed, prev.http_failed),
            http_retries: since(snap.http_retries, prev.http_retries),
            http_kbps: kbps(snap.http_bytes, prev.http_bytes),
        };
        if peaks.input_delay_ms.is_some() {
            self.totals.input_delay_ms = peaks.input_delay_ms;
        }
        self.totals.corrections.count += self.corrections.count;
        self.totals.corrections.largest = self.totals.corrections.largest.max(self.corrections.largest);
        self.start = now;
        self.events = 0;
        self.events_left = 0;
        self.event_wait = Duration::ZERO;
        self.corrections = Corrections::default();
        // (no circuit: the login screen, the offline demo)
        (snap.sim_count > 0).then(|| summary(&self.last))
    }
}

/// The log line of a period. `-` stands for "not measured in this period".
fn summary(p: &NetPeriod) -> String {
    let input = p.input_delay_ms.map_or_else(|| "-".to_string(), |ms| format!("{ms:.1}"));
    format!(
        "ping={} avg={} | udp in={:.0} out={:.0} pkt/s {:.0} / {:.0} kb/s | resent={} given_up={} dup={} unacked={} gaps={} late={} \
         | loop_max={:.2} | events={} left_max={} wait_max={:.2} | agent_updates={} input_delay_max={input} \
         | corrections={} largest={:.2} | http active={} queued={} done={} failed={} retries={} {:.0} kb/s",
        p.ping_ms,
        p.ping_avg_ms,
        p.packets_in,
        p.packets_out,
        p.kbps_in,
        p.kbps_out,
        p.resent,
        p.given_up,
        p.duplicates,
        p.unacked,
        p.gaps,
        p.late,
        p.loop_busy_ms,
        p.events,
        p.events_left,
        p.event_wait_ms,
        p.agent_updates,
        p.corrections.count,
        p.corrections.largest,
        p.http_active,
        p.http_queued,
        p.http_done,
        p.http_failed,
        p.http_retries,
        p.http_kbps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn in_session(stats: &NetStats) {
        stats.sim_count.store(1, Ordering::Relaxed);
    }

    #[test]
    fn one_line_a_second_with_what_happened_since_the_last() {
        let stats = NetStats::default();
        in_session(&stats);
        let mut p = NetProfile::new(true);
        let t = p.start;
        // the first period starts the counts
        assert!(p.end_frame_at(&stats, t + Duration::from_millis(500)).is_none());
        assert!(p.end_frame_at(&stats, t + PERIOD).is_some());

        stats.packets_in.fetch_add(400, Ordering::Relaxed);
        stats.bytes_in.fetch_add(250_000, Ordering::Relaxed);
        stats.resent.fetch_add(3, Ordering::Relaxed);
        stats.seq_gaps.fetch_add(5, Ordering::Relaxed);
        stats.seq_late.fetch_add(2, Ordering::Relaxed);
        stats.ping_ms.store(187, Ordering::Relaxed);
        stats.http_bytes.fetch_add(125_000, Ordering::Relaxed);
        stats.http_retries.fetch_add(1, Ordering::Relaxed);
        stats.agent_update_sent();
        p.event_applied(t, t + Duration::from_millis(12));
        p.event_applied(t, t + Duration::from_millis(4));
        p.events_left(7);
        p.events_left(2);
        p.corrections(Corrections { count: 1, largest: 2.5 });
        p.corrections(Corrections { count: 0, largest: 0.2 });
        // two seconds later: rates are per second, counts are for the period
        let line = p.end_frame_at(&stats, t + PERIOD * 3).expect("second period");
        let last = p.last;
        assert_eq!((last.packets_in, last.kbps_in, last.http_kbps), (200.0, 1000.0, 500.0));
        assert_eq!((last.resent, last.gaps, last.late, last.http_retries), (3, 5, 2, 1));
        assert_eq!((last.events, last.events_left), (2, 7));
        assert!((last.event_wait_ms - 12.0).abs() < 1e-3, "{}", last.event_wait_ms);
        assert_eq!(last.corrections, Corrections { count: 1, largest: 2.5 });
        assert_eq!(last.input_delay_ms, None);
        assert!(
            line.starts_with("ping=187 avg=0 | udp in=200 out=0 pkt/s 1000 / 0 kb/s | resent=3 "),
            "{line}"
        );
        assert!(
            line.contains("| events=2 left_max=7 wait_max=12.00 | agent_updates=1 input_delay_max=- |"),
            "{line}"
        );
        assert!(
            line.contains("| corrections=1 largest=2.50 | http active=0 queued=0 done=0 failed=0 retries=1 500 kb/s"),
            "{line}"
        );

        // the next period starts from nothing
        p.end_frame_at(&stats, t + PERIOD * 4).expect("third period");
        assert_eq!(
            (p.last.packets_in, p.last.resent, p.last.events, p.last.events_left),
            (0.0, 0, 0, 0)
        );
        assert_eq!(p.last.corrections, Corrections::default());
    }

    #[test]
    fn totals_keep_the_last_input_delay_and_every_correction() {
        let stats = NetStats::default();
        in_session(&stats);
        let mut p = NetProfile::new(false);
        let t = p.start;
        stats.note_controls(true);
        stats.agent_update_sent();
        p.corrections(Corrections { count: 2, largest: 1.5 });
        p.end_frame_at(&stats, t + PERIOD);
        let delay = p.totals.input_delay_ms.expect("measured");
        assert_eq!(p.last.input_delay_ms, Some(delay));
        // a second without a key change or a correction: the totals stay
        p.corrections(Corrections { count: 1, largest: 0.8 });
        p.end_frame_at(&stats, t + PERIOD * 2);
        assert_eq!(p.last.input_delay_ms, None);
        assert_eq!(p.totals.input_delay_ms, Some(delay));
        assert_eq!(p.totals.corrections, Corrections { count: 3, largest: 1.5 });
        // a new session starts them again
        p.reset();
        assert_eq!(p.totals, NetTotals::default());
    }

    #[test]
    fn nothing_is_logged_without_a_circuit() {
        let stats = NetStats::default();
        let mut p = NetProfile::new(true);
        let t = p.start;
        assert!(p.end_frame_at(&stats, t + PERIOD).is_none());
        in_session(&stats);
        assert!(p.end_frame_at(&stats, t + PERIOD * 2).is_some());
    }
}
