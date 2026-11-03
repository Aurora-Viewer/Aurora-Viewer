//! Native appearance editing for the remaining LLWearableType entries.
use super::*;

impl App {
    pub(super) fn close_wearable_editor(&mut self) -> bool {
        if !self.wearable_ui.open {
            return true;
        }
        self.wearable_ui.dismiss_picker();
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
        self.wearable_local_demo();
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
        if variant.starts_with("local") || variant.starts_with("picker-") {
            let mut entry = crate::imports::Entry::new(
                "local-wearable.png".into(),
                Ok(crate::imports::Source::Texture(demo_local_image(false))),
                Default::default(),
            );
            entry.id = DEMO_LOCAL_TEXTURE;
            entry.local = true;
            self.imports.entries.push(entry);
        }
    }

    fn wearable_local_demo(&mut self) {
        if !self.demo || !self.wearable_ui.open {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_WEARABLE") else { return };
        let Some((_, variant)) = mode
            .split_once(':')
            .filter(|(_, variant)| variant.starts_with("local") || variant.starts_with("picker-"))
        else {
            return;
        };
        if variant.starts_with("picker-") {
            if (500..700).contains(&self.frame_count)
                && self.wearable_ui.picker_slot.is_none()
                && self.world.local_assets.contains_key(&DEMO_LOCAL_TEXTURE)
                && let Some(slot) = self
                    .wearable_ui
                    .draft
                    .as_ref()
                    .and_then(|draft| crate::wearable_editor::slots(draft.edited.kind).first())
                    .copied()
            {
                self.wearable_ui.open_picker(&self.world, slot);
                self.wearable_ui.picker.show_local_tab();
                // Leave the avatar visible during the automatic click captures.
                ui::widgets::move_floater(
                    &self.egui_ctx,
                    "texture_picker",
                    egui::pos2(self.egui_ctx.content_rect().right() - 340.0, 70.0),
                );
            }
            return;
        }
        if self.frame_count == 900
            && variant == "local-reload"
            && let Some(entry) = self.imports.entries.iter_mut().find(|entry| entry.id == DEMO_LOCAL_TEXTURE)
        {
            entry.source = Some(crate::imports::Source::Texture(demo_local_image(true)));
            entry.preview = None;
            entry.preview_pending = false;
            let _ = entry.invalidate_preview();
        }
        if self.frame_count == 1000 && variant == "local-cancel" {
            self.wearable_ui.close(&mut self.world);
            return;
        }
        if self.frame_count < 500 || !self.world.local_assets.contains_key(&DEMO_LOCAL_TEXTURE) {
            return;
        }
        let Some(draft) = &self.wearable_ui.draft else { return };
        let kind = draft.edited.kind;
        if crate::wearable_editor::slots(kind)
            .iter()
            .any(|slot| draft.edited.textures.get(slot) != Some(&DEMO_LOCAL_TEXTURE))
        {
            for &slot in crate::wearable_editor::slots(kind) {
                self.wearable_ui.set_texture(&self.world, slot, DEMO_LOCAL_TEXTURE);
            }
            self.wearable_ui.changed(&mut self.world);
        }
        if self.frame_count == 700
            && variant == "local-picker"
            && let Some(slot) = crate::wearable_editor::slots(kind).first()
        {
            self.wearable_ui.open_picker(&self.world, *slot);
            self.wearable_ui.picker.show_local_tab();
        }
    }

    /// Native drag/drop and picker clicks for `AURORA_DEMO_WEARABLE`.
    /// The drag starts at 700 and drops at 720. Picker demos click a local
    /// row at 700, then Cancel or the title-bar close button at 1000.
    pub(super) fn wearable_drop_demo_input(&mut self, raw: &mut egui::RawInput) {
        let Ok(mode) = std::env::var("AURORA_DEMO_WEARABLE") else { return };
        if mode.contains(":picker-") {
            let selecting = matches!(self.frame_count, 700 | 701);
            let cancel = mode.ends_with(":picker-cancel") && matches!(self.frame_count, 1000 | 1001);
            let close = mode.ends_with(":picker-close") && matches!(self.frame_count, 1000 | 1001);
            if !selecting && !cancel && !close {
                return;
            }
            let ctx = &self.egui_ctx;
            let rect = if selecting {
                ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("wearable_picker_row", DEMO_LOCAL_TEXTURE))))
            } else if cancel {
                ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("wearable_picker_cancel")))
            } else {
                ctx.memory(|memory| memory.area_rect(egui::Id::unique("texture_picker")))
            };
            if let Some(rect) = rect {
                let pos = if close {
                    egui::pos2(rect.right() - 18.0, rect.top() + 18.0)
                } else {
                    rect.center()
                };
                raw.events.push(egui::Event::PointerMoved(pos));
                raw.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(self.frame_count, 700 | 1000),
                    modifiers: egui::Modifiers::NONE,
                });
            }
            return;
        }
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

const DEMO_LOCAL_TEXTURE: uuid::Uuid = uuid::Uuid::from_u128(0xa0e0_1548_2000_0000_0000_0000_0000_0001);

fn demo_local_image(reloaded: bool) -> std::sync::Arc<aurora_assets::DecodedImage> {
    let mut data = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            data.extend(if (x / 8 + y / 8) % 2 == 0 {
                if reloaded { [30, 180, 90, 220] } else { [140, 40, 190, 220] }
            } else {
                [0; 4]
            });
        }
    }
    std::sync::Arc::new(aurora_assets::DecodedImage {
        width: 64,
        height: 64,
        components: 4,
        data,
        discard: 0,
    })
}

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
