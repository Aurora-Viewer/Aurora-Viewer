//! Native import inspector following the approved workshop layout.
//! Legacy Viewer options remain connected to the ordinary processing workers.
//! Reference: LLFloaterModelPreview / LLModelPreview (indra/newview,
//! llfloatermodelpreview.cpp and llmodelpreview.cpp, originally LGPL 2.1).

use super::*;

mod panels;
use panels::{avatar_controls, journal_controls};

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    entry: &mut Entry,
    prefs: &mut Preferences,
    actions: &mut Vec<Action>,
    height: f32,
    metadata: impl FnOnce(&mut egui::Ui, &mut Entry),
) {
    let width = ui.available_width();
    let (bounds, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    // Scroll contents must never become a minimum size for the parent
    // window, including at fractional DPI. Allocate the inspector once.
    let mut panel_ui = ui.new_child(egui::UiBuilder::new().id_salt(("mesh_panel", entry.id)).max_rect(bounds));
    panel_ui.set_clip_rect(bounds.intersect(ui.clip_rect()));
    let ui = &mut panel_ui;
    let metadata_width = (width - 278.0).max(240.0);
    let (header_rect, _) = ui.allocate_exact_size(Vec2::new(width, 170.0), egui::Sense::hover());
    let mut header_ui = ui.new_child(egui::UiBuilder::new().id_salt("mesh_header").max_rect(header_rect));
    header_ui.set_clip_rect(header_rect.intersect(ui.clip_rect()));
    header_ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(metadata_width, 170.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_width(metadata_width);
            metadata(ui, entry);
        });
        ui.horizontal_top(|ui| {
            preview(ui, p, entry, prefs, 230.0, 170.0, actions);
            ui.vertical(|ui| {
                if widgets::icon_button(ui, p, "zoom-in-filled", aurora_i18n::tr!("import-preview-enlarge"), true) {
                    actions.push(Action::Zoom(entry.id, None));
                }
                if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("import-preview-reset-view"), true) {
                    entry.view.reset_camera();
                }
                preview_colors_button(ui, p, entry.id, prefs);
                widgets::icon_menu(ui, p, "settings-filled", aurora_i18n::tr!("import-preview-settings"), |ui| {
                    ui.set_min_width(230.0);
                    preview_controls(ui, p, entry);
                });
            });
        });
    });
    ui.add_space(8.0);
    let Some(Source::Mesh(draft)) = &mut entry.source else { return };
    let section = mesh_section(ui, p, entry.id, draft);
    let summary_height = if draft.quote.is_some() { 54.0 } else { 28.0 };
    let summary_rect = egui::Rect::from_min_size(
        egui::pos2(bounds.left(), bounds.bottom() - summary_height),
        Vec2::new(width, summary_height),
    );
    let content_height = (summary_rect.top() - ui.cursor().top() - ui.spacing().item_spacing.y).max(0.0);
    let content_rect = egui::Rect::from_min_size(ui.next_widget_position(), Vec2::new(width, content_height));
    let mut content_ui = ui.new_child(egui::UiBuilder::new().id_salt("mesh_content").max_rect(content_rect));
    content_ui.set_clip_rect(content_rect.intersect(ui.clip_rect()));
    let before_lod = draft.preview_lod;
    let before_joint = draft.inspector.selected_joint.clone();
    let body = egui::ScrollArea::both()
        .id_salt(("mesh_data", entry.id))
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .max_height(content_height)
        .show(&mut content_ui, |ui| {
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::unique(("mesh_content_height", entry.id)), ui.available_height()));
            // Keep full-width rows clear of the rounded viewport edge at fractional DPI.
            ui.set_max_width((ui.available_width() - ui.spacing().item_spacing.x).max(0.0));
            mesh_controls(ui, p, entry.id, draft, prefs, actions, section, entry.from_live)
        });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique(("mesh_viewport", entry.id)), (body.inner_rect, body.content_size)));
    let changed = body.inner;
    let mut summary_ui = ui.new_child(egui::UiBuilder::new().id_salt("mesh_summary").max_rect(summary_rect));
    summary_ui.set_clip_rect(summary_rect.intersect(ui.clip_rect()));
    mesh_summary(&mut summary_ui, p, draft);
    if changed || before_lod != draft.preview_lod || before_joint != draft.inspector.selected_joint {
        entry.invalidate_preview();
    }
}

fn mesh_summary(ui: &mut egui::Ui, p: &Palette, draft: &mesh::Draft) {
    ui.horizontal(|ui| {
        widgets::section_header(ui, p, "cube-3d-sphere", aurora_i18n::tr!("import-mesh"));
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "import-mesh-info",
                models = draft.scene.models.len(),
                instances = draft.scene.instances.len(),
                triangles = draft.scene.models.iter().map(|m| m.triangles()).sum::<usize>()
            ))
            .color(p.muted),
        );
    });
    if let Some(quote) = &draft.quote {
        ui.horizontal(|ui| {
            for (key, label) in [
                ("resource_cost", aurora_i18n::tr!("import-land-impact")),
                ("model_streaming_cost", aurora_i18n::tr!("import-streaming-cost")),
                ("simulation_cost", aurora_i18n::tr!("import-server-cost")),
                ("physics_cost", aurora_i18n::tr!("import-physics-cost")),
            ] {
                ui.label(RichText::new(format!("{label}  {:.3}", quote.data[key].as_f64())));
            }
        });
    }
}

fn preview_controls(ui: &mut egui::Ui, p: &Palette, entry: &mut Entry) {
    let Some(Source::Mesh(draft)) = &mut entry.source else { return };
    let old_preview = (
        draft.preview_lod,
        draft.preview_physics,
        draft.show_textures,
        draft.preview_weights,
        draft.show_joints,
        draft.show_overrides,
        draft.explode,
        draft.preview_bend,
    );
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
    checkbox(ui, p, &mut entry.view.edges, aurora_i18n::tr!("import-edges"));
    checkbox(ui, p, &mut draft.show_textures, aurora_i18n::tr!("import-textures"));
    checkbox(ui, p, &mut draft.preview_physics, aurora_i18n::tr!("import-physics-preview"));
    checkbox(ui, p, &mut entry.view.uv, aurora_i18n::tr!("import-uv-guide"));
    let skinned = draft.inspector.avatar.valid() && draft.options.skin;
    ui.add_enabled_ui(skinned, |ui| {
        checkbox(ui, p, &mut draft.preview_weights, aurora_i18n::tr!("import-preview-weights"));
        checkbox(ui, p, &mut draft.show_joints, aurora_i18n::tr!("import-preview-joints"));
        checkbox(ui, p, &mut draft.show_overrides, aurora_i18n::tr!("import-preview-overrides"));
        ui.add(egui::Slider::new(&mut draft.preview_bend, 0.0..=90.0).text(aurora_i18n::tr!("import-preview-bend")));
    });
    if draft.preview_physics {
        ui.add(egui::Slider::new(&mut draft.explode, 0.0..=3.0).text(aurora_i18n::tr!("import-explode-hulls")));
    }
    if old_preview
        != (
            draft.preview_lod,
            draft.preview_physics,
            draft.show_textures,
            draft.preview_weights,
            draft.show_joints,
            draft.show_overrides,
            draft.explode,
            draft.preview_bend,
        )
    {
        entry.invalidate_preview();
    }
}

fn preview(
    ui: &mut egui::Ui,
    p: &Palette,
    entry: &mut Entry,
    prefs: &mut Preferences,
    width: f32,
    height: f32,
    __actions: &mut Vec<Action>,
) {
    let Some(Source::Mesh(draft)) = &entry.source else { return };
    apply_preview_colors(&mut entry.view, p, prefs);
    entry.view.material = draft.preview_physics.then(|| aurora_render::preview::Material {
        color: linear_color(prefs.preview_physics_fill.unwrap_or_else(|| palette_color(p.teal))),
        alpha_mode: u32::from(prefs.preview_physics_fill.is_some_and(|color| color[3] < 1.0)),
        ..Default::default()
    });
    if draft.preview_physics {
        entry.view.edge_color = prefs.preview_physics_edges.unwrap_or_else(|| palette_color(p.ink));
    }
    let response = if let Some(model) = &entry.model {
        aurora_render::preview::paint_with_pose(
            ui,
            egui::Id::unique(("mesh_preview", entry.id)),
            model.clone(),
            &mut entry.view,
            Vec2::new(width, height),
            entry.mesh_player.pose.clone(),
        )
    } else {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, p.field);
        if entry.preview_pending {
            ui.put(rect, egui::Spinner::new());
        }
        response
    };
    ui.painter().text(
        response.rect.left_top() + Vec2::splat(8.0),
        egui::Align2::LEFT_TOP,
        aurora_i18n::tr!("import-lod-level", level = lod_label(draft.preview_lod)),
        egui::TextStyle::Body.resolve(ui.style()),
        p.muted,
    );
    let canvas_rect = response.rect;
    response.on_hover_text(aurora_i18n::tr!("import-preview-camera-help"));
    if entry.preview_pending || entry.preview_not_before.is_some() {
        ui.put(
            egui::Rect::from_min_size(canvas_rect.right_top() + Vec2::new(-18.0, 2.0), Vec2::splat(14.0)),
            egui::Spinner::new().size(14.0),
        );
    }
    let mut hint_painter = ui.painter().clone();
    let hint = hint_painter.layout(
        aurora_i18n::tr!("import-preview-gestures").to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        p.muted,
        width,
    );
    let hint_height = hint.size().y + 4.0;
    let clip = hint_painter.clip_rect();
    hint_painter.set_clip_rect(egui::Rect::from_min_max(
        egui::pos2(clip.left(), clip.top() - hint_height),
        clip.max,
    ));
    hint_painter.galley(canvas_rect.left_top() - Vec2::new(0.0, hint_height), hint, p.muted);
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
fn mesh_section(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &mut mesh::Draft) -> usize {
    let section_id = egui::Id::unique(("mesh_section", id));
    let mut section = ui
        .ctx()
        .data_mut(|d| d.get_temp::<usize>(section_id))
        .unwrap_or(usize::from(draft.preview_physics))
        .min(4);
    if draft.inspector.unread {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
    }
    let journal = if draft.inspector.unread && ui.input(|i| i.time).rem_euclid(1.0) < 0.5 {
        format!("{} •", aurora_i18n::tr!("import-section-journal"))
    } else {
        aurora_i18n::tr!("import-section-journal").to_owned()
    };
    widgets::stretched_tabs(
        ui,
        p,
        &mut section,
        &[
            (aurora_i18n::tr!("import-mesh-section-lods"), true),
            (aurora_i18n::tr!("import-mesh-section-physics"), true),
            (aurora_i18n::tr!("import-mesh-section-upload"), true),
            (aurora_i18n::tr!("import-section-avatar"), true),
            (journal.as_str(), true),
        ],
    );
    if section == 4 {
        draft.inspector.unread = false;
    }
    ui.ctx().data_mut(|d| d.insert_temp(section_id, section));
    section
}

fn mesh_controls(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    draft: &mut mesh::Draft,
    prefs: &mut Preferences,
    actions: &mut Vec<Action>,
    section: usize,
    from_live: bool,
) -> bool {
    draft.inspector.verbose = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(egui::Id::unique("import_mesh_detailed_logging")))
        .unwrap_or(false);
    let old = draft.options.clone();
    let mut changed = false;
    let triangles = draft.scene.models.iter().map(|model| model.triangles()).sum::<usize>();
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
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 7.0;
            for level in (0..4).rev() {
                ui.push_id(level, |ui| {
                    let selected = draft.preview_lod == level;
                    let actual = draft
                        .prepared
                        .as_ref()
                        .map(|prepared| {
                            prepared
                                .lods
                                .iter()
                                .filter_map(|models| models[level].as_ref())
                                .map(|model| model.triangles())
                                .sum::<usize>()
                        })
                        .or_else(|| {
                            draft.lods[level]
                                .file
                                .as_ref()
                                .map(|scene| scene.models.iter().map(|model| model.triangles()).sum())
                        })
                        .or_else(|| (level == 3).then_some(triangles));
                    let response = ui.horizontal(|ui| {
                        ui.set_min_height(32.0);
                        let icon = ui.add_sized(Vec2::splat(26.0), egui::Button::new("").selected(selected));
                        lod_icon(ui, icon.rect, level, if selected { p.on_accent } else { p.muted });
                        ui.add_sized(Vec2::new(54.0, 26.0), egui::Label::new(lod_label(level)));
                        let lod = &mut draft.lods[level];
                        let mut edited = false;
                        let mut source = lod.source;
                        egui::ComboBox::from_id_salt(("source", id, level))
                            .width(160.0)
                            .truncate()
                            .selected_text(
                                if from_live && source == mesh::LodSource::File && lod.path.is_none() && (level == 3 || lod.file.is_some())
                                {
                                    aurora_i18n::tr!("blender-live-title")
                                } else {
                                    lod_source(source)
                                },
                            )
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
                            });
                        if source != lod.source {
                            if source == mesh::LodSource::File && level < 3 && lod.file.is_none() {
                                actions.push(Action::MeshFile(id, level));
                            } else {
                                lod.source = source;
                                edited = true;
                            }
                        }
                        if lod.source == mesh::LodSource::File {
                            if flat_button(ui, p, aurora_i18n::tr!("import-browse")).clicked() {
                                actions.push(Action::MeshFile(id, level));
                            }
                            ui.allocate_ui(Vec2::new(125.0, 26.0), |ui| {
                                if from_live && lod.path.is_none() && (level == 3 || lod.file.is_some()) {
                                    ui.add(egui::Label::new(aurora_i18n::tr!("blender-live-upload-source")).truncate())
                                        .on_hover_text(aurora_i18n::tr!("blender-live-upload-frozen"));
                                } else {
                                    source_path(ui, p, lod.path.as_deref());
                                }
                            });
                        } else if lod.source != mesh::LodSource::Above {
                            let before = lod.error_mode;
                            egui::ComboBox::from_id_salt(("criterion", id, level))
                                .width(126.0)
                                .truncate()
                                .selected_text(if lod.error_mode {
                                    aurora_i18n::tr!("import-error-threshold")
                                } else {
                                    aurora_i18n::tr!("import-triangle-target")
                                })
                                .show_ui(ui, |ui| {
                                    selection(ui, &mut lod.error_mode, false, aurora_i18n::tr!("import-triangle-target"));
                                    selection(ui, &mut lod.error_mode, true, aurora_i18n::tr!("import-error-threshold"));
                                });
                            edited |= before != lod.error_mode;
                            if lod.error_mode {
                                let mut percent = lod.error * 100.0;
                                let response = ui.add_sized(
                                    Vec2::new(62.0, 26.0),
                                    egui::DragValue::new(&mut percent).range(0.0..=100.0).speed(0.01).suffix(" %"),
                                );
                                if response.changed() {
                                    lod.error = percent * 0.01;
                                    edited = true;
                                }
                            } else {
                                edited |= ui
                                    .add_sized(Vec2::new(62.0, 26.0), egui::DragValue::new(&mut lod.triangles).range(0..=triangles))
                                    .changed();
                            }
                        }
                        if let Some(count) = actual {
                            ui.label(RichText::new(aurora_i18n::tr!(
                                "import-lod-count-short",
                                count = aurora_i18n::number(count as f64, 0)
                            )));
                            if level < 3 {
                                ui.label(
                                    RichText::new(aurora_i18n::tr!(
                                        "import-lod-retained",
                                        percent = aurora_i18n::number(count as f64 * 100.0 / triangles.max(1) as f64, 1)
                                    ))
                                    .color(p.muted),
                                );
                            }
                        }
                        if edited {
                            draft.preview_lod = level;
                            changed = true;
                        }
                    });
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("import_lod_card", id, level)), response.response.rect));
                    // Observe the press without placing an interaction over the native fields.
                    let position = ui.input(|input| input.pointer.primary_pressed().then(|| input.pointer.interact_pos()).flatten());
                    let clicked = ui.is_enabled()
                        && position.is_some_and(|position| {
                            response.response.rect.contains(position)
                                && ui.clip_rect().contains(position)
                                && ui.ctx().layer_id_at(position) == Some(ui.layer_id())
                        });
                    if clicked {
                        draft.preview_lod = level;
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                checkbox(ui, p, &mut draft.options.normals, aurora_i18n::tr!("import-generate-normals"));
                ui.add_enabled_ui(draft.options.normals, |ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("import-crease-angle")));
                    ui.add_sized(
                        Vec2::new(62.0, 26.0),
                        egui::DragValue::new(&mut draft.options.crease)
                            .range(0.0..=180.0)
                            .suffix("\u{00b0}"),
                    );
                });
            });
        });
    }
    if section == 1 {
        let physics_top = ui.cursor().top();
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("import-collision-step-source"));
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
                .width(180.0)
                .truncate()
                .selected_text(labels[(draft.options.physics_source + 1).clamp(0, 9) as usize])
                .show_ui(ui, |ui| {
                    for (i, label) in labels.into_iter().enumerate() {
                        selection(ui, &mut draft.options.physics_source, i as i32 - 1, label);
                    }
                });
            if draft.options.physics_source == 5 {
                source_path(ui, p, draft.physics_path.as_deref());
                if flat_button(ui, p, aurora_i18n::tr!("import-browse")).clicked() {
                    actions.push(Action::MeshFile(id, 4));
                }
            }
            if draft.options.physics_source == 8 && flat_button(ui, p, aurora_i18n::tr!("import-load-preset")).clicked() {
                actions.push(Action::PhysicsPreset(id, false));
            }
            if draft.physics_path.is_some() && flat_button(ui, p, aurora_i18n::tr!("import-save-preset")).clicked() {
                actions.push(Action::PhysicsPreset(id, true));
            }
        });
        ui.add_enabled_ui(draft.options.physics_source >= 0, |ui| {
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
            widgets::divider(ui, p);
            checkbox(ui, p, &mut draft.options.decompose, aurora_i18n::tr!("import-collision-step-hulls"));
            ui.add_enabled_ui(draft.options.decompose, |ui| {
                // Leave one spacing unit free so rounding six columns at
                // fractional DPI cannot create a horizontal scroll bar.
                let column_width = (ui.available_width() - 6.0 * ui.spacing().item_spacing.x) / 6.0;
                ui.horizontal_top(|ui| {
                    physics_column(ui, column_width, aurora_i18n::tr!("import-fill-mode"), |ui| {
                        let labels = [
                            aurora_i18n::tr!("import-fill-flood"),
                            aurora_i18n::tr!("import-fill-surface"),
                            aurora_i18n::tr!("import-fill-raycast"),
                        ];
                        egui::ComboBox::from_id_salt("physics_fill")
                            .width((column_width - 8.0).min(120.0))
                            .truncate()
                            .selected_text(labels[draft.options.fill_mode.min(2) as usize])
                            .show_ui(ui, |ui| {
                                for (index, label) in labels.into_iter().enumerate() {
                                    selection(ui, &mut draft.options.fill_mode, index as u32, label);
                                }
                            });
                    });
                    physics_column(ui, column_width, aurora_i18n::tr!("import-voxel-resolution"), |ui| {
                        ui.add_sized(
                            Vec2::new(90.0, 26.0),
                            egui::DragValue::new(&mut draft.options.resolution).range(1000..=16000000),
                        );
                    });
                    physics_column(ui, column_width, aurora_i18n::tr!("import-max-hulls"), |ui| {
                        ui.add_sized(
                            Vec2::new(76.0, 26.0),
                            egui::DragValue::new(&mut draft.options.max_hulls).range(1..=256),
                        );
                    });
                    physics_column(ui, column_width, aurora_i18n::tr!("import-hull-vertices"), |ui| {
                        ui.add_sized(
                            Vec2::new(76.0, 26.0),
                            egui::DragValue::new(&mut draft.options.max_vertices).range(3..=256),
                        );
                    });
                    physics_column(ui, column_width, aurora_i18n::tr!("import-error-tolerance"), |ui| {
                        ui.add_sized(
                            Vec2::new(76.0, 26.0),
                            egui::DragValue::new(&mut draft.options.tolerance).range(0.0001..=99.0).speed(0.001),
                        );
                    });
                    physics_column(ui, column_width, "", |ui| {
                        if flat_button(ui, p, aurora_i18n::tr!("import-collision-analyze")).clicked() {
                            changed = true;
                        }
                    });
                });
            });
        });
        let space = ui
            .ctx()
            .data(|d| d.get_temp::<f32>(egui::Id::unique(("mesh_content_height", id))))
            .unwrap_or(260.0);
        ui.add_space((space - (ui.cursor().top() - physics_top) - 44.0).max(8.0));
        widgets::divider(ui, p);
        if let Some(prepared) = &draft.prepared {
            let triangles: usize = prepared.physics.iter().flatten().map(|m| m.triangles()).sum();
            let vertices: usize = prepared
                .physics
                .iter()
                .flatten()
                .flat_map(|m| &m.faces)
                .map(|f| f.vertices.len())
                .sum();
            let hulls: usize = prepared.hulls.iter().map(|physics| physics.hulls.len()).sum();
            ui.label(aurora_i18n::tr!(
                "import-collision-results",
                triangles = triangles,
                vertices = vertices,
                hulls = hulls
            ));
        } else {
            ui.label(aurora_i18n::tr!("import-collision-results-pending"));
        }
    }
    if section == 2 {
        ui.label(RichText::new(aurora_i18n::tr!("import-upload-options-help")).color(p.muted));
        ui.add_space(4.0);
        ui.columns(2, |columns| {
            for column in columns.iter_mut() {
                column.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            }
            upload_options(&mut columns[0], p, id, draft, actions);
            load_options(&mut columns[1], p, prefs);
        });
        ui.add_space(6.0);
        load_settings(ui, p, prefs);
    }
    if section == 3 {
        avatar_controls(ui, p, id, draft);
    }
    if section == 4 {
        journal_controls(ui, p, id, draft);
    }
    draft.finish_edit(&old, changed)
}

fn physics_column(ui: &mut egui::Ui, width: f32, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(Vec2::new(width, 84.0), egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_width(width);
        ui.allocate_ui_with_layout(Vec2::new(width, 52.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_height(52.0);
            ui.add(egui::Label::new(label).wrap()).on_hover_text(label);
        });
        body(ui);
    });
}

fn upload_options(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &mut mesh::Draft, actions: &mut Vec<Action>) {
    fold(ui, p, aurora_i18n::tr!("import-modifiers"), true, |ui| {
        let (_, limit) = draft.dimensions_and_limit();
        ui.label(aurora_i18n::tr!("import-scale"));
        ui.add(egui::Slider::new(&mut draft.options.scale, 0.01..=limit.max(0.01)).logarithmic(true));
        let (dimensions, _) = draft.dimensions_and_limit();
        ui.label(aurora_i18n::tr!(
            "import-dimensions",
            x = format!("{:.3}", dimensions.x),
            y = format!("{:.3}", dimensions.y),
            z = format!("{:.3}", dimensions.z)
        ));
        checkbox(ui, p, &mut draft.options.textures, aurora_i18n::tr!("import-include-textures"));
    });
    if flat_button(ui, p, aurora_i18n::tr!("import-reset-settings")).clicked() {
        actions.push(Action::ResetMesh(id));
    }
}

fn source_path(ui: &mut egui::Ui, p: &Palette, path: Option<&std::path::Path>) {
    if let Some(path) = path {
        ui.add(egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy()).truncate())
            .on_hover_text(path.to_string_lossy());
    } else {
        ui.label(RichText::new(aurora_i18n::tr!("import-no-source-file")).color(p.warn));
    }
}
fn load_options(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    fold(ui, p, aurora_i18n::tr!("import-load-settings"), true, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("import-suffix-help")).color(p.muted));
        checkbox(ui, p, &mut prefs.mesh_auto_weights, aurora_i18n::tr!("import-auto-weights"));
        checkbox(
            ui,
            p,
            &mut prefs.mesh_auto_preview_weights,
            aurora_i18n::tr!("import-auto-preview-weights"),
        );
        checkbox(ui, p, &mut prefs.mesh_reliable_default, aurora_i18n::tr!("import-default-reliable"));
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("import-generated-model-limit"));
            ui.add(egui::DragValue::new(&mut prefs.mesh_model_limit).range(0..=u32::MAX));
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("import-mesh-upload-timeout"));
            ui.add(egui::DragValue::new(&mut prefs.mesh_upload_timeout).range(1..=36000));
        });
    });
}

fn load_settings(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    fold(ui, p, aurora_i18n::tr!("import-file-associations"), true, |ui| {
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
        let labels = [
            lod_label(3),
            lod_label(2),
            lod_label(1),
            lod_label(0),
            aurora_i18n::tr!("import-physics"),
        ];
        let font = egui::TextStyle::Body.resolve(ui.style());
        let label_width = ui.fonts_mut(|fonts| {
            labels
                .iter()
                .map(|label| fonts.layout_no_wrap((*label).into(), font.clone(), p.ink).size().x)
                .sum::<f32>()
        });
        let room = ui.available_width() - label_width - 9.0 * ui.spacing().item_spacing.x;
        let pairs = if room >= 400.0 { 5 } else { 2 };
        let field_width = if pairs == 5 { (room / 5.0).min(120.0) } else { 100.0 };
        egui::Grid::new("mesh_suffixes")
            .num_columns(pairs * 2)
            .min_col_width(0.0)
            .spacing(Vec2::new(8.0, 6.0))
            .show(ui, |ui| {
                for (index, level) in [3, 2, 1, 0, 4].into_iter().enumerate() {
                    ui.label(labels[index]);
                    ui.add_sized(
                        Vec2::new(field_width, ui.spacing().interact_size.y),
                        egui::TextEdit::singleline(&mut prefs.mesh_suffixes[level])
                            .char_limit(32)
                            .desired_width(field_width),
                    );
                    if (index + 1) % pairs == 0 || index == 4 {
                        ui.end_row();
                    }
                }
            });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("import-user-physics"));
            ui.add(egui::TextEdit::singleline(&mut prefs.mesh_physics_preset).desired_width(ui.available_width()));
        });
    });
}

// Topology glyphs requested for LOD density; labels remain in the preview and tooltips.
fn lod_icon(ui: &egui::Ui, rect: egui::Rect, level: usize, color: egui::Color32) {
    let faces = [
        [[20.0, 3.0], [35.0, 11.0], [20.0, 19.0], [5.0, 11.0]],
        [[5.0, 11.0], [20.0, 19.0], [20.0, 36.0], [5.0, 28.0]],
        [[20.0, 19.0], [35.0, 11.0], [35.0, 28.0], [20.0, 36.0]],
    ];
    let rect = rect.shrink(4.0);
    let point = |face: [[f32; 2]; 4], u: f32, v: f32| {
        let value = |axis| {
            face[0][axis] * (1.0 - u) * (1.0 - v) + face[1][axis] * u * (1.0 - v) + face[2][axis] * u * v + face[3][axis] * (1.0 - u) * v
        };
        rect.min + Vec2::new(value(0) / 40.0 * rect.width(), value(1) / 40.0 * rect.height())
    };
    let line = |a, b| ui.painter().line_segment([a, b], egui::Stroke::new(1.0, color));
    for face in faces {
        for i in 0..4 {
            line(
                point(face, if i == 1 || i == 2 { 1.0 } else { 0.0 }, if i >= 2 { 1.0 } else { 0.0 }),
                point(
                    face,
                    if (i + 1) % 4 == 1 || (i + 1) % 4 == 2 { 1.0 } else { 0.0 },
                    if (i + 1) % 4 >= 2 { 1.0 } else { 0.0 },
                ),
            );
        }
        for i in 1..level {
            let f = i as f32 / level as f32;
            line(point(face, f, 0.0), point(face, f, 1.0));
            line(point(face, 0.0, f), point(face, 1.0, f));
        }
        for y in 0..level {
            for x in 0..level {
                line(
                    point(face, x as f32 / level as f32, y as f32 / level as f32),
                    point(face, (x + 1) as f32 / level as f32, (y + 1) as f32 / level as f32),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_angle_uses_the_degree_symbol_without_a_spurious_character() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut draft = crate::imports::tests::mesh_fixture();
        draft.options.normals = true;
        draft.options.crease = 105.0;
        let entry = Entry::new("mesh.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
        let mut imports = Imports {
            open: true,
            demo_started: true,
            choice: Choice::Mesh,
            active: Some(entry.id),
            entries: vec![entry],
            ..Default::default()
        };
        let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut output = None;
        for _ in 0..4 {
            let mut frame = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    ..Default::default()
                },
                |ui| {
                    crate::imports::ui::show(
                        ui.ctx(),
                        &theme.palette(),
                        &mut imports,
                        &mut Preferences::default(),
                        &world,
                        &Default::default(),
                    );
                },
            );
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let output = output.expect("settled frame");
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        assert!(
            labels.iter().any(|text| text.starts_with("105"))
                && labels.contains(&"\u{00b0}")
                && labels.iter().all(|text| !text.contains('\u{00c2}')),
            "numeric value and separate degree suffix: {labels:?}"
        );
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && (text.galley.text().starts_with("105") || text.galley.text() == "\u{00b0}")
            {
                assert!(
                    shape
                        .clip_rect
                        .contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())),
                    "the whole angle remains visible in the compact importer"
                );
            }
        }
    }

    #[test]
    fn editing_the_error_threshold_selects_the_lod_and_rebuilds_its_geometry() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut draft = crate::imports::tests::mesh_fixture();
        // A closed surface can reduce as the error rises; an isolated quad cannot.
        let model = &mut Arc::make_mut(&mut draft.scene).models[0];
        model.center = glam::Vec3::ZERO;
        model.size = glam::Vec3::ONE;
        model.faces.truncate(1);
        let face = &mut model.faces[0];
        let prototype = face.vertices[0].clone();
        face.vertices = [-0.5, 0.5]
            .into_iter()
            .flat_map(|x| {
                let prototype = prototype.clone();
                [-0.5, 0.5].into_iter().flat_map(move |y| {
                    let prototype = prototype.clone();
                    [-0.5, 0.5].into_iter().map(move |z| aurora_assets::import_mesh::Vertex {
                        position: [x, y, z],
                        normal: [0.0, 0.0, 1.0],
                        uv: [0.0; 2],
                        ..prototype.clone()
                    })
                })
            })
            .collect();
        face.indices = vec![
            0, 2, 3, 0, 3, 1, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 4, 6, 0, 6, 2, 1, 3, 7, 1, 7, 5,
        ];
        draft.lods[2].error_mode = true;
        draft.lods[2].error = 0.0;
        let prepared = Arc::new(mesh::prepare(&draft).expect("strict threshold"));
        let original = prepared.lods[0][2].as_ref().expect("medium LOD").triangles();
        draft.prepared = Some(prepared);
        let mut entry = Entry::new("mesh.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
        let mut prefs = Preferences::default();
        let mut frame = |events, entry: &mut Entry| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(1000.0);
                    show(ui, &theme.palette(), entry, &mut prefs, &mut Vec::new(), 430.0, |_, _| {});
                },
            );
            output.textures_delta.clear();
            output
        };
        let mut output = frame(Vec::new(), &mut entry);
        for _ in 0..3 {
            output = frame(Vec::new(), &mut entry);
        }
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text().ends_with(" %") => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .expect("error threshold field visible");
        for pressed in [true, false] {
            frame(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut entry,
            );
        }
        frame(
            vec![
                egui::Event::Text("100".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut entry,
        );
        let Some(Source::Mesh(draft)) = &entry.source else {
            panic!("mesh draft")
        };
        assert_eq!(
            draft.lods[2].error, 1.0,
            "the UI percent reaches the simplifier as a relative error"
        );
        assert_eq!(
            draft.preview_lod, 2,
            "editing Medium must display Medium rather than the unchanged High level"
        );
        assert!(draft.prepared.is_none(), "the cached geometry must be invalidated");
        assert!(entry.preview_generation > 0);
        let reduced = mesh::prepare(draft).expect("relaxed threshold");
        let count = reduced.lods[0][2].as_ref().expect("medium LOD").triangles();
        assert!(
            count < original,
            "relaxed threshold: {count} triangles, strict threshold: {original}"
        );
    }

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
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(1000.0);
                    show(ui, &theme.palette(), entry, &mut prefs, &mut Vec::new(), 430.0, |_, _| {});
                },
            );
            output.textures_delta.clear();
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
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(1000.0);
                    show(ui, &theme.palette(), entry, &mut prefs, &mut actions, 430.0, |_, _| {});
                },
            );
            output.textures_delta.clear();
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
        assert_eq!(
            entry.preview_generation, 1,
            "opening the source field also selects its card for the preview"
        );
        assert!(entry.preview_error.is_none());

        drop(frame(Vec::new(), &mut entry));
        let card = ctx
            .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_lod_card", entry.id, 1))))
            .expect("Low card");
        let low = card.right_bottom() - Vec2::splat(4.0);
        entry.preview_error = Some("previous preparation failure".into());
        for pressed in [true, false] {
            assert!(frame(events(low, pressed), &mut entry).1.is_empty());
        }
        let Some(Source::Mesh(draft)) = &entry.source else { panic!("mesh") };
        assert_eq!(draft.preview_lod, 1);
        assert_eq!(entry.preview_generation, 2);
        assert!(entry.preview_error.is_none(), "new preview selection can retry after an error");
    }
}
