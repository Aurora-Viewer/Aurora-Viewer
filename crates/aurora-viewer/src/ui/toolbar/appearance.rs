//! Button presentation is independent of its placement and the bar's default.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Id, Vec2};
use serde::{Deserialize, Serialize};

use super::{ButtonMode, Edit, ToolbarSettings, ToolbarUi};
use crate::theme::Palette;
use crate::ui::skin::Layout;
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

impl Editor {
    /// A button's display, a block's button's included: its block's rule
    /// ("Inherited"), or one of its own (human request, 2026-11-26).
    pub(super) fn button_modes(ui: &mut egui::Ui, p: &Palette, value: &mut Appearance) {
        for (mode, label) in [
            None,
            Some(ButtonMode::IconsOnly),
            Some(ButtonMode::TextOnly),
            Some(ButtonMode::IconsWithText),
        ]
        .into_iter()
        .zip([
            aurora_i18n::tr!("toolbar-button-inherited"),
            aurora_i18n::tr!("toolbar-button-icons"),
            aurora_i18n::tr!("toolbar-button-text"),
            aurora_i18n::tr!("toolbar-button-icons-text"),
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
        // the icon, then "Change icon" beside it (human request, 2026-11-26)
        ui.horizontal(|ui| {
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
            let label = ui.add(egui::Label::new(aurora_i18n::tr!("toolbar-icon-change")).sense(egui::Sense::click()));
            if response.clicked() || label.clicked() {
                self.picker_open = !self.picker_open;
            }
        });
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

    /// The button and the key "Edit button" opened on, if any.
    pub(super) fn selected(&self) -> Option<(ToolButton, String)> {
        self.button.clone().map(|button| (button, self.key.clone()))
    }

    pub(super) fn select_as(&mut self, button: &ToolButton, key: &str) {
        self.select(button);
        self.key = key.to_owned();
    }
}

pub fn menu_key(command: &str) -> String {
    format!("{MENU_KEY}{command}")
}

const MENU_KEY: &str = "menu-entry:";

/// The "Edit button" window.
const WINDOW: &str = "toolbar_button_appearance";

fn is_menu_key(key: &str) -> bool {
    key.starts_with(MENU_KEY)
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
        let islands = self.islands;
        ctx.data_mut(|data| data.insert_temp(super::islands_id(), islands));
        ctx.data_mut(|data| data.insert_temp(Id::unique("toolbar_drop_handled"), false));
    }

    pub fn editing(ctx: &egui::Context) -> bool {
        ctx.data(|data| data.get_temp::<bool>(Id::unique("toolbar_interface_editing")))
            .unwrap_or(false)
    }

    pub(crate) fn intercepting(ctx: &egui::Context) -> bool {
        Self::editing(ctx) || Self::dragging(ctx)
    }

    /// A catalogue tile's right click, for a button placed nowhere: its
    /// settings, as a placed button's (human request, 2026-11-26).
    pub fn appearance_context(&mut self, response: &egui::Response, p: &Palette, button: &ToolButton, locked: bool) {
        if super::structure::is_separator(&button.command)
            || super::structure::is_divider(&button.command)
            || super::structure::is_block(&button.command)
        {
            return;
        }
        menu::context_menu_of_button(response, p, |ui| {
            super::section(ui, p, aurora_i18n::tr!("toolbar-button-settings"));
            self.edit_button_item(ui, p, button, &button.command, locked);
            super::native_context::start_group(ui, p);
            self.interface_edit_menu(ui, p, Some(&button.command));
        });
    }

    /// "Edit button…": its window, the same for every button, a menu row's
    /// and an interface's included (human request, 2026-11-26).
    pub(super) fn edit_button_item(&mut self, ui: &mut egui::Ui, p: &Palette, button: &ToolButton, key: &str, locked: bool) {
        if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-button-edit"), !locked) {
            self.appearance_editor.select_as(button, key);
        }
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
                frame: response.ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT),
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
        // its toolbar button, then its settings, as any button's
        menu::popup_at(ctx, egui::Id::unique("toolbar_menu_row_context"), context.pos, &mut open, p, |ui| {
            super::menu_membership_item(ui, p, &context.button.command, settings);
            super::section(ui, p, aurora_i18n::tr!("toolbar-button-settings"));
            self.edit_button_item(ui, p, &context.button, &key, settings.locked);
            super::native_context::start_group(ui, p);
            self.interface_edit_menu(ui, p, Some(&context.button.command));
        });
        // the right click on another row that closed it opened that row's
        if !open && context.frame != ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) {
            self.menu_row_context = None;
        }
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
                members: Vec::new(),
            });
        }
    }

    /// "Edit button" (human request, 2026-11-26), as "Edit block": its name,
    /// its icon, its shortcut and its display, "Inherited" taking its
    /// block's. A menu row has no name nor display of its own; an
    /// interface's button has its shortcut only, its name and its icon
    /// being its interface's.
    pub(super) fn show_appearance(&mut self, ctx: &egui::Context, p: &Palette, settings: &ToolbarSettings, layout: &Layout) {
        let Some(button) = self.appearance_editor.button.clone() else {
            return;
        };
        if super::structure::is_separator(&button.command) {
            self.show_separator(ctx, p, settings, &button.command);
            return;
        }
        let key = self.appearance_editor.key.clone();
        let menu_row = is_menu_key(&key);
        let interface = button.command.starts_with(super::drawers::INTERFACE_PREFIX);
        let placed = settings.placed_button(&button.command, layout).cloned();
        let shown = placed.clone().unwrap_or_else(|| button.clone());
        // a catalogue command placed somewhere takes a name of its own
        let named = !menu_row && !interface && placed.is_some() && super::catalogue::find(&button.command).is_some();
        let hint = super::label(&ToolButton {
            tooltip: String::new(),
            ..button.clone()
        })
        .to_owned();
        let saved = Some(super::label(&shown))
            .filter(|name| *name != hint)
            .unwrap_or_default()
            .to_owned();
        let mut name = saved.clone();
        let shortcut = (!menu_row && super::shortcuts::takes(&button.command)).then(|| button.command.clone());
        let mut value = settings.appearance.get(&key).cloned().unwrap_or_default();
        let before = value.clone();
        let mut open = true;
        let mut window = widgets::Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-button-edit-title"),
            ctx.content_rect().center() - Vec2::new(220.0, 260.0),
            Vec2::new(440.0, 520.0),
        );
        window.order = egui::Order::Foreground;
        window.show(ctx, p, &mut open, |ui| {
            ui.label(egui::RichText::new(super::label(&shown)).strong());
            ui.add_space(8.0);
            ui.add_enabled_ui(!settings.locked, |ui| {
                if named {
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("toolbar-button-name"));
                        ui.add(
                            egui::TextEdit::singleline(&mut name)
                                .id(Id::unique(("button_name", button.command.as_str())))
                                .hint_text(hint.as_str())
                                .char_limit(80)
                                .desired_width(f32::INFINITY),
                        );
                    });
                    ui.add_space(8.0);
                }
                if !interface {
                    self.appearance_editor.icon_button(ui, p, &shown, &value);
                    ui.add_space(12.0);
                }
                if let Some(shortcut) = shortcut.as_deref() {
                    let current = settings.shortcuts.get(shortcut);
                    super::block_window::shortcut_row(ui, p, self, Some(shortcut), &button.command, current);
                    ui.add_space(12.0);
                }
                if !menu_row && !interface {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-button-display")).strong());
                    Editor::button_modes(ui, p, &mut value);
                }
            });
        });
        if open
            && !interface
            && let Some(rect) = ctx.memory(|memory| memory.area_rect(Id::unique(WINDOW)))
        {
            self.appearance_editor
                .side_picker(ctx, p, &shown, &mut value, rect, settings.locked);
        }
        if value != before {
            self.pending = Some(Edit::Appearance(key, value));
        } else if name != saved {
            self.pending = Some(Edit::ButtonName(button.command.clone(), name));
        }
        if !open {
            self.appearance_editor.button = None;
            self.appearance_editor.close_picker();
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
