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
    prefs: &InventoryPreferences,
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
    let can_edit = rules::mutable(inv, id, &prefs.protected) && !matches!(it.asset_type, 2 | 24 | 25) && st.pending.is_none();
    let response = panel(
        ui,
        p,
        Some(icons),
        PropertyPanel {
            inventory: Some(inv),
            item: it,
            edit,
            can_edit,
            can_next_owner: can_edit,
            can_describe: can_edit && it.owner_mask & rules::MODIFY != 0,
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
    if response.save {
        let body = llsd_map! { "name" => rules::clean_name(&edit.name).unwrap_or_else(|_| it.name.clone()), "desc" => edit.desc.clone(),
        "permissions" => llsd_map! { "next_owner_mask" => edit.next as i32, "group_mask" => edit.group as i32, "everyone_mask" => edit.everyone as i32 },
        "sale_info" => llsd_map! { "sale_type" => i32::from(edit.sale_type), "sale_price" => edit.price } };
        match rules::patch(inv, id, body) {
            Ok(change) => {
                actions.push(InvAction::Edit(change));
                return true;
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
    pub save: bool,
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
            ui.add_enabled(
                can_describe,
                egui::TextEdit::singleline(&mut edit.name)
                    .char_limit(63)
                    .desired_width(ui.available_width()),
            );
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
        ui.add_enabled(
            can_describe,
            egui::TextEdit::multiline(&mut edit.desc)
                .char_limit(127)
                .desired_rows(2)
                .desired_width(ui.available_width()),
        );
        ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-properties-experience")).color(p.muted));
        crate::ui::widgets::divider(ui, p);
        ui.label(aurora_i18n::tr!("ui-inventory-properties-permissions"));
        ui.label(aurora_i18n::tr!("ui-inventory-properties-you-can"));
        ui.horizontal(|ui| {
            for (mask, label) in rights() {
                let mut on = it.owner_mask & mask != 0;
                ui.add_enabled_ui(false, |ui| crate::ui::widgets::checkbox(ui, p, &mut on, label));
            }
        });
        let may_share = can_edit && it.owner_mask & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER;
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("ui-inventory-properties-anyone"));
            check_mask(ui, p, may_share, &mut edit.everyone, rules::COPY, aurora_i18n::tr!("common-copy-2"));
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("common-group-2"));
            let group_share = rules::MODIFY | rules::MOVE | rules::COPY;
            check_mask(ui, p, may_share, &mut edit.group, group_share, aurora_i18n::tr!("common-share"));
        });
        ui.label(aurora_i18n::tr!("ui-inventory-properties-next-owner"));
        if it.asset_type == 56 {
            edit.next |= rules::COPY;
        }
        ui.horizontal(|ui| {
            for (mask, label) in rights() {
                check_mask(
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
        });
        crate::ui::widgets::divider(ui, p);
        let mut for_sale = edit.sale_type != 0;
        ui.horizontal(|ui| {
            if ui
                .add_enabled_ui(can_edit && it.owner_mask & rules::TRANSFER != 0, |ui| {
                    crate::ui::widgets::checkbox(ui, p, &mut for_sale, aurora_i18n::tr!("ui-inventory-properties-for-sale"))
                })
                .inner
                .changed()
            {
                edit.sale_type = if for_sale {
                    if it.owner_mask & rules::COPY != 0 { 2 } else { 1 }
                } else {
                    0
                };
            }
            ui.add_enabled_ui(for_sale && can_edit, |ui| {
                egui::ComboBox::from_id_salt("sale_type")
                    .selected_text(sale_name(edit.sale_type))
                    .show_ui(ui, |ui| {
                        for value in 1..=3 {
                            if ui
                                .add_enabled(
                                    value != 2 || it.owner_mask & rules::COPY != 0,
                                    egui::Button::selectable(edit.sale_type == value, sale_name(value)),
                                )
                                .clicked()
                            {
                                edit.sale_type = value;
                            }
                        }
                    });
            });
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("ui-inventory-properties-price-l"));
            ui.add_enabled(
                for_sale && can_edit,
                egui::DragValue::new(&mut edit.price)
                    .custom_parser(aurora_i18n::parse_number)
                    .range(0..=i32::MAX),
            );
        });
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
        let modified = edit.name != it.name
            || edit.desc != it.desc
            || edit.next != it.next_owner_mask
            || edit.group != it.group_mask
            || edit.everyone != it.everyone_mask
            || edit.sale_type != it.sale_type
            || edit.price != it.sale_price;
        if ui
            .add_enabled(
                can_edit && modified && rules::clean_name(&edit.name).is_ok(),
                egui::Button::new(aurora_i18n::tr!("common-save")),
            )
            .clicked()
        {
            response.save = true;
        }
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
fn check_mask(ui: &mut egui::Ui, p: &Palette, enabled: bool, value: &mut u32, mask: u32, label: &str) {
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
    }
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
mod resident_tests {
    use super::*;

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
