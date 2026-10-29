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

pub(super) struct BatchExport {
    item: Uuid,
    asset: Uuid,
    view: texture_view::ImageView,
}

pub(crate) fn window_id(item: Uuid) -> String {
    format!("resource_preview_{item}")
}

impl LibraryUi {
    /// LLTextureBridge::save_as / LLInventoryAction::saveMultipleTextures.
    /// Every image still passes the ordinary full-resolution export path.
    pub(crate) fn begin_exports(&mut self, ctx: &egui::Context, world: &World, items: Vec<Uuid>, png: bool) {
        let full = rules::COPY | rules::MODIFY | rules::TRANSFER;
        let mut seen = HashSet::new();
        let items: Vec<_> = items
            .into_iter()
            .filter(|id| seen.insert(*id))
            .filter(|id| {
                world
                    .inventory
                    .items
                    .get(id)
                    .is_some_and(|item| matches!(item.asset_type, 0 | 15) && item.owner_mask & full == full)
            })
            .collect();
        if items.len() == 1 {
            self.open_preview(ctx, world, items[0]);
            self.export_texture(items[0], png);
        } else if !items.is_empty() && self.export_folder.is_none() {
            if self.export_jobs.is_empty() && self.export_queue.is_empty() {
                self.export_error = None;
            }
            self.export_items = items.into_iter().map(|id| (id, png)).collect();
            let (sender, receiver) = crossbeam_channel::bounded(1);
            self.export_folder = Some(receiver);
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let folder = rfd::FileDialog::new()
                    .set_title(aurora_i18n::tr!("inventory-save-selected-textures"))
                    .pick_folder();
                let _ = sender.send(folder);
                wake.request_repaint();
            });
        }
    }

    fn update_exports(&mut self, ctx: &egui::Context, world: &World, scene: &mut Scene, actions: &mut Vec<Action>) {
        if let Some(folder) = self.export_folder.as_ref().and_then(|receiver| receiver.try_recv().ok()) {
            self.export_folder = None;
            let items = std::mem::take(&mut self.export_items);
            if let Some(folder) = folder {
                self.export_queue
                    .extend(items.into_iter().map(|(id, png)| (id, folder.clone(), png)));
            }
        }
        let full = rules::COPY | rules::MODIFY | rules::TRANSFER;
        let mut completed = false;
        // Bounded decoding and IO; a large selection does not open hundreds
        // of preview windows or retain all full-resolution images together.
        self.export_jobs.retain_mut(|job| {
            if world
                .inventory
                .items
                .get(&job.item)
                .is_none_or(|item| item.asset_id != job.asset || item.owner_mask & full != full)
            {
                return false;
            }
            self.wanted_images.insert(job.asset);
            scene.textures.note_usage_fixed(&job.asset, 16384.0);
            job.view.prepare(ctx, job.asset, scene);
            if job.view.export_finished() {
                self.message = job.view.message.clone();
                completed = true;
                if job.view.message != aurora_i18n::tr!("resource-preview-exported") && self.export_error.is_none() {
                    self.export_error = Some(job.view.message.clone());
                }
                false
            } else {
                true
            }
        });
        while self.export_jobs.len() < 3 {
            let Some((id, folder, png)) = self.export_queue.pop_front() else {
                break;
            };
            let Some(item) = world.inventory.items.get(&id).filter(|item| item.owner_mask & full == full) else {
                continue;
            };
            let name: String = item
                .name
                .chars()
                .map(|character| {
                    if character.is_control() || "<>:\"/\\|?*".contains(character) {
                        '_'
                    } else {
                        character
                    }
                })
                .take(100)
                .collect();
            let extension = if png { "png" } else { "tga" };
            let filename = format!("{}-{id}.{extension}", name.trim().trim_end_matches('.'));
            self.export_jobs.push(BatchExport {
                item: id,
                asset: item.asset_id,
                view: texture_view::ImageView::batch_export(folder.join(filename), png),
            });
        }
        if completed && self.export_jobs.is_empty() && self.export_queue.is_empty() && self.export_folder.is_none() {
            actions.push(Action::ExportResult(
                self.export_error.take().unwrap_or_else(|| self.message.clone()),
            ));
        }
    }
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
    pub(crate) fn demo_preview_controls(&mut self, mode: &str) {
        for window in &mut self.windows {
            window.texture.display.keep_ratio = !mode.ends_with("stretch");
            if let Some(editor) = &mut window.editor {
                editor.shape = if mode.ends_with("cube") {
                    crate::imports::preview::MaterialShape::Cube
                } else if mode.ends_with("aurora") {
                    crate::imports::preview::MaterialShape::Aurora
                } else {
                    crate::imports::preview::MaterialShape::Sphere
                };
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
        if world
            .inventory
            .items
            .get(&id)
            .is_some_and(|it| !world.inventory.can_preview(it.asset_type))
        {
            return;
        }
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
        self.update_exports(ctx, world, scene, actions);
        let mut windows = std::mem::take(&mut self.windows);
        windows.retain_mut(|window| self.preview_window(ctx, p, world, scene, images, prefs, window, actions));
        self.windows = windows;
        self.map_uploads.retain(|_, (item, _)| self.windows.iter().any(|w| w.id == *item));
        if let Some(id) = self.focus_preview.take() {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(window_id(id))));
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
        if !world.inventory.can_preview(item.asset_type) {
            return true;
        }
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
        floater.steady_size = material;
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
            } else {
                self.wanted_images.insert(asset);
                if item.owner_mask & (rules::COPY | rules::MODIFY | rules::TRANSFER) != rules::COPY | rules::MODIFY | rules::TRANSFER {
                    window.texture.export = None;
                    window.texture.export_path = None;
                }
                window.texture.prepare(ctx, asset, scene);
                let image = window.texture.image.as_ref().or_else(|| images.get(&asset));
                let dimensions = scene.textures.full_size(&asset).or(window.texture.dimensions);
                let before = window.texture.display;
                texture_view::toolbar(ui, p, &item, dimensions, &mut window.texture.display, actions);
                // the picture in exactly the room the description, the
                // buttons and the notes leave
                let (mut view, foot) = widgets::above_footer(ui);
                texture_view::picture(&mut view, p, image, dimensions, &window.texture.display, Some(item.id));
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
                        view.available_width().max(view.available_height()) * ctx.pixels_per_point()
                    },
                );
                foot.show(ui, |ui| {
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
            }
            if material {
                if let Some(editor) = &mut window.editor {
                    let model =
                        self.previews
                            .material_model(item.id, editor.draft.clone(), editor.shape, scene, &mut self.wanted_images, ctx);
                    let thumbnails = self.previews.thumbnails(item.id);
                    editor.show(ui, p, &item, world, images, &thumbnails, model, &mut window.view, &path, actions);
                } else {
                    ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-image"));
                }
            }
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
                widgets::Modal::new(egui::Id::unique(("material_close", item.id))).show(ctx, |ui| {
                    ui.label(aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes"));
                    ui.horizontal(|ui| {
                        if widgets::default_button(ui.add_enabled(
                            material_editor::can_edit(world, &item) && editor.dirty() && editor.saving.is_none(),
                            egui::Button::new(aurora_i18n::tr!("common-save")),
                        ))
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
