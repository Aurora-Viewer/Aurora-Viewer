//! Inline search/filter expansion and a Columns popout preserve the result table.
use super::*;
use crate::settings::area_search::{AreaSearch, DEFAULT_COLUMNS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Section {
    All,
    Search,
    Filters,
    Ranges,
    Columns,
}

impl AreaSearchUi {
    pub(crate) fn demo_controls(&mut self, mode: &str) {
        self.expanded = match mode {
            "fields" => Some(Section::Search),
            "filters" | "scope" => Some(Section::Filters),
            "ranges" => Some(Section::Ranges),
            _ => None,
        };
        self.demo_columns = mode == "columns";
        if mode == "all-columns" {
            self.visible_columns = Some([true; 10]);
        }
        if mode == "link" {
            let f = Filters {
                name: aurora_i18n::tr!("demo-fountain").into(),
                ..Default::default()
            };
            if let Some(crate::slapp::AppLink::AreaSearch(filters)) = crate::slapp::parse(&f.link()) {
                self.open_filters(*filters);
            }
        }
    }

    pub(crate) fn from_saved(saved: &AreaSearch) -> Self {
        let mut saved = saved.clone();
        saved.normalize();
        let mut ui = Self::default();
        ui.table.filters = saved.filters;
        ui.visible_columns = Some(saved.columns);
        ui
    }

    pub(crate) fn needs_costs(&self) -> bool {
        self.saved_columns()[4] || self.table.sort == Column::LandImpact
    }
    pub(crate) fn saved_columns(&self) -> [bool; 10] {
        self.visible_columns.unwrap_or(DEFAULT_COLUMNS)
    }
    pub(crate) fn open_filters(&mut self, filters: Filters) {
        self.table.filters = filters;
        self.table.filters.normalize();
        self.refresh = true;
    }

    pub(crate) fn reveal_search(&mut self) {
        self.expanded = Some(Section::Search);
    }

    pub(super) fn reset_section(&mut self, section: Section) {
        let f = &mut self.table.filters;
        let defaults = Filters::default();
        match section {
            Section::All => *f = defaults,
            Section::Search => *f = f.without_search(),
            Section::Filters => {
                f.locked = defaults.locked;
                f.copy = defaults.copy;
                f.modify = defaults.modify;
                f.transfer = defaults.transfer;
                f.media = defaults.media;
                f.probe = defaults.probe;
                f.only_attachments = defaults.only_attachments;
                f.physical = defaults.physical;
                f.phantom = defaults.phantom;
                f.temporary = defaults.temporary;
                f.click_action = defaults.click_action;

                f.include_attachments = defaults.include_attachments;
                f.exclude_physical = defaults.exclude_physical;
                f.exclude_temporary = defaults.exclude_temporary;
                f.exclude_probes = defaults.exclude_probes;
                f.children = defaults.children;
                f.neighbors = defaults.neighbors;
                f.parcel_only = defaults.parcel_only;
                f.mine = defaults.mine;
            }
            Section::Ranges => {
                f.sale = defaults.sale;
                f.min_price = defaults.min_price;
                f.max_price = defaults.max_price;
                f.distance_on = defaults.distance_on;
                f.min_distance = defaults.min_distance;
                f.max_distance = defaults.max_distance;
            }
            Section::Columns => {
                self.visible_columns = Some(DEFAULT_COLUMNS);
                return;
            }
        }
        self.table.filters.normalize();
        self.table.prepare();
        self.check_selection();
    }
    pub(super) fn controls(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            let label = aurora_i18n::tr!("area-search-columns");
            let width = widgets::action_button_fit(ui, "layout-columns", label);
            let left = (ui.available_width() - width - ui.spacing().item_spacing.x).max(0.0);
            ui.allocate_ui_with_layout(Vec2::new(left, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(left);
                let sections = [None, Some(Section::Search), Some(Section::Filters), Some(Section::Ranges)];
                let mut selected = sections.iter().position(|section| *section == self.expanded).unwrap_or(0);
                if widgets::tabs(
                    ui,
                    p,
                    &mut selected,
                    &[
                        (aurora_i18n::tr!("area-search-results"), true),
                        (aurora_i18n::tr!("area-search-search"), true),
                        (aurora_i18n::tr!("area-search-filters"), true),
                        (aurora_i18n::tr!("area-search-range-options"), true),
                    ],
                ) {
                    self.expanded = sections[selected];
                }
            });
            let response = widgets::action_button(ui, p, "layout-columns", label, widgets::ActionKind::Flat, Vec2::new(width, 26.0));
            if self.demo_columns {
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
            }
            let below = ui.ctx().content_rect().bottom() - response.rect.bottom() - 12.0;
            let open = menu::dropdown_placed(
                &response,
                p,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                Some((egui::RectAlign::BOTTOM_END, 4.0)),
                |ui| self.options(ui, p, Section::Columns, (below - 44.0).clamp(80.0, 440.0)),
            );
            // A sizing pass can be discarded. Retain the demo's request
            // until the popup has survived a complete subsequent frame.
            let request_id = response.id.with("area_search_requested");
            let frame = ui.ctx().cumulative_frame_nr();
            if self.demo_columns && open {
                let first = ui.ctx().data_mut(|d| {
                    let first = d.get_temp::<u64>(request_id).unwrap_or(frame);
                    d.insert_temp(request_id, first);
                    first
                });
                if frame > first {
                    self.demo_columns = false;
                }
            } else if !self.demo_columns {
                ui.ctx().data_mut(|d| d.remove::<u64>(request_id));
            }
        });
        if let Some(section) = self.expanded {
            // The form scrolls independently, reserving rows and the action
            // footer below it rather than growing or covering the floater.
            let window_height = egui::AreaState::load(ui.ctx(), egui::Id::unique(WINDOW))
                .and_then(|state| state.size)
                .map_or(550.0, |size| size.y);
            // The measured floater bounds remain reliable while egui sizes
            // scroll content beyond the visible viewport. Keep its chrome,
            // form footer and result rows outside this scrolling form.
            let height = (window_height - 340.0).clamp(24.0, 440.0);
            self.options(ui, p, section, height);
        }
    }
    fn options(&mut self, ui: &mut egui::Ui, p: &Palette, section: Section, body_height: f32) {
        if section == Section::Columns {
            ui.set_width(240.0);
        }
        ui.spacing_mut().item_spacing = Vec2::new(8.0, if section == Section::Columns { 0.0 } else { 6.0 });
        egui::ScrollArea::vertical()
            .id_salt(("area_search_options", section))
            .max_height(body_height)
            // The default 64-point minimum would consume the table reservation.
            .min_scrolled_height(0.0)
            .auto_shrink([false, true])
            .show(ui, |ui| match section {
                Section::Search => search_controls(ui, p, &mut self.table.filters),
                Section::Filters => filter_controls(ui, p, &mut self.table.filters),
                Section::Ranges => range_controls(ui, p, &mut self.table.filters),
                Section::Columns => {
                    let shown = self.visible_columns.get_or_insert(DEFAULT_COLUMNS);
                    for (i, column) in Column::ALL.into_iter().enumerate() {
                        menu::toggle(ui, p, "", column.label(), &mut shown[i]);
                    }
                    if !shown.iter().any(|on| *on) {
                        shown[1] = true;
                    }
                }
                Section::All => {}
            });
        ui.add_space(8.0);
        let reset = page_footer(ui, p, section, |ui| {
            if section == Section::Search && ui.button(aurora_i18n::tr!("area-search-copy-link")).clicked() {
                ui.ctx().copy_text(self.table.filters.link());
            }
            if matches!(section, Section::Filters | Section::Ranges) && ui.button(aurora_i18n::tr!("area-search-save-defaults")).clicked() {
                self.save_defaults = true;
            }
        });
        if reset {
            self.reset_section(section);
        }
        self.table.prepare();
        self.check_selection();
    }
}

/// One footer layout for the result table and every popout; Reset always
/// occupies the rightmost 28-point row, independent of the other actions.
pub(super) fn page_footer(ui: &mut egui::Ui, p: &Palette, section: Section, actions: impl FnOnce(&mut egui::Ui)) -> bool {
    let label = aurora_i18n::tr!("common-reset");
    let width = widgets::action_button_fit(ui, "restore", label);
    let tip = match section {
        Section::All => aurora_i18n::tr!("area-search-reset-all-tip"),
        Section::Search => aurora_i18n::tr!("area-search-reset-search-tip"),
        Section::Filters => aurora_i18n::tr!("area-search-reset-options-tip"),
        Section::Ranges => aurora_i18n::tr!("area-search-reset-ranges-tip"),
        Section::Columns => aurora_i18n::tr!("area-search-reset-columns-tip"),
    };
    ui.horizontal(|ui| {
        let left = (ui.available_width() - width - ui.spacing().item_spacing.x).max(0.0);
        ui.allocate_ui_with_layout(Vec2::new(left, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_size(Vec2::new(left, 28.0));
            actions(ui);
        });
        widgets::action_button(ui, p, "restore", label, widgets::ActionKind::Flat, Vec2::new(width, 28.0))
            .on_hover_text(tip)
            .clicked()
    })
    .inner
}

fn search_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    let columns = if ui.available_width() >= 620.0 { 2 } else { 1 };
    let width = ((ui.available_width() - 20.0) / columns as f32).floor();
    egui::Grid::new("area_search_fields")
        .num_columns(columns)
        .spacing([16.0, 10.0])
        .show(ui, |ui| {
            for (i, (label, text)) in [
                (aurora_i18n::tr!("inspect-col-name"), &mut f.name),
                (aurora_i18n::tr!("inspect-col-description"), &mut f.description),
                (aurora_i18n::tr!("inspect-col-owner"), &mut f.owner),
                (aurora_i18n::tr!("area-search-group"), &mut f.group),
                (aurora_i18n::tr!("inspect-col-creator"), &mut f.creator),
                (aurora_i18n::tr!("area-search-last-owner"), &mut f.last_owner),
            ]
            .into_iter()
            .enumerate()
            {
                ui.vertical(|ui| {
                    ui.label(label);
                    widgets::filter_field(ui, p, text, egui::Id::unique(("area_search_field", i)), width);
                });
                if (i + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
    widgets::checkbox(ui, p, &mut f.regex, aurora_i18n::tr!("area-search-regex"));
}
fn inverse(ui: &mut egui::Ui, p: &Palette, value: &mut bool, label: &str) {
    let mut excluded = !*value;
    widgets::checkbox(ui, p, &mut excluded, label);
    *value = !excluded;
}

fn range_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    f.normalize();
    widgets::section_header(ui, p, "adjustments-horizontal-filled", aurora_i18n::tr!("area-search-ranges"));
    ui.horizontal_wrapped(|ui| {
        widgets::checkbox(ui, p, &mut f.sale, aurora_i18n::tr!("area-search-sale"));
        ui.add_enabled_ui(f.sale, |ui| {
            ui.label(aurora_i18n::tr!("area-search-min"));
            ui.add(egui::DragValue::new(&mut f.min_price).range(0..=999999999).prefix("L$ "));
            ui.label(aurora_i18n::tr!("area-search-max"));
            ui.add(egui::DragValue::new(&mut f.max_price).range(0..=999999999).prefix("L$ "));
        });
    });
    ui.horizontal_wrapped(|ui| {
        widgets::checkbox(ui, p, &mut f.distance_on, aurora_i18n::tr!("area-search-distance-range"));
        ui.add_enabled_ui(f.distance_on, |ui| {
            ui.label(aurora_i18n::tr!("area-search-min"));
            ui.add(
                egui::DragValue::new(&mut f.min_distance)
                    .range(0.0..=999999999.0)
                    .max_decimals(1)
                    .suffix(" m"),
            );
            ui.label(aurora_i18n::tr!("area-search-max"));
            ui.add(
                egui::DragValue::new(&mut f.max_distance)
                    .range(0.0..=999999999.0)
                    .max_decimals(1)
                    .suffix(" m"),
            );
        });
    });
    f.normalize();
}

fn filter_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    f.normalize();
    if ui.available_width() >= 720.0 {
        ui.columns(2, |columns| {
            properties_controls(&mut columns[0], p, f);
            scope_controls(&mut columns[1], p, f);
        });
    } else {
        properties_controls(ui, p, f);
        ui.add_space(12.0);
        scope_controls(ui, p, f);
    }
    f.normalize();
}
fn properties_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    f.normalize();
    required_controls(ui, p, f);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("area-search-click-action"));
        let labels = [
            aurora_i18n::tr!("area-search-click-unrestricted"),
            aurora_i18n::tr!("area-search-click-any"),
            aurora_i18n::tr!("ui-context-touch"),
            aurora_i18n::tr!("area-search-sit"),
            aurora_i18n::tr!("common-buy-object"),
            aurora_i18n::tr!("area-search-click-pay"),
            aurora_i18n::tr!("area-search-click-open"),
            aurora_i18n::tr!("area-search-click-play"),
            aurora_i18n::tr!("area-search-click-media"),
            aurora_i18n::tr!("common-zoom-2"),
        ];
        egui::ComboBox::from_id_salt("area_search_click_action")
            .selected_text(labels[usize::from(f.click_action)])
            .show_ui(ui, |ui| {
                for (i, label) in labels.iter().enumerate() {
                    ui.selectable_value(&mut f.click_action, i as u8, *label);
                }
            });
    });
    f.normalize();
}

fn scope_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    excluded_controls(ui, p, f);
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        widgets::checkbox(ui, p, &mut f.parcel_only, aurora_i18n::tr!("area-search-parcel"));
        widgets::checkbox(ui, p, &mut f.mine, aurora_i18n::tr!("area-search-mine"));
    });
    f.normalize();
}

fn required_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    widgets::section_header(ui, p, "filter-filled", aurora_i18n::tr!("area-search-required"));
    let cols = if ui.available_width() >= 620.0 { 3 } else { 2 };
    egui::Grid::new("area_search_required")
        .num_columns(cols)
        .spacing([16.0, 2.0])
        .show(ui, |ui| {
            for (i, (value, enabled, label)) in [
                (&mut f.locked, true, aurora_i18n::tr!("build-ui-object-locked")),
                (&mut f.physical, !f.exclude_physical, aurora_i18n::tr!("area-search-physical")),
                (&mut f.phantom, true, aurora_i18n::tr!("area-search-phantom")),
                (&mut f.temporary, !f.exclude_temporary, aurora_i18n::tr!("area-search-temporary")),
                (
                    &mut f.only_attachments,
                    f.include_attachments,
                    aurora_i18n::tr!("area-search-attachment"),
                ),
                (&mut f.media, true, aurora_i18n::tr!("area-search-media")),
                (&mut f.copy, true, aurora_i18n::tr!("area-search-copy-perm")),
                (&mut f.modify, true, aurora_i18n::tr!("area-search-modify-perm")),
                (&mut f.transfer, true, aurora_i18n::tr!("area-search-transfer-perm")),
                (&mut f.probe, !f.exclude_probes, aurora_i18n::tr!("area-search-probes")),
            ]
            .into_iter()
            .enumerate()
            {
                ui.add_enabled_ui(enabled, |ui| widgets::checkbox(ui, p, value, label));
                if (i + 1) % cols == 0 {
                    ui.end_row();
                }
            }
        });
}

fn excluded_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    widgets::section_header(ui, p, "eye-off", aurora_i18n::tr!("area-search-excluded"));
    egui::Grid::new("area_search_excluded")
        .num_columns(2)
        .spacing([16.0, 2.0])
        .show(ui, |ui| {
            for (i, (value, inverted, label)) in [
                (&mut f.include_attachments, true, aurora_i18n::tr!("area-search-attachment")),
                (&mut f.exclude_physical, false, aurora_i18n::tr!("area-search-physical")),
                (&mut f.exclude_temporary, false, aurora_i18n::tr!("area-search-temporary")),
                (&mut f.exclude_probes, false, aurora_i18n::tr!("area-search-probes")),
                (&mut f.children, true, aurora_i18n::tr!("area-search-child-prim")),
                (&mut f.neighbors, true, aurora_i18n::tr!("area-search-neighbor-regions")),
            ]
            .into_iter()
            .enumerate()
            {
                if inverted {
                    inverse(ui, p, value, label);
                } else {
                    widgets::checkbox(ui, p, value, label);
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
}
