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
        // the photo window's values while it is open (the rows follow the
        // view)
        let photo = self.panels.snapshot.then(|| ui::options::PhotoState {
            sun: super::snapshot::env_view(&self.env_ui, &self.world.eep).sun,
            loading: self.world.eep.photo_loading(),
            viewfinder: self.viewfinder_on(),
            boost: self.settings.photo_held.is_some(),
        });
        let director_dof = self.director_dof().is_some();
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
                light: &mut self.snapshots.ui.light,
                photo,
                director_dof,
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
        // "Keep this light" of the banner, as the photo window's button
        if out.keep_light {
            self.keep_photo_light();
        }
        // a photo's value changed, or a saved setting that the photo shows:
        // the preview follows
        if (out.photo_changed || out.changed) && self.panels.snapshot {
            self.snapshots.preview_wanted = true;
        }
        out.env
    }

    /// AURORA_DEMO_PHOTOTOOLS=<tab>[:photo]: the window opened at the start,
    /// on that tab (`env`, `light`, `dof` or `general`), and with `:photo`
    /// the photo window too.
    fn demo_phototools(&mut self) {
        if self.phototools_demo_done {
            return;
        }
        let Some(value) = std::env::var_os("AURORA_DEMO_PHOTOTOOLS") else {
            return;
        };
        self.phototools_demo_done = true;
        let (tab, photo) = ui::phototools::Tab::parse(&value.to_string_lossy()).unwrap_or((0, false));
        self.phototools_ui.tab = tab;
        if photo {
            self.panels.snapshot = true;
        }
        ui::widgets::open_floater(&self.egui_ctx, ui::phototools::WINDOW, &mut self.panels.phototools);
    }
}
