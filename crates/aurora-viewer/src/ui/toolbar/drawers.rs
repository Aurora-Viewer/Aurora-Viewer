//! User-owned collapsible toolbar groups, anchored to their toolbar button.
//! Contents reuse the native renderer and guards. Custom drawers are one level deep.
//!
//! A floating block (Aurora's own, human request, 2026-11-25) is a drawer of
//! its own on the screen (`Drawer::floating`, drawn by `floating.rs`): it
//! keeps its buttons by the same rules (each command in one place), and its
//! button, placed in a bar or not, shows and hides it. The quick import bar
//! is one (`IMPORT_BAR`), which cannot be deleted.

use super::{Align, Bar, ButtonMode, Drag, Edit, Sizing, ToolbarSettings, ToolbarUi, structure};
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
/// The "Interfaces" block (human request, 2026-11-26): a drawer when it is
/// placed, whose buttons are the interfaces, the built-in presets then the
/// personal ones, that their eye keeps shown
/// (`ToolbarSettings::hidden_interfaces`). Its buttons are made as it is
/// drawn (`shown_buttons`): it keeps none and takes no drop.
pub const INTERFACES: &str = "drawer:interfaces";
/// The command of an Interfaces block's button: this, then a built-in
/// preset's key (`Preset::key`) or a personal interface's item id.
pub const INTERFACE_PREFIX: &str = "interface:";
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
    /// A Close button after its buttons while it is a floating block, which
    /// hides it (human request, 2026-11-26; `close_button`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub close_button: bool,
    /// Its orientation as a floating block, kept while it is a drawer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub vertical: bool,
    /// Its alignment and its buttons' size once it is a fixed block again
    /// ("Edit block", human request, 2026-11-26).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizing: Option<Sizing>,
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
            close_button: false,
            vertical: false,
            align: None,
            sizing: None,
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

/// The buttons a drawer or a floating block shows: its own, or for the
/// Interfaces block the interfaces its eyes keep (`interface_buttons`).
pub(super) fn shown_buttons<'a>(
    settings: &ToolbarSettings,
    toolbar: &ToolbarUi,
    id: &str,
    drawer: &'a Drawer,
) -> std::borrow::Cow<'a, [ToolButton]> {
    if id == INTERFACES {
        std::borrow::Cow::Owned(interface_buttons(settings, toolbar.notecards.interfaces()))
    } else {
        std::borrow::Cow::Borrowed(&drawer.buttons)
    }
}

/// The Interfaces block's buttons: the built-in presets, then the personal
/// interfaces `personal` lists (item id and name), less those whose eye is
/// closed.
pub(super) fn interface_buttons<'a>(
    settings: &ToolbarSettings,
    personal: impl IntoIterator<Item = (uuid::Uuid, &'a str)>,
) -> Vec<ToolButton> {
    let builtin = super::presets::Preset::ALL.into_iter().map(|preset| ToolButton {
        command: format!("{INTERFACE_PREFIX}{}", preset.key()),
        icon: preset.icon().into(),
        tooltip: preset.label().into(),
    });
    let personal = personal.into_iter().map(|(id, name)| ToolButton {
        command: format!("{INTERFACE_PREFIX}{id}"),
        icon: "file-text-filled".into(),
        tooltip: name.into(),
    });
    builtin
        .chain(personal)
        .filter(|button| !settings.hidden_interfaces.contains(&button.command[INTERFACE_PREFIX.len()..]))
        .collect()
}

/// Opens or closes the eye of the interface `key` (`Preset::key` or a
/// personal interface's item id): whether the Interfaces block shows it.
pub(super) fn show_interface(settings: &mut ToolbarSettings, key: &str, shown: bool) -> bool {
    if settings.locked {
        return false;
    }
    if shown {
        settings.hidden_interfaces.remove(key)
    } else {
        settings.hidden_interfaces.insert(key.to_owned())
    }
}

/// The Close button a floating block `id` ends with when it asks for one
/// (`Drawer::close_button`): an X, "Close" or both, as its buttons show.
pub(super) fn close_button(id: &str) -> ToolButton {
    ToolButton {
        command: format!("floating_close:{id}"),
        icon: "x".into(),
        tooltip: aurora_i18n::tr!("common-close").into(),
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
            vertical: drawer.vertical,
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
    let profile = take_profile(settings, &buttons[range.clone()]).unwrap_or_default();
    let icon = settings
        .appearance
        .get(&first.command)
        .and_then(|look| look.icon.clone())
        .unwrap_or_else(|| first.icon.clone());
    let name = if profile.name.is_empty() {
        super::label(&first).to_owned()
    } else {
        profile.name.clone()
    };
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
    if profile.look != Default::default() {
        settings.appearance.insert(id.clone(), profile.look.clone());
    }
    let vertical = profile.vertical.unwrap_or(bar.vertical());
    settings.drawers.insert(
        id.clone(),
        Drawer {
            buttons: contents,
            mode,
            closes: profile.closes.unwrap_or(true),
            hides_button: profile.hides_button,
            close_button: profile.close_button,
            vertical,
            align: block_style.align,
            sizing: block_style.sizing,
            floating: Some(Floating {
                name,
                icon,
                vertical,
                shown: true,
                closes: profile.closes.unwrap_or(false),
            }),
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
    let drawer = settings.drawers.get_mut(id)?;
    let floating = drawer.floating.take()?;
    drawer.vertical = floating.vertical;
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

/// What a fixed block keeps for when it becomes a drawer or a floating
/// block ("Edit block", human request, 2026-11-26): its button's name and
/// look, whether it closes after a click, its orientation and whether its
/// button hides while floating. Saved under one of its buttons
/// (`ToolbarSettings::block_profiles`), it goes to the drawer it becomes
/// and comes back when the drawer is a fixed block again.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BlockProfile {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub look: super::appearance::Appearance,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical: Option<bool>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hides_button: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub close_button: bool,
}

/// The profile of the fixed block of `members`: the first one saved under
/// one of them.
pub(super) fn profile<'a>(settings: &'a ToolbarSettings, members: &[ToolButton]) -> Option<&'a BlockProfile> {
    members.iter().find_map(|member| settings.block_profiles.get(&member.command))
}

/// Takes the profile of the fixed block of `members` out of the settings
/// (all of its members').
pub(super) fn take_profile(settings: &mut ToolbarSettings, members: &[ToolButton]) -> Option<BlockProfile> {
    let mut found = None;
    for member in members {
        if let Some(profile) = settings.block_profiles.remove(&member.command) {
            found.get_or_insert(profile);
        }
    }
    found
}

/// Saves `profile` for the fixed block of `members`, under its first button;
/// nothing for an empty one.
pub(super) fn put_profile(settings: &mut ToolbarSettings, members: &[ToolButton], profile: BlockProfile) {
    take_profile(settings, members);
    let first = members
        .iter()
        .find(|button| !structure::is_separator(&button.command) && !structure::is_divider(&button.command));
    if let Some(first) = first
        && profile != BlockProfile::default()
    {
        settings.block_profiles.insert(first.command.clone(), profile);
    }
}

/// A drawer's or a floating block's profile, for the fixed block it becomes
/// (Unfold): its button's name and look, its closing, its orientation.
pub(super) fn drawer_profile(settings: &mut ToolbarSettings, id: &str, drawer: &Drawer, name: &str) -> BlockProfile {
    BlockProfile {
        name: name.to_owned(),
        look: settings.appearance.remove(id).unwrap_or_default(),
        closes: Some(drawer.closes),
        vertical: Some(drawer.vertical),
        hides_button: drawer.hides_button,
        close_button: drawer.close_button,
    }
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
    if id == INTERFACES || is_drawer(&drag.button.command) || drag.members.iter().any(|button| is_drawer(&button.command)) {
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
    let contents = shown_buttons(&settings, toolbar, &button.command, drawer);
    let display = structure::Display::for_drawer(ctx, row.bar, &contents, editing);
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
