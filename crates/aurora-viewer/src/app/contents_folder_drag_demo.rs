//! `AURORA_DEMO_ACTIONS=open-folder[-drop]`: after `open` (the Open window
//! of the demo cube 976, yours, holding a notecard, a script and the no-copy
//! "Spare cube"), the inventory opens at frame 540 and a real pointer drag
//! takes the window's root "Contents" row from frame 620 onto the
//! inventory's Objects folder, held there from frame 640. `-drop` releases
//! at frame 720: the notecard and the script are copied into the folder at
//! once, and the move warning lists the spare cube.

use super::*;

/// The demo inventory's Objects folder (or its first folder of your own).
fn destination(inv: &crate::world::inventory::Inventory) -> Option<uuid::Uuid> {
    let top = |f: &&crate::world::inventory::Folder| !f.library && f.info.parent == inv.root;
    inv.folders
        .values()
        .filter(top)
        .find(|f| f.info.type_default == 6)
        .or_else(|| inv.folders.values().filter(top).min_by_key(|f| f.info.name.to_lowercase()))
        .map(|f| f.info.id)
}

impl App {
    pub(super) fn contents_folder_drag_demo_input(&mut self, raw: &mut egui::RawInput) {
        if !ui::inventory::task_drop::demo_enabled() {
            return;
        }
        let ctx = self.egui_ctx.clone();
        if self.frame_count == 540 {
            ui::widgets::open_floater(&ctx, "inventory", &mut self.panels.inventory);
            return;
        }
        let drop = std::env::var("AURORA_DEMO_ACTIONS").is_ok_and(|mode| mode.ends_with("-drop"));
        let Some(root) = ui::object_actions::contents_demo_root(&ctx) else {
            return;
        };
        let to = destination(&self.world.inventory).and_then(|folder| ui::inventory::task_drop::demo_target(&ctx, folder));
        let start = root.left_center() + egui::vec2(30.0, 0.0);
        let (pos, pressed) = match self.frame_count {
            620 => (start, Some(true)),
            622 => (start + egui::vec2(16.0, 4.0), None),
            640 => {
                let Some(to) = to else {
                    log::warn!("demo contents folder drag: no inventory folder in view");
                    return;
                };
                log::info!("demo contents folder drag: over the inventory folder");
                (to.left_center() + egui::vec2(40.0, 0.0), None)
            }
            720 if drop => {
                let Some(to) = to else { return };
                log::info!("demo contents folder drag: released on the inventory folder");
                (to.left_center() + egui::vec2(40.0, 0.0), Some(false))
            }
            _ => return,
        };
        raw.focused = true;
        raw.events.push(egui::Event::PointerMoved(pos));
        if let Some(pressed) = pressed {
            raw.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
}
