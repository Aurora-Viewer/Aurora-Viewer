//! Mesh import inspector. Firestorm's model preview options are grouped by
//! purpose; preview controls remain beside the model, separate from upload data.
//! Reference: LLFloaterModelPreview / LLModelPreview (indra/newview,
//! llfloatermodelpreview.cpp and llmodelpreview.cpp, originally LGPL 2.1).

use super::*;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, entry: &mut Entry, prefs: &mut Preferences, actions: &mut Vec<Action>, height: f32) {
    let Some(Source::Mesh(draft)) = &mut entry.source else { return };
    let old_preview = (
        draft.preview_lod,
        draft.preview_physics,
        draft.show_textures,
        draft.preview_weights,
        draft.show_joints,
        draft.show_overrides,
        draft.explode,
    );
    let preview_width = (ui.available_width() * 0.4).clamp(260.0, 340.0);
    let settings_width = ui.available_width() - preview_width - ui.spacing().item_spacing.x - 12.0;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(preview_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(preview_width);
            widgets::section_header(ui, p, "eye", aurora_i18n::tr!("import-preview"));
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("import-preview-lod"));
                egui::ComboBox::from_id_salt("mesh_preview_lod")
                    .width(140.0)
                    .selected_text(lod_label(draft.preview_lod))
                    .show_ui(ui, |ui| {
                        for level in (0..4).rev() {
                            ui.selectable_value(&mut draft.preview_lod, level, lod_label(level));
                        }
                    });
            });
            if entry.local {
                ui.label(aurora_i18n::tr!("import-local-mesh-instance"));
                egui::ComboBox::from_id_salt("local_mesh_instance")
                    .width(preview_width - 20.0)
                    .selected_text(
                        draft
                            .scene
                            .instances
                            .get(entry.local_model)
                            .map(|i| i.name.as_str())
                            .unwrap_or(aurora_i18n::tr!("import-mesh")),
                    )
                    .show_ui(ui, |ui| {
                        for (index, instance) in draft.scene.instances.iter().enumerate() {
                            ui.selectable_value(&mut entry.local_model, index, &instance.name);
                        }
                    });
                ui.checkbox(&mut entry.local_use_scale, aurora_i18n::tr!("import-local-mesh-scale"));
            }
            entry.view.material = draft.preview_physics.then(|| aurora_render::preview::Material {
                color: linear_color(prefs.preview_physics_fill.unwrap_or_else(|| palette_color(p.teal))),
                alpha_mode: if prefs.preview_physics_fill.is_some_and(|c| c[3] < 1.0) {
                    1
                } else {
                    0
                },
                ..Default::default()
            });
            if draft.preview_physics {
                entry.view.edge_color = prefs.preview_physics_edges.unwrap_or_else(|| palette_color(p.ink));
            }
            if let Some(model) = &entry.model {
                aurora_render::preview::paint(
                    ui,
                    egui::Id::new(("mesh_preview", entry.id)),
                    model.clone(),
                    &mut entry.view,
                    Vec2::new(preview_width, 245.0),
                );
            }
            ui.label(RichText::new(aurora_i18n::tr!("import-preview-camera-help")).small().color(p.muted));
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut entry.view.edges, aurora_i18n::tr!("import-edges"));
                ui.checkbox(&mut draft.show_textures, aurora_i18n::tr!("import-textures"));
                ui.checkbox(&mut draft.preview_physics, aurora_i18n::tr!("import-physics-preview"))
                    .on_hover_text(aurora_i18n::tr!("import-physics-preview-help"));
            });
            ui.collapsing(aurora_i18n::tr!("import-preview-tools"), |ui| {
                ui.checkbox(&mut entry.view.uv, aurora_i18n::tr!("import-uv-guide"));
                let skinned = draft.scene.models.iter().any(|m| m.skin.is_some());
                ui.add_enabled_ui(skinned, |ui| {
                    ui.checkbox(&mut draft.preview_weights, aurora_i18n::tr!("import-preview-weights"));
                    ui.checkbox(&mut draft.show_joints, aurora_i18n::tr!("import-preview-joints"));
                    ui.checkbox(&mut draft.show_overrides, aurora_i18n::tr!("import-preview-overrides"));
                });
                if draft.preview_physics {
                    ui.add(egui::Slider::new(&mut draft.explode, 0.0..=3.0).text(aurora_i18n::tr!("import-explode-hulls")));
                }
            });
            preview_colors(ui, p, prefs);
        });
        ui.add_space(12.0);
        ui.allocate_ui_with_layout(
            Vec2::new(settings_width, (height - 115.0).max(200.0)),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(settings_width);
                egui::ScrollArea::vertical()
                    .id_salt(("mesh_settings", entry.id))
                    .max_height((height - 115.0).max(200.0))
                    .show(ui, |ui| {
                        if mesh_controls(ui, p, entry.id, draft, entry.local, actions) {
                            entry.preview_generation += 1;
                            entry.model = None;
                        }
                        let section = ui
                            .ctx()
                            .data_mut(|d| d.get_temp::<usize>(egui::Id::new(("mesh_section", entry.id))))
                            .unwrap_or(0);
                        if section == 2 {
                            load_settings(ui, p, prefs);
                        }
                    });
            },
        );
    });
    if old_preview
        != (
            draft.preview_lod,
            draft.preview_physics,
            draft.show_textures,
            draft.preview_weights,
            draft.show_joints,
            draft.show_overrides,
            draft.explode,
        )
    {
        entry.model = None;
        entry.preview_generation += 1;
    }
}

fn lod_source(source: mesh::LodSource) -> &'static str {
    match source {
        mesh::LodSource::File => aurora_i18n::tr!("import-source-file"),
        mesh::LodSource::Auto => aurora_i18n::tr!("import-lod-auto"),
        mesh::LodSource::Combine => aurora_i18n::tr!("import-lod-combine"),
        mesh::LodSource::Fast => aurora_i18n::tr!("import-lod-fast"),
        mesh::LodSource::Reliable => aurora_i18n::tr!("import-lod-reliable"),
        mesh::LodSource::Above => aurora_i18n::tr!("import-lod-above"),
    }
}
fn mesh_controls(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &mut mesh::Draft, _local: bool, actions: &mut Vec<Action>) -> bool {
    let old = draft.options.clone();
    let mut changed = false;
    widgets::section_header(ui, p, "cube", aurora_i18n::tr!("import-mesh"));
    let triangles = draft.scene.models.iter().map(|m| m.triangles()).sum::<usize>();
    if let Some(prepared) = &draft.prepared
        && prepared.degenerate_triangles > 0
    {
        ui.label(RichText::new(aurora_i18n::tr!("import-degenerate-physics", count = prepared.degenerate_triangles)).color(p.warn));
    }
    ui.label(aurora_i18n::tr!(
        "import-mesh-info",
        models = draft.scene.models.len(),
        instances = draft.scene.instances.len(),
        triangles = triangles
    ));
    if !draft.scene.warnings.is_empty() {
        ui.collapsing(aurora_i18n::tr!("import-log"), |ui| {
            for warning in &draft.scene.warnings {
                ui.label(RichText::new(warning).color(p.warn));
            }
        });
    }
    let section_id = egui::Id::new(("mesh_section", id));
    let mut section = ui
        .ctx()
        .data_mut(|d| d.get_temp::<usize>(section_id))
        .unwrap_or(usize::from(draft.preview_physics))
        .min(2);
    ui.horizontal(|ui| {
        for (index, label) in [
            aurora_i18n::tr!("import-mesh-section-lods"),
            aurora_i18n::tr!("import-mesh-section-physics"),
            aurora_i18n::tr!("import-mesh-section-upload"),
        ]
        .into_iter()
        .enumerate()
        {
            if widgets::underlined_button(ui, p, label, section == index).clicked() {
                section = index;
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(section_id, section));
    if draft.pending.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            let progress = draft.progress.load(std::sync::atomic::Ordering::Relaxed);
            if progress > 0 {
                ui.label(aurora_i18n::tr!("import-analysis-progress", percent = progress));
            }
            if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                draft.invalidate();
            }
        });
    }
    ui.add_space(8.0);
    if section == 0 {
        ui.label(RichText::new(aurora_i18n::tr!("import-lods-help")).small().color(p.muted));
        ui.scope(|ui| {
            for level in (0..4).rev() {
                ui.push_id(level, |ui| {
                    // Keep the four LOD rows comparable in the initial viewport.
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let lod = &mut draft.lods[level];
                    let actual = draft
                        .prepared
                        .as_ref()
                        .map(|p| {
                            p.lods
                                .iter()
                                .filter_map(|m| m[level].as_ref())
                                .map(|m| m.triangles())
                                .sum::<usize>()
                        })
                        .or_else(|| lod.file.as_ref().map(|s| s.models.iter().map(|m| m.triangles()).sum()))
                        .or_else(|| (level == 3).then_some(triangles));
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized(
                                Vec2::new(64.0, ui.spacing().interact_size.y),
                                egui::Button::selectable(draft.preview_lod == level, lod_label(level)),
                            )
                            .on_hover_text(aurora_i18n::tr!("import-lod-preview-help"))
                            .clicked()
                        {
                            draft.preview_lod = level;
                        }
                        let before = lod.source;
                        egui::ComboBox::from_id_salt("source")
                            .width(180.0)
                            .selected_text(lod_source(lod.source))
                            .show_ui(ui, |ui| {
                                for source in [
                                    mesh::LodSource::File,
                                    mesh::LodSource::Auto,
                                    mesh::LodSource::Combine,
                                    mesh::LodSource::Fast,
                                    mesh::LodSource::Reliable,
                                    mesh::LodSource::Above,
                                ] {
                                    if level < 3 || source != mesh::LodSource::Above {
                                        ui.selectable_value(&mut lod.source, source, lod_source(source));
                                    }
                                }
                            })
                            .response
                            .on_hover_text(aurora_i18n::tr!("import-source"));
                        changed |= before != lod.source;
                        if let Some(count) = actual {
                            ui.label(
                                RichText::new(aurora_i18n::tr!("import-lod-count", count = count))
                                    .small()
                                    .color(p.muted),
                            );
                        }
                    });
                    if lod.source == mesh::LodSource::File {
                        ui.horizontal(|ui| {
                            if flat_button(ui, p, aurora_i18n::tr!("import-browse")).clicked() {
                                actions.push(Action::MeshFile(id, level));
                            }
                            source_path(ui, p, lod.path.as_deref());
                        });
                    } else if lod.source != mesh::LodSource::Above {
                        ui.horizontal(|ui| {
                            changed |= ui
                                .checkbox(&mut lod.error_mode, aurora_i18n::tr!("import-error-threshold"))
                                .changed();
                            if !lod.error_mode {
                                ui.label(aurora_i18n::tr!("import-triangle-target"));
                            }
                            changed |= if lod.error_mode {
                                ui.add(egui::DragValue::new(&mut lod.error).range(0.0..=1.0).speed(0.001)).changed()
                            } else {
                                ui.add(egui::DragValue::new(&mut lod.triangles).range(0..=triangles)).changed()
                            };
                        });
                    }
                    ui.separator();
                });
            }
            ui.checkbox(&mut draft.options.normals, aurora_i18n::tr!("import-generate-normals"));
            ui.add_enabled(
                draft.options.normals,
                egui::Slider::new(&mut draft.options.crease, 0.0..=180.0).text(aurora_i18n::tr!("import-crease-angle")),
            );
        });
    }
    if section == 1 {
        ui.label(RichText::new(aurora_i18n::tr!("import-physics-help")).small().color(p.muted));
        ui.scope(|ui| {
            ui.label(aurora_i18n::tr!("import-physics-source"));
            let labels = [
                aurora_i18n::tr!("import-convex-only"),
                lod_label(0),
                lod_label(1),
                lod_label(2),
                lod_label(3),
                aurora_i18n::tr!("import-bounding-box"),
                aurora_i18n::tr!("import-source-file"),
                aurora_i18n::tr!("import-physics-cube"),
                aurora_i18n::tr!("import-physics-hex"),
                aurora_i18n::tr!("import-user-physics"),
            ];
            egui::ComboBox::from_id_salt("physics_source")
                .selected_text(labels[(draft.options.physics_source + 1).clamp(0, 9) as usize])
                .show_ui(ui, |ui| {
                    for (i, label) in labels.into_iter().enumerate() {
                        ui.selectable_value(&mut draft.options.physics_source, i as i32 - 1, label);
                    }
                });
            if draft.options.physics_source == 5 {
                if flat_button(ui, p, aurora_i18n::tr!("import-choose-physics")).clicked() {
                    actions.push(Action::MeshFile(id, 4));
                }
                source_path(ui, p, draft.physics_path.as_deref());
            }
            if draft.options.physics_source == 8 && flat_button(ui, p, aurora_i18n::tr!("import-load-preset")).clicked() {
                actions.push(Action::PhysicsPreset(id, false));
            }
            if let Some(path) = &draft.physics_path {
                ui.horizontal_wrapped(|ui| {
                    ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                    if flat_button(ui, p, aurora_i18n::tr!("import-save-preset")).clicked() {
                        actions.push(Action::PhysicsPreset(id, true));
                    }
                });
            }
            ui.add_enabled_ui(draft.options.physics_source >= 0, |ui| {
                ui.checkbox(&mut draft.options.decompose, aurora_i18n::tr!("import-decompose"));
                let mut retain = draft.options.retain * 100.0;
                if ui
                    .add(
                        egui::Slider::new(&mut retain, 1.0..=100.0)
                            .suffix(" %")
                            .text(aurora_i18n::tr!("import-retain")),
                    )
                    .changed()
                {
                    draft.options.retain = retain * 0.01;
                }
                ui.add_enabled_ui(draft.options.decompose, |ui| {
                    let labels = [
                        aurora_i18n::tr!("import-fill-flood"),
                        aurora_i18n::tr!("import-fill-surface"),
                        aurora_i18n::tr!("import-fill-raycast"),
                    ];
                    egui::ComboBox::from_id_salt("physics_fill")
                        .selected_text(labels[draft.options.fill_mode.min(2) as usize])
                        .show_ui(ui, |ui| {
                            for (index, label) in labels.into_iter().enumerate() {
                                ui.selectable_value(&mut draft.options.fill_mode, index as u32, label);
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("import-voxel-resolution"));
                        ui.add(egui::DragValue::new(&mut draft.options.resolution).range(1000..=16000000));
                    });
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("import-max-hulls"));
                        ui.add(egui::DragValue::new(&mut draft.options.max_hulls).range(1..=256));
                    });
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("import-hull-vertices"));
                        ui.add(egui::DragValue::new(&mut draft.options.max_vertices).range(3..=256));
                    });
                    ui.add(
                        egui::Slider::new(&mut draft.options.tolerance, 0.0001..=99.0)
                            .logarithmic(true)
                            .text(aurora_i18n::tr!("import-error-tolerance")),
                    );
                });
            });
        });
    }
    if section == 2 {
        ui.label(RichText::new(aurora_i18n::tr!("import-upload-options-help")).small().color(p.muted));
        egui::CollapsingHeader::new(aurora_i18n::tr!("import-modifiers"))
            .default_open(true)
            .show(ui, |ui| {
                let (_, limit) = draft.dimensions_and_limit();
                ui.add(
                    egui::Slider::new(&mut draft.options.scale, 0.01..=limit.max(0.01))
                        .logarithmic(true)
                        .text(aurora_i18n::tr!("import-scale")),
                );
                let (dimensions, _) = draft.dimensions_and_limit();
                ui.label(aurora_i18n::tr!(
                    "import-dimensions",
                    x = format!("{:.3}", dimensions.x),
                    y = format!("{:.3}", dimensions.y),
                    z = format!("{:.3}", dimensions.z)
                ));
                ui.checkbox(&mut draft.options.textures, aurora_i18n::tr!("import-include-textures"));
            });
        ui.collapsing(aurora_i18n::tr!("import-rigging"), |ui| {
            let has_skin = draft.scene.models.iter().any(|m| m.skin.is_some());
            ui.add_enabled_ui(has_skin, |ui| {
                ui.checkbox(&mut draft.options.skin, aurora_i18n::tr!("import-include-weights"));
                ui.add_enabled_ui(draft.options.skin, |ui| {
                    ui.checkbox(&mut draft.options.joints, aurora_i18n::tr!("import-include-joints"));
                    ui.add_enabled_ui(draft.options.joints, |ui| {
                        ui.checkbox(&mut draft.options.lock_scale, aurora_i18n::tr!("import-lock-scale"));
                        ui.horizontal(|ui| {
                            ui.label(aurora_i18n::tr!("import-pelvis-offset"));
                            ui.add(egui::DragValue::new(&mut draft.options.pelvis).range(-3.0..=3.0).speed(0.01));
                        });
                    });
                });
                for skin in draft.scene.models.iter().filter_map(|m| m.skin.as_ref()) {
                    ui.label(skin.names.join(", "));
                }
            });
        });
        ui.horizontal(|ui| {
            if flat_button(ui, p, aurora_i18n::tr!("import-reset-settings")).clicked() {
                actions.push(Action::ResetMesh(id));
            }
        });
    }
    if section == 2
        && let Some(quote) = &draft.quote
    {
        ui.label(aurora_i18n::tr!("import-total", cost = quote.cost));
        for (key, label) in [
            ("resource_cost", aurora_i18n::tr!("import-land-impact")),
            ("model_streaming_cost", aurora_i18n::tr!("import-streaming-cost")),
            ("simulation_cost", aurora_i18n::tr!("import-server-cost")),
            ("physics_cost", aurora_i18n::tr!("import-physics-cost")),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.label(format!("{:.3}", quote.data[key].as_f64()));
            });
        }
    }
    draft.finish_edit(&old, changed)
}

fn source_path(ui: &mut egui::Ui, p: &Palette, path: Option<&std::path::Path>) {
    if let Some(path) = path {
        ui.add(egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy()).truncate())
            .on_hover_text(path.to_string_lossy());
    } else {
        ui.label(RichText::new(aurora_i18n::tr!("import-no-source-file")).color(p.warn));
    }
}
fn load_settings(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    ui.collapsing(aurora_i18n::tr!("import-load-settings"), |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("import-suffix-help")).small().color(p.muted));
        ui.checkbox(&mut prefs.mesh_auto_weights, aurora_i18n::tr!("import-auto-weights"));
        ui.checkbox(
            &mut prefs.mesh_auto_preview_weights,
            aurora_i18n::tr!("import-auto-preview-weights"),
        );
        ui.checkbox(&mut prefs.mesh_reliable_default, aurora_i18n::tr!("import-default-reliable"));
        ui.horizontal_wrapped(|ui| {
            for (label, values) in [
                (aurora_i18n::tr!("import-suffix-sl"), ["LOD0", "LOD1", "LOD2", "", "PHYS"]),
                (aurora_i18n::tr!("import-suffix-games"), ["LOD3", "LOD2", "LOD1", "LOD0", "PHYS"]),
                (aurora_i18n::tr!("import-suffix-words"), ["lowest", "low", "med", "high", "phys"]),
            ] {
                if flat_button(ui, p, label).clicked() {
                    prefs.mesh_suffixes = values.map(String::from);
                }
            }
        });
        for (level, suffix) in prefs.mesh_suffixes.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(if level < 4 {
                    lod_label(level)
                } else {
                    aurora_i18n::tr!("import-physics")
                });
                ui.add(egui::TextEdit::singleline(suffix).char_limit(32).desired_width(120.0));
            });
        }
        ui.label(aurora_i18n::tr!("import-user-physics"));
        ui.add(egui::TextEdit::singleline(&mut prefs.mesh_physics_preset).desired_width(ui.available_width()));
    });
}
