//! Process-wide accessibility flags read from Preferences › Accessibility
//! each frame (`set_from_settings`). Kept next to `high_contrast` /
//! `colorblind` so widgets can ask without threading Settings through
//! every paint path.

use crate::settings::Settings;
use std::sync::atomic::{AtomicBool, Ordering};

static UNDERLINE_LINKS: AtomicBool = AtomicBool::new(false);
static LARGER_CONTROLS: AtomicBool = AtomicBool::new(false);
static QUICK_TOOLTIPS: AtomicBool = AtomicBool::new(false);
static SHOW_KEYBOARD_FOCUS: AtomicBool = AtomicBool::new(false);

/// Refresh the process-wide flags from the saved settings (each frame).
pub fn set_from_settings(s: &Settings) {
    UNDERLINE_LINKS.store(s.underline_links, Ordering::Relaxed);
    LARGER_CONTROLS.store(s.larger_controls, Ordering::Relaxed);
    QUICK_TOOLTIPS.store(s.quick_tooltips, Ordering::Relaxed);
    SHOW_KEYBOARD_FOCUS.store(s.show_keyboard_focus, Ordering::Relaxed);
}

/// Always underline chat / app links (not only on hover or focus).
pub fn underline_links() -> bool {
    UNDERLINE_LINKS.load(Ordering::Relaxed)
}

pub fn larger_controls() -> bool {
    LARGER_CONTROLS.load(Ordering::Relaxed)
}

pub fn quick_tooltips() -> bool {
    QUICK_TOOLTIPS.load(Ordering::Relaxed)
}

pub fn show_keyboard_focus() -> bool {
    SHOW_KEYBOARD_FOCUS.load(Ordering::Relaxed)
}
