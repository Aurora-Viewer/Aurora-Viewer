//! Favorite wearables: links to clothing, body parts and objects kept in an
//! inventory folder, with one level of named subfolders. Behaviour of
//! FSFloaterWearableFavorites and assign_to_favorite_folder
//! (indra/newview/fsfloaterwearablefavorites.cpp, llwearableitemslist.cpp,
//! originally LGPL 2.1): the root view lists every favorite once, a
//! subfolder its own, and a favorite can sit in several subfolders.
//!
//! Aurora differences:
//! - the folder is `#Aurora/Wearable Favorites`, made by the first change
//!   rather than at login; `#Firestorm/#Wearable Favorites` stays read-only,
//!   like the AO's source (`ao/inventory.rs`);
//! - `sync` copies what that folder holds on request, adding only, and the
//!   most recent sort keeps reading the dates of the links it copied;
//! - dropping a favorite a folder already holds adds nothing;
//! - removing from the root view removes every link to the item, so that
//!   its row does not come straight back from a subfolder;
//! - a folder holding a real item moved in by hand is not deleted, as the
//!   delete would take the item with it.

use super::inventory::{FetchState, Inventory, actions};
use crate::ao::inventory::child;
use aurora_llsd::llsd_map;
use aurora_net::inventory::InvItem;
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const BRAND: &str = "#Aurora";
const FOLDER: &str = "Wearable Favorites";
const LEGACY_BRAND: &str = "#Firestorm";
const LEGACY_FOLDER: &str = "#Wearable Favorites";

/// FSWearableFavoritesSortOrder: its three choices, type and name first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Sort {
    Name,
    Recent,
    #[default]
    Type,
}

/// One row: a favorite and the item it stands for.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub link: Uuid,
    /// The original, which is what is worn.
    pub item: Uuid,
    pub name: String,
    pub asset_type: i32,
    pub inv_type: i32,
    pub flags: u32,
    /// When it became a favorite: the most recent sort reads the link's date,
    /// or the Legacy link's where that is older (`legacy_dates`).
    pub linked_at: i64,
}

pub fn root(inv: &Inventory) -> Option<Uuid> {
    child(inv, child(inv, inv.root, BRAND)?, FOLDER)
}

/// getWearableFavoritesFolderID.
pub fn legacy_root(inv: &Inventory) -> Option<Uuid> {
    child(inv, child(inv, inv.root, LEGACY_BRAND)?, LEGACY_FOLDER)
}

/// Every folder of `tree` is read.
fn tree_read(inv: &Inventory, tree: Uuid) -> bool {
    let mut todo = vec![tree];
    let mut seen = HashSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        match inv.folders.get(&id) {
            Some(f) if f.state == FetchState::Fetched => todo.extend(f.children.iter().copied()),
            _ => return false,
        }
    }
    true
}

/// The folders a change depends on are read: asking for `#Aurora/Wearable
/// Favorites` before that would make a second one, and a favorite an unread
/// folder holds would be linked twice.
fn loaded(inv: &Inventory) -> bool {
    let fetched = |id: Uuid| inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Fetched);
    fetched(inv.root)
        && [BRAND, LEGACY_BRAND]
            .iter()
            .filter_map(|name| child(inv, inv.root, name))
            .all(fetched)
        && [root(inv), legacy_root(inv)].into_iter().flatten().all(|tree| tree_read(inv, tree))
}

/// Ask for the folders the window reads, in bounded batches; true once all
/// of them are here.
pub fn fetch(inv: &mut Inventory) -> bool {
    let mut complete = inv.folders.get(&inv.root).is_some_and(|f| f.state == FetchState::Fetched);
    for name in [BRAND, LEGACY_BRAND] {
        if let Some(id) = child(inv, inv.root, name)
            && let Some(state) = inv.folders.get(&id).map(|f| f.state)
        {
            if state == FetchState::Unknown {
                inv.request(id);
            }
            complete &= state == FetchState::Fetched;
        }
    }
    for tree in [root(inv), legacy_root(inv)].into_iter().flatten() {
        complete &= actions::request_tree(inv, tree);
    }
    complete
}

/// Every item under `folder`, its subfolders included.
fn items_under(inv: &Inventory, folder: Uuid) -> Vec<Uuid> {
    let mut out = Vec::new();
    let mut todo = vec![folder];
    let mut seen = HashSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(f) = inv.folders.get(&id) {
            out.extend(f.items.iter().copied());
            todo.extend(f.children.iter().copied());
        }
    }
    out
}

/// The originals the links of both folders point at and the inventory has
/// not read yet, for one FetchItems.
pub fn missing(inv: &Inventory) -> Vec<Uuid> {
    let mut out: Vec<Uuid> = [root(inv), legacy_root(inv)]
        .into_iter()
        .flatten()
        .flat_map(|tree| items_under(inv, tree))
        .filter_map(|id| inv.items.get(&id))
        .filter(|it| it.asset_type == 24 && !inv.items.contains_key(&it.asset_id) && !inv.missing_items.contains(&it.asset_id))
        .map(|it| it.asset_id)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn wearable(it: &InvItem) -> bool {
    matches!(it.asset_type, 5 | 6 | 13)
}

/// When each original became a Legacy favorite. A sync makes its links
/// together, so their own date cannot order them.
fn legacy_dates(inv: &Inventory) -> HashMap<Uuid, i64> {
    let mut dates: HashMap<Uuid, i64> = HashMap::new();
    let links = legacy_root(inv).map(|legacy| items_under(inv, legacy)).unwrap_or_default();
    for link in links {
        if let Some((item, date)) = actions::original(inv, link).zip(inv.items.get(&link).map(|l| l.created_at)) {
            dates.entry(item).and_modify(|d| *d = date.min(*d)).or_insert(date);
        }
    }
    dates
}

/// The rows of `folder`: LLFindOutfitItems' types, and at the root one row
/// per original (updateList's "All Favorites").
pub fn entries(inv: &Inventory, folder: Uuid, root: Uuid) -> Vec<Entry> {
    let dates = legacy_dates(inv);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for link in items_under(inv, folder) {
        let Some(it) = actions::original(inv, link)
            .and_then(|id| inv.items.get(&id))
            .filter(|it| wearable(it))
        else {
            continue;
        };
        if folder != root || seen.insert(it.id) {
            out.push(Entry {
                link,
                item: it.id,
                name: it.name.clone(),
                asset_type: it.asset_type,
                inv_type: it.inv_type,
                flags: it.flags,
                linked_at: inv
                    .items
                    .get(&link)
                    .map_or(0, |l| dates.get(&it.id).map_or(l.created_at, |d| l.created_at.min(*d))),
            });
        }
    }
    out
}

/// LLWearableItemNameComparator, LLWearableItemCreationDateComparator
/// (newest first) and LLWearableItemTypeNameComparator as E_SORT_BY_TYPE_NAME
/// sets it: clothing by wearable type, then objects, then body parts by
/// wearable type, by name within each.
pub fn sort(entries: &mut [Entry], order: Sort) {
    let name = |e: &Entry| (e.name.to_lowercase(), e.link);
    match order {
        Sort::Name => entries.sort_by_cached_key(name),
        Sort::Recent => entries.sort_by_cached_key(|e| (std::cmp::Reverse(e.linked_at), name(e))),
        Sort::Type => entries.sort_by_cached_key(|e| {
            let group = match e.asset_type {
                5 => (0, e.flags & 0xff),
                13 => (2, e.flags & 0xff),
                _ => (1, 0),
            };
            (group, name(e))
        }),
    }
}

/// The subfolders by name, as the folder picker lists them.
pub fn folders(inv: &Inventory, root: Uuid) -> Vec<(Uuid, String)> {
    let mut out: Vec<(Uuid, String)> = inv
        .folders
        .get(&root)
        .into_iter()
        .flat_map(|f| f.children.iter())
        .filter_map(|id| inv.folders.get(id).map(|f| (*id, f.info.name.clone())))
        .collect();
    out.sort_by_cached_key(|(id, name)| (name.to_lowercase(), *id));
    out
}

fn originals(inv: &Inventory, folder: Uuid) -> HashSet<Uuid> {
    inv.folders
        .get(&folder)
        .into_iter()
        .flat_map(|f| f.items.iter())
        .filter_map(|id| actions::original(inv, *id))
        .collect()
}

fn new_folder_in(change: &mut Mutation, parent: Uuid, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    change.operations.push(Operation::Folder {
        id,
        parent,
        name: name.to_owned(),
        type_default: -1,
    });
    change.refresh.extend([parent, id]);
    id
}

/// The favorites' folder, made in `change` when there is none yet.
fn ensure_root(inv: &Inventory, change: &mut Mutation) -> Result<Uuid, String> {
    if !loaded(inv) {
        return Err(aurora_i18n::tr!("wearable-favorites-loading").into());
    }
    Ok(root(inv).unwrap_or_else(|| {
        let brand = child(inv, inv.root, BRAND).unwrap_or_else(|| new_folder_in(change, inv.root, BRAND));
        new_folder_in(change, brand, FOLDER)
    }))
}

/// A link to the original of `id`, for what the list accepts: the cargo of
/// handleDragAndDrop (body parts, clothing, objects), and what a link can
/// reach (not the library, not a coalesced object).
fn link_to(inv: &Inventory, id: Uuid) -> Option<OutfitLink> {
    let original = actions::original(inv, id)?;
    let it = inv.items.get(&original).filter(|it| wearable(it))?;
    (!actions::library(inv, original) && actions::outfit_link_target(inv, original)).then(|| OutfitLink {
        target: original,
        name: it.name.clone(),
        desc: it.desc.clone(),
        folder: false,
        inv_type: it.inv_type,
    })
}

fn push_links(change: &mut Mutation, parent: Uuid, links: Vec<OutfitLink>) {
    if !links.is_empty() {
        change.operations.push(Operation::Links { parent, links });
        change.refresh.push(parent);
    }
}

fn finish(mut change: Mutation) -> Mutation {
    change.refresh.sort();
    change.refresh.dedup();
    change
}

/// Links `ids` into `folder` (None: the root, made if needed), without those
/// it already holds. No operation: nothing to add.
pub fn add(inv: &Inventory, folder: Option<Uuid>, ids: &[Uuid]) -> Result<Mutation, String> {
    if !loaded(inv) {
        return Err(aurora_i18n::tr!("wearable-favorites-loading").into());
    }
    let mut have = folder
        .map(|f| originals(inv, f))
        .or_else(|| root(inv).map(|r| originals(inv, r)))
        .unwrap_or_default();
    let links: Vec<OutfitLink> = ids
        .iter()
        .filter_map(|id| link_to(inv, *id))
        .filter(|link| have.insert(link.target))
        .collect();
    let mut change = Mutation::default();
    if links.is_empty() {
        return Ok(change);
    }
    let parent = match folder {
        Some(folder) => folder,
        None => ensure_root(inv, &mut change)?,
    };
    push_links(&mut change, parent, links);
    Ok(finish(change))
}

/// handleRemove, for the rows' originals `items`: a subfolder loses its own
/// links, the root view every link under it. Only links go: a real item
/// moved into the folder by hand stays.
pub fn remove(inv: &Inventory, folder: Uuid, root: Uuid, items: &HashSet<Uuid>) -> Mutation {
    let mut change = Mutation::default();
    let under = if folder == root {
        items_under(inv, root)
    } else {
        inv.folders.get(&folder).map(|f| f.items.clone()).unwrap_or_default()
    };
    for id in under {
        if let Some(it) = inv.items.get(&id)
            && it.asset_type == 24
            && actions::original(inv, id).is_some_and(|original| items.contains(&original))
        {
            change.operations.push(Operation::Delete { id, folder: false });
            change.refresh.push(it.parent);
            change.removed.push(id);
        }
    }
    finish(change)
}

/// onNewFolderCallback's rule for a name: trimmed, without a slash.
pub fn valid_name(name: &str) -> Result<String, String> {
    if name.contains(['/', '\\']) {
        return Err(aurora_i18n::tr!("wearable-favorites-invalid-name", name = name.trim().to_string()));
    }
    actions::clean_name(name)
}

/// A subfolder; the id is this change's own, `Reply::created` maps it.
pub fn new_folder(inv: &Inventory, name: &str) -> Result<(Mutation, Uuid), String> {
    let name = valid_name(name)?;
    let mut change = Mutation::default();
    let parent = ensure_root(inv, &mut change)?;
    let id = new_folder_in(&mut change, parent, &name);
    Ok((finish(change), id))
}

pub fn rename(inv: &Inventory, folder: Uuid, name: &str) -> Result<Mutation, String> {
    actions::patch(inv, folder, llsd_map! { "name" => valid_name(name)? })
}

/// onDeleteFolderCallback: the subfolder goes, and with it only links.
pub fn delete_folder(inv: &Inventory, folder: Uuid) -> Result<Mutation, String> {
    if !tree_read(inv, folder) {
        return Err(aurora_i18n::tr!("wearable-favorites-loading").into());
    }
    if items_under(inv, folder)
        .iter()
        .any(|id| inv.items.get(id).is_none_or(|it| !matches!(it.asset_type, 24 | 25)))
    {
        return Err(aurora_i18n::tr!("wearable-favorites-delete-real-items").into());
    }
    let parent = inv
        .folders
        .get(&folder)
        .map(|f| f.info.parent)
        .ok_or(aurora_i18n::tr!("common-item-not-found"))?;
    Ok(Mutation {
        operations: vec![Operation::Delete { id: folder, folder: true }],
        refresh: vec![parent],
        removed: vec![folder],
        ..Default::default()
    })
}

/// What a sync would do.
#[derive(Debug, Default)]
pub struct Sync {
    pub change: Mutation,
    pub added: usize,
}

/// The Legacy folder's favorites into Aurora's: its loose links to the root,
/// each subfolder's to the one of the same name, made if needed. What Aurora
/// holds already, wherever a link came from, is left alone; nothing is
/// removed, and every write lands under `#Aurora`.
pub fn sync(inv: &Inventory) -> Result<Sync, String> {
    if !loaded(inv) || !missing(inv).is_empty() {
        return Err(aurora_i18n::tr!("wearable-favorites-loading").into());
    }
    let legacy = legacy_root(inv).ok_or(aurora_i18n::tr!("common-item-not-found"))?;
    let mut groups: Vec<(Option<&str>, Vec<Uuid>)> = vec![(None, inv.folders.get(&legacy).map(|f| f.items.clone()).unwrap_or_default())];
    let subfolders = folders(inv, legacy);
    groups.extend(subfolders.iter().map(|(id, name)| (Some(name.as_str()), items_under(inv, *id))));

    let mut out = Sync::default();
    let current = root(inv);
    let mut made_root = None;
    for (name, items) in groups {
        let existing = match (name, current) {
            (None, root) => root,
            (Some(name), Some(root)) => child(inv, root, name),
            (Some(_), None) => None,
        };
        let mut have = existing.map(|f| originals(inv, f)).unwrap_or_default();
        let links: Vec<OutfitLink> = items
            .iter()
            .filter_map(|id| link_to(inv, *id))
            .filter(|link| have.insert(link.target))
            .collect();
        if links.is_empty() && (name.is_none() || existing.is_some()) {
            continue;
        }
        out.added += links.len();
        let parent = match (existing, name) {
            (Some(folder), _) => folder,
            (None, name) => {
                let top = match made_root {
                    Some(top) => top,
                    None => {
                        let top = ensure_root(inv, &mut out.change)?;
                        made_root = Some(top);
                        top
                    }
                };
                name.map_or(top, |name| new_folder_in(&mut out.change, top, name))
            }
        };
        push_links(&mut out.change, parent, links);
    }
    out.change = finish(out.change);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{Folder, actions::demo_mutate};
    use aurora_net::inventory::InvFolder;

    const OWNER: Uuid = Uuid::from_u128(1);
    const ROOT: Uuid = Uuid::from_u128(2);
    const CLOTHES: Uuid = Uuid::from_u128(3);
    const SHIRT: Uuid = Uuid::from_u128(10);
    const HAIR: Uuid = Uuid::from_u128(11);
    const SHAPE: Uuid = Uuid::from_u128(12);
    const SCRIPT: Uuid = Uuid::from_u128(13);

    fn folder(inv: &mut Inventory, parent: Uuid, id: Uuid, name: &str) {
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
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
    }

    fn item(id: Uuid, parent: Uuid, name: &str, asset_type: i32, inv_type: i32) -> InvItem {
        InvItem {
            id,
            parent,
            name: name.into(),
            asset_type,
            inv_type,
            owner: OWNER,
            ..Default::default()
        }
    }

    fn inventory() -> Inventory {
        let mut inv = Inventory {
            owner: OWNER,
            root: ROOT,
            ..Default::default()
        };
        folder(&mut inv, Uuid::nil(), ROOT, "My Inventory");
        folder(&mut inv, ROOT, CLOTHES, "Clothes");
        inv.add_items(vec![
            item(SHIRT, CLOTHES, "Shirt", 5, 18),
            item(HAIR, CLOTHES, "Hair", 6, 17),
            item(SHAPE, CLOTHES, "Shape", 13, 18),
            item(SCRIPT, CLOTHES, "Script", 10, 10),
        ]);
        inv
    }

    fn link(inv: &mut Inventory, id: u128, parent: Uuid, target: Uuid) {
        let mut it = item(Uuid::from_u128(id), parent, "link", 24, 18);
        it.asset_id = target;
        inv.add_items(vec![it]);
    }

    /// `#Firestorm/#Wearable Favorites` with a loose favorite and a subfolder.
    fn with_legacy(inv: &mut Inventory) -> Uuid {
        folder(inv, ROOT, Uuid::from_u128(20), LEGACY_BRAND);
        let legacy = Uuid::from_u128(21);
        folder(inv, Uuid::from_u128(20), legacy, LEGACY_FOLDER);
        folder(inv, legacy, Uuid::from_u128(22), "Evening");
        link(inv, 30, legacy, SHIRT);
        link(inv, 31, Uuid::from_u128(22), HAIR);
        link(inv, 32, Uuid::from_u128(22), SHIRT);
        legacy
    }

    fn apply(inv: &mut Inventory, change: impl FnOnce(&Inventory) -> Mutation) {
        let change = change(inv);
        demo_mutate(inv, OWNER, change).expect("applied");
    }

    fn names(inv: &Inventory, folder: Uuid, root: Uuid) -> Vec<String> {
        let mut rows = entries(inv, folder, root);
        sort(&mut rows, Sort::Name);
        rows.into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn the_first_favorite_makes_the_folder_and_a_second_drop_adds_nothing() {
        let mut inv = inventory();
        assert!(root(&inv).is_none(), "reading the window writes nothing");
        apply(&mut inv, |inv| add(inv, None, &[SHIRT]).unwrap());
        let top = root(&inv).expect("made under #Aurora");
        assert_eq!(names(&inv, top, top), ["Shirt"]);
        assert!(add(&inv, None, &[SHIRT]).unwrap().operations.is_empty());
    }

    #[test]
    fn a_drop_takes_clothing_body_parts_and_objects_only_and_follows_links() {
        let mut inv = inventory();
        link(&mut inv, 40, CLOTHES, HAIR);
        apply(&mut inv, |inv| {
            add(inv, None, &[SCRIPT, SHAPE, Uuid::from_u128(40), CLOTHES]).unwrap()
        });
        let top = root(&inv).unwrap();
        assert_eq!(names(&inv, top, top), ["Hair", "Shape"]);
    }

    #[test]
    fn the_root_lists_each_favorite_once_and_a_subfolder_its_own() {
        let mut inv = inventory();
        apply(&mut inv, |inv| add(inv, None, &[SHIRT]).unwrap());
        apply(&mut inv, |inv| new_folder(inv, "Evening").unwrap().0);
        let top = root(&inv).unwrap();
        let evening = folders(&inv, top)[0].0;
        apply(&mut inv, |inv| add(inv, Some(evening), &[SHIRT, HAIR]).unwrap());
        assert_eq!(names(&inv, top, top), ["Hair", "Shirt"]);
        assert_eq!(names(&inv, evening, top), ["Hair", "Shirt"]);
    }

    #[test]
    fn removing_from_a_subfolder_keeps_the_others_and_the_root_clears_them_all() {
        let mut inv = inventory();
        apply(&mut inv, |inv| add(inv, None, &[SHIRT, HAIR]).unwrap());
        apply(&mut inv, |inv| new_folder(inv, "Evening").unwrap().0);
        let top = root(&inv).unwrap();
        let evening = folders(&inv, top)[0].0;
        apply(&mut inv, |inv| add(inv, Some(evening), &[SHIRT]).unwrap());

        apply(&mut inv, |inv| remove(inv, evening, top, &HashSet::from([SHIRT])));
        assert!(names(&inv, evening, top).is_empty());
        assert_eq!(names(&inv, top, top), ["Hair", "Shirt"]);

        apply(&mut inv, |inv| add(inv, Some(evening), &[SHIRT]).unwrap());
        apply(&mut inv, |inv| remove(inv, top, top, &HashSet::from([SHIRT])));
        assert_eq!(names(&inv, top, top), ["Hair"]);
        assert!(inv.items.contains_key(&SHIRT), "the original stays");
    }

    #[test]
    fn remove_leaves_a_real_item_moved_into_the_folder() {
        let mut inv = inventory();
        apply(&mut inv, |inv| add(inv, None, &[HAIR]).unwrap());
        let top = root(&inv).unwrap();
        inv.add_items(vec![item(Uuid::from_u128(50), top, "Moved here", 5, 18)]);
        let change = remove(&inv, top, top, &HashSet::from([Uuid::from_u128(50)]));
        assert!(change.operations.is_empty());
    }

    #[test]
    fn folder_names_follow_the_legacy_rule() {
        let inv = inventory();
        assert!(valid_name("a/b").is_err() && valid_name("a\\b").is_err() && valid_name("   ").is_err());
        assert_eq!(valid_name("  Evening ").unwrap(), "Evening");
        assert!(new_folder(&inv, "a/b").is_err());
    }

    #[test]
    fn the_three_sorts() {
        let row = |name: &str, kind: i32, at: i64| Entry {
            link: Uuid::new_v4(),
            item: Uuid::new_v4(),
            name: name.into(),
            asset_type: kind,
            inv_type: 18,
            flags: 0,
            linked_at: at,
        };
        let mut rows = vec![row("b hair", 6, 1), row("c shirt", 5, 3), row("a shape", 13, 2)];
        let order = |rows: &[Entry]| rows.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        sort(&mut rows, Sort::Name);
        assert_eq!(order(&rows), ["a shape", "b hair", "c shirt"]);
        sort(&mut rows, Sort::Recent);
        assert_eq!(order(&rows), ["c shirt", "a shape", "b hair"]);
        sort(&mut rows, Sort::Type);
        assert_eq!(order(&rows), ["c shirt", "b hair", "a shape"]);
    }

    #[test]
    fn the_most_recent_sort_keeps_the_legacy_dates_of_a_sync() {
        let mut inv = inventory();
        let legacy = with_legacy(&mut inv);
        let date = |inv: &mut Inventory, id: u128, at: i64| {
            let mut it = inv.items.get(&Uuid::from_u128(id)).expect("link").clone();
            it.created_at = at;
            inv.add_items(vec![it]);
        };
        link(&mut inv, 33, legacy, HAIR);
        date(&mut inv, 30, 100); // Shirt, first Legacy link
        date(&mut inv, 32, 300); // Shirt again, in a subfolder
        date(&mut inv, 31, 400); // Hair, first Legacy link
        date(&mut inv, 33, 200); // Hair, the older
        apply(&mut inv, |inv| sync(inv).expect("sync").change);
        let root = root(&inv).expect("made");
        let links: Vec<_> = inv.folders[&root].items.clone();
        for link in links {
            let mut it = inv.items[&link].clone();
            it.created_at = 900;
            inv.add_items(vec![it]);
        }
        let mut rows = entries(&inv, root, root);
        sort(&mut rows, Sort::Recent);
        assert_eq!(
            rows.iter().map(|e| (e.name.as_str(), e.linked_at)).collect::<Vec<_>>(),
            [("Hair", 200), ("Shirt", 100)]
        );

        // a favorite Aurora made itself keeps its own date
        apply(&mut inv, |inv| add(inv, None, &[SHAPE]).expect("add"));
        let shape = inv.folders[&root]
            .items
            .iter()
            .copied()
            .find(|id| !inv.items[id].asset_id.is_nil() && inv.items[id].asset_id == SHAPE);
        let mut it = inv.items[&shape.expect("link")].clone();
        it.created_at = 950;
        inv.add_items(vec![it]);
        let mut rows = entries(&inv, root, root);
        sort(&mut rows, Sort::Recent);
        assert_eq!(rows[0].name, "Shape");
    }

    #[test]
    fn sync_copies_the_legacy_favorites_and_never_writes_under_firestorm() {
        let mut inv = inventory();
        let legacy = with_legacy(&mut inv);
        let plan = sync(&inv).unwrap();
        assert_eq!(plan.added, 3);
        for op in &plan.change.operations {
            let target = match op {
                Operation::Folder { parent, .. } | Operation::Links { parent, .. } => *parent,
                other => panic!("sync only adds: {other:?}"),
            };
            assert!(!actions::under(&inv, target, legacy) && target != legacy);
        }
        apply(&mut inv, |_| plan.change);
        let top = root(&inv).unwrap();
        let (evening, name) = folders(&inv, top).remove(0);
        assert_eq!(name, "Evening");
        assert_eq!(names(&inv, evening, top), ["Hair", "Shirt"]);
        assert_eq!(names(&inv, top, top), ["Hair", "Shirt"]);
        assert_eq!(inv.folders[&legacy].items.len(), 1, "the source is unchanged");
    }

    #[test]
    fn a_second_sync_adds_nothing_and_keeps_what_was_removed_from_the_legacy_side() {
        let mut inv = inventory();
        with_legacy(&mut inv);
        apply(&mut inv, |inv| sync(inv).unwrap().change);
        let again = sync(&inv).unwrap();
        assert_eq!(again.added, 0);
        assert!(again.change.operations.is_empty());

        // gone from the Legacy folder: Aurora keeps its favorite
        inv.remove(&[Uuid::from_u128(30)]);
        let again = sync(&inv).unwrap();
        assert!(again.change.operations.is_empty());
        let top = root(&inv).unwrap();
        assert_eq!(names(&inv, top, top), ["Hair", "Shirt"]);
    }

    #[test]
    fn sync_only_fills_in_an_existing_subfolder_and_keeps_its_own_favorites() {
        let mut inv = inventory();
        with_legacy(&mut inv);
        apply(&mut inv, |inv| new_folder(inv, "Evening").unwrap().0);
        let top = root(&inv).unwrap();
        let evening = folders(&inv, top)[0].0;
        apply(&mut inv, |inv| add(inv, Some(evening), &[SHAPE, HAIR]).unwrap());
        let plan = sync(&inv).unwrap();
        assert_eq!(plan.added, 2, "the loose Shirt and the subfolder's Shirt");
        apply(&mut inv, |_| plan.change);
        assert_eq!(folders(&inv, top).len(), 1, "matched by name");
        assert_eq!(names(&inv, evening, top), ["Hair", "Shape", "Shirt"]);
    }

    #[test]
    fn nothing_is_written_before_the_folders_that_decide_are_read() {
        let mut inv = inventory();
        with_legacy(&mut inv);
        inv.folders.get_mut(&ROOT).unwrap().state = FetchState::Unknown;
        assert!(sync(&inv).is_err());
        assert!(add(&inv, None, &[SHIRT]).is_err());
        inv.folders.get_mut(&ROOT).unwrap().state = FetchState::Fetched;
        folder(&mut inv, ROOT, Uuid::from_u128(60), BRAND);
        inv.folders.get_mut(&Uuid::from_u128(60)).unwrap().state = FetchState::Fetching;
        assert!(add(&inv, None, &[SHIRT]).is_err(), "#Aurora may hold the folder already");
    }

    #[test]
    fn nothing_is_written_before_both_favorites_folders_are_read() {
        let mut inv = inventory();
        let legacy = with_legacy(&mut inv);
        apply(&mut inv, |inv| sync(inv).expect("sync").change);
        let top = root(&inv).expect("made");
        let evening = child(&inv, top, "Evening").expect("copied");
        let read = |inv: &mut Inventory, id: Uuid, state| inv.folders.get_mut(&id).unwrap().state = state;

        for folder in [top, evening, legacy, Uuid::from_u128(22)] {
            read(&mut inv, folder, FetchState::Fetching);
            assert!(sync(&inv).is_err(), "an unread folder would be linked twice");
            assert!(add(&inv, None, &[SHAPE]).is_err());
            read(&mut inv, folder, FetchState::Fetched);
        }
        assert!(sync(&inv).is_ok());
        assert!(add(&inv, None, &[SHAPE]).is_ok());
    }

    #[test]
    fn sync_waits_for_the_originals_of_the_legacy_links() {
        let mut inv = inventory();
        let legacy = with_legacy(&mut inv);
        let unread = Uuid::from_u128(70);
        link(&mut inv, 71, legacy, unread);
        assert!(sync(&inv).is_err(), "a favorite would be skipped unseen");
        inv.missing_items.insert(unread);
        assert_eq!(sync(&inv).expect("a broken link stops the wait").added, 3);
    }

    #[test]
    fn a_folder_holding_a_real_item_is_not_deleted() {
        let mut inv = inventory();
        apply(&mut inv, |inv| new_folder(inv, "Evening").expect("folder").0);
        let top = root(&inv).expect("made");
        let evening = child(&inv, top, "Evening").expect("made");
        apply(&mut inv, |inv| add(inv, Some(evening), &[SHIRT]).expect("add"));
        assert!(delete_folder(&inv, evening).is_ok(), "only links");

        let mut hair = inv.items[&HAIR].clone();
        hair.parent = evening;
        inv.add_items(vec![hair]);
        assert!(delete_folder(&inv, evening).is_err(), "the item would go with it");

        inv.folders.get_mut(&evening).unwrap().state = FetchState::Fetching;
        assert!(delete_folder(&inv, evening).is_err(), "its contents are unknown");
    }

    #[test]
    fn links_whose_original_is_unread_are_asked_for_once_each() {
        let mut inv = inventory();
        let legacy = with_legacy(&mut inv);
        let unread = Uuid::from_u128(70);
        link(&mut inv, 71, legacy, unread);
        link(&mut inv, 72, Uuid::from_u128(22), unread);
        assert_eq!(missing(&inv), [unread]);
        inv.missing_items.insert(unread);
        assert!(missing(&inv).is_empty(), "a broken link stops asking");
    }
}
