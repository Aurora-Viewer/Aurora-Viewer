//! "Edit block": one window for a fixed block, a drawer and a floating block
//! (human request, 2026-11-26), with the same fields whatever its type. A
//! block keeps what its other types use: a fixed block its button's name and
//! look and its floating options (`drawers::BlockProfile`), a drawer its
//! fixed options and its orientation, and each follows it when it changes
//! type (`drawers::take_profile`, `drawers::put_profile`).

use super::{Align, Bar, BarStyle, ButtonMode, Edit, Sizing, Style, ToolbarSettings, ToolbarUi, drawers, edit_mode, shortcuts, structure};
use crate::theme::Palette;
use crate::ui::{
    skin::{Layout, ToolButton},
    widgets,
};
use egui::{Id, Vec2};

/// The block a window edits.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Target {
    /// A drawer or a floating block, by its id.
    Drawer(String),
    /// A fixed block: its bar, one of its buttons, and its bar's style.
    Fixed(Bar, String, Style),
}

/// The window's fields, the same for every type.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Options {
    pub name: String,
    pub look: super::appearance::Appearance,
    pub closes: bool,
    pub mode: ButtonMode,
    pub align: Align,
    pub sizing: Sizing,
    pub vertical: bool,
    pub hides_button: bool,
    pub close_button: bool,
}

/// One change of the window: its block, and its fields before and after.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Change {
    pub target: Target,
    pub before: Options,
    pub after: Options,
}

/// What the window shows besides its fields.
struct View {
    options: Options,
    /// The button whose icon the window changes (a fixed block's first).
    button: ToolButton,
    hint: String,
    /// The bar whose alignment words the window uses.
    bar: Bar,
    /// The key of its shortcut; none for a block that cannot have one.
    shortcut: Option<String>,
    /// The id the tests read its widgets by.
    key: String,
}

fn view(settings: &ToolbarSettings, layout: &Layout, target: &Target) -> Option<View> {
    match target {
        Target::Drawer(id) => {
            let drawer = settings.drawers.get(id)?;
            let at = settings.location(id, layout);
            let bar = at.unwrap_or(Bar::Top);
            let placed = at.and_then(|bar| settings.bar(bar, layout).iter().find(|button| button.command == *id).cloned());
            let button = match (&drawer.floating, placed) {
                (Some(floating), _) => floating.button(id),
                (None, Some(button)) => button,
                (None, None) => crate::ui::bars::top::button(id, "folder-filled"),
            };
            let hint = match &drawer.floating {
                Some(_) if id == drawers::IMPORT_BAR => aurora_i18n::tr!("toolbar-import-bar").to_owned(),
                Some(_) => aurora_i18n::tr!("toolbar-floating-new").to_owned(),
                None => super::label(&button).to_owned(),
            };
            let saved = settings.block_styles.get(id).copied().unwrap_or_default();
            let base = settings.style(bar).resolve(bar, false);
            Some(View {
                options: Options {
                    name: drawer
                        .floating
                        .as_ref()
                        .map_or_else(|| button.tooltip.clone(), |floating| floating.name.clone()),
                    look: settings.appearance.get(id).cloned().unwrap_or_default(),
                    closes: drawer.floating.as_ref().map_or(drawer.closes, |floating| floating.closes),
                    mode: drawer.mode,
                    align: drawer.align.or(saved.align).unwrap_or(base.align),
                    sizing: drawer.sizing.or(saved.sizing).unwrap_or(base.sizing),
                    vertical: drawer.floating.as_ref().map_or(drawer.vertical, |floating| floating.vertical),
                    hides_button: drawer.hides_button,
                    close_button: drawer.close_button,
                },
                button,
                hint,
                bar,
                shortcut: Some(id.clone()),
                key: id.clone(),
            })
        }
        Target::Fixed(bar, command, base) => {
            let buttons = settings.bar(*bar, layout);
            let range = edit_mode::group(buttons, command)?;
            let members = &buttons[range];
            let first = members
                .iter()
                .find(|button| !structure::is_separator(&button.command) && !structure::is_divider(&button.command))?;
            let profile = drawers::profile(settings, members).cloned().unwrap_or_default();
            let saved = settings.block_styles.get(command).copied().unwrap_or_default();
            let icon = settings
                .appearance
                .get(&first.command)
                .and_then(|look| look.icon.clone())
                .unwrap_or_else(|| first.icon.clone());
            Some(View {
                options: Options {
                    name: profile.name.clone(),
                    look: profile.look.clone(),
                    closes: profile.closes.unwrap_or(true),
                    mode: saved.mode.unwrap_or(base.mode),
                    align: saved.align.unwrap_or(base.align),
                    sizing: saved.sizing.unwrap_or(base.sizing),
                    vertical: profile.vertical.unwrap_or(bar.vertical()),
                    hides_button: profile.hides_button,
                    close_button: profile.close_button,
                },
                button: ToolButton {
                    command: shortcuts::block_key(&first.command),
                    icon,
                    tooltip: super::label(first).to_owned(),
                },
                hint: super::label(first).to_owned(),
                bar: *bar,
                shortcut: shortcuts::block_of(settings, buttons, command),
                key: command.clone(),
            })
        }
    }
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, layout: &Layout) {
    let target = if let Some(id) = toolbar.drawer_editor.clone() {
        Target::Drawer(id)
    } else if let Some((bar, command, base)) = toolbar.block_editor.clone() {
        if settings.drawer_of(&command).is_some() {
            toolbar.block_editor = None;
            return;
        }
        Target::Fixed(bar, command, base)
    } else {
        return;
    };
    let Some(view) = view(settings, layout, &target) else {
        toolbar.drawer_editor = None;
        toolbar.block_editor = None;
        return;
    };
    let before = view.options.clone();
    let mut options = before.clone();
    let mut open = true;
    let mut window = widgets::Floater::new(
        "toolbar_drawer_settings",
        aurora_i18n::tr!("toolbar-block-edit-title"),
        ctx.content_rect().center() - Vec2::new(220.0, 400.0),
        Vec2::new(440.0, 800.0),
    );
    window.order = egui::Order::Foreground;
    window.show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("block_settings_fields")
            .max_height(ui.available_height())
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add_enabled_ui(!settings.locked, |ui| {
                    fields(ui, p, toolbar, settings, &view, &mut options);
                });
            });
    });
    if open && let Some(rect) = ctx.memory(|memory| memory.area_rect(Id::unique("toolbar_drawer_settings"))) {
        toolbar
            .drawer_appearance_editor
            .side_picker(ctx, p, &view.button, &mut options.look, rect, settings.locked);
    }
    if options != before {
        toolbar.pending = Some(Edit::BlockOptions(Box::new(Change {
            target,
            before,
            after: options,
        })));
    }
    if !open {
        toolbar.drawer_editor = None;
        toolbar.block_editor = None;
        toolbar.drawer_appearance_editor.close_picker();
    }
}

/// The window's fields in the human's order (2026-11-26): name, icon,
/// closing, shortcut, the button's and the buttons' displays, then the
/// options of the fixed and the floating modes, always shown. A floating
/// block has no Delete: emptied, it goes as any block (human request,
/// 2026-11-26; `drawers::remove_emptied`).
fn fields(ui: &mut egui::Ui, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, view: &View, options: &mut Options) {
    let key = view.key.as_str();
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("toolbar-block-name"));
        ui.add(
            egui::TextEdit::singleline(&mut options.name)
                .id(Id::unique(("drawer_name", key)))
                .hint_text(view.hint.as_str())
                .char_limit(80)
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(8.0);
    toolbar.drawer_appearance_editor.icon_button(ui, p, &view.button, &options.look);
    ui.add_space(12.0);
    let shortcut = view.shortcut.as_ref().and_then(|key| settings.shortcuts.get(key));
    // the Interfaces block: its name, its icon and its shortcut only
    // (human request, 2026-11-26)
    if key == drawers::INTERFACES {
        shortcut_row(ui, p, toolbar, view, shortcut);
        return;
    }
    let check = widgets::checkbox(ui, p, &mut options.closes, aurora_i18n::tr!("toolbar-drawer-closes"));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(Id::unique(("drawer_closes_check", key)), check.rect));
    let _ = check;
    ui.add_space(8.0);
    shortcut_row(ui, p, toolbar, view, shortcut);
    ui.add_space(12.0);
    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-block-button-mode")).strong());
    super::appearance::Editor::block_button_modes(ui, p, &mut options.look);
    ui.add_space(12.0);
    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-drawer-default-mode")).strong());
    for (value, _, label) in drawers::default_mode_rows() {
        widgets::radio_value(ui, p, &mut options.mode, value, label);
    }
    section(ui, p, aurora_i18n::tr!("toolbar-block-fixed-options"));
    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-block-alignment")).strong());
    for (value, _, label) in super::align_rows(view.bar) {
        widgets::radio_value(ui, p, &mut options.align, value, label);
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-block-sizing")).strong());
    for (value, _, label) in super::sizing_rows() {
        widgets::radio_value(ui, p, &mut options.sizing, value, label);
    }
    section(ui, p, aurora_i18n::tr!("toolbar-block-floating-options"));
    ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-floating-orientation")).strong());
    widgets::radio_value(ui, p, &mut options.vertical, false, aurora_i18n::tr!("toolbar-floating-horizontal"));
    widgets::radio_value(ui, p, &mut options.vertical, true, aurora_i18n::tr!("toolbar-floating-vertical"));
    ui.add_space(8.0);
    hides_check(ui, p, key, shortcut.is_some(), &mut options.hides_button);
    ui.add_space(4.0);
    let close = widgets::checkbox(ui, p, &mut options.close_button, aurora_i18n::tr!("toolbar-floating-close-button"));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(Id::unique(("drawer_close_button_check", key)), close.rect));
    #[cfg(not(test))]
    let _ = close;
}

fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(12.0);
    widgets::divider(ui, p);
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong().color(p.violet_light));
    ui.add_space(4.0);
}

/// Its shortcut as Preferences shows one, with Edit and Delete; greyed for a
/// block that cannot have one (a bar's own block in it).
fn shortcut_row(ui: &mut egui::Ui, p: &Palette, toolbar: &mut ToolbarUi, view: &View, shortcut: Option<&crate::keybinds::Binding>) {
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("toolbar-block-shortcut"));
        match shortcut {
            Some(b) => {
                let behind = p.panel;
                let caps = widgets::keycap::caps_sized(ui, b, behind, widgets::keycap::LARGE);
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(widgets::keycap::row_width(&caps), widgets::keycap::LARGE),
                    egui::Sense::hover(),
                );
                widgets::keycap::paint_row(ui, p, &caps, rect.left_center(), behind);
            }
            None => {
                ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-shortcut-none")).color(p.muted));
            }
        }
        let Some(key) = view.shortcut.as_deref() else { return };
        let edit = widgets::action_button(
            ui,
            p,
            "pencil-filled",
            aurora_i18n::tr!("common-edit"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, 28.0),
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique(("drawer_shortcut_edit", view.key.as_str())), edit.rect));
        if edit.clicked() {
            toolbar.edit_shortcut(key, shortcut);
        }
        if shortcut.is_some()
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
            toolbar.pending = Some(Edit::Shortcut(key.into(), None));
        }
    });
}

/// "Hide the button while floating", greyed without a shortcut: a click on
/// it says why (human request, 2026-11-26).
fn hides_check(ui: &mut egui::Ui, p: &Palette, key: &str, has_shortcut: bool, hides: &mut bool) {
    let check = ui
        .add_enabled_ui(has_shortcut, |ui| {
            widgets::checkbox(ui, p, hides, aurora_i18n::tr!("toolbar-drawer-hides-button"))
        })
        .inner;
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(Id::unique(("drawer_hides_check", key)), check.rect));
    let tip = Id::unique(("drawer_hides_tip", key));
    let now = ui.input(|input| input.time);
    if !has_shortcut
        && ui.input(|input| {
            input.pointer.primary_clicked() && input.pointer.interact_pos().is_some_and(|pos| check.interact_rect.contains(pos))
        })
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
}

/// Applies `Edit::BlockOptions`: what changed, on the block's current type.
pub(super) fn apply(settings: &mut ToolbarSettings, layout: &Layout, change: &Change) -> bool {
    if settings.locked {
        return false;
    }
    let (before, mut after) = (&change.before, &change.after);
    // the Interfaces block takes only its name and its icon
    let kept;
    if matches!(&change.target, Target::Drawer(id) if id == drawers::INTERFACES) {
        kept = Options {
            name: after.name.clone(),
            look: super::appearance::Appearance {
                icon: after.look.icon.clone(),
                ..before.look.clone()
            },
            ..before.clone()
        };
        after = &kept;
    }
    let name: String = after.name.chars().take(80).collect();
    let old = settings.clone();
    match &change.target {
        Target::Drawer(id) => {
            let Some(drawer) = settings.drawers.get_mut(id) else { return false };
            drawer.mode = after.mode;
            if after.align != before.align {
                drawer.align = Some(after.align);
            }
            if after.sizing != before.sizing {
                drawer.sizing = Some(after.sizing);
            }
            drawer.vertical = after.vertical;
            drawer.hides_button = after.hides_button;
            drawer.close_button = after.close_button;
            match &mut drawer.floating {
                Some(floating) => {
                    floating.name = name.clone();
                    floating.vertical = after.vertical;
                    floating.closes = after.closes;
                }
                None => drawer.closes = after.closes,
            }
            if after.look == Default::default() {
                settings.appearance.remove(id);
            } else {
                settings.appearance.insert(id.clone(), after.look.clone());
            }
            for bar in Bar::ALL {
                let mut buttons = settings.bar(bar, layout).to_vec();
                if let Some(button) = buttons.iter_mut().find(|button| button.command == *id) {
                    button.tooltip = name.clone();
                    settings.set_bar(bar, buttons);
                }
            }
        }
        Target::Fixed(bar, command, _) => {
            if settings.drawer_of(command).is_some() {
                return false;
            }
            let buttons = settings.bar(*bar, layout).to_vec();
            let Some(range) = edit_mode::group(&buttons, command) else {
                return false;
            };
            let members = &buttons[range];
            let saved = settings.block_styles.get(command).copied().unwrap_or_default();
            let style = BarStyle {
                mode: if after.mode != before.mode { Some(after.mode) } else { saved.mode },
                align: if after.align != before.align {
                    Some(after.align)
                } else {
                    saved.align
                },
                sizing: if after.sizing != before.sizing {
                    Some(after.sizing)
                } else {
                    saved.sizing
                },
            };
            for member in members {
                if style == BarStyle::default() {
                    settings.block_styles.remove(&member.command);
                } else {
                    settings.block_styles.insert(member.command.clone(), style);
                }
            }
            let mut profile = drawers::take_profile(settings, members).unwrap_or_default();
            profile.name = name;
            profile.look = after.look.clone();
            if after.closes != before.closes {
                profile.closes = Some(after.closes);
            }
            if after.vertical != before.vertical {
                profile.vertical = Some(after.vertical);
            }
            profile.hides_button = after.hides_button;
            profile.close_button = after.close_button;
            drawers::put_profile(settings, members, profile);
        }
    }
    *settings != old
}

/// The edit of the window's fields of `target` as `change` sets them.
#[cfg(test)]
pub(super) fn edit_of(settings: &ToolbarSettings, layout: &Layout, target: Target, change: impl FnOnce(&mut Options)) -> Edit {
    let before = view(settings, layout, &target).expect("a block").options;
    let mut after = before.clone();
    change(&mut after);
    Edit::BlockOptions(Box::new(Change { target, before, after }))
}
