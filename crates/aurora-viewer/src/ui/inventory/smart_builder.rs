//! The rule builder of the advanced inventory search (Filters floater).
//! Rules are chosen from menus and switches, never typed as an expression;
//! the tree itself (`smart::SmartQuery`) is what the matcher reads, so every
//! edit here takes effect on the next frame.
use super::smart::{Field, Group, Node, Op, Rule, SmartQuery};
use super::view::{Age, types};
use super::{InventoryUi, Palette, smart_store};
use crate::ui::widgets::{self, ActionKind};
use egui::{Align, Layout, RichText, Vec2};

/// Groups nest this deep below the root in the UI (the matcher has no limit).
const MAX_DEPTH: usize = 2;
/// The height of every control of a row: menus, fields, buttons and the
/// All / Any segments.
const CONTROL: f32 = 26.0;
/// The field and operator columns keep one width, so they line up from row
/// to row whatever their text.
const FIELD_W: f32 = 104.0;
const OP_W: f32 = 164.0;
/// The days and the hours of a Date rule.
const SPAN_W: f32 = 74.0;

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

/// The chosen rights of a Permissions rule, for its closed combo box.
fn rights_text(bits: u32) -> String {
    let chosen: Vec<&str> = rights()
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, label)| *label)
        .collect();
    if chosen.is_empty() {
        aurora_i18n::tr!("inventory-smart-rights-none").to_owned()
    } else {
        chosen.join(" + ")
    }
}

fn age_label(age: Age) -> &'static str {
    match age {
        Age::Older => aurora_i18n::tr!("inventory-smart-age-older"),
        Age::SinceLogout => aurora_i18n::tr!("inventory-smart-since-logout"),
        _ => aurora_i18n::tr!("inventory-smart-age-newer"),
    }
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
        // menus, fields and buttons share one height; rows breathe a little
        ui.spacing_mut().interact_size.y = CONTROL;
        ui.spacing_mut().item_spacing.y = 6.0;
        group_ui(ui, p, &mut query.root, 0);
        if query.root.nodes.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("inventory-smart-empty")).color(p.muted));
        }
        ui.add_space(6.0);
        widgets::divider(ui, p);
        ui.add_space(2.0);
        footer(ui, p, query, &mut active, salt);
    });
    st.active_smart = active;
}

/// Reset on the left; the saves on the right, the new one as the main action.
fn footer(ui: &mut egui::Ui, p: &Palette, query: &mut SmartQuery, active: &mut Option<u64>, salt: egui::Id) {
    // The name being typed lives in egui's memory: the prompt is a few frames
    // long and belongs to this window only.
    let prompt_id = salt.with("save_name");
    let mut prompt: Option<String> = ui.data(|d| d.get_temp::<Option<String>>(prompt_id)).flatten();
    let can_save = !query.is_empty();
    let updatable = active.is_some_and(|id| smart_store::get(id).is_some());
    let height = Vec2::new(0.0, CONTROL);
    let mut opened = false;
    ui.horizontal(|ui| {
        if widgets::action_button(ui, p, "", aurora_i18n::tr!("inventory-smart-reset"), ActionKind::Flat, height).clicked() {
            query.root = Group::default();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let save = widgets::action_button_if(
                ui,
                p,
                "device-floppy",
                aurora_i18n::tr!("inventory-smart-save"),
                ActionKind::Primary,
                height,
                can_save,
            )
            .on_hover_text(aurora_i18n::tr!("inventory-smart-save-tip"));
            if save.clicked() && prompt.is_none() {
                prompt = Some(String::new());
                opened = true;
            }
            if let Some(id) = *active
                && updatable
                && widgets::action_button_if(
                    ui,
                    p,
                    "device-floppy",
                    aurora_i18n::tr!("inventory-smart-update"),
                    ActionKind::Flat,
                    height,
                    can_save,
                )
                .clicked()
            {
                smart_store::update(id, query.clone());
            }
        });
    });
    if let Some(mut name) = prompt.take() {
        let (mut confirm, mut cancel) = (false, false);
        ui.label(RichText::new(aurora_i18n::tr!("inventory-smart-save-name")).color(p.muted));
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), CONTROL),
            Layout::right_to_left(Align::Center),
            |ui| {
                cancel |= widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, height).clicked();
                confirm |= widgets::action_button(
                    ui,
                    p,
                    "device-floppy",
                    aurora_i18n::tr!("inventory-smart-save-confirm"),
                    ActionKind::Primary,
                    height,
                )
                .clicked();
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let field = ui.add_sized(
                        Vec2::new(ui.available_width(), CONTROL),
                        egui::TextEdit::singleline(&mut name).align(egui::Align2::LEFT_CENTER),
                    );
                    if opened {
                        field.request_focus();
                    }
                    confirm |= field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
            },
        );
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

/// One line of the builder. `lead` lays out the left columns; Not and the
/// remove button (or its room, on the root) are right-aligned, so they line
/// up on every row whatever its field and depth; `fill` takes what is left
/// between them. True when the remove button was clicked.
fn row<T>(
    ui: &mut egui::Ui,
    p: &Palette,
    item: &mut T,
    not: fn(&mut T) -> &mut bool,
    remove_tip: Option<&str>,
    lead: impl FnOnce(&mut egui::Ui, &mut T),
    fill: impl FnOnce(&mut egui::Ui, &mut T),
) -> bool {
    let mut removed = false;
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), CONTROL),
        Layout::left_to_right(Align::Center),
        |ui| {
            lead(ui, item);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match remove_tip {
                    Some(tip) => {
                        removed = widgets::action_button(ui, p, "x", "", ActionKind::FlatDanger, Vec2::splat(CONTROL))
                            .on_hover_text(tip)
                            .clicked();
                    }
                    // the root has no remove button: its room, spacing included
                    None => {
                        ui.allocate_exact_size(Vec2::splat(CONTROL), egui::Sense::hover());
                    }
                }
                not_toggle(ui, p, not(item));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| fill(ui, item));
            });
        },
    );
    removed
}

/// Not, as a word-sized button beside remove, lit in the accent while the
/// rule or the group is turned around: quieter than a switch and its label,
/// and plainer than a glyph at this size.
fn not_toggle(ui: &mut egui::Ui, p: &Palette, on: &mut bool) {
    let kind = if *on { ActionKind::Primary } else { ActionKind::Flat };
    if widgets::action_button(ui, p, "", aurora_i18n::tr!("inventory-smart-not"), kind, Vec2::new(0.0, CONTROL))
        .on_hover_text(aurora_i18n::tr!("inventory-smart-not-tip"))
        .clicked()
    {
        *on = !*on;
    }
}

fn group_not(group: &mut Group) -> &mut bool {
    &mut group.not
}

fn rule_not(rule: &mut Rule) -> &mut bool {
    &mut rule.not
}

/// A group. The root lies on the floater; a nested group is a card of its
/// own, so its rules read as one block.
fn group_ui(ui: &mut egui::Ui, p: &Palette, group: &mut Group, depth: usize) -> bool {
    if depth == 0 {
        group_body(ui, p, group, depth)
    } else {
        widgets::surface(ui, p, |ui| group_body(ui, p, group, depth))
    }
}

/// A group's All / Any choice, Not and remove, its nodes, then the add
/// buttons.
fn group_body(ui: &mut egui::Ui, p: &Palette, group: &mut Group, depth: usize) -> bool {
    let remove_tip = (depth > 0).then(|| aurora_i18n::tr!("inventory-smart-remove-group"));
    let remove_self = row(
        ui,
        p,
        group,
        group_not,
        remove_tip,
        |ui, group| {
            let mut choice = usize::from(group.any);
            let options = [
                (aurora_i18n::tr!("inventory-smart-match-all"), true),
                (aurora_i18n::tr!("inventory-smart-match-any"), true),
            ];
            if widgets::segmented(ui, p, &mut choice, &options) {
                group.any = choice == 1;
            }
        },
        |_, _| {},
    );
    let mut remove = None;
    for (index, node) in group.nodes.iter_mut().enumerate() {
        ui.push_id(index, |ui| match node {
            Node::Rule(rule) => {
                if rule_row(ui, p, rule) {
                    remove = Some(index);
                }
            }
            Node::Group(inner) => {
                if group_ui(ui, p, inner, depth + 1) {
                    remove = Some(index);
                }
            }
        });
    }
    if let Some(index) = remove {
        group.nodes.remove(index);
    }
    // a zero width fits the label; the height is the rows'
    ui.horizontal(|ui| {
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

/// One rule: field, operator (or the choice that stands for it), value, Not,
/// remove. True when removed.
fn rule_row(ui: &mut egui::Ui, p: &Palette, rule: &mut Rule) -> bool {
    row(
        ui,
        p,
        rule,
        rule_not,
        Some(aurora_i18n::tr!("inventory-smart-remove-rule")),
        |ui, rule| {
            let mut field = rule.field;
            egui::ComboBox::from_id_salt("field")
                .width(FIELD_W)
                .selected_text(field_label(field))
                .show_ui(ui, |ui| {
                    for f in Field::ALL {
                        ui.selectable_value(&mut field, f, field_label(f));
                    }
                });
            if field != rule.field {
                // Another field means other defaults for the operator and
                // value; only Not is a choice about the rule as a whole.
                *rule = Rule {
                    not: rule.not,
                    ..Rule::of(field)
                };
            }
            op_editor(ui, p, rule);
        },
        value_editor,
    )
}

/// The second column: the text operator, or the types, rights or age a rule
/// of another field chooses there. Worn, Link and Coalesced need none: Not
/// turns them around.
fn op_editor(ui: &mut egui::Ui, p: &Palette, rule: &mut Rule) {
    match rule.field {
        f if f.is_text() => {
            egui::ComboBox::from_id_salt("op")
                .width(OP_W)
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
        Field::Type => {
            // Several choices in one popup: it stays open until clicked away.
            egui::ComboBox::from_id_salt("types")
                .width(OP_W)
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
            egui::ComboBox::from_id_salt("rights")
                .width(OP_W)
                .selected_text(rights_text(rule.bits))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    for (bit, label) in rights() {
                        let mut on = rule.bits & bit != 0;
                        if widgets::checkbox(ui, p, &mut on, label).changed() {
                            rule.bits = if on { rule.bits | bit } else { rule.bits & !bit };
                        }
                    }
                });
        }
        Field::Date => {
            egui::ComboBox::from_id_salt("age")
                .width(OP_W)
                .selected_text(age_label(rule.age))
                .show_ui(ui, |ui| {
                    for age in [Age::Newer, Age::Older, Age::SinceLogout] {
                        ui.selectable_value(&mut rule.age, age, age_label(age));
                    }
                });
        }
        _ => {}
    }
}

/// The third column, up to Not: the text to look for, or the span of a Date
/// rule.
fn value_editor(ui: &mut egui::Ui, rule: &mut Rule) {
    match rule.field {
        f if f.is_text() => {
            if rule.op.needs_value() {
                let edit = ui.add_sized(
                    Vec2::new(ui.available_width(), CONTROL),
                    egui::TextEdit::singleline(&mut rule.text).align(egui::Align2::LEFT_CENTER),
                );
                if rule.op == Op::Pattern {
                    edit.on_hover_text(aurora_i18n::tr!("inventory-smart-pattern-tip"));
                }
            }
        }
        Field::Date if matches!(rule.age, Age::Newer | Age::Older) => {
            let (mut days, mut hours) = (rule.hours / 24, rule.hours % 24);
            ui.add_sized(
                Vec2::new(SPAN_W, CONTROL),
                egui::DragValue::new(&mut days)
                    .custom_parser(aurora_i18n::parse_number)
                    .range(0..=36500)
                    .suffix(aurora_i18n::tr!("status-days")),
            );
            ui.add_sized(
                Vec2::new(SPAN_W, CONTROL),
                egui::DragValue::new(&mut hours)
                    .custom_parser(aurora_i18n::parse_number)
                    .range(0..=23)
                    .suffix(aurora_i18n::tr!("status-hours")),
            );
            // never 0 hours: that would match nothing newer, everything older
            rule.hours = (days * 24 + hours).max(1);
        }
        _ => {}
    }
}
