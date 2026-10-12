//! Mesh import inspector. Firestorm's model preview options are grouped by
//! purpose; preview controls remain beside the model, separate from upload data.
//! Reference: LLFloaterModelPreview / LLModelPreview (indra/newview,
//! llfloatermodelpreview.cpp and llmodelpreview.cpp, originally LGPL 2.1).

use super::*;

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    entry: &mut Entry,
    prefs: &mut Preferences,
    actions: &mut Vec<Action>,
    metadata: impl FnOnce(&mut egui::Ui, &mut Entry),
) {
    let Some(Source::Mesh(draft)) = &entry.source else { return };
    let old_preview = (
        draft.preview_lod,
        draft.preview_physics,
        draft.show_textures,
        draft.preview_weights,
        draft.show_joints,
        draft.show_overrides,
        draft.explode,
    );
    let metadata_width = 464.0;
    let preview_width = (ui.available_width() - metadata_width - ui.spacing().item_spacing.x - 8.0).max(180.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(metadata_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(metadata_width);
            metadata(ui, entry);
            ui.add_space(12.0);
            preview_controls(ui, p, entry, prefs);
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.set_width(preview_width);
            preview(ui, p, entry, prefs, preview_width);
        });
    });
    ui.add_space(8.0);
    let Some(Source::Mesh(draft)) = &mut entry.source else { return };
    let changed = mesh_controls(ui, p, entry.id, draft, actions);
    let section = ui
        .ctx()
        .data_mut(|data| data.get_temp::<usize>(egui::Id::new(("mesh_section", entry.id))))
        .unwrap_or(0);
    if section == 2 {
        load_settings(ui, p, prefs);
    }
    if changed
        || old_preview
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
        entry.invalidate_preview();
    }
}

fn preview_controls(ui: &mut egui::Ui, p: &Palette, entry: &mut Entry, prefs: &mut Preferences) {
    let Some(Source::Mesh(draft)) = &mut entry.source else { return };
    if entry.local {
        ui.label(aurora_i18n::tr!("import-local-mesh-instance"));
        egui::ComboBox::from_id_salt("local_mesh_instance")
            .width(ui.available_width() - 20.0)
            .selected_text(
                draft
                    .scene
                    .instances
                    .get(entry.local_model)
                    .map(|instance| instance.name.as_str())
                    .unwrap_or(aurora_i18n::tr!("import-mesh")),
            )
            .show_ui(ui, |ui| {
                for (index, instance) in draft.scene.instances.iter().enumerate() {
                    selection(ui, &mut entry.local_model, index, &instance.name);
                }
            });
        checkbox(ui, p, &mut entry.local_use_scale, aurora_i18n::tr!("import-local-mesh-scale"));
    }
    ui.horizontal_wrapped(|ui| {
        checkbox(ui, p, &mut entry.view.edges, aurora_i18n::tr!("import-edges"));
        checkbox(ui, p, &mut draft.show_textures, aurora_i18n::tr!("import-textures"));
        checkbox(ui, p, &mut draft.preview_physics, aurora_i18n::tr!("import-physics-preview"))
            .on_hover_text(aurora_i18n::tr!("import-physics-preview-help"));
    });
    ui.collapsing(aurora_i18n::tr!("import-preview-tools"), |ui| {
        checkbox(ui, p, &mut entry.view.uv, aurora_i18n::tr!("import-uv-guide"));
        let skinned = draft.scene.models.iter().any(|model| model.skin.is_some());
        ui.add_enabled_ui(skinned, |ui| {
            checkbox(ui, p, &mut draft.preview_weights, aurora_i18n::tr!("import-preview-weights"));
            checkbox(ui, p, &mut draft.show_joints, aurora_i18n::tr!("import-preview-joints"));
            checkbox(ui, p, &mut draft.show_overrides, aurora_i18n::tr!("import-preview-overrides"));
        });
        if draft.preview_physics {
            ui.add(egui::Slider::new(&mut draft.explode, 0.0..=3.0).text(aurora_i18n::tr!("import-explode-hulls")));
        }
    });
    preview_colors(ui, p, prefs);
}

fn preview(ui: &mut egui::Ui, p: &Palette, entry: &mut Entry, prefs: &Preferences, width: f32) {
    let Some(Source::Mesh(draft)) = &entry.source else { return };
    ui.horizontal(|ui| {
        widgets::section_header(ui, p, "eye", aurora_i18n::tr!("import-preview"));
        if entry.preview_pending || entry.preview_not_before.is_some() {
            ui.add(egui::Spinner::new().size(12.0));
        }
    });
    entry.view.material = draft.preview_physics.then(|| aurora_render::preview::Material {
        color: linear_color(prefs.preview_physics_fill.unwrap_or_else(|| palette_color(p.teal))),
        alpha_mode: u32::from(prefs.preview_physics_fill.is_some_and(|color| color[3] < 1.0)),
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
            Vec2::new(width, 180.0),
        );
    } else {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 180.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, p.field);
        if entry.preview_pending {
            ui.put(rect, egui::Spinner::new());
        }
    }
    ui.label(RichText::new(aurora_i18n::tr!("import-preview-camera-help")).small().color(p.muted));
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
fn mesh_controls(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &mut mesh::Draft, actions: &mut Vec<Action>) -> bool {
    let old = draft.options.clone();
    let mut changed = false;
    let triangles = draft.scene.models.iter().map(|m| m.triangles()).sum::<usize>();
    ui.horizontal(|ui| {
        widgets::section_header(ui, p, "cube", aurora_i18n::tr!("import-mesh"));
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "import-mesh-info",
                models = draft.scene.models.len(),
                instances = draft.scene.instances.len(),
                triangles = triangles
            ))
            .small()
            .color(p.muted),
        );
    });
    if let Some(prepared) = &draft.prepared
        && prepared.degenerate_triangles > 0
    {
        ui.label(RichText::new(aurora_i18n::tr!("import-degenerate-physics", count = prepared.degenerate_triangles)).color(p.warn));
    }
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
    widgets::tabs(
        ui,
        p,
        &mut section,
        &[
            (aurora_i18n::tr!("import-mesh-section-lods"), true),
            (aurora_i18n::tr!("import-mesh-section-physics"), true),
            (aurora_i18n::tr!("import-mesh-section-upload"), true),
        ],
    );
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
    ui.add_space(2.0);
    if section == 0 {
        ui.label(RichText::new(aurora_i18n::tr!("import-lods-help")).small().color(p.muted));
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().interact_size.y = 24.0;
            for level in (0..4).rev() {
                ui.push_id(level, |ui| {
                    let selected = draft.preview_lod == level;
                    egui::Frame::new()
                        .fill(p.field)
                        .stroke(egui::Stroke::new(
                            1.0,
                            if selected {
                                p.violet_light
                            } else {
                                p.muted_dim.gamma_multiply(0.65)
                            },
                        ))
                        .corner_radius(4)
                        .inner_margin(3)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
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
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .add_sized(
                                        Vec2::new(100.0, ui.spacing().interact_size.y),
                                        egui::Button::new(
                                            RichText::new(aurora_i18n::tr!("import-lod-level", level = lod_label(level))).strong(),
                                        )
                                        .selected(selected),
                                    )
                                    .on_hover_text(aurora_i18n::tr!("import-lod-preview-help"))
                                    .clicked()
                                {
                                    draft.preview_lod = level;
                                }
                                let mut source = lod.source;
                                egui::ComboBox::from_id_salt("source")
                                    .width(180.0)
                                    .selected_text(lod_source(lod.source))
                                    .show_ui(ui, |ui| {
                                        for choice in [
                                            mesh::LodSource::File,
                                            mesh::LodSource::Auto,
                                            mesh::LodSource::Combine,
                                            mesh::LodSource::Fast,
                                            mesh::LodSource::Reliable,
                                            mesh::LodSource::Above,
                                        ] {
                                            if level < 3 || choice != mesh::LodSource::Above {
                                                selection(ui, &mut source, choice, lod_source(choice));
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text(aurora_i18n::tr!("import-source"));
                                if source != lod.source {
                                    if source == mesh::LodSource::File && level < 3 && lod.file.is_none() {
                                        // Commit file mode only after the picker returns a decoded model.
                                        actions.push(Action::MeshFile(id, level));
                                    } else {
                                        lod.source = source;
                                        changed = true;
                                    }
                                }
                                if lod.source == mesh::LodSource::File {
                                    if flat_button(ui, p, aurora_i18n::tr!("import-browse")).clicked() {
                                        actions.push(Action::MeshFile(id, level));
                                    }
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(160.0, 26.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| source_path(ui, p, lod.path.as_deref()),
                                    );
                                } else if lod.source != mesh::LodSource::Above {
                                    changed |= checkbox(ui, p, &mut lod.error_mode, aurora_i18n::tr!("import-error-threshold"))
                                        .on_hover_text(aurora_i18n::tr!("import-error-threshold-help"))
                                        .changed();
                                    if !lod.error_mode {
                                        ui.label(aurora_i18n::tr!("import-triangle-target"));
                                    }
                                    changed |= if lod.error_mode {
                                        let mut percent = lod.error * 100.0;
                                        let response = ui
                                            .add(egui::DragValue::new(&mut percent).range(0.0..=100.0).speed(0.01).suffix(" %"))
                                            .on_hover_text(aurora_i18n::tr!("import-error-threshold-help"));
                                        if response.changed() {
                                            lod.error = percent * 0.01;
                                        }
                                        response.changed()
                                    } else {
                                        ui.add(egui::DragValue::new(&mut lod.triangles).range(0..=triangles)).changed()
                                    };
                                }
                                if let Some(count) = actual {
                                    ui.label(
                                        RichText::new(aurora_i18n::tr!("import-lod-count", count = count))
                                            .small()
                                            .color(p.muted),
                                    );
                                }
                            });
                        });
                });
            }
            checkbox(ui, p, &mut draft.options.normals, aurora_i18n::tr!("import-generate-normals"));
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
                        selection(ui, &mut draft.options.physics_source, i as i32 - 1, label);
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
                checkbox(ui, p, &mut draft.options.decompose, aurora_i18n::tr!("import-decompose"));
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
                                selection(ui, &mut draft.options.fill_mode, index as u32, label);
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
                checkbox(ui, p, &mut draft.options.textures, aurora_i18n::tr!("import-include-textures"));
            });
        ui.collapsing(aurora_i18n::tr!("import-rigging"), |ui| {
            let has_skin = draft.scene.models.iter().any(|m| m.skin.is_some());
            ui.add_enabled_ui(has_skin, |ui| {
                checkbox(ui, p, &mut draft.options.skin, aurora_i18n::tr!("import-include-weights"));
                ui.add_enabled_ui(draft.options.skin, |ui| {
                    checkbox(ui, p, &mut draft.options.joints, aurora_i18n::tr!("import-include-joints"));
                    ui.add_enabled_ui(draft.options.joints, |ui| {
                        checkbox(ui, p, &mut draft.options.lock_scale, aurora_i18n::tr!("import-lock-scale"));
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
        checkbox(ui, p, &mut prefs.mesh_auto_weights, aurora_i18n::tr!("import-auto-weights"));
        checkbox(
            ui,
            p,
            &mut prefs.mesh_auto_preview_weights,
            aurora_i18n::tr!("import-auto-preview-weights"),
        );
        checkbox(ui, p, &mut prefs.mesh_reliable_default, aurora_i18n::tr!("import-default-reliable"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_controls_stay_in_place_with_a_model_during_a_recalculation_and_without_a_model() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut entry = Entry::new(
            "mesh.dae".into(),
            Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
            Default::default(),
        );
        entry.model = Some(Arc::new(aurora_render::preview::Model {
            parts: Vec::new(),
            center: glam::Vec3::ZERO,
            radius: 1.0,
        }));
        let mut prefs = Preferences::default();
        let mut frame = |entry: &mut Entry| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(1000.0);
                    show(ui, &theme.palette(), entry, &mut prefs, &mut Vec::new(), |_, _| {});
                },
            );
            text_position(&output, &aurora_i18n::tr!("import-lod-level", level = lod_label(3)), false).y
        };
        let mut original = 0.0;
        for _ in 0..3 {
            original = frame(&mut entry);
        }
        entry.invalidate_preview();
        entry.preview_pending = true;
        assert!(
            (frame(&mut entry) - original).abs() < 0.1,
            "recalculation must retain the preview's height"
        );
        entry.model = None;
        assert!(
            (frame(&mut entry) - original).abs() < 0.1,
            "initial loading reserves the same height"
        );
    }

    fn text_position(output: &egui::FullOutput, label: &str, last: bool) -> egui::Pos2 {
        let positions: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .collect();
        if last {
            *positions.last().expect("text visible")
        } else {
            positions[0]
        }
    }

    #[test]
    fn picking_file_mode_waits_for_a_file_and_lod_clicks_clear_a_failed_preview() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut entry = Entry::new(
            "mesh.dae".into(),
            Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
            Default::default(),
        );
        let mut prefs = Preferences::default();
        let mut frame = |events, entry: &mut Entry| {
            let mut actions = Vec::new();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(1000.0);
                    show(ui, &theme.palette(), entry, &mut prefs, &mut actions, |_, _| {});
                },
            );
            (output, actions)
        };
        let events = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let mut output = None;
        for _ in 0..3 {
            output = Some(frame(Vec::new(), &mut entry).0);
        }
        let source = text_position(output.as_ref().expect("frame"), aurora_i18n::tr!("import-lod-auto"), false);
        for pressed in [true, false] {
            output = Some(frame(events(source, pressed), &mut entry).0);
        }
        for _ in 0..3 {
            output = Some(frame(Vec::new(), &mut entry).0);
        }
        let file = text_position(output.as_ref().expect("popup"), aurora_i18n::tr!("import-source-file"), true);
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(frame(events(file, pressed), &mut entry).1);
        }
        assert!(
            matches!(actions.as_slice(), [Action::MeshFile(id, 2)] if *id == entry.id),
            "file actions: {:?}",
            actions
                .iter()
                .map(|action| match action {
                    Action::MeshFile(_, level) => *level as i32,
                    _ => -1,
                })
                .collect::<Vec<_>>()
        );
        let Some(Source::Mesh(draft)) = &entry.source else { panic!("mesh") };
        assert!(
            draft.lods[2].source == mesh::LodSource::Auto,
            "cancelled file picker keeps the working source"
        );
        assert_eq!(entry.preview_generation, 0);
        assert!(entry.preview_error.is_none());

        output = Some(frame(Vec::new(), &mut entry).0);
        let low = text_position(
            output.as_ref().expect("frame"),
            &aurora_i18n::tr!("import-lod-level", level = lod_label(1)),
            false,
        );
        entry.preview_error = Some("previous preparation failure".into());
        for pressed in [true, false] {
            assert!(frame(events(low, pressed), &mut entry).1.is_empty());
        }
        let Some(Source::Mesh(draft)) = &entry.source else { panic!("mesh") };
        assert_eq!(draft.preview_lod, 1);
        assert_eq!(entry.preview_generation, 1);
        assert!(entry.preview_error.is_none(), "new preview selection can retry after an error");
    }
}
