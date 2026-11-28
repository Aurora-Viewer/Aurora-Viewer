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
    if let Some(essentials) = essentials_switch(ui, p, state.essentials, total) {
        state.essentials = essentials;
        if state.essentials && !page::ESSENTIALS.contains(&state.state) {
            let first = Category::of(state.state)
                .states()
                .find(|s| page::ESSENTIALS.contains(s))
                .unwrap_or(State::Standing);
            state.point_at(first);
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
            // Each group opens again on the slot it had unfolded, else on
            // the one it showed last.
            let remembered = state.open_slots[category.index()]
                .or(state.category_states[category.index()])
                .filter(|s| states.contains(s));
            let target = remembered
                .or_else(|| states.iter().copied().find(|s| *s == category.initial_state()))
                .unwrap_or(states[0]);
            if !on {
                state.point_at(target);
            }
            state.tab = 0;
        }
    }
    // Movement and Stands and turns at the column's foot, as the board has it
    // (lower, near the column's foot)
    let foot = 30.0;
    ui.add_space((ui.available_height() - foot).max(4.0));
    let on = state.tab == 1;
    if movement_row(ui, p, on).clicked() {
        state.tab = 1;
    }
}

/// The set's name in large type, the number of sets, and its animation count
/// with "Loads at login" for the default set. A click opens "Your sets".
fn set_card(ui: &mut egui::Ui, p: &Palette, catalog: &inventory::Catalog, set: Option<&Set>, state: &mut AoUi) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CARD), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if response.hovered() || state.sets_open {
        ui.painter().rect_filled(rect, radius, super::shades(p).slot);
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
    // the set that loads at login: a quiet mark, as its row in the list has
    if set.is_some_and(|s| s.default) {
        let ink = p.muted.gamma_multiply(0.85);
        let galley = ui.painter().layout_no_wrap(
            aurora_i18n::tr!("ao-loads-at-login").to_owned(),
            egui::FontId::proportional(11.0),
            ink,
        );
        let text_x = inner.right() - galley.size().x;
        widgets::paint_row_text(ui.painter(), galley, text_x, bottom, ink);
        widgets::paint_glyph(ui.painter(), "login-2", egui::pos2(text_x - 9.0, bottom), 12.0, ink);
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
    let radius = ui.visuals().window_corner_radius;
    // the chosen group in the accent, as the board shows it
    if on {
        ui.painter().rect_filled(rect, radius, p.violet);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius, super::shades(p).slot);
    }
    let (ink, quiet) = if on {
        (p.on_accent, p.on_accent.gamma_multiply(0.85))
    } else {
        (p.ink, p.muted)
    };
    let inner = rect.shrink2(Vec2::new(10.0, 0.0));
    let count_galley = ui
        .painter()
        .layout_no_wrap(count.to_owned(), egui::FontId::proportional(11.5), quiet);
    let room = inner.width() - count_galley.size().x - 8.0;
    widgets::paint_row_text(
        ui.painter(),
        count_galley.clone(),
        inner.right() - count_galley.size().x,
        inner.center().y,
        quiet,
    );
    let title = egui::FontId::new(13.5, super::super::fonts::semibold(ui.ctx()));
    widgets::paint_elided(ui, name, egui::pos2(inner.left(), inner.center().y - 8.0), room, title, ink);
    widgets::paint_elided(
        ui,
        line,
        egui::pos2(inner.left(), inner.center().y + 9.0),
        room,
        egui::FontId::proportional(11.0),
        quiet,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
    response
}

/// Movement: its icon and name on one line, its EXPERIMENTAL tag at the
/// end; chosen, in the accent like a group.
fn movement_row(ui: &mut egui::Ui, p: &Palette, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), egui::Sense::click());
    let radius = ui.visuals().window_corner_radius;
    if on {
        ui.painter().rect_filled(rect, radius, p.violet);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, radius, super::shades(p).slot);
    }
    let ink = if on { p.on_accent } else { p.ink };
    let y = rect.center().y;
    widgets::paint_glyph(ui.painter(), "gauge-filled", egui::pos2(rect.left() + 18.0, y), 15.0, ink);
    let title = egui::FontId::new(13.5, super::super::fonts::semibold(ui.ctx()));
    let name = aurora_i18n::tr!("ao-tab-movement-short");
    let tag_left = page::experimental_tag(ui, p, egui::pos2(rect.right() - 10.0, y), on);
    widgets::paint_elided(
        ui,
        name,
        egui::pos2(rect.left() + 34.0, y),
        tag_left - rect.left() - 42.0,
        title,
        ink,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
    response
}

/// All slots or Essentials: two halves on an inset, the chosen one on a
/// pill that slides, raised for all of them and in the accent for the
/// essentials. Returns the choice when it changes.
fn essentials_switch(ui: &mut egui::Ui, p: &Palette, essentials: bool, total: usize) -> Option<bool> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), egui::Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(rect, radius, p.field);
    let inner = rect.shrink(3.0);
    let half = inner.width() / 2.0;
    let at = ui.ctx().animate_bool(ui.scope_id().with("ao_essentials_pill"), essentials);
    let pill = egui::Rect::from_min_size(egui::pos2(inner.left() + at * half, inner.top()), Vec2::new(half, inner.height()));
    let pill_fill = p.raised.lerp_to_gamma(p.violet, at);
    ui.painter()
        .rect_filled(pill, ui.visuals().widgets.inactive.corner_radius, pill_fill);
    let mut chosen = None;
    for (i, (label, count)) in [
        (aurora_i18n::tr!("ao-all-slots"), total),
        (aurora_i18n::tr!("ao-essentials"), page::ESSENTIALS.len()),
    ]
    .into_iter()
    .enumerate()
    {
        let side = egui::Rect::from_min_size(
            egui::pos2(inner.left() + i as f32 * half, inner.top()),
            Vec2::new(half, inner.height()),
        );
        let response = ui.interact(side, ui.scope_id().with(("ao_essentials", i)), egui::Sense::click());
        let on = (i == 1) == essentials;
        let ink = match (on, i) {
            (true, 1) => p.on_accent,
            (true, _) => p.ink,
            (false, _) if response.hovered() => p.ink,
            _ => p.muted,
        };
        let font = egui::FontId::new(12.5, super::super::fonts::semibold(ui.ctx()));
        let name = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
        let figure = ui
            .painter()
            .layout_no_wrap(count.to_string(), egui::FontId::proportional(11.0), ink.gamma_multiply(0.75));
        let width = name.size().x + 6.0 + figure.size().x;
        let x = side.center().x - width / 2.0;
        let name_w = name.size().x;
        widgets::paint_row_text(ui.painter(), name, x, side.center().y, ink);
        widgets::paint_row_text(ui.painter(), figure, x + name_w + 6.0, side.center().y, ink.gamma_multiply(0.75));
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, label));
        if response.clicked() && !on {
            chosen = Some(i == 1);
        }
    }
    chosen
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
    // an eyebrow, as the board's sections have them
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let eyebrow = egui::FontId::new(11.0, super::super::fonts::semibold(ui.ctx()));
        ui.label(
            RichText::new(aurora_i18n::tr!("ao-your-sets").to_uppercase())
                .font(eyebrow)
                .color(p.muted),
        );
        ui.label(RichText::new(catalog.sets.len().to_string()).size(11.0).color(p.muted_dim));
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
            if link(ui, p, "arrows-exchange", aurora_i18n::tr!("ao-migrate-link")).clicked() {
                state.migration = Some(Default::default());
            }
        });
        if link(ui, p, "refresh", aurora_i18n::tr!("ao-reload")).clicked() {
            out.push(Action::Reload);
        }
    });
}

/// A quiet way, for the less used ones in the sets panel: its icon and
/// name, lit under the pointer.
fn link(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let galley = widgets::elided_galley(ui, RichText::new(label).font(font), ui.available_width() - 26.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + 26.0, 22.0), egui::Sense::click());
    let enabled = ui.is_enabled();
    let ink = if !enabled {
        p.muted_dim
    } else if response.hovered() {
        p.violet_light
    } else {
        p.muted
    };
    widgets::paint_glyph(ui.painter(), icon, egui::pos2(rect.left() + 10.0, rect.center().y), 13.0, ink);
    widgets::paint_row_text(ui.painter(), galley, rect.left() + 24.0, rect.center().y, ink);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response
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
        let response = set_line(ui, p, set, active, busy);
        if response.clicked() && !busy && !active {
            account.selected = set.id;
            state.sets_open = false;
            out.push(Action::Preferences);
        }
        set_menu(ui, p, inv, catalog, set, state, busy, response.rect, out);
    }
    if state.confirm_delete == Some(set.id) {
        widgets::surface(ui, p, |ui| {
            ui.add(egui::Label::new(aurora_i18n::tr!("ao-delete-set-confirm", set = set.name.as_str())).wrap());
            ui.horizontal(|ui| {
                let size = Vec2::new(90.0, 26.0);
                if widgets::action_button_if(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("common-delete"),
                    widgets::ActionKind::Danger,
                    size,
                    // (an edit under way would drop it: it waits)
                    !busy,
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

/// The set in use's own options in a grouped card under its row: each its
/// icon, name and what it does, its switch on the right; a click anywhere
/// on a row switches it.
fn set_options(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, set: &Set, state: &mut AoUi, busy: bool, out: &mut Vec<Action>) {
    let (mut sits, mut smart, mut mouselook) = (set.sits, set.smart, set.mouselook);
    let mut changed = false;
    let shade = super::shades(p);
    // (tucked under the row it belongs to)
    ui.add_space(-ui.spacing().item_spacing.y + 2.0);
    ui.add_enabled_ui(!busy, |ui| {
        egui::Frame::new()
            .fill(shade.slot.lerp_to_gamma(p.violet, 0.05))
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::symmetric(4, 4))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                changed |= option(ui, p, "armchair", "ao-sits", "ao-sits-line", &mut sits);
                changed |= option(ui, p, "sparkles-filled", "ao-smart", "ao-smart-line", &mut smart);
                changed |= option(ui, p, "crosshair", "ao-mouselook", "ao-mouselook-line", &mut mouselook);
            });
    });
    if changed {
        let mut edit = set.clone();
        (edit.sits, edit.smart, edit.mouselook) = (sits, smart, mouselook);
        push(out, state, inventory::save_set(inv, &edit));
    }
}

/// One option of the set: its icon, its name over what it does, its switch.
fn option(ui: &mut egui::Ui, p: &Palette, icon: &str, key: &'static str, line: &'static str, value: &mut bool) -> bool {
    let width = ui.available_width();
    let text_x = 30.0;
    let text_w = width - text_x - 52.0;
    let name = widgets::elided_galley(ui, RichText::new(aurora_i18n::text(key)).size(12.5), text_w);
    let what = ui.painter().layout(
        aurora_i18n::text(line).to_owned(),
        egui::FontId::proportional(11.0),
        p.muted,
        text_w,
    );
    let height = (10.0 + name.size().y + 1.0 + what.size().y + 10.0).max(40.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let enabled = ui.is_enabled();
    if response.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.45));
    }
    let lit = if *value { p.violet_pale } else { p.muted };
    let top = rect.top() + 10.0;
    widgets::paint_glyph(
        ui.painter(),
        icon,
        egui::pos2(rect.left() + 14.0, top + name.size().y / 2.0),
        14.0,
        lit,
    );
    let name_h = name.size().y;
    widgets::paint_row_text(ui.painter(), name, rect.left() + text_x, top + name_h / 2.0, p.ink);
    ui.painter()
        .galley(egui::pos2(rect.left() + text_x, top + name_h + 1.0), what, p.muted);
    // the switch, on top of the row
    let switch = egui::Rect::from_min_max(egui::pos2(rect.right() - 48.0, rect.top()), rect.right_bottom());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(switch)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.add_space(6.0);
    let mut changed = widgets::switch(&mut child, p, value).changed();
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Switch, enabled, *value, aurora_i18n::text(key)));
    if response.clicked() && enabled {
        *value = !*value;
        changed = true;
    }
    changed
}

/// A set's line in "Your sets": its crest (its first letter on a tile of its
/// own colours), its name over its count, the login mark of the set that
/// loads at login. The set in use is lit in the accent.
fn set_line(ui: &mut egui::Ui, p: &Palette, set: &Set, active: bool, busy: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), egui::Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let shade = super::shades(p);
    let hovered = response.hovered() && !busy;
    if active {
        ui.painter().rect(
            rect,
            radius,
            shade.item.lerp_to_gamma(p.violet, 0.16),
            egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.35)),
            egui::StrokeKind::Inside,
        );
        let edge = egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 9.0), Vec2::new(3.0, rect.height() - 18.0));
        ui.painter().rect_filled(edge, radius, p.violet_light);
    } else if hovered {
        ui.painter().rect_filled(rect, radius, shade.slot);
    }
    let tile = egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), Vec2::splat(30.0));
    crest(ui, p, tile, &set.name);
    let x = tile.right() + 10.0;
    // room for the login mark and the ⋯ button
    let room = rect.right() - x - 30.0 - if set.default { 22.0 } else { 0.0 };
    let title = egui::FontId::new(13.0, super::super::fonts::semibold(ui.ctx()));
    let ink = if active { p.ink } else { p.ink.gamma_multiply(0.92) };
    widgets::paint_elided(ui, &set.name, egui::pos2(x, rect.center().y - 8.0), room, title, ink);
    let mut line = aurora_i18n::tr!("ao-animation-count", count = page::animations(set));
    if active {
        line = format!("{line} · {}", aurora_i18n::tr!("ao-in-use"));
    }
    let line_ink = if active { p.violet_pale } else { p.muted };
    widgets::paint_elided(
        ui,
        &line,
        egui::pos2(x, rect.center().y + 9.0),
        room,
        egui::FontId::proportional(11.5),
        line_ink,
    );
    if set.default {
        let at = egui::pos2(rect.right() - 42.0, rect.center().y);
        widgets::paint_glyph(ui.painter(), "login-2", at, 14.0, if active { p.violet_pale } else { p.muted });
        let mark = egui::Rect::from_center_size(at, Vec2::splat(20.0));
        ui.interact(mark, ui.scope_id().with("ao_login_mark"), egui::Sense::hover())
            .on_hover_text(aurora_i18n::tr!("ao-loads-at-login"));
    }
    let label = if set.default {
        format!("{}, {}", set.name, aurora_i18n::tr!("ao-loads-at-login"))
    } else {
        set.name.clone()
    };
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, !busy, active, &label));
    response
}

/// A set's crest: its first letter or digit on a tile in two of the theme's
/// colours, chosen by its name, so each set keeps its own.
fn crest(ui: &egui::Ui, p: &Palette, tile: egui::Rect, name: &str) {
    let hues = [p.violet, p.indigo, p.teal, p.amber, p.rose, p.indigo_light];
    let hash = name.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let base = hues[hash as usize % hues.len()];
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(tile, radius, base.lerp_to_gamma(p.bar, 0.45));
    // a sheen on its top half
    let top = egui::Rect::from_min_max(tile.min, egui::pos2(tile.right(), tile.center().y));
    let mut upper = radius;
    (upper.sw, upper.se) = (0, 0);
    ui.painter().rect_filled(top, upper, p.ink.gamma_multiply(0.07));
    let letter = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "·".to_owned(), |c| c.to_uppercase().collect());
    let font = egui::FontId::new(14.0, super::super::fonts::semibold(ui.ctx()));
    ui.painter()
        .text(tile.center(), egui::Align2::CENTER_CENTER, letter, font, p.on_accent);
}

/// A set's ⋯ button, at the right end of its line, and its menu.
#[allow(clippy::too_many_arguments)]
fn set_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    set: &Set,
    state: &mut AoUi,
    busy: bool,
    line: egui::Rect,
    out: &mut Vec<Action>,
) {
    let more = egui::Rect::from_center_size(egui::pos2(line.right() - 16.0, line.center().y), Vec2::splat(26.0));
    let response = ui
        .interact(more, ui.scope_id().with("ao_set_more"), egui::Sense::click())
        .on_hover_text(aurora_i18n::tr!("ao-set-more"));
    if response.hovered() && !busy {
        ui.painter()
            .rect_filled(more, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let line_hovered = ui.rect_contains_pointer(line);
    let ink = if busy {
        p.muted_dim
    } else if response.hovered() {
        p.ink
    } else if line_hovered {
        p.muted
    } else {
        p.muted_dim
    };
    widgets::paint_glyph(ui.painter(), "dots", more.center(), 15.0, ink);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !busy, aurora_i18n::tr!("ao-set-more")));
    if busy {
        return;
    }
    super::super::menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        if super::super::menu::item(ui, p, "edit-filled", aurora_i18n::tr!("common-rename")) {
            state.name = set.name.clone();
            state.renaming = Some(set.id);
            state.confirm_delete = None;
        }
        if super::super::menu::item(ui, p, "copy-filled", aurora_i18n::tr!("ao-duplicate")) {
            // a copy of the set it is asked on, the set in use staying the same
            state.name = set.name.clone();
            state.new_dialog = Some(SetDialog::Clone(set.id));
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
