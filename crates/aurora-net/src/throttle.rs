//! The bandwidth asked from the simulator (AgentThrottle): how much it may
//! send per second on each of its seven channels. Port of LLViewerThrottle
//! (indra/newview/llviewerthrottle.cpp, originally LGPL 2.1).
//!
//! The simulator never sends faster than this, so it decides how long an
//! arrival takes: the objects of a region, and what the avatars wear, come
//! on the Task channel. The setting is a total (`ThrottleBandwidthKBPS`)
//! shared between the channels along four presets, and the total really
//! asked follows the link: it starts at one and a half times the setting
//! and is lowered by a tenth of it every five seconds while packets are
//! lost or the viewer falls behind, raised again when all is well.
//!
//! One deliberate difference from Firestorm, the default setting: 4,000
//! instead of 3,000, that is 6,000 kb/s asked at first instead of 4,500,
//! the most its own code ever asks (`MAX_BANDWIDTH` of `getThrottleGroup`,
//! reached there through the debug setting). Measured on Agni on
//! 2026-10-13, arrivals in busy regions that were not in the cache, same
//! link: with the 800 kb/s for the objects asked until then a simulator
//! sent about 600 kb/s and the avatars stayed clouds for 35 to 43 s at the
//! median; with Firestorm's default, 1,531 kb/s and 22 s; with this one,
//! 2,475 kb/s and 10 s, with 0.07 % of the packets lost.

use std::time::{Duration, Instant};

/// Resend, Land, Wind, Cloud, Task, Texture, Asset (`LLViewerThrottle::sNames`).
pub const CHANNELS: usize = 7;
/// What a simulator may send per second on each channel, in kb/s.
pub type Group = [f32; CHANNELS];

/// The setting by default (`ThrottleBandwidthKBPS` is 3,000 in Firestorm's
/// settings.xml: see the head of this file), and the most the Preferences
/// and the `bw` command give it: beyond, nothing more is asked.
pub const DEFAULT_KBPS: f32 = 4000.0;
/// The range the setting takes (`LLViewerThrottle::getMaxBandwidthKbps`).
pub const SETTING_KBPS: std::ops::RangeInclusive<f32> = 100.0..=10000.0;

/// `BW_PRESET_50`, `_300`, `_500` and `_1000`: the share of each channel at
/// those totals, interpolated between two of them and extrapolated beyond
/// the last from the last two (which keeps some channels from growing as
/// fast as the others).
const PRESETS: [Group; 4] = [
    [5.0, 10.0, 3.0, 3.0, 10.0, 10.0, 9.0],
    [30.0, 40.0, 9.0, 9.0, 86.0, 86.0, 40.0],
    [50.0, 70.0, 14.0, 14.0, 136.0, 136.0, 80.0],
    [100.0, 100.0, 20.0, 20.0, 310.0, 310.0, 140.0],
];
/// The total a group is built for (`MIN_BANDWIDTH`, `MAX_BANDWIDTH` of
/// `getThrottleGroup`).
const GROUP_KBPS: std::ops::RangeInclusive<f32> = 50.0..=6000.0;

/// Share of the setting asked at first and at most, and at least; the step
/// of a change (`MAX_FRACTIONAL`, `MIN_FRACTIONAL`, `STEP_FRACTIONAL`).
const MAX_FRACTION: f32 = 1.5;
const MIN_FRACTION: f32 = 0.2;
const STEP: f32 = 0.1;
/// Packets lost, in percent, above which less is asked, and under which
/// more is (`TIGHTEN_THROTTLE_THRESHOLD`, `EASE_THROTTLE_THRESHOLD`).
const TIGHTEN_LOSS: f32 = 3.0;
const EASE_LOSS: f32 = 0.5;
/// Backlog of the viewer, as a share of [`BACKLOG`], from which less is
/// asked and under which more is (`HIGH_BUFFER_LOAD_TRESHOLD`,
/// `LOW_BUFFER_LOAD_TRESHOLD`).
const HIGH_LOAD: f32 = 1.0;
const LOW_LOAD: f32 = 0.8;
/// `DYNAMIC_UPDATE_DURATION`.
const UPDATE_EVERY: Duration = Duration::from_secs(5);
/// Packets waiting to be processed that count as a full load
/// (`DEFAULT_BUFFER_RING_SIZE`, llpacketring.cpp). Firestorm counts those
/// its frame has not read from the socket yet; here the socket is always
/// read, and they are the events the main thread has not applied.
pub const BACKLOG: usize = 256;

fn total(group: &Group) -> f32 {
    group.iter().sum()
}

/// The most a group is built for: `GROUP_KBPS`'s end, unless
/// `AURORA_THROTTLE_MAX_KBPS=<n>` (6,000 to 50,000) raises it for a launch,
/// to measure what a simulator sends when asked for more than the Legacy
/// Viewer ever asks (with `AURORA_BANDWIDTH_KBPS` for the setting).
fn group_max() -> f32 {
    static MAX: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *MAX.get_or_init(|| {
        let end = *GROUP_KBPS.end();
        match std::env::var("AURORA_THROTTLE_MAX_KBPS")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            Some(n) if n.is_finite() => {
                let n = n.clamp(end, 50_000.0);
                log::info!("simulator bandwidth: up to {n:.0} kb/s asked (AURORA_THROTTLE_MAX_KBPS)");
                n
            }
            _ => end,
        }
    })
}

/// The channels for a total of `kbps` (`LLViewerThrottle::getThrottleGroup`).
pub fn group(kbps: f32) -> Group {
    let kbps = kbps.clamp(*GROUP_KBPS.start(), group_max());
    let above = PRESETS.iter().position(|p| total(p) > kbps);
    let (from, to) = match above {
        // (under the lowest preset: that one)
        Some(0) => return PRESETS[0],
        Some(i) => (i - 1, i),
        None => (PRESETS.len() - 2, PRESETS.len() - 1),
    };
    // (between two presets `base` is the lower one; beyond the last it is
    // the last, extended by the step from the one before)
    let base = if above.is_some() { from } else { to };
    let step: Group = std::array::from_fn(|c| PRESETS[to][c] - PRESETS[from][c]);
    let share = (kbps - total(&PRESETS[base])) / total(&step);
    std::array::from_fn(|c| PRESETS[base][c] + step[c] * share)
}

/// The bandwidth asked from the agent's simulator over a session.
#[derive(Debug, Clone)]
pub struct Throttle {
    /// The setting, in kb/s.
    setting: f32,
    /// Share of it asked now.
    fraction: f32,
    updated: Instant,
    /// Highest backlog seen since the last update, as a share of
    /// [`BACKLOG`] (`setBufferLoadRate`).
    load: f32,
    /// The packet counters at the last update.
    counted: Counted,
}

/// Packets of all the circuits since the start: received, sequence numbers
/// skipped, and packets that came after a later one (what filled a gap).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counted {
    pub received: u64,
    pub gaps: u64,
    pub late: u64,
}

impl Throttle {
    /// `LLViewerThrottle::load`: one and a half times the setting at first.
    pub fn new(setting_kbps: f32, now: Instant) -> Throttle {
        Throttle {
            setting: setting_kbps.clamp(*SETTING_KBPS.start(), *SETTING_KBPS.end()),
            fraction: MAX_FRACTION,
            updated: now,
            load: 0.0,
            counted: Counted::default(),
        }
    }

    /// The same setting and what is asked now, counting from `counted`
    /// (the counters of a new session start again).
    pub fn counting_from(mut self, counted: Counted) -> Throttle {
        self.counted = counted;
        self
    }

    pub fn setting(&self) -> f32 {
        self.setting
    }

    /// The total asked now, in kb/s.
    pub fn asked(&self) -> f32 {
        self.setting * self.fraction
    }

    /// What AgentThrottle carries: bits per second for each channel (the
    /// simulator wants them so, `LLViewerThrottleGroup::sendToSim`).
    pub fn bits_per_second(&self) -> Group {
        group(self.asked()).map(|kbps| kbps * 1024.0)
    }

    /// The viewer has `waiting` packets it has not processed yet.
    pub fn note_backlog(&mut self, waiting: usize) {
        self.load = self.load.max(waiting as f32 / BACKLOG as f32);
    }

    /// `LLViewerThrottle::updateDynamicThrottle`, every five seconds:
    /// `lost` is the share of the packets lost since the last time, in
    /// percent. True when what is asked changed, and must be sent.
    pub fn update(&mut self, now: Instant, lost: f32) -> bool {
        if now.duration_since(self.updated) < UPDATE_EVERY {
            return false;
        }
        self.updated = now;
        let load = std::mem::take(&mut self.load);
        let before = self.fraction;
        if lost > TIGHTEN_LOSS || load >= HIGH_LOAD {
            // (never under `MIN_BANDWIDTH` either)
            if self.asked() > *GROUP_KBPS.start() {
                self.fraction = (self.fraction - STEP).max(MIN_FRACTION);
            }
        } else if lost <= EASE_LOSS && load < LOW_LOAD && self.asked() < group_max() {
            self.fraction = (self.fraction + STEP).min(MAX_FRACTION);
        }
        self.fraction != before
    }

    /// [`Throttle::update`] from the packet counters: the share lost is
    /// that of the packets since the last update. The share, in percent,
    /// when what is asked changed and must be sent.
    pub fn update_from(&mut self, now: Instant, counted: Counted) -> Option<f32> {
        if now.duration_since(self.updated) < UPDATE_EVERY {
            return None;
        }
        let before = std::mem::replace(&mut self.counted, counted);
        let gaps = counted.gaps.saturating_sub(before.gaps);
        let missing = gaps.saturating_sub(counted.late.saturating_sub(before.late));
        let lost = loss_percent(counted.received.saturating_sub(before.received), missing);
        self.update(now, lost).then_some(lost)
    }
}

/// Share of the packets lost, in percent: `missing` never arrived, out of
/// those that did and those.
pub fn loss_percent(received: u64, missing: u64) -> f32 {
    let sent = received + missing;
    if sent == 0 { 0.0 } else { missing as f32 * 100.0 / sent as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Group, b: &Group) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
    }

    #[test]
    fn channels_follow_the_presets() {
        // a preset itself, and the total of every group is what was asked
        assert!(close(&group(1000.0), &PRESETS[3]));
        assert!(close(&group(300.0), &PRESETS[1]));
        for kbps in [50.0, 120.0, 400.0, 750.0, 1000.0, 3000.0, 4500.0, 6000.0] {
            assert!((total(&group(kbps)) - kbps).abs() < 0.01, "{kbps}");
        }
        // between two presets
        assert!(close(&group(400.0), &[40.0, 55.0, 11.5, 11.5, 111.0, 111.0, 60.0]));
        // beyond the last: the step from 500 to 1,000, as many times as needed
        assert!(close(&group(4500.0), &[450.0, 310.0, 62.0, 62.0, 1528.0, 1528.0, 560.0]));
        // never under the lowest preset nor above 6,000
        assert!(close(&group(10.0), &PRESETS[0]));
        assert!(close(&group(20_000.0), &group(6000.0)));
    }

    #[test]
    fn a_session_starts_at_one_and_a_half_times_the_setting() {
        // Firestorm's default
        let t = Throttle::new(3000.0, Instant::now());
        assert_eq!(t.asked(), 4500.0);
        // the objects, in bits per second
        assert!((t.bits_per_second()[4] - 1528.0 * 1024.0).abs() < 1.0);
        // ours: the most a group is built for, nothing more beyond it
        let t = Throttle::new(DEFAULT_KBPS, Instant::now());
        assert_eq!(t.asked(), *GROUP_KBPS.end());
        assert!((t.bits_per_second()[4] - 2050.0 * 1024.0).abs() < 1.0);
        assert_eq!(Throttle::new(10000.0, Instant::now()).bits_per_second(), t.bits_per_second());
        // the setting stays within Firestorm's range
        assert_eq!(Throttle::new(20.0, Instant::now()).setting(), 100.0);
        assert_eq!(Throttle::new(1e9, Instant::now()).setting(), 10000.0);
    }

    #[test]
    fn losses_and_a_backlog_lower_what_is_asked_and_a_clean_link_raises_it() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut t = Throttle::new(1000.0, t0);
        // not before five seconds
        assert!(!t.update(at(4), 50.0));
        // losses: a tenth of the setting less, each time
        assert!(t.update(at(5), 3.5));
        assert!((t.asked() - 1400.0).abs() < 0.01);
        assert!(!t.update(at(6), 3.5));
        assert!(t.update(at(10), 3.5));
        assert!((t.asked() - 1300.0).abs() < 0.01);
        // between the two thresholds: nothing moves
        assert!(!t.update(at(15), 1.0));
        // the viewer behind, without a loss: less too, once per period
        t.note_backlog(BACKLOG);
        t.note_backlog(3);
        assert!(t.update(at(20), 0.0));
        assert!((t.asked() - 1200.0).abs() < 0.01);
        // a backlog that is not small keeps it from rising
        t.note_backlog(BACKLOG * 9 / 10);
        assert!(!t.update(at(25), 0.0));
        // all well: back up, a step at a time, to one and a half times
        for step in 0..3 {
            assert!(t.update(at(30 + step * 5), 0.2));
        }
        assert!((t.asked() - 1500.0).abs() < 0.01);
        assert!(!t.update(at(45), 0.0));
    }

    #[test]
    fn what_is_asked_stays_within_bounds() {
        let t0 = Instant::now();
        let mut t = Throttle::new(1000.0, t0);
        for step in 1..40 {
            t.update(t0 + UPDATE_EVERY * step, 100.0);
        }
        assert!((t.asked() - 200.0).abs() < 0.01, "a fifth of the setting at least");
        // the lowest setting never goes under the lowest group
        let mut low = Throttle::new(100.0, t0);
        for step in 1..40 {
            low.update(t0 + UPDATE_EVERY * step, 100.0);
        }
        assert!((low.asked() - 50.0).abs() < 0.01, "{}", low.asked());
        // the highest setting is not raised above the largest group
        let mut high = Throttle::new(10000.0, t0);
        high.update(t0 + UPDATE_EVERY, 100.0);
        assert!(!high.update(t0 + UPDATE_EVERY * 2, 0.0), "already above 6,000");
    }

    #[test]
    fn the_share_lost_is_that_of_the_packets_since_the_last_update() {
        let t0 = Instant::now();
        let counted = |received, gaps, late| Counted { received, gaps, late };
        let mut t = Throttle::new(1000.0, t0).counting_from(counted(5000, 40, 10));
        // too soon: nothing is counted
        assert_eq!(t.update_from(t0 + Duration::from_secs(1), counted(5100, 90, 10)), None);
        // 950 more received, 60 gaps of which 10 were filled late: 5 %
        let lost = t.update_from(t0 + UPDATE_EVERY, counted(5950, 100, 20));
        assert!(lost.is_some_and(|l| (l - 5.0).abs() < 0.01), "{lost:?}");
        assert!((t.asked() - 1400.0).abs() < 0.01);
        // nothing lost since: back up
        let lost = t.update_from(t0 + UPDATE_EVERY * 2, counted(9000, 100, 20));
        assert_eq!(lost, Some(0.0));
        // at the top again: nothing to send
        assert_eq!(t.update_from(t0 + UPDATE_EVERY * 3, counted(9900, 100, 20)), None);
    }

    #[test]
    fn loss_is_a_share_of_what_was_sent() {
        assert_eq!(loss_percent(0, 0), 0.0);
        assert_eq!(loss_percent(1000, 0), 0.0);
        assert_eq!(loss_percent(97, 3), 3.0);
        assert_eq!(loss_percent(0, 5), 100.0);
    }
}
