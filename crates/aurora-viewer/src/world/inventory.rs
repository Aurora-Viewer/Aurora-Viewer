//! Client-side inventory model: login skeleton + lazily fetched folders.

use aurora_llsd::Llsd;
use aurora_net::inventory::{FolderContents, InvFolder, InvItem, parse_skeleton};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
#[cfg(test)]
#[path = "inventory/tests.rs"]
mod action_tests;
pub mod actions;
pub mod background;
pub mod demo;
pub mod preview;
pub mod wearable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchState {
    Unknown,
    Fetching,
    Fetched,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub info: InvFolder,
    pub children: Vec<Uuid>,
    pub items: Vec<Uuid>,
    pub state: FetchState,
    pub library: bool,
}

#[derive(Default)]
pub struct Inventory {
    /// Session-only RLVa preview restrictions, shared by every inventory floater.
    pub rlva_preview_mask: u8,
    pub root: Uuid,
    pub lib_root: Uuid,
    pub lib_owner: Uuid,
    pub folders: HashMap<Uuid, Folder>,
    pub items: HashMap<Uuid, InvItem>,
    /// Folders waiting to be fetched (id, library).
    pub queue: Vec<(Uuid, bool)>,
    /// Folders asked for while the cache file is being read (`hold_requests`).
    pub held: Option<Vec<Uuid>>,
    /// Bumped whenever folders or items change (cached views rebuild).
    pub generation: u64,
    /// Link originals the server answered do not exist, or still unreadable
    /// after `ITEM_FETCH_ATTEMPTS`: broken links, which outfits skip instead
    /// of waiting for them (LLViewerInventoryItem::getIsBrokenLink).
    pub missing_items: HashSet<Uuid>,
    /// Failed item reads (network, server error), by attempt.
    pub item_failures: HashMap<Uuid, u8>,
    /// Failed item reads to ask for again (`take_item_retries`).
    pub item_retries: HashSet<Uuid>,
}

/// Reads of one item before it counts as a broken link's original. A failed
/// read proves nothing; the Legacy Viewer treats a link still unresolved once
/// its background fetch ended as broken (LLAppearanceMgr::updateAppearanceFromCOF).
pub const ITEM_FETCH_ATTEMPTS: u8 = 3;

fn first_id(v: &Llsd, key: &str) -> Uuid {
    v.at(0)[key].as_uuid()
}

impl Inventory {
    pub fn can_preview(&self, asset_type: i32) -> bool {
        self.rlva_preview_mask
            & match asset_type {
                0 => 1,
                7 => 2,
                10 => 4,
                _ => 0,
            }
            == 0
    }
    /// Check ancestry in O(depth), without finding system folders by a full
    /// inventory scan. The bound also rejects malformed parent cycles.
    pub fn in_folder_type(&self, id: Uuid, kind: i32) -> bool {
        let mut current = self.items.get(&id).map_or(id, |item| item.parent);
        for _ in 0..=self.folders.len() {
            let Some(folder) = self.folders.get(&current) else { return false };
            if folder.info.type_default == kind {
                return true;
            }
            current = folder.info.parent;
        }
        false
    }
    /// Consume one original immediately after a reliable rez/drop is queued.
    /// Touch only its folder, rather than scanning the whole inventory.
    /// A failed rez is restored by the simulator's inventory update.
    pub fn consume_item(&mut self, id: Uuid) {
        if let Some(item) = self.items.remove(&id) {
            if let Some(folder) = self.folders.get_mut(&item.parent) {
                folder.items.retain(|other| *other != id);
            }
            self.generation += 1;
        }
    }
    pub fn from_login(raw: &Llsd) -> Inventory {
        let mut inv = Inventory {
            root: first_id(&raw["inventory-root"], "folder_id"),
            lib_root: first_id(&raw["inventory-lib-root"], "folder_id"),
            lib_owner: first_id(&raw["inventory-lib-owner"], "agent_id"),
            ..Default::default()
        };
        inv.add_folders(parse_skeleton(&raw["inventory-skeleton"]), false);
        inv.add_folders(parse_skeleton(&raw["inventory-skel-lib"]), true);
        inv
    }

    fn add_folders(&mut self, list: Vec<InvFolder>, library: bool) {
        for f in list {
            self.upsert_folder(f, library);
        }
        // rebuild child lists
        let ids: Vec<(Uuid, Uuid)> = self.folders.values().map(|f| (f.info.id, f.info.parent)).collect();
        for f in self.folders.values_mut() {
            f.children.clear();
        }
        for (id, parent) in ids {
            if let Some(p) = self.folders.get_mut(&parent)
                && !p.children.contains(&id)
            {
                p.children.push(id);
            }
        }
        self.sort_all();
    }

    fn upsert_folder(&mut self, mut f: InvFolder, library: bool) {
        match self.folders.get_mut(&f.id) {
            Some(existing) => {
                if !f.favorite_known && !f.favorite {
                    f.favorite = existing.info.favorite;
                    f.favorite_known = existing.info.favorite_known;
                }
                // A parent listing can carry older/unknown child versions.
                // Keep the version of the contents already confirmed locally.
                let version = existing.info.version;
                existing.info = f;
                existing.info.version = existing.info.version.max(version);
            }
            None => {
                self.folders.insert(
                    f.id,
                    Folder {
                        info: f,
                        children: Vec::new(),
                        items: Vec::new(),
                        state: FetchState::Unknown,
                        library,
                    },
                );
            }
        }
    }

    fn sort_children(&mut self, id: Uuid) {
        let Some(f) = self.folders.get(&id) else {
            return;
        };
        let mut kids = f.children.clone();
        let mut seen = HashSet::new();
        kids.retain(|child| self.folders.get(child).is_some_and(|f| f.info.parent == id) && seen.insert(*child));
        // system folders first, then by name; one lowercase name per entry
        // (a background reply can list thousands of folders at once)
        kids.sort_by_cached_key(|id| {
            let f = self.folders.get(id);
            let sys = f.is_some_and(|f| f.info.type_default >= 0 && f.info.type_default != 8);
            (!sys, f.map(|f| f.info.name.to_lowercase()).unwrap_or_default())
        });
        let mut items = f.items.clone();
        seen.clear();
        // FetchInventoryDescendents2 may include linked originals from other
        // folders. LLInventoryModel::updateItem indexes each by its parent.
        items.retain(|item| self.items.get(item).is_some_and(|it| it.parent == id) && seen.insert(*item));
        items.sort_by_cached_key(|i| self.items.get(i).map(|it| it.name.to_lowercase()).unwrap_or_default());
        if let Some(f) = self.folders.get_mut(&id) {
            f.children = kids;
            f.items = items;
        }
    }

    fn sort_all(&mut self) {
        let ids: Vec<Uuid> = self.folders.keys().copied().collect();
        for id in ids {
            self.sort_children(id);
        }
    }

    /// Queue a folder fetch if not fetched yet.
    pub fn request(&mut self, id: Uuid) {
        if let Some(held) = &mut self.held {
            if !held.contains(&id) {
                held.push(id);
            }
            return;
        }
        if let Some(f) = self.folders.get_mut(&id)
            && matches!(f.state, FetchState::Unknown | FetchState::Failed)
        {
            f.state = FetchState::Fetching;
            self.queue.push((id, f.library));
        }
    }

    pub fn apply(&mut self, contents: Vec<FolderContents>) {
        self.generation += 1;
        for c in contents {
            // Inventory versions advance as copies arrive (Firestorm's
            // LLInventoryModel::accountForUpdate). A delayed pre-copy HTTP
            // snapshot must not replace the newer, populated folder.
            if self
                .folders
                .get(&c.folder_id)
                .is_some_and(|f| f.state == FetchState::Fetched && c.version < f.info.version)
            {
                continue;
            }
            let library = self.folders.get(&c.folder_id).map(|f| f.library).unwrap_or(false);
            let child_ids: Vec<Uuid> = c.folders.iter().map(|f| f.id).collect();
            for f in c.folders {
                self.upsert_folder(f, library);
            }
            let item_ids: Vec<Uuid> = c.items.iter().map(|i| i.id).collect();
            if let Some(f) = self.folders.get_mut(&c.folder_id) {
                f.children = child_ids;
                f.items = item_ids;
                f.state = FetchState::Fetched;
                f.info.version = c.version;
            }
            self.add_items(c.items);
            self.sort_children(c.folder_id);
        }
    }

    /// Items fetched by id (FetchInventory2), listed in their folder when
    /// that folder is loaded.
    pub fn add_items(&mut self, items: Vec<InvItem>) {
        self.generation += 1;
        let mut changed = HashSet::new();
        for mut it in items {
            self.preserve_favorite(&mut it);
            if let Some(old) = self.items.get(&it.id)
                && old.parent != it.parent
                && let Some(folder) = self.folders.get_mut(&old.parent)
            {
                folder.items.retain(|id| *id != it.id);
                changed.insert(old.parent);
            }
            if let Some(f) = self.folders.get_mut(&it.parent)
                && f.state == FetchState::Fetched
            {
                if !f.items.contains(&it.id) {
                    f.items.push(it.id);
                }
                changed.insert(it.parent);
            }
            self.items.insert(it.id, it);
        }
        for id in changed {
            self.sort_children(id);
        }
    }

    /// LLInventoryModel::accountForUpdate / updateItem / updateCategory:
    /// apply server changes immediately, and advance each loaded parent's
    /// version once per update. Repeated records do not advance it again.
    pub fn server_updated(&mut self, folders: Vec<InvFolder>, mut items: Vec<InvItem>) {
        items.retain(|item| !item.id.is_nil());
        for item in &mut items {
            self.preserve_favorite(item);
        }
        let mut changed = HashSet::new();
        for folder in folders {
            let id = folder.id;
            let parent = folder.parent;
            if let Some(old) = self.folders.get(&id) {
                if old.info.parent != parent || old.info.name != folder.name {
                    changed.insert(old.info.parent);
                    changed.insert(parent);
                }
                let old_parent = old.info.parent;
                if old_parent != parent
                    && let Some(old) = self.folders.get_mut(&old_parent)
                {
                    old.children.retain(|child| *child != id);
                }
            } else {
                changed.insert(parent);
            }
            self.upsert_folder(folder, false);
            if let Some(parent) = self.folders.get_mut(&parent)
                && !parent.children.contains(&id)
            {
                parent.children.push(id);
            }
        }
        for item in &items {
            if self.items.get(&item.id) != Some(item) {
                changed.insert(item.parent);
                if let Some(old) = self.items.get(&item.id) {
                    changed.insert(old.parent);
                }
            }
        }
        for id in &changed {
            if let Some(folder) = self.folders.get_mut(id)
                && folder.state == FetchState::Fetched
                && folder.info.version >= 0
            {
                folder.info.version = folder.info.version.saturating_add(1);
            }
        }
        self.add_items(items);
        for id in changed {
            self.sort_children(id);
        }
        self.generation += 1;
    }

    /// Partial/UDP records lack favorite metadata. Preserve what AIS already
    /// confirmed, while a complete AIS record can explicitly clear the flag.
    fn preserve_favorite(&self, item: &mut InvItem) {
        if !item.favorite_known
            && !item.favorite
            && let Some(old) = self.items.get(&item.id)
        {
            item.favorite = old.favorite;
            item.favorite_known = old.favorite_known;
        }
    }

    /// AIS has acknowledged these new, empty categories. Subsequent server
    /// item updates can populate them without waiting for a folder fetch.
    pub fn created_folders(&mut self, folders: Vec<InvFolder>) {
        for folder in folders {
            if let Some(existing) = self.folders.get_mut(&folder.id) {
                // Bulk notifications can precede the AIS response (also
                // handled by Firestorm's createNewCategory callback). Keep
                // already received children/items while completing its state.
                if existing.state != FetchState::Fetched {
                    let known_descendants = existing.children.len().saturating_add(existing.items.len());
                    let version = folder.version.saturating_add(known_descendants.min(i32::MAX as usize) as i32);
                    existing.info.version = existing.info.version.max(version);
                    existing.state = FetchState::Fetched;
                    self.generation += 1;
                }
                continue;
            }
            let id = folder.id;
            self.server_updated(vec![folder], Vec::new());
            if let Some(folder) = self.folders.get_mut(&id) {
                folder.state = FetchState::Fetched;
            }
        }
    }

    /// A broken link's original: confirmed gone, and not received since.
    pub fn broken(&self, id: &Uuid) -> bool {
        self.missing_items.contains(id) && !self.items.contains_key(id)
    }

    /// The server answered that these items do not exist.
    pub fn items_missing(&mut self, ids: Vec<Uuid>) {
        self.generation += 1;
        for id in ids {
            self.item_failures.remove(&id);
            self.item_retries.remove(&id);
            self.missing_items.insert(id);
        }
    }

    /// These item reads failed: asked again, up to `ITEM_FETCH_ATTEMPTS`.
    pub fn items_failed(&mut self, ids: &[Uuid]) {
        self.generation += 1;
        for id in ids {
            let attempts = self.item_failures.entry(*id).or_insert(0);
            *attempts += 1;
            if *attempts >= ITEM_FETCH_ATTEMPTS {
                log::warn!("inventory item {id} still unreadable after {attempts} attempts: treated as a broken link");
                self.item_retries.remove(id);
                self.missing_items.insert(*id);
            } else {
                self.item_retries.insert(*id);
            }
        }
    }

    /// The failed item reads to send again, once each.
    pub fn take_item_retries(&mut self) -> HashSet<Uuid> {
        std::mem::take(&mut self.item_retries)
    }

    pub fn failed(&mut self, folders: &[Uuid]) {
        self.generation += 1;
        for id in folders {
            if let Some(f) = self.folders.get_mut(id) {
                f.state = FetchState::Failed;
            }
        }
    }

    /// Ask again for every folder whose fetch failed: the agent's region
    /// has its capabilities (LLInventoryModelBackgroundFetch::backgroundFetch
    /// waits for capabilitiesReceived instead of giving up).
    pub fn retry_failed(&mut self) {
        let failed: Vec<Uuid> = self
            .folders
            .iter()
            .filter(|(_, f)| f.state == FetchState::Failed)
            .map(|(id, _)| *id)
            .collect();
        for id in failed {
            self.request(id);
        }
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// AIS deletions remove descendants too; retain no stale search results.
    pub fn remove(&mut self, ids: &[Uuid]) {
        let mut removed: HashSet<_> = ids.iter().copied().collect();
        loop {
            let old = removed.len();
            for f in self.folders.values() {
                if removed.contains(&f.info.parent) {
                    removed.insert(f.info.id);
                }
            }
            if old == removed.len() {
                break;
            }
        }
        self.items.retain(|id, it| !removed.contains(id) && !removed.contains(&it.parent));
        self.folders.retain(|id, _| !removed.contains(id));
        self.sort_all();
        self.generation += 1;
    }

    /// Write fetched folders and their items (LLInventoryModel::saveToFile).
    pub fn save_cache(&self, path: &std::path::Path) {
        use aurora_llsd::llsd_map;
        let mut folders = Vec::new();
        let mut items = Vec::new();
        for f in self.folders.values().filter(|f| f.state == FetchState::Fetched) {
            folders.push(llsd_map! {
                "id" => f.info.id,
                "parent" => f.info.parent,
                "name" => f.info.name.clone(),
                "type" => f.info.type_default,
                "version" => f.info.version,
                "favorite" => f.info.favorite,
                "thumbnail" => f.info.thumbnail,
            });
            for id in &f.items {
                if let Some(it) = self.items.get(id) {
                    items.push(llsd_map! {
                            "id" => it.id,
                            "parent" => it.parent,
                            "name" => it.name.clone(),
                            "desc" => it.desc.clone(),
                            "asset_type" => it.asset_type,
                            "inv_type" => it.inv_type,
                            "asset_id" => it.asset_id,
                            "flags" => it.flags as i32,
                    "favorite" => it.favorite,
                        "thumbnail" => it.thumbnail,
                        "base_mask" => it.base_mask as i32,
                        "owner_mask" => it.owner_mask as i32,
                        "last_owner" => it.last_owner, "group_id" => it.group_id, "group_owned" => it.group_owned,
                        "sale_type" => i32::from(it.sale_type), "sale_price" => it.sale_price,
                            "creator" => it.creator,
                            "created_at" => it.created_at as i32,
                            "owner" => it.owner,
                            "group_mask" => it.group_mask as i32,
                            "everyone_mask" => it.everyone_mask as i32,
                            "next_owner_mask" => it.next_owner_mask as i32,
                        });
                }
            }
        }
        if folders.is_empty() {
            return;
        }
        let doc = llsd_map! {
            "version" => 3,
            "folders" => Llsd::Array(folders),
            "items" => Llsd::Array(items),
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if crate::cache::write(&tmp, &aurora_llsd::to_binary(&doc)).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    /// Restore cached folders whose version still matches the login
    /// skeleton (LLInventoryModel::loadSkeleton): they need no fetch. The
    /// file was read by [`read_cache`]; what is left here is cheap. What
    /// arrived while the file was being read is newer than the cache: an
    /// item already known is kept, and a folder that received one is left
    /// to the fetch.
    pub fn apply_cache(&mut self, cache: CachedInventory) -> usize {
        let touched: HashSet<Uuid> = self.items.values().map(|it| it.parent).collect();
        let mut valid = 0;
        for c in cache.folders {
            let Some(cur) = self.folders.get_mut(&c.id) else {
                continue;
            };
            if cur.state != FetchState::Unknown || cur.info.version != c.version || cur.info.version < 0 || touched.contains(&c.id) {
                continue;
            }
            cur.info.favorite = c.favorite;
            cur.info.favorite_known = true;
            cur.info.thumbnail = c.thumbnail;
            cur.state = FetchState::Fetched;
            let mut listed = Vec::with_capacity(c.items.len());
            for it in c.items {
                if let Entry::Vacant(slot) = self.items.entry(it.id) {
                    listed.push(it.id);
                    slot.insert(it);
                }
            }
            if let Some(cur) = self.folders.get_mut(&c.id) {
                cur.items = listed;
            }
            valid += 1;
        }
        self.generation += 1;
        valid
    }

    /// While the cache file is being read, folder requests wait: a folder the
    /// cache holds must not be fetched. [`Inventory::release_requests`] ends it.
    pub fn hold_requests(&mut self) {
        self.held.get_or_insert_with(Vec::new);
    }

    /// Queue the requests made since [`Inventory::hold_requests`], for the
    /// folders the cache did not restore.
    pub fn release_requests(&mut self) {
        for id in self.held.take().unwrap_or_default() {
            self.request(id);
        }
    }
}

/// One folder of the cache file: the version it was saved at and its items,
/// in display order.
struct CachedFolder {
    id: Uuid,
    version: i32,
    favorite: bool,
    thumbnail: Uuid,
    items: Vec<InvItem>,
}

/// An inventory cache file, read and decoded ([`read_cache`]).
pub struct CachedInventory {
    folders: Vec<CachedFolder>,
}

/// Read a cache file written by [`Inventory::save_cache`]. Everything that
/// costs is here and needs no inventory, so that it runs off the main thread
/// (27 ms for 6 500 items when it was done while the login reply was applied):
/// the file, its LLSD document, the items and their order in each folder,
/// as `sort_children` gives it.
pub fn read_cache(path: &std::path::Path) -> Option<CachedInventory> {
    let bytes = crate::cache::read_touch(path).ok()?;
    let (doc, _) = aurora_llsd::from_binary(&bytes).ok()?;
    // Older files were populated by legacy reads that could discard other
    // favorites. Reload those folders through AIS once, even at equal versions.
    if doc["version"].as_i32() != 3 {
        return None;
    }
    let mut by_parent: HashMap<Uuid, Vec<InvItem>> = HashMap::new();
    // old caches listed an item once per folder that showed it
    let mut seen = HashSet::new();
    for v in doc["items"].as_array() {
        let parent = v["parent"].as_uuid();
        let it = InvItem {
            id: v["id"].as_uuid(),
            parent,
            name: v["name"].to_string_value(),
            desc: v["desc"].to_string_value(),
            asset_type: v["asset_type"].as_i32(),
            inv_type: v["inv_type"].as_i32(),
            asset_id: v["asset_id"].as_uuid(),
            flags: v["flags"].as_i32() as u32,
            favorite: v["favorite"].as_bool(),
            favorite_known: true,
            creator: v["creator"].as_uuid(),
            created_at: v["created_at"].as_i32() as i64,
            owner: v["owner"].as_uuid(),
            group_mask: v["group_mask"].as_i32() as u32,
            everyone_mask: v["everyone_mask"].as_i32() as u32,
            next_owner_mask: v["next_owner_mask"].as_i32() as u32,
            thumbnail: v["thumbnail"].as_uuid(),
            base_mask: v["base_mask"].as_u32(),
            owner_mask: v["owner_mask"].as_u32(),
            last_owner: v["last_owner"].as_uuid(),
            group_id: v["group_id"].as_uuid(),
            group_owned: v["group_owned"].as_bool(),
            sale_type: v["sale_type"].as_u32() as u8,
            sale_price: v["sale_price"].as_i32(),
        };
        if it.id.is_nil() || !seen.insert(it.id) {
            continue;
        }
        by_parent.entry(parent).or_default().push(it);
    }
    let folders = doc["folders"]
        .as_array()
        .iter()
        .map(|f| {
            let id = f["id"].as_uuid();
            let mut items = by_parent.remove(&id).unwrap_or_default();
            items.sort_by_cached_key(|it| it.name.to_lowercase());
            CachedFolder {
                id,
                version: f["version"].as_i32(),
                favorite: f["favorite"].as_bool(),
                thumbnail: f["thumbnail"].as_uuid(),
                items,
            }
        })
        .collect();
    Some(CachedInventory { folders })
}

/// [`read_cache`] on its own thread; the receiver gets one answer (`None`:
/// no usable cache), or hangs up if the thread could not start.
pub fn read_cache_in_background(path: std::path::PathBuf) -> crossbeam_channel::Receiver<Option<CachedInventory>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let spawned = std::thread::Builder::new().name("aurora-inventory-cache".into()).spawn(move || {
        let _ = tx.send(read_cache(&path));
    });
    if let Err(e) = spawned {
        log::warn!("cannot start the inventory cache reader: {e}");
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Inventory {
        let mut inv = Inventory {
            root: Uuid::from_u128(800),
            ..Default::default()
        };
        inv.folders.insert(
            inv.root,
            Folder {
                info: InvFolder {
                    id: inv.root,
                    parent: Uuid::nil(),
                    name: aurora_i18n::tr!("common-inventory").into(),
                    type_default: 8,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        super::super::appearance::seed_demo(&mut inv, Uuid::from_u128(1));
        inv
    }

    #[test]
    fn fetched_link_targets_are_stored_under_their_own_parent_once() {
        let mut inv = fixture();
        let folder = Uuid::from_u128(704);
        let original = inv.items.remove(&Uuid::from_u128(723)).expect("original");
        inv.folders.get_mut(&inv.root).expect("root").items.retain(|id| *id != original.id);
        let mut link = inv.items[&inv.folders[&folder].items[0]].clone();
        link.asset_id = original.id;
        for _ in 0..2 {
            inv.apply(vec![FolderContents {
                folder_id: folder,
                owner_id: Uuid::from_u128(1),
                version: 4,
                folders: Vec::new(),
                items: vec![link.clone(), original.clone(), link.clone()],
            }]);
            assert_eq!(
                inv.folders[&folder].items,
                vec![link.id],
                "only direct children belong to the outfit"
            );
            assert_eq!(inv.items[&original.id].parent, original.parent, "linked target is retained");
            assert_eq!(inv.folders[&inv.root].items.iter().filter(|id| **id == original.id).count(), 1);
        }
    }

    #[test]
    fn changing_an_item_parent_removes_the_old_membership() {
        let mut inv = fixture();
        let mut item = inv.items[&Uuid::from_u128(723)].clone();
        let old = item.parent;
        item.parent = Uuid::from_u128(702);
        inv.add_items(vec![item.clone(), item.clone()]);
        assert!(!inv.folders[&old].items.contains(&item.id));
        assert_eq!(inv.folders[&item.parent].items, vec![item.id]);
    }

    #[test]
    fn new_copy_folders_and_items_appear_without_a_snapshot_and_survive_stale_reads() {
        for early_notification in [false, true] {
            let mut inv = fixture();
            let parent = Uuid::from_u128(9700);
            let child = Uuid::from_u128(9701);
            if early_notification {
                inv.server_updated(
                    vec![
                        InvFolder {
                            id: parent,
                            parent: inv.root,
                            name: "Copied folder".into(),
                            type_default: -1,
                            version: -1,
                            ..Default::default()
                        },
                        InvFolder {
                            id: child,
                            parent,
                            name: "Nested folder".into(),
                            type_default: -1,
                            version: -1,
                            ..Default::default()
                        },
                    ],
                    Vec::new(),
                );
            }
            inv.created_folders(vec![
                InvFolder {
                    id: parent,
                    parent: inv.root,
                    name: "Copied folder".into(),
                    type_default: -1,
                    version: 1,
                    ..Default::default()
                },
                InvFolder {
                    id: child,
                    parent,
                    name: "Nested folder".into(),
                    type_default: -1,
                    version: 1,
                    ..Default::default()
                },
            ]);
            assert_eq!(inv.folders[&parent].children, vec![child]);
            assert_eq!(inv.folders[&parent].state, FetchState::Fetched);
            assert_eq!(inv.folders[&parent].info.version, 2);
            let mut copied = inv.items[&Uuid::from_u128(723)].clone();
            copied.id = Uuid::from_u128(9900);
            copied.parent = child;
            inv.server_updated(Vec::new(), vec![copied.clone()]);
            assert_eq!(inv.folders[&child].items, vec![copied.id]);
            assert_eq!(inv.folders[&child].info.version, 2);
            inv.server_updated(Vec::new(), vec![copied.clone()]);
            assert_eq!(
                inv.folders[&child].info.version, 2,
                "duplicate records do not invent a newer version"
            );
            inv.apply(vec![FolderContents {
                folder_id: child,
                owner_id: Uuid::from_u128(1),
                version: 1,
                folders: Vec::new(),
                items: Vec::new(),
            }]);
            assert_eq!(inv.folders[&child].items, vec![copied.id]);
            assert!(inv.items.contains_key(&Uuid::from_u128(723)), "copy keeps the original");
        }
    }

    #[test]
    fn stale_folder_snapshots_do_not_erase_confirmed_copy_contents() {
        let mut inv = fixture();
        let folder = Uuid::from_u128(704);
        let mut copied = inv.items[&Uuid::from_u128(723)].clone();
        copied.id = Uuid::from_u128(9999);
        copied.parent = folder;
        inv.apply(vec![FolderContents {
            folder_id: folder,
            owner_id: Uuid::from_u128(1),
            version: 5,
            folders: Vec::new(),
            items: vec![copied.clone()],
        }]);
        let mut metadata = inv.folders[&folder].info.clone();
        metadata.version = 1;
        inv.apply(vec![FolderContents {
            folder_id: metadata.parent,
            owner_id: Uuid::from_u128(1),
            version: 6,
            folders: vec![metadata],
            items: Vec::new(),
        }]);
        inv.apply(vec![FolderContents {
            folder_id: folder,
            owner_id: Uuid::from_u128(1),
            version: 4,
            folders: Vec::new(),
            items: Vec::new(),
        }]);
        assert_eq!(inv.folders[&folder].info.version, 5);
        assert_eq!(inv.folders[&folder].items, vec![copied.id]);
    }

    #[test]
    fn old_caches_do_not_restore_duplicate_or_foreign_memberships() {
        let mut inv = fixture();
        let item = Uuid::from_u128(723);
        let folder = inv.folders.get_mut(&Uuid::from_u128(704)).expect("outfit");
        folder.info.favorite = true;
        folder.info.thumbnail = Uuid::from_u128(999);
        inv.folders
            .get_mut(&Uuid::from_u128(704))
            .expect("outfit")
            .items
            .extend([item, item]);
        let path = std::env::temp_dir().join(format!("aurora-inventory-{}.bin", Uuid::new_v4()));
        inv.save_cache(&path);
        let mut restored = fixture();
        for folder in restored.folders.values_mut() {
            folder.items.clear();
            folder.state = FetchState::Unknown;
        }
        restored.items.clear();
        let cache = read_cache(&path).expect("synthetic cache");
        assert!(restored.apply_cache(cache) > 0);
        assert!(restored.folders[&Uuid::from_u128(704)].info.favorite);
        assert_eq!(restored.folders[&Uuid::from_u128(704)].info.thumbnail, Uuid::from_u128(999));
        std::fs::remove_file(path).expect("synthetic cache cleanup");
        assert!(!restored.folders[&Uuid::from_u128(704)].items.contains(&item));
        assert_eq!(restored.folders[&restored.root].items.iter().filter(|id| **id == item).count(), 1);
    }

    /// The fixture as a login skeleton gives it: folders known, nothing fetched.
    fn skeleton() -> Inventory {
        let mut inv = fixture();
        for folder in inv.folders.values_mut() {
            folder.items.clear();
            folder.state = FetchState::Unknown;
        }
        inv.items.clear();
        inv
    }

    fn saved(inv: &Inventory) -> CachedInventory {
        let path = std::env::temp_dir().join(format!("aurora-inventory-{}.bin", Uuid::new_v4()));
        inv.save_cache(&path);
        let cache = read_cache(&path).expect("synthetic cache");
        std::fs::remove_file(path).expect("synthetic cache cleanup");
        cache
    }

    #[test]
    fn a_cache_restores_each_folder_in_display_order() {
        let mut inv = fixture();
        inv.sort_all();
        let fetched: Vec<Uuid> = inv
            .folders
            .values()
            .filter(|f| f.state == FetchState::Fetched)
            .map(|f| f.info.id)
            .collect();
        assert!(
            fetched.iter().any(|id| inv.folders[id].items.len() > 1),
            "fixture without a list to order"
        );
        let mut restored = skeleton();
        assert_eq!(restored.apply_cache(saved(&inv)), fetched.len());
        for id in fetched {
            assert_eq!(restored.folders[&id].state, FetchState::Fetched);
            assert_eq!(restored.folders[&id].items, inv.folders[&id].items);
        }
    }

    #[test]
    fn requests_wait_for_the_cache_and_skip_what_it_restored() {
        let cache = saved(&fixture());
        let mut restored = skeleton();
        // changed on the server since the cache was written
        let stale = Uuid::from_u128(704);
        restored.folders.get_mut(&stale).expect("outfit").info.version += 1;
        restored.hold_requests();
        let root = restored.root;
        for id in [root, stale, stale] {
            restored.request(id);
        }
        assert!(restored.queue.is_empty());
        assert_eq!(restored.folders[&stale].state, FetchState::Unknown);
        restored.apply_cache(cache);
        restored.release_requests();
        assert_eq!(restored.queue, vec![(stale, false)]);
        assert_eq!(restored.folders[&root].state, FetchState::Fetched);

        // no usable cache: the requests go out as they were made
        let mut bare = skeleton();
        bare.hold_requests();
        bare.request(root);
        bare.release_requests();
        assert_eq!(bare.queue, vec![(root, false)]);
    }

    #[test]
    fn what_arrived_during_the_read_is_newer_than_the_cache() {
        let inv = fixture();
        let cache = saved(&inv);
        let mut restored = skeleton();
        // an item of the root was moved to an outfit, and a folder fetched
        let (item, outfit, fetched) = (Uuid::from_u128(723), Uuid::from_u128(704), Uuid::from_u128(701));
        let mut moved = inv.items[&item].clone();
        assert_eq!(moved.parent, inv.root);
        moved.parent = outfit;
        moved.name = "moved".into();
        restored.add_items(vec![moved]);
        restored.folders.get_mut(&fetched).expect("current outfit").state = FetchState::Fetched;
        restored.apply_cache(cache);
        assert_eq!(restored.items[&item].name, "moved");
        assert!(!restored.folders[&restored.root].items.contains(&item));
        // the folder that received it is fetched, not restored without it
        assert_eq!(restored.folders[&outfit].state, FetchState::Unknown);
        assert!(restored.folders[&fetched].items.is_empty());
    }

    #[test]
    fn an_unreadable_cache_is_no_cache() {
        let path = std::env::temp_dir().join(format!("aurora-inventory-{}.bin", Uuid::new_v4()));
        assert!(read_cache(&path).is_none());
        let old = aurora_llsd::llsd_map! { "version" => 2, "folders" => Llsd::Array(Vec::new()), "items" => Llsd::Array(Vec::new()) };
        std::fs::write(&path, aurora_llsd::to_binary(&old)).expect("old inventory cache");
        assert!(
            read_cache(&path).is_none(),
            "reload legacy favorite metadata instead of restoring it"
        );
        std::fs::write(&path, b"not a cache").expect("synthetic cache");
        assert!(read_cache(&path).is_none());
        std::fs::remove_file(&path).expect("synthetic cache cleanup");
        assert!(read_cache_in_background(path).recv().expect("one answer").is_none());
    }
}
