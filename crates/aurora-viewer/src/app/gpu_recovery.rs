//! The window, the renderer and egui's input state, made the same way at
//! start and when the viewer recovers from a GPU device loss on a new window
//! and device (the window of a lost device cannot present again: its
//! swapchain is abandoned with the device).

use super::*;

/// How the main window opens.
pub(super) struct WindowPlan {
    pub size: winit::dpi::Size,
    pub position: Option<winit::dpi::PhysicalPosition<i32>>,
    /// Automated captures must not steal the user's keyboard focus.
    pub active: bool,
    pub fullscreen: Option<winit::window::Fullscreen>,
}

impl App {
    pub(super) fn open_window(&self, event_loop: &ActiveEventLoop, plan: &WindowPlan) -> Result<Arc<Window>, winit::error::OsError> {
        let mut attrs = crate::cli::with_desktop_id(Window::default_attributes())
            .with_title(crate::cli::window_title())
            // Metal needs a visible layer to present its first drawable.
            // Other platforms keep the unpainted client area hidden.
            .with_visible(cfg!(target_os = "macos"))
            .with_active(plan.active)
            .with_inner_size(plan.size)
            .with_min_inner_size(winit::dpi::LogicalSize::new(800, 560))
            .with_fullscreen(plan.fullscreen.clone())
            // none yet at start (it is decoded in the background)
            .with_window_icon(self.window_icon.clone());
        if let Some(position) = plan.position {
            attrs = attrs.with_position(position);
        }
        let window = Arc::new(event_loop.create_window(attrs)?);
        crate::window_hold::install(&window);
        crate::slurl_handoff::window_ready(&window);
        Ok(window)
    }

    /// The renderer for `window`, the user's settings applied once the
    /// caller has settled them.
    pub(super) fn open_renderer(&mut self, window: &Arc<Window>) -> Result<Renderer, aurora_render::RenderError> {
        let size = window.inner_size();
        if std::env::var_os("AURORA_NOVSYNC").is_some() {
            self.settings.vsync = false;
        }
        // the GPU of Preferences › Graphics; AURORA_GPU=<number|name|auto>
        // overrides it for this launch (adapter_choice.rs)
        let gpu = std::env::var("AURORA_GPU").unwrap_or_else(|_| self.settings.gpu_adapter.clone());
        // the pipelines the driver compiled at the earlier starts, kept in
        // the cache folder (aurora-render's pipeline_cache.rs)
        aurora_render::pipeline_cache::configure(crate::settings::cache_dir().join("pipelines"), aurora_version::version());
        // AURORA_GPU_START_FAIL=1: as if no card could draw, to see the
        // message the user gets then (graphics_start.rs); `lost` is read by
        // the renderer, which gets as far as a real device loss does
        if std::env::var("AURORA_GPU_START_FAIL").is_ok_and(|mode| mode != "lost") {
            return Err(aurora_render::RenderError::NoAdapter("AURORA_GPU_START_FAIL".into()));
        }
        // the VR runtime only when VR is wanted from the start: `--vr` (the
        // restart the VR button offers) or Preferences › Camera › VR (#2309)
        self.vr_set_up = crate::cli::get().vr || self.settings.camera.vr_at_start;
        let renderer = Renderer::new(window.clone(), size.width, size.height, self.settings.vsync, &gpu, self.vr_set_up)?;
        log::info!(
            "renderer ready: {} via {} ({} texture slots in {} pages at most, GPU timers: {})",
            renderer.info.name,
            renderer.info.backend,
            renderer.textures.capacity(),
            renderer.info.max_textures,
            renderer.info.timestamps
        );
        Ok(renderer)
    }

    /// What Preferences › Graphics shows of the GPUs, from the renderer's
    /// own list: here, not with the world's frames, so the login screen has
    /// it too (#894: only "Automatic" was offered before login).
    pub(super) fn note_gpus(&mut self, renderer: &Renderer) {
        self.options_ui.vram = (renderer.info.dedicated_mb, renderer.info.shared_mb, renderer.info.integrated);
        self.options_ui.gpus = renderer.info.adapters.clone();
        self.options_ui.gpu_in_use = renderer.info.name.clone();
        self.options_ui.gpu_notice = renderer.info.notice;
    }

    pub(super) fn egui_state_for(&self, window: &Window, renderer: &Renderer) -> egui_winit::State {
        let max_tex = renderer.device.limits().max_texture_dimension_2d as usize;
        egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_tex),
        )
    }
}

/// Losses closer than this count together.
const LOSS_SPAN: Duration = Duration::from_secs(5 * 60);

/// What a device loss leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LossStep {
    /// A new window and device, the settings as they are.
    Recover,
    /// Again within a few minutes: recovered with the safe graphics level.
    RecoverSafe,
    /// A third time: closed as before, with the crash report.
    Close,
}

/// The human's rule of 2026-11-03: of the losses of the last few minutes,
/// the first recovers as it was, the second with the safe graphics level
/// (the cause, a pass or the video memory, is likely still there), the
/// third closes. `history` keeps their times.
pub(super) fn loss_step(history: &mut std::collections::VecDeque<Instant>, now: Instant) -> LossStep {
    while history.front().is_some_and(|at| now.duration_since(*at) > LOSS_SPAN) {
        history.pop_front();
    }
    history.push_back(now);
    match history.len() {
        1 => LossStep::Recover,
        2 => LossStep::RecoverSafe,
        _ => LossStep::Close,
    }
}

/// Whether a device loss recovers instead of closing: Preferences › Graphics
/// (`gpu_recovery`, on by default), unless `AURORA_GPU_RECOVERY` forces it
/// for the launch. Most losses of a card shared with another program are a
/// driver reset of the whole card, which the other programs survive by
/// making their device again (#2237).
pub(super) fn enabled(setting: bool) -> bool {
    forced().unwrap_or(setting)
}

/// egui's CPU copy (egui_shadow.rs) is kept unless the recovery is forced
/// off: the setting can be turned on during the session, and a copy begun
/// then would miss the textures egui sent before.
pub(super) fn shadow_kept() -> bool {
    forced() != Some(false)
}

/// `AURORA_GPU_RECOVERY`: `1` on, `0` off, whatever the setting.
fn forced() -> Option<bool> {
    // read every frame: once per run
    static FORCED: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| parse_forced(std::env::var("AURORA_GPU_RECOVERY").ok().as_deref()))
}

fn parse_forced(value: Option<&str>) -> Option<bool> {
    match value.map(str::trim) {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    }
}

impl App {
    /// Before the lost renderer goes: what the scene and the app hold on it.
    fn forget_gpu_scene(&mut self, old: &mut Renderer) {
        self.scene.forget_gpu(old);
        self.probes.reset();
        self.forget_gpu_requests();
    }

    /// Answers the lost renderer will never give: a click's picks, the
    /// viewfinder's and the photo isolation's, a grab's hover, a photo under
    /// way. The new renderer numbers its tickets from the start again, so an
    /// old ticket could be answered with another request's result.
    fn forget_gpu_requests(&mut self) {
        self.clicks.forget_pending();
        self.snapshots.vf.pick = None;
        self.snapshots.isolate_picks.clear();
        if let Some(held) = self.interactions.held.as_mut() {
            held.drawn = None;
        }
        self.snapshots.gpu_lost();
    }

    /// On the new renderer: the textures get its slots, and what `clear`
    /// emptied that the world does not send again comes back.
    fn adopt_gpu_scene(&mut self, new: &mut Renderer) {
        self.scene.adopt_gpu(new);
        // what `clear` emptied that the world does not send again
        self.world.objects.particles_again();
        for region in self.world.regions.values_mut() {
            region.terrain_surface.gpu_reset();
        }
        self.media.gpu_reset();
        // the local images of the imports, the wearable editor and Blender Live
        self.republish_local_imports();
        self.wearable_ui.preview.gpu_reset();
        self.blender_live.gpu_reset(self.scene.textures.staging.as_deref());
    }

    /// After a GPU device loss: a new window and renderer, the session and
    /// the world kept, the scene reloaded on the new device. The old window
    /// stays on screen until the new one has presented (`retired_window`).
    /// False when no new renderer could be made: the caller closes as before.
    pub(super) fn recover_from_gpu_loss(&mut self, event_loop: &ActiveEventLoop, safe: bool) -> bool {
        let Some(old) = self.gfx.take() else {
            return false;
        };
        let started = Instant::now();
        let plan = WindowPlan {
            size: old.window.inner_size().into(),
            position: old.window.outer_position().ok(),
            active: old.window.has_focus(),
            fullscreen: old.window.fullscreen(),
        };
        let maximized = old.window.is_maximized();
        // the layout watch is the connection's keyboard, not the window's:
        // it carries over
        let Gfx {
            mut renderer,
            window,
            layout_watch,
            ..
        } = old;
        self.forget_gpu_scene(&mut renderer);
        // the lost renderer first: its surface is made from this window's
        // handles (Renderer::new's contract)
        drop(renderer);
        self.retired_window = Some(window);
        if safe {
            self.safe_graphics_after_losses();
        }
        let window = match self.open_window(event_loop, &plan) {
            Ok(w) => w,
            Err(e) => {
                log::error!("GPU recovery: cannot create a window: {e}");
                return false;
            }
        };
        let mut renderer = match self.open_renderer(&window) {
            Ok(r) => r,
            Err(e) => {
                log::error!("GPU recovery: no new renderer: {e}");
                return false;
            }
        };
        renderer.apply_settings(self.settings.render_settings());
        self.adopt_gpu_scene(&mut renderer);
        self.note_gpus(&renderer);
        self.screen_rate.poll(&window, Instant::now());
        let egui_state = self.egui_state_for(&window, &renderer);
        self.gfx = Some(Gfx {
            renderer,
            egui_state,
            window,
            shown: false,
            maximize_on_show: maximized,
            layout_watch,
        });
        self.gpu_stall = Default::default();
        // the crash report's state froze at the loss; it follows the session
        // again, and a report that comes later counts this recovery
        self.crash_kept = Default::default();
        self.crash_swapchain = Default::default();
        self.crash_recent = Default::default();
        crate::crash::gpu_recovered(started.elapsed().as_millis(), safe);
        // the new egui renderer has none of egui's textures
        self.egui_replay = true;
        log::warn!(
            "GPU recovery: new window and renderer in {} ms{}",
            started.elapsed().as_millis(),
            if safe { ", with the safe graphics level" } else { "" }
        );
        self.world.notifications.push(
            crate::world::notifications::Kind::Alert,
            aurora_i18n::tr!("app-gpu-recovered-title"),
            if safe {
                aurora_i18n::tr!("app-gpu-recovered-safe")
            } else {
                aurora_i18n::tr!("app-gpu-recovered")
            }
            .to_owned(),
            crate::world::notifications::Data::None,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_loss_recovers_the_second_with_safe_graphics_the_third_closes() {
        let mut history = std::collections::VecDeque::new();
        let t = Instant::now();
        let m = |n: u64| Duration::from_secs(n * 60);
        assert_eq!(loss_step(&mut history, t), LossStep::Recover);
        assert_eq!(loss_step(&mut history, t + m(2)), LossStep::RecoverSafe);
        assert_eq!(loss_step(&mut history, t + m(4)), LossStep::Close);
        // losses far apart each recover as the first
        let mut history = std::collections::VecDeque::new();
        assert_eq!(loss_step(&mut history, t), LossStep::Recover);
        assert_eq!(loss_step(&mut history, t + m(6)), LossStep::Recover);
        assert_eq!(loss_step(&mut history, t + m(12)), LossStep::Recover);
        // the first of three counts out after its five minutes
        let mut history = std::collections::VecDeque::new();
        assert_eq!(loss_step(&mut history, t), LossStep::Recover);
        assert_eq!(loss_step(&mut history, t + m(4)), LossStep::RecoverSafe);
        assert_eq!(loss_step(&mut history, t + m(6)), LossStep::RecoverSafe);
    }

    #[test]
    fn the_variable_forces_the_recovery_either_way_and_anything_else_leaves_the_setting() {
        assert_eq!(parse_forced(Some("1")), Some(true));
        assert_eq!(parse_forced(Some(" 0 ")), Some(false));
        assert_eq!(parse_forced(Some("")), None);
        assert_eq!(parse_forced(Some("yes")), None);
        assert_eq!(parse_forced(None), None);
    }
}
