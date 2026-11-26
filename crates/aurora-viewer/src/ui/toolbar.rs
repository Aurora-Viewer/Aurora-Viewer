//! Toolbar customization after LLToolBar::handleRightMouseDown,
//! LLToolBarView::handleDropTool and LLFloaterToybox (indra/llui/lltoolbar.cpp,
//! indra/newview/{lltoolbarview,llfloatertoybox}.cpp, originally LGPL 2.1).
//! Four toolbars: the bottom dock/bar, top row and the left/right sides;
//! a command stands in one of them at most. Main-menu commands retain their
//! original handlers and live checks through an explicitly identified catalogue.
//! Each bar has its own presentation, alignment and layout; individual commands
//! can override their icon and presentation in bars and the main menu.
//! Moves commit on release; dropping back in the catalogue removes a button.
//! Layout, appearance and lock are shared by all accounts at the human's request,
//! unlike the Legacy Viewer's per-account toolbar persistence.

pub(crate) mod appearance;
mod block_window;
pub(crate) mod blocks;
mod catalogue;
pub(crate) mod catalogue_view;
pub(crate) mod commands;
pub(crate) mod drawers;
mod drop_preview;
pub(crate) mod edit_mode;
mod editor;
pub(crate) mod floating;
pub(crate) mod native_context;
pub(crate) mod notecards;
pub(crate) mod presets;
pub(crate) mod profile;
pub(crate) mod selection;
pub(crate) mod shortcuts;
pub(crate) mod structure;
mod surface_drop;

pub use catalogue::{COMMANDS, find as command};

use super::{
    menu,
    skin::{Layout, ToolButton},
};
use crate::theme::Palette;
use egui::{Pos2, Rect, Response};
use serde::{Deserialize, Serialize};

/// A menu's Add / Remove: commands carry their stable identifier, never a
/// translated label, through the same edit path as toolbar dragging.
#[derive(Clone)]
struct MenuEdit {
    command: &'static str,
    remove: bool,
}

/// A menu row's Add / Remove (LLToolBarView::hasCommand): a row that stands
/// for a toolbar command, directly or through its editor alias, adds that
/// command at the end of the bottom bar, never its menu path. Returns whether
/// the entry was shown.
pub(crate) fn menu_membership_item(ui: &mut egui::Ui, p: &Palette, row: &str, settings: &ToolbarSettings) -> bool {
    let Some(command) = catalogue_view::alias(row)
        .and_then(catalogue::find)
        .or_else(|| catalogue::find(row))
        .filter(|command| command.ready)
    else {
        return false;
    };
    let remove = settings.placed(command.id, &Layout::default());
    let label = if remove {
        aurora_i18n::tr!("toolbar-remove-from-menu")
    } else {
        aurora_i18n::tr!("toolbar-add-from-menu")
    };
    if menu::item_if(ui, p, if remove { "trash-filled" } else { "plus" }, label, !settings.locked) {
        ui.ctx().data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<MenuEdit>>(egui::Id::unique("menu_toolbar_edits"))
                .push(MenuEdit {
                    command: command.id,
                    remove,
                });
        });
    }
    true
}

/// One of LLToolBarView's toolbars.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bar {
    Bottom,
    Left,
    Right,
    Top,
}

impl Bar {
    pub const ALL: [Bar; 4] = [Bar::Bottom, Bar::Left, Bar::Right, Bar::Top];

    pub fn vertical(self) -> bool {
        matches!(self, Bar::Left | Bar::Right)
    }

    fn index(self) -> usize {
        match self {
            Bar::Bottom => 0,
            Bar::Left => 1,
            Bar::Right => 2,
            Bar::Top => 3,
        }
    }
}

/// LLToolBarEnums::ButtonType (Firestorm adds the labels alone).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonMode {
    #[default]
    IconsOnly,
    IconsWithText,
    TextOnly,
}

/// LLToolBarEnums::Alignment: left / top, centre, right / bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

/// LLToolBarEnums::LayoutStyle: each button its own width, all the width of
/// the widest, or the buttons sharing the whole bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sizing {
    Auto,
    #[default]
    Fixed,
    Fill,
}

/// A toolbar's settings as saved: a field left unset follows the default
/// of the look in use (the human's decision of 2026-10-15), so that the
/// dock and Firestorm's bar keep their own until one is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarStyle {
    pub mode: Option<ButtonMode>,
    pub align: Option<Align>,
    pub sizing: Option<Sizing>,
}

/// A toolbar's settings, resolved for the look in use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub mode: ButtonMode,
    pub align: Align,
    pub sizing: Sizing,
}

impl BarStyle {
    /// The dock: icons, centred, fixed buttons (the mockup); Firestorm's
    /// bottom bar: icons filling the bar, as Aurora drew it before; the side
    /// bars: icons, centred (skins/default/toolbars.xml).
    pub fn resolve(&self, bar: Bar, islands: bool) -> Style {
        let default = match (bar, islands) {
            // Keep Aurora at the left when the address block folds. An
            // explicitly selected alignment still applies to the whole row.
            (Bar::Top, _) => Style {
                align: Align::Start,
                ..Default::default()
            },
            (Bar::Bottom, false) => Style {
                mode: ButtonMode::IconsOnly,
                align: Align::Start,
                sizing: Sizing::Fill,
            },
            _ => Style::default(),
        };
        Style {
            mode: self.mode.unwrap_or(default.mode),
            align: self.align.unwrap_or(default.align),
            sizing: self.sizing.unwrap_or(default.sizing),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarSettings {
    /// The bottom toolbar. None follows the skin; Some([]) deliberately
    /// keeps an empty toolbar.
    pub buttons: Option<Vec<ToolButton>>,
    pub locked: bool,
    /// One-time discovery of the new command; later manual removal is retained.
    pub vr_button_migrated: bool,
    /// The same for the Director, placed next to the photo button.
    pub director_button_migrated: bool,
    /// The quick import bar's block got its buttons, once (`drawers::ensure_import_bar`).
    pub import_bar_filled: bool,
    /// The same for its Ctrl+U.
    pub import_shortcut_given: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<ToolButton>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub right: Vec<ToolButton>,
    pub bottom_style: BarStyle,
    pub left_style: BarStyle,
    pub right_style: BarStyle,
    pub top: Vec<ToolButton>,
    pub top_style: BarStyle,
    /// Shared defaults of each root block, repeated for its members so that
    /// removing its first command preserves the block's presentation.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub block_styles: std::collections::BTreeMap<String, BarStyle>,
    pub appearance: std::collections::BTreeMap<String, appearance::Appearance>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub separators: std::collections::BTreeMap<String, structure::SeparatorStyle>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub drawers: std::collections::BTreeMap<String, drawers::Drawer>,
    /// What fixed blocks keep for when they become drawers or floating
    /// blocks, under one of their buttons (`drawers::BlockProfile`).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub block_profiles: std::collections::BTreeMap<String, drawers::BlockProfile>,
    /// The interfaces whose eye is closed: the Interfaces block does not
    /// show them (`drawers::INTERFACES`; a built-in preset's key or a
    /// personal interface's item id).
    #[serde(skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub hidden_interfaces: std::collections::BTreeSet<String>,
    /// Keyboard shortcuts of buttons, drawers, floating blocks and blocks
    /// (`shortcuts.rs`).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub shortcuts: std::collections::BTreeMap<String, crate::keybinds::Binding>,
    /// Explicit menu placements; commands absent here follow their native menu.
    pub menu_layout: std::collections::BTreeMap<String, Vec<String>>,
    /// None preserves the existing visible address bar.
    pub address_visible: Option<bool>,
    /// Older layouts acquire the native top controls once, keeping their buttons.
    pub composed: bool,
    /// Migrate the formerly implicit nearby-chat island once. Removal stays saved.
    pub chat_composed: bool,
}

impl ToolbarSettings {
    pub fn compose_chat(&mut self, layout: &Layout) -> bool {
        if self.chat_composed {
            return false;
        }
        if self.location("bar_chat", layout).is_none() {
            let mut buttons = self.buttons(layout).to_vec();
            if !buttons.is_empty() {
                buttons.insert(0, structure::separator("chat-tools"));
            }
            buttons.insert(0, crate::ui::bars::top::button("bar_chat", "message-filled"));
            self.buttons = Some(buttons);
            self.block_styles
                .entry("bar_chat".into())
                .or_default()
                .align
                .get_or_insert(Align::Start);
        }
        self.chat_composed = true;
        true
    }

    pub fn show_address_bar(&mut self, visible: bool) {
        self.address_visible = Some(visible);
        if visible
            && self.composed
            && !self
                .buttons
                .iter()
                .flatten()
                .chain(&self.top)
                .chain(&self.left)
                .chain(&self.right)
                .any(|button| button.command == "bar_navigation")
            && self.drawer_of("bar_navigation").is_none()
        {
            if !self.top.is_empty() {
                self.top.push(structure::separator(&uuid::Uuid::new_v4().to_string()));
            }
            self.top.push(crate::ui::bars::top::button("bar_navigation", "map-pin-filled"));
        }
    }
    /// Every bar's and block's gaps that follow one another made one
    /// (`structure::single_gaps`); a bottom bar still the skin's is left.
    pub fn single_gaps(&mut self) -> bool {
        let mut removed = Vec::new();
        for list in self
            .buttons
            .iter_mut()
            .chain([&mut self.top, &mut self.left, &mut self.right])
            .chain(self.drawers.values_mut().map(|drawer| &mut drawer.buttons))
        {
            removed.extend(structure::single_gaps(list));
        }
        for command in &removed {
            self.separators.remove(command);
        }
        !removed.is_empty()
    }

    pub fn compose_top(&mut self) -> bool {
        if self.composed {
            return self.split_chrome_blocks();
        }
        self.top = structure::top_buttons(&self.top);
        self.composed = true;
        self.split_chrome_blocks();
        self.sanitize();
        true
    }

    /// Replace aggregates in place, on every edge, without restoring removed
    /// controls or moving user separators. No version flag is needed: new IDs
    /// never expand again. Custom appearances follow each derived control.
    fn split_chrome_blocks(&mut self) -> bool {
        if !self
            .top
            .iter()
            .chain(&self.left)
            .chain(&self.right)
            .chain(self.buttons.iter().flatten())
            .any(|button| matches!(button.command.as_str(), "bar_account" | "bar_system"))
        {
            return false;
        }
        let present: std::collections::HashSet<_> = self
            .top
            .iter()
            .chain(&self.left)
            .chain(&self.right)
            .chain(self.buttons.iter().flatten())
            .map(|button| button.command.clone())
            .collect();
        let mut changed = false;
        for list in [&mut self.top, &mut self.left, &mut self.right]
            .into_iter()
            .chain(self.buttons.iter_mut())
        {
            let mut next = Vec::new();
            for button in list.iter() {
                let parts: &[&str] = match button.command.as_str() {
                    "bar_account" => &["bar_status", structure::SEPARATOR, "bar_balance"],
                    "bar_system" => &[
                        "bar_clock",
                        "bar_ping",
                        "bar_network",
                        structure::SEPARATOR,
                        "bar_fps",
                        structure::SEPARATOR,
                        "bar_media",
                        "bar_music",
                        "bar_volume",
                    ],
                    _ => {
                        next.push(button.clone());
                        continue;
                    }
                };
                changed = true;
                for &id in parts {
                    if id == structure::SEPARATOR {
                        next.push(structure::separator(&uuid::Uuid::new_v4().to_string()));
                        continue;
                    }
                    if present.contains(id) {
                        continue;
                    }
                    let command = catalogue::find(id).expect("internal chrome command");
                    next.push(ToolButton {
                        command: id.into(),
                        icon: command.icon.into(),
                        tooltip: String::new(),
                    });
                    if let Some(appearance) = self.appearance.get(&button.command).cloned() {
                        self.appearance.entry(id.into()).or_insert(appearance);
                    }
                }
            }
            if changed && *list != next {
                *list = next;
            }
        }
        if changed {
            self.sanitize();
        }
        changed
    }

    pub fn set_bug_visible(&mut self, layout: &Layout, shown: bool) -> bool {
        if self.placed("report_bug", layout) == shown {
            return false;
        }
        if shown {
            let at = self
                .top
                .iter()
                .position(|button| button.command == "aurora_menu")
                .map_or(0, |index| index + 1);
            let button = ToolButton {
                command: "report_bug".into(),
                icon: "bug-filled".into(),
                tooltip: String::new(),
            };
            if at > 0 {
                self.top.insert(at, structure::separator(&uuid::Uuid::new_v4().to_string()));
                self.top.insert(at + 1, button);
            } else {
                self.top.insert(0, button);
                if self.top.len() > 1 {
                    self.top.insert(1, structure::separator(&uuid::Uuid::new_v4().to_string()));
                }
            }
        } else {
            if self.buttons(layout).iter().any(|button| button.command == "report_bug") {
                self.buttons = Some(
                    self.buttons(layout)
                        .iter()
                        .filter(|button| button.command != "report_bug")
                        .cloned()
                        .collect(),
                );
            }
            for buttons in [&mut self.top, &mut self.left, &mut self.right] {
                buttons.retain(|button| button.command != "report_bug");
            }
            for drawer in self.drawers.values_mut() {
                drawer.buttons.retain(|button| button.command != "report_bug");
            }
        }
        true
    }
    pub fn migrate_vr_button(&mut self) -> bool {
        if self.vr_button_migrated {
            return false;
        }
        self.vr_button_migrated = true;
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        if !self.left.iter().chain(&self.right).chain(&self.top).any(|b| b.command == "vr_view")
            && let Some(buttons) = &mut self.buttons
            && !buttons.is_empty()
            && !buttons.iter().any(|b| b.command == "vr_view")
        {
            let at = buttons.iter().position(|b| b.command == "view").map_or(buttons.len(), |i| i + 1);
            buttons.insert(
                at,
                ToolButton {
                    command: "vr_view".into(),
                    icon: "device-vision-pro-filled".into(),
                    tooltip: String::new(),
                },
            );
        }
        true
    }

    /// One-time discovery of the Director (0.9.11): a bar the user arranged
    /// gets its button right after the photo button, on whichever bar that
    /// one stands, else at the end of the bottom bar. A bar that follows the
    /// skin already has it (`Layout::default`); an emptied bottom bar, a
    /// Director already placed or removed later are left alone.
    pub fn migrate_director_button(&mut self) -> bool {
        if self.director_button_migrated {
            return false;
        }
        self.director_button_migrated = true;
        let Some(bottom) = &mut self.buttons else {
            return true;
        };
        if bottom.is_empty() || bottom.iter().chain(&self.left).chain(&self.right).any(|b| b.command == "director") {
            return true;
        }
        let director = ToolButton {
            command: "director".into(),
            icon: "movie".into(),
            tooltip: String::new(),
        };
        let bar = [bottom, &mut self.left, &mut self.right]
            .into_iter()
            .find_map(|bar| bar.iter().position(|b| b.command == "snapshot").map(|i| (bar, i + 1)));
        match bar {
            Some((bar, at)) => bar.insert(at, director),
            None => self.buttons.get_or_insert_default().push(director),
        }
        true
    }

    /// "Customize interface" on a bar becomes the Interfaces block
    /// (`drawers::migrate_interfaces`).
    pub fn migrate_interfaces(&mut self) -> bool {
        drawers::migrate_interfaces(self)
    }

    /// The interfaces' eyes and shortcuts stay whatever interface is chosen
    /// (`drawers::keep_interface_choices`).
    pub fn keep_interface_choices(&mut self, previous: Option<&ToolbarSettings>) {
        if let Some(previous) = previous {
            drawers::keep_interface_choices(self, previous);
        }
    }

    /// Build - Edit and Build - Create took the build window's icons
    /// (2026-11-24): a button saved with the former ones, on any bar or in
    /// a drawer, takes the new ones; an icon the user chose is in its
    /// appearance and stays.
    pub fn migrate_build_icons(&mut self) -> bool {
        const FORMER: [(&str, &str); 2] = [("build_edit", "pencil-filled"), ("build_create", "circle-plus-filled")];
        let mut changed = false;
        let drawers = self.drawers.values_mut().flat_map(|drawer| drawer.buttons.iter_mut());
        let bars = self
            .buttons
            .iter_mut()
            .flatten()
            .chain(&mut self.left)
            .chain(&mut self.right)
            .chain(&mut self.top);
        for button in bars.chain(drawers) {
            if let Some((command, _)) = FORMER
                .iter()
                .find(|(command, icon)| button.command == *command && button.icon == *icon)
                && let Some(current) = catalogue::find(command)
            {
                button.icon = current.icon.into();
                changed = true;
            }
        }
        changed
    }

    pub fn buttons<'a>(&'a self, layout: &'a Layout) -> &'a [ToolButton] {
        self.buttons.as_deref().unwrap_or(&layout.toolbar)
    }

    pub fn bar<'a>(&'a self, bar: Bar, layout: &'a Layout) -> &'a [ToolButton] {
        match bar {
            Bar::Bottom => self.buttons(layout),
            Bar::Left => &self.left,
            Bar::Right => &self.right,
            Bar::Top => &self.top,
        }
    }

    pub fn style(&self, bar: Bar) -> BarStyle {
        match bar {
            Bar::Bottom => self.bottom_style,
            Bar::Left => self.left_style,
            Bar::Right => self.right_style,
            Bar::Top => self.top_style,
        }
    }

    fn style_mut(&mut self, bar: Bar) -> &mut BarStyle {
        match bar {
            Bar::Bottom => &mut self.bottom_style,
            Bar::Left => &mut self.left_style,
            Bar::Right => &mut self.right_style,
            Bar::Top => &mut self.top_style,
        }
    }

    /// The toolbar `command` stands in (LLToolBarView::hasCommand).
    pub fn location(&self, command: &str, layout: &Layout) -> Option<Bar> {
        Bar::ALL.into_iter().find(|&bar| {
            self.bar(bar, layout).iter().any(|b| {
                b.command == command
                    || self
                        .drawers
                        .get(&b.command)
                        .is_some_and(|drawer| drawer.buttons.iter().any(|b| b.command == command))
            })
        })
    }

    /// Owned identity so callers can compare a live control before and after a
    /// commit, including moves into, within and out of a placed drawer.
    pub fn command_placement(&self, command: &str, layout: &Layout) -> Option<(Bar, Option<String>, usize)> {
        for bar in Bar::ALL {
            for (index, button) in self.bar(bar, layout).iter().enumerate() {
                if button.command == command {
                    return Some((bar, None, index));
                }
                if let Some(index) = self
                    .drawers
                    .get(&button.command)
                    .and_then(|drawer| drawer.buttons.iter().position(|b| b.command == command))
                {
                    return Some((bar, Some(button.command.clone()), index));
                }
            }
        }
        None
    }

    /// Whether `command` stands anywhere: a toolbar, a drawer of one, or a
    /// floating block.
    pub fn placed(&self, command: &str, layout: &Layout) -> bool {
        self.location(command, layout).is_some()
            || self
                .drawers
                .values()
                .any(|drawer| drawer.floating.is_some() && drawer.buttons.iter().any(|b| b.command == command))
    }

    /// The quick import bar's floating block, once (`drawers::IMPORT_BAR`).
    pub fn ensure_import_bar(&mut self, layout: &Layout) -> bool {
        drawers::ensure_import_bar(self, layout)
    }

    /// The drawer, placed, or the floating block `command` stands in.
    pub fn drawer_of(&self, command: &str) -> Option<&str> {
        self.drawers
            .iter()
            .find(|(id, drawer)| {
                drawer.buttons.iter().any(|b| b.command == command)
                    && (drawer.floating.is_some()
                        || self
                            .top
                            .iter()
                            .chain(&self.left)
                            .chain(&self.right)
                            .chain(self.buttons.iter().flatten())
                            .any(|b| &b.command == *id))
            })
            .map(|(id, _)| id.as_str())
    }

    pub(super) fn set_bar(&mut self, bar: Bar, buttons: Vec<ToolButton>) {
        match bar {
            Bar::Top => self.top = buttons,
            Bar::Left => self.left = buttons,
            Bar::Right => self.right = buttons,
            Bar::Bottom => self.buttons = Some(buttons),
        }
    }

    /// No empty or retired command (`howto`), and each command once over the four toolbars
    /// (the bottom one first).
    pub fn sanitize(&mut self) {
        self.migrate_interfaces();
        self.separators.retain(|command, style| {
            *style = style.sanitized();
            command.starts_with(structure::PREFIX) && *style != Default::default()
        });
        let mut seen = std::collections::HashSet::new();
        if let Some(buttons) = &mut self.buttons {
            for button in buttons.iter_mut() {
                if matches!(button.command.as_str(), structure::SEPARATOR | structure::DIVIDER) {
                    *button = structure::instantiate(button.clone());
                }
            }
            buttons.retain(|b| kept_on_bar(&b.command) && seen.insert(b.command.clone()));
        }
        for side in [&mut self.left, &mut self.right, &mut self.top] {
            for button in side.iter_mut() {
                if matches!(button.command.as_str(), structure::SEPARATOR | structure::DIVIDER) {
                    *button = structure::instantiate(button.clone());
                }
            }
            side.retain(|b| kept_on_bar(&b.command) && seen.insert(b.command.clone()));
        }
        // a floating block stays without its button; it holds neither a
        // drawer, nor a block of the bars, nor another floating block
        self.drawers.retain(|id, drawer| {
            if !drawers::is_drawer(id) || (!seen.contains(id) && drawer.floating.is_none()) {
                return false;
            }
            let floating = drawer.floating.is_some();
            // the Interfaces block's buttons are the interfaces, made as it
            // is drawn (`drawers::shown_buttons`)
            if id == drawers::INTERFACES {
                drawer.buttons.clear();
            }
            for button in &mut drawer.buttons {
                if matches!(button.command.as_str(), structure::SEPARATOR | structure::DIVIDER) {
                    *button = structure::instantiate(button.clone());
                }
            }
            drawer.buttons.retain(|b| {
                !b.command.is_empty()
                    && !catalogue::retired(&b.command)
                    && !drawers::is_drawer(&b.command)
                    && b.command != drawers::FLOATING
                    && !(floating && structure::is_block(&b.command))
                    && seen.insert(b.command.clone())
            });
            true
        });
        // Block membership is reconciled on edits with the actual skin layout;
        // the default skin cannot validate an inherited custom bottom toolbar.
    }
}

/// A titled part of a right click, under a line when something is above it.
pub(crate) fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    native_context::start_group(ui, p);
    menu::heading(ui, p, title);
}

/// A bar keeps no empty or retired command, nor "Floating block" (it becomes
/// one).
fn kept_on_bar(command: &str) -> bool {
    !command.is_empty() && !catalogue::retired(command) && command != drawers::FLOATING
}

#[derive(Clone)]
struct Drag {
    button: ToolButton,
    /// The toolbar it was taken from; None from the window's catalogue.
    from: Option<Bar>,
    /// A whole root block; empty for an individual control.
    members: Vec<ToolButton>,
}

enum Edit {
    Insert(Bar, ToolButton, usize),
    InsertGroup(Bar, ToolButton, usize),
    MoveBlock(Bar, Vec<ToolButton>, usize, bool),
    DrawerBlock(String, Vec<ToolButton>, usize),
    RemoveBlock(Vec<String>),
    Remove(String),
    Restore,
    Clear,
    Lock,
    Style(Bar, BarStyle),
    BlockStyle(Bar, String, BarStyle),
    Appearance(String, appearance::Appearance),
    Separator(String, structure::SeparatorStyle),
    Split(Bar, String, bool),
    Merge(Bar, String, bool),
    Fold(Bar, String),
    Unfold(Bar, String),
    DrawerInsert(String, ToolButton, usize),
    DrawerFromDrop(Bar, String, Vec<ToolButton>, ButtonMode),
    /// The fields of "Edit block", for any type of block.
    BlockOptions(Box<block_window::Change>),
    MenuInsert(String, String, usize),
    /// A new floating block, dropped there on the screen.
    FloatingNew(Pos2),
    /// A floating block shown or hidden, locked bars included.
    FloatingShown(String, bool),
    /// An interface's eye opened or closed (`drawers::show_interface`).
    InterfaceShown(String, bool),
    /// A drawer, or the block of the button, made a floating block there.
    Float(Bar, String, Pos2),
    /// A floating block made a drawer (true) or a block of its bar again.
    Dock(String, bool),
    /// A shortcut set or removed, locked bars included.
    Shortcut(String, Option<crate::keybinds::Binding>),
}

/// Windows whose toolbar button lights while they are open
/// (is_running_function in commands.xml), given by the app each frame.
#[derive(Clone, Copy, Default)]
pub struct Shown {
    /// Avatar.IsMyProfileOpen.
    pub profile: bool,
    /// Floater.IsOpen build.
    pub build: bool,
    /// Camera, Move, Edit, Create and Land lit, in this order
    /// (BuildTool::lit; Aurora's commands).
    pub build_tools: [bool; 5],
    /// Floater.IsOpen fs_poser.
    pub poser: bool,
    /// Floater.IsOpen reporter.
    pub report_abuse: bool,
    /// Floater.IsOpen marketplace_listings.
    pub marketplace_listings: bool,
    /// Floater.IsOpen fs_teleporthistory: Places on its history tab.
    pub teleport_history: bool,
}

impl Shown {
    /// Whether the button of `command` lights, when it is one of these.
    pub fn lit(&self, command: &str) -> Option<bool> {
        Some(match command {
            "profile" => self.profile,
            "build" => self.build,
            "build_focus" => self.build_tools[0],
            "build_grab" => self.build_tools[1],
            "build_edit" => self.build_tools[2],
            "build_create" => self.build_tools[3],
            "build_land" => self.build_tools[4],
            "poser" => self.poser,
            "reporter" => self.report_abuse,
            "marketplacelistings" => self.marketplace_listings,
            "teleport_history" => self.teleport_history,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub struct ToolbarUi {
    pub open: bool,
    preset_view: selection::View,
    pub preset_request: Option<presets::Preset>,
    pub(crate) inferred_preset: Option<presets::Preset>,
    pub(crate) notecards: notecards::State,
    pub ao_quick: super::ao::quick::Quick,
    pub voice_options: bool,
    pub movelock_options: bool,
    pub movelock_active: bool,
    pub movement_active: bool,
    pub water_walk_active: bool,
    /// Our profile is shown on its Picks tab (Avatar.IsPicksTabOpen).
    pub picks_active: bool,
    /// The photo reel holds no photo (given by the app): its button is
    /// dimmed. And its quick view is open: the button is lit.
    pub photo_roll_empty: bool,
    /// An object we own is selected (given by the app before any bar is
    /// drawn): the Edit particles button runs (LLEnableEditParticleSource),
    /// otherwise it is dimmed.
    pub edit_particles: bool,
    /// Fly override (FSAlwaysFly) and Quickjump are on (given by the app
    /// before any bar is drawn): their buttons are lit.
    pub fly_override_active: bool,
    pub quickjump_active: bool,
    /// The grid has an Avatar Welcome Pack (given by the app): without one,
    /// its button is dimmed (GridFeatureCheck avatar_picker).
    pub avatar_welcome: bool,
    /// The grid has a Destination Guide (given by the app): without one,
    /// its button is dimmed (GridFeatureCheck destination_guide).
    pub destinations: bool,
    pub photo_roll_active: bool,
    /// Gaming mode is on (app/gaming_mode.rs).
    pub gaming_mode_active: bool,
    /// The nearby chat bar is shown (MainChatbarVisible).
    pub chat_bar_active: bool,
    /// The windows the app opens itself, for their buttons' highlight.
    pub shown: Shown,
    /// "Show Friends Only" is on (FSRenderFriendsOnly).
    pub friends_only_active: bool,
    pub show_name_tags: bool,
    /// World › HUDs › Show HUDs is on (given by the app): the HUDs button
    /// lights while they are hidden.
    pub show_huds: bool,
    /// The interface is hidden (given by the app): its button, reached
    /// through a screen edge, lights and shows it again.
    pub interface_hidden: bool,
    pub movelock_status: String,
    /// The violet of the Grid status button's flash (0: none), given by
    /// the app on a new entry of the grid's status feed.
    pub grid_status_flash: f32,
    /// Where the favourites live (given by the app), for the Favorites
    /// button's menu, and the choice made there (taken by the app).
    pub favorites_access: crate::settings::FavoritesAccess,
    pub access_choice: Option<crate::settings::FavoritesAccess>,
    /// Whether nearby chat flashes the Conversations button (given by the
    /// app), for that button's menu, and the choice made there (taken by
    /// the app).
    pub flash_nearby: bool,
    pub conversations_menu: Option<crate::ui::chat::ButtonMenu>,
    pub has_open_ims: bool,
    /// The look the bars are drawn in (given by the app): the islands' dock
    /// or Firestorm's bars.
    pub islands: bool,
    pending: Option<Edit>,
    /// true restores defaults, false clears all (Firestorm's confirmations).
    confirm: Option<bool>,
    /// The window's filter.
    search: String,
    category: Option<catalogue_view::Category>,
    advanced_catalogue: bool,
    appearance_editor: appearance::Editor,
    drawer_appearance_editor: appearance::Editor,
    context_icon_picker: appearance::ContextPicker,
    menu_row_context: Option<appearance::MenuRowContext>,
    pub(crate) selected: Option<String>,
    drawer_editor: Option<String>,
    /// A fixed block's settings window, while open: its bar, one of its
    /// buttons and its bar's style.
    block_editor: Option<(Bar, String, Style)>,
    pub(crate) drawer_action: Option<crate::ui::bars::BarAction>,
    /// A floating block just made, and where it was dropped.
    floating_placed: Option<(String, Pos2)>,
    /// The shortcut window, while open.
    shortcut_editor: Option<shortcuts::Editor>,
    /// The Preferences shortcuts (given by the app while the shortcut window
    /// is open), for its warnings.
    pub keybinds: Option<crate::keybinds::KeyBindings>,
}

impl ToolbarUi {
    pub(crate) fn profile_applied(&mut self, ctx: &egui::Context) {
        self.open = true;
        self.pending = None;
        self.confirm = None;
        self.preset_request = None;
        self.inferred_preset = None;
        self.selected = None;
        self.drawer_editor = None;
        self.block_editor = None;
        self.drawer_action = None;
        self.floating_placed = None;
        self.shortcut_editor = None;
        self.appearance_editor = Default::default();
        self.drawer_appearance_editor = Default::default();
        self.context_icon_picker = Default::default();
        self.menu_row_context = None;
        egui::DragAndDrop::clear_payload(ctx);
        egui::Popup::close_all(ctx);
    }

    /// Opens a drawer's or a floating block's settings ("Edit block…"), in
    /// place of a fixed block's.
    fn edit_drawer(&mut self, id: &str) {
        self.block_editor = None;
        self.drawer_editor = Some(id.into());
    }

    /// Opens the settings of the fixed block of `command` in `bar`, in place
    /// of a drawer's.
    fn edit_block(&mut self, bar: Bar, command: &str, base: Style) {
        self.drawer_editor = None;
        self.block_editor = Some((bar, command.into(), base));
    }
}

/// Built-in labels stay live when the language changes, including saved
/// buttons; a skin may rename a command, and keeps its name.
pub fn label(button: &ToolButton) -> &str {
    if button.command == drawers::IMPORT_BAR && button.tooltip.is_empty() {
        return aurora_i18n::tr!("toolbar-import-bar");
    }
    if button.command == drawers::INTERFACES && button.tooltip.is_empty() {
        return aurora_i18n::tr!("toolbar-interfaces");
    }
    if drawers::is_drawer(&button.command) && button.tooltip.is_empty() {
        return aurora_i18n::tr!("toolbar-drawer");
    }
    if structure::is_separator(&button.command) {
        return aurora_i18n::text("toolbar-spacer");
    }
    if structure::is_divider(&button.command) {
        return aurora_i18n::text("toolbar-separator");
    }
    if button.command.starts_with(commands::PREFIX)
        && let Some(key) = button.tooltip.strip_prefix('@')
        && let Some(text) = aurora_i18n::lookup(key)
    {
        return text;
    }
    match catalogue::find(&button.command) {
        Some(command)
            if button.tooltip.is_empty()
                || aurora_i18n::available_languages().any(|(language, _)| button.tooltip == command.label_in(language)) =>
        {
            command.label()
        }
        _ => &button.tooltip,
    }
}

/// Whether Aurora runs the command (else its button is dimmed, with "Not
/// available in Aurora yet").
pub fn ready(command: &str) -> bool {
    catalogue::ready(command)
}

/// Every command of the window: the skin's, then the catalogue's, sorted by
/// their label (LLFloaterToybox::postBuild).
fn catalogue(layout: &Layout) -> Vec<ToolButton> {
    let mut buttons = layout.toolbar.clone();
    buttons.extend(Layout::default().toolbar);
    buttons.extend(COMMANDS.iter().map(|c| ToolButton {
        command: c.id.into(),
        icon: c.icon.into(),
        tooltip: String::new(),
    }));
    let mut seen = std::collections::HashSet::new();
    buttons.retain(|b| !matches!(b.command.as_str(), "bar_account" | "bar_system") && seen.insert(b.command.clone()));
    buttons.sort_by_cached_key(|b| sort_key(label(b)));
    buttons
}

/// A label as the dictionary orders it: lower case, without accents
/// (LLFloaterToybox sorts with the language's collation).
fn sort_key(text: &str) -> String {
    let mut key = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => key.push('a'),
            'é' | 'è' | 'ê' | 'ë' => key.push('e'),
            'î' | 'ï' | 'í' | 'ì' => key.push('i'),
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => key.push('o'),
            'ù' | 'û' | 'ü' | 'ú' => key.push('u'),
            'ç' => key.push('c'),
            'ñ' => key.push('n'),
            'œ' => key.push_str("oe"),
            'æ' => key.push_str("ae"),
            '’' | '\'' => {}
            c => key.push(c),
        }
    }
    key
}

/// The pointer selects a boundary between buttons; use their real rectangles,
/// including the scroll offset, instead of dividing the bar into equal slots.
fn insertion(rects: &[Rect], pos: egui::Pos2, vertical: bool) -> usize {
    rects
        .iter()
        .position(|r| if vertical { pos.y < r.center().y } else { pos.x < r.center().x })
        .unwrap_or(rects.len())
}

impl ToolbarUi {
    /// Shared final row of interface-customization context menus.
    pub(crate) fn interface_edit_menu(&mut self, ui: &mut egui::Ui, p: &Palette, command: Option<&str>) {
        if native_context::item(ui, p) {
            self.open = true;
            let settings = appearance::settings(ui.ctx());
            self.selected = command
                .filter(|command| settings.placed(command, &Layout::default()))
                .map(str::to_owned);
        }
    }

    /// Closed menus with no menu-command buttons need no per-frame traversal.
    pub fn needs_commands(&self, ctx: &egui::Context, settings: Option<&ToolbarSettings>) -> bool {
        self.open
            || egui::Popup::is_any_open(ctx)
            || commands::catalogue(ctx).entries.is_empty()
            || settings.is_some_and(|settings| {
                settings
                    .buttons
                    .iter()
                    .flatten()
                    .chain(&settings.top)
                    .chain(&settings.left)
                    .chain(&settings.right)
                    .chain(settings.drawers.values().flat_map(|drawer| &drawer.buttons))
                    .any(|button| button.command.starts_with(commands::PREFIX))
            })
    }

    /// Individual properties stay available without exposing layout operations.
    pub(crate) fn button_appearance_option(&mut self, ui: &mut egui::Ui, p: &Palette, button: Option<&ToolButton>, locked: bool) {
        if let Some(button) = button
            && structure::is_separator(&button.command)
            && menu::item_if(ui, p, "arrows-horizontal", aurora_i18n::tr!("toolbar-separator-settings"), !locked)
        {
            self.appearance_editor.select(button);
        }
        if let Some(button) = button
            && !structure::is_separator(&button.command)
            && !structure::is_divider(&button.command)
            && !structure::is_block(&button.command)
        {
            if self.open {
                if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-button-appearance"), !locked) {
                    self.appearance_editor.select(button);
                }
            } else {
                let settings = appearance::settings(ui.ctx());
                if let Some(value) = self.context_icon_picker.options(ui, p, button, &button.command, &settings, locked) {
                    self.pending = Some(Edit::Appearance(button.command.clone(), value));
                }
            }
        }
    }

    fn action_options(&mut self, ui: &mut egui::Ui, p: &Palette, button: Option<&ToolButton>) {
        if button.is_some_and(|b| b.command == "speak")
            && menu::item(ui, p, "microphone-filled", aurora_i18n::tr!("ui-bars-microphone-options"))
        {
            self.voice_options = true;
        }
        if button.is_some_and(|b| matches!(b.command.as_str(), "movelock" | "move_lock"))
            && menu::item(ui, p, "lock-cog", aurora_i18n::tr!("movelock-options"))
        {
            self.movelock_options = true;
        }
        // the Conversations button: open the window, leave nearby chat
        // out of its flash
        if button.is_some_and(|b| b.command == "conversations") {
            menu::separator(ui, p);
            if let Some(choice) = crate::ui::chat::button_menu(ui, p, self.flash_nearby, self.has_open_ims) {
                self.conversations_menu = Some(choice);
            }
        }
        // the Favorites button: where the favourites live
        if button.is_some_and(|b| b.command == "favorites")
            && let Some(a) = crate::ui::bars::favorites::access_choice(ui, p, self.favorites_access)
        {
            self.access_choice = Some(a);
        }
    }

    pub fn context(&mut self, response: &Response, p: &Palette, bar: Bar, style: Style, button: Option<&ToolButton>, locked: bool) {
        let settings = appearance::settings(&response.ctx);
        let bar = button
            .and_then(|b| settings.location(&b.command, &Layout::default()))
            .unwrap_or(bar);
        // Buttons such as Favorites also own a left-click dropdown.
        menu::context_menu_of_button(response, p, |ui| {
            if !self.open
                && let Some(button) = button.filter(|button| drawers::is_floating(&settings, &button.command))
            {
                floating::context(ui, p, self, &button.command, locked);
                self.interface_edit_menu(ui, p, Some(&button.command));
                return;
            }
            // a drawer's button (a floating block's while the bars are
            // edited) is its block: its type and its settings (its shortcut
            // is in "Edit block…", human request, 2026-11-26)
            if let Some(button) = button.filter(|button| drawers::is_drawer(&button.command)) {
                section(ui, p, aurora_i18n::tr!("toolbar-block-settings"));
                // the Interfaces block is a drawer, always
                if button.command != drawers::INTERFACES {
                    blocks::kind_menu(ui, p, self, &settings, &button.command, Some((bar, response.rect)));
                }
                if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-block-edit"), !locked) {
                    self.edit_drawer(&button.command);
                }
                native_context::start_group(ui, p);
                self.interface_edit_menu(ui, p, Some(&button.command));
                return;
            }
            // a button's own settings, then its block's (human request,
            // 2026-11-25: "Button settings" and "Block settings")
            if !self.open {
                self.action_options(ui, p, button);
                let own = button.filter(|button| {
                    !structure::is_separator(&button.command)
                        && !structure::is_divider(&button.command)
                        && !structure::is_block(&button.command)
                });
                if own.is_some() {
                    section(ui, p, aurora_i18n::tr!("toolbar-button-settings"));
                } else {
                    native_context::start_group(ui, p);
                }
                self.button_appearance_option(ui, p, button, locked);
                // its shortcut after its look and its icon (human request,
                // 2026-11-25)
                if let Some(button) = own.filter(|button| shortcuts::takes(&button.command)) {
                    self.shortcut_item(ui, p, &settings, &button.command);
                }
                if let Some(button) = button.filter(|button| {
                    !structure::is_separator(&button.command)
                        && !structure::is_divider(&button.command)
                        && settings.drawer_of(&button.command).is_none()
                }) {
                    section(ui, p, aurora_i18n::tr!("toolbar-block-settings"));
                    blocks::kind_menu(ui, p, self, &settings, &button.command, Some((bar, response.rect)));
                    // a fixed block's settings, as a drawer's (human request,
                    // 2026-11-26)
                    if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-block-edit"), !locked) {
                        self.edit_block(bar, &button.command, style);
                    }
                }
                native_context::start_group(ui, p);
                self.interface_edit_menu(ui, p, button.map(|button| button.command.as_str()));
                return;
            }
            if self.open
                && let Some(button) = button
            {
                let (_, buttons) = drawers::contents(&settings, bar, &button.command, &Layout::default());
                for before in [true, false] {
                    if structure::is_separator(&button.command) && !before {
                        continue;
                    }
                    if !structure::is_separator(&button.command)
                        && menu::item_if(
                            ui,
                            p,
                            "separator-vertical",
                            aurora_i18n::text(if before {
                                "toolbar-edit-split-before"
                            } else {
                                "toolbar-edit-split-after"
                            }),
                            !locked && edit_mode::split_allowed(&buttons, &button.command, before),
                        )
                    {
                        self.pending = Some(Edit::Split(bar, button.command.clone(), before));
                    }
                    // a separator's own: the blocks on each side of it
                    if structure::is_separator(&button.command)
                        && menu::item_if(
                            ui,
                            p,
                            "arrows-join",
                            aurora_i18n::tr!("toolbar-edit-merge-groups"),
                            !locked && edit_mode::merge_range(&buttons, &button.command, before).is_some(),
                        )
                    {
                        self.pending = Some(Edit::Merge(bar, button.command.clone(), before));
                    }
                }
                // the block's type, merges and shortcut are in its block's menu
                // (human request, 2026-11-25)
                if shortcuts::takes(&button.command) {
                    self.shortcut_item(ui, p, &settings, &button.command);
                }
                menu::separator(ui, p);
            }
            if !locked && let Some(button) = button {
                if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("toolbar-remove")) {
                    self.pending = Some(Edit::Remove(button.command.clone()));
                }
                menu::separator(ui, p);
            }
            self.action_options(ui, p, button);
            if button.is_none_or(|b| settings.drawer_of(&b.command).is_none()) {
                menu::separator(ui, p);
                if let Some(button) = button.filter(|button| !structure::is_separator(&button.command)) {
                    blocks::style_menu(ui, p, self, &settings, bar, &button.command, style);
                } else {
                    let mut next = None;
                    for (align, icon, label) in align_rows(bar) {
                        if menu::check(ui, p, icon, label, style.align == align, true) {
                            next = Some(BarStyle {
                                align: Some(align),
                                ..Default::default()
                            });
                        }
                    }
                    // The background controls the whole edge; button/block defaults
                    // belong to the selected block in customization mode.
                    if let Some(next) = next {
                        self.pending = Some(Edit::Style(bar, next));
                    }
                }
            }
            menu::separator(ui, p);
            if menu::check(ui, p, "lock-filled", aurora_i18n::tr!("toolbar-locked"), locked, true) {
                self.pending = Some(Edit::Lock);
            }
            native_context::start_group(ui, p);
            self.button_appearance_option(ui, p, button, locked);
            self.interface_edit_menu(ui, p, button.map(|button| button.command.as_str()));
        });
    }

    pub fn button(&mut self, response: &Response, bar: Bar, style: Style, button: &ToolButton, p: &Palette, locked: bool) {
        let style = blocks::style(&response.ctx, &button.command, style);
        self.context(response, p, bar, style, Some(button), locked);
        self.button_source(response, bar, style, button, locked);
        if self.open && structure::is_separator(&button.command) && response.clicked() {
            self.selected = Some(button.command.clone());
            self.appearance_editor.select(button);
        }
    }

    /// An Interfaces block's button's right click: its shortcut only, its
    /// block's content being its own (human request, 2026-11-26).
    pub(crate) fn interface_context(&mut self, response: &Response, p: &Palette, command: &str) {
        let settings = appearance::settings(&response.ctx);
        menu::context_menu_of_button(response, p, |ui| {
            self.shortcut_item(ui, p, &settings, command);
        });
    }

    /// A click on an Interfaces block's button (`drawers::INTERFACE_PREFIX`
    /// and its key): its preset, or its personal interface, is applied.
    pub(crate) fn interface_clicked(&mut self, key: &str) {
        if let Some(preset) = presets::Preset::from_key(key) {
            self.preset_request = Some(preset);
        } else if let Ok(item) = key.parse() {
            self.notecards.request_load(item);
        }
    }

    /// `button`, or a floating block's button (`floating`): there a drag
    /// moves the whole block, never the button (human request, 2026-11-25).
    /// While the bars are edited its gaps and separators move like its
    /// buttons (human request, 2026-11-25): only its glass moves the block.
    #[allow(clippy::too_many_arguments)]
    pub fn button_in(
        &mut self,
        response: &Response,
        bar: Bar,
        style: Style,
        button: &ToolButton,
        p: &Palette,
        locked: bool,
        floating: bool,
    ) {
        if !floating || Self::intercepting(&response.ctx) {
            return self.button(response, bar, style, button, p, locked);
        }
        let style = blocks::style(&response.ctx, &button.command, style);
        self.context(response, p, bar, style, Some(button), locked);
        if response.dragged_by(egui::PointerButton::Primary) {
            floating::dragged(&response.ctx, &button.command);
        }
        if self.open && structure::is_separator(&button.command) && response.clicked() {
            self.selected = Some(button.command.clone());
            self.appearance_editor.select(button);
        }
    }

    pub(crate) fn button_source(&self, response: &Response, bar: Bar, style: Style, button: &ToolButton, locked: bool) {
        self.bar_source(response, button, bar, locked);
        surface_drop::collect(response, bar, style, button);
    }

    pub fn bar_source(&self, response: &Response, button: &ToolButton, bar: Bar, locked: bool) {
        if !locked && response.drag_started_by(egui::PointerButton::Primary) {
            response.dnd_set_drag_payload(Drag {
                button: button.clone(),
                from: Some(bar),
                members: Vec::new(),
            });
        }
    }

    /// Whether a toolbar button is being dragged: the empty toolbars show
    /// where it can go (LLToolBarView::draw, isToolDragged).
    pub fn dragging(ctx: &egui::Context) -> bool {
        egui::DragAndDrop::has_payload_of_type::<Drag>(ctx)
    }

    /// `response` covers the toolbar `bar` whose buttons stand at `rects`:
    /// while a button is dragged over it, the marker of where it would go;
    /// released there, it goes there.
    pub fn drop_bar(&mut self, response: &Response, bar: Bar, buttons: &[ToolButton], rects: &[Rect], locked: bool) {
        drop_preview::collect(response, bar, buttons, rects, locked);
    }

    /// Apply menu edits alongside drag/context edits. The current lock and
    /// membership win over a stale popup.
    pub fn commit_with_menus(&mut self, ctx: &egui::Context, layout: &Layout, settings: &mut ToolbarSettings) -> bool {
        let mut changed = self.commit(layout, settings);
        let edits = ctx
            .data_mut(|d| d.remove_temp::<Vec<MenuEdit>>(egui::Id::unique("menu_toolbar_edits")))
            .unwrap_or_default();
        for edit in edits {
            if settings.locked {
                continue;
            }
            let Some(command) = catalogue::find(edit.command).filter(|c| c.ready) else {
                continue;
            };
            let present = settings.placed(command.id, layout);
            if present != edit.remove {
                continue;
            }
            self.pending = Some(if edit.remove {
                Edit::Remove(command.id.to_owned())
            } else {
                Edit::Insert(
                    Bar::Bottom,
                    ToolButton {
                        command: command.id.to_owned(),
                        icon: command.icon.to_owned(),
                        tooltip: String::new(),
                    },
                    usize::MAX,
                )
            });
            changed |= self.commit(layout, settings);
        }
        changed
    }

    /// Save once for a real edit, never on hover, during dragging or on a no-op.
    pub fn commit(&mut self, layout: &Layout, settings: &mut ToolbarSettings) -> bool {
        self.commit_impl(layout, settings, false)
    }

    pub(crate) fn layout_configuration(ctx: &egui::Context) -> Option<std::sync::Arc<ToolbarSettings>> {
        drop_preview::configuration(ctx)
    }

    pub(crate) fn is_preview_item(ctx: &egui::Context, command: &str) -> bool {
        drop_preview::is_item(ctx, command)
    }

    fn proposed_drop(settings: &ToolbarSettings, layout: &Layout, bar: Bar, drag: &Drag, rank: usize, separate: bool) -> ToolbarSettings {
        let mut proposed = settings.clone();
        let mut transaction = Self {
            pending: Some(if !drag.members.is_empty() {
                Edit::MoveBlock(bar, drag.members.clone(), rank, separate)
            } else if separate {
                Edit::InsertGroup(bar, drag.button.clone(), rank)
            } else {
                Edit::Insert(bar, drag.button.clone(), rank)
            }),
            ..Default::default()
        };
        transaction.commit_impl(layout, &mut proposed, true);
        proposed
    }

    fn commit_impl(&mut self, layout: &Layout, settings: &mut ToolbarSettings, provisional: bool) -> bool {
        let moving: Vec<String> = match &self.pending {
            Some(Edit::Insert(_, button, _) | Edit::InsertGroup(_, button, _)) => vec![button.command.clone()],
            Some(Edit::MoveBlock(_, members, ..) | Edit::DrawerBlock(_, members, _)) => {
                members.iter().map(|button| button.command.clone()).collect()
            }
            _ => Vec::new(),
        };
        let counts = settings
            .drawers
            .iter()
            .map(|(id, drawer)| (id.clone(), drawer.buttons.len()))
            .collect();
        let changed = self.apply_edit(layout, settings, provisional);
        if changed {
            drawers::remove_emptied(settings, layout, &counts);
            settings.single_gaps();
            blocks::normalize_styles(settings, layout, &moving);
            shortcuts::forget_removed(settings);
        }
        changed
    }

    fn apply_edit(&mut self, layout: &Layout, settings: &mut ToolbarSettings, provisional: bool) -> bool {
        let Some(edit) = self.pending.take() else { return false };
        match &edit {
            // the Interfaces block stays, a drawer, its content its own
            // (human request, 2026-11-26): only its name, its icon and its
            // shortcuts change
            Edit::Remove(command) | Edit::Unfold(_, command) | Edit::Float(_, command, _) if command == drawers::INTERFACES => {
                return false;
            }
            // "Customize interface" alone is no button any more
            Edit::Insert(_, button, _) | Edit::InsertGroup(_, button, _) | Edit::DrawerInsert(_, button, _)
                if catalogue::retired(&button.command) =>
            {
                return false;
            }
            Edit::DrawerFromDrop(bar, target, sources, mode) => {
                return drawers::group_drop(settings, layout, *bar, target, sources, *mode);
            }
            Edit::BlockOptions(change) => return block_window::apply(settings, layout, change),
            Edit::FloatingShown(id, shown) => return drawers::show_floating(settings, id, *shown),
            Edit::InterfaceShown(key, shown) => return drawers::show_interface(settings, key, *shown),
            Edit::Shortcut(key, binding) => return shortcuts::set(settings, key, binding.clone()),
            Edit::Float(bar, command, pos) => {
                let Some(id) = drawers::float(settings, layout, *bar, command) else {
                    return false;
                };
                self.floating_placed = Some((id, *pos));
                return true;
            }
            Edit::Dock(id, drawer) => {
                let Some(bar) = drawers::dock(settings, layout, id) else {
                    return false;
                };
                if !*drawer {
                    self.pending = Some(Edit::Unfold(bar, id.clone()));
                    self.apply_edit(layout, settings, provisional);
                }
                return true;
            }
            Edit::FloatingNew(pos) => {
                if settings.locked || provisional {
                    return false;
                }
                let id = drawers::create_floating(settings, aurora_i18n::tr!("toolbar-floating-new"));
                self.floating_placed = Some((id, *pos));
                return true;
            }
            // "Interfaces" from the catalogue: its block, a drawer, made on
            // its first drop, then placed as a drawer's button is
            Edit::Insert(_, button, _) | Edit::InsertGroup(_, button, _)
                if button.command == drawers::INTERFACES && !settings.drawers.contains_key(drawers::INTERFACES) =>
            {
                if settings.locked {
                    return false;
                }
                settings.drawers.insert(drawers::INTERFACES.into(), drawers::Drawer::default());
            }
            // "Floating block" dropped on a bar: a new block, and its button
            // there (a stand-in while it is only previewed)
            Edit::Insert(bar, button, rank) | Edit::InsertGroup(bar, button, rank) if button.command == drawers::FLOATING => {
                if settings.locked {
                    return false;
                }
                let name = aurora_i18n::tr!("toolbar-floating-new");
                let id = if provisional {
                    drawers::FLOATING_PREVIEW.to_owned()
                } else {
                    drawers::create_floating(settings, name)
                };
                let toggle = drawers::floating_toggle(&id, name);
                self.pending = Some(if matches!(edit, Edit::InsertGroup(..)) {
                    Edit::InsertGroup(*bar, toggle, *rank)
                } else {
                    Edit::Insert(*bar, toggle, *rank)
                });
                self.apply_edit(layout, settings, provisional);
                return true;
            }
            Edit::MoveBlock(bar, members, rank, separate) => {
                return blocks::move_to(
                    settings,
                    layout,
                    members,
                    blocks::Destination::Bar(*bar, *rank, *separate),
                    provisional,
                );
            }
            Edit::DrawerBlock(id, _, _) | Edit::DrawerInsert(id, _, _) if id == drawers::INTERFACES => return false,
            Edit::DrawerBlock(id, members, rank) => {
                return blocks::move_to(settings, layout, members, blocks::Destination::Drawer(id, *rank), provisional);
            }
            Edit::RemoveBlock(commands) => {
                if settings.locked {
                    return false;
                }
                let mut changed = false;
                for command in commands {
                    self.pending = Some(Edit::Remove(command.clone()));
                    changed |= self.commit_impl(layout, settings, provisional);
                }
                return changed;
            }
            _ => {}
        }
        match edit {
            Edit::Split(bar, command, before) => {
                if settings.locked {
                    return false;
                }
                let (drawer, mut buttons) = drawers::contents(settings, bar, &command, layout);
                if !edit_mode::split_allowed(&buttons, &command, before) {
                    return false;
                }
                let index = buttons.iter().position(|b| b.command == command).unwrap_or(0) + usize::from(!before);
                buttons.insert(index, structure::separator(&uuid::Uuid::new_v4().to_string()));
                drawers::replace(settings, bar, drawer, buttons);
                return true;
            }
            Edit::Merge(bar, command, before) => {
                if settings.locked {
                    return false;
                }
                let (drawer, mut buttons) = drawers::contents(settings, bar, &command, layout);
                let Some(range) = edit_mode::merge_range(&buttons, &command, before) else {
                    return false;
                };
                for button in buttons.drain(range) {
                    settings.separators.remove(&button.command);
                }
                drawers::replace(settings, bar, drawer, buttons);
                return true;
            }
            Edit::Fold(bar, command) => {
                if settings.locked || settings.drawer_of(&command).is_some() {
                    return false;
                }
                let mut buttons = settings.bar(bar, layout).to_vec();
                let Some(range) = edit_mode::group(&buttons, &command) else {
                    return false;
                };
                if buttons[range.clone()].iter().any(|b| drawers::is_drawer(&b.command)) {
                    return false;
                }
                let id = format!("{}{}", drawers::PREFIX, uuid::Uuid::new_v4());
                let block_style = settings.block_styles.get(&command).copied().unwrap_or_default();
                let mode = block_style.mode.or(settings.style(bar).mode).unwrap_or_default();
                let profile = drawers::take_profile(settings, &buttons[range.clone()]).unwrap_or_default();
                let mut toggle = crate::ui::bars::top::button(&id, "folder-filled");
                if !profile.name.is_empty() {
                    toggle.tooltip = profile.name.clone();
                }
                if profile.look != Default::default() {
                    settings.appearance.insert(id.clone(), profile.look.clone());
                }
                let contents: Vec<ToolButton> = buttons.splice(range, [toggle]).collect();
                shortcuts::into_drawer(settings, &contents, &id);
                settings.drawers.insert(
                    id.clone(),
                    drawers::Drawer {
                        buttons: contents,
                        mode,
                        closes: profile.closes.unwrap_or(true),
                        hides_button: profile.hides_button,
                        close_button: profile.close_button,
                        vertical: profile.vertical.unwrap_or(false),
                        align: block_style.align,
                        sizing: block_style.sizing,
                        ..Default::default()
                    },
                );
                settings.block_styles.insert(id.clone(), block_style);
                settings.set_bar(bar, buttons);
                self.selected = Some(id);
                return true;
            }
            Edit::Unfold(bar, command) => {
                if settings.locked || drawers::is_floating(settings, &command) {
                    return false;
                }
                let mut buttons = settings.bar(bar, layout).to_vec();
                let Some(at) = buttons.iter().position(|b| b.command == command) else {
                    return false;
                };
                let Some(drawer) = settings.drawers.remove(&command) else {
                    return false;
                };
                shortcuts::out_of_drawer(settings, &command, &drawer.buttons);
                let profile = drawers::drawer_profile(settings, &command, &drawer, &buttons[at].tooltip);
                drawers::put_profile(settings, &drawer.buttons, profile);
                let mut block_style = settings.block_styles.get(&command).copied().unwrap_or_default();
                block_style.mode = Some(drawer.mode);
                block_style.align = drawer.align.or(block_style.align);
                block_style.sizing = drawer.sizing.or(block_style.sizing);
                for child in &drawer.buttons {
                    settings.block_styles.insert(child.command.clone(), block_style);
                }
                let mut contents = drawer.buttons;
                if !contents.is_empty() {
                    if at > 0 && !structure::is_separator(&buttons[at - 1].command) {
                        contents.insert(0, structure::separator(&uuid::Uuid::new_v4().to_string()));
                    }
                    if buttons.get(at + 1).is_some_and(|button| !structure::is_separator(&button.command)) {
                        contents.push(structure::separator(&uuid::Uuid::new_v4().to_string()));
                    }
                }
                buttons.splice(at..at + 1, contents);
                settings.set_bar(bar, buttons);
                self.selected = None;
                return true;
            }
            Edit::Separator(command, style) => {
                if settings.locked || !command.starts_with(structure::PREFIX) || !settings.placed(&command, layout) {
                    return false;
                }
                let style = style.sanitized();
                if settings.separators.get(&command).copied().unwrap_or_default() == style {
                    return false;
                }
                if style == Default::default() {
                    settings.separators.remove(&command);
                } else {
                    settings.separators.insert(command, style);
                }
                return true;
            }
            Edit::Lock => {
                settings.locked = !settings.locked;
                return true;
            }
            Edit::Appearance(command, appearance) => {
                if settings.locked {
                    return false;
                }
                let old = settings.appearance.get(&command).cloned().unwrap_or_default();
                if old == appearance {
                    return false;
                }
                if appearance == Default::default() {
                    settings.appearance.remove(&command);
                } else {
                    settings.appearance.insert(command, appearance);
                }
                return true;
            }
            Edit::MenuInsert(parent, command, rank) => {
                if settings.locked {
                    return false;
                }
                let catalogue = commands::current();
                if !catalogue.can_place(&command, &parent, &settings.menu_layout) {
                    return false;
                }
                let old = settings.menu_layout.clone();
                let children: Vec<_> = catalogue
                    .children(&parent, &settings.menu_layout)
                    .iter()
                    .map(|entry| entry.button.command.clone())
                    .collect();
                let before = children.iter().take(rank).filter(|id| **id == command).count();
                let mut children: Vec<_> = children.into_iter().filter(|id| *id != command).collect();
                children.insert(rank.saturating_sub(before).min(children.len()), command.clone());
                for items in settings.menu_layout.values_mut() {
                    items.retain(|id| *id != command);
                }
                settings.menu_layout.insert(parent, children);
                return settings.menu_layout != old;
            }
            // Firestorm keeps its alignment and modes menus while locked
            Edit::Style(bar, next) => {
                let style = settings.style_mut(bar);
                let old = *style;
                style.mode = next.mode.or(style.mode);
                style.align = next.align.or(style.align);
                style.sizing = next.sizing.or(style.sizing);
                return *style != old;
            }
            Edit::BlockStyle(bar, command, next) => {
                if settings.locked || settings.drawer_of(&command).is_some() {
                    return false;
                }
                let buttons = settings.bar(bar, layout);
                let Some(range) = edit_mode::group(buttons, &command) else {
                    return false;
                };
                let members: Vec<_> = buttons[range].iter().map(|button| button.command.clone()).collect();
                let old = settings.block_styles.get(&command).copied().unwrap_or_default();
                let style = BarStyle {
                    mode: next.mode.or(old.mode),
                    sizing: next.sizing.or(old.sizing),
                    align: next.align.or(old.align),
                };
                if style == old {
                    return false;
                }
                for member in members {
                    settings.block_styles.insert(member, style);
                }
                return true;
            }
            _ if settings.locked => return false,
            Edit::Restore => {
                // LLToolBarView::loadDefaultToolbars: the buttons and the settings
                let changed = settings.buttons.is_some()
                    || !settings.left.is_empty()
                    || !settings.right.is_empty()
                    || settings.top != ToolbarSettings::default().top
                    || !settings.appearance.is_empty()
                    || !settings.block_styles.is_empty()
                    || !settings.separators.is_empty()
                    || !settings.drawers.is_empty()
                    || !settings.menu_layout.is_empty()
                    || settings.address_visible.is_some()
                    || Bar::ALL.iter().any(|&b| settings.style(b) != BarStyle::default());
                let mut restored = ToolbarSettings {
                    locked: settings.locked,
                    ..Default::default()
                };
                restored.keep_interface_choices(Some(settings));
                *settings = restored;
                return changed;
            }
            _ => {}
        }
        let old: [Vec<ToolButton>; 4] = Bar::ALL.map(|bar| settings.bar(bar, layout).to_vec());
        let mut next = old.clone();
        let old_drawers = settings.drawers.clone();
        let mut next_drawers = old_drawers.clone();
        let separate = matches!(&edit, Edit::InsertGroup(..));
        match edit {
            Edit::DrawerInsert(id, button, rank) => {
                let floating = drawers::is_floating(settings, &id);
                if !next_drawers.contains_key(&id)
                    || (settings.location(&id, layout).is_none() && !floating)
                    || drawers::is_drawer(&button.command)
                    || button.command == drawers::FLOATING
                    || (floating && structure::is_block(&button.command))
                {
                    return false;
                }
                let button = structure::instantiate(button);
                let before = next_drawers[&id]
                    .buttons
                    .iter()
                    .take(rank)
                    .filter(|b| b.command == button.command)
                    .count();
                for list in &mut next {
                    list.retain(|b| b.command != button.command);
                }
                for drawer in next_drawers.values_mut() {
                    drawer.buttons.retain(|b| b.command != button.command);
                }
                if let Some(drawer) = next_drawers.get_mut(&id) {
                    let at = rank.saturating_sub(before).min(drawer.buttons.len());
                    drawer.buttons.insert(at, button);
                }
            }
            Edit::Insert(bar, button, rank) | Edit::InsertGroup(bar, button, rank) => {
                let button = if provisional && button.command == structure::DIVIDER {
                    structure::divider("drop-preview-entry")
                } else if provisional && button.command == structure::SEPARATOR {
                    structure::separator("drop-preview-entry")
                } else {
                    structure::instantiate(button)
                };
                let source = Bar::ALL.into_iter().find_map(|edge| {
                    let list = &next[edge.index()];
                    edit_mode::group(list, &button.command)
                        .filter(|range| {
                            !structure::is_divider(&button.command)
                                && list[range.clone()]
                                    .iter()
                                    .filter(|button| !structure::is_divider(&button.command))
                                    .count()
                                    == 1
                        })
                        .map(|range| (edge, range))
                });
                // Moving an existing singleton beside itself does not create a
                // group. Compare content order before touching its gap instances.
                if let Some((edge, _)) = &source
                    && *edge == bar
                {
                    let list = &next[bar.index()];
                    let mut reordered = list.clone();
                    let before = list.iter().take(rank).filter(|b| b.command == button.command).count();
                    reordered.retain(|b| b.command != button.command);
                    reordered.insert(rank.saturating_sub(before).min(reordered.len()), button.clone());
                    let commands = |items: &[ToolButton]| {
                        items
                            .iter()
                            .filter(|b| !structure::is_separator(&b.command) && !structure::is_divider(&b.command))
                            .map(|b| b.command.clone())
                            .collect::<Vec<_>>()
                    };
                    let previous = commands(list);
                    if (separate || previous.len() == 1) && commands(&reordered) == previous {
                        return false;
                    }
                }
                // A whole singleton group carries one boundary run with it.
                // Reuse its IDs/styles at the destination instead of leaving an
                // empty group behind and allocating another separator every drag.
                let mut carried = source
                    .filter(|_| separate)
                    .and_then(|(edge, group)| {
                        let list = &next[edge.index()];
                        let mut end = group.end;
                        while end < list.len() && structure::is_separator(&list[end].command) {
                            end += 1;
                        }
                        let range = if end < list.len() && end > group.end {
                            group.end..end
                        } else {
                            let mut start = group.start;
                            while start > 0 && structure::is_separator(&list[start - 1].command) {
                                start -= 1;
                            }
                            if start > 0 { start..group.start } else { group.start..group.start }
                        };
                        (!range.is_empty()).then(|| list[range].to_vec())
                    })
                    .unwrap_or_default();
                let removed: std::collections::HashSet<_> = carried.iter().map(|b| b.command.clone()).collect();
                let list = &next[bar.index()];
                let before = list
                    .iter()
                    .take(rank)
                    .filter(|b| b.command == button.command || removed.contains(&b.command))
                    .count();
                for list in &mut next {
                    list.retain(|b| b.command != button.command && !removed.contains(&b.command));
                }
                for drawer in next_drawers.values_mut() {
                    drawer.buttons.retain(|b| b.command != button.command);
                }
                let list = &mut next[bar.index()];
                let mut rank = rank.saturating_sub(before).min(list.len());
                if separate && !structure::is_separator(&button.command) {
                    // An exposed leading/trailing gap is already a boundary for
                    // the new group; do not leave it empty and create another.
                    if carried.is_empty() {
                        let before = list[..rank].iter().any(|b| !structure::is_separator(&b.command));
                        let after = list[rank..].iter().any(|b| !structure::is_separator(&b.command));
                        if before && !after {
                            while rank < list.len() && structure::is_separator(&list[rank].command) {
                                rank += 1;
                            }
                        } else if after && !before {
                            while rank > 0 && structure::is_separator(&list[rank - 1].command) {
                                rank -= 1;
                            }
                        }
                    }
                    let before = rank > 0 && !structure::is_separator(&list[rank - 1].command);
                    let after = list.get(rank).is_some_and(|b| !structure::is_separator(&b.command));
                    let mut entries = Vec::new();
                    for side in [true, false] {
                        if !side {
                            entries.push(button.clone());
                        }
                        if if side { before } else { after } {
                            if carried.is_empty() {
                                entries.push(structure::separator(&if provisional {
                                    format!("drop-preview-{}", if side { "before" } else { "after" })
                                } else {
                                    uuid::Uuid::new_v4().to_string()
                                }));
                            } else {
                                entries.append(&mut carried);
                            }
                        }
                    }
                    list.splice(rank..rank, entries);
                } else {
                    list.insert(rank, button);
                }
                for gap in carried {
                    settings.separators.remove(&gap.command);
                }
            }
            Edit::Remove(command) => {
                for list in &mut next {
                    list.retain(|b| b.command != command);
                }
                for drawer in next_drawers.values_mut() {
                    drawer.buttons.retain(|b| b.command != command);
                }
                // a floating block outlives its button
                if next_drawers.get(&command).is_some_and(|drawer| drawer.floating.is_none()) {
                    next_drawers.remove(&command);
                }
            }
            Edit::Clear => {
                next.iter_mut().for_each(Vec::clear);
                next_drawers.clear();
            }
            Edit::Restore
            | Edit::Lock
            | Edit::Style(..)
            | Edit::BlockStyle(..)
            | Edit::Appearance(..)
            | Edit::Separator(..)
            | Edit::MenuInsert(..) => {
                unreachable!()
            }
            Edit::Split(..) | Edit::Merge(..) | Edit::Fold(..) | Edit::Unfold(..) => {
                unreachable!()
            }
            Edit::MoveBlock(..)
            | Edit::DrawerBlock(..)
            | Edit::RemoveBlock(..)
            | Edit::DrawerFromDrop(..)
            | Edit::BlockOptions(..)
            | Edit::FloatingNew(..)
            | Edit::FloatingShown(..)
            | Edit::InterfaceShown(..)
            | Edit::Float(..)
            | Edit::Dock(..)
            | Edit::Shortcut(..) => unreachable!(),
        }
        if next == old && next_drawers == old_drawers {
            return false;
        }
        let navigation = |bars: &[Vec<ToolButton>; 4], drawers: &std::collections::BTreeMap<String, drawers::Drawer>| {
            bars.iter().flatten().any(|button| {
                button.command == "bar_navigation"
                    || drawers
                        .get(&button.command)
                        .is_some_and(|drawer| drawer.buttons.iter().any(|b| b.command == "bar_navigation"))
            })
        };
        if navigation(&next, &next_drawers) != navigation(&old, &old_drawers) {
            settings.address_visible = Some(navigation(&next, &next_drawers));
        }
        settings.drawers = next_drawers;
        let [bottom, left, right, top] = next;
        // an untouched bottom toolbar keeps following the skin
        if bottom != old[0] {
            settings.buttons = Some(bottom);
        }
        settings.left = left;
        settings.right = right;
        settings.top = top;
        true
    }
}

fn align_rows(bar: Bar) -> [(Align, &'static str, &'static str); 3] {
    if bar.vertical() {
        [
            (Align::Start, "layout-align-top-filled", aurora_i18n::tr!("toolbar-align-top")),
            (
                Align::Center,
                "layout-align-middle-filled",
                aurora_i18n::tr!("toolbar-align-middle"),
            ),
            (Align::End, "layout-align-bottom-filled", aurora_i18n::tr!("toolbar-align-bottom")),
        ]
    } else {
        [
            (Align::Start, "layout-align-left-filled", aurora_i18n::tr!("toolbar-align-left")),
            (
                Align::Center,
                "layout-align-center-filled",
                aurora_i18n::tr!("toolbar-align-center"),
            ),
            (Align::End, "layout-align-right-filled", aurora_i18n::tr!("toolbar-align-right")),
        ]
    }
}

fn mode_rows() -> [(ButtonMode, &'static str, &'static str); 3] {
    [
        (
            ButtonMode::IconsWithText,
            "list-details-filled",
            aurora_i18n::tr!("toolbar-mode-icons-text"),
        ),
        (ButtonMode::IconsOnly, "layout-grid-filled", aurora_i18n::tr!("toolbar-mode-icons")),
        (ButtonMode::TextOnly, "typography", aurora_i18n::tr!("toolbar-mode-text")),
    ]
}

fn sizing_rows() -> [(Sizing, &'static str, &'static str); 3] {
    [
        (Sizing::Auto, "arrow-autofit-width-filled", aurora_i18n::tr!("toolbar-size-auto")),
        (Sizing::Fixed, "layout-columns", aurora_i18n::tr!("toolbar-size-fixed")),
        (Sizing::Fill, "arrows-horizontal", aurora_i18n::tr!("toolbar-size-fill")),
    ]
}

pub use editor::{demo_events, demo_settings};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod commands_tests;
