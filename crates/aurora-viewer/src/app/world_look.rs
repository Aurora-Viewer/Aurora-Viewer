//! The world's own filters (`snapshot::looks::WorldLook`): their window
//! (`ui/world_look.rs`) driven, and their look on the 3D view outside the
//! photo mode and the Director. Aurora's own.

use super::*;
use crate::snapshot::looks::{self, Placement};

impl App {
    /// The World filters window, while it is open.
    pub(super) fn world_look_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.panels.world_look {
            return;
        }
        let mut open = true;
        let before = self.settings.world_look.clone();
        let stars = self.settings.snapshot.look_favorites.len();
        let photo_shown = self.panels.snapshot || self.panels.director;
        let mut actions = Vec::new();
        ui::world_look::show(
            ctx,
            p,
            &mut open,
            &mut self.snapshots.world_ui,
            &mut self.settings.world_look,
            &mut self.settings.snapshot.look_favorites,
            &self.snapshots.ui.lut_names,
            &self.snapshots.ui.thumbs,
            photo_shown,
            &mut actions,
        );
        self.panels.world_look = open;
        for action in actions {
            self.photo_lut_action(action, false);
        }
        if self.settings.world_look != before || self.settings.snapshot.look_favorites.len() != stars {
            self.settings.save();
        }
    }

    /// The world's filters on the whole 3D view of the `window` (pixels),
    /// with the look pointed at in their window tried
    /// (`looks::world_stack`); None while the photo window or the Director
    /// shows the photos' looks, or with no filter. `focus`: the depth of
    /// field's, for the depth looks (None: the scene at the middle).
    pub(super) fn world_look(&mut self, window: (u32, u32), focus: Option<f32>) -> Option<aurora_render::looks::PhotoLook> {
        let ui = &self.snapshots.world_ui.looks;
        let pointed = if self.panels.world_look {
            ui.hover.as_ref().map(|h| h.as_deref())
        } else {
            None
        };
        let photo_shown = self.panels.snapshot || self.panels.director;
        let replace = ui.replace_target(self.settings.world_look.looks.len());
        let stack = looks::world_stack(&self.settings.world_look, pointed, replace, photo_shown);
        if stack.is_empty() {
            return None;
        }
        let settings = self.settings.world_look.look_settings.clone();
        let layers = looks::resolve_stack(&stack, &settings, self.photo_luts());
        let place = Placement {
            origin: [0.0, 0.0],
            photo: window,
            scale: 1.0,
        };
        looks::frame_look(&layers, place, None, false, focus)
    }

    /// AURORA_DEMO_WORLD_LOOK=<look>[+<look>…][:window]: at frame 600, the
    /// world's filters switched on with those looks stacked in that order
    /// (the ids of AURORA_DEMO_PHOTO_LOOK: `Sepia`, `aurora:vhs`,
    /// `lut:<file>`, `pack:<pack>/<table>`), the last one chosen; `window`
    /// opens their window. Not saved.
    pub(super) fn world_look_demo(&mut self) {
        if !self.demo || self.frame_count != 600 {
            return;
        }
        let Ok(value) = std::env::var("AURORA_DEMO_WORLD_LOOK") else {
            return;
        };
        let (ids, window) = match value.strip_suffix(":window") {
            Some(ids) => (ids, true),
            None => (value.as_str(), false),
        };
        let w = &mut self.settings.world_look;
        w.looks = ids
            .split('+')
            .map(str::trim)
            .filter(|id| looks::name(id).is_some())
            .map(looks::Layer::new)
            .collect();
        w.on = !w.looks.is_empty();
        let ui = &mut self.snapshots.world_ui.looks;
        ui.selected = w.looks.len().saturating_sub(1);
        if let Some(layer) = w.looks.last() {
            ui.tab = looks::tab_of(&layer.id);
        }
        self.panels.world_look |= window;
        let names: Vec<String> = w.looks.iter().map(|l| l.id.clone()).collect();
        log::info!("world look demo: {} (window {window})", names.join(" + "));
    }
}
