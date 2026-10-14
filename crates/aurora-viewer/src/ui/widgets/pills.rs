//! The tabs, filter pills and search box of the redrawn windows (group
//! profile, About Land, People): first written for one window each, shared
//! here once a second window needed them.

use crate::theme::Palette;
use egui::{FontId, Pos2, Rect, Sense, Vec2};

/// One tab of a row of tabs in pills: an optional Tabler icon, its name, a
/// count after it (in a violet pill when `accent`, an unread count). The
/// selected tab sits on `raised`.
#[allow(clippy::too_many_arguments)]
pub fn pill_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    count: Option<String>,
    accent: bool,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let font = FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let ink = if !enabled {
        p.muted_dim
    } else if selected {
        p.ink
    } else {
        p.muted
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let count_ink = if accent { p.on_accent } else { p.muted_dim };
    let count = count.map(|t| {
        ui.painter()
            .layout_no_wrap(t, FontId::new(11.0, crate::ui::fonts::semibold(ui.ctx())), count_ink)
    });
    let pad = if accent { 5.0 } else { 0.0 };
    let count_w = count.as_ref().map_or(0.0, |g| 5.0 + g.size().x + pad * 2.0);
    let icon_w = if icon.is_empty() { 0.0 } else { 15.0 + 5.0 };
    let size = Vec2::new(12.0 + icon_w + galley.size().x + count_w + 12.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.raised);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let mut x = rect.left() + 12.0;
    if !icon.is_empty() {
        if let Some(t) = crate::ui::icons::global(icon) {
            let r = Rect::from_center_size(Pos2::new(x + 7.5, rect.center().y), Vec2::splat(15.0));
            ui.painter()
                .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), ink);
        }
        x += icon_w;
    }
    let w = galley.size().x;
    ui.painter()
        .galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0).round(), galley, ink);
    if let Some(g) = count {
        let pos = Pos2::new(x + w + 5.0 + pad, rect.center().y - g.size().y / 2.0);
        if accent {
            let pill = Rect::from_min_size(
                Pos2::new(x + w + 5.0, rect.center().y - 8.0),
                Vec2::new(g.size().x + pad * 2.0, 16.0),
            );
            ui.painter().rect_filled(pill, ui.visuals().window_corner_radius, p.violet);
        }
        ui.painter().galley(pos.round(), g, count_ink);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, selected, label));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp.on_hover_text(crate::ui::menu::not_yet())
    }
}

/// A filter or sort pill of a list's bar: lit in the accent while `on`,
/// with an optional Tabler icon before its label.
pub fn filter_pill(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(filter_pill_width(ui, icon, label), 26.0), Sense::click());
    paint_filter_pill(ui, p, rect, &resp, icon, label, on);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// `filter_pill` at a place of its own (a row's right end laid out apart).
pub fn filter_pill_at(ui: &mut egui::Ui, p: &Palette, rect: Rect, id: egui::Id, icon: &str, label: &str, on: bool) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    paint_filter_pill(ui, p, rect, &resp, icon, label, on);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_filter_pill(ui: &egui::Ui, p: &Palette, rect: Rect, resp: &egui::Response, icon: &str, label: &str, on: bool) {
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let ink = if on { p.ink } else { p.muted };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let icon_w = if icon.is_empty() { 0.0 } else { 13.0 + 5.0 };
    let radius = ui.visuals().window_corner_radius;
    let fill = if on {
        p.violet.gamma_multiply(0.28)
    } else if resp.hovered() {
        p.raised
    } else {
        p.bar
    };
    ui.painter().rect_filled(rect, radius, fill);
    if on {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    let mut x = rect.left() + 9.0;
    if !icon.is_empty() {
        if let Some(t) = crate::ui::icons::global(icon) {
            let r = Rect::from_center_size(Pos2::new(x + 6.5, rect.center().y), Vec2::splat(13.0));
            ui.painter()
                .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), ink);
        }
        x += icon_w;
    }
    ui.painter()
        .galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0).round(), galley, ink);
}

/// The width `dot_pill` takes.
pub fn dot_pill_width(ui: &egui::Ui, label: &str, count: Option<&str>, close: bool) -> f32 {
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let w = |t: &str| {
        ui.painter()
            .layout_no_wrap(t.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    9.0 + 9.0 + 6.0 + w(label) + count.map_or(0.0, |c| 5.0 + w(c)) + if close { 5.0 + 12.0 } else { 0.0 } + 9.0
}

/// A filter pill led by a dot of `dot` (a contact set's color), with a
/// count after its label, or a cross that takes it away (`close`). Returns
/// the pill's response and whether its cross was clicked.
pub fn dot_pill(
    ui: &mut egui::Ui,
    p: &Palette,
    dot: egui::Color32,
    label: &str,
    count: Option<&str>,
    on: bool,
    close: bool,
) -> (egui::Response, bool) {
    let width = dot_pill_width(ui, label, count, close);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    let fill = if on {
        p.violet.gamma_multiply(0.28)
    } else if resp.hovered() {
        p.raised
    } else {
        p.bar
    };
    ui.painter().rect_filled(rect, radius, fill);
    if on {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    let ink = if on || close { p.ink } else { p.muted };
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let mut x = rect.left() + 9.0;
    ui.painter().circle_filled(Pos2::new(x + 4.5, rect.center().y), 4.5, dot);
    x += 9.0 + 6.0;
    let g = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), ink);
    let w = g.size().x;
    ui.painter()
        .galley(Pos2::new(x, rect.center().y - g.size().y / 2.0).round(), g, ink);
    x += w;
    if let Some(c) = count {
        let g = ui.painter().layout_no_wrap(c.to_owned(), font, ink);
        ui.painter()
            .galley(Pos2::new(x + 5.0, rect.center().y - g.size().y / 2.0).round(), g, ink);
    }
    let mut closed = false;
    if close {
        let cross = Rect::from_center_size(Pos2::new(rect.right() - 9.0 - 6.0, rect.center().y), Vec2::splat(16.0));
        let c = ui.interact(cross, resp.id.with("close"), Sense::click());
        let tint = if c.hovered() { p.ink } else { p.muted };
        if let Some(t) = crate::ui::icons::global("x") {
            ui.painter().image(
                t.id(),
                Rect::from_center_size(cross.center(), Vec2::splat(12.0)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        closed = c.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    }
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), closed)
}

/// The width `filter_pill` takes.
pub fn filter_pill_width(ui: &egui::Ui, icon: &str, label: &str) -> f32 {
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x;
    (if icon.is_empty() { 0.0 } else { 13.0 + 5.0 }) + w + 18.0
}

/// The search field of a list: a magnifier then the text, on the field's
/// face.
pub fn search_box(ui: &mut egui::Ui, p: &Palette, text: &mut String, hint: &str, width: f32) -> egui::Response {
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.set_width(width - 18.0);
            // left to right even in a row laid out from the right
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                super::glyph(ui, "search-filled", p.muted, 13.0);
                ui.add(
                    egui::TextEdit::singleline(text)
                        .frame(egui::Frame::NONE)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                )
            })
            .inner
        })
        .inner
}
