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
mod catalogue;
pub(crate) mod catalogue_view;
pub(crate) mod commands;
mod editor;
pub(crate) mod structure;

pub use catalogue::COMMANDS;

use super::{
    menu,
    skin::{Layout, ToolButton},
};
use crate::theme::Palette;
use egui::{Rect, Response};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarSettings {
    /// The bottom toolbar. None follows the skin; Some([]) deliberately
    /// keeps an empty toolbar.
    pub buttons: Option<Vec<ToolButton>>,
    pub locked: bool,
    /// One-time discovery of the new command; later manual removal is retained.
    pub vr_button_migrated: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<ToolButton>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub right: Vec<ToolButton>,
    pub bottom_style: BarStyle,
    pub left_style: BarStyle,
    pub right_style: BarStyle,
    pub top: Vec<ToolButton>,
    pub top_style: BarStyle,
    pub appearance: std::collections::BTreeMap<String, appearance::Appearance>,
    /// Explicit menu placements; commands absent here follow their native menu.
    pub menu_layout: std::collections::BTreeMap<String, Vec<String>>,
    /// None preserves the existing visible address bar.
    pub address_visible: Option<bool>,
    /// Older layouts acquire the native top controls once, keeping their buttons.
    pub composed: bool,
}

impl Default for ToolbarSettings {
    fn default() -> Self {
        Self {
            buttons: None,
            locked: false,
            vr_button_migrated: false,
            left: Vec::new(),
            right: Vec::new(),
            bottom_style: Default::default(),
            left_style: Default::default(),
            right_style: Default::default(),
            top: vec![ToolButton {
                command: "location_bar".into(),
                icon: "map-pin-filled".into(),
                tooltip: String::new(),
            }],
            top_style: Default::default(),
            appearance: Default::default(),
            menu_layout: Default::default(),
            address_visible: None,
            composed: false,
        }
    }
}

impl ToolbarSettings {
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
        {
            if !self.top.is_empty() {
                self.top.push(structure::separator(&uuid::Uuid::new_v4().to_string()));
            }
            self.top.push(crate::ui::bars::top::button("bar_navigation", "map-pin-filled"));
        }
    }
    pub fn compose_top(&mut self) -> bool {
        if self.composed {
            return false;
        }
        self.top = structure::top_buttons(&self.top);
        self.composed = true;
        self.sanitize();
        true
    }

    pub fn set_bug_visible(&mut self, layout: &Layout, shown: bool) -> bool {
        if self.location("report_bug", layout).is_some() == shown {
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
        Bar::ALL
            .into_iter()
            .find(|&bar| self.bar(bar, layout).iter().any(|b| b.command == command))
    }

    /// No empty command, and each command once over the four toolbars
    /// (the bottom one first).
    pub fn sanitize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        if let Some(buttons) = &mut self.buttons {
            for button in buttons.iter_mut() {
                if button.command == structure::SEPARATOR {
                    *button = structure::instantiate(button.clone());
                }
            }
            buttons.retain(|b| !b.command.is_empty() && seen.insert(b.command.clone()));
        }
        for side in [&mut self.left, &mut self.right, &mut self.top] {
            for button in side.iter_mut() {
                if button.command == structure::SEPARATOR {
                    *button = structure::instantiate(button.clone());
                }
            }
            side.retain(|b| !b.command.is_empty() && seen.insert(b.command.clone()));
        }
    }
}

#[derive(Clone)]
struct Drag {
    button: ToolButton,
    /// The toolbar it was taken from; None from the window's catalogue.
    from: Option<Bar>,
}

enum Edit {
    Insert(Bar, ToolButton, usize),
    SeparatorAfter(Bar, Option<String>),
    Remove(String),
    Restore,
    Clear,
    Lock,
    Style(Bar, BarStyle),
    Appearance(String, appearance::Appearance),
    MenuInsert(String, String, usize),
}

/// Windows whose toolbar button lights while they are open
/// (is_running_function in commands.xml), given by the app each frame.
#[derive(Clone, Copy, Default)]
pub struct Shown {
    /// Avatar.IsMyProfileOpen.
    pub profile: bool,
    /// Floater.IsOpen build.
    pub build: bool,
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
    pub photo_roll_active: bool,
    /// The nearby chat bar is shown (MainChatbarVisible).
    pub chat_bar_active: bool,
    /// The windows the app opens itself, for their buttons' highlight.
    pub shown: Shown,
    /// "Show Friends Only" is on (FSRenderFriendsOnly).
    pub friends_only_active: bool,
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
    /// The look the bars are drawn in (given by the app): the islands' dock
    /// or Firestorm's bars.
    pub islands: bool,
    pending: Option<Edit>,
    /// true restores defaults, false clears all (Firestorm's confirmations).
    confirm: Option<bool>,
    /// The window's filter.
    search: String,
    category: Option<catalogue_view::Category>,
    appearance_editor: appearance::Editor,
}

/// Built-in labels stay live when the language changes, including saved
/// buttons; a skin may rename a command, and keeps its name.
pub fn label(button: &ToolButton) -> &str {
    if structure::is_separator(&button.command) {
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
    buttons.retain(|b| seen.insert(b.command.clone()));
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
                    .any(|button| button.command.starts_with(commands::PREFIX))
            })
    }

    /// The right-click menu of a toolbar or of one of its buttons:
    /// Firestorm's (menu_toolbars_horizontal.xml, _vertical.xml), with
    /// Aurora's microphone options and favourites access.
    pub fn context(&mut self, response: &Response, p: &Palette, bar: Bar, style: Style, button: Option<&ToolButton>, locked: bool) {
        // Buttons such as Favorites also own a left-click dropdown.
        menu::context_menu_of_button(response, p, |ui| {
            if let Some(button) = button
                && !structure::is_separator(&button.command)
                && !structure::is_block(&button.command)
                && !locked
                && menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("toolbar-button-appearance"))
            {
                self.appearance_editor.select(button);
            }
            if !locked && let Some(button) = button {
                if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("toolbar-remove")) {
                    self.pending = Some(Edit::Remove(button.command.clone()));
                }
                menu::separator(ui, p);
            }
            if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("toolbar-edit")) {
                self.open = true;
            }
            if !locked && menu::item(ui, p, "separator-vertical", aurora_i18n::tr!("toolbar-add-separator")) {
                self.open = true;
                self.pending = Some(Edit::SeparatorAfter(bar, button.map(|button| button.command.clone())));
            }
            if button.is_some_and(|b| b.command == "speak")
                && menu::item(ui, p, "microphone-filled", aurora_i18n::tr!("ui-bars-microphone-options"))
            {
                self.voice_options = true;
            }
            if button.is_some_and(|b| matches!(b.command.as_str(), "movelock" | "move_lock"))
                && menu::item(ui, p, "lock-filled", aurora_i18n::tr!("movelock-options"))
            {
                self.movelock_options = true;
            }
            // the Conversations button: open the window, leave nearby chat
            // out of its flash
            if button.is_some_and(|b| b.command == "conversations") {
                menu::separator(ui, p);
                if let Some(choice) = crate::ui::chat::button_menu(ui, p, self.flash_nearby) {
                    self.conversations_menu = Some(choice);
                }
            }
            // the Favorites button: where the favourites live
            if button.is_some_and(|b| b.command == "favorites")
                && let Some(a) = crate::ui::bars::favorites::access_choice(ui, p, self.favorites_access)
            {
                self.access_choice = Some(a);
            }
            menu::separator(ui, p);
            let mut next = None;
            for (align, icon, label) in align_rows(bar) {
                if menu::check(ui, p, icon, label, style.align == align, true) {
                    next = Some(BarStyle {
                        align: Some(align),
                        ..Default::default()
                    });
                }
            }
            menu::separator(ui, p);
            for (mode, icon, label) in mode_rows() {
                if menu::check(ui, p, icon, label, style.mode == mode, true) {
                    next = Some(BarStyle {
                        mode: Some(mode),
                        ..Default::default()
                    });
                }
            }
            if !bar.vertical() {
                menu::separator(ui, p);
                for (sizing, icon, label) in sizing_rows() {
                    if menu::check(ui, p, icon, label, style.sizing == sizing, true) {
                        next = Some(BarStyle {
                            sizing: Some(sizing),
                            ..Default::default()
                        });
                    }
                }
            }
            if let Some(next) = next {
                self.pending = Some(Edit::Style(bar, next));
            }
            menu::separator(ui, p);
            if menu::check(ui, p, "lock-filled", aurora_i18n::tr!("toolbar-locked"), locked, true) {
                self.pending = Some(Edit::Lock);
            }
        });
    }

    pub fn button(&mut self, response: &Response, bar: Bar, style: Style, button: &ToolButton, p: &Palette, locked: bool) {
        self.context(response, p, bar, style, Some(button), locked);
        self.bar_source(response, button, bar, locked);
    }

    pub fn bar_source(&self, response: &Response, button: &ToolButton, bar: Bar, locked: bool) {
        if !locked && response.drag_started_by(egui::PointerButton::Primary) {
            response.dnd_set_drag_payload(Drag {
                button: button.clone(),
                from: Some(bar),
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
    pub fn drop_bar(&mut self, response: &Response, bar: Bar, rects: &[Rect], p: &Palette, locked: bool) {
        if locked || response.dnd_hover_payload::<Drag>().is_none() {
            return;
        }
        let Some(pos) = response.ctx.pointer_interact_pos() else { return };
        let vertical = bar.vertical();
        let rank = insertion(rects, pos, vertical);
        let r = response.rect;
        let stroke = egui::Stroke::new(2.0, p.violet_light);
        let painter = response.ctx.layer_painter(response.layer_id);
        if vertical {
            let y = rects
                .get(rank)
                .map_or_else(|| rects.last().map_or(r.top() + 3.0, |b| b.bottom() + 1.0), |b| b.top() - 1.0)
                .clamp(r.top() + 1.0, r.bottom() - 1.0);
            painter.line_segment([egui::pos2(r.left() + 2.0, y), egui::pos2(r.right() - 2.0, y)], stroke);
        } else {
            let x = rects
                .get(rank)
                .map_or_else(|| rects.last().map_or(r.left() + 3.0, |b| b.right() + 2.0), |b| b.left() - 2.0)
                .clamp(r.left() + 1.0, r.right() - 1.0);
            painter.line_segment([egui::pos2(x, r.top() + 2.0), egui::pos2(x, r.bottom() - 2.0)], stroke);
        }
        if let Some(drag) = response.dnd_release_payload::<Drag>() {
            self.pending = Some(Edit::Insert(bar, drag.button.clone(), rank));
        }
    }

    /// Save once for a real edit, never on hover, during dragging or on a no-op.
    pub fn commit(&mut self, layout: &Layout, settings: &mut ToolbarSettings) -> bool {
        let Some(edit) = self.pending.take() else { return false };
        if let Edit::SeparatorAfter(bar, anchor) = edit {
            let list = settings.bar(bar, layout);
            let rank = anchor
                .as_ref()
                .and_then(|anchor| list.iter().position(|button| button.command == *anchor))
                .map_or(list.len(), |index| index + 1);
            self.pending = Some(Edit::Insert(bar, structure::separator(&uuid::Uuid::new_v4().to_string()), rank));
            return self.commit(layout, settings);
        }
        match edit {
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
            _ if settings.locked => return false,
            Edit::Restore => {
                // LLToolBarView::loadDefaultToolbars: the buttons and the settings
                let changed = settings.buttons.is_some()
                    || !settings.left.is_empty()
                    || !settings.right.is_empty()
                    || settings.top != ToolbarSettings::default().top
                    || !settings.appearance.is_empty()
                    || !settings.menu_layout.is_empty()
                    || settings.address_visible.is_some()
                    || Bar::ALL.iter().any(|&b| settings.style(b) != BarStyle::default());
                *settings = ToolbarSettings {
                    locked: settings.locked,
                    ..Default::default()
                };
                return changed;
            }
            _ => {}
        }
        let old: [Vec<ToolButton>; 4] = Bar::ALL.map(|bar| settings.bar(bar, layout).to_vec());
        let mut next = old.clone();
        match edit {
            Edit::Insert(bar, button, rank) => {
                let button = structure::instantiate(button);
                let list = &next[bar.index()];
                let before = list.iter().take(rank).filter(|b| b.command == button.command).count();
                for list in &mut next {
                    list.retain(|b| b.command != button.command);
                }
                let list = &mut next[bar.index()];
                let rank = rank.saturating_sub(before).min(list.len());
                list.insert(rank, button);
            }
            Edit::Remove(command) => {
                for list in &mut next {
                    list.retain(|b| b.command != command);
                }
            }
            Edit::Clear => next.iter_mut().for_each(Vec::clear),
            Edit::Restore | Edit::Lock | Edit::Style(..) | Edit::Appearance(..) | Edit::MenuInsert(..) | Edit::SeparatorAfter(..) => {
                unreachable!()
            }
        }
        if next == old {
            return false;
        }
        let navigation = |bars: &[Vec<ToolButton>; 4]| bars.iter().flatten().any(|button| button.command == "bar_navigation");
        if navigation(&next) != navigation(&old) {
            settings.address_visible = Some(navigation(&next));
        }
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
