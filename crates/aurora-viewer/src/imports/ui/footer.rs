//! Import actions pinned inside the Files and Folder preview tabs.
use super::*;

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    imports: &mut Imports,
    prefs: &Preferences,
    world: &crate::world::World,
    inventory_prefs: &crate::ui::inventory::InventoryPreferences,
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
            && imports.pending_files.is_none()
            && imports.folder_plan_valid(world, inventory_prefs)
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
                    !busy && draft.pending.is_none() && imports.folder_plan_valid(world, inventory_prefs),
                )
                .clicked()
            {
                actions.push(Action::Quote(entry.id));
            }
            let upload = aurora_i18n::tr!("import-upload");
            let upload = action_button_if(ui, p, "file-upload-filled", upload, ActionKind::Primary, size, can_upload);
            #[cfg(test)]
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique("import_upload_action"), (upload.rect, ui.clip_rect(), can_upload)));
            if upload.clicked() {
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
    ui.cursor().top() - footer_start
}

#[cfg(test)]
mod tests;
