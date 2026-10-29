//! Native appearance editing for the remaining LLWearableType entries.
use super::*;

impl App {
    pub(super) fn close_wearable_editor(&mut self) -> bool {
        if !self.wearable_ui.open {
            return true;
        }
        if self
            .wearable_ui
            .draft
            .as_ref()
            .is_some_and(|draft| draft.pending.is_some() || draft.dirty())
        {
            self.wearable_ui.close_confirmation = self.wearable_ui.draft.as_ref().is_some_and(|draft| draft.pending.is_none());
            return false;
        }
        self.wearable_ui.close(&mut self.world);
        true
    }

    pub(super) fn open_wearable_editor(&mut self, item: uuid::Uuid) {
        if !self.close_shape_editor_for_physics() || !self.close_physics_editor_for_shape() {
            return;
        }
        if let Some(command) = self.wearable_ui.open_item(&mut self.world, item) {
            if self.shape_camera.is_none() {
                self.shape_camera = Some(self.camera.clone());
            }
            self.panels.appearance = false;
            if self.demo {
                let Some(info) = self.world.inventory.items.get(&item) else {
                    return;
                };
                let asset = info.asset_id;
                let data = self
                    .demo_documents
                    .get(&item)
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| demo_asset(info, self.world.agent_id));
                self.wearable_ui.loaded(&mut self.world, item, asset, &data);
            } else {
                self.send(command);
            }
            self.focus_wearable_category();
        }
    }

    fn focus_wearable_category(&mut self) {
        if !self.world.avatar_poses.contains_key(&self.world.agent_id) {
            self.wearable_ui.camera_pending = true;
            return;
        }
        let turned = self.customize_turns();
        if let Some((eye, target)) =
            crate::shape_editor::camera::view(&self.world, &self.scene, self.wearable_ui.camera_group(), self.camera.fov_y, turned)
        {
            self.camera
                .set_appearance_view(eye, target, &mut self.world.agent, &self.settings.camera);
            self.wearable_ui.camera_pending = false;
        }
    }

    pub(super) fn wearable_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        self.wearable_ui.update(&mut self.world);
        let group = self.wearable_ui.group.clone();
        for action in ui::wearable::show(ctx, p, &mut self.world, &mut self.wearable_ui, &self.ui_images) {
            match action {
                ui::wearable::Action::Save(name) => {
                    if let Some(command) = self.wearable_ui.save(&self.world, name) {
                        if self.demo {
                            self.demo_save_wearable(command);
                        } else {
                            self.send(command);
                        }
                    }
                }
            }
        }
        if self.wearable_ui.open && (self.wearable_ui.camera_pending || self.wearable_ui.group != group) {
            self.focus_wearable_category();
        }
        if self.demo
            && self.frame_count == 650
            && std::env::var("AURORA_DEMO_WEARABLE").is_ok_and(|mode| mode.ends_with(":save"))
            && let Some(command) = self.wearable_ui.save(&self.world, None)
        {
            self.demo_save_wearable(command);
        }
        if self.wearable_ui.open
            && let Some(draft) = &self.wearable_ui.draft
        {
            let commands = self.wearable_ui.preview.update(&self.world, &mut self.scene, draft);
            for command in commands {
                if self.demo {
                    if let NetCommand::PreviewInventoryItem(item) = command {
                        let data = demo_asset(&item, self.world.agent_id);
                        self.wearable_ui.preview.loaded(item.id, item.asset_id, &data);
                    }
                } else {
                    self.send(command);
                }
            }
        }
    }

    pub(super) fn wearable_saved(&mut self, request: uuid::Uuid, result: &Result<aurora_net::inventory::InvItem, String>) -> bool {
        if self.wearable_ui.draft.as_ref().is_none_or(|draft| draft.pending != Some(request)) {
            return false;
        }
        if let Some((old, new)) = self.wearable_ui.saved(&mut self.world, request, result.clone()) {
            self.apply_appearance_action(crate::world::appearance::Action::ReplaceWearable { old, new });
        }
        true
    }

    pub(super) fn demo_wearable_editor(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_WEARABLE") else { return };
        let (name, variant) = mode.split_once(':').unwrap_or((&mode, ""));
        let names = [
            "shape",
            "skin",
            "hair",
            "eyes",
            "shirt",
            "pants",
            "shoes",
            "socks",
            "jacket",
            "gloves",
            "undershirt",
            "underpants",
            "skirt",
            "alpha",
            "tattoo",
            "physics",
            "universal",
        ];
        let Some(kind) = names
            .iter()
            .position(|kind| *kind == name)
            .filter(|kind| (1..=14).contains(kind) || *kind == 16)
            .map(|kind| kind as u8)
        else {
            log::warn!("unknown wearable editor demo type");
            return;
        };
        crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
        let id = uuid::Uuid::from_u128(0xa0e0_1098_0000_0000_0000_0000_0000_0000 + u128::from(kind));
        let item = aurora_net::inventory::InvItem {
            id,
            parent: crate::world::appearance::system_folder(&self.world.inventory, if kind < 4 { 13 } else { 5 })
                .unwrap_or(self.world.inventory.root),
            asset_id: uuid::Uuid::from_u128(0xa0e0_1098_1000_0000_0000_0000_0000_0000 + u128::from(kind)),
            name: aurora_i18n::tr!("wearable-demo-name").into(),
            owner: self.world.agent_id,
            creator: self.world.agent_id,
            base_mask: 0x7fff_ffff,
            owner_mask: 0x7fff_ffff,
            next_owner_mask: 0x7fff_ffff,
            asset_type: if kind < 4 { 13 } else { 5 },
            inv_type: 18,
            flags: u32::from(kind),
            ..Default::default()
        };
        self.world.inventory.add_items(vec![item]);
        match crate::world::appearance::plan(
            &self.world.inventory,
            crate::world::appearance::Action::WearItem {
                item: id,
                replace: false,
                point: 0,
            },
            &std::collections::HashMap::new(),
        ) {
            Ok((change, _)) => {
                if let Err(error) = crate::world::appearance::demo_mutate(&mut self.world.inventory, self.world.agent_id, &change) {
                    log::warn!("wearable demo: {error}");
                    return;
                }
            }
            Err(error) => {
                log::warn!("wearable demo: {error}");
                return;
            }
        }
        if let Some(item) = self.world.inventory.items.get_mut(&id) {
            if variant == "readonly" {
                item.owner_mask &= !0x4000;
            }
            if variant == "nocopy" {
                item.owner_mask &= !0x8000;
            }
        }
        self.open_wearable_editor(id);
        self.panels.inventory = false;
        self.panels.perf = false;
        if let Some(draft) = &mut self.wearable_ui.draft
            && matches!(variant, "dirty" | "save" | "preview")
        {
            if let Some(param) = crate::wearable_editor::catalog::PARAMS.iter().find(|param| param.kind == kind) {
                draft.edited.params.insert(param.id, param.max);
            } else if let Some(channels) = crate::wearable_editor::color_params(kind) {
                draft.edited.params.insert(channels[0], 0.4);
            } else if let Some(slot) = crate::wearable_editor::slots(kind).first() {
                draft.edited.textures.insert(*slot, crate::scene::avatar::IMG_INVISIBLE);
            }
        }
        if matches!(variant, "preview" | "drag" | "drop") {
            let texture = crate::demo::TEX_HOLES;
            let mut image = self.world.inventory.items[&id].clone();
            image.id = DEMO_TEXTURE_ITEM;
            image.asset_type = 0;
            image.inv_type = 0;
            image.asset_id = texture;
            image.name = aurora_i18n::tr!("common-texture").into();
            self.world.inventory.add_items(vec![image]);
            if let Some(slot) = crate::wearable_editor::slots(kind).first()
                && variant == "preview"
            {
                self.wearable_ui.set_texture(&self.world, *slot, texture);
            }
        }
        if variant == "save-as" {
            self.wearable_ui.copy_name = Some(aurora_i18n::tr!("wearable-demo-copy").into());
        }
        if variant == "dirty" {
            self.wearable_ui.close_confirmation = true;
        }
        self.wearable_ui.changed(&mut self.world);
    }

    /// `AURORA_DEMO_WEARABLE=<type>:drag|drop`: the demo texture carried from
    /// the inventory over the first texture row from frame 700, without a
    /// press; `drop` releases it there at frame 720.
    pub(super) fn wearable_drop_demo_input(&mut self, raw: &mut egui::RawInput) {
        let Ok(mode) = std::env::var("AURORA_DEMO_WEARABLE") else { return };
        let drop = mode.ends_with(":drop");
        if !drop && !mode.ends_with(":drag") || self.frame_count < 700 || drop && self.frame_count > 720 {
            return;
        }
        let ctx = self.egui_ctx.clone();
        if self.frame_count == 700 {
            ui::inventory::drag::demo_carry(&ctx, vec![DEMO_TEXTURE_ITEM]);
        }
        let Some(pos) = ui::wearable::demo_row(&ctx).map(|row| row.center()) else {
            return;
        };
        raw.events.push(egui::Event::PointerMoved(pos));
        if drop && self.frame_count == 720 {
            raw.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
}

/// The inventory texture of the `preview`, `drag` and `drop` demo modes.
const DEMO_TEXTURE_ITEM: uuid::Uuid = uuid::Uuid::from_u128(0xa0e0_1098_2000_0000_0000_0000_0000_0001);

fn demo_asset(item: &aurora_net::inventory::InvItem, owner: uuid::Uuid) -> Result<Vec<u8>, String> {
    let kind = (item.flags & 0xff) as u8;
    let bytes = crate::world::inventory::wearable::create(kind, &item.name, owner)?;
    if matches!(kind, 2 | 3) {
        let mut value = aurora_assets::wearable::Wearable::parse(&bytes).map_err(|_| aurora_i18n::tr!("wearable-error").to_owned())?;
        for texture in value.textures.values_mut() {
            *texture = if kind == 2 {
                crate::demo::TEX_HAIR_CARD
            } else {
                crate::demo::TEX_GRADIENT
            };
        }
        value.encode(&item.name).map_err(|_| aurora_i18n::tr!("wearable-error").to_owned())
    } else {
        Ok(bytes)
    }
}
