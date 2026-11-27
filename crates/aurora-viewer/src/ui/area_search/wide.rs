//! The Area Search window: a board of tiles (the selected objects in 3D, the
//! list, the distance, owners, what is for sale, the filters). Any tile grows,
//! the others staying beside it.

use super::list;
use super::model::{self, Digest};
use super::{Action, AreaSearchUi, Env, chart, sheet};
use crate::theme::Palette;
use crate::ui::board::{self, Form};
use crate::ui::object_stage::Showing;
use crate::ui::{menu, widgets};
use egui::{Rect, RichText, Sense, Stroke, Vec2};

/// The tiles of the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Tile {
    Stage,
    Objects,
    Distance,
    Owners,
    Sale,
    Filters,
}

const TILES: [board::Tile<Tile>; 6] = {
    const fn tile(id: Tile, columns: u8, rows: u8) -> board::Tile<Tile> {
        board::Tile { id, columns, rows }
    }
    [
        tile(Tile::Stage, 2, 5),
        tile(Tile::Objects, 2, 5),
        tile(Tile::Distance, 4, 1),
        tile(Tile::Owners, 1, 2),
        tile(Tile::Sale, 1, 2),
        tile(Tile::Filters, 2, 2),
    ]
};

pub(super) fn show(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, env: &Env<'_>, d: &Digest) -> Option<Action> {
    header(state, ui, p);
    ui.add_space(8.0);
    if state.grown.is_some() && board::escaped(ui) {
        state.grown = None;
    }
    let scale = crate::ui::fonts::scale(ui.ctx());
    let area = ui.available_rect_before_wrap();
    let (_, height) = board::place(&TILES, state.grown, area, scale);
    let mut action = None;
    let mut draw = |ui: &mut egui::Ui, area: Rect| {
        let (placed, height) = board::place(&TILES, state.grown, area, scale);
        for tile in placed {
            let id = egui::Id::unique(("area_search_tile", tile.id));
            let rect = board::slide(ui, id, tile.rect);
            let rect = board::in_sight(ui, rect, tile.form);
            let (toggled, asked) = one_tile(state, ui, p, env, d, id, rect, tile.id, tile.form);
            if toggled {
                state.grown = (state.grown != Some(tile.id)).then_some(tile.id);
            }
            if asked.is_some() {
                action = asked;
            }
        }
        height
    };
    if height > area.height() + 1.0 {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // the area in sight: the rows take their least height, a grown
            // tile the area's
            let area = Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), area.height()));
            let height = draw(ui, area);
            ui.allocate_space(Vec2::new(area.width(), height));
        });
    } else {
        draw(ui, area);
        ui.advance_cursor_after_rect(area);
    }
    action
}

/// Where to look, refresh and the "…" menu, and the search as wide as the tiles under it.
fn header(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette) {
    let scale = crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0 * scale), Sense::hover());
    let half = (line.width() - board::GAP) / 2.0;
    let field = Rect::from_center_size(
        egui::pos2(line.right() - half / 2.0, line.center().y),
        Vec2::new(half, 32.0 * scale),
    );
    let hint = aurora_i18n::tr!("area-search-hint-wide");
    board::search(ui, p, field, &mut state.table.filters.query, hint);
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("area_search_tools")
            .max_rect(line.with_max_x(field.left() - 8.0))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let ui = &mut left;
    ui.spacing_mut().item_spacing.x = 6.0;
    let (r, more) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    tool_face(ui, p, r, &more, "dots");
    let tip = aurora_i18n::tr!("area-search-more");
    more.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    let more = more.on_hover_text(tip);
    menu::dropdown_placed(
        &more,
        p,
        egui::PopupCloseBehavior::CloseOnClick,
        Some((egui::RectAlign::BOTTOM_END, 4.0)),
        |ui| state.more_menu(ui, p),
    );
    let (r, refresh) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    tool_face(ui, p, r, &refresh, "refresh");
    let tip = aurora_i18n::tr!("area-search-refresh");
    refresh.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    if refresh.on_hover_text(tip).clicked() {
        state.refresh = true;
    }
    // where to look, on the left (in the filters when there is no room)
    let labels: Vec<_> = state.scopes.iter().map(|s| (s.label(), true)).collect();
    state.scope_shown = widgets::segmented_size(ui, &labels).x <= ui.available_width();
    if state.scope_shown && labels.len() > 1 {
        let area = line.with_max_x(ui.min_rect().left());
        let mut left = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("area_search_scope")
                .max_rect(area)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        sheet::scope_choice(&mut left, p, &mut state.table.filters, &state.scopes);
    }
}

fn tool_face(ui: &egui::Ui, p: &Palette, r: Rect, resp: &egui::Response, icon: &str) {
    if resp.hovered() {
        ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let ink = if resp.hovered() { p.ink } else { p.muted };
    crate::ui::sheet::icon_at(ui, icon, r.center(), 16.0, ink);
}

#[allow(clippy::too_many_arguments)]
fn one_tile(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    d: &Digest,
    id: egui::Id,
    rect: Rect,
    tile: Tile,
    form: Form,
) -> (bool, Option<Action>) {
    match tile {
        Tile::Stage => (stage(state, ui, p, id, rect, form), None),
        Tile::Objects => objects(state, ui, p, env, d, id, rect, form),
        Tile::Distance => (distance(state, ui, p, d, id, rect, form), None),
        Tile::Owners => (owners(state, ui, p, env, d, id, rect, form), None),
        Tile::Sale => sale(state, ui, p, env, d, id, rect, form),
        Tile::Filters => (filters(state, ui, p, d, id, rect, form), None),
    }
}

/// The selected objects in 3D; empty while none is, so that nothing jumps.
fn stage(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, id: egui::Id, rect: Rect, form: Form) -> bool {
    let radius = ui.visuals().window_corner_radius;
    let ground = if state.selection.is_empty() {
        crate::ui::object_stage::ground_color(p)
    } else {
        state.stage.ground(p)
    };
    let stroke = widgets::border_stroke(p, Stroke::NONE);
    ui.painter().rect(rect, radius, ground, stroke, egui::StrokeKind::Inside);
    let title = aurora_i18n::tr!("area-search-tile-3d");
    let selected = state.selection.single();
    let row = selected.and_then(|id| state.table.rows.iter().find(|r| r.id == id)).cloned();
    let several = state.selection.ids.len();
    let name = match (&row, several) {
        (Some(row), _) => row.cell(super::Column::Name),
        (None, 0) => String::new(),
        (None, n) => aurora_i18n::tr!("inspect-objects-title", count = n),
    };
    let mut toggled = false;
    if several > 0 {
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let showing = Showing {
            // one selected object stands alone: its parts come apart
            alone: selected,
            outlined: state.prim,
            pointed: None,
        };
        let spoken = format!("{title}, {name}");
        let lines = if form == Form::Small { 0.0 } else { 2.0 };
        let top_room = (8.0 + lines * 18.0) * crate::ui::fonts::scale(ui.ctx()).max(1.0);
        if let Some(hit) = state.stage.show(&mut child, p, rect, top_room, showing, &spoken, &|hit| {
            // the prims of one object have no names here
            selected
                .is_none()
                .then(|| state.table.rows.iter().find(|r| r.id == hit.root).map(|r| r.name.clone()))?
        }) && form != Form::Small
        {
            state.prim = (state.prim != Some(hit.prim)).then_some(hit.prim);
        }
        let top = rect.top() + 20.0;
        let left = rect.left() + 12.0;
        let width = rect.width() - 54.0;
        let font = egui::FontId::new(if form == Form::Small { 12.5 } else { 16.0 }, crate::ui::fonts::semibold(ui.ctx()));
        widgets::paint_elided(ui, name.trim(), egui::pos2(left, top - 1.0), width, font, state.stage.ink(p));
        if let Some(row) = row.filter(|_| form != Form::Small) {
            let line = aurora_i18n::tr!(
                "area-search-stage-line",
                distance = super::table::metres(row.distance),
                li = row.land_impact_text(),
                owner = row.owner.as_str()
            );
            let y = top + 18.0 * crate::ui::fonts::scale(ui.ctx());
            widgets::paint_elided(
                ui,
                &line,
                egui::pos2(left, y),
                width + 30.0,
                egui::FontId::proportional(12.0),
                state.stage.ink(p),
            );
        }
    }
    if several > 0 || form == Form::Grown {
        let at = egui::pos2(rect.right() - 8.0, rect.top() + 20.0);
        if several > 0
            && let Some(face) = state.stage.face(p)
        {
            let r = Rect::from_center_size(at - Vec2::new(12.0, 0.0), Vec2::splat(24.0));
            ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, face);
        }
        toggled = board::grow_button(ui, p, id, at, form, title);
    }
    toggled
}

#[allow(clippy::too_many_arguments)]
fn objects(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    d: &Digest,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> (bool, Option<Action>) {
    let title = aurora_i18n::tr!("area-search-tab-objects");
    let n = aurora_i18n::integer(state.table.visible.len() as i64);
    let head = board::Head {
        title,
        note: if form == Form::Small { "" } else { &n },
        spoken: &format!("{title}, {n}"),
    };
    // the card of the selected object stands under the list, its own tile
    let row = state
        .selection
        .single()
        .filter(|_| form == Form::Usual)
        .and_then(|id| state.table.rows.iter().find(|r| r.id == id))
        .cloned();
    let mut action = None;
    let mut list = rect;
    if let Some(row) = &row {
        let scale = crate::ui::fonts::scale(ui.ctx()).max(1.0);
        let height = if state.card_open { 150.0 * scale } else { 56.0 * scale };
        let height = board::slide_value(ui, id.with("card"), height).min(rect.height() * 0.6);
        let card = rect.with_min_y(rect.bottom() - height);
        list.max.y = card.top() - board::GAP;
        board::surface(ui, p, card, Form::Usual);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("area_search_card")
                .max_rect(card)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(card.intersect(ui.clip_rect()));
        action = list::details(state, &mut child, p, env, d, row);
    }
    // the order: nearest or heaviest first
    if form == Form::Usual {
        let nearest = state.table.sort != super::Column::LandImpact;
        let label = if nearest {
            aurora_i18n::tr!("area-search-nearest")
        } else {
            aurora_i18n::tr!("area-search-heaviest")
        };
        let width = widgets::action_button_fit(ui, "", label);
        let at = Rect::from_min_size(
            egui::pos2(list.right() - 10.0 - board::TITLE - 6.0 - width, list.top() + 7.0),
            Vec2::new(width, 26.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt("area_search_order").max_rect(at));
        let flat = widgets::ActionKind::Flat;
        // (the tile's surface is painted under it first)
        board::surface(ui, p, list, form);
        if widgets::action_button(&mut child, p, "", label, flat, Vec2::new(width, 26.0))
            .on_hover_text(aurora_i18n::tr!("area-search-order-tip"))
            .clicked()
        {
            (state.table.sort, state.table.descending) = if nearest {
                (super::Column::LandImpact, true)
            } else {
                (super::Column::Distance, false)
            };
            state.table.dirty = true;
        }
    }
    let (toggled, asked) = board::tile_on(ui, p, id, list, form, Some(head), form != Form::Usual, |ui| match form {
        Form::Small => {
            board::figure(ui, p, &n, aurora_i18n::tr!("area-search-in-reach"));
            None
        }
        Form::Usual => list::objects(state, ui, p, env, d),
        Form::Grown => {
            state.column_pills(ui, p);
            list::table(state, ui, p, env)
        }
    });
    (toggled, asked.or(action))
}

fn distance(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, d: &Digest, id: egui::Id, rect: Rect, form: Form) -> bool {
    let title = aurora_i18n::tr!("area-search-range");
    let text = chart::summary(d, state.table.visible.len(), state.table.rows.len());
    let spoken = format!("{title}, {} {}", text.0, text.1);
    let label = chart::range_label(&state.table.filters, &d.chart);
    if form == Form::Small {
        let head = board::Head {
            title,
            note: "",
            spoken: &spoken,
        };
        return board::tile(ui, p, id, rect, form, Some(head), |ui| {
            let word = label.as_deref().unwrap_or(aurora_i18n::tr!("area-search-objects-word"));
            board::figure(ui, p, &text.0, word);
        })
        .0;
    }
    let scale = crate::ui::fonts::scale(ui.ctx()).max(1.0);
    if form == Form::Usual {
        board::surface(ui, p, rect, form);
        let inner = rect.shrink2(Vec2::new(12.0, 5.0));
        let line = inner.with_max_y(inner.top() + 19.0 * scale).with_max_x(inner.right() - 34.0);
        let toggled = board::grow_button(ui, p, id, egui::pos2(inner.right() + 2.0, line.center().y), form, title);
        if chart::summary_line(ui, p, line, &text, label.as_deref()) {
            chart::set(&mut state.table.filters, &d.chart, 0.0, d.chart.reach);
        }
        let bars = inner.with_min_y(line.bottom() + 1.0);
        chart::bars(ui, p, bars, &d.chart, &mut state.table.filters);
        return toggled;
    }
    let head = board::Head {
        title,
        note: "",
        spoken: &spoken,
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| {
        let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0 * scale), Sense::hover());
        if chart::summary_line(ui, p, line, &text, label.as_deref()) {
            chart::set(&mut state.table.filters, &d.chart, 0.0, d.chart.reach);
        }
        ui.add_space(8.0);
        let height = (ui.available_height() * 0.45).clamp(80.0, 220.0);
        let (bars, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        chart::bars(ui, p, bars, &d.chart, &mut state.table.filters);
        ui.add_space(10.0);
        let cols = [
            board::Col::text(aurora_i18n::tr!("area-search-band"), 1.0),
            board::Col::text(aurora_i18n::tr!("area-search-range"), 0.8),
            board::Col::figure(aurora_i18n::tr!("area-search-tab-objects"), 100.0),
            board::Col::figure(aurora_i18n::tr!("area-search-land-impact"), 120.0),
        ];
        let mut none = None;
        board::header(ui, p, "area_search_band_heads", &cols, &mut none);
        let nav = board::Nav::begin(ui, "area_search_bands", model::BANDS.len(), 5);
        let (low, high) = chart::range(&state.table.filters, &d.chart);
        for (band, value) in d.bands.iter().enumerate() {
            let from = model::BANDS[band];
            let to = model::BANDS.get(band + 1).copied().unwrap_or(d.chart.reach);
            let on = state.table.filters.distance_on && low == from && high == to;
            let lit = if on { board::Lit::On } else { board::Lit::Off };
            let (line, resp) = board::row(ui, p, &nav, band, 27.0 * scale, lit);
            let cells = board::col_rects(&cols, line);
            board::cell(ui, cells[0], model::band_name(band), p.ink);
            let range = aurora_i18n::tr!(
                "area-search-between",
                from = aurora_i18n::integer(from as i64),
                to = super::table::metres(to)
            );
            board::cell(ui, cells[1], &range, p.muted);
            board::figure_cell(ui, cells[2], &aurora_i18n::integer(value.count as i64), p.ink);
            board::figure_cell(ui, cells[3], &aurora_i18n::integer(value.land_impact.round() as i64), p.ink);
            let label = format!("{}, {range}, {}", model::band_name(band), value.count);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
            if resp.clicked() {
                let (low, high) = if on { (0.0, d.chart.reach) } else { (from, to) };
                chart::set(&mut state.table.filters, &d.chart, low, high);
            }
        }
    })
    .0
}

/// How many rows of a short list fit, and how many are left.
fn fitting(ui: &egui::Ui, count: usize) -> (usize, usize) {
    let height = list::ROW * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let room = ui.available_height();
    if count as f32 * height <= room + 1.0 {
        return (count, 0);
    }
    let fit = (((room - 17.0) / height).floor().max(1.0) as usize).min(count);
    (fit, count - fit)
}

fn and_more(ui: &mut egui::Ui, p: &Palette, more: usize) {
    if more > 0 {
        ui.add_space(2.0);
        let text = aurora_i18n::tr!("board-more", count = more);
        ui.label(RichText::new(text).size(11.5).color(p.muted));
    }
}

#[allow(clippy::too_many_arguments)]
fn owners(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    d: &Digest,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> bool {
    let title = aurora_i18n::tr!("area-search-tab-owners");
    let n = aurora_i18n::integer(d.owners.len() as i64);
    let head = board::Head {
        title,
        note: if form == Form::Small { "" } else { &n },
        spoken: &format!("{title}, {n}"),
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => {
            let n = aurora_i18n::integer(d.owner_count as i64);
            board::figure(ui, p, &n, aurora_i18n::tr!("area-search-in-reach"));
        }
        Form::Usual => {
            let (count, more) = fitting(ui, d.owners.len());
            list::owners(state, ui, p, env, d, count, false);
            and_more(ui, p, more);
        }
        Form::Grown => list::owners(state, ui, p, env, d, usize::MAX, true),
    })
    .0
}

#[allow(clippy::too_many_arguments)]
fn sale(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    d: &Digest,
    id: egui::Id,
    rect: Rect,
    form: Form,
) -> (bool, Option<Action>) {
    let title = aurora_i18n::tr!("area-search-sale");
    let n = aurora_i18n::integer(d.sale.len() as i64);
    let head = board::Head {
        title,
        note: if form == Form::Small { "" } else { &n },
        spoken: &format!("{title}, {n}"),
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => {
            let from = d.sale.first().map_or(String::new(), |i| {
                aurora_i18n::tr!("area-search-from-price", price = state.table.rows[*i].cell(super::Column::Price))
            });
            board::figure(ui, p, &n, &from);
            None
        }
        Form::Usual => {
            if d.sale.is_empty() {
                let text = aurora_i18n::tr!("area-search-nothing-for-sale");
                ui.label(RichText::new(text).size(11.5).color(p.muted));
                return None;
            }
            let (count, more) = fitting(ui, d.sale.len());
            let asked = list::sale(state, ui, p, env, d, count, false);
            and_more(ui, p, more);
            asked
        }
        Form::Grown => list::sale(state, ui, p, env, d, usize::MAX, true),
    })
}

fn filters(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, d: &Digest, id: egui::Id, rect: Rect, form: Form) -> bool {
    let title = aurora_i18n::tr!("area-search-filters");
    let on = model::filters_on(&state.table.filters);
    // what the tiles of kinds do not show themselves
    let others = on.iter().filter(|o| !matches!(o, model::On::Kind(_))).count();
    let note = match form {
        Form::Small => String::new(),
        Form::Grown if on.is_empty() => String::new(),
        Form::Grown => aurora_i18n::tr!("area-search-on", count = on.len()),
        Form::Usual if others > 0 => aurora_i18n::tr!("area-search-more-on", count = others),
        Form::Usual => aurora_i18n::tr!("area-search-kind-tip").to_owned(),
    };
    let head = board::Head {
        title,
        note: &note,
        spoken: &format!("{title}, {}", aurora_i18n::tr!("area-search-on", count = on.len())),
    };
    board::tile(ui, p, id, rect, form, Some(head), |ui| match form {
        Form::Small => {
            let n = aurora_i18n::integer(on.len() as i64);
            board::figure(ui, p, &n, aurora_i18n::tr!("area-search-on-word"));
        }
        Form::Usual => {
            let height = ((ui.available_height() - 6.0) / 2.0 - 3.0).max(34.0);
            sheet::kinds(ui, p, &mut state.table.filters, Some(&d.kinds), height);
        }
        Form::Grown => {
            let foot = 40.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0);
            let height = (ui.available_height() - foot).max(60.0);
            egui::ScrollArea::vertical()
                .id_salt("area_search_sheet")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let scopes: &[model::Scope] = if state.scope_shown { &[] } else { &state.scopes };
                    sheet::body(ui, p, &mut state.table.filters, scopes);
                });
            ui.add_space(6.0);
            state.sheet_buttons(ui, p);
        }
    })
    .0
}
