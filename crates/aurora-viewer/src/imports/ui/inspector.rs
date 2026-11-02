//! Unified and type-specific windows share the existing per-asset inspectors.

use super::*;

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    imports: &mut Imports,
    prefs: &mut Preferences,
    world: &crate::world::World,
    inventory_prefs: &crate::ui::inventory::InventoryPreferences,
    prices: &Prices,
    actions: &mut Vec<Action>,
) {
    let mut open = imports.open;
    let busy = imports.busy() || imports.confirm;
    let window = imports.window_choice();
    let unified = window == Choice::Bulk;
    let window_id = egui::Id::unique(window.window_id());
    if imports.demo_started && !imports.launcher_open && imports.preview_zoom.is_none() {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, window_id));
    }
    let (size, min_size) = dimensions(window);
    let title = if unified {
        aurora_i18n::tr!("import-window-title").to_owned()
    } else {
        aurora_i18n::tr!("import-type-title", type = window.label())
    };
    let offset = window.window_index().map_or(0.0, |index| 24.0 * index as f32);
    let mut floater = Floater::new(window.window_id(), title, IMPORT_POSITION + Vec2::splat(offset), size).steady_size();
    floater.min_size = min_size;
    floater.show(ctx, p, &mut open, |ui| {
        let height = ui.available_height().max(100.0);
        let files_width = (ui.available_width() - 840.0).clamp(176.0, 228.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(files_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(files_width);
                queue::show(ui, p, imports, prefs, height, busy, actions);
            });
            let (separator, _) = ui.allocate_exact_size(Vec2::new(1.0, height), egui::Sense::hover());
            ui.painter()
                .vline(separator.center().x, separator.y_range(), widgets::divider_stroke(p));
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width()).max(260.0), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let (bounds, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
                    let mut right = ui.new_child(egui::UiBuilder::new().id_salt("import_right_pane").max_rect(bounds));
                    right.set_clip_rect(bounds.intersect(ui.clip_rect()));
                    let ui = &mut right;
                    let mut tab = if unified {
                        imports.tab as usize
                    } else {
                        usize::from(imports.tab == Tab::Permissions)
                    };
                    let unified_tabs = [
                        (aurora_i18n::tr!("import-tab-batch"), true),
                        (aurora_i18n::tr!("import-tab-folder"), true),
                        (aurora_i18n::tr!("import-tab-permissions"), true),
                    ];
                    let type_tabs = [
                        (aurora_i18n::tr!("import-tab-batch"), true),
                        (aurora_i18n::tr!("import-tab-permissions"), true),
                    ];
                    let tabs: &[(&str, bool)] = if unified { &unified_tabs } else { &type_tabs };
                    widgets::tabs(ui, p, &mut tab, tabs);
                    imports.tab = match (unified, tab) {
                        (true, 1) => Tab::Folder,
                        (true, 2) | (false, 1) => Tab::Permissions,
                        _ => Tab::Batch,
                    };
                    widgets::divider(ui, p);
                    if imports.tab != Tab::Permissions {
                        folder_panel::destination_row(ui, p, imports, prefs, world, inventory_prefs, busy);
                        widgets::divider(ui, p);
                    }
                    let remaining = egui::Rect::from_min_max(ui.next_widget_position(), bounds.max);
                    let has_footer = imports.tab != Tab::Permissions;
                    let footer_key = window_id.with("tab_footer_height");
                    let footer_height = if has_footer {
                        ctx.data(|data| data.get_temp::<f32>(footer_key))
                            .unwrap_or(52.0)
                            .min((remaining.height() - 100.0).max(0.0))
                    } else {
                        0.0
                    };
                    let content =
                        egui::Rect::from_min_max(remaining.min, egui::pos2(remaining.right(), remaining.bottom() - footer_height));
                    let mut content_ui = ui.new_child(egui::UiBuilder::new().id_salt("import_tab_content").max_rect(content));
                    content_ui.set_clip_rect(content.intersect(ui.clip_rect()));
                    let height = content.height();
                    {
                        let ui = &mut content_ui;
                        match imports.tab {
                            Tab::Folder => {
                                egui::ScrollArea::vertical()
                                    .id_salt("import_folders_tab")
                                    .max_height(height)
                                    .show(ui, |ui| {
                                        folder_panel::show(ui, p, imports, world, height.max(500.0), busy);
                                    });
                            }
                            Tab::Permissions => {
                                egui::ScrollArea::both()
                                    .id_salt("import_permissions_tab")
                                    .max_height(height)
                                    .show(ui, |ui| {
                                        ui.add_enabled_ui(!busy, |ui| permission_panel::show(ui, p, prefs));
                                    });
                            }
                            Tab::Batch => {
                                let mesh_inspector = imports
                                    .active
                                    .and_then(|id| imports.entries.iter().find(|e| e.id == id))
                                    .is_some_and(|entry| !entry.local && matches!(entry.source, Some(Source::Mesh(_))));
                                // the details stay in their column: while the scroll bar
                                // slides in, rows a frame behind its strip would otherwise
                                // widen the window a little each frame
                                let column = egui::Rect::from_min_max(ui.next_widget_position(), ui.max_rect().max);
                                let mut column_ui = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(column)
                                        .layout(egui::Layout::top_down(egui::Align::Min)),
                                );
                                column_ui.set_clip_rect(column.intersect(ui.clip_rect()));
                                let mut show_details = |ui: &mut egui::Ui| {
                                    let detail_top = ui.cursor().top();
                                    if let Some(entry) = imports.active.and_then(|id| imports.entries.iter_mut().find(|e| e.id == id)) {
                                        apply_preview_colors(&mut entry.view, p, prefs);
                                        let metadata = (entry.name.clone(), entry.description.clone(), entry.folder);
                                        let preview_generation = entry.preview_generation;
                                        let preview_error = entry.preview_error.clone();
                                        if entry.local {
                                            checkbox(ui, p, &mut entry.local_live, aurora_i18n::tr!("import-local-live"));
                                        }
                                        if let Status::Done(item) = entry.status
                                            && flat_button(ui, p, aurora_i18n::tr!("import-open-inventory")).clicked()
                                        {
                                            actions.push(Action::Open(item));
                                        }
                                        if entry.source.is_some()
                                            && matches!(entry.status, Status::Error(_))
                                            && tool_button(ui, p, aurora_i18n::tr!("import-restore-draft"), !busy).clicked()
                                        {
                                            actions.push(Action::RestoreDraft(entry.id));
                                        }
                                        ui.add_enabled_ui(!busy && matches!(entry.status, Status::Ready), |ui| {
                                            if !matches!(entry.source, Some(Source::Mesh(_))) {
                                                metadata_controls(ui, entry);
                                                ui.add_space(12.0);
                                            }
                                            if matches!(entry.source, Some(Source::Mesh(_))) {
                                                let remaining_height = (height - (ui.cursor().top() - detail_top)).max(100.0);
                                                mesh_panel::show(ui, p, entry, prefs, actions, remaining_height, |ui, entry| {
                                                    metadata_controls(ui, entry);
                                                    if entry.from_live {
                                                        ui.small(aurora_i18n::tr!("blender-live-upload-source"))
                                                            .on_hover_text(aurora_i18n::tr!("blender-live-upload-frozen"));
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(aurora_i18n::tr!("blender-live-upload-materials"))
                                                                    .small(),
                                                            )
                                                            .wrap(),
                                                        );
                                                    }
                                                });
                                            } else if let Some(source) = &mut entry.source {
                                                match source {
                                                    Source::Texture(image) => texture(
                                                        ui,
                                                        p,
                                                        entry.id,
                                                        image,
                                                        entry.empty_alpha,
                                                        &mut entry.options,
                                                        prefs,
                                                        &entry.preview,
                                                        actions,
                                                        &mut entry.preview_generation,
                                                        &mut entry.texture_target,
                                                        &entry.model,
                                                        &mut entry.view,
                                                    ),
                                                    Source::Sound(sound) => sound_panel::show(
                                                        ui,
                                                        p,
                                                        entry.id,
                                                        &entry.name,
                                                        sound,
                                                        &mut entry.sound,
                                                        imports.sound_limit,
                                                        actions,
                                                    ),
                                                    Source::Animation { animation, bvh, .. } => {
                                                        let old_base = entry.player.base;
                                                        let settings_width =
                                                            (ui.available_width() - 230.0 - ui.spacing().item_spacing.x).max(330.0);
                                                        ui.horizontal_top(|ui| {
                                                            ui.allocate_ui_with_layout(
                                                                Vec2::new(230.0, 360.0),
                                                                egui::Layout::top_down(egui::Align::Min),
                                                                |ui| {
                                                                    ui.set_width(230.0);
                                                                    widgets::section_header(
                                                                        ui,
                                                                        p,
                                                                        "run",
                                                                        aurora_i18n::tr!("import-preview"),
                                                                    );
                                                                    let bases = [
                                                                        aurora_i18n::tr!("import-base-standing"),
                                                                        aurora_i18n::tr!("import-base-walking"),
                                                                        aurora_i18n::tr!("import-base-sitting"),
                                                                        aurora_i18n::tr!("import-base-flying"),
                                                                    ];
                                                                    ui.vertical(|ui| {
                                                                        ui.label(aurora_i18n::tr!("import-animation-base"));
                                                                        egui::ComboBox::from_id_salt("animation_base")
                                                                            .selected_text(bases[entry.player.base.min(3)])
                                                                            .show_ui(ui, |ui| {
                                                                                for (i, label) in bases.into_iter().enumerate() {
                                                                                    selection(ui, &mut entry.player.base, i, label);
                                                                                }
                                                                            });
                                                                    })
                                                                    .response
                                                                    .on_hover_text(aurora_i18n::tr!("import-animation-base-help"));
                                                                    let canvas = if let Some(model) = &entry.model {
                                                                        aurora_render::preview::paint_with_pose(
                                                                            ui,
                                                                            ui.scope_id().with(("animation_preview", entry.id)),
                                                                            model.clone(),
                                                                            &mut entry.view,
                                                                            Vec2::new(230.0, 190.0),
                                                                            entry.player.pose.clone(),
                                                                        )
                                                                    } else {
                                                                        widgets::picture(ui, p, None, Vec2::new(230.0, 190.0), "run", "")
                                                                    };
                                                                    let reset = preview_camera_controls(ui, p, &mut entry.view);
                                                                    ui.ctx().data_mut(|data| {
                                                                        data.insert_temp(
                                                                            egui::Id::unique(("animation_canvas", entry.id)),
                                                                            canvas.rect,
                                                                        );
                                                                        data.insert_temp(
                                                                            egui::Id::unique(("animation_reset", entry.id)),
                                                                            reset.rect,
                                                                        );
                                                                    });
                                                                    actions.extend(animation_transport(
                                                                        ui,
                                                                        p,
                                                                        entry.id,
                                                                        &mut entry.player,
                                                                        animation,
                                                                        &mut prefs.preview_on_avatar,
                                                                    ));
                                                                    if old_base != entry.player.base {
                                                                        entry
                                                                            .player
                                                                            .seek(animation.local_time(f64::from(entry.player.position))
                                                                                as f32);
                                                                    }
                                                                },
                                                            );
                                                            ui.allocate_ui_with_layout(
                                                                Vec2::new(settings_width, 360.0),
                                                                egui::Layout::top_down(egui::Align::Min),
                                                                |ui| {
                                                                    ui.set_width(settings_width);
                                                                    animation_model_controls(
                                                                        ui,
                                                                        p,
                                                                        entry.id,
                                                                        &entry.animation_model,
                                                                        &prefs.animation_models,
                                                                        actions,
                                                                    );
                                                                    let before = (
                                                                        animation.base_priority,
                                                                        animation.looping,
                                                                        animation.loop_in,
                                                                        animation.loop_out,
                                                                        animation.ease_in,
                                                                        animation.ease_out,
                                                                        animation.hand_pose,
                                                                        animation.emote_name.clone(),
                                                                    );
                                                                    let mut header = super::animation::metadata(animation);
                                                                    animation_controls(
                                                                        ui,
                                                                        p,
                                                                        &mut header,
                                                                        bvh.as_ref(),
                                                                        animation.joints.len(),
                                                                    );
                                                                    let changed = before
                                                                        != (
                                                                            header.base_priority,
                                                                            header.looping,
                                                                            header.loop_in,
                                                                            header.loop_out,
                                                                            header.ease_in,
                                                                            header.ease_out,
                                                                            header.hand_pose,
                                                                            header.emote_name.clone(),
                                                                        );
                                                                    if changed {
                                                                        if animation.hand_pose != header.hand_pose
                                                                            || animation.emote_name != header.emote_name
                                                                        {
                                                                            entry.model = None;
                                                                            entry.preview_generation += 1;
                                                                            entry.preview_error = None;
                                                                        }
                                                                        let current = Arc::make_mut(animation);
                                                                        header.joints = std::mem::take(&mut current.joints);
                                                                        if current.base_priority != header.base_priority {
                                                                            for joint in &mut header.joints {
                                                                                joint.priority = header.base_priority;
                                                                            }
                                                                        }
                                                                        *current = header;
                                                                    }
                                                                    if changed && entry.player.bound.is_some() {
                                                                        actions.push(Action::Play(entry.id, true));
                                                                    }
                                                                },
                                                            );
                                                        });
                                                    }
                                                    Source::Material { value, images } => {
                                                        entry.view.material = Some(super::preview::material(value));
                                                        let old = value.clone();
                                                        let old_maps = images.each_ref().map(|image| image.as_ref().map(|i| i.key));
                                                        let old_shape = entry.material_shape;
                                                        let compact = ui.available_width() < 650.0;
                                                        material_panel::show(
                                                            ui,
                                                            p,
                                                            entry.id,
                                                            value,
                                                            images,
                                                            actions,
                                                            &mut entry.material_shape,
                                                            &entry.material_previews,
                                                            prefs,
                                                            entry.model.as_ref().filter(|_| imports.demo_mode != "material-loading"),
                                                            &mut entry.view,
                                                            compact,
                                                        );
                                                        if old != *value
                                                            || old_shape != entry.material_shape
                                                            || old_maps != images.each_ref().map(|image| image.as_ref().map(|i| i.key))
                                                        {
                                                            entry.preview_generation += 1;
                                                        }
                                                        if old_shape != entry.material_shape
                                                            || old_maps != images.each_ref().map(|image| image.as_ref().map(|i| i.key))
                                                        {
                                                            entry.model = None;
                                                            entry.preview_error = None;
                                                        }
                                                    }
                                                    Source::Mesh(_) => {}
                                                }
                                            }
                                        });
                                        if metadata != (entry.name.clone(), entry.description.clone(), entry.folder) {
                                            if let Some(Source::Mesh(draft)) = &mut entry.source {
                                                draft.invalidate_quote();
                                            }
                                            entry.preview_generation += 1;
                                        }
                                        if entry.preview_generation != preview_generation
                                            && let Some(error) = preview_error
                                            && imports.message == error
                                            && entry.preview_error.is_none()
                                        {
                                            imports.message.clear();
                                        }
                                        if entry.local && matches!(entry.source, Some(Source::Mesh(_))) {
                                            local_settings(ui, p, prefs);
                                        }
                                        if !matches!(entry.source, Some(Source::Sound(_) | Source::Mesh(_) | Source::Material { .. })) {
                                            preview_colors(ui, p, prefs);
                                        }
                                    } else {
                                        ui.vertical_centered(|ui| {
                                            ui.add_space(70.0);
                                            ui.label(RichText::new(aurora_i18n::tr!("import-empty")).color(p.muted));
                                        });
                                    }
                                };
                                if mesh_inspector {
                                    // Mesh owns a bounded viewport between its header and summary.
                                    // An outer scroll bar would feed its width back into that layout.
                                    show_details(&mut column_ui);
                                } else {
                                    egui::ScrollArea::vertical()
                                        .id_salt("import_details")
                                        .max_height(height)
                                        .show(&mut column_ui, show_details);
                                }
                                ui.allocate_rect(column_ui.min_rect().intersect(column), egui::Sense::hover());
                            }
                        }
                    }
                    if has_footer {
                        let footer_rect = egui::Rect::from_min_max(egui::pos2(remaining.left(), content.bottom()), remaining.max);
                        let mut footer_ui = ui.new_child(egui::UiBuilder::new().id_salt("import_tab_footer").max_rect(footer_rect));
                        footer_ui.set_clip_rect(footer_rect.intersect(ui.clip_rect()));
                        let measured = footer::show(&mut footer_ui, p, imports, prefs, world, inventory_prefs, prices, actions);
                        ctx.data_mut(|data| data.insert_temp(footer_key, measured));
                    }
                },
            );
        });
    });
    imports.open = open;
}

pub(super) fn dimensions(choice: Choice) -> (Vec2, Vec2) {
    match choice {
        Choice::Texture => (Vec2::new(820.0, 700.0), Vec2::new(740.0, 580.0)),
        Choice::Sound => (Vec2::new(780.0, 680.0), Vec2::new(740.0, 580.0)),
        Choice::Animation => (Vec2::new(900.0, 740.0), Vec2::new(840.0, 640.0)),
        Choice::Material => (Vec2::new(720.0, 790.0), Vec2::new(620.0, 660.0)),
        Choice::Mesh => (Vec2::new(1040.0, 780.0), Vec2::new(1020.0, 650.0)),
        Choice::Local => (Vec2::new(1040.0, 780.0), Vec2::new(900.0, 650.0)),
        Choice::Bulk => (Vec2::new(1160.0, 820.0), Vec2::new(1060.0, 650.0)),
    }
}

fn local_settings(ui: &mut egui::Ui, p: &Palette, prefs: &mut Preferences) {
    fold(ui, p, aurora_i18n::tr!("import-local-settings"), false, |ui| {
        ui.horizontal_wrapped(|ui| {
            checkbox(ui, p, &mut prefs.local_assume_meters, aurora_i18n::tr!("import-local-meters"));
            checkbox(ui, p, &mut prefs.local_mesh_auto_reload, aurora_i18n::tr!("import-local-live"));
            ui.add(
                egui::DragValue::new(&mut prefs.local_reload_seconds)
                    .range(0.1..=3600.0)
                    .suffix(" s"),
            );
        });
    });
}
