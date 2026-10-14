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
pub(super) fn open(inv: &Inventory, id: Uuid, st: &mut InventoryUi, prefs: &InventoryPreferences, actions: &mut Vec<InvAction>) {
    if let Some(original) = rules::original(inv, id)
        && inv.items.get(&original).is_some_and(|it| it.asset_type == 21)
    {
        actions.push(InvAction::Gesture(crate::ui::gestures::Action::Open(original)));
        return;
    }
    if st.preview_dirty || st.save_pending {
        st.message = aurora_i18n::tr!("ui-inventory-context-save-or-close-the-open-document-before-opening-another").into();
        return;
    }
    let Some(original) = rules::original(inv, id) else {
        st.message = aurora_i18n::tr!("common-the-link-is-broken").into();
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
            replace: match it.asset_type {
                6 => !prefs.double_click_add_objects,
                // Physics and body parts remain replacements, like
                // LLWearableBridge::performAction / LLInvFVBridgeAction.
                5 if it.flags & 0xff != 15 => !prefs.double_click_add_clothes,
                _ => true,
            },
            point: 0,
        }));
    } else if it.asset_type == 10 {
        actions.push(InvAction::Preview(original));
    } else {
        st.preview = Some(original);
        st.reset_document();
        actions.push(InvAction::Preview(original));
    }
}
pub(super) fn new_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    enabled: bool,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    actions: &mut Vec<InvAction>,
) {
    if menu::item_if(ui, p, "folder-filled", aurora_i18n::tr!("common-new-folder"), enabled) {
        submit(st, actions, rules::create_folder(id, aurora_i18n::tr!("common-new-folder")));
    }
    for (kind, icon, name) in [
        (NewItem::Script, "code", aurora_i18n::tr!("common-new-script")),
        (NewItem::Note, "file-text-filled", aurora_i18n::tr!("common-new-notecard")),
        (NewItem::Gesture, "hand-move", aurora_i18n::tr!("ui-inventory-context-new-gesture")),
        (
            NewItem::Material,
            "droplet-filled",
            aurora_i18n::tr!("ui-inventory-context-new-material"),
        ),
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
    menu::submenu(
        ui,
        p,
        "shirt-filled",
        aurora_i18n::tr!("ui-inventory-context-new-clothes"),
        enabled,
        |ui| {
            for (kind, name) in [
                (4, aurora_i18n::tr!("common-new-shirt")),
                (5, aurora_i18n::tr!("ui-inventory-context-new-pants")),
                (6, aurora_i18n::tr!("common-new-shoes")),
                (7, aurora_i18n::tr!("common-new-socks")),
                (8, aurora_i18n::tr!("common-new-jacket")),
                (12, aurora_i18n::tr!("common-new-skirt")),
                (9, aurora_i18n::tr!("common-new-gloves")),
                (10, aurora_i18n::tr!("ui-inventory-context-new-tank-top")),
                (11, aurora_i18n::tr!("ui-inventory-context-new-underpants")),
                (13, aurora_i18n::tr!("ui-inventory-context-new-alpha-mask")),
                (14, aurora_i18n::tr!("common-new-tattoo")),
                (16, aurora_i18n::tr!("ui-inventory-context-new-universal")),
                (15, aurora_i18n::tr!("ui-inventory-context-new-physics-properties")),
            ] {
                entry(
                    ui,
                    p,
                    "shirt-filled",
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
        },
    );
    menu::submenu(
        ui,
        p,
        "man-filled",
        aurora_i18n::tr!("ui-inventory-context-new-body-parts"),
        enabled,
        |ui| {
            for (kind, name) in [
                (0, aurora_i18n::tr!("common-new-shape")),
                (1, aurora_i18n::tr!("common-new-skin")),
                (2, aurora_i18n::tr!("common-new-hair")),
                (3, aurora_i18n::tr!("common-new-eyes")),
            ] {
                entry(
                    ui,
                    p,
                    "man-filled",
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
        },
    );
    menu::submenu(
        ui,
        p,
        "sun-filled",
        aurora_i18n::tr!("ui-inventory-context-new-settings"),
        enabled,
        |ui| {
            for (kind, name) in [
                (0, aurora_i18n::tr!("ui-inventory-context-new-sky")),
                (1, aurora_i18n::tr!("ui-inventory-context-new-water")),
                (2, aurora_i18n::tr!("ui-inventory-context-new-day-cycle")),
            ] {
                entry(
                    ui,
                    p,
                    "sun-filled",
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
        },
    );
    menu::submenu(
        ui,
        p,
        "file-upload-filled",
        aurora_i18n::tr!("ui-inventory-context-use-as-default-folder-for"),
        enabled,
        |ui| {
            for (slot, name) in [
                aurora_i18n::tr!("ui-inventory-context-image-loads"),
                aurora_i18n::tr!("ui-inventory-context-sound-loads"),
                aurora_i18n::tr!("ui-inventory-context-animation-loads"),
                aurora_i18n::tr!("ui-inventory-context-model-loads"),
                aurora_i18n::tr!("ui-inventory-context-pbr-material-loads"),
            ]
            .iter()
            .enumerate()
            {
                if menu::check(ui, p, "file-upload-filled", name, prefs.upload_folders[slot] == id, enabled) {
                    prefs.upload_folders[slot] = id;
                }
            }
        },
    );
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
                    "trash-filled",
                    aurora_i18n::tr!("common-empty-trash"),
                    ready && folder.is_some_and(|f| f.state == FetchState::Fetched && (!f.items.is_empty() || !f.children.is_empty())),
                ) {
                    st.dialog = Some(EditDialog::Empty(id));
                }
            } else {
                if menu::item_if(
                    ui,
                    p,
                    "restore",
                    aurora_i18n::tr!("ui-inventory-context-restore-object"),
                    ready && movable && single,
                ) && let Some(to) = rules::default_folder(inv, id)
                {
                    submit(st, actions, rules::move_selection(inv, &ids, to, &prefs.protected, &facts.worn));
                }
                if menu::item_if(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("ui-inventory-context-purge-object"),
                    ready && super::delete::allowed(inv, &ids, prefs, facts),
                ) {
                    st.dialog = Some(EditDialog::Delete(ids.clone(), true));
                }
            }
            menu::separator(ui, p);
        } else {
            let share = ready && writable && ids.iter().all(|id| rules::offer(inv, *id, &prefs.protected, &facts.worn).is_ok());
            if menu::item_if(ui, p, "share", aurora_i18n::tr!("common-share"), share) {
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
                    "shirt-filled",
                    aurora_i18n::tr!("ui-inventory-context-replace-current-outfit"),
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
                    aurora_i18n::tr!("ui-inventory-context-add-to-current-outfit"),
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
                    "hanger-2-filled",
                    aurora_i18n::tr!("ui-inventory-context-wear-objects"),
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
                    "circle-x-filled",
                    aurora_i18n::tr!("ui-inventory-context-remove-from-current-outfit"),
                    can_wear && content.iter().any(|l| facts.worn.contains(&l.target)),
                    InvAction::Appearance(Wear::RemoveOutfit(id)),
                    actions,
                );
                menu::separator(ui, p);
            } else {
                let opens = single && target.is_some() && matches!(kind, 0 | 1 | 2 | 3 | 7 | 10 | 20 | 21 | 56 | 57);
                let (icon, label) = if kind == 3 {
                    ("location-filled", aurora_i18n::tr!("common-teleport"))
                } else {
                    ("folder-open-filled", aurora_i18n::tr!("common-open"))
                };
                if menu::item_if(ui, p, icon, label, opens) {
                    open(inv, id, st, prefs, actions);
                }
                if let Some(landmark) = original.filter(|it| it.asset_type == 3)
                    && menu::item_if(
                        ui,
                        p,
                        "info-circle-filled",
                        aurora_i18n::tr!("ui-inventory-context-about-landmark"),
                        single && !landmark.asset_id.is_nil(),
                    )
                {
                    actions.push(InvAction::AboutLandmark(landmark.id, landmark.asset_id));
                }
                if menu::item_if(ui, p, "info-circle-filled", aurora_i18n::tr!("common-properties"), single) {
                    st.property_edit = None;
                    st.dialog = Some(EditDialog::Properties(id));
                }
            }
            let rename = ready && single && rules::renameable(inv, id, &prefs.protected);
            if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename"), rename) {
                st.begin_rename(inv, id);
            }
            if menu::item_if(
                ui,
                p,
                "photo-filled",
                aurora_i18n::tr!("common-image-2"),
                ready && single && writable && !linked,
            ) {
                st.thumbnail.open(id);
            }
            if let Some(f) = folder {
                if matches!(f.info.type_default, -1 | 47) && !f.library {
                    let protected = prefs.protected.contains(&id);
                    if menu::item(
                        ui,
                        p,
                        "lock-filled",
                        if protected {
                            aurora_i18n::tr!("ui-inventory-context-unlock")
                        } else {
                            aurora_i18n::tr!("ui-inventory-context-lock")
                        },
                    ) {
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
                    "refresh",
                    aurora_i18n::tr!("ui-inventory-context-reload-folder"),
                    single && id != inv.root && id != inv.lib_root,
                ) {
                    st.message.clear();
                    actions.push(InvAction::Preview(id));
                }
                if menu::item_if(
                    ui,
                    p,
                    "external-link-filled",
                    aurora_i18n::tr!("ui-inventory-context-show-in-a-new-window"),
                    single,
                ) {
                    st.open_folder_window(id);
                }
                menu::separator(ui, p);
                if menu::item_if(
                    ui,
                    p,
                    "folder-open-filled",
                    aurora_i18n::tr!("ui-inventory-context-open-in-a-new-window"),
                    single,
                ) {
                    st.open_folder_window(id);
                }
            } else if let Some(it) = item {
                let knowable = !matches!(kind, 6 | 10 | 11 | 22 | 49)
                    && it.owner_mask & (rules::COPY | rules::MODIFY | rules::TRANSFER) == rules::COPY | rules::MODIFY | rules::TRANSFER;
                if menu::item_if(
                    ui,
                    p,
                    "copy-filled",
                    aurora_i18n::tr!("ui-inventory-context-copy-object-uuid"),
                    single && knowable && !it.asset_id.is_nil(),
                ) {
                    ui.ctx().copy_text(it.asset_id.to_string());
                }
                // listed for an object that is not worn (LLObjectBridge::buildContextMenu)
                if kind == 6 && !worn {
                    entry(
                        ui,
                        p,
                        "restore",
                        aurora_i18n::tr!("ui-inventory-context-return-to-last-location"),
                        single && writable,
                        InvAction::Restore(target.unwrap_or(id)),
                        actions,
                    );
                }
            }
        }
        menu::separator(ui, p);
        let copyable = clipboard::copy_allowed(inv, &ids);
        if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-2"), ready && copyable) {
            clipboard::store(ui.ctx(), st, ids.clone(), false);
        }
        if menu::item_if(ui, p, "cut", aurora_i18n::tr!("common-cut"), ready && movable) {
            clipboard::store(ui.ctx(), st, ids.clone(), true);
        }
        if menu::item_if(ui, p, "folder-filled", aurora_i18n::tr!("inventory-move-to"), ready && movable) {
            st.begin_move_to(id);
        }
        let to = folder.map(|_| id).or_else(|| rules::parent(inv, id));
        let paste = ready
            && single
            && to.is_some_and(|to| rules::paste_allowed(inv, &st.clipboard, to, st.cut, false, &prefs.protected, &facts.worn));
        let paste_links = ready
            && single
            && to.is_some_and(|to| rules::paste_allowed(inv, &st.clipboard, to, false, true, &prefs.protected, &facts.worn))
            && !st.cut;
        if menu::item_if(ui, p, "clipboard-text-filled", aurora_i18n::tr!("common-paste"), paste)
            && let Some(to) = to
        {
            clipboard::paste(inv, to, false, st, prefs, facts, actions);
        }
        if menu::item_if(
            ui,
            p,
            "link-filled",
            aurora_i18n::tr!("ui-inventory-context-paste-as-link"),
            paste_links,
        ) && let Some(to) = to
        {
            clipboard::paste(inv, to, true, st, prefs, facts, actions);
        }
        if linked {
            if menu::item_if(
                ui,
                p,
                "external-link-filled",
                aurora_i18n::tr!("ui-inventory-context-find-original"),
                single && target.is_some(),
            ) && let Some(target) = target
            {
                st.show_original(inv, target);
            }
        } else if menu::item_if(
            ui,
            p,
            "link-filled",
            aurora_i18n::tr!("ui-inventory-context-find-all-links"),
            single,
        ) {
            st.search.clear();
            if st.saved_filters.is_none() {
                st.saved_filters = Some(st.filters.clone());
            }
            st.filters.links = super::view::Links::Only;
            st.filters.always_folders = false;
            st.filters_initialized = true;
            st.links_filter = Some(id);
            st.fetch_all = true;
            st.tab = 0;
        }
        if item.is_some()
            && menu::item_if(
                ui,
                p,
                "link-filled",
                aurora_i18n::tr!("common-replace-links"),
                single && ready && target.is_some() && !rules::library(inv, id),
            )
        {
            st.dialog = Some(EditDialog::ReplaceLinks(target.unwrap_or(id), None));
            st.fetch_all = true;
        }
        if !trash {
            menu::separator(ui, p);
            let removable = super::delete::allowed(inv, &ids, prefs, facts);
            if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), ready && removable) {
                st.dialog = Some(EditDialog::Delete(ids.clone(), false));
            }
            if folder.is_none()
                && menu::item_if(
                    ui,
                    p,
                    "folder-filled",
                    aurora_i18n::tr!("ui-inventory-context-move-to-default-folder"),
                    ready && movable && single,
                )
                && let Some(to) = rules::default_folder(inv, id)
            {
                submit(st, actions, rules::move_selection(inv, &ids, to, &prefs.protected, &facts.worn));
            }
            menu::separator(ui, p);
            if menu::item_if(
                ui,
                p,
                "folder-filled",
                aurora_i18n::tr!("ui-inventory-context-create-folder-from-selection"),
                ready && movable,
            ) {
                submit(st, actions, rules::group(inv, &ids, &prefs.protected, &facts.worn));
            }
            if let Some(f) = folder
                && menu::item_if(
                    ui,
                    p,
                    "arrows-maximize",
                    aurora_i18n::tr!("ui-inventory-context-ungroup-folder-items"),
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
                "star-filled",
                if favorite {
                    aurora_i18n::tr!("common-remove-from-favorites")
                } else {
                    aurora_i18n::tr!("common-add-to-favorites")
                },
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
                        "player-play-filled",
                        aurora_i18n::tr!("ui-inventory-context-play-in-second-life"),
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
                        "player-play-filled",
                        aurora_i18n::tr!("common-play-locally"),
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
                        "volume",
                        aurora_i18n::tr!("common-play"),
                        single && original.is_some_and(|it| !it.asset_id.is_nil()),
                        InvAction::Sound(original.map(|it| it.asset_id).unwrap_or_default()),
                        actions,
                    );
                }
                let selected_items: Vec<_> = ids.iter().filter_map(|id| rules::original(inv, *id)).collect();
                if kind == 21 {
                    let active = selected_items.iter().all(|id| facts.active_gestures.contains(id));
                    entry(
                        ui,
                        p,
                        "player-play-filled",
                        aurora_i18n::tr!("common-play"),
                        single && ready,
                        InvAction::Gesture(crate::ui::gestures::Action::Play(target.unwrap_or(id))),
                        actions,
                    );
                    entry(
                        ui,
                        p,
                        "check",
                        if active {
                            aurora_i18n::tr!("gesture-deactivate")
                        } else {
                            aurora_i18n::tr!("gesture-activate")
                        },
                        ready && !selected_items.is_empty() && selected_items.iter().all(|id| !rules::library(inv, *id)),
                        InvAction::Gesture(crate::ui::gestures::Action::Active(selected_items.clone(), !active)),
                        actions,
                    );
                }
                if single
                    && original.is_some_and(|it| it.asset_type == 13 && it.flags & 0xff == 0 && facts.worn.contains(&it.id))
                    && menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit"))
                {
                    crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::EditWorn(target.unwrap_or(id)));
                }
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
                // LLObjectBridge::buildContextMenu: a worn object lists
                // "Touch", "Edit" and "Detach from yourself" in place
                // of the entries that wear it
                let worn_objects = objects && !selected_items.is_empty() && selected_items.iter().all(|id| facts.worn.contains(id));
                if worn_objects {
                    let item = target.unwrap_or(id);
                    if menu::item_if(
                        ui,
                        p,
                        "click-filled",
                        aurora_i18n::tr!("ui-context-touch"),
                        single && facts.touchable.contains(&item),
                    ) {
                        crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::TouchWorn(item));
                    }
                    // get_is_item_editable: any worn object
                    if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit"), single) {
                        crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::EditWorn(item));
                    }
                } else {
                    entry(
                        ui,
                        p,
                        "shirt-filled",
                        aurora_i18n::tr!("common-wear"),
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
                        aurora_i18n::tr!("common-add"),
                        add,
                        InvAction::Appearance(Wear::WearItems {
                            items: selected_items.clone(),
                            replace: false,
                            point: 0,
                        }),
                        actions,
                    );
                }
                let attach_menus: &[bool] = if worn_objects { &[] } else { &[false, true] };
                for &hud in attach_menus {
                    menu::submenu(
                        ui,
                        p,
                        if hud { "device-desktop-filled" } else { "man-filled" },
                        if hud {
                            aurora_i18n::tr!("common-attach-to-hud")
                        } else {
                            aurora_i18n::tr!("common-attach-to")
                        },
                        wear && objects,
                        |ui| {
                            egui::ScrollArea::vertical().max_height(450.0).show(ui, |ui| {
                                for (point, is_hud, name) in facts.points.iter().filter(|(_, is_hud, _)| *is_hud == hud) {
                                    let label = if *is_hud { name.clone() } else { format!("{name} ({point})") };
                                    entry(
                                        ui,
                                        p,
                                        "man-filled",
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
                        "circle-x-filled",
                        if kind == 6 {
                            aurora_i18n::tr!("ui-inventory-context-detach-from-yourself")
                        } else {
                            aurora_i18n::tr!("common-remove")
                        },
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
                    "sun-filled",
                    aurora_i18n::tr!("common-apply-to-myself-only"),
                    single && !it.asset_id.is_nil(),
                    InvAction::Environment(it.asset_id, it.flags),
                    actions,
                );
            }
            menu::separator(ui, p);
            for (copy, label) in [
                (true, aurora_i18n::tr!("ui-inventory-context-copy-to-marketplace-listings")),
                (false, aurora_i18n::tr!("ui-inventory-context-move-to-marketplace-listings")),
            ] {
                let plan = rules::marketplace(inv, &ids, copy, &prefs.protected, &facts.worn);
                if menu::item_if(ui, p, "tag-filled", label, ready && st.merchant && plan.is_ok()) {
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
        let mut state = InventoryUi {
            merchant: true,
            ..Default::default()
        };
        menu_frame(ctx, inv, target, &mut state, Vec::new())
    }
    fn menu_frame(
        ctx: &egui::Context,
        inv: &Inventory,
        target: Uuid,
        state: &mut InventoryUi,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let p = crate::theme::Theme::default().palette();
        headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                events,
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
                            state,
                            &mut InventoryPreferences::default(),
                            &Facts {
                                agent: Uuid::nil(),
                                worn: HashSet::new(),
                                worn_labels: Default::default(),
                                points: Vec::new(),
                                appearance_busy: false,
                                active_gestures: Default::default(),
                                touchable: Default::default(),
                                names: Default::default(),
                            },
                            &mut Vec::new(),
                        )
                    },
                );
            },
        ))
    }
    #[test]
    fn landmark_menu_keeps_teleport_and_the_place_profile_from_places() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        let mut landmark = inv.items[&Uuid::from_u128(8103)].clone();
        landmark.id = Uuid::from_u128(8201);
        landmark.asset_type = 3;
        landmark.inv_type = 3;
        inv.add_items(vec![landmark]);
        for _ in 0..4 {
            frame(&ctx, &inv, Uuid::from_u128(8201));
        }
        let output = frame(&ctx, &inv, Uuid::from_u128(8201));
        for label in [
            aurora_i18n::tr!("common-teleport"),
            aurora_i18n::tr!("ui-inventory-context-about-landmark"),
            aurora_i18n::tr!("common-properties"),
            aurora_i18n::tr!("common-rename"),
            aurora_i18n::tr!("common-delete"),
        ] {
            assert!(
                output.shapes.iter().any(|s| text_position(&s.shape, label).is_some()),
                "missing {label}"
            );
        }
    }
    #[test]
    fn folder_window_menu_targets_the_clicked_folder_including_the_library() {
        for target in [8000, 8003] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let target = Uuid::from_u128(target);
            let mut st = InventoryUi {
                search: "main inventory filter".into(),
                ..Default::default()
            };
            for _ in 0..4 {
                menu_frame(&ctx, &inv, target, &mut st, Vec::new());
            }
            let output = menu_frame(&ctx, &inv, target, &mut st, Vec::new());
            let pos = output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, aurora_i18n::tr!("ui-inventory-context-show-in-a-new-window")))
                .expect("folder window option")
                + egui::vec2(4.0, 4.0);
            for pressed in [true, false] {
                menu_frame(
                    &ctx,
                    &inv,
                    target,
                    &mut st,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(st.windows.len(), 1);
            assert_eq!(st.windows[0].root, target);
            assert!(st.windows[0].state.search.is_empty());
            assert_eq!(st.search, "main inventory filter");
        }
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
            aurora_i18n::tr!("common-new-script"),
            aurora_i18n::tr!("ui-inventory-context-new-clothes"),
            aurora_i18n::tr!("ui-inventory-context-new-body-parts"),
            aurora_i18n::tr!("ui-inventory-context-new-settings"),
            aurora_i18n::tr!("common-rename"),
            aurora_i18n::tr!("ui-inventory-context-reload-folder"),
            aurora_i18n::tr!("common-cut"),
            aurora_i18n::tr!("ui-inventory-context-create-folder-from-selection"),
            aurora_i18n::tr!("ui-inventory-context-move-to-marketplace-listings"),
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
            aurora_i18n::tr!("common-open"),
            aurora_i18n::tr!("common-properties"),
            aurora_i18n::tr!("ui-inventory-context-play-in-second-life"),
            aurora_i18n::tr!("common-play-locally"),
            aurora_i18n::tr!("common-attach-to"),
            aurora_i18n::tr!("common-attach-to-hud"),
        ] {
            assert!(
                output.shapes.iter().any(|s| text_position(&s.shape, label).is_some()),
                "missing {label}"
            );
        }
    }

    #[test]
    fn worn_object_menu_touches_edits_and_detaches_instead_of_wearing() {
        use crate::ui::context::CtxAction;
        let item = Uuid::from_u128(8100);
        // the menu of the worn item, then a click on `label`: what it asked for
        let click = |touchable: bool, label: &str| {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let p = crate::theme::Theme::default().palette();
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let mut state = InventoryUi::default();
            let facts = Facts {
                agent: Uuid::nil(),
                worn: HashSet::from([item]),
                active_gestures: Default::default(),
                worn_labels: Default::default(),
                points: Vec::new(),
                appearance_busy: false,
                touchable: if touchable { HashSet::from([item]) } else { HashSet::new() },
                names: Default::default(),
            };
            let mut actions = Vec::new();
            let mut open = true;
            let mut frame = |events| {
                headless_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        menu::popup_at(ui.ctx(), egui::Id::new("worn_menu"), egui::pos2(500.0, 80.0), &mut open, &p, |ui| {
                            show(
                                ui,
                                &p,
                                &inv,
                                item,
                                &mut state,
                                &mut InventoryPreferences::default(),
                                &facts,
                                &mut actions,
                            );
                        });
                    },
                ))
            };
            for _ in 0..4 {
                let _ = frame(Vec::new());
            }
            let output = frame(Vec::new());
            let find = |label: &str| output.shapes.iter().find_map(|s| text_position(&s.shape, label));
            // LLObjectBridge::buildContextMenu, worn: no entry that wears it
            for hidden in [
                aurora_i18n::tr!("common-wear"),
                aurora_i18n::tr!("common-add"),
                aurora_i18n::tr!("common-attach-to"),
                aurora_i18n::tr!("common-attach-to-hud"),
                aurora_i18n::tr!("ui-inventory-context-return-to-last-location"),
            ] {
                assert!(find(hidden).is_none(), "unexpected {hidden}");
            }
            assert!(find(aurora_i18n::tr!("ui-inventory-context-detach-from-yourself")).is_some());
            let pos = find(label).unwrap_or_else(|| panic!("missing {label}")) + egui::vec2(4.0, 4.0);
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
            crate::ui::context::take_requests(&ctx)
        };
        let touch = aurora_i18n::tr!("ui-context-touch");
        let edit = aurora_i18n::tr!("common-edit");
        // "Touch" only when the object handles touches
        assert!(matches!(&click(true, touch)[..], [CtxAction::TouchWorn(id)] if *id == item));
        assert!(click(false, touch).is_empty());
        assert!(matches!(&click(false, edit)[..], [CtxAction::EditWorn(id)] if *id == item));
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
                agent: Uuid::nil(),
                worn: if detach { ids.iter().copied().collect() } else { HashSet::new() },
                worn_labels: Default::default(),
                points: Vec::new(),
                appearance_busy: false,
                active_gestures: Default::default(),
                touchable: Default::default(),
                names: Default::default(),
            };
            let mut actions = Vec::new();
            let mut open = true;
            let mut frame = |events| {
                headless_output(ctx.run_ui(
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
                ))
            };
            for _ in 0..4 {
                let _ = frame(Vec::new());
            }
            let output = frame(Vec::new());
            let label = if detach {
                aurora_i18n::tr!("ui-inventory-context-detach-from-yourself")
            } else {
                aurora_i18n::tr!("common-add")
            };
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
