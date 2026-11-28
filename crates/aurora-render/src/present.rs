//! Presentation mode of the swapchain: one rule for the creation of the
//! surface and for every later change of the vertical synchronization, so a
//! session presents the same way whether the setting was read at launch or
//! switched in the preferences.
//!
//! Firestorm only has `RenderVSyncEnable` (`LLWindowWin32::toggleVSync`,
//! indra/llwindow/llwindowwin32.cpp: `wglSwapIntervalEXT(1)` or `(0)`). On
//! Vulkan the unsynchronized side has two modes; the one in use is written to
//! the log and to the bug reports, since what the driver and the compositor
//! do with it depends on the screen under the window (variable refresh rate,
//! screens of different rates). Every configuration of the surface goes
//! through [`configure`], which counts the swapchain recreations for the
//! crash reports.

use std::sync::atomic::{AtomicU64, Ordering};

use wgpu::PresentMode;

/// Mode to configure the surface with, among those the surface supports.
///
/// - vsync on: `FifoRelaxed` (a late frame is shown at once instead of
///   waiting one more refresh), else `Fifo`, which every surface has;
/// - vsync off: `Mailbox` (never waits for the screen, no tearing), else
///   `Immediate` (never waits, tears), else `Fifo`.
///
/// `forced` (AURORA_PRESENT_MODE) wins when the surface supports it.
pub(crate) fn choose(supported: &[PresentMode], vsync: bool, forced: Option<PresentMode>) -> PresentMode {
    let has = |m: PresentMode| supported.contains(&m);
    if let Some(mode) = forced.filter(|m| has(*m)) {
        return mode;
    }
    let wanted: &[PresentMode] = if vsync {
        &[PresentMode::FifoRelaxed]
    } else {
        &[PresentMode::Mailbox, PresentMode::Immediate]
    };
    wanted.iter().copied().find(|m| has(*m)).unwrap_or(PresentMode::Fifo)
}

/// AURORA_PRESENT_MODE value (diagnostics): `immediate`, `mailbox`, `fifo`
/// or `fifo-relaxed`, whatever the case.
pub(crate) fn parse(value: &str) -> Option<PresentMode> {
    match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "immediate" => Some(PresentMode::Immediate),
        "mailbox" => Some(PresentMode::Mailbox),
        "fifo" => Some(PresentMode::Fifo),
        "fifo-relaxed" | "fiforelaxed" => Some(PresentMode::FifoRelaxed),
        _ => None,
    }
}

/// The mode forced for this process, read once.
pub(crate) fn forced() -> Option<PresentMode> {
    static FORCED: std::sync::OnceLock<Option<PresentMode>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        let value = std::env::var("AURORA_PRESENT_MODE").ok()?;
        let mode = parse(&value);
        if mode.is_none() {
            log::warn!("AURORA_PRESENT_MODE={value}: unknown mode (immediate, mailbox, fifo, fifo-relaxed)");
        }
        mode
    })
}

/// Whether the window's surface is made again before a switch from `old` to
/// `new`, for DLSS frame generation (`fg`): from a mode that never waits for
/// the screen to Fifo, on Wayland ([`WindowHandles`]).
///
/// NVIDIA's Vulkan driver (620.73.04) paces Fifo through the compositor's
/// commit timing (`wp_commit_timer_v1`, one for each Vulkan surface, which
/// lives as long as the surface, not the swapchain). Mailbox sends it no
/// times, so after a while without frame generation its first targets are
/// as old as that while, then one is as far ahead, which the compositor
/// obeys: no swapchain image comes back for that long (seen at 15 s, the
/// time frame generation had been off). A new surface starts a new timer
/// (8 switches out of 8 without a stall, against 6 stalls in 14 switches
/// without).
pub(crate) fn renews_surface(old: PresentMode, new: PresentMode, fg: bool) -> bool {
    fg && new == PresentMode::Fifo && matches!(old, PresentMode::Mailbox | PresentMode::Immediate)
}

/// What the window's surface is made from again ([`renews_surface`]): the
/// instance and the window's raw handles, kept by the window's renderer
/// alone. Wayland only: the driver's commit timer is a Wayland object.
#[derive(Clone)]
pub(crate) struct WindowHandles {
    instance: wgpu::Instance,
    display: wgpu::rwh::RawDisplayHandle,
    window: wgpu::rwh::RawWindowHandle,
}

// SAFETY: the handles are plain pointers to the window's `wl_display` and
// `wl_surface`. They are only used on the render thread, by the window's
// renderer, to make the window's surface again; the host keeps the window
// until that renderer is dropped (`Renderer::new`'s surface, main_thread.rs),
// as for the surface first made from them.
unsafe impl Send for WindowHandles {}
// SAFETY: as above; nothing is written through them.
unsafe impl Sync for WindowHandles {}

impl WindowHandles {
    /// None off Wayland, or without the handles.
    pub(crate) fn wayland(
        instance: &wgpu::Instance,
        window: &(impl wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle),
    ) -> Option<Self> {
        let window_handle = window.window_handle().ok()?.as_raw();
        let display = window.display_handle().ok()?.as_raw();
        matches!(window_handle, wgpu::rwh::RawWindowHandle::Wayland(_)).then(|| Self {
            instance: instance.clone(),
            display,
            window: window_handle,
        })
    }

    /// A new surface on the window. The old one must be gone first: the
    /// driver's `wp_fifo_v1` and `wp_commit_timer_v1` exist once for each
    /// `wl_surface`, and a second of either is a protocol error.
    ///
    /// # Safety
    /// The window must still be alive (the caller is its renderer).
    pub(crate) unsafe fn surface(&self) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError> {
        // SAFETY: the caller's promise, and the handles are the window's own.
        unsafe {
            self.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(self.display),
                raw_window_handle: self.window,
            })
        }
    }
}

/// Swapchain recreations since the start (every `configure` below).
static RECREATIONS: AtomicU64 = AtomicU64::new(0);

/// Configures the surface, counted. wgpu first waits for the GPU to finish
/// its work, then destroys and recreates the swapchain (wgpu-core
/// `Device::configure_surface`): a window resized by its edge does it on
/// almost every frame, 33 ms each in demo at 3200 x 1800 (RTX 3060 Laptop). The
/// crash reports say how many there were just before the end (#1701, a
/// device lost while the window was being resized).
///
/// sl-ngx's presenter, if one runs, is paused through it (dlss.rs
/// `pause_presenter`): wgpu waits for the whole device when it makes a
/// swapchain again, which no queue may be used beside.
pub(crate) fn configure(surface: &wgpu::Surface<'_>, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
    RECREATIONS.fetch_add(1, Ordering::Relaxed);
    #[cfg(all(feature = "dlss", target_os = "linux"))]
    let _paused = crate::dlss::pause_presenter();
    surface.configure(device, config);
}

/// Swapchain recreations since the start.
pub fn recreations() -> u64 {
    RECREATIONS.load(Ordering::Relaxed)
}

/// Stable name of a mode for the log, the profile and the bug reports.
pub fn name(mode: PresentMode) -> &'static str {
    match mode {
        PresentMode::Immediate => "Immediate",
        PresentMode::Mailbox => "Mailbox",
        PresentMode::Fifo => "Fifo",
        PresentMode::FifoRelaxed => "FifoRelaxed",
        PresentMode::AutoVsync => "AutoVsync",
        PresentMode::AutoNoVsync => "AutoNoVsync",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};

    const ALL: [PresentMode; 4] = [Fifo, FifoRelaxed, Mailbox, Immediate];

    #[test]
    fn vsync_off_never_waits_for_the_screen_when_the_surface_allows_it() {
        assert_eq!(choose(&ALL, false, None), Mailbox);
        assert_eq!(choose(&[Fifo, Immediate], false, None), Immediate);
        assert_eq!(choose(&[Fifo, FifoRelaxed, Immediate], false, None), Immediate);
        // nothing but synchronized modes: the one every surface has
        assert_eq!(choose(&[Fifo, FifoRelaxed], false, None), Fifo);
        assert_eq!(choose(&[], false, None), Fifo);
    }

    #[test]
    fn vsync_on_is_always_synchronized() {
        assert_eq!(choose(&ALL, true, None), FifoRelaxed);
        assert_eq!(choose(&[Fifo, Mailbox, Immediate], true, None), Fifo);
        assert_eq!(choose(&[], true, None), Fifo);
    }

    #[test]
    fn chosen_mode_is_always_one_the_surface_has() {
        for supported in [&ALL[..], &[Fifo, Immediate], &[Fifo, Mailbox], &[Fifo, FifoRelaxed], &[Fifo]] {
            for vsync in [false, true] {
                for forced in [None, Some(Immediate), Some(Mailbox), Some(Fifo), Some(FifoRelaxed)] {
                    let mode = choose(supported, vsync, forced);
                    assert!(supported.contains(&mode), "{supported:?} vsync={vsync} forced={forced:?}: {mode:?}");
                }
            }
        }
    }

    #[test]
    fn forced_mode_only_when_supported() {
        assert_eq!(choose(&ALL, false, Some(Immediate)), Immediate);
        assert_eq!(choose(&ALL, true, Some(Immediate)), Immediate);
        assert_eq!(choose(&ALL, false, Some(Fifo)), Fifo);
        // not offered by the surface: the usual rule
        assert_eq!(choose(&[Fifo, Mailbox], false, Some(Immediate)), Mailbox);
        assert_eq!(choose(&[Fifo], true, Some(Mailbox)), Fifo);
    }

    #[test]
    fn surface_made_again_only_for_frame_generation_from_an_unpaced_mode() {
        assert!(renews_surface(Mailbox, Fifo, true));
        assert!(renews_surface(Immediate, Fifo, true));
        // the timer was kept fed: vsync on, or Fifo already
        assert!(!renews_surface(FifoRelaxed, Fifo, true));
        assert!(!renews_surface(Fifo, Fifo, true));
        // frame generation off, or AURORA_PRESENT_MODE keeping another mode
        assert!(!renews_surface(Mailbox, Fifo, false));
        assert!(!renews_surface(Fifo, Mailbox, false));
        assert!(!renews_surface(Mailbox, Immediate, true));
    }

    #[test]
    fn present_mode_names() {
        assert_eq!(parse("immediate"), Some(Immediate));
        assert_eq!(parse(" Mailbox "), Some(Mailbox));
        assert_eq!(parse("FIFO"), Some(Fifo));
        assert_eq!(parse("fifo-relaxed"), Some(FifoRelaxed));
        assert_eq!(parse("fifo_relaxed"), Some(FifoRelaxed));
        assert_eq!(parse("FifoRelaxed"), Some(FifoRelaxed));
        for bad in ["", "vsync", "auto", "1"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        for mode in ALL {
            assert_eq!(parse(name(mode)), Some(mode));
        }
    }
}
