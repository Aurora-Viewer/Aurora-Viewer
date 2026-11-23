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

/// The microphone's glyph reports live voice state and cannot be overridden.
pub(crate) fn icon_editable(command: &str) -> bool {
    command != "speak"
}

#[derive(Default)]
pub(super) struct Editor {
    button: Option<ToolButton>,
    key: String,
    picker_key: String,
    search: String,
    icons: Vec<String>,
    picker_open: bool,
}

#[derive(Clone)]
struct ContextSource {
    button: ToolButton,
    key: String,
    parent: Id,
    owner: egui::Rect,
}

#[derive(Clone)]
struct PickerProtection {
    parent: Id,
    rect: egui::Rect,
    frame: u64,
}

fn protection_id() -> Id {
    Id::unique("toolbar_context_icon_picker_protection")
}

pub(crate) fn context_picker_contains_pointer(ctx: &egui::Context) -> bool {
    let protection = ctx.data(|data| data.get_temp::<PickerProtection>(protection_id()));
    protection.is_some_and(|protection| {
        egui::Popup::is_id_open(ctx, protection.parent)
            && protection.frame.saturating_add(1) >= ctx.cumulative_frame_nr()
            && ctx.pointer_interact_pos().is_some_and(|pos| protection.rect.contains(pos))
    })
}

#[derive(Default)]
pub(super) struct ContextPicker {
    editor: Editor,
    source: Option<ContextSource>,
}

impl ContextPicker {
    pub(super) fn filter(&mut self, query: &str) {
        self.editor.filter(query);
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.source = None;
        self.editor.close_picker();
        ctx.data_mut(|data| data.remove::<PickerProtection>(protection_id()));
    }

    pub(super) fn options(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        button: &ToolButton,
        key: &str,
        settings: &ToolbarSettings,
        locked: bool,
    ) -> Option<Appearance> {
        let parent = ui.layer_id().id;
        if self.editor.key != key || self.source.as_ref().is_some_and(|source| source.parent != parent) {
            self.close(ui.ctx());
            self.editor.select_as(button, key);
        }
        let mut value = settings.appearance.get(key).cloned().unwrap_or_default();
        let before = value.clone();
        // the Aurora menu's rows always show their icon and label
        let modes = if is_menu_key(key) { &[][..] } else { &MODES[..] };
        for &(mode, icon, label) in modes {
            if menu::check(ui, p, icon, aurora_i18n::text(label), value.mode == mode, !locked) {
                value.mode = mode;
            }
        }
        if !modes.is_empty() {
            menu::separator(ui, p);
        }
        if icon_editable(&button.command) {
            let response = ui
                .add_enabled_ui(!locked, |ui| {
                    menu::suggestion_row(
                        ui,
                        p,
                        "pencil-filled",
                        aurora_i18n::tr!("toolbar-icon-change"),
                        "",
                        self.editor.picker_open,
                    )
                })
                .inner;
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique(("toolbar_icon_button_rect", &button.command)), response.rect));
            if response.clicked() {
                self.editor.picker_open = !self.editor.picker_open;
            }
        } else {
            self.close(ui.ctx());
        }
        if self.editor.picker_open {
            self.source = Some(ContextSource {
                button: button.clone(),
                key: key.into(),
                parent,
                owner: ui.ctx().memory(|memory| memory.area_rect(parent)).unwrap_or_else(|| ui.min_rect()),
            });
        } else {
            self.source = None;
            ui.ctx().data_mut(|data| data.remove::<PickerProtection>(protection_id()));
        }
        (value != before).then_some(value)
    }

    pub(super) fn show(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings) -> Option<(String, Appearance)> {
        let Some(source) = self.source.clone() else {
            ctx.data_mut(|data| data.remove::<PickerProtection>(protection_id()));
            return None;
        };
        if !self.editor.picker_open || !egui::Popup::is_id_open(ctx, source.parent) {
            self.close(ctx);
            return None;
        }
        let owner = ctx.memory(|memory| memory.area_rect(source.parent)).unwrap_or(source.owner);
        let mut value = settings.appearance.get(&source.key).cloned().unwrap_or_default();
        let before = value.clone();
        self.editor.side_picker(ctx, p, &source.button, &mut value, owner, settings.locked);
        if self.editor.picker_open {
            if let Some(rect) = ctx.memory(|memory| memory.area_rect(Id::unique(("toolbar_icon_side_picker", &source.button.command)))) {
                let frame = ctx.cumulative_frame_nr();
                ctx.data_mut(|data| {
                    data.insert_temp(
                        protection_id(),
                        PickerProtection {
                            parent: source.parent,
                            rect,
                            frame,
                        },
                    )
                });
            }
        } else {
            self.close(ctx);
        }
        (value != before).then_some((source.key, value))
    }
}

impl Editor {
    pub(super) fn modes(ui: &mut egui::Ui, p: &Palette, value: &mut Appearance) {
        for (mode, label) in [
            None,
            Some(ButtonMode::IconsOnly),
            Some(ButtonMode::TextOnly),
            Some(ButtonMode::IconsWithText),
        ]
        .into_iter()
        .zip([
            aurora_i18n::tr!("toolbar-button-default"),
            aurora_i18n::tr!("toolbar-mode-icons"),
            aurora_i18n::tr!("toolbar-mode-text"),
            aurora_i18n::tr!("toolbar-mode-icons-text"),
        ]) {
            widgets::radio_value(ui, p, &mut value.mode, mode, label);
        }
    }

    fn prepare_picker(&mut self, button: &ToolButton) {
        if self.picker_key != button.command {
            self.picker_key = button.command.clone();
            self.search.clear();
            self.icons = icons::names();
            self.picker_open = false;
        }
    }

    pub(super) fn close_picker(&mut self) {
        self.picker_open = false;
    }

    pub(super) fn icon_button(&mut self, ui: &mut egui::Ui, p: &Palette, button: &ToolButton, value: &Appearance) {
        if !icon_editable(&button.command) {
            self.close_picker();
            return;
        }
        self.prepare_picker(button);
        let response = widgets::action_button(
            ui,
            p,
            value.icon.as_deref().unwrap_or(&button.icon),
            "",
            widgets::ActionKind::Flat,
            Vec2::splat(32.0),
        )
        .on_hover_text(aurora_i18n::tr!("toolbar-icon-choose"));
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique(("toolbar_icon_button_rect", &button.command)), response.rect));
        if response.clicked() {
            self.picker_open = !self.picker_open;
        }
    }

    pub(super) fn side_picker(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        button: &ToolButton,
        value: &mut Appearance,
        owner: egui::Rect,
        locked: bool,
    ) {
        if !icon_editable(&button.command) {
            self.close_picker();
            return;
        }
        if !self.picker_open {
            return;
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.picker_open = false;
            return;
        }
        let screen = ctx.content_rect().shrink(8.0);
        let size = Vec2::new(360.0_f32.min(screen.width()), 400.0_f32.min(screen.height()));
        let x = if owner.right() + 8.0 + size.x <= screen.right() {
            owner.right() + 8.0
        } else {
            owner.left() - 8.0 - size.x
        };
        let position = egui::pos2(
            x.clamp(screen.left(), (screen.right() - size.x).max(screen.left())),
            owner.top().clamp(screen.top(), (screen.bottom() - size.y).max(screen.top())),
        );
        egui::Area::new(Id::unique(("toolbar_icon_side_picker", &button.command)))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_max_size(size);
                widgets::overlay(ui, p, egui::Margin::same(8), |ui| {
                    ui.set_width((size.x - 18.0).max(0.0));
                    ui.add_enabled_ui(!locked, |ui| self.picker(ui, p, button, value));
                });
            });
    }

    pub(super) fn picker(&mut self, ui: &mut egui::Ui, p: &Palette, button: &ToolButton, value: &mut Appearance) {
        if !icon_editable(&button.command) {
            self.close_picker();
            return;
        }
        self.prepare_picker(button);
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
                &mut self.search,
                aurora_i18n::tr!("toolbar-icon-search"),
                ui.available_width(),
            );
        });
        let filter = self.search.to_lowercase();
        let names: Vec<_> = self
            .icons
            .iter()
            .filter(|name| filter.is_empty() || name.contains(&filter))
            .collect();
        const COLS: usize = 8;
        let icon_height = ui.available_height().clamp(64.0, 290.0);
        egui::ScrollArea::vertical()
            .id_salt(("button_icon_picker", &button.command))
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
                            let response = widgets::action_button(ui, p, name, "", kind, Vec2::splat(32.0)).on_hover_text(name.as_str());
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(Id::unique(("toolbar_icon_choice", &button.command, name.as_str())), response.rect)
                            });
                            if response.clicked() {
                                value.icon = Some((*name).clone());
                            }
                        }
                    });
                }
            });
    }

    pub(super) fn filter(&mut self, query: &str) {
        self.search = query.into();
    }

    pub fn select(&mut self, button: &ToolButton) {
        self.button = Some(button.clone());
        self.key = button.command.clone();
        self.picker_key = button.command.clone();
        self.search.clear();
        self.icons = if super::structure::is_separator(&button.command) {
            Vec::new()
        } else {
            icons::names()
        };
    }

    fn select_as(&mut self, button: &ToolButton, key: &str) {
        self.select(button);
        self.key = key.to_owned();
    }
}

pub fn menu_key(command: &str) -> String {
    format!("{MENU_KEY}{command}")
}

const MENU_KEY: &str = "menu-entry:";

fn is_menu_key(key: &str) -> bool {
    key.starts_with(MENU_KEY)
}

/// The display choices of a toolbar button's right click.
const MODES: [(Option<ButtonMode>, &str, &str); 4] = [
    (Some(ButtonMode::IconsOnly), "layout-grid-filled", "toolbar-button-icons"),
    (Some(ButtonMode::TextOnly), "typography", "toolbar-button-text"),
    (Some(ButtonMode::IconsWithText), "list-details-filled", "toolbar-button-icons-text"),
    (None, "layers-subtract", "toolbar-button-inherit"),
];

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

/// The menu row whose right-click popup is open, and where it was made.
#[derive(Clone)]
pub(super) struct MenuRowContext {
    button: ToolButton,
    pos: egui::Pos2,
    frame: u64,
}

impl ToolbarUi {
    pub fn prepare(&self, ctx: &egui::Context, settings: &ToolbarSettings) {
        publish(ctx, settings);
        super::surface_drop::prepare(ctx);
        super::drop_preview::prepare(ctx, settings);
        super::blocks::prepare(ctx);
        crate::ui::bars::location_popup::prepare(ctx, self.open);
        let editing = self.open;
        ctx.data_mut(|data| data.insert_temp(Id::unique("toolbar_interface_editing"), editing));
        ctx.data_mut(|data| data.insert_temp(Id::unique("toolbar_drop_handled"), false));
    }

    pub fn editing(ctx: &egui::Context) -> bool {
        ctx.data(|data| data.get_temp::<bool>(Id::unique("toolbar_interface_editing")))
            .unwrap_or(false)
    }

    pub(crate) fn intercepting(ctx: &egui::Context) -> bool {
        Self::editing(ctx) || Self::dragging(ctx)
    }

    pub fn appearance_context(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool) {
        if super::structure::is_separator(&button.command)
            || super::structure::is_divider(&button.command)
            || super::structure::is_block(&button.command)
        {
            return;
        }
        self.appearance_context_for(response, p, button, locked, &button.command);
    }

    /// A menu row's right click opens a popup of its own: egui keeps one
    /// remembered popup open at a time, so a context menu nested in the
    /// dropdown would replace the dropdown and vanish with it. The dropdown
    /// stays open behind it until a choice or a click elsewhere.
    pub fn menu_appearance_context(&mut self, response: &egui::Response, button: &ToolButton) {
        if response.secondary_clicked() {
            self.menu_row_context = Some(MenuRowContext {
                button: button.clone(),
                pos: response.interact_pointer_pos().unwrap_or(response.rect.left_bottom()),
                frame: response.ctx.cumulative_frame_nr(),
            });
        } else if response.hovered()
            && self
                .menu_row_context
                .as_ref()
                .is_some_and(|context| context.button.command != button.command)
        {
            // the menu behind follows the pointer to another row or submenu:
            // the context of the row left behind closes instead of staying
            // open, then hidden under that submenu
            self.menu_row_context = None;
            response.ctx.request_repaint();
        }
    }

    pub(super) fn show_menu_row_context(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings) {
        let Some(context) = self.menu_row_context.clone() else {
            return;
        };
        let key = menu_key(&context.button.command);
        let mut open = true;
        menu::popup_at(ctx, egui::Id::unique("toolbar_menu_row_context"), context.pos, &mut open, p, |ui| {
            if super::menu_membership_item(ui, p, &context.button.command, settings) {
                menu::separator(ui, p);
            }
            self.appearance_items(ui, p, &context.button, settings.locked, &key);
        });
        // the right click on another row that closed it opened that row's
        if !open && context.frame != ctx.cumulative_frame_nr() {
            self.menu_row_context = None;
        }
    }

    fn appearance_context_for(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool, key: &str) {
        menu::context_menu_of_button(response, p, |ui| self.appearance_items(ui, p, button, locked, key));
    }

    fn appearance_items(&mut self, ui: &mut egui::Ui, p: &Palette, button: &ToolButton, locked: bool, key: &str) {
        if self.open {
            if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-button-appearance"), !locked) {
                self.appearance_editor.select_as(button, key);
            }
        } else {
            let settings = settings(ui.ctx());
            if let Some(value) = self.context_icon_picker.options(ui, p, button, key, &settings, locked) {
                self.pending = Some(Edit::Appearance(key.into(), value));
            }
        }
        self.interface_edit_menu(ui, p, Some(&button.command));
    }

    pub fn edit_button(&mut self, button: &ToolButton) {
        self.appearance_editor.select(button);
    }

    pub(super) fn show_context_appearance(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings) {
        if let Some((key, value)) = self.context_icon_picker.show(ctx, p, settings) {
            self.pending = Some(Edit::Appearance(key, value));
        }
    }

    pub fn source(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool) {
        let _ = p;
        if !locked && response.drag_started_by(egui::PointerButton::Primary) {
            response.dnd_set_drag_payload(super::Drag {
                button: button.clone(),
                from: None,
                members: Vec::new(),
            });
        }
    }

    pub(super) fn show_appearance(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings) {
        let Some(button) = self.appearance_editor.button.clone() else {
            return;
        };
        if super::structure::is_separator(&button.command) {
            self.show_separator(ctx, p, settings, &button.command);
            return;
        }
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
                if !is_menu_key(&key) {
                    Editor::modes(ui, p, &mut value);
                    ui.add_space(10.0);
                }
                self.appearance_editor.picker(ui, p, &button, &mut value);
            });
        });
        if value != before {
            self.pending = Some(Edit::Appearance(key, value));
        }
        if !open {
            self.appearance_editor.button = None;
        }
    }

    fn show_separator(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings, command: &str) {
        use super::structure::SeparatorStyle;
        let mut value = settings.separators.get(command).copied().unwrap_or_default().sanitized();
        let before = value;
        let mut open = true;
        widgets::Floater::new(
            "toolbar_separator_settings",
            aurora_i18n::tr!("toolbar-separator-settings"),
            ctx.content_rect().center() - Vec2::new(190.0, 110.0),
            Vec2::new(380.0, 220.0),
        )
        .show(ctx, p, &mut open, |ui| {
            ui.add_enabled_ui(!settings.locked, |ui| {
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("toolbar-separator-size"));
                    ui.add(
                        egui::DragValue::new(&mut value.size)
                            .range(SeparatorStyle::MIN..=SeparatorStyle::MAX)
                            .speed(1.0)
                            .fixed_decimals(0),
                    );
                });
                ui.add_space(8.0);
                widgets::switch_labeled(ui, p, &mut value.stretch, aurora_i18n::tr!("toolbar-separator-stretch"));
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("toolbar-separator-help"))
                        .size(12.0)
                        .color(p.muted),
                );
                ui.add_space(8.0);
                if ui.button(aurora_i18n::tr!("common-default")).clicked() {
                    value = Default::default();
                }
            });
        });
        if value != before {
            self.pending = Some(Edit::Separator(command.into(), value));
        }
        if !open {
            self.appearance_editor.button = None;
        }
    }
}
