//! The tabs in pills of the redrawn windows (group profile, Controls):
//! first written for one window, shared here once a second one needed them.

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
    tab(ui, p, icon, label, count, accent, selected, enabled, None)
}

/// A `pill_tab` of the given width, its icon and name in the middle (the
/// two halves of the Controls window).
pub fn pill_tab_wide(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, selected: bool, width: f32) -> egui::Response {
    tab(ui, p, icon, label, None, false, selected, true, Some(width))
}

#[allow(clippy::too_many_arguments)]
fn tab(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    count: Option<String>,
    accent: bool,
    selected: bool,
    enabled: bool,
    width: Option<f32>,
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
    let content = icon_w + galley.size().x + count_w;
    let size = Vec2::new(width.unwrap_or(12.0 + content + 12.0), 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.raised);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let mut x = match width {
        Some(_) => rect.center().x - content / 2.0,
        None => rect.left() + 12.0,
    };
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
