//! Live search and filter popouts leave the virtualized result table in place.
use super::*;
use crate::settings::area_search::{AreaSearch, DEFAULT_COLUMNS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Section {
    All,
    Search,
    Filters,
    Columns,
}

impl AreaSearchUi {
    pub(crate) fn demo_controls(&mut self, mode: &str) {
        self.reveal_filters = mode == "fields";
        self.filters_open = mode == "filters";
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

    pub(super) fn reset_section(&mut self, section: Section) {
        self.table.filters = match section {
            Section::All => Filters::default(),
            Section::Search => self.table.filters.without_search(),
            Section::Filters => {
                let f = &self.table.filters;
                Filters {
                    query: f.query.clone(),
                    name: f.name.clone(),
                    description: f.description.clone(),
                    owner: f.owner.clone(),
                    group: f.group.clone(),
                    creator: f.creator.clone(),
                    last_owner: f.last_owner.clone(),
                    regex: f.regex,
                    ..Default::default()
                }
            }
            Section::Columns => {
                self.visible_columns = Some(DEFAULT_COLUMNS);
                return;
            }
        };
        self.table.prepare();
        self.check_selection();
    }

    pub(super) fn tool_popouts(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for section in [Section::Columns, Section::Filters, Section::Search] {
                let (icon, label, requested) = match section {
                    Section::Search => ("search-filled", aurora_i18n::tr!("area-search-search"), self.reveal_filters),
                    Section::Filters => (
                        "adjustments-horizontal-filled",
                        aurora_i18n::tr!("area-search-filters"),
                        self.filters_open,
                    ),
                    Section::Columns => ("layout-columns", aurora_i18n::tr!("area-search-columns"), self.demo_columns),
                    Section::All => continue,
                };
                let width = widgets::action_button_fit(ui, icon, label);
                let form_style = ui.style().clone();
                let response = widgets::action_button(ui, p, icon, label, widgets::ActionKind::Flat, Vec2::new(width, 26.0));
                if requested {
                    egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
                }
                let below = ui.ctx().content_rect().bottom() - response.rect.bottom() - 12.0;
                let body_height = (below - 44.0).clamp(80.0, 440.0);
                let open = menu::dropdown_placed(
                    &response,
                    p,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    Some((egui::RectAlign::BOTTOM_END, 4.0)),
                    |ui| {
                        if section != Section::Columns {
                            ui.set_style(form_style);
                        }
                        self.options(ui, p, section, body_height);
                    },
                );
                // Grid sizing may discard the first layout pass. Keep an
                // explicit open request through a later complete frame.
                let request_id = response.id.with("area_search_requested");
                let frame = ui.ctx().cumulative_frame_nr();
                let settled = if requested && open {
                    let first = ui.ctx().data_mut(|d| {
                        let first = d.get_temp::<u64>(request_id).unwrap_or(frame);
                        d.insert_temp(request_id, first);
                        first
                    });
                    frame > first
                } else {
                    false
                };
                if !requested {
                    ui.ctx().data_mut(|d| d.remove::<u64>(request_id));
                }
                if settled {
                    match section {
                        Section::Search => self.reveal_filters = false,
                        Section::Filters => self.filters_open = false,
                        Section::Columns => self.demo_columns = false,
                        Section::All => {}
                    }
                }
            }
        });
    }

    fn options(&mut self, ui: &mut egui::Ui, p: &Palette, section: Section, body_height: f32) {
        let screen = ui.ctx().content_rect();
        let width = if section == Section::Columns { 240.0_f32 } else { 760.0 };
        ui.set_width(width.min((screen.width() - 24.0).max(240.0)));
        ui.spacing_mut().item_spacing = Vec2::new(8.0, if section == Section::Columns { 0.0 } else { 6.0 });
        egui::ScrollArea::vertical()
            .id_salt(("area_search_options", section))
            .max_height(body_height)
            .auto_shrink([false, true])
            .show(ui, |ui| match section {
                Section::Search => search_controls(ui, p, &mut self.table.filters),
                Section::Filters => filter_controls(ui, p, &mut self.table.filters),
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
            if section == Section::Filters && ui.button(aurora_i18n::tr!("area-search-save-defaults")).clicked() {
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

fn filter_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    f.normalize();
    if ui.available_width() >= 720.0 {
        ui.columns(2, |columns| {
            required_controls(&mut columns[0], p, f);
            excluded_controls(&mut columns[1], p, f);
        });
    } else {
        required_controls(ui, p, f);
        ui.add_space(12.0);
        excluded_controls(ui, p, f);
    }
    ui.add_space(12.0);
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
    ui.horizontal_wrapped(|ui| {
        widgets::checkbox(ui, p, &mut f.parcel_only, aurora_i18n::tr!("area-search-parcel"));
        widgets::checkbox(ui, p, &mut f.mine, aurora_i18n::tr!("area-search-mine"));
    });
    f.normalize();
}

fn required_controls(ui: &mut egui::Ui, p: &Palette, f: &mut Filters) {
    widgets::section_header(ui, p, "filter-filled", aurora_i18n::tr!("area-search-required"));
    let cols = 2;
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
