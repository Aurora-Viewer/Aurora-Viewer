//! Quick preferences (the toolbar's `quickprefs`): the window of
//! `ui/quick_prefs.rs` and what a change of it sets going.
use super::*;

impl App {
    /// Each frame, with the environment window: the actions of the
    /// environment block come back for `env_actions`.
    pub(super) fn quick_prefs_window(
        &mut self,
        ctx: &egui::Context,
        p: &crate::theme::Palette,
        lighting: &mut bool,
    ) -> Vec<ui::environment::EnvAction> {
        if self.demo {
            self.demo_quick_prefs();
        }
        let mut open = self.panels.quick_prefs;
        let hover_enabled = self.world.hover_height_available();
        let out = self.quick_prefs_ui.show(
            ctx,
            p,
            &mut open,
            &mut self.settings,
            ui::quick_prefs::Hover {
                height: &mut self.world.hover_height,
                enabled: hover_enabled,
            },
            &mut self.env_ui,
            self.world.eep.local_view(),
            self.world.eep.local_loading(),
            lighting,
        );
        self.panels.quick_prefs = open;
        // applied like a change of Preferences (bandwidth, draw distance,
        // render settings…) by the frame's settings pass
        if out.changed {
            self.folders.settings_changed = true;
        }
        out.env
    }

    /// AURORA_DEMO_QUICKPREFS=1: the window opened at the start.
    fn demo_quick_prefs(&mut self) {
        if self.quick_prefs_demo_done || std::env::var_os("AURORA_DEMO_QUICKPREFS").is_none() {
            return;
        }
        self.quick_prefs_demo_done = true;
        ui::widgets::open_floater(&self.egui_ctx, ui::quick_prefs::WINDOW, &mut self.panels.quick_prefs);
    }
}
