//! Stable material preview and four map cards visible together.
use super::*;

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    material: &mut PbrMaterial,
    images: &mut [Option<LocalImage>; 4],
    actions: &mut Vec<Action>,
    shape: &mut super::super::preview::MaterialShape,
    previews: &[Option<(Uuid, egui::TextureHandle)>; 4],
    prefs: &mut Preferences,
    model: Option<&Arc<aurora_render::preview::Model>>,
    view: &mut aurora_render::preview::View,
) {
    let preview_width = 230.0;
    let gap = ui.spacing().item_spacing.x;
    let maps_width = (ui.available_width() - preview_width - gap).max(420.0);
    let card_width = (maps_width - gap) * 0.5;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(preview_width, 330.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(preview_width);
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt(("material_shape", id))
                    .selected_text(shape.label())
                    .show_ui(ui, |ui| {
                        for value in super::super::preview::MaterialShape::ALL {
                            selection(ui, shape, value, value.label());
                        }
                    });
                if widgets::icon_button(ui, p, "zoom-in-filled", aurora_i18n::tr!("import-preview-enlarge"), true) {
                    actions.push(Action::Zoom(id, None));
                }
                let rect = egui::Rect::from_min_size(ui.next_widget_position(), Vec2::splat(20.0));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(egui::Id::new(("material_reset", id)), rect));
                if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("import-preview-reset-view"), true) {
                    view.reset_camera();
                }
                widgets::icon_menu(ui, p, "palette-filled", aurora_i18n::tr!("import-preview-colors"), |ui| {
                    preview_colors_fields(ui, p, prefs)
                });
            });
            let rect = if let Some(model) = model {
                aurora_render::preview::paint(
                    ui,
                    egui::Id::new(("material_preview", id)),
                    model.clone(),
                    view,
                    Vec2::splat(preview_width),
                )
                .rect
            } else {
                widgets::picture(ui, p, None, Vec2::splat(preview_width), "cube-3d-sphere", "").rect
            };
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new(("material_canvas", id)), rect));
            camera_hint(ui, p);
            checkbox(ui, p, &mut material.double_sided, aurora_i18n::tr!("import-double-sided"));
        });
        ui.allocate_ui_with_layout(Vec2::new(maps_width, 330.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(maps_width);
            for pair in [[0, 2], [3, 1]] {
                ui.horizontal_top(|ui| {
                    for slot in pair {
                        ui.push_id(slot, |ui| {
                            let card = ui
                                .allocate_ui_with_layout(Vec2::new(card_width, 182.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                    widgets::surface(ui, p, |ui| {
                                        ui.set_min_size(Vec2::new(card_width - 22.0, 160.0));
                                        ui.set_max_width(card_width - 22.0);
                                        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                                        map_card(ui, p, id, slot, material, images, actions, previews);
                                    });
                                })
                                .response
                                .rect;
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(egui::Id::new(("material_card", id, slot)), card));
                        });
                    }
                });
            }
        });
    });
}

fn map_card(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    slot: usize,
    material: &mut PbrMaterial,
    images: &mut [Option<LocalImage>; 4],
    actions: &mut Vec<Action>,
    previews: &[Option<(Uuid, egui::TextureHandle)>; 4],
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(map_label(slot)).small().strong());
        if widgets::icon_button(ui, p, "folder-open-filled", aurora_i18n::tr!("import-browse"), true) {
            actions.push(Action::MapFile(id, slot));
        }
        if widgets::icon_button(ui, p, "archive-filled", aurora_i18n::tr!("common-inventory"), true) {
            actions.push(Action::MapInventory(id, slot));
        }
        if widgets::icon_button(
            ui,
            p,
            "x",
            aurora_i18n::tr!("common-remove"),
            images[slot].is_some() || maps::ids(material)[slot].is_some(),
        ) {
            images[slot] = None;
            match slot {
                0 => material.base_color_texture = None,
                1 => material.normal_texture = None,
                2 => material.metallic_roughness_texture = None,
                _ => material.emissive_texture = None,
            }
        }
    });
    let name = images[slot].as_ref().map(|image| image.name.as_str()).unwrap_or("");
    ui.add_sized(
        Vec2::new(ui.available_width(), 14.0),
        egui::Label::new(RichText::new(name).small().color(p.muted)).truncate(),
    )
    .on_hover_text(name);
    let parameters_width = (ui.available_width() - 88.0 - ui.spacing().item_spacing.x).max(100.0);
    ui.horizontal_top(|ui| {
        let texture = previews[slot]
            .as_ref()
            .filter(|(key, _)| images[slot].as_ref().is_some_and(|image| image.key == *key))
            .map(|(_, texture)| texture);
        let picture = widgets::picture(ui, p, texture, Vec2::splat(88.0), "photo-filled", aurora_i18n::tr!("import-no-map"));
        if texture.is_some() {
            if let Some(icon) = crate::ui::icons::global("zoom-in-filled") {
                ui.put(
                    egui::Rect::from_min_size(picture.rect.right_bottom() - Vec2::splat(24.0), Vec2::splat(20.0)),
                    egui::Image::new(&icon)
                        .fit_to_exact_size(Vec2::splat(18.0))
                        .tint(p.ink)
                        .sense(egui::Sense::hover()),
                );
            }
            if picture.clicked() {
                actions.push(Action::Zoom(id, Some(slot)));
            }
        }
        ui.allocate_ui_with_layout(Vec2::new(parameters_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(parameters_width);
            match slot {
                0 => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("build-ui-texture-tint")).small());
                        factor_swatch(ui, p, "base", &mut material.base_color_factor, true);
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("common-alpha")).small());
                        ui.add(
                            egui::DragValue::new(&mut material.base_color_factor[3])
                                .range(0.0..=1.0)
                                .speed(0.01),
                        );
                    });
                    egui::ComboBox::from_id_salt("alpha")
                        .width(parameters_width - 8.0)
                        .selected_text(alpha_name(material.alpha_mode))
                        .show_ui(ui, |ui| {
                            for mode in [
                                aurora_assets::AlphaMode::Opaque,
                                aurora_assets::AlphaMode::Blend,
                                aurora_assets::AlphaMode::Mask,
                            ] {
                                selection(ui, &mut material.alpha_mode, mode, alpha_name(mode));
                            }
                        });
                    if material.alpha_mode == aurora_assets::AlphaMode::Mask {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(aurora_i18n::tr!("import-alpha-cutoff-short")).small());
                            ui.add(egui::DragValue::new(&mut material.alpha_cutoff).range(0.0..=1.0).speed(0.01))
                                .on_hover_text(aurora_i18n::tr!("import-alpha-cutoff"));
                        });
                    }
                }
                2 => {
                    ui.label(RichText::new(aurora_i18n::tr!("import-metallic")).small());
                    ui.add(egui::DragValue::new(&mut material.metallic_factor).range(0.0..=1.0).speed(0.01));
                    ui.label(RichText::new(aurora_i18n::tr!("import-roughness")).small());
                    ui.add(egui::DragValue::new(&mut material.roughness_factor).range(0.0..=1.0).speed(0.01));
                }
                3 => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("build-ui-texture-tint")).small());
                        factor_swatch(ui, p, "emission", &mut material.emissive_factor, false);
                    });
                }
                _ => {}
            }
        });
    });
}
