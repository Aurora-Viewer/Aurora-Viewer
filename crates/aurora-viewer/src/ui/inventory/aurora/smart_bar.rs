//! The chip bar of the advanced inventory search (Aurora layout): the
//! search field's place while the window is in advanced mode. Each top-level
//! rule of the query is a chip (a click opens its menu, its cross removes
//! it), a field adds a rule from a short list of readings of what was typed,
//! and the end of the bar holds the All/Any pill, "Add filter" (the Filters
//! floater), Save and the way back to quick search. Nobody writes a pattern
//! here: the operators are named, and `smart::pattern` turns them into one.

use super::super::smart::{Field, Group, Node, Op, Rule};
use super::super::smart_store;
use super::super::view::Age;
use super::super::*;
use crate::ui::{menu, widgets};
use egui::{Pos2, Rect, Sense, Vec2};

/// The bar's height, that of the toolbar's fields.
const BAR: f32 = 30.0;
/// A chip's height.
const CHIP: f32 = 24.0;
/// The room kept for typing a new rule next to the chips.
const INPUT_MIN: f32 = 90.0;
/// The suggestions offered for what was typed, at most.
const SHOWN: usize = 8;
/// The width of the small icon buttons at the end of the bar.
const ICON_W: f32 = 24.0;
/// The gap between the bar's parts.
const GAP: f32 = 6.0;
/// Below this width the bar keeps its chips and its field only.
const COMPACT: f32 = 260.0;

/// What the bar remembers between frames (per bar, in egui's memory).
#[derive(Clone, Default)]
struct Draft {
    /// The rule being typed.
    text: String,
    /// The suggestion Enter takes.
    picked: usize,
    /// The name being typed for a new saved search; None while not saving.
    naming: Option<String>,
}

/// The localized name of a field.
fn field_name(field: Field) -> &'static str {
    match field {
        Field::Name => aurora_i18n::tr!("inventory-smart-field-name"),
        Field::Description => aurora_i18n::tr!("inventory-smart-field-description"),
        Field::Creator => aurora_i18n::tr!("inventory-smart-field-creator"),
        Field::Uuid => aurora_i18n::tr!("inventory-smart-field-uuid"),
        Field::Folder => aurora_i18n::tr!("inventory-smart-field-folder"),
        Field::Type => aurora_i18n::tr!("inventory-smart-field-type"),
        Field::Permissions => aurora_i18n::tr!("inventory-smart-field-permissions"),
        Field::Worn => aurora_i18n::tr!("inventory-smart-field-worn"),
        Field::Link => aurora_i18n::tr!("inventory-smart-field-link"),
        Field::Coalesced => aurora_i18n::tr!("inventory-smart-field-coalesced"),
        Field::Date => aurora_i18n::tr!("inventory-smart-field-date"),
    }
}

/// The localized name of an operator.
fn op_name(op: Op) -> &'static str {
    match op {
        Op::Contains => aurora_i18n::tr!("inventory-smart-op-contains"),
        Op::Is => aurora_i18n::tr!("inventory-smart-op-is"),
        Op::StartsWith => aurora_i18n::tr!("inventory-smart-op-starts-with"),
        Op::EndsWith => aurora_i18n::tr!("inventory-smart-op-ends-with"),
        Op::Pattern => aurora_i18n::tr!("inventory-smart-op-pattern"),
        Op::EndsInNumber => aurora_i18n::tr!("inventory-smart-op-ends-in-number"),
        Op::HasVersion => aurora_i18n::tr!("inventory-smart-op-has-version"),
    }
}

/// A chip's text: `Name contains "x"`, a bare `Name ends in a number`, the
/// fixed phrase of a yes/no field, the age of a Date rule; "not: …" around
/// it for a negated rule.
fn rule_label(rule: &Rule) -> String {
    let text = match rule.field {
        f if f.is_text() && rule.op.needs_value() => {
            aurora_i18n::tr!(
                "inventory-smart-chip",
                field = field_name(f),
                op = op_name(rule.op),
                value = rule.text.as_str()
            )
        }
        f if f.is_text() => aurora_i18n::tr!("inventory-smart-chip-bare", field = field_name(f), op = op_name(rule.op)),
        Field::Worn => aurora_i18n::tr!("inventory-smart-worn").to_owned(),
        Field::Link => aurora_i18n::tr!("inventory-smart-link").to_owned(),
        Field::Coalesced => aurora_i18n::tr!("inventory-smart-coalesced").to_owned(),
        Field::Date => match rule.age {
            Age::Any => field_name(Field::Date).to_owned(),
            Age::SinceLogout => aurora_i18n::tr!("inventory-smart-since-logout").to_owned(),
            Age::Newer => aurora_i18n::tr!(
                "inventory-smart-chip-bare",
                field = aurora_i18n::tr!("inventory-smart-age-newer"),
                op = aurora_i18n::tr!("inventory-smart-hours", hours = rule.hours)
            ),
            Age::Older => aurora_i18n::tr!(
                "inventory-smart-chip-bare",
                field = aurora_i18n::tr!("inventory-smart-age-older"),
                op = aurora_i18n::tr!("inventory-smart-hours", hours = rule.hours)
            ),
        },
        // the types and rights are chosen in the rule builder: the chip says
        // how many are asked for
        f if rule.bits == 0 => format!("{} · {}", field_name(f), aurora_i18n::tr!("inventory-smart-types-none")),
        f => aurora_i18n::tr!(
            "inventory-smart-chip-bare",
            field = field_name(f),
            op = rule.bits.count_ones().to_string()
        ),
    };
    if rule.not {
        aurora_i18n::tr!("inventory-smart-chip-not", rule = text)
    } else {
        text
    }
}

/// A nested group's chip: "Group (3)", negated like a rule.
fn group_label(group: &Group) -> String {
    let text = aurora_i18n::tr!(
        "inventory-smart-chip-bare",
        field = aurora_i18n::tr!("inventory-smart-group"),
        op = group.nodes.len().to_string()
    );
    if group.not {
        aurora_i18n::tr!("inventory-smart-chip-not", rule = text)
    } else {
        text
    }
}

fn node_label(node: &Node) -> String {
    match node {
        Node::Rule(rule) => rule_label(rule),
        Node::Group(group) => group_label(group),
    }
}

/// The readings of what was typed, best first: a UUID is an id; a `*` or a
/// `?` is a pattern for the name; then the usual ways to look for a word.
fn suggestions(text: &str) -> Vec<Rule> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let mut list = Vec::new();
    if uuid::Uuid::parse_str(text).is_ok() {
        list.push(Rule::text(Field::Uuid, Op::Is, text));
    }
    if text.contains(['*', '?']) {
        list.push(Rule::text(Field::Name, Op::Pattern, text));
    }
    list.extend([
        Rule::text(Field::Name, Op::Contains, text),
        Rule::text(Field::Description, Op::Contains, text),
        Rule::text(Field::Creator, Op::Is, text),
        Rule::text(Field::Name, Op::StartsWith, text),
        Rule::text(Field::Name, Op::EndsWith, text),
        Rule::text(Field::Name, Op::Is, text),
        Rule::text(Field::Folder, Op::Contains, text),
    ]);
    list.truncate(SHOWN);
    list
}

/// What a chip's menu or cross asked for.
enum Edit {
    Remove(usize),
}

/// A small icon button at `rect`, hot like the toolbar's.
fn icon_button(ui: &mut egui::Ui, p: &Palette, rect: Rect, id: egui::Id, icon: &str, tip: &str, lit: bool) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    let hot = response.hovered() || response.has_focus();
    if hot || lit {
        let fill = if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised };
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    }
    widgets::paint_glyph(ui.painter(), icon, rect.center(), 14.0, if hot || lit { p.ink } else { p.muted });
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    response.on_hover_text(tip)
}

/// The menu of a rule's chip: how it compares, its value, Not, Remove.
fn rule_menu(ui: &mut egui::Ui, p: &Palette, rule: &mut Rule) -> bool {
    let mut remove = false;
    if rule.field.is_text() {
        for op in Op::ALL {
            if menu::check(ui, p, "search-filled", op_name(op), rule.op == op, true) {
                rule.op = op;
            }
        }
        if rule.op.needs_value() {
            menu::separator(ui, p);
            ui.add(
                egui::TextEdit::singleline(&mut rule.text)
                    .desired_width(180.0)
                    .hint_text(field_name(rule.field)),
            );
            if rule.op == Op::Pattern {
                menu::info(ui, p, "info-circle", aurora_i18n::tr!("inventory-smart-pattern-tip"));
            }
        }
        menu::separator(ui, p);
    }
    menu::toggle(ui, p, "x", aurora_i18n::tr!("inventory-smart-not"), &mut rule.not);
    if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("inventory-smart-remove-rule")) {
        remove = true;
    }
    remove
}

/// The menu of a nested group's chip (its rules are edited in the rule
/// builder): flip All/Any, Not, Remove.
fn group_menu(ui: &mut egui::Ui, p: &Palette, group: &mut Group) -> bool {
    let all = aurora_i18n::tr!("inventory-smart-match-all");
    let any = aurora_i18n::tr!("inventory-smart-match-any");
    if menu::check(ui, p, "search-filled", all, !group.any, true) {
        group.any = false;
    }
    if menu::check(ui, p, "search-filled", any, group.any, true) {
        group.any = true;
    }
    menu::separator(ui, p);
    menu::toggle(ui, p, "x", aurora_i18n::tr!("inventory-smart-not"), &mut group.not);
    menu::item(ui, p, "trash-filled", aurora_i18n::tr!("inventory-smart-remove-group"))
}

/// A chip at `pos`: its text and a cross; returns its rect, the chip's own
/// response (opens the menu) and whether the cross was hit.
fn chip(ui: &mut egui::Ui, p: &Palette, id: egui::Id, pos: Pos2, galley: std::sync::Arc<egui::Galley>) -> (egui::Response, bool) {
    let size = Vec2::new(10.0 + galley.size().x + 6.0 + 14.0 + 6.0, CHIP);
    let rect = Rect::from_min_size(pos, size);
    let response = ui.interact(rect, id, Sense::click());
    let cross = Rect::from_center_size(Pos2::new(rect.right() - 6.0 - 7.0, rect.center().y), Vec2::splat(18.0));
    // registered after the chip, so it wins where they overlap
    let cross_response = ui.interact(cross, id.with("remove"), Sense::click());
    let hot = response.hovered() || cross_response.hovered();
    let fill = if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised };
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    widgets::paint_row_text(ui.painter(), galley, rect.left() + 10.0, rect.center().y, p.ink);
    widgets::paint_glyph(
        ui.painter(),
        "x",
        cross.center(),
        13.0,
        if cross_response.hovered() { p.ink } else { p.muted },
    );
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tip = aurora_i18n::tr!("inventory-chip-remove");
    cross_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    let removed = cross_response.on_hover_text(tip).clicked();
    (response, removed)
}

/// The suggestion list under the field; the row clicked, if any. Like the
/// chat's (`chat/suggest.rs`): an area of menu rows under the field.
fn list_area(ui: &egui::Ui, p: &Palette, id: egui::Id, field: Rect, rules: &[Rule], picked: usize) -> Option<usize> {
    let mut clicked = None;
    egui::Area::new(id)
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(field.left_bottom() + Vec2::new(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_min_width(240.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, rule) in rules.iter().enumerate() {
                    if menu::suggestion_row(ui, p, "search-filled", &rule_label(rule), "", picked == i).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}

/// The bar, `width` wide, in place of the search field.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, width: f32) {
    let id = ui.scope_id().with("inventory_smart_bar");
    let mut draft: Draft = ui.ctx().data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let active = st.active_smart;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, BAR), Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field);
    let mid = rect.center().y;

    // the end of the bar, from the right: leave, save, add a filter, All/Any.
    // The narrow width has no room for them: the toolbar's own switch leaves,
    // its Filters button opens the rule builder, which saves and flips All/Any.
    let compact = width < COMPACT;
    let mut leave = false;
    let mut open_filters = false;
    let mut save = None;
    let mut flip = false;
    let mut limit = rect.right() - GAP;
    if !compact {
        let mut right = rect.right() - 3.0;
        let cell = |right: f32| Rect::from_center_size(Pos2::new(right - ICON_W / 2.0, mid), Vec2::new(ICON_W, BAR - 6.0));
        let leave_rect = cell(right);
        if icon_button(
            ui,
            p,
            leave_rect,
            id.with("leave"),
            "x",
            aurora_i18n::tr!("inventory-search-quick"),
            false,
        )
        .clicked()
        {
            leave = true;
        }
        right = leave_rect.left() - 2.0;
        let save_rect = cell(right);
        save = Some(icon_button(
            ui,
            p,
            save_rect,
            id.with("save"),
            "device-floppy-filled",
            aurora_i18n::tr!("inventory-smart-save-tip"),
            draft.naming.is_some(),
        ));
        right = save_rect.left() - 2.0;
        let add_rect = cell(right);
        if icon_button(
            ui,
            p,
            add_rect,
            id.with("add"),
            "plus",
            aurora_i18n::tr!("inventory-smart-add-rule"),
            false,
        )
        .clicked()
        {
            open_filters = true;
        }
        right = add_rect.left() - 2.0;

        // All / Any pill
        let any = st.smart_query().is_some_and(|q| q.root.any);
        let pill_text = if any {
            aurora_i18n::tr!("inventory-smart-match-any")
        } else {
            aurora_i18n::tr!("inventory-smart-match-all")
        };
        let pill_galley = ui
            .painter()
            .layout_no_wrap(pill_text.to_owned(), egui::FontId::proportional(11.5), p.violet_light);
        let pill_rect = Rect::from_center_size(
            Pos2::new(right - (pill_galley.size().x + 14.0) / 2.0, mid),
            Vec2::new(pill_galley.size().x + 14.0, CHIP),
        );
        let pill = ui.interact(pill_rect, id.with("pill"), Sense::click());
        let pill_fill = if pill.hovered() {
            p.raised.lerp_to_gamma(p.ink, 0.16)
        } else {
            p.raised
        };
        ui.painter().rect_filled(pill_rect, ui.visuals().window_corner_radius, pill_fill);
        widgets::paint_row_text(ui.painter(), pill_galley, pill_rect.left() + 7.0, mid, p.violet_light);
        if pill.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        pill.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, pill_text));
        flip = pill.on_hover_text(aurora_i18n::tr!("inventory-smart-match-tip")).clicked();
        limit = pill_rect.left() - GAP;
    }
    let input_min = if compact { INPUT_MIN / 2.0 } else { INPUT_MIN };

    // the chips, then the field for the next rule
    let mut edit = None;
    let mut x = rect.left() + 5.0;
    let mut hidden: Vec<(usize, String)> = Vec::new();
    let labels: Vec<String> = st
        .smart_query()
        .map(|q| q.root.nodes.iter().map(node_label).collect())
        .unwrap_or_default();
    let more_w = ui
        .painter()
        .layout_no_wrap(
            aurora_i18n::tr!("inventory-smart-more", count = labels.len()),
            egui::FontId::proportional(12.0),
            p.ink,
        )
        .size()
        .x
        + 20.0;
    for (i, label) in labels.iter().enumerate() {
        let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(12.0), p.ink);
        let w = 10.0 + galley.size().x + 6.0 + 14.0 + 6.0;
        // the chip must leave room for the field, and for "+N" if more follow
        let behind = if i + 1 < labels.len() { more_w + GAP } else { 0.0 };
        if !hidden.is_empty() || x + w + GAP + input_min + behind > limit {
            hidden.push((i, label.clone()));
            continue;
        }
        let (response, removed) = chip(ui, p, id.with(("chip", i)), Pos2::new(x, mid - CHIP / 2.0), galley);
        x += w + GAP;
        if removed {
            edit = Some(Edit::Remove(i));
        }
        if let Some(query) = st.smart_query()
            && let Some(node) = query.root.nodes.get_mut(i)
        {
            let remove = menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| match node {
                Node::Rule(rule) => {
                    if rule_menu(ui, p, rule) {
                        edit = Some(Edit::Remove(i));
                        ui.close();
                    }
                }
                Node::Group(group) => {
                    if group_menu(ui, p, group) {
                        edit = Some(Edit::Remove(i));
                        ui.close();
                    }
                }
            });
            let _ = remove;
        }
    }
    if !hidden.is_empty() {
        let text = aurora_i18n::tr!("inventory-smart-more", count = hidden.len());
        let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(12.0), p.ink);
        let size = Vec2::new(galley.size().x + 20.0, CHIP);
        let more = Rect::from_min_size(Pos2::new(x, mid - CHIP / 2.0), size);
        let response = ui.interact(more, id.with("more"), Sense::click());
        let fill = if response.hovered() {
            p.raised.lerp_to_gamma(p.ink, 0.16)
        } else {
            p.raised
        };
        ui.painter().rect_filled(more, ui.visuals().window_corner_radius, fill);
        widgets::paint_row_text(ui.painter(), galley, more.left() + 10.0, mid, p.ink);
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            // the rules that did not fit: a row each, a click removes it
            for (i, label) in &hidden {
                if menu::item(ui, p, "x", label) {
                    edit = Some(Edit::Remove(*i));
                }
            }
        });
        x += size.x + GAP;
    }

    // the field: type a word, pick what it means
    let input_rect = Rect::from_min_max(
        Pos2::new(x, rect.top()),
        Pos2::new(limit.max(x + 20.0).min(rect.right()), rect.bottom()),
    );
    let input_id = id.with("input");
    let focused = ui.memory(|m| m.has_focus(input_id));
    let rules = if focused { suggestions(&draft.text) } else { Vec::new() };
    let mut take = None;
    if !rules.is_empty() {
        // keys before the field takes them: it would lose its focus on Enter
        let n = rules.len();
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                draft.picked = (draft.picked + 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                draft.picked = (draft.picked + n - 1) % n;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                take = Some(draft.picked.min(n - 1));
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                draft.text.clear();
            }
        });
    }
    let centre = widgets::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
    let field = ui.put(
        input_rect,
        egui::TextEdit::singleline(&mut draft.text)
            .id(input_id)
            .frame(egui::Frame::NONE)
            .hint_text(aurora_i18n::tr!("inventory-smart-add-hint"))
            .align(egui::Align2::LEFT_CENTER)
            .margin(egui::Margin {
                left: 5,
                right: 5,
                ..centre
            }),
    );
    // `put` leaves the row's cursor after the field: the toolbar goes on
    // after the whole bar
    ui.advance_cursor_after_rect(rect);
    if field.changed() {
        draft.picked = 0;
    }
    if !rules.is_empty()
        && let Some(i) = list_area(ui, p, id.with("list"), rect, &rules, draft.picked.min(rules.len() - 1))
    {
        take = Some(i);
    }
    if let Some(i) = take
        && let Some(rule) = rules.get(i)
        && let Some(query) = st.smart_query()
    {
        query.root.nodes.push(Node::Rule(rule.clone()));
        draft.text.clear();
        draft.picked = 0;
        field.request_focus();
    }

    // the chips' and the pill's effects
    if let Some(query) = st.smart_query() {
        if flip {
            query.root.any = !query.root.any;
        }
        if let Some(Edit::Remove(i)) = edit
            && i < query.root.nodes.len()
        {
            query.root.nodes.remove(i);
        }
    }

    // Save: update the saved search being edited, or name a new one
    if let Some(save) = &save
        && let Some(saved) = active.and_then(smart_store::get)
        && draft.naming.is_none()
    {
        let update = menu::dropdown(save, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            if menu::item(ui, p, "device-floppy-filled", aurora_i18n::tr!("inventory-smart-update"))
                && let Some(query) = st.filters.smart.clone()
            {
                smart_store::update(saved.id, query);
            }
            if menu::item(ui, p, "plus", aurora_i18n::tr!("inventory-smart-save")) {
                draft.naming = Some(aurora_i18n::tr!("inventory-smart-untitled").to_owned());
            }
        });
        let _ = update;
    } else if save.as_ref().is_some_and(|s| s.clicked()) {
        draft.naming = match draft.naming {
            Some(_) => None,
            None => Some(aurora_i18n::tr!("inventory-smart-untitled").to_owned()),
        };
    }
    if let Some(name) = draft.naming.as_mut() {
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Area::new(id.with("naming"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_TOP)
            .fixed_pos(Pos2::new(rect.right(), rect.bottom() + 4.0))
            .show(ui.ctx(), |ui| {
                widgets::overlay(ui, p, egui::Margin::same(8), |ui| {
                    ui.set_min_width(220.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(name)
                            .id(id.with("name"))
                            .hint_text(aurora_i18n::tr!("inventory-smart-save-name"))
                            .desired_width(f32::INFINITY),
                    );
                    if !edit.has_focus() && !ui.memory(|m| m.had_focus_last_frame(edit.id)) {
                        edit.request_focus();
                    }
                    if edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirmed = true;
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancelled = true;
                    }
                    ui.add_space(6.0);
                    if widgets::primary_button_sized(
                        ui,
                        p,
                        aurora_i18n::tr!("inventory-smart-save-confirm"),
                        Vec2::new(ui.available_width(), BAR),
                    )
                    .clicked()
                    {
                        confirmed = true;
                    }
                });
            });
        if confirmed {
            let trimmed = name.trim();
            let name = if trimmed.is_empty() {
                aurora_i18n::tr!("inventory-smart-untitled").to_owned()
            } else {
                trimmed.to_owned()
            };
            if let Some(query) = st.filters.smart.clone() {
                st.active_smart = Some(smart_store::add(&name, query));
            }
            draft.naming = None;
        } else if cancelled {
            draft.naming = None;
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(id, draft));
    if open_filters {
        st.filters_open = true;
    }
    if leave {
        st.leave_advanced();
    }
}
