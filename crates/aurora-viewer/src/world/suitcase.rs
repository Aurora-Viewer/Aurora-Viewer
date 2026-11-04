//! What stays behind on a Hypergrid trip. With OpenSim's suitcase
//! (HGSuitcaseInventoryService), other grids see only the My Suitcase
//! folder (LLFolderType::FT_MY_SUITCASE, 100): an attachment or a wearable
//! worn from anywhere else cannot be loaded there, and goes missing on
//! arrival. The Legacy Viewer says nothing; Aurora asks before the
//! teleport leaves (`App::teleport_to_location_looking`).

use super::inventory::Inventory;
use uuid::Uuid;

/// LLFolderType::FT_MY_SUITCASE.
pub const MY_SUITCASE: i32 = 100;

/// The agent's My Suitcase folder; None on a grid without the suitcase,
/// where a visited grid reads the whole inventory.
pub fn folder(inv: &Inventory) -> Option<Uuid> {
    inv.folders
        .values()
        .filter(|f| !f.library && f.info.type_default == MY_SUITCASE)
        .map(|f| f.info.id)
        .min()
}

/// Whether `folder` is My Suitcase or inside it.
fn inside(inv: &Inventory, suitcase: Uuid, mut folder: Uuid) -> bool {
    // bounded: a malformed parent chain must not hang the check
    for _ in 0..64 {
        if folder == suitcase {
            return true;
        }
        match inv.folders.get(&folder) {
            Some(f) if !f.info.parent.is_nil() => folder = f.info.parent,
            _ => return false,
        }
    }
    false
}

/// A worn item that stays behind: its name and what kind it is (for its
/// inventory icon).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftBehind {
    pub name: String,
    pub asset_type: i32,
    pub inv_type: i32,
    pub flags: u32,
}

/// The worn attachments and wearables (the Current Outfit folder's links)
/// whose items are not in My Suitcase, by name; empty when everything worn
/// travels, or on a grid without the suitcase.
pub fn left_behind(inv: &Inventory) -> Vec<LeftBehind> {
    let (Some(suitcase), Some(cof)) = (folder(inv), super::appearance::cof(inv)) else {
        return Vec::new();
    };
    let mut items: Vec<LeftBehind> = super::appearance::folder_links(inv, cof)
        .into_iter()
        .filter(|link| !link.folder)
        .filter_map(|link| inv.items.get(&link.target))
        .filter(|item| matches!(item.asset_type, 5 | 6 | 13))
        .filter(|item| !inside(inv, suitcase, item.parent))
        .map(|item| LeftBehind {
            name: item.name.clone(),
            asset_type: item.asset_type,
            inv_type: item.inv_type,
            flags: item.flags,
        })
        .collect();
    items.sort_by_key(|item| item.name.to_lowercase());
    items.dedup();
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// Root 1 with My Suitcase 2 (holding 3), Objects 4 and the Current
    /// Outfit 5; `worn` items linked from the outfit.
    fn inventory(suitcase: bool, worn: &[(u128, u128, &str)]) -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        let folder = |n: u128, parent: u128, type_default: i32| InvFolder {
            id: id(n),
            parent: id(parent),
            name: format!("Folder {n}"),
            type_default,
            version: 1,
            ..Default::default()
        };
        let mut folders = vec![folder(1, 0, 8), folder(3, 2, -1), folder(4, 1, 6), folder(5, 1, 46)];
        if suitcase {
            folders.push(folder(2, 1, MY_SUITCASE));
        }
        inv.add_folders(folders, false);
        let mut items = Vec::new();
        for (n, &(item, parent, name)) in worn.iter().enumerate() {
            items.push(InvItem {
                id: id(item),
                parent: id(parent),
                name: name.into(),
                asset_type: 6,
                inv_type: 17,
                ..Default::default()
            });
            items.push(InvItem {
                id: id(900 + n as u128),
                parent: id(5),
                name: name.into(),
                asset_type: 24,
                asset_id: id(item),
                ..Default::default()
            });
        }
        inv.add_items(items);
        // as a fetch of each folder lists it
        let listed: Vec<(Uuid, Uuid)> = inv.items.values().map(|it| (it.parent, it.id)).collect();
        for (parent, item) in listed {
            if let Some(f) = inv.folders.get_mut(&parent)
                && !f.items.contains(&item)
            {
                f.items.push(item);
            }
        }
        inv
    }

    #[test]
    fn worn_items_outside_the_suitcase_are_named() {
        let inv = inventory(true, &[(10, 4, "Tail"), (11, 2, "Ears"), (12, 3, "Head"), (13, 4, "body")]);
        assert_eq!(folder(&inv), Some(id(2)));
        let names: Vec<String> = left_behind(&inv).into_iter().map(|item| item.name).collect();
        assert_eq!(names, ["body", "Tail"], "in My Suitcase or below it: they travel");
    }

    #[test]
    fn no_suitcase_no_warning() {
        let inv = inventory(false, &[(10, 4, "Tail")]);
        assert!(left_behind(&inv).is_empty(), "the visited grid reads the whole inventory");
    }
}
