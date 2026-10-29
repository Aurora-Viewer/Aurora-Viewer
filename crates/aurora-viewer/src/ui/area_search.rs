//! Area Search: rezzed objects received by the viewer, following
//! FSAreaSearch::isSearchableObject / matchObject (indra/newview/fsareasearch.cpp).
//! Avatars are excluded; attachments and linked prims are optional.
//! The right-click menu of the results follows menu_fs_area_search.xml and
//! FSPanelAreaSearchList::onContextMenuItemClick / onContextMenuItemEnable.

mod controls;
mod selection;
mod table;
#[cfg(test)]
mod tests;
pub(crate) use selection::Selection;
pub(crate) use table::{Column, Filters, Row, Table};

use super::menu;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use egui::{RichText, Sense, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

pub const WINDOW: &str = "area_search";

#[derive(Default)]
pub struct AreaSearchUi {
    pub(crate) table: Table,
    visible_columns: Option<[bool; 10]>,
    demo_columns: bool,
    pub(crate) save_defaults: bool,
    pub(crate) selection: Selection,
    pub(crate) refresh: bool,
    pub(crate) filters_open: bool,
    /// Opens the search fields once, to show the filter the menu set.
    pub(crate) reveal_filters: bool,
    /// Where the right-click menu opened.
    menu_at: Option<egui::Pos2>,
    /// The result list the selection was last checked against.
    checked: u64,
    /// AURORA_DEMO_AREA_SEARCH=menu: right click the second row once shown.
    pub(crate) demo_menu: bool,
}

/// What the window asks of the viewer. Several objects: every selected row,
/// as the Legacy Viewer acts on getAllSelected().
pub enum Action {
    Inspect(Vec<Uuid>),
    Zoom(Uuid),
    ScriptInfo(Vec<Uuid>),
    Touch(Vec<Uuid>),
    Sit(Uuid),
    Teleport(Uuid),
    Edit(Vec<Uuid>),
    Return(Vec<Uuid>),
    Delete(Vec<Uuid>),
    Blacklist(Vec<Uuid>),
    Buy(Uuid),
    /// "Filter My Objects": the owner field set to our name.
    FilterMine,
}

impl AreaSearchUi {
    /// `draw_distance` enables Zoom (onContextMenuItemEnable "in_dd").
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, parcel_known: bool, draw_distance: f32) -> Option<Action> {
        let mut action = None;
        let mut window = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-area-search"),
            egui::pos2(250.0, 120.0),
            Vec2::new(800.0, 550.0),
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
            self.table.prepare();
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
            ui.horizontal(|ui| self.tool_popouts(ui, p));
            if let Some(error) = &self.table.regex_error {
                ui.label(RichText::new(aurora_i18n::tr!("area-search-regex-error", error = error.as_str())).color(p.warn));
            }
            self.table.prepare();
            self.check_selection();
            if self.table.filters.parcel_only && !parcel_known {
                ui.label(RichText::new(aurora_i18n::tr!("area-search-parcel-pending")).color(p.warn));
            }
            ui.label(RichText::new(aurora_i18n::tr!("area-search-scope")).size(11.0).color(p.muted));
            let (mut table_ui, footer) = widgets::above_footer(ui);
            let mut columns = self.columns();
            let minima: Vec<_> = columns.iter().map(|(column, _)| column_min_width(ui, *column)).collect();
            let minimum: f32 = minima.iter().sum();
            let table_width = table_ui.available_width().max(minimum + 16.0);
            egui::ScrollArea::horizontal()
                .id_salt("area_search_horizontal")
                .auto_shrink([false, false])
                .show(&mut table_ui, |ui| {
                    ui.set_width(table_width);

                    let width = widgets::header_width(ui, ui.scope_id().with("area_search_rows"));
                    let extra = (width - minimum).max(0.0);
                    for ((_, fraction), min) in columns.iter_mut().zip(&minima) {
                        *fraction = (*min + extra * *fraction) / width;
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for &(column, fraction) in &columns {
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
                    self.check_selection();
                    if self.table.visible.is_empty() {
                        ui.label(RichText::new(aurora_i18n::tr!("area-search-empty")).color(p.muted));
                    }
                    let height = ui.available_height();
                    let list = egui::ScrollArea::vertical()
                        .id_salt("area_search_rows")
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show_rows(ui, 25.0, self.table.visible.len(), |ui, range| {
                            for i in range {
                                let row = &self.table.rows[self.table.visible[i]];
                                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 25.0), Sense::click());
                                let selected = self.selection.contains(&row.id);
                                if selected || response.hovered() {
                                    let color = if selected { p.violet.gamma_multiply(0.28) } else { p.raised };
                                    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, color);
                                }
                                let mut x = rect.left();
                                for &(column, fraction) in &columns {
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
                                    let m = ui.input(|i| i.modifiers);
                                    let (rows, visible) = (&self.table.rows, &self.table.visible);
                                    self.selection
                                        .click(row.id, m.command, m.shift, || visible.iter().map(|i| rows[*i].id).collect());
                                }
                                if response.double_clicked() && !ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                                    action = Some(Action::Zoom(row.id));
                                }
                                let demo = self.demo_menu && i == 1;
                                if response.secondary_clicked() || demo {
                                    self.demo_menu = false;
                                    self.selection.context_click(row.id);
                                    self.menu_at = Some(response.interact_pointer_pos().unwrap_or(rect.center()));
                                    egui::Popup::open_id(ui.ctx(), menu_id());
                                }
                                response.on_hover_text(format!("{}\n{}\n{}", row.name, row.description, row.id));
                            }
                        });
                    widgets::remember_scroll_bar(ui, ui.scope_id().with("area_search_rows"), &list);
                });
            if let Some(pos) = self.menu_at {
                let mut chosen = None;
                menu::context_menu_at(ui, menu_id(), pos, p, |ui| chosen = self.menu(ui, p, draw_distance));
                if !egui::Popup::is_id_open(ui.ctx(), menu_id()) {
                    self.menu_at = None;
                }
                if chosen.is_some() {
                    action = chosen;
                }
            }
            footer.show(ui, |ui| {
                let reset = controls::page_footer(ui, p, controls::Section::All, |ui| {
                    let single = self.selection.single();
                    let ids = &self.selection.ids;
                    ui.add_enabled_ui(!ids.is_empty(), |ui| {
                        if widgets::action_button(
                            ui,
                            p,
                            "zoom-scan-filled",
                            aurora_i18n::tr!("area-search-inspect"),
                            widgets::ActionKind::Flat,
                            Vec2::new(0.0, 28.0),
                        )
                        .clicked()
                        {
                            action = Some(Action::Inspect(ids.clone()));
                        }
                        if ui
                            .add_enabled_ui(single.is_some(), |ui| {
                                widgets::action_button(
                                    ui,
                                    p,
                                    "zoom-in-filled",
                                    aurora_i18n::tr!("area-search-zoom"),
                                    widgets::ActionKind::Flat,
                                    Vec2::new(0.0, 28.0),
                                )
                            })
                            .inner
                            .clicked()
                            && let Some(id) = single
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
                        {
                            let ids: Vec<_> = ids.iter().map(Uuid::to_string).collect();
                            ui.ctx().copy_text(ids.join("\n"));
                        }
                    });
                });
                if reset {
                    self.reset_section(controls::Section::All);
                }
            });
        });
        action
    }

    /// Selected rows that were filtered out or left the list cannot be
    /// acted on; checked once per new result list.
    fn check_selection(&mut self) {
        if self.checked == self.table.generation {
            return;
        }
        self.checked = self.table.generation;
        if self.selection.is_empty() {
            return;
        }
        let visible: HashSet<_> = self.table.visible.iter().map(|i| self.table.rows[*i].id).collect();
        self.selection.retain(&visible);
    }

    /// menu_fs_area_search.xml with its enable rules (onContextMenuItemEnable):
    /// "one" for Sit, Teleport to and Buy, "in_dd" for Zoom. Buy also needs
    /// the object to be for sale; the Legacy Viewer only says it is not
    /// after the click.
    fn menu(&mut self, ui: &mut egui::Ui, p: &Palette, draw_distance: f32) -> Option<Action> {
        let ids = &self.selection.ids;
        let any = !ids.is_empty();
        let one = self.selection.single();
        let row = one.and_then(|id| self.table.rows.iter().find(|r| r.id == id));
        let in_dd = row.is_some_and(|r| r.distance < draw_distance);
        let for_sale = row.is_some_and(|r| r.price.is_some());
        // built on click: the selection can hold thousands of rows
        enum Make {
            Each(fn(Vec<Uuid>) -> Action),
            One(fn(Uuid) -> Action),
        }
        let entries = [
            (
                "code",
                aurora_i18n::tr!("ui-context-script-information"),
                any,
                Make::Each(Action::ScriptInfo),
            ),
            ("click-filled", aurora_i18n::tr!("ui-context-touch"), any, Make::Each(Action::Touch)),
            (
                "armchair",
                aurora_i18n::tr!("area-search-sit"),
                one.is_some(),
                Make::One(Action::Sit),
            ),
            (
                "location-filled",
                aurora_i18n::tr!("common-teleport-to"),
                one.is_some(),
                Make::One(Action::Teleport),
            ),
            ("zoom-in-filled", aurora_i18n::tr!("common-zoom-2"), in_dd, Make::One(Action::Zoom)),
            (
                "zoom-scan-filled",
                aurora_i18n::tr!("ui-context-inspect-2"),
                any,
                Make::Each(Action::Inspect),
            ),
            ("pencil-filled", aurora_i18n::tr!("common-edit"), any, Make::Each(Action::Edit)),
            (
                "arrow-back-up",
                aurora_i18n::tr!("ui-context-return"),
                any,
                Make::Each(Action::Return),
            ),
            ("trash-filled", aurora_i18n::tr!("common-delete"), any, Make::Each(Action::Delete)),
            (
                "eye-closed",
                aurora_i18n::tr!("ui-context-derender-blacklist"),
                any,
                Make::Each(Action::Blacklist),
            ),
            (
                "shopping-cart-filled",
                aurora_i18n::tr!("common-buy-object"),
                for_sale,
                Make::One(Action::Buy),
            ),
        ];
        let mut action = None;
        for (icon, label, enabled, make) in entries {
            if menu::item_if(ui, p, icon, label, enabled) {
                action = Some(match make {
                    Make::Each(make) => make(ids.clone()),
                    Make::One(make) => make(one.unwrap_or_default()),
                });
            }
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "filter-filled", aurora_i18n::tr!("area-search-filter-mine")) {
            action = Some(Action::FilterMine);
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "select-all", aurora_i18n::tr!("common-select-all")) {
            let all = self.table.visible.iter().map(|i| self.table.rows[*i].id).collect();
            self.selection.select_all(all);
        }
        if menu::item_if(ui, p, "deselect", aurora_i18n::tr!("area-search-clear-selection"), any) {
            self.selection.clear();
        }
        action
    }
}

fn menu_id() -> egui::Id {
    egui::Id::unique("area_search_menu")
}

impl AreaSearchUi {
    fn columns(&self) -> Vec<(Column, f32)> {
        let shown = self
            .visible_columns
            .unwrap_or([true, true, true, true, true, false, true, false, false, false]);
        let weights = [0.16, 0.29, 0.26, 0.10, 0.16, 0.10, 0.23, 0.23, 0.23, 0.23];
        let total: f32 = weights.iter().zip(shown).filter_map(|(w, on)| on.then_some(w)).sum();
        Column::ALL
            .into_iter()
            .zip(weights)
            .zip(shown)
            .filter_map(|((column, weight), on)| on.then_some((column, weight / total)))
            .collect()
    }
}

/// Keep optional columns readable. Overflow scrolls the header and virtualized
/// rows together rather than reducing numeric headers to indistinguishable dots.
fn column_min_width(ui: &egui::Ui, column: Column) -> f32 {
    let content: f32 = match column {
        Column::Name => 130.0,
        Column::Description => 150.0,
        Column::Owner | Column::Group | Column::Creator | Column::LastOwner => 140.0,
        Column::LandImpact | Column::PrimCount => 90.0,
        Column::Distance | Column::Price => 72.0,
    };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let label = ui.painter().layout_no_wrap(column.label().into(), font, ui.visuals().text_color());
    content.max(label.size().x + 2.0 * ui.spacing().button_padding.x + 24.0)
}
