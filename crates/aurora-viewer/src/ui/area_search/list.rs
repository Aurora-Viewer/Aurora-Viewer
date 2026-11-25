//! The lists of Area Search: objects by distance, owners, what is for sale,
//! the whole table, and the card of the selected object.

use super::model::{self, Digest, Line};
use super::table::metres;
use super::{Action, AreaSearchUi, Column, Env, Row};
use crate::theme::Palette;
use crate::ui::board::{self, Lit};
use crate::ui::inspect::wide::{group_face, land_weight};
use crate::ui::{menu, widgets};
use egui::{Rect, RichText, Sense, Vec2};
use uuid::Uuid;

/// Height of a row.
pub(super) const ROW: f32 = 24.0;

fn bold(ui: &egui::Ui, size: f32) -> egui::FontId {
    egui::FontId::new(size, crate::ui::fonts::semibold(ui.ctx()))
}

/// The heading text of a distance band.
fn band_note(band: usize, count: usize) -> String {
    let from = model::BANDS[band];
    let range = match (from, model::BANDS.get(band + 1)) {
        (0.0, Some(to)) => aurora_i18n::tr!("area-search-under", distance = metres(*to)),
        (_, Some(to)) => aurora_i18n::tr!("area-search-between", from = aurora_i18n::integer(from as i64), to = metres(*to)),
        (_, None) => aurora_i18n::tr!("area-search-beyond", distance = metres(from)),
    };
    format!("{range} · {}", aurora_i18n::integer(count as i64))
}

/// What a click on a row does, shared by every list.
impl AreaSearchUi {
    /// The face of an owner: a group's is square; hidden by RLVa, initials.
    pub(super) fn owner_face(&self, ui: &egui::Ui, p: &Palette, env: &Env<'_>, r: Rect, who: Uuid, name: &str, group: bool, hidden: bool) {
        if group {
            group_face(ui, p, r);
            return;
        }
        let pic = (!hidden && !who.is_nil()).then_some(who);
        self.wanted_pics.borrow_mut().extend(pic);
        widgets::avatar_disc(ui, p, r, pic.and_then(|id| env.pics.get(&id)), name, false);
    }

    /// Selection, zoom, menu and tooltip of a row of the results.
    pub(super) fn respond(&mut self, ui: &egui::Ui, env: &Env<'_>, resp: egui::Response, visible: usize) -> Option<Action> {
        let row = &self.table.rows[self.table.visible[visible]];
        let id = row.id;
        let mut action = None;
        if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            let (rows, list) = (&self.table.rows, &self.table.visible);
            let order = || list.iter().map(|i| rows[*i].id).collect();
            self.selection.click(id, m.command || self.several, m.shift, order);
        }
        if resp.double_clicked() && !self.several && !ui.input(|i| i.modifiers.command || i.modifiers.shift) {
            action = Some(Action::Zoom(id));
        }
        let demo = self.demo_menu && visible == 1;
        if resp.secondary_clicked() || demo {
            self.demo_menu = false;
            self.selection.context_click(id);
            let at = resp.interact_pointer_pos().unwrap_or(resp.rect.center());
            self.open_menu(ui, env, at);
        }
        let row = &self.table.rows[self.table.visible[visible]];
        resp.on_hover_ui(|ui| {
            let lines = [row.name.trim(), crate::hover_tips::written(&row.description), &id.to_string()];
            let lines: Vec<&str> = lines.into_iter().filter(|s| !s.is_empty()).collect();
            ui.label(lines.join("\n"));
        });
        action
    }

    pub(super) fn open_menu(&mut self, ui: &egui::Ui, env: &Env<'_>, at: egui::Pos2) {
        self.menu_locks = (env.locks)(&self.selection.ids);
        self.menu_at = Some(at);
        egui::Popup::open_id(ui.ctx(), super::menu_id());
    }

    /// Ctrl+A on the list.
    pub(super) fn select_all_key(&mut self, ui: &egui::Ui, nav: &board::Nav) {
        if nav.focused(ui) && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            let all = self.table.visible.iter().map(|i| self.table.rows[*i].id).collect();
            self.selection.select_all(all);
        }
    }
}

/// One sentence for a screen reader.
fn spoken(row: &Row) -> String {
    aurora_i18n::tr!(
        "area-search-spoken",
        name = row.cell(Column::Name),
        distance = metres(row.distance),
        li = row.land_impact_text(),
        owner = row.owner.as_str()
    )
}

/// How the rows of the list are laid out.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Style {
    /// Owner's face, name, distance, land impact.
    Faces,
    /// Distance, name, land impact, owner: the small window's columns.
    Columns,
}

/// Where the columns of the small window are: distance, name, land impact,
/// owner.
pub(super) fn small_columns(line: Rect) -> [Rect; 4] {
    let owner = (line.width() * 0.32).clamp(80.0, 170.0);
    let x = [
        line.left(),
        line.left() + 58.0,
        line.right() - owner - 44.0,
        line.right() - owner,
        line.right(),
    ];
    [0, 1, 2, 3].map(|i| Rect::from_min_max(egui::pos2(x[i], line.top()), egui::pos2(x[i + 1], line.bottom())))
}

/// The headings of the small window's list.
pub(super) fn small_header(ui_state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette) {
    let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let cols = [
        (Column::Distance, aurora_i18n::tr!("area-search-col-distance"), 6.0),
        (Column::Name, Column::Name.label(), 4.0),
        (Column::LandImpact, aurora_i18n::tr!("area-search-col-li"), 6.0),
        (Column::Owner, Column::Owner.label(), 2.0),
    ];
    for ((column, title, pad), cell) in cols.into_iter().zip(small_columns(line)) {
        heading(ui_state, ui, p, column, title, cell, pad, false);
    }
    ui.painter().hline(line.x_range(), line.bottom(), widgets::divider_stroke(p));
}

/// One heading that sorts.
#[allow(clippy::too_many_arguments)]
fn heading(state: &mut AreaSearchUi, ui: &egui::Ui, p: &Palette, column: Column, title: &str, cell: Rect, pad: f32, right: bool) {
    let resp = ui.interact(cell, ui.scope_id().with(("area_search_head", column.index())), Sense::click());
    let sorted = state.table.sort == column;
    let ink = if sorted || resp.hovered() { p.ink } else { p.muted };
    let font = egui::FontId::proportional(11.5);
    let mark = if sorted { 13.0 } else { 0.0 };
    let y = cell.center().y;
    let mut end = cell.right() - 8.0;
    if right {
        board::text_ending(ui, cell.right() - 8.0 - mark, y, title, 11.5, ink);
    } else {
        let left = cell.left() + pad;
        widgets::paint_elided(ui, title, egui::pos2(left, y), cell.width() - pad - mark, font.clone(), ink);
        let w = ui.painter().layout_no_wrap(title.to_owned(), font, ink).size().x;
        end = (left + w + 8.0).min(cell.right() - 6.0);
    }
    if sorted {
        let icon = if state.table.descending {
            "caret-down-filled"
        } else {
            "caret-up-filled"
        };
        crate::ui::sheet::icon_at(ui, icon, egui::pos2(end, y), 8.0, p.violet_light);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, sorted, title));
    if resp.on_hover_text(column.label()).clicked() {
        state.table.sort_by(column);
    }
}

/// The objects, under their distance bands while the nearest come first. The
/// single selected object's card opens under its row with `card`.
#[allow(clippy::too_many_arguments)]
pub(super) fn objects(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    d: &Digest,
    style: Style,
    card: bool,
) -> Option<Action> {
    let mut action = None;
    if state.table.visible.is_empty() {
        empty(state, ui, p);
        return None;
    }
    let scale = crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let height = if style == Style::Faces { ROW } else { 27.0 } * scale;
    let nav = board::Nav::begin(ui, "area_search_objects", d.lines.len(), 12);
    state.select_all_key(ui, &nav);
    // where the open card goes, as tall as it was drawn last
    let open = state.selection.single().filter(|_| card).and_then(|id| {
        d.lines
            .iter()
            .position(|l| matches!(l, Line::Row(at) if state.table.rows[state.table.visible[*at]].id == id))
    });
    let card_id = egui::Id::unique("area_search_card_height");
    let tall = ui.data(|data| data.get_temp::<f32>(card_id)).unwrap_or(150.0);
    board::virtual_rows(
        ui,
        "area_search_objects",
        d.lines.len(),
        height,
        open.map(|i| (i, tall)),
        &nav,
        |ui, i, line, under| {
            match d.lines[i] {
                Line::Band(band) => {
                    // (a stop for the keyboard that does nothing)
                    nav.row(ui, line, i);
                    band_line(ui, p, line, band, &d.bands[band], style);
                }
                Line::Row(at) => {
                    let row = &state.table.rows[state.table.visible[at]];
                    let on = state.selection.contains(&row.id);
                    let lit = if on { Lit::Outlined } else { Lit::Off };
                    let resp = board::row_at(ui, p, &nav, i, line, lit);
                    match style {
                        Style::Faces => face_row(state, ui, p, env, line, row, on),
                        Style::Columns => column_row(state, ui, p, env, line, row, on),
                    }
                    let label = spoken(row);
                    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
                    action = state.respond(ui, env, resp, at).or(action.take());
                    if let Some(under) = under {
                        let row = state.table.rows[state.table.visible[at]].clone();
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt("area_search_card")
                                .max_rect(under.shrink2(Vec2::new(2.0, 3.0)))
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        action = details(state, &mut child, p, env, d, &row, true).or(action.take());
                        let used = child.min_rect().height() + 8.0;
                        if (used - tall).abs() > 0.5 {
                            ui.data_mut(|data| data.insert_temp(card_id, used));
                            ui.ctx().request_repaint();
                        }
                    }
                }
            }
        },
    );
    action
}

pub(super) fn empty(state: &AreaSearchUi, ui: &mut egui::Ui, p: &Palette) {
    ui.add_space(10.0);
    let text = if state.table.rows.is_empty() {
        aurora_i18n::tr!("area-search-nothing")
    } else {
        aurora_i18n::tr!("area-search-empty")
    };
    ui.label(RichText::new(text).size(12.0).color(p.muted));
}

fn band_line(ui: &egui::Ui, p: &Palette, line: Rect, band: usize, value: &model::Band, style: Style) {
    let y = line.center().y;
    let left = line.left() + 5.0;
    let title = model::band_name(band);
    let galley = ui.painter().layout_no_wrap(title.to_owned(), bold(ui, 12.5), p.ink);
    let width = galley.size().x;
    widgets::paint_ink_left(ui.painter(), galley, egui::pos2(left, y), p.ink);
    let li = aurora_i18n::tr!("area-search-li", value = aurora_i18n::integer(value.land_impact.round() as i64));
    let right = board::text_ending(ui, line.right() - 6.0, y, &li, 11.5, p.muted);
    let note = match style {
        Style::Faces => aurora_i18n::integer(value.count as i64),
        Style::Columns => band_note(band, value.count),
    };
    let at = egui::pos2(left + width + 8.0, y);
    widgets::paint_elided(
        ui,
        &note,
        at,
        (right - at.x - 8.0).max(10.0),
        egui::FontId::proportional(11.5),
        p.muted,
    );
    let id = ui.scope_id().with(("area_search_band", band));
    ui.interact(line, id, Sense::hover())
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, format!("{title}, {note}, {li}")));
}

/// Face, name, check, distance, land impact.
fn face_row(state: &AreaSearchUi, ui: &egui::Ui, p: &Palette, env: &Env<'_>, line: Rect, row: &Row, on: bool) {
    let y = line.center().y;
    let face = Rect::from_center_size(egui::pos2(line.left() + 14.0, y), Vec2::splat(18.0));
    state.owner_face(ui, p, env, face, row.owner_id, &row.owner, row.group_owned(), row.names_hidden);
    let weight = land_weight(row.land_impact.unwrap_or(0.0));
    let right = board::badge(ui, p, line.right() - 6.0, y, row.land_impact_text(), weight) - 8.0;
    let mut right = board::text_ending(ui, right, y, &metres(row.distance), 11.5, p.muted) - 8.0;
    if on {
        right = board::check(ui, p, right, y);
    }
    let left = face.right() + 7.0;
    let font = if on { bold(ui, 12.5) } else { egui::FontId::proportional(12.5) };
    let ink = if row.loaded { p.ink } else { p.muted };
    let name = row.cell(Column::Name);
    widgets::paint_elided(ui, name.trim(), egui::pos2(left, y), (right - left).max(16.0), font, ink);
}

/// Distance, name, land impact, owner.
fn column_row(state: &AreaSearchUi, ui: &egui::Ui, p: &Palette, env: &Env<'_>, line: Rect, row: &Row, on: bool) {
    let y = line.center().y;
    let cells = small_columns(line);
    let small = egui::FontId::proportional(11.5);
    let at = cells[0].left() + 6.0;
    widgets::paint_elided(ui, &metres(row.distance), egui::pos2(at, y), cells[0].width() - 8.0, small, p.muted);
    let mut right = cells[1].right() - 4.0;
    if on {
        right = board::check(ui, p, right, y);
    }
    let left = cells[1].left() + 4.0;
    let font = if on { bold(ui, 12.5) } else { egui::FontId::proportional(12.5) };
    let ink = if row.loaded { p.ink } else { p.muted };
    let name = row.cell(Column::Name);
    widgets::paint_elided(ui, name.trim(), egui::pos2(left, y), (right - left).max(16.0), font, ink);
    let weight = land_weight(row.land_impact.unwrap_or(0.0));
    board::badge(ui, p, cells[2].right() - 8.0, y, row.land_impact_text(), weight);
    let face = Rect::from_center_size(egui::pos2(cells[3].left() + 9.0, y), Vec2::splat(18.0));
    state.owner_face(ui, p, env, face, row.owner_id, &row.owner, row.group_owned(), row.names_hidden);
    let left = face.right() + 6.0;
    let width = (cells[3].right() - left - 4.0).max(10.0);
    widgets::paint_elided(ui, &row.owner, egui::pos2(left, y), width, egui::FontId::proportional(12.0), p.ink);
}

/// The card of the selected object: who owns it and its land impact; opened,
/// who made it and its other figures too.
#[allow(clippy::too_many_arguments)]
pub(super) fn details(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    d: &Digest,
    row: &Row,
    frame: bool,
) -> Option<Action> {
    let mut action = None;
    let radius = ui.visuals().window_corner_radius;
    let face = egui::Frame::new().corner_radius(radius).inner_margin(8);
    let face = if frame { face.fill(p.bar) } else { face };
    face.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 6.0;
        let open = state.card_open;
        let gap = 6.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let arrow = 26.0;
            let width = ((ui.available_width() - arrow - 2.0 * gap) / 2.0).floor();
            // (under `@shownames` the owner is a stand-in, a group too)
            let known = !row.names_hidden && !row.owner_id.is_nil();
            let owner_button = known.then(|| {
                if row.group_owned() {
                    Action::Group(row.owner_id)
                } else {
                    Action::Profile(row.owner_id)
                }
            });
            let (who, group, hidden) = (row.owner_id, row.group_owned(), row.names_hidden);
            let title = aurora_i18n::tr!("inspect-owned-by");
            if person(
                state,
                ui,
                p,
                env,
                width,
                title,
                who,
                &row.owner,
                group,
                hidden,
                owner_button.is_some(),
            ) {
                action = owner_button;
            }
            if open {
                let button = (!row.names_hidden && !row.creator_id.is_nil()).then_some(Action::Profile(row.creator_id));
                let title = aurora_i18n::tr!("inspect-made-by");
                if person(
                    state,
                    ui,
                    p,
                    env,
                    width,
                    title,
                    row.creator_id,
                    &row.creator,
                    false,
                    hidden,
                    button.is_some(),
                ) {
                    action = button;
                }
            } else {
                impact(ui, p, d, row, width);
            }
            let (icon, tip) = if open {
                ("chevron-down", aurora_i18n::tr!("area-search-card-less"))
            } else {
                ("chevron-up", aurora_i18n::tr!("area-search-card-more"))
            };
            let icon = if frame && !open {
                "chevron-down"
            } else if frame {
                "chevron-up"
            } else {
                icon
            };
            if widgets::icon_toggle_sized(ui, p, icon, tip, false, arrow) {
                state.card_open = !open;
            }
        });
        if open {
            let n = |v: u32| aurora_i18n::integer(i64::from(v));
            let price = row.price.map_or_else(
                || aurora_i18n::tr!("area-search-not-for-sale").to_owned(),
                |v| format!("{} {}", aurora_i18n::currency(), aurora_i18n::integer(i64::from(v))),
            );
            let facts = env.facts;
            let triangles = facts.map_or("…".to_owned(), |f| n(f.triangles));
            let memory = facts.map_or("…".to_owned(), |f| crate::ui::inspect::megabytes(f.texture_memory));
            let cells = [
                (row.land_impact_text(), aurora_i18n::tr!("area-search-land-impact")),
                (n(row.prim_count), aurora_i18n::tr!("inspect-tile-prims")),
                (metres(row.distance), aurora_i18n::tr!("area-search-range")),
                (triangles, aurora_i18n::tr!("inspect-col-triangles")),
                (memory, aurora_i18n::tr!("inspect-totals-textures")),
                (price, aurora_i18n::tr!("area-search-price")),
            ];
            let width = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
            for (l, line) in cells.chunks(3).enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (c, (value, word)) in line.iter().enumerate() {
                        let (r, resp) =
                            ui.allocate_exact_size(Vec2::new(width, 38.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0)), Sense::hover());
                        let li = l == 0 && c == 0;
                        let weight = if li { land_weight(row.land_impact.unwrap_or(0.0)) } else { 1 };
                        figure_card(ui, p, r, value, word, weight, li);
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{word}, {value}")));
                    }
                });
            }
        }
    });
    action
}

fn figure_card(ui: &egui::Ui, p: &Palette, r: Rect, value: &str, word: &str, weight: u8, tier: bool) {
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let heavy = tier && weight == 2;
    let face = if heavy {
        p.warn.gamma_multiply(0.14)
    } else {
        p.raised.gamma_multiply(0.7)
    };
    ui.painter().rect_filled(r, radius, face);
    let ink = match (tier, weight) {
        (true, 2) => p.warn,
        (true, 0) => p.muted,
        _ => p.ink,
    };
    let off = 8.0 * crate::ui::fonts::scale(ui.ctx());
    let (left, y) = (r.left() + 8.0, r.center().y);
    widgets::paint_elided(ui, value, egui::pos2(left, y - off), r.width() - 14.0, bold(ui, 13.5), ink);
    let small = egui::FontId::proportional(11.0);
    widgets::paint_elided(ui, word, egui::pos2(left, y + off), r.width() - 14.0, small, p.muted);
}

/// The land impact, large, in the colour of its badge, with its place among
/// the objects listed.
fn impact(ui: &mut egui::Ui, p: &Palette, d: &Digest, row: &Row, width: f32) {
    let height = 40.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let weight = land_weight(row.land_impact.unwrap_or(0.0));
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let (face, ink) = match weight {
        2 => (p.warn.gamma_multiply(0.14), p.warn),
        1 => (p.raised.gamma_multiply(0.7), p.ink),
        _ => (p.raised.gamma_multiply(0.7), p.muted),
    };
    ui.painter().rect_filled(r, radius, face);
    let value = row.land_impact_text();
    let y = r.center().y;
    let galley = ui.painter().layout_no_wrap(value.clone(), bold(ui, 20.0), ink);
    let w = galley.size().x;
    widgets::paint_ink_left(ui.painter(), galley, egui::pos2(r.left() + 10.0, y), ink);
    let left = r.left() + 10.0 + w + 9.0;
    let room = (r.right() - left - 4.0).max(10.0);
    let off = 7.5 * crate::ui::fonts::scale(ui.ctx());
    let title = aurora_i18n::tr!("area-search-land-impact");
    let rank = row.land_impact.map(|v| {
        let rank = d.rank(v);
        if d.ranked() <= 99 || rank > 99 {
            aurora_i18n::tr!("area-search-rank-here", rank = rank)
        } else {
            aurora_i18n::tr!("area-search-rank", rank = rank, all = aurora_i18n::integer(d.ranked() as i64))
        }
    });
    let small = egui::FontId::proportional(11.0);
    match &rank {
        Some(rank) => {
            widgets::paint_elided(ui, title, egui::pos2(left, y - off), room, small.clone(), p.ink);
            widgets::paint_elided(ui, rank, egui::pos2(left, y + off), room, small, p.muted);
        }
        None => {
            widgets::paint_elided(ui, title, egui::pos2(left, y), room, small, p.ink);
        }
    }
    let label = format!("{title}, {value}, {}", rank.unwrap_or_default());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &label));
}

/// A card naming someone; true when their profile was asked for.
#[allow(clippy::too_many_arguments)]
fn person(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    width: f32,
    title: &str,
    who: Uuid,
    name: &str,
    group: bool,
    hidden: bool,
    can_open: bool,
) -> bool {
    let scale = crate::ui::fonts::scale(ui.ctx());
    let sense = if can_open { Sense::click() } else { Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, 40.0 * scale.max(1.0)), sense);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = if can_open && resp.hovered() { 1.0 } else { 0.7 };
    ui.painter().rect_filled(r, radius, p.raised.gamma_multiply(face));
    let y = r.center().y;
    let pic = Rect::from_center_size(egui::pos2(r.left() + 22.0, y), Vec2::splat(28.0));
    state.owner_face(ui, p, env, pic, who, name, group, hidden);
    let mut right = r.right() - 8.0;
    if can_open {
        let icon = if group { "users" } else { "user-filled" };
        crate::ui::sheet::icon_at(ui, icon, egui::pos2(right - 8.0, y), 14.0, p.muted);
        right -= 22.0;
    }
    let left = pic.right() + 8.0;
    let off = 7.5 * scale;
    let room = (right - left).max(10.0);
    widgets::paint_elided(
        ui,
        title,
        egui::pos2(left, y - off),
        room,
        egui::FontId::proportional(11.0),
        p.muted,
    );
    let shown = if name.is_empty() {
        aurora_i18n::tr!("common-loading-2")
    } else {
        name
    };
    widgets::paint_elided(ui, shown, egui::pos2(left, y + off), room, bold(ui, 12.5), p.ink);
    let label = format!("{title} {shown}");
    let role = if can_open { egui::Role::Button } else { egui::Role::Label };
    resp.widget_info(|| egui::WidgetInfo::labeled(role, true, &label));
    let tip = if group {
        aurora_i18n::tr!("area-search-open-group")
    } else {
        aurora_i18n::tr!("inspect-open-profile")
    };
    let clicked = resp.clicked();
    if can_open {
        resp.on_hover_text(tip);
    }
    can_open && clicked
}

// ---- owners, for sale ----

/// Owners, heaviest first; a click narrows everything to one. `full` also
/// says their share and opens their five nearest objects.
pub(super) fn owners(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, env: &Env<'_>, d: &Digest, count: usize, full: bool) {
    let total: f32 = d.owners.iter().map(|o| o.land_impact).sum();
    let count = count.min(d.owners.len());
    let scale = crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let height = if full { 30.0 } else { ROW } * scale;
    let salt = if full { "area_search_owner_table" } else { "area_search_owners" };
    let nav = board::Nav::begin(ui, salt, count, 8);
    let hidden = state.table.rows.first().is_some_and(|r| r.names_hidden);
    let mut draw = |ui: &mut egui::Ui, i: usize, line: Rect| {
        let owner = &d.owners[i];
        let on = state.table.owner == Some(owner.id);
        let resp = board::row_at(ui, p, &nav, i, line, if on { Lit::On } else { Lit::Off });
        let y = line.center().y;
        let mut left = line.left() + 3.0;
        if full {
            let rank = aurora_i18n::integer(i as i64 + 1);
            board::text_ending(ui, line.left() + 30.0, y, &rank, 11.5, p.muted);
            left += 34.0;
        }
        let face = Rect::from_center_size(egui::pos2(left + 10.0, y), Vec2::splat(18.0));
        state.owner_face(ui, p, env, face, owner.id, &owner.name, owner.group, hidden);
        let mut right = line.right() - 5.0;
        if full {
            let share = crate::ui::inspect::model::share(owner.land_impact.round() as u64, total.round().max(1.0) as u64);
            let text = aurora_i18n::tr!("inspect-percent", percent = share);
            right = board::text_ending(ui, right, y, &text, 11.5, p.muted).min(line.right() - 42.0) - 8.0;
        }
        let li = owner.land_impact.round() as u32;
        right = board::badge(ui, p, right, y, crate::ui::inspect::model::short(li), 1) - 8.0;
        if full {
            let n = aurora_i18n::tr!("area-search-object-count", count = owner.count as i64);
            right = board::text_ending(ui, right, y, &n, 11.5, p.muted) - 8.0;
        }
        if on {
            right = board::check(ui, p, right, y);
        }
        let left = face.right() + 7.0;
        let font = if on { bold(ui, 12.5) } else { egui::FontId::proportional(12.5) };
        let name = if owner.name.is_empty() {
            aurora_i18n::tr!("common-loading-2")
        } else {
            &owner.name
        };
        widgets::paint_elided(ui, name, egui::pos2(left, y), (right - left).max(16.0), font, p.ink);
        let label = aurora_i18n::tr!("area-search-owner-spoken", name = name, count = owner.count as i64, li = li);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
        if resp.clicked() {
            state.table.owner = (!on).then_some(owner.id);
        }
    };
    if full {
        board::virtual_rows(ui, salt, count, height, None, &nav, |ui, i, line, _| draw(ui, i, line));
    } else {
        for i in 0..count {
            let (_, line) = ui.allocate_space(Vec2::new(ui.available_width(), height));
            draw(ui, i, line);
        }
    }
}

/// What is for sale, cheapest first; a click selects the object.
pub(super) fn sale(
    state: &mut AreaSearchUi,
    ui: &mut egui::Ui,
    p: &Palette,
    env: &Env<'_>,
    d: &Digest,
    count: usize,
    full: bool,
) -> Option<Action> {
    let mut action = None;
    let count = count.min(d.sale.len());
    let height = if full { 27.0 } else { ROW } * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let salt = if full { "area_search_sale_table" } else { "area_search_sale" };
    let nav = board::Nav::begin(ui, salt, count, 8);
    let places: std::collections::HashMap<usize, usize> = if full {
        state.table.visible.iter().enumerate().map(|(at, i)| (*i, at)).collect()
    } else {
        Default::default()
    };
    let mut draw = |ui: &mut egui::Ui, i: usize, line: Rect| {
        let index = d.sale[i];
        let row = &state.table.rows[index];
        let on = state.selection.contains(&row.id);
        let resp = board::row_at(ui, p, &nav, i, line, if on { Lit::Outlined } else { Lit::Off });
        let y = line.center().y;
        let price = row.cell(Column::Price);
        let mut right = board::text_ending(ui, line.right() - 6.0, y, &price, 12.0, p.ink) - 8.0;
        if full {
            right = board::text_ending(ui, line.right() - 90.0, y, &metres(row.distance), 11.5, p.muted) - 10.0;
            right = right.min(line.right() - 150.0);
        }
        let mut left = line.left() + 4.0;
        if full {
            let face = Rect::from_center_size(egui::pos2(left + 12.0, y), Vec2::splat(18.0));
            state.owner_face(ui, p, env, face, row.owner_id, &row.owner, row.group_owned(), row.names_hidden);
            left = face.right() + 7.0;
        }
        let name = row.cell(Column::Name);
        widgets::paint_elided(
            ui,
            name.trim(),
            egui::pos2(left, y),
            (right - left).max(16.0),
            egui::FontId::proportional(12.5),
            p.ink,
        );
        let label = format!("{}, {price}", spoken(row));
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
        let at = if full {
            places.get(&index).copied()
        } else {
            // (a handful of rows: looked for only when used)
            (resp.clicked() || resp.secondary_clicked() || resp.hovered())
                .then(|| state.table.visible.iter().position(|v| *v == index))
                .flatten()
        };
        if let Some(at) = at {
            action = state.respond(ui, env, resp, at).or(action.take());
        }
    };
    if full {
        board::virtual_rows(ui, salt, count, height, None, &nav, |ui, i, line, _| draw(ui, i, line));
    } else {
        for i in 0..count {
            let (_, line) = ui.allocate_space(Vec2::new(ui.available_width(), height));
            draw(ui, i, line);
        }
    }
    action
}

// ---- the whole table ----

/// Width of a column of the whole table, its heading readable.
fn column_width(ui: &egui::Ui, column: Column) -> f32 {
    let content: f32 = match column {
        Column::Name => 180.0,
        Column::Description => 170.0,
        Column::Owner | Column::Group | Column::Creator | Column::LastOwner => 150.0,
        Column::LandImpact | Column::PrimCount => 80.0,
        Column::Distance | Column::Price => 76.0,
    };
    let font = egui::FontId::proportional(11.5);
    let label = ui.painter().layout_no_wrap(column.label().into(), font, egui::Color32::PLACEHOLDER);
    content.max(label.size().x + 34.0) * crate::ui::fonts::scale(ui.ctx()).max(1.0).sqrt()
}

impl AreaSearchUi {
    /// The columns shown, left to right: the name is always first.
    pub(super) fn shown_columns(&self) -> Vec<Column> {
        let on = self.saved_columns();
        Column::SHOWN.into_iter().filter(|c| *c == Column::Name || on[c.index()]).collect()
    }

    /// The columns to choose from, and their reset.
    pub(super) fn column_pills(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(4.0);
            let mut on = self.saved_columns();
            for column in Column::SHOWN.into_iter().skip(1) {
                let i = column.index();
                if board::pill(ui, p, column.label(), on[i]) {
                    on[i] = !on[i];
                }
            }
            on[Column::Name.index()] = true;
            if on != self.saved_columns() {
                self.visible_columns = Some(on);
            }
            if on != crate::settings::area_search::DEFAULT_COLUMNS {
                let label = aurora_i18n::tr!("area-search-reset-columns");
                let flat = widgets::ActionKind::Flat;
                if widgets::action_button(ui, p, "", label, flat, Vec2::new(0.0, 26.0))
                    .on_hover_text(aurora_i18n::tr!("area-search-reset-columns-tip"))
                    .clicked()
                {
                    self.visible_columns = Some(crate::settings::area_search::DEFAULT_COLUMNS);
                }
            }
        });
        ui.add_space(4.0);
    }
}

/// Every chosen column; the name stays in view when they scroll sideways.
pub(super) fn table(state: &mut AreaSearchUi, ui: &mut egui::Ui, p: &Palette, env: &Env<'_>) -> Option<Action> {
    let mut action = None;
    let columns = state.shown_columns();
    let area = ui.available_rect_before_wrap();
    let bar = ui.spacing().scroll.allocated_width();
    let widths: Vec<f32> = columns.iter().map(|c| column_width(ui, *c)).collect();
    let name_w = widths[0].min(area.width() * 0.6);
    let least: f32 = widths.iter().skip(1).sum();
    let room = (area.width() - bar - name_w).max(40.0);
    // what is left over goes to the columns of text
    let spare = (room - least).max(0.0);
    let texts = columns.iter().skip(1).filter(|c| !c.numeric()).count() + 1;
    let extra = spare / texts as f32;
    let name_w = name_w + extra;
    let room = room - extra;
    let widths: Vec<f32> = columns
        .iter()
        .zip(&widths)
        .skip(1)
        .map(|(c, w)| if c.numeric() { *w } else { *w + extra })
        .collect();
    let across: f32 = widths.iter().sum();
    let scrolls = across > room + 1.0;
    let offset_id = egui::Id::unique("area_search_across");
    let mut offset = ui.data(|d| d.get_temp::<f32>(offset_id)).unwrap_or(0.0);
    if ui.rect_contains_pointer(area) {
        offset -= ui.input(|i| i.smooth_scroll_delta.x);
    }
    offset = if scrolls { offset.clamp(0.0, across - room) } else { 0.0 };
    let cells = |line: Rect| -> Vec<Rect> {
        let mut x = line.left() + name_w - offset;
        widths
            .iter()
            .map(|w| {
                let r = Rect::from_min_max(egui::pos2(x, line.top()), egui::pos2(x + w, line.bottom()));
                x += w;
                r
            })
            .collect()
    };
    // headings
    let (line, _) = ui.allocate_exact_size(Vec2::new(area.width(), 24.0), Sense::hover());
    let name_cell = line.with_max_x(line.left() + name_w);
    heading(state, ui, p, Column::Name, Column::Name.label(), name_cell, 8.0, false);
    let moving = Rect::from_min_max(name_cell.right_top(), egui::pos2(line.right() - bar, line.bottom()));
    {
        let mut head = ui.new_child(egui::UiBuilder::new().id_salt("area_search_heads").max_rect(moving));
        head.set_clip_rect(moving.intersect(ui.clip_rect()));
        for (column, cell) in columns.iter().skip(1).zip(cells(line)) {
            let shown = cell.intersect(moving);
            if shown.width() > 4.0 {
                heading(state, &head, p, *column, column.label(), cell, 8.0, column.numeric());
            }
        }
    }
    ui.painter().hline(line.x_range(), line.bottom(), widgets::divider_stroke(p));
    state.table.prepare();
    if state.table.visible.is_empty() {
        empty(state, ui, p);
        return None;
    }
    let count = state.table.visible.len();
    let height = 26.0 * crate::ui::fonts::scale(ui.ctx()).max(1.0);
    let nav = board::Nav::begin(ui, "area_search_table", count, 14);
    state.select_all_key(ui, &nav);
    let foot = if scrolls { bar + 2.0 } else { 0.0 };
    let list = ui.available_rect_before_wrap();
    let list = list.with_max_y(list.bottom() - foot);
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt("area_search_rows").max_rect(list));
    board::virtual_rows(&mut child, "area_search_table", count, height, None, &nav, |ui, i, line, _| {
        let row = &state.table.rows[state.table.visible[i]];
        let on = state.selection.contains(&row.id);
        let resp = board::row_at(ui, p, &nav, i, line, if on { Lit::Outlined } else { Lit::Off });
        let y = line.center().y;
        let mut right = line.left() + name_w - 4.0;
        if on {
            right = board::check(ui, p, right, y);
        }
        let name = row.cell(Column::Name);
        let left = line.left() + 8.0;
        let ink = if row.loaded { p.ink } else { p.muted };
        widgets::paint_elided(
            ui,
            name.trim(),
            egui::pos2(left, y),
            right - left,
            egui::FontId::proportional(12.5),
            ink,
        );
        let moving = Rect::from_min_max(egui::pos2(line.left() + name_w, line.top()), line.right_bottom());
        let mut cut = ui.new_child(egui::UiBuilder::new().id_salt(("area_search_cells", i)).max_rect(moving));
        cut.set_clip_rect(moving.intersect(ui.clip_rect()));
        for (column, cell) in columns.iter().skip(1).zip(cells(line)) {
            if !cell.intersects(moving) {
                continue;
            }
            match column {
                Column::LandImpact => {
                    let weight = land_weight(row.land_impact.unwrap_or(0.0));
                    board::badge(&cut, p, cell.right() - 8.0, y, row.land_impact_text(), weight);
                }
                Column::Owner => {
                    let face = Rect::from_center_size(egui::pos2(cell.left() + 17.0, y), Vec2::splat(18.0));
                    state.owner_face(&cut, p, env, face, row.owner_id, &row.owner, row.group_owned(), row.names_hidden);
                    board::cell(&cut, cell.with_min_x(face.right()), &row.owner, p.ink);
                }
                Column::Description => board::cell(&cut, cell, crate::hover_tips::written(&row.description), p.muted),
                c if c.numeric() => board::figure_cell(&cut, cell, &row.cell(*c), p.muted),
                c => board::cell(&cut, cell, row.cell(*c).trim(), p.muted),
            }
        }
        let label = spoken(row);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
        action = state.respond(ui, env, resp, i).or(action.take());
    });
    if scrolls {
        // the bar of the columns, under the rows
        let strip = Rect::from_min_max(
            egui::pos2(list.left() + name_w, list.bottom() + 1.0),
            egui::pos2(list.right() - bar, list.bottom() + foot),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt("area_search_bar").max_rect(strip));
        let out = egui::ScrollArea::horizontal()
            .id_salt("area_search_across")
            .auto_shrink([false, false])
            .horizontal_scroll_offset(offset)
            .show(&mut child, |ui| {
                ui.allocate_space(Vec2::new(across, 1.0));
            });
        offset = out.state.clamped_offset().x;
    }
    ui.data_mut(|d| d.insert_temp(offset_id, offset));
    ui.advance_cursor_after_rect(area);
    action
}

/// The "…" menu: the link, the defaults, the separate resets.
impl AreaSearchUi {
    pub(super) fn more_menu(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if menu::item(ui, p, "link", aurora_i18n::tr!("area-search-copy-link")) {
            ui.ctx().copy_text(self.table.filters.link());
        }
        if menu::item(ui, p, "device-floppy-filled", aurora_i18n::tr!("area-search-save-defaults")) {
            self.save_defaults = true;
        }
        menu::separator(ui, p);
        for (section, label) in [
            (super::Section::Search, aurora_i18n::tr!("area-search-reset-search")),
            (super::Section::Filters, aurora_i18n::tr!("area-search-reset-filters")),
            (super::Section::Ranges, aurora_i18n::tr!("area-search-reset-ranges")),
            (super::Section::Columns, aurora_i18n::tr!("area-search-reset-columns")),
            (super::Section::All, aurora_i18n::tr!("area-search-reset-all")),
        ] {
            if menu::item(ui, p, "restore", label) {
                self.reset_section(section);
            }
        }
    }
}
