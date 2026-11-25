//! Import actions pinned inside the Files and Folder preview tabs.
use super::*;

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    imports: &mut Imports,
    prefs: &mut Preferences,
    world: &crate::world::World,
    prices: &Prices,
    actions: &mut Vec<Action>,
) -> f32 {
    let footer_start = ui.cursor().top();
    #[cfg(test)]
    let ctx = ui.ctx().clone();
    let busy = imports.busy() || imports.confirm;
    let connected = imports.demo_started || world.login.is_some();
    widgets::divider(ui, p);
    if !imports.message.is_empty() {
        egui::ScrollArea::vertical()
            .id_salt("import_error_details")
            .max_height(100.0)
            .show(ui, |ui| {
                ui.label(RichText::new(&imports.message).color(p.danger));
            });
    }
    if let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|entry| entry.id == id))
        && !entry.local
        && let Some(Source::Mesh(draft)) = &entry.source
        && let Some(error) = draft.prepared.as_ref().and_then(|prepared| prepared.physics_error())
    {
        ui.add(egui::Label::new(RichText::new(error).color(p.warn)).wrap());
    }
    if prefs.show_payment && imports.spent > 0 {
        ui.label(RichText::new(aurora_i18n::tr!("import-paid", cost = imports.spent)).color(p.success));
    }
    ui.horizontal_wrapped(|ui| {
        let local = imports.choice == Choice::Local;
        ui.label(RichText::new(price_label(imports, prices, connected, world.agent_id)).strong());
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
            let count = imports.selected_local_uploads().count();
            let can_stage = count > 0 && !busy && imports.pending_files.is_none();
            let stage = action_button_if(
                ui,
                p,
                "file-upload-filled",
                &aurora_i18n::tr!("import-local-stage-upload", count = count),
                ActionKind::Flat,
                size,
                can_stage,
            )
            .on_hover_text(aurora_i18n::tr!("import-local-stage-upload-help"));
            #[cfg(test)]
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::unique("import_stage_local_uploads"),
                    (stage.rect, ui.clip_rect(), can_stage),
                );
            });
            if stage.clicked() {
                actions.push(Action::StageLocalUploads);
            }
            image_size_global(ui, imports, prefs, busy || stage.clicked());
        } else {
            if let Some(entry) = imports
                .active
                .and_then(|id| imports.entries.iter_mut().find(|entry| entry.id == id))
                && !entry.local
                && let Some(Source::Mesh(draft)) = &mut entry.source
                && draft.prepared.as_ref().is_some_and(|prepared| prepared.degenerate_triangles > 0)
            {
                let repair = action_button_if(
                    ui,
                    p,
                    "cube-3d-sphere",
                    aurora_i18n::tr!("import-physics-convex-repair"),
                    ActionKind::Flat,
                    size,
                    !busy && draft.pending.is_none(),
                )
                .on_hover_text(aurora_i18n::tr!("import-physics-convex-help"));
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique("import_mesh_repair_action"), (repair.rect, ui.clip_rect())));
                if repair.clicked() {
                    // This is an explicit collision-only approximation, not removal of
                    // rejected triangles (which could open holes in the original mesh).
                    draft.options.decompose = true;
                    draft.preview_physics = true;
                    draft.invalidate();
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("mesh_section", entry.id)), 1usize));
                    entry.invalidate_preview();
                }
            }
            if matches!(imports.choice, Choice::Mesh | Choice::Bulk)
                && let Some(entry) = imports.active.and_then(|id| imports.entries.iter().find(|e| e.id == id))
                && let Some(Source::Mesh(draft)) = &entry.source
            {
                let can_quote = !busy
                    && draft.pending.is_none()
                    && draft.prepared.as_ref().is_some_and(|prepared| prepared.degenerate_triangles == 0)
                    && imports.folder_plan_valid(world);
                let mut quote = action_button_if(
                    ui,
                    p,
                    "coin-filled",
                    aurora_i18n::tr!("import-calculate-fee"),
                    ActionKind::Flat,
                    size,
                    can_quote,
                );
                if let Some(error) = draft.prepared.as_ref().and_then(|prepared| prepared.physics_error()) {
                    quote = quote.on_disabled_hover_text(error);
                }
                #[cfg(test)]
                ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::unique("import_mesh_quote_action"),
                        (quote.rect, ui.clip_rect(), can_quote),
                    )
                });
                if quote.clicked() {
                    actions.push(Action::Quote(entry.id));
                }
            }
            let total = imports.total(prices);
            let selected = imports
                .entries
                .iter()
                .any(|e| imports.choice.accepts(e) && e.selected && matches!(e.status, Status::Ready));
            let can_upload = connected
                && !mesh_blocked(imports, connected, world.agent_id)
                && !busy
                && imports.pending_files.is_none()
                && imports.folder_plan_valid(world)
                && selected
                && total.is_some_and(|cost| cost == 0 || world.balance.is_some_and(|b| b >= cost));
            let upload = aurora_i18n::tr!("import-upload");
            let upload = action_button_if(ui, p, "file-upload-filled", upload, ActionKind::Primary, size, can_upload);
            #[cfg(test)]
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique("import_upload_action"), (upload.rect, ui.clip_rect(), can_upload)));
            if upload.clicked() {
                if needs_confirmation(imports, world, total) {
                    imports.confirm = true;
                    imports.confirm_choice = Some(imports.choice);
                } else {
                    actions.push(start_action(imports.choice));
                }
            }
            if matches!(
                imports.window_choice(),
                Choice::Bulk | Choice::Texture | Choice::Material | Choice::Mesh
            ) {
                // Freeze the size on the frame that confirms or starts the upload.
                image_size_global(ui, imports, prefs, busy || upload.clicked());
            }
        }
        if busy {
            ui.spinner();
            if flat_button(ui, p, aurora_i18n::tr!("import-cancel-remaining")).clicked() {
                actions.push(Action::Cancel);
            }
        }
    });
    ui.cursor().top() - footer_start
}

fn image_size_global(ui: &mut egui::Ui, imports: &mut Imports, prefs: &mut Preferences, busy: bool) {
    let old = prefs.limit_texture_size;
    let label = aurora_i18n::tr!("import-image-size-limit");
    let label_width = ui
        .painter()
        .layout_no_wrap(
            label.to_owned(),
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x;
    let gap = 16.0;
    let selector_width = 100.0;
    // Keep the label and selector together when the footer wraps, with extra
    // space separating them from the import action on a wide row.
    ui.allocate_ui_with_layout(
        Vec2::new(
            gap + label_width + ui.spacing().item_spacing.x + selector_width,
            ui.spacing().interact_size.y,
        ),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.add_space(gap);
            ui.add_enabled_ui(!busy, |ui| {
                let label = ui.label(label);
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(egui::Id::unique("import_image_size_global_label"), label.rect));
                let _ = label;
                let response = egui::ComboBox::from_id_salt("import_image_size_global")
                    .width(selector_width)
                    .selected_text(aurora_i18n::tr!(
                        "import-image-size-pixels",
                        size = if prefs.limit_texture_size { 1024 } else { 2048 }
                    ))
                    .show_ui(ui, |ui| {
                        for (limit, maximum) in [(true, 1024), (false, 2048)] {
                            let response = selection(
                                ui,
                                &mut prefs.limit_texture_size,
                                limit,
                                aurora_i18n::tr!("import-image-size-pixels", size = maximum),
                            );
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(egui::Id::unique(("import_image_size_global_choice", limit)), response.rect);
                            });
                            let _ = response;
                        }
                    })
                    .response
                    .on_hover_text(aurora_i18n::tr!("import-image-size-global-help"));
                #[cfg(test)]
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::unique("import_image_size_global"),
                        (response.rect, ui.clip_rect(), response.enabled()),
                    );
                });
                let _ = response;
            })
        },
    );
    if old != prefs.limit_texture_size {
        imports.sync_image_limits(prefs);
    }
}

/// The confirmation is a payment check: an upload the account's benefits make
/// free (L$0) starts at once, like the Legacy Viewer's upload buttons, which ask
/// nothing more than their price. It stays when it still has something to say:
/// the destination folders it will create.
fn needs_confirmation(imports: &Imports, world: &crate::world::World, total: Option<i32>) -> bool {
    total != Some(0) || (imports.folders.active() && !imports.folder_plan_ready(world))
}

#[cfg(test)]
mod tests;
