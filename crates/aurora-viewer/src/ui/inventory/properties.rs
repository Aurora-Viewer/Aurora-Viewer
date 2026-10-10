//! Compact LLFloaterProperties layout: thumbnail, resident names, masks and sale.
use super::*;
use aurora_llsd::llsd_map;
use std::collections::HashMap;

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
        ui.label("L’élément n’existe plus.");
        return false;
    };
    if st.property_edit.as_ref().is_none_or(|(item, _)| *item != id) {
        st.property_edit = Some((id, PropertyEdit::from(it)));
    }
    let Some((_, edit)) = &mut st.property_edit else {
        return false;
    };
    let can_edit = rules::mutable(inv, id, &prefs.protected) && !matches!(it.asset_type, 2 | 24 | 25) && st.pending.is_none();
    let can_describe = can_edit && it.owner_mask & rules::MODIFY != 0;
    let mut done = false;
    egui::ScrollArea::vertical().max_height(550.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
        ui.spacing_mut().interact_size.y = 22.0;
        ui.horizontal(|ui| {
            icon(ui, icons, item_icon(it.asset_type, it.inv_type), p);
            ui.add_enabled(can_describe, egui::TextEdit::singleline(&mut edit.name).char_limit(63).desired_width(ui.available_width()));
        });
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            thumbnail::picture(ui, p, it.thumbnail, 128.0, images, &mut st.wanted_images);
            ui.vertical(|ui| {
                for (label, resident) in [("Propriétaire :", it.owner), ("Créateur :", it.creator)] {
                    ui.label(RichText::new(label).size(12.0).color(p.muted));
                    let name = facts.names.get(&resident).map(String::as_str).unwrap_or("Inconnu");
                    if ui.link(RichText::new(name).size(12.0).color(p.violet_light)).on_hover_text(resident.to_string()).clicked() && !resident.is_nil() {
                        actions.push(InvAction::Profile(resident));
                    }
                }
                ui.label(RichText::new("Acquis le :").size(12.0).color(p.muted));
                ui.label(RichText::new(acquired(it.created_at)).size(12.0));
                if ui.add_enabled(can_edit, egui::Button::new("Photo…")).clicked() { st.thumbnail.open(id); }
            });
        });
        ui.label("Description :");
        ui.add_enabled(can_describe, egui::TextEdit::multiline(&mut edit.desc).char_limit(127).desired_rows(2).desired_width(ui.available_width()));
        ui.label(RichText::new("Expérience :").color(p.muted));
        ui.separator(); ui.label("Autorisations"); ui.label("Vous pouvez :");
        ui.horizontal(|ui| { for (mask, label) in rights() { let mut on = it.owner_mask & mask != 0; ui.add_enabled(false, egui::Checkbox::new(&mut on, label)); } });
        let may_share = can_edit && it.owner_mask & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER;
        ui.horizontal(|ui| { ui.label("N’importe qui :"); check_mask(ui, may_share, &mut edit.everyone, rules::COPY, "Copier"); });
        ui.horizontal(|ui| { ui.label("Groupe :");
            let group_share = rules::MODIFY | rules::MOVE | rules::COPY;
            check_mask(ui, may_share, &mut edit.group, group_share, "Partager");
        });
        ui.label("Prochain propriétaire :");
        ui.horizontal(|ui| { for (mask, label) in rights() {
            check_mask(ui, can_edit && it.base_mask & mask != 0 && (mask != rules::TRANSFER || edit.next & rules::COPY != 0), &mut edit.next, mask, label);
            if edit.next & rules::COPY == 0 { edit.next |= rules::TRANSFER; }
        } });
        ui.separator();
        let mut for_sale = edit.sale_type != 0;
        ui.horizontal(|ui| {
            if ui.add_enabled(can_edit && it.owner_mask & rules::TRANSFER != 0, egui::Checkbox::new(&mut for_sale, "À vendre")).changed() {
                edit.sale_type = if for_sale { if it.owner_mask & rules::COPY != 0 { 2 } else { 1 } } else { 0 };
            }
            ui.add_enabled_ui(for_sale && can_edit, |ui| {
                egui::ComboBox::from_id_salt("sale_type").selected_text(sale_name(edit.sale_type)).show_ui(ui, |ui| {
                    for value in 1..=3 { if ui.add_enabled(value != 2 || it.owner_mask & rules::COPY != 0, egui::Button::selectable(edit.sale_type == value, sale_name(value))).clicked() { edit.sale_type = value; } }
                });
            });
        });
        ui.horizontal(|ui| { ui.label("Prix : L$"); ui.add_enabled(for_sale && can_edit, egui::DragValue::new(&mut edit.price).range(0..=i32::MAX)); });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (label, mask) in [("B", it.base_mask), ("O", it.owner_mask), ("G", edit.group), ("E", edit.everyone), ("N", edit.next)] {
                ui.label(RichText::new(format!("{label}: {}", mask_text(mask))).size(11.0)).on_hover_text("B : base · O : propriétaire · G : groupe · E : tous · N : prochain propriétaire\nV : déplacer · M : modifier · C : copier · T : transférer");
            }
        });
        let modified = edit.name != it.name || edit.desc != it.desc || edit.next != it.next_owner_mask || edit.group != it.group_mask
            || edit.everyone != it.everyone_mask || edit.sale_type != it.sale_type || edit.price != it.sale_price;
        if ui.add_enabled(can_edit && modified && rules::clean_name(&edit.name).is_ok(), egui::Button::new("Enregistrer")).clicked() {
            let body = llsd_map! { "name" => rules::clean_name(&edit.name).unwrap_or_else(|_| it.name.clone()), "desc" => edit.desc.clone(),
                "permissions" => llsd_map! { "next_owner_mask" => edit.next as i32, "group_mask" => edit.group as i32, "everyone_mask" => edit.everyone as i32 },
                "sale_info" => llsd_map! { "sale_type" => i32::from(edit.sale_type), "sale_price" => edit.price } };
            match rules::patch(inv, id, body) { Ok(change) => { actions.push(InvAction::Edit(change)); done = true; }, Err(reason) => st.message = reason }
        }
    });
    done
}
fn rights() -> [(u32, &'static str); 3] {
    [
        (rules::MODIFY, "Modifier"),
        (rules::COPY, "Copier"),
        (rules::TRANSFER, "Transférer"),
    ]
}
fn check_mask(ui: &mut egui::Ui, enabled: bool, value: &mut u32, mask: u32, label: &str) {
    let mut on = *value & mask == mask;
    if ui.add_enabled(enabled, egui::Checkbox::new(&mut on, label)).changed() {
        if on {
            *value |= mask;
        } else {
            *value &= !mask;
        }
    }
}
fn sale_name(value: u8) -> &'static str {
    match value {
        1 => "Original",
        3 => "Contenu",
        _ => "Copie",
    }
}
fn mask_text(mask: u32) -> String {
    [(rules::MOVE, 'V'), (rules::MODIFY, 'M'), (rules::COPY, 'C'), (rules::TRANSFER, 'T')]
        .into_iter()
        .filter_map(|(bit, c)| (mask & bit != 0).then_some(c))
        .collect()
}
fn acquired(timestamp: i64) -> String {
    if timestamp <= 0 {
        return "Inconnue".into();
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
