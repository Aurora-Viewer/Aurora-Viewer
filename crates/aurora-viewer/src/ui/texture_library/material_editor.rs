//! Inventory material editing follows LLMaterialEditor::{loadAsset,saveIfNeeded,
//! onClickSaveAs,finishSaveAs} and panel_gltf_material.xml (Legacy Viewer).
//! Drafts only feed the preview cache; world materials change after a save ack.
use super::*;
use aurora_assets::{PbrMaterial, material::AlphaMode};
use std::sync::Arc;

pub(super) struct Editor {
    pub asset: Uuid,
    pub original: Arc<PbrMaterial>,
    pub draft: Arc<PbrMaterial>,
    pub saving: Option<Uuid>,
    pub submitted: Option<Arc<PbrMaterial>>,
    pub message: String,
    picker: super::super::texture_picker::TexturePicker,
    map_slot: Option<usize>,
    save_as: Option<String>,
    pub confirm_close: bool,
    pub close_after_save: bool,
    pub close_ready: bool,
    deferred_maps: Vec<(usize, Uuid)>,
}
impl Editor {
    pub fn new(asset: Uuid, material: Arc<PbrMaterial>) -> Self {
        Self {
            asset,
            original: material.clone(),
            draft: material,
            saving: None,
            submitted: None,
            message: String::new(),
            picker: Default::default(),
            map_slot: None,
            save_as: None,
            confirm_close: false,
            close_after_save: false,
            close_ready: false,
            deferred_maps: Vec::new(),
        }
    }
    pub fn dirty(&self) -> bool {
        self.original != self.draft
    }
    pub fn begin_save(&mut self, request: Uuid) {
        self.saving = Some(request);
        self.submitted = Some(self.draft.clone());
        self.message.clear();
    }
    pub fn finish_save(&mut self, result: &Result<aurora_net::inventory::InvItem, String>) {
        self.saving = None;
        match result {
            Ok(item) => {
                self.asset = catalogue::asset(item);
                if let Some(value) = self.submitted.take() {
                    self.original = value;
                }
                self.message = aurora_i18n::tr!("resource-preview-saved").into();
                self.close_ready = self.close_after_save && !self.dirty();
            }
            Err(reason) => {
                self.submitted = None;
                self.message.clone_from(reason);
                self.close_after_save = false;
            }
        }
        for (slot, asset) in std::mem::take(&mut self.deferred_maps) {
            self.set_map(slot, asset);
        }
        self.close_ready &= !self.dirty();
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        item: &aurora_net::inventory::InvItem,
        world: &World,
        images: &HashMap<Uuid, egui::TextureHandle>,
        actions: &mut Vec<Action>,
    ) {
        let editable = can_edit(world, item);
        let mut value = (*self.draft).clone();
        let height = (ui.available_height() - 92.0).max(40.0);
        egui::ScrollArea::vertical()
            .id_salt("material_settings")
            .auto_shrink([false; 2])
            .max_height(height)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add_enabled_ui(editable && self.saving.is_none(), |ui| {
                    widgets::switch_labeled(ui, p, &mut value.double_sided, aurora_i18n::tr!("import-double-sided"));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(aurora_i18n::tr!("import-base-color"));
                        crate::imports::ui::factor_swatch(ui, p, "base_factor", &mut value.base_color_factor, false);
                        scalar(ui, aurora_i18n::tr!("common-alpha"), &mut value.base_color_factor[3]);
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(aurora_i18n::tr!("import-alpha-mode"));
                        egui::ComboBox::from_id_salt("alpha_mode")
                            .selected_text(alpha_label(value.alpha_mode))
                            .show_ui(ui, |ui| {
                                for mode in [AlphaMode::Opaque, AlphaMode::Blend, AlphaMode::Mask] {
                                    ui.selectable_value(&mut value.alpha_mode, mode, alpha_label(mode));
                                }
                            });
                        ui.add_enabled_ui(value.alpha_mode == AlphaMode::Mask, |ui| {
                            scalar(ui, aurora_i18n::tr!("import-alpha-cutoff"), &mut value.alpha_cutoff)
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        scalar(ui, aurora_i18n::tr!("import-metallic"), &mut value.metallic_factor);
                        scalar(ui, aurora_i18n::tr!("import-roughness"), &mut value.roughness_factor);
                    });
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("import-emissive"));
                        crate::imports::ui::factor_swatch(ui, p, "emissive_factor", &mut value.emissive_factor, false);
                    });
                    for slot in [0, 2, 3, 1] {
                        let map = map_mut(&mut value, slot);
                        ui.push_id(slot, |ui| {
                            ui.horizontal(|ui| {
                                let response =
                                    widgets::picture(ui, p, map.and_then(|id| images.get(&id)), Vec2::splat(44.0), "photo-filled", "");
                                if response.clicked() {
                                    self.picker.open_with_defaults(map_label(slot), map.unwrap_or_default(), None, true);
                                    self.map_slot = Some(slot);
                                }
                                ui.vertical(|ui| {
                                    ui.label(map_label(slot));
                                    ui.horizontal(|ui| {
                                        if widgets::icon_button(
                                            ui,
                                            p,
                                            "folder-open-filled",
                                            aurora_i18n::tr!("resource-preview-upload-map"),
                                            true,
                                        ) {
                                            actions.push(Action::MaterialMapFile { item: item.id, slot });
                                        }
                                        if ui.small_button(aurora_i18n::tr!("common-inventory")).clicked() {
                                            self.picker.open_with_defaults(map_label(slot), map.unwrap_or_default(), None, true);
                                            self.map_slot = Some(slot);
                                        }
                                        if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-remove"), map.is_some()) {
                                            *map = None;
                                        }
                                        if widgets::icon_button(
                                            ui,
                                            p,
                                            "copy-filled",
                                            aurora_i18n::tr!("resource-preview-copy-uuid"),
                                            map.is_some(),
                                        ) {
                                            ui.ctx().copy_text(map.unwrap_or_default().to_string());
                                        }
                                    });
                                });
                            });
                        });
                    }
                });
                if !editable {
                    ui.label(RichText::new(aurora_i18n::tr!("resource-preview-read-only")).small().color(p.muted));
                }
            });
        if value != *self.draft {
            self.draft = Arc::new(value);
            self.message.clear();
        }
        ui.horizontal_wrapped(|ui| {
            let save = ui.add_enabled(
                editable && self.dirty() && self.saving.is_none(),
                egui::Button::new(aurora_i18n::tr!("common-save")),
            );
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new(("resource_material_save", item.id)), save.rect));
            if save.clicked() {
                actions.push(Action::SaveMaterial {
                    item: item.id,
                    material: (*self.draft).clone(),
                    name: None,
                });
            }
            if ui
                .add_enabled(
                    item.owner_mask & rules::COPY != 0 && self.saving.is_none(),
                    egui::Button::new(aurora_i18n::tr!("common-save-as")),
                )
                .clicked()
            {
                self.save_as = Some(item.name.clone());
            }
            if ui
                .add_enabled(
                    self.dirty() && self.saving.is_none(),
                    egui::Button::new(aurora_i18n::tr!("common-cancel")),
                )
                .clicked()
            {
                self.draft = self.original.clone();
                self.message.clear();
                self.picker.open = false;
            }
        });
        if let Some(name) = &mut self.save_as {
            let mut submit = false;
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(name).desired_width((ui.available_width() - 130.0).max(60.0)));
                submit = ui
                    .add_enabled(!name.trim().is_empty(), egui::Button::new(aurora_i18n::tr!("common-save")))
                    .clicked();
                cancel = ui.button(aurora_i18n::tr!("common-cancel")).clicked();
            });
            if submit {
                actions.push(Action::SaveMaterial {
                    item: item.id,
                    material: (*self.draft).clone(),
                    name: self.save_as.take(),
                });
            } else if cancel {
                self.save_as = None;
            }
        }
        if self.saving.is_some() {
            ui.label(aurora_i18n::tr!("resource-preview-saving"));
        } else if !self.message.is_empty() {
            ui.label(&self.message);
        } else if self.dirty() {
            ui.label(RichText::new(aurora_i18n::tr!("resource-preview-unsaved")).small().color(p.warn));
        } else {
            ui.label(RichText::new(aurora_i18n::tr!("resource-preview-no-fee")).small().color(p.muted));
        }
    }
    pub fn show_picker(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> HashSet<Uuid> {
        if let Some(asset) = self.picker.show(ctx, p, world, images)
            && let Some(slot) = self.map_slot.take()
            && self.saving.is_none()
        {
            // LLMaterialEditor::postBuild filters replacement maps by COPY|TRANSFER.
            let allowed = asset.is_nil()
                || [aurora_prim::te::BLANK_TEXTURE, aurora_prim::te::TRANSPARENT_TEXTURE].contains(&asset)
                || world.inventory.items.values().any(|it| {
                    it.asset_id == asset
                        && matches!(it.asset_type, 0 | 15)
                        && it.owner_mask & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER
                });
            if allowed {
                let mut value = (*self.draft).clone();
                *map_mut(&mut value, slot) = (!asset.is_nil()).then_some(asset);
                self.draft = Arc::new(value);
            } else {
                self.message = aurora_i18n::tr!("resource-preview-map-rights").into();
            }
        }
        std::mem::take(&mut self.picker.wanted_images)
    }
    pub fn set_map(&mut self, slot: usize, asset: Uuid) {
        if self.saving.is_some() {
            self.deferred_maps.retain(|(pending, _)| *pending != slot);
            self.deferred_maps.push((slot, asset));
            return;
        }
        let mut value = (*self.draft).clone();
        *map_mut(&mut value, slot) = Some(asset);
        self.draft = Arc::new(value);
        self.message.clear();
    }
}
pub(super) fn can_edit(world: &World, item: &aurora_net::inventory::InvItem) -> bool {
    item.owner == world.agent_id && !item.group_owned && item.owner_mask & rules::MODIFY != 0 && !rules::library(&world.inventory, item.id)
}
fn scalar(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    ui.label(label);
    ui.add(egui::DragValue::new(value).range(0.0..=1.0).speed(0.01).max_decimals(3));
}
fn alpha_label(mode: AlphaMode) -> &'static str {
    match mode {
        AlphaMode::Opaque => aurora_i18n::tr!("import-alpha-opaque"),
        AlphaMode::Blend => aurora_i18n::tr!("import-alpha-blend"),
        AlphaMode::Mask => aurora_i18n::tr!("import-alpha-mask"),
    }
}
fn map_label(slot: usize) -> &'static str {
    match slot {
        0 => aurora_i18n::tr!("import-base-color"),
        1 => aurora_i18n::tr!("import-normal"),
        2 => aurora_i18n::tr!("import-metallic-roughness"),
        _ => aurora_i18n::tr!("import-emissive"),
    }
}
fn map_mut(value: &mut PbrMaterial, slot: usize) -> &mut Option<Uuid> {
    match slot {
        0 => &mut value.base_color_texture,
        1 => &mut value.normal_texture,
        2 => &mut value.metallic_roughness_texture,
        _ => &mut value.emissive_texture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drafts_are_independent_and_failed_saves_retain_every_parameter() {
        let source = Arc::new(PbrMaterial::default());
        let mut first = Editor::new(Uuid::from_u128(1), source.clone());
        let second = Editor::new(Uuid::from_u128(1), source.clone());
        let draft = Arc::make_mut(&mut first.draft);
        draft.roughness_factor = 0.25;
        draft.alpha_mode = AlphaMode::Mask;
        draft.normal_texture = Some(Uuid::from_u128(22));
        draft.transforms[1].rotation = 0.6;
        assert!(first.dirty());
        assert!(!second.dirty());
        assert_eq!(second.draft, source);
        first.begin_save(Uuid::from_u128(2));
        first.finish_save(&Err("Rejected".into()));
        assert!(first.dirty());
        assert_eq!(first.draft.roughness_factor, 0.25);
        assert_eq!(first.draft.transforms[1].rotation, 0.6);
        first.begin_save(Uuid::from_u128(3));
        first.finish_save(&Ok(aurora_net::inventory::InvItem {
            asset_id: Uuid::from_u128(4),
            ..Default::default()
        }));
        assert!(!first.dirty());
        assert_eq!(first.asset, Uuid::from_u128(4));
        assert_eq!(source.roughness_factor, 1.0);
    }
    #[test]
    fn a_map_upload_finishing_during_save_is_kept_as_a_new_draft() {
        let mut editor = Editor::new(Uuid::from_u128(1), Arc::new(PbrMaterial::default()));
        editor.close_after_save = true;
        editor.begin_save(Uuid::from_u128(2));
        editor.set_map(1, Uuid::from_u128(3));
        assert_eq!(editor.draft.normal_texture, None);
        editor.finish_save(&Ok(aurora_net::inventory::InvItem {
            asset_id: Uuid::from_u128(4),
            ..Default::default()
        }));
        assert_eq!(editor.original.normal_texture, None);
        assert_eq!(editor.draft.normal_texture, Some(Uuid::from_u128(3)));
        assert!(editor.dirty());
        assert!(!editor.close_ready);
    }
}
