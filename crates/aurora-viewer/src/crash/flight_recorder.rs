//! Fixed-size, low-cost history of the last minute before a GPU incident.
//! Samples contain only numeric metrics and closed enums; never user text.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const SAMPLE_CAP: usize = 600;
const EVENT_CAP: usize = 128;
const SAMPLE_INTERVAL_MS: u64 = 100;
const METRIC_COUNT: usize = 15;
const WINDOW_SECONDS_MS: u64 = 60_000;
const RECENT_SECONDS_MS: u64 = 5_000;
const MAX_DUMP_BYTES: u64 = 512 * 1024;

static RECORDER: OnceLock<Mutex<Recorder>> = OnceLock::new();

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    #[default]
    SessionEnd,
    GpuLoss,
    GpuStall,
    GpuRecovery,
    Panic,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Unknown,
    Login,
    Loading,
    World,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    #[default]
    Unknown,
    Focused,
    Background,
    Minimized,
    Occluded,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EventKind {
    #[default]
    ScreenChanged,
    RegionChanged,
    GraphicsPresetChanged,
    EffectsChanged,
    SwapchainRecreated,
    PhotoChanged,
    PoserChanged,
    TeleportStarted,
    TeleportEnded,
    OverlayHookSeen,
    SafeGraphicsApplied,
    GpuStall,
    GpuRecovered,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq)]
struct Sample {
    at_ms: u64,
    values: [Option<f32>; METRIC_COUNT],
    passes: [Pass; 3],
    window: WindowState,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq)]
struct Pass {
    name: PassName,
    ms: Option<f32>,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PassName {
    #[default]
    Unknown,
    ShadowsDepth,
    Effects,
    TerrainSky,
    Objects,
    Water,
    Transparent,
    Post,
}

impl PassName {
    fn from_key(key: &str) -> Self {
        match key {
            "shadows_depth" => Self::ShadowsDepth,
            "effects" => Self::Effects,
            "terrain_sky" => Self::TerrainSky,
            "objects" => Self::Objects,
            "water" => Self::Water,
            "transparent" => Self::Transparent,
            "post" => Self::Post,
            _ => Self::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::ShadowsDepth => "shadows/depth",
            Self::Effects => "effects",
            Self::TerrainSky => "terrain/sky",
            Self::Objects => "objects",
            Self::Water => "water",
            Self::Transparent => "transparent",
            Self::Post => "post",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct Event {
    at_ms: u64,
    kind: EventKind,
    from: u32,
    to: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Dump {
    schema: u32,
    trigger: Trigger,
    end_ms: u64,
    samples: Vec<Sample>,
    events: Vec<Event>,
}

/// Values copied from objects that already track frame and scene state.
/// `region_token` is compared in memory only; it is never copied to a sample.
#[derive(Debug, Clone, Copy, Default)]
pub struct SampleInput {
    pub cpu_frame_ms: Option<f32>,
    pub gpu_frame_ms: Option<f32>,
    pub gpu_passes: [Option<(&'static str, f32)>; 3],
    pub vram_used_mb: Option<u64>,
    pub vram_budget_mb: Option<u64>,
    pub texture_pages: Option<u32>,
    pub texture_pressure: Option<u32>,
    pub texture_pressure_max: Option<u32>,
    /// Not exposed by wgpu 30.0.1; remains absent until Aurora owns a
    /// backend-neutral counter for these resource allocations.
    pub bind_group_rebuilds: Option<u64>,
    /// wgpu does not expose the driver's descriptor-pool occupancy.
    pub descriptor_sets_in_use: Option<u64>,
    /// The indirect command buffer is GPU-written and is not read back per
    /// frame; no readback is added to the render path for this diagnostic.
    pub max_indirect_index_count: Option<u32>,
    pub pipelines_compiling: Option<u32>,
    pub uploads_in_flight: Option<u32>,
    pub swapchain_recreations: Option<u64>,
    pub rss_mb: Option<u64>,
    pub screen: Stage,
    pub region_token: Option<u64>,
    pub graphics_preset: u8,
    pub effects_mask: u32,
    pub photo_open: bool,
    pub poser_open: bool,
    pub window: WindowState,
}

#[derive(Debug)]
struct Recorder {
    origin: Option<Instant>,
    last_sample_ms: Option<u64>,
    samples: [Sample; SAMPLE_CAP],
    sample_head: usize,
    sample_len: usize,
    events: [Event; EVENT_CAP],
    event_head: usize,
    event_len: usize,
    last_stage: Stage,
    last_region: Option<u64>,
    last_preset: Option<u8>,
    last_effects: Option<u32>,
    last_ui: Option<u8>,
    last_swapchain_total: Option<u64>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            origin: None,
            last_sample_ms: None,
            samples: std::array::from_fn(|_| Sample::default()),
            sample_head: 0,
            sample_len: 0,
            events: std::array::from_fn(|_| Event::default()),
            event_head: 0,
            event_len: 0,
            last_stage: Stage::Unknown,
            last_region: None,
            last_preset: None,
            last_effects: None,
            last_ui: None,
            last_swapchain_total: None,
        }
    }
}

impl Recorder {
    fn elapsed_ms(&mut self, now: Instant) -> u64 {
        let origin = *self.origin.get_or_insert(now);
        now.saturating_duration_since(origin).as_millis().min(u128::from(u64::MAX)) as u64
    }

    fn sample_at(&mut self, at_ms: u64, input: SampleInput) {
        if self
            .last_sample_ms
            .is_some_and(|last| at_ms.saturating_sub(last) < SAMPLE_INTERVAL_MS)
        {
            return;
        }
        self.last_sample_ms = Some(at_ms);
        let swapchain_delta = input.swapchain_recreations.map(|total| {
            let delta = self.last_swapchain_total.map_or(0, |previous| total.saturating_sub(previous));
            self.last_swapchain_total = Some(total);
            delta
        });
        self.observe_events(at_ms, input, swapchain_delta);
        let mut values = [None; METRIC_COUNT];
        values[0] = finite(input.cpu_frame_ms);
        values[1] = finite(input.gpu_frame_ms);
        values[2] = input.vram_used_mb.map(|value| value as f32);
        values[3] = input.vram_budget_mb.map(|value| value as f32);
        values[4] = input
            .vram_used_mb
            .zip(input.vram_budget_mb)
            .map(|(used, budget)| budget.saturating_sub(used) as f32);
        values[5] = input.texture_pages.map(|value| value as f32);
        values[6] = input.texture_pressure.map(|value| value as f32);
        values[7] = input.texture_pressure_max.map(|value| value as f32);
        values[8] = input.bind_group_rebuilds.map(|value| value as f32);
        values[9] = input.descriptor_sets_in_use.map(|value| value as f32);
        values[10] = input.max_indirect_index_count.map(|value| value as f32);
        values[11] = input.pipelines_compiling.map(|value| value as f32);
        values[12] = input.uploads_in_flight.map(|value| value as f32);
        values[13] = swapchain_delta.map(|delta| delta as f32);
        values[14] = input.rss_mb.map(|value| value as f32);
        let mut passes = [Pass::default(); 3];
        for (pass, option) in passes.iter_mut().zip(input.gpu_passes) {
            if let Some((name, ms)) = pass_option(option) {
                *pass = Pass {
                    name: PassName::from_key(name),
                    ms: Some(ms),
                };
            }
        }
        self.samples[self.sample_head] = Sample {
            at_ms,
            values,
            passes,
            window: input.window,
        };
        self.sample_head = (self.sample_head + 1) % SAMPLE_CAP;
        self.sample_len = (self.sample_len + 1).min(SAMPLE_CAP);
    }

    fn sample_if_due(&mut self, at_ms: u64, input: impl FnOnce() -> SampleInput) {
        if self
            .last_sample_ms
            .is_some_and(|last| at_ms.saturating_sub(last) < SAMPLE_INTERVAL_MS)
        {
            return;
        }
        self.sample_at(at_ms, input());
    }

    fn observe_events(&mut self, at_ms: u64, input: SampleInput, swapchain_delta: Option<u64>) {
        if input.screen != self.last_stage {
            self.push_event(at_ms, EventKind::ScreenChanged, self.last_stage as u32, input.screen as u32);
            self.last_stage = input.screen;
        }
        if self.last_region.is_some() && input.region_token != self.last_region {
            self.push_event(at_ms, EventKind::RegionChanged, 0, 1);
        }
        self.last_region = input.region_token;
        if let Some(previous) = self.last_preset
            && previous != input.graphics_preset
        {
            self.push_event(
                at_ms,
                EventKind::GraphicsPresetChanged,
                u32::from(previous),
                u32::from(input.graphics_preset),
            );
        }
        self.last_preset = Some(input.graphics_preset);
        if let Some(previous) = self.last_effects
            && previous != input.effects_mask
        {
            self.push_event(at_ms, EventKind::EffectsChanged, previous, input.effects_mask);
        }
        self.last_effects = Some(input.effects_mask);
        let ui = u8::from(input.photo_open) | (u8::from(input.poser_open) << 1);
        if let Some(previous) = self.last_ui
            && previous != ui
        {
            self.push_event(at_ms, EventKind::PhotoChanged, u32::from(previous & 1), u32::from(ui & 1));
            self.push_event(at_ms, EventKind::PoserChanged, u32::from(previous >> 1), u32::from(ui >> 1));
        }
        self.last_ui = Some(ui);
        if swapchain_delta.is_some_and(|delta| delta > 0) {
            self.push_event(at_ms, EventKind::SwapchainRecreated, 0, 1);
        }
    }

    fn push_event(&mut self, at_ms: u64, kind: EventKind, from: u32, to: u32) {
        self.events[self.event_head] = Event { at_ms, kind, from, to };
        self.event_head = (self.event_head + 1) % EVENT_CAP;
        self.event_len = (self.event_len + 1).min(EVENT_CAP);
    }

    fn event(&mut self, kind: EventKind) {
        let at_ms = self.origin.map_or(0, |origin| {
            Instant::now()
                .saturating_duration_since(origin)
                .as_millis()
                .min(u128::from(u64::MAX)) as u64
        });
        self.push_event(at_ms, kind, 0, 1);
    }

    fn dump(&self, trigger: Trigger, end_ms: u64) -> Dump {
        let samples = self.ordered_samples();
        let events = self.ordered_events();
        Dump {
            schema: 1,
            trigger,
            end_ms,
            samples,
            events,
        }
    }

    fn ordered_samples(&self) -> Vec<Sample> {
        let start = (self.sample_head + SAMPLE_CAP - self.sample_len) % SAMPLE_CAP;
        (0..self.sample_len)
            .map(|offset| self.samples[(start + offset) % SAMPLE_CAP])
            .collect()
    }

    fn ordered_events(&self) -> Vec<Event> {
        let start = (self.event_head + EVENT_CAP - self.event_len) % EVENT_CAP;
        (0..self.event_len)
            .map(|offset| self.events[(start + offset) % EVENT_CAP])
            .collect()
    }
}

fn finite(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite())
}

fn pass_option(option: Option<(&'static str, f32)>) -> Option<(&'static str, f32)> {
    option.filter(|(_, ms)| ms.is_finite())
}

fn recorder() -> &'static Mutex<Recorder> {
    RECORDER.get_or_init(|| Mutex::new(Recorder::default()))
}

pub fn start() {
    let mut recorder = recorder().lock().unwrap_or_else(|e| e.into_inner());
    if recorder.origin.is_none() {
        recorder.origin = Some(Instant::now());
    }
}

/// Build the frame sample only when the 10 Hz recorder cadence is due. In
/// particular this keeps OS memory queries and renderer snapshots off the
/// other nine frames in each interval.
pub fn sample_if_due(input: impl FnOnce() -> SampleInput) {
    let mut recorder = recorder().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let at_ms = recorder.elapsed_ms(now);
    recorder.sample_if_due(at_ms, input);
}

fn note_event(kind: EventKind) {
    if let Ok(mut recorder) = recorder().try_lock() {
        recorder.event(kind);
    }
}

pub fn teleport_started() {
    note_event(EventKind::TeleportStarted);
}

pub fn teleport_ended() {
    note_event(EventKind::TeleportEnded);
}

pub fn overlay_hook_seen() {
    note_event(EventKind::OverlayHookSeen);
}

pub fn safe_graphics_applied() {
    note_event(EventKind::SafeGraphicsApplied);
}

pub fn gpu_stall() {
    note_event(EventKind::GpuStall);
}

pub fn gpu_stall_recovered() {
    note_event(EventKind::GpuRecovered);
}

pub fn gpu_recovered() {
    note_event(EventKind::GpuRecovered);
}

pub fn capture(dir: &Path, trigger: Trigger) {
    let Ok(mut recorder) = recorder().try_lock() else { return };
    let end_ms = recorder.elapsed_ms(Instant::now());
    let dump = recorder.dump(trigger, end_ms);
    drop(recorder);
    if let Ok(bytes) = serde_json::to_vec(&dump)
        && bytes.len() as u64 <= MAX_DUMP_BYTES
        && let Err(error) = crate::atomic_file::publish(&dir.join("flight-recorder.json"), &bytes)
    {
        log::warn!("flight recorder snapshot was not published: {error}");
    }
}

pub fn report_from_json(json: &str) -> Option<String> {
    let dump = serde_json::from_str::<Dump>(json).ok()?;
    (dump.schema == 1).then(|| report(&dump))
}

#[derive(Clone, Copy)]
enum Metric {
    CpuFrame,
    GpuFrame,
    VramUsed,
    VramBudget,
    VramHeadroom,
    TexturePages,
    TexturePressure,
    TexturePressureMax,
    BindGroupRebuilds,
    DescriptorSets,
    MaxIndirectIndex,
    PipelinesCompiling,
    UploadsInFlight,
    SwapchainRecreations,
    Rss,
}

impl Metric {
    const ALL: [Self; METRIC_COUNT] = [
        Self::CpuFrame,
        Self::GpuFrame,
        Self::VramUsed,
        Self::VramBudget,
        Self::VramHeadroom,
        Self::TexturePages,
        Self::TexturePressure,
        Self::TexturePressureMax,
        Self::BindGroupRebuilds,
        Self::DescriptorSets,
        Self::MaxIndirectIndex,
        Self::PipelinesCompiling,
        Self::UploadsInFlight,
        Self::SwapchainRecreations,
        Self::Rss,
    ];

    fn index(self) -> usize {
        match self {
            Self::CpuFrame => 0,
            Self::GpuFrame => 1,
            Self::VramUsed => 2,
            Self::VramBudget => 3,
            Self::VramHeadroom => 4,
            Self::TexturePages => 5,
            Self::TexturePressure => 6,
            Self::TexturePressureMax => 7,
            Self::BindGroupRebuilds => 8,
            Self::DescriptorSets => 9,
            Self::MaxIndirectIndex => 10,
            Self::PipelinesCompiling => 11,
            Self::UploadsInFlight => 12,
            Self::SwapchainRecreations => 13,
            Self::Rss => 14,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::CpuFrame => "CPU frame ms",
            Self::GpuFrame => "GPU frame ms",
            Self::VramUsed => "VRAM used MB",
            Self::VramBudget => "VRAM budget MB",
            Self::VramHeadroom => "VRAM headroom MB",
            Self::TexturePages => "Texture pages",
            Self::TexturePressure => "Texture pressure step",
            Self::TexturePressureMax => "Texture pressure maximum",
            Self::BindGroupRebuilds => "Texture bind-group rebuilds",
            Self::DescriptorSets => "Descriptor sets in use",
            Self::MaxIndirectIndex => "Max indirect index_count",
            Self::PipelinesCompiling => "Pipelines compiling",
            Self::UploadsInFlight => "Uploads in flight",
            Self::SwapchainRecreations => "Swapchain recreations",
            Self::Rss => "Process RSS MB",
        }
    }
}

fn report(dump: &Dump) -> String {
    use std::fmt::Write;
    let trigger = match dump.trigger {
        Trigger::SessionEnd => "session end",
        Trigger::GpuLoss => "GPU loss",
        Trigger::GpuStall => "GPU stall",
        Trigger::GpuRecovery => "GPU recovery",
        Trigger::Panic => "panic",
    };
    let mut out = format!("Flight recorder (60 seconds before {trigger})\n");
    let mut events = dump.events.iter().filter(|event| event.at_ms <= dump.end_ms).collect::<Vec<_>>();
    events.sort_by_key(|event| event.at_ms);
    if events.is_empty() {
        out.push_str("Timeline: no events recorded\n");
    } else {
        out.push_str("Timeline:\n");
        for event in events.into_iter().rev().take(24).rev() {
            let seconds = dump.end_ms.saturating_sub(event.at_ms) as f32 / 1000.0;
            let _ = writeln!(out, "- {seconds:.1}s before: {}", event_label(*event));
        }
    }
    out.push_str("Metrics (min / max / last, 60s; then 5s):\n");
    for metric in Metric::ALL {
        let long = metric_summary(dump, metric, WINDOW_SECONDS_MS);
        let short = metric_summary(dump, metric, RECENT_SECONDS_MS);
        let _ = writeln!(out, "- {}: {long}; {short}", metric.label());
    }
    for pass_index in 0..3 {
        let label = dump
            .samples
            .iter()
            .rev()
            .find_map(|sample| {
                let name = sample.passes.get(pass_index)?.name;
                (name != PassName::Unknown).then_some(name.label())
            })
            .unwrap_or("not collected");
        let long = pass_summary(dump, pass_index, WINDOW_SECONDS_MS);
        let short = pass_summary(dump, pass_index, RECENT_SECONDS_MS);
        let _ = writeln!(out, "- GPU pass {} ({label}): {long}; {short}", pass_index + 1);
    }
    let window = dump
        .samples
        .iter()
        .rev()
        .find(|sample| sample.at_ms <= dump.end_ms)
        .map_or("not collected", |sample| window_label(sample.window));
    let _ = writeln!(out, "- Window state at last sample: {window}");
    out
}

fn metric_summary(dump: &Dump, metric: Metric, span_ms: u64) -> String {
    let values = dump
        .samples
        .iter()
        .filter(|sample| sample.at_ms <= dump.end_ms && dump.end_ms.saturating_sub(sample.at_ms) <= span_ms)
        .filter_map(|sample| sample.values[metric.index()]);
    summarize(values)
}

fn pass_summary(dump: &Dump, index: usize, span_ms: u64) -> String {
    let values = dump
        .samples
        .iter()
        .filter(|sample| sample.at_ms <= dump.end_ms && dump.end_ms.saturating_sub(sample.at_ms) <= span_ms)
        .filter_map(|sample| sample.passes.get(index).and_then(|pass| pass.ms));
    summarize(values)
}

fn summarize(values: impl Iterator<Item = f32>) -> String {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut last = None;
    for value in values.filter(|value| value.is_finite()) {
        min = min.min(value);
        max = max.max(value);
        last = Some(value);
    }
    match last {
        Some(last) => format!("{min:.1} / {max:.1} / {last:.1}"),
        None => "not collected".to_owned(),
    }
}

fn event_label(event: Event) -> String {
    match event.kind {
        EventKind::ScreenChanged => format!("login stage: {} → {}", stage_label(event.from), stage_label(event.to)),
        EventKind::RegionChanged => "region changed".to_owned(),
        EventKind::GraphicsPresetChanged => {
            format!(
                "graphics preset: {} → {}",
                preset_label(event.from as u8),
                preset_label(event.to as u8)
            )
        }
        EventKind::EffectsChanged => format!("graphics effects: {} → {}", effect_labels(event.from), effect_labels(event.to)),
        EventKind::SwapchainRecreated => "swapchain recreated".to_owned(),
        EventKind::PhotoChanged => format!("photo mode {}", on_off(event.to)),
        EventKind::PoserChanged => format!("poser {}", on_off(event.to)),
        EventKind::TeleportStarted => "teleport started".to_owned(),
        EventKind::TeleportEnded => "teleport ended".to_owned(),
        EventKind::OverlayHookSeen => "overlay hook observed".to_owned(),
        EventKind::SafeGraphicsApplied => "safe graphics applied".to_owned(),
        EventKind::GpuStall => "GPU stall notice".to_owned(),
        EventKind::GpuRecovered => "GPU recovered".to_owned(),
    }
}

fn on_off(value: u32) -> &'static str {
    if value == 0 { "closed" } else { "opened" }
}

fn stage_label(value: u32) -> &'static str {
    match value {
        1 => "login",
        2 => "loading",
        3 => "world",
        _ => "unknown",
    }
}

fn preset_label(value: u8) -> &'static str {
    match value {
        1 => "potato",
        2 => "low",
        3 => "medium",
        4 => "high",
        5 => "deluxe",
        _ => "custom",
    }
}

fn effect_labels(mask: u32) -> String {
    const EFFECTS: &[(&str, u32)] = &[
        ("shadows", 1 << 0),
        ("contact shadows", 1 << 1),
        ("ambient occlusion", 1 << 2),
        ("SSR", 1 << 3),
        ("bloom", 1 << 4),
        ("glow", 1 << 5),
        ("indirect light", 1 << 6),
        ("probes", 1 << 7),
        ("water reflections", 1 << 8),
        ("mirrors", 1 << 9),
        ("HDR emissive", 1 << 10),
        ("VR", 1 << 11),
    ];
    let labels = EFFECTS
        .iter()
        .filter_map(|(label, bit)| (mask & bit != 0).then_some(*label))
        .collect::<Vec<_>>();
    if labels.is_empty() { "none".to_owned() } else { labels.join(", ") }
}

fn window_label(value: WindowState) -> &'static str {
    match value {
        WindowState::Unknown => "not collected",
        WindowState::Focused => "focused",
        WindowState::Background => "background",
        WindowState::Minimized => "minimized",
        WindowState::Occluded => "occluded",
    }
}

pub fn read_report(dir: &Path) -> Option<String> {
    let text = crate::crash::read_limited(&dir.join("flight-recorder.json"), MAX_DUMP_BYTES).ok()?;
    report_from_json(&text)
}

/// Current resident memory without allocation on the frame path.
#[cfg(target_os = "linux")]
pub fn current_rss_mb() -> Option<u64> {
    let fd = unsafe { libc::open(c"/proc/self/statm".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let mut bytes = [0u8; 64];
    let length = unsafe { libc::read(fd, bytes.as_mut_ptr().cast(), bytes.len()) };
    unsafe { libc::close(fd) };
    if length <= 0 {
        return None;
    }
    let bytes = &bytes[..length as usize];
    let mut fields = bytes.split(|byte| byte.is_ascii_whitespace()).filter(|field| !field.is_empty());
    let _total_pages = fields.next()?;
    let resident_pages = parse_u64(fields.next()?)?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    (page_size > 0).then(|| resident_pages.saturating_mul(page_size as u64) >> 20)
}

#[cfg(windows)]
pub fn current_rss_mb() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let size = counters.cb;
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (ok != 0).then_some(counters.WorkingSetSize as u64 >> 20)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn current_rss_mb() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn parse_u64(bytes: &[u8]) -> Option<u64> {
    let mut value = 0u64;
    if bytes.is_empty() {
        return None;
    }
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn input(gpu: f32, cpu: f32) -> SampleInput {
        SampleInput {
            cpu_frame_ms: Some(cpu),
            gpu_frame_ms: Some(gpu),
            gpu_passes: [Some(("effects", gpu)), None, None],
            ..Default::default()
        }
    }

    #[test]
    fn ring_keeps_only_the_last_six_hundred_samples_and_orders_them() {
        let mut recorder = Recorder::default();
        for index in 0..SAMPLE_CAP + 2 {
            recorder.sample_at(index as u64 * SAMPLE_INTERVAL_MS, input(index as f32, index as f32));
        }
        let dump = recorder.dump(Trigger::GpuLoss, (SAMPLE_CAP + 1) as u64 * SAMPLE_INTERVAL_MS);
        assert_eq!(dump.samples.len(), SAMPLE_CAP);
        assert_eq!(dump.samples[0].values[1], Some(2.0));
        assert_eq!(dump.samples[SAMPLE_CAP - 1].values[1], Some(601.0));
    }

    #[test]
    fn samples_are_rate_limited_to_ten_hz() {
        let mut recorder = Recorder::default();
        recorder.sample_at(0, input(1.0, 1.0));
        recorder.sample_at(99, input(2.0, 2.0));
        recorder.sample_at(100, input(3.0, 3.0));
        assert_eq!(recorder.sample_len, 2);
        assert_eq!(recorder.samples[1].values[1], Some(3.0));
    }

    #[test]
    fn throttled_sample_does_not_collect_renderer_or_os_metrics() {
        let mut recorder = Recorder::default();
        recorder.sample_at(0, input(1.0, 1.0));
        let mut collected = false;
        recorder.sample_if_due(99, || {
            collected = true;
            input(2.0, 2.0)
        });
        assert!(!collected);
        recorder.sample_if_due(100, || {
            collected = true;
            input(3.0, 3.0)
        });
        assert!(collected);
        assert_eq!(recorder.sample_len, 2);
    }

    #[test]
    fn event_ring_caps_and_orders_events() {
        let mut recorder = Recorder::default();
        for at_ms in 0..EVENT_CAP + 2 {
            recorder.push_event(at_ms as u64, EventKind::TeleportStarted, 0, 1);
        }
        let dump = recorder.dump(Trigger::GpuStall, 200);
        assert_eq!(dump.events.len(), EVENT_CAP);
        assert_eq!(dump.events[0].at_ms, 2);
        assert_eq!(dump.events[EVENT_CAP - 1].at_ms, 129);
    }

    #[test]
    fn summary_uses_the_correct_minimum_maximum_and_last_for_each_window() {
        let mut recorder = Recorder::default();
        for (at, gpu, cpu) in [(0, 1.0, 10.0), (1_000, 8.0, 20.0), (55_000, 5.0, 30.0), (59_000, 3.0, 40.0)] {
            recorder.sample_at(at, input(gpu, cpu));
        }
        let dump = recorder.dump(Trigger::GpuLoss, 60_000);
        let summary = report(&dump);
        assert!(summary.contains("GPU frame ms: 1.0 / 8.0 / 3.0; 3.0 / 5.0 / 3.0"), "{summary}");
        assert!(
            summary.contains("GPU pass 1 (effects): 1.0 / 8.0 / 3.0; 3.0 / 5.0 / 3.0"),
            "{summary}"
        );
    }

    #[test]
    fn serialized_dump_uses_fixed_labels_and_never_user_text() {
        let mut recorder = Recorder::default();
        let private_region_token = 0x7fed_cafe_d00d_u64;
        recorder.sample_at(
            0,
            SampleInput {
                region_token: Some(private_region_token),
                ..input(4.0, 8.0)
            },
        );
        recorder.push_event(0, EventKind::OverlayHookSeen, 0, 1);
        let dump = recorder.dump(Trigger::GpuLoss, 0);
        let json = serde_json::to_string(&dump).unwrap();
        assert_eq!(json, serde_json::to_string(&dump).unwrap());
        assert!(json.contains("overlay_hook_seen"));
        assert!(!json.contains(&private_region_token.to_string()));
        assert!(!json.contains("Private User"));
        assert!(report_from_json(&json).unwrap().contains("overlay hook observed"));
        assert!(report_from_json("{\"schema\":999}").is_none());
    }

    #[test]
    fn ten_thousand_ring_samples_stay_under_one_hundred_milliseconds() {
        let mut recorder = Recorder::default();
        let started = Instant::now();
        for index in 0..10_000 {
            recorder.sample_at(index as u64 * SAMPLE_INTERVAL_MS, input((index % 100) as f32, 10.0));
        }
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "sampling took {:?}",
            started.elapsed()
        );
        assert_eq!(recorder.sample_len, SAMPLE_CAP);
    }
}
