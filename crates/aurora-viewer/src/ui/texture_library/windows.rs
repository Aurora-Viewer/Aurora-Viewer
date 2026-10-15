//! Per-inventory-item windows, following LLFloaterReg::showInstance in
//! LLTextureBridgeAction::doIt and LLMaterialBridgeAction::doIt (Firestorm
//! indra/newview/llinventorybridge.cpp). The library and inventory share them.

use super::*;

pub(super) struct PreviewWindow {
    pub id: Uuid,
    pub view: aurora_render::preview::View,
    first_show: bool,
}

pub(crate) fn window_id(item: Uuid) -> String {
    format!("resource_preview_{item}")
}

impl LibraryUi {
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
            });
        }
        widgets::open_floater(ctx, &window_id(item.id), &mut existing);
        self.focus_preview = Some(item.id);
    }

    pub(super) fn show_previews(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        scene: &mut Scene,
        images: &HashMap<Uuid, egui::TextureHandle>,
        prefs: &mut Preferences,
    ) {
        // Keep one shared worker/GPU cache for every swatch and window.
        let mut windows = std::mem::take(&mut self.windows);
        windows.retain_mut(|window| self.preview_window(ctx, p, world, scene, images, prefs, window));
        self.windows = windows;
        if let Some(id) = self.focus_preview.take() {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(window_id(id))));
        }
    }

    fn preview_window(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        scene: &mut Scene,
        images: &HashMap<Uuid, egui::TextureHandle>,
        prefs: &mut Preferences,
        window: &mut PreviewWindow,
    ) -> bool {
        let Some(item) = catalogue::resource(&world.inventory, window.id) else {
            return false;
        };
        let material = item.asset_type == 57;
        let asset = catalogue::asset(item);
        let path = super::super::inventory::inventory_path(&world.inventory, item.parent);
        let id = window_id(item.id);
        // Human-requested defaults by resource type. Only new windows inherit
        // the last manual size; visible windows retain their own geometry.
        if window.first_show {
            if let Some(size) = prefs.preview_size(material) {
                widgets::size_floater(ctx, &id, size);
            }
            window.first_show = false;
        }
        let mut open = true;
        let size = prefs.preview_size(material).unwrap_or(Vec2::new(360.0, 420.0));
        let mut floater = Floater::new(&id, &item.name, ctx.content_rect().center() - size * 0.5, size)
            .cascade(widgets::CascadeGroup::inventory(item.asset_type));
        floater.min_size = Vec2::new(220.0, 260.0);
        floater.show(ctx, p, &mut open, |ui| {
            let side = ui.available_width().min((ui.available_height() - 58.0).max(96.0)).min(320.0);
            if material {
                if let Some(model) = self.previews.model(asset, scene, &mut self.wanted_images, ctx) {
                    window.view.background = previews::view(p).background;
                    let response = aurora_render::preview::paint(ui, ui.id().with(item.id), model, &mut window.view, Vec2::splat(side));
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new(("resource_material_scene", item.id)), response.rect));
                } else {
                    ui.allocate_space(Vec2::splat(side));
                }
            } else {
                self.wanted_images.insert(asset);
                if let Some(image) = images.get(&asset) {
                    let response = ui.add(
                        egui::Image::new(image)
                            .fit_to_exact_size(Vec2::splat(side))
                            .maintain_aspect_ratio(true)
                            .sense(egui::Sense::click_and_drag()),
                    );
                    response.dnd_set_drag_payload(InvDrag(item.id));
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new(("resource_texture_preview", item.id)), response.rect));
                    response.on_hover_text(aurora_i18n::tr!("texture-library-drag-hint"));
                } else {
                    ui.allocate_ui(Vec2::splat(side), |ui| {
                        ui.centered_and_justified(|ui| ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-image")));
                    });
                }
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
            } else {
                ui.add(egui::Label::new(RichText::new(&path).size(11.0).color(p.muted)).truncate())
                    .on_hover_text(&path);
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
        if let Some(size) = widgets::take_floater_resize(ctx, &id) {
            prefs.record_preview_size(material, size);
        }
        open
    }
}
