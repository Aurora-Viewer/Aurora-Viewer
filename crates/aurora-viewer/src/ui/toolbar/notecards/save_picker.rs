//! A preset can be named or chosen explicitly for replacement before saving.
use super::library::Catalog;
use crate::{theme::Palette, ui::widgets};
use uuid::Uuid;

pub(super) fn show(ctx: &egui::Context, p: &Palette, catalog: &Catalog, name: &mut String, selected: &mut Option<Uuid>) -> (bool, bool) {
    let mut close = false;
    let mut save = false;
    let modal = widgets::Modal::new(egui::Id::unique("toolbar_notecard_save")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-preset-save-title")).color(p.violet_light));
        ui.label(aurora_i18n::tr!("toolbar-preset-save-name"));
        let field = ui.add(egui::TextEdit::singleline(name).char_limit(63).desired_width(ui.available_width()));
        #[cfg(test)]
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_note_name"), field.rect));
        if field.changed() {
            *selected = None;
        }
        if catalog.loading {
            ui.label(aurora_i18n::tr!("common-loading"));
        } else if !catalog.entries.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("toolbar_save_presets")
                .max_height(180.0)
                .show(ui, |ui| {
                    for entry in &catalog.entries {
                        let active = *selected == Some(entry.id) && name.trim() == entry.name;
                        let response = ui
                            .add_enabled_ui(entry.writable, |ui| {
                                let width = ui.available_width();
                                widgets::action_button(
                                    ui,
                                    p,
                                    "file-text-filled",
                                    &entry.name,
                                    if active {
                                        widgets::ActionKind::Primary
                                    } else {
                                        widgets::ActionKind::Flat
                                    },
                                    egui::vec2(width, 30.0),
                                )
                            })
                            .inner;
                        #[cfg(test)]
                        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("toolbar_save_preset", entry.id)), response.rect));
                        if response.clicked() {
                            *selected = Some(entry.id);
                            name.clone_from(&entry.name);
                        }
                    }
                });
        }
        let target = catalog.target(name, *selected);
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            match &target {
                Ok(Some(_)) => {
                    ui.label(aurora_i18n::tr!("toolbar-preset-save-replace"));
                }
                Ok(None) => {
                    ui.label(aurora_i18n::tr!("toolbar-preset-save-new"));
                }
                Err(reason) if !name.trim().is_empty() && !catalog.loading => {
                    ui.label(egui::RichText::new(reason).color(p.muted));
                }
                _ => {}
            }
            let row = egui::vec2(240.0 + ui.spacing().item_spacing.x, 30.0);
            ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                close = ui
                    .add_sized(egui::vec2(120.0, 30.0), egui::Button::new(aurora_i18n::tr!("common-cancel")))
                    .clicked();
                let button = widgets::default_button(
                    ui.add_enabled_ui(target.is_ok(), |ui| {
                        ui.add_sized(egui::vec2(120.0, 30.0), egui::Button::new(aurora_i18n::tr!("common-save")))
                    })
                    .inner,
                );
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_note_save"), button.rect));
                save = button.clicked();
            });
        });
    });
    (close || modal.should_close(), save)
}
