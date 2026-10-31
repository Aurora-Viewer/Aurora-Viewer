//! Synthetic inventory for context-menu validation; never reads grid data.

use super::*;

pub const LOST_FOLDER: Uuid = Uuid::from_u128(50_016);

/// Explicit offline startup-recovery fixture. No account inventory is read.
pub fn seed_lost(inv: &mut Inventory, agent: Uuid) {
    inv.owner = agent;
    inv.hierarchy_repaired = false;
    for (id, parent, kind, name) in [
        (LOST_FOLDER, inv.root, 16, aurora_i18n::tr!("common-lost-and-found").to_owned()),
        (
            Uuid::from_u128(50_014),
            Uuid::from_u128(50_099),
            -1,
            format!("{} 1", aurora_i18n::tr!("world-inventory-demo-subfolder")),
        ),
        (
            Uuid::from_u128(50_015),
            Uuid::from_u128(50_014),
            -1,
            format!("{} 2", aurora_i18n::tr!("world-inventory-demo-subfolder")),
        ),
    ] {
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name,
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
    inv.generation += 1;
}

pub fn lost_reply(agent: Uuid) -> aurora_net::inventory::orphans::Contents {
    aurora_net::inventory::orphans::Contents {
        folders: Vec::new(),
        items: [
            (50_020, Uuid::nil()),
            (50_021, Uuid::from_u128(0x10)),
            (50_022, Uuid::from_u128(50_099)),
        ]
        .into_iter()
        .map(|(n, parent)| InvItem {
            id: Uuid::from_u128(n),
            parent,
            owner: agent,
            creator: agent,
            name: format!("{} {n}", aurora_i18n::tr!("common-demo-object")),
            asset_type: 6,
            inv_type: 6,
            owner_mask: 0x7fffffff,
            ..Default::default()
        })
        .collect(),
    }
}

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
            desc: if asset_type == 7 {
                aurora_i18n::tr!("notecard-demo-description").into()
            } else {
                String::new()
            },
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
    if std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| mode.contains("hover-photo"))
        && let Some(object) = inv.items.get_mut(&original)
    {
        object.thumbnail = crate::demo::TEX_SNAPSHOT;
    }
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
    if std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| matches!(mode.as_str(), "link-icons" | "aurora-link-icons")) {
        for (kind, &name) in super::super::appearance::wearables().iter().enumerate() {
            inv.add_items(vec![InvItem {
                id: Uuid::from_u128(82_000 + kind as u128),
                parent: personal,
                name: name.into(),
                asset_type: if kind < 4 { 13 } else { 5 },
                inv_type: 18,
                flags: kind as u32,
                owner: agent,
                ..Default::default()
            }]);
        }
        let mut targets: Vec<_> = inv
            .items
            .values()
            .filter(|it| it.parent == personal && !matches!(it.asset_type, 24 | 25))
            .cloned()
            .collect();
        targets.sort_by_key(|it| it.id);
        let mut links: Vec<_> = targets
            .into_iter()
            .enumerate()
            .map(|(n, it)| InvItem {
                id: Uuid::from_u128(83_000 + n as u128),
                parent: personal,
                name: aurora_i18n::tr!("inventory-kind-link-to", kind = &it.name),
                asset_type: 24,
                asset_id: it.id,
                inv_type: it.inv_type,
                owner: agent,
                ..Default::default()
            })
            .collect();
        links.push(InvItem {
            id: Uuid::from_u128(84_000),
            parent: personal,
            name: aurora_i18n::tr!("inventory-kind-folder-link").into(),
            asset_type: 25,
            asset_id: Uuid::from_u128(8001),
            owner: agent,
            ..Default::default()
        });
        links.push(InvItem {
            id: Uuid::from_u128(84_001),
            parent: personal,
            name: aurora_i18n::tr!("inventory-kind-broken-link").into(),
            asset_type: 24,
            asset_id: Uuid::from_u128(84_999),
            owner: agent,
            ..Default::default()
        });
        inv.add_items(links);
    }
    inv.queue.clear();
    inv.sort_all();
}

/// Enough top-level folders to scroll Aurora's expanded inventory sidebar.
pub fn seed_sidebar(inv: &mut Inventory) {
    inv.created_folders(
        (0..80)
            .map(|n| InvFolder {
                id: Uuid::from_u128(30_000 + n),
                parent: inv.root,
                name: format!("Folder {n:03}"),
                type_default: -1,
                ..Default::default()
            })
            .collect(),
    );
    for n in 0..80 {
        if let Some(folder) = inv.folders.get_mut(&Uuid::from_u128(30_000 + n)) {
            folder.state = FetchState::Fetched;
        }
    }
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

/// Three purchases whose alphabetical and newest-content orders are opposite.
pub fn seed_received_sort(inv: &mut Inventory, agent: Uuid) {
    seed_received(inv, agent);
    let inbox = Uuid::from_u128(8_300_000);
    let oldest = Uuid::from_u128(8_300_001);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    if let Some(folder) = inv.folders.get_mut(&oldest) {
        folder.info.name = aurora_i18n::tr!("inventory-sort-demo-old").into();
    }
    for item in inv.items.values_mut().filter(|item| item.parent == oldest) {
        item.created_at = time - 3 * 3600;
    }
    for (number, name, hours) in [
        (8_300_010, aurora_i18n::tr!("inventory-sort-demo-middle"), 2),
        (8_300_020, aurora_i18n::tr!("inventory-sort-demo-new"), 1),
    ] {
        let id = Uuid::from_u128(number);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent: inbox,
                    name: name.into(),
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
        if let Some(folder) = inv.folders.get_mut(&inbox) {
            folder.children.push(id);
        }
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(number + 1),
            parent: id,
            name: aurora_i18n::tr!("common-demo-object").into(),
            asset_type: 6,
            inv_type: 6,
            owner: agent,
            creator: agent,
            created_at: time - hours * 3600,
            base_mask: actions::COPY | actions::MODIFY | actions::TRANSFER,
            owner_mask: actions::COPY | actions::MODIFY | actions::TRANSFER,
            ..Default::default()
        }]);
    }
}

/// `AURORA_DEMO_INVENTORY=received-big`: a long-time buyer's Received Items,
/// `folders` purchases of `items` items each, to measure the tab's cost.
pub fn seed_received_big(inv: &mut Inventory, agent: Uuid, folders: u128, items: u128) {
    seed_received(inv, agent);
    let inbox = Uuid::from_u128(8_300_000);
    for f in 0..folders {
        let id = Uuid::from_u128(8_400_000 + f);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent: inbox,
                    name: format!("Marketplace purchase {f:03}"),
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
        if let Some(folder) = inv.folders.get_mut(&inbox) {
            folder.children.push(id);
        }
        inv.add_items(
            (0..items)
                .map(|i| {
                    let item = 9_000_000 + f * 1_000 + i;
                    InvItem {
                        id: Uuid::from_u128(item),
                        parent: id,
                        asset_id: Uuid::from_u128(item + 500_000),
                        name: format!("Purchase {f:03} item {i:02}"),
                        asset_type: 6,
                        inv_type: 6,
                        owner: agent,
                        creator: agent,
                        base_mask: super::actions::COPY | super::actions::MODIFY | super::actions::TRANSFER,
                        owner_mask: super::actions::COPY | super::actions::MODIFY | super::actions::TRANSFER,
                        ..Default::default()
                    }
                })
                .collect(),
        );
    }
}

pub fn target(view: &str, inv: &Inventory) -> Option<Uuid> {
    if let Some(mode) = view.strip_prefix("aurora-menu-") {
        return target(
            match mode {
                "folder" | "all-folder" | "folder-uuid" | "folder-uuid-restored" => "folder",
                "texture" | "all-texture" | "textures-export" => return Some(Uuid::from_u128(8104)),
                "trash" | "all-trash" => {
                    return inv.folders.values().find(|f| f.info.type_default == 14).map(|f| f.info.id);
                }
                "animation" => "animation",
                "script" => "script",
                "link" => "link",
                "object" | "organize" | "custom" | "locked" | "hidden" | "bars" | "drop" | "filter" | "export" | "load" | "empty"
                | "load-search" | "empty-custom" | "restored" => "object",
                _ => return None,
            },
            inv,
        );
    }
    Some(Uuid::from_u128(match view {
        "outfit-menu" | "outfit-menu-loading" => 704,
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
        | "folder-window-search"
        | "image-layout-folder"
        | "image-layout-folder-wide" => 8000,
        "move-item" | "move-multi" | "move-filter" | "move-window" | "move-menu" => 8100,
        "uuid" | "uuid-restricted" | "uuid-missing" | "uuid-multi" => 8104,
        "landmark" | "aurora-landmark" => 8120,
        "object"
        | "properties"
        | "properties-no-mod"
        | "properties-no-creator"
        | "rename"
        | "multi-add"
        | "multi-detach"
        | "delete"
        | "delete-key"
        | "delete-button"
        | "clipboard"
        | "clipboard-copy"
        | "clipboard-cut"
        | "clipboard-folder"
        | "clipboard-cut-folder"
        | "clipboard-progress"
        | "clipboard-window"
        | "keyboard"
        | "keyboard-fold"
        | "aurora-keyboard"
        | "aurora-keyboard-fold"
        | "aurora-keyboard-wide" => 8100,
        "animation"
        | "animation-open"
        | "animation-play-local"
        | "animation-play-world"
        | "animation-properties"
        | "image"
        | "image-photo"
        | "image-picker"
        | "image-photo-save"
        | "image-layout"
        | "image-layout-wide" => 8101,
        "script" => 8102,
        "note" | "note-open" | "note-edit" | "note-save" | "note-close" | "note-error" | "note-readonly" | "note-embedded" | "note-new"
        | "note-task" | "note-task-save" | "note-task-error" | "note-task-readonly" | "note-drop" | "note-image" | "note-landmark"
        | "note-copy" | "note-task-embedded" | "note-menu-right" | "note-markdown" | "note-source" | "note-classic" | "note-format"
        | "note-remember" | "note-lsl" | "note-lsl-markdown" => 8103,
        "nocopy" => 8109,
        "notransfer" => 8110,
        "link" => 8113,
        "link-icons" | "aurora-link-icons" => 83_000,
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
    let markdown = std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| {
        matches!(
            mode.as_str(),
            "note-markdown" | "note-source" | "note-classic" | "note-format" | "note-remember" | "note-lsl" | "note-lsl-markdown"
        )
    });
    let lsl = std::env::var("AURORA_DEMO_INVENTORY").is_ok_and(|mode| matches!(mode.as_str(), "note-lsl" | "note-lsl-markdown"));
    let text = if lsl {
        aurora_i18n::tr!("notecard-lsl-demo")
    } else if markdown {
        aurora_i18n::tr!("notecard-markdown-demo")
    } else {
        aurora_i18n::tr!("notecard-demo-text")
    };
    let embedded = embedded || markdown;
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

/// `AURORA_DEMO_INVENTORY=big[:<items>][,recent][,search=<text>][,toggle=<s>][,churn=<ms>][,replace]`:
/// a big inventory (`seed_big`) to measure what the Inventory window costs.
/// `recent` opens the Recent tab, `search` filters the list, `toggle` opens
/// and closes the window every `s` seconds (closed and open alternate in one
/// run), `churn` changes one item every `ms` milliseconds, as the server's
/// inventory updates do, `replace` opens Replace links on the first object
/// (`BIG_LINKED`), which a link of every fifty items points to.
#[derive(Debug, Clone, PartialEq)]
pub struct Big {
    pub items: usize,
    pub recent: bool,
    pub search: String,
    pub toggle: f32,
    pub churn_ms: u64,
    pub replace: bool,
}

/// The first object of `seed_big`: the links of every block of fifty items
/// point to the object that starts the block, and the first one is this.
pub const BIG_LINKED: Uuid = Uuid::from_u128(21_000_000);

pub fn parse_big(mode: &str) -> Option<Big> {
    let mut parts = mode.split(',');
    let head = parts.next()?;
    let items = match head.strip_prefix("big") {
        Some("") => 30_000,
        Some(count) => count.strip_prefix(':')?.parse().ok()?,
        None => return None,
    };
    let mut big = Big {
        items,
        recent: false,
        search: String::new(),
        toggle: 0.0,
        churn_ms: 0,
        replace: false,
    };
    for part in parts {
        match part.split_once('=') {
            None if part == "recent" => big.recent = true,
            None if part == "replace" => big.replace = true,
            Some(("search", text)) => big.search = text.into(),
            Some(("toggle", s)) => big.toggle = s.parse().unwrap_or(0.0),
            Some(("churn", ms)) => big.churn_ms = ms.parse().unwrap_or(0),
            _ => log::warn!("AURORA_DEMO_INVENTORY: unknown option {part}"),
        }
    }
    Some(big)
}

pub fn big() -> Option<&'static Big> {
    static BIG: std::sync::OnceLock<Option<Big>> = std::sync::OnceLock::new();
    BIG.get_or_init(|| std::env::var("AURORA_DEMO_INVENTORY").ok().and_then(|mode| parse_big(&mode)))
        .as_ref()
}

/// `AURORA_DEMO_INVENTORY=big[:<items>]`: the inventory of a long-time
/// resident, `items` items (30,000 by default) in about a tenth as many
/// folders, up to five levels deep: store brands, their products, extras and
/// fit packs, and one flat folder of 2,000 loose objects. Mixed types,
/// permissions, links and dates (a few hundred since the last day, for
/// Recent). Returns the folders a resident would keep open: the loose
/// objects, the first top folder, three of its brands and their products.
pub fn seed_big(inv: &mut Inventory, agent: Uuid, items: usize) -> Vec<Uuid> {
    const TOPS: [&str; 10] = [
        "Clothing",
        "Store purchases",
        "Furniture",
        "Hair",
        "Shoes",
        "Gadgets",
        "Building kits",
        "Textures archive",
        "Landmarks archive",
        "Old stuff",
    ];
    const BRAND: [&str; 12] = [
        "Velvet", "North", "Silver", "Urban", "Lumen", "Cedar", "Atlas", "Nova", "Echo", "Moss", "Indigo", "Copper",
    ];
    const HOUSE: [&str; 8] = ["Atelier", "Works", "Design", "Studio", "Lab", "Forge", "House", "Craft"];
    const PRODUCT: [&str; 10] = [
        "Jacket",
        "Dress",
        "Sofa",
        "Lamp",
        "Boots",
        "Bob",
        "Pose pack",
        "Gazebo",
        "Wood set",
        "HUD",
    ];
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let mut next_folder = 20_000_000_u128;
    let mut add_folder = |inv: &mut Inventory, parent: Uuid, name: String| {
        let id = Uuid::from_u128(next_folder);
        next_folder += 1;
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
        id
    };
    let root = inv.root;
    let loose = add_folder(inv, root, "Objects (unsorted)".into());
    let tops: Vec<Uuid> = TOPS.iter().map(|name| add_folder(inv, root, (*name).into())).collect();
    // About 11.7 folders per brand: the brand, 8 products, 2 extras and
    // two thirds of a fit pack.
    let brands_per_top = (items / 10 / 117).max(1);
    let mut leaves = Vec::new();
    let mut open = vec![loose, tops[0]];
    for (t, top) in tops.iter().enumerate() {
        for b in 0..brands_per_top {
            let n = t * brands_per_top + b;
            let brand_name = format!("{} {} {n:03}", BRAND[n % BRAND.len()], HOUSE[(n / BRAND.len()) % HOUSE.len()]);
            let brand = add_folder(inv, *top, brand_name.clone());
            if t == 0 && b < 3 {
                open.push(brand);
            }
            for p in 0..8 {
                let product = add_folder(inv, brand, format!("{brand_name} - {} {p:02}", PRODUCT[(n + p) % PRODUCT.len()]));
                if t == 0 && b < 3 {
                    open.push(product);
                }
                leaves.push(product);
                if p % 4 == 0 {
                    let extras = add_folder(inv, product, "Extras".into());
                    leaves.push(extras);
                    if (n + p).is_multiple_of(3) {
                        leaves.push(add_folder(inv, extras, "Fit packs".into()));
                    }
                }
            }
        }
    }
    let mut list = Vec::with_capacity(items);
    for n in 0..items {
        let parent = if n < 2_000 { loose } else { leaves[n % leaves.len()] };
        // Objects, clothing, body parts, textures, notecards, landmarks,
        // scripts, animations, sounds, links.
        let (asset_type, inv_type) = match n % 50 {
            0..20 => (6, 6),
            20..30 => (5, 18),
            30..32 => (13, 18),
            32..37 => (0, 0),
            37..41 => (7, 7),
            41..44 => (3, 3),
            44..46 => (10, 10),
            46..48 => (20, 19),
            48 => (1, 1),
            _ => (24, 6),
        };
        let id = Uuid::from_u128(21_000_000 + n as u128);
        let mut owner_mask = actions::COPY | actions::MODIFY | actions::TRANSFER;
        if n % 7 == 0 {
            owner_mask &= !actions::COPY;
        }
        if n % 11 == 0 {
            owner_mask &= !actions::MODIFY;
        }
        if n % 13 == 0 {
            owner_mask &= !actions::TRANSFER;
        }
        list.push(InvItem {
            id,
            parent,
            // a link points at an earlier object
            asset_id: if asset_type == 24 {
                Uuid::from_u128(21_000_000 + (n - n % 50) as u128)
            } else {
                Uuid::from_u128(22_000_000 + n as u128)
            },
            name: format!(
                "{} {} {} {:04}{}",
                BRAND[n % BRAND.len()],
                HOUSE[n % HOUSE.len()],
                PRODUCT[(n / 3) % PRODUCT.len()],
                n,
                if n % 5 == 0 { " (wear me)" } else { "" }
            ),
            desc: if n % 4 == 0 {
                format!("Version {}.{}", n % 9, n % 7)
            } else {
                String::new()
            },
            asset_type,
            inv_type,
            flags: if asset_type == 6 && n % 9 == 0 {
                crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS
            } else {
                0
            },
            owner: agent,
            creator: if n % 10 == 0 {
                agent
            } else {
                Uuid::from_u128(30_000_000 + (n % 400) as u128)
            },
            created_at: if n % 100 == 0 {
                now - (n as i64 * 37) % 86_000
            } else {
                now - 86_400 - (n as i64 * 3_571) % (5 * 365 * 86_400)
            },
            base_mask: actions::COPY | actions::MODIFY | actions::TRANSFER,
            owner_mask,
            next_owner_mask: actions::COPY | actions::MODIFY | actions::TRANSFER,
            ..Default::default()
        });
    }
    inv.add_items(list);
    inv.sort_all();
    inv.generation += 1;
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_mode_reads_its_size_and_options() {
        assert_eq!(parse_big("1"), None);
        assert_eq!(parse_big("big").map(|b| b.items), Some(30_000));
        assert_eq!(parse_big("big:5000").map(|b| b.items), Some(5_000));
        assert_eq!(parse_big("bigger"), None);
        let big = parse_big("big:800,recent,search=jacket,toggle=4,churn=250,replace").expect("big mode");
        assert_eq!(
            big,
            Big {
                items: 800,
                recent: true,
                search: "jacket".into(),
                toggle: 4.0,
                churn_ms: 250,
                replace: true,
            }
        );
    }

    #[test]
    fn big_inventory_has_a_tenth_as_many_folders_up_to_five_levels() {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            Folder {
                info: InvFolder {
                    id: inv.root,
                    name: "My Inventory".into(),
                    type_default: 8,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        let open = seed_big(&mut inv, Uuid::from_u128(2), 12_000);
        assert_eq!(inv.items.len(), 12_000);
        assert!((1_000..1_400).contains(&inv.folders.len()), "{} folders", inv.folders.len());
        let depth = |mut id: Uuid| {
            let mut depth = 0;
            while let Some(folder) = inv.folders.get(&id) {
                id = folder.info.parent;
                depth += 1;
            }
            depth
        };
        assert_eq!(inv.folders.keys().map(|id| depth(*id)).max(), Some(6));
        let listed: usize = inv.folders.values().map(|f| f.items.len()).sum();
        assert_eq!(listed, 12_000);
        assert!(open.iter().all(|id| inv.folders.contains_key(id)));
    }
}
