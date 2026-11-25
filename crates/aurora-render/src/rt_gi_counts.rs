//! `AURORA_RT_GI_DIAG=count` (docs/TESTING.md), a diagnosis of the traced
//! indirect light's cost in a busy place: how many reads of the probes its
//! trace, its sky pass and the probes' own rays (rt_gi_probes.rs) make a
//! frame, in which cascade, how many of their probes the depth settles
//! (gi_sample.wgsl `gi_depth_sure`), how many weigh too little for a ray
//! (`gi_tiny`), how many visibility rays are traced, how long they are and
//! how many find the probe hidden, and how many candidate hits each ray
//! hands the shader, alpha-masked or not. The shader counts
//! (shaders/gi_count.wgsl, in those passes in the place of gi.rs
//! `PLAIN_HOOKS`); this reads the
//! counters back a few frames late, without waiting on the GPU, and logs
//! their mean a frame every [`REPORT`] frames. The counts cost time of their
//! own: the timings of such a run are not the viewer's.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The copies of each pass's counters (gi_count.wgsl `GI_COUNT_SLOTS`).
const SLOTS: u32 = 64;
/// The passes counted: the trace (`cs_gi`), the sky (`cs_gi_sky`), the
/// probes' rays (gi_trace.wgsl `cs_gi_trace`).
const PASSES: u32 = 3;
/// gi_sample.wgsl's `GI_NOTE_*` and gi_count.wgsl's.
const KINDS: u32 = 15;
const WORDS: u64 = (SLOTS * PASSES * KINDS) as u64;
const PASS_NAMES: [&str; PASSES as usize] = ["trace", "sky", "probes"];
/// The frames between two lines of the log.
const REPORT: u32 = 240;
const RING: usize = 3;

// The kinds, as gi_sample.wgsl and gi_count.wgsl number them.
const READ: usize = 0;
const ASKED: usize = 3;
const SURE: usize = 4;
const TINY: usize = 5;
const RAY: usize = 6;
const HIDDEN: usize = 9;
const CANDIDATE: usize = 10;
const MASKED: usize = 11;
/// The rays' length in centimetres, per cascade.
const LENGTH: usize = 12;

/// `GiCounts::ring` states, as the profile's `Timer`'s.
const FREE: u8 = 0;
const WRITTEN: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

/// The hooks rt_gi.wgsl's module takes in the place of gi.rs `PLAIN_HOOKS`.
pub(crate) fn hooks() -> String {
    format!("const GI_COUNT_SLOTS: u32 = {SLOTS}u;\nconst GI_COUNT_PASSES: u32 = {PASSES}u;\nconst GI_COUNT_KINDS: u32 = {KINDS}u;\n")
        + include_str!("shaders/gi_count.wgsl")
}

/// The counters (binding 24 of the trace's, the sky's and the probes' rays'
/// groups) and their read back.
pub(crate) struct GiCounts {
    pub(crate) buffer: wgpu::Buffer,
    ring: Vec<(wgpu::Buffer, Arc<AtomicU8>)>,
    next: usize,
    /// Each pass's kinds, summed over the frames read back.
    sums: Vec<u64>,
    samples: u32,
    frames: u32,
}

impl GiCounts {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt gi counts"),
            size: WORDS * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ring = (0..RING)
            .map(|_| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("rt gi counts readback"),
                    size: crate::readback::readback_size(WORDS * 4),
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (b, Arc::new(AtomicU8::new(FREE)))
            })
            .collect();
        log::warn!("rt gi counts: on (AURORA_RT_GI_DIAG=count, a diagnosis: the timings carry the counts' cost)");
        GiCounts {
            buffer,
            ring,
            next: 0,
            sums: vec![0; (PASSES * KINDS) as usize],
            samples: 0,
            frames: 0,
        }
    }

    /// Before a frame's passes: the counters of the frames before are mapped
    /// or read; every [`REPORT`] frames, the log's line.
    pub(crate) fn harvest(&mut self) -> Option<String> {
        for (buffer, state) in &self.ring {
            match state.load(Ordering::Acquire) {
                WRITTEN => {
                    state.store(MAPPING, Ordering::Release);
                    let state = state.clone();
                    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                        state.store(if r.is_ok() { MAPPED } else { FREE }, Ordering::Release);
                    });
                }
                MAPPED => {
                    if let Ok(data) = buffer.slice(..).get_mapped_range() {
                        let mut words = vec![0u32; WORDS as usize];
                        bytemuck::cast_slice_mut::<u32, u8>(&mut words).copy_from_slice(&data[..WORDS as usize * 4]);
                        add_slots(&mut self.sums, &words);
                        self.samples += 1;
                    }
                    buffer.unmap();
                    state.store(FREE, Ordering::Release);
                }
                _ => {}
            }
        }
        self.frames += 1;
        if self.frames < REPORT || self.samples == 0 {
            return None;
        }
        let text = line(&self.sums, self.samples);
        self.sums.iter_mut().for_each(|s| *s = 0);
        self.samples = 0;
        self.frames = 0;
        Some(text)
    }

    /// After a frame's passes: its counters to a free read back, then back
    /// to 0 for the next.
    pub(crate) fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let (readback, state) = &self.ring[self.next];
        if state.load(Ordering::Acquire) == FREE {
            encoder.copy_buffer_to_buffer(&self.buffer, 0, readback, 0, WORDS * 4);
            state.store(WRITTEN, Ordering::Release);
            self.next = (self.next + 1) % RING;
        }
        encoder.clear_buffer(&self.buffer, 0, None);
    }
}

/// Adds the slots' copies of each pass's kinds (`words`, as gi_count.wgsl
/// lays them out) into `sums`.
fn add_slots(sums: &mut [u64], words: &[u32]) {
    let per_slot = (PASSES * KINDS) as usize;
    for slot in words.chunks_exact(per_slot) {
        for (sum, &w) in sums.iter_mut().zip(slot) {
            *sum += u64::from(w);
        }
    }
}

/// The log's line: each pass's counts, a frame's mean over `samples` frames.
fn line(sums: &[u64], samples: u32) -> String {
    let k = f64::from(samples.max(1));
    let mut parts = Vec::new();
    for (p, name) in PASS_NAMES.iter().enumerate() {
        let at = |kind: usize| sums[p * KINDS as usize + kind] as f64 / k;
        let reads = at(READ) + at(READ + 1) + at(READ + 2);
        let asked = at(ASKED);
        let rays = at(RAY) + at(RAY + 1) + at(RAY + 2);
        let share = |n: f64, of: f64| if of > 0.0 { 100.0 * n / of } else { 0.0 };
        // a cascade's mean ray, in metres
        let long = |c: usize| {
            if at(RAY + c) > 0.0 {
                at(LENGTH + c) / at(RAY + c) / 100.0
            } else {
                0.0
            }
        };
        parts.push(format!(
            "{name}: reads {} (cascades {}/{}/{}), probes asked {} ({:.1} a read): depth sure {:.1}%, tiny {:.1}%, rays {:.1}% = {} (cascades {}/{}/{}, {:.2}/{:.2}/{:.2} m long), hidden {:.1}% of the rays, candidates {:.2} a ray, {:.1}% masked",
            amount(reads),
            amount(at(READ)),
            amount(at(READ + 1)),
            amount(at(READ + 2)),
            amount(asked),
            if reads > 0.0 { asked / reads } else { 0.0 },
            share(at(SURE), asked),
            share(at(TINY), asked),
            share(rays, asked),
            amount(rays),
            amount(at(RAY)),
            amount(at(RAY + 1)),
            amount(at(RAY + 2)),
            long(0),
            long(1),
            long(2),
            share(at(HIDDEN), rays),
            if rays > 0.0 { at(CANDIDATE) / rays } else { 0.0 },
            share(at(MASKED), at(CANDIDATE)),
        ));
    }
    format!("rt gi counts (a frame, mean of {samples} frames): {}", parts.join("; "))
}

/// A count for the log: 912, 41.3k, 2.07M.
fn amount(n: f64) -> String {
    if n >= 1e6 {
        format!("{:.2}M", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.1}k", n / 1e3)
    } else {
        format!("{n:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slots' copies add up per pass and kind, and the line gives a
    /// frame's mean with the shares of the probes asked and of the rays.
    #[test]
    fn counts_add_up_and_read() {
        let mut words = vec![0u32; WORDS as usize];
        let per_slot = (PASSES * KINDS) as usize;
        // two slots of the trace: 1000 reads of cascade 0, 8000 probes
        // asked, 2000 sure, 1000 tiny, 5000 rays (4000 + 1000, 1.5 m and
        // 3 m long), 500 hidden, 10000 candidates of which 2500 masked
        for slot in [0, 7] {
            let w = &mut words[slot * per_slot..(slot + 1) * per_slot];
            w[READ] = 500;
            w[ASKED] = 4000;
            w[SURE] = 1000;
            w[TINY] = 500;
            w[RAY] = 2000;
            w[RAY + 1] = 500;
            w[HIDDEN] = 250;
            w[CANDIDATE] = 5000;
            w[MASKED] = 1250;
            w[LENGTH] = 300_000;
            w[LENGTH + 1] = 150_000;
        }
        // the sky in the last slot, the probes' rays in the first
        let last = (SLOTS as usize - 1) * per_slot + KINDS as usize;
        words[last + READ + 2] = 30;
        words[2 * KINDS as usize + READ + 1] = 8;
        let mut sums = vec![0; per_slot];
        add_slots(&mut sums, &words);
        assert_eq!(sums[READ], 1000);
        assert_eq!(sums[RAY + 1], 1000);
        assert_eq!(sums[KINDS as usize + READ + 2], 30);
        // over two frames: half a frame's
        let text = line(&sums, 2);
        assert!(
            text.starts_with("rt gi counts (a frame, mean of 2 frames): trace: reads 500 (cascades 500/0/0)"),
            "{text}"
        );
        assert!(
            text.contains(
                "probes asked 4.0k (8.0 a read): depth sure 25.0%, tiny 12.5%, rays 62.5% = 2.5k (cascades 2.0k/500/0, 1.50/3.00/0.00 m long)"
            ),
            "{text}"
        );
        assert!(
            text.contains("hidden 10.0% of the rays, candidates 2.00 a ray, 25.0% masked"),
            "{text}"
        );
        assert!(text.contains("; sky: reads 15 (cascades 0/0/15)"), "{text}");
        assert!(text.contains("; probes: reads 4 (cascades 0/4/0)"), "{text}");
        assert_eq!(amount(2_070_000.0), "2.07M");
        assert_eq!(amount(912.0), "912");
    }

    /// The shader's constants are these.
    #[test]
    fn hooks_carry_the_layout() {
        let h = hooks();
        assert!(h.contains("const GI_COUNT_SLOTS: u32 = 64u;"));
        assert!(h.contains("const GI_COUNT_PASSES: u32 = 3u;"));
        assert!(h.contains("const GI_COUNT_KINDS: u32 = 15u;"));
        assert!(h.contains("const GI_NOTE_MASKED: u32 = 11u;"));
    }
}
