//! The "Toolbar buttons" window (LLFloaterToybox, floater_toybox.xml), in
//! the layout of the bars mockup: the commands in a grid of tiles with a
//! filter, each toolbar's mode, alignment and size, Locked, Clear all and
//! Restore defaults. A command already in a toolbar stays in the grid,
//! hollow, and cannot be added twice (LLFloaterToybox::draw); dragging it
//! moves the existing button to another bar. A command Aurora
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
/// Responsive tiles: five across at the default width, 4 px apart.
const MIN_TILE_W: f32 = 144.0;

pub(super) fn columns(width: f32) -> usize {
    ((width + TILE_GAP) / (MIN_TILE_W + TILE_GAP)).floor().clamp(1.0, 8.0) as usize
}
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

fn default_height(screen: Rect) -> f32 {
    // Leave the dock reachable on compact screens, including before the
    // first frame reports the bars' exact work area.
    (screen.height() - 180.0).clamp(360.0, 600.0)
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
        Bar::Top => aurora_i18n::tr!("toolbar-in-top"),
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
fn tile(ui: &mut egui::Ui, p: &Palette, rect: Rect, button: &ToolButton, display: &str, state: Tile, movable: bool) -> egui::Response {
    let sense = if state == Tile::Free {
        egui::Sense::click_and_drag()
    } else if state == Tile::Placed && movable {
        egui::Sense::drag()
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
        display,
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
        Bar::Top => aurora_i18n::tr!("toolbar-bar-top"),
    };
    let mut out = None;
    ui.horizontal_wrapped(|ui| {
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
        let mut open = self.open;
        let mut window = widgets::Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-title"),
            default_pos(ctx),
            Vec2::new(w, default_height(screen)),
        )
        .help(aurora_i18n::tr!("toolbar-help-modes"))
        .steady_size();
        window.min_size = Vec2::new(560.0, 360.0);
        window.show(ctx, p, &mut open, |ui| {
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
            let all = super::catalogue_view::entries(layout, settings, &super::commands::catalogue(ctx));
            let hint = aurora_i18n::tr!("toolbar-search", count = all.len().to_string());
            ui.horizontal(|ui| {
                widgets::pills::search_box(ui, p, &mut self.search, &hint, ui.available_width());
            });
            ui.horizontal_wrapped(|ui| {
                let selected = self
                    .category
                    .map_or(aurora_i18n::tr!("toolbar-category-all"), |category| category.label());
                super::menu::button(ui, p, egui::Button::new(selected), |ui| {
                    if super::menu::check(
                        ui,
                        p,
                        "layout-grid-filled",
                        aurora_i18n::tr!("toolbar-category-all"),
                        self.category.is_none(),
                        true,
                    ) {
                        self.category = None;
                    }
                    for category in super::catalogue_view::Category::ALL {
                        if super::menu::check(ui, p, category.icon(), category.label(), self.category == Some(category), true) {
                            self.category = Some(category);
                        }
                    }
                });
                let mut locked = settings.locked;
                if widgets::checkbox(ui, p, &mut locked, aurora_i18n::tr!("toolbar-locked-bars")).changed() {
                    self.pending = Some(Edit::Lock);
                }
                if ui
                    .add_enabled(!settings.locked, egui::Button::new(aurora_i18n::tr!("toolbar-clear")))
                    .clicked()
                {
                    self.confirm = Some(false);
                }
                if ui
                    .add_enabled(!settings.locked, egui::Button::new(aurora_i18n::tr!("toolbar-restore")))
                    .clicked()
                {
                    self.confirm = Some(true);
                }
            });
            widgets::collapsing(
                ui,
                p,
                "toolbar_bar_settings",
                aurora_i18n::tr!("toolbar-bar-settings"),
                false,
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("toolbar_style_scroll")
                        .max_height((ui.available_height() * 0.45).max(60.0))
                        .min_scrolled_height(0.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let mut card = crate::ui::sheet::card_begin(ui);
                            card.child.spacing_mut().item_spacing.y = 8.0;
                            for bar in Bar::ALL {
                                if (matches!(bar, Bar::Bottom | Bar::Top) || !settings.bar(bar, layout).is_empty())
                                    && let Some(next) = style_row(&mut card.child, p, bar, islands, settings.style(bar))
                                {
                                    self.pending = Some(Edit::Style(bar, next));
                                }
                            }
                            crate::ui::sheet::card_end(ui, p, card);
                        });
                },
            );
            let filter = super::sort_key(self.search.trim());
            egui::ScrollArea::vertical()
                .id_salt("toolbar_tiles")
                .max_height(ui.available_height().max(0.0))
                .min_scrolled_height(0.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let full = ui.available_width();
                    let columns = columns(full);
                    let tile_w = ((full - TILE_GAP * (columns - 1) as f32) / columns as f32).floor();
                    for category in super::catalogue_view::Category::ALL {
                        if self.category.is_some_and(|selected| selected != category) {
                            continue;
                        }
                        let shown: Vec<_> = all
                            .iter()
                            .filter(|entry| entry.category == category && entry.matches(&filter))
                            .collect();
                        if shown.is_empty() {
                            continue;
                        }
                        widgets::section_header(ui, p, category.icon(), category.label());
                        let rows = shown.len().div_ceil(columns);
                        let height = rows as f32 * TILE_H + rows.saturating_sub(1) as f32 * TILE_GAP;
                        let (grid, _) = ui.allocate_exact_size(Vec2::new(full, height), egui::Sense::hover());
                        for (i, entry) in shown.iter().enumerate() {
                            let button = &entry.button;
                            let (row, col) = (i / columns, i % columns);
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
                            let response = tile(ui, p, rect, button, &entry.label(), state, !settings.locked);
                            if (super::structure::is_block(&button.command) || super::structure::is_separator(&button.command))
                                && let Some(bar) = placed
                            {
                                self.context(
                                    &response,
                                    p,
                                    bar,
                                    settings.style(bar).resolve(bar, islands),
                                    Some(button),
                                    settings.locked,
                                );
                            } else {
                                self.appearance_context(&response, p, button, settings.locked);
                            }
                            ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("toolbar_catalogue", &button.command)), rect));
                            if !settings.locked && response.drag_started_by(egui::PointerButton::Primary) {
                                response.dnd_set_drag_payload(Drag {
                                    button: button.clone(),
                                    from: None,
                                });
                            }
                            match state {
                                Tile::Free => {
                                    if response.clicked() {
                                        self.pending = Some(Edit::Insert(Bar::Bottom, button.clone(), settings.buttons(layout).len()));
                                    }
                                    response.on_hover_text(entry.label());
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
                    }
                });
        });
        self.open = open;
        self.show_appearance(ctx, p, settings);
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
    if mode.starts_with("separators") {
        let button = |id: &str| {
            let command = catalogue::find(id).expect("internal toolbar demo command");
            ToolButton {
                command: id.into(),
                icon: command.icon.into(),
                tooltip: String::new(),
            }
        };
        settings.top = super::structure::top_buttons(&[button("location_bar"), button("snapshot")]);
        settings.buttons = Some(vec![
            button("inventory"),
            super::structure::separator("demo-bottom"),
            button("people"),
            button("preferences"),
        ]);
        settings.left = vec![button("worldmap"), super::structure::separator("demo-left"), button("build")];
        settings.right = vec![button("radar"), super::structure::separator("demo-right"), button("search")];
        settings.composed = true;
        settings.locked = false;
        settings.address_visible = Some(true);
        settings.appearance.clear();
        if matches!(mode, "separators-bottom" | "separators-side") {
            let bottom = mode == "separators-bottom";
            let commands: &[&str] = if bottom {
                &["bar_navigation", "bar_notifications"]
            } else {
                &["bar_account", "bar_system", "bar_notifications"]
            };
            for (index, command) in commands.iter().enumerate() {
                settings.top.retain(|button| button.command != *command);
                let list = if bottom {
                    settings.buttons.as_mut().expect("demo bottom")
                } else if index == 0 {
                    &mut settings.left
                } else {
                    &mut settings.right
                };
                list.push(super::structure::separator(&format!("demo-block-{index}")));
                list.push(button(command));
            }
        }
        return;
    }
    if matches!(
        mode,
        "chrome" | "appearance" | "menu-layout" | "address" | "top-drag" | "categories" | "resized" | "menu-hover"
    ) {
        settings.composed = false;
        let catalogue = super::commands::current();
        let button = |id: &str| {
            catalogue.find(id).map(|entry| entry.button.clone()).or_else(|| {
                super::catalogue::find(id).map(|command| ToolButton {
                    command: id.into(),
                    icon: command.icon.into(),
                    tooltip: String::new(),
                })
            })
        };
        settings.top = [
            "location_bar",
            "menu:common-build/import-menu",
            "menu:common-world/common-environment",
        ]
        .into_iter()
        .filter_map(button)
        .collect();
        settings.left = [
            "menu:common-world/ui-bars-animation-explorer",
            "menu:common-world/avatar-render-title",
        ]
        .into_iter()
        .filter_map(button)
        .collect();
        settings.right = ["menu:common-advanced/quiet", "menu:common-world/common-world-map"]
            .into_iter()
            .filter_map(button)
            .collect();
        settings.menu_layout.clear();
        settings
            .menu_layout
            .insert(String::new(), vec!["menu:common-build/import-menu".into(), "location_bar".into()]);
        settings.appearance.insert(
            "menu:common-build/import-menu".into(),
            super::appearance::Appearance {
                mode: Some(ButtonMode::IconsWithText),
                icon: Some("star-filled".into()),
            },
        );
        settings.appearance.insert(
            "inventory".into(),
            super::appearance::Appearance {
                mode: Some(ButtonMode::TextOnly),
                icon: None,
            },
        );
        settings.appearance.insert(
            "people".into(),
            super::appearance::Appearance {
                mode: Some(ButtonMode::IconsWithText),
                icon: None,
            },
        );
        settings.address_visible = Some(mode == "address" && settings.address_visible.unwrap_or(false));
        settings.locked = false;
        if mode == "top-drag" {
            settings.top.truncate(1);
            settings.left.clear();
            settings.right.clear();
            settings.appearance.clear();
            settings.address_visible = Some(true);
        }
        return;
    }
    if matches!(mode, "empty-left" | "empty-right") {
        let bottom = settings.buttons(layout).to_vec();
        settings.buttons = Some(Vec::new());
        settings.left.clear();
        settings.right.clear();
        if mode == "empty-left" {
            settings.left = bottom;
        } else {
            settings.right = bottom;
        }
        settings.locked = false;
        return;
    }
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
    if mode.starts_with("separators") {
        if mode == "separators-editor" && frame == 300 {
            state.open = true;
            state.search = aurora_i18n::tr!("toolbar-separator").into();
            state.category = None;
        }
        if matches!(mode, "separators-bottom" | "separators-side")
            && matches!(frame, 500 | 501)
            && let Some(response) = ctx.read_response(crate::ui::bars::tool_button_id("bar_notifications"))
        {
            let pos = response.rect.center();
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 500,
                    modifiers: Default::default(),
                },
            ];
        }
        return Vec::new();
    }
    if mode == "top-drag" {
        let pointer = |pos, pressed| {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            events
        };
        if matches!(frame, 400 | 401) {
            let opener = ctx
                .read_response(egui::Id::unique("bars_island_menu"))
                .or_else(|| ctx.read_response(egui::Id::unique("classic_main_menu")));
            if let Some(opener) = opener {
                return pointer(opener.rect.center(), Some(frame == 400));
            }
        }
        let source_id = egui::Id::unique("demo_top_drag_source");
        let target_id = egui::Id::unique("demo_top_drag_target");
        if frame == 500 {
            let source = ctx.data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", "menu:common-build/import-menu"))));
            if let (Some(source), Some(bar)) = (source, ctx.read_response(egui::Id::unique("top_toolbar_background"))) {
                let target = if ctx.read_response(egui::Id::unique("classic_main_menu")).is_some() {
                    bar.rect.center()
                } else {
                    egui::pos2(bar.rect.right() + 24.0, bar.rect.center().y)
                };
                ctx.data_mut(|data| {
                    data.insert_temp(source_id, source.center());
                    data.insert_temp(target_id, target);
                });
                return pointer(source.center(), Some(true));
            }
        }
        if frame == 501
            && let Some(source) = ctx.data(|data| data.get_temp::<egui::Pos2>(source_id))
        {
            return pointer(source + Vec2::new(12.0, 0.0), None);
        }
        if (502..=680).contains(&frame)
            && let Some(target) = ctx.data(|data| data.get_temp::<egui::Pos2>(target_id))
        {
            return pointer(target, (frame == 680).then_some(false));
        }
        return Vec::new();
    }
    if matches!(
        mode,
        "chrome" | "appearance" | "menu-layout" | "address" | "categories" | "resized" | "menu-hover"
    ) {
        if frame == 300 {
            state.open = matches!(mode, "chrome" | "categories" | "resized");
            if matches!(mode, "categories" | "resized") {
                widgets::place_floater(ctx, WINDOW, default_pos(ctx), Vec2::new(width(ctx.content_rect()), 600.0));
                state.category = (mode == "categories").then_some(super::catalogue_view::Category::World);
                state.search.clear();
            }
            if mode == "appearance"
                && let Some(entry) = super::commands::catalogue(ctx).find("menu:common-build/import-menu")
            {
                state.edit_button(&entry.button);
                state.appearance_editor.filter("star");
            }
        }
        if mode == "menu-hover"
            && frame == 500
            && let Some(response) = ctx.read_response(egui::Id::unique("bars_island_menu"))
        {
            return vec![egui::Event::PointerMoved(response.rect.center())];
        }
        if mode == "resized" && matches!(frame, 500..=502) {
            let id = egui::Id::unique("demo_toolbar_resize_grip");
            if frame == 500
                && let Some(rect) = ctx.memory(|memory| memory.area_rect(egui::Id::unique(WINDOW)))
            {
                ctx.data_mut(|data| data.insert_temp(id, rect.right_bottom() - Vec2::splat(1.0)));
            }
            if let Some(grip) = ctx.data(|data| data.get_temp::<egui::Pos2>(id)) {
                let pos = if frame == 500 { grip } else { grip + Vec2::new(170.0, 70.0) };
                let mut events = vec![egui::Event::PointerMoved(pos)];
                if frame != 501 {
                    events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: frame == 500,
                        modifiers: Default::default(),
                    });
                }
                return events;
            }
        }
        if mode == "address"
            && matches!(frame, 300 | 301 | 500 | 501)
            && let Some(response) = ctx.read_response(crate::ui::bars::tool_button_id("location_bar"))
        {
            let pos = response.rect.center();
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: matches!(frame, 300 | 500),
                    modifiers: Default::default(),
                },
            ];
        }
        if mode == "menu-layout"
            && matches!(frame, 540 | 541)
            && let Some(response) = ctx.read_response(egui::Id::unique("bars_island_menu"))
        {
            let pos = response.rect.center();
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 540,
                    modifiers: Default::default(),
                },
            ];
        }
        return Vec::new();
    }
    if matches!(mode, "side" | "empty-left" | "empty-right") {
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
