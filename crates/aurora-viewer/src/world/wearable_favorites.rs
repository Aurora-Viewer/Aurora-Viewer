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
use std::collections::{HashMap, HashSet};
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
    /// The order the user dragged the rows into (`Orders`). Aurora only:
    /// the Legacy window has no manual order (human request, 2026-11-15).
    Custom,
}

/// The custom orders of one account: for each view (the root for All, else
/// a tab's folder), its wearables in the order they were dragged into.
/// Kept in the settings, per account (human decision, 2026-11-15): one
/// order for All and one for each tab, on this computer only.
pub type Orders = HashMap<Uuid, Vec<Uuid>>;

impl From<u8> for Sort {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Name,
            1 => Self::Recent,
            CUSTOM => Self::Custom,
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
            Sort::Custom => CUSTOM,
        }
    }
}

/// The stored value of `Sort::Custom`, far from the Legacy values (0 to 3)
/// so that a later Legacy order cannot take it.
const CUSTOM: u8 = 100;

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
/// wearable lists' type order (`appearance::type_rank`), then the name;
/// custom: the order of `saved` (the view's entry of `Orders`), the
/// wearables it does not hold yet after it in type order.
pub fn sort(inv: &Inventory, entries: &mut [Entry], sort: Sort, saved: &[Uuid]) {
    let name = |id: &Uuid| crate::natural_order::key(inv.items.get(id).map_or("", |it| it.name.as_str()));
    match sort {
        Sort::Name => entries.sort_by_cached_key(|e| (name(&e.item), e.item)),
        Sort::Recent => entries.sort_by_cached_key(|e| {
            let date = inv.items.get(&e.link).map_or(0, |it| it.created_at);
            (std::cmp::Reverse(date), name(&e.item), e.item)
        }),
        Sort::Type => entries.sort_by_cached_key(|e| (super::appearance::type_rank(inv.items.get(&e.item)), name(&e.item), e.item)),
        Sort::Custom => {
            self::sort(inv, entries, Sort::Type, &[]);
            let place: HashMap<Uuid, usize> = saved.iter().enumerate().map(|(i, id)| (*id, i)).collect();
            // stable: the unsaved keep their type order, after the saved
            entries.sort_by_key(|e| place.get(&e.item).copied().unwrap_or(usize::MAX));
        }
    }
}

/// `order` with the wearable at `from` moved to `to`: what a row dragged
/// in the list leaves as the new custom order.
pub fn moved(order: &[Uuid], from: usize, to: usize) -> Vec<Uuid> {
    let mut out = order.to_vec();
    if from < out.len() {
        let item = out.remove(from);
        out.insert(to.min(out.len()), item);
    }
    out
}

/// `order` with `items` put at `at` (those it already holds stay where they
/// are): wearables dropped from the inventory between two rows.
pub fn inserted(order: &[Uuid], at: usize, items: &[Uuid]) -> Vec<Uuid> {
    let mut out = order.to_vec();
    let mut seen: HashSet<Uuid> = order.iter().copied().collect();
    let new: Vec<Uuid> = items.iter().copied().filter(|id| seen.insert(*id)).collect();
    let at = at.min(out.len());
    out.splice(at..at, new);
    out
}

/// The search: every word in the wearable's name or in `detail`, the
/// row's "type · point" line, case ignored. Wider than the Legacy
/// LLInventoryItemsList::setFilterSubString, which matches the name only
/// (human request, 2026-11-15: find what is worn on a point).
pub fn matches(inv: &Inventory, entry: &Entry, filter: &str, detail: &str) -> bool {
    let Some(it) = inv.items.get(&entry.item) else {
        return false;
    };
    let text = format!("{} {}", it.name, detail).to_lowercase();
    filter.split_whitespace().all(|word| text.contains(&word.to_lowercase()))
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

/// handleRemove: the links to `items` under `shown` go, the wearables stay
/// (remove_inventory_item on each link). In a tab, that tab's link; in the
/// root view, every link to the wearable, in the root and in the tabs: the
/// Legacy Viewer removes only the link it happened to list there, and the
/// wearable stayed on the list while another tab held it (deliberate
/// difference).
pub fn remove(inv: &Inventory, shown: Uuid, items: &[Uuid]) -> Option<Mutation> {
    let items: HashSet<Uuid> = items.iter().copied().collect();
    let mut change = Mutation::default();
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
            let Some(it) = inv.items.get(link).filter(|it| it.parent == id && matches!(it.asset_type, 24 | 25)) else {
                continue;
            };
            if actions::original(inv, it.id).is_some_and(|o| items.contains(&o)) {
                change.operations.push(Operation::Delete { id: it.id, folder: false });
                change.refresh.push(id);
            }
        }
        stack.extend(&f.children);
    }
    change.refresh.sort_unstable();
    change.refresh.dedup();
    (!change.operations.is_empty()).then_some(change)
}

/// Custom order chosen (from the sort menu, or by a first drag): each of
/// `views` that has no order of its own (`has` false) keeps the one it
/// showed under `from`, instead of falling back to the type order; one
/// that has an order gets it back.
pub fn seeded_orders(inv: &Inventory, views: &[Uuid], from: Sort, has: impl Fn(Uuid) -> bool) -> Vec<(Uuid, Vec<Uuid>)> {
    views
        .iter()
        .filter(|view| !has(**view))
        .map(|view| {
            let mut list = entries(inv, *view);
            sort(inv, &mut list, from, &[]);
            (*view, list.iter().map(|e| e.item).collect())
        })
        .collect()
}

/// onNewFolderCallback / onRenameFolderCallback: the name trimmed, refused
/// when empty or holding a slash (FSWearableFavoritesFolderInvalidName).
pub fn folder_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.contains(['/', '\\']) {
        return Err(aurora_i18n::tr!("ui-wearable-favorites-invalid-name", name = name.to_string()));
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
/// with it. Its contents must be known: a folder not read yet looks empty,
/// and the delete cannot be undone.
pub fn delete_folder(inv: &Inventory, root: Uuid, folder: Uuid) -> Option<Mutation> {
    let f = inv.folders.get(&folder).filter(|f| f.info.parent == root && folder != root)?;
    let only_links = f.state == FetchState::Fetched
        && f.children.is_empty()
        && f.items
            .iter()
            .all(|id| inv.items.get(id).is_some_and(|it| matches!(it.asset_type, 24 | 25)));
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
