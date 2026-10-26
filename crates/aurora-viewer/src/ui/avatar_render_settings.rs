//! Persisted avatar display exceptions, following
//! FSFloaterAvatarRenderSettings and FSAvatarRenderPersistenceMenu
//! (indra/newview/fsfloateravatarrendersettings.cpp, originally LGPL 2.1).

use super::avatar_picker::AvatarPicker;
use super::menu;
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Sense, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

pub const WINDOW: &str = "avatar_render_settings";
const ROW_H: f32 = 26.0;

pub struct Row {
    pub id: Uuid,
    pub name: String,
    pub mode: u8,
    pub can_name: bool,
}

#[derive(Debug, PartialEq)]
pub struct Change {
    pub ids: Vec<Uuid>,
    /// 0 removes the exception, 1 never, 2 always fully renders.
    pub mode: u8,
}

#[derive(Default)]
pub struct AvatarRenderSettings {
    filter: String,
    selected: HashSet<Uuid>,
    anchor: Option<Uuid>,
    sort_setting: bool,
    descending: bool,
    picker: AvatarPicker,
    add_mode: u8,
}

fn label(mode: u8) -> &'static str {
    match mode {
        1 => aurora_i18n::tr!("ui-context-never-fully-render"),
        2 => aurora_i18n::tr!("ui-context-always-show-full-detail"),
        _ => aurora_i18n::tr!("ui-context-remove-from-display-exceptions"),
    }
}

fn icon(mode: u8) -> &'static str {
    match mode {
        1 => "ghost-filled",
        2 => "user-scan",
        _ => "trash-filled",
    }
}

fn short_label(mode: u8) -> &'static str {
    match mode {
        1 => aurora_i18n::tr!("avatar-render-never"),
        2 => aurora_i18n::tr!("avatar-render-always"),
        _ => aurora_i18n::tr!("common-remove"),
    }
}

impl AvatarRenderSettings {
    fn rows<'a>(&self, rows: &'a [Row]) -> Vec<&'a Row> {
        let filter = self.filter.trim().to_lowercase();
        let mut shown: Vec<_> = rows
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&filter) || (r.can_name && r.id.to_string().contains(&filter)))
            .collect();
        shown.sort_by(|a, b| {
            let name = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            let order = if self.sort_setting { a.mode.cmp(&b.mode).then(name) } else { name };
            let order = order.then(a.id.cmp(&b.id));
            if self.descending { order.reverse() } else { order }
        });
        shown
    }

    fn click(&mut self, id: Uuid, order: &[Uuid], modifiers: egui::Modifiers) {
        if modifiers.shift
            && let Some(anchor) = self.anchor
            && let (Some(a), Some(b)) = (order.iter().position(|i| *i == anchor), order.iter().position(|i| *i == id))
        {
            if !modifiers.command {
                self.selected.clear();
            }
            self.selected.extend(&order[a.min(b)..=a.max(b)]);
            return;
        }
        if modifiers.command {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
        } else {
            self.selected.clear();
            self.selected.insert(id);
        }
        self.anchor = Some(id);
    }

    fn change(&self, mode: u8, order: &[Uuid]) -> Change {
        Change {
            ids: order.iter().copied().filter(|id| self.selected.contains(id)).collect(),
            mode,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, rows: &[Row], world: &mut World, open: &mut bool) -> Option<Change> {
        if !*open {
            self.picker.open = false;
            return None;
        }
        let mut change = None;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("avatar-render-title"),
            egui::pos2(260.0, 140.0),
            Vec2::new(620.0, 420.0),
        );
        floater.min_size = Vec2::new(460.0, 260.0);
        floater.show(ctx, p, open, |ui| {
            ui.horizontal(|ui| {
                let add_label = aurora_i18n::tr!("common-add");
                let add_width = widgets::action_button_fit(ui, "plus", add_label);
                let width = (ui.available_width() - add_width - ui.spacing().item_spacing.x).max(80.0);
                let search = widgets::clearable_search_box(ui, p, &mut self.filter, aurora_i18n::tr!("avatar-render-filter"), width);
                if super::find_shortcut::take(ui, &[search.id]) {
                    search.request_focus();
                }
                let add = widgets::action_button_if(
                    ui,
                    p,
                    "plus",
                    add_label,
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                    !world.rlva.has("shownames"),
                );
                menu::dropdown(&add, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    for mode in [2, 1] {
                        if menu::item(ui, p, icon(mode), label(mode)) {
                            self.add_mode = mode;
                            self.picker.open("avatar_render_picker", true);
                            ui.close();
                        }
                    }
                });
            });
            ui.add_space(6.0);
            let shown = self.rows(rows);
            let order: Vec<_> = shown.iter().map(|r| r.id).collect();
            // Filtered-out and externally removed rows cannot be edited by accident.
            self.selected.retain(|id| order.contains(id));
            let footer = 32.0 + ui.spacing().item_spacing.y;
            let height = (ui.available_height() - footer).max(0.0);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_height(height);
                    ui.horizontal(|ui| {
                        let width = ui.available_width();
                        for (setting, title, width) in [
                            (false, aurora_i18n::tr!("common-name"), width * 0.52),
                            (
                                true,
                                aurora_i18n::tr!("avatar-render-detail"),
                                width * 0.48 - ui.spacing().item_spacing.x,
                            ),
                        ] {
                            let resp = ui.add_sized([width, 24.0], egui::Button::new(title));
                            if self.sort_setting == setting {
                                widgets::paint_glyph(
                                    ui.painter(),
                                    widgets::sort_caret(!self.descending),
                                    egui::pos2(resp.rect.right() - 10.0, resp.rect.center().y),
                                    10.0,
                                    p.muted,
                                );
                            }
                            if resp.clicked() {
                                if self.sort_setting == setting {
                                    self.descending = !self.descending;
                                } else {
                                    self.sort_setting = setting;
                                    self.descending = false;
                                }
                            }
                        }
                    });
                    egui::ScrollArea::vertical().id_salt(WINDOW).auto_shrink([false, false]).show_rows(
                        ui,
                        ROW_H,
                        shown.len(),
                        |ui, range| {
                            if shown.is_empty() {
                                ui.label(RichText::new(aurora_i18n::tr!("avatar-render-empty")).color(p.muted));
                            }
                            for row in &shown[range] {
                                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                                let selected = self.selected.contains(&row.id);
                                if selected || resp.hovered() {
                                    let color = if selected {
                                        p.violet.gamma_multiply(0.30)
                                    } else {
                                        p.raised.gamma_multiply(0.6)
                                    };
                                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, color);
                                }
                                let split = rect.left() + rect.width() * 0.52;
                                widgets::paint_elided(
                                    ui,
                                    &row.name,
                                    egui::pos2(rect.left() + 6.0, rect.center().y),
                                    split - rect.left() - 12.0,
                                    egui::FontId::proportional(12.5),
                                    p.ink,
                                );
                                widgets::paint_elided(
                                    ui,
                                    short_label(row.mode),
                                    egui::pos2(split + 6.0, rect.center().y),
                                    rect.right() - split - 12.0,
                                    egui::FontId::proportional(12.0),
                                    p.muted,
                                );
                                let tip = if row.can_name {
                                    format!("{}\n{}", row.name, row.id)
                                } else {
                                    row.name.clone()
                                };
                                resp.clone().on_hover_text(tip);
                                if resp.clicked() {
                                    self.click(row.id, &order, ui.input(|i| i.modifiers));
                                }
                                if resp.secondary_clicked() && !selected {
                                    self.selected.clear();
                                    self.selected.insert(row.id);
                                    self.anchor = Some(row.id);
                                }
                                menu::context_menu(&resp, p, |ui| {
                                    for mode in [2, 1, 0] {
                                        let checked = self.selected.iter().all(|id| rows.iter().any(|r| r.id == *id && r.mode == mode));
                                        if menu::check(ui, p, icon(mode), label(mode), checked, true) {
                                            change = Some(self.change(mode, &order));
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        },
                    );
                },
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for mode in [2, 1, 0] {
                    if widgets::action_button_if(
                        ui,
                        p,
                        icon(mode),
                        short_label(mode),
                        ActionKind::Flat,
                        Vec2::new(0.0, 26.0),
                        !self.selected.is_empty(),
                    )
                    .on_hover_text(label(mode))
                    .clicked()
                    {
                        change = Some(self.change(mode, &order));
                    }
                }
            });
            // Delete belongs to this list only, never a text field or another window.
            if !ctx.egui_wants_keyboard_input()
                && !self.picker.open
                && ctx
                    .pointer_latest_pos()
                    .is_some_and(|at| ctx.layer_id_at(at) == Some(ui.layer_id()))
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            {
                change = Some(self.change(0, &order));
            }
        });
        if !*open || world.rlva.has("shownames") {
            self.picker.open = false;
        }
        if let Some(ids) = self.picker.show(ctx, p, world) {
            change = Some(Change { ids, mode: self.add_mode });
        }
        change
    }
}

#[cfg(test)]
#[path = "avatar_render_settings_tests.rs"]
mod tests;
