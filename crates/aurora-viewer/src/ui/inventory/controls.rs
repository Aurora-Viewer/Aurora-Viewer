//! Firestorm inventory view menu and finder options, presented with Aurora widgets.
use super::view::{Age, Creator, Filters, InventoryPreferences, Links, SearchField, types};
use super::{Inventory, InventoryUi, Palette};
use crate::ui::widgets::{self, Floater, flat_button, switch, underlined_button};
use egui::{RichText, Vec2};

fn check(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut bool) -> bool {
    ui.horizontal(|ui| {
        let changed = switch(ui, p, value).changed();
        ui.label(RichText::new(label).color(p.ink));
        changed
    })
    .inner
}

pub(super) fn preferences_for_tab(ui: &mut egui::Ui, p: &Palette, prefs: &mut InventoryPreferences, tab: usize) -> bool {
    let mut changed = false;
    let sort = if tab == super::RECEIVED_TAB {
        &mut prefs.received_sort
    } else {
        &mut prefs.sort
    };
    ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-controls-display-order")).color(p.muted));
    ui.horizontal(|ui| {
        changed |= ui
            .selectable_value(&mut sort.by_date, true, aurora_i18n::tr!("ui-inventory-controls-newest-first"))
            .changed();
        changed |= ui
            .selectable_value(&mut sort.by_date, false, aurora_i18n::tr!("common-by-name"))
            .changed();
    });
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-folders-always-by-name"),
        &mut sort.folders_by_name,
    );
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-system-folders-first"),
        &mut sort.system_first,
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
    if prefs.show_recent {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            changed |= recent_period(ui, &mut prefs.recent_hours);
        });
    }
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
    ui.label(RichText::new(aurora_i18n::tr!("task-transfer-preferences")).color(p.muted));
    changed |= check(ui, p, aurora_i18n::tr!("task-transfer-warn-move"), &mut prefs.warn_task_move);
    changed |= check(
        ui,
        p,
        aurora_i18n::tr!("task-transfer-warn-scripted"),
        &mut prefs.warn_scripted_task_move,
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

/// How far back the Recent tab goes (`InventoryPreferences::recent_hours`):
/// since the last logout, or a set number of days and hours, like the
/// "Since Logoff" box and the spinners of floater_inventory_view_finder.xml.
pub fn recent_period(ui: &mut egui::Ui, hours: &mut u32) -> bool {
    let mut changed = false;
    let mut since_logout = *hours == 0;
    egui::ComboBox::from_id_salt(ui.scope_id().with("inventory_recent_period"))
        .selected_text(if since_logout {
            aurora_i18n::tr!("ui-inventory-controls-since-last-logout")
        } else {
            aurora_i18n::tr!("ui-inventory-recent-set-period")
        })
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(&mut since_logout, true, aurora_i18n::tr!("ui-inventory-controls-since-last-logout"))
                .changed();
            changed |= ui
                .selectable_value(&mut since_logout, false, aurora_i18n::tr!("ui-inventory-recent-set-period"))
                .changed();
        });
    if since_logout {
        *hours = 0;
        return changed;
    }
    // a period starts at a week, and never falls to 0 ("since logout")
    let total = if *hours == 0 {
        7 * 24
    } else {
        (*hours).min(super::view::RECENT_HOURS_MAX)
    };
    let (mut days, mut rest) = (total / 24, total % 24);
    changed |= ui
        .add(
            egui::DragValue::new(&mut days)
                .custom_parser(aurora_i18n::parse_number)
                .range(0..=10_000)
                .suffix(aurora_i18n::tr!("status-days")),
        )
        .changed();
    changed |= ui
        .add(
            egui::DragValue::new(&mut rest)
                .custom_parser(aurora_i18n::parse_number)
                .range(0..=23)
                .suffix(aurora_i18n::tr!("status-hours")),
        )
        .changed();
    *hours = (days * 24 + rest).max(1);
    changed
}

pub fn toolbar(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    init_filters(st, prefs);
    ui.horizontal_wrapped(|ui| {
        filter_actions(ui, p, st);
    });
}

/// A window starts with the filters kept by default.
pub fn init_filters(st: &mut InventoryUi, prefs: &InventoryPreferences) {
    if !st.filters_initialized {
        st.filters.clone_from(&prefs.filter_defaults);
        st.filters_initialized = true;
    }
}

fn filter_actions(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    if underlined_button(
        ui,
        p,
        aurora_i18n::tr!("ui-inventory-controls-filters"),
        st.filters_open || st.filters.active(),
    )
    .clicked()
    {
        st.filters_open = !st.filters_open;
    }
    if underlined_button(ui, p, aurora_i18n::tr!("common-preferences"), st.preferences_open).clicked() {
        st.preferences_open = !st.preferences_open;
    }
    tree_actions(ui, p, st);
}

fn tree_actions(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    if underlined_button(ui, p, aurora_i18n::tr!("common-collapse"), false)
        .on_hover_text(aurora_i18n::tr!("ui-inventory-controls-collapse-displayed-folders"))
        .clicked()
    {
        st.expand_all = Some(false);
    }
    if underlined_button(ui, p, aurora_i18n::tr!("common-expand"), false)
        .on_hover_text(aurora_i18n::tr!("ui-inventory-controls-expand-displayed-folders"))
        .clicked()
    {
        st.expand_all = Some(true);
    }
}

pub fn search(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences, hint: &str) {
    if st.advanced {
        advanced_search(ui, p, st);
        return;
    }
    let searching = !st.search.trim().is_empty();
    ui.horizontal(|ui| {
        let selector_width = 112.0_f32.min(ui.available_width() * 0.4);
        let field_width = (ui.available_width() - selector_width - ui.spacing().item_spacing.x).max(40.0);
        search_input(ui, p, st, hint, field_width);
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
                // A mode, not a field: it keeps the typed text as rules.
                if ui
                    .selectable_label(false, aurora_i18n::tr!("inventory-search-advanced-menu"))
                    .clicked()
                {
                    st.enter_advanced(prefs.search_field);
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!(
                "ui-inventory-controls-case-insensitive-search-combines-terms-quotes-search-for-an"
            ));
    });
    after_search(inv, st, searching);
}

/// The search line in advanced mode: the rules in one line (a click opens
/// the builder), the way back to quick search, and the saved searches.
fn advanced_search(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    let saved = super::smart_store::list();
    ui.horizontal(|ui| {
        let summary = st.filters.smart.as_ref().map(super::smart_builder::summary).unwrap_or_default();
        let reserve = 150.0 + if saved.is_empty() { 0.0 } else { 112.0 };
        let width = (ui.available_width() - reserve).max(60.0);
        let line = ui
            .scope(|ui| {
                ui.set_max_width(width);
                ui.add(
                    egui::Label::new(RichText::new(summary).color(p.ink))
                        .truncate()
                        .sense(egui::Sense::click()),
                )
            })
            .inner
            .on_hover_text(aurora_i18n::tr!("inventory-search-advanced-tip"));
        if line.clicked() {
            st.filters_open = true;
        }
        if !saved.is_empty() {
            let current = saved.iter().find(|s| Some(s.id) == st.active_smart).map(|s| s.name.clone());
            let mut open = None;
            egui::ComboBox::from_id_salt("inventory_smart_saved")
                .width(100.0)
                .truncate()
                .selected_text(current.unwrap_or_else(|| aurora_i18n::tr!("inventory-smart-section").to_owned()))
                .show_ui(ui, |ui| {
                    for s in &saved {
                        if ui.selectable_label(Some(s.id) == st.active_smart, &s.name).clicked() {
                            open = Some((s.id, s.query.clone()));
                        }
                    }
                });
            if let Some((id, query)) = open {
                st.open_smart(Some(id), query);
            }
        }
        if widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-search-quick")).clicked() {
            st.leave_advanced();
        }
    });
}

/// The filter field and its clear action: a search, the filters, or the
/// mode listing an item's links. Returns the field.
pub fn search_input(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, hint: &str, width: f32) -> egui::Response {
    let clear_shown = !st.search.is_empty() || st.filters.active() || st.links_filter.is_some();
    let tip = if st.links_filter.is_some() {
        aurora_i18n::tr!("ui-inventory-controls-exit-linked-mode")
    } else {
        aurora_i18n::tr!("common-clear")
    };
    let (field, cleared) = super::tree::search_field(ui, p, &mut st.search, hint, width, clear_shown, tip);
    if st.focus_search && !ui.is_sizing_pass() && ui.is_enabled() {
        st.focus_search = false;
        field.request_focus();
    }
    super::clipboard::demo_search(&field, st);
    if cleared {
        if st.links_filter.take().is_some() {
            if let Some(filters) = st.saved_filters.take() {
                st.filters = filters;
            }
        } else {
            st.search.clear();
            st.filters = Filters::default();
            st.saved_filters = None;
            if st.advanced {
                // Clearing empties the rules; the window stays in advanced mode.
                st.filters.smart = Some(Default::default());
                st.active_smart = None;
            }
        }
    }
    field
}

/// Once a search is emptied (`searching` it was before the field), the
/// plain list goes back to where it was, and to the selection.
pub fn after_search(inv: &Inventory, st: &mut InventoryUi, searching: bool) {
    if searching && st.search.trim().is_empty() {
        // The plain list shares its scroll area with the results: it goes back
        // to its place from before the search, unless a selection it shows is
        // revealed below (that reveal's offset wins).
        st.plain_scroll_restore = Some(st.plain_scroll);
    }
    if searching
        && st.search.trim().is_empty()
        && let Some(id) = st.selected.filter(|id| st.selection.contains(id))
    {
        st.reveal_item(inv, id);
    }
}

pub fn dialogs(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let screen = ctx.content_rect();
    let preferences_size = Vec2::new(410.0, 600.0_f32.min((screen.height() - 90.0).max(180.0)));
    // the rule builder's rows hold a field, an operator, a value, Not and
    // remove: wider than the quick filters' checkboxes
    let filters_w = if st.advanced { 520.0 } else { 410.0 };
    let filters_size = Vec2::new(filters_w, 710.0_f32.min((screen.height() - 90.0).max(180.0)));
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
                preferences_for_tab(ui, p, prefs, st.tab);
                ui.add_space(6.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-inventory-controls-preferences-kept-between-sessions"))
                        .size(12.0)
                        .color(p.muted),
                );
            });
    });
    // The body edits the whole window state in advanced mode, so the open flag
    // is borrowed on its own.
    let mut filters_open = st.filters_open;
    Floater::new(
        &format!("inventory_filters_{}", st.ui_id),
        aurora_i18n::tr!("ui-inventory-controls-inventory-filters"),
        screen.center() - filters_size / 2.0,
        filters_size,
    )
    .show(ctx, p, &mut filters_open, |ui| {
        let height = ui.available_height().min(ctx.content_rect().height() - 90.0).max(180.0);
        egui::ScrollArea::vertical()
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if st.advanced {
                    super::smart_builder::show(ui, p, st);
                    return;
                }
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
                let note = match prefs.recent_hours {
                    0 => aurora_i18n::tr!("ui-inventory-controls-recent-since-your-last-logout-24-hours-on-first").to_owned(),
                    hours => aurora_i18n::tr!("ui-inventory-recent-note-period", days = hours / 24, hours = hours % 24),
                };
                ui.label(RichText::new(note).size(12.0).color(p.muted));
            });
    });
    st.filters_open = filters_open;
}
