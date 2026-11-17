//! Voice link statistics, logged every [`STATS_PERIOD`] during a call: what
//! waits where between the network and the speakers, and between the
//! microphone and the network, plus what the RTCP reports say about both
//! streams. They tell a delay built up in the viewer (a queue that fills)
//! from one built up on the network or on the voice server (#2702).
//!
//! The receive and send tasks only bump relaxed atomics (a few nanoseconds
//! a packet, 50 packets a second each way); the session loop takes the
//! window and reads webrtc's statistics once per period. Nothing runs in
//! the audio device callbacks.
//!
//! webrtc 0.21 leaves two fields of its statistics empty: the RTCP round
//! trip of the media streams (`RTCRemoteInboundRtpStreamStats::round_trip_time`
//! is always 0) and the packets it drops from a full track queue
//! (`packets_discarded`). The ICE round trip stands in for the first; the
//! largest queue seen at once and the missing packets for the second. It
//! also counts only standalone receiver reports: a peer that sends media
//! too may carry its reports inside its sender reports, and then the
//! server's view of our stream stays "-" (seen in the loopback test).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use webrtc::peer_connection::{RTCStatsReport, RTCStatsReportEntry};

/// How often a stats line is written during a call.
pub(crate) const STATS_PERIOD: Duration = Duration::from_secs(5);

/// RTP clock of Opus.
const OPUS_CLOCK: i64 = 48_000;

/// Counters of one period, bumped by the receive and send tasks.
#[derive(Default)]
pub(crate) struct LinkStats {
    rx_packets: AtomicU32,
    rx_missing: AtomicU32,
    rx_queue_max: AtomicU32,
    rx_lag_max_us: AtomicU32,
    rx_lag_last_us: AtomicU32,
    sink_max_frames: AtomicU32,
    sink_last_frames: AtomicU32,
    tx_packets: AtomicU32,
    tx_wait_max_us: AtomicU32,
    tx_wait_total_us: AtomicU64,
    mic_backlog_max: AtomicU32,
}

fn clamp_u32(v: impl TryInto<u32>) -> u32 {
    v.try_into().unwrap_or(u32::MAX)
}

impl LinkStats {
    /// One received packet decoded, arriving `lag` later than the earliest
    /// arrival seen for its media time ([`ArrivalLag`]).
    pub(crate) fn rx_packet(&self, lag: Duration) {
        self.rx_packets.fetch_add(1, Ordering::Relaxed);
        let us = clamp_u32(lag.as_micros());
        self.rx_lag_max_us.fetch_max(us, Ordering::Relaxed);
        self.rx_lag_last_us.store(us, Ordering::Relaxed);
    }

    /// Packets that never reached the decoder (lost on the way or dropped by a
    /// full webrtc queue); up to five of them in a row are concealed.
    pub(crate) fn rx_missing(&self, packets: u32) {
        self.rx_missing.fetch_add(packets, Ordering::Relaxed);
    }

    /// Track events taken at one wake of the receive task (the webrtc
    /// queue's depth at that moment), and the voice sink's fill after them.
    pub(crate) fn rx_wake(&self, queued: u32, sink_frames: usize) {
        self.rx_queue_max.fetch_max(queued, Ordering::Relaxed);
        let frames = clamp_u32(sink_frames);
        self.sink_max_frames.fetch_max(frames, Ordering::Relaxed);
        self.sink_last_frames.store(frames, Ordering::Relaxed);
    }

    /// One packet handed to webrtc after waiting `wait` in `write_sample`
    /// (the driver's queue was full when it is more than a few µs).
    pub(crate) fn tx_packet(&self, wait: Duration) {
        self.tx_packets.fetch_add(1, Ordering::Relaxed);
        let us = clamp_u32(wait.as_micros());
        self.tx_wait_max_us.fetch_max(us, Ordering::Relaxed);
        self.tx_wait_total_us.fetch_add(u64::from(us), Ordering::Relaxed);
    }

    /// Microphone samples (48 kHz) waiting when the sender looked.
    pub(crate) fn mic_backlog(&self, samples: usize) {
        self.mic_backlog_max.fetch_max(clamp_u32(samples), Ordering::Relaxed);
    }

    /// The period's values; the counters start again from zero.
    pub(crate) fn take(&self) -> Window {
        let take = |a: &AtomicU32| a.swap(0, Ordering::Relaxed);
        Window {
            rx_packets: take(&self.rx_packets),
            rx_missing: take(&self.rx_missing),
            rx_queue_max: take(&self.rx_queue_max),
            rx_lag_max_us: take(&self.rx_lag_max_us),
            rx_lag_last_us: self.rx_lag_last_us.load(Ordering::Relaxed),
            sink_max_frames: take(&self.sink_max_frames),
            sink_last_frames: self.sink_last_frames.load(Ordering::Relaxed),
            tx_packets: take(&self.tx_packets),
            tx_wait_max_us: take(&self.tx_wait_max_us),
            tx_wait_total_us: self.tx_wait_total_us.swap(0, Ordering::Relaxed),
            mic_backlog_max: take(&self.mic_backlog_max),
        }
    }
}

/// One period of [`LinkStats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Window {
    pub rx_packets: u32,
    pub rx_missing: u32,
    pub rx_queue_max: u32,
    pub rx_lag_max_us: u32,
    pub rx_lag_last_us: u32,
    pub sink_max_frames: u32,
    pub sink_last_frames: u32,
    pub tx_packets: u32,
    pub tx_wait_max_us: u32,
    pub tx_wait_total_us: u64,
    pub mic_backlog_max: u32,
}

/// How much later than its best a packet arrives for its media time: the
/// arrival clock minus the RTP timestamp, over the smallest value seen. A
/// queue that fills between the server and the decoder (the network, the
/// webrtc track queue) makes it grow by the time the queue holds; it stays
/// flat when the server itself sends the sound late.
#[derive(Debug, Default)]
pub(crate) struct ArrivalLag {
    start: Option<Instant>,
    last_ts: u32,
    media: i64,
    best_us: i64,
}

impl ArrivalLag {
    /// A packet with RTP timestamp `ts` taken at `now`; returns its lag.
    pub(crate) fn packet(&mut self, now: Instant, ts: u32) -> Duration {
        let Some(start) = self.start else {
            *self = Self {
                start: Some(now),
                last_ts: ts,
                media: 0,
                best_us: 0,
            };
            return Duration::ZERO;
        };
        // Signed step: wraps around and slightly reordered packets.
        self.media += i64::from(ts.wrapping_sub(self.last_ts) as i32);
        self.last_ts = ts;
        let arrival_us = i64::try_from(now.saturating_duration_since(start).as_micros()).unwrap_or(i64::MAX);
        let lag = arrival_us - self.media * 1_000_000 / OPUS_CLOCK;
        self.best_us = self.best_us.min(lag);
        Duration::from_micros(u64::try_from(lag - self.best_us).unwrap_or(0))
    }
}

/// What the RTCP reports and ICE say, from webrtc's statistics.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct RtcpView {
    /// Jitter the server reports for our stream (ms), its loss fraction
    /// and how many receiver reports it sent; `None` before its first one.
    pub server_jitter_ms: Option<f64>,
    pub server_lost_fraction: f64,
    pub server_reports: u64,
    /// Jitter we measure on the server's stream (ms) and its lost packets.
    pub our_jitter_ms: Option<f64>,
    pub our_lost: i64,
    /// Round trip of the ICE checks on the nominated pair (ms).
    pub ice_rtt_ms: Option<f64>,
}

/// RTP jitter (timestamp units of the 48 kHz clock) in milliseconds.
fn jitter_ms(ticks: f64) -> f64 {
    ticks * 1000.0 / OPUS_CLOCK as f64
}

impl RtcpView {
    pub(crate) fn from_report(report: &RTCStatsReport) -> Self {
        let mut view = Self::default();
        for entry in report.iter() {
            match entry {
                RTCStatsReportEntry::RemoteInboundRtp(s) if s.round_trip_time_measurements > 0 => {
                    view.server_jitter_ms = Some(jitter_ms(s.received_rtp_stream_stats.jitter));
                    view.server_lost_fraction = s.fraction_lost;
                    view.server_reports = s.round_trip_time_measurements;
                }
                RTCStatsReportEntry::InboundRtp(s) if s.received_rtp_stream_stats.packets_received > 0 => {
                    view.our_jitter_ms = Some(jitter_ms(s.received_rtp_stream_stats.jitter));
                    view.our_lost = s.received_rtp_stream_stats.packets_lost;
                }
                RTCStatsReportEntry::IceCandidatePair(p) if p.nominated && p.current_round_trip_time > 0.0 => {
                    view.ice_rtt_ms = Some(p.current_round_trip_time * 1000.0);
                }
                _ => {}
            }
        }
        view
    }
}

fn ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

fn opt_ms(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:.1} ms"))
}

/// The log line of a period. `frames` are 48 kHz frames.
pub(crate) fn line(w: &Window, r: &RtcpView) -> String {
    let frames_ms = |f: u32| f64::from(f) / 48.0;
    format!(
        "voice stats: rx {} packets, {} missing, queue max {}, lag over best {:.0} ms (max {:.0}), sink {:.0} ms (max {:.0}) | \
         tx {} packets, write wait max {:.1} ms (total {:.1}), mic backlog max {:.0} ms | \
         server sees our jitter {}, loss {:.1}% ({} reports); we see its jitter {}, lost {}; ICE rtt {}",
        w.rx_packets,
        w.rx_missing,
        w.rx_queue_max,
        ms(u64::from(w.rx_lag_last_us)),
        ms(u64::from(w.rx_lag_max_us)),
        frames_ms(w.sink_last_frames),
        frames_ms(w.sink_max_frames),
        w.tx_packets,
        ms(u64::from(w.tx_wait_max_us)),
        ms(w.tx_wait_total_us),
        frames_ms(w.mic_backlog_max),
        opt_ms(r.server_jitter_ms),
        r.server_lost_fraction * 100.0,
        r.server_reports,
        opt_ms(r.our_jitter_ms),
        r.our_lost,
        opt_ms(r.ice_rtt_ms),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_is_taken_and_restarts() {
        let s = LinkStats::default();
        s.rx_packet(Duration::from_millis(3));
        s.rx_packet(Duration::from_millis(1));
        s.rx_missing(2);
        s.rx_wake(1, 2_880);
        s.rx_wake(4, 960);
        s.tx_packet(Duration::from_micros(10));
        s.tx_packet(Duration::from_micros(500));
        s.mic_backlog(1_920);
        let w = s.take();
        assert_eq!(
            w,
            Window {
                rx_packets: 2,
                rx_missing: 2,
                rx_queue_max: 4,
                rx_lag_max_us: 3_000,
                rx_lag_last_us: 1_000,
                sink_max_frames: 2_880,
                sink_last_frames: 960,
                tx_packets: 2,
                tx_wait_max_us: 500,
                tx_wait_total_us: 510,
                mic_backlog_max: 1_920,
            }
        );
        // Maxima and counts restart; the last values are kept.
        let w = s.take();
        assert_eq!((w.rx_packets, w.rx_queue_max, w.tx_wait_max_us), (0, 0, 0));
        assert_eq!((w.rx_lag_last_us, w.sink_last_frames), (1_000, 960));
    }

    #[test]
    fn arrival_lag_shows_a_queue_that_fills() {
        let t0 = Instant::now();
        let mut lag = ArrivalLag::default();
        let ts0 = u32::MAX - 960 * 10; // wraps during the test
        // 50 packets on time, with up to 5 ms of network jitter.
        for i in 0..50u32 {
            let jitter = Duration::from_millis(u64::from(i % 6));
            let got = lag.packet(t0 + Duration::from_millis(u64::from(20 * i)) + jitter, ts0.wrapping_add(960 * i));
            assert!(got <= Duration::from_millis(5), "{i}: {got:?}");
        }
        // A queue holding 5 s: each packet is taken 5 s after its time.
        for i in 50..100u32 {
            let got = lag.packet(t0 + Duration::from_millis(u64::from(20 * i) + 5_000), ts0.wrapping_add(960 * i));
            assert_eq!(got, Duration::from_secs(5));
        }
        // A packet earlier than all before becomes the new best.
        assert_eq!(
            lag.packet(t0 + Duration::from_millis(1_990), ts0.wrapping_add(960 * 100)),
            Duration::ZERO
        );
    }

    #[test]
    fn line_reads_and_marks_missing_reports() {
        let w = Window {
            rx_packets: 250,
            rx_missing: 1,
            rx_queue_max: 2,
            rx_lag_max_us: 40_000,
            rx_lag_last_us: 12_000,
            sink_max_frames: 4_320,
            sink_last_frames: 2_880,
            tx_packets: 250,
            tx_wait_max_us: 300,
            tx_wait_total_us: 2_000,
            mic_backlog_max: 960,
        };
        let r = RtcpView {
            server_jitter_ms: Some(jitter_ms(144.0)),
            server_lost_fraction: 0.0,
            server_reports: 5,
            our_jitter_ms: None,
            our_lost: 0,
            ice_rtt_ms: Some(34.0),
        };
        assert_eq!(
            line(&w, &r),
            "voice stats: rx 250 packets, 1 missing, queue max 2, lag over best 12 ms (max 40), sink 60 ms (max 90) | \
             tx 250 packets, write wait max 0.3 ms (total 2.0), mic backlog max 20 ms | \
             server sees our jitter 3.0 ms, loss 0.0% (5 reports); we see its jitter -, lost 0; ICE rtt 34.0 ms"
        );
    }
}
