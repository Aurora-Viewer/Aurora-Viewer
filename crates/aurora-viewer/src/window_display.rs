//! Display identity and recovery for native window placement.
//!
//! This is geometry support for the existing viewer and detached-window
//! paths, not a second window manager. Coordinates are physical pixels;
//! saved window sizes are logical points.

use serde::{Deserialize, Serialize};
use winit::dpi::PhysicalPosition;
use winit::monitor::MonitorHandle;
use winit::window::Window;

const SCALE_FACTOR_DENOMINATOR: u32 = 1000;
const DECORATION_WIDTH: u32 = 16;
const DECORATION_HEIGHT: u32 = 40;
const TITLE_WIDTH: i64 = 96;
const TITLE_HEIGHT: i64 = 40;
const REACHABLE_WIDTH: i64 = 40;
const REACHABLE_HEIGHT: i64 = 10;

/// The display identity and geometry at the time window placement was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SavedDisplay {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Scale factor in thousandths, avoiding non-finite persisted floats.
    pub scale_milli: u32,
}

impl Default for SavedDisplay {
    fn default() -> Self {
        Self {
            name: None,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scale_milli: SCALE_FACTOR_DENOMINATOR,
        }
    }
}

impl SavedDisplay {
    pub(crate) fn from_monitor(monitor: &MonitorHandle) -> Self {
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
pub(crate) struct Display {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_milli: u32,
}

impl Display {
    pub(crate) fn saved(&self) -> SavedDisplay {
        SavedDisplay {
            name: self.name.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            scale_milli: self.scale_milli,
        }
    }
}

impl From<&MonitorHandle> for Display {
    fn from(monitor: &MonitorHandle) -> Self {
        SavedDisplay::from_monitor(monitor).into()
    }
}

impl From<SavedDisplay> for Display {
    fn from(saved: SavedDisplay) -> Self {
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

impl From<&SavedDisplay> for Display {
    fn from(saved: &SavedDisplay) -> Self {
        saved.clone().into()
    }
}

/// Persisted placement of one detached interface window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SavedWindowPlacement {
    pub position: Option<(i32, i32)>,
    pub display: Option<SavedDisplay>,
}

impl SavedWindowPlacement {
    pub(crate) fn from_window(window: &Window) -> Option<Self> {
        let position = window.outer_position().ok().map(|p| (p.x, p.y))?;
        Some(Self {
            position: Some(position),
            display: window.current_monitor().as_ref().map(SavedDisplay::from_monitor),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    pub monitor_index: usize,
    pub position: PhysicalPosition<i32>,
}

/// Finds the current monitor corresponding to a winit handle.
pub(crate) fn monitor_index(monitor: Option<&MonitorHandle>, displays: &[Display]) -> Option<usize> {
    let saved = monitor.map(SavedDisplay::from_monitor)?;
    matching_display(&saved, displays)
}

/// Restores a physical position on the saved display when it still exists,
/// scaling its offset when that display's DPI or origin changed. Otherwise it
/// keeps a reachable position or centers on `home`. `asked` is the position
/// requested by the current viewer/window layout. No monitor data means the
/// caller should leave placement to the operating system.
pub(crate) fn restore_placement(
    saved_display: Option<&SavedDisplay>,
    saved_position: Option<(i32, i32)>,
    asked: Option<(i32, i32)>,
    logical_size: (u32, u32),
    displays: &[Display],
    home: usize,
) -> Option<Placement> {
    if displays.is_empty() {
        return None;
    }
    let home = if home < displays.len() { home } else { 0 };
    let matched = saved_position.and_then(|_| saved_display.and_then(|saved| matching_display(saved, displays)));
    let saved_reachable = saved_position.and_then(|(x, y)| displays.iter().position(|d| titlebar_reachable(d, x, y)));
    let asked_reachable = asked.and_then(|(x, y)| displays.iter().position(|d| titlebar_reachable(d, x, y)));
    let monitor_index = matched.or(saved_reachable).or(asked_reachable).unwrap_or(home);
    let display = &displays[monitor_index];

    if let (Some(saved), Some((x, y)), Some(index)) = (saved_display, saved_position, matched) {
        let target = &displays[index];
        if same_geometry(saved, target) && titlebar_reachable(target, x, y) {
            return Some(Placement {
                monitor_index: index,
                position: PhysicalPosition::new(x, y),
            });
        }
        let scale = i64::from(target.scale_milli);
        let denominator = i64::from(saved.scale_milli.max(1));
        let translated = (
            i64::from(target.x) + (i64::from(x) - i64::from(saved.x)) * scale / denominator,
            i64::from(target.y) + (i64::from(y) - i64::from(saved.y)) * scale / denominator,
        );
        let (x, y) = clamp_to_display(target, translated.0, translated.1, logical_size);
        return Some(Placement {
            monitor_index: index,
            position: PhysicalPosition::new(x, y),
        });
    }

    if let Some((x, y)) = saved_position
        && titlebar_reachable(display, x, y)
    {
        return Some(Placement {
            monitor_index,
            position: PhysicalPosition::new(x, y),
        });
    }
    if let Some((x, y)) = asked
        && asked_reachable == Some(monitor_index)
        && titlebar_reachable(display, x, y)
    {
        return Some(Placement {
            monitor_index,
            position: PhysicalPosition::new(x, y),
        });
    }

    let (x, y) = center_on_display(display, logical_size);
    Some(Placement {
        monitor_index,
        position: PhysicalPosition::new(x, y),
    })
}

/// A useful part of the title bar must remain visible to recover a window.
pub(crate) fn titlebar_reachable(display: &Display, x: i32, y: i32) -> bool {
    let left = i64::from(display.x);
    let top = i64::from(display.y);
    let right = left + i64::from(display.width);
    let bottom = top + i64::from(display.height);
    let x = i64::from(x);
    let y = i64::from(y);
    let visible_width = (x + TITLE_WIDTH).min(right) - x.max(left);
    let visible_height = (y + TITLE_HEIGHT).min(bottom) - y.max(top);
    visible_width >= REACHABLE_WIDTH && visible_height >= REACHABLE_HEIGHT
}

/// Keep a restored window whole on its display when it fits; on smaller
/// displays, keep its upper-left title area reachable.
pub(crate) fn clamp_to_display(display: &Display, x: i64, y: i64, logical_size: (u32, u32)) -> (i32, i32) {
    let (window_width, window_height) = decorated_size(logical_size, display.scale_milli);
    let left = i64::from(display.x);
    let top = i64::from(display.y);
    let right = left + i64::from(display.width);
    let bottom = top + i64::from(display.height);
    let max_x = (right - i64::from(window_width)).max(left);
    let max_y = (bottom - i64::from(window_height)).max(top);
    (
        x.clamp(left, max_x).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y.clamp(top, max_y).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    )
}

fn center_on_display(display: &Display, logical_size: (u32, u32)) -> (i32, i32) {
    let (width, height) = decorated_size(logical_size, display.scale_milli);
    let x = i64::from(display.x) + (i64::from(display.width) - i64::from(width).min(i64::from(display.width))).max(0) / 2;
    let y = i64::from(display.y) + (i64::from(display.height) - i64::from(height).min(i64::from(display.height))).max(0) / 2;
    clamp_to_display(display, x, y, logical_size)
}

fn decorated_size(logical_size: (u32, u32), scale_milli: u32) -> (u32, u32) {
    let scale = u64::from(scale_milli.max(1));
    let width = (u64::from(logical_size.0) + u64::from(DECORATION_WIDTH)) * scale / u64::from(SCALE_FACTOR_DENOMINATOR);
    let height = (u64::from(logical_size.1) + u64::from(DECORATION_HEIGHT)) * scale / u64::from(SCALE_FACTOR_DENOMINATOR);
    (width.min(u64::from(u32::MAX)) as u32, height.min(u64::from(u32::MAX)) as u32)
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
            saved.width == display.width && saved.height == display.height && saved.scale_milli.abs_diff(display.scale_milli) <= 50
        })
        .min_by_key(|(_, display)| display_distance(saved, display))
        .map(|(index, _)| index)
}

fn display_distance(saved: &SavedDisplay, display: &Display) -> u64 {
    let position = (i64::from(saved.x) - i64::from(display.x)).unsigned_abs() + (i64::from(saved.y) - i64::from(display.y)).unsigned_abs();
    let dimensions = (u64::from(saved.width.abs_diff(display.width)) + u64::from(saved.height.abs_diff(display.height))) * 4;
    let scale = u64::from(saved.scale_milli.abs_diff(display.scale_milli)) * 100;
    position + dimensions + scale
}

fn same_geometry(saved: &SavedDisplay, display: &Display) -> bool {
    saved.x == display.x
        && saved.y == display.y
        && saved.width == display.width
        && saved.height == display.height
        && saved.scale_milli == display.scale_milli
}

fn scale_milli(scale: f64) -> u32 {
    if scale.is_finite() && scale > 0.0 {
        (scale * f64::from(SCALE_FACTOR_DENOMINATOR)).round().clamp(250.0, 10_000.0) as u32
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
    fn a_reachable_single_monitor_position_is_unchanged() {
        let screen = display("Primary", 0, 0, 1920, 1080, 1000);
        let saved = SavedDisplay {
            name: Some("Primary".into()),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale_milli: 1000,
        };
        let placement = restore_placement(Some(&saved), Some((120, 90)), None, (800, 600), &[screen], 0).unwrap();
        assert_eq!(placement.position, PhysicalPosition::new(120, 90));
    }

    #[test]
    fn follows_the_named_display_when_order_origin_and_scale_change() {
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
        let placement = restore_placement(Some(&saved), Some((-1820, 80)), None, (1200, 800), &displays, 0).unwrap();
        assert_eq!(placement.monitor_index, 1);
        assert_eq!(placement.position, PhysicalPosition::new(2685, -170));
    }

    #[test]
    fn a_disconnected_display_falls_back_to_the_home_display_center() {
        let saved = SavedDisplay {
            name: Some("Disconnected".into()),
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale_milli: 1000,
        };
        let displays = [
            display("Primary", 0, 0, 1600, 900, 1000),
            display("Side", 1600, 0, 1280, 1024, 1000),
        ];
        let placement = restore_placement(Some(&saved), Some((-1800, 100)), None, (1200, 700), &displays, 1).unwrap();
        assert_eq!(placement.monitor_index, 1);
        assert_eq!(placement.position, PhysicalPosition::new(1632, 142));
    }

    #[test]
    fn a_legacy_position_still_visible_on_any_display_is_preserved() {
        let displays = [
            display("Primary", 0, 0, 1600, 900, 1000),
            display("Side", -1280, 0, 1280, 1024, 1000),
        ];
        let placement = restore_placement(None, Some((-1200, 100)), None, (800, 600), &displays, 0).unwrap();
        assert_eq!(placement.monitor_index, 1);
        assert_eq!(placement.position, PhysicalPosition::new(-1200, 100));
    }

    #[test]
    fn an_offscreen_asked_position_uses_the_home_display() {
        let displays = [display("Primary", 0, 0, 1600, 900, 1000)];
        let placement = restore_placement(None, None, Some((-32000, -32000)), (800, 600), &displays, 0).unwrap();
        assert_eq!(placement.position, PhysicalPosition::new(392, 130));
    }

    #[test]
    fn a_small_display_keeps_the_title_area_reachable_for_a_large_window() {
        let screen = display("Small", -320, -200, 640, 480, 1000);
        let (x, y) = clamp_to_display(&screen, -900, -800, (1200, 700));
        assert_eq!((x, y), (-320, -200));
        assert!(titlebar_reachable(&screen, x, y));
    }

    #[test]
    fn no_display_information_leaves_the_window_to_the_caller() {
        assert!(restore_placement(None, Some((-3000, 0)), None, (800, 600), &[], 0).is_none());
    }

    #[test]
    fn saved_outside_placement_round_trips_for_settings_persistence() {
        let saved = SavedWindowPlacement {
            position: Some((-1820, 80)),
            display: Some(SavedDisplay {
                name: Some("Panel A".into()),
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
                scale_milli: 1250,
            }),
        };
        let encoded = serde_json::to_string(&saved).unwrap();
        let decoded: SavedWindowPlacement = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, saved);
    }
}
