//! Inventory material editing follows LLMaterialEditor::{loadAsset,saveIfNeeded,
//! onClickSaveAs,finishSaveAs} and panel_gltf_material.xml (Legacy Viewer).
//! Drafts only feed the preview cache; world materials change after a save ack.
use super::*;
use aurora_assets::{PbrMaterial, material::AlphaMode};
use std::sync::Arc;

pub(super) struct Editor {
    pub asset: Uuid,
    pub shape: crate::imports::preview::MaterialShape,
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
    map_names: [(Option<Uuid>, u64, String); 4],
}
impl Editor {
    pub fn new(asset: Uuid, material: Arc<PbrMaterial>) -> Self {
        Self {
            asset,
            shape: Default::default(),
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
            map_names: std::array::from_fn(|_| (None, u64::MAX, String::new())),
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
        thumbnails: &previews::Thumbnails,
        model: Option<Arc<aurora_render::preview::Model>>,
        view: &mut aurora_render::preview::View,
        path: &str,
        actions: &mut Vec<Action>,
    ) {
        // Allocate the chosen window size once. Scroll contents and wrapped
        // footer rows must not feed a larger minimum back into the floater.
        let (bounds, _) = ui.allocate_exact_size(ui.available_size().max(Vec2::ZERO), egui::Sense::hover());
        let mut footer = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("material_footer")
                .max_rect(bounds)
                .layout(egui::Layout::bottom_up(egui::Align::Min)),
        );
        footer.set_clip_rect(bounds.intersect(ui.clip_rect()));
        let first_action = actions.len();
        self.footer(&mut footer, p, item, world, actions);
        let body = egui::Rect::from_min_max(
            bounds.min,
            egui::pos2(
                bounds.right(),
                (footer.min_rect().top() - ui.spacing().item_spacing.y).max(bounds.top()),
            ),
        );
        let mut content = ui.new_child(egui::UiBuilder::new().id_salt("material_content").max_rect(body));
        content.set_clip_rect(body.intersect(ui.clip_rect()));
        let editable = can_edit(world, item);
        let mut value = (*self.draft).clone();
        let output = egui::ScrollArea::vertical()
            .id_salt("material_settings")
            .auto_shrink([false; 2])
            .min_scrolled_height(0.0)
            .max_height(body.height())
            .show(&mut content, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let choice = egui::ComboBox::from_id_salt("material_shape")
                        .selected_text(self.shape.label())
                        .show_ui(ui, |ui| {
                            for shape in crate::imports::preview::MaterialShape::ALL {
                                let response = ui.selectable_value(&mut self.shape, shape, shape.label());
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(egui::Id::unique(("resource_material_shape", item.id, shape)), response.rect)
                                });
                            }
                        });
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::unique(("resource_material_shape_choice", item.id)), choice.response.rect)
                    });
                    if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("import-preview-reset-view"), true) {
                        view.reset_camera();
                    }
                });
                let side = ui.available_width().min((body.height() * 0.33).clamp(96.0, 240.0));
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - side) * 0.5).max(0.0));
                    view.background = previews::view(p).background;
                    let response = if let Some(model) = model {
                        aurora_render::preview::paint(ui, ui.scope_id().with(item.id), model, view, Vec2::splat(side))
                    } else {
                        widgets::picture(ui, p, None, Vec2::splat(side), "cube-3d-sphere", "")
                    }
                    .on_hover_text(aurora_i18n::tr!("import-preview-gestures"));
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("resource_material_scene", item.id)), response.rect));
                });
                ui.horizontal_wrapped(|ui| {
                    let label = aurora_i18n::tr!("texture-library-material-drag");
                    let width = widgets::action_button_fit(ui, "grip-vertical", label).min(ui.available_width());
                    let path_width = (ui.available_width() - width - ui.spacing().item_spacing.x).max(0.0);
                    ui.add_sized(
                        [path_width, 24.0],
                        egui::Label::new(RichText::new(path).small().color(p.muted)).truncate(),
                    )
                    .on_hover_text(path);
                    let drag = widgets::action_button(ui, p, "grip-vertical", label, ActionKind::Flat, Vec2::new(width, 24.0))
                        .interact(egui::Sense::click_and_drag())
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text(aurora_i18n::tr!("texture-library-material-drag-hint"));
                    drag.dnd_set_drag_payload(InvDrag(item.id));
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("resource_material_handle", item.id)), drag.rect));
                });
                ui.add_enabled_ui(editable && self.saving.is_none(), |ui| {
                    for slot in [0, 2, 3, 1] {
                        ui.push_id(slot, |ui| {
                            self.map_row(ui, p, item, world, &mut value, slot, images, thumbnails, actions)
                        });
                    }
                });
                if !editable {
                    ui.label(RichText::new(aurora_i18n::tr!("resource-preview-read-only")).small().color(p.muted));
                }
                ui.label(
                    RichText::new(if item.owner_mask & rules::COPY != 0 {
                        aurora_i18n::tr!("texture-library-copy")
                    } else {
                        aurora_i18n::tr!("texture-library-no-copy")
                    })
                    .small()
                    .color(p.muted),
                );
            });
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::unique(("resource_material_viewport", item.id)),
                (output.inner_rect, output.content_size),
            )
        });
        if value != *self.draft {
            self.draft = Arc::new(value);
            self.message.clear();
        }
        // The bottom-anchored footer is laid out first. Include numeric edits
        // committed on focus loss by this frame's click on Save / Save As.
        for action in &mut actions[first_action..] {
            if let Action::SaveMaterial {
                item: target, material, ..
            } = action
                && *target == item.id
            {
                *material = (*self.draft).clone();
            }
        }
    }
    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, item: &aurora_net::inventory::InvItem, world: &World, actions: &mut Vec<Action>) {
        let editable = can_edit(world, item);
        if self.saving.is_some() {
            ui.label(aurora_i18n::tr!("resource-preview-saving"));
        } else if !self.message.is_empty() {
            ui.label(&self.message);
        } else if self.dirty() {
            ui.label(RichText::new(aurora_i18n::tr!("resource-preview-unsaved")).small().color(p.warn));
        } else {
            ui.label(RichText::new(aurora_i18n::tr!("resource-preview-no-fee")).small().color(p.muted));
        }
        if let Some(name) = &mut self.save_as {
            let mut submit = false;
            let mut cancel = false;
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(name).desired_width((ui.available_width() - 130.0).max(0.0)));
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
        ui.horizontal_wrapped(|ui| {
            let save = ui.add_enabled(
                editable && self.dirty() && self.saving.is_none(),
                egui::Button::new(aurora_i18n::tr!("common-save")),
            );
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::unique(("resource_material_save", item.id)), save.rect));
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
    }
    fn map_row(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        item: &aurora_net::inventory::InvItem,
        world: &World,
        value: &mut PbrMaterial,
        slot: usize,
        images: &HashMap<Uuid, egui::TextureHandle>,
        thumbnails: &previews::Thumbnails,
        actions: &mut Vec<Action>,
    ) {
        let map = *map_mut(value, slot);
        let texture = thumbnails[slot]
            .as_ref()
            .filter(|(id, _)| Some(*id) == map)
            .map(|(_, image)| image)
            .or_else(|| map.and_then(|id| images.get(&id)));
        let side = if ui.available_width() >= 350.0 { 95.0 } else { 56.0 };
        let width = (ui.available_width() - side - ui.spacing().item_spacing.x).max(0.0);
        ui.horizontal_top(|ui| {
            let response = widgets::picture(ui, p, texture, Vec2::splat(side), "photo-filled", "");
            self.drop_map(ui, p, response.rect, world, value, slot);
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::unique(("resource_material_map", item.id, slot)),
                    (map, texture.map(egui::TextureHandle::id), response.rect),
                )
            });
            if response.clicked() {
                self.open_map(slot, map);
            }
            ui.allocate_ui_with_layout(Vec2::new(width, side), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                ui.horizontal_wrapped(|ui| {
                    let title_width = (width - 4.0 * 24.0 - 4.0 * ui.spacing().item_spacing.x).max(0.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(title_width, 20.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_size(Vec2::new(title_width, 20.0));
                            ui.add(egui::Label::new(RichText::new(map_label(slot)).strong()).truncate())
                        },
                    )
                    .inner
                    .on_hover_text(map_label(slot));
                    if widgets::icon_button(ui, p, "folder-open-filled", aurora_i18n::tr!("resource-preview-upload-map"), true) {
                        actions.push(Action::MaterialMapFile { item: item.id, slot });
                    }
                    if widgets::icon_button(ui, p, "archive-filled", aurora_i18n::tr!("common-inventory"), true) {
                        self.open_map(slot, map);
                    }
                    if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-remove"), map.is_some()) {
                        *map_mut(value, slot) = None;
                    }
                    if widgets::icon_button(ui, p, "copy-filled", aurora_i18n::tr!("resource-preview-copy-uuid"), map.is_some()) {
                        ui.ctx().copy_text(map.unwrap_or_default().to_string());
                    }
                });
                let cached = &mut self.map_names[slot];
                if cached.0 != map || cached.1 != world.inventory.generation {
                    *cached = (
                        map,
                        world.inventory.generation,
                        map.map(|id| {
                            world
                                .inventory
                                .items
                                .values()
                                .find(|it| it.asset_id == id)
                                .map(|it| it.name.clone())
                                .unwrap_or_else(|| id.to_string())
                        })
                        .unwrap_or_else(|| aurora_i18n::tr!("import-no-map").into()),
                    );
                }
                ui.add(egui::Label::new(RichText::new(&cached.2).small().color(p.muted)).truncate())
                    .on_hover_text(&cached.2);
                ui.horizontal_wrapped(|ui| match slot {
                    0 => {
                        ui.label(aurora_i18n::tr!("build-ui-texture-tint"));
                        crate::imports::ui::factor_swatch(ui, p, "base_factor", &mut value.base_color_factor, true);
                        scalar(ui, aurora_i18n::tr!("common-alpha"), &mut value.base_color_factor[3]);
                    }
                    2 => {
                        scalar(ui, aurora_i18n::tr!("import-metallic"), &mut value.metallic_factor);
                        scalar(ui, aurora_i18n::tr!("import-roughness"), &mut value.roughness_factor);
                    }
                    3 => {
                        ui.label(aurora_i18n::tr!("build-ui-texture-tint"));
                        crate::imports::ui::factor_swatch(ui, p, "emissive_factor", &mut value.emissive_factor, false);
                    }
                    _ => {}
                });
                if slot == 0 {
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("alpha_mode")
                            .width(120.0)
                            .selected_text(alpha_label(value.alpha_mode))
                            .show_ui(ui, |ui| {
                                for mode in [AlphaMode::Opaque, AlphaMode::Blend, AlphaMode::Mask] {
                                    ui.selectable_value(&mut value.alpha_mode, mode, alpha_label(mode));
                                }
                            })
                            .response
                            .on_hover_text(aurora_i18n::tr!("import-alpha-mode"));
                        if value.alpha_mode == AlphaMode::Mask {
                            scalar(ui, aurora_i18n::tr!("import-alpha-cutoff-short"), &mut value.alpha_cutoff);
                        }
                    });
                    widgets::switch_labeled(ui, p, &mut value.double_sided, aurora_i18n::tr!("import-double-sided"));
                }
            });
        });
    }
    /// LLTextureCtrl::handleDragAndDrop: a texture dragged from the inventory
    /// onto a map's swatch replaces that map, through the same filter as the
    /// picker. Disabled swatches (read-only material, save in flight) refuse it.
    fn drop_map(&mut self, ui: &egui::Ui, p: &Palette, rect: egui::Rect, world: &World, value: &mut PbrMaterial, slot: usize) {
        if !ui.rect_contains_pointer(rect) {
            return;
        }
        let Some(cargo) = egui::DragAndDrop::payload::<InvDrag>(ui.ctx()) else {
            return;
        };
        let texture = world.inventory.items.get(&cargo.0).filter(|it| is_texture(it));
        let allowed = ui.is_enabled() && texture.is_some_and(map_source);
        crate::ui::inventory::drag::feedback(
            ui.ctx(),
            if allowed {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::NotAllowed
            },
        );
        crate::ui::inventory::tree::drop_outline(ui, p, rect, allowed);
        if !ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
            return;
        }
        egui::DragAndDrop::clear_payload(ui.ctx());
        match texture {
            Some(item) if allowed => {
                *map_mut(value, slot) = Some(item.asset_id);
                self.message.clear();
            }
            // TooltipTextureRestrictedDrop
            Some(_) if ui.is_enabled() => self.message = aurora_i18n::tr!("resource-preview-map-rights").into(),
            _ => {}
        }
    }
    fn open_map(&mut self, slot: usize, map: Option<Uuid>) {
        self.picker.open_with_defaults(map_label(slot), map.unwrap_or_default(), None, true);
        self.map_slot = Some(slot);
    }
    pub fn show_picker(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> HashSet<Uuid> {
        if let Some(asset) = self.picker.show_live(ctx, p, world, images)
            && let Some(slot) = self.map_slot
            && self.saving.is_none()
        {
            let allowed = self.picker.cancelled()
                || asset.is_nil()
                || crate::scene::avatar::bake_slot_for(&asset).is_some()
                || [aurora_prim::te::BLANK_TEXTURE, aurora_prim::te::TRANSPARENT_TEXTURE].contains(&asset)
                || world.inventory.items.values().any(|it| it.asset_id == asset && map_source(it));
            if allowed {
                let mut value = (*self.draft).clone();
                *map_mut(&mut value, slot) = (!asset.is_nil()).then_some(asset);
                self.draft = Arc::new(value);
            } else {
                self.message = aurora_i18n::tr!("resource-preview-map-rights").into();
            }
        }
        if !self.picker.open {
            self.map_slot = None;
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
fn is_texture(item: &aurora_net::inventory::InvItem) -> bool {
    matches!(item.asset_type, 0 | 15)
}
/// LLMaterialEditor::postBuild filters replacement maps by COPY|TRANSFER,
/// for the picker and for drops alike.
fn map_source(item: &aurora_net::inventory::InvItem) -> bool {
    is_texture(item) && item.owner_mask & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER
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
    fn pbr_bom_choices_preview_cancel_and_save_the_magic_uuid_in_every_map() {
        for slot in 0..4 {
            let mut ui = crate::ui::texture_picker::tests::PickerUi::new();
            let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut editor = Editor::new(Uuid::from_u128(1), Arc::new(PbrMaterial::default()));
            let magic = crate::scene::avatar::BAKES[0].2;
            for cancel in [true, false] {
                editor.open_map(slot, None);
                ui.settle(|ctx, p| {
                    editor.show_picker(ctx, p, &mut world, &HashMap::new());
                });
                for label in [aurora_i18n::tr!("picker-bom"), aurora_i18n::tr!("picker-bom-head")] {
                    ui.click(label, |ctx, p| {
                        editor.show_picker(ctx, p, &mut world, &HashMap::new());
                    });
                    ui.settle(|ctx, p| {
                        editor.show_picker(ctx, p, &mut world, &HashMap::new());
                    });
                }
                assert_eq!(*map_mut(Arc::make_mut(&mut editor.draft), slot), Some(magic));
                assert!(editor.picker.open);
                assert!(editor.message.is_empty());
                ui.click(
                    if cancel {
                        aurora_i18n::tr!("common-cancel")
                    } else {
                        aurora_i18n::tr!("common-ok")
                    },
                    |ctx, p| {
                        editor.show_picker(ctx, p, &mut world, &HashMap::new());
                    },
                );
                assert_eq!(*map_mut(Arc::make_mut(&mut editor.draft), slot), (!cancel).then_some(magic));
            }
        }
    }

    #[test]
    fn live_picker_edits_the_same_map_repeatedly_and_cancel_restores_an_unlisted_original() {
        let mut ui = crate::ui::texture_picker::tests::PickerUi::new();
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let original = Uuid::from_u128(77);
        let mut editor = Editor::new(
            Uuid::from_u128(1),
            Arc::new(PbrMaterial {
                normal_texture: Some(original),
                ..Default::default()
            }),
        );
        editor.open_map(1, Some(original));
        ui.settle(|ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        ui.click(aurora_i18n::tr!("common-blank"), |ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        assert_eq!(editor.draft.normal_texture, Some(aurora_prim::te::BLANK_TEXTURE));
        assert!(editor.picker.open);
        ui.click(aurora_i18n::tr!("common-none-2"), |ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        assert_eq!(editor.draft.normal_texture, None);
        Arc::make_mut(&mut editor.draft).roughness_factor = 0.25;
        ui.click(aurora_i18n::tr!("common-cancel"), |ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        assert_eq!(editor.draft.normal_texture, Some(original));
        assert_eq!(editor.draft.roughness_factor, 0.25);
        assert_eq!(editor.map_slot, None);
    }
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
    fn only_fully_permissive_textures_fill_a_map() {
        let texture = |asset_type, owner_mask| aurora_net::inventory::InvItem {
            asset_type,
            owner_mask,
            ..Default::default()
        };
        let full = rules::COPY | rules::MODIFY | rules::TRANSFER;
        assert!(map_source(&texture(0, full)));
        assert!(map_source(&texture(0, rules::COPY | rules::TRANSFER)));
        assert!(!map_source(&texture(0, rules::COPY | rules::MODIFY)));
        assert!(!map_source(&texture(0, rules::MODIFY | rules::TRANSFER)));
        assert!(!map_source(&texture(7, full)));
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
