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

/// Swapchain recreations since the start (every `configure` below).
static RECREATIONS: AtomicU64 = AtomicU64::new(0);

/// Configures the surface, counted. wgpu first waits for the GPU to finish
/// its work, then destroys and recreates the swapchain (wgpu-core
/// `Device::configure_surface`): a window resized by its edge does it on
/// almost every frame, 33 ms each in demo at 3200 x 1800 (RTX 3060 Laptop). The
/// crash reports say how many there were just before the end (#1701, a
/// device lost while the window was being resized).
pub(crate) fn configure(surface: &wgpu::Surface<'_>, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) {
    RECREATIONS.fetch_add(1, Ordering::Relaxed);
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
