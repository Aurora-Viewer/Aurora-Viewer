//! Synthetic inventory for context-menu validation; never reads grid data.

use super::*;

pub fn seed(inv: &mut Inventory, agent: Uuid) {
    super::super::appearance::seed_demo(inv, agent);
    if inv.lib_root.is_nil() {
        inv.lib_root = Uuid::from_u128(8003);
        inv.lib_owner = Uuid::from_u128(8004);
        inv.folders.insert(
            inv.lib_root,
            Folder {
                info: InvFolder {
                    id: inv.lib_root,
                    name: "Library".into(),
                    type_default: 8,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: true,
            },
        );
    }
    let personal = Uuid::from_u128(8000);
    for (number, parent, name, kind) in [
        (8000, inv.root, aurora_i18n::tr!("world-inventory-demo-right-click-tests"), -1),
        (8001, personal, aurora_i18n::tr!("world-inventory-demo-subfolder"), -1),
        (8002, inv.root, aurora_i18n::tr!("common-marketplace-listings"), 53),
    ] {
        let id = Uuid::from_u128(number);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: kind,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    for folder in inv.folders.values_mut() {
        folder.state = FetchState::Fetched;
        folder.children.clear();
    }
    let parents: Vec<_> = inv.folders.values().map(|f| (f.info.id, f.info.parent)).collect();
    for (id, parent) in parents {
        if let Some(f) = inv.folders.get_mut(&parent) {
            f.children.push(id);
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    inv.add_items(
        [
            (8100, aurora_i18n::tr!("common-demo-object"), 6, 6, 0),
            (8101, aurora_i18n::tr!("world-inventory-demo-demo-animation"), 20, 19, 0),
            (8102, aurora_i18n::tr!("common-demo-script"), 10, 10, 0),
            (8103, aurora_i18n::tr!("world-inventory-demo-instructions"), 7, 7, 0),
            (8104, aurora_i18n::tr!("world-inventory-demo-demo-texture"), 0, 0, 0),
            (8105, aurora_i18n::tr!("world-inventory-demo-demo-sound"), 1, 1, 0),
            (8106, aurora_i18n::tr!("world-inventory-demo-demo-gesture"), 21, 20, 0),
            (8107, aurora_i18n::tr!("world-inventory-demo-demo-sky"), 56, 25, 0),
            (8108, aurora_i18n::tr!("world-inventory-demo-demo-material"), 57, 26, 0),
            (8109, aurora_i18n::tr!("world-inventory-demo-no-copy-object"), 6, 6, 0),
            (8110, aurora_i18n::tr!("world-inventory-demo-non-transferable-object"), 6, 6, 0),
            (8111, aurora_i18n::tr!("world-inventory-demo-demo-shirt"), 5, 18, 4),
            (8112, aurora_i18n::tr!("world-inventory-demo-demo-silhouette"), 13, 18, 0),
        ]
        .into_iter()
        .map(|(id, name, asset_type, inv_type, flags)| InvItem {
            id: Uuid::from_u128(id),
            parent: personal,
            asset_id: if asset_type == 20 {
                crate::demo::IDLE_ANIM
            } else if asset_type == 0 {
                crate::demo::TEX_SNAPSHOT
            } else {
                Uuid::from_u128(9000 + id)
            },
            name: name.into(),
            asset_type,
            inv_type,
            flags,
            creator: agent,
            owner: agent,
            created_at: now,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff
                & match id {
                    8109 => !actions::COPY,
                    8110 => !actions::TRANSFER,
                    _ => u32::MAX,
                },
            next_owner_mask: actions::COPY | actions::MODIFY | actions::TRANSFER,
            ..Default::default()
        })
        .collect(),
    );
    let original = Uuid::from_u128(8100);
    inv.add_items(vec![InvItem {
        id: Uuid::from_u128(8113),
        parent: personal,
        name: aurora_i18n::tr!("world-inventory-demo-link-to-object").into(),
        asset_type: 24,
        inv_type: 6,
        asset_id: original,
        owner: agent,
        base_mask: 0x7fffffff,
        owner_mask: 0x7fffffff,
        ..Default::default()
    }]);
    inv.queue.clear();
    inv.sort_all();
}

/// A Marketplace inbox separate from the normal inventory, for tab validation.
pub fn seed_received(inv: &mut Inventory, agent: Uuid) {
    let inbox = Uuid::from_u128(8_300_000);
    let purchase = Uuid::from_u128(8_300_001);
    for (id, parent, name, kind) in [
        (inbox, inv.root, aurora_i18n::tr!("ui-inventory-received-purchases"), 50),
        (purchase, inbox, aurora_i18n::tr!("common-demo-object"), -1),
    ] {
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: kind,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(folder) = inv.folders.get_mut(&parent) {
            folder.children.push(id);
        }
    }
    inv.add_items(
        [
            (8_300_002, 6, 6, aurora_i18n::tr!("common-demo-object")),
            (8_300_003, 7, 7, aurora_i18n::tr!("world-inventory-demo-instructions")),
        ]
        .into_iter()
        .map(|(id, asset_type, inv_type, name)| InvItem {
            id: Uuid::from_u128(id),
            parent: purchase,
            asset_id: Uuid::from_u128(id + 100),
            name: name.into(),
            asset_type,
            inv_type,
            owner: agent,
            creator: agent,
            base_mask: super::actions::COPY | super::actions::MODIFY | super::actions::TRANSFER,
            owner_mask: super::actions::COPY | super::actions::MODIFY | super::actions::TRANSFER,
            ..Default::default()
        })
        .collect(),
    );
}

pub fn target(view: &str) -> Option<Uuid> {
    Some(Uuid::from_u128(match view {
        "folder"
        | "clothes"
        | "body"
        | "settings"
        | "uploads"
        | "new-folder"
        | "new-script"
        | "new-note"
        | "folder-window"
        | "move-folder"
        | "folder-window-search" => 8000,
        "move-item" | "move-multi" | "move-filter" | "move-window" | "move-menu" => 8100,
        "object"
        | "properties"
        | "rename"
        | "multi-add"
        | "multi-detach"
        | "delete"
        | "clipboard"
        | "clipboard-copy"
        | "clipboard-cut"
        | "clipboard-folder"
        | "clipboard-cut-folder"
        | "clipboard-window" => 8100,
        "animation" | "animation-open" | "animation-properties" | "image" | "image-photo" | "image-picker" | "image-photo-save" => 8101,
        "script" => 8102,
        "note" | "note-open" | "note-edit" | "note-save" | "note-close" | "note-error" | "note-readonly" | "note-embedded" | "note-new"
        | "note-task" | "note-task-save" | "note-task-error" | "note-task-readonly" | "note-drop" | "note-image" | "note-landmark"
        | "note-copy" | "note-task-embedded" | "note-menu-right" => 8103,
        "nocopy" => 8109,
        "notransfer" => 8110,
        "link" => 8113,
        _ => return None,
    }))
}

pub fn seed_clipboard_tree(inv: &mut Inventory) {
    for (number, parent, name) in [
        (
            8221,
            Uuid::from_u128(8001),
            aurora_i18n::tr!("inventory-demo-nested-folder", level = 2),
        ),
        (
            8222,
            Uuid::from_u128(8221),
            aurora_i18n::tr!("inventory-demo-nested-folder", level = 3),
        ),
        (8300, inv.root, aurora_i18n::tr!("inventory-demo-destination").into()),
    ] {
        let id = Uuid::from_u128(number);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name,
                    type_default: -1,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(folder) = inv.folders.get_mut(&parent) {
            folder.children.push(id);
        }
    }
    let items = [(8220, 8001, 8104), (8223, 8221, 8100), (8224, 8222, 8102)]
        .into_iter()
        .filter_map(|(number, parent, original)| {
            let mut item = inv.items.get(&Uuid::from_u128(original))?.clone();
            item.id = Uuid::from_u128(number);
            item.parent = Uuid::from_u128(parent);
            Some(item)
        })
        .collect();
    inv.add_items(items);
}

pub fn note_asset(embedded: bool) -> Vec<u8> {
    let text = aurora_i18n::tr!("notecard-demo-text");
    if embedded {
        let mut card = aurora_net::inventory::notecard::Notecard::default();
        let mut text = format!("{text}\n");
        for (id, name, asset_type, inv_type, asset) in [
            (
                8104,
                aurora_i18n::tr!("world-inventory-demo-demo-texture"),
                0,
                0,
                crate::demo::TEX_SNAPSHOT,
            ),
            (
                8120,
                aurora_i18n::tr!("common-landmark"),
                3,
                3,
                crate::demo::place::landmark_ids(1).1,
            ),
            (8100, aurora_i18n::tr!("common-demo-object"), 6, 6, Uuid::from_u128(18100)),
        ] {
            let position = text.chars().count();
            if card
                .insert_attachment(
                    &mut text,
                    position,
                    &InvItem {
                        id: Uuid::from_u128(id),
                        name: name.into(),
                        asset_type,
                        inv_type,
                        asset_id: asset,
                        base_mask: 0x7fffffff,
                        owner_mask: 0x7fffffff,
                        next_owner_mask: 0xe000,
                        ..Default::default()
                    },
                )
                .is_err()
            {
                return Vec::new();
            }
            text.push('\n');
        }
        card.encode(&text).unwrap_or_default()
    } else {
        aurora_net::inventory::notecard::Notecard::default()
            .encode(text)
            .unwrap_or_default()
    }
}

/// Sorting/filter fixtures with distinct ages, owners, permissions and paths.
pub fn seed_view(inv: &mut Inventory, agent: Uuid, large: bool) {
    let root = inv.root;
    let personal = Uuid::from_u128(8200);
    for (n, parent, name, kind) in [
        (8200, root, aurora_i18n::tr!("world-inventory-demo-a-personal-folder"), -1),
        (8201, root, aurora_i18n::tr!("common-textures"), 0),
        (8202, personal, aurora_i18n::tr!("world-inventory-demo-attachments"), -1),
        (8203, personal, aurora_i18n::tr!("common-clothing"), -1),
        (8204, personal, aurora_i18n::tr!("common-body"), -1),
    ] {
        if kind >= 0 && actions::system(inv, kind).is_some() {
            continue;
        }
        let id = Uuid::from_u128(n);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: kind,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(folder) = inv.folders.get_mut(&parent) {
            folder.children.push(id);
        }
    }
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let parent = Uuid::from_u128(8000);
    let mut items: Vec<_> = inv
        .items
        .values()
        .filter(|it| it.parent == parent && !matches!(it.asset_type, 24 | 25))
        .cloned()
        .collect();
    for it in &mut items {
        it.created_at = time - (it.id.as_u128() as i64 - 8100) * 3600;
        it.desc = aurora_i18n::tr!("world-inventory-demo-demo-item-for-inventory-filters").into();
        if it.id == Uuid::from_u128(8109) {
            it.creator = Uuid::from_u128(100);
            it.flags |= 0x200000;
        }
    }
    items.extend(
        inv.items
            .values()
            .filter(|it| it.parent == root && (710..724).contains(&it.id.as_u128()))
            .map(|it| {
                let mut it = it.clone();
                it.parent = Uuid::from_u128(match it.asset_type {
                    6 => 8202,
                    5 => 8203,
                    _ => 8204,
                });
                it
            }),
    );
    if large {
        items.extend((0..1500).map(|n| InvItem {
            id: Uuid::from_u128(10000 + n),
            parent,
            name: aurora_i18n::tr!("world-inventory-demo-item", n = format!("{}", n)),
            asset_type: 6,
            inv_type: 6,
            created_at: time - n as i64 * 60,
            owner: agent,
            creator: agent,
            owner_mask: actions::COPY | actions::MODIFY | actions::TRANSFER,
            ..Default::default()
        }));
    }
    inv.add_items(items);
    inv.sort_all();
}

/// Long labels in nested folders, for narrowing and horizontal scrolling.
pub fn seed_long_names(inv: &mut Inventory) {
    for (id, name) in [
        (
            8200,
            aurora_i18n::tr!("world-inventory-demo-demo-collection-jeans-substance-materials-textures-and-attachments"),
        ),
        (8202, aurora_i18n::tr!("status-jeans-substance-textures-and-accessories-in")),
    ] {
        if let Some(folder) = inv.folders.get_mut(&Uuid::from_u128(id)) {
            folder.info.name = name.into();
        }
    }
    for (id, name) in [
        (
            8100,
            aurora_i18n::tr!("world-inventory-demo-jeans-substance-cuir-jeans-jeans-substance-cuir-normal-demo"),
        ),
        (
            710,
            aurora_i18n::tr!("world-inventory-demo-jeans-substance-worn-item-with-a-very-long-name"),
        ),
    ] {
        if let Some(item) = inv.items.get_mut(&Uuid::from_u128(id)) {
            item.name = name.into();
        }
    }
    inv.generation += 1;
    inv.sort_all();
}
