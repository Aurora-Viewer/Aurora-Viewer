//! What the resource monitor shows of each part of the viewer: its CPU,
//! frame time, GPU, memory, video memory, network and disk, attributed
//! from what the viewer measures, never from fixed guesses:
//!
//! - **CPU** (share of the machine): the system's load of each group of
//!   threads (`sys_probe`). A group that works for a single part counts for
//!   it (`aurora-render` for rendering, `aurora-pages` for textures…); the
//!   main thread is shared out by the time of the frame's steps
//!   (`frame_profile`), the render thread by its steps, the frame workers by
//!   the steps they run in parallel, the decode pool by the time of its
//!   texture and mesh jobs, the network runtime by the bytes of the UDP
//!   circuits and of each kind of HTTP request. The media plugins are
//!   processes of their own, read apart, with the processes they start.
//! - **Frame time**: the steps of the main thread and of the render thread,
//!   per frame.
//! - **GPU**: the GPU timers of the frame's elements.
//! - **Memory**: the media processes; the viewer's own memory is not
//!   followed by part, so it is all "not attributed".
//! - **Video memory**: the texture pages, the geometry, the rest of what
//!   the viewer allocated (render targets, shadow maps, tables), and what
//!   the driver counts beyond it.
//! - **Network**: the circuits' UDP traffic, the HTTP requests by kind.
//! - **Disk**: the disk cache and the log files, by origin (`disk_io`).
//!
//! A value nobody measured stays `None` (a dash), never 0.

use crate::deep_profile::ProbeTotals;
use crate::disk_io::Origin;
use crate::frame_profile::{Averages, Lap, Part};
use crate::sys_probe::CpuSnapshot;
use aurora_net::probe::HttpKind;

/// The columns of the subsystems table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Column {
    /// Share of the machine's processor (%).
    Cpu,
    /// Processor time per frame on the frame's two threads (ms).
    Ms,
    /// GPU time per frame (ms).
    Gpu,
    /// Memory (bytes).
    Ram,
    /// Video memory (bytes).
    Vram,
    /// Network traffic in and out (kb/s).
    Net,
    /// Disk reads and writes (bytes/s).
    Disk,
}

impl Column {
    pub const ALL: [Column; 7] = [
        Column::Cpu,
        Column::Ms,
        Column::Gpu,
        Column::Ram,
        Column::Vram,
        Column::Net,
        Column::Disk,
    ];
}

/// What a part costs, by [`Column`]; `None` where nothing measured it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage(pub [Option<f64>; Column::ALL.len()]);

impl Usage {
    pub fn get(&self, c: Column) -> Option<f64> {
        self.0[c as usize]
    }

    /// `get`, 0 where unknown (sorting, sums).
    pub fn or_zero(&self, c: Column) -> f64 {
        self.get(c).unwrap_or(0.0)
    }

    fn add(&mut self, c: Column, v: f64) {
        if v.is_finite() {
            let slot = &mut self.0[c as usize];
            *slot = Some(slot.unwrap_or(0.0) + v.max(0.0));
        }
    }

    fn merge(&mut self, other: &Usage) {
        for c in Column::ALL {
            if let Some(v) = other.get(c) {
                self.add(c, v);
            }
        }
    }
}

macro_rules! rows {
    ($($id:ident $(< $parent:ident)?),* $(,)?) => {
        /// The parts of the viewer, in the table's order: each part, then
        /// its sub-parts.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum RowId { $($id),* }

        impl RowId {
            pub const ALL: &'static [RowId] = &[$(RowId::$id),*];

            /// The part a sub-part belongs to.
            pub fn parent(self) -> Option<RowId> {
                match self {
                    $($(RowId::$id => Some(RowId::$parent),)?)*
                    #[allow(unreachable_patterns)]
                    _ => None,
                }
            }
        }
    };
}

rows! {
    Render,
    RenderShadows < Render,
    RenderObjects < Render,
    RenderTerrain < Render,
    RenderWater < Render,
    RenderBlend < Render,
    RenderPost < Render,
    RenderSubmit < Render,
    Textures,
    TexFetch < Textures,
    TexDecode < Textures,
    TexUpload < Textures,
    TexPages < Textures,
    TexCache < Textures,
    Meshes,
    MeshFetch < Meshes,
    MeshDecode < Meshes,
    MeshGpu < Meshes,
    MeshCache < Meshes,
    Avatars,
    AvPoses < Avatars,
    AvComplexity < Avatars,
    AvAnimations < Avatars,
    AvImpostors < Avatars,
    Scene,
    SceneCull < Scene,
    SceneSync < Scene,
    SceneParticles < Scene,
    SceneLights < Scene,
    SceneCache < Scene,
    World,
    WorldIn < World,
    WorldOut < World,
    WorldEvents < World,
    Interface,
    UiBars < Interface,
    UiTags < Interface,
    UiChat < Interface,
    UiPeople < Interface,
    UiInventory < Interface,
    UiMenus < Interface,
    UiPlaces < Interface,
    UiMaps < Interface,
    UiMonitor < Interface,
    UiOptions < Interface,
    UiPass < Interface,
    UiOther < Interface,
    Media,
    Audio,
    AudioMixer < Audio,
    AudioStream < Audio,
    Voice,
    Services,
    SvcInventory < Services,
    SvcCaps < Services,
    SvcLogs < Services,
    CacheUpkeep,
    Other,
}

impl RowId {
    /// The parts, without their sub-parts.
    pub fn parents() -> impl Iterator<Item = RowId> {
        RowId::ALL.iter().copied().filter(|r| r.parent().is_none())
    }

    /// The sub-parts of a part, in order.
    pub fn kids(self) -> impl Iterator<Item = RowId> {
        RowId::ALL.iter().copied().filter(move |r| r.parent() == Some(self))
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A media plugin process the viewer runs.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaProcess {
    pub pid: u32,
    /// What it shows (a parcel's media, a face's page, a profile's web
    /// page).
    pub label: String,
    /// A video or audio player (libvlc) rather than a web browser.
    pub video: bool,
}

/// Video memory now (bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vram {
    pub pages: u64,
    pub geometry: u64,
    /// What the viewer's device allocated (wgpu's counters).
    pub allocated: Option<u64>,
    /// What the driver counts for the process.
    pub usage: Option<u64>,
    pub budget: Option<u64>,
}

impl Vram {
    /// The driver counts more for the viewer than the budget it grants.
    pub fn over_budget(&self) -> bool {
        matches!((self.usage, self.budget), (Some(u), Some(b)) if u > b)
    }
}

/// Everything one sample attributes.
#[derive(Default)]
pub struct Inputs<'a> {
    pub cpu: Option<&'a CpuSnapshot>,
    pub frame: Option<&'a Averages>,
    pub probes: Option<&'a ProbeTotals>,
    /// UDP traffic (kb/s): in, out.
    pub udp_kbps: [f64; 2],
    /// Disk bytes per second, read and written, by [`Origin::ALL`].
    pub disk: [[f64; 2]; Origin::ALL.len()],
    /// The viewer's working set (bytes).
    pub working_set: Option<u64>,
    pub vram: Vram,
    pub media: &'a [MediaProcess],
}

/// What each part costs over one sample.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attribution {
    /// By [`RowId::ALL`]; a part holds its sub-parts' costs and its own.
    rows: Vec<Usage>,
    /// The media processes, in the order given.
    pub media: Vec<(MediaProcess, Usage)>,
}

impl Attribution {
    pub fn get(&self, id: RowId) -> Usage {
        self.rows.get(id.index()).copied().unwrap_or_default()
    }

    /// A column over every part.
    pub fn total(&self, c: Column) -> Option<f64> {
        let mut known = false;
        let mut sum = 0.0;
        for id in RowId::parents() {
            if let Some(v) = self.get(id).get(c) {
                known = true;
                sum += v;
            }
        }
        known.then_some(sum)
    }
}

/// Groups of threads that work for a single part (their names without
/// the pool's number, `sys_probe::group_of`).
const GROUPS: &[(&str, RowId)] = &[
    ("aurora-pages", RowId::TexPages),
    ("aurora-texcache", RowId::TexCache),
    ("aurora-trash", RowId::Scene),
    ("aurora-audio-out", RowId::AudioMixer),
    ("aurora-audio-stream", RowId::AudioStream),
    ("aurora-voice", RowId::Voice),
    ("aurora-voice-rt", RowId::Voice),
    ("aurora-audio-in", RowId::Voice),
    ("aurora-inventory-cache", RowId::SvcInventory),
    ("aurora-chatlog", RowId::SvcLogs),
    ("aurora-transcript", RowId::SvcLogs),
    ("aurora-notification-history", RowId::SvcLogs),
    ("aurora-favorites", RowId::Services),
    ("aurora-web-profile", RowId::Services),
    ("aurora-cache", RowId::CacheUpkeep),
    ("aurora-web-cache", RowId::CacheUpkeep),
    ("aurora-monitor", RowId::UiMonitor),
    ("aurora-probe", RowId::UiMonitor),
    ("aurora-fonts", RowId::Interface),
    ("aurora-hud-scales", RowId::Interface),
    ("aurora-local-previews", RowId::Interface),
    ("aurora-photo-save", RowId::Interface),
    ("aurora-geometry-needs", RowId::Meshes),
    ("aurora-hover-height", RowId::Avatars),
];

/// Groups shared out by what they ran ([`attribute`]).
const SHARED: [&str; 6] = [
    "main",
    "aurora-render",
    "aurora-render-finish",
    "aurora-worker",
    "aurora-jobs",
    "aurora-net",
];

/// The main thread's steps (ms per frame), each to the part it works for.
fn main_steps(f: &Averages) -> Vec<(RowId, f64)> {
    let lap = |l: Lap| f64::from(f.lap(l));
    let part = |p: Part| f64::from(f.part(p));
    let mut out = vec![
        (RowId::WorldEvents, lap(Lap::Events)),
        (RowId::Services, lap(Lap::Social)),
        (RowId::Avatars, lap(Lap::AgentCamera)),
        (RowId::AvPoses, lap(Lap::Poses)),
        (RowId::AvComplexity, lap(Lap::Complexity)),
        (RowId::SceneSync, lap(Lap::Sync)),
        (RowId::Media, lap(Lap::Media)),
        (RowId::Scene, lap(Lap::Extras) + lap(Lap::Hover)),
        (RowId::SceneCull, lap(Lap::Lists)),
        (RowId::Interface, lap(Lap::UiPrep) + lap(Lap::Actions)),
        (RowId::RenderSubmit, f64::from(f.packet_ms)),
    ];
    // the laps whose parts are measured: the parts, then the rest of the lap
    let split = |out: &mut Vec<(RowId, f64)>, lap_ms: f64, parts: &[(Part, RowId)], rest: RowId| {
        let mut left = lap_ms;
        for &(p, row) in parts {
            let ms = part(p).min(left.max(0.0));
            left -= ms;
            out.push((row, ms));
        }
        out.push((rest, left.max(0.0)));
    };
    split(
        &mut out,
        lap(Lap::Results),
        &[
            (Part::Geometry, RowId::MeshGpu),
            (Part::Fetched, RowId::Textures),
            (Part::Decoded, RowId::Textures),
        ],
        RowId::Scene,
    );
    // (texture pages are made during the uploads: part of `Upload`)
    let pages = part(Part::Pages);
    split(
        &mut out,
        lap(Lap::Stream),
        &[
            (Part::TexUpdate, RowId::TexFetch),
            (Part::Assets, RowId::MeshFetch),
            (Part::Upload, RowId::TexUpload),
            (Part::Maintain, RowId::TexCache),
            (Part::Skin, RowId::AvPoses),
        ],
        RowId::Textures,
    );
    if let Some(upload) = out.iter_mut().find(|(r, _)| *r == RowId::TexUpload) {
        let moved = pages.min(upload.1);
        upload.1 -= moved;
        out.push((RowId::TexPages, moved));
    }
    split(
        &mut out,
        lap(Lap::Params),
        &[
            (Part::ParamsPartSources, RowId::SceneParticles),
            (Part::ParamsPartSim, RowId::SceneParticles),
            (Part::ParamsPartList, RowId::SceneParticles),
            (Part::ParamsLights, RowId::SceneLights),
            (Part::ParamsProbes, RowId::SceneLights),
            (Part::ParamsMirror, RowId::RenderWater),
            (Part::ParamsEnv, RowId::RenderTerrain),
            (Part::ParamsScreen, RowId::Interface),
        ],
        RowId::Scene,
    );
    split(
        &mut out,
        lap(Lap::Ui),
        &[
            (Part::UiBars, RowId::UiBars),
            (Part::UiTags, RowId::UiTags),
            (Part::UiOverlays, RowId::UiOther),
            (Part::UiChat, RowId::UiChat),
            (Part::UiPeople, RowId::UiPeople),
            (Part::UiInventory, RowId::UiInventory),
            (Part::UiMenus, RowId::UiMenus),
            (Part::UiPlaces, RowId::UiPlaces),
            (Part::UiMaps, RowId::UiMaps),
            (Part::UiPerf, RowId::UiMonitor),
            (Part::UiOptions, RowId::UiOptions),
            (Part::UiRest, RowId::UiOther),
            (Part::UiPass, RowId::UiPass),
        ],
        RowId::Interface,
    );
    out
}

/// The render thread's steps (ms per frame), each to its part; the steps
/// that wait by design (`acquire`, `present`) are left out.
fn render_steps(f: &Averages) -> Vec<(RowId, f64)> {
    let ph = |k: &str| f64::from(f.phase(k));
    vec![
        (RowId::RenderShadows, ph("shadows") + ph("prepass")),
        (RowId::RenderPost, ph("ssao") + ph("post")),
        (RowId::RenderWater, ph("reflections")),
        (RowId::AvImpostors, ph("impostors")),
        (RowId::RenderObjects, ph("scene_a")),
        (RowId::RenderBlend, ph("scene_b")),
        (RowId::UiPass, ph("egui") + ph("tessellate")),
        (
            RowId::RenderSubmit,
            ph("resources") + ph("lists") + ph("setup") + ph("finish") + ph("submit"),
        ),
    ]
}

/// The part each kind of HTTP request downloads for.
fn http_row(kind: HttpKind) -> RowId {
    match kind {
        HttpKind::Texture | HttpKind::Material => RowId::TexFetch,
        HttpKind::Mesh => RowId::MeshFetch,
        HttpKind::Animation => RowId::AvAnimations,
        HttpKind::Sound => RowId::Audio,
        HttpKind::Inventory => RowId::SvcInventory,
        HttpKind::Settings | HttpKind::Landmark | HttpKind::Asset | HttpKind::Cap => RowId::SvcCaps,
    }
}

/// The part each origin of disk bytes belongs to.
fn disk_row(origin: Origin) -> RowId {
    match origin {
        Origin::Textures => RowId::TexCache,
        Origin::Meshes => RowId::MeshCache,
        Origin::Objects => RowId::SceneCache,
        Origin::Inventory => RowId::SvcInventory,
        Origin::Logs => RowId::SvcLogs,
        Origin::Other => RowId::Other,
    }
}

/// Shares `total` out over `weights` in proportion; nothing when they add
/// up to nothing.
fn share(usage: &mut [Usage], c: Column, total: f64, weights: &[(RowId, f64)]) -> bool {
    let sum: f64 = weights.iter().map(|w| w.1.max(0.0)).sum();
    if total <= 0.0 || sum <= 0.0 {
        return false;
    }
    for &(row, w) in weights {
        if w > 0.0 {
            usage[row.index()].add(c, total * w / sum);
        }
    }
    true
}

/// What each part cost over the sample `inp` describes.
pub fn attribute(inp: &Inputs) -> Attribution {
    let mut u = vec![Usage::default(); RowId::ALL.len()];
    let main = inp.frame.map(main_steps).unwrap_or_default();
    let render = inp.frame.map(render_steps).unwrap_or_default();

    // frame time
    for &(row, ms) in main.iter().chain(&render) {
        if ms > 0.0 {
            u[row.index()].add(Column::Ms, ms);
        }
    }
    // GPU
    if let Some(g) = inp.frame.and_then(|f| f.gpu) {
        let rows = [
            RowId::RenderShadows,
            RowId::RenderPost,
            RowId::RenderTerrain,
            RowId::RenderObjects,
            RowId::RenderWater,
            RowId::RenderBlend,
            RowId::RenderPost,
        ];
        for (row, ms) in rows.iter().zip(g) {
            u[row.index()].add(Column::Gpu, f64::from(ms));
        }
    }

    // network: UDP, then HTTP by kind (kb/s)
    let mut net_weights = vec![(RowId::WorldIn, inp.udp_kbps[0]), (RowId::WorldOut, inp.udp_kbps[1])];
    if let Some(p) = inp.probes {
        let secs = f64::from(p.secs.max(1e-3));
        for (kind, period) in aurora_net::probe::HTTP_KINDS.iter().zip(&p.http.kinds) {
            net_weights.push((http_row(*kind), period.bytes as f64 * 8.0 / 1000.0 / secs));
        }
    }
    for &(row, kbps) in &net_weights {
        if kbps > 0.0 || matches!(row, RowId::WorldIn | RowId::WorldOut) {
            u[row.index()].add(Column::Net, kbps);
        }
    }

    // disk (bytes/s)
    for (origin, [r, w]) in Origin::ALL.iter().zip(inp.disk) {
        u[disk_row(*origin).index()].add(Column::Disk, r + w);
    }

    // video memory
    let v = inp.vram;
    u[RowId::TexPages.index()].add(Column::Vram, v.pages as f64);
    u[RowId::MeshGpu.index()].add(Column::Vram, v.geometry as f64);
    if let Some(allocated) = v.allocated {
        u[RowId::Render.index()].add(Column::Vram, allocated.saturating_sub(v.pages + v.geometry) as f64);
        if let Some(usage) = v.usage {
            u[RowId::Other.index()].add(Column::Vram, usage.saturating_sub(allocated) as f64);
        }
    }

    // memory: the viewer's own is not followed by part
    if let Some(ws) = inp.working_set {
        u[RowId::Other.index()].add(Column::Ram, ws as f64);
    }

    // processor
    let mut media = Vec::new();
    if let Some(cpu) = inp.cpu {
        let cores = cpu.cores.max(1) as f64;
        let group = |name: &str| -> f64 {
            cpu.groups
                .iter()
                .filter(|g| g.group == name)
                .map(|g| f64::from(g.percent))
                .sum::<f64>()
                / cores
        };
        let mut attributed = 0.0;
        for &(name, row) in GROUPS {
            let v = group(name);
            if v > 0.0 {
                u[row.index()].add(Column::Cpu, v);
                attributed += v;
            }
        }
        for name in SHARED {
            attributed += group(name);
        }
        // the main thread by its steps; without them, all of it unattributed
        let main_cpu = group("main");
        if !share(&mut u, Column::Cpu, main_cpu, &main) {
            u[RowId::Other.index()].add(Column::Cpu, main_cpu);
        }
        let render_cpu = group("aurora-render") + group("aurora-render-finish");
        if !share(&mut u, Column::Cpu, render_cpu, &render) {
            u[RowId::Render.index()].add(Column::Cpu, render_cpu);
        }
        // the frame's parallel loops: culling and lists, the sync's plan, poses
        let parallel: Vec<(RowId, f64)> = main
            .iter()
            .filter(|(r, _)| matches!(r, RowId::SceneCull | RowId::SceneSync | RowId::AvPoses))
            .copied()
            .collect();
        let workers = group("aurora-worker");
        if !share(&mut u, Column::Cpu, workers, &parallel) {
            u[RowId::Scene.index()].add(Column::Cpu, workers);
        }
        // the decode pool by the time of its jobs
        let jobs = group("aurora-jobs");
        let job_weights: Vec<(RowId, f64)> = inp
            .probes
            .map(|p| {
                let (n, ms) = p.texture_decodes();
                let (m, mesh_ms) = p.mesh_builds();
                vec![
                    (RowId::TexDecode, n as f64 * f64::from(ms)),
                    (RowId::MeshDecode, m as f64 * f64::from(mesh_ms)),
                ]
            })
            .unwrap_or_default();
        if !share(&mut u, Column::Cpu, jobs, &job_weights) {
            u[RowId::Other.index()].add(Column::Cpu, jobs);
        }
        // the network runtime by the bytes it carried
        let net = group("aurora-net");
        if !share(&mut u, Column::Cpu, net, &net_weights) {
            u[RowId::World.index()].add(Column::Cpu, net);
        }
        // threads ended since, unnamed ones (drivers, the system's)
        let process = f64::from(cpu.process);
        u[RowId::Other.index()].add(Column::Cpu, (process - attributed).max(0.0));

        for m in inp.media {
            let mut usage = Usage::default();
            if let Some(c) = cpu.children.iter().find(|c| c.pid == m.pid) {
                usage.add(Column::Cpu, f64::from(c.percent) / cores);
                usage.add(Column::Ram, c.working_set as f64);
            }
            media.push((m.clone(), usage));
        }
    } else {
        media = inp.media.iter().map(|m| (m.clone(), Usage::default())).collect();
    }

    // each part holds its sub-parts and its media processes
    for id in RowId::ALL {
        if let Some(parent) = id.parent() {
            let kid = u[id.index()];
            u[parent.index()].merge(&kid);
        }
    }
    for (_, usage) in &media {
        u[RowId::Media.index()].merge(usage);
    }
    Attribution { rows: u, media }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys_probe::{ChildLoad, GroupLoad};

    fn group(name: &str, percent: f32) -> GroupLoad {
        GroupLoad {
            group: name.into(),
            threads: 1,
            percent,
            busiest: percent,
            priority: Some(0),
        }
    }

    #[test]
    fn parts_hold_their_sub_parts() {
        assert_eq!(RowId::TexPages.parent(), Some(RowId::Textures));
        assert_eq!(RowId::Textures.parent(), None);
        assert_eq!(RowId::Render.kids().count(), 7);
        assert!(RowId::parents().all(|p| p.parent().is_none()));
        // every sub-part comes after its part, before the next part
        for (i, id) in RowId::ALL.iter().enumerate() {
            if let Some(p) = id.parent() {
                let at = RowId::ALL.iter().position(|r| *r == p).expect("listed");
                assert!(at < i && RowId::ALL[at + 1..i].iter().all(|r| r.parent() == Some(p)));
            }
        }
    }

    #[test]
    fn the_main_thread_is_shared_out_by_its_steps() {
        // 16 cores; the main thread busy a whole core: 6.25 % of the machine
        let cpu = CpuSnapshot {
            cores: 16,
            process: 6.25 + 1.0,
            groups: vec![group("main", 100.0), group("aurora-pages", 16.0)],
            ..CpuSnapshot::default()
        };
        // 3 ms of interface (2 of them in the bars), 1 ms of lists
        let frame = Averages::with(
            &[(Lap::Ui, 3.0), (Lap::Lists, 1.0), (Lap::Limiter, 12.0)],
            &[(Part::UiBars, 2.0)],
            &[],
            None,
        );
        let a = attribute(&Inputs {
            cpu: Some(&cpu),
            frame: Some(&frame),
            ..Inputs::default()
        });
        let cpu_of = |r: RowId| a.get(r).get(Column::Cpu).unwrap_or(0.0);
        assert!((cpu_of(RowId::Interface) - 6.25 * 0.75).abs() < 1e-9);
        assert!((cpu_of(RowId::UiBars) - 6.25 * 0.5).abs() < 1e-9);
        assert!((cpu_of(RowId::SceneCull) - 6.25 * 0.25).abs() < 1e-9);
        // a single-purpose group goes to its part, and its part's parent
        assert!((cpu_of(RowId::TexPages) - 1.0).abs() < 1e-9);
        assert!((cpu_of(RowId::Textures) - 1.0).abs() < 1e-9);
        // waiting (the limiter) costs no frame time to anyone
        assert_eq!(a.get(RowId::Interface).get(Column::Ms), Some(3.0));
        assert!((a.total(Column::Cpu).unwrap_or(0.0) - 7.25).abs() < 1e-9);
    }

    #[test]
    fn what_nothing_measured_stays_unknown() {
        let a = attribute(&Inputs::default());
        assert_eq!(a.get(RowId::Avatars).get(Column::Cpu), None);
        assert_eq!(a.get(RowId::Avatars).get(Column::Gpu), None);
        assert_eq!(a.get(RowId::Render).get(Column::Vram), None);
        // the circuits are always counted, even quiet
        assert_eq!(a.get(RowId::World).get(Column::Net), Some(0.0));
    }

    #[test]
    fn video_memory_by_kind() {
        let a = attribute(&Inputs {
            vram: Vram {
                pages: 3000,
                geometry: 1000,
                allocated: Some(5000),
                usage: Some(5600),
                budget: Some(8000),
            },
            ..Inputs::default()
        });
        assert_eq!(a.get(RowId::Textures).get(Column::Vram), Some(3000.0));
        assert_eq!(a.get(RowId::Meshes).get(Column::Vram), Some(1000.0));
        assert_eq!(a.get(RowId::Render).get(Column::Vram), Some(1000.0));
        assert_eq!(a.get(RowId::Other).get(Column::Vram), Some(600.0));
        assert_eq!(a.total(Column::Vram), Some(5600.0));
    }

    #[test]
    fn past_the_drivers_budget() {
        let v = |usage, budget| Vram {
            usage,
            budget,
            ..Vram::default()
        };
        assert!(v(Some(8100), Some(8000)).over_budget());
        assert!(!v(Some(8000), Some(8000)).over_budget());
        // no figures from the driver: nothing said
        assert!(!v(Some(8100), None).over_budget());
        assert!(!v(None, Some(8000)).over_budget());
    }

    #[test]
    fn media_processes_are_read_apart() {
        let cpu = CpuSnapshot {
            cores: 4,
            process: 10.0,
            children: vec![ChildLoad {
                pid: 77,
                percent: 40.0,
                working_set: 300,
            }],
            ..CpuSnapshot::default()
        };
        let media = [
            MediaProcess {
                pid: 77,
                label: "page".into(),
                video: false,
            },
            MediaProcess {
                pid: 78,
                label: "ended".into(),
                video: true,
            },
        ];
        let a = attribute(&Inputs {
            cpu: Some(&cpu),
            media: &media,
            ..Inputs::default()
        });
        assert_eq!(a.media.len(), 2);
        assert_eq!(a.media[0].1.get(Column::Cpu), Some(10.0));
        assert_eq!(a.media[1].1.get(Column::Cpu), None);
        assert_eq!(a.get(RowId::Media).get(Column::Ram), Some(300.0));
        // the viewer's own time, nothing named: not attributed
        assert_eq!(a.get(RowId::Other).get(Column::Cpu), Some(10.0));
    }

    #[test]
    fn disk_bytes_go_to_their_cache() {
        let mut disk = [[0.0; 2]; Origin::ALL.len()];
        disk[Origin::Meshes as usize] = [100.0, 50.0];
        disk[Origin::Logs as usize] = [0.0, 8.0];
        let a = attribute(&Inputs { disk, ..Inputs::default() });
        assert_eq!(a.get(RowId::MeshCache).get(Column::Disk), Some(150.0));
        assert_eq!(a.get(RowId::Services).get(Column::Disk), Some(8.0));
    }
}
