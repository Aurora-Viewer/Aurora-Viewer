//! The "Toolbar buttons" window (LLFloaterToybox, floater_toybox.xml), in
//! the layout of the bars mockup: the commands in a grid of tiles with a
//! filter, each toolbar's mode, alignment and size, Locked, Clear all and
//! Restore defaults. A command already in a toolbar stays in the grid,
//! hollow, and cannot be added twice (LLFloaterToybox::draw); one Aurora
//! does not have yet is dimmed. A click on a free tile adds it at the end
//! of the bottom toolbar (Aurora's, for the keyboard).

use super::{Align, Bar, BarStyle, ButtonMode, Drag, Edit, Sizing, ToolbarSettings, ToolbarUi, catalogue, label, ready};
use crate::theme::Palette;
use crate::ui::{
    skin::{Layout, ToolButton},
    widgets,
};
use egui::{Rect, Vec2};

/// The window's id, for its layout.
pub(crate) const WINDOW: &str = "toolbar_editor";
/// The tiles: five a row, 4 px apart.
const COLUMNS: usize = 5;
const TILE_GAP: f32 = 4.0;
const TILE_H: f32 = 27.0;
const TILE_ICON: f32 = 15.0;
const TILE_PAD: f32 = 8.0;
const TILE_FONT: f32 = 11.5;
/// The window's width: the mockup's 820 px (760 on a screen under 1408 px),
/// never wider than the screen.
fn width(screen: Rect) -> f32 {
    let wanted: f32 = if screen.width() < 1408.0 { 760.0 } else { 820.0 };
    wanted.min(screen.width() - 32.0).max(320.0)
}

/// The rows of tiles the grid shows before it scrolls: the mockup's six (five
/// on a screen under 1408 px).
fn rows_shown(screen: Rect) -> usize {
    if screen.width() < 1408.0 { 5 } else { 6 }
}

/// Where it opens: centred (floater_toybox.xml `positioning="centered"`),
/// near the top of the windows' room, as in the mockup.
fn default_pos(ctx: &egui::Context) -> egui::Pos2 {
    let screen = ctx.content_rect();
    let area = crate::ui::layout::work_area(ctx);
    egui::pos2(screen.center().x - width(screen) / 2.0, area.top() + 22.0)
}

/// What the tooltip of a placed command says (Toolbar_*_Tooltip).
fn placed_tip(bar: Bar, islands: bool) -> &'static str {
    match bar {
        Bar::Bottom if islands => aurora_i18n::tr!("toolbar-in-dock"),
        Bar::Bottom => aurora_i18n::tr!("toolbar-in-bottom"),
        Bar::Left => aurora_i18n::tr!("toolbar-in-left"),
        Bar::Right => aurora_i18n::tr!("toolbar-in-right"),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tile {
    Free,
    Placed,
    /// Not in Aurora yet.
    Missing,
    /// Being dragged out of the window.
    Dragged,
}

/// One tile of the grid: its icon and label on the face the state gives.
fn tile(ui: &mut egui::Ui, p: &Palette, rect: Rect, button: &ToolButton, state: Tile) -> egui::Response {
    let sense = if state == Tile::Free {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::hover()
    };
    let response = ui.interact(rect, egui::Id::unique(("toolbar_tile", &button.command)), sense);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fade = if state == Tile::Dragged { 0.35 } else { 1.0 };
    let painter = ui.painter();
    match state {
        Tile::Free | Tile::Dragged => {
            let face = if response.hovered() && state == Tile::Free {
                p.raised.lerp_to_gamma(p.ink, 0.08)
            } else {
                p.raised
            };
            painter.rect_filled(rect, radius, face.gamma_multiply(fade));
        }
        // hollow: the mockup's 1 px inner outline
        Tile::Placed => {
            painter.rect_stroke(
                rect.shrink(0.5),
                radius,
                egui::Stroke::new(1.0, p.ink.gamma_multiply(0.06)),
                egui::StrokeKind::Inside,
            );
        }
        Tile::Missing => {}
    }
    let (ink, icon_tint) = match state {
        Tile::Free | Tile::Dragged => (p.ink.gamma_multiply(fade), p.muted.gamma_multiply(fade)),
        Tile::Placed => (p.muted_dim, p.muted_dim),
        Tile::Missing => (p.muted_dim.gamma_multiply(0.55), p.muted_dim.gamma_multiply(0.55)),
    };
    let icon = Rect::from_center_size(
        egui::pos2(rect.left() + TILE_PAD + TILE_ICON / 2.0, rect.center().y),
        Vec2::splat(TILE_ICON),
    );
    if button.icon == "AO" {
        let galley = painter.layout_no_wrap("AO".to_owned(), egui::FontId::proportional(10.0), icon_tint);
        widgets::paint_ink_centered(painter, galley, icon.center(), icon_tint);
    } else if let Some(texture) = crate::ui::icons::global(&button.icon) {
        painter.image(
            texture.id(),
            icon,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            icon_tint,
        );
    }
    let text_left = icon.right() + 7.0;
    widgets::paint_elided(
        ui,
        label(button),
        egui::pos2(text_left, rect.center().y),
        rect.right() - TILE_PAD / 2.0 - text_left,
        egui::FontId::proportional(TILE_FONT),
        ink,
    );
    response
}

/// One toolbar's row of settings: its name, then its modes, alignments and
/// (the bottom one) sizes, as segmented choices.
fn style_row(ui: &mut egui::Ui, p: &Palette, bar: Bar, islands: bool, saved: BarStyle) -> Option<BarStyle> {
    let style = saved.resolve(bar, islands);
    let name = match bar {
        Bar::Bottom if islands => aurora_i18n::tr!("toolbar-bar-dock"),
        Bar::Bottom => aurora_i18n::tr!("toolbar-bar-bottom"),
        Bar::Left => aurora_i18n::tr!("toolbar-bar-left"),
        Bar::Right => aurora_i18n::tr!("toolbar-bar-right"),
    };
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(108.0, 24.0), egui::Sense::hover());
        ui.painter().text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
            p.ink,
        );
        let modes = [ButtonMode::IconsOnly, ButtonMode::IconsWithText, ButtonMode::TextOnly];
        let mut i = modes.iter().position(|m| *m == style.mode).unwrap_or(0);
        let labels = [
            (aurora_i18n::tr!("toolbar-seg-icons"), true),
            (aurora_i18n::tr!("toolbar-seg-icons-text"), true),
            (aurora_i18n::tr!("toolbar-seg-text"), true),
        ];
        if widgets::segmented(ui, p, &mut i, &labels) {
            out = Some(BarStyle {
                mode: Some(modes[i]),
                ..Default::default()
            });
        }
        let aligns = [Align::Start, Align::Center, Align::End];
        let mut i = aligns.iter().position(|a| *a == style.align).unwrap_or(1);
        let labels = if bar.vertical() {
            [
                (aurora_i18n::tr!("toolbar-seg-top"), true),
                (aurora_i18n::tr!("toolbar-seg-middle"), true),
                (aurora_i18n::tr!("toolbar-seg-bottom"), true),
            ]
        } else {
            [
                (aurora_i18n::tr!("toolbar-seg-start"), true),
                (aurora_i18n::tr!("toolbar-seg-middle"), true),
                (aurora_i18n::tr!("toolbar-seg-end"), true),
            ]
        };
        if widgets::segmented(ui, p, &mut i, &labels) {
            out = Some(BarStyle {
                align: Some(aligns[i]),
                ..Default::default()
            });
        }
        if !bar.vertical() {
            let sizes = [Sizing::Auto, Sizing::Fixed, Sizing::Fill];
            let mut i = sizes.iter().position(|s| *s == style.sizing).unwrap_or(1);
            let labels = [
                (aurora_i18n::tr!("toolbar-seg-auto"), true),
                (aurora_i18n::tr!("toolbar-seg-fixed"), true),
                (aurora_i18n::tr!("toolbar-seg-fill"), true),
            ];
            if widgets::segmented(ui, p, &mut i, &labels) {
                out = Some(BarStyle {
                    sizing: Some(sizes[i]),
                    ..Default::default()
                });
            }
        }
    });
    out
}

impl ToolbarUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, layout: &Layout, settings: &ToolbarSettings) {
        let screen = ctx.content_rect();
        let w = width(screen);
        let islands = self.islands;
        let dragged = egui::DragAndDrop::payload::<Drag>(ctx);
        let was_open = self.open;
        widgets::Floater::new(WINDOW, aurora_i18n::tr!("toolbar-title"), default_pos(ctx), Vec2::new(w, 420.0))
            .help(aurora_i18n::tr!("toolbar-help-modes"))
            .auto_height()
            .show(ctx, p, &mut self.open, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.label(
                    egui::RichText::new(if islands {
                        aurora_i18n::tr!("toolbar-help")
                    } else {
                        aurora_i18n::tr!("toolbar-help-classic")
                    })
                    .size(12.0)
                    .color(p.muted),
                );
                let all = super::catalogue(layout);
                let hint = aurora_i18n::tr!("toolbar-search", count = all.len().to_string());
                // in a row: the field's own layout would take the window's height
                ui.horizontal(|ui| {
                    widgets::pills::search_box(ui, p, &mut self.search, &hint, ui.available_width());
                });
                let filter = super::sort_key(self.search.trim());
                let shown: Vec<&ToolButton> = all
                    .iter()
                    .filter(|b| filter.is_empty() || super::sort_key(label(b)).contains(&filter) || b.command.contains(&filter))
                    .collect();
                // the grid of tiles
                let full = ui.available_width();
                let tile_w = ((full - TILE_GAP * (COLUMNS - 1) as f32) / COLUMNS as f32).floor();
                let rows = shown.len().div_ceil(COLUMNS).max(1);
                let height = rows as f32 * TILE_H + (rows - 1) as f32 * TILE_GAP;
                let shown_rows = rows_shown(screen);
                let window = shown_rows as f32 * TILE_H + (shown_rows - 1) as f32 * TILE_GAP;
                egui::ScrollArea::vertical()
                    .id_salt("toolbar_tiles")
                    .max_height(window)
                    .min_scrolled_height(window)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let (grid, _) = ui.allocate_exact_size(Vec2::new(full, height), egui::Sense::hover());
                        for (i, button) in shown.iter().enumerate() {
                            let (row, col) = (i / COLUMNS, i % COLUMNS);
                            let rect = Rect::from_min_size(
                                grid.min + Vec2::new(col as f32 * (tile_w + TILE_GAP), row as f32 * (TILE_H + TILE_GAP)),
                                Vec2::new(tile_w, TILE_H),
                            );
                            let placed = settings.location(&button.command, layout);
                            let state = if !ready(&button.command) {
                                Tile::Missing
                            } else if placed.is_some() {
                                Tile::Placed
                            } else if dragged
                                .as_ref()
                                .is_some_and(|d| d.from.is_none() && d.button.command == button.command)
                            {
                                Tile::Dragged
                            } else {
                                Tile::Free
                            };
                            let state = if settings.locked && state == Tile::Free {
                                Tile::Placed
                            } else {
                                state
                            };
                            let response = tile(ui, p, rect, button, state);
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(egui::Id::unique(("toolbar_catalogue", &button.command)), rect));
                            match state {
                                Tile::Free => {
                                    if response.drag_started_by(egui::PointerButton::Primary) {
                                        response.dnd_set_drag_payload(Drag {
                                            button: (*button).clone(),
                                            from: None,
                                        });
                                    }
                                    if response.clicked() {
                                        let at = settings.buttons(layout).len();
                                        self.pending = Some(Edit::Insert(Bar::Bottom, (*button).clone(), at));
                                    }
                                }
                                Tile::Placed => {
                                    if let Some(bar) = placed {
                                        response.on_hover_text(placed_tip(bar, islands));
                                    }
                                }
                                Tile::Missing => {
                                    response.on_hover_text(crate::ui::menu::not_yet());
                                }
                                Tile::Dragged => {}
                            }
                        }
                    });
                // each toolbar's settings: the bottom one, and a side one
                // that holds buttons
                let mut card = crate::ui::sheet::card_begin(ui);
                card.child.spacing_mut().item_spacing.y = 8.0;
                for bar in Bar::ALL {
                    if (bar == Bar::Bottom || !settings.bar(bar, layout).is_empty())
                        && let Some(next) = style_row(&mut card.child, p, bar, islands, settings.style(bar))
                    {
                        self.pending = Some(Edit::Style(bar, next));
                    }
                }
                crate::ui::sheet::card_end(ui, p, card);
                ui.horizontal(|ui| {
                    let mut locked = settings.locked;
                    if widgets::checkbox(ui, p, &mut locked, aurora_i18n::tr!("toolbar-locked-bars")).changed() {
                        self.pending = Some(Edit::Lock);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!settings.locked, egui::Button::new(aurora_i18n::tr!("toolbar-restore")))
                            .clicked()
                        {
                            self.confirm = Some(true);
                        }
                        if ui
                            .add_enabled(!settings.locked, egui::Button::new(aurora_i18n::tr!("toolbar-clear")))
                            .clicked()
                        {
                            self.confirm = Some(false);
                        }
                    });
                });
            });
        // a toolbar button dropped on the window goes back to it
        // (LLFloaterToybox::handleDragAndDrop on its read-only toolbar)
        let window = ctx.memory(|m| m.area_rect(egui::Id::unique(WINDOW)));
        if let (true, Some(window), Some(drag)) = (was_open, window, dragged.as_ref())
            && drag.from.is_some()
            && !settings.locked
            && ctx.input(|i| i.pointer.any_released())
            && ctx.pointer_interact_pos().is_some_and(|pos| window.contains(pos))
        {
            self.pending = Some(Edit::Remove(drag.button.command.clone()));
            egui::DragAndDrop::clear_payload(ctx);
        }
        // ConfirmRestoreToolbarDefaults / ConfirmClearAllToolbars
        if let Some(restore) = self.confirm {
            let width = (screen.width() - 80.0).clamp(240.0, 400.0);
            let shown = widgets::Modal::new(egui::Id::unique("toolbar_confirm")).show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                ui.horizontal(|ui| {
                    widgets::glyph(ui, "layout-bottombar-filled", p.violet_light, 20.0);
                    ui.label(
                        egui::RichText::new(if restore {
                            aurora_i18n::tr!("toolbar-restore")
                        } else {
                            aurora_i18n::tr!("toolbar-clear")
                        })
                        .size(17.0)
                        .strong()
                        .color(p.ink),
                    );
                });
                ui.label(
                    egui::RichText::new(if restore {
                        aurora_i18n::tr!("toolbar-confirm-restore")
                    } else {
                        aurora_i18n::tr!("toolbar-confirm-clear")
                    })
                    .size(13.0)
                    .color(p.muted),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let size = Vec2::new((width - 8.0) / 2.0, 32.0);
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), widgets::ActionKind::Flat, size).clicked() {
                        self.confirm = None;
                    }
                    let (icon, kind) = if restore {
                        ("refresh", widgets::ActionKind::Primary)
                    } else {
                        ("trash-filled", widgets::ActionKind::Danger)
                    };
                    if widgets::action_button(ui, p, icon, aurora_i18n::tr!("common-ok"), kind, size).clicked() {
                        self.pending = Some(if restore { Edit::Restore } else { Edit::Clear });
                        self.confirm = None;
                    }
                });
            });
            if shown.should_close() {
                self.confirm = None;
            }
        }
        if let Some(drag) = dragged
            && let Some(pointer) = ctx.pointer_interact_pos()
        {
            ghost(ctx, p, &drag.button, pointer);
        }
    }
}

/// The button being dragged, under the pointer: the mockup's violet ghost.
fn ghost(ctx: &egui::Context, p: &Palette, button: &ToolButton, pointer: egui::Pos2) {
    egui::Area::new(egui::Id::unique("toolbar_drag_preview"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(pointer + Vec2::new(12.0, -30.0))
        .fade_in(false)
        .show(ctx, |ui| {
            let font = egui::FontId::new(12.0, crate::ui::fonts::semibold(ctx));
            let galley = ui.painter().layout_no_wrap(label(button).to_owned(), font, p.on_accent);
            let size = Vec2::new(10.0 + 16.0 + 6.0 + galley.size().x + 10.0, 28.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let radius = ui.visuals().window_corner_radius;
            let painter = ui.painter();
            painter.add(ui.visuals().popup_shadow.as_shape(rect, radius));
            painter.rect_filled(rect, radius, p.violet.gamma_multiply(0.85));
            let icon = Rect::from_center_size(egui::pos2(rect.left() + 10.0 + 8.0, rect.center().y), Vec2::splat(16.0));
            if button.icon == "AO" {
                let ao = painter.layout_no_wrap("AO".to_owned(), egui::FontId::proportional(11.0), p.on_accent);
                widgets::paint_ink_centered(painter, ao, icon.center(), p.on_accent);
            } else if let Some(texture) = crate::ui::icons::global(&button.icon) {
                painter.image(
                    texture.id(),
                    icon,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    p.on_accent,
                );
            }
            let at = crate::ui::widgets::row_text_origin(painter, &galley, icon.right() + 6.0, rect.center().y);
            painter.galley(at, galley, p.on_accent);
        });
}

/// The settings of a demo run (`AURORA_DEMO_TOOLBAR`): `custom` is the
/// mockup's "custom" state (Environments, Radar and Teleport history in the
/// left bar, Performance out of the dock); `side` the same bars without the
/// window (`sidemenu`: Radar's right click at frame 650); `right` them in
/// the right bar; `labels` adds the dock's icons and labels and the left
/// bar's labels. Never saved.
pub fn demo_settings(settings: &mut ToolbarSettings, layout: &Layout) {
    let Some(mode) = demo_mode() else { return };
    if !matches!(mode, "custom" | "side" | "sidemenu" | "labels" | "right") {
        return;
    }
    let side: Vec<ToolButton> = ["myenvironments", "radar", "teleport_history"]
        .iter()
        .filter_map(|id| catalogue::find(id))
        .map(|c| ToolButton {
            command: c.id.into(),
            icon: c.icon.into(),
            tooltip: String::new(),
        })
        .collect();
    let mut bottom = settings.buttons(layout).to_vec();
    bottom.retain(|b| b.command != "performance" && !side.iter().any(|s| s.command == b.command));
    settings.buttons = Some(bottom);
    settings.left.clear();
    settings.right.clear();
    if mode == "right" {
        settings.right = side;
    } else {
        settings.left = side;
    }
    settings.locked = false;
    if mode == "labels" {
        settings.bottom_style.mode = Some(ButtonMode::IconsWithText);
        settings.left_style.mode = Some(ButtonMode::TextOnly);
    }
}

fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_TOOLBAR").ok()).as_deref()
}

/// Offline scenarios use the real widgets and pointer events, as the human does.
pub fn demo_events(ctx: &egui::Context, frame: u64, state: &mut ToolbarUi) -> Vec<egui::Event> {
    let Some(mode) = demo_mode() else {
        return Vec::new();
    };
    if mode == "side" {
        return Vec::new();
    }
    // the side toolbar's right click, without the window
    let menu = matches!(mode, "menu" | "sidemenu");
    if frame == 300 && mode != "sidemenu" {
        let size = Vec2::new(width(ctx.content_rect()), 420.0);
        widgets::place_floater(ctx, WINDOW, default_pos(ctx), size);
        state.open = true;
    }
    if matches!(mode, "1" | "labels" | "right") {
        return Vec::new();
    }
    let bar = |command| ctx.read_response(crate::ui::bars::tool_button_id(command)).map(|r| r.rect);
    let catalogue = |command: &str| ctx.data(|d| d.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", command))));
    let source = match mode {
        "reorder" | "menu" => bar("preferences").map(|r| r.center()),
        "sidemenu" => bar("radar").map(|r| r.center()),
        "custom" => catalogue("build").map(|r| r.center()),
        _ => catalogue("friends").map(|r| r.center()),
    };
    let Some(source) = source else { return Vec::new() };
    // the mockup's drop guide: between the mini-map and Sit
    let target = if mode == "custom" {
        bar("minimap").map(|r| egui::pos2(r.right() + 1.0, r.center().y))
    } else {
        bar("inventory").map(|r| r.center())
    };
    let Some(target) = target else { return Vec::new() };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: if menu {
            egui::PointerButton::Secondary
        } else {
            egui::PointerButton::Primary
        },
        pressed,
        modifiers: Default::default(),
    };
    match frame {
        650 => vec![egui::Event::PointerMoved(source), button(source, true)],
        651 if menu => vec![button(source, false)],
        651 => vec![egui::Event::PointerMoved(source + Vec2::new(12.0, 0.0))],
        652..=655 if !menu => vec![egui::Event::PointerMoved(target)],
        656 if !menu && !matches!(mode, "drag" | "custom") => vec![button(target, false)],
        _ => Vec::new(),
    }
}
