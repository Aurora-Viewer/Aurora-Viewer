//! Inventory rename, properties, thumbnail and confirmation floaters.
//! Follows LLFloaterProperties / LLFloaterInventory (Firestorm, LGPL 2.1).

use super::*;
use crate::ui::widgets::{Floater, flat_button};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::Operation;

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let Some(mut dialog) = st.dialog.take() else {
        return;
    };
    let mut open = true;
    let mut done = false;
    let title = match &dialog {
        EditDialog::Rename(..) => "Renommer",
        EditDialog::Folder(..) => "Nouveau dossier",
        EditDialog::Properties(..) => "Propriétés",
        EditDialog::Delete(_, true) => "Purger la sélection",
        EditDialog::Delete(_, false) => "Supprimer la sélection",
        EditDialog::Empty(_) => "Vider la corbeille",
        EditDialog::Ungroup(_) => "Dégrouper le dossier",
        EditDialog::ReplaceLinks(..) => "Remplacer les liens",
        EditDialog::Discard(..) => "Enregistrer les modifications ?",
        EditDialog::Share(..) => "Partager la sélection",
    };
    Floater::new(
        &format!("inventory_dialog_{}", st.ui_id),
        title,
        ctx.content_rect().center() - egui::vec2(210.0, 130.0),
        Vec2::new(420.0, 260.0),
    )
    .show(ctx, p, &mut open, |ui| {
        match &mut dialog {
            EditDialog::Rename(id, name) | EditDialog::Folder(id, name) => {
                ui.label("Nom :");
                let r = ui.add(egui::TextEdit::singleline(name).char_limit(63).desired_width(ui.available_width()));
                if !r.has_focus() && ctx.input(|i| !i.pointer.any_pressed()) {
                    r.request_focus();
                }
                let enter = r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let valid = rules::clean_name(name).is_ok();
                ui.add_space(8.0);
                if ui
                    .add_enabled(valid && st.pending.is_none(), egui::Button::new("Enregistrer"))
                    .clicked()
                    || (enter && valid)
                {
                    let plan = if title == "Renommer" {
                        rules::clean_name(name).and_then(|name| rules::patch(inv, *id, llsd_map! { "name" => name }))
                    } else {
                        rules::create_folder(*id, name)
                    };
                    match plan {
                        Ok(change) => {
                            actions.push(InvAction::Edit(change));
                            done = true;
                        }
                        Err(reason) => st.message = reason,
                    }
                }
            }
            EditDialog::Properties(id) => {
                if let Some(it) = inv.items.get(id) {
                    if st.property_edit.as_ref().is_none_or(|(item, _)| item != id) { st.property_edit = Some((*id, PropertyEdit::from(it))); }
                    let Some((_, edit)) = &mut st.property_edit else { return; };
                    let can_edit = rules::mutable(inv, *id, &prefs.protected) && !matches!(it.asset_type, 2 | 24 | 25) && st.pending.is_none();
                    egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                        ui.label("Nom :"); ui.add_enabled(can_edit && it.owner_mask & rules::MODIFY != 0, egui::TextEdit::singleline(&mut edit.name).char_limit(63));
                        ui.label("Description :"); ui.add_enabled(can_edit && it.owner_mask & rules::MODIFY != 0, egui::TextEdit::multiline(&mut edit.desc).char_limit(127).desired_rows(2));
                        ui.add(egui::Label::new(it.id.to_string()).selectable(true));
                        ui.label(format!("Créateur : {}", it.creator)); ui.label(format!("Propriétaire : {}", it.owner));
                        ui.label(format!("Créé le {}", aurora_net::inventory::created_date(it.created_at)));
                        ui.label("Vos droits :");
                        for (mask, label) in [(rules::MODIFY, "Modifier"), (rules::COPY, "Copier"), (rules::TRANSFER, "Transférer")] {
                            let mut on = it.owner_mask & mask != 0; ui.add_enabled(false, egui::Checkbox::new(&mut on, label));
                        }
                        ui.separator(); ui.label("Droits du prochain propriétaire :");
                        for (mask, label) in [(rules::MODIFY, "Modifier"), (rules::COPY, "Copier"), (rules::TRANSFER, "Transférer")] {
                            let mut on = edit.next & mask != 0;
                            if ui.add_enabled(can_edit && it.base_mask & mask != 0 && (mask != rules::TRANSFER || edit.next & rules::COPY != 0), egui::Checkbox::new(&mut on, label)).changed() {
                                if on { edit.next |= mask; } else { edit.next &= !mask; }
                                if edit.next & rules::COPY == 0 { edit.next |= rules::TRANSFER; }
                            }
                        }
                        for (mask, label, value) in [(rules::MODIFY, "Partager avec le groupe", &mut edit.group), (rules::COPY, "Autoriser tout le monde à copier", &mut edit.everyone)] {
                            let mut on = *value & mask != 0;
                            if ui.add_enabled(can_edit && it.owner_mask & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER, egui::Checkbox::new(&mut on, label)).changed() {
                                if on { *value |= mask; } else { *value &= !mask; }
                            }
                        }
                        ui.separator(); ui.label("Vente :");
                        ui.add_enabled_ui(can_edit && it.owner_mask & rules::TRANSFER != 0, |ui| {
                            egui::ComboBox::from_id_salt("sale_type").selected_text(["Pas à vendre", "Original", "Copie", "Contenu"].get(edit.sale_type as usize).copied().unwrap_or("Pas à vendre")).show_ui(ui, |ui| {
                                for (value, label) in [(0, "Pas à vendre"), (1, "Original"), (2, "Copie"), (3, "Contenu")] {
                                    if ui.add_enabled(value != 2 || it.owner_mask & rules::COPY != 0, egui::Button::selectable(edit.sale_type == value, label)).clicked() { edit.sale_type = value; }
                                }
                            });
                            ui.add(egui::DragValue::new(&mut edit.price).range(0..=i32::MAX).suffix(" L$"));
                        });
                        let modified = edit.name != it.name || edit.desc != it.desc || edit.next != it.next_owner_mask || edit.group != it.group_mask || edit.everyone != it.everyone_mask || edit.sale_type != it.sale_type || edit.price != it.sale_price;
                        if ui.add_enabled(can_edit && modified && rules::clean_name(&edit.name).is_ok(), egui::Button::new("Enregistrer")).clicked() {
                            let body = llsd_map! { "name" => rules::clean_name(&edit.name).unwrap_or_else(|_| it.name.clone()), "desc" => edit.desc.clone(),
                                "permissions" => llsd_map! { "next_owner_mask" => edit.next as i32, "group_mask" => edit.group as i32, "everyone_mask" => edit.everyone as i32 },
                                "sale_info" => llsd_map! { "sale_type" => i32::from(edit.sale_type), "sale_price" => edit.price } };
                            match rules::patch(inv, *id, body) { Ok(change) => { actions.push(InvAction::Edit(change)); done = true; }, Err(reason) => st.message = reason }
                        }
                    });
                } else { ui.label("L’élément n’existe plus."); }
            }
            EditDialog::Discard(id) => {
                ui.label("Ce document contient des modifications non enregistrées.");
                if ui.add_enabled(!st.save_pending, egui::Button::new("Enregistrer")).clicked()
                    && let Some(Ok(text)) = &st.preview_text
                {
                    actions.push(InvAction::SaveContent {
                        item: *id,
                        text: text.clone(),
                    });
                    done = true;
                }
                if flat_button(ui, p, "Abandonner les modifications").clicked() {
                    st.preview = None;
                    st.preview_dirty = false;
                    done = true;
                }
            }
            EditDialog::Share(ids, resident) => {
                ui.label(format!("Envoyer {} élément(s) au résident {} ?", ids.len(), resident));
                if ids
                    .iter()
                    .any(|id| rules::offer(inv, *id, &prefs.protected, &facts.worn).is_ok_and(|bucket| rules::offer_has_no_copy(inv, &bucket)))
                {
                    ui.label(RichText::new("Les éléments non copiables quitteront votre inventaire.").color(p.warn));
                }
                if flat_button(ui, p, "Partager").clicked() {
                    actions.push(InvAction::Share {
                        items: ids.clone(),
                        resident: *resident,
                    });
                    done = true;
                }
            }
            EditDialog::ReplaceLinks(old, selected) => {
                if st.fetch_all { ui.label("Chargement de tous les dossiers avant le remplacement…"); }
                ui.label(format!("Remplacer les liens vers « {} » par :", rules::name(inv, *old)));
                let old_type = inv.items.get(old).map(|it| (it.asset_type, it.flags & 0xff));
                let mut candidates: Vec<_> = inv
                    .items
                    .values()
                    .filter(|it| {
                        it.id != *old && !matches!(it.asset_type, 24 | 25) && !rules::library(inv, it.id) && !rules::in_type(inv, it.id, 14)
                    })
                    .filter(|it| {
                        old_type
                            .is_some_and(|(kind, subtype)| it.asset_type == kind && (!matches!(kind, 5 | 13) || it.flags & 0xff == subtype))
                    })
                    .collect();
                candidates.sort_by_key(|it| it.name.to_lowercase());
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    for it in candidates {
                        if ui.selectable_label(*selected == Some(it.id), &it.name).clicked() {
                            *selected = Some(it.id);
                        }
                    }
                });
                if ui
                    .add_enabled(selected.is_some() && st.pending.is_none() && !st.fetch_all && !facts.appearance_busy, egui::Button::new("Remplacer"))
                    .clicked()
                    && let Some(new) = *selected {
                        let mut change = Mutation::default();
                        for link in inv
                            .items
                            .values()
                            .filter(|it| it.asset_type == 24 && it.asset_id == *old && !rules::in_type(inv, it.id, 14) && (rules::mutable(inv, it.id, &prefs.protected) || rules::in_type(inv, it.id, 46)))
                        {
                            let replacement = &inv.items[&new];
                            let outfit = rules::in_type(inv, link.id, 47) || rules::in_type(inv, link.id, 46);
                            let desc = if outfit { if replacement.asset_type == 5 { link.desc.clone() } else { String::new() } } else { replacement.desc.clone() };
                            change.operations.push(Operation::Links { parent: link.parent, links: vec![aurora_net::outfits::OutfitLink {
                                target: new, name: replacement.name.clone(), desc, folder: false, inv_type: replacement.inv_type,
                            }] });
                            change.operations.push(Operation::Delete { id: link.id, folder: false });
                            change.removed.push(link.id);
                            change.refresh.push(link.parent);
                        }
                        if change.operations.is_empty() {
                            st.message = "Aucun lien modifiable dans les dossiers chargés.".into();
                        } else {
                            actions.push(InvAction::Edit(change));
                        }
                        done = true;
                    }
            }
            EditDialog::Delete(ids, purge) => {
                ui.label(format!("{} élément(s) sélectionné(s).", ids.len()));
                ui.label(if *purge {
                    "Cette suppression est définitive."
                } else {
                    "Les éléments seront déplacés dans la corbeille."
                });
                ui.add_space(8.0);
                if ui
                    .add_enabled(st.pending.is_none(), egui::Button::new(if *purge { "Purger" } else { "Supprimer" }))
                    .clicked()
                {
                    let plan = if *purge {
                        let ids = rules::roots(inv, ids);
                        let refresh = ids.iter().filter_map(|id| rules::parent(inv, *id)).collect();
                        let operations = ids
                            .iter()
                            .map(|id| Operation::Delete {
                                id: *id,
                                folder: inv.folders.contains_key(id),
                            })
                            .collect();
                        if ids.iter().all(|id| {
                            rules::movable(inv, *id, &prefs.protected)
                                && rules::in_type(inv, *id, 14)
                                && !facts.worn.iter().any(|w| rules::under(inv, *w, *id))
                        }) {
                            Ok(Mutation {
                                operations,
                                refresh,
                                removed: ids,
                                ..Default::default()
                            })
                        } else {
                            Err("La sélection ne peut plus être purgée.".into())
                        }
                    } else {
                        rules::system(inv, 14)
                            .ok_or_else(|| "Corbeille introuvable.".to_owned())
                            .and_then(|to| rules::move_selection(inv, ids, to, &prefs.protected, &facts.worn))
                    };
                    match plan {
                        Ok(change) => {
                            actions.push(InvAction::Edit(change));
                            done = true;
                        }
                        Err(reason) => st.message = reason,
                    }
                }
            }
            EditDialog::Empty(id) => {
                ui.label("Tous les éléments de la corbeille seront supprimés définitivement.");
                if ui
                    .add_enabled(st.pending.is_none(), egui::Button::new("Vider la corbeille"))
                    .clicked()
                    && let Some(f) = inv.folders.get(id)
                        && f.info.type_default == 14
                        && f.state == FetchState::Fetched
                    {
                        let removed: Vec<_> = f.items.iter().chain(&f.children).copied().collect();
                        if !facts.worn.iter().any(|w| rules::under(inv, *w, *id)) {
                            actions.push(InvAction::Edit(Mutation {
                                operations: vec![Operation::Purge(*id)],
                                refresh: vec![*id],
                                removed,
                                ..Default::default()
                            }));
                            done = true;
                        } else {
                            st.message = "La corbeille contient un élément porté.".into();
                        }
                    }
            }
            EditDialog::Ungroup(id) => {
                ui.label("Déplacer le contenu dans le dossier parent et mettre le dossier vide dans la corbeille ?");
                if ui.add_enabled(st.pending.is_none(), egui::Button::new("Dégrouper")).clicked() {
                    match rules::ungroup(inv, *id, &prefs.protected, &facts.worn) {
                        Ok(change) => {
                            actions.push(InvAction::Edit(change));
                            done = true;
                        }
                        Err(reason) => st.message = reason,
                    }
                }
            }
        }
        if !st.message.is_empty() { ui.label(RichText::new(&st.message).color(p.warn)); }
        if flat_button(ui, p, "Fermer / Annuler").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = true;
        }
    });
    if open && !done {
        st.dialog = Some(dialog);
    }
}

pub(super) fn preview(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
) {
    let Some(id) = st.preview else {
        return;
    };
    let Some(it) = inv.items.get(&id) else {
        st.preview = None;
        return;
    };
    let mut open = true;
    Floater::new(
        &format!("inventory_preview_{}_{}", st.ui_id, id),
        it.name.clone(),
        ctx.content_rect().center() - egui::vec2(250.0, 200.0),
        Vec2::new(500.0, 400.0),
    )
    .show(ctx, p, &mut open, |ui| {
        if it.asset_type == 0 {
            st.wanted_images.insert(it.asset_id);
            if let Some(t) = images.get(&it.asset_id) {
                ui.add(egui::Image::new(t).max_size(Vec2::splat(350.0)).fit_to_original_size(1.0));
            } else {
                ui.label("Chargement de l’image…");
            }
        } else if matches!(it.asset_type, 20 | 1) {
            if flat_button(ui, p, "Jouer").clicked() {
                if it.asset_type == 20 {
                    actions.push(InvAction::Animation {
                        asset: it.asset_id,
                        local: false,
                        start: true,
                    });
                } else {
                    actions.push(InvAction::Sound(it.asset_id));
                }
            }
            if it.asset_type == 20 {
                if flat_button(ui, p, "Jouer localement").clicked() {
                    actions.push(InvAction::Animation {
                        asset: it.asset_id,
                        local: true,
                        start: true,
                    });
                }
                if flat_button(ui, p, "Arrêter").clicked()
                    && let Some((asset, local)) = st.playing
                {
                    actions.push(InvAction::Animation {
                        asset,
                        local,
                        start: false,
                    });
                }
            }
        } else {
            if it.asset_type == 56 && !it.asset_id.is_nil() && flat_button(ui, p, "Appliquer uniquement à moi-même").clicked() {
                actions.push(InvAction::Environment(it.asset_id, it.flags));
            }
            match &mut st.preview_text {
                Some(Ok(text)) => {
                    egui::ScrollArea::both().max_height(300.0).show(ui, |ui| {
                        if st.preview_can_edit && !st.save_pending {
                            let response = ui.add(
                                egui::TextEdit::multiline(text)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace),
                            );
                            st.preview_dirty |= response.changed();
                        } else {
                            ui.add(egui::Label::new(RichText::new(text.as_str()).monospace()).selectable(true));
                        }
                    });
                }
                Some(Err(reason)) => {
                    ui.label(RichText::new(reason.as_str()).color(p.warn));
                }
                None => {
                    ui.label("Chargement…");
                }
            }
            if matches!(it.asset_type, 7 | 10)
                && st.preview_can_edit
                && ui
                    .add_enabled(st.preview_dirty && !st.save_pending, egui::Button::new("Enregistrer"))
                    .clicked()
                && let Some(Ok(text)) = &st.preview_text
            {
                actions.push(InvAction::SaveContent {
                    item: it.id,
                    text: text.clone(),
                });
            }
            if !st.message.is_empty() {
                ui.label(RichText::new(&st.message).color(p.warn));
            }
        }
    });
    if !open {
        if st.preview_dirty || st.save_pending {
            st.dialog = Some(EditDialog::Discard(id));
            return;
        }
        st.preview = None;
        if let Some((asset, local)) = st.playing {
            actions.push(InvAction::Animation {
                asset,
                local,
                start: false,
            });
        }
    }
}
