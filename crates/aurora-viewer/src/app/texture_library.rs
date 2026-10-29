//! Library lifecycle and world drops. Behaviour follows Firestorm
//! LLToolDragAndDrop::{dad3dApplyToObject,handleDropMaterialProtections,
//! dropTextureOneFace,dropMaterialOneFace} (LGPL 2.1): Shift all faces,
//! Ctrl contents, restricted assets retained in the object's inventory.

use super::*;
use crate::ui::texture_library::{self as library, catalogue};
use crate::world::inventory::actions as rules;
use aurora_net::build::BuildCmd;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct DragState {
    candidates: Vec<usize>,
    ray: Option<(Vec3, Vec3)>,
    checked: Option<Instant>,
    target: Option<(usize, u8)>,
}
pub(super) struct Drop {
    item: Uuid,
    object: usize,
    face: u8,
    all: bool,
    contents: bool,
    remove_pbr: bool,
}

fn permissions(item: &aurora_net::inventory::InvItem, agent: Uuid) -> u32 {
    if item.owner == agent && !item.group_owned {
        item.owner_mask
    } else {
        item.everyone_mask
    }
}
fn restricted(item: &aurora_net::inventory::InvItem, agent: Uuid) -> bool {
    let mask = permissions(item, agent);
    mask & rules::COPY == 0 || mask & rules::TRANSFER == 0 || item.asset_type == 57 && mask & rules::MODIFY == 0
}
fn requires_contents(inv: &crate::world::inventory::Inventory, item: &aurora_net::inventory::InvItem, agent: Uuid) -> bool {
    !rules::library(inv, item.id) && restricted(item, agent)
}
// The selection is counted by prim, not by face: a no-copy resource may
// cover several faces of one object but cannot be moved into two objects.
fn selection_copy_denied(item: &aurora_net::inventory::InvItem, agent: Uuid, objects: usize) -> bool {
    objects > 1 && permissions(item, agent) & rules::COPY == 0
}

impl App {
    pub(super) fn record_library_use(&mut self, id: Uuid) {
        if !self.demo && ui::texture_library::catalogue::contains(&self.world.inventory, id) {
            self.settings.texture_library.activate(self.world.agent_id);
            self.settings.texture_library.record(id);
            self.settings.save();
        }
    }
    pub(super) fn record_library_asset(&mut self, asset: Uuid, kind: i32) {
        let inv = &self.world.inventory;
        let matches = |item: &&aurora_net::inventory::InvItem| {
            item.asset_type == kind && item.asset_id == asset && ui::texture_library::catalogue::contains(inv, item.id)
        };
        let item = self
            .inventory_ui
            .preview
            .and_then(|id| inv.items.get(&id))
            .filter(matches)
            .map(|it| it.id)
            .or_else(|| inv.items.values().filter(matches).map(|it| it.id).min());
        if let Some(id) = item {
            self.record_library_use(id);
        }
    }

    pub(super) fn texture_library_demo_input(&self, raw: &mut egui::RawInput) {
        let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY") else {
            return;
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        if mode.starts_with("types") {
            if let Some(point) = self
                .egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("library_type_selector")))
            {
                match self.frame_count {
                    650 => raw.events.extend([egui::Event::PointerMoved(point), button(point, true)]),
                    651 => raw.events.push(button(point, false)),
                    _ => {}
                }
            }
            return;
        }
        if mode == "preview-sizes" {
            let (item, start, delta) = if self.frame_count < 690 {
                (crate::demo::texture_library::preview_items()[0], 640, egui::vec2(-80.0, -130.0))
            } else {
                (crate::demo::texture_library::preview_items()[2], 710, egui::vec2(-20.0, -100.0))
            };
            if let Some(rect) = self
                .egui_ctx
                .memory(|m| m.area_rect(egui::Id::unique(library::preview_window_id(item))))
            {
                let source = rect.right_bottom() - egui::Vec2::splat(2.0);
                let origin_key = egui::Id::unique(("library_demo_resize_origin", item));
                if self.frame_count == start {
                    self.egui_ctx.data_mut(|d| d.insert_temp(origin_key, source));
                    raw.events.push(egui::Event::PointerMoved(source));
                }
                if let Some(origin) = self.egui_ctx.data(|d| d.get_temp::<egui::Pos2>(origin_key)) {
                    let target = origin + delta;
                    match self.frame_count.saturating_sub(start) {
                        1 if self.frame_count >= start => raw.events.push(button(origin, true)),
                        2..=5 if self.frame_count >= start => raw.events.push(egui::Event::PointerMoved(target)),
                        6 if self.frame_count >= start => raw.events.push(button(target, false)),
                        _ => {}
                    }
                }
            }
            return;
        }
        if mode == "preview-controls-drop" {
            // The texture inspector's image dragged onto the material's empty metallic-roughness map (#1279).
            let [texture, _, material, _] = crate::demo::texture_library::preview_items();
            let source = self
                .egui_ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_texture_preview", texture))));
            let target = self.egui_ctx.data(|d| {
                d.get_temp::<(Option<Uuid>, Option<egui::TextureId>, egui::Rect)>(egui::Id::unique((
                    "resource_material_map",
                    material,
                    2usize,
                )))
            });
            if let (Some(source), Some((_, _, target))) = (source, target) {
                let (source, target) = (source.center(), target.center());
                match self.frame_count {
                    760 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    761 => raw.events.push(egui::Event::PointerMoved(source + egui::vec2(12.0, 0.0))),
                    762..=764 => raw.events.push(egui::Event::PointerMoved(target)),
                    765 => raw.events.push(button(target, false)),
                    _ => {}
                }
            }
            return;
        }
        if mode.starts_with("material-preview") {
            let item = self
                .egui_ctx
                .data(|d| d.get_temp::<Uuid>(egui::Id::unique("texture_library_first_item")))
                .unwrap_or_default();
            if let Some(source) = self
                .egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("texture_library_first_card")))
            {
                match self.frame_count {
                    650 | 662 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    651 | 663 => raw.events.push(button(source, false)),
                    _ => {}
                }
            }
            if mode != "material-preview"
                && let Some(rect) = self
                    .egui_ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_scene", item))))
            {
                let source = rect.center();
                let rotated = source + egui::vec2(45.0, 18.0);
                match self.frame_count {
                    700 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    701..=703 => raw.events.push(egui::Event::PointerMoved(rotated)),
                    704 => raw.events.push(button(rotated, false)),
                    _ => {}
                }
            }
            if mode == "material-preview-drag"
                && let Some(rect) = self
                    .egui_ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_handle", item))))
            {
                let source = rect.center();
                let target = egui::pos2(480.0, 430.0);
                match self.frame_count {
                    760 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    761 => raw.events.push(egui::Event::PointerMoved(source + egui::vec2(12.0, 0.0))),
                    762..=764 => raw.events.push(egui::Event::PointerMoved(target)),
                    765 => raw.events.push(button(target, false)),
                    _ => {}
                }
            }
            return;
        }
        let inventory = matches!(mode.as_str(), "preview" | "preview-drag" | "inventory-drag");
        if !inventory
            && !matches!(
                mode.as_str(),
                "drop" | "apply" | "apply-pbr" | "objects-drop" | "objects-nocopy-drop"
            )
        {
            return;
        }
        let item = if mode.starts_with("objects") {
            crate::demo::texture_library::first_object()
        } else {
            crate::demo::texture_library::first_texture()
        };
        let source = if inventory {
            self.egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", self.inventory_ui.ui_id, item))))
        } else {
            self.egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("texture_library_first_card")))
        };
        if let Some(source) = source {
            if matches!(mode.as_str(), "apply" | "apply-pbr") {
                match self.frame_count {
                    650 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    651 => raw.events.push(button(source, false)),
                    _ => {}
                }
                return;
            }
            if matches!(mode.as_str(), "preview" | "preview-drag") {
                match self.frame_count {
                    650 | 662 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    651 | 663 => raw.events.push(button(source, false)),
                    _ => {}
                }
            }
        }
        let source = if mode == "preview-drag" {
            self.egui_ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_texture_preview", item))))
                .map(|r| r.center())
        } else {
            source
        };
        let Some(source) = source else { return };
        let start = if inventory { 720 } else { 650 };
        let target = if mode.starts_with("objects") {
            let point = self.world.agent.position + Vec3::new(5.0, 2.0, 0.0);
            let Some(target) = self
                .world
                .ground_height(point)
                .and_then(|z| self.build.cam.project(Vec3::new(point.x, point.y, z)))
            else {
                return;
            };
            target
        } else {
            egui::pos2(480.0, 430.0)
        };
        if mode != "preview" {
            match self.frame_count.saturating_sub(start) {
                0 if self.frame_count >= start => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                1 => raw.events.push(egui::Event::PointerMoved(source + egui::vec2(12.0, 0.0))),
                2..=4 => raw.events.push(egui::Event::PointerMoved(target)),
                5 => raw.events.push(button(target, false)),
                _ => {}
            }
        }
    }

    pub(super) fn show_texture_library(&mut self, ctx: &egui::Context, p: &Palette) {
        if self.demo
            && !self.library_demo_seeded
            && self.frame_count >= 600
            && !self.world.inventory.root.is_nil()
            && let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
        {
            crate::demo::texture_library::seed(&mut self.world.inventory, &mut self.scene, self.world.agent_id, mode == "empty");
            let objects: Vec<_> = self
                .world
                .objects
                .iter()
                .filter(|(_, o)| !o.is_avatar() && !o.is_tree())
                .map(|(idx, _)| idx)
                .collect();
            for idx in objects {
                if let Some(object) = self.world.objects.get_mut(idx) {
                    object.owner_id = self.world.agent_id;
                    object.update_flags |= crate::ui::context::flags::OBJECT_MODIFY | crate::ui::context::flags::OBJECT_YOU_OWNER;
                }
            }
            self.library_demo_seeded = true;
            self.settings.texture_library.presentation =
                if matches!(mode.as_str(), "horizontal" | "objects-horizontal" | "types-horizontal") {
                    library::Presentation::Horizontal
                } else {
                    library::Presentation::Vertical
                };
            self.panels.texture_library =
                !matches!(mode.as_str(), "baseline" | "preview" | "preview-drag" | "inventory-drag") && !mode.starts_with("cascade");
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.perf = false;
            self.panels.minimap = false;
            self.panels.ao = false;
            if matches!(mode.as_str(), "objects-drop" | "objects-nocopy-drop") && std::env::var_os("AURORA_DEMO_CAM").is_none() {
                self.camera.demo_orbit(0.0, 0.6, Some(9.0), &self.settings.camera);
            }
            let inventory_drag = crate::demo::texture_library::inventory_drag_enabled();
            self.panels.inventory = matches!(mode.as_str(), "inventory" | "objects-inventory") || inventory_drag;
            if mode == "objects-inventory" {
                self.inventory_ui
                    .show_original(&self.world.inventory, crate::demo::texture_library::first_object());
            }
            if inventory_drag {
                let item = crate::demo::texture_library::first_texture();
                self.inventory_ui.show_original(&self.world.inventory, item);
                ui::widgets::place_floater(
                    ctx,
                    &library::preview_window_id(item),
                    egui::pos2(20.0, 60.0),
                    egui::vec2(360.0, 280.0),
                );
            } else if mode == "inventory"
                && let Some(root) = catalogue::child(&self.world.inventory, self.world.inventory.root, catalogue::AURORA)
            {
                self.inventory_ui.show_folder(&self.world.inventory, root);
            }
            self.settings.texture_library.texture_preview_size = None;
            self.settings.texture_library.material_preview_size = None;
            self.texture_library.demo(&mode, self.world.agent_id);
            if ui::inventory::drag::demo::enabled() {
                self.panels.texture_library = false;
                self.panels.inventory = false;
                ui::inventory::drag::demo::setup(ctx, &mut self.world.inventory, &mut self.inventory_ui, &mut self.settings.inventory);
            }
            if mode.starts_with("cascade") {
                let kind = match mode.as_str() {
                    "cascade-notecard" => 7,
                    "cascade-script" => 10,
                    _ => 0,
                };
                crate::demo::texture_library::seed_cascade(&mut self.world.inventory, self.world.agent_id, kind);
            }
        }
        if self.demo
            && self.library_demo_seeded
            && let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
            && mode.starts_with("cascade")
            && (620..=720).contains(&self.frame_count)
            && (self.frame_count - 620).is_multiple_of(20)
        {
            let kind = match mode.as_str() {
                "cascade-material" => 57,
                "cascade-notecard" => 7,
                "cascade-script" => 10,
                _ => 0,
            };
            let items = crate::demo::texture_library::cascade_items(kind);
            self.apply_inventory_action(crate::ui::inventory::InvAction::Preview(
                items[((self.frame_count - 620) / 20) as usize],
            ));
        }
        if self.demo
            && self.library_demo_seeded
            && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").is_ok_and(|mode| mode.starts_with("multi-preview"))
        {
            let items = crate::demo::texture_library::preview_items();
            for (index, item) in items.into_iter().enumerate() {
                // Reapply after egui has measured the new windows: its initial
                // size estimate otherwise clamps the second row too high.
                if self.frame_count == 620 + index as u64 * 20 || self.frame_count == 700 {
                    ui::widgets::place_floater(
                        ctx,
                        &library::preview_window_id(item),
                        egui::pos2(20.0 + (index % 2) as f32 * 340.0, 60.0 + (index / 2) as f32 * 345.0),
                        egui::vec2(300.0, 290.0),
                    );
                    self.texture_library.open_preview(ctx, &self.world, item);
                }
            }
            if self.frame_count == 720 {
                self.texture_library.open_preview(ctx, &self.world, items[0]);
            }
            if self.frame_count >= 760 && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").as_deref() == Ok("multi-preview-closed") {
                self.panels.texture_library = false;
            }
        }
        if self.demo && self.library_demo_seeded && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").as_deref() == Ok("preview-sizes") {
            let items = crate::demo::texture_library::preview_items();
            for (item, frame) in items.into_iter().zip([620, 670, 690, 740]) {
                if self.frame_count == frame {
                    self.texture_library.open_preview(ctx, &self.world, item);
                }
            }
            if self.frame_count == 780 {
                for (index, item) in items.into_iter().enumerate() {
                    ui::widgets::move_floater(
                        ctx,
                        &library::preview_window_id(item),
                        egui::pos2(20.0 + (index % 2) as f32 * 380.0, 60.0 + (index / 2) as f32 * 345.0),
                    );
                }
                log::info!(
                    "demo preview window sizes: texture={:?}, material={:?}",
                    self.settings.texture_library.texture_preview_size,
                    self.settings.texture_library.material_preview_size
                );
            }
        }
        if self.demo
            && self.library_demo_seeded
            && let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
            && mode.starts_with("preview-controls")
            && matches!(self.frame_count, 620 | 700)
        {
            self.panels.texture_library = false;
            self.settings.texture_library.texture_display = Default::default();
            let items = crate::demo::texture_library::preview_items();
            for (index, item) in [items[0], items[2]].into_iter().enumerate() {
                self.texture_library.open_preview(ctx, &self.world, item);
                ui::widgets::place_floater(
                    ctx,
                    &library::preview_window_id(item),
                    egui::pos2(24.0 + index as f32 * 660.0, 60.0),
                    egui::vec2(620.0, 720.0),
                );
            }
            if self.frame_count == 700 {
                self.texture_library.demo_preview_controls(&mode);
            }
        }
        let preferences = self.settings.texture_library.clone();
        let actions = self.texture_library.show(
            ctx,
            p,
            &mut self.world,
            &mut self.scene,
            &self.ui_images,
            &mut self.settings.texture_library,
            &mut self.panels.texture_library,
        );
        self.panels.texture_library_presentation = self.settings.texture_library.presentation;
        for action in actions {
            match action {
                library::Action::Change(mut change) => {
                    if self.texture_library.pending.is_some() {
                        continue;
                    }
                    change.defer_folder_contents = false;
                    let request = Uuid::new_v4();
                    self.texture_library.pending = Some(request);
                    self.texture_library.pending_refresh.clone_from(&change.refresh);
                    self.send(NetCommand::EditInventory { request, change });
                }
                library::Action::Apply(id) => self.apply_texture_library_selection(id),
                library::Action::Open(id) => {
                    if let Some(item) = self.world.inventory.items.get(&id) {
                        let action = if item.asset_type == 3 {
                            ui::inventory::InvAction::AboutLandmark(id, item.asset_id)
                        } else {
                            ui::inventory::InvAction::Preview(id)
                        };
                        self.apply_inventory_action(action);
                    }
                }
                library::Action::Reveal(id) => {
                    self.inventory_ui.show_folder(&self.world.inventory, id);
                    ui::widgets::open_floater(ctx, "inventory", &mut self.panels.inventory);
                }
                library::Action::SaveMaterial { item, material, name } => self.resource_material_save(item, material, name),
                library::Action::RefreshTexture(asset) => {
                    self.ui_images.remove(&asset);
                    self.scene.textures.refresh(&asset);
                    self.texture_library.refresh_texture(&self.world, asset);
                }
                library::Action::ExportResult(message) => self.world.system_message(message),
                library::Action::ExportTexture { item, png } => {
                    let full = rules::COPY | rules::MODIFY | rules::TRANSFER;
                    if self.world.inventory.items.get(&item).is_some_and(|it| it.owner_mask & full == full) {
                        self.texture_library.export_texture(item, png);
                    }
                }
                library::Action::Profile(resident) => self.apply_inventory_action(ui::inventory::InvAction::Profile(resident)),
                library::Action::MaterialMapFile { item, slot } => {
                    if !self.world.inventory.items.get(&item).is_some_and(|it| {
                        it.owner == self.world.agent_id
                            && !it.group_owned
                            && it.owner_mask & rules::MODIFY != 0
                            && !rules::library(&self.world.inventory, item)
                    }) {
                        continue;
                    }
                    if let Some(entry) = self
                        .imports
                        .choose_material_map_upload(&self.scene.jobs, ctx, self.settings.imports.texture())
                    {
                        self.texture_library.track_map_upload(entry, item, slot);
                    }
                }
                library::Action::Discard(item) => {
                    if let Some(trash) = rules::system(&self.world.inventory, 14)
                        && let Ok(change) = rules::move_selection(
                            &self.world.inventory,
                            &[item],
                            trash,
                            &self.settings.inventory.protected,
                            &HashSet::new(),
                        )
                        && self.texture_library.pending.is_none()
                    {
                        let request = Uuid::new_v4();
                        self.texture_library.pending = Some(request);
                        self.texture_library.pending_refresh.clone_from(&change.refresh);
                        self.texture_library.close_texture(item);
                        self.send(NetCommand::EditInventory { request, change });
                    }
                }
            }
        }
        if self.settings.texture_library != preferences {
            self.settings.save();
        }
    }
    fn resource_material_save(&mut self, id: Uuid, material: aurora_assets::PbrMaterial, name: Option<String>) {
        let Some(item) = self.world.inventory.items.get(&id).cloned() else {
            return;
        };
        let own = item.owner == self.world.agent_id && !item.group_owned;
        let editable = own && item.owner_mask & rules::MODIFY != 0 && !rules::library(&self.world.inventory, id);
        let copy = name.is_some();
        if item.asset_type != 57 || if copy { item.owner_mask & rules::COPY == 0 } else { !editable } {
            return;
        }
        if !editable && self.texture_library.material_draft_dirty(id) {
            return;
        }
        let name = match name.map(|name| rules::clean_name(&name)).transpose() {
            Ok(name) => name,
            Err(reason) => {
                self.texture_library.material_message(id, reason);
                return;
            }
        };
        let request = Uuid::new_v4();
        if !self.texture_library.begin_material_save(id, request) {
            return;
        }
        if let Some(name) = name {
            self.send(NetCommand::CopyInventoryItem {
                request,
                copy: aurora_net::inventory::operations::CopyItem {
                    item: id,
                    owner: item.owner,
                    parent: if rules::library(&self.world.inventory, id) || rules::in_type(&self.world.inventory, id, 53) {
                        rules::system(&self.world.inventory, 57).unwrap_or(self.world.inventory.root)
                    } else {
                        item.parent
                    },
                    name,
                },
            });
        } else {
            match material.encode_asset() {
                Ok(data) => self.send(NetCommand::SaveInventoryContent {
                    request,
                    item: id,
                    asset_type: 57,
                    data,
                }),
                Err(reason) => self.resource_material_saved(request, Err(reason.to_string())),
            }
        }
    }
    pub(super) fn resource_material_copied(&mut self, request: Uuid, result: Result<aurora_net::inventory::InvItem, String>) {
        match result {
            Ok(item) => {
                self.world.inventory.add_items(vec![item.clone()]);
                if let Some(material) = self.texture_library.material_copy_ready(request, item.id) {
                    match material.encode_asset() {
                        Ok(data) => self.send(NetCommand::SaveInventoryContent {
                            request,
                            item: item.id,
                            asset_type: 57,
                            data,
                        }),
                        Err(reason) => self.resource_material_saved(request, Err(reason.to_string())),
                    }
                } else {
                    self.resource_material_saved(request, Ok(item));
                }
            }
            Err(reason) => self.resource_material_saved(request, Err(reason)),
        }
    }
    pub(super) fn resource_material_saved(&mut self, request: Uuid, result: Result<aurora_net::inventory::InvItem, String>) {
        let material = self.texture_library.material_saved(request, &result);
        if let Ok(item) = result {
            if let Some(material) = material {
                self.scene.materials.insert(item.asset_id, material);
            }
            self.world.inventory.add_items(vec![item]);
        }
    }
    pub(super) fn texture_library_result(
        &mut self,
        request: Uuid,
        result: &Result<aurora_net::inventory::operations::Reply, String>,
    ) -> bool {
        if !self.texture_library.mutation_result(request, result) {
            return false;
        }
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents.clone());
                self.world.inventory.created_folders(reply.folders.clone());
            }
            Err(reason) => {
                log::warn!("texture library inventory update failed: {reason}");
                // A compound mutation can partly succeed. Read parents back
                // before retrying so a created folder is never duplicated.
                for id in std::mem::take(&mut self.texture_library.pending_refresh) {
                    if let Some(folder) = self.world.inventory.folders.get_mut(&id) {
                        folder.state = crate::world::inventory::FetchState::Unknown;
                        self.world.inventory.request(id);
                    }
                }
                self.world.inventory.generation += 1;
            }
        }
        self.texture_library.pending_refresh.clear();
        true
    }
    pub(super) fn texture_library_drag_input(&mut self, ctx: &egui::Context, renderer: &Renderer) -> Option<Drop> {
        if !matches!(self.screen, Screen::World) || self.world.teleporting || self.exit_prompt.open {
            return None;
        }
        let drag = ui::inventory::world_drag(ctx)?;
        let item = catalogue::resource(&self.world.inventory, drag.item)?;
        let asset = catalogue::asset(item);
        if asset.is_nil() {
            return None;
        }
        let position = (drag.position.x * ctx.pixels_per_point(), drag.position.y * ctx.pixels_per_point());
        let ray = renderer.cursor_ray(position.0, position.1)?;
        let refresh = drag.released
            || self.library_drag.ray != Some(ray)
            || self
                .library_drag
                .checked
                .is_none_or(|at| at.elapsed() >= Duration::from_millis(100));
        if refresh {
            let hud = self.scene.hud_edit_pick(&self.world, position);
            let surface = if let Some((idx, _, ray)) = hud {
                self.scene.touch_surface(&self.world, idx, ray).map(|s| (idx, s.face as u8))
            } else {
                self.scene
                    .inventory_surface(&self.world, ray, self.settings.draw_distance, &mut self.library_drag.candidates)
                    .and_then(|(_, idx)| idx)
                    .and_then(|idx| self.scene.touch_surface(&self.world, idx, ray).map(|s| (idx, s.face as u8)))
            };
            self.library_drag.target = surface;
            self.library_drag.ray = Some(ray);
            self.library_drag.checked = Some(Instant::now());
        }
        let target = self.library_drag.target;
        let allowed = target
            .and_then(|(idx, face)| self.world.objects.get(idx).map(|o| (idx, face, o)))
            .is_some_and(|(idx, face, o)| {
                !o.is_avatar()
                    && !o.is_tree()
                    && o.te.as_ref().is_some_and(|te| (face as usize) < te.faces.len())
                    && (if drag.ctrl {
                        // A world drop follows willObjectAcceptInventory;
                        // the Contents panel's New script rule is narrower.
                        super::inventory_drag::target_allowed(&self.world, idx, item, true).is_ok()
                    } else {
                        self.build.can_modify(&self.world, idx)
                    })
            });
        let protected_asset = requires_contents(&self.world.inventory, item, self.world.agent_id);
        let mut reason = None;
        if !allowed || drag.ctrl && self.world.inventory.items.get(&drag.item).is_some_and(|i| i.asset_type == 24) {
            reason = Some(aurora_i18n::tr!("texture-library-locked"));
        }
        if allowed
            && let Some((idx, _)) = target
            && (protected_asset || drag.ctrl)
        {
            if protected_asset && !drag.ctrl {
                let object = self.world.objects.get(idx)?.full_id;
                self.build.request_contents(&self.world, idx, false);
                if self
                    .build
                    .contents
                    .objects
                    .get(&object)
                    .is_none_or(|inventory| inventory.loading || inventory.requested.is_none())
                {
                    reason = Some(aurora_i18n::tr!("texture-library-wait-inventory"));
                } else {
                    let retained = self.build.contents.objects.get(&object).is_some_and(|inv| {
                        inv.items
                            .iter()
                            .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
                    });
                    if !retained && super::inventory_drag::target_allowed(&self.world, idx, item, true).is_err() {
                        reason = Some(aurora_i18n::tr!("texture-library-locked"));
                    }
                }
            } else if super::inventory_drag::target_allowed(&self.world, idx, item, true).is_err() {
                reason = Some(aurora_i18n::tr!("texture-library-locked"));
            }
        }
        self.inventory_drag.cursor = Some(if reason.is_none() {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        });
        if reason.is_none()
            && let Some((idx, face)) = target
            && !drag.ctrl
        {
            let mut highlighted = self.build.face_selection(&self.world, &self.settings.build);
            if face < 64 {
                *highlighted.entry(idx).or_default() |= 1u64 << face;
                self.scene.set_face_selection(&mut self.world, highlighted);
            }
        }
        if drag.released {
            egui::DragAndDrop::clear_payload(ctx);
            if let Some(reason) = reason {
                self.world.system_message(reason);
                return None;
            }
            let (object, face) = target?;
            Some(Drop {
                item: drag.item,
                object,
                face,
                all: self.shift,
                contents: drag.ctrl,
                remove_pbr: self.alt,
            })
        } else {
            None
        }
    }
    pub(super) fn apply_texture_library_drop(&mut self, drop: Drop) {
        let Some(item) = catalogue::resource(&self.world.inventory, drop.item).cloned() else {
            return;
        };
        let Some(object) = self.world.objects.get(drop.object) else {
            return;
        };
        let object_id = object.full_id;
        let count = self
            .media
            .face_count(&self.world, &self.scene, drop.object)
            .unwrap_or_else(|| self.build.num_faces(&self.world, drop.object));
        self.build.face_counts.insert(object_id, count);
        let faces: Vec<_> = if drop.all {
            (0..count.min(256)).map(|i| i as u8).collect()
        } else {
            vec![drop.face]
        };
        if self.apply_inventory_resource(&item, drop.object, &faces, drop.contents, drop.remove_pbr) && !drop.contents {
            self.texture_resource_applied(item.id);
        }
        self.flush_build();
    }

    fn apply_texture_library_selection(&mut self, id: Uuid) {
        if !self.build.open || self.build.tool != crate::build::Tool::Edit {
            return;
        }
        let Some(item) = catalogue::resource(&self.world.inventory, id).cloned() else {
            return;
        };
        // The expanded texture entry is not a geometry face count (notably
        // on meshes). Refresh bounds before collecting the selected faces.
        for idx in self.build.sel_prims(&self.world) {
            if let Some(object) = self.world.objects.get(idx) {
                let count = self
                    .media
                    .face_count(&self.world, &self.scene, idx)
                    .unwrap_or_else(|| self.build.num_faces(&self.world, idx));
                self.build.face_counts.insert(object.full_id, count);
            }
        }
        let targets = self.build.inventory_surface_selection(&self.world);
        if targets.is_empty() {
            return;
        }
        // LLSelectMgr::selectionSetImage refuses duplicating a no-copy item.
        if selection_copy_denied(&item, self.world.agent_id, targets.len()) {
            self.world.system_notice(
                aurora_i18n::tr!("texture-library-no-copy-multiple"),
                crate::world::notice_topics::Topic::Items,
            );
            return;
        }
        let mut waiting = false;
        if requires_contents(&self.world.inventory, &item, self.world.agent_id) {
            for (idx, _) in &targets {
                let Some(object) = self.world.objects.get(*idx) else { continue };
                let object_id = object.full_id;
                self.build.request_contents(&self.world, *idx, false);
                let inventory = self.build.contents.objects.get(&object_id);
                if inventory.is_none_or(|inv| inv.loading || inv.requested.is_none()) {
                    waiting = true;
                } else if !inventory.is_some_and(|inv| {
                    inv.items
                        .iter()
                        .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
                }) && super::inventory_drag::target_allowed(&self.world, *idx, &item, true).is_err()
                {
                    self.world.system_notice(
                        aurora_i18n::tr!("texture-library-locked"),
                        crate::world::notice_topics::Topic::Items,
                    );
                    return;
                }
            }
        }
        if waiting {
            self.world.system_notice(
                aurora_i18n::tr!("texture-library-wait-selection"),
                crate::world::notice_topics::Topic::Items,
            );
            return;
        }
        let mut applied = false;
        for (idx, faces) in targets {
            applied |= self.apply_inventory_resource(&item, idx, &faces, false, false);
        }
        if applied {
            self.texture_resource_applied(item.id);
        }
        self.flush_build();
    }

    /// The same inventory retention and surface edit path is used by preview
    /// drags, inventory drags and clicks on the library's current selection.
    fn apply_inventory_resource(
        &mut self,
        item: &aurora_net::inventory::InvItem,
        idx: usize,
        faces: &[u8],
        contents: bool,
        remove_pbr: bool,
    ) -> bool {
        let Some(object) = self.world.objects.get(idx) else { return false };
        let object_id = object.full_id;
        let asset = catalogue::asset(item);
        if asset.is_nil() || !contents && !self.build.can_modify(&self.world, idx) {
            return false;
        }
        let retained = self.build.contents.objects.get(&object_id).is_some_and(|inv| {
            inv.items
                .iter()
                .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
        });
        if contents || requires_contents(&self.world.inventory, item, self.world.agent_id) && !retained {
            if super::inventory_drag::target_allowed(&self.world, idx, item, true).is_err() {
                return false;
            }
            let task = crate::build::task_inventory::from_inventory(object_id, item);
            self.build.send(BuildCmd::UpdateTaskInventory {
                handle: object.key.region,
                local_id: object.key.local_id,
                item: Box::new(task),
            });
            if permissions(item, self.world.agent_id) & rules::COPY == 0 {
                self.world.inventory.consume_item(item.id);
            }
            self.build.request_contents(&self.world, idx, true);
        }
        if !contents {
            let mut env = crate::build::ui::Env {
                images: &self.ui_images,
                media: &mut self.media,
                scene: &mut self.scene,
                zoom_fraction: self.camera.zoom_fraction(),
                property_lines: &mut self.settings.world_overlays.property_lines,
                demo: self.demo,
            };
            self.build
                .apply_inventory_surface(&mut self.world, &mut env, idx, faces, asset, item.asset_type == 57, remove_pbr);
        }
        true
    }

    fn texture_resource_applied(&mut self, id: Uuid) {
        self.settings.texture_library.activate(self.world.agent_id);
        self.settings.texture_library.record(id);
        self.settings.save();
        self.world.system_notice(
            aurora_i18n::tr!("texture-library-applied"),
            crate::world::notice_topics::Topic::Items,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;
    #[test]
    fn texture_library_never_duplicates_a_no_copy_resource_across_selected_objects() {
        let agent = Uuid::from_u128(90);
        for kind in [0, 57] {
            let mut item = InvItem {
                owner: agent,
                asset_type: kind,
                owner_mask: rules::TRANSFER,
                ..Default::default()
            };
            assert!(!selection_copy_denied(&item, agent, 1));
            assert!(selection_copy_denied(&item, agent, 2));
            item.owner_mask |= rules::COPY;
            assert!(!selection_copy_denied(&item, agent, 2));
            item.owner = Uuid::from_u128(91);
            assert!(selection_copy_denied(&item, agent, 2));
        }
    }
    #[test]
    fn texture_library_retains_restricted_assets_with_the_effective_permission_mask() {
        let agent = Uuid::from_u128(90);
        let mut item = InvItem {
            owner: agent,
            asset_type: 0,
            owner_mask: rules::COPY | rules::TRANSFER,
            ..Default::default()
        };
        assert!(!restricted(&item, agent), "textures do not require Modify");
        item.asset_type = 57;
        assert!(restricted(&item, agent), "no-modify materials must enter contents");
        item.owner_mask |= rules::MODIFY;
        assert!(!restricted(&item, agent));
        item.owner_mask &= !rules::COPY;
        assert!(restricted(&item, agent));
        item.owner_mask |= rules::COPY;
        item.owner_mask &= !rules::TRANSFER;
        assert!(restricted(&item, agent));
        item.owner_mask = rules::COPY | rules::TRANSFER | rules::MODIFY;
        item.owner = Uuid::from_u128(91);
        assert!(restricted(&item, agent), "foreign assets use everyone permissions");
        item.everyone_mask = rules::COPY | rules::TRANSFER | rules::MODIFY;
        assert!(!restricted(&item, agent));
    }
}
