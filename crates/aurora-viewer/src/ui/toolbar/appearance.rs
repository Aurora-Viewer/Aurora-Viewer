//! Button presentation is independent of its placement and the bar's default.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Id, Vec2};
use serde::{Deserialize, Serialize};

use super::{ButtonMode, Edit, ToolbarSettings, ToolbarUi};
use crate::theme::Palette;
use crate::ui::{icons, menu, skin::ToolButton, widgets};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub mode: Option<ButtonMode>,
    pub icon: Option<String>,
}

#[derive(Default)]
pub(super) struct Editor {
    button: Option<ToolButton>,
    key: String,
    search: String,
    icons: Vec<String>,
}

impl Editor {
    pub(super) fn filter(&mut self, query: &str) {
        self.search = query.into();
    }

    pub fn select(&mut self, button: &ToolButton) {
        self.button = Some(button.clone());
        self.key = button.command.clone();
        self.search.clear();
        self.icons = icons::names();
    }

    fn select_as(&mut self, button: &ToolButton, key: &str) {
        self.select(button);
        self.key = key.to_owned();
    }
}

pub fn menu_key(command: &str) -> String {
    format!("menu-entry:{command}")
}

pub fn publish(ctx: &egui::Context, settings: &ToolbarSettings) {
    ctx.data_mut(|data| {
        data.insert_temp(Id::unique("toolbar_appearance"), Arc::new(settings.appearance.clone()));
        data.insert_temp(Id::unique("toolbar_configuration"), Arc::new(settings.clone()));
    });
}

pub fn get(ctx: &egui::Context, command: &str) -> Appearance {
    ctx.data(|data| data.get_temp::<Arc<BTreeMap<String, Appearance>>>(Id::unique("toolbar_appearance")))
        .and_then(|settings| settings.get(command).cloned())
        .unwrap_or_default()
}

pub fn settings(ctx: &egui::Context) -> Arc<ToolbarSettings> {
    ctx.data(|data| data.get_temp::<Arc<ToolbarSettings>>(Id::unique("toolbar_configuration")))
        .unwrap_or_default()
}

impl ToolbarUi {
    pub fn prepare(&self, ctx: &egui::Context, settings: &ToolbarSettings) {
        publish(ctx, settings);
    }

    pub fn appearance_context(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool) {
        if super::structure::is_separator(&button.command) || super::structure::is_block(&button.command) {
            return;
        }
        self.appearance_context_for(response, p, button, locked, &button.command);
    }

    pub fn menu_appearance_context(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool) {
        self.appearance_context_for(response, p, button, locked, &menu_key(&button.command));
    }

    fn appearance_context_for(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool, key: &str) {
        menu::context_menu_of_button(response, p, |ui| {
            if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-button-appearance"), !locked) {
                self.appearance_editor.select_as(button, key);
            }
            if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("toolbar-edit")) {
                self.open = true;
            }
        });
    }

    pub fn edit_button(&mut self, button: &ToolButton) {
        self.appearance_editor.select(button);
    }

    pub fn source(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool) {
        let _ = p;
        if !locked && response.drag_started_by(egui::PointerButton::Primary) {
            response.dnd_set_drag_payload(super::Drag {
                button: button.clone(),
                from: None,
            });
        }
    }

    pub(super) fn show_appearance(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings) {
        let Some(button) = self.appearance_editor.button.clone() else {
            return;
        };
        let key = self.appearance_editor.key.clone();
        let mut open = true;
        let mut value = settings.appearance.get(&key).cloned().unwrap_or_default();
        let before = value.clone();
        widgets::Floater::new(
            "toolbar_button_appearance",
            aurora_i18n::tr!("toolbar-button-appearance"),
            ctx.content_rect().center() - Vec2::new(210.0, 250.0),
            Vec2::new(420.0, 500.0),
        )
        .show(ctx, p, &mut open, |ui| {
            ui.label(egui::RichText::new(super::label(&button)).strong());
            ui.add_space(8.0);
            ui.add_enabled_ui(!settings.locked, |ui| {
                let modes = [
                    None,
                    Some(ButtonMode::IconsOnly),
                    Some(ButtonMode::TextOnly),
                    Some(ButtonMode::IconsWithText),
                ];
                for (mode, label) in modes.into_iter().zip([
                    aurora_i18n::tr!("toolbar-button-default"),
                    aurora_i18n::tr!("toolbar-mode-icons"),
                    aurora_i18n::tr!("toolbar-mode-text"),
                    aurora_i18n::tr!("toolbar-mode-icons-text"),
                ]) {
                    widgets::radio_value(ui, p, &mut value.mode, mode, label);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    widgets::glyph(ui, value.icon.as_deref().unwrap_or(&button.icon), p.ink, 24.0);
                    if ui.button(aurora_i18n::tr!("toolbar-icon-default")).clicked() {
                        value.icon = None;
                    }
                });
                ui.horizontal(|ui| {
                    widgets::clearable_search_box(
                        ui,
                        p,
                        &mut self.appearance_editor.search,
                        aurora_i18n::tr!("toolbar-icon-search"),
                        ui.available_width(),
                    );
                });
                let filter = self.appearance_editor.search.to_lowercase();
                let names: Vec<_> = self
                    .appearance_editor
                    .icons
                    .iter()
                    .filter(|name| filter.is_empty() || name.contains(&filter))
                    .collect();
                const COLS: usize = 8;
                let icon_height = ui.available_height().clamp(64.0, 290.0);
                egui::ScrollArea::vertical()
                    .id_salt("button_icon_picker")
                    .max_height(icon_height)
                    .show_rows(ui, 34.0, names.len().div_ceil(COLS), |ui, rows| {
                        for row in rows {
                            ui.horizontal(|ui| {
                                for name in names.iter().skip(row * COLS).take(COLS) {
                                    let selected = value.icon.as_deref().unwrap_or(&button.icon) == name.as_str();
                                    let kind = if selected {
                                        widgets::ActionKind::Primary
                                    } else {
                                        widgets::ActionKind::Flat
                                    };
                                    let response =
                                        widgets::action_button(ui, p, name, "", kind, Vec2::splat(32.0)).on_hover_text(name.as_str());
                                    if response.clicked() {
                                        value.icon = Some((*name).clone());
                                    }
                                }
                            });
                        }
                    });
            });
        });
        if value != before {
            self.pending = Some(Edit::Appearance(key, value));
        }
        if !open {
            self.appearance_editor.button = None;
        }
    }
}
