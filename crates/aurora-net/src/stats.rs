//! Lock-free network counters shared with the UI.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

/// Main simulator clock used by LLViewerObject-style motion extrapolation.
#[derive(Debug, Clone, Copy)]
pub struct SimulatorMotion {
    pub handle: Option<crate::RegionHandle>,
    pub time_dilation: f32,
    pub last_packet: Option<Instant>,
}

impl Default for SimulatorMotion {
    fn default() -> Self {
        Self {
            handle: None,
            time_dilation: 1.0,
            last_packet: None,
        }
    }
}

impl SimulatorMotion {
    pub fn packet_age(&self) -> f32 {
        self.last_packet.map_or(f32::INFINITY, |last| last.elapsed().as_secs_f32())
    }
}

#[derive(Default, Debug)]
pub struct NetStats {
    /// One coherent snapshot: neighbor packets must not keep the main circuit alive.
    pub simulator_motion: Mutex<SimulatorMotion>,
    /// The same for every simulator of the session, the agent's included:
    /// the objects of a region are extrapolated on its own clock.
    pub simulators: Mutex<Vec<SimulatorMotion>>,
    pub packets_in: AtomicU64,
    pub packets_out: AtomicU64,
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    pub resent: AtomicU64,
    pub dropped_reliable: AtomicU64,
    pub duplicates: AtomicU64,
    pub send_errors: AtomicU64,
    pub decode_errors: AtomicU64,
    pub ping_ms: AtomicU32,
    /// Smoothed round trip of the main circuit (what the resend delay uses).
    pub ping_avg_ms: AtomicU32,
    pub unacked: AtomicU32,
    pub http_in_flight: AtomicU32,
    pub http_queued: AtomicU32,
    /// What the agent's simulator was last asked to send at most (kb/s,
    /// throttle.rs; 0 before the first request).
    pub throttle_asked_kbps: AtomicU32,
    pub http_bytes: AtomicU64,
    pub http_completed: AtomicU64,
    pub http_failed: AtomicU64,
    /// Asset requests sent again after a retryable failure.
    pub http_retries: AtomicU64,
    /// Event queue polls the simulator held and that ended without an event
    /// (the normal case in a quiet place), and those counted as errors.
    pub eq_idle_polls: AtomicU64,
    pub eq_errors: AtomicU64,
    pub sim_count: AtomicU32,
    /// Objects the simulators announced by their cache entry
    /// (ObjectUpdateCached), found in the object cache or asked for in full.
    pub object_cache_hits: AtomicU64,
    pub object_cache_misses: AtomicU64,
    /// Objects received in full (ObjectUpdate, ObjectUpdateCompressed).
    pub objects_full: AtomicU64,
    /// Sequence numbers skipped by incoming packets, and packets that came
    /// after a later one (a resend, or out of order): the difference is what
    /// is still missing.
    pub seq_gaps: AtomicU64,
    pub seq_late: AtomicU64,
    pub agent_updates: AtomicU64,
    /// Zero-coded packets received, their received bytes and their bytes
    /// once decoded (LLMessageSystem's mCompressedPacketsIn,
    /// mCompressedBytesIn and mUncompressedBytesIn), and the packets that
    /// came from no circuit of ours (mOffCircuitPackets): for the viewer
    /// statistics (viewer_stats.rs).
    pub zero_coded_in: AtomicU64,
    pub zero_coded_bytes_in: AtomicU64,
    pub zero_decoded_bytes_in: AtomicU64,
    pub off_circuit: AtomicU64,
    /// Texture and mesh bytes the fetcher received (the statistics'
    /// TEXTURE_NETWORK_DATA_RECEIVED and LLMeshRepository::sBytesReceived).
    pub texture_bytes: AtomicU64,
    pub mesh_bytes: AtomicU64,
    /// Oldest change of the control flags not sent yet (`note_controls`).
    control_changed: Mutex<Option<Instant>>,
    /// Longest wait of a control change before its AgentUpdate, and longest
    /// time the session loop spent on one wake-up (microseconds); both are
    /// reset by `take_peaks`.
    input_delay_us: AtomicU32,
    loop_busy_us: AtomicU32,
}

/// Peaks since the previous `NetStats::take_peaks`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NetPeaks {
    /// None when no control change was sent.
    pub input_delay_ms: Option<f32>,
    pub loop_busy_ms: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NetStatsSnapshot {
    pub packets_in: u64,
    pub packets_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub resent: u64,
    pub dropped_reliable: u64,
    pub duplicates: u64,
    pub ping_ms: u32,
    pub ping_avg_ms: u32,
    pub unacked: u32,
    pub http_in_flight: u32,
    pub http_queued: u32,
    pub http_bytes: u64,
    pub http_completed: u64,
    pub http_failed: u64,
    pub http_retries: u64,
    pub eq_idle_polls: u64,
    pub eq_errors: u64,
    pub sim_count: u32,
    pub object_cache_hits: u64,
    pub object_cache_misses: u64,
    pub objects_full: u64,
    pub seq_gaps: u64,
    pub seq_late: u64,
    pub agent_updates: u64,
}

fn micros(d: std::time::Duration) -> u32 {
    d.as_micros().min(u128::from(u32::MAX)) as u32
}

impl NetStats {
    /// The viewer wrote new controls: remember when their flags changed (a
    /// key pressed or released), until an AgentUpdate carries them.
    pub fn note_controls(&self, flags_changed: bool) {
        if flags_changed {
            self.control_changed.lock().get_or_insert_with(Instant::now);
        }
    }

    /// No AgentUpdate is sent for now (teleport, logout): a change made
    /// meanwhile is not timed.
    pub fn forget_control_change(&self) {
        *self.control_changed.lock() = None;
    }

    /// An AgentUpdate left; `flag_change` when it carries control flags the
    /// simulator did not have, which ends the wait of the change being timed.
    pub fn agent_update_sent(&self, flag_change: bool) {
        self.agent_updates.fetch_add(1, Ordering::Relaxed);
        if flag_change && let Some(changed) = self.control_changed.lock().take() {
            // (+1: a delay below the microsecond still counts as one sent)
            self.input_delay_us.fetch_max(micros(changed.elapsed()) + 1, Ordering::Relaxed);
        }
    }

    /// The session loop handled one wake-up that started at `since`.
    pub fn loop_busy(&self, since: Instant) {
        self.loop_busy_us.fetch_max(micros(since.elapsed()), Ordering::Relaxed);
    }

    pub fn take_peaks(&self) -> NetPeaks {
        let input = self.input_delay_us.swap(0, Ordering::Relaxed);
        NetPeaks {
            input_delay_ms: (input != 0).then(|| (input - 1) as f32 / 1000.0),
            loop_busy_ms: self.loop_busy_us.swap(0, Ordering::Relaxed) as f32 / 1000.0,
        }
    }

    pub fn snapshot(&self) -> NetStatsSnapshot {
        let l = |a: &AtomicU64| a.load(Ordering::Relaxed);
        NetStatsSnapshot {
            packets_in: l(&self.packets_in),
            packets_out: l(&self.packets_out),
            bytes_in: l(&self.bytes_in),
            bytes_out: l(&self.bytes_out),
            resent: l(&self.resent),
            dropped_reliable: l(&self.dropped_reliable),
            duplicates: l(&self.duplicates),
            ping_ms: self.ping_ms.load(Ordering::Relaxed),
            ping_avg_ms: self.ping_avg_ms.load(Ordering::Relaxed),
            unacked: self.unacked.load(Ordering::Relaxed),
            http_in_flight: self.http_in_flight.load(Ordering::Relaxed),
            http_queued: self.http_queued.load(Ordering::Relaxed),
            http_bytes: l(&self.http_bytes),
            http_completed: l(&self.http_completed),
            http_failed: l(&self.http_failed),
            http_retries: l(&self.http_retries),
            eq_idle_polls: l(&self.eq_idle_polls),
            eq_errors: l(&self.eq_errors),
            sim_count: self.sim_count.load(Ordering::Relaxed),
            object_cache_hits: l(&self.object_cache_hits),
            object_cache_misses: l(&self.object_cache_misses),
            objects_full: l(&self.objects_full),
            seq_gaps: l(&self.seq_gaps),
            seq_late: l(&self.seq_late),
            agent_updates: l(&self.agent_updates),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn input_delay_runs_from_the_first_unsent_change_to_its_agent_update() {
        let stats = NetStats::default();
        // nothing changed: an update measures nothing
        stats.note_controls(false);
        stats.agent_update_sent(false);
        assert_eq!(stats.take_peaks().input_delay_ms, None);
        // a press, then a release before anything was sent: the press counts
        stats.note_controls(true);
        let pressed = (*stats.control_changed.lock()).expect("pending change");
        std::thread::sleep(Duration::from_millis(5));
        stats.note_controls(true);
        assert_eq!(*stats.control_changed.lock(), Some(pressed));
        // an update that still carries the old flags (keep-alive, camera)
        // leaves the change waiting
        stats.agent_update_sent(false);
        assert_eq!(*stats.control_changed.lock(), Some(pressed));
        stats.agent_update_sent(true);
        let delay = stats.take_peaks().input_delay_ms.expect("measured");
        assert!(delay >= 5.0, "{delay}");
        // taken: the next period starts empty, and the update count stays
        assert_eq!(stats.take_peaks(), NetPeaks::default());
        assert_eq!(stats.snapshot().agent_updates, 3);
    }

    #[test]
    fn loop_peak_keeps_the_longest_wake_up() {
        let stats = NetStats::default();
        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(3));
        stats.loop_busy(start);
        stats.loop_busy(Instant::now());
        assert!(stats.take_peaks().loop_busy_ms >= 3.0);
        assert_eq!(stats.take_peaks().loop_busy_ms, 0.0);
    }
}
