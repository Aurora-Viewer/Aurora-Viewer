//! How far the renderer's start has gone, readable from any thread: the
//! startup splash (in the viewer) shows it while `Renderer::new` holds the
//! main thread. Atomics only, so reading never waits on the renderer.
//!
//! The pipeline count is every render and compute pipeline made through
//! `pipeline_cache::CachedPipelines`; the total is the count of the last
//! start on this GPU, kept in the pipeline cache file (`pipeline_cache.rs`),
//! or [`DEFAULT_PIPELINES`] the first time.

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// Pipelines a start makes when no earlier start said (141 measured in the
/// demo, RTX 4090, 2026-11-01): only the first start of a GPU and build
/// uses it.
pub const DEFAULT_PIPELINES: u32 = 140;

/// The renderer's steps, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Phase {
    /// `Renderer::new` not called yet.
    Starting = 0,
    /// Vulkan (Metal) instance, adapters, device.
    GpuDevice = 1,
    /// Layouts, shaders and pipelines (`Backend::new`): the long part of a
    /// start whose shaders no cache holds.
    Pipelines = 2,
    /// `Renderer::new` returned.
    Ready = 3,
}

impl Phase {
    fn from_u8(v: u8) -> Phase {
        match v {
            1 => Phase::GpuDevice,
            2 => Phase::Pipelines,
            3 => Phase::Ready,
            _ => Phase::Starting,
        }
    }
}

/// One reading of the progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    /// Pipelines made since the process started.
    pub pipelines: u32,
    /// Pipelines this start is expected to make (never below `pipelines`).
    pub expected: u32,
}

/// The counters of one process (one instance, [`COUNTERS`]; the tests make
/// their own).
pub(crate) struct Counters {
    phase: AtomicU8,
    pipelines: AtomicU32,
    expected: AtomicU32,
    /// Pipelines made when the renderer was ready.
    at_ready: AtomicU32,
}

impl Counters {
    pub(crate) const fn new() -> Counters {
        Counters {
            phase: AtomicU8::new(Phase::Starting as u8),
            pipelines: AtomicU32::new(0),
            expected: AtomicU32::new(0),
            at_ready: AtomicU32::new(0),
        }
    }

    fn snapshot(&self) -> Snapshot {
        let pipelines = self.pipelines.load(Ordering::Relaxed);
        let expected = match self.expected.load(Ordering::Relaxed) {
            0 => DEFAULT_PIPELINES,
            n => n,
        };
        Snapshot {
            phase: Phase::from_u8(self.phase.load(Ordering::Acquire)),
            pipelines,
            expected: expected.max(pipelines),
        }
    }

    /// Moves to `phase`; never back (a later step already reached stays).
    fn enter(&self, phase: Phase) {
        if phase == Phase::Ready && self.phase.load(Ordering::Acquire) != Phase::Ready as u8 {
            self.at_ready.store(self.pipelines.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        self.phase.fetch_max(phase as u8, Ordering::AcqRel);
    }

    fn pipeline_made(&self) {
        self.pipelines.fetch_add(1, Ordering::Relaxed);
    }

    fn expect(&self, pipelines: u32) {
        self.expected.store(pipelines, Ordering::Relaxed);
    }

    fn made_at_ready(&self) -> Option<u32> {
        (self.phase.load(Ordering::Acquire) == Phase::Ready as u8).then(|| self.at_ready.load(Ordering::Relaxed))
    }
}

static COUNTERS: Counters = Counters::new();

/// The progress now.
pub fn snapshot() -> Snapshot {
    COUNTERS.snapshot()
}

/// Moves to `phase`; never back.
pub(crate) fn enter(phase: Phase) {
    COUNTERS.enter(phase);
}

pub(crate) fn pipeline_made() {
    COUNTERS.pipeline_made();
}

/// The total the last start of this GPU and build made.
pub(crate) fn expect(pipelines: u32) {
    COUNTERS.expect(pipelines);
}

/// Pipelines this start made before the renderer was ready, once it is
/// (`None` before): what the next start expects.
pub(crate) fn made_at_ready() -> Option<u32> {
    COUNTERS.made_at_ready()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_only_move_forward_and_the_total_covers_the_count() {
        let c = Counters::new();
        assert_eq!(c.snapshot().phase, Phase::Starting);
        assert_eq!(c.snapshot().expected, DEFAULT_PIPELINES);
        c.enter(Phase::Pipelines);
        c.enter(Phase::GpuDevice);
        assert_eq!(c.snapshot().phase, Phase::Pipelines);
        c.expect(2);
        for _ in 0..3 {
            c.pipeline_made();
        }
        let s = c.snapshot();
        assert_eq!((s.pipelines, s.expected), (3, 3));
        assert_eq!(c.made_at_ready(), None);
        c.enter(Phase::Ready);
        assert_eq!(c.made_at_ready(), Some(3));
        c.pipeline_made();
        c.enter(Phase::Ready);
        assert_eq!(c.made_at_ready(), Some(3), "later pipelines are not part of the start");
    }
}
