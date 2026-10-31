//! Shared source wrapper controls below the inventory destination.

use super::*;

#[derive(Default, Clone)]
struct Editor {
    source: String,
    applied: String,
    text: String,
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, imports: &mut Imports, prefs: &mut Preferences) {
    let key = egui::Id::unique("import_source_root_editor");
    let mut editor = ui.ctx().data_mut(|data| data.get_temp::<Editor>(key)).unwrap_or_default();
    let sources = imports.source_roots();
    if !sources.contains(&editor.source) {
        editor.source = sources.first().cloned().unwrap_or_default();
        editor.applied.clear();
    }
    ui.horizontal_wrapped(|ui| {
        let response = checkbox(
            ui,
            p,
            &mut prefs.include_source_folder,
            aurora_i18n::tr!("import-include-source-folder"),
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique("import_include_source_folder"), response.rect));
        #[cfg(not(test))]
        let _ = response;
        if sources.is_empty() {
            return;
        }
        ui.add_enabled_ui(prefs.include_source_folder, |ui| {
            if sources.len() > 1 {
                egui::ComboBox::from_id_salt("import_source_root")
                    .selected_text(&editor.source)
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for source in &sources {
                            if ui.selectable_value(&mut editor.source, source.clone(), source).changed() {
                                editor.applied.clear();
                            }
                        }
                    });
            }
            let name = imports.folders.roots.name(&editor.source);
            if editor.applied != name {
                editor.text.clone_from(&name);
                editor.applied = name;
            }
            let field = ui
                .add(egui::TextEdit::singleline(&mut editor.text).desired_width(160.0).char_limit(63))
                .on_hover_text(aurora_i18n::tr!("import-source-folder-name-help", source = editor.source.as_str()));
            let valid = folders::parse_path(&editor.text).is_some_and(|path| path.len() == 1);
            let apply = tool_button(ui, p, aurora_i18n::tr!("common-apply"), valid);
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::unique("import_source_root_name"), field.rect);
                data.insert_temp(egui::Id::unique("import_source_root_apply"), apply.rect);
            });
            #[cfg(not(test))]
            let _ = field;
            if apply.clicked() && imports.rename_source_root(&editor.source, &editor.text) {
                editor.applied = imports.folders.roots.name(&editor.source);
                editor.text.clone_from(&editor.applied);
            }
        });
    });
    ui.ctx().data_mut(|data| data.insert_temp(key, editor));
}

#[cfg(test)]
mod tests;
