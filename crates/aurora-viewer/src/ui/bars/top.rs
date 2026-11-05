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
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    islands::slot_face(ui, p, rect, response.hovered() || open, false);
    let appearance = crate::ui::toolbar::appearance::get(ui.ctx(), &button.command);
    let look = ButtonLook {
        size: rect.size(),
        icon: islands::ICON,
        dock: true,
        mode: appearance.mode.unwrap_or(settings.top_style.resolve(Bar::Top, true).mode),
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

pub fn reserve(ui: &egui::Ui, settings: &ToolbarSettings, mut span: Rect) -> Rect {
    let style = settings.top_style.resolve(Bar::Top, true);
    let widths: f32 = settings
        .top
        .iter()
        .map(|button| natural_width(ui.ctx(), button, style.mode, islands::SLOT, islands::ICON, 38.0))
        .sum();
    let width = if settings.top.is_empty() && !ToolbarUi::dragging(ui.ctx()) {
        0.0
    } else {
        (widths + islands::GAP * settings.top.len().saturating_sub(1) as f32 + islands::PAD * 2.0)
            .max(36.0)
            .min((span.width() - 80.0).max(36.0))
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
    if settings.top.is_empty() && !ToolbarUi::dragging(ui.ctx()) {
        return BarAction::None;
    }
    let style = settings.top_style.resolve(Bar::Top, !classic);
    let row = Row {
        bar: Bar::Top,
        style,
        ..*base
    };
    let mut action = BarAction::None;
    let mut body = |ui: &mut egui::Ui| {
        let zone = ui.max_rect();
        let target = ui.interact(zone, Id::unique("top_toolbar_background"), egui::Sense::click());
        toolbar.context(&target, p, Bar::Top, style, None, settings.locked);
        egui::ScrollArea::horizontal()
            .id_salt("top_toolbar_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = islands::GAP;
                    let natural: Vec<_> = settings
                        .top
                        .iter()
                        .map(|button| natural_width(ui.ctx(), button, style.mode, islands::SLOT, islands::ICON, 38.0))
                        .collect();
                    let widths = sizing::fit(&natural, style.sizing, true, zone.width(), islands::GAP, islands::SLOT);
                    let total = widths.iter().sum::<f32>() + islands::GAP * settings.top.len().saturating_sub(1) as f32;
                    ui.add_space(sizing::place(style.align, 0.0, zone.width(), total));
                    let look = |button: &crate::ui::skin::ToolButton| {
                        let index = settings
                            .top
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
                    let rects = toolbar_row(ui, p, icons, &settings.top, look, toolbar, panels, mic, &row, &mut action);
                    let drop = ui.interact(zone, Id::unique("top_toolbar_drop"), egui::Sense::hover());
                    toolbar.drop_bar(&drop, Bar::Top, &rects, p, settings.locked);
                });
            });
    };
    if classic {
        let shown = egui::Panel::top("custom_top_toolbar")
            .frame(super::bar_frame(p))
            .exact_size(36.0)
            .show(ui, |ui| {
                let painter = ui.painter().clone();
                let slot = painter.add(egui::Shape::Noop);
                body(ui);
                crate::ui::widgets::finish_bar_glass(&painter, slot, ui.max_rect(), p);
            });
        let _ = shown;
    } else {
        let ctx = ui.ctx().clone();
        if let Some(rect) = ctx.data(|data| data.get_temp::<Rect>(rect_id())) {
            islands::area(&ctx, p, "custom_top_toolbar", rect, |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(islands::PAD)));
                body(&mut child);
            });
        }
    }
    action
}
