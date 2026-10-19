//! Deep profile (AURORA_PROFILE=1): beside the `perf summary` and `net
//! summary` lines of each second, every 10 s one line per area, so that a
//! long session can be ranked by what costs the most: `msg summary` (UDP
//! messages by type, object updates by kind), `http summary` (requests by
//! kind of asset and by host), `asset summary` (texture and mesh decodes,
//! their queues, GPU uploads, disk caches), `avatar summary` (how the
//! avatars are drawn, what they wear and play), `memory summary` (the
//! process and the GPU by kind). The `cpu summary` line comes from its own
//! thread (sys_probe.rs). Off without the variable: the counting points read
//! one flag (`aurora_net::probe::enabled`) and stop there. docs/TESTING.md
//! #profiling lists the fields.

use crate::scene::{AvatarCounts, AvatarSample};
use crate::sys_probe::ProcessMemory;
use aurora_net::probe::{self, Durations, Hist, HttpTotals, MessageCount};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

/// Time covered by one set of lines.
const PERIOD: Duration = Duration::from_secs(10);
/// The probes are read this often (the resource monitor's second).
const SECOND: Duration = Duration::from_secs(1);
/// Message types listed in each direction.
const TOP_MESSAGES: usize = 15;
/// Hosts listed in `http summary`.
const TOP_HOSTS: usize = 5;

/// Durations of the background jobs, counted by the jobs themselves.
struct AssetProbes {
    /// A texture decode job, from its JPEG 2000 data to its staged chain.
    texture: Hist,
    /// The BC7 encoding of the decodes that tried it (part of `texture`).
    bc7: Hist,
    /// A mesh level of detail decoded and turned into GPU geometry.
    mesh_lod: Hist,
    /// The header and skin of downloaded or cached mesh bytes.
    mesh_parse: Hist,
    mesh_cache: [AtomicU64; 2],
}

static ASSETS: AssetProbes = AssetProbes {
    texture: Hist::new(),
    bc7: Hist::new(),
    mesh_lod: Hist::new(),
    mesh_parse: Hist::new(),
    mesh_cache: [AtomicU64::new(0), AtomicU64::new(0)],
};

/// The start of a timed job, when the probes are on.
pub fn timer() -> Option<Instant> {
    probe::enabled().then(Instant::now)
}

/// A texture decode job that started at `start` ended; `bc7` is the time
/// its BC7 encoding took when it tried one.
pub fn texture_decoded(start: Option<Instant>, bc7: Option<Duration>) {
    if let Some(start) = start {
        ASSETS.texture.add(start.elapsed());
        if let Some(bc7) = bc7 {
            ASSETS.bc7.add(bc7);
        }
    }
}

/// A mesh level of detail built from `start`.
pub fn mesh_built(start: Option<Instant>) {
    if let Some(start) = start {
        ASSETS.mesh_lod.add(start.elapsed());
    }
}

/// Mesh bytes parsed from `start`.
pub fn mesh_parsed(start: Option<Instant>) {
    if let Some(start) = start {
        ASSETS.mesh_parse.add(start.elapsed());
    }
}

/// A mesh's disk cache file was found (`hit`) or not.
pub fn mesh_cache_read(hit: bool) {
    if probe::enabled() {
        ASSETS.mesh_cache[usize::from(!hit)].fetch_add(1, Relaxed);
    }
}

/// What the main thread knows of each frame.
#[derive(Debug, Default, Clone, Copy)]
pub struct FrameSample {
    /// Texture bytes sent to the GPU this frame.
    pub texture_upload_bytes: u64,
    /// Textures waiting for a decode or being decoded, and decoded ones
    /// waiting for their upload.
    pub decoding: usize,
    pub pending_upload: usize,
    /// Background jobs queued or running, and geometries being built.
    pub jobs: usize,
    pub geom_pending: usize,
    pub meshes_fetching: usize,
    pub avatars: AvatarCounts,
    /// Skeletons posed this frame and those built again.
    pub posed: usize,
    pub skeletons_rebuilt: usize,
}

/// GPU memory at the end of a period (bytes).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GpuMemory {
    /// What the system lets the process use and what the driver counts for
    /// it, and its live textures and buffers (VK_EXT_memory_budget; None
    /// when the driver does not say).
    pub video: Option<aurora_render::vram::VideoMemory>,
    pub textures: u32,
    pub texture_bytes: u64,
    pub pages: u32,
    pub page_bytes: u64,
    pub geometry_bytes: u64,
    /// Geometry arena in use: vertices and indices.
    pub geometry_used: u64,
    /// What the texture streamer keeps the pages under, and its pressure
    /// step (texture_budget.rs).
    pub texture_target: u64,
    pub pressure: u8,
}

/// What the main thread reads at the end of a period only.
#[derive(Debug, Default, Clone, Copy)]
pub struct PeriodSample {
    pub avatars: AvatarSample,
    pub gpu: GpuMemory,
    pub process: Option<ProcessMemory>,
    /// Texture disk cache since the start: JPEG 2000 reads found, missed,
    /// and textures read back as BC7 without a decode.
    pub texture_cache: [u64; 3],
    /// Geometry bytes written to the GPU since the start.
    pub geometry_uploaded: u64,
    /// Asset requests waiting for a slot now.
    pub http_queued: u32,
}

/// Sums of the frames of a period.
#[derive(Debug, Default, Clone, Copy)]
struct Frames {
    count: u32,
    texture_upload_bytes: u64,
    decoding: (u64, usize),
    pending_upload: (u64, usize),
    jobs: (u64, usize),
    geom_pending: usize,
    meshes_fetching: usize,
    avatars_total_max: usize,
    pictured: u64,
    posed: (u64, usize),
    skeletons_rebuilt: (u64, usize),
    last: AvatarCounts,
}

/// Totals since the start, kept to report the period's share.
#[derive(Debug, Default, Clone, Copy)]
struct Before {
    texture_cache: [u64; 3],
    geometry_uploaded: u64,
    page_faults: u32,
}

/// What the probes counted over a span: the network's messages and
/// requests, the background jobs' durations.
#[derive(Debug, Default, Clone)]
pub struct ProbeTotals {
    pub messages: [Vec<MessageCount>; 2],
    pub objects: [u64; 8],
    pub http: HttpTotals,
    texture: Durations,
    bc7: Durations,
    mesh_lod: Durations,
    mesh_parse: Durations,
    /// Mesh disk cache reads found, missed.
    pub mesh_cache: [u64; 2],
    /// Length of the span (s).
    pub secs: f32,
}

impl ProbeTotals {
    /// What the probes counted since the last take; they start again from
    /// nothing.
    fn take(secs: f32) -> ProbeTotals {
        ProbeTotals {
            messages: probe::take_messages(),
            objects: probe::take_objects(),
            http: probe::take_http(),
            texture: ASSETS.texture.take(),
            bc7: ASSETS.bc7.take(),
            mesh_lod: ASSETS.mesh_lod.take(),
            mesh_parse: ASSETS.mesh_parse.take(),
            mesh_cache: [ASSETS.mesh_cache[0].swap(0, Relaxed), ASSETS.mesh_cache[1].swap(0, Relaxed)],
            secs,
        }
    }

    /// Adds a later span.
    fn merge(&mut self, later: &ProbeTotals) {
        probe::merge_messages(&mut self.messages, &later.messages);
        for (a, b) in self.objects.iter_mut().zip(later.objects) {
            *a += b;
        }
        self.http.merge(&later.http);
        self.texture.merge(&later.texture);
        self.bc7.merge(&later.bc7);
        self.mesh_lod.merge(&later.mesh_lod);
        self.mesh_parse.merge(&later.mesh_parse);
        self.mesh_cache[0] += later.mesh_cache[0];
        self.mesh_cache[1] += later.mesh_cache[1];
        self.secs += later.secs;
    }

    /// Texture decodes of the span: (count, median ms).
    pub fn texture_decodes(&self) -> (u64, f32) {
        (self.texture.count(), self.texture.percentile(50.0))
    }

    /// Mesh levels of detail built over the span: (count, median ms).
    pub fn mesh_builds(&self) -> (u64, f32) {
        (self.mesh_lod.count(), self.mesh_lod.percentile(50.0))
    }
}

pub struct DeepProfile {
    enabled: bool,
    /// The probes count for the resource monitor, without a log line.
    live: bool,
    start: Option<Instant>,
    frames: Frames,
    before: Option<Before>,
    /// Start of the second being counted.
    second: Option<Instant>,
    /// The probes of the 10 s period so far (with AURORA_PROFILE).
    probes: ProbeTotals,
    /// The probes of the last second, and its number.
    last_second: Option<ProbeTotals>,
    serial: u64,
}

impl Default for DeepProfile {
    fn default() -> Self {
        DeepProfile::new(std::env::var_os("AURORA_PROFILE").is_some())
    }
}

impl DeepProfile {
    /// Turns the probes on and starts the CPU thread when `enabled`; call
    /// it from the main thread (its id names the `main` group).
    pub fn new(enabled: bool) -> Self {
        if enabled {
            probe::enable();
            crate::sys_probe::start(crate::sys_probe::current_thread_id());
        }
        DeepProfile {
            enabled,
            live: false,
            start: None,
            frames: Frames::default(),
            before: None,
            second: None,
            probes: ProbeTotals::default(),
            last_second: None,
            serial: 0,
        }
    }

    /// The probes count while the resource monitor is open (they are on
    /// for good with AURORA_PROFILE); nothing more is logged.
    pub fn set_live(&mut self, live: bool) {
        if live == self.live {
            return;
        }
        self.live = live;
        if !self.enabled {
            probe::set_enabled(live);
            self.second = None;
            self.last_second = None;
        }
    }

    /// Reads the probes once a second, while they count; true when the
    /// main thread should call [`DeepProfile::end_frame`] too (logging).
    pub fn tick(&mut self) -> bool {
        if !self.enabled && !self.live {
            return false;
        }
        let now = Instant::now();
        let second = *self.second.get_or_insert(now);
        let elapsed = now.duration_since(second);
        if elapsed >= SECOND {
            self.second = Some(now);
            let taken = ProbeTotals::take(elapsed.as_secs_f32());
            if self.enabled {
                self.probes.merge(&taken);
            }
            self.last_second = Some(taken);
            self.serial += 1;
        }
        self.enabled
    }

    /// What the probes counted over the last second, with the second's
    /// number (None while they are off or before the first second).
    pub fn last_second(&self) -> Option<(u64, &ProbeTotals)> {
        self.last_second.as_ref().map(|p| (self.serial, p))
    }

    /// End of a frame: adds it to the period, and every 10 s writes the
    /// lines, reading what only the end of a period needs from `period`.
    pub fn end_frame(&mut self, frame: FrameSample, period: impl FnOnce() -> PeriodSample) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let start = *self.start.get_or_insert(now);
        self.add(&frame);
        let elapsed = now.duration_since(start);
        if elapsed < PERIOD {
            return;
        }
        self.start = Some(now);
        let sample = period();
        let frames = std::mem::take(&mut self.frames);
        let secs = elapsed.as_secs_f32();
        let before = self.before.replace(Before {
            texture_cache: sample.texture_cache,
            geometry_uploaded: sample.geometry_uploaded,
            page_faults: sample.process.map_or(0, |p| p.page_faults),
        });
        // the part of a second not read yet belongs to this period
        let rest = ProbeTotals::take(self.second.map_or(0.0, |s| now.duration_since(s).as_secs_f32()));
        self.second = Some(now);
        let mut p = std::mem::take(&mut self.probes);
        p.merge(&rest);
        log::info!("msg summary: {}", probe::message_line(&p.messages, &p.objects, secs, TOP_MESSAGES));
        log::info!("http summary: {}", probe::http_line(&p.http, sample.http_queued, secs, TOP_HOSTS));
        let jobs = JobTimes {
            texture: p.texture,
            bc7: p.bc7,
            mesh_lod: p.mesh_lod,
            mesh_parse: p.mesh_parse,
            mesh_cache: p.mesh_cache,
        };
        let before = before.unwrap_or(Before {
            texture_cache: sample.texture_cache,
            geometry_uploaded: sample.geometry_uploaded,
            page_faults: sample.process.map_or(0, |p| p.page_faults),
        });
        log::info!("asset summary: {}", asset_line(&frames, &jobs, &sample, &before, secs));
        log::info!("avatar summary: {}", avatar_line(&frames, &sample.avatars, secs));
        log::info!("memory summary: {}", memory_line(&sample, &before, secs));
    }

    fn add(&mut self, f: &FrameSample) {
        let s = &mut self.frames;
        s.count += 1;
        s.texture_upload_bytes += f.texture_upload_bytes;
        let sum_max = |acc: &mut (u64, usize), v: usize| {
            acc.0 += v as u64;
            acc.1 = acc.1.max(v);
        };
        sum_max(&mut s.decoding, f.decoding);
        sum_max(&mut s.pending_upload, f.pending_upload);
        sum_max(&mut s.jobs, f.jobs);
        sum_max(&mut s.posed, f.posed);
        sum_max(&mut s.skeletons_rebuilt, f.skeletons_rebuilt);
        s.geom_pending = s.geom_pending.max(f.geom_pending);
        s.meshes_fetching = s.meshes_fetching.max(f.meshes_fetching);
        s.avatars_total_max = s.avatars_total_max.max(f.avatars.total);
        s.pictured += f.avatars.pictured as u64;
        s.last = f.avatars;
    }
}

/// The jobs' durations over a period.
struct JobTimes {
    texture: Durations,
    bc7: Durations,
    mesh_lod: Durations,
    mesh_parse: Durations,
    mesh_cache: [u64; 2],
}

fn avg((sum, _): (u64, usize), frames: u32) -> f32 {
    sum as f32 / frames.max(1) as f32
}

const MB: f32 = 1024.0 * 1024.0;

/// `asset summary`: decodes (ms), queues (average and most over the
/// frames), disk caches, uploads to the GPU (MB/s).
fn asset_line(f: &Frames, j: &JobTimes, s: &PeriodSample, before: &Before, secs: f32) -> String {
    let secs = secs.max(1e-3);
    let cache: [u64; 3] = std::array::from_fn(|i| s.texture_cache[i].saturating_sub(before.texture_cache[i]));
    format!(
        "texture_decode {} | bc7 {} | decoding avg={:.1} max={} upload_wait avg={:.1} max={} | j2k_cache hit={} miss={} bc7_cache_hit={} \
         | mesh_decode {} | mesh_parse {} | mesh_cache hit={} miss={} meshes_fetching_max={} | jobs avg={:.1} max={} geom_pending_max={} \
         | gpu_upload textures={:.2} MB/s geometry={:.2} MB/s",
        j.texture.describe(),
        j.bc7.describe(),
        avg(f.decoding, f.count),
        f.decoding.1,
        avg(f.pending_upload, f.count),
        f.pending_upload.1,
        cache[0],
        cache[1],
        cache[2],
        j.mesh_lod.describe(),
        j.mesh_parse.describe(),
        j.mesh_cache[0],
        j.mesh_cache[1],
        f.meshes_fetching,
        avg(f.jobs, f.count),
        f.jobs.1,
        f.geom_pending,
        f.texture_upload_bytes as f32 / MB / secs,
        s.geometry_uploaded.saturating_sub(before.geometry_uploaded) as f32 / MB / secs,
    )
}

/// `avatar summary`: how the avatars of the last frame are drawn, what they
/// wear (triangles in thousands) and play, and the skeletons posed and
/// built again per frame.
fn avatar_line(f: &Frames, a: &AvatarSample, secs: f32) -> String {
    let c = &f.last;
    let mut out = format!(
        "total={} max={} full={} full_in_view={} impostors={} impostors_in_view={} pictures/s={:.1} silhouettes={} loading={}",
        c.total,
        f.avatars_total_max,
        c.full,
        c.full_in_view,
        c.impostors,
        c.impostors_in_view,
        f.pictured as f32 / secs.max(1e-3),
        c.silhouettes,
        c.loading,
    );
    let _ = write!(
        out,
        " | complexity total={} max={} avg={:.0} known={} | attachments={} full={} rigged={} rigged_tris_k={:.0} full={:.0} worn_tris_k_full={:.0} \
         | animations={} max={} | posed avg={:.1} max={} skeletons_rebuilt avg={:.2} max={}",
        a.complexity_total,
        a.complexity_max,
        a.complexity_total as f32 / a.complexity_known.max(1) as f32,
        a.complexity_known,
        a.attachments,
        a.attachments_full,
        a.rigged,
        a.rigged_triangles as f32 / 1000.0,
        a.rigged_triangles_full as f32 / 1000.0,
        a.worn_triangles_full as f32 / 1000.0,
        a.animations,
        a.animations_max,
        avg(f.posed, f.count),
        f.posed.1,
        avg(f.skeletons_rebuilt, f.count),
        f.skeletons_rebuilt.1,
    );
    out
}

/// `memory summary` (MB): the process, then the GPU by kind; `other` is
/// what the process holds on the GPU beside the texture pages and the
/// geometry (render targets, shadow maps, tables…).
fn memory_line(s: &PeriodSample, before: &Before, secs: f32) -> String {
    let mb = |b: u64| b as f32 / MB;
    let mut out = match s.process {
        Some(p) => format!(
            "process working_set={:.0} peak={:.0} private={:.0} page_faults/s={:.0}",
            mb(p.working_set),
            mb(p.peak_working_set),
            mb(p.private),
            p.page_faults.wrapping_sub(before.page_faults) as f32 / secs.max(1e-3)
        ),
        None => "process -".to_string(),
    };
    let g = &s.gpu;
    match g.video {
        Some(v) => {
            let _ = write!(
                out,
                " | gpu budget={:.0} usage={:.0} allocated={:.0} other={:.0}",
                mb(v.budget),
                mb(v.usage),
                mb(v.allocated),
                mb(v.allocated.saturating_sub(g.page_bytes + g.geometry_bytes))
            );
        }
        None => out.push_str(" | gpu budget=-"),
    }
    let _ = write!(
        out,
        " | textures={} texture_mb={:.0} pages={} pages_mb={:.0} texture_target_mb={:.0} pressure={} | geometry_mb={:.0} geometry_used_mb={:.0}",
        g.textures,
        mb(g.texture_bytes),
        g.pages,
        mb(g.page_bytes),
        mb(g.texture_target),
        g.pressure,
        mb(g.geometry_bytes),
        mb(g.geometry_used),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_add_up_to_averages_and_peaks() {
        let mut p = DeepProfile {
            enabled: true,
            live: false,
            start: None,
            frames: Frames::default(),
            before: None,
            second: None,
            probes: ProbeTotals::default(),
            last_second: None,
            serial: 0,
        };
        for (decoding, rebuilt) in [(10, 0), (30, 2)] {
            p.add(&FrameSample {
                texture_upload_bytes: 1 << 20,
                decoding,
                skeletons_rebuilt: rebuilt,
                avatars: AvatarCounts {
                    total: 40,
                    pictured: 3,
                    ..AvatarCounts::default()
                },
                ..FrameSample::default()
            });
        }
        let f = p.frames;
        assert_eq!((f.count, f.decoding, f.skeletons_rebuilt, f.pictured), (2, (40, 30), (2, 2), 6));
        let jobs = JobTimes {
            texture: Durations::default(),
            bc7: Durations::default(),
            mesh_lod: Durations::default(),
            mesh_parse: Durations::default(),
            mesh_cache: [4, 1],
        };
        let sample = PeriodSample {
            texture_cache: [15, 5, 7],
            geometry_uploaded: 30 << 20,
            ..PeriodSample::default()
        };
        let before = Before {
            texture_cache: [10, 5, 2],
            geometry_uploaded: 10 << 20,
            page_faults: 0,
        };
        let line = asset_line(&f, &jobs, &sample, &before, 10.0);
        assert!(
            line.starts_with("texture_decode n=0 | bc7 n=0 | decoding avg=20.0 max=30 "),
            "{line}"
        );
        assert!(line.contains("| j2k_cache hit=5 miss=0 bc7_cache_hit=5 |"), "{line}");
        assert!(line.contains("| mesh_cache hit=4 miss=1 "), "{line}");
        assert!(line.ends_with("| gpu_upload textures=0.20 MB/s geometry=2.00 MB/s"), "{line}");
        let line = avatar_line(&f, &AvatarSample::default(), 2.0);
        assert!(
            line.starts_with("total=40 max=40 full=0 full_in_view=0 impostors=0 impostors_in_view=0 pictures/s=3.0 "),
            "{line}"
        );
        assert!(line.ends_with("skeletons_rebuilt avg=1.00 max=2"), "{line}");
    }

    #[test]
    fn the_memory_line_splits_the_gpu_by_kind() {
        let sample = PeriodSample {
            process: Some(ProcessMemory {
                working_set: 2048 << 20,
                peak_working_set: 3072 << 20,
                private: 2560 << 20,
                page_faults: 5000,
            }),
            gpu: GpuMemory {
                video: Some(aurora_render::vram::VideoMemory {
                    budget: 7000 << 20,
                    usage: 4000 << 20,
                    allocated: 3500 << 20,
                }),
                textures: 1200,
                texture_bytes: 1500 << 20,
                pages: 40,
                page_bytes: 2000 << 20,
                geometry_bytes: 500 << 20,
                geometry_used: 300 << 20,
                texture_target: 2500 << 20,
                pressure: 1,
            },
            ..PeriodSample::default()
        };
        let before = Before {
            page_faults: 1000,
            ..Before::default()
        };
        assert_eq!(
            memory_line(&sample, &before, 10.0),
            "process working_set=2048 peak=3072 private=2560 page_faults/s=400 | gpu budget=7000 usage=4000 allocated=3500 other=1000 \
             | textures=1200 texture_mb=1500 pages=40 pages_mb=2000 texture_target_mb=2500 pressure=1 | geometry_mb=500 geometry_used_mb=300"
        );
        assert_eq!(
            memory_line(&PeriodSample::default(), &Before::default(), 10.0),
            "process - | gpu budget=- | textures=0 texture_mb=0 pages=0 pages_mb=0 texture_target_mb=0 pressure=0 | geometry_mb=0 geometry_used_mb=0"
        );
    }
}
