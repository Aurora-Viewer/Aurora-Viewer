//! Dedicated per-type floaters and the complete Multiple inspector.
//! Firestorm uses separate upload_image, upload_sound and upload_anim floaters
//! (llviewermenufile.cpp); Aurora retains a file list for each type.

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
    let connected = imports.demo_started || world.login.is_some();
    let window = imports.window_choice();
    if open {
        imports.inspector.get_or_insert(window);
    }
    let multiple = window == Choice::Bulk;
    let window_id = egui::Id::unique(window.window_id());
    if imports.demo_started && !imports.launcher_open && !imports.settings_open && imports.preview_zoom.is_none() {
        // Keep the requested demo inspector above other scripted floaters.
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, window_id));
    }
    let (size, min_size) = dimensions(window);
    let title = aurora_i18n::tr!("import-type-title", type = window.label());
    let mut floater = Floater::new(window.window_id(), title, IMPORT_POSITION, size);
    floater.min_size = min_size;
    floater.show(ctx, p, &mut open, |ui| {
        if multiple {
            let choices = [
                Choice::Texture,
                Choice::Sound,
                Choice::Animation,
                Choice::Material,
                Choice::Mesh,
                Choice::Bulk,
                Choice::Local,
            ];
            let mut selected = choices.iter().position(|choice| *choice == imports.choice).unwrap_or(0);
            let labels = choices.map(|choice| (choice.label(), !busy));
            if widgets::tabs(ui, p, &mut selected, &labels) {
                imports.select_type(choices[selected]);
            }
            widgets::divider(ui, p);
            if imports.choice == Choice::Local {
                local_settings(ui, p, prefs);
            }
        }
        ui.horizontal(|ui| {
            let choose = if imports.choice == Choice::Mesh {
                aurora_i18n::tr!("import-mesh-choose-high")
            } else {
                aurora_i18n::tr!("import-choose-files")
            };
            let size = Vec2::new(0.0, ui.spacing().interact_size.y);
            let can_choose = !busy && imports.pending_files.is_none();
            if action_button_if(ui, p, "folder-open-filled", choose, ActionKind::Flat, size, can_choose).clicked() {
                actions.push(Action::Files);
            }
            if imports.pending_files.is_some() {
                ui.spinner();
                ui.label(aurora_i18n::tr!("import-loading"));
            }
            if let Some(id) = imports.active {
                let reloadable = imports.entries.iter().any(|e| e.id == id && !e.from_live);
                if tool_button(ui, p, aurora_i18n::tr!("import-reload"), !busy && reloadable).clicked() {
                    actions.push(Action::Reload(id));
                }
                if tool_button(ui, p, aurora_i18n::tr!("common-remove"), !busy).clicked() {
                    actions.push(Action::Remove(id));
                }
            }
            if imports.entries.iter().any(|e| matches!(e.status, Status::Cancelled))
                && tool_button(ui, p, aurora_i18n::tr!("import-resume"), !busy).clicked()
            {
                actions.push(Action::Resume);
            }
            if imports.choice == Choice::Bulk {
                ui.add_enabled_ui(!busy, |ui| {
                    checkbox(ui, p, &mut imports.scale_bulk_1k, aurora_i18n::tr!("import-limit-bulk-1k"));
                });
            }
        });
        // Reserve the actual footer, independently of the outer Window's
        // remembered size. Subtracting a fixed margin from that size creates
        // a growth loop when a receipt, message or permission row appears.
        let extra_rows = usize::from(!imports.message.is_empty()) + usize::from(prefs.show_payment && imports.spent > 0);
        let extra_height = extra_rows as f32 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
        let footer_base = ctx.data_mut(|d| d.get_temp::<f32>(window_id.with("footer_base"))).unwrap_or(52.0);
        let height = (ui.available_height() - footer_base - extra_height).max(100.0);
        let files_width = match window {
            Choice::Material => 148.0,
            Choice::Animation => 160.0,
            _ => 180.0,
        };
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(files_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(files_width);
                widgets::section_header(ui, p, "files-filled", aurora_i18n::tr!("import-files"));
                egui::ScrollArea::vertical()
                    .id_salt("import_files")
                    .max_height(height - 30.0)
                    .show(ui, |ui| {
                        for entry in imports.entries.iter_mut().filter(|e| imports.choice.accepts(e)) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.spacing_mut().button_padding.x = 6.0;
                                ui.add_enabled_ui(!busy && matches!(entry.status, Status::Ready | Status::Error(_)), |ui| {
                                    // An unlabeled checkbox needs only its mark, not the trailing room of a text row.
                                    ui.spacing_mut().interact_size.y = ui.spacing().icon_width;
                                    checkbox(ui, p, &mut entry.selected, "");
                                });
                                let name_width = (ui.available_width() - 18.0 - ui.spacing().item_spacing.x).max(40.0);
                                if ui
                                    .add_sized(
                                        Vec2::new(name_width, 26.0),
                                        egui::Button::selectable(imports.active == Some(entry.id), "")
                                            .left_text(&entry.name)
                                            .truncate(),
                                    )
                                    .on_hover_text(if entry.from_live {
                                        aurora_i18n::tr!("blender-live-upload-frozen").to_string()
                                    } else {
                                        entry.path.to_string_lossy().into_owned()
                                    })
                                    .clicked()
                                {
                                    imports.active = Some(entry.id);
                                    if matches!(entry.source, Some(Source::Animation { .. })) {
                                        actions.push(Action::Play(entry.id, true));
                                    }
                                }
                                file_status(ui, p, entry);
                            });
                        }
                    });
            });
            let (separator, _) = ui.allocate_exact_size(Vec2::new(1.0, height), egui::Sense::hover());
            ui.painter()
                .vline(separator.center().x, separator.y_range(), widgets::divider_stroke(p));
            ui.allocate_ui_with_layout(
                Vec2::new((ui.available_width()).max(260.0), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let mesh_inspector = imports
                        .active
                        .and_then(|id| imports.entries.iter().find(|e| e.id == id))
                        .is_some_and(|entry| !entry.local && matches!(entry.source, Some(Source::Mesh(_))));
                    // the details stay in their column: while the scroll bar
                    // slides in, rows a frame behind its strip would otherwise
                    // widen the window a little each frame
                    let column = ui.max_rect();
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
                                    metadata_controls(
                                        ui,
                                        p,
                                        entry,
                                        &world.inventory,
                                        &inventory_prefs.protected,
                                        &mut imports.destination,
                                        imports.demo_mode == "destination",
                                    );
                                    ui.add_space(12.0);
                                }
                                if matches!(entry.source, Some(Source::Mesh(_))) {
                                    let remaining_height = (height - (ui.cursor().top() - detail_top)).max(100.0);
                                    mesh_panel::show(ui, p, entry, prefs, actions, remaining_height, |ui, entry| {
                                        metadata_controls(
                                            ui,
                                            p,
                                            entry,
                                            &world.inventory,
                                            &inventory_prefs.protected,
                                            &mut imports.destination,
                                            false,
                                        );
                                        if entry.from_live {
                                            ui.small(aurora_i18n::tr!("blender-live-upload-source"))
                                                .on_hover_text(aurora_i18n::tr!("blender-live-upload-frozen"));
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(aurora_i18n::tr!("blender-live-upload-materials")).small(),
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
                                            let settings_width = (ui.available_width() - 230.0 - ui.spacing().item_spacing.x).max(330.0);
                                            ui.horizontal_top(|ui| {
                                                ui.allocate_ui_with_layout(
                                                    Vec2::new(230.0, 360.0),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.set_width(230.0);
                                                        widgets::section_header(ui, p, "run", aurora_i18n::tr!("import-preview"));
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
                                                                egui::Id::unique(("animation_preview", entry.id)),
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
                                                            data.insert_temp(egui::Id::unique(("animation_canvas", entry.id)), canvas.rect);
                                                            data.insert_temp(egui::Id::unique(("animation_reset", entry.id)), reset.rect);
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
                                                                .seek(animation.local_time(f64::from(entry.player.position)) as f32);
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
                                                        animation_controls(ui, p, &mut header, bvh.as_ref(), animation.joints.len());
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
                                                !multiple,
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
                            if !multiple && entry.local && matches!(entry.source, Some(Source::Mesh(_))) {
                                local_settings(ui, p, prefs);
                            }
                            if multiple && !matches!(entry.source, Some(Source::Sound(_) | Source::Mesh(_) | Source::Material { .. })) {
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
                },
            );
        });
        let footer_start = ui.cursor().top();
        widgets::divider(ui, p);
        if !imports.message.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("import_error_details")
                .max_height(100.0)
                .show(ui, |ui| {
                    ui.label(RichText::new(&imports.message).color(p.danger));
                });
        }
        if prefs.show_payment && imports.spent > 0 {
            ui.label(RichText::new(aurora_i18n::tr!("import-paid", cost = imports.spent)).color(p.success));
        }
        ui.horizontal_wrapped(|ui| {
            let total = imports.total(prices);
            let local = imports.choice == Choice::Local;
            ui.label(RichText::new(price_label(imports, prices, connected, world.agent_id)).strong());
            let selected = imports
                .entries
                .iter()
                .any(|e| imports.choice.accepts(e) && e.selected && matches!(e.status, Status::Ready));
            let can_upload = connected
                && !mesh_blocked(imports, connected, world.agent_id)
                && !busy
                && selected
                && total.is_some_and(|cost| cost == 0 || world.balance.is_some_and(|b| b >= cost));
            // a wrapping row: the buttons stay in its layout
            let size = Vec2::new(0.0, ui.spacing().interact_size.y);
            if local {
                if let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && action_button_if(
                        ui,
                        p,
                        "check",
                        aurora_i18n::tr!("import-local-apply"),
                        ActionKind::Primary,
                        size,
                        !entry.local_pending && entry.source.is_some(),
                    )
                    .clicked()
                {
                    actions.push(Action::Local(entry.id));
                }
                if let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && entry.source.is_some()
                    && flat_button(ui, p, aurora_i18n::tr!("import-local-clear")).clicked()
                {
                    actions.push(Action::ClearLocal(entry.id));
                }
                if let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && entry.source.is_some()
                    && flat_button(ui, p, aurora_i18n::tr!("import-local-rez")).clicked()
                {
                    actions.push(Action::RezLocal(entry.id));
                }
            } else {
                if matches!(imports.choice, Choice::Mesh | Choice::Bulk)
                    && let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                    && let Some(Source::Mesh(draft)) = &entry.source
                    && action_button_if(
                        ui,
                        p,
                        "coin-filled",
                        aurora_i18n::tr!("import-calculate-fee"),
                        ActionKind::Flat,
                        size,
                        !busy && draft.pending.is_none(),
                    )
                    .clicked()
                {
                    actions.push(Action::Quote(entry.id));
                }
                let upload = aurora_i18n::tr!("import-upload");
                if action_button_if(ui, p, "file-upload-filled", upload, ActionKind::Primary, size, can_upload).clicked() {
                    imports.confirm = true;
                }
            }
            if busy {
                ui.spinner();
                if flat_button(ui, p, aurora_i18n::tr!("import-cancel-remaining")).clicked() {
                    actions.push(Action::Cancel);
                }
            }
        });
        ctx.data_mut(|d| {
            d.insert_temp(
                window_id.with("footer_base"),
                (ui.cursor().top() - footer_start - extra_height).max(0.0),
            )
        });
    });
    imports.open = open;
}

pub(super) fn dimensions(choice: Choice) -> (Vec2, Vec2) {
    match choice {
        Choice::Sound => (Vec2::new(780.0, 650.0), Vec2::new(740.0, 620.0)),
        Choice::Texture => (Vec2::new(820.0, 630.0), Vec2::new(740.0, 560.0)),
        Choice::Animation => (Vec2::new(780.0, 700.0), Vec2::new(760.0, 620.0)),
        Choice::Material => (Vec2::new(580.0, 790.0), Vec2::new(560.0, 720.0)),
        Choice::Mesh => (Vec2::new(1040.0, 720.0), Vec2::new(1020.0, 580.0)),
        Choice::Local => (Vec2::new(900.0, 720.0), Vec2::new(880.0, 580.0)),
        Choice::Bulk => (Vec2::new(900.0, 760.0), Vec2::new(880.0, 580.0)),
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
