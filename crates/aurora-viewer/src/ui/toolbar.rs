//! Toolbar customization after LLToolBar::handleRightMouseDown,
//! LLToolBarView::handleDropTool and LLFloaterToybox (indra/llui/lltoolbar.cpp,
//! indra/newview/{lltoolbarview,llfloatertoybox}.cpp, originally LGPL 2.1).
//! Three toolbars, as Firestorm's: the bottom one (Firestorm's bar or the
//! dock of the islands), the left and the right ones; a command stands in
//! one of them at most. Each has Firestorm's button mode, alignment and
//! layout style (menu_toolbars_horizontal.xml / _vertical.xml).
//! Moves commit on release; dropping back in the window removes a button.
//! At the human's request, the layout and lock are shared by all accounts,
//! unlike Firestorm's per-account toolbar persistence.

mod catalogue;
mod editor;

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
}

impl Bar {
    pub const ALL: [Bar; 3] = [Bar::Bottom, Bar::Left, Bar::Right];

    pub fn vertical(self) -> bool {
        self != Bar::Bottom
    }

    fn index(self) -> usize {
        match self {
            Bar::Bottom => 0,
            Bar::Left => 1,
            Bar::Right => 2,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

impl ToolbarSettings {
    pub fn migrate_vr_button(&mut self) -> bool {
        if self.vr_button_migrated {
            return false;
        }
        self.vr_button_migrated = true;
        #[cfg(target_os = "linux")]
        if !self.left.iter().chain(&self.right).any(|b| b.command == "vr_view")
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
        }
    }

    pub fn style(&self, bar: Bar) -> BarStyle {
        match bar {
            Bar::Bottom => self.bottom_style,
            Bar::Left => self.left_style,
            Bar::Right => self.right_style,
        }
    }

    fn style_mut(&mut self, bar: Bar) -> &mut BarStyle {
        match bar {
            Bar::Bottom => &mut self.bottom_style,
            Bar::Left => &mut self.left_style,
            Bar::Right => &mut self.right_style,
        }
    }

    /// The toolbar `command` stands in (LLToolBarView::hasCommand).
    pub fn location(&self, command: &str, layout: &Layout) -> Option<Bar> {
        Bar::ALL
            .into_iter()
            .find(|&bar| self.bar(bar, layout).iter().any(|b| b.command == command))
    }

    /// No empty command, and each command once over the three toolbars
    /// (the bottom one first).
    pub fn sanitize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        if let Some(buttons) = &mut self.buttons {
            buttons.retain(|b| !b.command.is_empty() && seen.insert(b.command.clone()));
        }
        for side in [&mut self.left, &mut self.right] {
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
    Remove(String),
    Restore,
    Clear,
    Lock,
    Style(Bar, BarStyle),
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
}

/// Built-in labels stay live when the language changes, including saved
/// buttons; a skin may rename a command, and keeps its name.
pub fn label(button: &ToolButton) -> &str {
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
    /// The right-click menu of a toolbar or of one of its buttons:
    /// Firestorm's (menu_toolbars_horizontal.xml, _vertical.xml), with
    /// Aurora's microphone options and favourites access.
    pub fn context(&mut self, response: &Response, p: &Palette, bar: Bar, style: Style, button: Option<&ToolButton>, locked: bool) {
        menu::context_menu(response, p, |ui| {
            if !locked && let Some(button) = button {
                if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("toolbar-remove")) {
                    self.pending = Some(Edit::Remove(button.command.clone()));
                }
                menu::separator(ui, p);
            }
            if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("toolbar-edit")) {
                self.open = true;
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
        match edit {
            Edit::Lock => {
                settings.locked = !settings.locked;
                return true;
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
                    || Bar::ALL.iter().any(|&b| settings.style(b) != BarStyle::default());
                *settings = ToolbarSettings {
                    locked: settings.locked,
                    ..Default::default()
                };
                return changed;
            }
            _ => {}
        }
        let old: [Vec<ToolButton>; 3] = Bar::ALL.map(|bar| settings.bar(bar, layout).to_vec());
        let mut next = old.clone();
        match edit {
            Edit::Insert(bar, button, rank) => {
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
            Edit::Restore | Edit::Lock | Edit::Style(..) => unreachable!(),
        }
        if next == old {
            return false;
        }
        let [bottom, left, right] = next;
        // an untouched bottom toolbar keeps following the skin
        if bottom != old[0] {
            settings.buttons = Some(bottom);
        }
        settings.left = left;
        settings.right = right;
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
