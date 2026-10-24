//! High contrast (Preferences › Accessibility › Vision, #906): for poor
//! vision, the interface's palette is replaced by a sober one of opaque
//! black surfaces, white text and one light violet accent, whatever the
//! skin's colours. The Legacy Viewer has no such option; this is Aurora's
//! own.
//!
//! - Text meets WCAG 2 AAA (7:1) against every surface it sits on, borders
//!   and the accent's faces the non-text 3:1 (`tests` measures each pair).
//! - The states keep their hue but differ by lightness too: danger darkest,
//!   then warning, then success, so they stay apart without colour.
//! - Surfaces are opaque (`panel_alpha` 255): no glass, the world never
//!   shows through a window. Buttons, fields, windows, islands and
//!   dividers get a light border (`Palette::high_contrast`), which the
//!   usual look leaves out.
//!
//! The choice is process-wide (`set`), read where the palette is built,
//! before the colour blindness adaptation (`crate::colorblind`), which
//! keeps luminances.

use crate::theme::{Palette, hex};
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// Whether the interface uses the high contrast palette from now on (the
/// palette is built again by its users).
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

pub fn current() -> bool {
    ON.load(Ordering::Relaxed)
}

/// `AURORA_DEMO_CONTRAST=1|0` (demo mode only): this run's choice, never
/// saved in the settings.
pub fn demo_override() -> Option<bool> {
    match std::env::var("AURORA_DEMO_CONTRAST").ok()?.trim() {
        "1" | "on" | "high" => Some(true),
        "0" | "off" => Some(false),
        _ => None,
    }
}

/// The high contrast palette. Black surfaces, so that every text colour
/// and border is measured against the same ground; `raised` (button faces,
/// rails) stays dark enough for white and light grey text, and the border
/// tells it from the panel.
pub fn palette() -> Palette {
    Palette {
        violet: hex("#D0BCFF"),
        indigo: hex("#D0BCFF"),
        teal: hex("#8BE9FF"),
        navy: hex("#000000"),
        panel: hex("#000000"),
        bar: hex("#000000"),
        field: hex("#000000"),
        raised: hex("#1A1A1A"),
        violet_light: hex("#E4D8FF"),
        violet_pale: hex("#F0EAFF"),
        ink: hex("#FFFFFF"),
        muted: hex("#E8E8E8"),
        muted_dim: hex("#D3D3D3"),
        amber: hex("#FFD24D"),
        rose: hex("#FF9ECF"),
        danger: hex("#FF7B7B"),
        success: hex("#8CFFA0"),
        warn: hex("#FFB000"),
        danger_dim: hex("#FF7B7B"),
        indigo_light: hex("#D0BCFF"),
        on_accent: hex("#000000"),
        panel_alpha: 255,
        high_contrast: true,
    }
}

#[cfg(test)]
mod tests;
