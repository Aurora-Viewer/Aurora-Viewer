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
        (8000, inv.root, "Essais du clic droit", -1),
        (8001, personal, "Sous-dossier", -1),
        (8002, inv.root, "Annonces Place du marché", 53),
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
            (8100, "Objet de démonstration", 6, 6, 0),
            (8101, "Animation de démonstration", 20, 19, 0),
            (8102, "Script de démonstration", 10, 10, 0),
            (8103, "Instructions", 7, 7, 0),
            (8104, "Texture de démonstration", 0, 0, 0),
            (8105, "Son de démonstration", 1, 1, 0),
            (8106, "Geste de démonstration", 21, 20, 0),
            (8107, "Ciel de démonstration", 56, 25, 0),
            (8108, "Matériau de démonstration", 57, 26, 0),
            (8109, "Objet non copiable", 6, 6, 0),
            (8110, "Objet non transférable", 6, 6, 0),
            (8111, "Chemise de démonstration", 5, 18, 4),
            (8112, "Silhouette de démonstration", 13, 18, 0),
        ]
        .into_iter()
        .map(|(id, name, asset_type, inv_type, flags)| InvItem {
            id: Uuid::from_u128(id),
            parent: personal,
            asset_id: if asset_type == 20 {
                crate::demo::IDLE_ANIM
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
        name: "Lien vers l’objet".into(),
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
        | "folder-window-search" => 8000,
        "object" | "properties" | "rename" | "multi-add" | "multi-detach" | "delete" => 8100,
        "animation" | "animation-open" | "animation-properties" | "image" | "image-photo" | "image-picker" | "image-photo-save" => 8101,
        "script" => 8102,
        "note" => 8103,
        "nocopy" => 8109,
        "notransfer" => 8110,
        "link" => 8113,
        _ => return None,
    }))
}

/// Sorting/filter fixtures with distinct ages, owners, permissions and paths.
pub fn seed_view(inv: &mut Inventory, agent: Uuid, large: bool) {
    let root = inv.root;
    for (n, name, kind) in [(8200, "A — Dossier personnel", -1), (8201, "Textures", 0)] {
        if kind >= 0 && actions::system(inv, kind).is_some() {
            continue;
        }
        let id = Uuid::from_u128(n);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent: root,
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
        if let Some(folder) = inv.folders.get_mut(&root) {
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
        it.desc = "Élément de démonstration pour les filtres d'inventaire".into();
        if it.id == Uuid::from_u128(8109) {
            it.creator = Uuid::from_u128(100);
            it.flags |= 0x200000;
        }
    }
    if large {
        items.extend((0..1500).map(|n| InvItem {
            id: Uuid::from_u128(10000 + n),
            parent,
            name: format!("Élément {n}"),
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
