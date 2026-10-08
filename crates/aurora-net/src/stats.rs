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
    pub unacked: AtomicU32,
    pub http_in_flight: AtomicU32,
    pub http_queued: AtomicU32,
    pub http_bytes: AtomicU64,
    pub http_completed: AtomicU64,
    pub http_failed: AtomicU64,
    pub sim_count: AtomicU32,
    pub object_cache_hits: AtomicU64,
    pub object_cache_misses: AtomicU64,
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
    pub unacked: u32,
    pub http_in_flight: u32,
    pub http_queued: u32,
    pub http_bytes: u64,
    pub http_completed: u64,
    pub http_failed: u64,
    pub sim_count: u32,
    pub object_cache_hits: u64,
    pub object_cache_misses: u64,
}

impl NetStats {
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
            unacked: self.unacked.load(Ordering::Relaxed),
            http_in_flight: self.http_in_flight.load(Ordering::Relaxed),
            http_queued: self.http_queued.load(Ordering::Relaxed),
            http_bytes: l(&self.http_bytes),
            http_completed: l(&self.http_completed),
            http_failed: l(&self.http_failed),
            sim_count: self.sim_count.load(Ordering::Relaxed),
            object_cache_hits: l(&self.object_cache_hits),
            object_cache_misses: l(&self.object_cache_misses),
        }
    }
}
