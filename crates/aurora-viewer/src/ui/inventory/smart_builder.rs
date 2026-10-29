//! The rule builder of the advanced inventory search (Filters floater).
//! Rules are chosen from menus and switches, never typed as an expression;
//! the tree itself (`smart::SmartQuery`) is what the matcher reads, so every
//! edit here takes effect on the next frame.
use super::smart::{Field, Group, Node, Op, Rule, SmartQuery};
use super::view::{Age, types};
use super::{InventoryUi, Palette, smart_store};
use crate::ui::widgets::{self, ActionKind};
use egui::{RichText, Vec2};

/// Groups nest this deep below the root in the UI (the matcher has no limit).
const MAX_DEPTH: usize = 2;
const CONTROL: f32 = 27.0;

fn field_label(field: Field) -> &'static str {
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

fn op_label(op: Op) -> &'static str {
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

/// The permission bits, in the order of the legacy Filters floater.
fn rights() -> [(u32, &'static str); 3] {
    [
        (super::rules::MODIFY, aurora_i18n::tr!("common-edit")),
        (super::rules::COPY, aurora_i18n::tr!("common-copy-2")),
        (super::rules::TRANSFER, aurora_i18n::tr!("common-transfer")),
    ]
}

/// The chosen item types of a Type rule, for its closed combo box and the
/// one-line summary.
fn types_text(bits: u32) -> String {
    let chosen: Vec<&str> = types()
        .iter()
        .filter(|(kind, _)| bits & (1_u32 << kind) != 0)
        .map(|(_, label)| *label)
        .collect();
    match chosen.as_slice() {
        [] => aurora_i18n::tr!("inventory-smart-types-none").to_owned(),
        [one] => (*one).to_owned(),
        many => aurora_i18n::tr!("inventory-chip-types", count = many.len()),
    }
}

fn rule_summary(rule: &Rule) -> String {
    let field = field_label(rule.field);
    let body = match rule.field {
        f if f.is_text() && rule.op.needs_value() => {
            aurora_i18n::tr!(
                "inventory-smart-chip",
                field = field,
                op = op_label(rule.op),
                value = rule.text.as_str()
            )
        }
        f if f.is_text() => aurora_i18n::tr!("inventory-smart-chip-bare", field = field, op = op_label(rule.op)),
        Field::Type => aurora_i18n::tr!("inventory-chip-type", types = types_text(rule.bits)),
        Field::Permissions => rights()
            .iter()
            .filter(|(bit, _)| rule.bits & bit != 0)
            .map(|(_, label)| *label)
            .collect::<Vec<_>>()
            .join(" + "),
        Field::Worn => aurora_i18n::tr!("inventory-smart-worn").to_owned(),
        Field::Link => aurora_i18n::tr!("inventory-smart-link").to_owned(),
        Field::Coalesced => aurora_i18n::tr!("inventory-smart-coalesced").to_owned(),
        _ => match rule.age {
            Age::SinceLogout => aurora_i18n::tr!("inventory-smart-since-logout").to_owned(),
            Age::Older => aurora_i18n::tr!("inventory-chip-older", hours = rule.hours),
            _ => aurora_i18n::tr!("inventory-chip-newer", hours = rule.hours),
        },
    };
    if rule.not {
        aurora_i18n::tr!("inventory-smart-chip-not", rule = body)
    } else {
        body
    }
}

fn group_summary(group: &Group, nested: bool) -> String {
    // Blank rules are left out of the search, so they are left out here.
    let parts: Vec<String> = group
        .nodes
        .iter()
        .filter_map(|node| match node {
            Node::Rule(rule) => (!rule.is_blank()).then(|| rule_summary(rule)),
            Node::Group(inner) => {
                let text = group_summary(inner, true);
                (!text.is_empty()).then_some(text)
            }
        })
        .collect();
    let joined = parts.join(if group.any { " | " } else { " · " });
    let joined = if group.not && !joined.is_empty() {
        aurora_i18n::tr!("inventory-smart-chip-not", rule = joined)
    } else {
        joined
    };
    if nested && parts.len() > 1 { format!("({joined})") } else { joined }
}

/// One line for the rules of the search field in advanced mode.
pub(super) fn summary(query: &SmartQuery) -> String {
    let text = group_summary(&query.root, false);
    if text.is_empty() {
        aurora_i18n::tr!("inventory-smart-add-hint").to_owned()
    } else {
        text
    }
}

/// The body of the Filters floater in advanced mode.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    let ui_id = st.ui_id;
    let mut active = st.active_smart;
    let query = st.filters.smart.get_or_insert_with(SmartQuery::default);
    let salt = egui::Id::unique(("inventory_smart_builder", ui_id));
    ui.push_id(salt, |ui| {
        group_ui(ui, p, &mut query.root, 0);
        if query.root.nodes.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("inventory-smart-empty")).color(p.muted));
        }
        ui.add_space(6.0);
        footer(ui, p, query, &mut active, salt);
    });
    st.active_smart = active;
}

fn footer(ui: &mut egui::Ui, p: &Palette, query: &mut SmartQuery, active: &mut Option<u64>, salt: egui::Id) {
    // The name being typed lives in egui's memory: the prompt is a few frames
    // long and belongs to this window only.
    let prompt_id = salt.with("save_name");
    let mut prompt: Option<String> = ui.data(|d| d.get_temp::<Option<String>>(prompt_id)).flatten();
    ui.horizontal_wrapped(|ui| {
        if widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-smart-reset")).clicked() {
            query.root = Group::default();
        }
        let can_save = !query.is_empty();
        if let Some(id) = *active
            && smart_store::get(id).is_some()
            && ui
                .add_enabled_ui(can_save, |ui| {
                    widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-smart-update"))
                })
                .inner
                .clicked()
        {
            smart_store::update(id, query.clone());
        }
        let save = ui
            .add_enabled_ui(can_save, |ui| widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-smart-save")))
            .inner
            .on_hover_text(aurora_i18n::tr!("inventory-smart-save-tip"));
        if save.clicked() && prompt.is_none() {
            prompt = Some(String::new());
        }
    });
    if let Some(mut name) = prompt.take() {
        let (mut confirm, mut cancel) = (false, false);
        ui.label(RichText::new(aurora_i18n::tr!("inventory-smart-save-name")).color(p.muted));
        ui.horizontal(|ui| {
            let field = ui.add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width() * 0.5));
            confirm |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            confirm |= widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-smart-save-confirm")).clicked();
            cancel |= widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked();
        });
        if confirm {
            let name = name.trim();
            let name = if name.is_empty() {
                aurora_i18n::tr!("inventory-smart-untitled")
            } else {
                name
            };
            *active = Some(smart_store::add(name, query.clone()));
        } else if !cancel {
            prompt = Some(name);
        }
    }
    ui.data_mut(|d| d.insert_temp(prompt_id, prompt));
}

/// A group: its All / Any toggle, Not, the add buttons, then its nodes.
fn group_ui(ui: &mut egui::Ui, p: &Palette, group: &mut Group, depth: usize) -> bool {
    let mut remove_self = false;
    ui.horizontal_wrapped(|ui| {
        if depth > 0 {
            ui.label(RichText::new(aurora_i18n::tr!("inventory-smart-group")).color(p.muted));
        }
        widgets::radio_value(ui, p, &mut group.any, false, aurora_i18n::tr!("inventory-smart-match-all"))
            .on_hover_text(aurora_i18n::tr!("inventory-smart-match-tip"));
        widgets::radio_value(ui, p, &mut group.any, true, aurora_i18n::tr!("inventory-smart-match-any"))
            .on_hover_text(aurora_i18n::tr!("inventory-smart-match-tip"));
        widgets::switch_labeled(ui, p, &mut group.not, aurora_i18n::tr!("inventory-smart-not"))
            .on_hover_text(aurora_i18n::tr!("inventory-smart-not-tip"));
        if depth > 0
            && widgets::action_button(ui, p, "x", "", ActionKind::FlatDanger, Vec2::splat(CONTROL))
                .on_hover_text(aurora_i18n::tr!("inventory-smart-remove-group"))
                .clicked()
        {
            remove_self = true;
        }
    });
    let mut remove = None;
    for (index, node) in group.nodes.iter_mut().enumerate() {
        ui.push_id(index, |ui| match node {
            Node::Rule(rule) => {
                if rule_row(ui, p, rule) {
                    remove = Some(index);
                }
            }
            Node::Group(inner) => {
                ui.indent("group", |ui| {
                    if group_ui(ui, p, inner, depth + 1) {
                        remove = Some(index);
                    }
                });
            }
        });
    }
    if let Some(index) = remove {
        group.nodes.remove(index);
    }
    // a zero width fits the label; the height is taken as given
    ui.horizontal_wrapped(|ui| {
        if widgets::action_button(
            ui,
            p,
            "plus",
            aurora_i18n::tr!("inventory-smart-add-rule"),
            ActionKind::Flat,
            Vec2::new(0.0, CONTROL),
        )
        .clicked()
        {
            group.nodes.push(Node::Rule(Rule::default()));
        }
        if depth < MAX_DEPTH
            && widgets::action_button(
                ui,
                p,
                "plus",
                aurora_i18n::tr!("inventory-smart-add-group"),
                ActionKind::Flat,
                Vec2::new(0.0, CONTROL),
            )
            .clicked()
        {
            group.nodes.push(Node::Group(Group::default()));
        }
    });
    remove_self
}

/// One rule: field, operator, value, Not, remove. True when removed.
fn rule_row(ui: &mut egui::Ui, p: &Palette, rule: &mut Rule) -> bool {
    let mut removed = false;
    ui.horizontal_wrapped(|ui| {
        let mut field = rule.field;
        egui::ComboBox::from_id_salt("field")
            .width(110.0)
            .selected_text(field_label(field))
            .show_ui(ui, |ui| {
                for f in Field::ALL {
                    ui.selectable_value(&mut field, f, field_label(f));
                }
            });
        if field != rule.field {
            // Another field means other defaults for the operator and value;
            // only Not is a choice about the rule as a whole.
            *rule = Rule {
                not: rule.not,
                ..Rule::of(field)
            };
        }
        if rule.field.is_text() {
            egui::ComboBox::from_id_salt("op")
                .width(120.0)
                .selected_text(op_label(rule.op))
                .show_ui(ui, |ui| {
                    for op in Op::ALL {
                        ui.selectable_value(&mut rule.op, op, op_label(op));
                    }
                });
            // An op without a value hides the field: no hidden text may still match.
            if !rule.op.needs_value() {
                rule.text.clear();
            }
        }
        value_editor(ui, p, rule);
        widgets::switch_labeled(ui, p, &mut rule.not, aurora_i18n::tr!("inventory-smart-not"))
            .on_hover_text(aurora_i18n::tr!("inventory-smart-not-tip"));
        if widgets::action_button(ui, p, "x", "", ActionKind::FlatDanger, Vec2::splat(CONTROL))
            .on_hover_text(aurora_i18n::tr!("inventory-smart-remove-rule"))
            .clicked()
        {
            removed = true;
        }
    });
    removed
}

fn value_editor(ui: &mut egui::Ui, p: &Palette, rule: &mut Rule) {
    match rule.field {
        f if f.is_text() => {
            if rule.op.needs_value() {
                // the value takes what the row leaves after Not and remove,
                // so they stay on its line: the switch, its label and the
                // button, with the spacing between each
                let not = ui
                    .painter()
                    .layout_no_wrap(
                        aurora_i18n::tr!("inventory-smart-not").to_owned(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        p.ink,
                    )
                    .size()
                    .x;
                let trail = widgets::SWITCH_SIZE.x + not + CONTROL + 4.0 * ui.spacing().item_spacing.x;
                // the field's own margins come on top of its desired width
                let margins = 2.0 * 4.0;
                // in a wrapping row, `available_width` is the whole row, not what is left
                let room = ui.available_size_before_wrap().x - ui.spacing().item_spacing.x - trail - margins;
                let edit = ui.add(egui::TextEdit::singleline(&mut rule.text).desired_width(room.clamp(60.0, 240.0)));
                if rule.op == Op::Pattern {
                    edit.on_hover_text(aurora_i18n::tr!("inventory-smart-pattern-tip"));
                }
            }
        }
        Field::Type => {
            // Several choices in one popup: it stays open until clicked away.
            egui::ComboBox::from_id_salt("types")
                .width(140.0)
                .selected_text(types_text(rule.bits))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    for (kind, label) in types().iter() {
                        let bit = 1_u32 << kind;
                        let mut on = rule.bits & bit != 0;
                        if widgets::checkbox(ui, p, &mut on, *label).changed() {
                            rule.bits = if on { rule.bits | bit } else { rule.bits & !bit };
                        }
                    }
                });
        }
        Field::Permissions => {
            for (bit, label) in rights() {
                let mut on = rule.bits & bit != 0;
                if widgets::switch_labeled(ui, p, &mut on, label).changed() {
                    rule.bits = if on { rule.bits | bit } else { rule.bits & !bit };
                }
            }
        }
        // Worn, Link and Coalesced are true or false: Not turns them around.
        Field::Worn | Field::Link | Field::Coalesced => {}
        _ => {
            widgets::radio_value(ui, p, &mut rule.age, Age::Newer, aurora_i18n::tr!("inventory-smart-age-newer"));
            widgets::radio_value(ui, p, &mut rule.age, Age::Older, aurora_i18n::tr!("inventory-smart-age-older"));
            widgets::radio_value(
                ui,
                p,
                &mut rule.age,
                Age::SinceLogout,
                aurora_i18n::tr!("inventory-smart-since-logout"),
            );
            if matches!(rule.age, Age::Newer | Age::Older) {
                let (mut days, mut hours) = (rule.hours / 24, rule.hours % 24);
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
                // never 0 hours: that would match nothing newer, everything older
                rule.hours = (days * 24 + hours).max(1);
            }
        }
    }
}
