//! Source wrappers and named folders for loose files below the destination.

use super::*;
use std::collections::BTreeSet;

#[derive(Default, Clone)]
struct Editor {
    source: String,
    applied: String,
    text: String,
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, imports: &mut Imports, prefs: &mut Preferences) {
    let sources: BTreeSet<_> = imports
        .entries
        .iter()
        .filter(|entry| imports.choice.accepts(entry))
        .filter_map(|entry| entry.source_folders.first().cloned())
        .collect();
    if !sources.is_empty() {
        source(ui, p, imports, prefs, &sources);
    }
    if imports
        .entries
        .iter()
        .any(|entry| imports.choice.accepts(entry) && entry.source_folders.is_empty())
    {
        loose(ui, p, imports);
    }
}

fn source(ui: &mut egui::Ui, p: &Palette, imports: &mut Imports, prefs: &mut Preferences, sources: &BTreeSet<String>) {
    let key = ui.scope_id().with("import_source_root_editor");
    let mut editor = ui.ctx().data_mut(|data| data.get_temp::<Editor>(key)).unwrap_or_default();
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
        ui.add_enabled_ui(prefs.include_source_folder, |ui| {
            if sources.len() > 1 {
                egui::ComboBox::from_id_salt("import_source_root")
                    .selected_text(&editor.source)
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for source in sources {
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

#[derive(Default, Clone)]
struct LooseEditor {
    applied: Option<String>,
    text: String,
}

fn loose(ui: &mut egui::Ui, p: &Palette, imports: &mut Imports) {
    let key = ui.scope_id().with("import_loose_folder_editor");
    let mut editor = ui.ctx().data_mut(|data| data.get_temp::<LooseEditor>(key)).unwrap_or_default();
    if let Some(name) = &imports.folders.roots.loose
        && editor.applied.as_ref() != Some(name)
    {
        editor.text.clone_from(name);
    }
    editor.applied = imports.folders.roots.loose.clone();
    let mut enabled = imports.folders.roots.loose.is_some();
    let toggle = checkbox(ui, p, &mut enabled, aurora_i18n::tr!("import-create-folder"))
        .on_hover_text(aurora_i18n::tr!("import-create-folder-help"));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique("import_create_folder"), toggle.rect));
    if toggle.changed() {
        let name = folders::parse_path(&editor.text)
            .filter(|path| path.len() == 1)
            .map(|path| path[0].clone())
            .unwrap_or_default();
        imports.set_loose_folder(enabled.then_some(name.as_str()));
        editor.applied = imports.folders.roots.loose.clone();
    }
    if enabled {
        ui.horizontal_wrapped(|ui| {
            ui.label(aurora_i18n::tr!("import-folder-name"));
            let field = ui.add(egui::TextEdit::singleline(&mut editor.text).desired_width(160.0).char_limit(63));
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::unique("import_loose_folder_name"), field.rect);
            });
            if field.changed() {
                let name = folders::parse_path(&editor.text)
                    .filter(|path| path.len() == 1)
                    .map(|path| path[0].clone())
                    .unwrap_or_default();
                if imports.set_loose_folder(Some(&name)) {
                    editor.applied = imports.folders.roots.loose.clone();
                }
            }
        });
    }
    ui.ctx().data_mut(|data| data.insert_temp(key, editor));
}

#[cfg(test)]
mod tests;
