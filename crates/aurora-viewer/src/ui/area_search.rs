//! Area Search: rezzed objects received by the viewer, following
//! FSAreaSearch::isSearchableObject / matchObject (indra/newview/fsareasearch.cpp).
//! Attachments and avatars are excluded; linked prims are optional.

mod table;
#[cfg(test)]
mod tests;
pub(crate) use table::{Column, Filters, Row, Table};

use super::widgets::{self, Floater};
use crate::theme::Palette;
use egui::{RichText, Sense, Vec2};
use uuid::Uuid;

pub const WINDOW: &str = "area_search";

#[derive(Default)]
pub struct AreaSearchUi {
    pub(crate) table: Table,
    pub(crate) selected: Option<Uuid>,
    pub(crate) refresh: bool,
    pub(crate) filters_open: bool,
}

pub enum Action {
    Inspect(Uuid),
    Zoom(Uuid),
}

impl AreaSearchUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, parcel_known: bool) -> Option<Action> {
        let mut action = None;
        let mut window = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-area-search"),
            egui::pos2(250.0, 120.0),
            Vec2::new(800.0, 510.0),
        );
        window.min_size = Vec2::new(560.0, 350.0);
        window.show(ctx, p, open, |ui| {
            ui.horizontal(|ui| {
                let refresh_label = aurora_i18n::tr!("area-search-refresh");
                let refresh_width = widgets::action_button_fit(ui, "refresh", refresh_label);
                let width = (ui.available_width() - refresh_width - ui.spacing().item_spacing.x)
                    .max(120.0)
                    .floor();
                widgets::clearable_search_box(ui, p, &mut self.table.filters.query, aurora_i18n::tr!("area-search-hint"), width);
                if widgets::action_button(
                    ui,
                    p,
                    "refresh",
                    refresh_label,
                    widgets::ActionKind::Flat,
                    Vec2::new(refresh_width, 26.0),
                )
                .clicked()
                {
                    self.refresh = true;
                }
            });
            ui.horizontal_wrapped(|ui| {
                widgets::checkbox(ui, p, &mut self.table.filters.parcel_only, aurora_i18n::tr!("area-search-parcel"));
                widgets::checkbox(ui, p, &mut self.table.filters.children, aurora_i18n::tr!("area-search-children"));
                widgets::checkbox(ui, p, &mut self.table.filters.mine, aurora_i18n::tr!("area-search-mine"));
                widgets::checkbox(ui, p, &mut self.table.filters.neighbors, aurora_i18n::tr!("area-search-neighbors"));
            });
            widgets::collapsing(
                ui,
                p,
                "area_search_filters",
                aurora_i18n::tr!("area-search-filters"),
                self.filters_open,
                |ui| {
                    let wide = ui.available_width() >= 660.0;
                    egui::Grid::new("area_search_fields")
                        .num_columns(if wide { 4 } else { 2 })
                        .show(ui, |ui| {
                            for (i, (label, text)) in [
                                (aurora_i18n::tr!("inspect-col-name"), &mut self.table.filters.name),
                                (aurora_i18n::tr!("inspect-col-description"), &mut self.table.filters.description),
                                (aurora_i18n::tr!("inspect-col-owner"), &mut self.table.filters.owner),
                                (aurora_i18n::tr!("inspect-col-creator"), &mut self.table.filters.creator),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                ui.label(label);
                                widgets::filter_field(ui, p, text, egui::Id::unique(("area_search_field", i)), 180.0);
                                if !wide || i % 2 == 1 {
                                    ui.end_row();
                                }
                            }
                        });
                    ui.horizontal_wrapped(|ui| {
                        widgets::checkbox(ui, p, &mut self.table.filters.sale, aurora_i18n::tr!("area-search-sale"));
                        widgets::checkbox(ui, p, &mut self.table.filters.physical, aurora_i18n::tr!("area-search-physical"));
                        widgets::checkbox(ui, p, &mut self.table.filters.phantom, aurora_i18n::tr!("area-search-phantom"));
                        widgets::checkbox(ui, p, &mut self.table.filters.temporary, aurora_i18n::tr!("area-search-temporary"));
                    });
                    ui.horizontal(|ui| {
                        widgets::checkbox(ui, p, &mut self.table.filters.distance_on, aurora_i18n::tr!("area-search-distance"));
                        ui.add_enabled(
                            self.table.filters.distance_on,
                            egui::DragValue::new(&mut self.table.filters.max_distance)
                                .range(0.0..=65536.0)
                                .suffix(" m"),
                        );
                        if ui.button(aurora_i18n::tr!("area-search-reset")).clicked() {
                            self.table.filters = Filters::default();
                        }
                    });
                },
            );
            self.table.prepare();
            if self
                .selected
                .is_some_and(|id| !self.table.visible.iter().any(|i| self.table.rows[*i].id == id))
            {
                self.selected = None;
            }
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "area-search-count",
                    shown = self.table.visible.len() as i64,
                    total = self.table.rows.len() as i64,
                    pending = self.table.pending as i64
                ))
                .size(12.0)
                .color(p.muted),
            );
            if self.table.filters.parcel_only && !parcel_known {
                ui.label(RichText::new(aurora_i18n::tr!("area-search-parcel-pending")).color(p.warn));
            }
            ui.label(RichText::new(aurora_i18n::tr!("area-search-scope")).size(11.0).color(p.muted));
            let width = widgets::header_width(ui, ui.scope_id().with("area_search_rows"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (column, fraction) in columns() {
                    let label = column.label();
                    let label = if self.table.sort == column {
                        format!("{label} {}", if self.table.descending { "↓" } else { "↑" })
                    } else {
                        label.to_owned()
                    };
                    if ui
                        .add_sized([width * fraction - 4.0, 24.0], egui::Button::new(label).truncate())
                        .on_hover_text(column.label())
                        .clicked()
                    {
                        self.table.descending = self.table.sort == column && !self.table.descending;
                        self.table.sort = column;
                        self.table.dirty = true;
                    }
                }
            });
            self.table.prepare();
            if self.table.visible.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("area-search-empty")).color(p.muted));
            }
            let height = (ui.available_height() - 40.0).max(60.0);
            let list = egui::ScrollArea::vertical()
                .id_salt("area_search_rows")
                .max_height(height)
                .auto_shrink([false, false])
                .show_rows(ui, 25.0, self.table.visible.len(), |ui, range| {
                    for i in range {
                        let row = &self.table.rows[self.table.visible[i]];
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 25.0), Sense::click());
                        let selected = self.selected == Some(row.id);
                        if selected || response.hovered() {
                            let color = if selected { p.violet.gamma_multiply(0.28) } else { p.raised };
                            ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, color);
                        }
                        let mut x = rect.left();
                        for (column, fraction) in columns() {
                            let w = rect.width() * fraction;
                            let text = row.cell(column);
                            let mut job = egui::text::LayoutJob::simple_singleline(text, egui::FontId::proportional(12.0), p.ink);
                            job.wrap = egui::text::TextWrapping::truncate_at_width(w - 10.0);
                            let galley = ui.painter().layout_job(job);
                            let at = widgets::row_text_origin(ui.painter(), &galley, x + 4.0, rect.center().y);
                            ui.painter().galley(at, galley, p.ink);
                            x += w;
                        }
                        if response.clicked() {
                            self.selected = Some(row.id);
                        }
                        if response.double_clicked() {
                            action = Some(Action::Zoom(row.id));
                        }
                        response.on_hover_text(format!("{}\n{}\n{}", row.name, row.description, row.id));
                    }
                });
            widgets::remember_scroll_bar(ui, ui.scope_id().with("area_search_rows"), &list);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.selected.is_some(), |ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "info-circle-filled",
                        aurora_i18n::tr!("area-search-inspect"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked()
                        && let Some(id) = self.selected
                    {
                        action = Some(Action::Inspect(id));
                    }
                    if widgets::action_button(
                        ui,
                        p,
                        "zoom-in-filled",
                        aurora_i18n::tr!("area-search-zoom"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked()
                        && let Some(id) = self.selected
                    {
                        action = Some(Action::Zoom(id));
                    }
                    if widgets::action_button(
                        ui,
                        p,
                        "copy-filled",
                        aurora_i18n::tr!("area-search-copy"),
                        widgets::ActionKind::Flat,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked()
                        && let Some(id) = self.selected
                    {
                        ui.ctx().copy_text(id.to_string());
                    }
                });
            });
        });
        action
    }
}

fn columns() -> [(Column, f32); 5] {
    [
        (Column::Distance, 0.12),
        (Column::Name, 0.29),
        (Column::Owner, 0.23),
        (Column::Description, 0.26),
        (Column::Price, 0.10),
    ]
}
