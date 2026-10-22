//! On-demand Legacy Viewer / Alchemy set picker; native selection changes
//! only after AIS success.

use super::*;

pub(super) fn show(ctx: &egui::Context, p: &Palette, catalog: &inventory::Catalog, state: &mut AoUi, out: &mut Vec<Action>) {
    let Some(migration) = state.migration.as_mut() else { return };
    let mut cancel = false;
    let modal = widgets::Modal::new(egui::Id::unique("ao_migrate_firestorm")).show(ctx, |ui| {
        ui.set_width(450.0);
        ui.label(RichText::new(aurora_i18n::tr!("ao-migrate")).strong().color(p.ink));
        ui.label(aurora_i18n::tr!("ao-migrate-help"));
        ui.add_space(4.0);
        let mut alchemy = migration.alchemy;
        ui.horizontal(|ui| {
            widgets::radio_value(ui, p, &mut alchemy, false, aurora_i18n::tr!("ao-migrate-legacy"));
            widgets::radio_value(ui, p, &mut alchemy, true, aurora_i18n::tr!("ao-migrate-alchemy"));
        });
        migration.set_alchemy(alchemy);
        let folder = migration.viewer().path();
        if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-migrate-reload")).clicked() {
            out.push(Action::ReloadMigration);
        }
        ui.add_space(8.0);
        if migration.source.failed {
            ui.label(RichText::new(aurora_i18n::tr!("ao-fetch-failed")).color(p.warn));
        } else if !migration.ready() {
            ui.label(aurora_i18n::tr!("ao-loading"));
        } else if migration.source.sets.is_empty() {
            ui.label(migration.missing());
        } else {
            ui.label(aurora_i18n::tr!("ao-migrate-source", folder = folder));
            let mut picked = None;
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for set in &migration.source.sets {
                    if ui.selectable_label(set.id == migration.selected, &set.name).clicked() {
                        picked = Some(set.id);
                    }
                }
            });
            if let Some(id) = picked {
                migration.select(id);
            }
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("ao-migrate-destination"));
            ui.add(egui::TextEdit::singleline(&mut migration.name).desired_width(f32::INFINITY));
            if let Err(reason) = migration.validate(catalog) {
                ui.label(RichText::new(reason).color(p.warn));
            }
        }
        if !migration.message.is_empty() {
            ui.label(RichText::new(&migration.message).color(p.warn));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if widgets::action_button_if(
                ui,
                p,
                "copy",
                aurora_i18n::tr!("ao-migrate-confirm"),
                widgets::ActionKind::Primary,
                Vec2::new(0.0, 28.0),
                migration.validate(catalog).is_ok(),
            )
            .clicked()
            {
                out.push(Action::Migrate);
            }
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                cancel = true;
            }
        });
    });
    if cancel || modal.should_close() {
        state.migration = None;
    }
}
