//! Compact material preview and unframed map rows; Multiple keeps its grid.
use super::*;

const MAP_PREVIEW_SIZE: f32 = 56.0 * 1.7;

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
    stacked: bool,
) {
    if stacked {
        preview(ui, p, id, material, actions, shape, prefs, model, view, true);
        ui.add_space(8.0);
        for slot in [0, 2, 3, 1] {
            let card = ui
                .push_id(slot, |ui| {
                    ui.scope(|ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                        clean_map(ui, p, id, slot, material, images, actions, previews);
                    })
                    .response
                    .rect
                })
                .inner;
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::unique(("material_card", id, slot)), card));
        }
    } else {
        let preview_width = 230.0;
        let gap = ui.spacing().item_spacing.x;
        let maps_width = (ui.available_width() - preview_width - gap).max(420.0);
        let card_width = (maps_width - gap) * 0.5;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(preview_width, 330.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                preview(ui, p, id, material, actions, shape, prefs, model, view, false);
            });
            ui.allocate_ui_with_layout(Vec2::new(maps_width, 330.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(maps_width);
                for pair in [[0, 2], [3, 1]] {
                    ui.horizontal_top(|ui| {
                        for slot in pair {
                            ui.push_id(slot, |ui| {
                                let card = ui
                                    .allocate_ui_with_layout(Vec2::new(card_width, 204.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                        widgets::surface(ui, p, |ui| {
                                            ui.set_min_size(Vec2::new(card_width - 22.0, 182.0));
                                            ui.set_max_width(card_width - 22.0);
                                            ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                                            map_card(ui, p, id, slot, material, images, actions, previews, false);
                                        });
                                    })
                                    .response
                                    .rect;
                                ui.ctx()
                                    .data_mut(|data| data.insert_temp(egui::Id::unique(("material_card", id, slot)), card));
                            });
                        }
                    });
                }
            });
        });
    }
}

fn preview(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    material: &mut PbrMaterial,
    actions: &mut Vec<Action>,
    shape: &mut super::super::preview::MaterialShape,
    prefs: &mut Preferences,
    model: Option<&Arc<aurora_render::preview::Model>>,
    view: &mut aurora_render::preview::View,
    compact: bool,
) {
    if compact {
        ui.horizontal_wrapped(|ui| {
            preview_tools(ui, p, id, actions, shape, prefs, view);
            checkbox(ui, p, &mut material.double_sided, aurora_i18n::tr!("import-double-sided"));
        });
        apply_preview_colors(view, p, prefs);
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - 176.0) * 0.5).max(0.0));
            let response = if let Some(model) = model {
                aurora_render::preview::paint(
                    ui,
                    ui.scope_id().with(("material_preview", id)),
                    model.clone(),
                    view,
                    Vec2::splat(176.0),
                )
            } else {
                widgets::picture(ui, p, None, Vec2::splat(176.0), "cube-3d-sphere", "")
            }
            .on_hover_text(aurora_i18n::tr!("import-preview-gestures"));
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::unique(("material_canvas", id)), response.rect));
        });
        return;
    }
    preview_tools(ui, p, id, actions, shape, prefs, view);
    apply_preview_colors(view, p, prefs);
    let rect = ui
        .horizontal(|ui| {
            ui.add_space(((ui.available_width() - 230.0) * 0.5).max(0.0));
            if let Some(model) = model {
                aurora_render::preview::paint(
                    ui,
                    ui.scope_id().with(("material_preview", id)),
                    model.clone(),
                    view,
                    Vec2::splat(230.0),
                )
                .rect
            } else {
                widgets::picture(ui, p, None, Vec2::splat(230.0), "cube-3d-sphere", "").rect
            }
        })
        .inner;
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique(("material_canvas", id)), rect));
    camera_hint(ui, p);
    checkbox(ui, p, &mut material.double_sided, aurora_i18n::tr!("import-double-sided"));
}

fn preview_tools(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    actions: &mut Vec<Action>,
    shape: &mut super::super::preview::MaterialShape,
    prefs: &mut Preferences,
    view: &mut aurora_render::preview::View,
) {
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
            .data_mut(|data| data.insert_temp(egui::Id::unique(("material_reset", id)), rect));
        if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("import-preview-reset-view"), true) {
            view.reset_camera();
        }
        preview_colors_button(ui, p, id, prefs);
    });
}

fn clean_map(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    slot: usize,
    material: &mut PbrMaterial,
    images: &mut [Option<LocalImage>; 4],
    actions: &mut Vec<Action>,
    previews: &[Option<(Uuid, egui::TextureHandle)>; 4],
) {
    let width = (ui.available_width() - MAP_PREVIEW_SIZE - ui.spacing().item_spacing.x).max(240.0);
    ui.horizontal_top(|ui| {
        let texture = previews[slot]
            .as_ref()
            .filter(|(key, _)| images[slot].as_ref().is_some_and(|image| image.key == *key))
            .map(|(_, texture)| texture);
        let picture = widgets::picture(ui, p, texture, Vec2::splat(MAP_PREVIEW_SIZE), "photo-filled", "");
        if texture.is_some() && picture.clicked() {
            actions.push(Action::Zoom(id, Some(slot)));
        }
        ui.allocate_ui_with_layout(Vec2::new(width, MAP_PREVIEW_SIZE), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(width);
            ui.spacing_mut().interact_size.y = 20.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let title_width = (ui.available_width() - 72.0).max(40.0);
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
                    maps::set(material, slot, None);
                }
            });
            let name = images[slot]
                .as_ref()
                .map(|image| image.name.as_str())
                .unwrap_or_else(|| aurora_i18n::tr!("import-no-map"));
            ui.add(egui::Label::new(RichText::new(name).small().color(p.muted)).truncate())
                .on_hover_text(name);
            ui.horizontal_wrapped(|ui| match slot {
                0 => {
                    ui.label(aurora_i18n::tr!("build-ui-texture-tint"));
                    factor_swatch(ui, p, "base", &mut material.base_color_factor, true);
                    ui.label(aurora_i18n::tr!("common-alpha"));
                    ui.add_sized(
                        Vec2::new(48.0, 24.0),
                        egui::DragValue::new(&mut material.base_color_factor[3])
                            .range(0.0..=1.0)
                            .speed(0.01),
                    );
                }
                2 => {
                    ui.label(aurora_i18n::tr!("import-metallic"));
                    ui.add_sized(
                        Vec2::new(48.0, 24.0),
                        egui::DragValue::new(&mut material.metallic_factor).range(0.0..=1.0).speed(0.01),
                    );
                    ui.label(aurora_i18n::tr!("import-roughness"));
                    ui.add_sized(
                        Vec2::new(48.0, 24.0),
                        egui::DragValue::new(&mut material.roughness_factor).range(0.0..=1.0).speed(0.01),
                    );
                }
                3 => {
                    ui.label(aurora_i18n::tr!("build-ui-texture-tint"));
                    factor_swatch(ui, p, "emission", &mut material.emissive_factor, false);
                }
                _ => {}
            });
            if slot == 0 {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("alpha")
                        .width(120.0)
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
                        ui.label(aurora_i18n::tr!("import-alpha-cutoff-short"));
                        ui.add_sized(
                            Vec2::new(48.0, 24.0),
                            egui::DragValue::new(&mut material.alpha_cutoff).range(0.0..=1.0).speed(0.01),
                        )
                        .on_hover_text(aurora_i18n::tr!("import-alpha-cutoff"));
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
    compact: bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = 20.0;
        let title_width = (ui.available_width() - 72.0).max(40.0);
        ui.allocate_ui_with_layout(
            Vec2::new(title_width, 20.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(Vec2::new(title_width, 20.0));
                ui.add(egui::Label::new(RichText::new(map_label(slot)).small().strong()).truncate())
            },
        )
        .inner
        .on_hover_text(map_label(slot));
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
    let caption_size = Vec2::new(ui.available_width(), 14.0);
    ui.allocate_ui_with_layout(caption_size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(caption_size);
        ui.add(egui::Label::new(RichText::new(name).small().color(p.muted)).truncate())
    })
    .inner
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
        ui.allocate_ui_with_layout(
            Vec2::new(parameters_width, if compact { 88.0 } else { 128.0 }),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(parameters_width);
                ui.spacing_mut().button_padding.x = 6.0;
                egui::Grid::new("map_parameters")
                    .num_columns(2)
                    .min_col_width(0.0)
                    .spacing(Vec2::new(4.0, 6.0))
                    .show(ui, |ui| match slot {
                        0 => {
                            parameter_label(ui, aurora_i18n::tr!("build-ui-texture-tint"));
                            factor_swatch(ui, p, "base", &mut material.base_color_factor, true);
                            ui.end_row();
                            parameter_value(ui, aurora_i18n::tr!("common-alpha"), &mut material.base_color_factor[3]);
                        }
                        2 => {
                            parameter_value(ui, aurora_i18n::tr!("import-metallic"), &mut material.metallic_factor);
                            parameter_value(ui, aurora_i18n::tr!("import-roughness"), &mut material.roughness_factor);
                        }
                        3 => {
                            parameter_label(ui, aurora_i18n::tr!("build-ui-texture-tint"));
                            factor_swatch(ui, p, "emission", &mut material.emissive_factor, false);
                            ui.end_row();
                        }
                        _ => {}
                    });
                if slot == 0 {
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
                        egui::Grid::new("map_cutoff")
                            .num_columns(2)
                            .min_col_width(0.0)
                            .spacing(Vec2::new(4.0, 6.0))
                            .show(ui, |ui| {
                                parameter_value(ui, aurora_i18n::tr!("import-alpha-cutoff-short"), &mut material.alpha_cutoff);
                            })
                            .response
                            .on_hover_text(aurora_i18n::tr!("import-alpha-cutoff"));
                    }
                }
            },
        );
    });
}

fn parameter_label(ui: &mut egui::Ui, label: &str) {
    ui.allocate_ui_with_layout(Vec2::new(50.0, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(Vec2::new(50.0, 26.0));
        ui.add(egui::Label::new(RichText::new(label).small()).truncate())
    })
    .inner
    .on_hover_text(label);
}

fn parameter_value(ui: &mut egui::Ui, label: &str, value: &mut f32) {
    parameter_label(ui, label);
    ui.add_sized(Vec2::new(44.0, 26.0), egui::DragValue::new(value).range(0.0..=1.0).speed(0.01));
    ui.end_row();
}
