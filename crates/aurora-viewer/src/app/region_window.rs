//! What the Region / Estate window reads of the app before it is drawn:
//! the objects selected in the world for the telehub (LLFloaterTelehub's
//! edit selection: its root prims) and the state of the terrain's ids as
//! materials (LLTerrainMaterials::makeMaterialsReady, originally LGPL 2.1).

use super::App;

/// LL_PCODE_VOLUME.
const PCODE_VOLUME: u8 = 9;

impl App {
    pub(super) fn feed_region_window(&mut self, ctx: &egui::Context) {
        let region = self.world.main_region;
        // refresh: only root prims of the window's region (all_volume)
        let roots = self.build.roots(&self.world);
        let objects: Vec<_> = roots.iter().filter_map(|i| self.world.objects.get(*i)).collect();
        let all_volume = objects.iter().all(|o| o.pcode == PCODE_VOLUME);
        self.region_ui.selection = if all_volume {
            objects
                .iter()
                .filter(|o| Some(o.key.region) == region)
                .map(|o| o.key.local_id)
                .collect()
        } else {
            Vec::new()
        };
        self.region_ui.prepare_materials(ctx, &mut self.scene);
    }
}
