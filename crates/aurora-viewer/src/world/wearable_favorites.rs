//! Favorite Wearables: a folder of links to wearables and attachments,
//! with subfolders, listed by its own window (`ui/appearance/wearable_favorites.rs`).
//! Port of FSFloaterWearableFavorites::{getWearableFavoritesFolderID,
//! initCategory, updateList, refreshFolderCombo, onItemDAD, handleRemove,
//! onNewFolderCallback, onRenameFolderCallback, onDeleteFolderCallback}
//! (indra/newview/fsfloaterwearablefavorites.cpp), assign_to_favorite_folder
//! (indra/newview/llwearableitemslist.cpp), LLFindOutfitItems and the
//! comparators of LLWearableItemsList (same file), originally LGPL 2.1.
//!
//! The Legacy Viewer keeps the folder in its own root folder
//! (ROOT_FIRESTORM_FOLDER); Aurora's is `#Aurora`, like the bridge, the AO
//! and the particle scripts. Every change is an ordinary inventory
//! mutation: folders by AIS CreateInventory / UpdateCategory /
//! RemoveCategory, links by link_inventory_object, removals by
//! remove_inventory_item, as the Legacy Viewer sends them.

use super::inventory::Inventory;
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// The viewer's folder at the root (ROOT_FIRESTORM_FOLDER).
pub const ROOT: &str = super::bridge::FOLDER;
/// FS_WEARABLE_FAVORITES_FOLDER.
pub const FOLDER: &str = "#Wearable Favorites";
/// ROOT_FIRESTORM_FOLDER (indra/newview/llinventoryfunctions.h): the Legacy
/// Viewer's own root folder, where it keeps its favorites.
pub const LEGACY_ROOT: &str = "#Firestorm";

/// FSWearableFavoritesSortOrder, LLWearableItemsList::ESortOrder without
/// E_SORT_BY_TYPE_LAYER, which the window's menu does not offer. The
/// default is the setting's: 3, by type then name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    Name,
    MostRecent,
    #[default]
    TypeName,
}

/// One row: the link (or an item put in the folder by hand) and its original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub link: Uuid,
    pub target: Uuid,
}

fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    let folder = inv.folders.get(&parent)?;
    folder
        .children
        .iter()
        .copied()
        .filter(|id| {
            inv.folders
                .get(id)
                .is_some_and(|f| !f.library && f.info.parent == parent && f.info.name == name)
        })
        .min()
}

/// getWearableFavoritesFolderID: `#Wearable Favorites` in `#Aurora`.
pub fn folder(inv: &Inventory) -> Option<Uuid> {
    child(inv, child(inv, inv.root, ROOT)?, FOLDER)
}

/// getWearableFavoritesFolderID as the Legacy Viewer reads it: the same
/// folder name in its own root folder.
pub fn legacy_folder(inv: &Inventory) -> Option<Uuid> {
    child(inv, child(inv, inv.root, LEGACY_ROOT)?, FOLDER)
}

/// initCategory: the folders to create when it does not exist yet, the
/// viewer's root folder too when that one is missing. `None` when there is
/// nothing to create or no inventory yet.
pub fn create_plan(inv: &Inventory) -> Option<Mutation> {
    if inv.root.is_nil() || !inv.folders.contains_key(&inv.root) || folder(inv).is_some() {
        return None;
    }
    let mut change = Mutation::default();
    let root = child(inv, inv.root, ROOT).unwrap_or_else(|| new_folder(&mut change, inv.root, ROOT));
    new_folder(&mut change, root, FOLDER);
    change.refresh.sort();
    change.refresh.dedup();
    Some(change)
}

fn new_folder(change: &mut Mutation, parent: Uuid, name: &str) -> Uuid {
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

/// The subfolders of the favorites, by name (refreshFolderCombo, compareDict).
pub fn subfolders(inv: &Inventory, root: Uuid) -> Vec<(Uuid, String)> {
    let mut folders: Vec<(Uuid, String)> = inv
        .folders
        .get(&root)
        .into_iter()
        .flat_map(|f| &f.children)
        .filter_map(|id| inv.folders.get(id).filter(|f| f.info.parent == root))
        .map(|f| (f.info.id, f.info.name.clone()))
        .collect();
    folders.sort_by_cached_key(|(id, name)| (name.to_lowercase(), *id));
    folders
}

/// The original a row stands for, when it is one LLFindOutfitItems keeps:
/// clothing, a body part, an object or a gesture (a broken link is none).
fn original(inv: &Inventory, id: Uuid) -> Option<Uuid> {
    let it = inv.items.get(&id)?;
    let target = if it.asset_type == 24 { it.asset_id } else { it.id };
    inv.items
        .get(&target)
        .filter(|t| matches!(t.asset_type, 5 | 6 | 13 | 21))
        .map(|t| t.id)
}

/// updateList: what `shown` lists, its subfolders included
/// (collectDescendentsIf, subfolders before the folder's own items). The
/// whole favorites (`shown == root`) list each original once, by its first
/// link, as a wearable may be linked in several folders.
pub fn entries(inv: &Inventory, root: Uuid, shown: Uuid) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut seen_folders = HashSet::new();
    collect(inv, shown, &mut out, &mut seen_folders);
    if shown == root {
        let mut seen = HashSet::new();
        out.retain(|e: &Entry| seen.insert(e.target));
    }
    out
}

fn collect(inv: &Inventory, id: Uuid, out: &mut Vec<Entry>, seen: &mut HashSet<Uuid>) {
    if !seen.insert(id) {
        return;
    }
    let Some(f) = inv.folders.get(&id) else { return };
    for child in &f.children {
        if inv.folders.get(child).is_some_and(|c| c.info.parent == id) {
            collect(inv, *child, out, seen);
        }
    }
    for item in &f.items {
        if inv.items.get(item).is_some_and(|it| it.parent == id)
            && let Some(target) = original(inv, *item)
        {
            out.push(Entry { link: *item, target });
        }
    }
}

/// Whether every folder of `shown` has been fetched (isCategoryComplete).
pub fn loaded(inv: &Inventory, shown: Uuid) -> bool {
    let mut todo = vec![shown];
    let mut seen = HashSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else { return false };
        if f.state != super::inventory::FetchState::Fetched {
            return false;
        }
        todo.extend(f.children.iter().copied());
    }
    true
}

/// LLWearableItemNameComparator, LLWearableItemCreationDateComparator and
/// LLWearableItemTypeNameComparator with the window's order: clothing by
/// wearable type then name, objects by name, body parts by wearable type
/// then name, gestures by name.
pub fn sort(inv: &Inventory, entries: &mut [Entry], order: SortOrder) {
    entries.sort_by_cached_key(|e| {
        let it = inv.items.get(&e.target);
        let name = it.map(|it| it.name.to_uppercase()).unwrap_or_default();
        let (rank, kind) = match it.map(|it| it.asset_type) {
            Some(5) => (1, it.map_or(0, |it| it.flags & 0xff)),
            Some(6) => (2, 0),
            Some(13) => (3, it.map_or(0, |it| it.flags & 0xff)),
            Some(21) => (4, 0),
            _ => (5, 0),
        };
        let (rank, kind, recent) = match order {
            SortOrder::Name => (0, 0, 0),
            SortOrder::MostRecent => (0, 0, -it.map_or(0, |it| it.created_at)),
            SortOrder::TypeName => (rank, kind, 0),
        };
        (rank, kind, recent, name, e.target)
    });
}

/// The filter of the list: a case-insensitive part of the row's title, the
/// worn tag included (LLPanelInventoryListItemBase::notify "match_filter").
pub fn matches(filter: &str, name: &str, worn: Option<&str>) -> bool {
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        return true;
    }
    let title = match worn {
        Some(tag) => format!("{name} {tag}"),
        None => name.to_owned(),
    };
    title.to_lowercase().contains(&filter)
}

/// The originals a drop links in `to` (FSWearableFavoritesItemsList::
/// handleDragAndDrop, onItemDAD): clothing, body parts and objects of our
/// own inventory, a link standing for its original. Those `to` already
/// links are left out, as assign_to_favorite_folder does. Stricter than
/// onItemDAD, which links again what the folder already holds (a second
/// row for the same wearable) and does not refuse a Library item.
pub fn droppable(inv: &Inventory, ids: &[Uuid], to: Uuid) -> Vec<Uuid> {
    let present: HashSet<Uuid> = entries_in(inv, to).into_iter().map(|e| e.target).collect();
    let mut out = Vec::new();
    for id in ids {
        let Some(target) = super::inventory::actions::original(inv, *id) else {
            continue;
        };
        let Some(it) = inv.items.get(&target) else { continue };
        if !matches!(it.asset_type, 5 | 6 | 13)
            || super::inventory::actions::library(inv, target)
            || present.contains(&target)
            || out.contains(&target)
        {
            continue;
        }
        out.push(target);
    }
    out
}

/// The rows directly in `folder` (favorite_folder_contains_linked_item).
fn entries_in(inv: &Inventory, folder: Uuid) -> Vec<Entry> {
    inv.folders
        .get(&folder)
        .into_iter()
        .flat_map(|f| &f.items)
        .filter(|id| inv.items.get(id).is_some_and(|it| it.parent == folder))
        .filter_map(|id| original(inv, *id).map(|target| Entry { link: *id, target }))
        .collect()
}

fn outfit_link(it: &aurora_net::inventory::InvItem) -> OutfitLink {
    OutfitLink {
        target: it.id,
        name: it.name.clone(),
        desc: it.desc.clone(),
        folder: false,
        inv_type: it.inv_type,
    }
}

/// link_inventory_object for each original, into `to`.
pub fn link(inv: &Inventory, targets: &[Uuid], to: Uuid) -> Option<Mutation> {
    let links: Vec<OutfitLink> = targets.iter().filter_map(|id| inv.items.get(id)).map(outfit_link).collect();
    if links.is_empty() {
        return None;
    }
    Some(Mutation {
        operations: vec![Operation::Links { parent: to, links }],
        refresh: vec![to],
        ..Default::default()
    })
}

/// What importing the Legacy Viewer's favorites does.
#[derive(Debug)]
pub struct Import {
    pub change: Mutation,
    /// The links it creates.
    pub links: usize,
    /// The subfolders it creates.
    pub folders: usize,
}

/// The import of the Legacy Viewer's favorites (`#Firestorm/#Wearable
/// Favorites`) into `favorites`: each original it links is linked at the
/// same place of ours, its subfolders recreated by name (a subfolder of that
/// name is reused). Nothing is copied or moved and the Legacy folder is not
/// touched; what we already link at that place, a broken link and a Library
/// item (as `droppable` refuses) are left out, so a second run has nothing
/// to do. `None` when there is nothing to import, or while either tree is
/// not fully fetched (a partial view would link twice).
pub fn import_plan(inv: &Inventory, favorites: Uuid) -> Option<Import> {
    let legacy = legacy_folder(inv)?;
    if !loaded(inv, legacy) || !loaded(inv, favorites) {
        return None;
    }
    let mut plan = Import {
        change: Mutation::default(),
        links: 0,
        folders: 0,
    };
    let mut seen = HashSet::new();
    import_folder(inv, legacy, favorites, true, &mut plan, &mut seen);
    plan.change.refresh.sort();
    plan.change.refresh.dedup();
    (plan.links > 0).then_some(plan)
}

/// One Legacy folder into `to`; `existing` when `to` is ours already (a new
/// folder has nothing in it yet).
fn import_folder(inv: &Inventory, from: Uuid, to: Uuid, existing: bool, plan: &mut Import, seen: &mut HashSet<Uuid>) {
    if !seen.insert(from) {
        return;
    }
    let Some(f) = inv.folders.get(&from) else { return };
    let mut have: HashSet<Uuid> = if existing {
        entries_in(inv, to).into_iter().map(|e| e.target).collect()
    } else {
        HashSet::new()
    };
    let mut links = Vec::new();
    for item in &f.items {
        let Some(it) = inv.items.get(item).filter(|it| it.parent == from) else {
            continue;
        };
        let Some(target) = original(inv, it.id) else { continue };
        if super::inventory::actions::library(inv, target) || !have.insert(target) {
            continue;
        }
        if let Some(original) = inv.items.get(&target) {
            links.push(outfit_link(original));
        }
    }
    if !links.is_empty() {
        plan.links += links.len();
        plan.change.refresh.push(to);
        plan.change.operations.push(Operation::Links { parent: to, links });
    }
    let mut subfolders: Vec<_> = f
        .children
        .iter()
        .filter_map(|id| inv.folders.get(id).filter(|c| c.info.parent == from && !c.library))
        .map(|c| (c.info.name.clone(), c.info.id))
        .collect();
    subfolders.sort();
    for (name, id) in subfolders {
        let (dest, existing) = match existing.then(|| child(inv, to, &name)).flatten() {
            Some(dest) => (dest, true),
            None => {
                // Built once into its own plan, appended only when something
                // is linked in it (a probe plus a second pass would double
                // the work at every level of a deep chain).
                let dest = Uuid::new_v4();
                let mut sub = Import {
                    change: Mutation::default(),
                    links: 0,
                    folders: 0,
                };
                import_folder(inv, id, dest, false, &mut sub, seen);
                if sub.links > 0 {
                    plan.folders += 1 + sub.folders;
                    plan.links += sub.links;
                    plan.change.operations.push(Operation::Folder {
                        id: dest,
                        parent: to,
                        name: name.clone(),
                        type_default: -1,
                    });
                    plan.change.refresh.extend([to, dest]);
                    plan.change.operations.append(&mut sub.change.operations);
                    plan.change.refresh.append(&mut sub.change.refresh);
                }
                continue;
            }
        };
        import_folder(inv, id, dest, existing, plan, seen);
    }
}

/// Whether `id` is a link (an item put in the folder by hand is not).
pub fn is_link(inv: &Inventory, id: Uuid) -> bool {
    inv.items.get(&id).is_some_and(|it| it.asset_type == 24)
}

/// handleRemove: remove_inventory_item of each selected row. Stricter than
/// the Legacy Viewer, which would also delete an original dropped in the
/// folder by hand: only links are removed, an original is never deleted.
pub fn remove(inv: &Inventory, rows: &[Uuid]) -> Option<Mutation> {
    let mut change = Mutation::default();
    for id in rows {
        let Some(it) = inv.items.get(id).filter(|_| is_link(inv, *id)) else {
            continue;
        };
        change.operations.push(Operation::Delete { id: *id, folder: false });
        change.removed.push(*id);
        change.refresh.push(it.parent);
    }
    change.refresh.sort();
    change.refresh.dedup();
    (!change.operations.is_empty()).then_some(change)
}

/// Why a folder name is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    /// Empty once trimmed: the Legacy Viewer does nothing.
    Empty,
    /// "/" or "\" (FSWearableFavoritesFolderInvalidName).
    Slash(String),
}

/// onNewFolderCallback / onRenameFolderCallback: the trimmed name, without
/// "/" nor "\".
pub fn folder_name(name: &str) -> Result<String, NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.contains(['/', '\\']) {
        return Err(NameError::Slash(name.to_owned()));
    }
    Ok(aurora_net::inventory::operations::folder_name(name))
}

/// createNewCategory of a favorites folder; returns the mutation and the
/// id asked for the new folder (AIS may give another).
pub fn create_folder(root: Uuid, name: &str) -> (Mutation, Uuid) {
    let mut change = Mutation::default();
    let id = new_folder(&mut change, root, name);
    (change, id)
}

/// update_inventory_category with the new name.
pub fn rename_folder(inv: &Inventory, id: Uuid, name: &str) -> Option<Mutation> {
    let parent = inv.folders.get(&id)?.info.parent;
    Some(Mutation {
        operations: vec![Operation::Patch {
            id,
            folder: true,
            body: llsd_map! { "name" => name.to_owned() },
        }],
        refresh: vec![parent, id],
        ..Default::default()
    })
}

/// Whether the folder or a subfolder holds an item that is not a link (an
/// original put there by hand or moved in): deleting the folder would delete
/// it too.
pub fn holds_originals(inv: &Inventory, id: Uuid) -> bool {
    let mut todo = vec![id];
    let mut seen = HashSet::new();
    while let Some(f) = todo.pop() {
        if !seen.insert(f) {
            continue;
        }
        let Some(f) = inv.folders.get(&f) else { continue };
        if f.items.iter().any(|item| inv.items.contains_key(item) && !is_link(inv, *item)) {
            return true;
        }
        todo.extend(f.children.iter().copied());
    }
    false
}

/// remove_inventory_category: the folder and its links. Stricter than the
/// Legacy Viewer: `None` while the folder holds an original anywhere in its
/// subtree (`holds_originals`) or while a subfolder is not fetched yet
/// (`loaded`), so that no original is ever deleted.
pub fn delete_folder(inv: &Inventory, id: Uuid) -> Option<Mutation> {
    // An unfetched subfolder may hold an original that is not known yet.
    if !loaded(inv, id) || holds_originals(inv, id) {
        return None;
    }
    let parent = inv.folders.get(&id)?.info.parent;
    let mut removed = vec![id];
    let mut todo = vec![id];
    let mut seen = HashSet::new();
    while let Some(f) = todo.pop().filter(|f| seen.insert(*f)).and_then(|f| inv.folders.get(&f)) {
        removed.extend(f.items.iter().copied());
        removed.extend(f.children.iter().copied());
        todo.extend(f.children.iter().copied());
    }
    Some(Mutation {
        operations: vec![Operation::Delete { id, folder: true }],
        refresh: vec![parent],
        removed,
        ..Default::default()
    })
}

/// `AURORA_DEMO_WEARABLE_FAVORITES`: the favorites of the Appearance demo
/// (`appearance::seed_demo`): worn and spare clothing, attachments and body
/// parts, and an "Evening" folder holding two of them.
pub fn seed_demo(inv: &mut Inventory, agent: Uuid) {
    use super::inventory::{FetchState, Folder};
    let folder = |inv: &mut Inventory, id: Uuid, parent: Uuid, name: &str| {
        inv.folders.insert(
            id,
            Folder {
                info: aurora_net::inventory::InvFolder {
                    id,
                    parent,
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
        if let Some(p) = inv.folders.get_mut(&parent)
            && !p.children.contains(&id)
        {
            p.children.push(id);
        }
    };
    let root = inv.root;
    let brand = child(inv, root, ROOT).unwrap_or(Uuid::from_u128(780));
    if !inv.folders.contains_key(&brand) {
        folder(inv, brand, root, ROOT);
    }
    let favorites = Uuid::from_u128(781);
    let evening = Uuid::from_u128(782);
    folder(inv, favorites, brand, FOLDER);
    folder(inv, evening, favorites, "Evening");
    // the originals' creation dates, for "Sort by most recent"
    for (n, id) in (710..724).enumerate() {
        if let Some(it) = inv.items.get_mut(&Uuid::from_u128(id)) {
            it.created_at = 1_700_000_000 + (n as i64 * 86_400);
        }
    }
    let links = |ids: &[u128]| -> Vec<OutfitLink> {
        ids.iter()
            .filter_map(|id| inv.items.get(&Uuid::from_u128(*id)))
            .map(|it| OutfitLink {
                target: it.id,
                name: it.name.clone(),
                desc: it.desc.clone(),
                folder: false,
                inv_type: it.inv_type,
            })
            .collect()
    };
    // hair (body part), tattoo and jacket (worn), spare shirt, mesh hair
    // and boots (worn), the spare attachment
    let top = links(&[712, 714, 715, 722, 717, 720, 723]);
    let night = links(&[715, 719]);
    super::appearance::demo_replace(inv, agent, evening, &night);
    // the folder's contents with its subfolder: a read lists both
    let evening_info = inv.folders[&evening].info.clone();
    let items = top
        .iter()
        .map(|l| aurora_net::inventory::InvItem {
            id: Uuid::new_v4(),
            parent: favorites,
            name: l.name.clone(),
            desc: l.desc.clone(),
            asset_type: 24,
            inv_type: l.inv_type,
            asset_id: l.target,
            creator: agent,
            owner: agent,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            ..Default::default()
        })
        .collect();
    inv.apply(vec![aurora_net::inventory::FolderContents {
        folder_id: favorites,
        owner_id: agent,
        version: 2,
        folders: vec![evening_info],
        items,
    }]);
}

/// `AURORA_DEMO_WEARABLE_FAVORITES=import`: the Legacy Viewer's folder next
/// to ours (`#Firestorm/#Wearable Favorites`): one link ours has already,
/// one it lacks, a broken link, an "Evening" subfolder (ours exists) and a
/// "Party" one (ours does not).
pub fn seed_demo_legacy(inv: &mut Inventory, agent: Uuid) {
    use super::inventory::{FetchState, Folder};
    let root = inv.root;
    let folder = |inv: &mut Inventory, id: u128, parent: Uuid, name: &str| {
        let id = Uuid::from_u128(id);
        inv.folders.insert(
            id,
            Folder {
                info: aurora_net::inventory::InvFolder {
                    id,
                    parent,
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
        if let Some(p) = inv.folders.get_mut(&parent) {
            p.children.push(id);
        }
        id
    };
    let brand = folder(inv, 790, root, LEGACY_ROOT);
    let favorites = folder(inv, 791, brand, FOLDER);
    let evening = folder(inv, 792, favorites, "Evening");
    let party = folder(inv, 793, favorites, "Party");
    let links = |inv: &Inventory, ids: &[u128]| -> Vec<OutfitLink> {
        ids.iter()
            .filter_map(|id| inv.items.get(&Uuid::from_u128(*id)))
            .map(outfit_link)
            .collect()
    };
    let mut top = links(inv, &[712, 713]);
    top.push(OutfitLink {
        target: Uuid::from_u128(799),
        name: "Sold long ago".into(),
        desc: String::new(),
        folder: false,
        inv_type: 18,
    });
    let night = links(inv, &[715, 716]);
    let fun = links(inv, &[718, 721]);
    // the contents of each folder with its subfolders: a read lists both
    for (id, links) in [(evening, night), (party, fun), (favorites, top)] {
        let folders = inv.folders[&id]
            .children
            .iter()
            .filter_map(|c| inv.folders.get(c).map(|f| f.info.clone()))
            .collect();
        let items = links
            .iter()
            .map(|l| aurora_net::inventory::InvItem {
                id: Uuid::new_v4(),
                parent: id,
                name: l.name.clone(),
                desc: l.desc.clone(),
                asset_type: 24,
                inv_type: l.inv_type,
                asset_id: l.target,
                creator: agent,
                owner: agent,
                base_mask: 0x7fffffff,
                owner_mask: 0x7fffffff,
                ..Default::default()
            })
            .collect();
        inv.apply(vec![aurora_net::inventory::FolderContents {
            folder_id: id,
            owner_id: agent,
            version: 2,
            folders,
            items,
        }]);
    }
}

#[cfg(test)]
mod tests;
