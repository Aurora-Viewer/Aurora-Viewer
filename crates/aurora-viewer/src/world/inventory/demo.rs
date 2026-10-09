//! Synthetic inventory for context-menu validation; never reads grid data.

use super::*;

pub fn seed(inv: &mut Inventory, agent: Uuid) {
    super::super::appearance::seed_demo(inv, agent);
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
            asset_id: Uuid::from_u128(9000 + id),
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
        "folder" | "clothes" | "body" | "settings" | "uploads" => 8000,
        "object" | "properties" => 8100,
        "animation" => 8101,
        "script" => 8102,
        "note" => 8103,
        "nocopy" => 8109,
        "notransfer" => 8110,
        "link" => 8113,
        _ => return None,
    }))
}
