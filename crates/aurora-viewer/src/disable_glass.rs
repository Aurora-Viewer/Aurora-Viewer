//! Disable glass (Preferences › Accessibility › Vision): windows, bars,
//! menus, name plates and dialogs stay opaque, without frosting the world
//! behind them. High contrast already does this by making surfaces opaque
//! (`crate::high_contrast`). The Legacy Viewer has no such option; this is
//! Aurora's own.
//!
//! The skin's colours stay. `panel_alpha` becomes 255, so every glass check
//! skips the frosted pass, and `aurora_render::set_ui_blur` turns off the
//! modal backdrop blur. The choice is process-wide (`set`), read where the
//! palette is built.

use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// Whether the interface skips its glass from this on (the palette is
/// built again by its users).
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
    aurora_render::set_ui_blur(!on);
}

pub fn current() -> bool {
    ON.load(Ordering::Relaxed)
}

/// `AURORA_DEMO_GLASS=0|1` (demo mode only): this run's choice, never saved
/// in the settings. `0` turns the glass off (`1` forces it on).
pub fn demo_override() -> Option<bool> {
    match std::env::var("AURORA_DEMO_GLASS").ok()?.trim() {
        "0" | "off" | "disable" => Some(true),
        "1" | "on" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn off_by_default() {
        assert!(!crate::settings::Settings::default().disable_glass);
    }
}
