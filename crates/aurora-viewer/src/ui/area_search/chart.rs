//! The distance chart: how many objects the other filters keep at each
//! distance, and the range, set by its two handles, by dragging over the bars
//! or with the arrow keys.

use super::model::{BARS, Chart, Digest};
use super::table::{Filters, metres};
use crate::theme::Palette;
use crate::ui::widgets;
use egui::{Color32, Rect, Sense, Stroke, Vec2};

/// Height of the line of figures under the bars.
const SCALE: f32 = 14.0;
/// The knob of a handle.
const KNOB: f32 = 5.5;

/// The range as it stands: the whole chart when there is none.
pub(super) fn range(f: &Filters, chart: &Chart) -> (f32, f32) {
    if f.distance_on {
        (f.min_distance.min(chart.reach), f.max_distance.min(chart.reach))
    } else {
        (0.0, chart.reach)
    }
}

/// Set the range; the whole chart is no limit at all.
pub(super) fn set(f: &mut Filters, chart: &Chart, low: f32, high: f32) {
    let (low, high) = (low.min(high).max(0.0), high.max(low).min(chart.reach));
    f.distance_on = low > 0.0 || high < chart.reach;
    f.min_distance = low;
    f.max_distance = if f.distance_on { high } else { Filters::default().max_distance };
    f.normalize();
}

/// "Within 64 m", "10 to 64 m".
pub(super) fn range_label(f: &Filters, chart: &Chart) -> Option<String> {
    if !f.distance_on {
        return None;
    }
    let (low, high) = range(f, chart);
    Some(if low <= 0.0 {
        aurora_i18n::tr!("area-search-within", distance = metres(high))
    } else {
        aurora_i18n::tr!("area-search-between", from = metres(low), to = metres(high))
    })
}

/// The counts over the chart: "1,815 of 4,982 objects · 12,400 LI · 93 owners".
pub(super) fn summary(d: &Digest, shown: usize, total: usize) -> (String, String) {
    let parts = [
        aurora_i18n::tr!("area-search-of-objects", total = total as i64),
        aurora_i18n::tr!("area-search-li-count", value = aurora_i18n::integer(d.land_impact.round() as i64)),
        aurora_i18n::tr!("area-search-owner-count", count = d.owner_count as i64),
    ];
    (aurora_i18n::integer(shown as i64), parts.join(" · "))
}

/// The summary line over `rect`, and the range's cross at its right (before
/// `right`). Returns true when the range was taken away.
pub(super) fn summary_line(ui: &mut egui::Ui, p: &Palette, rect: Rect, text: &(String, String), label: Option<&str>) -> bool {
    let y = rect.center().y;
    let bold = egui::FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()));
    let mut right = rect.right();
    let mut pressed = false;
    if let Some(label) = label {
        let galley = ui
            .painter()
            .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), p.violet_pale);
        let width = galley.size().x + 24.0;
        let r = Rect::from_min_max(egui::pos2(right - width, rect.top()), egui::pos2(right, rect.bottom()));
        let resp = ui.interact(r, ui.scope_id().with("area_search_range"), Sense::click());
        let ink = if resp.hovered() { p.ink } else { p.violet_pale };
        widgets::paint_ink_left(ui.painter(), galley, egui::pos2(r.left(), y), ink);
        crate::ui::sheet::icon_at(ui, "x", egui::pos2(r.right() - 8.0, y), 12.0, ink);
        let tip = aurora_i18n::tr!("area-search-any-distance");
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label}. {tip}")));
        pressed = resp.on_hover_text(tip).clicked();
        right = r.left() - 8.0;
    }
    let count = ui.painter().layout_no_wrap(text.0.clone(), bold, p.ink);
    let width = count.size().x;
    widgets::paint_ink_left(ui.painter(), count, egui::pos2(rect.left(), y), p.ink);
    let left = rect.left() + width + 7.0;
    let font = egui::FontId::proportional(12.0);
    widgets::paint_elided(ui, &text.1, egui::pos2(left, y), (right - left).max(10.0), font, p.muted);
    ui.interact(rect.with_max_x(right), ui.scope_id().with("area_search_summary"), Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{} {}", text.0, text.1)));
    pressed
}

/// The bars, the scale and the two handles over `rect`.
pub(super) fn bars(ui: &mut egui::Ui, p: &Palette, rect: Rect, chart: &Chart, f: &mut Filters) {
    let scale_h = SCALE * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let area = Rect::from_min_max(
        egui::pos2(rect.left() + KNOB, rect.top()),
        egui::pos2(rect.right() - KNOB, rect.bottom() - scale_h),
    );
    if area.width() < 20.0 || area.height() < 4.0 {
        return;
    }
    let id = ui.scope_id().with("area_search_chart");
    let at = |d: f32| area.left() + (d / chart.reach).clamp(0.0, 1.0) * area.width();
    let metres_at = |x: f32| ((x - area.left()) / area.width()).clamp(0.0, 1.0) * chart.reach;
    // dragging over the bars picks a range
    let drag = ui.interact(area.expand2(Vec2::new(0.0, 2.0)), id, Sense::drag());
    let start = id.with("from");
    if drag.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        ui.data_mut(|d| d.insert_temp(start, metres_at(pos.x)));
    }
    if drag.dragged()
        && let Some(pos) = drag.interact_pointer_pos()
        && let Some(from) = ui.data(|d| d.get_temp::<f32>(start))
    {
        let to = metres_at(pos.x);
        if (to - from).abs() >= 1.0 {
            set(f, chart, from.min(to).round(), from.max(to).round());
        }
    }
    let (low, high) = range(f, chart);
    let tallest = chart.tallest().max(1) as f32;
    let pitch = area.width() / BARS as f32;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let pointer = drag.hover_pos();
    for (i, count) in chart.bars.iter().enumerate() {
        if *count == 0 {
            continue;
        }
        let h = (*count as f32 / tallest * area.height()).max(2.0);
        let x = area.left() + i as f32 * pitch;
        let bar = Rect::from_min_max(egui::pos2(x + 1.0, area.bottom() - h), egui::pos2(x + pitch - 1.0, area.bottom()));
        let inside = chart.bar_in(i, low, high);
        let near = (i as f32 + 0.5) * chart.reach / BARS as f32 <= 10.0;
        let hovered = pointer.is_some_and(|at| at.x >= x && at.x < x + pitch);
        let color = match (inside, hovered) {
            (_, true) => p.violet_pale,
            (true, false) if near => p.violet_light,
            (true, false) => p.violet,
            (false, false) => p.muted_dim,
        };
        let top = egui::CornerRadius { sw: 0, se: 0, ..radius };
        ui.painter().rect_filled(bar, top, color);
    }
    if let Some(at) = pointer.filter(|_| !drag.dragged()) {
        let i = (((at.x - area.left()) / pitch) as usize).min(BARS - 1);
        let width = chart.reach / BARS as f32;
        drag.on_hover_text(aurora_i18n::tr!(
            "area-search-bar-tip",
            count = chart.bars[i] as i64,
            from = metres(i as f32 * width),
            to = metres((i + 1) as f32 * width)
        ));
    }
    ui.painter().hline(area.x_range(), area.bottom(), Stroke::new(1.0, p.muted_dim));
    let y = rect.bottom() - scale_h / 2.0 + 1.0;
    let font = egui::FontId::proportional(11.0);
    for n in 0..=4 {
        let d = chart.reach * n as f32 / 4.0;
        let text = if n == 4 {
            metres(d)
        } else {
            aurora_i18n::integer(d.round() as i64)
        };
        let galley = ui.painter().layout_no_wrap(text, font.clone(), p.muted);
        let w = galley.size().x;
        let x = match n {
            // (beside the handles' knobs, not under them)
            0 => rect.left() + 2.0 * KNOB + 2.0,
            4 => rect.right() - 2.0 * KNOB - 2.0 - w,
            _ => at(d) - w / 2.0,
        };
        widgets::paint_ink_left(ui.painter(), galley, egui::pos2(x, y), p.muted);
    }
    let (mut new_low, mut new_high) = (low, high);
    let names = [aurora_i18n::tr!("area-search-from"), aurora_i18n::tr!("area-search-to")];
    for (which, value) in [(0, &mut new_low), (1, &mut new_high)] {
        handle(ui, p, id.with(which), area, at(*value), value, chart.reach, names[which], metres_at);
    }
    if (new_low, new_high) != (low, high) {
        // a handle does not cross the other
        if new_low != low {
            new_low = new_low.min(high);
        } else {
            new_high = new_high.max(low);
        }
        set(f, chart, new_low, new_high);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle(
    ui: &mut egui::Ui,
    p: &Palette,
    id: egui::Id,
    area: Rect,
    x: f32,
    value: &mut f32,
    reach: f32,
    name: &str,
    metres_at: impl Fn(f32) -> f32,
) {
    let hit = Rect::from_min_max(
        egui::pos2(x - 12.0, area.top() - 2.0),
        egui::pos2(x + 12.0, area.bottom() + KNOB + 2.0),
    );
    let resp = ui.interact(hit, id, Sense::click_and_drag());
    if resp.dragged()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        *value = metres_at(pos.x).round();
    }
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            );
        });
        let step = ui.input_mut(|i| {
            let mut step = 0.0;
            for (key, sign) in [(egui::Key::ArrowLeft, -1.0), (egui::Key::ArrowRight, 1.0)] {
                // (Shift first: a plain consume would take it too)
                if i.consume_key(egui::Modifiers::SHIFT, key) {
                    step += 10.0 * sign;
                } else if i.consume_key(egui::Modifiers::NONE, key) {
                    step += sign;
                }
            }
            step
        });
        if step != 0.0 {
            *value = (value.round() + step).clamp(0.0, reach);
        }
    }
    let active = resp.hovered() || resp.dragged() || resp.has_focus();
    let color = if active { p.violet_pale } else { p.ink };
    ui.painter().vline(x, area.y_range(), Stroke::new(2.0, color));
    ui.painter().circle_filled(egui::pos2(x, area.bottom()), KNOB, color);
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        bubble(ui, p, egui::pos2(x, area.top()), area, &metres(*value));
    }
    let text = metres(*value);
    resp.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Slider, true, format!("{name} {text}"));
        info.value = Some(f64::from(*value));
        info
    });
}

/// The value of the handle in use.
fn bubble(ui: &egui::Ui, p: &Palette, at: egui::Pos2, area: Rect, text: &str) {
    let ground: Color32 = p.panel.to_opaque();
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), ground);
    let size = galley.size() + Vec2::new(10.0, 3.0);
    let left = (at.x + 6.0).min(area.right() - size.x).max(area.left());
    let r = Rect::from_min_size(egui::pos2(left, at.y - 2.0), size);
    ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, p.ink);
    widgets::paint_ink_left(ui.painter(), galley, egui::pos2(left + 5.0, r.center().y), ground);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_search_whole_chart_is_no_limit() {
        let chart = Chart::new([3.0, 300.0].into_iter());
        assert_eq!(chart.reach, 512.0);
        let mut f = Filters::default();
        assert_eq!(range(&f, &chart), (0.0, 512.0));
        set(&mut f, &chart, 0.0, 64.0);
        assert!(f.distance_on);
        assert_eq!((f.min_distance, f.max_distance), (0.0, 64.0));
        assert_eq!(
            range_label(&f, &chart),
            Some(aurora_i18n::tr!("area-search-within", distance = metres(64.0)))
        );
        // dragged backwards
        set(&mut f, &chart, 80.0, 20.0);
        assert_eq!(range(&f, &chart), (20.0, 80.0));
        set(&mut f, &chart, 0.0, 600.0);
        assert!(!f.distance_on);
        assert_eq!(range_label(&f, &chart), None);
    }
}
