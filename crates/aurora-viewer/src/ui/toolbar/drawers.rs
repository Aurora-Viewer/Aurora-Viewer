//! User-owned collapsible toolbar groups, anchored to their toolbar button.
//! Contents reuse the native renderer and guards. Custom drawers are one level deep.
//!
//! A floating block (Aurora's own, human request, 2026-11-25) is a drawer of
//! its own on the screen (`Drawer::floating`, drawn by `floating.rs`): it
//! keeps its buttons by the same rules (each command in one place), and its
//! button, placed in a bar or not, shows and hides it. The quick import bar
//! is one (`IMPORT_BAR`), which cannot be deleted.

use super::{Bar, ButtonMode, Drag, Edit, ToolbarSettings, ToolbarUi, structure};
use crate::theme::Palette;
use crate::ui::{
    Panels, bars,
    icons::Icons,
    skin::{Layout, ToolButton},
    widgets,
};
use egui::{Id, Vec2};

pub const PREFIX: &str = "drawer:";
/// The quick import bar's floating block; the toolbar's "Import bar" (the
/// `import` command) shows and hides it, and it cannot be deleted.
pub const IMPORT_BAR: &str = "drawer:import_bar";
/// The import block's button: the import icon (human request, 2026-11-25).
pub const IMPORT_ICON: &str = "file-upload-filled";
/// The catalogue's "Floating block": dropped on the screen, or on a bar with
/// its button there, a new empty floating block.
pub const FLOATING: &str = "toolbar_floating";
pub const FLOATING_ICON: &str = "app-window-filled";
/// Its button while the drop is only previewed on a bar.
pub(super) const FLOATING_PREVIEW: &str = "drawer:drop-preview-floating";
/// The quick import bar's buttons, in its order, its gap included (human
/// request, 2026-11-25).
const IMPORT_COMMANDS: [&str; 9] = [
    "import_texture",
    "import_animation",
    "import_sound",
    "import_material",
    "import_mesh",
    "divider:import-bar-files",
    "import_advanced",
    "import_local",
    "import_settings",
];

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Drawer {
    pub buttons: Vec<ToolButton>,
    pub mode: ButtonMode,
    /// The drawer folds once one of its buttons is used (human request,
    /// 2026-11-25; on by default, and for the drawers saved before).
    pub closes: bool,
    /// Its button leaves its bar while it is a floating block that has a
    /// shortcut, outside interface editing (human request, 2026-11-26).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hides_button: bool,
    /// A floating block rather than a drawer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floating: Option<Floating>,
}

impl Default for Drawer {
    fn default() -> Self {
        Self {
            buttons: Vec::new(),
            mode: ButtonMode::IconsWithText,
            closes: true,
            hides_button: false,
            floating: None,
        }
    }
}

/// What a floating block keeps beside a drawer's. Its place on the screen is
/// a window's (`ui_layout.ron`).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Floating {
    /// Its button's name, placed or not.
    pub name: String,
    /// Its button's icon, `FLOATING_ICON` when empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub icon: String,
    pub vertical: bool,
    /// Shown (the import bar's is the import preference's instead).
    pub shown: bool,
    /// Hidden once one of its buttons is used or after a click elsewhere, as
    /// a drawer that closes (human request, 2026-11-25; off by default: a
    /// floating block stays where it is put).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub closes: bool,
}

impl Floating {
    /// The button that shows and hides the block `id`.
    pub fn button(&self, id: &str) -> ToolButton {
        ToolButton {
            command: id.into(),
            icon: if self.icon.is_empty() { FLOATING_ICON } else { &self.icon }.into(),
            tooltip: self.name.clone(),
        }
    }
}

/// The button that shows and hides the floating block `id`.
pub fn floating_toggle(id: &str, name: &str) -> ToolButton {
    ToolButton {
        command: id.into(),
        icon: FLOATING_ICON.into(),
        tooltip: name.into(),
    }
}

pub fn is_floating(settings: &ToolbarSettings, id: &str) -> bool {
    settings.drawers.get(id).is_some_and(|drawer| drawer.floating.is_some())
}

/// Whether the button of the floating block `id` leaves its bar outside
/// interface editing: asked for, and its shortcut still shows the block
/// (human request, 2026-11-26).
pub fn button_hidden(settings: &ToolbarSettings, id: &str) -> bool {
    settings
        .drawers
        .get(id)
        .is_some_and(|drawer| drawer.hides_button && drawer.floating.is_some())
        && settings.shortcuts.contains_key(id)
}

/// The import block, once, with its buttons (human request, 2026-11-25):
/// taken from wherever they stand, a drawer left with nothing but gaps going
/// with its button. Then it is a block like the others: changed, made a
/// fixed block or deleted, it is not made again (a preset that has it, as
/// the Aurora one, brings it back).
pub(super) fn ensure_import_bar(settings: &mut ToolbarSettings, layout: &Layout) -> bool {
    // its first fill had a separator where a block's divider belongs
    let mut changed = false;
    if let Some(drawer) = settings.drawers.get_mut(IMPORT_BAR) {
        if let Some(floating) = drawer.floating.as_mut()
            && floating.icon.is_empty()
        {
            floating.icon = IMPORT_ICON.into();
            changed = true;
        }
        for button in &mut drawer.buttons {
            if button.command == "separator:import-bar-files" {
                *button = structure::divider("import-bar-files");
                changed = true;
            }
        }
    }
    if changed {
        settings.separators.remove("separator:import-bar-files");
    }
    // Ctrl+U shows and hides it, once (human request, 2026-11-25), unless
    // the key went to another target meanwhile
    if !settings.import_shortcut_given {
        settings.import_shortcut_given = true;
        let ctrl_u = crate::keybinds::Binding {
            input: crate::keybinds::Input::Key("KeyU".into()),
            ctrl: true,
            shift: false,
            alt: false,
        };
        if !settings.shortcuts.values().any(|bound| *bound == ctrl_u) {
            settings.shortcuts.entry(IMPORT_BAR.into()).or_insert(ctrl_u);
        }
        changed = true;
    }
    if settings.import_bar_filled {
        return changed;
    }
    settings.import_bar_filled = true;
    let counts = settings
        .drawers
        .iter()
        .map(|(id, drawer)| (id.clone(), drawer.buttons.len()))
        .collect();
    let taken = |button: &ToolButton| IMPORT_COMMANDS.contains(&button.command.as_str());
    for bar in Bar::ALL {
        let mut buttons = settings.bar(bar, layout).to_vec();
        if buttons.iter().any(taken) {
            buttons.retain(|button| !taken(button));
            settings.set_bar(bar, buttons);
        }
    }
    for (id, drawer) in &mut settings.drawers {
        if id != IMPORT_BAR {
            drawer.buttons.retain(|button| !taken(button));
        }
    }
    remove_emptied(settings, layout, &counts);
    let buttons = IMPORT_COMMANDS
        .iter()
        .filter_map(|command| {
            if structure::is_divider(command) {
                return Some(structure::divider(command.trim_start_matches(structure::DIVIDER_PREFIX)));
            }
            let found = super::catalogue::find(command)?;
            Some(ToolButton {
                command: (*command).into(),
                icon: found.icon.into(),
                tooltip: String::new(),
            })
        })
        .collect();
    let drawer = settings.drawers.entry(IMPORT_BAR.into()).or_insert_with(|| Drawer {
        mode: ButtonMode::IconsOnly,
        floating: Some(Floating {
            icon: IMPORT_ICON.into(),
            ..Floating::default()
        }),
        ..Drawer::default()
    });
    drawer.buttons = buttons;
    true
}

/// A drawer or a bar's block made a floating block (human request,
/// 2026-11-25): a drawer keeps its button, its name and its icon; a block
/// gets a button where it stood, with its first button's icon and name. None
/// when it holds what a floating block cannot (a bar's block, a drawer).
pub(super) fn float(settings: &mut ToolbarSettings, layout: &Layout, bar: Bar, command: &str) -> Option<String> {
    if settings.locked {
        return None;
    }
    if is_drawer(command) {
        let toggle = settings.bar(bar, layout).iter().find(|button| button.command == command)?.clone();
        let drawer = settings.drawers.get_mut(command)?;
        if drawer.floating.is_some() || drawer.buttons.iter().any(|button| structure::is_block(&button.command)) {
            return None;
        }
        drawer.floating = Some(Floating {
            name: toggle.tooltip,
            icon: toggle.icon,
            vertical: false,
            shown: true,
            closes: false,
        });
        return Some(command.into());
    }
    if settings.drawer_of(command).is_some() {
        return None;
    }
    let mut buttons = settings.bar(bar, layout).to_vec();
    let range = super::edit_mode::group(&buttons, command)?;
    if buttons[range.clone()]
        .iter()
        .any(|button| is_drawer(&button.command) || structure::is_block(&button.command))
    {
        return None;
    }
    let first = buttons[range.clone()]
        .iter()
        .find(|button| !structure::is_separator(&button.command) && !structure::is_divider(&button.command))?
        .clone();
    let icon = settings
        .appearance
        .get(&first.command)
        .and_then(|look| look.icon.clone())
        .unwrap_or_else(|| first.icon.clone());
    let name = super::label(&first).to_owned();
    let id = format!("{PREFIX}{}", uuid::Uuid::new_v4());
    let block_style = settings.block_styles.get(command).copied().unwrap_or_default();
    let mode = block_style.mode.or(settings.style(bar).mode).unwrap_or_default();
    let toggle = ToolButton {
        command: id.clone(),
        icon: icon.clone(),
        tooltip: name.clone(),
    };
    let contents: Vec<ToolButton> = buttons.splice(range, [toggle]).collect();
    super::shortcuts::into_drawer(settings, &contents, &id);
    settings.set_bar(bar, buttons);
    settings.block_styles.insert(id.clone(), block_style);
    settings.drawers.insert(
        id.clone(),
        Drawer {
            buttons: contents,
            mode,
            floating: Some(Floating {
                name,
                icon,
                vertical: bar.vertical(),
                shown: true,
                closes: false,
            }),
            ..Drawer::default()
        },
    );
    Some(id)
}

/// Whether `command` (a drawer's button, or one of a block's) can become a
/// floating block.
pub(super) fn floatable(settings: &ToolbarSettings, layout: &Layout, bar: Bar, command: &str) -> bool {
    float(&mut settings.clone(), layout, bar, command).is_some()
}

/// A floating block made a drawer again, its button where it stands (at the
/// bottom bar's end when it stands nowhere): the bar it stands in. The
/// import block too (human request, 2026-11-25).
pub(super) fn dock(settings: &mut ToolbarSettings, layout: &Layout, id: &str) -> Option<Bar> {
    if settings.locked {
        return None;
    }
    let floating = settings.drawers.get_mut(id)?.floating.take()?;
    if let Some(bar) = settings.location(id, layout) {
        return Some(bar);
    }
    let mut buttons = settings.bar(Bar::Bottom, layout).to_vec();
    if buttons.last().is_some_and(|button| !structure::is_separator(&button.command)) {
        buttons.push(structure::separator(&uuid::Uuid::new_v4().to_string()));
    }
    buttons.push(floating.button(id));
    settings.set_bar(Bar::Bottom, buttons);
    Some(Bar::Bottom)
}

/// A new floating block, shown, under the name `name`.
pub(super) fn create_floating(settings: &mut ToolbarSettings, name: &str) -> String {
    let id = format!("{PREFIX}{}", uuid::Uuid::new_v4());
    settings.drawers.insert(
        id.clone(),
        Drawer {
            mode: ButtonMode::IconsOnly,
            floating: Some(Floating {
                name: name.into(),
                shown: true,
                ..Floating::default()
            }),
            ..Drawer::default()
        },
    );
    id
}

/// A floating block's name, orientation, display, closing and button look.
pub(super) fn update_floating(settings: &mut ToolbarSettings, layout: &Layout, id: &str, options: &FloatingOptions) -> bool {
    let name: String = options.name.chars().take(80).collect();
    let own = settings.appearance.get(id).cloned().unwrap_or_default();
    if settings.locked {
        return false;
    }
    let Some(drawer) = settings.drawers.get_mut(id) else { return false };
    let Some(floating) = &mut drawer.floating else { return false };
    if floating.name == name
        && floating.vertical == options.vertical
        && floating.closes == options.closes
        && drawer.hides_button == options.hides_button
        && drawer.mode == options.mode
        && own == options.look
    {
        return false;
    }
    floating.name = name.clone();
    floating.vertical = options.vertical;
    floating.closes = options.closes;
    drawer.hides_button = options.hides_button;
    drawer.mode = options.mode;
    if options.look == Default::default() {
        settings.appearance.remove(id);
    } else {
        settings.appearance.insert(id.into(), options.look.clone());
    }
    for bar in Bar::ALL {
        let mut buttons = settings.bar(bar, layout).to_vec();
        if let Some(button) = buttons.iter_mut().find(|button| button.command == id) {
            button.tooltip = name.clone();
            settings.set_bar(bar, buttons);
        }
    }
    true
}

/// The settings window's fields of a floating block.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FloatingOptions {
    pub name: String,
    pub mode: ButtonMode,
    pub vertical: bool,
    pub closes: bool,
    pub hides_button: bool,
    pub look: super::appearance::Appearance,
}

/// Shows or hides the floating block `id`, locked bars included, as a
/// window is.
pub(super) fn show_floating(settings: &mut ToolbarSettings, id: &str, shown: bool) -> bool {
    match settings.drawers.get_mut(id).and_then(|drawer| drawer.floating.as_mut()) {
        Some(floating) if floating.shown != shown => {
            floating.shown = shown;
            true
        }
        _ => false,
    }
}

/// Deletes a floating block, its button with it; its buttons go back to
/// the catalogue. The import block too (human request, 2026-11-25).
pub(super) fn delete_floating(settings: &mut ToolbarSettings, layout: &Layout, id: &str) -> bool {
    if settings.locked || !is_floating(settings, id) {
        return false;
    }
    settings.drawers.remove(id);
    settings.appearance.remove(id);
    for bar in Bar::ALL {
        let mut buttons = settings.bar(bar, layout).to_vec();
        if buttons.iter().any(|button| button.command == id) {
            buttons.retain(|button| button.command != id);
            settings.set_bar(bar, buttons);
        }
    }
    true
}

pub(super) fn show_settings(ctx: &egui::Context, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, layout: &Layout) {
    let Some(id) = toolbar.drawer_editor.clone() else { return };
    let Some(drawer) = settings.drawers.get(&id) else {
        toolbar.drawer_editor = None;
        return;
    };
    if let Some(floating) = &drawer.floating {
        floating_settings(ctx, p, toolbar, settings, &id, drawer, floating);
        return;
    }
    let Some(bar) = settings.location(&id, layout) else {
        toolbar.drawer_editor = None;
        return;
    };
    let Some(button) = settings.bar(bar, layout).iter().find(|button| button.command == id) else {
        return;
    };
    let mut open = true;
    let mut name = button.tooltip.clone();
    let mut mode = drawer.mode;
    let mut closes = drawer.closes;
    let mut hides = drawer.hides_button;
    let mut own = settings.appearance.get(&id).cloned().unwrap_or_default();
    let before = (name.clone(), mode, own.clone());
    let mut window = widgets::Floater::new(
        "toolbar_drawer_settings",
        aurora_i18n::tr!("toolbar-block-edit-title"),
        ctx.content_rect().center() - Vec2::new(220.0, 230.0),
        Vec2::new(440.0, 460.0),
    );
    window.order = egui::Order::Foreground;
    window.show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("drawer_settings_fields")
            .max_height(ui.available_height())
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_enabled_ui(!settings.locked, |ui| {
                    block_fields(
                        ui,
                        p,
                        toolbar,
                        &id,
                        button,
                        BlockFields {
                            name: &mut name,
                            hint: super::label(button),
                            look: &mut own,
                            closes: &mut closes,
                            hides: &mut hides,
                            mode: &mut mode,
                            shortcut: settings.shortcuts.get(&id),
                        },
                    );
                });
            });
    });
    if open && let Some(rect) = ctx.memory(|memory| memory.area_rect(Id::unique("toolbar_drawer_settings"))) {
        toolbar
            .drawer_appearance_editor
            .side_picker(ctx, p, button, &mut own, rect, settings.locked);
    }
    if (name.clone(), mode, own.clone()) != before {
        toolbar.pending = Some(Edit::DrawerOptions(id.clone(), name, mode, own));
    } else if closes != drawer.closes {
        toolbar.pending = Some(Edit::DrawerCloses(id.clone(), closes));
    } else if hides != drawer.hides_button {
        toolbar.pending = Some(Edit::DrawerHidesButton(id.clone(), hides));
    }
    if !open {
        toolbar.drawer_editor = None;
        toolbar.drawer_appearance_editor.close_picker();
    }
}

/// A floating block's settings, as a drawer's (human request, 2026-11-25):
/// its button's name, look and icon, its buttons' default mode, whether it
/// closes after a click, then its orientation, and Delete.
fn floating_settings(
    ctx: &egui::Context,
    p: &Palette,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
    id: &str,
    drawer: &Drawer,
    floating: &Floating,
) {
    let own = id != IMPORT_BAR;
    let button = floating.button(id);
    let mut open = true;
    let mut delete = false;
    let mut options = FloatingOptions {
        name: floating.name.clone(),
        mode: drawer.mode,
        vertical: floating.vertical,
        closes: floating.closes,
        hides_button: drawer.hides_button,
        look: settings.appearance.get(id).cloned().unwrap_or_default(),
    };
    let before = options.clone();
    let mut window = widgets::Floater::new(
        "toolbar_drawer_settings",
        aurora_i18n::tr!("toolbar-block-edit-title"),
        ctx.content_rect().center() - Vec2::new(220.0, 230.0),
        Vec2::new(440.0, 460.0),
    );
    window.order = egui::Order::Foreground;
    window.show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("floating_settings_fields")
            .max_height(ui.available_height())
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_enabled_ui(!settings.locked, |ui| {
                    block_fields(
                        ui,
                        p,
                        toolbar,
                        id,
                        &button,
                        BlockFields {
                            name: &mut options.name,
                            hint: if own {
                                aurora_i18n::tr!("toolbar-floating-new")
                            } else {
                                aurora_i18n::tr!("toolbar-import-bar")
                            },
                            look: &mut options.look,
                            closes: &mut options.closes,
                            hides: &mut options.hides_button,
                            mode: &mut options.mode,
                            shortcut: settings.shortcuts.get(id),
                        },
                    );
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-floating-orientation")).strong());
                    widgets::radio_value(ui, p, &mut options.vertical, false, aurora_i18n::tr!("toolbar-floating-horizontal"));
                    widgets::radio_value(ui, p, &mut options.vertical, true, aurora_i18n::tr!("toolbar-floating-vertical"));
                    {
                        ui.add_space(12.0);
                        delete = widgets::action_button(
                            ui,
                            p,
                            "trash-filled",
                            aurora_i18n::tr!("toolbar-floating-delete"),
                            widgets::ActionKind::Danger,
                            Vec2::new(ui.available_width(), 32.0),
                        )
                        .clicked();
                    }
                });
            });
    });
    if open && let Some(rect) = ctx.memory(|memory| memory.area_rect(Id::unique("toolbar_drawer_settings"))) {
        toolbar
            .drawer_appearance_editor
            .side_picker(ctx, p, &button, &mut options.look, rect, settings.locked);
    }
    if delete {
        toolbar.pending = Some(Edit::FloatingDelete(id.into()));
        open = false;
    } else if options != before {
        toolbar.pending = Some(Edit::FloatingOptions(id.into(), options));
    }
    if !open {
        toolbar.drawer_editor = None;
        toolbar.drawer_appearance_editor.close_picker();
    }
}

/// What a drawer's and a floating block's windows share.
struct BlockFields<'a> {
    name: &'a mut String,
    hint: &'a str,
    look: &'a mut super::appearance::Appearance,
    closes: &'a mut bool,
    hides: &'a mut bool,
    mode: &'a mut ButtonMode,
    shortcut: Option<&'a crate::keybinds::Binding>,
}

/// The fields of a drawer's or a floating block's settings, in the human's
/// order (2026-11-26): its name on one line, its icon, whether it closes
/// after a click, its shortcut, whether it hides its button while floating
/// (greyed without a shortcut), then its own button's display and its
/// buttons' default display.
fn block_fields(ui: &mut egui::Ui, p: &Palette, toolbar: &mut ToolbarUi, id: &str, button: &ToolButton, fields: BlockFields<'_>) {
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("toolbar-block-name"));
        ui.add(
            egui::TextEdit::singleline(fields.name)
                .id(Id::unique(("drawer_name", id)))
                .hint_text(fields.hint)
                .char_limit(80)
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(8.0);
    toolbar.drawer_appearance_editor.icon_button(ui, p, button, fields.look);
    ui.add_space(12.0);
    let check = widgets::checkbox(ui, p, fields.closes, aurora_i18n::tr!("toolbar-drawer-closes"));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(Id::unique(("drawer_closes_check", id)), check.rect));
    let _ = check;
    ui.add_space(8.0);
    // its shortcut as Preferences shows one, with Edit and Delete
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("toolbar-block-shortcut"));
        match fields.shortcut {
            Some(b) => {
                let behind = p.panel;
                let caps = widgets::keycap::caps(ui, b, behind);
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(widgets::keycap::row_width(&caps), widgets::keycap::HEIGHT),
                    egui::Sense::hover(),
                );
                widgets::keycap::paint_row(ui, p, &caps, rect.left_center(), behind);
            }
            None => {
                ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-shortcut-none")).color(p.muted));
            }
        }
        let edit = widgets::action_button(
            ui,
            p,
            "keyboard",
            aurora_i18n::tr!("common-edit"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, 28.0),
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique(("drawer_shortcut_edit", id)), edit.rect));
        if edit.clicked() {
            toolbar.edit_shortcut(id, fields.shortcut);
        }
        if fields.shortcut.is_some()
            && widgets::action_button(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("common-delete"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            )
            .clicked()
        {
            toolbar.pending = Some(Edit::Shortcut(id.into(), None));
        }
    });
    ui.add_space(8.0);
    // greyed without a shortcut: a click on it says why (human request,
    // 2026-11-26)
    let has_shortcut = fields.shortcut.is_some();
    let check = ui
        .add_enabled_ui(has_shortcut, |ui| {
            widgets::checkbox(ui, p, fields.hides, aurora_i18n::tr!("toolbar-drawer-hides-button"))
        })
        .inner;
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(Id::unique(("drawer_hides_check", id)), check.rect));
    let tip = Id::unique(("drawer_hides_tip", id));
    let now = ui.input(|input| input.time);
    if !has_shortcut
        && ui.input(|input| input.pointer.primary_clicked() && input.pointer.interact_pos().is_some_and(|pos| check.rect.contains(pos)))
    {
        ui.ctx().data_mut(|data| data.insert_temp(tip, now + 2.0));
    }
    if let Some(until) = ui.ctx().data(|data| data.get_temp::<f64>(tip)) {
        if has_shortcut || now >= until {
            ui.ctx().data_mut(|data| data.remove::<f64>(tip));
        } else {
            check.show_tooltip_text(aurora_i18n::tr!("toolbar-drawer-hides-needs-shortcut"));
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(until - now));
        }
    }
    ui.add_space(12.0);
    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-block-button-mode")).strong());
    super::appearance::Editor::block_button_modes(ui, p, fields.look);
    ui.add_space(12.0);
    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-drawer-default-mode")).strong());
    for (value, _, label) in default_mode_rows() {
        widgets::radio_value(ui, p, fields.mode, value, label);
    }
}

/// The buttons' default display, in the human's order (2026-11-26): icons
/// only, labels only, then both.
pub(super) fn default_mode_rows() -> [(ButtonMode, &'static str, &'static str); 3] {
    let [both, icons, labels] = super::mode_rows();
    [icons, labels, both]
}

pub fn is_drawer(command: &str) -> bool {
    command.starts_with(PREFIX)
}

/// Human-requested extension of LLToolBar::updateLayoutAsNeeded
/// (indra/llui/lltoolbar.cpp): icon drawers unfold inward along the short axis.
/// Live blocks keep the orientation required by their actual visible content.
pub(crate) fn vertical<'a>(
    bar: Bar,
    buttons: impl IntoIterator<Item = &'a ToolButton>,
    mode: ButtonMode,
    settings: &ToolbarSettings,
) -> bool {
    !bar.vertical()
        || buttons.into_iter().any(|button| {
            if structure::is_separator(&button.command) || structure::is_divider(&button.command) {
                return false;
            }
            if structure::is_block(&button.command) {
                return !matches!(
                    button.command.as_str(),
                    "bar_notifications" | "bar_conversations" | "bar_status" | "bar_media" | "bar_music" | "bar_volume"
                );
            }
            settings
                .appearance
                .get(&button.command)
                .and_then(|value| value.mode)
                .unwrap_or(mode)
                != ButtonMode::IconsOnly
        })
}

pub(super) fn area_id(command: &str) -> Id {
    Id::unique(("toolbar_drawer", command))
}

pub fn is_open(ctx: &egui::Context, command: &str) -> bool {
    ctx.data(|data| data.get_temp::<bool>(area_id(command).with("open")))
        .unwrap_or(false)
}

pub fn set_open(ctx: &egui::Context, command: &str, open: bool) {
    ctx.data_mut(|data| data.insert_temp(area_id(command).with("open"), open));
}

pub fn reveal_navigation(ctx: &egui::Context, settings: &ToolbarSettings) {
    if let Some(id) = settings.drawer_of("bar_navigation") {
        set_open(ctx, id, true);
    }
}

// Fixtures retain coverage for empty drawers loaded from older layouts.
#[cfg(test)]
pub(super) fn create(settings: &mut ToolbarSettings, bar: Bar, layout: &Layout) -> String {
    let id = format!("{PREFIX}{}", uuid::Uuid::new_v4());
    settings.drawers.insert(id.clone(), Drawer::default());
    let mut buttons = settings.bar(bar, layout).to_vec();
    buttons.push(bars::top::button(&id, "folder-filled"));
    settings.set_bar(bar, buttons);
    id
}

/// A newly created placeholder stays available to fill. Once populated, moving
/// its last control away removes both the drawer and its root toggle atomically.
pub(super) fn remove_emptied(settings: &mut ToolbarSettings, layout: &Layout, previous: &std::collections::BTreeMap<String, usize>) {
    let removed: Vec<_> = settings
        .drawers
        .iter()
        .filter(|(id, drawer)| {
            drawer.floating.is_none()
                && previous.get(*id).is_some_and(|count| *count > drawer.buttons.len())
                && drawer
                    .buttons
                    .iter()
                    .all(|button| structure::is_separator(&button.command) || structure::is_divider(&button.command))
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in &removed {
        settings.drawers.remove(id);
    }
    for bar in Bar::ALL {
        let mut buttons = settings.bar(bar, layout).to_vec();
        if buttons.iter().any(|button| removed.contains(&button.command)) {
            buttons.retain(|button| !removed.contains(&button.command));
            settings.set_bar(bar, buttons);
        }
    }
}

pub(super) fn update_options(
    settings: &mut ToolbarSettings,
    layout: &Layout,
    id: &str,
    name: &str,
    mode: ButtonMode,
    value: &super::appearance::Appearance,
) -> bool {
    if settings.locked || !settings.drawers.contains_key(id) {
        return false;
    }
    let Some(bar) = settings.location(id, layout) else { return false };
    let mut buttons = settings.bar(bar, layout).to_vec();
    let Some(button) = buttons.iter_mut().find(|button| button.command == id) else {
        return false;
    };
    let name: String = name.chars().take(80).collect();
    if button.tooltip == name && settings.drawers[id].mode == mode && settings.appearance.get(id).cloned().unwrap_or_default() == *value {
        return false;
    }
    button.tooltip = name;
    settings.set_bar(bar, buttons);
    if let Some(drawer) = settings.drawers.get_mut(id) {
        drawer.mode = mode;
    }
    if *value == Default::default() {
        settings.appearance.remove(id);
    } else {
        settings.appearance.insert(id.into(), value.clone());
    }
    true
}

/// Human-requested grouping gesture: the target keeps its place and determines
/// the content default; per-control appearance overrides survive the move.
pub(super) fn group_drop(
    settings: &mut ToolbarSettings,
    layout: &Layout,
    bar: Bar,
    target: &str,
    sources: &[ToolButton],
    mode: ButtonMode,
) -> bool {
    if settings.locked
        || sources.is_empty()
        || is_drawer(target)
        || structure::is_separator(target)
        || structure::is_divider(target)
        || sources
            .iter()
            .any(|button| button.command == target || is_drawer(&button.command) || button.command == FLOATING)
    {
        return false;
    }
    let Some(target) = settings.bar(bar, layout).iter().find(|button| button.command == target).cloned() else {
        return false;
    };
    if sources.len() > 1
        && !Bar::ALL.into_iter().any(|bar| {
            let list = settings.bar(bar, layout);
            super::edit_mode::group(list, &sources[0].command).is_some_and(|range| list[range] == *sources)
        })
    {
        return false;
    }
    let sources: Vec<_> = sources.iter().cloned().map(structure::instantiate).collect();
    let mut next = settings.clone();
    for edge in Bar::ALL {
        let mut list = next.bar(edge, layout).to_vec();
        list.retain(|button| !sources.iter().any(|source| source.command == button.command));
        next.set_bar(edge, list);
    }
    for drawer in next.drawers.values_mut() {
        drawer
            .buttons
            .retain(|button| !sources.iter().any(|source| source.command == button.command));
    }
    let mut list = next.bar(bar, layout).to_vec();
    let Some(at) = list.iter().position(|button| button.command == target.command) else {
        return false;
    };
    let id = format!("{PREFIX}{}", uuid::Uuid::new_v4());
    list[at] = bars::top::button(&id, "folder-filled");
    if let Some(style) = next.block_styles.get(&target.command).copied() {
        next.block_styles.insert(id.clone(), style);
    }
    next.set_bar(bar, list);
    next.drawers.insert(
        id,
        Drawer {
            buttons: std::iter::once(target).chain(sources).collect(),
            mode,
            ..Drawer::default()
        },
    );
    *settings = next;
    true
}

pub(super) fn contents(settings: &ToolbarSettings, bar: Bar, anchor: &str, layout: &Layout) -> (Option<String>, Vec<ToolButton>) {
    if let Some(id) = settings.drawer_of(anchor) {
        (Some(id.to_owned()), settings.drawers[id].buttons.clone())
    } else {
        (None, settings.bar(bar, layout).to_vec())
    }
}

/// "Close after a click" of the drawer `id`.
/// A drawer's "Hide the button while floating".
pub(super) fn update_hides_button(settings: &mut ToolbarSettings, id: &str, hides: bool) -> bool {
    match settings.drawers.get_mut(id) {
        Some(drawer) if !settings.locked && drawer.hides_button != hides => {
            drawer.hides_button = hides;
            true
        }
        _ => false,
    }
}

pub(super) fn update_closes(settings: &mut ToolbarSettings, id: &str, closes: bool) -> bool {
    match settings.drawers.get_mut(id) {
        Some(drawer) if !settings.locked && drawer.closes != closes => {
            drawer.closes = closes;
            true
        }
        _ => false,
    }
}

/// A click on one of the drawer's buttons (not a block, a separator or a
/// divider, nor while the toolbars are edited) folds a drawer that closes:
/// once the menu or the window the click opened leaves no popup. Only a click
/// that reached the drawer counts: one on a window laid over it, or on a
/// button scrolled out of its `visible` area, does not.
pub(super) fn close_after_click(
    ctx: &egui::Context,
    command: &str,
    buttons: &[ToolButton],
    rects: &[egui::Rect],
    visible: egui::Rect,
    layer: egui::LayerId,
    editing: bool,
) {
    let Some(pos) = ctx.input(|input| input.pointer.any_click().then(|| input.pointer.interact_pos()).flatten()) else {
        return;
    };
    if editing || !visible.contains(pos) || ctx.layer_id_at(pos) != Some(layer) {
        return;
    }
    let used = buttons.iter().zip(rects).any(|(button, rect)| {
        rect.contains(pos)
            && !structure::is_block(&button.command)
            && !structure::is_separator(&button.command)
            && !structure::is_divider(&button.command)
    });
    if used {
        ctx.data_mut(|data| data.insert_temp(area_id(command).with("close"), true));
    }
}

/// A click anywhere but on the drawer or its own button (the world, a HUD,
/// another button, a window, even one laid over the drawer) folds a drawer
/// that closes after a click too, as a lost focus: once no popup is left. A
/// drag is not a click: a button dragged into the drawer leaves it open.
pub(super) fn close_on_click_outside(ctx: &egui::Context, command: &str, toggle: egui::Rect, layer: egui::LayerId) {
    let Some(pos) = ctx.input(|input| input.pointer.any_click().then(|| input.pointer.interact_pos()).flatten()) else {
        return;
    };
    if !toggle.contains(pos) && ctx.layer_id_at(pos) != Some(layer) && !egui::DragAndDrop::has_any_payload(ctx) {
        ctx.data_mut(|data| data.insert_temp(area_id(command).with("close"), true));
    }
}

pub(super) fn replace(settings: &mut ToolbarSettings, bar: Bar, drawer: Option<String>, buttons: Vec<ToolButton>) {
    if let Some(id) = drawer {
        if let Some(drawer) = settings.drawers.get_mut(&id) {
            drawer.buttons = buttons;
        }
    } else {
        settings.set_bar(bar, buttons);
    }
}

pub(super) fn insertion_edit(id: &str, rank: usize, drag: &Drag) -> Option<Edit> {
    if is_drawer(&drag.button.command) || drag.members.iter().any(|button| is_drawer(&button.command)) {
        return None;
    }
    Some(if drag.members.is_empty() {
        Edit::DrawerInsert(id.into(), drag.button.clone(), rank)
    } else {
        Edit::DrawerBlock(id.into(), drag.members.clone(), rank)
    })
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    response: &egui::Response,
    p: &Palette,
    icons: &Icons,
    toolbar: &mut ToolbarUi,
    panels: &mut Panels,
    mic: &mut bars::MicButton,
    row: &bars::Row<'_>,
    button: &ToolButton,
) {
    let ctx = &response.ctx;
    let settings = ToolbarUi::layout_configuration(ctx).unwrap_or_else(|| super::appearance::settings(ctx));
    if settings.drawer_of(&button.command).is_some() {
        return;
    }
    let Some(drawer) = settings.drawers.get(&button.command) else {
        return;
    };
    // a floating block's button shows and hides it, without a chevron; the
    // import block's display is the import windows' (`toolbar_row`)
    if drawer.floating.is_some() {
        if button.command != IMPORT_BAR
            && !ToolbarUi::editing(ctx)
            && !super::surface_drop::claimed(ctx, &button.command)
            && response.clicked()
        {
            toolbar.pending = Some(Edit::FloatingShown(button.command.clone(), !is_open(ctx, &button.command)));
        }
        return;
    }
    let arrow = if row.bar == Bar::Right { "chevron-left" } else { "chevron-right" };
    widgets::paint_glyph(
        &ctx.layer_painter(response.layer_id),
        arrow,
        response.rect.right_bottom() - Vec2::splat(4.0),
        7.0,
        p.muted,
    );
    let mut open = is_open(ctx, &button.command);
    if !super::surface_drop::claimed(ctx, &button.command) && response.clicked() {
        open = !open;
    }
    if open && ctx.input(|input| input.key_pressed(egui::Key::Escape)) && !egui::Popup::is_any_open(ctx) {
        open = false;
    }
    let editing = ToolbarUi::editing(ctx);
    // its own settings window keeps it open, and so does editing the bars
    let kept = editing || toolbar.drawer_editor.as_deref() == Some(button.command.as_str());
    let close = area_id(&button.command).with("close");
    if ctx.data(|data| data.get_temp::<bool>(close)).unwrap_or(false) && (kept || !egui::Popup::is_any_open(ctx)) {
        ctx.data_mut(|data| data.remove::<bool>(close));
        open &= !(drawer.closes && !kept);
    }
    if !open {
        ctx.data_mut(|data| data.remove::<bool>(close));
        set_open(ctx, &button.command, false);
        return;
    }
    let screen = ctx.content_rect();
    let display = structure::Display::for_drawer(ctx, row.bar, &drawer.buttons, editing);
    let buttons = display.buttons();
    let vertical = buttons.is_empty() || vertical(row.bar, buttons, drawer.mode, &settings);
    let area = area_id(&button.command);
    let spacing = ctx.global_style().spacing.item_spacing;
    let inset = egui::Margin::same((bars::islands::PAD - widgets::overlay_frame(ctx, p).stroke.width).max(0.0).round() as i8);
    let frame_margin = widgets::overlay_frame(ctx, p).inner_margin(inset).total_margin().sum();
    let mut room = screen.shrink(8.0);
    match row.bar {
        Bar::Top => room.min.y = (response.rect.bottom() + 8.0).min(room.bottom()),
        Bar::Bottom => room.max.y = (response.rect.top() - 8.0).max(room.top()),
        Bar::Left => room.min.x = (response.rect.right() + 8.0).min(room.right()),
        Bar::Right => room.max.x = (response.rect.left() - 8.0).max(room.left()),
    }
    let square = response.rect.height();
    let widths: Vec<_> = buttons
        .iter()
        .map(|button| bars::natural_width(ctx, button, drawer.mode, square, 16.0, square + 8.0))
        .collect();
    let heights: Vec<_> = buttons
        .iter()
        .map(|button| {
            if structure::is_divider(&button.command) {
                if vertical { structure::DIVIDER_SIZE } else { square }
            } else if structure::is_separator(&button.command) {
                if vertical {
                    structure::style(ctx, &button.command).size
                } else {
                    square
                }
            } else if structure::is_block(&button.command) {
                bars::chrome::height(ctx, &button.command)
            } else {
                square
            }
        })
        .collect();
    let natural_width = if buttons.is_empty() {
        220.0
    } else if vertical {
        buttons
            .iter()
            .zip(&widths)
            .filter(|(button, _)| !structure::is_separator(&button.command) && !structure::is_divider(&button.command))
            .map(|(_, width)| *width)
            .fold(square, f32::max)
    } else {
        widths.iter().sum::<f32>() + spacing.x * buttons.len().saturating_sub(1) as f32
    };
    let width_limit = (room.width() - frame_margin.x).max(0.0);
    let height_limit = (room.height() - frame_margin.y).max(0.0);
    let mut width = natural_width.min(width_limit);
    let natural_height = if buttons.is_empty() {
        let font = egui::TextStyle::Body.resolve(&ctx.global_style());
        ctx.fonts_mut(|fonts| fonts.layout(aurora_i18n::tr!("toolbar-drawer-empty").into(), font, p.ink, width.max(1.0)))
            .size()
            .y
            + spacing.y
            + 56.0
    } else if vertical {
        heights.iter().sum::<f32>() + spacing.y * buttons.len().saturating_sub(1) as f32
    } else {
        heights.iter().copied().fold(square, f32::max)
    };
    // The shared solid scrollbar reserves a strip. Keep it outside the buttons
    // on either axis, only when the current content really needs scrolling.
    let strip = widgets::reserved_scroll_style().allocated_width();
    if vertical && natural_height > height_limit {
        width = (natural_width + strip).min(width_limit);
    }
    let horizontal_strip = if !vertical && natural_width > width + 0.5 { strip } else { 0.0 };
    let content_height = (natural_height.ceil() + horizontal_strip).min(height_limit);
    // Derive both axes from current content, never a previous clipped Area size.
    let wanted = Vec2::new(width + frame_margin.x, frame_margin.y + content_height);
    let start = match row.bar {
        Bar::Left => egui::pos2(response.rect.right() + 8.0, response.rect.top() - bars::islands::PAD),
        Bar::Right => egui::pos2(response.rect.left() - 8.0 - wanted.x, response.rect.top() - bars::islands::PAD),
        Bar::Top => egui::pos2(response.rect.left() - bars::islands::PAD, response.rect.bottom() + 8.0),
        Bar::Bottom => egui::pos2(response.rect.left() - bars::islands::PAD, response.rect.top() - 8.0 - wanted.y),
    };
    let position = egui::pos2(
        start.x.clamp(room.left(), (room.right() - wanted.x).max(room.left())),
        start.y.clamp(room.top(), (room.bottom() - wanted.y).max(room.top())),
    );
    egui::Area::new(area)
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .layout(egui::Layout::top_down(egui::Align::Min))
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.set_max_size(wanted);
            widgets::overlay(ui, p, inset, |ui| {
                ui.set_width(width);
                let mut action = bars::BarAction::None;
                let mut rects = Vec::new();
                let scroll = egui::ScrollArea::new([!vertical, vertical])
                    .id_salt(("drawer_scroll", &button.command))
                    .max_height(content_height)
                    .max_width(width)
                    .min_scrolled_width(0.0)
                    .min_scrolled_height(0.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let content_width = ui.available_width().min(width);
                        ui.with_layout(
                            if vertical {
                                egui::Layout::top_down(egui::Align::Min)
                            } else {
                                egui::Layout::left_to_right(egui::Align::Center)
                            },
                            |ui| {
                                if buttons.is_empty() {
                                    ui.label(aurora_i18n::tr!("toolbar-drawer-empty"));
                                    ui.allocate_space(Vec2::new(content_width, 56.0));
                                }
                                let nested = bars::Row {
                                    bar: if vertical { Bar::Left } else { Bar::Top },
                                    style: super::Style {
                                        mode: drawer.mode,
                                        align: super::Align::Start,
                                        sizing: super::Sizing::Auto,
                                    },
                                    editing,
                                    ..*row
                                };
                                let sizes: Vec<_> = buttons
                                    .iter()
                                    .zip(widths.iter().zip(&heights))
                                    .map(|(b, (natural_width, height))| {
                                        (
                                            b.command.clone(),
                                            bars::ButtonLook {
                                                size: Vec2::new(
                                                    if vertical && !structure::is_block(&b.command) {
                                                        content_width
                                                    } else {
                                                        natural_width.min(content_width)
                                                    },
                                                    if vertical { *height } else { content_height.min(natural_height) },
                                                ),
                                                icon: 16.0,
                                                dock: true,
                                                mode: drawer.mode,
                                                left: vertical,
                                                opens: if vertical {
                                                    egui::RectAlign::RIGHT
                                                } else {
                                                    egui::RectAlign::BOTTOM_START
                                                },
                                                editing,
                                            },
                                        )
                                    })
                                    .collect();
                                rects = bars::toolbar_row(
                                    ui,
                                    p,
                                    icons,
                                    buttons,
                                    |b| {
                                        sizes
                                            .iter()
                                            .find(|(id, _)| id == &b.command)
                                            .map(|(_, look)| *look)
                                            .unwrap_or_else(|| bars::ButtonLook::bar(width))
                                    },
                                    toolbar,
                                    panels,
                                    mic,
                                    &nested,
                                    &mut action,
                                );
                            },
                        );
                    });
                ctx.data_mut(|data| {
                    data.insert_temp(
                        Id::unique(("drawer_scroll_geometry", &button.command)),
                        (scroll.content_size, scroll.inner_rect),
                    )
                });
                let zone = ui.min_rect();
                let target = ui.interact(zone, Id::unique(("drawer_drop", &button.command)), egui::Sense::hover());
                super::drop_preview::protect(ctx, zone, ui.layer_id());
                if !settings.locked
                    && target.dnd_hover_payload::<Drag>().is_some()
                    && let Some(drag) = target.dnd_release_payload::<Drag>()
                {
                    ctx.data_mut(|data| data.insert_temp(Id::unique("toolbar_drop_handled"), true));
                    let rank = ctx.pointer_interact_pos().map_or(drawer.buttons.len(), |pos| {
                        super::insertion(&display.full_rects(&rects), pos, vertical)
                    });
                    if let Some(edit) = insertion_edit(&button.command, rank, &drag) {
                        toolbar.pending = Some(edit);
                    }
                }
                if drawer.closes && !kept {
                    close_on_click_outside(ctx, &button.command, response.rect, ui.layer_id());
                    close_after_click(ctx, &button.command, buttons, &rects, scroll.inner_rect, ui.layer_id(), editing);
                }
                if !matches!(action, bars::BarAction::None) {
                    toolbar.drawer_action = Some(action);
                }
            });
        });
    set_open(ctx, &button.command, open);
}
