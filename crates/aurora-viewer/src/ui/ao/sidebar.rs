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
/// A set's line in "Your sets", the gap between two, and its "Use" button.
const SET_ROW: f32 = 46.0;
const SET_GAP: f32 = 4.0;
const USE_W: f32 = 40.0;
/// The tiles that make a set.
const TILE_H: f32 = 86.0;

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
    let selected = state.edited(catalog, account);
    set_card(ui, p, catalog, selected, state);
    use_banner(ui, p, catalog, selected, account, out);
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
        widgets::paint_glyph(ui.painter(), "circle-check-filled", egui::pos2(text_x - 9.0, bottom), 12.0, ink);
    }
    let response = response.on_hover_text(aurora_i18n::tr!("ao-sets-tip"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    if response.clicked() {
        state.sets_open = !state.sets_open;
        state.renaming = None;
        state.confirm_delete = None;
    }
}

/// Under the set card while the open set is not the one in use: which one
/// plays, and "Use this set" (FloaterAO's Activate).
fn use_banner(
    ui: &mut egui::Ui,
    p: &Palette,
    catalog: &inventory::Catalog,
    open: Option<&Set>,
    account: &mut Account,
    out: &mut Vec<Action>,
) {
    let (Some(open), Some(active)) = (open, catalog.selected(account.selected)) else {
        return;
    };
    if open.id == active.id {
        return;
    }
    let shade = super::shades(p);
    egui::Frame::new()
        .fill(shade.slot.lerp_to_gamma(p.amber, 0.06))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ao-not-in-use", set = active.name.as_str()))
                        .size(11.5)
                        .color(p.muted),
                )
                .wrap(),
            );
            let size = Vec2::new(ui.available_width(), 28.0);
            if widgets::primary_button_sized(ui, p, aurora_i18n::tr!("ao-use-this-set"), size).clicked() {
                account.selected = open.id;
                out.push(Action::Preferences);
            }
        });
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

/// "Your sets": every set, dragged up and down to order them, opened by a
/// click and put in use by "Use"; the open set's options; the ways to make
/// or bring a set.
fn sets_panel(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    account: &mut Account,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    let busy = state.busy() || catalog.loading;
    let open = state.edited(catalog, account).map(|s| s.id);
    egui::ScrollArea::vertical()
        .id_salt("ao_sets")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            eyebrow(ui, p, aurora_i18n::tr!("ao-your-sets"), Some(catalog.sets.len()));
            if catalog.sets.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("ao-empty")).color(p.muted));
            }
            set_list(ui, p, inv, catalog, account, state, open, busy, out);
            if let Some(set) = state.confirm_delete.and_then(|id| catalog.sets.iter().find(|s| s.id == id)) {
                confirm_delete(ui, p, inv, set, state, busy, out);
            }
            if let Some(set) = open.and_then(|id| catalog.sets.iter().find(|s| s.id == id)) {
                ui.add_space(4.0);
                eyebrow(ui, p, aurora_i18n::tr!("ao-set-options"), None);
                set_options(ui, p, inv, set, state, busy, out);
            }
            ui.add_space(4.0);
            eyebrow(ui, p, aurora_i18n::tr!("ao-add-sets"), None);
            ui.add_enabled_ui(!busy, |ui| create_tiles(ui, p, inv, state));
            more_ways(ui, p, state, busy, out);
        });
}

/// A section's name in small capitals, with its count.
fn eyebrow(ui: &mut egui::Ui, p: &Palette, label: &str, count: Option<usize>) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let font = egui::FontId::new(11.0, super::super::fonts::semibold(ui.ctx()));
        ui.label(RichText::new(label.to_uppercase()).font(font).color(p.muted));
        if let Some(count) = count {
            ui.label(RichText::new(count.to_string()).size(11.0).color(p.muted_dim));
        }
    });
}

/// The sets, one line each, in the user's order: a drag lifts a line and
/// the others make room, as the Effects window's filters do
/// (`widgets::reorder`).
#[allow(clippy::too_many_arguments)]
fn set_list(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    account: &mut Account,
    state: &mut AoUi,
    open: Option<Uuid>,
    busy: bool,
    out: &mut Vec<Action>,
) {
    let sets = account.ordered(&catalog.sets);
    let keys: Vec<Uuid> = sets.iter().map(|s| s.id).collect();
    let width = ui.available_width();
    let responses: Vec<egui::Response> = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.y = SET_GAP;
            sets.iter()
                .map(|set| {
                    let (_, rect) = ui.allocate_space(Vec2::new(width, SET_ROW));
                    ui.interact(rect, ui.scope_id().with(("ao_set_row", set.id)), egui::Sense::click_and_drag())
                })
                .collect()
        })
        .inner;
    let id = ui.scope_id().with("ao_set_order");
    let still = busy || state.renaming.is_some();
    let layout = widgets::reorder::update(ui, id, &keys, &responses, !still, None);
    for (i, set) in sets.iter().enumerate() {
        let rect = layout.rect(i, responses[i].rect);
        let line = SetLine {
            open: Some(set.id) == open,
            active: set.id == state.active_set,
            quiet: layout.busy(),
        };
        if layout.lifted == Some(i) {
            widgets::reorder::lifted_card(ui, p, id.with(i), rect, |ui| {
                paint_set(ui, p, set, rect, &responses[i], line, true);
            });
            continue;
        }
        if state.renaming == Some(set.id) {
            rename_field(ui, rect, inv, catalog, set, state, out);
            continue;
        }
        let use_clicked = paint_set(ui, p, set, rect, &responses[i], line, busy);
        if use_clicked && !busy {
            account.selected = set.id;
            out.push(Action::Preferences);
        } else if responses[i].clicked() && !busy && !line.open {
            // opened to edit; it plays only once put in use
            state.editing = Some(set.id);
            state.sets_open = false;
        }
        if !layout.busy() {
            set_menu(ui, p, inv, catalog, set, state, account, busy, rect, out);
        }
    }
    if let Some((from, to)) = layout.moved {
        account.move_set(&keys, from, to);
        out.push(Action::Preferences);
    }
}

/// How a set's line is shown.
#[derive(Clone, Copy)]
struct SetLine {
    /// Open in the window.
    open: bool,
    /// The set the AO plays.
    active: bool,
    /// One is carried: no hover.
    quiet: bool,
}

/// One set's line: its name over its count and "Active", the login mark,
/// and "Use" under the pointer for a set not in use. The open set is lit in
/// the accent. Returns whether "Use" was clicked.
fn paint_set(ui: &mut egui::Ui, p: &Palette, set: &Set, rect: egui::Rect, response: &egui::Response, line: SetLine, busy: bool) -> bool {
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let shade = super::shades(p);
    let hovered = !line.quiet && ui.rect_contains_pointer(rect) && !busy;
    if line.open {
        ui.painter().rect(
            rect,
            radius,
            shade.item.lerp_to_gamma(p.violet, 0.16),
            egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.35)),
            egui::StrokeKind::Inside,
        );
        let edge = egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 9.0), Vec2::new(3.0, rect.height() - 18.0));
        ui.painter().rect_filled(edge, radius, p.violet_light);
    } else if hovered || response.dragged() {
        ui.painter().rect_filled(rect, radius, shade.slot);
    } else {
        ui.painter().rect_filled(rect, radius, shade.slot.gamma_multiply(0.45));
    }
    let x = rect.left() + 14.0;
    // the login mark and the ⋯ button keep the right end; "Use" comes
    // before them under the pointer
    let use_w = if hovered && !line.active { USE_W + 6.0 } else { 0.0 };
    let marks = 30.0 + if set.default { 20.0 } else { 0.0 } + use_w;
    let room = rect.right() - x - marks;
    let title = egui::FontId::new(13.0, super::super::fonts::semibold(ui.ctx()));
    widgets::paint_elided(ui, &set.name, egui::pos2(x, rect.center().y - 8.0), room, title, p.ink);
    let count = aurora_i18n::tr!("ao-animation-count", count = page::animations(set));
    let small = egui::FontId::proportional(11.5);
    let count_w = widgets::elided_galley(ui, RichText::new(&count).font(small.clone()), room).size().x;
    widgets::paint_elided(ui, &count, egui::pos2(x, rect.center().y + 9.0), room, small.clone(), p.muted);
    if line.active {
        // the set the AO plays: a live dot and "Active", as "Playing now"
        let at = x + count_w + 10.0;
        ui.painter().circle_filled(egui::pos2(at, rect.center().y + 9.0), 2.5, p.teal);
        let label = aurora_i18n::tr!("ao-active");
        widgets::paint_elided(
            ui,
            label,
            egui::pos2(at + 6.0, rect.center().y + 9.0),
            (room - count_w - 16.0).max(0.0),
            small,
            p.teal,
        );
    }
    let mut right = rect.right() - 30.0;
    if set.default {
        let at = egui::pos2(right - 8.0, rect.center().y);
        widgets::paint_glyph(ui.painter(), "circle-check-filled", at, 14.0, p.muted);
        let mark = egui::Rect::from_center_size(at, Vec2::splat(18.0));
        ui.interact(mark, ui.scope_id().with(("ao_login_mark", set.id)), egui::Sense::hover())
            .on_hover_text(aurora_i18n::tr!("ao-loads-at-login"));
        right -= 20.0;
    }
    let mut used = false;
    if use_w > 0.0 {
        let pill = egui::Rect::from_min_size(egui::pos2(right - USE_W - 2.0, rect.center().y - 11.0), Vec2::new(USE_W, 22.0));
        let r = ui
            .interact(pill, ui.scope_id().with(("ao_use_set", set.id)), egui::Sense::click())
            .on_hover_text(aurora_i18n::tr!("ao-use-set-tip"));
        let fill = if r.hovered() { p.violet } else { shade.item_hot };
        let ink = if r.hovered() { p.on_accent } else { p.ink };
        ui.painter().rect_filled(pill, radius, fill);
        let font = egui::FontId::new(11.5, super::super::fonts::semibold(ui.ctx()));
        ui.painter().text(
            pill.center(),
            egui::Align2::CENTER_CENTER,
            aurora_i18n::tr!("ao-use-set"),
            font,
            ink,
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, aurora_i18n::tr!("ao-use-set")));
        used = r.clicked();
    }
    let mut label = set.name.clone();
    if line.active {
        label = format!("{label}, {}", aurora_i18n::tr!("ao-active"));
    }
    if set.default {
        label = format!("{label}, {}", aurora_i18n::tr!("ao-loads-at-login"));
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, !busy, line.open, &label));
    used
}

/// Deleting a set asks first, under the list.
fn confirm_delete(ui: &mut egui::Ui, p: &Palette, inv: &mut Inventory, set: &Set, state: &mut AoUi, busy: bool, out: &mut Vec<Action>) {
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

/// The two ways to make a set, as tiles: a new empty one, and one read from
/// a notecard.
fn create_tiles(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut AoUi) {
    let gap = 8.0;
    let width = (ui.available_width() - gap) / 2.0;
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TILE_H), egui::Sense::hover());
    let left = egui::Rect::from_min_size(row.min, Vec2::new(width, TILE_H));
    let right = egui::Rect::from_min_size(egui::pos2(left.right() + gap, row.top()), Vec2::new(width, TILE_H));
    if tile(ui, p, left, "plus", aurora_i18n::tr!("ao-new"), aurora_i18n::tr!("ao-new-line")).clicked() {
        state.name.clear();
        state.new_dialog = Some(SetDialog::New);
    }
    if tile(
        ui,
        p,
        right,
        "file-text-filled",
        aurora_i18n::tr!("ao-import-short"),
        aurora_i18n::tr!("ao-import-tile-line"),
    )
    .clicked()
    {
        state.picker = Some(true);
        state.folder = inv.root;
    }
}

/// A tile: its icon in a disc, its name, what it makes (on two lines when
/// it needs them); lit under the pointer.
fn tile(ui: &mut egui::Ui, p: &Palette, rect: egui::Rect, icon: &str, title: &str, line: &str) -> egui::Response {
    let response = ui.interact(rect, ui.scope_id().with(("ao_tile", icon)), egui::Sense::click());
    let enabled = ui.is_enabled();
    let hot = response.hovered() && enabled;
    let lit = ui.ctx().animate_bool(response.id.with("lit"), hot);
    let shade = super::shades(p);
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect(
        rect,
        radius,
        shade.slot.lerp_to_gamma(p.violet, 0.10 * lit),
        egui::Stroke::new(1.0, shade.edge.lerp_to_gamma(p.violet_light, 0.55 * lit)),
        egui::StrokeKind::Inside,
    );
    let disc = egui::pos2(rect.left() + 22.0, rect.top() + 22.0);
    ui.painter().circle_filled(disc, 12.0, p.violet.gamma_multiply(0.22 + 0.5 * lit));
    let glyph = if enabled {
        p.violet_pale.lerp_to_gamma(p.on_accent, lit)
    } else {
        p.muted_dim
    };
    widgets::paint_glyph(ui.painter(), icon, disc, 13.0, glyph);
    let room = rect.width() - 20.0;
    let x = rect.left() + 10.0;
    let title_font = egui::FontId::new(12.5, super::super::fonts::semibold(ui.ctx()));
    let ink = if enabled { p.ink } else { p.muted };
    widgets::paint_elided(ui, title, egui::pos2(x, rect.top() + 49.0), room, title_font, ink);
    let mut job =
        egui::text::LayoutJob::single_section(line.to_owned(), egui::TextFormat::simple(egui::FontId::proportional(11.0), p.muted));
    job.wrap = egui::text::TextWrapping {
        max_width: room,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(x, rect.top() + 58.0), galley, p.muted);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, title));
    response
}

/// The less used ways, as a grouped list: bring sets from another viewer,
/// and read the sets again from the inventory.
fn more_ways(ui: &mut egui::Ui, p: &Palette, state: &mut AoUi, busy: bool, out: &mut Vec<Action>) {
    let shade = super::shades(p);
    egui::Frame::new()
        .fill(shade.slot.lerp_to_gamma(p.violet, 0.05))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let migrate = !busy && !state.startup_loading;
            if way(ui, p, "arrows-exchange", aurora_i18n::tr!("ao-migrate-link"), migrate).clicked() && migrate {
                state.migration = Some(Default::default());
            }
            if way(ui, p, "refresh", aurora_i18n::tr!("ao-reload"), !busy).clicked() && !busy {
                out.push(Action::Reload);
            }
        });
}

/// One row of the grouped list: its icon, its name, a chevron.
fn way(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, enabled: bool) -> egui::Response {
    // its name wraps rather than being cut
    let width = ui.available_width();
    let ink = if enabled { p.ink } else { p.muted };
    let text = ui
        .painter()
        .layout(label.to_owned(), egui::FontId::proportional(12.5), ink, width - 52.0);
    let height = (text.size().y + 18.0).max(34.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    let hot = response.hovered() && enabled;
    if hot {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.45));
    }
    let y = rect.center().y;
    let tint = if !enabled {
        p.muted_dim
    } else if hot {
        p.violet_light
    } else {
        p.violet_pale
    };
    widgets::paint_glyph(ui.painter(), icon, egui::pos2(rect.left() + 14.0, y), 14.0, tint);
    ui.painter()
        .galley(egui::pos2(rect.left() + 30.0, y - text.size().y / 2.0), text, ink);
    let chevron = if hot { p.ink } else { p.muted_dim };
    widgets::paint_glyph(ui.painter(), "chevron-right", egui::pos2(rect.right() - 14.0, y), 13.0, chevron);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response
}

/// The set's name edited in place: Enter saves, Escape or a click away
/// cancels.
fn rename_field(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    inv: &Inventory,
    catalog: &inventory::Catalog,
    set: &Set,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    // in place of the set's line
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(6.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut child;
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

/// The open set's own options in a grouped card: each its
/// icon, name and what it does, its switch on the right; a click anywhere
/// on a row switches it.
fn set_options(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, set: &Set, state: &mut AoUi, busy: bool, out: &mut Vec<Action>) {
    let (mut sits, mut smart, mut mouselook) = (set.sits, set.smart, set.mouselook);
    let mut changed = false;
    let shade = super::shades(p);
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

/// A set's ⋯ button, at the right end of its line, and its menu.
#[allow(clippy::too_many_arguments)]
fn set_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    set: &Set,
    state: &mut AoUi,
    account: &mut Account,
    busy: bool,
    line: egui::Rect,
    out: &mut Vec<Action>,
) {
    let more = egui::Rect::from_center_size(egui::pos2(line.right() - 16.0, line.center().y), Vec2::splat(26.0));
    let response = ui
        .interact(more, ui.scope_id().with(("ao_set_more", set.id)), egui::Sense::click())
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
        let active = set.id == state.active_set;
        if super::super::menu::item_if(ui, p, "player-play-filled", aurora_i18n::tr!("ao-use-this-set"), !active) {
            account.selected = set.id;
            out.push(Action::Preferences);
        }
        super::super::menu::separator(ui, p);
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
