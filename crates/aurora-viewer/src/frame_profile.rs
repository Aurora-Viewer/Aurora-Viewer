//! Frame profile (AURORA_PROFILE=1): where the time of each frame goes, as
//! one summary line in the log every second. The laps of the frame loop
//! cover the whole time between two frames; the renderer adds its own
//! steps, the GPU time by element and counters (draws, synced objects,
//! posed avatars, bytes sent to the GPU). Besides the averages, the line
//! gives the longest time of each step over the second (`max:`), so that a
//! slow frame among fast ones can be pinned on the step that caused it.
//! Off without the variable.

use aurora_render::{GpuElement, RENDER_PHASES, RenderStats};
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
    /// `Scene::sync`: objects to GPU records.
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
    /// Interface input, build tools and selection outlines.
    UiPrep,
    /// The egui windows (`run_ui`).
    Ui,
    /// What is under the cursor: hover cursor, touch / sit targets.
    Hover,
    /// egui shapes to triangles.
    Tessellate,
    /// Interface actions, avatar pictures, maps.
    Actions,
    /// `Renderer::render`, swapchain wait included.
    Render,
    /// Frame limiter.
    Limiter,
    /// Captures and the performance window statistics.
    Tail,
}

impl Lap {
    const ALL: [Lap; 22] = [
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
        Lap::Tessellate,
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
            Lap::Tessellate => "tessellate",
            Lap::Actions => "actions",
            Lap::Render => "render",
            Lap::Limiter => "limiter",
            Lap::Tail => "tail",
        }
    }
}

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
}

/// Sums over the current period.
#[derive(Default)]
struct Period {
    frames: u32,
    /// Draw lists culled on the GPU in the last frame (gpu_cull.rs).
    gpu_cull: bool,
    /// Frame times (ms), for the 95th percentile and the maximum.
    frame_ms: Vec<f32>,
    laps: [f32; LAPS],
    phases: [f32; PHASES],
    /// Longest time of each step in a single frame.
    laps_max: [f32; LAPS],
    phases_max: [f32; PHASES],
    gpu: [f32; GPU],
    gpu_frames: u32,
    draws: u64,
    calls: u64,
    triangles: u64,
    shadow_draws: u64,
    particles: u64,
    occluded: u64,
    records_bytes: u64,
    palette_bytes: u64,
    textures: u32,
    /// GPU memory of the textures and the geometry at the end (bytes).
    texture_bytes: u64,
    /// Texture pages and their allocated memory (free layers included).
    texture_pages: u32,
    texture_page_bytes: u64,
    geometry_bytes: u64,
    scene: [u64; 10],
}

pub struct FrameProfile {
    enabled: bool,
    last: Option<Instant>,
    /// Laps of the frame being measured (ms).
    frame: [f32; LAPS],
    started: Option<Instant>,
    period: Period,
    /// Settings line last logged (logged again when they change).
    settings: String,
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
            last: None,
            frame: [0.0; LAPS],
            started: None,
            period: Period::default(),
            settings: String::new(),
        }
    }

    /// End a step of the frame now.
    pub fn lap(&mut self, lap: Lap) {
        if self.enabled {
            self.lap_at(lap, Instant::now());
        }
    }

    fn lap_at(&mut self, lap: Lap, now: Instant) {
        if let Some(last) = self.last {
            self.frame[lap as usize] += (now - last).as_secs_f32() * 1000.0;
        }
        self.last = Some(now);
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

    /// End the frame (after the `Tail` lap); logs the summary once a second.
    pub fn end_frame(&mut self, render: &RenderStats, scene: &SceneCounts) {
        if self.enabled
            && let Some(line) = self.end_frame_at(render, scene, Instant::now())
        {
            log::info!("perf summary: {line}");
        }
    }

    fn end_frame_at(&mut self, render: &RenderStats, scene: &SceneCounts, now: Instant) -> Option<String> {
        let started = *self.started.get_or_insert(now);
        let frame = std::mem::replace(&mut self.frame, [0.0; LAPS]);
        let p = &mut self.period;
        p.frames += 1;
        p.frame_ms.push(frame.iter().sum());
        add(&mut p.laps, &frame);
        add(&mut p.phases, &render.cpu_phases);
        keep_max(&mut p.laps_max, &frame);
        keep_max(&mut p.phases_max, &render.cpu_phases);
        if let Some(g) = render.gpu_elements {
            add(&mut p.gpu, &g);
            p.gpu_frames += 1;
        }
        p.draws += render.draws as u64;
        p.calls += render.draw_calls as u64;
        p.triangles += render.triangles;
        p.shadow_draws += render.shadow_draws as u64;
        p.particles += render.particles as u64;
        p.occluded += render.occluded.unwrap_or(0) as u64;
        p.records_bytes += render.records_uploaded;
        p.palette_bytes += render.palettes_uploaded;
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
        ]) {
            *sum += v as u64;
        }
        let elapsed = now - started;
        if elapsed < PERIOD {
            return None;
        }
        self.started = Some(now);
        let p = std::mem::take(&mut self.period);
        Some(summary(&p, elapsed))
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

/// One line: averages per frame over the period (times in ms); renderer
/// steps prefixed with `r_`, GPU elements with `g_`. The `max:` segment
/// lists the steps (`r_*` included) whose longest single-frame time reached
/// [`MAX_SHOWN_MS`], longest first (`-` when none did).
fn summary(p: &Period, elapsed: Duration) -> String {
    let n = p.frames.max(1) as f32;
    let avg = |v: f32| v / n;
    let count = |v: u64| v as f32 / n;
    let frame_avg = p.frame_ms.iter().sum::<f32>() / n;
    let max = p.frame_ms.iter().copied().fold(0.0f32, f32::max);
    let mut out = format!(
        "fps={:.1} frame={frame_avg:.2} p95={:.2} max={max:.2} slow={} |",
        p.frames as f32 / elapsed.as_secs_f32().max(1e-3),
        p95(&p.frame_ms),
        slow_frames(&p.frame_ms)
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
        .filter(|(_, _, ms)| *ms >= MAX_SHOWN_MS)
        .collect();
    slow.sort_by(|a, b| b.2.total_cmp(&a.2));
    if slow.is_empty() {
        out.push_str(" -");
    }
    for (prefix, key, ms) in slow {
        let _ = write!(out, " {prefix}{key}={ms:.2}");
    }
    out.push_str(" |");
    for (key, ms) in RENDER_PHASES.iter().zip(p.phases) {
        let _ = write!(out, " r_{key}={:.2}", avg(ms));
    }
    let gpu_n = p.gpu_frames.max(1) as f32;
    let _ = write!(out, " | gpu={:.2}", p.gpu.iter().sum::<f32>() / gpu_n);
    for (el, ms) in GpuElement::ALL.iter().zip(p.gpu) {
        let _ = write!(out, " g_{}={:.3}", el.key(), ms / gpu_n);
    }
    let s = &p.scene;
    let _ = write!(
        out,
        " | draws={:.0} calls={:.0} tris_k={:.0} shadow_draws={:.0} particles={:.0} occluded={:.0} cull={} \
         blend={:.0} blend_glow={:.0} glow_alpha={:.0} | objects={:.0} visible={:.0} synced={:.0} rebuilt={:.1} posed={:.1} \
         records_kb={:.1} palettes_kb={:.1} | textures={} texture_mb={} texture_pages={} pages_mb={} geometry_mb={} jobs={:.0} geom_pending={:.0}",
        count(p.draws),
        count(p.calls),
        count(p.triangles) / 1000.0,
        count(p.shadow_draws),
        count(p.particles),
        count(p.occluded),
        if p.gpu_cull { "gpu" } else { "cpu" },
        count(s[5]),
        count(s[6]),
        count(s[7]),
        count(s[0]),
        count(s[1]),
        count(s[2]),
        count(s[3]),
        count(s[4]),
        count(p.records_bytes) / 1024.0,
        count(p.palette_bytes) / 1024.0,
        p.textures,
        p.texture_bytes >> 20,
        p.texture_pages,
        p.texture_page_bytes >> 20,
        p.geometry_bytes >> 20,
        count(s[8]),
        count(s[9]),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(m: u64) -> Duration {
        Duration::from_millis(m)
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
        assert_eq!(p.end_frame_at(&RenderStats::default(), &SceneCounts::default(), t + ms(20)), None);
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
            posed: 2,
            ..Default::default()
        };
        let mut lines = Vec::new();
        for i in 0..=50u64 {
            p.lap_at(Lap::Sync, t + ms(i * 20));
            if let Some(l) = p.end_frame_at(&render, &scene, t + ms(i * 20)) {
                lines.push(l);
            }
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        assert!(l.starts_with("fps=51.0 frame=19.61"), "{l}");
        assert!(l.contains(" calls=40 "), "{l}");
        assert!(l.contains(" r_finish=0.00 ") && l.contains(" g_objects=0.000 "), "{l}");
        assert!(l.contains(" synced=30 "), "{l}");
        assert!(l.contains(" posed=2.0 "), "{l}");
        assert!(l.contains(" records_kb=2.0 "), "{l}");
        assert!(l.contains(" texture_mb=3 texture_pages=12 pages_mb=5 "), "{l}");
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
            if let Some(l) = p.end_frame_at(&render, &SceneCounts::default(), at) {
                lines.push(l);
            }
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        let l = &lines[0];
        let key = format!("r_{}", RENDER_PHASES[1]);
        let max = l.split(" | max: ").nth(1).and_then(|s| s.split(" |").next()).unwrap_or_default();
        assert!(max.starts_with("render=20.00 media=6.10 "), "{max}");
        assert!(max.ends_with(&format!(" {key}=1.50")), "{max}");
        assert!(!max.contains("sync="), "{max}");
        // nothing over 1 ms
        let quiet = summary(&Period::default(), PERIOD);
        assert!(quiet.contains(" | max: - |"), "{quiet}");
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
    fn disabled_profile_records_nothing() {
        let mut p = FrameProfile::new(false);
        p.lap(Lap::Sync);
        p.end_frame(&RenderStats::default(), &SceneCounts::default());
        assert!(p.last.is_none());
        assert_eq!(p.period.frames, 0);
    }
}
