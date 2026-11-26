//! A screen's name and size, read without panicking.
//!
//! winit 0.30's Windows `MonitorHandle::name` and `size` unwrap
//! `GetMonitorInfoW` (winit `platform_impl/windows/monitor.rs`), which fails
//! for a screen Windows no longer describes: a handle kept from before the
//! screen was turned off, unplugged or put to sleep, or a remote session
//! changing its displays. The viewer panicked there in `ScreenRate::poll`
//! (#4851); the start's window placement, the windows drawn outside the
//! viewer, the splash and the crash window read the size the same way, and
//! winit sizes a borderless fullscreen window with it. On
//! Windows both are read here from `GetMonitorInfoW`, and a screen that
//! answers nothing has no name and no size; elsewhere winit's own calls do
//! not panic and are used as they are.

use winit::dpi::PhysicalSize;
use winit::monitor::MonitorHandle;

/// The screen's name (Windows: its device name, `\\.\DISPLAY1`), or None
/// when the system no longer describes it.
pub fn name(monitor: &MonitorHandle) -> Option<String> {
    #[cfg(windows)]
    {
        win::info(monitor).map(|info| device_name(&info.szDevice))
    }
    #[cfg(not(windows))]
    {
        monitor.name()
    }
}

/// The screen's size in pixels, or None when the system no longer
/// describes it.
pub fn size(monitor: &MonitorHandle) -> Option<PhysicalSize<u32>> {
    #[cfg(windows)]
    {
        win::info(monitor).map(|info| {
            let r = info.monitorInfo.rcMonitor;
            rect_size(r.left, r.top, r.right, r.bottom)
        })
    }
    #[cfg(not(windows))]
    {
        Some(monitor.size())
    }
}

/// Borderless fullscreen on `monitor`. On Windows, None when the system no
/// longer describes that screen (or there is none): winit 0.30 sizes the
/// window with the unwrapping `size()`, and resolves `Borderless(None)` to the
/// current screen the same way (platform_impl/windows/window.rs), so the
/// window stays as it is instead. Elsewhere as before: Wayland may know no
/// current screen and fills the one the window is on.
pub fn borderless(monitor: Option<MonitorHandle>) -> Option<winit::window::Fullscreen> {
    #[cfg(windows)]
    {
        let monitor = monitor?;
        size(&monitor)?;
        Some(winit::window::Fullscreen::Borderless(Some(monitor)))
    }
    #[cfg(not(windows))]
    {
        Some(winit::window::Fullscreen::Borderless(monitor))
    }
}

/// A wide device name up to its first NUL, as winit decodes it.
#[cfg(any(windows, test))]
fn device_name(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

/// The size of a screen rectangle; an inverted one is empty, not huge.
#[cfg(any(windows, test))]
fn rect_size(left: i32, top: i32, right: i32, bottom: i32) -> PhysicalSize<u32> {
    let span = |from: i32, to: i32| u32::try_from(i64::from(to) - i64::from(from)).unwrap_or(0);
    PhysicalSize::new(span(left, right), span(top, bottom))
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW};
    use winit::monitor::MonitorHandle;
    use winit::platform::windows::MonitorHandleExtWindows;

    pub(super) fn info(monitor: &MonitorHandle) -> Option<MONITORINFOEXW> {
        // SAFETY: MONITORINFOEXW is plain data, valid all zeros.
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // (winit hands the HMONITOR as an integer)
        let handle = monitor.hmonitor() as HMONITOR;
        // SAFETY: `info` is a writable MONITORINFOEXW whose cbSize says so,
        // which GetMonitorInfoW fills through its MONITORINFO header; a
        // stale or unknown handle only makes the call fail.
        let ok = unsafe { GetMonitorInfoW(handle, (&raw mut info).cast::<MONITORINFO>()) };
        (ok != 0).then_some(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_name_stops_at_its_nul() {
        let mut wide = [0u16; 32];
        for (slot, c) in wide.iter_mut().zip(r"\\.\DISPLAY2".encode_utf16()) {
            *slot = c;
        }
        assert_eq!(device_name(&wide), r"\\.\DISPLAY2");
        assert_eq!(device_name(&[0; 32]), "");
        // no NUL at all: the whole buffer
        let full: Vec<u16> = "A".repeat(32).encode_utf16().collect();
        assert_eq!(device_name(&full).len(), 32);
    }

    #[test]
    fn a_screen_rectangle_gives_its_size_and_never_wraps() {
        assert_eq!(rect_size(0, 0, 2560, 1440), PhysicalSize::new(2560, 1440));
        // a screen left of and above the main one
        assert_eq!(rect_size(-1920, -1080, 0, 0), PhysicalSize::new(1920, 1080));
        // inverted: empty, not 4 billion pixels
        assert_eq!(rect_size(10, 10, 0, 0), PhysicalSize::new(0, 0));
        // the widest span there is: computed without overflowing i32
        assert_eq!(rect_size(i32::MIN, 0, i32::MAX, 1), PhysicalSize::new(u32::MAX, 1));
    }
}
