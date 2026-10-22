//! The resource monitor's drawing pieces: cards, graphs over the minute,
//! segmented bars and their legends, meters, sortable table headers and
//! the cells of its tables, all in the theme's colours.

use crate::theme::Palette;
use crate::ui::widgets;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};

pub const ROW_H: f32 = 28.0;
pub const SMALL: f32 = 11.0;
pub const BODY: f32 = 12.0;

/// The colours of the parts of a breakdown, as the compact view's
/// (`perf::part_colors`): neighbours stay apart for colour-blind readers,
/// the seventh (neutral) is the rest; the eighth only serves breakdowns
/// with eight parts (the cache by folder).
pub fn part_colors(p: &Palette) -> [Color32; 8] {
    [
        p.indigo_light,
        p.rose,
        p.amber,
        p.violet,
        p.teal,
        p.violet_pale,
        p.muted_dim,
        p.success,
    ]
}

/// A cell's tint for `frac` of its column's largest value.
pub fn heat(p: &Palette, frac: f64) -> Color32 {
    p.violet.gamma_multiply(0.06 + 0.30 * frac.clamp(0.0, 1.0) as f32)
}

/// A surface inside the window: the bars' colour, a thin border, the
/// theme's corners.
pub fn card<R>(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(p.bar)
        .stroke(Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
}

/// A card's title row: the title in ink, a note on the right in muted.
pub fn card_title(ui: &mut egui::Ui, p: &Palette, title: &str, right: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).size(13.0).strong().color(p.ink));
        if !right.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(right).size(BODY).color(p.muted)).truncate());
            });
        }
    });
    ui.add_space(4.0);
}

/// A text in the accent that does something when clicked.
pub fn link(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(BODY), p.violet_light);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let color = if resp.hovered() { p.violet_pale } else { p.violet_light };
    ui.painter().galley(rect.min, galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A label of a given size and colour.
pub fn text(ui: &mut egui::Ui, s: impl Into<String>, size: f32, color: Color32) -> egui::Response {
    ui.label(egui::RichText::new(s).size(size).color(color))
}

/// A line of one value over the minute (oldest first): its points.
fn points(values: &[f64], rect: Rect, max: f64, slots: usize) -> Vec<Pos2> {
    let n = values.len();
    if n == 0 {
        return Vec::new();
    }
    let step = rect.width() / (slots.max(2) - 1) as f32;
    let x0 = rect.right() - (n - 1) as f32 * step;
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let f = if max > 0.0 { (v / max).clamp(0.0, 1.0) as f32 } else { 0.0 };
            pos2(x0 + i as f32 * step, rect.bottom() - 1.0 - f * (rect.height() - 3.0))
        })
        .collect()
}

/// One line of a graph: its values, colour, and whether the area under it
/// is filled.
pub struct Line<'a> {
    pub values: &'a [f64],
    pub color: Color32,
    pub fill: bool,
}

/// A graph of the minute in `size`: the lines over a field with a dashed
/// middle mark, scaled to `max`; under the pointer, a cursor and `hover`'s
/// text for that point (its index in the values).
pub fn graph(
    ui: &mut egui::Ui,
    p: &Palette,
    size: Vec2,
    lines: &[Line],
    max: f64,
    slots: usize,
    hover: Option<&dyn Fn(usize) -> String>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    let y = (rect.center().y).round() + 0.5;
    painter.extend(egui::Shape::dashed_line(
        &[pos2(rect.left(), y), pos2(rect.right(), y)],
        Stroke::new(1.0, p.raised),
        3.0,
        3.0,
    ));
    let mut first: Option<Vec<Pos2>> = None;
    for line in lines {
        let pts = points(line.values, rect, max, slots);
        if pts.len() < 2 {
            continue;
        }
        if line.fill {
            let fill = line.color.gamma_multiply(0.14);
            let mut mesh = egui::Mesh::default();
            for (i, pt) in pts.iter().enumerate() {
                mesh.colored_vertex(*pt, fill);
                mesh.colored_vertex(pos2(pt.x, rect.bottom()), fill);
                if i > 0 {
                    let k = i as u32 * 2;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k + 1, k);
                }
            }
            painter.add(mesh);
        }
        painter.add(egui::Shape::line(pts.clone(), Stroke::new(1.5, line.color)));
        first.get_or_insert(pts);
    }
    if let (Some(pos), Some(pts), Some(hover)) = (resp.hover_pos(), first, hover) {
        let i = pts
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.x - pos.x).abs().total_cmp(&(b.1.x - pos.x).abs()))
            .map_or(0, |(i, _)| i);
        let x = pts[i].x;
        painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, p.muted_dim));
        let galley = painter.layout_no_wrap(hover(i), egui::FontId::proportional(SMALL), p.ink);
        let box_size = galley.size() + vec2(10.0, 4.0);
        let bx = if x + 6.0 + box_size.x > rect.right() {
            x - 6.0 - box_size.x
        } else {
            x + 6.0
        };
        let r = Rect::from_min_size(pos2(bx.max(rect.left()), rect.top() + 2.0), box_size);
        painter.rect_filled(r, ui.visuals().window_corner_radius, p.panel);
        painter.galley(r.min + vec2(5.0, 2.0), galley, p.ink);
    }
    resp
}

/// A round top for a scale: 1, 2, 2.5 or 5 times a power of ten, at least
/// `v` and never 0.
pub fn nice_max(v: f64) -> f64 {
    let v = v.max(1e-6);
    let pow = 10f64.powf(v.log10().floor());
    for s in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if s * pow >= v {
            return s * pow;
        }
    }
    10.0 * pow
}

/// One part of a segmented bar: its value and colour.
pub struct Segment {
    pub value: f64,
    pub color: Color32,
}

/// A bar `height` tall split in segments, in order, over `total` (the sum
/// when 0): the parts touch with a 2 px gap, rounded at the two ends.
pub fn segments(ui: &mut egui::Ui, p: &Palette, height: f32, parts: &[Segment], total: f64) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    let radius = ui.visuals().window_corner_radius;
    painter.rect_filled(rect, radius, p.field);
    let sum: f64 = parts.iter().map(|s| s.value.max(0.0)).sum();
    let total = if total > 0.0 { total.max(sum) } else { sum };
    if total <= 0.0 {
        return resp;
    }
    let wide: Vec<bool> = parts.iter().map(|s| s.value / total * f64::from(rect.width()) >= 0.5).collect();
    let first = wide.iter().position(|w| *w).unwrap_or(0);
    let last = wide.iter().rposition(|w| *w).unwrap_or(0);
    let mut x = rect.left();
    for (i, s) in parts.iter().enumerate() {
        let w = (s.value.max(0.0) / total * f64::from(rect.width())) as f32;
        let (l, r) = (x, x + w);
        x = r;
        if !wide[i] {
            continue;
        }
        let l = if i > first { l + 1.0 } else { l };
        let r = if i < last { r - 1.0 } else { r };
        if r - l < 0.5 {
            continue;
        }
        let corners = egui::CornerRadius {
            nw: if i == first { radius.nw } else { 0 },
            sw: if i == first { radius.sw } else { 0 },
            ne: if i == last && r >= rect.right() - 0.5 { radius.ne } else { 0 },
            se: if i == last && r >= rect.right() - 0.5 { radius.se } else { 0 },
        };
        painter.rect_filled(Rect::from_min_max(pos2(l, rect.top()), pos2(r, rect.bottom())), corners, s.color);
    }
    resp
}

/// A legend in `columns`: a dot of each part's colour, its name, its value.
pub fn legend(ui: &mut egui::Ui, p: &Palette, columns: usize, items: &[(Color32, String, String)]) {
    let columns = columns.max(1);
    let gap = 16.0;
    let width = ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(40.0);
    for row in items.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (color, name, value) in row {
                let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
                let painter = ui.painter();
                let cy = rect.center().y;
                painter.circle_filled(pos2(rect.left() + 4.0, cy), 3.5, *color);
                let font = egui::FontId::proportional(BODY);
                let vg = painter.layout_no_wrap(value.clone(), font.clone(), p.ink);
                let vx = rect.right() - vg.size().x;
                widgets::paint_row_text(painter, vg, vx, cy, p.ink);
                let room = (vx - rect.left() - 18.0).max(10.0);
                if widgets::paint_elided(ui, name, pos2(rect.left() + 12.0, cy), room, font, p.muted) {
                    resp.on_hover_text(name.as_str());
                }
            }
        });
    }
}

/// A thin meter: `frac` of `rect` in `color`.
pub fn paint_meter(ui: &egui::Ui, p: &Palette, rect: Rect, frac: f32, color: Color32) {
    let radius = widgets::inner_radius(ui.visuals());
    ui.painter().rect_filled(rect, radius, p.field);
    let w = rect.width() * frac.clamp(0.0, 1.0);
    if w >= 1.0 {
        ui.painter()
            .rect_filled(Rect::from_min_size(rect.min, vec2(w, rect.height())), radius, color);
    }
}

/// A table's column: its title, its width (the first column takes what is
/// left), its value on the right.
pub struct Col<'a> {
    pub title: &'a str,
    pub width: f32,
    pub right: bool,
    /// A total above the title (the subsystems' table).
    pub total: Option<String>,
    /// The column can sort the table; it does now.
    pub sortable: bool,
    pub sorted: bool,
}

impl<'a> Col<'a> {
    pub fn new(title: &'a str, width: f32, right: bool) -> Col<'a> {
        Col {
            title,
            width,
            right,
            total: None,
            sortable: false,
            sorted: false,
        }
    }

    pub fn sort(mut self, sorted: bool) -> Self {
        self.sortable = true;
        self.sorted = sorted;
        self
    }

    pub fn total(mut self, total: String) -> Self {
        self.total = Some(total);
        self
    }
}

/// The x ranges of the columns over `width`.
pub fn col_ranges(cols: &[Col], left: f32, width: f32) -> Vec<(f32, f32)> {
    let fixed: f32 = cols.iter().skip(1).map(|c| c.width).sum();
    let first = (width - fixed).max(60.0);
    let mut x = left;
    cols.iter()
        .enumerate()
        .map(|(i, c)| {
            let w = if i == 0 { first } else { c.width };
            let r = (x, x + w);
            x += w;
            r
        })
        .collect()
}

/// A table's header: the titles (and totals) in muted, the sorted one in
/// ink with a caret; returns the column clicked.
pub fn header(ui: &mut egui::Ui, p: &Palette, cols: &[Col]) -> Option<usize> {
    let tall = cols.iter().any(|c| c.total.is_some());
    let h = if tall { 40.0 } else { 26.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let ranges = col_ranges(cols, rect.left(), rect.width());
    let mut clicked = None;
    for (i, (c, (l, r))) in cols.iter().zip(&ranges).enumerate() {
        let cell = Rect::from_min_max(pos2(*l, rect.top()), pos2(*r, rect.bottom()));
        let resp = if c.sortable {
            Some(ui.interact(cell, ui.scope_id().with(("monitor-col", i)), Sense::click()))
        } else {
            None
        };
        let hot = resp.as_ref().is_some_and(egui::Response::hovered);
        let color = if c.sorted || hot { p.ink } else { p.muted };
        let painter = ui.painter();
        let pad = 10.0;
        let font = egui::FontId::proportional(SMALL);
        let title = painter.layout_no_wrap(c.title.to_owned(), font, color);
        let caret = if c.sorted { 12.0 } else { 0.0 };
        let tw = title.size().x + caret;
        let tx = if c.right { cell.right() - pad - tw } else { cell.left() + pad };
        let ty = cell.bottom() - 13.0;
        widgets::paint_row_text(painter, title, tx, ty, color);
        if c.sorted {
            widgets::paint_glyph(painter, "chevron-down", pos2(tx + tw - 5.0, ty), 11.0, color);
        }
        if let Some(total) = &c.total {
            let g = painter.layout_no_wrap(total.clone(), egui::FontId::proportional(15.0), p.ink);
            let gx = if c.right {
                cell.right() - pad - g.size().x
            } else {
                cell.left() + pad
            };
            let galley_h = g.size().y;
            painter.galley(pos2(gx, ty - 8.0 - galley_h), g, p.ink);
        }
        if let Some(resp) = resp
            && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
        {
            clicked = Some(i);
        }
    }
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.raised));
    clicked
}

/// Paints a cell's text in its column, cut short to fit.
pub fn cell_text(ui: &egui::Ui, range: (f32, f32), cy: f32, s: &str, right: bool, color: Color32, size: f32) {
    let pad = 10.0;
    let font = egui::FontId::proportional(size);
    if right {
        let g = ui.painter().layout_no_wrap(s.to_owned(), font, color);
        let x = range.1 - pad - g.size().x;
        widgets::paint_row_text(ui.painter(), g, x.max(range.0 + 2.0), cy, color);
    } else {
        widgets::paint_elided(ui, s, pos2(range.0 + pad, cy), range.1 - range.0 - pad * 2.0, font, color);
    }
}

/// A table row's surface: selected, hovered or plain, and its divider.
pub fn row_surface(ui: &egui::Ui, p: &Palette, rect: Rect, selected: bool, hovered: bool) {
    if selected {
        ui.painter()
            .rect_filled(rect, ui.visuals().window_corner_radius, p.indigo.gamma_multiply(0.45));
    } else if hovered {
        ui.painter()
            .rect_filled(rect, ui.visuals().window_corner_radius, p.raised.gamma_multiply(0.45));
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.raised.gamma_multiply(0.6)));
}

/// Name and value, the name muted: one row of a list of figures.
pub fn kv_row(ui: &mut egui::Ui, p: &Palette, k: &str, v: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let cy = rect.center().y;
    let font = egui::FontId::proportional(BODY);
    let vg = ui.painter().layout_no_wrap(v.to_owned(), font.clone(), p.ink);
    let vx = rect.right() - vg.size().x;
    widgets::paint_row_text(ui.painter(), vg, vx, cy, p.ink);
    widgets::paint_elided(ui, k, pos2(rect.left(), cy), (vx - rect.left() - 12.0).max(10.0), font, p.muted);
    ui.painter()
        .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, p.raised.gamma_multiply(0.6)));
}

/// Small figures side by side: a muted name over a value.
pub fn figures(ui: &mut egui::Ui, p: &Palette, columns: usize, items: &[(String, String)]) {
    let columns = columns.max(1);
    let gap = 12.0;
    let width = ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(40.0);
    for row in items.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (k, v) in row {
                ui.allocate_ui_with_layout(vec2(width, 34.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(k).size(SMALL).color(p.muted_dim)).truncate());
                        ui.add(egui::Label::new(egui::RichText::new(v).size(BODY).strong().color(p.ink)).truncate());
                    });
                });
            }
        });
    }
}

/// A status dot and its text, in a pill.
pub fn pill(ui: &mut egui::Ui, p: &Palette, dot: Color32, label: &str) {
    egui::Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                widgets::status_dot(ui, dot);
                ui.label(egui::RichText::new(label).size(SMALL).color(p.ink));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_end_on_round_values() {
        assert_eq!(nice_max(0.0), 1e-6);
        assert_eq!(nice_max(7.0), 10.0);
        assert_eq!(nice_max(1.2), 2.0);
        assert_eq!(nice_max(2.2), 2.5);
        assert_eq!(nice_max(3100.0), 5000.0);
    }

    #[test]
    fn the_first_column_takes_the_room_left() {
        let cols = [Col::new("a", 0.0, false), Col::new("b", 100.0, true), Col::new("c", 50.0, true)];
        assert_eq!(col_ranges(&cols, 10.0, 400.0), [(10.0, 260.0), (260.0, 360.0), (360.0, 410.0)]);
    }
}
