//! Synthetic inventory for context-menu validation; never reads grid data.

use super::*;
use aurora_net::inventory::FolderContents;
use aurora_net::inventory::ais::{FetchError, FetchFailure};
use aurora_net::{NetCommand, NetEvent};
use std::collections::HashMap;

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

/// A Lost and Found folder alone, when the demo inventory has none (the
/// script recovery scenario puts the recovered scripts there).
pub fn seed_lost_folder(inv: &mut Inventory) {
    if inv.system_folder(16).is_some() {
        return;
    }
    inv.folders.insert(
        LOST_FOLDER,
        Folder {
            info: InvFolder {
                id: LOST_FOLDER,
                parent: inv.root,
                name: aurora_i18n::tr!("common-lost-and-found").to_owned(),
                type_default: 16,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    let root = inv.root;
    if let Some(folder) = inv.folders.get_mut(&root) {
        folder.children.push(LOST_FOLDER);
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
    // A calling card made by a viewer (create_inventory_callingcard): ours,
    // naming the wolf by key in its description. Opening it shows his profile,
    // named Orion Example by the demo's name service.
    inv.add_items(vec![InvItem {
        id: Uuid::from_u128(8114),
        parent: personal,
        name: aurora_i18n::tr!("demo-orion-example").into(),
        desc: crate::demo::DEMO_LOUP.to_string(),
        asset_type: 2,
        inv_type: 2,
        creator: agent,
        owner: agent,
        created_at: now,
        base_mask: actions::MOVE | actions::TRANSFER,
        owner_mask: actions::MOVE | actions::TRANSFER,
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
    // Clipboard feedback scenarios reuse the same source tree in either layout.
    let view = view
        .strip_prefix("aurora-")
        .filter(|mode| mode.starts_with("clipboard"))
        .unwrap_or(view);
    let view = if view.starts_with("clipboard-cut-folder") {
        "clipboard-cut-folder"
    } else if view.starts_with("clipboard-cut") {
        "clipboard-cut"
    } else {
        view
    };
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
        | "delete-enter"
        | "delete-tab"
        | "clipboard"
        | "clipboard-copy"
        | "clipboard-cut"
        | "clipboard-folder"
        | "delete-confirm"
        | "aurora-delete-confirm"
        | "aurora-delete-confirm-medium"
        | "aurora-delete-confirm-wide"
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

/// Hundreds of empty top folders, as some residents keep: still a long list
/// once every folder is collapsed.
pub fn seed_many_folders(inv: &mut Inventory, count: u128) {
    let root = inv.root;
    for n in 0..count {
        let id = Uuid::from_u128(0x00c0_11a9_5e00_0000 + n);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent: root,
                    name: aurora_i18n::tr!("world-inventory-demo-folder", n = format!("{n:03}")),
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
        if let Some(folder) = inv.folders.get_mut(&root) {
            folder.children.push(id);
        }
    }
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

/// `AURORA_DEMO_INVENTORY=big[:<items>][,recent][,search=<text>][,toggle=<s>][,churn=<ms>][,replace][,fetch]`:
/// a big inventory (`seed_big`) to measure what the Inventory window costs.
/// `recent` opens the Recent tab, `search` filters the list, `toggle` opens
/// and closes the window every `s` seconds (closed and open alternate in one
/// run), `churn` changes one item every `ms` milliseconds, as the server's
/// inventory updates do, `replace` opens Replace links on the first object
/// (`BIG_LINKED`), which a link of every fifty items points to. `fetch` loads
/// the inventory through a simulated AIS server instead of seeding it ready:
/// `AURORA_DEMO_INVENTORY_FAULT` then injects its failures.
#[derive(Debug, Clone, PartialEq)]
pub struct Big {
    pub items: usize,
    pub recent: bool,
    pub search: String,
    pub toggle: f32,
    pub churn_ms: u64,
    pub replace: bool,
    pub fetch: bool,
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
        fetch: false,
    };
    for part in parts {
        match part.split_once('=') {
            None if part == "recent" => big.recent = true,
            None if part == "replace" => big.replace = true,
            None if part == "fetch" => big.fetch = true,
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

/// `AURORA_DEMO_INVENTORY=big:...,fetch`: what the simulated AIS server
/// answers from, while the live inventory starts empty.
pub struct FetchSource {
    by_parent: HashMap<Uuid, Vec<InvItem>>,
    by_id: HashMap<Uuid, Uuid>,
}

impl FetchSource {
    /// Take the seeded inventory's items out of the live model, leaving its
    /// folder tree in place and every folder to be fetched.
    pub fn take_from(inv: &mut Inventory) -> Self {
        let by_parent: HashMap<Uuid, Vec<InvItem>> = inv
            .folders
            .values()
            .filter_map(|folder| {
                let items: Vec<InvItem> = folder.items.iter().filter_map(|id| inv.items.get(id).cloned()).collect();
                (!items.is_empty()).then_some((folder.info.id, items))
            })
            .collect();
        let by_id = inv.items.values().map(|item| (item.id, item.parent)).collect();
        inv.items.drain();
        for folder in inv.folders.values_mut() {
            folder.items.clear();
            folder.state = FetchState::Unknown;
        }
        inv.generation += 1;
        FetchSource { by_parent, by_id }
    }

    fn items(&self, folder: Uuid) -> Vec<InvItem> {
        self.by_parent.get(&folder).cloned().unwrap_or_default()
    }

    fn item(&self, id: Uuid) -> Option<InvItem> {
        let parent = *self.by_id.get(&id)?;
        self.by_parent.get(&parent)?.iter().find(|item| item.id == id).cloned()
    }
}

/// `AURORA_DEMO_INVENTORY_FAULT=transient[=<n>][,too-big][,limit=<n>]`: the
/// intended failures of the `fetch` demo. Ignored without a `fetch` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Faults {
    pub transient_left: u32,
    pub too_big: bool,
    pub limit: usize,
    pub over_limit: Option<Uuid>,
}

impl Default for Faults {
    fn default() -> Self {
        Faults {
            transient_left: 0,
            too_big: false,
            limit: Self::DEFAULT_LIMIT,
            over_limit: None,
        }
    }
}

impl Faults {
    /// A reply over this many folders and items is refused with the server's
    /// 403, as AIS refuses a reply that would be too big.
    pub const DEFAULT_LIMIT: usize = 20_000;

    pub fn from_env() -> Self {
        std::env::var("AURORA_DEMO_INVENTORY_FAULT")
            .ok()
            .map(|mode| Self::parse(&mode))
            .unwrap_or_default()
    }

    pub fn parse(mode: &str) -> Self {
        let mut faults = Self::default();
        for part in mode.split(',') {
            match part.split_once('=') {
                None if part == "transient" => faults.transient_left = 1,
                None if part == "too-big" => faults.too_big = true,
                Some(("transient", n)) => faults.transient_left = n.parse().unwrap_or(1),
                Some(("limit", n)) => faults.limit = n.parse().unwrap_or(Self::DEFAULT_LIMIT).max(1),
                _ => log::warn!("AURORA_DEMO_INVENTORY_FAULT: unknown option {part}"),
            }
        }
        faults
    }

    /// Point `too-big` at the seeded inventory's loose-objects folder.
    pub fn arm(&mut self, loose: Uuid) {
        if self.too_big {
            self.over_limit = Some(loose);
        }
    }
}

/// The `fetch` demo's cold load: seed the tree and items, then move the items
/// into the source so the background walk is the only loader.
pub fn seed_big_fetch(inv: &mut Inventory, agent: Uuid, items: usize) -> (Vec<Uuid>, FetchSource) {
    let open = seed_big(inv, agent, items);
    let source = FetchSource::take_from(inv);
    (open, source)
}

fn under(inv: &Inventory, mut id: Uuid, ancestor: Uuid) -> bool {
    for _ in 0..=inv.folders.len() {
        if id == ancestor {
            return true;
        }
        let Some(folder) = inv.folders.get(&id) else {
            return false;
        };
        if folder.info.parent.is_nil() {
            return false;
        }
        id = folder.info.parent;
    }
    false
}

fn fault_for(inv: &Inventory, targets: &[Uuid], depth: i32, tree: bool, faults: &mut Faults) -> Option<FetchError> {
    if let Some(wanted) = faults.over_limit
        && (targets.contains(&wanted) || (tree && depth > 0 && targets.iter().any(|target| under(inv, wanted, *target))))
    {
        return Some(FetchError::new(FetchFailure::TooBig, Some(403)));
    }
    if tree && faults.transient_left > 0 {
        faults.transient_left -= 1;
        return Some(FetchError::new(FetchFailure::Transient, Some(503)));
    }
    None
}

fn subtree(
    inv: &Inventory,
    source: &FetchSource,
    owner: Uuid,
    folder: Uuid,
    depth: i32,
    budget: &mut usize,
    out: &mut Vec<FolderContents>,
) -> Result<(), FetchError> {
    if depth < 0 {
        return Ok(());
    }
    let children = inv.folders.get(&folder).map(|folder| folder.children.clone()).unwrap_or_default();
    let items = source.items(folder);
    let cost = 1 + children.len() + items.len();
    if cost > *budget {
        return Err(FetchError::new(FetchFailure::TooBig, Some(403)));
    }
    *budget -= cost;
    let folders = children
        .iter()
        .filter_map(|id| inv.folders.get(id).map(|folder| folder.info.clone()))
        .collect();
    let version = inv.folders.get(&folder).map_or(1, |folder| folder.info.version.max(1));
    out.push(FolderContents {
        folder_id: folder,
        owner_id: owner,
        version,
        folders,
        items,
    });
    if depth > 0 {
        for child in children {
            subtree(inv, source, owner, child, depth - 1, budget, out)?;
        }
    }
    Ok(())
}

fn tree(
    inv: &Inventory,
    source: &FetchSource,
    owner: Uuid,
    parent: Uuid,
    children: &[Uuid],
    depth: i32,
    budget: &mut usize,
) -> Result<Vec<FolderContents>, FetchError> {
    let mut out = Vec::new();
    if children.is_empty() {
        subtree(inv, source, owner, parent, depth, budget, &mut out)?;
    } else {
        for child in children {
            subtree(inv, source, owner, *child, depth - 1, budget, &mut out)?;
        }
    }
    Ok(out)
}

/// The `fetch` demo's AIS server: answers tree, folder and item reads from
/// the seeded source, with the intended failures of [`Faults`]. `None` for
/// any other command (the ordinary demo reply then handles it).
pub fn fetch_reply(cmd: &NetCommand, inv: &Inventory, source: &FetchSource, faults: &mut Faults) -> Option<Vec<NetEvent>> {
    match cmd {
        NetCommand::FetchInventoryTree {
            request,
            parent,
            children,
            depth,
            owner,
            ..
        } => {
            let targets: Vec<Uuid> = if children.is_empty() { vec![*parent] } else { children.clone() };
            let result = match fault_for(inv, &targets, *depth, true, faults) {
                Some(error) => Err(error),
                None => {
                    let mut budget = faults.limit;
                    tree(inv, source, *owner, *parent, children, *depth, &mut budget)
                }
            };
            Some(vec![NetEvent::InventoryTreeFetched { request: *request, result }])
        }
        NetCommand::FetchInventory { folders, owner, .. } => {
            let mut contents = Vec::new();
            let mut failed = Vec::new();
            let mut too_big = Vec::new();
            for folder in folders {
                match fault_for(inv, &[*folder], 0, false, faults) {
                    Some(error) if error.kind == FetchFailure::TooBig => too_big.push(*folder),
                    Some(_) => failed.push(*folder),
                    None => {
                        let mut budget = faults.limit;
                        match tree(inv, source, *owner, *folder, &[], 0, &mut budget) {
                            Ok(mut read) => contents.append(&mut read),
                            Err(error) if error.kind == FetchFailure::TooBig => too_big.push(*folder),
                            Err(_) => failed.push(*folder),
                        }
                    }
                }
            }
            let mut events = Vec::new();
            if !contents.is_empty() {
                events.push(NetEvent::InventoryContents(contents));
            }
            if !failed.is_empty() {
                events.push(NetEvent::InventoryFetchFailed {
                    folders: failed,
                    too_big: false,
                    status: None,
                });
            }
            if !too_big.is_empty() {
                events.push(NetEvent::InventoryFetchFailed {
                    folders: too_big,
                    too_big: true,
                    status: Some(403),
                });
            }
            Some(events)
        }
        NetCommand::FetchItems { items, .. } => {
            let found: Vec<InvItem> = items.iter().filter_map(|id| source.item(*id)).collect();
            Some(if found.is_empty() {
                Vec::new()
            } else {
                vec![NetEvent::InventoryItems(found)]
            })
        }
        _ => None,
    }
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
        assert!(parse_big("big:800,fetch").is_some_and(|big| big.fetch));
        assert_eq!(parse_big("big").map(|b| b.fetch), Some(false));
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
                fetch: false,
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

    fn fetch_fixture(items: usize) -> (Inventory, FetchSource, Vec<Uuid>) {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            owner: Uuid::from_u128(2),
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
        let (open, source) = seed_big_fetch(&mut inv, Uuid::from_u128(2), items);
        (inv, source, open)
    }

    #[test]
    fn fetch_source_moves_the_items_out_and_leaves_an_unfetched_tree() {
        let (inv, source, open) = fetch_fixture(12_000);
        assert_eq!(inv.item_count(), 0, "the live model starts cold");
        assert!(inv.folders.values().all(|folder| folder.state == FetchState::Unknown));
        assert!(inv.folders.values().all(|folder| folder.items.is_empty()));
        assert!(open.iter().all(|id| inv.folders.contains_key(id)));
        assert_eq!(source.by_parent.values().map(Vec::len).sum::<usize>(), 12_000);
        assert_eq!(source.by_id.len(), 12_000);
        assert!(!inv.folders[&inv.root].children.is_empty(), "the tree is still known");
    }

    #[test]
    fn fetch_reply_mirrors_ais_depth_subset_and_budget() {
        let (inv, source, open) = fetch_fixture(300);
        let mut faults = Faults::default();
        let read = |parent: Uuid, children: Vec<Uuid>, depth: i32| NetCommand::FetchInventoryTree {
            request: 1,
            parent,
            children,
            depth,
            owner: inv.owner,
            library: false,
        };
        let contents = |events: Vec<NetEvent>| match events.into_iter().next() {
            Some(NetEvent::InventoryTreeFetched { result: Ok(contents), .. }) => contents,
            _ => panic!("tree reply"),
        };

        let all = contents(fetch_reply(&read(inv.root, vec![], 50), &inv, &source, &mut faults).expect("tree"));
        assert_eq!(all.len(), inv.folders.len(), "the whole seeded tree");
        assert_eq!(all[0].folder_id, inv.root, "parents first");
        assert_eq!(all.iter().map(|folder| folder.items.len()).sum::<usize>(), 300);

        let shallow = contents(fetch_reply(&read(inv.root, vec![], 0), &inv, &source, &mut faults).expect("tree"));
        assert_eq!(shallow.len(), 1, "depth 0 is the folder alone");

        let child = inv.folders[&inv.root].children[0];
        let subset = contents(fetch_reply(&read(inv.root, vec![child], 50), &inv, &source, &mut faults).expect("subset"));
        assert_eq!(subset[0].folder_id, child);
        assert!(
            subset.iter().all(|folder| folder.folder_id != inv.root),
            "only the requested subtree"
        );

        let over = |faults: &mut Faults| {
            let read = read(inv.root, vec![], 50);
            fetch_reply(&read, &inv, &source, faults).expect("tree")
        };
        assert!(matches!(
            over(&mut Faults { limit: 1, ..Default::default() }).as_slice(),
            [NetEvent::InventoryTreeFetched { result: Err(error), .. }] if error.kind == FetchFailure::TooBig
        ));

        let mut transient = Faults {
            transient_left: 1,
            ..Default::default()
        };
        assert!(matches!(
            over(&mut transient).as_slice(),
            [NetEvent::InventoryTreeFetched { result: Err(error), .. }] if error.kind == FetchFailure::Transient
        ));
        assert!(matches!(
            over(&mut transient).as_slice(),
            [NetEvent::InventoryTreeFetched { result: Ok(_), .. }]
        ));

        let mut too_big = Faults {
            too_big: true,
            ..Default::default()
        };
        too_big.arm(open[0]);
        let loose = NetCommand::FetchInventoryTree {
            request: 2,
            parent: open[0],
            children: Vec::new(),
            depth: 0,
            owner: inv.owner,
            library: false,
        };
        assert!(matches!(
            fetch_reply(&loose, &inv, &source, &mut too_big).expect("tree").as_slice(),
            [NetEvent::InventoryTreeFetched { result: Err(error), .. }] if error.kind == FetchFailure::TooBig
        ));
        let failed = NetCommand::FetchInventory {
            folders: vec![open[0]],
            owner: inv.owner,
            library: false,
        };
        assert!(matches!(
            fetch_reply(&failed, &inv, &source, &mut too_big).expect("folders").as_slice(),
            [NetEvent::InventoryFetchFailed { folders, too_big: true, status: Some(403) }] if folders == &[open[0]]
        ));
    }

    #[test]
    fn fetch_reply_serves_link_originals_by_id() {
        let (inv, source, _) = fetch_fixture(300);
        let id = source.by_id.keys().copied().next().expect("item");
        let read = NetCommand::FetchItems {
            items: vec![id],
            owner: inv.owner,
        };
        let events = fetch_reply(&read, &inv, &source, &mut Faults::default()).expect("items");
        assert!(matches!(
            events.as_slice(),
            [NetEvent::InventoryItems(items)] if items.iter().any(|item| item.id == id)
        ));
    }

    #[test]
    fn fault_modes_are_read_and_armed() {
        let mut faults = Faults::parse("transient=2,too-big,limit=5000");
        assert_eq!(faults.transient_left, 2);
        assert!(faults.too_big);
        assert_eq!(faults.limit, 5_000);
        assert_eq!(faults.over_limit, None);
        faults.arm(Uuid::from_u128(20_000_000));
        assert_eq!(faults.over_limit, Some(Uuid::from_u128(20_000_000)));
        assert_eq!(Faults::parse("transient").transient_left, 1);
        assert_eq!(Faults::parse("").limit, Faults::DEFAULT_LIMIT);
    }
}
