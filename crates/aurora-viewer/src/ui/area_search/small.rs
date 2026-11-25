//! The small Area Search: the distance chart over the search, the filters as
//! chips, and one list at a time (objects, owners, the whole table).

use super::list::{self, Style};
use super::model::{self, Digest};
use super::{Action, AreaSearchUi, Env, chart, sheet};
use crate::theme::Palette;
use crate::ui::{board, widgets};
use egui::{Rect, Sense, Stroke, Vec2};

pub(super) fn show(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, env: &Env<'_>, d: &Digest) -> Option<Action> {
    ui.spacing_mut().item_spacing.y = 0.0;
    chart_card(state, ui, p, d);
    ui.add_space(8.0);
    search_row(state, ui, p);
    chips(state, ui, p, d);
    ui.add_space(8.0);
    if state.sheet {
        filters(state, ui, p);
        return None;
    }
    let tabs = [
        (aurora_i18n::tr!("area-search-tab-objects"), state.table.visible.len()),
        (aurora_i18n::tr!("area-search-tab-owners"), d.owner_count),
        (aurora_i18n::tr!("area-search-tab-details"), state.table.visible.len()),
    ];
    widgets::count_tabs(ui, p, "area_search_tabs", &mut state.tab, &tabs);
    ui.add_space(4.0);
    match state.tab {
        0 => {
            list::small_header(state, ui, p);
            list::objects(state, ui, p, env, d, Style::Columns, true)
        }
        1 => {
            if d.owners.is_empty() {
                list::empty(state, ui, p);
                return None;
            }
            let before = state.table.owner;
            list::owners(state, ui, p, env, d, usize::MAX, true);
            // an owner picked: their objects
            if state.table.owner.is_some() && state.table.owner != before {
                state.tab = 0;
            }
            None
        }
        _ => {
            state.column_pills(ui, p);
            list::table(state, ui, p, env)
        }
    }
}

fn chart_card(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, d: &Digest) {
    let scale = crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 72.0 * scale), Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.bar);
    let inner = rect.shrink2(Vec2::new(10.0, 6.0));
    let line = inner.with_max_y(inner.top() + 18.0 * scale);
    let text = chart::summary(d, state.table.visible.len(), state.table.rows.len());
    let label = chart::range_label(&state.table.filters, &d.chart);
    if chart::summary_line(ui, p, line, &text, label.as_deref()) {
        chart::set(&mut state.table.filters, &d.chart, 0.0, d.chart.reach);
    }
    let bars = inner.with_min_y(line.bottom() + 3.0);
    chart::bars(ui, p, bars, &d.chart, &mut state.table.filters);
}

fn search_row(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette) {
    let count = model::filters_on(&state.table.filters).len();
    let label = if count > 0 {
        aurora_i18n::tr!("area-search-filters-count", count = count)
    } else {
        aurora_i18n::tr!("area-search-filters").to_owned()
    };
    let icon = "adjustments-horizontal-filled";
    let width = widgets::action_button_fit(ui, icon, &label);
    let height = 30.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let field = line.with_max_x(line.right() - width - 6.0);
    let hint = aurora_i18n::tr!("area-search-hint-short");
    board::search(ui, p, field, &mut state.table.filters.query, hint);
    let button = line.with_min_x(line.right() - width);
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt("area_search_filters").max_rect(button));
    let kind = if state.sheet {
        widgets::ActionKind::Primary
    } else {
        widgets::ActionKind::Flat
    };
    if widgets::action_button(&mut child, p, icon, &label, kind, Vec2::new(width, height)).clicked() {
        state.sheet = !state.sheet;
    }
}

/// The filters that are on, each with its cross; the picked owner too.
fn chips(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, d: &Digest) {
    let on = model::filters_on(&state.table.filters);
    if on.is_empty() && state.table.owner.is_none() {
        return;
    }
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(5.0);
        if let Some(id) = state.table.owner {
            let name = d.owners.iter().find(|o| o.id == id).map_or("", |o| o.name.as_str());
            let name = if name.is_empty() {
                aurora_i18n::tr!("common-loading-2")
            } else {
                name
            };
            if chip(ui, p, "user-filled", name, false) {
                state.table.owner = None;
            }
        }
        for what in on {
            let f = &state.table.filters;
            let (icon, label, hides) = (what.icon(f), what.label(f), what.hides(f));
            if chip(ui, p, icon, &label, hides) {
                what.clear(&mut state.table.filters);
            }
        }
        if widgets::flat_button(ui, p, aurora_i18n::tr!("common-clear"))
            .on_hover_text(aurora_i18n::tr!("area-search-clear-tip"))
            .clicked()
        {
            state.table.filters.reset_all_but_query();
            for what in model::filters_on(&state.table.filters) {
                what.clear(&mut state.table.filters);
            }
            state.table.owner = None;
        }
    });
}

/// One chip: pressed, the filter goes. Hidden kinds are struck and red.
fn chip(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str, hides: bool) -> bool {
    let font = egui::FontId::proportional(12.0);
    let width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), p.ink)
        .size()
        .x
        .min(170.0);
    let height = 26.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width + 52.0, height), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let (edge, ink, disc) = if hides {
        (p.danger, p.danger, p.danger.gamma_multiply(0.25))
    } else {
        (p.violet_light, p.ink, p.violet)
    };
    let face = if resp.hovered() { 0.22 } else { 0.1 };
    let y = rect.center().y;
    ui.painter().rect(
        rect,
        radius,
        edge.gamma_multiply(face),
        Stroke::new(1.0, edge.gamma_multiply(0.7)),
        egui::StrokeKind::Inside,
    );
    let at = egui::pos2(rect.left() + 15.0, y);
    ui.painter().circle_filled(at, 9.0, disc);
    crate::ui::sheet::icon_at(ui, icon, at, 12.0, if hides { ink } else { p.on_accent });
    if hides {
        let d = Vec2::splat(6.5);
        ui.painter().line_segment([at - d, at + d], Stroke::new(1.5, ink));
    }
    widgets::paint_elided(ui, text, egui::pos2(rect.left() + 29.0, y), width + 1.0, font, ink);
    crate::ui::sheet::icon_at(ui, "x", egui::pos2(rect.right() - 11.0, y), 11.0, p.muted);
    let tip = if hides {
        aurora_i18n::tr!("area-search-chip-hidden", name = text)
    } else {
        aurora_i18n::tr!("board-remove", name = text)
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &tip));
    resp.on_hover_text(tip).clicked()
}

fn filters(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette) {
    let area: Rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(area, ui.visuals().window_corner_radius, p.bar);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("area_search_sheet")
            .max_rect(area.shrink2(Vec2::new(10.0, 4.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let foot = 78.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let height = (child.available_height() - foot).max(60.0);
    egui::ScrollArea::vertical()
        .id_salt("area_search_sheet")
        .max_height(height)
        .auto_shrink([false, false])
        .show(&mut child, |ui| sheet::body(ui, p, &mut state.table.filters, true));
    child.add_space(6.0);
    state.sheet_buttons(&mut child, p);
    child.add_space(6.0);
    state.table.prepare();
    let label = aurora_i18n::tr!("area-search-show", count = state.table.visible.len() as i64);
    let width = child.available_width();
    if widgets::primary_button_sized(&mut child, p, &label, Vec2::new(width, 28.0)).clicked() {
        state.sheet = false;
    }
    ui.advance_cursor_after_rect(area);
}
