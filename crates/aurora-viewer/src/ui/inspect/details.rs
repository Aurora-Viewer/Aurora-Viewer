//! The Prims table of Inspect's grown Prims tile: the Legacy table of every
//! prim (LLFloaterInspect::refresh, floater_inspect.xml). The columns are
//! FSInspectColumnConfig's, scrolled sideways when they do not fit.

use super::{Column, InspectUi, Row, kilobytes, shown, slt_datetime};
use crate::theme::Palette;
use crate::ui::widgets;
use egui::{Rect, RichText, Sense, Stroke, Vec2};

/// The cells of `columns` over `row`: figures at their width, the columns of
/// text sharing the rest, the name a little wider.
fn cells(columns: &[Column], row: Rect) -> Vec<(Column, Rect)> {
    let fixed: f32 = columns.iter().filter_map(|c| c.fixed_width()).sum();
    let weight = |c: Column| if c == Column::Name { 1.4 } else { 1.0 };
    let weights: f32 = columns.iter().filter(|c| c.fixed_width().is_none()).map(|c| weight(*c)).sum();
    let unit = if weights > 0.0 {
        ((row.width() - fixed) / weights).max(60.0)
    } else {
        0.0
    };
    let mut x = row.left();
    columns
        .iter()
        .map(|&c| {
            let w = c.fixed_width().unwrap_or(unit * weight(c));
            let cell = Rect::from_min_max(egui::pos2(x, row.top()), egui::pos2(x + w, row.bottom()));
            x += w;
            (c, cell)
        })
        .collect()
}

fn cell_text(row: &Row, column: Column) -> String {
    match column {
        Column::Name => row.name.clone(),
        Column::Description => row.description.clone(),
        Column::Owner => row.owner_name.clone(),
        Column::Creator => row.creator_name.clone(),
        Column::Faces => aurora_i18n::integer(i64::from(row.faces)),
        Column::Vertices => aurora_i18n::integer(i64::from(row.vertices)),
        Column::Triangles => aurora_i18n::integer(i64::from(row.triangles)),
        Column::TextureMemory => kilobytes(row.texture_memory),
        Column::Vram => kilobytes(row.vram),
        Column::Created => slt_datetime(row.created),
    }
}

/// Rows in the order of the table: the linksets' own, or a sorted column.
fn sorted(mut list: Vec<&Row>, sort: Option<(Column, bool)>) -> Vec<&Row> {
    let Some((column, descending)) = sort else {
        return list;
    };
    let text = |a: &str, b: &str| crate::natural_order::cmp(a, b);
    list.sort_by(|a, b| {
        let order = match column {
            Column::Name => text(&a.name, &b.name),
            Column::Description => text(&a.description, &b.description),
            Column::Owner => text(&a.owner_name, &b.owner_name),
            Column::Creator => text(&a.creator_name, &b.creator_name),
            Column::Faces => a.faces.cmp(&b.faces),
            Column::Vertices => a.vertices.cmp(&b.vertices),
            Column::Triangles => a.triangles.cmp(&b.triangles),
            Column::TextureMemory => a.texture_memory.cmp(&b.texture_memory),
            Column::Vram => a.vram.cmp(&b.vram),
            Column::Created => a.created.cmp(&b.created),
        };
        if descending { order.reverse() } else { order }
    });
    list
}

fn text_right(ui: &egui::Ui, text: String, cell: Rect, color: egui::Color32) {
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(12.0), color);
    let left = egui::pos2(cell.right() - 8.0 - galley.size().x, cell.center().y);
    widgets::paint_ink_left(ui.painter(), galley, left, color);
}

/// The whole table in a grown tile: every chosen column, scrolled sideways
/// when they do not fit.
pub(super) fn table(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, list: Vec<&Row>, chosen: u32) {
    let columns: Vec<Column> = Column::ALL
        .into_iter()
        .filter(|c| *c == Column::Name || shown(chosen, *c))
        .collect();
    let least: f32 = columns
        .iter()
        .map(|c| c.fixed_width().unwrap_or(if *c == Column::Name { 170.0 } else { 130.0 }))
        .sum();
    let width = ui.available_width().max(least + 30.0);
    egui::ScrollArea::horizontal()
        .id_salt("inspect_prims_across")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(width);
            header(state, ui, p, &columns);
            rows(state, ui, p, list, &columns);
        });
}

fn header(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, columns: &[Column]) {
    let width = widgets::header_width(ui, ui.scope_id().with("inspect_rows"));
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    // over the rows' own width, the scroll bar left out
    let line = rect.with_max_x(rect.left() + width);
    for (column, cell) in cells(columns, line) {
        let resp = ui.interact(cell, ui.scope_id().with(("inspect_header", column.bit())), Sense::click());
        let sorted = state.sort.filter(|(c, _)| *c == column);
        let color = if sorted.is_some() || resp.hovered() { p.ink } else { p.muted };
        let room = cell.width() - if sorted.is_some() { 22.0 } else { 12.0 };
        let font = egui::FontId::proportional(11.5);
        if column.numeric() {
            let end = Rect::from_min_max(cell.min, egui::pos2(cell.left() + room + 8.0, cell.bottom()));
            text_right(ui, column.title().to_owned(), end, color);
        } else {
            widgets::paint_elided(
                ui,
                column.title(),
                egui::pos2(cell.left() + 8.0, cell.center().y),
                room,
                font,
                color,
            );
        }
        if let Some((_, descending)) = sorted {
            let arrow = if descending { "caret-down-filled" } else { "caret-up-filled" };
            crate::ui::sheet::icon_at(ui, arrow, egui::pos2(cell.right() - 8.0, cell.center().y), 8.0, p.violet_light);
        }
        if resp.clicked() {
            state.sort = match sorted {
                Some((_, false)) => Some((column, true)),
                // a third click goes back to the linksets' own order
                Some((_, true)) => None,
                None => Some((column, false)),
            };
        }
        resp.on_hover_text(column.title());
    }
    let stroke = widgets::divider_stroke(p);
    ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
}

/// The rows, already those the search keeps.
fn rows(state: &mut InspectUi, ui: &mut egui::Ui, p: &Palette, rows: Vec<&Row>, columns: &[Column]) {
    let list = sorted(rows, state.sort);
    let grouped = state.sort.is_none();
    let out = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        if list.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(aurora_i18n::tr!("inspect-empty")).size(12.0).color(p.muted));
        }
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let nav = crate::ui::board::Nav::begin(ui, "inspect_prims", list.len(), 10);
        for (i, row) in list.into_iter().enumerate() {
            let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), 24.0));
            // what is out of sight is neither drawn nor sensed
            if !ui.is_rect_visible(rect) && i != nav.at() {
                continue;
            }
            let resp = nav.row(ui, rect, i);
            if ui.is_rect_visible(rect) {
                if state.selected == Some(row.id) {
                    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
                    ui.painter()
                        .rect_stroke(rect, radius, Stroke::new(1.0, p.violet), egui::StrokeKind::Inside);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
                }
                for (column, cell) in cells(columns, rect) {
                    let text = cell_text(row, column);
                    if column.numeric() {
                        text_right(ui, text, cell, p.muted);
                        continue;
                    }
                    // sorted, the prims leave their linkset: no indent
                    let indent = if column == Column::Name && grouped && !row.root {
                        22.0
                    } else {
                        8.0
                    };
                    let color = if column == Column::Name { p.ink } else { p.muted };
                    let left = egui::pos2(cell.left() + indent, cell.center().y);
                    widgets::paint_elided(
                        ui,
                        text.trim(),
                        left,
                        cell.width() - indent - 4.0,
                        egui::FontId::proportional(12.5),
                        color,
                    );
                }
            }
            let label = format!(
                "{}, {} {}",
                row.name.trim(),
                aurora_i18n::integer(i64::from(row.triangles)),
                aurora_i18n::tr!("inspect-col-triangles")
            );
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            if resp.clicked() {
                state.selected = Some(row.id);
            }
        }
    });
    widgets::remember_scroll_bar(ui, ui.scope_id().with("inspect_rows"), &out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn row(n: u128, name: &str, triangles: u32) -> Row {
        Row {
            id: Uuid::from_u128(n),
            root: n == 1,
            name: name.into(),
            triangles,
            ..Row::default()
        }
    }

    #[test]
    fn rows_keep_their_linkset_order_until_a_column_is_sorted() {
        let rows = [row(1, "Racine", 12), row(2, "bras", 900), row(3, "Anneau", 40)];
        let names = |sort| {
            sorted(rows.iter().collect(), sort)
                .into_iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(None), ["Racine", "bras", "Anneau"]);
        // names without regard to case, figures as numbers
        assert_eq!(names(Some((Column::Name, false))), ["Anneau", "bras", "Racine"]);
        assert_eq!(names(Some((Column::Triangles, true))), ["bras", "Anneau", "Racine"]);
    }

    #[test]
    fn hidden_columns_leave_their_width_to_the_columns_of_text() {
        let line = Rect::from_min_size(egui::pos2(10.0, 0.0), Vec2::new(900.0, 22.0));
        let all = cells(&Column::ALL, line);
        assert_eq!(all.len(), 10);
        assert!((all.last().map_or(0.0, |c| c.1.right()) - line.right()).abs() < 0.01);
        let width = |list: &[(Column, Rect)], c: Column| list.iter().find(|(k, _)| *k == c).map_or(0.0, |(_, r)| r.width());
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(width(&all, Column::Faces), 52.0));
        assert!(width(&all, Column::Name) > width(&all, Column::Owner));
        let few = cells(&[Column::Name, Column::Triangles], line);
        assert!(close(width(&few, Column::Name), 900.0 - 70.0));
    }

    #[test]
    fn cells_show_the_legacy_units() {
        let mut r = row(2, "bras", 1234);
        r.texture_memory = 3 * 1024 * 1024;
        r.created = 1_700_000_000;
        // kilobytes, as the Legacy TMem and VRAM columns
        assert_eq!(cell_text(&r, Column::TextureMemory), aurora_i18n::integer(3072));
        // 2023-11-14 22:13:20 UTC is 14:13 in SLT (PST)
        let date = aurora_i18n::tr!("inspect-date", year = "2023", month = "11", day = "14", hour = "14", minute = "13");
        assert_eq!(cell_text(&r, Column::Created), date);
        r.created = 0;
        assert_eq!(cell_text(&r, Column::Created), "");
    }
}
