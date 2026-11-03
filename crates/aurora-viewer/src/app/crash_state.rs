//! What a crash report says the viewer was doing (crash/watchdog.rs): every
//! two seconds a few numbers already measured for the status bar and the
//! texture budget, and fixed names. A few atomics and one video memory
//! query, nothing measured for it alone. Once the GPU is lost the state
//! stops there ([`KeptState`]). The report also says which GPU effects were
//! on, how long the GPU frames took (the latest, the longest of 10 s and
//! their heaviest pass), the texture uploads, the time since the login and
//! the last region change, what was still loading and the window's state
//! ([`Recent`], [`Timeline`], [`effects`]): what tells a driver reset in a
//! heavy frame, one right after the login and one caused by an effect apart.

use std::collections::VecDeque;

use super::*;

const EVERY: Duration = Duration::from_secs(2);

/// The span the swapchain recreations are counted over.
const SWAPCHAIN_SPAN: Duration = Duration::from_secs(10);

/// Swapchain recreations in the last [`SWAPCHAIN_SPAN`], from the running
/// total read at each note. Frozen at the first note after the GPU was
/// lost: a report then tells whether the window was being resized just
/// before the loss (#1701), not in the seconds the viewer took to close.
#[derive(Debug, Default)]
pub(super) struct SwapchainRecent {
    /// (when, running total), oldest first, over the span.
    samples: VecDeque<(Instant, u64)>,
    frozen: Option<u64>,
}

impl SwapchainRecent {
    fn note(&mut self, now: Instant, total: u64, device_lost: bool) -> u64 {
        if let Some(count) = self.frozen {
            return count;
        }
        while self.samples.len() > 1 && self.samples.get(1).is_some_and(|(at, _)| now.duration_since(*at) >= SWAPCHAIN_SPAN) {
            self.samples.pop_front();
        }
        let count = self.samples.front().map_or(0, |(_, oldest)| total.saturating_sub(*oldest));
        self.samples.push_back((now, total));
        if device_lost {
            self.frozen = Some(count);
        }
        count
    }
}

/// What the frames since the previous note measured, for the report: a
/// compare and a few additions a frame, nothing allocated. The longest GPU
/// frame is kept for [`PEAK_SPAN`] before the loss.
#[derive(Debug, Default)]
pub(super) struct Recent {
    /// (when, longest GPU ms, its heaviest pass) of each past note
    /// interval, oldest first; expired by the time of the frame itself.
    peaks: VecDeque<(Instant, f32, Option<&'static str>)>,
    /// The same for the interval under way.
    current: Option<(Instant, f32, Option<&'static str>)>,
    last_ms: Option<f32>,
    last_pass: Option<&'static str>,
    uploads: u32,
    upload_bytes: u64,
    upload_peak_bytes: u64,
    /// The longest the GPU kept the render thread (`stalled_for`).
    stall_ms: u64,
    frozen: bool,
    /// What the note at the loss gave, which stays.
    at_loss: Option<Sample>,
}

/// What [`Recent`] gives a note.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Sample {
    gpu_ms: Option<f32>,
    peak: Option<(f32, Option<&'static str>)>,
    last_pass: Option<&'static str>,
    uploads: (u32, f32),
    upload_peak_mb: f32,
    stall_ms: u64,
}

/// The pass that took most of a frame's GPU time (`GpuElement::key`).
fn heaviest_pass(elements: &[f32; aurora_render::GpuElement::ALL.len()]) -> Option<&'static str> {
    let (index, ms) = elements.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1))?;
    (ms > 0.0).then(|| aurora_render::GpuElement::ALL[index].key())
}

const MB: f32 = (1 << 20) as f32;

/// How far back the longest GPU frame looks.
const PEAK_SPAN: Duration = Duration::from_secs(10);

impl Recent {
    /// One frame: the GPU time the renderer measured last (it arrives a
    /// frame or two late, without waiting on the GPU) and the uploads.
    pub(super) fn frame(&mut self, now: Instant, stats: &aurora_render::RenderStats, uploads: usize, upload_bytes: u64) {
        if self.frozen {
            return;
        }
        if let Some(ms) = stats.gpu_ms {
            let pass = stats.gpu_elements.as_ref().and_then(heaviest_pass);
            self.last_ms = Some(ms);
            self.last_pass = pass;
            if self.current.is_none_or(|c| ms > c.1) {
                self.current = Some((now, ms, pass));
            }
        }
        self.uploads = self.uploads.saturating_add(uploads.min(u32::MAX as usize) as u32);
        self.upload_bytes += upload_bytes;
        self.upload_peak_bytes = self.upload_peak_bytes.max(upload_bytes);
    }

    /// The render thread's stall, asked every frame: a hang (a driver reset
    /// follows a GPU task of about 2 s) from a loss with no stall before.
    pub(super) fn stall(&mut self, held: Option<Duration>) {
        if !self.frozen
            && let Some(held) = held
        {
            self.stall_ms = self.stall_ms.max(held.as_millis().min(u128::from(u64::MAX)) as u64);
        }
    }

    /// The numbers for a note. A new interval begins after it, except at the
    /// loss: then nothing changes any more, like the swapchain count.
    pub(super) fn note(&mut self, now: Instant, device_lost: bool) -> Sample {
        if let Some(kept) = self.at_loss {
            return kept;
        }
        let peak = self
            .peaks
            .iter()
            .copied()
            .chain(self.current)
            .filter(|p| now.saturating_duration_since(p.0) <= PEAK_SPAN)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|p| (p.1, p.2));
        let sample = Sample {
            gpu_ms: self.last_ms,
            peak,
            last_pass: self.last_pass,
            uploads: (self.uploads, self.upload_bytes as f32 / MB),
            upload_peak_mb: self.upload_peak_bytes as f32 / MB,
            stall_ms: self.stall_ms,
        };
        if device_lost {
            self.frozen = true;
            self.at_loss = Some(sample);
        } else {
            self.peaks.extend(self.current.take());
            while self.peaks.front().is_some_and(|p| now.saturating_duration_since(p.0) > PEAK_SPAN) {
                self.peaks.pop_front();
            }
            self.uploads = 0;
            self.upload_bytes = 0;
            self.upload_peak_bytes = 0;
        }
        sample
    }
}

/// Where the session stands in time: since the login began, since the world
/// showed and since the agent's region last changed, for a loss soon after
/// the login or a teleport.
#[derive(Debug, Default)]
pub(super) struct Timeline {
    session: Option<Instant>,
    world: Option<Instant>,
    region: Option<Instant>,
    handle: Option<u64>,
}

impl Timeline {
    /// Every frame, a few comparisons.
    /// `login_busy`: the login request is under way on the login screen. The
    /// clock starts with the request and stops only when the screen is idle
    /// again (a failed login), not while the login screen waits for it.
    pub(super) fn frame(&mut self, now: Instant, screen: &Screen, login_busy: bool, region: Option<u64>) {
        match screen {
            Screen::Login if login_busy => {
                self.session.get_or_insert(now);
            }
            Screen::Login => *self = Self::default(),
            Screen::Loading { .. } => {
                self.session.get_or_insert(now);
                self.world = None;
            }
            Screen::World => {
                self.session.get_or_insert(now);
                self.world.get_or_insert(now);
            }
        }
        if region != self.handle {
            self.handle = region;
            self.region = region.map(|_| now);
        }
    }

    fn seconds(since: Option<Instant>, now: Instant) -> Option<u64> {
        since.map(|at| now.saturating_duration_since(at).as_secs())
    }
}

/// The GPU effects and quality switches on, as the report's fixed words
/// (`EFFECT_WORDS` in crash/watchdog.rs): what tells a hang caused by an effect
/// from one that is not. Conditions follow the renderer: the contact shadows
/// need the shadows, the indirect light needs the GTAO.
pub(super) fn effects(settings: &crate::settings::Settings, photo: bool, vr: bool) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let mut word = |on: bool, text: std::fmt::Arguments| {
        if on {
            if !out.is_empty() {
                out.push(',');
            }
            let _ = out.write_fmt(text);
        }
    };
    let shadows = settings.shadow_quality > 0;
    let ao = settings.ssao > 0;
    word(shadows, format_args!("shadows{}", settings.shadow_quality));
    word(shadows && settings.projector_shadows, format_args!("projector-shadows"));
    word(shadows && settings.contact_shadows, format_args!("contact-shadows"));
    word(ao && settings.ao_method == 0, format_args!("ao-legacy{}", settings.ssao));
    word(ao && settings.ao_method == 1, format_args!("ao-gtao{}", settings.gtao_quality));
    word(settings.ssr, format_args!("ssr"));
    match settings.antialiasing {
        1 => word(true, format_args!("aa-fxaa")),
        2 => word(true, format_args!("aa-msaa2")),
        3 => word(true, format_args!("aa-msaa4")),
        4 => word(true, format_args!("aa-msaa8")),
        5 => word(true, format_args!("aa-taa")),
        6 => word(true, format_args!("aa-smaa")),
        _ => {}
    }
    word(settings.bloom, format_args!("bloom"));
    word(settings.glow, format_args!("glow"));
    word(
        ao && settings.ao_method == 1 && settings.screen_indirect_light,
        format_args!("indirect-light"),
    );
    word(settings.reflection_probes > 0, format_args!("probes{}", settings.reflection_probes));
    word(
        settings.water_reflections > 0,
        format_args!("water-reflections{}", settings.water_reflections),
    );
    word(settings.mirrors > 0, format_args!("mirrors{}", settings.mirrors));
    word(true, format_args!("scale{}", settings.render_scale_percent));
    word(true, format_args!("draw{}", settings.draw_distance.round() as u32));
    word(settings.hdr_emissive, format_args!("hdr"));
    word(photo, format_args!("photo"));
    word(vr, format_args!("vr"));
    out
}

/// What does not change in a session: the Vulkan layers on offer. Looked up
/// once, on a thread of its own; empty until it is done.
fn session_layers() -> &'static str {
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;
    static LAYERS: OnceLock<String> = OnceLock::new();
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !STARTED.swap(true, Ordering::Relaxed)
        && std::thread::Builder::new()
            .name("aurora-crash-facts".into())
            .spawn(|| {
                let _ = LAYERS.set(aurora_render::vulkan_layers::loaded_names().join(","));
            })
            .is_err()
    {
        let _ = LAYERS.set(String::new());
    }
    LAYERS.get().map_or("", String::as_str)
}

/// The other viewers on this card and how many of them are in front, as
/// the instances registry (instances.rs) keeps them. None when the registry
/// is not running (demo and timed runs).
fn viewers_on_card(running: bool, peers: &crate::instances::Peers) -> Option<(u32, u32)> {
    running.then(|| {
        let n = |it: usize| u32::try_from(it).unwrap_or(u32::MAX);
        (n(peers.list.len()), n(peers.list.iter().filter(|p| p.active).count()))
    })
}

/// The card's memory in use by every program and by the others (MB), from
/// the counters of gpu_load.rs (Windows) and this viewer's own use.
fn card_memory_mb(all: Option<u64>, own: u64) -> (Option<u64>, Option<u64>) {
    (all.map(|a| a >> 20), all.map(|a| a.saturating_sub(own) >> 20))
}

/// The state a report shows once the GPU is lost. After the loss the viewer
/// only shows its notice and closes: the window loses the focus to the box
/// and drops to the background frame limit (most reports of 0.8.4 and 0.8.5
/// said 15 fps), and the frame time and the video memory describe those
/// seconds, not the frame that lost the GPU. So the last note before the
/// loss stays, noted once more with the swapchain recreations counted up to
/// the loss, and nothing is noted after it.
#[derive(Debug, Default)]
pub(super) struct KeptState {
    last: Option<crate::crash::State>,
    lost: bool,
}

/// What a note counts up to the loss, and so replaces in the last state
/// before it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Live {
    swapchain_10s: u64,
    sample: Sample,
    /// How the GPU was lost, once it is.
    loss: Option<(&'static str, &'static str, &'static str)>,
}

impl Live {
    fn apply(self, state: &mut crate::crash::State) {
        let Sample {
            gpu_ms,
            peak,
            last_pass,
            uploads,
            upload_peak_mb,
            stall_ms,
        } = self.sample;
        state.loss = self.loss;
        state.stall_ms = stall_ms;
        state.swapchain_10s = self.swapchain_10s;
        state.gpu_ms = gpu_ms;
        state.gpu_peak_ms = peak.map(|p| p.0);
        state.gpu_peak_pass = peak.and_then(|p| p.1);
        state.gpu_last_pass = last_pass;
        state.uploads = uploads;
        state.upload_peak_mb = upload_peak_mb;
    }
}

impl KeptState {
    /// What to note now, if anything: the fresh state while the GPU answers;
    /// at the loss, the last state before it (the fresh one when the GPU was
    /// lost before the first note); after it, nothing.
    fn next(&mut self, lost: bool, live: Live, fresh: impl FnOnce() -> crate::crash::State) -> Option<crate::crash::State> {
        if self.lost {
            return None;
        }
        if lost {
            self.lost = true;
            let mut kept = self.last.take().unwrap_or_else(fresh);
            live.apply(&mut kept);
            return Some(kept);
        }
        let state = fresh();
        self.last = Some(state.clone());
        Some(state)
    }

    /// The loss is noted and the state no longer changes.
    fn done(&self) -> bool {
        self.lost
    }
}

impl App {
    pub(super) fn crash_state(&mut self, renderer: &aurora_render::Renderer, window: &winit::window::Window) {
        let now = Instant::now();
        if self.crash_kept.done() {
            return;
        }
        self.crash_recent.stall(renderer.stalled_for());
        // the loss is noted at once, the closing may not leave two seconds
        let lost = renderer.device_lost();
        if !lost && self.crash_state_at.is_some_and(|at| now.duration_since(at) < EVERY) {
            return;
        }
        self.crash_state_at = Some(now);
        let swapchain_10s = self.crash_swapchain.note(now, aurora_render::present::recreations(), lost);
        let live = Live {
            swapchain_10s,
            sample: self.crash_recent.note(now, lost),
            loss: if lost { aurora_render::loss_details() } else { None },
        };
        let mut kept = std::mem::take(&mut self.crash_kept);
        let next = kept.next(lost, live, || self.crash_state_now(renderer, window, now, live));
        self.crash_kept = kept;
        if let Some(state) = next {
            crate::crash::note_state(state);
        }
    }

    fn crash_state_now(
        &self,
        renderer: &aurora_render::Renderer,
        window: &winit::window::Window,
        now: Instant,
        live: Live,
    ) -> crate::crash::State {
        let video = renderer.video_memory.read(&renderer.device);
        let size = window.inner_size();
        let mut state = crate::crash::State {
            screen: match self.screen {
                Screen::Login => "login",
                Screen::Loading { .. } => "loading",
                Screen::World => "world",
            },
            fps: self.perf.fps,
            frame_ms: self.perf.frames.avg,
            window: (size.width, size.height),
            preset: match crate::graphics_presets::Preset::matching(&self.settings) {
                Some(crate::graphics_presets::Preset::Potato) => "potato",
                Some(crate::graphics_presets::Preset::Low) => "low",
                Some(crate::graphics_presets::Preset::Medium) => "medium",
                Some(crate::graphics_presets::Preset::High) => "high",
                Some(crate::graphics_presets::Preset::Deluxe) => "deluxe",
                None => "custom",
            },
            vram_used_mb: video.map(|v| v.usage >> 20),
            vram_budget_mb: video.map(|v| v.budget >> 20),
            texture_pressure: (
                u32::from(self.scene.textures.stats.pressure),
                u32::from(crate::scene::texture_budget::MAX_PRESSURE),
            ),
            objects: self.scene.stats.objects,
            avatars: self.scene.stats.avatars.total,
            regions: self.world.regions.len(),
            ..Default::default()
        };
        live.apply(&mut state);
        state.effects = effects(&self.settings, self.snapshots.drawing(), renderer.vr_engaged());
        state.window_state = if window.is_minimized().unwrap_or(false) {
            "minimized"
        } else if self.occluded {
            "occluded"
        } else if !self.focused {
            "background"
        } else {
            "focused"
        };
        state.session_s = Timeline::seconds(self.crash_timeline.session, now);
        state.world_s = Timeline::seconds(self.crash_timeline.world, now);
        state.region_s = Timeline::seconds(self.crash_timeline.region, now);
        let tex = &self.scene.textures.stats;
        state.layers = session_layers().to_owned();
        let running = !self.demo && self.capture.is_none() && !self.frame_profile.enabled();
        state.viewers_on_card = viewers_on_card(running, &self.instances.peers());
        (state.card_used_mb, state.card_others_mb) = card_memory_mb(self.scene.textures.card_used(), self.scene.textures.video_usage());
        state.loading = (tex.fetching + tex.decoding, tex.pending_upload, self.scene.stats.geom_pending);
        state
    }

    /// Every frame, after the render: what the report counts over time.
    pub(super) fn crash_frame(&mut self) {
        let tex = &self.scene.textures.stats;
        self.crash_recent
            .frame(Instant::now(), &self.last_render, tex.uploads_frame, tex.uploaded_bytes_frame);
        self.crash_timeline
            .frame(Instant::now(), &self.screen, self.login_form.busy, self.world.main_region);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn swapchain_recreations_are_counted_over_the_last_ten_seconds() {
        let mut recent = SwapchainRecent::default();
        let t = Instant::now();
        // the first note has nothing to compare with
        assert_eq!(recent.note(t, 3, false), 0);
        // a drag: 20 recreations a second, noted every two seconds
        assert_eq!(recent.note(t + s(2), 43, false), 40);
        assert_eq!(recent.note(t + s(4), 83, false), 80);
        assert_eq!(recent.note(t + s(10), 203, false), 200);
        // past ten seconds the oldest notes drop out
        assert_eq!(recent.note(t + s(12), 243, false), 200);
        // the drag ends: back to nothing within ten seconds
        assert_eq!(recent.note(t + s(14), 250, false), 167);
        assert_eq!(recent.note(t + s(24), 250, false), 0);
    }

    #[test]
    fn the_count_stays_as_it_was_when_the_gpu_was_lost() {
        let mut recent = SwapchainRecent::default();
        let t = Instant::now();
        assert_eq!(recent.note(t, 0, false), 0);
        assert_eq!(recent.note(t + s(2), 30, false), 30);
        // lost: this note still counts what came before the loss
        assert_eq!(recent.note(t + s(4), 70, true), 70);
        // the viewer closing after it does not change the report
        assert_eq!(recent.note(t + s(20), 71, true), 70);
        assert_eq!(recent.note(t + s(30), 71, false), 70);
    }

    fn live(swapchain_10s: u64) -> Live {
        Live {
            swapchain_10s,
            ..Default::default()
        }
    }

    fn state(fps: f32, swapchain_10s: u64) -> crate::crash::State {
        crate::crash::State {
            screen: "world",
            fps,
            swapchain_10s,
            ..Default::default()
        }
    }

    #[test]
    fn the_report_keeps_the_last_state_before_the_gpu_was_lost() {
        let mut kept = KeptState::default();
        assert_eq!(kept.next(false, live(0), || state(90.0, 0)), Some(state(90.0, 0)));
        assert_eq!(kept.next(false, live(3), || state(85.0, 3)), Some(state(85.0, 3)));
        // lost: the notice takes the focus and the frame rate falls to the
        // background limit; the report keeps the frame before, with the
        // recreations counted up to the loss
        assert_eq!(
            kept.next(true, live(5), || panic!("no fresh state once lost")),
            Some(state(85.0, 5))
        );
        assert!(kept.done());
        // the seconds the viewer takes to close change nothing
        assert_eq!(kept.next(true, live(5), || state(15.0, 5)), None);
        assert_eq!(kept.next(false, live(9), || state(15.0, 9)), None);
    }

    #[test]
    fn effects_name_what_is_on_and_only_in_fixed_words() {
        let mut settings = crate::settings::Settings {
            shadow_quality: 3,
            contact_shadows: true,
            ssao: 2,
            ao_method: 1,
            gtao_quality: 2,
            antialiasing: 6,
            bloom: true,
            screen_indirect_light: true,
            ..Default::default()
        };
        let list = effects(&settings, true, true);
        for word in [
            "shadows3",
            "contact-shadows",
            "ao-gtao2",
            "aa-smaa",
            "bloom",
            "indirect-light",
            "photo",
            "vr",
            "scale",
            "draw",
        ] {
            assert!(list.split(',').any(|w| w.starts_with(word)), "{word} missing in {list}");
        }
        // the contact shadows and the indirect light need what they ride on
        settings.shadow_quality = 0;
        settings.ssao = 0;
        let list = effects(&settings, false, false);
        assert!(
            !list.contains("contact-shadows") && !list.contains("indirect-light") && !list.contains("shadows"),
            "{list}"
        );
        assert!(!list.contains("photo") && !list.contains("vr"), "{list}");
    }

    #[test]
    fn what_a_note_publishes_is_accepted_by_the_report() {
        let settings = crate::settings::Settings {
            shadow_quality: 4,
            projector_shadows: true,
            contact_shadows: true,
            ssao: 1,
            ao_method: 0,
            ssr: true,
            glow: true,
            reflection_probes: 3,
            water_reflections: 2,
            mirrors: 1,
            hdr_emissive: true,
            antialiasing: 3,
            ..Default::default()
        };
        let list = effects(&settings, true, true);
        assert!(crate::crash::summary_state_value(&list).is_some(), "{list}");
        for element in aurora_render::GpuElement::ALL {
            assert!(crate::crash::summary_state_value(element.key()).is_some(), "{}", element.key());
        }
        for name in crate::crash::WINDOW_STATES {
            assert!(crate::crash::summary_state_value(name).is_some(), "{name}");
        }
    }

    fn stats(gpu_ms: f32, elements: [f32; 7]) -> aurora_render::RenderStats {
        aurora_render::RenderStats {
            gpu_ms: Some(gpu_ms),
            gpu_elements: Some(elements),
            ..Default::default()
        }
    }

    #[test]
    fn the_longest_gpu_frame_and_its_heaviest_pass_stay_for_ten_seconds() {
        let mut recent = Recent::default();
        let t = Instant::now();
        recent.frame(t, &stats(8.0, [1.0, 1.0, 1.0, 4.0, 0.0, 1.0, 0.0]), 2, 3 << 20);
        recent.frame(t + s(1), &stats(2400.0, [1.0, 2300.0, 1.0, 4.0, 0.0, 1.0, 0.0]), 1, 1 << 20);
        recent.frame(t + s(1), &stats(9.0, [1.0, 1.0, 1.0, 5.0, 0.0, 1.0, 0.0]), 0, 0);
        recent.stall(Some(Duration::from_millis(2300)));
        let first = recent.note(t + s(2), false);
        assert_eq!(first.peak, Some((2400.0, Some("effects"))));
        assert_eq!(first.gpu_ms, Some(9.0));
        assert_eq!(first.last_pass, Some("objects"));
        assert_eq!(first.uploads.0, 3);
        assert!((first.uploads.1 - 4.0).abs() < 0.01 && (first.upload_peak_mb - 3.0).abs() < 0.01);
        assert_eq!(first.stall_ms, 2300);
        // the next interval counts its own uploads, the peak stays in the 10 s
        recent.frame(t + s(3), &stats(7.0, [0.0; 7]), 1, 1 << 20);
        let second = recent.note(t + s(4), false);
        assert_eq!(second.peak, Some((2400.0, Some("effects"))));
        assert_eq!(second.uploads.0, 1);
        // a loss between notes still sees the slow frame of 10 s before it
        // (it was at 1 s; the loss at 10.9 s)
        let between = recent.note(t + s(10) + Duration::from_millis(900), false);
        assert_eq!(between.peak, Some((2400.0, Some("effects"))));
        // and once it is older than that it is gone
        recent.frame(t + s(12), &stats(5.0, [0.0; 7]), 0, 0);
        assert_eq!(recent.note(t + s(12) + Duration::from_millis(100), false).peak, Some((7.0, None)));
        // once lost, what comes after changes nothing
        let at_loss = recent.note(t + s(13), true);
        recent.frame(t + s(14), &stats(5000.0, [0.0; 7]), 9, 9 << 20);
        assert_eq!(recent.note(t + s(15), true), at_loss);
    }

    #[test]
    fn the_other_viewers_and_the_cards_memory_are_counted_from_the_registry() {
        let peer = |active| crate::instances::Own {
            usage: 1 << 30,
            active,
            luid: 7,
        };
        let peers = crate::instances::Peers {
            list: vec![peer(true), peer(false), peer(false)],
        };
        assert_eq!(viewers_on_card(true, &peers), Some((3, 1)));
        assert_eq!(viewers_on_card(true, &Default::default()), Some((0, 0)));
        // a demo or a timed run does not run the registry: no row
        assert_eq!(viewers_on_card(false, &peers), None);
        // 5 GB in use on the card, 2 GB of it ours
        assert_eq!(card_memory_mb(Some(5 << 30), 2 << 30), (Some(5120), Some(3072)));
        assert_eq!(card_memory_mb(None, 2 << 30), (None, None));
        // what the viewer holds can read past the card's figure for a moment
        assert_eq!(card_memory_mb(Some(1 << 30), 2 << 30), (Some(1024), Some(0)));
    }

    #[test]
    fn the_timeline_counts_from_the_login_the_world_and_the_region() {
        let mut timeline = Timeline::default();
        let t = Instant::now();
        timeline.frame(t, &Screen::Login, false, None);
        assert!(timeline.session.is_none());
        // the request runs on the login screen: the clock starts there
        timeline.frame(t, &Screen::Login, true, None);
        timeline.frame(t + s(1), &Screen::Loading { since: t }, true, Some(7));
        timeline.frame(t + s(5), &Screen::World, false, Some(7));
        timeline.frame(t + s(9), &Screen::World, false, Some(8));
        let now = t + s(10);
        assert_eq!(Timeline::seconds(timeline.session, now), Some(10));
        assert_eq!(Timeline::seconds(timeline.world, now), Some(5));
        assert_eq!(Timeline::seconds(timeline.region, now), Some(1));
        // a failed login: idle again, the clock stops
        timeline.frame(t + s(20), &Screen::Login, false, None);
        assert!(timeline.session.is_none() && timeline.region.is_none());
    }

    #[test]
    fn a_gpu_lost_before_the_first_note_still_gives_a_state() {
        let mut kept = KeptState::default();
        assert_eq!(kept.next(true, live(2), || state(60.0, 0)), Some(state(60.0, 2)));
        assert_eq!(kept.next(true, live(2), || state(15.0, 2)), None);
    }
}
