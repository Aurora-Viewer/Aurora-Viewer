//! Toolbar profiles travel through the existing inventory notecard commands.
use super::App;
use crate::{
    theme::Palette,
    ui::{
        self,
        toolbar::{ToolbarSettings, presets::Preset},
    },
};
use aurora_net::inventory::{InvItem, notecard::Notecard};
use uuid::Uuid;

impl App {
    pub(super) fn toolbar_notecard_ui(&mut self, ctx: &egui::Context, p: &Palette) {
        self.demo_toolbar_notecards();
        let interfaces = self
            .settings
            .toolbar
            .as_ref()
            .is_some_and(|toolbar| toolbar.drawers.contains_key(ui::toolbar::drawers::INTERFACES));
        if self.toolbar_ui.open || self.toolbar_ui.notecards.active() || interfaces {
            self.toolbar_ui.notecards.sync_library(&mut self.world.inventory);
        }
        if let Some(command) = self.toolbar_ui.notecards.removal_command(
            &mut self.world.inventory,
            &self.settings.inventory.protected,
            self.settings.inventory.confirm_delete,
        ) {
            self.send(command);
        }
        if !self.toolbar_ui.notecards.active() {
            return;
        }
        let default = ToolbarSettings::default();
        let layout = self.dock_layout.of(&self.skin.layout, self.settings.chat_bar.hides());
        let command = self.toolbar_ui.notecards.show(
            ctx,
            p,
            &mut self.world.inventory,
            self.settings.toolbar.as_ref().unwrap_or(&default),
            layout,
            !self.settings.classic_bars,
        );
        if let Some(command) = command {
            self.send(command);
        }
    }

    pub(super) fn toolbar_notecard_preview(&mut self, item: Uuid, asset: Uuid, result: &Result<Vec<u8>, String>) {
        if !self.toolbar_ui.notecards.loading(item) {
            return;
        }
        let default = ToolbarSettings::default();
        let profile = self.toolbar_ui.notecards.loaded(
            &self.world.inventory,
            self.settings.toolbar.as_ref().unwrap_or(&default),
            !self.settings.classic_bars,
            item,
            asset,
            result,
        );
        if let Some(profile) = profile {
            ui::toolbar::profile::apply(profile, &mut self.settings);
            if let Some(mut selection) = self.toolbar_ui.notecards.take_selection() {
                let layout = self.dock_layout.of(&self.skin.layout, self.settings.chat_bar.hides());
                selection.baseline = ui::toolbar::profile::snapshot(
                    self.settings.toolbar.as_ref().unwrap_or(&default),
                    layout,
                    !self.settings.classic_bars,
                );
                self.settings.toolbar_selection = Some(selection);
            }
            self.toolbar_ui.profile_applied(&self.egui_ctx);
            self.settings.save();
        }
        self.egui_ctx.request_repaint();
    }

    fn demo_toolbar_notecards(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_TOOLBAR") else { return };
        if !mode.starts_with("notecard-") {
            return;
        }
        if !matches!(self.frame_count, 300 | 310 | 340 | 360) {
            return;
        }
        let stage = egui::Id::unique(("toolbar_notecard_demo_stage", self.frame_count));
        if self.egui_ctx.data_mut(|data| {
            let done = data.get_temp::<bool>(stage).unwrap_or(false);
            data.insert_temp(stage, true);
            done
        }) {
            return;
        }
        if self.frame_count == 300 {
            crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
            self.world.inventory.owner = self.world.agent_id;
            let preset = if mode == "notecard-roundtrip-legacy" {
                Preset::Legacy
            } else {
                Preset::OldSchool
            };
            preset.apply(&mut self.settings, &self.skin.layout);
            self.toolbar_ui.open = true;
            let Ok(text) = ui::toolbar::profile::write(&Preset::Photographer.layout(&self.skin.layout), &self.skin.layout, true) else {
                return;
            };
            let parent = self
                .world
                .inventory
                .folders
                .values()
                .find(|folder| !folder.library && folder.info.type_default == 7)
                .map_or(self.world.inventory.root, |folder| folder.info.id);
            for (id, name, text) in [
                (90_001, aurora_i18n::tr!("toolbar-notecard-demo-valid"), text.as_str()),
                (90_002, aurora_i18n::tr!("toolbar-notecard-demo-invalid"), "{}"),
            ] {
                let Ok(data) = Notecard::default().encode(text) else { continue };
                let item = InvItem {
                    id: Uuid::from_u128(id),
                    asset_id: Uuid::from_u128(id + 100),
                    parent,
                    name: name.into(),
                    asset_type: 7,
                    inv_type: 7,
                    owner: self.world.agent_id,
                    creator: self.world.agent_id,
                    base_mask: crate::world::inventory::actions::FULL,
                    owner_mask: crate::world::inventory::actions::FULL,
                    next_owner_mask: crate::world::inventory::actions::FULL,
                    ..Default::default()
                };
                self.demo_documents.insert(item.id, data);
                self.world.inventory.add_items(vec![item]);
            }
        }
        // Let the editor finish opening before its resource chooser takes focus.
        if self.frame_count == 310 && !mode.starts_with("notecard-preset-") {
            self.toolbar_ui.notecards.open(
                if matches!(mode.as_str(), "notecard-save" | "notecard-readable")
                    || mode.starts_with("notecard-roundtrip")
                    || mode.starts_with("notecard-library")
                {
                    ui::inventory::context_notecards::Mode::Export
                } else {
                    ui::inventory::context_notecards::Mode::Load
                },
            );
        }
        if self.frame_count == 340 && mode.starts_with("notecard-preset-") {
            if mode == "notecard-preset-dirty" {
                if let Some(toolbar) = &mut self.settings.toolbar {
                    toolbar.address_visible = Some(false);
                }
            } else if matches!(
                mode.as_str(),
                "notecard-preset-selector" | "notecard-preset-personal" | "notecard-preset-delete"
            ) {
                self.toolbar_ui
                    .notecards
                    .open_save(self.settings.toolbar_selection.as_ref().map(|selection| &selection.source));
                if let Some(toolbar) = self.settings.toolbar.as_ref()
                    && let Some(command) = self.toolbar_ui.notecards.demo_save(
                        &mut self.world.inventory,
                        toolbar,
                        &self.skin.layout,
                        !self.settings.classic_bars,
                    )
                {
                    self.send(command);
                }
            }
        }
        if self.frame_count == 360 && mode == "notecard-preset-selector" && self.toolbar_ui.notecards.demo_saved_item().is_some() {
            if let Some(toolbar) = &mut self.settings.toolbar {
                toolbar.address_visible = Some(false);
            }
            self.toolbar_ui
                .notecards
                .open_save(self.settings.toolbar_selection.as_ref().map(|selection| &selection.source));
        }
        if self.frame_count == 360 && mode == "notecard-preset-delete" {
            self.toolbar_ui.notecards.sync_library(&mut self.world.inventory);
            if let Some(selection) = self.settings.toolbar_selection.as_ref() {
                self.toolbar_ui.notecards.demo_remove(&selection.source);
            }
        }
        if self.frame_count == 340
            && (mode.starts_with("notecard-roundtrip") || mode == "notecard-readable" || mode.starts_with("notecard-library"))
            && let Some(toolbar) = self.settings.toolbar.as_ref()
            && let Some(command) =
                self.toolbar_ui
                    .notecards
                    .demo_save(&mut self.world.inventory, toolbar, &self.skin.layout, !self.settings.classic_bars)
        {
            self.send(command);
        }
        if self.frame_count == 360 && mode.starts_with("notecard-library") && self.toolbar_ui.notecards.demo_saved_item().is_some() {
            self.toolbar_ui.notecards = Default::default();
            if mode == "notecard-library-load" {
                self.settings.classic_bars = true;
                self.settings.toolbar = Some(Preset::Photographer.layout(&self.skin.layout));
            }
        }
        if self.frame_count == 360
            && mode == "notecard-readable"
            && let Some(item) = self.toolbar_ui.notecards.demo_saved_item()
        {
            self.toolbar_ui.open = false;
            self.toolbar_ui.notecards = Default::default();
            self.inventory_ui.preview = Some(item);
            self.apply_inventory_action(ui::inventory::InvAction::Preview(item, None));
        }
        if self.frame_count == 360
            && mode.starts_with("notecard-roundtrip")
            && let Some(item) = self.toolbar_ui.notecards.demo_saved_item()
        {
            self.settings.classic_bars = !self.settings.classic_bars;
            self.settings.toolbar = Some(Preset::Photographer.layout(&self.skin.layout));
            if let Some(command) = self.toolbar_ui.notecards.demo_load(
                &self.world.inventory,
                self.settings.toolbar.as_ref(),
                !self.settings.classic_bars,
                item,
            ) {
                self.send(command);
            }
        }
        if self.frame_count == 340 && mode == "notecard-invalid" {
            let id = Uuid::from_u128(if mode == "notecard-invalid" { 90_002 } else { 90_001 });
            if let Some(command) = self.toolbar_ui.notecards.demo_load(
                &self.world.inventory,
                self.settings.toolbar.as_ref(),
                !self.settings.classic_bars,
                id,
            ) {
                self.send(command);
            }
        }
    }
}
