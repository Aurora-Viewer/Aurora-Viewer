//! The Favorite Wearables window (`ui/appearance/wearable_favorites.rs`):
//! its folder fetched and created while it is open, its changes sent as
//! inventory and outfit changes, its sort order saved.
use super::*;

impl App {
    pub(super) fn wearable_favorites_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.panels.wearable_favorites {
            // an inventory change already on its way would refuse ours
            let writable = (self.demo || self.world.inventory_writable()) && self.inventory_ui.pending.is_none();
            if let Some(change) = ui::appearance::wearable_favorites::prepare(&mut self.world.inventory, &mut self.appearance_ui, writable)
            {
                self.apply_inventory_action(ui::inventory::InvAction::Edit(change));
            }
        }
        let mut open = self.panels.wearable_favorites;
        let mut sort = self.settings.wearable_favorites_sort;
        let writable = (self.demo || self.world.inventory_writable()) && self.inventory_ui.pending.is_none();
        let actions =
            ui::appearance::wearable_favorites::show(ctx, p, &self.world, &mut self.appearance_ui, &mut open, &mut sort, writable);
        self.panels.wearable_favorites = open;
        // the import is marked sent only once the inventory took it: a change
        // already pending discards it, and the button must stay
        self.appearance_ui
            .wearable_favorites
            .import_settled(self.inventory_ui.pending.is_some());
        if let Some(change) = ui::appearance::wearable_favorites::take_import(&mut self.appearance_ui) {
            self.apply_inventory_action(ui::inventory::InvAction::Edit(change));
            if self.inventory_ui.pending.is_some() {
                self.appearance_ui.wearable_favorites.import_accepted();
            }
        }
        if sort != self.settings.wearable_favorites_sort {
            self.settings.wearable_favorites_sort = sort;
            self.settings.save();
        }
        for action in actions {
            self.apply_appearance_action(action);
        }
    }

    /// `AURORA_DEMO_WEARABLE_FAVORITES=1|evening|recent|create|import|import-done`: the window
    /// open on the favorites of the Appearance demo (`evening`: its Evening
    /// folder; `recent`: sorted by most recent; `create`: no favorites yet,
    /// the window creates the folder; `import`: the Legacy
    /// Viewer's favorites beside, the window offers to import them; `import-done`: after the import).
    pub(super) fn demo_wearable_favorites(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_WEARABLE_FAVORITES") else {
            return;
        };
        if std::env::var_os("AURORA_DEMO_APPEARANCE").is_none() {
            crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
            for cmd in
                crate::world::appearance::sync_commands(&self.world.inventory, self.world.agent_id, &self.world.outfit_attachment_items())
            {
                self.send(cmd);
            }
        }
        if mode != "create" {
            crate::world::wearable_favorites::seed_demo(&mut self.world.inventory, self.world.agent_id);
        }
        if mode == "import" || mode == "import-done" {
            crate::world::wearable_favorites::seed_demo_legacy(&mut self.world.inventory, self.world.agent_id);
        }
        // the click already made, to capture the window after it
        if mode == "import-done"
            && let Some(root) = crate::world::wearable_favorites::folder(&self.world.inventory)
            && let Some(plan) = crate::world::wearable_favorites::import_plan(&self.world.inventory, root)
        {
            self.apply_inventory_action(ui::inventory::InvAction::Edit(plan.change));
        }
        if mode == "evening" {
            ui::appearance::wearable_favorites::demo_pick(&self.world.inventory, &mut self.appearance_ui, "Evening");
        }
        if mode == "recent" {
            self.settings.wearable_favorites_sort = crate::world::wearable_favorites::SortOrder::MostRecent;
        }
        self.panels.wearable_favorites = true;
        self.panels.perf = false;
    }
}
