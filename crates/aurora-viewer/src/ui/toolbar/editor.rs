//! The "Toolbar buttons" window (LLFloaterToybox, floater_toybox.xml), in
//! the layout of the bars mockup: the commands in a grid of tiles with a
//! filter, categories, Locked, Clear all and
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

impl ToolbarUi {
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        layout: &Layout,
        settings: &ToolbarSettings,
        selected_preset: Option<&super::selection::Selection>,
    ) {
        if let Some(request) = super::native_context::take(ctx) {
            self.open = true;
            self.selected = request.command.filter(|command| settings.placed(command, layout));
        }
        self.preset_view.refresh(settings, layout, self.islands);
        let source = self.preset_view.source(selected_preset);
        if selected_preset.is_none()
            && let Some(super::selection::Source::Builtin(preset)) = source.as_ref()
        {
            self.inferred_preset = Some(*preset);
        }
        let dirty = self.preset_view.dirty(selected_preset);
        let screen = ctx.content_rect();
        let w = width(screen);
        let islands = self.islands;
        super::blocks::show(ctx, p, self, settings, layout);
        let dragged = egui::DragAndDrop::payload::<Drag>(ctx);
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
            ui.horizontal(|ui| {
                let name = source.as_ref().map_or(aurora_i18n::tr!("toolbar-preset-custom"), |source| {
                    self.notecards.preset_label(source)
                });
                let caption = aurora_i18n::tr!("toolbar-current-preset", name = name);
                let personal = matches!(source.as_ref(), Some(super::selection::Source::Personal { .. }));
                let spacing = ui.spacing().item_spacing.x;
                let actions_width = if dirty { 150.0 + spacing } else { 0.0 } + if personal { 130.0 + spacing } else { 0.0 };
                let name_width = (ui.available_width() - actions_width).max(0.0);
                let (name_rect, name_response) = ui.allocate_exact_size(Vec2::new(name_width, 30.0), egui::Sense::hover());
                let font = egui::TextStyle::Body.resolve(ui.style());
                let badge = dirty.then(|| {
                    ui.painter()
                        .layout_no_wrap(aurora_i18n::tr!("toolbar-preset-unsaved").to_owned(), font.clone(), p.violet_light)
                });
                let caption_limit = (name_rect.width() - badge.as_ref().map_or(0.0, |badge| badge.size().x + spacing)).max(0.0);
                let caption_width = ui
                    .painter()
                    .layout_no_wrap(caption.clone(), font.clone(), p.ink)
                    .size()
                    .x
                    .min(caption_limit);
                widgets::paint_elided(
                    ui,
                    caption.as_str(),
                    egui::pos2(name_rect.left(), name_rect.center().y),
                    caption_width,
                    font,
                    p.ink,
                );
                if let Some(badge) = badge {
                    widgets::paint_ink_left(
                        ui.painter(),
                        badge,
                        egui::pos2(name_rect.left() + caption_width + spacing, name_rect.center().y),
                        p.violet_light,
                    );
                }
                name_response.on_hover_text(caption);
                if dirty
                    && ui
                        .add_enabled_ui(!self.notecards.busy(), |ui| {
                            widgets::action_button(
                                ui,
                                p,
                                "device-floppy-filled",
                                aurora_i18n::tr!("common-save"),
                                widgets::ActionKind::Flat,
                                Vec2::new(150.0, 30.0),
                            )
                        })
                        .inner
                        .clicked()
                {
                    self.notecards.open_save(source.as_ref());
                }
                if let Some(source @ super::selection::Source::Personal { .. }) = source.as_ref() {
                    let enabled = !self.notecards.busy() && self.notecards.can_remove(source);
                    let response = ui
                        .add_enabled_ui(enabled, |ui| {
                            widgets::action_button(
                                ui,
                                p,
                                "trash-filled",
                                aurora_i18n::tr!("common-delete"),
                                widgets::ActionKind::SoftDanger,
                                Vec2::new(130.0, 30.0),
                            )
                        })
                        .inner;
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_interface_delete"), response.rect));
                    if response.clicked() {
                        self.notecards.remove(source);
                    }
                }
            });
            ui.add_space(4.0);
            // Keep the command controls and one catalogue row reachable on short windows.
            let preset_height = (ui.available_height() - 290.0).max(48.0);
            egui::ScrollArea::vertical()
                .id_salt("toolbar_preset_section")
                .max_height(preset_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    widgets::section_header(ui, p, "layout-grid-filled", aurora_i18n::tr!("toolbar-presets"));
                    ui.add_enabled_ui(!settings.locked, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for preset in super::presets::Preset::ALL {
                                if widgets::action_button(
                                    ui,
                                    p,
                                    preset.icon(),
                                    preset.label(),
                                    if source == Some(super::selection::Source::Builtin(preset)) {
                                        widgets::ActionKind::Primary
                                    } else {
                                        widgets::ActionKind::Flat
                                    },
                                    Vec2::new(190.0, 30.0),
                                )
                                .clicked()
                                {
                                    self.preset_request = Some(preset);
                                }
                            }
                        });
                    });
                    self.notecards.presets(ui, p, settings.locked, source.as_ref());
                });
            ui.add_space(8.0);
            super::edit_mode::header(ui, p, settings);
            let all = super::catalogue_view::entries(layout, settings, &super::commands::catalogue(ctx));
            let count = all.iter().filter(|entry| entry.in_view(self.advanced_catalogue, "")).count();
            let hint = aurora_i18n::tr!("toolbar-search", count = count.to_string());
            ui.horizontal(|ui| {
                widgets::pills::search_box(ui, p, &mut self.search, &hint, ui.available_width());
            });
            let searching = !self.search.trim().is_empty();
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!searching, |ui| {
                    let mut selected = usize::from(self.advanced_catalogue);
                    if widgets::segmented(
                        ui,
                        p,
                        &mut selected,
                        &[
                            (aurora_i18n::tr!("toolbar-section-main"), true),
                            (aurora_i18n::tr!("toolbar-section-advanced"), true),
                        ],
                    ) {
                        self.advanced_catalogue = selected == 1;
                    }
                });
                ui.add_space(16.0);
                // Structural tools stay outside the searchable command catalogue:
                // the separator, and a new floating block beside it
                for id in [super::structure::DIVIDER, super::drawers::FLOATING] {
                    let command = catalogue::find(id).expect("built-in structural tool");
                    let tool = ToolButton {
                        command: command.id.into(),
                        icon: command.icon.into(),
                        tooltip: String::new(),
                    };
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(180.0, TILE_H), egui::Sense::hover());
                    let response = tile(
                        ui,
                        p,
                        rect,
                        &tool,
                        label(&tool),
                        if settings.locked { Tile::Placed } else { Tile::Free },
                        false,
                    );
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("toolbar_catalogue", &tool.command)), rect));
                    if !settings.locked {
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            response.dnd_set_drag_payload(Drag {
                                button: tool.clone(),
                                from: None,
                                members: Vec::new(),
                            });
                        }
                        if response.clicked() {
                            self.pending = Some(Edit::Insert(Bar::Bottom, tool, settings.buttons(layout).len()));
                        }
                    }
                }
            });
            if searching {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("toolbar-search-all"))
                        .size(12.0)
                        .color(p.muted),
                );
            }
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
            let filter = super::sort_key(self.search.trim());
            egui::ScrollArea::vertical()
                .id_salt(("toolbar_tiles", self.advanced_catalogue, !filter.is_empty()))
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
                            .filter(|entry| entry.category == category && entry.in_view(self.advanced_catalogue, &filter))
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
                            } else if placed.is_some() || settings.placed(&button.command, layout) {
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
                            if (super::structure::is_block(&button.command)
                                || super::structure::is_separator(&button.command)
                                || super::structure::is_divider(&button.command))
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
                                    members: Vec::new(),
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
        if !self.open {
            self.selected = None;
        }
        self.show_appearance(ctx, p, settings);
        self.show_context_appearance(ctx, p, settings);
        self.show_menu_row_context(ctx, p, settings);
        super::drawers::show_settings(ctx, p, self, settings, layout);
        self.show_shortcut_editor(ctx, p, settings, layout);
        super::surface_drop::show(ctx, p, self, settings, layout);
        super::drop_preview::show(ctx, p, self, settings, layout);
        let removing = self.pending.is_none()
            && dragged
                .as_ref()
                .is_some_and(|drag| super::drop_preview::removal(ctx, drag, settings));
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique("toolbar_drag_remove"), removing));
        if removing
            && ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
            && let Some(drag) = &dragged
        {
            self.pending = Some(if drag.members.is_empty() {
                Edit::Remove(drag.button.command.clone())
            } else {
                Edit::RemoveBlock(drag.members.iter().map(|button| button.command.clone()).collect())
            });
            egui::DragAndDrop::clear_payload(ctx);
        }
        // "Floating block" let go over the world: a new one there
        if self.pending.is_none()
            && ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
            && let Some(drag) = &dragged
            && let Some(pos) = super::drop_preview::floating_spot(ctx, drag, settings)
        {
            self.pending = Some(Edit::FloatingNew(pos - Vec2::splat(22.0)));
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
            ghost(ctx, p, &drag, pointer, self.islands);
        }
    }
}

/// The detached drag item is a small glass group; near a destination the real
/// provisional toolbar renders it instead, using its eventual size and mode.
fn ghost(ctx: &egui::Context, p: &Palette, drag: &Drag, pointer: egui::Pos2, islands: bool) {
    if super::drop_preview::snapped(ctx, &drag.button.command) {
        return;
    }
    if !drag.members.is_empty() {
        ghost_block(ctx, p, drag, pointer, islands);
        return;
    }
    let removing = ctx
        .data(|data| data.get_temp::<bool>(egui::Id::unique("toolbar_drag_remove")))
        .unwrap_or(false);
    let button = &drag.button;
    let appearance = super::appearance::get(ctx, &button.command);
    let settings = super::appearance::settings(ctx);
    let mode = appearance.mode.unwrap_or_else(|| {
        settings
            .drawer_of(&button.command)
            .and_then(|id| settings.drawers.get(id))
            .map(|drawer| drawer.mode)
            .unwrap_or_else(|| {
                drag.from
                    .map_or(ButtonMode::IconsWithText, |bar| settings.style(bar).resolve(bar, islands).mode)
            })
    });
    let icon_name = appearance.icon.as_deref().unwrap_or(&button.icon);
    let width =
        crate::ui::bars::natural_width(ctx, button, mode, 32.0, 16.0, 38.0).clamp(28.0, (ctx.content_rect().width() - 24.0).max(28.0));
    let width = width.max(if removing { 100.0 } else { 0.0 });
    let shown = egui::Area::new(egui::Id::unique("toolbar_drag_preview"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(pointer + Vec2::new(12.0, 12.0))
        .fade_in(false)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                if removing {
                    remove_badge(ui, p, false);
                }
                let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 32.0), egui::Sense::hover());
                let painter = ui.painter();
                let has_icon = mode != ButtonMode::TextOnly;
                let icon = Rect::from_center_size(
                    if mode == ButtonMode::IconsOnly {
                        rect.center()
                    } else {
                        rect.left_center() + Vec2::new(16.0, 0.0)
                    },
                    Vec2::splat(16.0),
                );
                if has_icon {
                    if icon_name == "AO" {
                        let text = painter.layout_no_wrap("AO".into(), egui::FontId::proportional(12.0), p.ink);
                        widgets::paint_ink_centered(painter, text, icon.center(), p.ink);
                    } else if let Some(texture) = crate::ui::icons::global(icon_name) {
                        painter.image(
                            texture.id(),
                            icon,
                            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            p.ink,
                        );
                    }
                }
                if mode != ButtonMode::IconsOnly {
                    let left = if has_icon {
                        icon.right() + crate::ui::bars::sizing::ICON_GAP
                    } else {
                        rect.left() + crate::ui::bars::sizing::LABEL_PAD
                    };
                    widgets::paint_elided(
                        ui,
                        label(button),
                        egui::pos2(left, rect.center().y),
                        rect.right() - left - 8.0,
                        egui::FontId::proportional(12.0),
                        p.ink,
                    );
                }
            });
        });
    if removing {
        removal_outline(ctx, p, &shown.response);
    }
}

fn remove_badge(ui: &mut egui::Ui, p: &Palette, whole: bool) {
    ui.horizontal(|ui| {
        widgets::glyph(ui, "trash-filled", p.danger, 15.0);
        ui.label(
            egui::RichText::new(if whole {
                aurora_i18n::tr!("toolbar-remove-block")
            } else {
                aurora_i18n::tr!("toolbar-remove")
            })
            .color(p.danger),
        );
    });
}

fn removal_outline(ctx: &egui::Context, p: &Palette, response: &egui::Response) {
    ctx.layer_painter(response.layer_id).rect_stroke(
        response.rect,
        ctx.global_style().visuals.window_corner_radius,
        egui::Stroke::new(2.0, p.danger),
        egui::StrokeKind::Inside,
    );
}

fn ghost_block(ctx: &egui::Context, p: &Palette, drag: &Drag, pointer: egui::Pos2, islands: bool) {
    let settings = super::appearance::settings(ctx);
    let vertical = drag.from.is_some_and(Bar::vertical);
    let inherited = drag
        .from
        .map_or(ButtonMode::IconsWithText, |bar| settings.style(bar).resolve(bar, islands).mode);
    let removing = ctx
        .data(|data| data.get_temp::<bool>(egui::Id::unique("toolbar_drag_remove")))
        .unwrap_or(false);
    let shown = egui::Area::new(egui::Id::unique("toolbar_drag_preview"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(pointer + Vec2::splat(12.0))
        .fade_in(false)
        .show(ctx, |ui| {
            let looks: Vec<_> = drag
                .members
                .iter()
                .map(|button| {
                    let appearance = super::appearance::get(ctx, &button.command);
                    let mode = appearance.mode.unwrap_or(inherited);
                    let icon = if mode == ButtonMode::TextOnly {
                        ""
                    } else {
                        appearance.icon.as_deref().unwrap_or(&button.icon)
                    };
                    let label = if mode == ButtonMode::IconsOnly { "" } else { label(button) };
                    let width = if super::structure::is_divider(&button.command) {
                        super::structure::DIVIDER_SIZE
                    } else if mode == ButtonMode::IconsOnly {
                        32.0
                    } else {
                        widgets::action_button_fit(ui, icon, label)
                    };
                    (button, icon.to_owned(), label.to_owned(), width)
                })
                .collect();
            let width = if vertical {
                looks.iter().map(|look| look.3).fold(80.0, f32::max)
            } else {
                looks.iter().map(|look| look.3).sum::<f32>() + 2.0 * looks.len().saturating_sub(1) as f32
            };
            ui.set_width(
                width
                    .max(if removing { 130.0 } else { 24.0 })
                    .min((ctx.content_rect().width() - 24.0).max(24.0)),
            );
            widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                if removing {
                    remove_badge(ui, p, true);
                }
                let body = |ui: &mut egui::Ui| {
                    for (button, icon, label, width) in &looks {
                        if super::structure::is_divider(&button.command) {
                            let (rect, _) = ui.allocate_exact_size(
                                if vertical {
                                    Vec2::new(ui.available_width(), 12.0)
                                } else {
                                    Vec2::new(12.0, 30.0)
                                },
                                egui::Sense::hover(),
                            );
                            let ends = if vertical {
                                [rect.left_center(), rect.right_center()]
                            } else {
                                [rect.center_top(), rect.center_bottom()]
                            };
                            ui.painter().line_segment(ends, egui::Stroke::new(1.0, p.muted_dim));
                        } else {
                            let width = if vertical { ui.available_width() } else { *width };
                            widgets::action_button(
                                ui,
                                p,
                                if icon == "AO" { "" } else { icon },
                                if icon == "AO" && label.is_empty() { "AO" } else { label },
                                widgets::ActionKind::Flat,
                                Vec2::new(width, 30.0),
                            );
                        }
                    }
                };
                if vertical {
                    body(ui);
                } else {
                    ui.horizontal_wrapped(body);
                }
            });
        });
    if removing {
        removal_outline(ctx, p, &shown.response);
    }
}
/// The settings of a demo run (`AURORA_DEMO_TOOLBAR`): `custom` is the
/// mockup's "custom" state (Environments, Radar and Teleport history in the
/// left bar, Performance out of the dock); `side` the same bars without the
/// window (`sidemenu`: Radar's right click at frame 650); `right` them in
/// the right bar; `labels` adds the dock's icons and labels and the left
/// bar's labels. Never saved.
pub fn demo_settings(settings: &mut ToolbarSettings, layout: &Layout) {
    let Some(mode) = demo_mode() else { return };
    if mode.starts_with("notecard-") {
        return;
    }
    if let Some(preset) = super::presets::demo_mode(mode) {
        *settings = preset.layout(layout);
        return;
    }
    // Existing fixtures describe the earlier implicit chat island explicitly.
    settings.chat_composed = false;
    if mode == "defaults" || mode.starts_with("chat-") || mode.starts_with("contexts-") {
        *settings = ToolbarSettings::default();
        settings.compose_top();
        settings.compose_chat(layout);
        if mode.contains("microphone") {
            settings.buttons = Some(vec![crate::ui::bars::top::button("speak", "microphone-filled")]);
            settings.bottom_style = super::BarStyle {
                sizing: Some(Sizing::Auto),
                align: Some(Align::Center),
                ..Default::default()
            };
        }
        if matches!(mode, "contexts-icons-inventory-left" | "contexts-icons-inventory-right") {
            let bar = if mode.ends_with("left") { Bar::Left } else { Bar::Right };
            let mut transaction = ToolbarUi {
                pending: Some(Edit::Insert(
                    bar,
                    crate::ui::bars::top::button("inventory", "briefcase-filled"),
                    usize::MAX,
                )),
                ..Default::default()
            };
            transaction.commit(layout, settings);
            settings.appearance.insert(
                "inventory".into(),
                super::appearance::Appearance {
                    mode: Some(ButtonMode::TextOnly),
                    icon: None,
                },
            );
        }
        if mode != "defaults" && !mode.starts_with("contexts-") {
            let bar = proximity_bar(mode);
            if bar != Bar::Bottom {
                let mut transaction = ToolbarUi {
                    pending: Some(Edit::Insert(
                        bar,
                        crate::ui::bars::top::button("bar_chat", "message-filled"),
                        usize::MAX,
                    )),
                    ..Default::default()
                };
                transaction.commit(layout, settings);
            }
            if mode.contains("drawer") {
                let mut split = ToolbarUi {
                    pending: Some(Edit::Split(bar, "bar_chat".into(), true)),
                    ..Default::default()
                };
                split.commit(layout, settings);
                let mut transaction = ToolbarUi {
                    pending: Some(Edit::Fold(bar, "bar_chat".into())),
                    ..Default::default()
                };
                transaction.commit(layout, settings);
                if let Some(drawer) = settings.drawer_of("bar_chat") {
                    // A fixed ID keeps the per-frame fixture's open state stable.
                    let old = drawer.to_owned();
                    let value = settings.drawers.remove(&old).expect("demo chat drawer");
                    settings.drawers.insert("drawer:demo-chat".into(), value);
                    let mut buttons = settings.bar(bar, layout).to_vec();
                    for button in &mut buttons {
                        if button.command == old {
                            button.command = "drawer:demo-chat".into();
                        }
                    }
                    settings.set_bar(bar, buttons);
                }
            }
        }
        return;
    }
    if mode.starts_with("separators")
        || mode.starts_with("conversations-")
        || mode.starts_with("split-controls")
        || mode.starts_with("interface-edit")
        || mode.starts_with("drawer-")
        || mode.starts_with("proximity-")
        || mode.starts_with("edge-groups-")
        || mode.starts_with("blocks-")
        || mode.starts_with("slurl-")
    {
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
        settings.block_styles.clear();
        settings.separators.clear();
        settings.drawers.clear();
        if mode.starts_with("blocks-alignment") {
            settings.chat_composed = true;
            settings.top = vec![
                button("aurora_menu"),
                super::structure::separator("align-top-a"),
                button("bar_navigation"),
                super::structure::separator("align-top-b"),
                button("bar_notifications"),
            ];
            settings.buttons = Some(vec![
                button("bar_chat"),
                super::structure::separator("align-bottom-a"),
                button("inventory"),
                button("people"),
                super::structure::separator("align-bottom-b"),
                button("preferences"),
            ]);
            settings.left = vec![
                button("worldmap"),
                super::structure::separator("align-left-a"),
                button("build"),
                super::structure::separator("align-left-b"),
                button("search"),
            ];
            settings.right = vec![
                button("radar"),
                super::structure::separator("align-right-a"),
                button("bar_fps"),
                super::structure::separator("align-right-b"),
                button("bar_volume"),
            ];
            for bar in Bar::ALL {
                settings.style_mut(bar).sizing = Some(Sizing::Auto);
                let groups = settings
                    .bar(bar, layout)
                    .split(|button| super::structure::is_separator(&button.command))
                    .map(<[ToolButton]>::to_vec)
                    .collect::<Vec<_>>();
                for (group, align) in groups.into_iter().zip([Align::Start, Align::Center, Align::End]) {
                    for button in group {
                        settings.block_styles.insert(
                            button.command,
                            super::BarStyle {
                                align: Some(align),
                                ..Default::default()
                            },
                        );
                    }
                }
            }
            return;
        }
        if mode.starts_with("blocks-") || mode.starts_with("slurl-") {
            settings.top = vec![
                button("aurora_menu"),
                button("bar_navigation"),
                super::structure::separator("demo-top"),
                button("location_bar"),
                button("snapshot"),
                super::structure::divider("demo-top"),
                button("preferences"),
                super::structure::separator("demo-top-end"),
                button("bar_fps"),
                button("bar_notifications"),
            ];
            settings.left = vec![button("worldmap"), super::structure::divider("demo-left"), button("build")];
            settings.right = vec![button("radar"), super::structure::divider("demo-right"), button("search")];
            settings.buttons = Some(vec![
                button("inventory"),
                super::structure::divider("demo-bottom"),
                button("people"),
            ]);
            for bar in Bar::ALL {
                settings.style_mut(bar).sizing = Some(Sizing::Auto);
            }
            if mode.starts_with("blocks-scope-") {
                for bar in Bar::ALL {
                    settings.set_bar(bar, Vec::new());
                }
                let bar = proximity_bar(mode);
                settings.set_bar(
                    bar,
                    vec![
                        button("inventory"),
                        button("people"),
                        super::structure::separator("demo-scope"),
                        button("worldmap"),
                        button("build"),
                    ],
                );
                for (command, mode, sizing) in [
                    (
                        "inventory",
                        ButtonMode::TextOnly,
                        if mode.contains("fill") { Sizing::Fill } else { Sizing::Auto },
                    ),
                    ("worldmap", ButtonMode::IconsOnly, Sizing::Fixed),
                ] {
                    let mut transaction = ToolbarUi {
                        pending: Some(Edit::BlockStyle(
                            bar,
                            command.into(),
                            BarStyle {
                                mode: Some(mode),
                                sizing: Some(sizing),
                                align: None,
                            },
                        )),
                        ..Default::default()
                    };
                    transaction.commit(layout, settings);
                }
            }
            if matches!(mode, "blocks-context-drawer" | "blocks-unfold") {
                settings.top = vec![
                    button("preferences"),
                    crate::ui::bars::top::button("drawer:demo-unfold", "folder-filled"),
                    button("snapshot"),
                ];
                settings.buttons = Some(Vec::new());
                settings.drawers.insert(
                    "drawer:demo-unfold".into(),
                    super::drawers::Drawer {
                        buttons: vec![button("inventory"), button("people")],
                        mode: ButtonMode::IconsWithText,
                        ..Default::default()
                    },
                );
                if mode == "blocks-unfold" {
                    let mut transaction = ToolbarUi {
                        pending: Some(Edit::Unfold(Bar::Top, "drawer:demo-unfold".into())),
                        ..Default::default()
                    };
                    transaction.commit(layout, settings);
                    for (index, button) in settings.top.iter_mut().enumerate() {
                        if super::structure::is_separator(&button.command) {
                            *button = super::structure::separator(&format!("demo-unfold-{index}"));
                        }
                    }
                }
            }
            if mode.starts_with("slurl-") {
                let bar = match mode {
                    "slurl-bottom" => Bar::Bottom,
                    "slurl-left" => Bar::Left,
                    "slurl-right" => Bar::Right,
                    _ => Bar::Top,
                };
                settings.top.retain(|button| button.command != "location_bar");
                let mut list = settings.bar(bar, layout).to_vec();
                list.push(button("location_bar"));
                settings.set_bar(bar, list);
            }
        }
        if mode.starts_with("edge-groups-") {
            for bar in Bar::ALL {
                settings.set_bar(bar, Vec::new());
                *settings.style_mut(bar) = Default::default();
            }
            let middle = mode == "edge-groups-middle";
            settings.top = vec![button("bar_fps"), super::structure::separator("demo-fps")];
            if middle {
                settings
                    .top
                    .extend([button("bar_notifications"), super::structure::separator("demo-bell")]);
            }
            settings
                .top
                .extend([button("bar_media"), button("bar_music"), button("bar_volume")]);
            if !middle {
                if mode != "edge-groups-joined" {
                    settings.top.push(super::structure::separator("demo-bell"));
                }
                settings.top.push(button("bar_notifications"));
            }
            settings.top_style.align = Some(match mode {
                "edge-groups-start" => Align::Start,
                "edge-groups-center" => Align::Center,
                _ => Align::End,
            });
            settings.address_visible = Some(false);
        }
        if mode.starts_with("proximity-") {
            for bar in Bar::ALL {
                settings.set_bar(bar, Vec::new());
                *settings.style_mut(bar) = Default::default();
                settings.style_mut(bar).sizing = Some(Sizing::Auto);
            }
            let bar = proximity_bar(mode);
            settings.set_bar(bar, vec![button("inventory"), button("people")]);
            if mode.contains("isolated") {
                settings.set_bar(
                    bar,
                    vec![button("inventory"), super::structure::separator("demo-isolated"), button("radar")],
                );
                settings.separators.insert(
                    "separator:demo-isolated".into(),
                    super::structure::SeparatorStyle {
                        size: 80.0,
                        stretch: false,
                    },
                );
            }
            if mode.ends_with("-offset") {
                settings.top_style.align = Some(Align::Center);
            }
        }
        if mode == "interface-edit-empty" {
            for bar in Bar::ALL {
                settings.set_bar(bar, Vec::new());
            }
        }
        if mode.starts_with("drawer-") {
            let children: &[&str] = if mode == "drawer-native" {
                &[
                    "bar_status",
                    "bar_balance",
                    "bar_clock",
                    "bar_media",
                    "bar_music",
                    "bar_volume",
                    "bar_conversations",
                ]
            } else {
                &["inventory", "worldmap", "preferences", "bar_volume"]
            };
            for bar in Bar::ALL {
                let buttons = settings
                    .bar(bar, layout)
                    .iter()
                    .filter(|b| !children.contains(&b.command.as_str()))
                    .cloned()
                    .collect();
                settings.set_bar(bar, buttons);
            }
            if mode.starts_with("drawer-overflow") {
                for bar in Bar::ALL {
                    settings.set_bar(bar, Vec::new());
                }
            }
            let bar = match mode {
                "drawer-right" | "drawer-edit" | "drawer-native" | "drawer-icons-right" | "drawer-mixed-right" => Bar::Right,
                "drawer-bottom" | "drawer-overflow-bottom" | "drawer-icons-bottom" => Bar::Bottom,
                "drawer-overflow-top" | "drawer-icons-top" => Bar::Top,
                _ => Bar::Left,
            };
            let mut buttons = settings.bar(bar, layout).to_vec();
            buttons.push(crate::ui::bars::top::button("drawer:demo", "folder-filled"));
            settings.set_bar(bar, buttons);
            settings.drawers.insert(
                "drawer:demo".into(),
                super::drawers::Drawer {
                    buttons: children.iter().map(|id| button(id)).collect(),
                    ..Default::default()
                },
            );
            if mode.starts_with("drawer-mode-")
                || mode.starts_with("drawer-icons-")
                || mode.starts_with("drawer-mixed-")
                || mode.starts_with("drawer-overflow")
                || mode.starts_with("drawer-settings")
                || mode == "drawer-empty-icons"
            {
                let drawer = settings.drawers.get_mut("drawer:demo").expect("demo drawer");
                drawer.buttons = ["inventory", "worldmap", "preferences", "search"].map(button).into();
                drawer.mode = if mode.starts_with("drawer-icons-") || mode.starts_with("drawer-mixed-") {
                    ButtonMode::IconsOnly
                } else {
                    match mode {
                        "drawer-mode-icons" | "drawer-empty-icons" => ButtonMode::IconsOnly,
                        "drawer-mode-text" => ButtonMode::TextOnly,
                        _ => ButtonMode::IconsWithText,
                    }
                };
                if mode.starts_with("drawer-mixed-") {
                    settings.appearance.insert(
                        "worldmap".into(),
                        super::appearance::Appearance {
                            mode: Some(ButtonMode::IconsWithText),
                            ..Default::default()
                        },
                    );
                }
                if mode == "drawer-empty-icons" {
                    drawer.buttons.clear();
                }
                if mode.starts_with("drawer-overflow") || mode == "drawer-icons-overflow" {
                    drawer.buttons = catalogue::COMMANDS
                        .iter()
                        .filter(|command| !super::structure::is_block(command.id) && !super::structure::is_separator(command.id))
                        .take(32)
                        .map(|command| button(command.id))
                        .collect();
                }
                let children = drawer.buttons.iter().map(|button| button.command.clone()).collect::<Vec<_>>();
                for bar in Bar::ALL {
                    let list = settings
                        .bar(bar, layout)
                        .iter()
                        .filter(|button| !children.contains(&button.command))
                        .cloned()
                        .collect();
                    settings.set_bar(bar, list);
                }
            }
        }
        if mode.starts_with("drawer-group-insert") {
            let buttons = vec![button("people"), button("conversations")];
            if mode.ends_with("-side") {
                settings.buttons = Some(Vec::new());
                settings.right = buttons;
            } else {
                settings.buttons = Some(buttons);
            }
        }
        if mode == "separators-size" {
            for (id, size) in [("top-1", 72.0), ("demo-left", 80.0), ("demo-bottom", 96.0)] {
                settings
                    .separators
                    .insert(format!("separator:{id}"), super::structure::SeparatorStyle { size, stretch: false });
            }
        }
        if mode.starts_with("separators-stretch") {
            for id in ["top-5", "demo-bottom", "demo-left", "demo-right"] {
                settings.separators.insert(
                    format!("separator:{id}"),
                    super::structure::SeparatorStyle {
                        stretch: true,
                        ..Default::default()
                    },
                );
            }
            settings
                .buttons
                .as_mut()
                .expect("demo bottom")
                .insert(3, super::structure::separator("demo-bottom-second"));
            settings.separators.insert(
                "separator:demo-bottom-second".into(),
                super::structure::SeparatorStyle { size: 24.0, stretch: true },
            );
        }
        if matches!(mode, "split-controls-side" | "split-controls-bottom") {
            let bottom = mode == "split-controls-bottom";
            for (index, command) in [
                "bar_status",
                "bar_balance",
                "bar_clock",
                "bar_fps",
                "bar_media",
                "bar_music",
                "bar_volume",
            ]
            .into_iter()
            .enumerate()
            {
                settings.top.retain(|button| button.command != command);
                let list = if bottom {
                    settings.buttons.as_mut().expect("demo bottom")
                } else if index < 2 {
                    &mut settings.left
                } else {
                    &mut settings.right
                };
                list.push(super::structure::separator(&format!("demo-split-{index}")));
                list.push(button(command));
            }
        }
        if mode.starts_with("conversations-") {
            settings.top.retain(|button| button.command != "bar_conversations");
            let list = if mode.starts_with("conversations-left") {
                &mut settings.left
            } else if mode.starts_with("conversations-right") {
                &mut settings.right
            } else if mode == "conversations-bottom" {
                settings.buttons.as_mut().expect("demo bottom")
            } else {
                &mut settings.top
            };
            list.push(super::structure::separator("demo-conversations"));
            list.push(button("bar_conversations"));
        }
        if matches!(mode, "separators-bottom" | "separators-side") {
            let bottom = mode == "separators-bottom";
            let commands: &[&str] = if bottom {
                &["bar_navigation", "bar_notifications"]
            } else {
                &[
                    "bar_status",
                    "bar_balance",
                    "bar_clock",
                    "bar_fps",
                    "bar_media",
                    "bar_music",
                    "bar_volume",
                    "bar_notifications",
                ]
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
        "chrome"
            | "appearance"
            | "menu-layout"
            | "address"
            | "top-drag"
            | "categories"
            | "resized"
            | "menu-hover"
            | "catalogue-main"
            | "catalogue-advanced"
            | "catalogue-search"
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
    if matches!(mode, "menu-add" | "menu-remove") {
        settings.locked = false;
        let mut buttons = settings.buttons(layout).to_vec();
        buttons.retain(|button| button.command != "import");
        if mode == "menu-remove" {
            let command = catalogue::find("import").expect("the catalogue holds the Import command");
            buttons.push(ToolButton {
                command: command.id.into(),
                icon: command.icon.into(),
                tooltip: String::new(),
            });
        }
        settings.buttons = Some(buttons);
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

fn proximity_bar(mode: &str) -> Bar {
    if mode.contains("left") {
        Bar::Left
    } else if mode.contains("right") {
        Bar::Right
    } else if mode.contains("bottom") {
        Bar::Bottom
    } else {
        Bar::Top
    }
}

/// Offline scenarios use the real widgets and pointer events, as the human does.
pub fn demo_events(ctx: &egui::Context, frame: u64, state: &mut ToolbarUi) -> Vec<egui::Event> {
    let Some(mode) = demo_mode() else {
        return Vec::new();
    };
    if mode.starts_with("notecard-") {
        return Vec::new();
    }
    if super::presets::demo_mode(mode).is_some() {
        if frame == 300 {
            state.open = mode.ends_with("-edit") || mode.ends_with("-advanced");
            if mode.ends_with("-advanced") {
                state.advanced_catalogue = true;
                state.search = "import_".into();
            }
        }
        return Vec::new();
    }
    if let Some(target) = mode.strip_prefix("contexts-") {
        let repeated = target.starts_with("repeat-");
        let target = target.trim_start_matches("repeat-");
        let icons = target.starts_with("icons-");
        let target = target.trim_start_matches("icons-");
        let editing = target.starts_with("edit-");
        if frame == 300 {
            state.open = editing;
        }
        let command = match target.trim_start_matches("edit-") {
            "bug" => "report_bug",
            "menu" => "aurora_menu",
            "microphone" => "speak",
            "navigation" => "bar_navigation",
            "status" => "bar_status",
            "balance" => "bar_balance",
            "clock" => "bar_clock",
            "fps" => "bar_fps",
            "media" => "bar_media",
            "music" => "bar_music",
            "volume" => "bar_volume",
            "notifications" => "bar_notifications",
            _ => "inventory",
        };
        if matches!(frame, 400 | 401)
            && let Some(source) = ctx.read_response(if command == "speak" {
                egui::Id::unique("toolbar_background")
            } else {
                crate::ui::bars::tool_button_id(command)
            })
        {
            let pos = source.rect.center();
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique("demo_toolbar_context_anchor"), pos));
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: frame == 400,
                    modifiers: Default::default(),
                },
            ];
        }
        if repeated
            && matches!(frame, 500 | 501)
            && let Some(pos) = ctx.data(|data| data.get_temp::<egui::Pos2>(egui::Id::unique("demo_toolbar_context_anchor")))
        {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: frame == 500,
                    modifiers: Default::default(),
                },
            ];
        }
        if icons
            && matches!(frame, 500 | 501)
            && let Some(rect) = ctx.data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_icon_button_rect", command))))
        {
            let pos = rect.center();
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
        if icons && frame == 520 {
            state.context_icon_picker.filter("star");
        }
        return Vec::new();
    }
    if mode == "defaults" || mode.starts_with("chat-") {
        if frame == 300 {
            state.open = mode.contains("edit");
            if mode.contains("drawer") {
                super::drawers::set_open(ctx, "drawer:demo-chat", true);
            }
        }
        return Vec::new();
    }
    if mode.starts_with("blocks-") || mode.starts_with("slurl-") {
        if mode.starts_with("blocks-alignment") {
            if frame == 300 {
                state.open = mode.ends_with("edit");
            }
            return Vec::new();
        }
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
        if mode.starts_with("slurl-")
            && matches!(frame, 400 | 401)
            && let Some(source) = ctx.read_response(crate::ui::bars::tool_button_id("location_bar"))
        {
            return pointer(source.rect.center(), Some(frame == 400));
        }
        if frame == 300 && mode.starts_with("blocks-") && mode != "blocks-dividers" && !mode.starts_with("blocks-context-") {
            state.open = true;
        }
        if mode.starts_with("blocks-settings-") && matches!(frame, 400 | 401) {
            let (bar, command) = match mode {
                "blocks-settings-bottom" => (Bar::Bottom, "inventory"),
                "blocks-settings-left" => (Bar::Left, "worldmap"),
                "blocks-settings-right" => (Bar::Right, "radar"),
                _ => (Bar::Top, "location_bar"),
            };
            if let Some(rect) = ctx.data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_block_settings", bar, command)))) {
                return pointer(rect.center(), Some(frame == 400));
            }
        }
        if mode.starts_with("blocks-context-") && matches!(frame, 400 | 401) {
            let command = if mode == "blocks-context-drawer" {
                "drawer:demo-unfold"
            } else if mode == "blocks-context-aurora" {
                "aurora_menu"
            } else {
                "inventory"
            };
            if let Some(response) = ctx.read_response(crate::ui::bars::tool_button_id(command)) {
                let pos = response.rect.center();
                return vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed: frame == 400,
                        modifiers: Default::default(),
                    },
                ];
            }
        }
        if matches!(mode, "blocks-drag" | "blocks-remove" | "blocks-remove-item") {
            if matches!(frame, 500 | 501) {
                let id = if mode == "blocks-remove-item" {
                    crate::ui::bars::tool_button_id("snapshot")
                } else {
                    egui::Id::unique(("toolbar_block_grip", Bar::Bottom, "inventory"))
                };
                if let Some(source) = ctx.read_response(id) {
                    return pointer(
                        source.rect.center() + if frame == 501 { Vec2::new(12.0, 0.0) } else { Vec2::ZERO },
                        (frame == 500).then_some(true),
                    );
                }
            }
            if frame >= 510 {
                let screen = ctx.content_rect();
                let target = if mode == "blocks-drag" {
                    egui::pos2(screen.right() - 16.0, screen.height() * 0.75)
                } else {
                    egui::pos2(screen.width() * 0.82, screen.height() * 0.5)
                };
                return pointer(target, None);
            }
        }
        return Vec::new();
    }
    if mode.starts_with("proximity-") {
        if frame == 300 {
            state.open = true;
            state.search = catalogue::find("radar").expect("demo radar").label().into();
            state.category = None;
            state.advanced_catalogue = false;
        }
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
        if matches!(frame, 500 | 501)
            && let Some(source) = ctx.data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", "radar"))))
        {
            return pointer(
                source.center() + if frame == 501 { Vec2::new(12.0, 0.0) } else { Vec2::ZERO },
                (frame == 500).then_some(true),
            );
        }
        let target_id = egui::Id::unique("demo_proximity_target");
        if frame == 510
            && let Some(anchor) = ctx.read_response(crate::ui::bars::tool_button_id(if mode.contains("isolated") {
                "radar"
            } else {
                "people"
            }))
        {
            let bar = proximity_bar(mode);
            let offset = if mode.contains("isolated") {
                if bar.vertical() {
                    Vec2::new(0.0, -40.0)
                } else {
                    Vec2::new(-40.0, 0.0)
                }
            } else if mode.ends_with("-join") {
                Vec2::ZERO
            } else if mode.ends_with("-offset") {
                Vec2::new(0.0, 50.0)
            } else if bar.vertical() {
                Vec2::new(0.0, 60.0)
            } else {
                Vec2::new(60.0, 0.0)
            };
            let point = if mode.ends_with("-free") {
                ctx.content_rect().center() + Vec2::new(0.0, 100.0)
            } else {
                anchor.rect.center() + offset
            };
            ctx.data_mut(|data| data.insert_temp(target_id, point));
        }
        if frame >= 510
            && let Some(pos) = ctx.data(|data| data.get_temp::<egui::Pos2>(target_id))
        {
            return pointer(pos, None);
        }
        return Vec::new();
    }
    if mode.starts_with("drawer-drop") || mode.starts_with("drawer-group") {
        if frame == 300 {
            super::drawers::set_open(ctx, "drawer:demo", mode.ends_with("-open"));
        }
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
        let target_id = egui::Id::unique("demo_drawer_icon_drop");
        if matches!(frame, 500 | 501)
            && let Some(source) = ctx.read_response(crate::ui::bars::tool_button_id("snapshot"))
        {
            if frame == 501 {
                ctx.data_mut(|data| data.insert_temp(target_id.with("source"), source.rect.center() + Vec2::new(12.0, 0.0)));
            }
            return pointer(
                source.rect.center() + if frame == 501 { Vec2::new(12.0, 0.0) } else { Vec2::ZERO },
                (frame == 500).then_some(true),
            );
        }
        if frame == 499 {
            let command = if mode.starts_with("drawer-group") {
                "people"
            } else {
                "drawer:demo"
            };
            if let Some(target) = ctx.read_response(crate::ui::bars::tool_button_id(command)) {
                let mut point = target.rect.center();
                if mode.starts_with("drawer-group-insert") {
                    if mode.ends_with("-side") {
                        point.y = target.rect.top() + target.rect.height() * 0.8;
                    } else {
                        point.x = target.rect.left() + target.rect.width() * 0.8;
                    }
                }
                ctx.data_mut(|data| data.insert_temp(target_id, point));
            }
        }
        if frame >= 502
            && let Some(point) = ctx.data(|data| data.get_temp::<egui::Pos2>(target_id))
        {
            let source = ctx
                .data(|data| data.get_temp::<egui::Pos2>(target_id.with("source")))
                .unwrap_or(point);
            let position = source.lerp(point, ((frame - 502) as f32 / 70.0).min(1.0));
            return pointer(position, None);
        }
        return Vec::new();
    }
    if mode.starts_with("interface-edit") || mode.starts_with("drawer-") {
        if frame == 300 {
            if mode.starts_with("interface-edit") || mode == "drawer-edit" {
                state.open = true;
                state.search.clear();
                state.category = None;
                state.advanced_catalogue = false;
            }
            if mode == "interface-edit-selected" {
                state.selected = Some("bar_clock".into());
            }
            if mode == "drawer-edit" {
                state.selected = Some("drawer:demo".into());
            }
            if mode.starts_with("drawer-") {
                super::drawers::set_open(ctx, "drawer:demo", true);
                if mode.starts_with("drawer-settings") {
                    state.drawer_editor = Some("drawer:demo".into());
                }
            }
        }
        if mode == "drawer-settings-icons-left" && matches!(frame, 350..=352) {
            let point_id = egui::Id::unique("demo_drawer_settings_drag");
            if frame == 350
                && let Some(rect) = ctx.memory(|memory| memory.area_rect(egui::Id::unique("toolbar_drawer_settings")))
            {
                ctx.data_mut(|data| data.insert_temp(point_id, rect.center_top() + Vec2::new(0.0, 18.0)));
            }
            if let Some(start) = ctx.data(|data| data.get_temp::<egui::Pos2>(point_id)) {
                let pos = start
                    + if frame == 350 {
                        Vec2::ZERO
                    } else {
                        Vec2::new(ctx.content_rect().width() * 0.4, 0.0)
                    };
                let mut events = vec![egui::Event::PointerMoved(pos)];
                if frame != 351 {
                    events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: frame == 350,
                        modifiers: Default::default(),
                    });
                }
                return events;
            }
        }
        if mode.starts_with("drawer-settings-icons-")
            && matches!(frame, 400 | 401)
            && let Some(rect) = ctx.data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_icon_button_rect", "drawer:demo"))))
        {
            let pos = rect.center();
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 400,
                    modifiers: Default::default(),
                },
            ];
        }
        return Vec::new();
    }
    if mode.starts_with("split-controls") {
        if mode == "split-controls-editor" && frame == 300 {
            state.open = true;
            state.search.clear();
            state.category = Some(super::catalogue_view::Category::Interface);
            state.advanced_catalogue = false;
        }
        return Vec::new();
    }
    if mode.starts_with("conversations-") {
        if mode.ends_with("overflow")
            && matches!(frame, 500 | 501)
            && let Some(response) = ctx.read_response(egui::Id::unique("conversation_thumbnails_more"))
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
    if mode.starts_with("catalogue-") {
        if frame == 300 {
            state.open = true;
            state.category = None;
            state.advanced_catalogue = mode == "catalogue-advanced";
            state.search = if mode == "catalogue-search" {
                aurora_i18n::tr!("toolbar-command-debug-settings").into()
            } else {
                String::new()
            };
        }
        return Vec::new();
    }
    if mode.starts_with("separators") {
        if mode == "separators-stretch-editor" && frame == 300 {
            state.edit_button(&super::structure::separator("demo-bottom"));
        }
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
    // With AURORA_DEMO_MENU_SEARCH set to the translated Import label,
    // right-click its real search result after the menu opens. Capture 750.
    if matches!(mode, "menu-add" | "menu-remove") {
        if matches!(frame, 650 | 651)
            && let Some(rect) = ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", "menu:common-build/import-menu"))))
                .filter(|rect| rect.is_positive())
        {
            let pos = rect.center();
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed: frame == 650,
                    modifiers: egui::Modifiers::NONE,
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
