//! Legacy Viewer Avatar and Journal panel layout, using real import diagnostics.
//! Reference: rigging_panel / logs_panel in indra/newview/skins/default/xui/en/floater_model_preview.xml.
use super::*;

pub(super) fn avatar_controls(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &mut mesh::Draft) {
    let top = ui.cursor().top();
    let mut source_lod = draft.preview_lod.min(3);
    while source_lod < 3 && draft.lods[source_lod].source == mesh::LodSource::Above {
        source_lod += 1;
    }
    let avatar = if draft.lods[source_lod].source == mesh::LodSource::File {
        draft
            .inspector
            .lod_avatars
            .get(&source_lod)
            .unwrap_or(&draft.inspector.avatar)
            .clone()
    } else {
        draft.inspector.avatar.clone()
    };
    if !avatar.valid() {
        draft.options.skin = false;
    }
    let previous_skin = draft.options.skin;
    let previous_joints = draft.options.joints;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(215.0, 88.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(215.0);
            ui.add_enabled_ui(avatar.valid(), |ui| {
                checkbox(ui, p, &mut draft.options.skin, aurora_i18n::tr!("import-include-weights"));
            });
            ui.add_enabled_ui(draft.options.skin, |ui| {
                checkbox(ui, p, &mut draft.options.joints, aurora_i18n::tr!("import-include-joints"));
            });
            ui.add_enabled_ui(draft.options.skin && draft.options.joints, |ui| {
                checkbox(ui, p, &mut draft.options.lock_scale, aurora_i18n::tr!("import-lock-scale"));
            });
        });
        ui.vertical(|ui| {
            ui.label(aurora_i18n::tr!("import-pelvis-offset"));
            ui.add_sized(
                Vec2::new(80.0, 26.0),
                egui::DragValue::new(&mut draft.options.pelvis)
                    .range(-3.0..=3.0)
                    .speed(0.001)
                    .fixed_decimals(3),
            );
            if avatar.too_many {
                ui.label(RichText::new(aurora_i18n::tr!("import-avatar-too-many")).color(p.warn));
            }
            if avatar.unknown {
                ui.label(RichText::new(aurora_i18n::tr!("import-avatar-unknown")).color(p.warn));
            }
        });
    });
    if previous_skin != draft.options.skin {
        draft.preview_weights = draft.options.skin;
    }
    if previous_joints != draft.options.joints && draft.preview_weights {
        draft.show_overrides = draft.options.joints;
    }
    if !draft.options.skin {
        draft.preview_weights = false;
        draft.options.joints = false;
        draft.show_joints = false;
        draft.show_overrides = false;
    }
    if !draft.options.joints {
        draft.options.lock_scale = false;
    }
    let height = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(egui::Id::unique(("mesh_content_height", id))))
        .unwrap_or(280.0);
    let list_height = (height - (ui.cursor().top() - top) - 64.0).max(60.0);
    let mut selected = draft.inspector.selected_joint.clone();
    if !avatar.joints.contains_key(&selected) {
        selected = avatar.joints.keys().next().cloned().unwrap_or_else(|| "mPelvis".into());
    }
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(215.0, list_height + 45.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(215.0);
                ui.label(aurora_i18n::tr!("import-avatar-joints"));
                egui::ScrollArea::vertical()
                    .id_salt(("import_joint_list", id))
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .min_scrolled_height(list_height)
                    .show(ui, |ui| {
                        if draft.options.skin {
                            for (joint, models) in &avatar.joints {
                                let conflict = draft.options.joints && mesh::inspector::Avatar::conflicts(models);
                                let label = RichText::new(joint).color(if conflict { p.warn } else { p.ink });
                                if ui.selectable_label(selected == *joint, label).clicked() {
                                    selected = joint.clone();
                                }
                            }
                        }
                    });
                let count = if draft.options.skin { avatar.joints.len() } else { 0 };
                let conflicts = if draft.options.skin && draft.options.joints {
                    avatar
                        .joints
                        .values()
                        .filter(|models| mesh::inspector::Avatar::conflicts(models))
                        .count()
                } else {
                    0
                };
                ui.label(RichText::new(aurora_i18n::tr!("import-avatar-conflicts", count = conflicts, joints = count)).color(p.muted));
            },
        );
        ui.vertical(|ui| {
            ui.label(aurora_i18n::tr!("import-avatar-positions", joint = selected.as_str()));
            let table_width = (ui.available_width() - 18.0).max(200.0);
            egui::ScrollArea::both()
                .id_salt(("import_joint_positions", id))
                .max_height(list_height)
                .auto_shrink([false, false])
                .min_scrolled_height(list_height)
                .show(ui, |ui| {
                    egui::Grid::new(("import_joint_models", id))
                        .striped(true)
                        .min_col_width(0.0)
                        .spacing(Vec2::new(6.0, 4.0))
                        .show(ui, |ui| {
                            table_cell(ui, table_width * 0.49, aurora_i18n::tr!("import-avatar-model"));
                            for axis in ["X", "Y", "Z"] {
                                table_cell(ui, table_width * 0.17, axis);
                            }
                            ui.end_row();
                            if draft.options.skin
                                && let Some(models) = avatar.joints.get(&selected)
                            {
                                for (name, position) in models {
                                    table_cell(ui, table_width * 0.49, name);
                                    for axis in 0..3 {
                                        if draft.options.joints
                                            && let Some(position) = position
                                        {
                                            table_cell(ui, table_width * 0.17, &format!("{:.3}", position[axis]));
                                        } else {
                                            table_cell(ui, table_width * 0.17, "–");
                                        }
                                    }
                                    ui.end_row();
                                }
                            }
                        });
                });
        });
    });
    draft.inspector.selected_joint = selected;
}

pub(super) fn journal_controls(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &mut mesh::Draft) {
    checkbox(ui, p, &mut draft.inspector.verbose, aurora_i18n::tr!("import-journal-verbose"));
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique("import_mesh_detailed_logging"), draft.inspector.verbose));
    let height = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(egui::Id::unique(("mesh_content_height", id))))
        .unwrap_or(280.0);
    egui::ScrollArea::vertical()
        .id_salt(("import_journal", id))
        .stick_to_bottom(true)
        .max_height((height - 32.0).max(60.0))
        .show(ui, |ui| {
            let mut text = draft.inspector.log.as_str();
            ui.add_sized(
                Vec2::new(ui.available_width(), (height - 48.0).max(80.0)),
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Body)
                    .desired_width(f32::INFINITY)
                    .desired_rows(8),
            );
        });
}

fn table_cell(ui: &mut egui::Ui, width: f32, text: &str) {
    ui.add_sized(Vec2::new(width, 24.0), egui::Label::new(text).truncate())
        .on_hover_text(text);
}
