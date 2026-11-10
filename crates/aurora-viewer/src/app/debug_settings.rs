//! Debug Settings (Advanced.ShowDebugSettings): the window of
//! `ui/debug_settings.rs`, RLVa's locks and what a change sets going.
use super::*;

impl App {
    /// Each frame.
    pub(super) fn debug_settings_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo {
            self.demo_debug_settings();
        }
        let mut open = self.panels.debug_settings;
        // the same rules as `options_ui.rlva_*`, and any restriction at all
        let locks = crate::settings::registry::Locks {
            camera_preset: self.world.rlva.camera_preset_locked(uuid::Uuid::nil()),
            camera_fov: self.world.rlva.camera_owner().is_some(),
            sendim: self.world.rlva.has("sendim"),
            restricted: !self.world.rlva.restrictions().is_empty(),
        };
        let changed = self.debug_settings_ui.show(ctx, p, &mut open, &mut self.settings, locks);
        self.panels.debug_settings = open;
        // applied and saved like a change of Preferences by the frame's
        // settings pass
        if changed {
            self.folders.settings_changed = true;
        }
    }

    /// AURORA_DEMO_DEBUG_SETTINGS=1: the window opened at the start, past its
    /// warning; `warning`: on its warning; any other value is the search typed
    /// in it.
    fn demo_debug_settings(&mut self) {
        if self.debug_settings_demo_done {
            return;
        }
        let Some(mode) = std::env::var_os("AURORA_DEMO_DEBUG_SETTINGS") else {
            return;
        };
        self.debug_settings_demo_done = true;
        let mode = mode.to_string_lossy();
        self.debug_settings_ui.demo(if mode == "1" { "" } else { &mode });
        ui::widgets::open_floater(&self.egui_ctx, ui::debug_settings::WINDOW, &mut self.panels.debug_settings);
    }
}
