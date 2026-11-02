//! Frame profile (AURORA_PROFILE=1): where the time of each frame goes, as
//! one summary line in the log every second. The laps of the frame loop
//! cover the whole time between two frames; the renderer adds its own
//! steps, the GPU time by element and counters (draws, synced objects,
//! posed avatars, bytes sent to the GPU). Besides the averages, the line
//! gives the longest time of each step over the second (`max:`), so that a
//! slow frame among fast ones can be pinned on the step that caused it,
//! and the longest time a step spent off the processor (`off_cpu:`,
//! aurora-render's thread_cpu.rs): a slow step that waited (a lock, a core)
//! rather than worked. Off without the variable.
//!
//! Two threads: the laps are the main thread's; the renderer's steps
//! (`r_*`) and their total (`rt_frame`) are the render thread's when there
//! is one (aurora-render's render_thread.rs), running beside the laps of
//! the next frame. The `thread` segment says how the two meet: what the
//! main thread spent on the frame packet (`packet`), how long it waited
//! for the render thread (`wait_render`, both inside the `render` lap) and
//! how long the render thread waited for a packet (`rt_idle`).

use aurora_render::light_tiles::LightCulling;
use aurora_render::{BlendDiscard, GpuElement, RENDER_PHASES, RenderStats, thread_cpu};
use std::fmt::Write;
use std::time::{Duration, Instant};

/// Steps of `App::frame`, in order. Each lap ends one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lap {
    /// Window events and anything else outside `App::frame`.
    Between,
    /// Quit check, logo, skin reload.
    Start,
    /// Network events applied to the world.
    Events,
    /// Names, outfit, groups, inventory requests (twice a second).
    Social,
    /// Inputs, audio, sounds, agent, camera, voice.
    AgentCamera,
    /// Finished background jobs (geometry, textures, meshes).
    Results,
    /// Avatar and animesh poses.
    Poses,
    Complexity,
    /// Local build drags, edited focus camera, then `Scene::sync` to GPU records.
    Sync,
    Media,
    /// Loading clouds, particles, ban lines.
    Extras,
    /// Culling and draw lists.
    Lists,
    /// Texture and mesh streaming, diagnostics.
    Stream,
    /// Loading transition, lights, environment, frame parameters.
    Params,
    /// Interface input and build floater preparation.
    UiPrep,
    /// The egui windows (`run_ui`).
    Ui,
    /// What is under the cursor: hover cursor, touch / sit targets.
    Hover,
    /// Interface actions, avatar pictures, maps.
    Actions,
    /// `Renderer::render`: the frame packet and the wait for the render
    /// thread to take it; without a render thread, the whole frame's
    /// encoding, swapchain wait included.
    Render,
    /// Frame limiter.
    Limiter,
    /// Captures and the performance window statistics.
    Tail,
}

impl Lap {
    const ALL: [Lap; 21] = [
        Lap::Between,
        Lap::Start,
        Lap::Events,
        Lap::Social,
        Lap::AgentCamera,
        Lap::Results,
        Lap::Poses,
        Lap::Complexity,
        Lap::Sync,
        Lap::Media,
        Lap::Extras,
        Lap::Lists,
        Lap::Stream,
        Lap::Params,
        Lap::UiPrep,
        Lap::Ui,
        Lap::Hover,
        Lap::Actions,
        Lap::Render,
        Lap::Limiter,
        Lap::Tail,
    ];

    fn key(self) -> &'static str {
        match self {
            Lap::Between => "between",
            Lap::Start => "start",
            Lap::Events => "events",
            Lap::Social => "social",
            Lap::AgentCamera => "agent_camera",
            Lap::Results => "results",
            Lap::Poses => "poses",
            Lap::Complexity => "complexity",
            Lap::Sync => "sync",
            Lap::Media => "media",
            Lap::Extras => "extras",
            Lap::Lists => "lists",
            Lap::Stream => "stream",
            Lap::Params => "params",
            Lap::UiPrep => "ui_prep",
            Lap::Ui => "ui",
            Lap::Hover => "hover",
            Lap::Actions => "actions",
            Lap::Render => "render",
            Lap::Limiter => "limiter",
            Lap::Tail => "tail",
        }
    }
}

/// Sub-steps of the main thread's laps. Streaming (`s_*` in the summary):
/// what `Results`, `Stream` and `Sync` spend their time on, so that a slow
/// frame while content loads names its cause. Frame parameters (`p_*`):
/// what the `Params` lap spends its time on. Interface (`u_*`): what the
/// `Ui` lap spends its time on, by group of windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// Downloads handed to the streamers (`Results`).
    Fetched,
    /// Finished geometry put in the arena (`Results`).
    Geometry,
    /// Other finished jobs: decoded textures queued, cache reads (`Results`).
    Decoded,
    /// `TextureStreamer::update`: fetches and decodes started.
    TexUpdate,
    /// Mesh, animation, sound, material and settings streamers.
    Assets,
    /// Skin bind matrices sent to the GPU.
    Skin,
    /// Decoded textures sent to the GPU, texture pages included.
    Upload,
    /// Texture pages created during the uploads (part of `Upload`).
    Pages,
    /// Texture eviction, cache writes, memory budget.
    Maintain,
    /// Streaming diagnostics in the log.
    Diag,
    /// `Scene::sync`: removed objects, changed objects, the frame's list.
    SyncList,
    /// `Scene::sync`: placement of the listed objects (in parallel).
    SyncPlan,
    /// `Scene::sync`: moves applied, full syncs within the time budget.
    SyncApply,
    /// `Scene::sync`: anchors of the worn objects the GPU places, palettes
    /// uploaded, centres of the anchored objects.
    SyncAnchors,
    /// `Scene::sync`: faces classified again after a texture changed alpha
    /// class.
    SyncAlpha,
    /// Loading to world transition, teleport screen, arrival diagnostics.
    ParamsScreen,
    /// Sky and water of the frame, their textures.
    ParamsEnv,
    /// `Scene::find_mirror`: the nearest visible mirror.
    ParamsMirror,
    /// Particles: systems set or stopped by object updates.
    ParamsPartSources,
    /// Particles: simulation of every source.
    ParamsPartSim,
    /// Particles: the frame's instances, sorted back to front.
    ParamsPartList,
    /// Local lights nearest to the camera.
    ParamsLights,
    /// Reflection probes (candidate scan, slots, face to capture), then
    /// the frame parameters put together.
    ParamsProbes,
    /// Top and bottom bars.
    UiBars,
    /// Avatar name tags and hover text of the prims.
    UiTags,
    /// Beacon, HUD text, build overlay and window, debug overlay, chat
    /// toasts.
    UiOverlays,
    /// Conversations and contacts.
    UiChat,
    /// People window and its map.
    UiPeople,
    /// Inventory, appearance, animation overrider.
    UiInventory,
    /// Context menu, confirmations, media, object windows, notecards,
    /// scripts, notifications.
    UiMenus,
    /// About land, profiles, places.
    UiPlaces,
    /// Minimap and world map.
    UiMaps,
    /// Performance window.
    UiPerf,
    /// Environment and preferences.
    UiOptions,
    /// Teleport screen, bug report, exit prompt; the whole screen outside
    /// the world (login, loading).
    UiRest,
    /// egui around the windows: start and end of the pass (shapes gathered
    /// from the layers), interface sounds.
    UiPass,
}

impl Part {
    pub const ALL: [Part; 36] = [
        Part::Fetched,
        Part::Geometry,
        Part::Decoded,
        Part::TexUpdate,
        Part::Assets,
        Part::Skin,
        Part::Upload,
        Part::Pages,
        Part::Maintain,
        Part::Diag,
        Part::SyncList,
        Part::SyncPlan,
        Part::SyncApply,
        Part::SyncAnchors,
        Part::SyncAlpha,
        Part::ParamsScreen,
        Part::ParamsEnv,
        Part::ParamsMirror,
        Part::ParamsPartSources,
        Part::ParamsPartSim,
        Part::ParamsPartList,
        Part::ParamsLights,
        Part::ParamsProbes,
        Part::UiBars,
        Part::UiTags,
        Part::UiOverlays,
        Part::UiChat,
        Part::UiPeople,
        Part::UiInventory,
        Part::UiMenus,
        Part::UiPlaces,
        Part::UiMaps,
        Part::UiPerf,
        Part::UiOptions,
        Part::UiRest,
        Part::UiPass,
    ];

    /// Prefix of the part's key in the summary: its segment.
    fn prefix(self) -> &'static str {
        match self as usize {
            i if i < Part::ParamsScreen as usize => "s_",
            i if i < Part::UiBars as usize => "p_",
            _ => "u_",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Part::Fetched => "fetched",
            Part::Geometry => "geometry",
            Part::Decoded => "decoded",
            Part::TexUpdate => "tex_update",
            Part::Assets => "assets",
            Part::Skin => "skin",
            Part::Upload => "upload",
            Part::Pages => "pages",
            Part::Maintain => "maintain",
            Part::Diag => "diag",
            Part::SyncList => "sync_list",
            Part::SyncPlan => "sync_plan",
            Part::SyncApply => "sync_apply",
            Part::SyncAnchors => "sync_anchors",
            Part::SyncAlpha => "sync_alpha",
            Part::ParamsScreen => "screen",
            Part::ParamsEnv => "env",
            Part::ParamsMirror => "mirror",
            Part::ParamsPartSources => "part_sources",
            Part::ParamsPartSim => "part_sim",
            Part::ParamsPartList => "part_list",
            Part::ParamsLights => "lights",
            Part::ParamsProbes => "probes",
            Part::UiBars => "bars",
            Part::UiTags => "tags",
            Part::UiOverlays => "overlays",
            Part::UiChat => "chat",
            Part::UiPeople => "people",
            Part::UiInventory => "inventory",
            Part::UiMenus => "menus",
            Part::UiPlaces => "places",
            Part::UiMaps => "maps",
            Part::UiPerf => "perf",
            Part::UiOptions => "options",
            Part::UiRest => "rest",
            Part::UiPass => "pass",
        }
    }
}

const PARTS: usize = Part::ALL.len();

/// Time of each [`Part`] in the current frame (ms), filled by the scene and
/// by the interface and taken by [`FrameProfile::end_frame`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts([f32; PARTS]);

// by hand: the standard library stops at arrays of 32
impl Default for Parts {
    fn default() -> Self {
        Parts([0.0; PARTS])
    }
}

impl Parts {
    pub fn add(&mut self, part: Part, ms: f32) {
        self.0[part as usize] += ms;
    }

    /// Add the time since `since` to `part`; returns now (for the next one).
    pub fn lap(&mut self, part: Part, since: Instant) -> Instant {
        let now = Instant::now();
        self.add(part, (now - since).as_secs_f32() * 1000.0);
        now
    }

    /// Add the times of `other`, filled apart for the same frame.
    pub fn merge(&mut self, other: &Parts) {
        add(&mut self.0, &other.0);
    }
}

/// The `thread` segment of the summary: `packet` and `wait_render` on the
/// main thread (inside the `render` lap), `rt_frame` and `rt_idle` on the
/// render thread.
/// `pace`: the frame pacing's sleep before the next frame (pacing.rs);
/// `input`: from the frame's input to its presentation.
const THREAD_KEYS: [&str; 6] = ["packet", "wait_render", "rt_frame", "rt_idle", "pace", "input"];

const LAPS: usize = Lap::ALL.len();
const GPU: usize = GpuElement::ALL.len();
const PHASES: usize = RENDER_PHASES.len();
/// Time covered by one summary line.
const PERIOD: Duration = Duration::from_secs(1);
/// Steps whose longest time over the period reaches this (ms) are listed
/// in the `max:` segment.
const MAX_SHOWN_MS: f32 = 1.0;

/// Scene counts of a frame, summed over the period.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneCounts {
    pub objects: usize,
    pub visible: usize,
    pub synced: usize,
    pub rebuilt: usize,
    pub posed: usize,
    pub blend: usize,
    /// Blended faces with glow: each one splits the blended multi-draw.
    pub blend_glow: usize,
    /// Alpha-pool glow faces: two single draws each.
    pub glow_alpha: usize,
    pub jobs: usize,
    pub geom_pending: usize,
    /// Objects whose full sync waits for a frame with time left.
    pub sync_backlog: usize,
    /// Worn objects placed by the GPU: not visited by the sync.
    pub anchored: usize,
}

/// Sums over the current period.
#[derive(Default)]
struct Period {
    frames: u32,
    /// Draw lists culled on the GPU in the last frame (gpu_cull.rs).
    gpu_cull: bool,
    /// Frames drawn by the render thread (last frame).
    threaded: bool,
    /// Main thread and render thread meeting: sums and longest of
    /// [packet, wait_render, rt_frame, rt_idle, pace, input] (ms).
    thread: [f32; THREAD_KEYS.len()],
    thread_max: [f32; THREAD_KEYS.len()],
    journal_bytes: u64,
    /// Frame times (ms), for the 95th percentile and the maximum.
    frame_ms: Vec<f32>,
    laps: [f32; LAPS],
    phases: [f32; PHASES],
    /// Longest time of each step in a single frame.
    laps_max: [f32; LAPS],
    /// Longest time of each step off the processor in a single frame.
    laps_off_max: [f32; LAPS],
    phases_off_max: [f32; PHASES],
    phases_max: [f32; PHASES],
    parts: Parts,
    parts_max: Parts,
    gpu: [f32; GPU],
    /// Depth prepasses alone: single-sample, multisampled.
    gpu_depth: [f32; 2],
    /// The ambient occlusion alone (inside g_effects).
    gpu_ssao: f32,
    /// The contact shadows alone (inside g_effects).
    gpu_contact: f32,
    /// The bloom levels alone (inside g_post).
    gpu_bloom: f32,
    /// The rest of g_effects: planar reflections, probe, scene copy, SSR.
    gpu_effects: [f32; 4],
    gpu_frames: u32,
    /// Parts of the shadows and depth GPU time (`ShadowStats::gpu_ms`).
    gpu_shadow: [f32; 5],
    gpu_shadow_frames: u32,
    /// Shadow casters by cascade: draws, those of the CPU lists, triangles
    /// of the GPU casters, rigged casters and their triangles.
    shadow: [[u64; 3]; 5],
    /// Shadow path of the period's frames (`shadow_cull::path`).
    shadow_path: u32,
    draws: u64,
    blend_triangles: u64,
    /// Frames whose blended faces dropped their invisible fragments before
    /// the lighting.
    blend_early: u32,
    lights: u64,
    probes: u64,
    /// Frames whose fragments walked the lights of their screen tile and
    /// depth only.
    light_tiles: u32,
    calls: u64,
    triangles: u64,
    shadow_draws: u64,
    particles: u64,
    occluded: u64,
    records_bytes: u64,
    palette_bytes: u64,
    cull_bytes: u64,
    vertex_bytes: u64,
    textures: u32,
    /// GPU memory of the textures and the geometry at the end (bytes).
    texture_bytes: u64,
    /// Texture pages and their allocated memory (free layers included).
    texture_pages: u32,
    texture_page_bytes: u64,
    geometry_bytes: u64,
    scene: [u64; 12],
    /// Interface: egui shapes, meshes and vertices, and the most vertices
    /// in a single frame.
    ui: [u64; 3],
    ui_vertices_max: u32,
}

pub struct FrameProfile {
    enabled: bool,
    /// Measured without a log line: the resource monitor is open and
    /// reads [`FrameProfile::averages`].
    live: bool,
    /// Averages per frame of the last period measured.
    averages: Option<Averages>,
    last: Option<Instant>,
    /// Laps of the frame being measured (ms).
    frame: [f32; LAPS],
    /// Cycles the thread ran in each lap of the frame being measured.
    frame_cycles: [u64; LAPS],
    last_cycles: u64,
    started: Option<Instant>,
    period: Period,
    /// Summaries written so far.
    periods: u64,
    /// Settings line last logged (logged again when they change).
    settings: String,
    /// Added to the summary lines: the variant an A/B run was on during
    /// the line's second (`xmode=`).
    mark: &'static str,
}

impl Default for FrameProfile {
    fn default() -> Self {
        FrameProfile::new(std::env::var_os("AURORA_PROFILE").is_some())
    }
}

impl FrameProfile {
    pub fn new(enabled: bool) -> Self {
        FrameProfile {
            enabled,
            live: false,
            averages: None,
            last: None,
            frame: [0.0; LAPS],
            frame_cycles: [0; LAPS],
            last_cycles: 0,
            started: None,
            period: Period::default(),
            periods: 0,
            settings: String::new(),
            mark: "",
        }
    }

    /// The variant the next summary lines are measured on (` | xmode=…`,
    /// or nothing).
    pub fn set_mark(&mut self, mark: &'static str) {
        self.mark = mark;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Measure the frames without AURORA_PROFILE (the resource monitor
    /// shows them); nothing is logged. Turned off, the averages are
    /// forgotten.
    pub fn set_live(&mut self, live: bool) {
        if live == self.live {
            return;
        }
        self.live = live;
        if !self.measuring() {
            self.averages = None;
            self.last = None;
            self.started = None;
            self.period = Period::default();
            self.frame = [0.0; LAPS];
            self.frame_cycles = [0; LAPS];
        }
    }

    fn measuring(&self) -> bool {
        self.enabled || self.live
    }

    /// Averages per frame of the last second measured (with
    /// AURORA_PROFILE or [`FrameProfile::set_live`]).
    pub fn averages(&self) -> Option<&Averages> {
        self.averages.as_ref()
    }

    /// AURORA_BLEND_EARLY_DISCARD=alt: the blended faces take the old path
    /// (invisible fragments lit, then dropped) during one summary in three,
    /// so that one run measures both paths on the same scene, second after
    /// second (`blend_early=` in the summary tells them apart). Three rather
    /// than two: work that comes back every other second would otherwise
    /// always fall on the same path.
    pub fn blend_late_discard(&self) -> bool {
        self.enabled && BlendDiscard::from_env() == BlendDiscard::Alternate && self.periods.is_multiple_of(3)
    }

    /// AURORA_LIGHT_TILES=alt: likewise, the fragments walk the whole light
    /// list during one summary in three (`light_tiles=` in the summary).
    pub fn all_lights(&self) -> bool {
        self.enabled && LightCulling::from_env() == LightCulling::Alternate && self.periods.is_multiple_of(3)
    }

    /// AURORA_PROBE_WALK=alt: likewise, the fragments walk the whole list
    /// of reflection probes during one summary in three (`xmode=old`).
    pub fn probe_walk_list(&self) -> bool {
        use aurora_render::probes::ProbeWalk;
        self.enabled && ProbeWalk::from_env() == ProbeWalk::Alternate && self.periods.is_multiple_of(3)
    }

    /// End a step of the frame now.
    pub fn lap(&mut self, lap: Lap) {
        if self.measuring() {
            self.lap_cpu_at(lap, Instant::now(), thread_cpu::cycles());
        }
    }

    /// A lap that ran the whole of its time (tests).
    #[cfg(test)]
    fn lap_at(&mut self, lap: Lap, now: Instant) {
        let ms = self.last.map_or(0.0, |last| (now - last).as_secs_f64() * 1000.0);
        let cycles = self.last_cycles + (ms * thread_cpu::cycles_per_ms()) as u64;
        self.lap_cpu_at(lap, now, cycles);
    }

    /// `cycles`: what the thread has run so far ([`thread_cpu::cycles`]).
    fn lap_cpu_at(&mut self, lap: Lap, now: Instant, cycles: u64) {
        if let Some(last) = self.last {
            self.frame[lap as usize] += (now - last).as_secs_f32() * 1000.0;
            self.frame_cycles[lap as usize] += cycles.wrapping_sub(self.last_cycles);
        }
        self.last = Some(now);
        self.last_cycles = cycles;
    }

    /// Log the settings that change the frame cost, when they change.
    pub fn settings(&mut self, line: impl FnOnce() -> String) {
        if !self.enabled {
            return;
        }
        let line = line();
        if line != self.settings {
            log::info!("perf settings: {line}");
            self.settings = line;
        }
    }

    /// End the frame (after the `Tail` lap); logs the summary once a
    /// second, and tells when it did.
    pub fn end_frame(&mut self, render: &RenderStats, scene: &SceneCounts, parts: &Parts) -> bool {
        if self.measuring()
            && let Some(line) = self.end_frame_at(render, scene, parts, Instant::now())
            && self.enabled
        {
            log::info!("perf summary: {line}{}", self.mark);
            return true;
        }
        false
    }

    fn end_frame_at(&mut self, render: &RenderStats, scene: &SceneCounts, parts: &Parts, now: Instant) -> Option<String> {
        let started = *self.started.get_or_insert(now);
        let frame = std::mem::replace(&mut self.frame, [0.0; LAPS]);
        let cycles = std::mem::replace(&mut self.frame_cycles, [0; LAPS]);
        let p = &mut self.period;
        p.frames += 1;
        p.frame_ms.push(frame.iter().sum());
        add(&mut p.laps, &frame);
        add(&mut p.phases, &render.cpu_phases);
        keep_max(&mut p.laps_max, &frame);
        keep_max(
            &mut p.laps_off_max,
            &std::array::from_fn(|i| thread_cpu::waited_ms(frame[i], cycles[i])),
        );
        keep_max(&mut p.phases_off_max, &render.cpu_waits);
        keep_max(&mut p.phases_max, &render.cpu_phases);
        add(&mut p.parts.0, &parts.0);
        keep_max(&mut p.parts_max.0, &parts.0);
        let thread = [
            render.packet_ms,
            render.wait_render_ms,
            render.thread_ms,
            render.thread_idle_ms,
            render.pace_ms,
            render.input_ms.unwrap_or(0.0),
        ];
        add(&mut p.thread, &thread);
        keep_max(&mut p.thread_max, &thread);
        p.threaded = render.render_thread;
        p.journal_bytes += render.journal_bytes;
        if let Some(g) = render.gpu_elements {
            add(&mut p.gpu, &g);
            p.gpu_frames += 1;
        }
        if let Some(d) = render.gpu_depth {
            add(&mut p.gpu_depth, &d);
        }
        p.gpu_ssao += render.gpu_ssao.unwrap_or(0.0);
        p.gpu_contact += render.gpu_contact.unwrap_or(0.0);
        p.gpu_bloom += render.gpu_bloom.unwrap_or(0.0);
        if let Some(e) = render.gpu_effects {
            add(&mut p.gpu_effects, &e);
        }
        p.draws += render.draws as u64;
        p.blend_triangles += render.blend_triangles;
        p.blend_early += render.blend_early_discard as u32;
        p.lights += render.lights as u64;
        p.probes += render.probes as u64;
        p.light_tiles += render.light_tiles as u32;
        p.calls += render.draw_calls as u64;
        p.triangles += render.triangles;
        p.shadow_draws += render.shadow_draws as u64;
        let sh = &render.shadow;
        if let Some(g) = sh.gpu_ms {
            add(&mut p.gpu_shadow, &g);
            p.gpu_shadow_frames += 1;
        }
        for (sum, v) in p
            .shadow
            .iter_mut()
            .zip([sh.draws, sh.cpu_draws, sh.triangles, sh.rigged_draws, sh.rigged_triangles])
        {
            for k in 0..3 {
                sum[k] += v[k] as u64;
            }
        }
        p.particles += render.particles as u64;
        p.occluded += render.occluded.unwrap_or(0) as u64;
        p.records_bytes += render.records_uploaded;
        p.palette_bytes += render.palettes_uploaded;
        p.cull_bytes += render.cull_uploaded;
        p.vertex_bytes += render.vertices_uploaded;
        p.textures = render.textures;
        p.texture_bytes = render.texture_bytes;
        p.texture_pages = render.texture_pages;
        p.texture_page_bytes = render.texture_page_bytes;
        p.geometry_bytes = render.geometry_bytes;
        p.gpu_cull = render.gpu_cull;
        let s = scene;
        for (sum, v) in p.scene.iter_mut().zip([
            s.objects,
            s.visible,
            s.synced,
            s.rebuilt,
            s.posed,
            s.blend,
            s.blend_glow,
            s.glow_alpha,
            s.jobs,
            s.geom_pending,
            s.sync_backlog,
            s.anchored,
        ]) {
            *sum += v as u64;
        }
        for (sum, v) in p.ui.iter_mut().zip([render.ui_shapes, render.ui_meshes, render.ui_vertices]) {
            *sum += v as u64;
        }
        p.ui_vertices_max = p.ui_vertices_max.max(render.ui_vertices);
        let elapsed = now - started;
        if elapsed < PERIOD {
            return None;
        }
        self.started = Some(now);
        self.periods += 1;
        let mut p = std::mem::take(&mut self.period);
        self.averages = Some(Averages::of(&p, elapsed));
        if !self.enabled {
            return Some(String::new());
        }
        // (with AURORA_SHADOW_LEGACY=ab, the next period compares the other path)
        p.shadow_path = aurora_render::shadow_cull::next_path();
        Some(summary(&p, elapsed))
    }
}

/// Averages per frame of one period (ms), for the resource monitor.
#[derive(Clone, Debug, PartialEq)]
pub struct Averages {
    pub fps: f32,
    pub frame_ms: f32,
    laps: [f32; LAPS],
    parts: [f32; PARTS],
    phases: [f32; PHASES],
    /// GPU elements, when the GPU timed them.
    pub gpu: Option<[f32; GPU]>,
    /// The main thread handing the frame's packet over (inside the
    /// `render` lap, whose rest waits for the render thread).
    pub packet_ms: f32,
}

impl Averages {
    fn of(p: &Period, elapsed: Duration) -> Averages {
        let n = p.frames.max(1) as f32;
        Averages {
            fps: p.frames as f32 / elapsed.as_secs_f32().max(1e-3),
            frame_ms: p.frame_ms.iter().sum::<f32>() / n,
            laps: p.laps.map(|v| v / n),
            parts: p.parts.0.map(|v| v / n),
            phases: p.phases.map(|v| v / n),
            gpu: (p.gpu_frames > 0).then(|| p.gpu.map(|v| v / p.gpu_frames as f32)),
            packet_ms: p.thread[0] / n,
        }
    }

    pub fn lap(&self, lap: Lap) -> f32 {
        self.laps[lap as usize]
    }

    pub fn part(&self, part: Part) -> f32 {
        self.parts[part as usize]
    }

    /// Averages made of the given steps (tests of their readers).
    #[cfg(test)]
    pub fn with(laps: &[(Lap, f32)], parts: &[(Part, f32)], phases: &[(&str, f32)], gpu: Option<[f32; GPU]>) -> Averages {
        let mut a = Averages {
            fps: 60.0,
            frame_ms: 1000.0 / 60.0,
            laps: [0.0; LAPS],
            parts: [0.0; PARTS],
            phases: [0.0; PHASES],
            gpu,
            packet_ms: 0.0,
        };
        for &(lap, ms) in laps {
            a.laps[lap as usize] = ms;
        }
        for &(part, ms) in parts {
            a.parts[part as usize] = ms;
        }
        for &(key, ms) in phases {
            if let Some(i) = RENDER_PHASES.iter().position(|k| *k == key) {
                a.phases[i] = ms;
            }
        }
        a
    }

    /// A step of the render thread, by its key in [`RENDER_PHASES`]
    /// (`shadows`, `scene_a`…); 0 for an unknown key.
    pub fn phase(&self, key: &str) -> f32 {
        RENDER_PHASES.iter().position(|k| *k == key).map_or(0.0, |i| self.phases[i])
    }
}

fn add<const N: usize>(sum: &mut [f32; N], v: &[f32; N]) {
    for (s, v) in sum.iter_mut().zip(v) {
        *s += v;
    }
}

fn keep_max<const N: usize>(max: &mut [f32; N], v: &[f32; N]) {
    for (m, v) in max.iter_mut().zip(v) {
        *m = m.max(*v);
    }
}

/// The value under which 95 % of the frame times fall.
fn p95(frames: &[f32]) -> f32 {
    if frames.is_empty() {
        return 0.0;
    }
    let mut v = frames.to_vec();
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) * 95).div_ceil(100)]
}

/// Frames slower than this many times the period's median count as slow
/// (`slow=` in the summary: the hitches the frame graph shows).
const SLOW_FACTOR: f32 = 1.5;

/// Number of frames over [`SLOW_FACTOR`] times the median frame time.
fn slow_frames(frames: &[f32]) -> usize {
    if frames.is_empty() {
        return 0;
    }
    let mut v = frames.to_vec();
    v.sort_by(f32::total_cmp);
    let median = v[v.len() / 2];
    frames.iter().filter(|&&f| f > median * SLOW_FACTOR).count()
}

/// The frame times of the period by whole millisecond, the empty ones left
/// out (`ft=8:118,9:1,14:1`: 118 frames of 8 to 9 ms…; 99 holds everything
/// from 99 ms on). Unlike `p95` and `max`, they add up over a run: its
/// p99 and p99.9 and its frames over 10 ms come from the sum.
fn frame_histogram(frames: &[f32]) -> String {
    let mut counts = std::collections::BTreeMap::<u32, u32>::new();
    for &f in frames {
        *counts.entry((f.max(0.0) as u32).min(99)).or_default() += 1;
    }
    let mut out = String::new();
    for (ms, n) in counts {
        if !out.is_empty() {
            out.push(',');
        }
        let _ = write!(out, "{ms}:{n}");
    }
    if out.is_empty() {
        out.push('-');
    }
    out
}

/// One line: averages per frame over the period (times in ms); renderer
/// steps prefixed with `r_`, streaming parts with `s_`, the parts of the
/// frame parameters with `p_`, interface parts with `u_` (followed by the
/// interface's counts), GPU elements with `g_`. The `max:` segment lists
/// the steps (`r_*`, `s_*`, `p_*`, `u_*` and the
/// `thread` segment's included) whose longest single-frame time reached
/// [`MAX_SHOWN_MS`], longest first (`-` when none did); `off_cpu:` does
/// the same for the time the steps of the two threads spent off the
/// processor, those that wait by design left out (`render`, `limiter`,
/// `r_acquire`, `r_present`). The `thread` segment comes before the
/// renderer steps it totals.
fn summary(p: &Period, elapsed: Duration) -> String {
    let n = p.frames.max(1) as f32;
    let avg = |v: f32| v / n;
    let count = |v: u64| v as f32 / n;
    let frame_avg = p.frame_ms.iter().sum::<f32>() / n;
    let max = p.frame_ms.iter().copied().fold(0.0f32, f32::max);
    let mut out = format!(
        "fps={:.1} frame={frame_avg:.2} p95={:.2} max={max:.2} slow={} ft={} |",
        p.frames as f32 / elapsed.as_secs_f32().max(1e-3),
        p95(&p.frame_ms),
        slow_frames(&p.frame_ms),
        frame_histogram(&p.frame_ms)
    );
    for (lap, ms) in Lap::ALL.iter().zip(p.laps) {
        let _ = write!(out, " {}={:.2}", lap.key(), avg(ms));
    }
    out.push_str(" | max:");
    let mut slow: Vec<(&str, &str, f32)> = Lap::ALL
        .iter()
        .zip(p.laps_max)
        .map(|(lap, ms)| ("", lap.key(), ms))
        .chain(RENDER_PHASES.iter().zip(p.phases_max).map(|(key, ms)| ("r_", *key, ms)))
        .chain(
            Part::ALL
                .iter()
                .zip(p.parts_max.0)
                .map(|(part, ms)| (part.prefix(), part.key(), ms)),
        )
        .chain(THREAD_KEYS.iter().zip(p.thread_max).map(|(key, ms)| ("", *key, ms)))
        .filter(|(_, _, ms)| *ms >= MAX_SHOWN_MS)
        .collect();
    slow.sort_by(|a, b| b.2.total_cmp(&a.2));
    if slow.is_empty() {
        out.push_str(" -");
    }
    for (prefix, key, ms) in slow {
        let _ = write!(out, " {prefix}{key}={ms:.2}");
    }
    out.push_str(" | off_cpu:");
    let mut waited: Vec<(&str, &str, f32)> = Lap::ALL
        .iter()
        .zip(p.laps_off_max)
        .filter(|(lap, _)| !matches!(lap, Lap::Render | Lap::Limiter))
        .map(|(lap, ms)| ("", lap.key(), ms))
        .chain(
            RENDER_PHASES
                .iter()
                .zip(p.phases_off_max)
                .filter(|(key, _)| !matches!(**key, "acquire" | "present"))
                .map(|(key, ms)| ("r_", *key, ms)),
        )
        .filter(|(_, _, ms)| *ms >= MAX_SHOWN_MS)
        .collect();
    waited.sort_by(|a, b| b.2.total_cmp(&a.2));
    if waited.is_empty() {
        out.push_str(" -");
    }
    for (prefix, key, ms) in waited {
        let _ = write!(out, " {prefix}{key}={ms:.2}");
    }
    let _ = write!(out, " | thread={}", if p.threaded { "on" } else { "off" });
    for (key, ms) in THREAD_KEYS.iter().zip(p.thread) {
        let _ = write!(out, " {key}={:.2}", avg(ms));
    }
    out.push_str(" |");
    for (key, ms) in RENDER_PHASES.iter().zip(p.phases) {
        let _ = write!(out, " r_{key}={:.2}", avg(ms));
    }
    let mut segment = "";
    for (part, ms) in Part::ALL.iter().zip(p.parts.0) {
        if part.prefix() != segment {
            segment = part.prefix();
            out.push_str(" |");
        }
        let _ = write!(out, " {segment}{}={:.2}", part.key(), avg(ms));
    }
    let _ = write!(
        out,
        " ui_shapes={:.0} ui_meshes={:.0} ui_vertices={:.0} ui_vertices_max={}",
        count(p.ui[0]),
        count(p.ui[1]),
        count(p.ui[2]),
        p.ui_vertices_max
    );
    let gpu_n = p.gpu_frames.max(1) as f32;
    let _ = write!(out, " | gpu={:.2}", p.gpu.iter().sum::<f32>() / gpu_n);
    for (el, ms) in GpuElement::ALL.iter().zip(p.gpu) {
        let _ = write!(out, " g_{}={:.3}", el.key(), ms / gpu_n);
    }
    // the rest of g_effects, beside g_ssao and g_contact
    for (key, ms) in ["planar", "probe", "copy", "ssr"].iter().zip(p.gpu_effects) {
        let _ = write!(out, " g_{key}={:.3}", ms / gpu_n);
    }
    // shadows and depth in detail: culling, cascades, prepass; then the
    // casters of each cascade
    let shadow_n = p.gpu_shadow_frames.max(1) as f32;
    for (key, ms) in ["cull", "shadow0", "shadow1", "shadow2", "depth"].iter().zip(p.gpu_shadow) {
        let _ = write!(out, " g_{key}={:.3}", ms / shadow_n);
    }
    // with MSAA, the prepass of the multisampled depth (inside g_shadows_depth too)
    let _ = write!(out, " g_depth_msaa={:.3}", p.gpu_depth[1] / gpu_n);
    // the ambient occlusion alone (inside g_effects)
    let _ = write!(out, " g_ssao={:.3}", p.gpu_ssao / gpu_n);
    // the contact shadows alone (inside g_effects)
    let _ = write!(out, " g_contact={:.3}", p.gpu_contact / gpu_n);
    // the bloom levels alone (inside g_post)
    let _ = write!(out, " g_bloom={:.3}", p.gpu_bloom / gpu_n);
    let _ = write!(out, " | shadow_path={}", aurora_render::shadow_cull::path_name(p.shadow_path));
    for (key, by_cascade, unit) in [
        ("sd", p.shadow[0], 1.0),
        ("sd_cpu", p.shadow[1], 1.0),
        ("st_k", p.shadow[2], 1000.0),
        ("sd_rig", p.shadow[3], 1.0),
        ("st_rig_k", p.shadow[4], 1000.0),
    ] {
        for (k, v) in by_cascade.iter().enumerate() {
            let _ = write!(out, " {key}{k}={:.0}", count(*v) / unit);
        }
    }
    let s = &p.scene;
    let _ = write!(
        out,
        " | draws={:.0} calls={:.0} tris_k={:.0} shadow_draws={:.0} particles={:.0} occluded={:.0} cull={} \
         blend={:.0} blend_tris_k={:.0} blend_early={:.2} blend_glow={:.0} glow_alpha={:.0} lights={:.0} light_tiles={:.2} probes={:.0} | objects={:.0} visible={:.0} synced={:.0} anchored={:.0} rebuilt={:.1} posed={:.1} \
         records_kb={:.1} palettes_kb={:.1} cull_kb={:.1} vertices_kb={:.1} journal_kb={:.1} | textures={} texture_mb={} texture_pages={} pages_mb={} geometry_mb={} \
         jobs={:.0} geom_pending={:.0} sync_backlog={:.0}",
        count(p.draws),
        count(p.calls),
        count(p.triangles) / 1000.0,
        count(p.shadow_draws),
        count(p.particles),
        count(p.occluded),
        if p.gpu_cull { "gpu" } else { "cpu" },
        count(s[5]),
        count(p.blend_triangles) / 1000.0,
        p.blend_early as f32 / n,
        count(s[6]),
        count(s[7]),
        count(p.lights),
        p.light_tiles as f32 / n,
        count(p.probes),
        count(s[0]),
        count(s[1]),
        count(s[2]),
        count(s[11]),
        count(s[3]),
        count(s[4]),
        count(p.records_bytes) / 1024.0,
        count(p.palette_bytes) / 1024.0,
        count(p.cull_bytes) / 1024.0,
        count(p.vertex_bytes) / 1024.0,
        count(p.journal_bytes) / 1024.0,
        p.textures,
        p.texture_bytes >> 20,
        p.texture_pages,
        p.texture_page_bytes >> 20,
        p.geometry_bytes >> 20,
        count(s[8]),
        count(s[9]),
        count(s[10]),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(m: u64) -> Duration {
        Duration::from_millis(m)
    }

    fn phase(key: &str) -> usize {
        RENDER_PHASES.iter().position(|k| *k == key).expect("a step of the renderer")
    }

    #[test]
    fn laps_cover_the_whole_frame() {
        let mut p = FrameProfile::new(true);
        let t = Instant::now();
        p.lap_at(Lap::Tail, t);
        p.lap_at(Lap::Between, t + ms(1));
        p.lap_at(Lap::Sync, t + ms(5));
        p.lap_at(Lap::Render, t + ms(12));
        p.lap_at(Lap::Tail, t + ms(20));
        assert_eq!(
            p.end_frame_at(&RenderStats::default(), &SceneCounts::default(), &Parts::default(), t + ms(20)),
            None
        );
        let f = &p.period.frame_ms;
        assert_eq!(f.len(), 1);
        assert!((f[0] - 20.0).abs() < 0.01, "{f:?}");
        assert!((p.period.laps[Lap::Sync as usize] - 4.0).abs() < 0.01);
        assert!((p.period.laps[Lap::Render as usize] - 7.0).abs() < 0.01);
    }

    #[test]
    fn summary_once_a_second_with_averages() {
        let mut p = FrameProfile::new(true);
        let t = Instant::now();
        let render = RenderStats {
            draws: 100,
            draw_calls: 40,
            records_uploaded: 2048,
            texture_bytes: 3 << 20,
            texture_pages: 12,
            texture_page_bytes: 5 << 20,
            ..Default::default()
        };
        let scene = SceneCounts {
            synced: 30,
            anchored: 7,
            posed: 2,
            ..Default::default()
        };
        let mut lines = Vec::new();
        for i in 0..=50u64 {
            p.lap_at(Lap::Sync, t + ms(i * 20));
            if let Some(l) = p.end_frame_at(&render, &scene, &Parts::default(), t + ms(i * 20)) {
                lines.push(l);
            }
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        assert!(l.starts_with("fps=51.0 frame=19.61"), "{l}");
        assert!(l.contains(" calls=40 "), "{l}");
        assert!(l.contains(" r_finish=0.00 ") && l.contains(" g_objects=0.000 "), "{l}");
        assert!(
            l.contains(" g_depth=0.000 g_depth_msaa=0.000 g_ssao=0.000 g_contact=0.000 g_bloom=0.000 | shadow_path="),
            "{l}"
        );
        assert!(l.contains(" synced=30 anchored=7 "), "{l}");
        assert!(l.contains(" posed=2.0 "), "{l}");
        assert!(l.contains(" records_kb=2.0 "), "{l}");
        assert!(
            l.contains(" | thread=off packet=0.00 wait_render=0.00 rt_frame=0.00 rt_idle=0.00 pace=0.00 input=0.00 | r_acquire="),
            "{l}"
        );
        assert!(l.contains(" texture_mb=3 texture_pages=12 pages_mb=5 "), "{l}");
    }

    /// What the blended faces draw and what a lit fragment may walk, and
    /// which of the compared paths drew the frames.
    #[test]
    fn summary_counts_the_blended_faces_and_the_lights() {
        let mut p = FrameProfile::new(true);
        let t = Instant::now();
        let scene = SceneCounts {
            blend: 318,
            blend_glow: 21,
            ..Default::default()
        };
        let mut lines = Vec::new();
        for i in 0..=50u64 {
            let render = RenderStats {
                blend_triangles: 250_000,
                blend_early_discard: i % 2 == 0,
                lights: 64,
                light_tiles: true,
                probes: 16,
                ..Default::default()
            };
            if let Some(l) = p.end_frame_at(&render, &scene, &Parts::default(), t + ms(i * 20)) {
                lines.push(l);
            }
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        // (26 of the 51 frames dropped the invisible fragments early)
        assert!(
            lines[0].contains(
                " blend=318 blend_tris_k=250 blend_early=0.51 blend_glow=21 glow_alpha=0 lights=64 light_tiles=1.00 probes=16 | objects="
            ),
            "{}",
            lines[0]
        );
        // the paths compared by `alt` change with the summaries, one in three
        assert_eq!(p.periods, 1);
    }

    #[test]
    fn summary_names_the_step_of_a_slow_frame() {
        let mut p = FrameProfile::new(true);
        let t = Instant::now();
        let mut render = RenderStats::default();
        let mut lines = Vec::new();
        let mut at = t;
        for i in 0..=50u64 {
            // one frame in 50 spends 6 ms in the media step, the others 0.1
            let media = if i == 25 { 6 } else { 0 };
            p.lap_at(Lap::Sync, at);
            at += ms(media) + Duration::from_micros(100);
            p.lap_at(Lap::Media, at);
            at += ms(20);
            p.lap_at(Lap::Render, at);
            render.cpu_phases[1] = if i == 10 { 1.5 } else { 0.2 };
            let mut parts = Parts::default();
            parts.add(Part::Upload, if i == 30 { 1.25 } else { 0.1 });
            parts.add(Part::SyncApply, 0.5);
            if let Some(l) = p.end_frame_at(&render, &SceneCounts::default(), &parts, at) {
                lines.push(l);
            }
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        let key = format!("r_{}", RENDER_PHASES[1]);
        let max = l.split(" | max: ").nth(1).and_then(|s| s.split(" |").next()).unwrap_or_default();
        assert!(max.starts_with("render=20.00 media=6.10 "), "{max}");
        assert!(max.ends_with(&format!(" {key}=1.50 s_upload=1.25")), "{max}");
        assert!(l.contains(" s_upload=0.12 "), "{l}");
        // the parts of the scene sync are listed with the streaming ones
        assert!(
            l.contains(" s_sync_list=0.00 s_sync_plan=0.00 s_sync_apply=0.50 s_sync_anchors=0.00 s_sync_alpha=0.00 "),
            "{l}"
        );
        assert!(l.ends_with(" sync_backlog=0"), "{l}");
        assert!(!max.contains("sync="), "{max}");
        // nothing over 1 ms
        let quiet = summary(&Period::default(), PERIOD);
        assert!(quiet.contains(" | max: - |"), "{quiet}");
    }

    #[test]
    fn summary_tells_the_two_threads_apart() {
        let mut p = FrameProfile::new(true);
        let t = Instant::now();
        let finish = phase("finish");
        let mut lines = Vec::new();
        let mut at = t;
        for i in 0..1000u64 {
            // main thread: 3 ms of simulation, then `render` = 0.1 ms of
            // packet + 1 ms waiting for the render thread; one frame of the
            // second waits 4 ms behind a slow render thread frame
            let wait = if i == 20 { 4.0 } else { 1.0 };
            p.lap_at(Lap::Tail, at);
            at += ms(3);
            p.lap_at(Lap::Sync, at);
            at += Duration::from_micros(100 + (wait * 1000.0) as u64);
            p.lap_at(Lap::Render, at);
            let mut render = RenderStats {
                render_thread: true,
                packet_ms: 0.1,
                wait_render_ms: wait,
                // render thread: 4 ms a frame, of which 2.5 in finish; it
                // never waits for the main thread here
                thread_ms: if i == 20 { 7.0 } else { 4.0 },
                thread_idle_ms: 0.0,
                journal_bytes: 4096,
                ..Default::default()
            };
            render.cpu_phases[finish] = 2.5;
            if let Some(l) = p.end_frame_at(&render, &SceneCounts::default(), &Parts::default(), at) {
                lines.push(l);
                break;
            }
        }
        // 245 frames of 4.1 ms (one of 7.1) in the second
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        // the main thread laps: `render` holds the packet and the wait
        assert!(l.contains(" sync=3.00 "), "{l}");
        assert!(l.contains(" render=1.11 "), "{l}");
        // the meeting of the two threads, then the render thread steps
        assert!(
            l.contains(" | thread=on packet=0.10 wait_render=1.01 rt_frame=4.01 rt_idle=0.00 pace=0.00 input=0.00 | r_acquire=0.00 "),
            "{l}"
        );
        assert!(l.contains(" r_finish=2.50 "), "{l}");
        assert!(l.contains(" journal_kb=4.0 "), "{l}");
        // the slow frame is named on both sides
        let max = l.split(" | max: ").nth(1).and_then(|s| s.split(" |").next()).unwrap_or_default();
        assert!(max.contains("rt_frame=7.00") && max.contains("wait_render=4.00"), "{max}");
        assert!(max.contains("render=4.10"), "{max}");
    }

    #[test]
    fn summary_details_the_interface_and_names_the_steps_that_waited() {
        let mut p = FrameProfile::new(true);
        let rate = thread_cpu::cycles_per_ms();
        let mut lines = Vec::new();
        let (mut at, mut cycles) = (Instant::now(), 0u64);
        for i in 0..=50u64 {
            p.lap_cpu_at(Lap::Sync, at, cycles);
            // the interface works 1 ms a frame; in one frame it also waits
            // 5 ms (for a lock, for a core)
            at += ms(if i == 7 { 6 } else { 1 });
            cycles += rate as u64;
            p.lap_cpu_at(Lap::Ui, at, cycles);
            // the limiter sleeps: a wait by design, not reported
            at += ms(19);
            p.lap_cpu_at(Lap::Limiter, at, cycles);
            let mut parts = Parts::default();
            parts.add(Part::UiTags, 0.25);
            parts.add(Part::ParamsPartSim, if i == 5 { 1.5 } else { 0.5 });
            parts.add(Part::UiPass, if i == 7 { 5.75 } else { 0.75 });
            // render thread: one `finish` waited 2.5 ms; the swapchain wait
            // of `acquire` is by design too
            let mut render = RenderStats {
                ui_shapes: 600,
                ui_meshes: 40,
                ui_vertices: if i == 3 { 90_000 } else { 20_000 },
                ..Default::default()
            };
            render.cpu_waits[phase("finish")] = if i == 9 { 2.5 } else { 0.0 };
            render.cpu_waits[phase("acquire")] = 8.0;
            if let Some(l) = p.end_frame_at(&render, &SceneCounts::default(), &parts, at) {
                lines.push(l);
            }
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        // the parts of the interface have their own segment, with its counts
        // so have the parts of the frame parameters, before it
        assert!(
            l.contains(" s_sync_alpha=0.00 | p_screen=0.00 p_env=0.00 p_mirror=0.00 p_part_sources=0.00 p_part_sim=0.52 "),
            "{l}"
        );
        assert!(l.contains(" p_lights=0.00 p_probes=0.00 | u_bars=0.00 u_tags=0.25 "), "{l}");
        assert!(
            l.contains(" u_pass=0.85 ui_shapes=600 ui_meshes=40 ui_vertices=21373 ui_vertices_max=90000 | gpu="),
            "{l}"
        );
        let segment = |name: &str| l.split(name).nth(1).and_then(|s| s.split(" |").next()).unwrap_or_default();
        assert!(segment(" | max: ").contains(" u_pass=5.75"), "{l}");
        assert!(segment(" | max: ").contains(" p_part_sim=1.50"), "{l}");
        // where the thread's CPU time is unknown, only the render thread's
        // waits (given here) are listed
        let waited = if rate > 0.0 { "ui=5.00 r_finish=2.50" } else { "r_finish=2.50" };
        assert_eq!(segment(" | off_cpu: "), waited, "{l}");
        let quiet = summary(&Period::default(), PERIOD);
        assert!(quiet.contains(" | off_cpu: - | thread=off "), "{quiet}");
    }

    #[test]
    fn p95_ignores_the_rare_slow_frames() {
        let mut v = vec![10.0; 99];
        v.push(100.0);
        assert_eq!(p95(&v), 10.0);
        assert_eq!(p95(&[]), 0.0);
        assert_eq!(p95(&[3.0]), 3.0);
    }

    #[test]
    fn slow_frames_count_the_hitches() {
        let mut v = vec![6.4; 98];
        v.extend([10.5, 10.0, 9.0]);
        assert_eq!(slow_frames(&v), 2);
        assert_eq!(slow_frames(&[6.4; 10]), 0);
        assert_eq!(slow_frames(&[]), 0);
    }

    #[test]
    fn frame_histogram_counts_by_whole_millisecond() {
        let mut v = vec![8.3; 118];
        v.extend([9.99, 14.2, 250.0, 120.0]);
        assert_eq!(frame_histogram(&v), "8:118,9:1,14:1,99:2");
        assert_eq!(frame_histogram(&[]), "-");
    }

    #[test]
    fn disabled_profile_records_nothing() {
        let mut p = FrameProfile::new(false);
        p.lap(Lap::Sync);
        p.end_frame(&RenderStats::default(), &SceneCounts::default(), &Parts::default());
        assert!(p.last.is_none());
        assert_eq!(p.period.frames, 0);
    }
}
