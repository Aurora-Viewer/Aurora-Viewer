//! What the resource monitor draws, gathered by the app every half second
//! (`app/monitor.rs`) and kept for a minute: the latest snapshot whole,
//! the history as the few numbers the graphs need.

use super::model::{Attribution, Column, RowId, Vram};
use crate::deep_profile::ProbeTotals;
use crate::disk_io::Origin;
use crate::sys_probe::{CpuSnapshot, ProcessMemory};
use std::collections::VecDeque;
use std::time::SystemTime;

/// Samples kept: a minute at one every half second.
pub const HISTORY: usize = 120;

/// The frame's parts as the compact view draws them (`perf.rs`): CPU and,
/// when the GPU times them, GPU (ms).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameParts {
    pub cpu: [f32; 7],
    pub gpu: Option<[f32; 7]>,
    /// The render thread's time per frame (ms); None without one.
    pub render_thread: Option<f32>,
}

/// The texture streamer (scene/textures.rs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureInfo {
    pub loaded: usize,
    pub total: usize,
    pub fetching: usize,
    pub decoding: usize,
    pub pending_upload: usize,
    pub uploads_frame: usize,
    pub upload_bytes_frame: u64,
    pub failing: usize,
    /// What the pages may hold, and the pressure step (texture_budget.rs).
    pub target: u64,
    pub pressure: u8,
    pub max_pressure: u8,
    /// Textures lowered on the GPU this session.
    pub reduced: u64,
    /// The budget follows the video memory (the setting).
    pub budget_auto: bool,
    /// Background jobs queued or running (decodes, geometry), meshes
    /// downloading.
    pub jobs: usize,
    pub meshes_fetching: usize,
}

/// The geometry arena and the GPU buffers written each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeometryInfo {
    pub allocated: u64,
    pub used: u64,
    pub pending: usize,
    pub records: usize,
    /// Bytes written per frame: draw records, skeleton palettes, culling
    /// tables, the render thread's journal.
    pub records_frame: u64,
    pub palettes_frame: u64,
    pub cull_frame: u64,
    pub journal_frame: u64,
}

/// The link with the simulators over the last second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetInfo {
    pub in_kbps: f32,
    pub out_kbps: f32,
    pub ping_ms: u32,
    pub loss: f32,
    pub packets_in: f32,
    pub packets_out: f32,
    pub resent: u64,
    pub given_up: u64,
    pub unacked: u32,
    pub input_delay_ms: Option<f32>,
    pub loop_ms: f32,
    pub http_active: u32,
    pub http_queued: u32,
    /// Corrections of the agent's position this session, and the largest
    /// (m); the longest an event waited over the last second (ms).
    pub corrections: (u64, f32),
    pub event_wait_ms: f32,
    /// Downloaded over HTTP this session (bytes), requests failed.
    pub downloaded: u64,
    pub http_failed: u64,
    /// Asked of the simulator (kb/s) and the setting.
    pub throttle_asked: u32,
    pub throttle_setting: u32,
}

/// One simulator the viewer has a circuit with.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionLink {
    pub name: String,
    pub main: bool,
    pub dilation: f32,
    /// Seconds since its last packet.
    pub packet_age: f32,
}

/// The disk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskInfo {
    /// Bytes per second read and written over the last seconds, by
    /// [`Origin::ALL`].
    pub rates: [[f64; 2]; Origin::ALL.len()],
    /// Bytes of each cache folder (cache.rs `DIRS`).
    pub by_dir: Vec<(&'static str, u64)>,
    pub used: u64,
    pub limit: u64,
    pub last_trim: Option<(SystemTime, usize, u64)>,
    /// Found in the cache, as a share of the reads (%), with the reads:
    /// textures (JPEG 2000), textures already compressed, meshes, region
    /// objects.
    pub hits: [Option<(f32, u64)>; 4],
}

/// How an avatar is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarMode {
    Full,
    Impostor,
    Silhouette,
    Loading,
    /// Beyond the draw distance.
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AvatarRow {
    pub id: uuid::Uuid,
    pub name: String,
    pub distance: f32,
    pub complexity: Option<u32>,
    pub triangles: u64,
    pub attachments: usize,
    /// Video memory of its textures at the size they are shown (bytes).
    pub vram: u64,
    pub mode: AvatarMode,
}

/// What makes an object cost more than its triangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Particles,
    Light,
    Media,
    Sound,
    Animation,
    Transparent,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectRow {
    /// The root prim's full id (Inspect).
    pub id: uuid::Uuid,
    /// None until its properties have arrived.
    pub name: Option<String>,
    pub prims: usize,
    pub distance: f32,
    pub triangles: u64,
    pub faces: usize,
    /// Texture memory at full size (Inspect's TMem) and in video memory
    /// at the size shown (bytes).
    pub tmem: u64,
    pub vram: u64,
    pub features: Vec<Feature>,
}

/// The scene around.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneInfo {
    pub avatars: Vec<AvatarRow>,
    pub objects: Vec<ObjectRow>,
    pub avatar_count: usize,
    pub full: usize,
    pub impostors: usize,
    pub silhouettes: usize,
    pub rigged_triangles: u64,
    pub attachments: usize,
    pub object_count: usize,
    pub visible: usize,
    pub triangles: u64,
    pub draws: usize,
    pub shadow_draws: usize,
    pub particles: usize,
    pub lights: usize,
    /// Objects hidden by the occlusion test (None without it), culled on
    /// the graphics card or not.
    pub occluded: Option<u32>,
    pub gpu_cull: bool,
    /// Reflection probes: known, ready.
    pub probes: (usize, usize),
    pub sounds: usize,
    /// The agent's own complexity.
    pub own_complexity: Option<u32>,
    /// Planar reflections drawn: water, mirror.
    pub reflections: (bool, bool),
}

/// The simulator of the agent's region (SimStats).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionHealth {
    pub name: String,
    pub dilation: Option<f32>,
    pub fps: Option<f32>,
    pub physics_fps: Option<f32>,
    pub frame_ms: Option<f32>,
    pub physics_ms: Option<f32>,
    pub script_ms: Option<f32>,
    pub net_ms: Option<f32>,
    pub agent_ms: Option<f32>,
    pub images_ms: Option<f32>,
    pub other_ms: Option<f32>,
    pub spare_ms: Option<f32>,
    pub scripts: Option<f32>,
    pub agents: Option<f32>,
    pub child_agents: Option<f32>,
    pub objects: Option<f32>,
    pub capacity: u32,
}

/// Everything of one half second.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub attribution: Attribution,
    pub cpu: Option<CpuSnapshot>,
    pub probes: Option<ProbeTotals>,
    pub fps: f32,
    pub frame_ms: f32,
    pub parts: FrameParts,
    pub memory: Option<ProcessMemory>,
    pub page_faults_per_s: Option<f32>,
    pub vram: Vram,
    pub texture: TextureInfo,
    pub geometry: GeometryInfo,
    pub net: NetInfo,
    pub regions: Vec<RegionLink>,
    pub disk: DiskInfo,
    pub scene: SceneInfo,
    pub region: Option<RegionHealth>,
}

/// One point of the history.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Point {
    pub fps: f32,
    /// The viewer's processor (% of the machine), its media processes',
    /// the machine's.
    pub cpu: Option<f32>,
    pub media_cpu: Option<f32>,
    pub machine: Option<f32>,
    pub gpu_ms: Option<f32>,
    pub ram: Option<f64>,
    pub vram: Option<f64>,
    pub net_in: f32,
    pub net_out: f32,
    pub disk_read: f64,
    pub disk_write: f64,
    pub dilation: Option<f32>,
    /// The processor of each part (% of the machine), by [`RowId::ALL`].
    pub rows: Vec<f32>,
}

impl Point {
    pub fn of(s: &Snapshot) -> Point {
        let a = &s.attribution;
        let media = a.get(RowId::Media).get(Column::Cpu);
        Point {
            fps: s.fps,
            cpu: s.cpu.as_ref().map(|c| c.process),
            media_cpu: media.map(|v| v as f32),
            machine: s.cpu.as_ref().and_then(|c| c.machine),
            gpu_ms: s.parts.gpu.map(|g| g.iter().sum()),
            ram: s.memory.map(|m| m.working_set as f64),
            vram: s.vram.usage.map(|v| v as f64),
            net_in: s.net.in_kbps,
            net_out: s.net.out_kbps,
            disk_read: s.disk.rates.iter().map(|r| r[0]).sum(),
            disk_write: s.disk.rates.iter().map(|r| r[1]).sum(),
            dilation: s.region.as_ref().and_then(|r| r.dilation),
            rows: RowId::ALL.iter().map(|r| a.get(*r).or_zero(Column::Cpu) as f32).collect(),
        }
    }
}

/// The latest snapshot and a minute of points.
#[derive(Default)]
pub struct History {
    pub last: Option<Snapshot>,
    pub points: VecDeque<Point>,
}

impl History {
    pub fn push(&mut self, s: Snapshot) {
        if self.points.len() >= HISTORY {
            self.points.pop_front();
        }
        self.points.push_back(Point::of(&s));
        self.last = Some(s);
    }

    pub fn clear(&mut self) {
        self.last = None;
        self.points.clear();
    }

    /// One value over the minute (oldest first), `None` points as 0.
    pub fn series(&self, f: impl Fn(&Point) -> Option<f64>) -> Vec<f64> {
        self.points.iter().map(|p| f(p).unwrap_or(0.0)).collect()
    }

    /// The processor of a part over the minute.
    pub fn row_series(&self, id: RowId) -> Vec<f64> {
        let i = RowId::ALL.iter().position(|r| *r == id).unwrap_or(0);
        self.series(|p| p.rows.get(i).map(|v| f64::from(*v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_keeps_a_minute() {
        let mut h = History::default();
        let snap = |fps: f32| Snapshot {
            attribution: Attribution::default(),
            cpu: None,
            probes: None,
            fps,
            frame_ms: 1000.0 / fps,
            parts: FrameParts::default(),
            memory: None,
            page_faults_per_s: None,
            vram: Vram::default(),
            texture: TextureInfo::default(),
            geometry: GeometryInfo::default(),
            net: NetInfo::default(),
            regions: Vec::new(),
            disk: DiskInfo::default(),
            scene: SceneInfo::default(),
            region: None,
        };
        for i in 0..HISTORY + 5 {
            h.push(snap(i as f32 + 1.0));
        }
        assert_eq!(h.points.len(), HISTORY);
        let fps = h.series(|p| Some(f64::from(p.fps)));
        assert_eq!((fps[0], fps[HISTORY - 1]), (6.0, (HISTORY + 5) as f64));
        assert_eq!(h.last.as_ref().map(|s| s.fps), Some((HISTORY + 5) as f32));
        assert_eq!(h.row_series(RowId::Textures).len(), HISTORY);
    }
}
