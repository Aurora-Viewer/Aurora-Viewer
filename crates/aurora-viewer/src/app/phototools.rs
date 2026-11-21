//! The Phototools window (the toolbar's `phototools` command): the window
//! of `ui/phototools.rs` and what a change of it sets going.
use super::*;

impl App {
    /// Each frame, with the environment window: the actions of the
    /// environment block come back for `env_actions`.
    pub(super) fn phototools_window(
        &mut self,
        ctx: &egui::Context,
        p: &crate::theme::Palette,
        lighting: &mut bool,
    ) -> Vec<ui::environment::EnvAction> {
        if self.demo {
            self.demo_phototools();
        }
        let mut open = self.panels.phototools;
        let out = self.phototools_ui.show(
            ctx,
            p,
            &mut open,
            &mut self.settings,
            &self.options_ui,
            ui::phototools::Env {
                ui: &mut self.env_ui,
                view: self.world.eep.local_view(),
                loading: self.world.eep.local_loading(),
                time_of_day: self.panels.time_of_day,
                lighting,
                clouds: &mut self.snapshots.ui.light.clouds_paused,
            },
        );
        self.panels.phototools = open;
        // applied like a change of Preferences (draw distance, render
        // settings…) by the frame's settings pass
        if out.changed {
            self.folders.settings_changed = true;
        }
        // "New Sky / Water / Day Preset", as the World menu's entries
        if let Some(kind) = out.new_preset {
            self.open_new_environment(kind);
        }
        if out.toggle_flycam {
            self.toggle_flycam_from_input(false);
        }
        out.env
    }

    /// AURORA_DEMO_PHOTOTOOLS=<tab>: the window opened at the start, on
    /// that tab (`env` or `general`; `1` the first).
    fn demo_phototools(&mut self) {
        if self.phototools_demo_done {
            return;
        }
        let Some(value) = std::env::var_os("AURORA_DEMO_PHOTOTOOLS") else {
            return;
        };
        self.phototools_demo_done = true;
        self.phototools_ui.tab = ui::phototools::Tab::parse(&value.to_string_lossy()).unwrap_or(0);
        ui::widgets::open_floater(&self.egui_ctx, ui::phototools::WINDOW, &mut self.panels.phototools);
    }
}
