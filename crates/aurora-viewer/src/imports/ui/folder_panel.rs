//! The batch's inventory plan, edited before any folder or paid asset exists.

use super::*;
use crate::imports::folders::{self, Layout};
use crate::ui::inventory::tree;

#[cfg(test)]
mod tests;

/// Batch routing is shared by every upload draft; local previews have no destination.
pub(super) fn destination_row(
    ui: &mut egui::Ui,
    p: &Palette,
    imports: &mut Imports,
    prefs: &mut Preferences,
    world: &crate::world::World,
    inventory: &crate::ui::inventory::InventoryPreferences,
    busy: bool,
) {
    if imports.choice == Choice::Local {
        ui.label(aurora_i18n::tr!("import-local-free"));
        return;
    }
    let layout = prefs
        .folder_layout
        .get_or_insert(if prefs.use_default_folders { Layout::Mirror } else { Layout::Flat });
    let mut selected_layout = *layout;
    ui.add_enabled_ui(!busy, |ui| {
        checkbox(
            ui,
            p,
            &mut prefs.use_default_folders,
            aurora_i18n::tr!("import-use-default-folders"),
        );
        ui.add_enabled_ui(!prefs.use_default_folders, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(aurora_i18n::tr!("import-destination"));
                let mut folder = Imports::batch_folder(world, prefs.batch_folder);
                if imports.demo_mode == "destination" {
                    imports.destination.demo(ui);
                }
                imports.destination.show(ui, p, &world.inventory, &inventory.protected, &mut folder);
                if !prefs.use_default_folders {
                    prefs.batch_folder = folder;
                }
            });
            source_root::show(ui, p, imports, prefs);
            ui.horizontal_wrapped(|ui| {
                ui.label(aurora_i18n::tr!("import-folders-arrange"));
                for value in [Layout::Mirror, Layout::Type, Layout::Flat] {
                    widgets::radio_value(ui, p, &mut selected_layout, value, layout_label(value));
                }
            });
        });
    });
    prefs.folder_layout = Some(selected_layout);
    imports.sync_destinations(prefs, world, inventory);
}

fn layout_label(layout: Layout) -> &'static str {
    match layout {
        Layout::Mirror => aurora_i18n::tr!("import-folders-mirror"),
        Layout::Type => aurora_i18n::tr!("import-folders-type"),
        Layout::Flat => aurora_i18n::tr!("import-folders-flat"),
    }
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, imports: &mut Imports, world: &crate::world::World, height: f32, busy: bool) {
    let (bounds, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let mut pane = ui.new_child(egui::UiBuilder::new().id_salt("import_folder_plan").max_rect(bounds));
    pane.set_clip_rect(bounds.intersect(ui.clip_rect()));
    let ui = &mut pane;
    let top = ui.cursor().top();
    let selected = imports.entries.iter().filter(|e| !e.local && e.selected).count();
    let preview = folder_preview::build(ui.ctx(), imports, world);
    imports.folders.collapsed.retain(|path| preview.nodes.contains_key(path));
    let paths = &preview.nodes;
    let files = &preview.files;
    let root = imports
        .folders
        .active()
        .then(|| destination_path(world, imports.folders.destination));
    ui.label(aurora_i18n::tr!("import-folders-summary", folders = paths.len(), files = selected));
    let footer_key = ui.scope_id().with("footer_height");
    let footer_height = ui.ctx().data(|data| data.get_temp::<f32>(footer_key)).unwrap_or(160.0);
    let row_height = ui.spacing().interact_size.y.max(26.0);
    ui.columns(2, |columns| {
        widgets::section_header(&mut columns[0], p, "folder-filled", aurora_i18n::tr!("import-folders-preview"));
        widgets::section_header(&mut columns[1], p, "files-filled", aurora_i18n::tr!("import-files"));
        let body_top = columns[0].cursor().top().max(columns[1].cursor().top());
        let room = (height - (body_top - top) - footer_height - columns[0].spacing().item_spacing.y).max(0.0);
        // A flattened tree is virtualized even when every folder is expanded.
        let mut rows: Vec<(Vec<String>, Option<usize>)> = Vec::new();
        for index in files.get(&Vec::new()).into_iter().flatten() {
            rows.push((Vec::new(), Some(*index)));
        }
        let mut hidden: Option<&Vec<String>> = None;
        for path in paths.keys() {
            if hidden.is_some_and(|parent| path.starts_with(parent) && path.len() > parent.len()) {
                continue;
            }
            hidden = None;
            rows.push((path.clone(), None));
            if imports.folders.collapsed.contains(path) {
                hidden = Some(path);
                continue;
            }
            for index in files.get(path).into_iter().flatten() {
                rows.push((path.clone(), Some(*index)));
            }
        }
        let mut toggle = None;
        egui::ScrollArea::both()
            .id_salt("import_plan_tree")
            .max_height(room)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .show_rows(&mut columns[0], row_height, rows.len(), |ui, range| {
                for (path, file) in &rows[range] {
                    ui.horizontal(|ui| {
                        ui.set_min_height(row_height);
                        ui.add_space((path.len().saturating_sub(usize::from(file.is_none())) as f32) * 16.0);
                        if let Some(index) = file {
                            ui.add_space(12.0);
                            let entry = &imports.entries[*index];
                            widgets::glyph(ui, "file-description-filled", p.muted, 16.0);
                            ui.add(egui::Label::new(&entry.name).truncate());
                        } else {
                            let open = !imports.folders.collapsed.contains(path);
                            let caret = tree::caret(ui, p, open);
                            let node = &paths[path];
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    egui::Id::unique(("import_plan_caret", node.edit.as_ref().unwrap_or(path).clone())),
                                    caret.rect,
                                )
                            });
                            if caret.clicked() {
                                toggle = Some(path.clone());
                            }
                            widgets::glyph(ui, if open { "folder-open-filled" } else { "folder-filled" }, p.violet_light, 16.0);
                            if ui
                                .selectable_label(node.edit.is_some() && imports.folders.editing == node.edit, &node.name)
                                .clicked()
                            {
                                imports.folders.editing = node.edit.clone();
                                imports.folders.edited_path = node.edit.as_ref().map(|path| path.join("/")).unwrap_or_default();
                            }
                        }
                    });
                }
            });
        if let Some(path) = toggle {
            if !imports.folders.collapsed.remove(&path) {
                imports.folders.collapsed.insert(path);
            }
            columns[0].ctx().request_repaint();
        }
        let indices: Vec<_> = imports
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Choice::Bulk.accepts(e).then_some(i))
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("import_plan_files")
            .max_height(room)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .show_rows(&mut columns[1], row_height, indices.len(), |ui, range| {
                for &index in &indices[range] {
                    let entry = &mut imports.entries[index];
                    ui.push_id(entry.id, |ui| {
                        let active = imports.active == Some(entry.id);
                        if file_row(ui, p, entry, world, root.as_deref(), active, busy) {
                            imports.active = Some(entry.id);
                        }
                    });
                }
            });
    });
    let footer_top = ui.cursor().top();
    ui.add_enabled_ui(imports.folders.enabled && !busy, |ui| {
        if let Some(old) = imports.folders.editing.clone() {
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("import-folders-path"));
                ui.add(
                    egui::TextEdit::singleline(&mut imports.folders.edited_path)
                        .desired_width(260.0)
                        .char_limit(4095),
                );
                let parsed = folders::parse_path(&imports.folders.edited_path);
                if tool_button(ui, p, aurora_i18n::tr!("common-apply"), parsed.is_some()).clicked()
                    && let Some(path) = parsed
                {
                    folders::rename(&mut imports.entries, &old, &path);
                    imports.folders.collapsed.clear();
                    imports.folders.editing = None;
                }
            });
        }
    });
    if imports.folders.active() {
        ui.label(aurora_i18n::tr!("import-folders-auto-create"));
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(footer_key, ui.cursor().top() - footer_top + ui.spacing().item_spacing.y));
}

fn destination_path(world: &crate::world::World, folder: Uuid) -> String {
    let path = crate::ui::inventory::inventory_path(&world.inventory, folder);
    if path.is_empty() {
        aurora_i18n::tr!("import-no-folder").into()
    } else {
        path
    }
}

#[derive(Clone)]
struct FilePathEdit {
    applied: Vec<String>,
    text: String,
}

fn file_row(
    ui: &mut egui::Ui,
    p: &Palette,
    entry: &mut Entry,
    world: &crate::world::World,
    root: Option<&str>,
    active: bool,
    busy: bool,
) -> bool {
    let width = ui.available_width();
    let name_width = (width * 0.26).clamp(64.0, 140.0);
    let path_width = (width - name_width - 96.0 - ui.spacing().item_spacing.x * 3.0).max(60.0);
    let key = egui::Id::unique(("import_file_path_edit", entry.id));
    let path = entry.upload_path.clone().unwrap_or_default();
    let mut edit = ui
        .ctx()
        .data_mut(|data| data.get_temp::<FilePathEdit>(key))
        .filter(|edit| edit.applied == path)
        .unwrap_or_else(|| FilePathEdit {
            text: path.join("/"),
            applied: path,
        });
    let mut target = if let Some(root) = root {
        if edit.applied.is_empty() {
            root.to_owned()
        } else {
            format!("{root}/{}", edit.applied.join("/"))
        }
    } else {
        destination_path(world, entry.folder)
    };
    let enabled = root.is_some() && !busy && matches!(entry.status, Status::Ready | Status::Error(_));
    let clicked = ui
        .horizontal(|ui| {
            ui.set_min_height(26.0);
            let name = ui
                .add_sized(
                    Vec2::new(name_width, 26.0),
                    egui::Button::selectable(active, &entry.name).truncate(),
                )
                .on_hover_text(entry.path.to_string_lossy());
            let field = ui
                .add_enabled_ui(enabled, |ui| {
                    let text = if root.is_some() { &mut edit.text } else { &mut target };
                    ui.add_sized(
                        Vec2::new(path_width, 26.0),
                        egui::TextEdit::singleline(text)
                            .char_limit(4095)
                            .hint_text(root.unwrap_or_default()),
                    )
                })
                .inner
                .on_hover_text(format!("{target}\n{}", aurora_i18n::tr!("import-folders-path-help")));
            let parsed = folders::parse_path(&edit.text);
            let apply = tool_button(ui, p, aurora_i18n::tr!("common-apply"), enabled && parsed.is_some());
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::unique(("import_file_path_field", entry.id)), field.rect);
                data.insert_temp(
                    egui::Id::unique(("import_file_path_apply", entry.id)),
                    (apply.rect, enabled && parsed.is_some()),
                );
            });
            #[cfg(not(test))]
            let _ = field;
            if apply.clicked()
                && let Some(path) = parsed
            {
                if entry.upload_path.as_ref() != Some(&path)
                    && let Some(Source::Mesh(draft)) = &mut entry.source
                {
                    draft.invalidate_quote();
                    entry.preview_generation += 1;
                }
                edit.text = path.join("/");
                edit.applied.clone_from(&path);
                entry.upload_path = Some(path);
            }
            file_status(ui, p, entry);
            name.clicked()
        })
        .inner;
    if root.is_some() {
        ui.ctx().data_mut(|data| data.insert_temp(key, edit));
    }
    clicked
}
