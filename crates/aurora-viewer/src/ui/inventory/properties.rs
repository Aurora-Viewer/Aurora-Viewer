//! Compact LLFloaterProperties layout: thumbnail, resident names, masks and sale.
use super::*;
use aurora_llsd::llsd_map;
use std::collections::HashMap;

pub(crate) fn window<'a>(ctx: &egui::Context, id: &'a str) -> crate::ui::widgets::Floater<'a> {
    crate::ui::widgets::Floater::new(
        id,
        aurora_i18n::tr!("ui-inventory-dialogs-object-properties"),
        ctx.content_rect().center() - egui::vec2(175.0, 305.0),
        Vec2::new(350.0, 610.0),
    )
}

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    id: Uuid,
    actions: &mut Vec<InvAction>,
) -> bool {
    let Some(it) = inv.items.get(&id) else {
        ui.label(aurora_i18n::tr!("ui-inventory-properties-the-item-no-longer-exists"));
        return false;
    };
    if st.property_edit.as_ref().is_none_or(|(item, _)| *item != id) {
        st.property_edit = Some((id, PropertyEdit::from(it)));
    }
    let Some((_, edit)) = &mut st.property_edit else {
        return false;
    };
    let can_edit = rules::mutable(inv, id) && !matches!(it.asset_type, 2 | 24 | 25) && st.pending.is_none();
    let response = panel(
        ui,
        p,
        Some(icons),
        PropertyPanel {
            inventory: Some(inv),
            item: it,
            edit,
            can_edit,
            can_next_owner: can_edit && rules::next_owner_editable(it),
            can_describe: can_edit && rules::held(inv, it) & rules::MODIFY != 0,
            can_thumbnail: can_edit,
            names: &facts.names,
            images,
            wanted_images: &mut st.wanted_images,
        },
    );
    if response.thumbnail {
        st.thumbnail.open(id);
    }
    if let Some(resident) = response.profile {
        actions.push(InvAction::Profile(resident));
    }
    if let Some(edit) = response.edit {
        let name = if edit.name == it.name {
            it.name.clone()
        } else {
            rules::clean_name(&edit.name).unwrap_or_else(|_| it.name.clone())
        };
        let body = llsd_map! { "name" => name, "desc" => edit.desc.clone(),
        "permissions" => llsd_map! { "next_owner_mask" => edit.next as i32, "group_mask" => edit.group as i32, "everyone_mask" => edit.everyone as i32 },
        "sale_info" => llsd_map! { "sale_type" => i32::from(edit.sale_type), "sale_price" => edit.price } };
        match rules::patch(inv, id, body) {
            Ok(change) => {
                actions.push(InvAction::Edit(change));
            }
            Err(reason) => st.message = reason,
        }
    }
    false
}

#[derive(Clone, Copy)]
enum Role {
    Owner,
    Creator,
}

#[derive(Debug, PartialEq)]
enum ResidentLabel<'a> {
    /// A resident's name opening their profile.
    Link(&'a str),
    /// No resident to open, with the reason on hover.
    Plain(&'static str, Option<&'static str>),
}

/// What the owner / creator row shows. The simulator sends no creator for an
/// object whose parts were made by several residents
/// (LLPermissions::accumulate keeps a creator only when all parts share it),
/// which is common on no-modify objects: the Legacy Viewer shows
/// "(unknown / multiple)" without a link then (LLSidepanelItemInfo::refreshFromItem).
fn resident_label<'a>(role: Role, id: Uuid, names: &'a HashMap<Uuid, String>) -> ResidentLabel<'a> {
    if id.is_nil() {
        return match role {
            Role::Creator => ResidentLabel::Plain(
                aurora_i18n::tr!("ui-inventory-properties-creator-unknown-multiple"),
                Some(aurora_i18n::tr!("ui-inventory-properties-creator-unknown-multiple-hint")),
            ),
            Role::Owner => ResidentLabel::Plain(aurora_i18n::tr!("common-unknown"), None),
        };
    }
    ResidentLabel::Link(names.get(&id).map(String::as_str).unwrap_or(aurora_i18n::tr!("common-unknown")))
}

/// One properties layout for agent inventory and object contents. The caller
/// supplies access and saves through the appropriate inventory protocol.
pub(crate) struct PropertyPanel<'a> {
    pub inventory: Option<&'a Inventory>,
    pub item: &'a aurora_net::inventory::InvItem,
    pub edit: &'a mut PropertyEdit,
    pub can_edit: bool,
    pub can_next_owner: bool,
    pub can_describe: bool,
    pub can_thumbnail: bool,
    pub names: &'a HashMap<Uuid, String>,
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub wanted_images: &'a mut HashSet<Uuid>,
}

#[derive(Default)]
pub(crate) struct PropertyResponse {
    /// Only committed controls are merged into the current item. A checkbox
    /// must not submit an unfinished name or description from another field.
    pub edit: Option<PropertyEdit>,
    pub thumbnail: bool,
    pub profile: Option<Uuid>,
}

pub(crate) fn panel(ui: &mut egui::Ui, p: &Palette, icons: Option<&Icons>, data: PropertyPanel<'_>) -> PropertyResponse {
    let PropertyPanel {
        inventory,
        item: it,
        edit,
        can_edit,
        can_next_owner,
        can_describe,
        can_thumbnail,
        names,
        images,
        wanted_images,
    } = data;
    // what the agent may do with the item (object contents: the caller's mask)
    let mine = inventory.map_or(it.owner_mask, |inv| rules::held(inv, it));
    // Follow simulator readback (including refused edits) without replacing a
    // text field's unfinished draft. Agent inventory disables edits in flight.
    if can_edit {
        edit.next = it.next_owner_mask;
        edit.group = it.group_mask;
        edit.everyone = it.everyone_mask;
        edit.sale_type = it.sale_type;
        if !edit.commit_price {
            edit.price = it.sale_price;
        }
    }
    let mut response = PropertyResponse::default();
    egui::ScrollArea::vertical().max_height(550.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
        ui.spacing_mut().interact_size.y = 22.0;
        ui.horizontal(|ui| {
            let (name, color) = inventory.map_or_else(
                || {
                    (
                        item_icon(it.asset_type, it.inv_type, it.flags),
                        item_icon_color(it.asset_type, it.inv_type, p),
                    )
                },
                |inv| inventory_item_icon(inv, it, p),
            );
            if let Some(icons) = icons {
                tree::icon(ui, icons, name, p, Some(color));
            } else if let Some(texture) = crate::ui::icons::global(name) {
                ui.add(egui::Image::new(&texture).fit_to_exact_size(Vec2::splat(16.0)).tint(color));
            } else {
                ui.add_space(16.0);
            }
            let name = crate::ui::widgets::restricted_text_field(
                ui,
                can_describe,
                &mut edit.name,
                crate::ui::widgets::TextPolicy::PrintableNoPipe,
                |b, ui| egui::TextEdit::singleline(b).char_limit(63).desired_width(ui.available_width()),
            );
            // LLFloaterProperties::onCommitName / onCommitDescription
            // (indra/newview/llfloaterproperties.cpp): text commits on focus
            // loss; permission and sale controls commit on each change.
            edit.commit_name |= name.lost_focus();
            if can_describe && edit.commit_name && !ui.input(|input| input.pointer.any_down()) {
                edit.commit_name = false;
                if edit.name != it.name
                    && let Ok(name) = rules::clean_name(&edit.name)
                {
                    edit.name = name.clone();
                    response.edit.get_or_insert_with(|| PropertyEdit::from(it)).name = name;
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            thumbnail::picture(ui, p, it.thumbnail, 128.0, images, wanted_images);
            ui.vertical(|ui| {
                for (label, resident, role) in [
                    (aurora_i18n::tr!("common-owner-2"), it.owner, Role::Owner),
                    (aurora_i18n::tr!("common-creator-2"), it.creator, Role::Creator),
                ] {
                    ui.label(RichText::new(label).size(12.0).color(p.muted));
                    match resident_label(role, resident, names) {
                        ResidentLabel::Link(name) => {
                            if ui
                                .link(RichText::new(name).size(12.0).color(p.violet_light))
                                .on_hover_text(resident.to_string())
                                .clicked()
                            {
                                response.profile = Some(resident);
                            }
                        }
                        ResidentLabel::Plain(text, hint) => {
                            let label = ui.label(RichText::new(text).size(12.0).color(p.muted));
                            if let Some(hint) = hint {
                                label.on_hover_text(hint);
                            }
                        }
                    }
                }
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-inventory-properties-acquired-on"))
                        .size(12.0)
                        .color(p.muted),
                );
                ui.label(RichText::new(acquired(it.created_at)).size(12.0));
                if ui
                    .add_enabled(
                        can_thumbnail,
                        egui::Button::new(aurora_i18n::tr!("ui-inventory-properties-picture")),
                    )
                    .clicked()
                {
                    response.thumbnail = true;
                }
            });
        });
        ui.label(aurora_i18n::tr!("common-description"));
        let description = crate::ui::widgets::restricted_text_field(
            ui,
            can_describe,
            &mut edit.desc,
            crate::ui::widgets::TextPolicy::MultilineNoPipe,
            |b, ui| {
                egui::TextEdit::multiline(b)
                    .char_limit(127)
                    .desired_rows(2)
                    .desired_width(ui.available_width())
            },
        );
        edit.commit_desc |= description.lost_focus();
        if can_describe && edit.commit_desc && !ui.input(|input| input.pointer.any_down()) {
            edit.commit_desc = false;
            if edit.desc != it.desc {
                response.edit.get_or_insert_with(|| PropertyEdit::from(it)).desc = edit.desc.clone();
            }
        }
        ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-properties-experience")).color(p.muted));
        crate::ui::widgets::divider(ui, p);
        ui.label(aurora_i18n::tr!("ui-inventory-properties-permissions"));
        ui.label(aurora_i18n::tr!("ui-inventory-properties-you-can"));
        ui.horizontal(|ui| {
            for (mask, label) in rights() {
                let mut on = mine & mask != 0;
                ui.add_enabled_ui(false, |ui| crate::ui::widgets::checkbox(ui, p, &mut on, label));
            }
        });
        let may_share = can_edit && mine & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER;
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("ui-inventory-properties-anyone"));
            if check_mask(ui, p, may_share, &mut edit.everyone, rules::COPY, aurora_i18n::tr!("common-copy-2")) {
                response.edit.get_or_insert_with(|| PropertyEdit::from(it)).everyone = edit.everyone;
            }
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("common-group-2"));
            let group_share = rules::MODIFY | rules::MOVE | rules::COPY;
            if check_mask(ui, p, may_share, &mut edit.group, group_share, aurora_i18n::tr!("common-share")) {
                response.edit.get_or_insert_with(|| PropertyEdit::from(it)).group = edit.group;
            }
        });
        ui.label(aurora_i18n::tr!("ui-inventory-properties-next-owner"));
        if it.asset_type == 56 {
            edit.next |= rules::COPY;
        }
        ui.horizontal(|ui| {
            let mut changed = false;
            for (mask, label) in rights() {
                changed |= check_mask(
                    ui,
                    p,
                    can_next_owner
                        && it.base_mask & mask != 0
                        && !(mask == rules::COPY && it.asset_type == 56)
                        && (mask != rules::TRANSFER || edit.next & rules::COPY != 0),
                    &mut edit.next,
                    mask,
                    label,
                );
                if edit.next & rules::COPY == 0 {
                    edit.next |= rules::TRANSFER;
                }
            }
            if changed {
                response.edit.get_or_insert_with(|| PropertyEdit::from(it)).next = edit.next;
            }
        });
        crate::ui::widgets::divider(ui, p);
        let mut for_sale = edit.sale_type != 0;
        let mut sale_changed = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled_ui(can_edit && mine & rules::TRANSFER != 0, |ui| {
                    crate::ui::widgets::checkbox(ui, p, &mut for_sale, aurora_i18n::tr!("ui-inventory-properties-for-sale"))
                })
                .inner
                .changed()
            {
                sale_changed = true;
                edit.sale_type = if for_sale { if mine & rules::COPY != 0 { 2 } else { 1 } } else { 0 };
            }
            ui.add_enabled_ui(for_sale && can_edit, |ui| {
                egui::ComboBox::from_id_salt("sale_type")
                    .selected_text(sale_name(edit.sale_type))
                    .show_ui(ui, |ui| {
                        for value in 1..=3 {
                            if ui
                                .add_enabled(
                                    value != 2 || mine & rules::COPY != 0,
                                    egui::Button::selectable(edit.sale_type == value, sale_name(value)),
                                )
                                .clicked()
                            {
                                sale_changed |= edit.sale_type != value;
                                edit.sale_type = value;
                            }
                        }
                    });
            });
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("ui-inventory-properties-price-l"));
            edit.commit_price |= ui
                .add_enabled(
                    for_sale && can_edit,
                    egui::DragValue::new(&mut edit.price)
                        .update_while_editing(false)
                        .custom_parser(aurora_i18n::parse_number)
                        .range(0..=i32::MAX),
                )
                .changed();
        });
        // Keep a numeric drag uninterrupted, and combine a typed price's
        // focus loss with the control clicked next, like the text fields.
        if can_edit && edit.commit_price && !ui.input(|input| input.pointer.any_down()) {
            edit.commit_price = false;
            sale_changed = true;
        }
        if sale_changed {
            let committed = response.edit.get_or_insert_with(|| PropertyEdit::from(it));
            committed.sale_type = edit.sale_type;
            committed.price = edit.price;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (label, mask) in [
                ("B", it.base_mask),
                ("O", it.owner_mask),
                ("G", edit.group),
                ("E", edit.everyone),
                ("N", edit.next),
            ] {
                ui.label(RichText::new(format!("{label}: {}", mask_text(mask))).size(11.0))
                    .on_hover_text(aurora_i18n::tr!("ui-inventory-properties-b-base-o-owner-g-group-e-everyone-n"));
            }
        });
    });
    response
}
fn rights() -> [(u32, &'static str); 3] {
    [
        (rules::MODIFY, aurora_i18n::tr!("common-edit")),
        (rules::COPY, aurora_i18n::tr!("common-copy-2")),
        (rules::TRANSFER, aurora_i18n::tr!("common-transfer")),
    ]
}
fn check_mask(ui: &mut egui::Ui, p: &Palette, enabled: bool, value: &mut u32, mask: u32, label: &str) -> bool {
    let mut on = *value & mask == mask;
    if ui
        .add_enabled_ui(enabled, |ui| crate::ui::widgets::checkbox(ui, p, &mut on, label))
        .inner
        .changed()
    {
        if on {
            *value |= mask;
        } else {
            *value &= !mask;
        }
        return true;
    }
    false
}
fn sale_name(value: u8) -> &'static str {
    match value {
        1 => aurora_i18n::tr!("common-original"),
        3 => aurora_i18n::tr!("common-contents"),
        _ => aurora_i18n::tr!("common-copy"),
    }
}
fn mask_text(mask: u32) -> String {
    [(rules::MOVE, 'V'), (rules::MODIFY, 'M'), (rules::COPY, 'C'), (rules::TRANSFER, 'T')]
        .into_iter()
        .filter_map(|(bit, c)| (mask & bit != 0).then_some(c))
        .collect()
}
pub(super) fn acquired(timestamp: i64) -> String {
    if timestamp <= 0 {
        return aurora_i18n::tr!("ui-inventory-properties-unknown").into();
    }
    let seconds = timestamp.rem_euclid(86400);
    format!(
        "{} {:02}:{:02}:{:02} UTC",
        aurora_net::inventory::created_date(timestamp),
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        inv: &Inventory,
        st: &mut InventoryUi,
        events: Vec<egui::Event>,
    ) -> (Vec<(String, egui::Rect)>, Vec<InvAction>) {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 800.0))),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                assert!(
                    !show(
                        ui,
                        &crate::theme::Theme::default().palette(),
                        &Icons::default(),
                        inv,
                        st,
                        &Facts::default(),
                        &HashMap::new(),
                        Uuid::from_u128(8102),
                        &mut actions
                    ),
                    "Saving properties must keep the dialog open"
                );
            },
        );
        fn text(shape: &egui::Shape, result: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => result.push((t.galley.text().to_owned(), t.galley.rect.translate(t.pos.to_vec2()))),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text(shape, result)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for shape in &output.shapes {
            text(&shape.shape, &mut texts);
        }
        output.textures_delta.clear();
        (texts, actions)
    }

    fn click(ctx: &egui::Context, inv: &Inventory, st: &mut InventoryUi, point: egui::Pos2) -> Vec<InvAction> {
        let mut actions = frame(ctx, inv, st, vec![egui::Event::PointerMoved(point)]).1;
        for pressed in [true, false] {
            actions.extend(
                frame(
                    ctx,
                    inv,
                    st,
                    vec![egui::Event::PointerButton {
                        pos: point,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: Default::default(),
                    }],
                )
                .1,
            );
        }
        actions
    }

    #[test]
    fn inventory_properties_save_controls_once_and_ignore_uncommitted_text() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let id = Uuid::from_u128(8102);
        let original = inv.items[&id].clone();
        let mut st = InventoryUi::default();
        for _ in 0..3 {
            assert!(frame(&ctx, &inv, &mut st, vec![]).1.is_empty());
        }
        st.property_edit.as_mut().unwrap().1.name = "|".into();
        st.property_edit.as_mut().unwrap().1.desc = "Uncommitted description".into();
        for (label, next_owner) in [
            (aurora_i18n::tr!("common-edit"), true),
            (aurora_i18n::tr!("common-share"), false),
            (aurora_i18n::tr!("common-copy-2"), true),
            (aurora_i18n::tr!("ui-inventory-properties-for-sale"), false),
        ] {
            let (labels, actions) = frame(&ctx, &inv, &mut st, vec![]);
            assert!(actions.is_empty());
            assert!(!labels.iter().any(|(text, _)| text == aurora_i18n::tr!("common-save")));
            let next_y = labels
                .iter()
                .find(|(text, _)| text == aurora_i18n::tr!("ui-inventory-properties-next-owner"))
                .unwrap()
                .1
                .bottom();
            let point = labels
                .iter()
                .find(|(text, rect)| text == label && (!next_owner || rect.center().y > next_y && rect.center().y < next_y + 30.0))
                .unwrap()
                .1
                .center();
            let actions = click(&ctx, &inv, &mut st, point);
            let [InvAction::Edit(change)] = &actions[..] else {
                panic!("Expected one inventory mutation for {label}")
            };
            let [aurora_net::inventory::operations::Operation::Patch { id: target, body, .. }] = &change.operations[..] else {
                panic!("Expected the original inventory item")
            };
            assert_eq!(*target, id);
            assert_eq!(body["name"].as_str(), original.name);
            assert_eq!(body["desc"].as_str(), original.desc);
            rules::demo_mutate(&mut inv, Uuid::from_u128(2), change.clone()).unwrap();
            for _ in 0..3 {
                assert!(frame(&ctx, &inv, &mut st, vec![]).1.is_empty(), "No duplicate after readback");
            }
        }
        assert_ne!(inv.items[&id].next_owner_mask, original.next_owner_mask);
        assert_ne!(inv.items[&id].group_mask, original.group_mask);
        assert_ne!(inv.items[&id].sale_type, original.sale_type);
        st.pending = Some(Uuid::new_v4());
        let (labels, _) = frame(&ctx, &inv, &mut st, vec![]);
        let point = labels
            .iter()
            .rev()
            .find(|(text, _)| text == aurora_i18n::tr!("common-edit"))
            .unwrap()
            .1
            .center();
        assert!(
            click(&ctx, &inv, &mut st, point).is_empty(),
            "A pending update disables further edits"
        );
    }

    #[test]
    fn inventory_property_name_commits_on_enter_instead_of_each_keystroke() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let id = Uuid::from_u128(8102);
        inv.items.get_mut(&id).expect("demo object").name = "Objet de démonstration".into();
        let mut st = InventoryUi::default();
        for _ in 0..3 {
            frame(&ctx, &inv, &mut st, vec![]);
        }
        let (labels, _) = frame(&ctx, &inv, &mut st, vec![]);
        let point = labels.iter().find(|(text, _)| text == &inv.items[&id].name).unwrap().1.center();
        assert!(click(&ctx, &inv, &mut st, point).is_empty());
        assert!(frame(&ctx, &inv, &mut st, vec![egui::Event::Text(" renamed".into())]).1.is_empty());
        let actions = frame(
            &ctx,
            &inv,
            &mut st,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        )
        .1;
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("Name validation must save once")
        };
        let [aurora_net::inventory::operations::Operation::Patch { body, .. }] = &change.operations[..] else {
            panic!("Expected a name update")
        };
        assert!(body["name"].as_str().contains("renamed"));
        rules::demo_mutate(&mut inv, Uuid::from_u128(2), change.clone()).unwrap();
        assert!(frame(&ctx, &inv, &mut st, vec![]).1.is_empty());

        st.property_edit.as_mut().unwrap().1.desc = "Original description".into();
        inv.items.get_mut(&id).unwrap().desc = "Original description".into();
        let (labels, _) = frame(&ctx, &inv, &mut st, vec![]);
        let point = labels.iter().find(|(text, _)| text == "Original description").unwrap().1.center();
        assert!(click(&ctx, &inv, &mut st, point).is_empty());
        assert!(frame(&ctx, &inv, &mut st, vec![egui::Event::Text(" updated".into())]).1.is_empty());
        let (labels, _) = frame(&ctx, &inv, &mut st, vec![]);
        let point = labels
            .iter()
            .rev()
            .find(|(text, _)| text == aurora_i18n::tr!("common-edit"))
            .unwrap()
            .1
            .center();
        let actions = click(&ctx, &inv, &mut st, point);
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("Focus loss and a permission click must save together")
        };
        let [aurora_net::inventory::operations::Operation::Patch { body, .. }] = &change.operations[..] else {
            panic!("Expected a description update")
        };
        assert!(body["desc"].as_str().contains("updated"));
        assert_ne!(body["permissions"]["next_owner_mask"].as_u32(), inv.items[&id].next_owner_mask);
    }

    #[test]
    fn inventory_property_price_saves_complete_numbers_and_completed_drags() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let id = Uuid::from_u128(8102);
        let item = inv.items.get_mut(&id).unwrap();
        item.sale_type = 2;
        item.sale_price = 42;
        let mut st = InventoryUi::default();
        for _ in 0..3 {
            frame(&ctx, &inv, &mut st, vec![]);
        }
        let price_point =
            |texts: &[(String, egui::Rect)], value: i32| texts.iter().find(|(text, _)| text == &value.to_string()).unwrap().1.center();
        for (price, enter) in [(12, true), (345, false)] {
            let (labels, _) = frame(&ctx, &inv, &mut st, vec![]);
            assert!(click(&ctx, &inv, &mut st, price_point(&labels, inv.items[&id].sale_price)).is_empty());
            for digit in price.to_string().chars() {
                assert!(
                    frame(&ctx, &inv, &mut st, vec![egui::Event::Text(digit.to_string())]).1.is_empty(),
                    "Typing a number must not submit its first digit and disable the field"
                );
            }
            let actions = if enter {
                frame(
                    &ctx,
                    &inv,
                    &mut st,
                    vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                )
                .1
            } else {
                let (labels, _) = frame(&ctx, &inv, &mut st, vec![]);
                let point = labels
                    .iter()
                    .rev()
                    .find(|(text, _)| text == aurora_i18n::tr!("common-edit"))
                    .unwrap()
                    .1
                    .center();
                click(&ctx, &inv, &mut st, point)
            };
            let [InvAction::Edit(change)] = &actions[..] else {
                panic!("Expected one complete price update")
            };
            let [aurora_net::inventory::operations::Operation::Patch { body, .. }] = &change.operations[..] else {
                panic!("Expected a price patch")
            };
            assert_eq!(body["sale_info"]["sale_price"].as_i32(), price);
            if !enter {
                assert_ne!(body["permissions"]["next_owner_mask"].as_u32(), inv.items[&id].next_owner_mask);
            }
            rules::demo_mutate(&mut inv, Uuid::from_u128(2), change.clone()).unwrap();
            assert!(frame(&ctx, &inv, &mut st, vec![]).1.is_empty());
        }
        let (labels, _) = frame(&ctx, &inv, &mut st, vec![]);
        let start = price_point(&labels, inv.items[&id].sale_price);
        assert!(frame(&ctx, &inv, &mut st, vec![egui::Event::PointerMoved(start)]).1.is_empty());
        assert!(
            frame(
                &ctx,
                &inv,
                &mut st,
                vec![egui::Event::PointerButton {
                    pos: start,
                    pressed: true,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                }]
            )
            .1
            .is_empty()
        );
        for offset in [40.0, 70.0] {
            assert!(
                frame(
                    &ctx,
                    &inv,
                    &mut st,
                    vec![egui::Event::PointerMoved(start + egui::vec2(offset, 0.0))]
                )
                .1
                .is_empty(),
                "A numeric drag stays usable until release"
            );
        }
        let actions = frame(
            &ctx,
            &inv,
            &mut st,
            vec![egui::Event::PointerButton {
                pos: start + egui::vec2(70.0, 0.0),
                pressed: false,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            }],
        )
        .1;
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("Expected one update after the numeric drag")
        };
        let [aurora_net::inventory::operations::Operation::Patch { body, .. }] = &change.operations[..] else {
            panic!("Expected a dragged price patch")
        };
        assert!(body["sale_info"]["sale_price"].as_i32() > 345);
    }

    #[test]
    fn an_object_without_creator_shows_unknown_multiple_without_a_link() {
        let names = HashMap::new();
        assert_eq!(
            resident_label(Role::Creator, Uuid::nil(), &names),
            ResidentLabel::Plain(
                aurora_i18n::tr!("ui-inventory-properties-creator-unknown-multiple"),
                Some(aurora_i18n::tr!("ui-inventory-properties-creator-unknown-multiple-hint"))
            )
        );
        assert_eq!(
            resident_label(Role::Owner, Uuid::nil(), &names),
            ResidentLabel::Plain(aurora_i18n::tr!("common-unknown"), None)
        );
    }

    #[test]
    fn a_known_creator_is_a_link_whatever_the_permissions() {
        let creator = Uuid::from_u128(7);
        let names = HashMap::from([(creator, "Nova Exemple".to_owned())]);
        assert_eq!(resident_label(Role::Creator, creator, &names), ResidentLabel::Link("Nova Exemple"));
        // a name still loading keeps the link
        let other = Uuid::from_u128(8);
        assert_eq!(
            resident_label(Role::Creator, other, &names),
            ResidentLabel::Link(aurora_i18n::tr!("common-unknown"))
        );
    }

    #[test]
    fn a_no_modify_item_keeps_the_creator_it_was_sent() {
        let creator = Uuid::from_u128(9);
        let v = aurora_llsd::llsd_map! {
            "item_id" => Uuid::from_u128(1),
            "shadow_id" => Uuid::from_u128(2),
            "permissions" => aurora_llsd::llsd_map! {
                "creator_id" => creator,
                "owner_id" => Uuid::from_u128(3),
                "base_mask" => 0x0008_8000_i32,
                "owner_mask" => 0x0008_8000_i32,
            },
        };
        let it = aurora_net::inventory::item_from_llsd(&v).expect("item");
        assert_eq!(it.owner_mask & rules::MODIFY, 0);
        assert_eq!(it.creator, creator);
    }
}
