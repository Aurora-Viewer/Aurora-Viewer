//! LL{Folder,Item,Animation,Object,Wearable}Bridge::buildContextMenu and
//! menu_inventory.xml (Firestorm indra/newview, originally LGPL 2.1).

use super::*;
use crate::ui::menu;
use crate::world::appearance::Action as Wear;

/// LLInvFVBridge::getClipboardEntries / LLAssetType::lookupIsAssetIDKnowable
/// (indra/newview/llinventorybridge.cpp, indra/llcommon/llassettype.cpp).
/// An inventory item's UUID is not its asset UUID. Links have their own
/// clipboard entries and must never expose their target item as an asset.
pub(super) fn exposes_asset_uuid(kind: i32) -> bool {
    matches!(kind, 0 | 1 | 3 | 5 | 7 | 13 | 20 | 21 | 56 | 57 | 58 | 59)
}

pub(super) fn copyable_asset_uuid(item: &aurora_net::inventory::InvItem) -> Option<Uuid> {
    let full_perm = rules::COPY | rules::MODIFY | rules::TRANSFER;
    (exposes_asset_uuid(item.asset_type) && item.owner_mask & full_perm == full_perm && !item.asset_id.is_nil()).then_some(item.asset_id)
}

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
/// A double-click on an item row: the bridge's `openItem`
/// (indra/newview/llinventorybridge.cpp, originally LGPL 2.1). Objects,
/// clothing and body parts toggle (`wear_toggle`); one of the Library is
/// copied into the inventory, then worn (LLAppearanceMgr::wearItemsOnAvatar,
/// copy_inventory_item with wear_on_avatar_cb). A sound plays in the world
/// for everyone (LLSoundBridgeAction: send_sound_trigger, where "Open"
/// shows its preview). Every other type opens like the "Open" entry. A
/// link acts on its target, like LLInvFVBridgeAction::doAction.
pub(super) fn double_click(
    inv: &Inventory,
    id: Uuid,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    let original = rules::original(inv, id);
    match original.and_then(|original| inv.items.get(&original)) {
        Some(it) if matches!(it.asset_type, 5 | 6 | 13) => match wear_toggle(it, facts.worn.contains(&it.id), prefs) {
            Some(Wear::WearItem { item, replace, .. }) if rules::library(inv, item) => {
                actions.push(InvAction::WearCopy { item, replace });
            }
            wear => actions.extend(wear.map(InvAction::Appearance)),
        },
        Some(it) if it.asset_type == 1 => {
            // send_sound_trigger sends nothing for a null sound
            if !it.asset_id.is_nil() {
                actions.push(InvAction::TriggerSound(it.asset_id));
            }
        }
        _ => open(inv, id, st, actions),
    }
}

/// What a double-click does to a wearable, by LLObjectBridge::openItem and
/// LLWearableBridge::openItem: a worn object detaches and worn clothing
/// comes off (LLAppearanceMgr::removeItemFromAvatar); a worn body part
/// stays, since it can only be replaced (`canRemoveFromAvatar`). Anything
/// not worn is put on, added or replacing as the double-click preferences
/// say (FSDoubleClickAddInventoryObjects / FSDoubleClickAddInventoryClothing);
/// physics and body parts always replace.
pub(super) fn wear_toggle(it: &aurora_net::inventory::InvItem, worn: bool, prefs: &InventoryPreferences) -> Option<Wear> {
    if worn {
        return (it.asset_type != 13).then_some(Wear::Remove(it.id));
    }
    let replace = match it.asset_type {
        6 => !prefs.double_click_add_objects,
        5 if it.flags & 0xff != 15 => !prefs.double_click_add_clothes,
        _ => true,
    };
    Some(Wear::WearItem {
        item: it.id,
        replace,
        point: 0,
    })
}

/// The "Open" entry, and the double-click of what is not worn: objects,
/// clothing and body parts never come here (`double_click`).
pub(super) fn open(inv: &Inventory, id: Uuid, st: &mut InventoryUi, actions: &mut Vec<InvAction>) {
    if let Some(original) = rules::original(inv, id)
        && inv.items.get(&original).is_some_and(|it| it.asset_type == 21)
    {
        actions.push(InvAction::Gesture(crate::ui::gestures::Action::Open(original)));
        return;
    }
    // Resource windows are independent of document editors and their drafts.
    if let Some(item) = crate::ui::texture_library::catalogue::resource(inv, id) {
        actions.push(InvAction::Preview(item.id));
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
    } else if it.asset_type == 10 {
        actions.push(InvAction::Preview(original));
    } else if it.asset_type == 56 && crate::world::env_select::SettingsKind::from_flags(it.flags).is_some() {
        // LLSettingsBridgeAction: a sky, water or day cycle opens its editor
        actions.push(InvAction::EditEnvironment(original));
    } else {
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
    entry(
        ui,
        p,
        "shield-lock-filled",
        aurora_i18n::tr!("inventory-default-permissions"),
        true,
        InvAction::DefaultPermissions,
        actions,
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
    let mut catalog = super::context_catalog::build(inv, id, st, prefs, facts);
    let scope = super::context_catalog::scope(inv, id);
    prefs.context_menus.unify();
    st.menu_editor
        .prepare(ui.ctx(), id, ui.layer_id().id, &scope, &mut prefs.context_menus);
    let profile = prefs.context_menus.profiles.entry(scope.clone()).or_default();
    if profile.locked
        && let Some(folder) = catalog.prepare_outfit
    {
        actions.push(InvAction::PrepareOutfit(folder));
    }
    let editing = (!profile.locked).then(|| super::context_catalog::editing_entries(&catalog.entries, facts));
    let entries = editing.as_deref().unwrap_or(&catalog.entries);
    if let Some(index) = super::context_customize::show(ui, p, id, &scope, entries, profile, &mut st.menu_editor)
        && let Some(command) = catalog.commands.get_mut(index).and_then(Option::take)
    {
        super::context_catalog::execute(command, ui, inv, st, prefs, facts, actions);
        ui.close();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, epaint::Shape};
    #[test]
    fn resource_previews_preserve_an_unsaved_document() {
        let mut inv = Inventory::default();
        let document = Uuid::from_u128(100);
        let mut st = InventoryUi {
            preview: Some(document),
            preview_dirty: true,
            preview_text: Some(Ok("unsaved draft".into())),
            ..Default::default()
        };
        for (number, asset_type) in [(101, 0), (102, 57)] {
            let id = Uuid::from_u128(number);
            inv.add_items(vec![aurora_net::inventory::InvItem {
                id,
                asset_type,
                ..Default::default()
            }]);
            let mut actions = Vec::new();
            open(&inv, id, &mut st, &mut actions);
            assert!(matches!(actions.as_slice(), [InvAction::Preview(item)] if *item == id));
            assert_eq!(st.preview, Some(document));
            assert!(st.preview_dirty);
            assert_eq!(st.preview_text, Some(Ok("unsaved draft".into())));
            assert!(st.message.is_empty());
        }
    }
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
                    egui::Id::unique("inventory_menu_test"),
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
                                worn_at: Default::default(),
                                points: Vec::new(),
                                appearance_busy: false,
                                active_gestures: Default::default(),
                                touchable: Default::default(),
                                names: Default::default(),
                                ..Default::default()
                            },
                            &mut Vec::new(),
                        )
                    },
                );
            },
        ))
    }
    #[test]
    fn asset_uuid_copy_follows_legacy_types_permissions_and_available_assets() {
        let full_perm = rules::COPY | rules::MODIFY | rules::TRANSFER;
        let mut item = aurora_net::inventory::InvItem {
            id: Uuid::from_u128(1),
            asset_id: Uuid::from_u128(2),
            owner_mask: full_perm,
            ..Default::default()
        };
        for kind in -1..=60 {
            item.asset_type = kind;
            let allowed = [0, 1, 3, 5, 7, 13, 20, 21, 56, 57, 58, 59].contains(&kind);
            assert_eq!(copyable_asset_uuid(&item), allowed.then_some(item.asset_id), "asset type {kind}");
        }
        item.asset_type = 0;
        for missing in [rules::COPY, rules::MODIFY, rules::TRANSFER] {
            item.owner_mask = full_perm & !missing;
            assert_eq!(copyable_asset_uuid(&item), None, "missing permission {missing:x}");
        }
        item.owner_mask = full_perm;
        item.asset_id = Uuid::nil();
        assert_eq!(copyable_asset_uuid(&item), None);
    }

    #[test]
    fn asset_uuid_menu_copies_the_asset_without_changing_the_inventory_clipboard() {
        for (kind, mask, multiple, expected) in [
            (0, u32::MAX, false, Some(true)),
            (1, u32::MAX, false, Some(true)),
            (7, u32::MAX, false, Some(true)),
            (20, u32::MAX, false, Some(true)),
            (56, u32::MAX, false, Some(true)),
            (57, u32::MAX, false, Some(true)),
            (2, u32::MAX, false, None),
            (4, u32::MAX, false, None),
            (6, u32::MAX, false, None),
            (10, u32::MAX, false, None),
            (12, u32::MAX, false, None),
            (49, u32::MAX, false, None),
            (-1, u32::MAX, false, None),
            (0, !rules::TRANSFER, false, Some(false)),
            (0, u32::MAX, true, Some(false)),
        ] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let target = Uuid::from_u128(8104);
            let item = inv.items.get_mut(&target).expect("demo texture");
            item.asset_type = kind;
            item.owner_mask = mask;
            let asset = item.asset_id;
            assert_ne!(asset, target);
            let clipboard = vec![Uuid::from_u128(8100)];
            let mut state = InventoryUi {
                clipboard: clipboard.clone(),
                cut: true,
                selection: if multiple {
                    HashSet::from([target, clipboard[0]])
                } else {
                    HashSet::from([target])
                },
                ..Default::default()
            };
            for _ in 0..4 {
                menu_frame(&ctx, &inv, target, &mut state, Vec::new());
            }
            let output = menu_frame(&ctx, &inv, target, &mut state, Vec::new());
            let pos = output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, aurora_i18n::tr!("ui-inventory-context-copy-object-uuid")));
            let Some(expected) = expected else {
                assert!(pos.is_none(), "asset type {kind} never exposes a UUID");
                continue;
            };
            let pos = pos.expect("asset UUID entry") + egui::vec2(4.0, 4.0);
            let mut copied = Vec::new();
            for pressed in [true, false] {
                let output = menu_frame(
                    &ctx,
                    &inv,
                    target,
                    &mut state,
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
                copied.extend(output.platform_output.commands.into_iter().filter_map(|cmd| {
                    if let egui::OutputCommand::CopyText(text) = cmd {
                        Some(text)
                    } else {
                        None
                    }
                }));
            }
            assert_eq!(
                copied,
                if expected { vec![asset.to_string()] } else { Vec::new() },
                "type {kind}, multiple {multiple}"
            );
            assert_eq!(state.clipboard, clipboard);
            assert!(state.cut);
        }
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        inv.items.get_mut(&Uuid::from_u128(8113)).expect("demo link").asset_id = Uuid::from_u128(8104);
        for target in [8000, 8113] {
            let target = Uuid::from_u128(target);
            let mut state = InventoryUi::default();
            for _ in 0..4 {
                menu_frame(&ctx, &inv, target, &mut state, Vec::new());
            }
            let output = menu_frame(&ctx, &inv, target, &mut state, Vec::new());
            assert!(
                !output
                    .shapes
                    .iter()
                    .any(|s| { text_position(&s.shape, aurora_i18n::tr!("ui-inventory-context-copy-object-uuid")).is_some() }),
                "folders and links have no asset UUID entry"
            );
        }
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
            aurora_i18n::tr!("common-copy-slurl"),
            aurora_i18n::tr!("common-show-on-map"),
            aurora_i18n::tr!("common-properties"),
            aurora_i18n::tr!("common-rename"),
        ] {
            assert!(
                output.shapes.iter().any(|s| text_position(&s.shape, label).is_some()),
                "missing {label}"
            );
        }
        let positions: Vec<_> = [
            aurora_i18n::tr!("ui-inventory-context-about-landmark"),
            aurora_i18n::tr!("common-copy-slurl"),
            aurora_i18n::tr!("common-show-on-map"),
            aurora_i18n::tr!("common-properties"),
        ]
        .into_iter()
        .map(|label| {
            output
                .shapes
                .iter()
                .find_map(|s| text_position(&s.shape, label))
                .expect("menu row")
                .y
        })
        .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "destination actions stay together above properties"
        );
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
    fn folder_menu_keeps_navigation_and_default_expanded_organize_visible_without_edit_controls() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        for _ in 0..4 {
            frame(&ctx, &inv, Uuid::from_u128(8000));
        }
        let output = frame(&ctx, &inv, Uuid::from_u128(8000));
        for label in [
            aurora_i18n::tr!("common-rename"),
            aurora_i18n::tr!("ui-inventory-context-reload-folder"),
            aurora_i18n::tr!("inventory-menu-organize"),
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
        for hidden in [
            aurora_i18n::tr!("inventory-menu-create-group"),
            aurora_i18n::tr!("inventory-menu-reset"),
            aurora_i18n::tr!("inventory-menu-hidden"),
            aurora_i18n::tr!("common-new-script"),
        ] {
            assert!(
                !output.shapes.iter().any(|s| text_position(&s.shape, hidden).is_some()),
                "unexpected expanded option: {hidden}"
            );
        }
    }
    #[test]
    fn animation_menu_exposes_both_play_modes_without_attachment_controls() {
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
        ] {
            assert!(
                output.shapes.iter().any(|s| text_position(&s.shape, label).is_some()),
                "missing {label}"
            );
        }
        for hidden in [
            aurora_i18n::tr!("common-attach-to"),
            aurora_i18n::tr!("common-attach-to-hud"),
            aurora_i18n::tr!("inventory-menu-attachments"),
        ] {
            assert!(!output.shapes.iter().any(|s| text_position(&s.shape, hidden).is_some()));
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
                worn_at: Default::default(),
                points: Vec::new(),
                appearance_busy: false,
                touchable: if touchable { HashSet::from([item]) } else { HashSet::new() },
                names: Default::default(),
                ..Default::default()
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
                            egui::Id::unique("worn_menu"),
                            egui::pos2(500.0, 80.0),
                            &mut open,
                            &p,
                            |ui| {
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
                            },
                        );
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
                worn_at: Default::default(),
                points: Vec::new(),
                appearance_busy: false,
                active_gestures: Default::default(),
                touchable: Default::default(),
                names: Default::default(),
                ..Default::default()
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
                            egui::Id::unique("batch_menu"),
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
    #[test]
    fn favorites_menu_actions_include_every_selected_object() {
        for remove in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let p = crate::theme::Theme::default().palette();
            let mut inv = Inventory::default();
            crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
            let ids: Vec<_> = [8100, 8109, 8110].into_iter().map(Uuid::from_u128).collect();
            for (index, id) in ids.iter().enumerate() {
                inv.items.get_mut(id).expect("object").favorite = remove || index == 0;
            }
            let mut state = InventoryUi {
                selection: ids.iter().copied().collect(),
                ..Default::default()
            };
            let facts = Facts {
                agent: Uuid::nil(),
                worn: HashSet::new(),
                worn_labels: Default::default(),
                worn_at: Default::default(),
                points: Vec::new(),
                appearance_busy: false,
                active_gestures: Default::default(),
                touchable: Default::default(),
                names: Default::default(),
                ..Default::default()
            };
            // These commands were dragged out of the icon submenu into text.
            let mut preferences = InventoryPreferences::default();
            let profile = preferences.context_menus.profiles.entry("shared".into()).or_default();
            for key in ["favorite", "favorite-add", "favorite-remove"] {
                profile.layout.placements.insert(key.into(), "text".into());
            }
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
                            egui::Id::unique("favorite_batch_menu"),
                            egui::pos2(500.0, 80.0),
                            &mut open,
                            &p,
                            |ui| {
                                show(ui, &p, &inv, ids[0], &mut state, &mut preferences, &facts, &mut actions);
                            },
                        );
                    },
                ))
            };
            for _ in 0..4 {
                let _ = frame(Vec::new());
            }
            let output = frame(Vec::new());
            let label = if remove {
                aurora_i18n::tr!("common-remove-from-favorites")
            } else {
                aurora_i18n::tr!("common-add-to-favorites")
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
            let [InvAction::Edit(change)] = &actions[..] else {
                panic!("favorite action");
            };
            assert_eq!(change.operations.len(), if remove { 3 } else { 2 });
            for operation in &change.operations {
                let aurora_net::inventory::operations::Operation::Patch { id, folder: false, body } = operation else {
                    panic!("favorite item patch");
                };
                assert!(ids.contains(id));
                assert_eq!(body["favorite"]["toggled"].as_bool(), !remove);
            }
        }
    }
}
