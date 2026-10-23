//! Project selection and actions share the existing account-local job queue.
use super::*;

impl Editor {
    pub(super) fn library_idle(&self) -> bool {
        !self.project.busy
            && self.project.library.ready
            && !self.picking
            && self.project.library.command.is_none()
            && self.project.library.delete.is_none()
            && self.project.action.is_none()
    }
    pub(super) fn select_library_project(&mut self, id: Uuid) {
        if self.project.library.active.as_ref().is_some_and(|p| p.id == id) {
            self.tab = 1;
            return;
        }
        self.begin_project_creation();
        self.project.library.command = Some(Command::Load(id));
        self.project.library.open_bindings = Some(id);
        self.project.library.edit = None;
    }
    pub(in crate::ik_edit) fn project_tabs_ui(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let active = self.project.library.active.is_some();
        if !self.clip.active && !active {
            self.tab = 5;
        }
        let pages: &[u8] = if self.clip.active { &[0, 2, 3] } else { &[5, 1, 4, 2, 3] };
        let mut selected = pages.iter().position(|t| *t == self.tab).unwrap_or(0);
        let labels: Vec<_> = pages
            .iter()
            .map(|page| {
                let available = self.clip.active || *page == 5 || (active && (*page == 1 || !self.graph.definition.profiles.is_empty()));
                (
                    match page {
                        0 => aurora_i18n::tr!("ik-edit-tab-pose"),
                        1 => aurora_i18n::tr!("ik-edit-tab-bindings"),
                        2 => aurora_i18n::tr!("ik-edit-tab-animations"),
                        3 => aurora_i18n::tr!("ik-edit-tab-visible"),
                        5 => aurora_i18n::tr!("ik-edit-tab-projects"),
                        _ => aurora_i18n::tr!("ik-edit-tab-virtual"),
                    },
                    available
                        && (!self.picking
                            || [1, 3].contains(page)
                            || (*page == 5 && (self.project_target_picking() || self.object_import_picking()))),
                )
            })
            .collect();
        widgets::tabs(ui, p, &mut selected, &labels);
        self.tab = pages[selected];
    }
    pub(in crate::ik_edit) fn library_browser_ui(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        use widgets::{ActionKind, action_button_if};
        self.library_controls_ui(ui, p, world);
        let size = egui::vec2(0.0, 26.0);
        if self.project.library.edit.as_ref().is_none_or(|edit| edit.target.is_none()) {
            self.library_name_ui(ui, p);
        }
        if let Some(identity) = self.project.library.delete.clone() {
            ui.label(aurora_i18n::tr!("ik-library-delete-confirm", name = &identity.name));
            ui.horizontal(|ui| {
                if action_button_if(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::text("ik-library-delete-confirm-button"),
                    ActionKind::Danger,
                    size,
                    !self.project.busy,
                )
                .clicked()
                {
                    self.project.library.command = Some(Command::Manage(identity.id, Management::Delete));
                    self.project.library.delete = None;
                }
                if ui.button(aurora_i18n::tr!("common-cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.project.library.delete = None;
                }
            });
        }
        ui.add_space(ui.spacing().item_spacing.y);
        ui.add(
            egui::TextEdit::singleline(&mut self.project.library.filter)
                .hint_text(aurora_i18n::tr!("ik-library-search"))
                .desired_width(f32::INFINITY),
        );
        let filter = self.project.library.filter.to_lowercase();
        let entries: Vec<_> = self
            .project
            .library
            .entries
            .iter()
            .filter(|p| p.name.to_lowercase().contains(filter.trim()))
            .cloned()
            .collect();
        let enabled = self.library_idle();
        egui::ScrollArea::vertical()
            .id_salt("ik-project-list")
            .auto_shrink([false, true])
            .max_height(ui.available_height().max(60.0))
            .show(ui, |ui| {
                for entry in entries {
                    let selected = self.project.library.active.as_ref().is_some_and(|p| p.id == entry.id);
                    ui.push_id(entry.id, |ui| {
                        ui.horizontal(|ui| {
                            let name_width = (ui.available_width() - 5.0 * (26.0 + ui.spacing().item_spacing.x)).max(40.0);
                            if ui
                                .add_enabled_ui(enabled, |ui| {
                                    ui.add_sized(
                                        egui::vec2(name_width, 30.0),
                                        egui::Button::selectable(selected, (entry.name.as_str(), egui::Atom::grow())).truncate(),
                                    )
                                })
                                .inner
                                .on_hover_text(&entry.name)
                                .clicked()
                            {
                                self.select_library_project(entry.id);
                            }
                            for (icon, key, kind) in [
                                ("pencil-filled", "common-rename", ActionKind::Flat),
                                ("copy-filled", "ik-library-copy", ActionKind::Flat),
                                ("file-export", "ik-project-file-export", ActionKind::Flat),
                                ("note", "ik-project-notecard-save", ActionKind::Flat),
                                ("trash-filled", "common-delete", ActionKind::SoftDanger),
                            ] {
                                if action_button_if(
                                    ui,
                                    p,
                                    icon,
                                    "",
                                    kind,
                                    egui::Vec2::splat(26.0),
                                    enabled && (icon != "note" || self.project.saving.is_none()),
                                )
                                .on_hover_text(aurora_i18n::text(key))
                                .clicked()
                                {
                                    match icon {
                                        "pencil-filled" => self.begin_library_row_edit(&entry, EditKind::Rename),
                                        "copy-filled" => self.begin_library_row_edit(&entry, EditKind::Duplicate),
                                        "file-export" => {
                                            self.project.library.command = Some(Command::Manage(entry.id, Management::File));
                                        }
                                        "note" => {
                                            self.project.library.command = Some(Command::Manage(entry.id, Management::Notecard));
                                        }
                                        _ => {
                                            self.begin_project_creation();
                                            self.project.library.edit = None;
                                            self.project.library.delete = Some(entry.clone());
                                        }
                                    }
                                }
                            }
                        });
                    });
                    if self.project.library.edit.as_ref().is_some_and(|edit| edit.target == Some(entry.id)) {
                        self.library_name_ui(ui, p);
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests;
