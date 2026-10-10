//! LL{Folder,Item,Animation,Object,Wearable}Bridge::buildContextMenu and
//! menu_inventory.xml (Firestorm indra/newview, originally LGPL 2.1).

use super::*;
use crate::ui::menu;
use crate::world::appearance::Action as Wear;
use aurora_net::inventory::operations::metadata_favorite;

fn entry(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, enabled: bool, action: InvAction, actions: &mut Vec<InvAction>) {
    if menu::item_if(ui, p, icon, label, enabled) {
        actions.push(action);
    }
}
fn submit(st: &mut InventoryUi, actions: &mut Vec<InvAction>, plan: Result<Mutation, String>) {
    match plan {
        Ok(change) => actions.push(InvAction::Edit(change)),
        Err(reason) => {
            st.message = reason;
            st.consume_clipboard = false;
        }
    }
}
pub(super) fn open(inv: &Inventory, id: Uuid, st: &mut InventoryUi, actions: &mut Vec<InvAction>) {
    if st.preview_dirty || st.save_pending {
        st.message = "Enregistrez ou fermez le document ouvert avant d’en ouvrir un autre.".into();
        return;
    }
    let Some(original) = rules::original(inv, id) else {
        st.message = "Le lien est cassé.".into();
        return;
    };
    if inv.folders.contains_key(&original) {
        st.show_original(inv, original);
        return;
    }
    let Some(it) = inv.items.get(&original) else {
        return;
    };
    if it.asset_type == 2 {
        actions.push(InvAction::Profile(it.creator));
    } else if it.asset_type == 3 && !it.asset_id.is_nil() {
        actions.push(InvAction::TeleportLandmark(it.asset_id));
    } else if matches!(it.asset_type, 5 | 6 | 13) {
        actions.push(InvAction::Appearance(Wear::WearItem {
            item: original,
            replace: true,
            point: 0,
        }));
    } else {
        st.preview = Some(original);
        st.preview_text = None;
        actions.push(InvAction::Preview(original));
    }
}
fn new_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    enabled: bool,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    if menu::item_if(ui, p, "folder", "Nouveau dossier", enabled) {
        submit(st, actions, rules::create_folder(id, "Nouveau dossier"));
    }
    for (kind, icon, name) in [
        (NewItem::Script, "code", "Nouveau script"),
        (NewItem::Note, "note", "Nouvelle note"),
        (NewItem::Gesture, "hand-waving", "Nouveau geste"),
        (NewItem::Material, "cube", "Nouveau matériau"),
    ] {
        entry(
            ui,
            p,
            icon,
            name,
            enabled,
            InvAction::Create {
                parent: id,
                kind,
                name: name.into(),
            },
            actions,
        );
    }
    menu::submenu(ui, p, "t-shirt", "Nouveaux habits", enabled, |ui| {
        for (kind, name) in [
            (4, "Nouvelle chemise"),
            (5, "Nouveau pantalon"),
            (6, "Nouvelles chaussures"),
            (7, "Nouvelles chaussettes"),
            (8, "Nouvelle veste"),
            (12, "Nouvelle jupe"),
            (9, "Nouveaux gants"),
            (10, "Nouveau débardeur"),
            (11, "Nouveau caleçon"),
            (13, "Nouveau masque alpha"),
            (14, "Nouveau tatouage"),
            (16, "Nouvel environnement universel"),
            (15, "Nouvelles propriétés physiques"),
        ] {
            entry(
                ui,
                p,
                "t-shirt",
                name,
                enabled,
                InvAction::Create {
                    parent: id,
                    kind: NewItem::Wearable(kind),
                    name: name.into(),
                },
                actions,
            );
        }
    });
    menu::submenu(ui, p, "person", "Nouvelles parties du corps", enabled, |ui| {
        for (kind, name) in [
            (0, "Nouvelle silhouette"),
            (1, "Nouvelle peau"),
            (2, "Nouveaux cheveux"),
            (3, "Nouveaux yeux"),
        ] {
            entry(
                ui,
                p,
                "person",
                name,
                enabled,
                InvAction::Create {
                    parent: id,
                    kind: NewItem::Wearable(kind),
                    name: name.into(),
                },
                actions,
            );
        }
    });
    menu::submenu(ui, p, "sun", "Nouveaux paramètres", enabled, |ui| {
        for (kind, name) in [(0, "Nouveau ciel"), (1, "Nouvelle eau"), (2, "Nouveau cycle du jour")] {
            entry(
                ui,
                p,
                "sun",
                name,
                enabled,
                InvAction::Create {
                    parent: id,
                    kind: NewItem::Settings(kind),
                    name: name.into(),
                },
                actions,
            );
        }
    });
    menu::submenu(ui, p, "arrow-square-out", "Utiliser comme dossier par défaut pour", enabled, |ui| {
        for (slot, name) in [
            "Chargements d’images",
            "Chargements de sons",
            "Chargements d’animations",
            "Chargements de modèles",
            "Chargements de matériaux PBR",
        ]
        .iter()
        .enumerate()
        {
            if menu::check(ui, p, "arrow-square-out", name, prefs.upload_folders[slot] == id, enabled) {
                prefs.upload_folders[slot] = id;
            }
        }
    });
}

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    id: Uuid,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    // Scroll on small screens; an inactive entry must keep the menu open.
    // Area's initial height is 400 px; let the menu measure its full content
    // instead of permanently clipping it to that first-frame estimate.
    let height = (ui.ctx().content_rect().height() - 32.0).max(100.0);
    ui.set_max_height(height);
    egui::ScrollArea::vertical().max_height(height).show(ui, |ui| {
        let folder = inv.folders.get(&id);
        let item = inv.items.get(&id);
        let target = rules::original(inv, id);
        let original = target.and_then(|id| inv.items.get(&id));
        let kind = original.or(item).map_or(-1, |it| it.asset_type);
        let ids = st.selected_ids(id);
        let single = ids.len() == 1;
        let ready = st.pending.is_none();
        let writable = rules::mutable(inv, id, &prefs.protected);
        let movable = ids.iter().all(|id| rules::movable(inv, *id, &prefs.protected));
        let worn = target.is_some_and(|id| facts.worn.contains(&id));
        let trash = rules::in_type(inv, id, 14);
        let linked = item.is_some_and(|it| matches!(it.asset_type, 24 | 25));
        if trash {
            if folder.is_some_and(|f| f.info.type_default == 14) {
                if menu::item_if(
                    ui,
                    p,
                    "trash",
                    "Vider la corbeille",
                    ready && folder.is_some_and(|f| f.state == FetchState::Fetched && (!f.items.is_empty() || !f.children.is_empty())),
                ) {
                    st.dialog = Some(EditDialog::Empty(id));
                }
            } else {
                if menu::item_if(ui, p, "arrow-counter-clockwise", "Restaurer l’objet", ready && movable && single)
                    && let Some(to) = rules::default_folder(inv, id)
                {
                    submit(st, actions, rules::move_selection(inv, &ids, to, &prefs.protected, &facts.worn));
                }
                if menu::item_if(ui, p, "trash", "Purger l’objet", ready && movable && !worn) {
                    st.dialog = Some(EditDialog::Delete(ids.clone(), true));
                }
            }
            menu::separator(ui, p);
        } else {
            let share = ready && writable && ids.iter().all(|id| rules::offer(inv, *id, &prefs.protected, &facts.worn).is_ok());
            if menu::item_if(ui, p, "share-network", "Partager", share) {
                st.share_items = ids.clone();
                st.share_picker.open(&format!("inventory_share_{}", st.ui_id), false);
            }
            if let Some(f) = folder {
                new_menu(
                    ui,
                    p,
                    id,
                    ready && single && rules::destination(inv, id, &prefs.protected) && f.info.type_default != 16,
                    st,
                    prefs,
                    actions,
                );
                let content = crate::world::appearance::wearable_contents(inv, id).unwrap_or_default();
                let can_wear = single && !facts.appearance_busy && !content.is_empty() && !rules::library(inv, id);
                menu::separator(ui, p);
                entry(
                    ui,
                    p,
                    "t-shirt",
                    "Remplacer la tenue actuelle",
                    can_wear,
                    InvAction::Appearance(Wear::WearContents {
                        folder: id,
                        mode: crate::world::appearance::WearMode::ReplaceOutfit,
                    }),
                    actions,
                );
                entry(
                    ui,
                    p,
                    "plus",
                    "Ajouter à la tenue actuelle",
                    can_wear,
                    InvAction::Appearance(Wear::WearContents {
                        folder: id,
                        mode: crate::world::appearance::WearMode::Append,
                    }),
                    actions,
                );
                entry(
                    ui,
                    p,
                    "coat-hanger",
                    "Porter les objets",
                    can_wear,
                    InvAction::Appearance(Wear::WearContents {
                        folder: id,
                        mode: crate::world::appearance::WearMode::ReplaceItems,
                    }),
                    actions,
                );
                entry(
                    ui,
                    p,
                    "x-circle",
                    "Enlever de la tenue actuelle",
                    can_wear && content.iter().any(|l| facts.worn.contains(&l.target)),
                    InvAction::Appearance(Wear::RemoveOutfit(id)),
                    actions,
                );
                menu::separator(ui, p);
            } else {
                let opens = single && target.is_some() && matches!(kind, 0 | 1 | 2 | 3 | 7 | 10 | 20 | 21 | 56 | 57);
                if menu::item_if(ui, p, "folder-open", "Ouvrir", opens) {
                    open(inv, id, st, actions);
                }
                if menu::item_if(ui, p, "info", "Propriétés", single) {
                    st.property_edit = None;
                    st.dialog = Some(EditDialog::Properties(id));
                }
            }
            let rename = ready && single && rules::renameable(inv, id, &prefs.protected);
            if menu::item_if(ui, p, "pencil-simple", "Renommer", rename) {
                st.begin_rename(inv, id);
            }
            if menu::item_if(ui, p, "image", "Image…", ready && single && writable && !linked) {
                st.thumbnail.open(id);
            }
            if let Some(f) = folder {
                if matches!(f.info.type_default, -1 | 47) && !f.library {
                    let protected = prefs.protected.contains(&id);
                    if menu::item(ui, p, "lock-simple", if protected { "Déprotéger" } else { "Protéger" }) {
                        if protected {
                            prefs.protected.remove(&id);
                        } else {
                            prefs.protected.insert(id);
                        }
                    }
                }
                if menu::item_if(
                    ui,
                    p,
                    "arrows-clockwise",
                    "Recharger le dossier",
                    single && id != inv.root && id != inv.lib_root,
                ) {
                    st.message.clear();
                    actions.push(InvAction::Preview(id));
                }
                if menu::item_if(ui, p, "arrow-square-out", "Afficher dans une nouvelle fenêtre", single) {
                    st.windows.push(InventoryWindow {
                        id: Uuid::new_v4(),
                        root: if f.library { inv.lib_root } else { inv.root },
                        state: Box::new(InventoryUi::default()),
                        open: true,
                    });
                    if let Some(w) = st.windows.last_mut() {
                        w.state.show_original(inv, id);
                    }
                }
                menu::separator(ui, p);
                if menu::item_if(ui, p, "folder-open", "Ouvrir dans une nouvelle fenêtre", single) {
                    st.windows.push(InventoryWindow {
                        id: Uuid::new_v4(),
                        root: id,
                        state: Box::new(InventoryUi::default()),
                        open: true,
                    });
                }
            } else if let Some(it) = item {
                let knowable = !matches!(kind, 6 | 10 | 11 | 22 | 49)
                    && it.owner_mask & (rules::COPY | rules::MODIFY | rules::TRANSFER) == rules::COPY | rules::MODIFY | rules::TRANSFER;
                if menu::item_if(
                    ui,
                    p,
                    "copy",
                    "Copier l’UUID de l’objet",
                    single && knowable && !it.asset_id.is_nil(),
                ) {
                    ui.ctx().copy_text(it.asset_id.to_string());
                }
                if kind == 6 {
                    entry(
                        ui,
                        p,
                        "arrow-counter-clockwise",
                        "Remettre à la dernière position",
                        single && writable && !worn,
                        InvAction::Restore(target.unwrap_or(id)),
                        actions,
                    );
                }
            }
        }
        menu::separator(ui, p);
        let copyable = ids.iter().all(|id| {
            inv.folders.contains_key(id)
                || inv
                    .items
                    .get(id)
                    .is_some_and(|it| it.owner_mask & rules::COPY != 0 || !matches!(it.asset_type, 2 | 49))
        });
        if menu::item_if(ui, p, "copy", "Copier", ready && copyable) {
            st.clipboard = ids.clone();
            st.cut = false;
            if let Some(id) = ids.first() {
                ui.ctx().copy_text(id.to_string());
            }
        }
        if menu::item_if(ui, p, "arrow-u-up-left", "Couper", ready && movable) {
            st.clipboard = ids.clone();
            st.cut = true;
            if let Some(id) = ids.first() {
                ui.ctx().copy_text(id.to_string());
            }
        }
        let to = folder.map(|_| id).or_else(|| rules::parent(inv, id));
        let paste = ready
            && single
            && to.is_some_and(|to| rules::paste_allowed(inv, &st.clipboard, to, st.cut, false, &prefs.protected, &facts.worn));
        let paste_links = ready
            && single
            && to.is_some_and(|to| rules::paste_allowed(inv, &st.clipboard, to, false, true, &prefs.protected, &facts.worn))
            && !st.cut;
        if menu::item_if(ui, p, "clipboard-text", "Coller", paste)
            && let Some(to) = to
        {
            st.consume_clipboard = st.cut;
            submit(
                st,
                actions,
                rules::paste(inv, &st.clipboard, to, st.cut, false, &prefs.protected, &facts.worn),
            );
        }
        if menu::item_if(ui, p, "link", "Coller comme lien", paste_links)
            && let Some(to) = to
        {
            submit(
                st,
                actions,
                rules::paste(inv, &st.clipboard, to, false, true, &prefs.protected, &facts.worn),
            );
        }
        if linked {
            if menu::item_if(ui, p, "arrow-square-out", "Trouver l’original", single && target.is_some())
                && let Some(target) = target
            {
                st.show_original(inv, target);
            }
        } else if menu::item_if(ui, p, "link", "Trouver tous les liens", single) {
            st.search.clear();
            st.links_filter = Some(id);
            st.fetch_all = true;
            st.tab = 0;
        }
        if item.is_some()
            && menu::item_if(
                ui,
                p,
                "link",
                "Remplacer les liens",
                single && ready && target.is_some() && !rules::library(inv, id),
            )
        {
            st.dialog = Some(EditDialog::ReplaceLinks(target.unwrap_or(id), None));
            st.fetch_all = true;
        }
        if !trash {
            menu::separator(ui, p);
            let removable =
                movable && ids.iter().all(|id| !facts.worn.iter().any(|w| rules::under(inv, *w, *id))) && !rules::in_type(inv, id, 46);
            if menu::item_if(ui, p, "trash", "Supprimer", ready && removable) {
                st.dialog = Some(EditDialog::Delete(ids.clone(), false));
            }
            if folder.is_none()
                && menu::item_if(ui, p, "folder", "Déplacer vers le dossier par défaut", ready && movable && single)
                && let Some(to) = rules::default_folder(inv, id)
            {
                submit(st, actions, rules::move_selection(inv, &ids, to, &prefs.protected, &facts.worn));
            }
            menu::separator(ui, p);
            if menu::item_if(ui, p, "folder", "Créer un dossier à partir de la sélection", ready && movable) {
                submit(st, actions, rules::group(inv, &ids, &prefs.protected, &facts.worn));
            }
            if let Some(f) = folder
                && menu::item_if(
                    ui,
                    p,
                    "arrows-out",
                    "Dégrouper les éléments du dossier",
                    ready
                        && movable
                        && single
                        && f.state == FetchState::Fetched
                        && (!f.items.is_empty() || !f.children.is_empty())
                        && f.info.type_default != 47,
                )
            {
                st.dialog = Some(EditDialog::Ungroup(id));
            }
            let favorite = folder.map_or_else(|| item.is_some_and(|it| it.favorite), |f| f.info.favorite);
            if menu::item_if(
                ui,
                p,
                "star",
                if favorite { "Supprimer des favoris" } else { "Ajouter aux favoris" },
                ready && single && !rules::library(inv, id) && id != inv.root,
            ) {
                submit(st, actions, rules::patch(inv, id, metadata_favorite(!favorite)));
            }
            if folder.is_none() {
                menu::separator(ui, p);
                if kind == 20 {
                    let asset = original.map(|it| it.asset_id).unwrap_or_default();
                    entry(
                        ui,
                        p,
                        "play",
                        "Jouer dans Second Life",
                        single && !asset.is_nil(),
                        InvAction::Animation {
                            asset,
                            local: false,
                            start: true,
                        },
                        actions,
                    );
                    entry(
                        ui,
                        p,
                        "play",
                        "Jouer localement",
                        single && !asset.is_nil(),
                        InvAction::Animation {
                            asset,
                            local: true,
                            start: true,
                        },
                        actions,
                    );
                }
                if kind == 1 {
                    entry(
                        ui,
                        p,
                        "speaker-high",
                        "Jouer",
                        single && original.is_some_and(|it| !it.asset_id.is_nil()),
                        InvAction::Sound(original.map(|it| it.asset_id).unwrap_or_default()),
                        actions,
                    );
                }
                let selected_items: Vec<_> = ids.iter().filter_map(|id| rules::original(inv, *id)).collect();
                let wear = ready
                    && !facts.appearance_busy
                    && selected_items.len() == ids.len()
                    && selected_items.iter().all(|id| {
                        !facts.worn.contains(id)
                            && !rules::library(inv, *id)
                            && !rules::in_type(inv, *id, 14)
                            && inv.items.get(id).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13))
                    });
                let objects = selected_items
                    .iter()
                    .all(|id| inv.items.get(id).is_some_and(|it| it.asset_type == 6));
                let add = wear
                    && selected_items
                        .iter()
                        .all(|id| inv.items.get(id).is_some_and(|it| it.asset_type != 13));
                entry(
                    ui,
                    p,
                    "t-shirt",
                    "Porter",
                    wear && single,
                    InvAction::Appearance(Wear::WearItem {
                        item: target.unwrap_or(id),
                        replace: true,
                        point: 0,
                    }),
                    actions,
                );
                entry(
                    ui,
                    p,
                    "plus",
                    "Ajouter",
                    add,
                    InvAction::Appearance(Wear::WearItems {
                        items: selected_items.clone(),
                        replace: false,
                        point: 0,
                    }),
                    actions,
                );
                for hud in [false, true] {
                    menu::submenu(
                        ui,
                        p,
                        if hud { "monitor" } else { "person" },
                        if hud { "Attacher au HUD" } else { "Attacher à" },
                        wear && objects,
                        |ui| {
                            egui::ScrollArea::vertical().max_height(450.0).show(ui, |ui| {
                                for (point, is_hud, name) in facts.points.iter().filter(|(_, is_hud, _)| *is_hud == hud) {
                                    let label = if *is_hud { name.clone() } else { format!("{name} ({point})") };
                                    entry(
                                        ui,
                                        p,
                                        "person",
                                        &label,
                                        true,
                                        InvAction::Appearance(Wear::WearItems {
                                            items: selected_items.clone(),
                                            replace: false,
                                            point: *point,
                                        }),
                                        actions,
                                    );
                                }
                            });
                        },
                    );
                }
                if selected_items.iter().any(|id| facts.worn.contains(id)) {
                    entry(
                        ui,
                        p,
                        "x-circle",
                        if kind == 6 { "Détacher de vous" } else { "Enlever" },
                        ready
                            && !facts.appearance_busy
                            && selected_items.len() == ids.len()
                            && selected_items
                                .iter()
                                .all(|id| facts.worn.contains(id) && inv.items.get(id).is_some_and(|it| matches!(it.asset_type, 5 | 6))),
                        InvAction::Appearance(Wear::RemoveItems(selected_items)),
                        actions,
                    );
                }
            }
            if kind == 56
                && let Some(it) = original
            {
                entry(
                    ui,
                    p,
                    "sun",
                    "Appliquer uniquement à moi-même",
                    single && !it.asset_id.is_nil(),
                    InvAction::Environment(it.asset_id, it.flags),
                    actions,
                );
            }
            menu::separator(ui, p);
            for (copy, label) in [
                (true, "Copier dans les annonces Place du marché"),
                (false, "Déplacer dans les annonces Place du marché"),
            ] {
                let plan = rules::marketplace(inv, &ids, copy, &prefs.protected, &facts.worn);
                if menu::item_if(ui, p, "tag", label, ready && st.merchant && plan.is_ok()) {
                    submit(st, actions, plan);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, epaint::Shape};
    fn text_position(shape: &Shape, label: &str) -> Option<Pos2> {
        match shape {
            Shape::Text(t) if t.galley.text() == label => Some(t.pos),
            Shape::Vec(shapes) => shapes.iter().find_map(|s| text_position(s, label)),
            _ => None,
        }
    }
    fn frame(ctx: &egui::Context, inv: &Inventory, target: Uuid) -> egui::FullOutput {
        let p = crate::theme::Theme::default().palette();
        let mut state = InventoryUi {
            merchant: true,
            ..Default::default()
        };
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                ..Default::default()
            },
            |ui| {
                menu::popup_at(
                    ui.ctx(),
                    egui::Id::new("inventory_menu_test"),
                    egui::pos2(500.0, 80.0),
                    &mut true,
                    &p,
                    |ui| {
                        show(
                            ui,
                            &p,
                            inv,
                            target,
                            &mut state,
                            &mut InventoryPreferences::default(),
                            &Facts {
                                worn: HashSet::new(),
                                points: Vec::new(),
                                appearance_busy: false,
                                names: Default::default(),
                            },
                            &mut Vec::new(),
                        )
                    },
                );
            },
        )
    }
    #[test]
    fn the_entire_folder_menu_is_visible_including_the_last_marketplace_row() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        for _ in 0..4 {
            frame(&ctx, &inv, Uuid::from_u128(8000));
        }
        let output = frame(&ctx, &inv, Uuid::from_u128(8000));
        for label in [
            "Nouveau script",
            "Nouveaux habits",
            "Nouvelles parties du corps",
            "Nouveaux paramètres",
            "Renommer",
            "Recharger le dossier",
            "Couper",
            "Créer un dossier à partir de la sélection",
            "Déplacer dans les annonces Place du marché",
        ] {
            let (position, clip) = output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, label).map(|pos| (pos, s.clip_rect)))
                .unwrap_or_else(|| panic!("missing visible option: {label}"));
            assert!(clip.contains(position), "clipped {label}: {position:?}");
            assert!(position.y < 900.0);
        }
    }
    #[test]
    fn animation_menu_exposes_both_play_modes_and_the_attachment_submenus() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        for _ in 0..4 {
            frame(&ctx, &inv, Uuid::from_u128(8101));
        }
        let output = frame(&ctx, &inv, Uuid::from_u128(8101));
        for label in [
            "Ouvrir",
            "Propriétés",
            "Jouer dans Second Life",
            "Jouer localement",
            "Attacher à",
            "Attacher au HUD",
        ] {
            assert!(
                output.shapes.iter().any(|s| text_position(&s.shape, label).is_some()),
                "missing {label}"
            );
        }
    }

    #[test]
    fn add_and_detach_menu_actions_include_every_selected_object() {
        for detach in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let p = crate::theme::Theme::default().palette();
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let ids: Vec<_> = [8100, 8109, 8110].into_iter().map(Uuid::from_u128).collect();
            let mut state = InventoryUi {
                selection: ids.iter().copied().collect(),
                ..Default::default()
            };
            let facts = Facts {
                worn: if detach { ids.iter().copied().collect() } else { HashSet::new() },
                points: Vec::new(),
                appearance_busy: false,
                names: Default::default(),
            };
            let mut actions = Vec::new();
            let mut open = true;
            let mut frame = |events| {
                ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        menu::popup_at(
                            ui.ctx(),
                            egui::Id::new("batch_menu"),
                            egui::pos2(500.0, 80.0),
                            &mut open,
                            &p,
                            |ui| {
                                show(
                                    ui,
                                    &p,
                                    &inv,
                                    ids[0],
                                    &mut state,
                                    &mut InventoryPreferences::default(),
                                    &facts,
                                    &mut actions,
                                );
                            },
                        );
                    },
                )
            };
            for _ in 0..4 {
                let _ = frame(Vec::new());
            }
            let output = frame(Vec::new());
            let label = if detach { "Détacher de vous" } else { "Ajouter" };
            let pos = output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, label))
                .expect("batch action")
                + egui::vec2(4.0, 4.0);
            for pressed in [true, false] {
                let _ = frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            if detach {
                assert!(matches!(&actions[..], [InvAction::Appearance(Wear::RemoveItems(items))] if *items == ids));
            } else {
                assert!(
                    matches!(&actions[..], [InvAction::Appearance(Wear::WearItems { items, replace: false, point: 0 })] if *items == ids)
                );
            }
        }
    }
}
