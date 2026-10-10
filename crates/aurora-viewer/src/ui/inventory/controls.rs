//! Firestorm inventory view menu and finder options, presented with Aurora widgets.
use super::view::{Age, Creator, Filters, InventoryPreferences, Links, SearchField, types};
use super::{InventoryUi, Palette};
use crate::ui::widgets::{Floater, flat_button, switch};
use egui::{RichText, Vec2};

fn check(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut bool) -> bool {
    ui.horizontal(|ui| {
        let changed = switch(ui, p, value).changed();
        ui.label(RichText::new(label).color(p.ink));
        changed
    })
    .inner
}

pub fn preferences(ui: &mut egui::Ui, p: &Palette, prefs: &mut InventoryPreferences) -> bool {
    let mut changed = false;
    ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-controls-display-order")).color(p.muted));
    ui.horizontal(|ui| {
        changed |= ui
            .selectable_value(
                &mut prefs.sort.by_date,
                true,
                aurora_i18n::tr!("ui-inventory-controls-newest-first"),
            )
            .changed();
        changed |= ui
            .selectable_value(&mut prefs.sort.by_date, false, aurora_i18n::tr!("common-by-name"))
            .changed();
    });
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-folders-always-by-name"),
        &mut prefs.sort.folders_by_name,
    );
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-system-folders-first"),
        &mut prefs.sort.system_first,
    );
    ui.add_space(6.0);
    ui.label(RichText::new(aurora_i18n::tr!("common-display")).color(p.muted));
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-show-library"),
        &mut prefs.show_library,
    );
    changed |= check(ui, p, aurora_i18n::tr!("ui-inventory-controls-recent-tab"), &mut prefs.show_recent);
    changed |= check(ui, p, aurora_i18n::tr!("ui-inventory-controls-worn-tab"), &mut prefs.show_worn);
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-favorites-tab"),
        &mut prefs.show_favorites,
    );
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-separate-search-in-each-tab"),
        &mut prefs.separate_searches,
    );
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-double-click-add-objects"),
        &mut prefs.double_click_add_objects,
    );
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-double-click-add-clothes"),
        &mut prefs.double_click_add_clothes,
    );
    ui.add_space(6.0);
    ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-controls-also-search-in")).color(p.muted));
    changed |= check(ui, p, aurora_i18n::tr!("ui-inventory-controls-library"), &mut prefs.search_library);
    changed |= check(ui, p, aurora_i18n::tr!("ui-inventory-controls-trash"), &mut prefs.search_trash);
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-outfit-folders"),
        &mut prefs.search_outfits,
    );
    changed
}

pub fn toolbar(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    if !st.filters_initialized {
        st.filters.clone_from(&prefs.filter_defaults);
        st.filters_initialized = true;
    }
    let filter_label = if st.filters.active() {
        aurora_i18n::tr!("ui-inventory-controls-filters-2")
    } else {
        aurora_i18n::tr!("ui-inventory-controls-filters")
    };
    let clear_label = if st.links_filter.is_some() {
        aurora_i18n::tr!("ui-inventory-controls-exit-linked-mode")
    } else if !st.search.is_empty() || st.filters.active() {
        aurora_i18n::tr!("common-clear")
    } else {
        ""
    };
    let labels = [
        filter_label,
        aurora_i18n::tr!("common-preferences"),
        clear_label,
        aurora_i18n::tr!("common-collapse"),
        aurora_i18n::tr!("common-expand"),
    ];
    let width: f32 = labels
        .iter()
        .filter(|label| !label.is_empty())
        .map(|label| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap((*label).into(), egui::FontId::proportional(12.0), p.ink)
                    .size()
                    .x
            }) + 24.0
                + ui.spacing().item_spacing.x
        })
        .sum();
    if ui.available_width() < width {
        ui.horizontal_wrapped(|ui| filter_actions(ui, p, st));
        ui.horizontal(|ui| tree_actions(ui, p, st));
    } else {
        ui.horizontal(|ui| {
            filter_actions(ui, p, st);
            tree_actions(ui, p, st);
        });
    }
}

fn filter_actions(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    if flat_button(
        ui,
        p,
        if st.filters.active() {
            aurora_i18n::tr!("ui-inventory-controls-filters-2")
        } else {
            aurora_i18n::tr!("ui-inventory-controls-filters")
        },
    )
    .clicked()
    {
        st.filters_open = !st.filters_open;
    }
    if flat_button(ui, p, aurora_i18n::tr!("common-preferences")).clicked() {
        st.preferences_open = !st.preferences_open;
    }
    if st.links_filter.is_some() {
        if flat_button(ui, p, aurora_i18n::tr!("ui-inventory-controls-exit-linked-mode")).clicked() {
            st.links_filter = None;
            if let Some(filters) = st.saved_filters.take() {
                st.filters = filters;
            }
        }
    } else if (!st.search.is_empty() || st.filters.active()) && flat_button(ui, p, aurora_i18n::tr!("common-clear")).clicked() {
        st.search.clear();
        st.filters = Filters::default();
        st.links_filter = None;
        st.saved_filters = None;
    }
}

fn tree_actions(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if flat_button(ui, p, aurora_i18n::tr!("common-expand"))
            .on_hover_text(aurora_i18n::tr!("ui-inventory-controls-expand-displayed-folders"))
            .clicked()
        {
            st.expand_all = Some(true);
        }
        if flat_button(ui, p, aurora_i18n::tr!("common-collapse"))
            .on_hover_text(aurora_i18n::tr!("ui-inventory-controls-collapse-displayed-folders"))
            .clicked()
        {
            st.expand_all = Some(false);
        }
    });
}

pub fn search(ui: &mut egui::Ui, st: &mut InventoryUi, prefs: &mut InventoryPreferences, hint: &str) {
    ui.horizontal(|ui| {
        let selector_width = 112.0_f32.min(ui.available_width() * 0.4);
        let field_width = (ui.available_width() - selector_width - ui.spacing().item_spacing.x).max(40.0);
        ui.add_sized(
            [field_width, ui.spacing().interact_size.y],
            egui::TextEdit::singleline(&mut st.search).hint_text(hint),
        );
        egui::ComboBox::from_id_salt("inventory_search_field")
            .width(selector_width)
            .truncate()
            .selected_text(match prefs.search_field {
                SearchField::Name => aurora_i18n::tr!("common-name"),
                SearchField::Description => aurora_i18n::tr!("ui-inventory-controls-description"),
                SearchField::Creator => aurora_i18n::tr!("common-creator"),
                SearchField::Uuid => "UUID",
                SearchField::All => aurora_i18n::tr!("ui-inventory-controls-all-fields"),
            })
            .show_ui(ui, |ui| {
                for (field, label) in [
                    (SearchField::Name, aurora_i18n::tr!("common-name")),
                    (SearchField::Description, aurora_i18n::tr!("ui-inventory-controls-description")),
                    (SearchField::Creator, aurora_i18n::tr!("common-creator")),
                    (SearchField::Uuid, "UUID"),
                    (SearchField::All, aurora_i18n::tr!("ui-inventory-controls-all-fields")),
                ] {
                    ui.selectable_value(&mut prefs.search_field, field, label);
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!(
                "ui-inventory-controls-case-insensitive-search-combines-terms-quotes-search-for-an"
            ));
    });
}

pub fn dialogs(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let screen = ctx.content_rect();
    let preferences_size = Vec2::new(410.0, 600.0_f32.min((screen.height() - 90.0).max(180.0)));
    let filters_size = Vec2::new(410.0, 710.0_f32.min((screen.height() - 90.0).max(180.0)));
    Floater::new(
        &format!("inventory_preferences_{}", st.ui_id),
        aurora_i18n::tr!("ui-inventory-controls-inventory-preferences"),
        screen.center() - preferences_size / 2.0,
        preferences_size,
    )
    .show(ctx, p, &mut st.preferences_open, |ui| {
        let height = ui.available_height().min(ctx.content_rect().height() - 90.0).max(180.0);
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                preferences(ui, p, prefs);
                ui.add_space(6.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-inventory-controls-preferences-kept-between-sessions"))
                        .size(12.0)
                        .color(p.muted),
                );
            });
    });
    Floater::new(
        &format!("inventory_filters_{}", st.ui_id),
        aurora_i18n::tr!("ui-inventory-controls-inventory-filters"),
        screen.center() - filters_size / 2.0,
        filters_size,
    )
    .show(ctx, p, &mut st.filters_open, |ui| {
        let height = ui.available_height().min(ctx.content_rect().height() - 90.0).max(180.0);
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let f = &mut st.filters;
                ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-controls-item-types")).color(p.muted));
                ui.horizontal(|ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("common-all")).clicked() {
                        f.types = u32::MAX;
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("common-none")).clicked() {
                        f.types = 0;
                    }
                });
                egui::Grid::new("inventory_types").num_columns(2).show(ui, |ui| {
                    for (n, (kind, label)) in types().iter().enumerate() {
                        let bit = 1_u32 << kind;
                        let mut on = f.types & bit != 0;
                        if check(ui, p, label, &mut on) {
                            if on {
                                f.types |= bit;
                            } else {
                                f.types &= !bit;
                            }
                            // Selecting every known type restores unknown types too.
                            if types().iter().all(|(kind, _)| f.types & (1_u32 << kind) != 0) {
                                f.types = u32::MAX;
                            }
                        }
                        if n % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(5.0);
                ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-controls-required-permissions")).color(p.muted));
                ui.horizontal(|ui| {
                    for (bit, label) in [
                        (super::rules::MODIFY, aurora_i18n::tr!("common-edit")),
                        (super::rules::COPY, aurora_i18n::tr!("common-copy-2")),
                        (super::rules::TRANSFER, aurora_i18n::tr!("common-transfer")),
                    ] {
                        let mut on = f.permissions & bit != 0;
                        if check(ui, p, label, &mut on) {
                            if on {
                                f.permissions |= bit;
                            } else {
                                f.permissions &= !bit;
                            }
                        }
                    }
                });
                egui::ComboBox::from_id_salt("inventory_links")
                    .selected_text(match f.links {
                        Links::Include => aurora_i18n::tr!("ui-inventory-controls-include-links"),
                        Links::Only => aurora_i18n::tr!("ui-inventory-controls-links-only"),
                        Links::Exclude => aurora_i18n::tr!("ui-inventory-controls-hide-links"),
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Links::Include, aurora_i18n::tr!("ui-inventory-controls-include-links")),
                            (Links::Only, aurora_i18n::tr!("ui-inventory-controls-links-only")),
                            (Links::Exclude, aurora_i18n::tr!("ui-inventory-controls-hide-links")),
                        ] {
                            ui.selectable_value(&mut f.links, v, label);
                        }
                    });
                egui::ComboBox::from_id_salt("inventory_creator")
                    .selected_text(match f.creator {
                        Creator::All => aurora_i18n::tr!("ui-inventory-controls-all-creators"),
                        Creator::Me => aurora_i18n::tr!("ui-inventory-controls-created-by-me"),
                        Creator::Others => aurora_i18n::tr!("ui-inventory-controls-created-by-others"),
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Creator::All, aurora_i18n::tr!("ui-inventory-controls-all-creators")),
                            (Creator::Me, aurora_i18n::tr!("ui-inventory-controls-created-by-me")),
                            (Creator::Others, aurora_i18n::tr!("ui-inventory-controls-created-by-others")),
                        ] {
                            ui.selectable_value(&mut f.creator, v, label);
                        }
                    });
                check(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-inventory-controls-also-show-folders-without-matching-items"),
                    &mut f.always_folders,
                );
                check(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-inventory-controls-coalesced-objects-only"),
                    &mut f.coalesced,
                );
                ui.add_space(5.0);
                ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-controls-creation-date")).color(p.muted));
                egui::ComboBox::from_id_salt("inventory_age")
                    .selected_text(match f.age {
                        Age::Any => aurora_i18n::tr!("ui-inventory-controls-all-dates"),
                        Age::SinceLogout => aurora_i18n::tr!("ui-inventory-controls-since-last-logout"),
                        Age::Newer => aurora_i18n::tr!("ui-inventory-controls-newer-than"),
                        Age::Older => aurora_i18n::tr!("ui-inventory-controls-older-than"),
                    })
                    .show_ui(ui, |ui| {
                        for (v, label) in [
                            (Age::Any, aurora_i18n::tr!("ui-inventory-controls-all-dates")),
                            (Age::SinceLogout, aurora_i18n::tr!("ui-inventory-controls-since-last-logout")),
                            (Age::Newer, aurora_i18n::tr!("ui-inventory-controls-newer-than")),
                            (Age::Older, aurora_i18n::tr!("ui-inventory-controls-older-than")),
                        ] {
                            ui.selectable_value(&mut f.age, v, label);
                        }
                    });
                if matches!(f.age, Age::Newer | Age::Older) {
                    let (mut days, mut hours) = (f.hours / 24, f.hours % 24);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut days)
                                .custom_parser(aurora_i18n::parse_number)
                                .range(0..=36500)
                                .suffix(aurora_i18n::tr!("status-days")),
                        );
                        ui.add(
                            egui::DragValue::new(&mut hours)
                                .custom_parser(aurora_i18n::parse_number)
                                .range(0..=23)
                                .suffix(aurora_i18n::tr!("status-hours")),
                        );
                    });
                    f.hours = days * 24 + hours;
                }
                ui.add_space(7.0);
                ui.horizontal(|ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("common-reset")).clicked() {
                        *f = Filters::default();
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("ui-inventory-controls-keep-by-default")).clicked() {
                        prefs.filter_defaults.clone_from(f);
                    }
                });
                ui.label(
                    RichText::new(aurora_i18n::tr!(
                        "ui-inventory-controls-recent-since-your-last-logout-24-hours-on-first"
                    ))
                    .size(12.0)
                    .color(p.muted),
                );
            });
    });
}
