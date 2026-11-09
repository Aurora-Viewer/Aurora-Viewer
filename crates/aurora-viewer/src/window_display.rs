//! Placement of the native viewer window across monitor topology and DPI changes.

use serde::{Deserialize, Serialize};
use winit::monitor::MonitorHandle;

const SCALE_FACTOR_DENOMINATOR: u32 = 1000;
const DECORATION_WIDTH: u32 = 16;
const DECORATION_HEIGHT: u32 = 40;
const REACHABLE_TITLE_WIDTH: i64 = 96;
const REACHABLE_TITLE_HEIGHT: i64 = 40;

/// The native screen the viewer occupied when its restored window geometry was saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedDisplay {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Scale factor in thousandths, avoiding non-finite persisted floats.
    pub scale_milli: u32,
}

impl SavedDisplay {
    pub fn from_monitor(monitor: &MonitorHandle) -> Self {
        let position = monitor.position();
        let size = monitor.size();
        Self {
            name: monitor
                .name()
                .filter(|name| !name.trim().is_empty())
                .map(|name| name.chars().take(256).collect()),
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
            scale_milli: scale_milli(monitor.scale_factor()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_milli: u32,
}

impl From<&MonitorHandle> for Display {
    fn from(monitor: &MonitorHandle) -> Self {
        let saved = SavedDisplay::from_monitor(monitor);
        Self {
            name: saved.name,
            x: saved.x,
            y: saved.y,
            width: saved.width,
            height: saved.height,
            scale_milli: saved.scale_milli,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupPlacement {
    pub monitor_index: usize,
    pub x: i32,
    pub y: i32,
}

/// Selects and clamps a restored physical position. With no usable saved screen,
/// place the window on the primary display rather than leaving it unreachable.
pub fn startup_placement(
    enabled: bool,
    saved_display: Option<&SavedDisplay>,
    saved_position: Option<(i32, i32)>,
    logical_size: (u32, u32),
    displays: &[Display],
    primary: usize,
) -> Option<StartupPlacement> {
    if !enabled || displays.is_empty() || primary >= displays.len() {
        return None;
    }

    let (saved_x, saved_y) = saved_position?;
    let target = saved_display.and_then(|saved| matching_display(saved, displays));
    let index = target
        .or_else(|| displays.iter().position(|display| titlebar_reachable(display, saved_x, saved_y)))
        .unwrap_or(primary);
    let display = &displays[index];

    let (x, y) = if let Some(saved) = saved_display.filter(|_| target.is_some()) {
        let offset_x = (saved_x as i64 - saved.x as i64) * display.scale_milli as i64 / saved.scale_milli.max(1) as i64;
        let offset_y = (saved_y as i64 - saved.y as i64) * display.scale_milli as i64 / saved.scale_milli.max(1) as i64;
        (display.x as i64 + offset_x, display.y as i64 + offset_y)
    } else if titlebar_reachable(display, saved_x, saved_y) {
        (saved_x as i64, saved_y as i64)
    } else {
        // A disconnected display or an old off-screen position: center on the
        // primary display instead of retaining an unreachable rectangle.
        let (width, height) = decorated_size(logical_size, display.scale_milli);
        (
            display.x as i64 + (display.width as i64 - (width as i64).min(display.width as i64)).max(0) / 2,
            display.y as i64 + (display.height as i64 - (height as i64).min(display.height as i64)).max(0) / 2,
        )
    };
    let (x, y) = clamp_to_display(display, x, y, logical_size);
    Some(StartupPlacement {
        monitor_index: index,
        x,
        y,
    })
}

/// True when a useful strip of the title area intersects this display.
pub fn titlebar_reachable(display: &Display, x: i32, y: i32) -> bool {
    let (left, top) = (display.x as i64, display.y as i64);
    let (right, bottom) = (left + display.width as i64, top + display.height as i64);
    let (x, y) = (x as i64, y as i64);
    x + REACHABLE_TITLE_WIDTH > left && x < right && y + REACHABLE_TITLE_HEIGHT > top && y < bottom
}

/// Keeps the window wholly visible when it fits, and its title area reachable
/// when a screen is smaller than the window's minimum size.
pub fn clamp_to_display(display: &Display, x: i64, y: i64, logical_size: (u32, u32)) -> (i32, i32) {
    let (window_width, window_height) = decorated_size(logical_size, display.scale_milli);
    let (left, top) = (display.x as i64, display.y as i64);
    let (right, bottom) = (left + display.width as i64, top + display.height as i64);
    let max_x = (right - window_width as i64).max(left);
    let max_y = (bottom - window_height as i64).max(top);
    (
        x.clamp(left, max_x).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        y.clamp(top, max_y).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
    )
}

fn decorated_size(logical_size: (u32, u32), scale: u32) -> (u32, u32) {
    let scale = scale.max(1) as u64;
    let width = (logical_size.0 as u64 + DECORATION_WIDTH as u64) * scale / SCALE_FACTOR_DENOMINATOR as u64;
    let height = (logical_size.1 as u64 + DECORATION_HEIGHT as u64) * scale / SCALE_FACTOR_DENOMINATOR as u64;
    (width.min(u32::MAX as u64) as u32, height.min(u32::MAX as u64) as u32)
}

fn matching_display(saved: &SavedDisplay, displays: &[Display]) -> Option<usize> {
    let named: Vec<_> = displays
        .iter()
        .enumerate()
        .filter(|(_, display)| saved.name.as_ref().is_some_and(|name| display.name.as_ref() == Some(name)))
        .collect();
    if !named.is_empty() {
        return named
            .into_iter()
            .min_by_key(|(_, display)| display_distance(saved, display))
            .map(|(index, _)| index);
    }

    displays
        .iter()
        .enumerate()
        .filter(|(_, display)| {
            saved.width == display.width && saved.height == display.height && scale_difference(saved.scale_milli, display.scale_milli) <= 50
        })
        .min_by_key(|(_, display)| display_distance(saved, display))
        .map(|(index, _)| index)
}

fn display_distance(saved: &SavedDisplay, display: &Display) -> u64 {
    let position = (saved.x as i64 - display.x as i64).unsigned_abs() + (saved.y as i64 - display.y as i64).unsigned_abs();
    let dimensions = (saved.width.abs_diff(display.width) as u64 + saved.height.abs_diff(display.height) as u64) * 4;
    let scale = scale_difference(saved.scale_milli, display.scale_milli) as u64 * 100;
    position + dimensions + scale
}

fn scale_difference(a: u32, b: u32) -> u32 {
    a.abs_diff(b)
}

fn scale_milli(scale: f64) -> u32 {
    if scale.is_finite() && scale > 0.0 {
        (scale * SCALE_FACTOR_DENOMINATOR as f64).round().clamp(250.0, 10_000.0) as u32
    } else {
        SCALE_FACTOR_DENOMINATOR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(name: &str, x: i32, y: i32, width: u32, height: u32, scale_milli: u32) -> Display {
        Display {
            name: Some(name.into()),
            x,
            y,
            width,
            height,
            scale_milli,
        }
    }

    #[test]
    fn keeps_the_window_on_its_named_display_when_monitor_order_and_origin_change() {
        let saved = SavedDisplay {
            name: Some("Panel A".into()),
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale_milli: 1000,
        };
        let displays = [
            display("Panel B", 0, 0, 2560, 1440, 1500),
            display("Panel A", 2560, -200, 1920, 1080, 1250),
        ];
        let placement = startup_placement(true, Some(&saved), Some((-1820, 80)), (1200, 800), &displays, 0).unwrap();
        assert_eq!(placement.monitor_index, 1);
        assert_eq!((placement.x, placement.y), (2685, -170));
    }

    #[test]
    fn missing_display_falls_back_to_a_centered_primary_position() {
        let saved = SavedDisplay {
            name: Some("Disconnected".into()),
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale_milli: 1000,
        };
        let displays = [display("Primary", 0, 0, 1600, 900, 1000)];
        let placement = startup_placement(true, Some(&saved), Some((-1800, 100)), (1200, 700), &displays, 0).unwrap();
        assert_eq!(placement.monitor_index, 0);
        assert_eq!((placement.x, placement.y), (192, 80));
    }

    #[test]
    fn keeps_a_legacy_saved_position_on_any_connected_monitor() {
        let displays = [
            display("Primary", 0, 0, 1600, 900, 1000),
            display("Side", -1280, 0, 1280, 1024, 1000),
        ];
        let placement = startup_placement(true, None, Some((-1200, 100)), (800, 600), &displays, 0).unwrap();
        assert_eq!(placement.monitor_index, 1);
        assert_eq!((placement.x, placement.y), (-1200, 100));
    }

    #[test]
    fn disabled_recovery_leaves_placement_to_the_operating_system() {
        let displays = [display("Primary", 0, 0, 1600, 900, 1000)];
        assert_eq!(startup_placement(false, None, Some((-3000, 0)), (1200, 700), &displays, 0), None);
    }

    #[test]
    fn clamps_window_to_bounds_and_keeps_a_title_strip_reachable_on_small_screens() {
        let large = display("Large", 0, 0, 1600, 900, 1000);
        assert_eq!(clamp_to_display(&large, 1500, 850, (1200, 700)), (384, 160));
        let small = display("Small", -320, -200, 640, 480, 1000);
        let (x, y) = clamp_to_display(&small, -900, -800, (1200, 700));
        assert_eq!((x, y), (-320, -200));
        assert!(titlebar_reachable(&small, x, y));
    }
}
