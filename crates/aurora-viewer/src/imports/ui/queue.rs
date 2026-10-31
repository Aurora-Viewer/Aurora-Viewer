//! One mixed queue; type icons filter visibility without changing upload selection.
use super::*;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, imports: &mut Imports, height: f32, busy: bool, actions: &mut Vec<Action>) {
    // Queue changes must not become a minimum size for the shared window.
    let (bounds, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let mut pane = ui.new_child(egui::UiBuilder::new().id_salt("import_queue_pane").max_rect(bounds));
    pane.set_clip_rect(bounds.intersect(ui.clip_rect()));
    let ui = &mut pane;
    let top = ui.cursor().top();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (index, (choice, icon)) in [
            (Choice::Texture, "photo-filled"),
            (Choice::Sound, "file-music-filled"),
            (Choice::Animation, "run"),
            (Choice::Material, "droplet-filled"),
            (Choice::Mesh, "cube-3d-sphere"),
        ]
        .into_iter()
        .enumerate()
        {
            if widgets::icon_toggle_sized(ui, p, icon, choice.label(), imports.type_filters[index], 30.0) {
                imports.type_filters[index] = !imports.type_filters[index];
            }
        }
        if widgets::icon_toggle_sized(ui, p, "eye-filled", Choice::Local.label(), imports.choice == Choice::Local, 30.0) {
            imports.choice = if imports.choice == Choice::Local {
                Choice::Bulk
            } else {
                Choice::Local
            };
            imports.active = None;
        }
    });
    ui.add_space(6.0);
    let can_choose = !busy && imports.pending_files.is_none();
    let size = Vec2::new(ui.available_width(), 28.0);
    if action_button_if(
        ui,
        p,
        "files-filled",
        aurora_i18n::tr!("import-choose-files"),
        ActionKind::Flat,
        size,
        can_choose,
    )
    .clicked()
    {
        actions.push(Action::Files);
    }
    if imports.choice != Choice::Local
        && action_button_if(
            ui,
            p,
            "folder-open-filled",
            aurora_i18n::tr!("import-choose-folder"),
            ActionKind::Flat,
            size,
            can_choose,
        )
        .clicked()
    {
        actions.push(Action::Folder);
    }
    if imports.pending_files.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(aurora_i18n::tr!("import-loading"));
        });
    }
    imports.reconcile_active();
    let rows = imports.visible_indices();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let count = format!(
            "{}/{}",
            rows.len(),
            imports
                .entries
                .iter()
                .filter(|e| e.local == (imports.choice == Choice::Local))
                .count()
        );
        let count_width = ui
            .painter()
            .layout_no_wrap(count.clone(), egui::TextStyle::Body.resolve(ui.style()), p.muted)
            .size()
            .x;
        widgets::glyph(ui, "files-filled", p.violet_light, 17.0);
        let title_width = (ui.available_width() - count_width - 20.0 - ui.spacing().item_spacing.x * 2.0).max(1.0);
        ui.add_sized(
            Vec2::new(title_width, 20.0),
            egui::Label::new(RichText::new(aurora_i18n::tr!("import-queue")).size(13.0).strong().color(p.ink)).truncate(),
        )
        .on_hover_text(aurora_i18n::tr!("import-queue"));
        ui.label(RichText::new(count).color(p.muted));
        let can_clear = can_choose && imports.entries.iter().any(|entry| imports.choice.accepts(entry));
        let clear = action_button_if(ui, p, "trash-filled", "", ActionKind::Flat, Vec2::splat(20.0), can_clear)
            .on_hover_text(aurora_i18n::tr!("import-queue-clear"));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique("import_queue_clear"), clear.rect));
        if clear.clicked() {
            // Reuse ordinary removal, including preview/quote/resource cleanup.
            actions.extend(
                imports
                    .entries
                    .iter()
                    .filter(|entry| imports.choice.accepts(entry))
                    .map(|entry| Action::Remove(entry.id)),
            );
        }
    });
    ui.add_space(4.0);
    let row_height = ui.spacing().interact_size.y.max(28.0);
    let footer_key = ui.scope_id().with("footer_height");
    let footer_height = ui.ctx().data(|data| data.get_temp::<f32>(footer_key)).unwrap_or(110.0);
    let room = (height - (ui.cursor().top() - top) - footer_height).max(0.0);
    egui::ScrollArea::vertical()
        .id_salt("import_queue")
        .max_height(room)
        .min_scrolled_height(0.0)
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            if rows.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("import-queue-empty")).color(p.muted));
            }
            for &index in &rows[range] {
                let entry = &mut imports.entries[index];
                ui.push_id(entry.id, |ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), row_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add_enabled_ui(!busy && matches!(entry.status, Status::Ready | Status::Error(_)), |ui| {
                                checkbox(ui, p, &mut entry.selected, "");
                            });
                            let kind = Imports::type_choice(entry);
                            let icon = match kind {
                                Choice::Texture => "photo-filled",
                                Choice::Sound => "file-music-filled",
                                Choice::Animation => "run",
                                Choice::Material => "droplet-filled",
                                Choice::Mesh => "cube-3d-sphere",
                                _ => "file-description-filled",
                            };
                            widgets::glyph(ui, icon, p.violet_light, 20.0);
                            let selected = imports.active == Some(entry.id);
                            let show_status = !matches!(entry.status, Status::Ready);
                            let status_width = if show_status { 18.0 + ui.spacing().item_spacing.x } else { 0.0 };
                            let response = ui.add_sized(
                                Vec2::new((ui.available_width() - status_width).max(1.0), row_height),
                                egui::Button::selectable(selected, "").left_text(&entry.name).truncate(),
                            );
                            let rect = response.rect;
                            if show_status {
                                file_status(ui, p, entry);
                            }
                            let text = status(entry);
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(egui::Id::unique(("import_queue_row", entry.id)), rect));
                            if response.on_hover_text(format!("{}\n{text}", entry.path.display())).clicked() {
                                imports.active = Some(entry.id);
                                imports.tab = Tab::Batch;
                                if matches!(entry.source, Some(Source::Animation { .. })) {
                                    actions.push(Action::Play(entry.id, true));
                                }
                            }
                        },
                    )
                });
            }
        });
    let footer_top = ui.cursor().top();
    ui.horizontal_wrapped(|ui| {
        if let Some(id) = imports.active {
            if tool_button(
                ui,
                p,
                aurora_i18n::tr!("import-reload"),
                !busy && imports.entries.iter().any(|e| e.id == id && !e.from_live),
            )
            .clicked()
            {
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
    });
    ui.add_enabled_ui(!busy, |ui| {
        checkbox(ui, p, &mut imports.scale_bulk_1k, aurora_i18n::tr!("import-limit-bulk-1k"));
    });
    ui.ctx()
        .data_mut(|data| data.insert_temp(footer_key, ui.cursor().top() - footer_top + ui.spacing().item_spacing.y));
}
