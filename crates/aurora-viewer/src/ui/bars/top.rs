//! A fourth toolbar in the top row, sharing the existing button renderer,
//! menus, microphone, overflow scrolling and drag/drop machinery.

use super::{BarAction, ButtonLook, MicButton, Row, islands, natural_width, sizing, toolbar_row};
use crate::theme::Palette;
use crate::ui::{
    Panels,
    icons::Icons,
    skin::Layout,
    toolbar::{Bar, ToolbarSettings, ToolbarUi},
};
use egui::{Id, Rect, Vec2};

pub fn button(command: &str, icon: &str) -> crate::ui::skin::ToolButton {
    crate::ui::skin::ToolButton {
        command: command.into(),
        icon: icon.into(),
        tooltip: String::new(),
    }
}

pub fn width(ui: &egui::Ui, settings: &ToolbarSettings, button: &crate::ui::skin::ToolButton) -> f32 {
    natural_width(
        ui.ctx(),
        button,
        settings.top_style.resolve(Bar::Top, true).mode,
        islands::SLOT,
        islands::ICON,
        38.0,
    )
}

/// Native top-row controls keep their anchors while sharing button appearance.
pub fn native(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    settings: &ToolbarSettings,
    button: &crate::ui::skin::ToolButton,
    rect: Rect,
    id: Id,
) -> egui::Response {
    native_styled(ui, p, icons, button, rect, id, settings.top_style.resolve(Bar::Top, true).mode)
}

pub fn native_styled(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    button: &crate::ui::skin::ToolButton,
    rect: Rect,
    id: Id,
    mode: crate::ui::toolbar::ButtonMode,
) -> egui::Response {
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    islands::slot_face(ui, p, rect, response.hovered() || open, false);
    let appearance = crate::ui::toolbar::appearance::get(ui.ctx(), &button.command);
    let look = ButtonLook {
        size: rect.size(),
        icon: islands::ICON,
        dock: true,
        mode: appearance
            .mode
            .or(crate::ui::toolbar::blocks::saved_style(ui.ctx(), &button.command).mode)
            .unwrap_or(mode),
        left: false,
        opens: egui::RectAlign::BOTTOM_START,
        editing: false,
    };
    let label = crate::ui::toolbar::label(button);
    let (icon, text) = super::content(ui, rect, look, label);
    if let Some(rect) = icon {
        super::paint_icon(
            ui,
            icons,
            appearance.icon.as_deref().unwrap_or(&button.icon),
            rect,
            islands::ICON,
            p.ink,
        );
    }
    if let Some((left, room)) = text {
        crate::ui::widgets::paint_elided(
            ui,
            label,
            egui::pos2(left, rect.center().y),
            room,
            egui::FontId::proportional(super::sizing::LABEL_FONT),
            p.ink,
        );
    }
    response.on_hover_text(label)
}

fn rect_id() -> Id {
    Id::unique("custom_top_toolbar_rect")
}

pub fn reserve(ui: &egui::Ui, settings: &ToolbarSettings, mut span: Rect, minimum_place: f32) -> Rect {
    let style = settings.top_style.resolve(Bar::Top, true);
    let dragging = ToolbarUi::dragging(ui.ctx()) && !settings.locked;
    let natural: Vec<_> = settings
        .top
        .iter()
        .map(|button| natural_width(ui.ctx(), button, style.mode, islands::SLOT, islands::ICON, 38.0))
        .collect();
    let room = (span.width()
        - if settings.address_visible.unwrap_or(true) {
            minimum_place + islands::MARGIN
        } else {
            0.0
        })
    .max(36.0);
    let widths = sizing::toolbar(
        ui.ctx(),
        &settings.top,
        &natural,
        style.sizing,
        style.mode != crate::ui::toolbar::ButtonMode::IconsOnly,
        room - islands::PAD * 2.0,
        islands::GAP,
        islands::SLOT,
    );
    let width = if settings.top.is_empty() && !dragging && !ToolbarUi::editing(ui.ctx()) {
        0.0
    } else if dragging
        || ToolbarUi::editing(ui.ctx())
        || style.sizing == crate::ui::toolbar::Sizing::Fill
        || sizing::has_alignment(ui.ctx(), &settings.top)
    {
        // LLToolBarView::draw exposes the whole destination during a drag.
        // The compact island alone leaves no room to append another button.
        room
    } else {
        (widths.iter().sum::<f32>() + islands::GAP * settings.top.len().saturating_sub(1) as f32 + islands::PAD * 2.0)
            .max(36.0)
            .min(room)
    };
    let rect = Rect::from_min_size(span.min, Vec2::new(width, span.height()));
    ui.ctx().data_mut(|data| data.insert_temp(rect_id(), rect));
    if width > 0.0 {
        span.min.x += width + islands::MARGIN;
    }
    span
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    _layout: &Layout,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
    panels: &mut Panels,
    mic: &mut MicButton,
    base: &Row,
    classic: bool,
) -> BarAction {
    if settings.composed {
        return composed(ui, p, icons, toolbar, settings, panels, mic, base, classic);
    }
    let dragging = ToolbarUi::dragging(ui.ctx()) && !settings.locked;
    if settings.top.is_empty() && !dragging && !ToolbarUi::editing(ui.ctx()) {
        return BarAction::None;
    }
    let style = settings.top_style.resolve(Bar::Top, !classic);
    let display = crate::ui::toolbar::structure::Display::for_bar(ui.ctx(), Bar::Top, &settings.top, base.editing);
    let buttons = display.buttons();
    let row = Row {
        bar: Bar::Top,
        style,
        ..*base
    };
    let mut action = BarAction::None;
    let mut body = |ui: &mut egui::Ui| {
        let zone = ui.max_rect();
        crate::ui::toolbar::edit_mode::region(ui, p, Bar::Top, zone, buttons.is_empty());
        let target = ui.interact(zone, Id::unique("top_toolbar_background"), egui::Sense::click());
        toolbar.context(&target, p, Bar::Top, style, None, settings.locked);
        let rects = egui::ScrollArea::horizontal()
            .id_salt("top_toolbar_scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = islands::GAP;
                    let natural: Vec<_> = buttons
                        .iter()
                        .map(|button| natural_width(ui.ctx(), button, style.mode, islands::SLOT, islands::ICON, 38.0))
                        .collect();
                    let mut widths = sizing::toolbar(
                        ui.ctx(),
                        buttons,
                        &natural,
                        style.sizing,
                        style.mode != crate::ui::toolbar::ButtonMode::IconsOnly,
                        zone.width(),
                        islands::GAP,
                        islands::SLOT,
                    );
                    let aligned = sizing::alignment(ui.ctx(), buttons, &mut widths, zone.width(), islands::GAP, style);
                    let total = widths.iter().sum::<f32>() + islands::GAP * buttons.len().saturating_sub(1) as f32;
                    // Expanding the destination must not move existing compact
                    // buttons away from the pointer that began the drag.
                    let aligned_width = if !classic && dragging && style.sizing != crate::ui::toolbar::Sizing::Fill {
                        total.min(zone.width())
                    } else {
                        zone.width()
                    };
                    ui.add_space(aligned.unwrap_or_else(|| sizing::place(style.align, 0.0, aligned_width, total)));
                    let look = |button: &crate::ui::skin::ToolButton| {
                        let index = buttons
                            .iter()
                            .position(|candidate| candidate.command == button.command)
                            .unwrap_or(0);
                        ButtonLook {
                            size: Vec2::new(widths.get(index).copied().unwrap_or(islands::SLOT), islands::SLOT),
                            icon: islands::ICON,
                            dock: true,
                            mode: style.mode,
                            left: false,
                            opens: egui::RectAlign::BOTTOM_START,
                            editing: row.editing,
                        }
                    };
                    let rects = toolbar_row(ui, p, icons, buttons, look, toolbar, panels, mic, &row, &mut action);
                    if dragging {
                        let left = rects.last().map_or(zone.left(), |rect| rect.right() + islands::MARGIN);
                        crate::ui::widgets::paint_elided(
                            ui,
                            aurora_i18n::tr!("toolbar-drop-here"),
                            egui::pos2(left, zone.center().y),
                            (zone.right() - left).max(0.0),
                            egui::FontId::proportional(super::sizing::LABEL_FONT),
                            p.muted,
                        );
                    }
                    rects
                })
                .inner
            })
            .inner;
        // Handle the destination outside the scroller: its content clip must
        // not exclude blank space or the island's padding from a menu drop.
        let drop_zone = if classic { zone } else { zone.expand(islands::PAD) };
        let drop = ui.interact(drop_zone, Id::unique("top_toolbar_drop"), egui::Sense::hover());
        toolbar.drop_bar(&drop, Bar::Top, display.original(), &display.full_rects(&rects), settings.locked);
    };
    if classic {
        let shown = egui::Panel::top("custom_top_toolbar")
            .frame(if base.editing {
                super::bar_frame(p).fill(p.bar.gamma_multiply(0.0))
            } else {
                super::bar_frame(p)
            })
            .exact_size(36.0)
            .show(ui, |ui| {
                let painter = ui.painter().clone();
                let slot = painter.add(egui::Shape::Noop);
                body(ui);
                if !base.editing {
                    crate::ui::widgets::finish_bar_glass(&painter, slot, ui.max_rect(), p);
                }
            });
        let _ = shown;
    } else {
        let ctx = ui.ctx().clone();
        if let Some(rect) = ctx.data(|data| data.get_temp::<Rect>(rect_id())) {
            islands::area_surface(
                &ctx,
                p,
                "custom_top_toolbar",
                rect,
                !base.editing && !sizing::has_alignment(&ctx, buttons),
                |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(islands::PAD)));
                    body(&mut child);
                },
            );
        }
    }
    action
}

#[allow(clippy::too_many_arguments)]
fn composed(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
    panels: &mut Panels,
    mic: &mut MicButton,
    base: &Row,
    classic: bool,
) -> BarAction {
    let style = settings.top_style.resolve(Bar::Top, !classic);
    let row = Row {
        bar: Bar::Top,
        style,
        ..*base
    };
    let display = crate::ui::toolbar::structure::Display::for_bar(ui.ctx(), Bar::Top, &settings.top, row.editing);
    let buttons = display.buttons();
    let mut action = BarAction::None;
    let height = buttons
        .iter()
        .filter(|button| crate::ui::toolbar::structure::is_block(&button.command))
        .map(|button| super::chrome::height(ui.ctx(), &button.command))
        .fold(islands::HEIGHT, f32::max);
    let split = buttons
        .iter()
        .any(|button| crate::ui::toolbar::structure::is_separator(&button.command))
        || sizing::has_alignment(ui.ctx(), buttons);
    let mut body = |ui: &mut egui::Ui, rect: Rect| {
        crate::ui::toolbar::edit_mode::region(ui, p, Bar::Top, rect, buttons.is_empty());
        let target = ui.interact(rect, Id::unique("top_toolbar_background"), egui::Sense::click());
        toolbar.context(&target, p, Bar::Top, style, None, settings.locked);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(islands::PAD, 0.0))));
        child.set_clip_rect(rect);
        let rects = egui::ScrollArea::horizontal()
            .id_salt("composed_top_toolbar_scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = islands::GAP;
                    let natural: Vec<_> = buttons
                        .iter()
                        .map(|button| natural_width(ui.ctx(), button, style.mode, islands::SLOT, islands::ICON, 38.0))
                        .collect();
                    let mut widths = sizing::toolbar(
                        ui.ctx(),
                        buttons,
                        &natural,
                        style.sizing,
                        style.mode != crate::ui::toolbar::ButtonMode::IconsOnly,
                        rect.width() - islands::PAD * 2.0,
                        islands::GAP,
                        islands::SLOT,
                    );
                    let total = widths.iter().sum::<f32>() + islands::GAP * buttons.len().saturating_sub(1) as f32;
                    let aligned = sizing::alignment(
                        ui.ctx(),
                        buttons,
                        &mut widths,
                        rect.width() - islands::PAD * 2.0,
                        islands::GAP,
                        style,
                    );
                    if aligned.is_none()
                        && let Some(index) = buttons.iter().position(|button| button.command == "bar_navigation")
                        && widths[index] > 0.0
                    {
                        widths[index] += (rect.width() - islands::PAD * 2.0 - total).max(0.0);
                    }
                    let total = widths.iter().sum::<f32>() + islands::GAP * buttons.len().saturating_sub(1) as f32;
                    ui.add_space(aligned.unwrap_or_else(|| sizing::place(style.align, 0.0, rect.width() - islands::PAD * 2.0, total)));
                    let look = |button: &crate::ui::skin::ToolButton| {
                        let index = buttons
                            .iter()
                            .position(|candidate| candidate.command == button.command)
                            .unwrap_or(0);
                        ButtonLook {
                            size: Vec2::new(widths.get(index).copied().unwrap_or(islands::SLOT), islands::SLOT),
                            icon: islands::ICON,
                            dock: !classic,
                            mode: style.mode,
                            left: false,
                            opens: egui::RectAlign::BOTTOM_START,
                            editing: row.editing,
                        }
                    };
                    toolbar_row(ui, p, icons, buttons, look, toolbar, panels, mic, &row, &mut action)
                })
                .inner
            })
            .inner;
        let drop = ui.interact(rect, Id::unique("top_toolbar_drop"), egui::Sense::hover());
        toolbar.drop_bar(&drop, Bar::Top, display.original(), &display.full_rects(&rects), settings.locked);
    };
    if classic {
        egui::Panel::top("composed_top_toolbar")
            .frame(
                (if row.editing {
                    super::bar_frame(p).fill(p.bar.gamma_multiply(0.0))
                } else {
                    super::bar_frame(p)
                })
                .inner_margin(egui::Margin::ZERO),
            )
            .exact_size(height)
            .show(ui, |ui| {
                body(ui, ui.max_rect());
            });
    } else {
        let screen = ui.ctx().content_rect();
        let rect = Rect::from_min_size(
            screen.min + Vec2::splat(islands::MARGIN),
            Vec2::new(screen.width() - islands::MARGIN * 2.0, height),
        );
        let ctx = ui.ctx().clone();
        ctx.data_mut(|data| data.insert_temp(rect_id(), rect));
        islands::area_surface(&ctx, p, "composed_top_toolbar", rect, !split && !row.editing, |ui| body(ui, rect));
    }
    action
}
