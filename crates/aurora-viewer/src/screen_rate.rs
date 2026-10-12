//! Refresh rate of the screen the window is on, kept current.
//!
//! The frame limiter needs it outside the world (the login screen never runs
//! above the screen rate) and to know when the vertical synchronization
//! already paces the frames. The window moves between screens of different
//! rates and a screen changes its mode while the viewer runs; winit reports
//! neither, so the screen under the window is asked again from time to time:
//! on Windows `MonitorFromWindow` and `EnumDisplaySettingsExW` for the
//! current mode, a few microseconds.
//!
//! Firestorm reads the rate once, of the primary display, when its window is
//! created (`LLWindowWin32`, indra/llwindow/llwindowwin32.cpp: `mRefreshRate`
//! from `EnumDisplaySettings(NULL, ENUM_CURRENT_SETTINGS)`) and only bounds
//! its automatic frame rate target with it. Each viewer process follows its
//! own window, so two viewers on two screens each keep their own rate.

use std::time::{Duration, Instant};
use winit::monitor::MonitorHandle;
use winit::window::Window;

/// Time between two looks at the screen under the window.
const PERIOD: Duration = Duration::from_secs(1);
/// Rate assumed while the system gives none.
const FALLBACK_HZ: f32 = 60.0;
/// Below this the value is not a rate: Windows reports 0 or 1 for "the
/// default rate of the display hardware" (virtual and remote displays).
const MIN_HZ: f32 = 10.0;

/// Rate in Hz from what the system reports in millihertz.
pub fn rate_hz(millihertz: Option<u32>) -> f32 {
    match millihertz.map(|mhz| mhz as f32 / 1000.0) {
        Some(hz) if hz >= MIN_HZ => hz,
        _ => FALLBACK_HZ,
    }
}

/// The screen is looked at again once `PERIOD` has passed since `checked`.
fn due(checked: Option<Instant>, now: Instant) -> bool {
    checked.is_none_or(|t| now.saturating_duration_since(t) >= PERIOD)
}

pub struct ScreenRate {
    hz: f32,
    monitor: Option<MonitorHandle>,
    checked: Option<Instant>,
}

impl Default for ScreenRate {
    fn default() -> Self {
        Self {
            hz: FALLBACK_HZ,
            monitor: None,
            checked: None,
        }
    }
}

impl ScreenRate {
    /// Current rate of the screen under the window (Hz).
    pub fn hz(&self) -> f32 {
        self.hz
    }

    /// Follow the window: at most once per `PERIOD`, the screen it is on and
    /// the rate of that screen's current mode. Changes go to the log.
    pub fn poll(&mut self, window: &Window, now: Instant) {
        if !due(self.checked, now) {
            return;
        }
        self.checked = Some(now);
        let monitor = window.current_monitor();
        let hz = rate_hz(monitor.as_ref().and_then(|m| m.refresh_rate_millihertz()));
        if monitor != self.monitor || hz != self.hz {
            let name = monitor.as_ref().and_then(|m| m.name()).unwrap_or_else(|| "unknown".into());
            log::info!("screen under the window: {name}, {hz:.0} Hz");
            self.monitor = monitor;
            self.hz = hz;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_from_the_system_value() {
        assert_eq!(rate_hz(Some(144_000)), 144.0);
        assert_eq!(rate_hz(Some(59_940)), 59.94);
        assert_eq!(rate_hz(Some(600_000)), 600.0);
        // no answer, or "hardware default" (0 or 1 Hz): never a 1 fps cap
        assert_eq!(rate_hz(None), 60.0);
        assert_eq!(rate_hz(Some(0)), 60.0);
        assert_eq!(rate_hz(Some(1_000)), 60.0);
        assert_eq!(rate_hz(Some(10_000)), 10.0);
    }

    #[test]
    fn screen_is_looked_at_once_per_period() {
        let t0 = Instant::now();
        assert!(due(None, t0));
        assert!(!due(Some(t0), t0));
        assert!(!due(Some(t0), t0 + Duration::from_millis(999)));
        assert!(due(Some(t0), t0 + PERIOD));
        // a clock read before the last check never panics nor triggers
        assert!(!due(Some(t0 + PERIOD), t0));
    }
}
