//! The AO window's left column: the set card, the "Your sets" panel it opens,
//! the All slots / Essentials switch, the five groups and Movement.
//!
//! The set actions are FloaterAO's (indra/newview/ao.cpp): the set list
//! (`onSelectSet`), Add (`onClickAdd`), Remove (`onClickRemove`, which asks
//! first, "RemoveAOSet"), the default set (`onCheckDefault`), Override sits,
//! Smart sits and Disable stands in mouselook (`onCheckOverrideSits`,
//! `onCheckSmart`, `onCheckDisableStands`), and Reload (`onClickReload`).
//! Rename, Duplicate, Import and Migrate are Aurora's, as before.
use super::category::Category;
use super::*;

/// Height of the set card's two lines.
const CARD: f32 = 58.0;
/// Height of a group's row.
const GROUP_ROW: f32 = 44.0;

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    runtime: &mut Runtime,
    account: &mut Account,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    ui.spacing_mut().item_spacing.y = 8.0;
    let selected = catalog.selected(account.selected);
    set_card(ui, p, catalog, selected, state);
    if state.sets_open {
        sets_panel(ui, p, inv, catalog, account, state, out);
        return;
    }
    let total: usize = Category::ALL.iter().map(|c| c.states().count()).sum();
    let mut chosen = usize::from(state.essentials);
    if widgets::count_tabs(
        ui,
        p,
        "ao_essentials",
        &mut chosen,
        &[
            (aurora_i18n::tr!("ao-all-slots"), total),
            (aurora_i18n::tr!("ao-essentials"), page::ESSENTIALS.len()),
        ],
    ) {
        state.essentials = chosen == 1;
        if state.essentials && !page::ESSENTIALS.contains(&state.state) {
            let first = Category::of(state.state)
                .states()
                .find(|s| page::ESSENTIALS.contains(s))
                .unwrap_or(State::Standing);
            state.select_state(first);
        }
    }
    let hint = if state.essentials {
        aurora_i18n::tr!("ao-essentials-hint", count = page::ESSENTIALS.len())
    } else {
        aurora_i18n::tr!("ao-all-slots-hint", count = total)
    };
    ui.label(RichText::new(hint).small().color(p.muted));
    let current = Category::of(state.state);
    for category in Category::ALL {
        let states: Vec<State> = page::shown(category, state.essentials).collect();
        if states.is_empty() {
            continue;
        }
        let filled = states.iter().filter(|s| selected.is_some_and(|set| page::filled(set, **s))).count();
        let on = state.tab == 0 && current == category;
        let count = format!("{filled}/{}", states.len());
        if group_row(ui, p, category.label(), category.description(), &count, on).clicked() {
            // Each group opens again on the slot it had open.
            let remembered = state.category_states[category.index()].filter(|s| states.contains(s));
            let target = remembered
                .or_else(|| states.iter().copied().find(|s| *s == category.initial_state()))
                .unwrap_or(states[0]);
            if !on {
                state.select_state(target);
            }
            state.tab = 0;
        }
    }
    ui.add_space(4.0);
    let on = state.tab == 1;
    let tag = aurora_i18n::tr!("ao-experimental");
    if group_row(
        ui,
        p,
        aurora_i18n::tr!("ao-tab-movement-short"),
        aurora_i18n::tr!("ao-movement-line"),
        tag,
        on,
    )
    .clicked()
    {
        state.tab = 1;
    }
    ui.add_space(4.0);
    // Stands and turns pause for the session only (AOEngine::setOverrideStands
    // is not saved either).
    toggle(ui, p, "ao-stands", &mut runtime.stands);
}

/// The set's name in large type, the number of sets, and its animation count
/// with "Loads at login" for the default set. A click opens "Your sets".
fn set_card(ui: &mut egui::Ui, p: &Palette, catalog: &inventory::Catalog, set: Option<&Set>, state: &mut AoUi) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CARD), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if response.hovered() || state.sets_open {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.6));
    }
    let inner = rect.shrink2(Vec2::new(10.0, 7.0));
    let painter = ui.painter();
    let name = set.map_or_else(|| aurora_i18n::tr!("ao-no-set").to_owned(), |s| s.name.clone());
    let sets = aurora_i18n::tr!("ao-set-count", count = catalog.sets.len());
    let small = egui::FontId::proportional(12.0);
    let sets_galley = painter.layout_no_wrap(sets.clone(), small.clone(), p.muted);
    let chevron = 16.0;
    let right = inner.right() - chevron;
    let top = inner.top() + 14.0;
    widgets::paint_glyph(
        painter,
        if state.sets_open { "chevron-up" } else { "chevron-down" },
        egui::pos2(inner.right() - chevron / 2.0, top),
        14.0,
        if response.hovered() { p.ink } else { p.muted },
    );
    widgets::paint_row_text(painter, sets_galley.clone(), right - sets_galley.size().x - 4.0, top, p.muted);
    let room = right - sets_galley.size().x - 12.0 - inner.left();
    let title = egui::FontId::new(21.0, super::super::fonts::semibold(ui.ctx()));
    widgets::paint_elided(ui, &name, egui::pos2(inner.left(), top), room, title, p.ink);
    let bottom = inner.bottom() - 8.0;
    let line = set.map_or_else(
        || aurora_i18n::tr!("ao-no-set-line").to_owned(),
        |s| aurora_i18n::tr!("ao-animation-count", count = page::animations(s)),
    );
    widgets::paint_elided(ui, &line, egui::pos2(inner.left(), bottom), inner.width() * 0.55, small, p.muted);
    if set.is_some_and(|s| s.default) {
        let tag = aurora_i18n::tr!("ao-loads-at-login").to_uppercase();
        let galley = ui
            .painter()
            .layout_no_wrap(tag, egui::FontId::new(10.5, super::super::fonts::semibold(ui.ctx())), p.violet_pale);
        widgets::paint_row_text(ui.painter(), galley.clone(), inner.right() - galley.size().x, bottom, p.violet_pale);
    }
    let response = response.on_hover_text(aurora_i18n::tr!("ao-sets-tip"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    if response.clicked() {
        state.sets_open = !state.sets_open;
        state.renaming = None;
        state.confirm_delete = None;
    }
}

/// A group of slots: its name, what it holds below, and its count on the
/// right. The chosen one stands out.
fn group_row(ui: &mut egui::Ui, p: &Palette, name: &str, line: &str, count: &str, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), GROUP_ROW), egui::Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if on {
        ui.painter().rect_filled(rect, radius, p.raised);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let inner = rect.shrink2(Vec2::new(10.0, 0.0));
    let count_galley = ui
        .painter()
        .layout_no_wrap(count.to_owned(), egui::FontId::proportional(11.5), p.muted);
    let room = inner.width() - count_galley.size().x - 8.0;
    widgets::paint_row_text(
        ui.painter(),
        count_galley.clone(),
        inner.right() - count_galley.size().x,
        inner.center().y,
        if on { p.ink } else { p.muted },
    );
    let title = egui::FontId::new(13.5, super::super::fonts::semibold(ui.ctx()));
    widgets::paint_elided(ui, name, egui::pos2(inner.left(), inner.center().y - 8.0), room, title, p.ink);
    widgets::paint_elided(
        ui,
        line,
        egui::pos2(inner.left(), inner.center().y + 9.0),
        room,
        egui::FontId::proportional(11.5),
        p.muted,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
    response
}

/// "Your sets": every set with its ⋯ menu, the open set's options, and the
/// ways to make or bring a set.
fn sets_panel(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    account: &mut Account,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(aurora_i18n::tr!("ao-your-sets")).strong());
        ui.label(RichText::new(catalog.sets.len().to_string()).color(p.muted));
    });
    let busy = state.busy() || catalog.loading;
    let height = (ui.available_height() - 112.0).max(120.0);
    egui::ScrollArea::vertical()
        .id_salt("ao_sets")
        .max_height(height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for set in &catalog.sets {
                ui.push_id(set.id, |ui| set_row(ui, p, inv, catalog, set, account, state, busy, out));
            }
            if catalog.sets.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("ao-empty")).color(p.muted));
            }
        });
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal(|ui| {
            let half = Vec2::new((ui.available_width() - ui.spacing().item_spacing.x) / 2.0, 28.0);
            if widgets::action_button(ui, p, "plus", aurora_i18n::tr!("ao-new"), widgets::ActionKind::Flat, half).clicked() {
                state.name.clear();
                state.new_dialog = Some(SetDialog::New);
            }
            if widgets::action_button(
                ui,
                p,
                "file-text-filled",
                aurora_i18n::tr!("ao-import"),
                widgets::ActionKind::Flat,
                half,
            )
            .clicked()
            {
                state.picker = Some(true);
                state.folder = inv.root;
            }
        });
        ui.add_enabled_ui(!state.startup_loading, |ui| {
            if link(ui, p, aurora_i18n::tr!("ao-migrate-link")).clicked() {
                state.migration = Some(Default::default());
            }
        });
        if link(ui, p, aurora_i18n::tr!("ao-reload")).clicked() {
            out.push(Action::Reload);
        }
    });
}

/// A quiet text button, for the less used ways in the sets panel.
fn link(ui: &mut egui::Ui, p: &Palette, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(label).color(p.violet_pale)).frame(false))
}

#[allow(clippy::too_many_arguments)]
fn set_row(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    set: &Set,
    account: &mut Account,
    state: &mut AoUi,
    busy: bool,
    out: &mut Vec<Action>,
) {
    let active = set.id == account.selected;
    if state.renaming == Some(set.id) {
        rename_field(ui, inv, catalog, set, state, out);
    } else {
        // The ⋯ menu first, from the right, then the row takes the rest.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (more, response) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
                let response = response.on_hover_text(aurora_i18n::tr!("ao-set-more"));
                if response.hovered() && !busy {
                    ui.painter()
                        .rect_filled(more, ui.visuals().widgets.inactive.corner_radius, p.raised);
                }
                let ink = if busy { p.muted_dim } else { p.muted };
                widgets::paint_glyph(ui.painter(), "dots", more.center(), 15.0, ink);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !busy, aurora_i18n::tr!("ao-set-more")));
                if !busy {
                    super::super::menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                        if super::super::menu::item(ui, p, "edit-filled", aurora_i18n::tr!("common-rename")) {
                            state.name = set.name.clone();
                            state.renaming = Some(set.id);
                            state.confirm_delete = None;
                        }
                        if super::super::menu::item(ui, p, "copy-filled", aurora_i18n::tr!("ao-duplicate")) {
                            // Duplicate makes a copy of the set it is asked on.
                            account.selected = set.id;
                            state.selected_set = set.id;
                            state.name = set.name.clone();
                            state.new_dialog = Some(SetDialog::Clone);
                        }
                        if super::super::menu::check(ui, p, "login-2", aurora_i18n::tr!("ao-load-at-login"), set.default, true) {
                            set_default(inv, catalog, set, !set.default, state, out);
                        }
                        if super::super::menu::item(ui, p, "folder-filled", aurora_i18n::tr!("ao-show-in-inventory")) {
                            out.push(Action::ShowInInventory(set.id));
                        }
                        if super::super::menu::item(ui, p, "trash-filled", aurora_i18n::tr!("ao-delete-set")) {
                            state.confirm_delete = Some(set.id);
                            state.renaming = None;
                        }
                    });
                }
                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::click());
                let radius = ui.visuals().widgets.inactive.corner_radius;
                if active {
                    ui.painter().rect_filled(rect, radius, p.raised);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
                }
                let inner = rect.shrink2(Vec2::new(8.0, 0.0));
                let title = egui::FontId::new(13.0, super::super::fonts::semibold(ui.ctx()));
                widgets::paint_elided(
                    ui,
                    &set.name,
                    egui::pos2(inner.left(), inner.center().y - 8.0),
                    inner.width(),
                    title,
                    p.ink,
                );
                let mut line = aurora_i18n::tr!("ao-animation-count", count = page::animations(set));
                if active {
                    line = format!("{line} · {}", aurora_i18n::tr!("ao-in-use"));
                }
                if set.default {
                    line = format!("{line} · {}", aurora_i18n::tr!("ao-loads-at-login"));
                }
                widgets::paint_elided(
                    ui,
                    &line,
                    egui::pos2(inner.left(), inner.center().y + 9.0),
                    inner.width(),
                    egui::FontId::proportional(11.5),
                    p.muted,
                );
                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, !busy, active, &set.name));
                if response.clicked() && !busy && !active {
                    account.selected = set.id;
                    state.sets_open = false;
                    out.push(Action::Preferences);
                }
            })
        });
    }
    if state.confirm_delete == Some(set.id) {
        widgets::surface(ui, p, |ui| {
            ui.add(egui::Label::new(aurora_i18n::tr!("ao-delete-set-confirm", set = set.name.as_str())).wrap());
            ui.horizontal(|ui| {
                let size = Vec2::new(90.0, 26.0);
                if widgets::action_button(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("common-delete"),
                    widgets::ActionKind::Danger,
                    size,
                )
                .clicked()
                {
                    push(out, state, inventory::remove(inv, set.id));
                    state.confirm_delete = None;
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    state.confirm_delete = None;
                }
            });
        });
    }
    if active {
        set_options(ui, p, inv, set, state, busy, out);
    }
}

/// The set's name edited in place: Enter saves, Escape or a click away
/// cancels.
fn rename_field(ui: &mut egui::Ui, inv: &Inventory, catalog: &inventory::Catalog, set: &Set, state: &mut AoUi, out: &mut Vec<Action>) {
    let response = widgets::restricted_text_field(ui, true, &mut state.name, widgets::TextPolicy::AoName, |b, _| {
        egui::TextEdit::singleline(b).desired_width(f32::INFINITY)
    });
    if !response.has_focus() && !response.lost_focus() {
        response.request_focus();
    }
    let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter {
        match inventory::valid_name(&state.name) {
            Ok(name) if name == set.name => state.renaming = None,
            Ok(name) => {
                if catalog.sets.iter().any(|s| s.id != set.id && s.name == name) {
                    state.message = aurora_i18n::tr!("ao-name-or-loading").into();
                } else {
                    let mut edit = set.clone();
                    edit.name = name;
                    push(out, state, inventory::save_set(inv, &edit));
                    state.renaming = None;
                }
            }
            Err(reason) => state.message = reason,
        }
    } else if response.lost_focus() {
        state.renaming = None;
    }
}

/// Loads at login: only one set does, so turning it on for this set turns it
/// off for the one that had it (FloaterAO::onCheckDefault,
/// AOEngine::setDefaultSet).
fn set_default(inv: &Inventory, catalog: &inventory::Catalog, set: &Set, on: bool, state: &mut AoUi, out: &mut Vec<Action>) {
    let mut change = Mutation::default();
    let result: Result<(), String> = (|| {
        for old in catalog.sets.iter().filter(|s| s.id == set.id || (on && s.default)) {
            let mut edit = old.clone();
            edit.default = on && old.id == set.id;
            let saved = inventory::save_set(inv, &edit)?;
            change.operations.extend(saved.operations);
            change.refresh.extend(saved.refresh);
        }
        Ok(())
    })();
    push(out, state, result.map(|()| change));
}

/// The open set's own options, under its row.
fn set_options(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, set: &Set, state: &mut AoUi, busy: bool, out: &mut Vec<Action>) {
    ui.add_enabled_ui(!busy, |ui| {
        ui.indent("ao_set_options", |ui| {
            let (mut sits, mut smart, mut mouselook) = (set.sits, set.smart, set.mouselook);
            let mut changed = false;
            changed |= option(ui, p, "ao-sits", "ao-sits-line", &mut sits);
            changed |= option(ui, p, "ao-smart", "ao-smart-line", &mut smart);
            changed |= option(ui, p, "ao-mouselook", "ao-mouselook-line", &mut mouselook);
            if changed {
                let mut edit = set.clone();
                (edit.sits, edit.smart, edit.mouselook) = (sits, smart, mouselook);
                push(out, state, inventory::save_set(inv, &edit));
            }
        });
    });
}

/// A set option: its switch, its name, and what it does below.
fn option(ui: &mut egui::Ui, p: &Palette, key: &'static str, line: &'static str, value: &mut bool) -> bool {
    let changed = toggle(ui, p, key, value);
    ui.add(egui::Label::new(RichText::new(aurora_i18n::text(line)).small().color(p.muted)).wrap());
    changed
}
