//! The Favorite Wearables folder: where it lives, what it lists and the
//! inventory changes of the window (`ui/appearance/favorites.rs`). Port of
//! FSFloaterWearableFavorites (indra/newview/fsfloaterwearablefavorites.cpp,
//! originally LGPL 2.1): favorites are links to the originals, in the root
//! folder or in one of its subfolders.
//!
//! Deliberate difference (human decision, 2026-11-11): the folder is
//! `#Aurora/Wearable Favorites`, beside the AO's `#Aurora/Aurora AO`,
//! instead of the Legacy Viewer's `#Firestorm/#Wearable Favorites`.

use super::inventory::{FetchState, Inventory, actions};
use aurora_net::inventory::operations::{Mutation, Operation};
use std::collections::HashSet;
use uuid::Uuid;

/// Aurora's own folder at the inventory root, shared with the AO.
pub const BRAND: &str = "#Aurora";
pub const FOLDER: &str = "Wearable Favorites";

/// The list's order, stored with the Legacy Viewer's values of
/// `FSWearableFavoritesSortOrder` (LLWearableItemsList::ESortOrder).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum Sort {
    Name,
    /// The most recently added first (the link's creation date,
    /// LLWearableItemCreationDateComparator).
    Recent,
    /// The Legacy default.
    #[default]
    Type,
}

impl From<u8> for Sort {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Name,
            1 => Self::Recent,
            _ => Self::Type,
        }
    }
}

impl From<Sort> for u8 {
    fn from(sort: Sort) -> Self {
        match sort {
            Sort::Name => 0,
            Sort::Recent => 1,
            Sort::Type => 3,
        }
    }
}

/// One favorite: the link and the wearable it points to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub link: Uuid,
    pub item: Uuid,
}

/// getWearableFavoritesFolderID: `#Aurora/Wearable Favorites`, if it exists.
pub fn root(inv: &Inventory) -> Option<Uuid> {
    let brand = crate::ao::inventory::child(inv, inv.root, BRAND)?;
    crate::ao::inventory::child(inv, brand, FOLDER)
}

/// initCategory: the change that creates the folder, and `#Aurora` with it
/// when it is missing. None while the folder exists, or while the inventory
/// root is not loaded (a folder created then could duplicate one the
/// skeleton has not listed yet).
pub fn create(inv: &Inventory) -> Option<Mutation> {
    if root(inv).is_some() || inv.root.is_nil() || !inv.folders.contains_key(&inv.root) {
        return None;
    }
    let mut change = Mutation::default();
    let brand = crate::ao::inventory::child(inv, inv.root, BRAND).unwrap_or_else(|| folder(&mut change, inv.root, BRAND));
    folder(&mut change, brand, FOLDER);
    Some(change)
}

fn folder(change: &mut Mutation, parent: Uuid, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    change.operations.push(Operation::Folder {
        id,
        parent,
        name: name.into(),
        type_default: -1,
    });
    change.refresh.extend([parent, id]);
    id
}

/// refreshFolderCombo: the root's subfolders, by name.
pub fn folders(inv: &Inventory, root: Uuid) -> Vec<(Uuid, String)> {
    let mut list: Vec<(Uuid, String)> = inv
        .folders
        .get(&root)
        .into_iter()
        .flat_map(|f| &f.children)
        .filter_map(|id| inv.folders.get(id).filter(|f| f.info.parent == root))
        .map(|f| (f.info.id, f.info.name.clone()))
        .collect();
    list.sort_by_cached_key(|(id, name)| (crate::natural_order::key(name), *id));
    list
}

/// Whether every folder of the view is loaded (the empty message waits for
/// it, as isCategoryComplete does).
pub fn loaded(inv: &Inventory, folder: Uuid) -> bool {
    let mut stack = vec![folder];
    while let Some(id) = stack.pop() {
        let Some(f) = inv.folders.get(&id) else {
            return false;
        };
        if f.state != FetchState::Fetched {
            return false;
        }
        stack.extend(&f.children);
    }
    true
}

/// updateList: the wearables under `shown` (LLFindOutfitItems: clothing,
/// body parts and objects, a broken link left out), each original once.
/// The Legacy Viewer de-duplicates the root view only; a subfolder cannot
/// hold the same item twice here (`add` skips it), so the rule is the same.
pub fn entries(inv: &Inventory, shown: Uuid) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut stack = vec![shown];
    let mut visited = HashSet::new();
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else {
            continue;
        };
        for link in &f.items {
            let Some(it) = inv.items.get(link).filter(|it| it.parent == id) else {
                continue;
            };
            let Some(item) = actions::original(inv, it.id).filter(|o| wearable(inv, *o)) else {
                continue;
            };
            if seen.insert(item) {
                out.push(Entry { link: it.id, item });
            }
        }
        stack.extend(f.children.iter().rev());
    }
    out
}

fn wearable(inv: &Inventory, id: Uuid) -> bool {
    inv.items.get(&id).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13))
}

/// The list in the chosen order. Name: the wearable's name; most recent:
/// the link's creation date, newest first, then the name; type: the
/// wearable lists' type order (`appearance::type_rank`), then the name.
pub fn sort(inv: &Inventory, entries: &mut [Entry], sort: Sort) {
    let name = |id: &Uuid| crate::natural_order::key(inv.items.get(id).map_or("", |it| it.name.as_str()));
    match sort {
        Sort::Name => entries.sort_by_cached_key(|e| (name(&e.item), e.item)),
        Sort::Recent => entries.sort_by_cached_key(|e| {
            let date = inv.items.get(&e.link).map_or(0, |it| it.created_at);
            (std::cmp::Reverse(date), name(&e.item), e.item)
        }),
        Sort::Type => entries.sort_by_cached_key(|e| {
            (
                super::appearance::type_rank(inv.items.get(&e.item)),
                name(&e.item),
                e.item,
            )
        }),
    }
}

/// The search: every word in the wearable's name, case ignored
/// (LLInventoryItemsList::setFilterSubString matches the name).
pub fn matches(inv: &Inventory, entry: &Entry, filter: &str) -> bool {
    let Some(it) = inv.items.get(&entry.item) else {
        return false;
    };
    let name = it.name.to_lowercase();
    filter.split_whitespace().all(|word| name.contains(&word.to_lowercase()))
}

/// The wearables of `ids` a drop may add to `folder`: what
/// FSWearableFavoritesItemsList::handleDragAndDrop accepts (body parts,
/// clothing and objects), as their originals. A Library item has no place
/// in the agent's own links, and a wearable already in the folder is left
/// out instead of being linked twice (stricter than the Legacy Viewer's
/// onItemDAD, which links it again; assign_to_favorite_folder skips it).
pub fn addable(inv: &Inventory, folder: Uuid, ids: &[Uuid]) -> Vec<Uuid> {
    let mut held: HashSet<Uuid> = inv
        .folders
        .get(&folder)
        .into_iter()
        .flat_map(|f| &f.items)
        .filter_map(|id| actions::original(inv, *id))
        .collect();
    ids.iter()
        .filter_map(|id| actions::original(inv, *id))
        .filter(|id| wearable(inv, *id) && !actions::library(inv, *id))
        .filter(|id| held.insert(*id))
        .collect()
}

/// onItemDAD / assign_to_favorite_folder: links to `items` in `folder`.
pub fn add(inv: &Inventory, folder: Uuid, items: &[Uuid]) -> Option<Mutation> {
    let links: Vec<_> = items
        .iter()
        .filter_map(|id| inv.items.get(id))
        .map(super::appearance::item_link)
        .collect();
    if links.is_empty() || !inv.folders.contains_key(&folder) {
        return None;
    }
    Some(Mutation {
        operations: vec![Operation::Links { parent: folder, links }],
        refresh: vec![folder],
        ..Default::default()
    })
}

/// handleRemove: the links go, the wearables stay (remove_inventory_item
/// on each link).
pub fn remove(inv: &Inventory, links: &[Uuid]) -> Option<Mutation> {
    let mut change = Mutation::default();
    for id in links {
        let Some(it) = inv.items.get(id).filter(|it| matches!(it.asset_type, 24 | 25)) else {
            continue;
        };
        change.operations.push(Operation::Delete { id: *id, folder: false });
        change.refresh.push(it.parent);
    }
    change.refresh.sort_unstable();
    change.refresh.dedup();
    (!change.operations.is_empty()).then_some(change)
}

/// onNewFolderCallback / onRenameFolderCallback: the name trimmed, refused
/// when empty or holding a slash (FSWearableFavoritesFolderInvalidName).
pub fn folder_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.contains(['/', '\\']) {
        return Err(aurora_i18n::tr!("ui-wearable-favorites-invalid-name", name = name.to_string()).into());
    }
    actions::clean_name(name)
}

/// onNewFolderCallback: a subfolder of the root.
pub fn new_folder(root: Uuid, name: &str) -> Result<(Uuid, Mutation), String> {
    let change = actions::create_folder(root, &folder_name(name)?)?;
    let id = match change.operations.first() {
        Some(Operation::Folder { id, .. }) => *id,
        _ => return Err(aurora_i18n::tr!("common-item-not-found").into()),
    };
    Ok((id, change))
}

/// onRenameFolderCallback: only a subfolder, never the root.
pub fn rename_folder(inv: &Inventory, root: Uuid, folder: Uuid, name: &str) -> Result<Mutation, String> {
    if folder == root || inv.folders.get(&folder).is_none_or(|f| f.info.parent != root) {
        return Err(aurora_i18n::tr!("common-item-not-found").into());
    }
    actions::patch(inv, folder, aurora_llsd::llsd_map! { "name" => folder_name(name)? })
}

/// onDeleteFolderCallback: a subfolder and its links (remove_inventory_category);
/// never the root, and nothing but links inside, so no wearable is lost
/// with it.
pub fn delete_folder(inv: &Inventory, root: Uuid, folder: Uuid) -> Option<Mutation> {
    let f = inv.folders.get(&folder).filter(|f| f.info.parent == root && folder != root)?;
    let only_links = f.children.is_empty()
        && f
            .items
            .iter()
            .all(|id| inv.items.get(id).is_none_or(|it| matches!(it.asset_type, 24 | 25)));
    if !only_links {
        return None;
    }
    Some(Mutation {
        operations: vec![Operation::Delete { id: folder, folder: true }],
        refresh: vec![root],
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "wearable_favorites_tests.rs"]
mod tests;
