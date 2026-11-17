//! Autotune's frame (settings/autotune.rs): each frame the work time goes to
//! the tuner, whose decision is written through the hold that keeps the
//! user's own values; the avatars the user keeps in full are handed to the
//! scene. Started from the Performance window, or at the login with
//! "Continuous" kept (LLPerfStats::Tunables::initialiseFromSettings).
use super::*;
use crate::settings::autotune::{Decision, Frame, Hold, Status, Tuned, Tuner};

/// Autotune in the app.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The tuner while it runs (None: off, or a finished run's values held).
    pub tuner: Option<Tuner>,
    /// The frames since the avatars kept in full were listed.
    keep_age: u32,
    /// The rate the loop was held to last frame, for the window.
    held_to: f32,
}

/// The avatars kept in full are listed again this often (frames): friends
/// and exceptions change seldom, and a slot found a second late is drawn
/// as an impostor for that second at most.
const KEEP_EVERY: u32 = 30;

impl App {
    /// Where Autotune stands, for the Performance window.
    pub(crate) fn autotune_status(&self) -> Status {
        match &self.autotune.tuner {
            Some(t) => t.status,
            None if self.settings.autotune_hold.is_some() => Status::Finished,
            None => Status::Off,
        }
    }

    /// Start (the window's Start, or the login with "Continuous" kept).
    pub(super) fn autotune_start(&mut self) {
        if self.autotune.tuner.is_none() {
            log::info!(
                "autotune: start, target {} fps, {:?}{}",
                self.settings.autotune.target_fps,
                self.settings.autotune.strategy,
                if self.settings.autotune.continuous { ", continuous" } else { "" }
            );
            self.autotune.tuner = Some(Tuner::start(Instant::now()));
        }
    }

    /// Stop and give the user's values back (the window's Stop, or "Give
    /// back my settings" after a run without "Continuous", or the logout).
    pub(super) fn autotune_stop(&mut self) {
        let was = self.autotune.tuner.take().is_some();
        if let Some(hold) = self.settings.autotune_hold.take() {
            let before = Tuned::read(&self.settings);
            hold.restore(&mut self.settings);
            log::info!(
                "autotune: stop, draw distance {} -> {} m, avatars in full {} -> {}",
                before.draw_distance,
                self.settings.draw_distance,
                before.max_avatars,
                self.settings.max_avatars
            );
        } else if was {
            log::info!("autotune: stop, nothing changed");
        }
        self.autotune_clear_scene();
    }

    /// Keep the values found as the user's (after a run without
    /// "Continuous"): the hold ends without giving anything back.
    pub(super) fn autotune_keep(&mut self) {
        self.autotune.tuner = None;
        if self.settings.autotune_hold.take().is_some() {
            log::info!(
                "autotune: values kept as the user's, draw distance {} m, avatars in full {}",
                self.settings.draw_distance,
                self.settings.max_avatars
            );
        }
        self.autotune_clear_scene();
    }

    /// At the arrival of a login: "Continuous" kept starts it again.
    pub(super) fn autotune_login(&mut self) {
        if self.settings.autotune.runs_at_login() {
            self.autotune_start();
        }
    }

    fn autotune_clear_scene(&mut self) {
        self.scene.avatar_keep_full.clear();
        self.scene.avatar_keep_full_within = 0;
        self.scene.avatar_count_radius = 0.0;
    }

    /// One frame: `work` is the frame's time without the frame limiter's
    /// wait (None while Windows holds the window), `held_to` the rate the
    /// loop is held to (0 for none).
    pub(super) fn autotune_frame(&mut self, work: Option<Duration>, held_to: f32) {
        self.autotune.held_to = held_to;
        if self.autotune.tuner.is_none() && self.settings.autotune_hold.is_none() {
            return;
        }
        let work = if self.demo { self.autotune_demo_work(work) } else { work };
        let paused = work.is_none()
            || !self.focused
            || self.vr_camera.is_some()
            || !self.in_world()
            || self.world.teleporting
            || self.settings.photo_held.is_some()
            || self.settings.draw_stepping.is_some();
        let mut hold = self.settings.autotune_hold.take();
        if let Some(h) = &mut hold {
            h.sync(&self.settings);
        }
        let applied = Tuned::read(&self.settings);
        let user = hold.as_ref().map_or(applied, |h| h.user(&self.settings));
        let config = self.settings.autotune.clone();
        if let Some(tuner) = &mut self.autotune.tuner {
            let counts = self.scene.stats.avatars;
            let frame = Frame {
                now: Instant::now(),
                work_ms: work.map_or(0.0, |w| w.as_secs_f32() * 1000.0),
                cap_fps: held_to,
                paused,
                avatars: (counts.full + counts.impostors) as u32,
                within_distance: config.by_distance.then_some(self.scene.stats.avatars_within as u32),
            };
            match tuner.tick(&config, frame, applied, user) {
                Decision::Keep => {}
                Decision::Set(to) => {
                    let h = hold.get_or_insert_with(|| Hold::new(&self.settings));
                    h.write(&mut self.settings, to);
                    let live = Tuned::read(&self.settings);
                    tuner.written(live);
                    log::info!(
                        "autotune: {:?}, median {:.1} ms -> draw distance {} m, avatars in full {}",
                        tuner.status,
                        tuner.frame_ms().unwrap_or(0.0),
                        live.draw_distance,
                        live.max_avatars
                    );
                }
                Decision::Finish => log::info!("autotune: target held, finished; the values stay for the session"),
            }
            if tuner.status == Status::Finished {
                self.autotune.tuner = None;
            }
        }
        self.settings.autotune_hold = hold;
        self.autotune_scene(user);
    }

    /// The scene's side: the avatars the user keeps in full while the
    /// avatars in full are under the user's limit, and the distance mode's
    /// count.
    fn autotune_scene(&mut self, user: Tuned) {
        let running = self.autotune.tuner.is_some();
        self.scene.avatar_count_radius = if running && self.settings.autotune.by_distance {
            self.settings.autotune.impostor_distance
        } else {
            0.0
        };
        if self.settings.max_avatars >= user.max_avatars {
            if !self.scene.avatar_keep_full.is_empty() || self.scene.avatar_keep_full_within != 0 {
                self.scene.avatar_keep_full.clear();
                self.scene.avatar_keep_full_within = 0;
            }
            return;
        }
        self.autotune.keep_age += 1;
        if self.autotune.keep_age < KEEP_EVERY && self.scene.avatar_keep_full_within == user.max_avatars as usize {
            return;
        }
        self.autotune.keep_age = 0;
        self.scene.avatar_keep_full_within = user.max_avatars as usize;
        let keep = &mut self.scene.avatar_keep_full;
        keep.clear();
        // "Always render fully" (LLVOAvatar::AV_ALWAYS_RENDER)
        for (id, mode) in &self.world.render_exceptions {
            if *mode == 2
                && let Some(i) = self.world.objects.index_of_uuid(id)
            {
                keep.insert(i);
            }
        }
        // friends, when friends are always shown fully (isTooSlow: render_friend)
        if self.settings.complexity_mode >= 1 {
            for f in &self.world.social.friends {
                if let Some(i) = self.world.objects.index_of_uuid(&f.id) {
                    keep.insert(i);
                }
            }
        }
    }
}

impl App {
    /// What the Performance window shows of Autotune.
    pub(super) fn autotune_view(&self) -> crate::ui::monitor::autotune::View {
        let live = Tuned::read(&self.settings);
        crate::ui::monitor::autotune::View {
            config: self.settings.autotune.clone(),
            status: self.autotune_status(),
            frame_ms: self.autotune.tuner.as_ref().and_then(Tuner::frame_ms),
            held_to: self.autotune.held_to,
            live,
            user: self.settings.autotune_hold.as_ref().map_or(live, |h| h.user(&self.settings)),
        }
    }

    /// The window's Start, Stop, Keep and choices.
    pub(super) fn autotune_request(&mut self, request: crate::ui::monitor::autotune::Request) {
        use crate::ui::monitor::autotune::Request;
        match request {
            Request::Start => self.autotune_start(),
            Request::Stop => self.autotune_stop(),
            Request::Keep => self.autotune_keep(),
            Request::Config(mut config) => {
                config.sanitize();
                self.settings.autotune = config;
                self.settings.save();
            }
        }
    }
}

/// `AURORA_DEMO_AUTOTUNE`: `1` runs it on the real frames, `slow` on a
/// simulated heavy scene ([`demo_frame_ms`]), `once` the same without
/// "Continuous" (it stops on its own, with Keep and Give back).
fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_AUTOTUNE").ok().map(|v| v.trim().to_owned()))
        .as_deref()
}

/// The demo's heavy scene: a frame costs 12 ms, 2.4 ms per avatar drawn in
/// full and 0.06 ms per metre of draw distance, so that at a 30 fps target
/// both levers are needed (16 avatars at 256 m is about 66 ms).
fn demo_frame_ms(avatars_full: usize, draw_distance: f32) -> f32 {
    12.0 + 2.4 * avatars_full as f32 + 0.06 * draw_distance
}

impl App {
    /// The demo's start: the Performance window on the target frame rate,
    /// running towards 30 fps. Demo values only: a permission check is
    /// never relaxed.
    pub(super) fn autotune_demo(&mut self) {
        let Some(mode) = demo_mode() else {
            return;
        };
        self.panels.perf = true;
        ui::perf::set_full(&self.egui_ctx, true);
        ui::monitor::demo_tab(&self.egui_ctx, ui::monitor::Tab::Autotune);
        self.settings.autotune.target_fps = 30;
        self.settings.autotune.continuous = mode != "once";
        self.autotune_start();
    }

    /// The simulated frame time of `slow` and `once` (the real one with `1`).
    fn autotune_demo_work(&self, work: Option<Duration>) -> Option<Duration> {
        match demo_mode() {
            Some("slow" | "once") => work.map(|_| {
                let ms = demo_frame_ms(self.scene.stats.avatars.full, self.settings.draw_distance);
                Duration::from_secs_f32(ms / 1000.0)
            }),
            _ => work,
        }
    }
}
