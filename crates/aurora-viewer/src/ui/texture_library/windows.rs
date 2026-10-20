//! Per-inventory-item windows, following LLFloaterReg::showInstance in
//! LLTextureBridgeAction::doIt and LLMaterialBridgeAction::doIt (Legacy Viewer,
//! indra/newview/llinventorybridge.cpp). The library and inventory share them.

use super::*;

pub(super) struct PreviewWindow {
    pub id: Uuid,
    pub view: aurora_render::preview::View,
    first_show: bool,
    pub(super) editor: Option<material_editor::Editor>,
    pub(super) texture: texture_view::ImageView,
    description: Option<String>,
}

pub(crate) fn window_id(item: Uuid) -> String {
    format!("resource_preview_{item}")
}

impl LibraryUi {
    pub(crate) fn prepare_exit(&mut self, ctx: &egui::Context) -> bool {
        let Some(window) = self.windows.iter_mut().find(|window| {
            window
                .editor
                .as_ref()
                .is_some_and(|editor| editor.dirty() || editor.saving.is_some())
        }) else {
            return true;
        };
        if let Some(editor) = &mut window.editor {
            editor.confirm_close = true;
        }
        widgets::open_floater(ctx, &window_id(window.id), &mut true);
        self.focus_preview = Some(window.id);
        false
    }
    pub(crate) fn material_message(&mut self, item: Uuid, message: String) {
        if let Some(editor) = self.windows.iter_mut().find(|w| w.id == item).and_then(|w| w.editor.as_mut()) {
            editor.message = message;
        }
    }
    pub(crate) fn demo_preview_controls(&mut self, stretch: bool) {
        for window in &mut self.windows {
            window.texture.display.keep_ratio = !stretch;
            if let Some(editor) = &mut window.editor {
                let draft = std::sync::Arc::make_mut(&mut editor.draft);
                draft.metallic_factor = 0.35;
                draft.roughness_factor = 0.6;
                draft.double_sided = true;
            }
        }
    }
    pub(crate) fn track_map_upload(&mut self, entry: Uuid, item: Uuid, slot: usize) {
        self.map_uploads.retain(|_, target| *target != (item, slot));
        self.map_uploads.insert(entry, (item, slot));
    }
    pub(crate) fn map_uploaded(&mut self, entry: Uuid, item: &aurora_net::inventory::InvItem) {
        if let Some((target, slot)) = self.map_uploads.remove(&entry)
            && let Some(editor) = self.windows.iter_mut().find(|w| w.id == target).and_then(|w| w.editor.as_mut())
        {
            editor.set_map(slot, item.asset_id);
        }
    }
    pub(crate) fn close_texture(&mut self, item: Uuid) {
        self.windows.retain(|window| window.id != item || window.editor.is_some());
    }
    pub(crate) fn material_draft_dirty(&self, item: Uuid) -> bool {
        self.windows
            .iter()
            .find(|w| w.id == item)
            .and_then(|w| w.editor.as_ref())
            .is_some_and(|e| e.dirty())
    }
    pub(crate) fn material_save_pending(&self, request: Uuid) -> bool {
        self.windows
            .iter()
            .any(|window| window.editor.as_ref().is_some_and(|editor| editor.saving == Some(request)))
    }
    pub(crate) fn begin_material_save(&mut self, item: Uuid, request: Uuid) -> bool {
        let Some(editor) = self.windows.iter_mut().find(|w| w.id == item).and_then(|w| w.editor.as_mut()) else {
            return false;
        };
        if editor.saving.is_some() {
            return false;
        }
        editor.begin_save(request);
        true
    }
    pub(crate) fn material_copy_ready(&mut self, request: Uuid, copied: Uuid) -> Option<aurora_assets::PbrMaterial> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.editor.as_ref().is_some_and(|e| e.saving == Some(request)))?;
        window.id = copied;
        let editor = window.editor.as_ref()?;
        editor
            .dirty()
            .then(|| (**editor.submitted.as_ref().unwrap_or(&editor.draft)).clone())
    }
    pub(crate) fn material_saved(
        &mut self,
        request: Uuid,
        result: &Result<aurora_net::inventory::InvItem, String>,
    ) -> Option<aurora_assets::PbrMaterial> {
        let window = self
            .windows
            .iter_mut()
            .find(|w| w.editor.as_ref().is_some_and(|e| e.saving == Some(request)))?;
        let editor = window.editor.as_mut()?;
        let material = editor.submitted.as_deref().cloned();
        editor.finish_save(result);
        material
    }
    pub(crate) fn export_texture(&mut self, item: Uuid, png: bool) {
        if let Some(window) = self.windows.iter_mut().find(|w| w.id == item) {
            window.texture.export = Some(png);
        }
    }
    pub(crate) fn refresh_texture(&mut self, world: &World, asset: Uuid) {
        for window in &mut self.windows {
            if catalogue::resource(&world.inventory, window.id).is_some_and(|it| it.asset_id == asset) {
                let display = window.texture.display;
                window.texture = Default::default();
                window.texture.display = display;
            }
        }
    }
    pub fn open_preview(&mut self, ctx: &egui::Context, world: &World, id: Uuid) {
        if self.agent != world.agent_id {
            self.reset();
            self.agent = world.agent_id;
        }
        let Some(item) = catalogue::resource(&world.inventory, id) else {
            return;
        };
        let mut existing = self.windows.iter().any(|window| window.id == item.id);
        if !existing {
            self.windows.push(PreviewWindow {
                id: item.id,
                view: Default::default(),
                first_show: true,
                editor: None,
                texture: Default::default(),
                description: None,
            });
        }
        widgets::open_floater(ctx, &window_id(item.id), &mut existing);
        self.focus_preview = Some(item.id);
    }

    pub(super) fn show_previews(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        scene: &mut Scene,
        images: &HashMap<Uuid, egui::TextureHandle>,
        prefs: &mut Preferences,
        actions: &mut Vec<Action>,
    ) {
        // Keep one shared worker/GPU cache for every swatch and window.
        let mut windows = std::mem::take(&mut self.windows);
        windows.retain_mut(|window| self.preview_window(ctx, p, world, scene, images, prefs, window, actions));
        self.windows = windows;
        self.map_uploads.retain(|_, (item, _)| self.windows.iter().any(|w| w.id == *item));
        if let Some(id) = self.focus_preview.take() {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(window_id(id))));
        }
    }

    fn preview_window(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        scene: &mut Scene,
        images: &HashMap<Uuid, egui::TextureHandle>,
        prefs: &mut Preferences,
        window: &mut PreviewWindow,
        actions: &mut Vec<Action>,
    ) -> bool {
        let Some(item) = catalogue::resource(&world.inventory, window.id).cloned() else {
            return false;
        };
        let material = item.asset_type == 57;
        let asset = catalogue::asset(&item);
        let path = super::super::inventory::inventory_path(&world.inventory, item.parent);
        let id = window_id(item.id);
        // Human-requested defaults by resource type. Only new windows inherit
        // the last manual size; visible windows retain their own geometry.
        if window.first_show {
            window.texture.display = prefs.texture_display;
            if let Some(size) = prefs.preview_size(material) {
                widgets::size_floater(ctx, &id, size);
            }
            window.first_show = false;
        }
        let mut open = true;
        let mut close_clicked = false;
        let size = prefs.preview_size(material).unwrap_or(if material {
            Vec2::new(480.0, 680.0)
        } else {
            Vec2::new(460.0, 520.0)
        });
        let mut floater = Floater::new(&id, &item.name, ctx.content_rect().center() - size * 0.5, size)
            .cascade(widgets::CascadeGroup::inventory(item.asset_type));
        floater.min_size = Vec2::new(220.0, 260.0);
        floater.show(ctx, p, &mut open, |ui| {
            if material {
                if window
                    .editor
                    .as_ref()
                    .is_none_or(|editor| editor.asset != asset && !editor.dirty() && editor.saving.is_none())
                    && let Some(value) = if item.asset_id.is_nil() {
                        Some(std::sync::Arc::new(aurora_assets::PbrMaterial::default()))
                    } else {
                        scene.materials.get(&asset)
                    }
                {
                    window.editor = Some(material_editor::Editor::new(asset, value));
                }
                let side = ui.available_width().min((ui.available_height() * 0.33).clamp(72.0, 240.0));
                let model = window.editor.as_ref().and_then(|editor| {
                    self.previews
                        .material_model(item.id, editor.draft.clone(), scene, &mut self.wanted_images, ctx)
                });
                if let Some(model) = model {
                    window.view.background = previews::view(p).background;
                    let response = aurora_render::preview::paint(ui, ui.id().with(item.id), model, &mut window.view, Vec2::splat(side));
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new(("resource_material_scene", item.id)), response.rect));
                } else {
                    ui.allocate_space(Vec2::splat(side));
                }
            } else {
                self.wanted_images.insert(asset);
                window.texture.prepare(ctx, asset, scene);
                let image = window.texture.image.as_ref().or_else(|| images.get(&asset));
                let dimensions = scene.textures.full_size(&asset).or(window.texture.dimensions);
                let before = window.texture.display;
                let footer = 112.0
                    + if window.texture.metadata.uploader.is_some() { 24.0 } else { 0.0 }
                    + if window.texture.metadata.date.is_some() { 20.0 } else { 0.0 }
                    + if !window.texture.message.is_empty() || window.texture.export.is_some() {
                        24.0
                    } else {
                        0.0
                    };
                texture_view::show(ui, p, &item, image, dimensions, &mut window.texture.display, actions, footer);
                if window.texture.display != before {
                    prefs.texture_display = window.texture.display;
                }
                if window.texture.display.actual_size || window.texture.export.is_some() {
                    scene.textures.note_usage_fixed(&asset, 16384.0);
                }
                scene.textures.note_usage(
                    &asset,
                    if window.texture.display.actual_size || window.texture.export.is_some() {
                        16384.0
                    } else {
                        ui.available_width().max(ui.available_height()) * ctx.pixels_per_point()
                    },
                );
            }
            if material {
                ui.horizontal(|ui| {
                    let label = aurora_i18n::tr!("texture-library-material-drag");
                    let width = widgets::action_button_fit(ui, "grip-vertical", label);
                    let path_width = (ui.available_width() - width - ui.spacing().item_spacing.x).max(0.0);
                    ui.add_sized(
                        [path_width, 24.0],
                        egui::Label::new(RichText::new(&path).size(11.0).color(p.muted)).truncate(),
                    )
                    .on_hover_text(&path);
                    // Only this handle starts a permission-aware inventory drag;
                    // the scene keeps its orbit gesture and independent camera.
                    let drag = widgets::action_button(ui, p, "grip-vertical", label, ActionKind::Flat, Vec2::new(width, 24.0))
                        .interact(egui::Sense::click_and_drag())
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text(aurora_i18n::tr!("texture-library-material-drag-hint"));
                    drag.dnd_set_drag_payload(InvDrag(item.id));
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new(("resource_material_handle", item.id)), drag.rect));
                });
                if let Some(editor) = &mut window.editor {
                    editor.show(ui, p, &item, world, images, actions);
                } else {
                    ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-image"));
                }
            } else {
                let description = window.description.get_or_insert_with(|| item.desc.clone());
                let response = ui.add_enabled(
                    material_editor::can_edit(world, &item),
                    egui::TextEdit::singleline(description)
                        .hint_text(aurora_i18n::tr!("common-description"))
                        .char_limit(127)
                        .desired_width(ui.available_width()),
                );
                if response.lost_focus() && *description != item.desc && material_editor::can_edit(world, &item) {
                    actions.push(Action::Change(Mutation {
                        operations: vec![Operation::Patch {
                            id: item.id,
                            folder: false,
                            body: aurora_llsd::llsd_map! { "desc" => description.clone() },
                        }],
                        refresh: vec![item.parent],
                        ..Default::default()
                    }));
                }
                if let Some(uploader) = window.texture.metadata.uploader {
                    world.social.want_name(uploader);
                    let name = world.person_name(&uploader).unwrap_or_else(|| uploader.to_string());
                    if ui
                        .small_button(aurora_i18n::tr!("resource-preview-uploader", name = name))
                        .clicked()
                    {
                        actions.push(Action::Profile(uploader));
                    }
                }
                if let Some(date) = &window.texture.metadata.date {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("resource-preview-upload-date", date = date.clone()))
                            .small()
                            .color(p.muted),
                    );
                }
                if window.texture.export.is_some() {
                    ui.label(aurora_i18n::tr!("resource-preview-export-loading"));
                }
                if !window.texture.message.is_empty() {
                    ui.label(&window.texture.message);
                }
                ui.add(egui::Label::new(RichText::new(&path).size(11.0).color(p.muted)).truncate())
                    .on_hover_text(&path);
                ui.horizontal(|ui| {
                    if ui.button(aurora_i18n::tr!("common-ok")).clicked() {
                        close_clicked = true;
                    }
                    if ui
                        .add_enabled(
                            !rules::library(&world.inventory, item.id) && !rules::in_type(&world.inventory, item.id, 14),
                            egui::Button::new(aurora_i18n::tr!("resource-preview-discard")),
                        )
                        .clicked()
                    {
                        actions.push(Action::Discard(item.id));
                    }
                });
            }
            ui.label(
                RichText::new(if item.owner_mask & rules::COPY != 0 {
                    aurora_i18n::tr!("texture-library-copy")
                } else {
                    aurora_i18n::tr!("texture-library-no-copy")
                })
                .size(11.0)
                .color(p.muted),
            );
        });
        open &= !close_clicked;
        if let Some(editor) = &mut window.editor {
            if editor.close_ready {
                return false;
            }
            if !open && (editor.dirty() || editor.saving.is_some()) {
                open = true;
                editor.confirm_close = true;
            }
            self.wanted_images.extend(editor.show_picker(ctx, p, world, images));
            if editor.confirm_close {
                widgets::Modal::new(egui::Id::new(("material_close", item.id))).show(ctx, |ui| {
                    ui.label(aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes"));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                material_editor::can_edit(world, &item) && editor.dirty() && editor.saving.is_none(),
                                egui::Button::new(aurora_i18n::tr!("common-save")),
                            )
                            .clicked()
                        {
                            editor.close_after_save = true;
                            editor.confirm_close = false;
                            actions.push(Action::SaveMaterial {
                                item: item.id,
                                material: (*editor.draft).clone(),
                                name: None,
                            });
                        }
                        if ui
                            .add_enabled(
                                editor.saving.is_none(),
                                egui::Button::new(aurora_i18n::tr!("ui-inventory-dialogs-discard-changes")),
                            )
                            .clicked()
                        {
                            open = false;
                        }
                        if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                            editor.confirm_close = false;
                        }
                    });
                });
            }
        }
        if let Some(size) = widgets::take_floater_resize(ctx, &id) {
            prefs.record_preview_size(material, size);
        }
        open
    }
}
