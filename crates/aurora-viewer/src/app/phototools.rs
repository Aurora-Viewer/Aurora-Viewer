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
        // view); nothing worked out while Phototools is closed
        let photo = (open && self.panels.snapshot).then(|| ui::options::PhotoState {
            sun: super::snapshot::env_view(&self.env_ui, &self.world.eep).sun,
            loading: self.world.eep.photo_loading(),
            viewfinder: self.viewfinder_on(),
            busy: self.photo_busy(),
            boost: self.settings.photo_held.is_some(),
        });
        let director_dof = open && self.director_dof().is_some();
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
            self.snapshots.fov_before,
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

    /// Phototools over the full-frame photo view (the viewfinder, or the
    /// photo window's full frame), whose branch of the frame draws no other
    /// window: what the normal frame gives it (the environment lists, the
    /// room to move in, "Personal lighting", the environment actions).
    pub(super) fn phototools_over_photo(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.panels.phototools {
            if !self.env_scan {
                self.env_scan = true;
                self.env_ui.scan_left = usize::MAX;
            }
            self.env_ui.refresh(ctx, &self.world.inventory);
            let screen = ctx.content_rect();
            ui::layout::set_work_area(ctx, screen);
            ui::layout::set_move_area(ctx, screen);
        }
        self.env_ui.rlva_locked = self.world.rlva.has("setenv") || self.world.rlva.has("setsphere");
        let mut lighting = self.panels.personal_lighting;
        let actions = self.phototools_window(ctx, p, &mut lighting);
        self.panels.personal_lighting = lighting;
        self.env_actions(actions);
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
