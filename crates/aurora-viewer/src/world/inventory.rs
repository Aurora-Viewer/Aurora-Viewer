//! Client-side inventory model: login skeleton + lazily fetched folders.

use aurora_llsd::Llsd;
use aurora_net::inventory::{FolderContents, InvFolder, InvItem, parse_skeleton};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
#[cfg(test)]
#[path = "inventory/tests.rs"]
mod action_tests;
pub mod actions;
pub mod asset_ids;
pub mod background;
pub mod demo;
pub mod items;
mod lost;
pub(crate) mod outgoing;
mod parents;
pub mod preview;
mod system;
pub mod trash_origins;
pub mod undo;
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
    /// Root/system lookups retained until the inventory changes.
    pub(crate) system_folders: system::Cache,
    /// Full-permission assets retained until the inventory changes.
    pub(crate) full_assets: asset_ids::Cache,
    /// Session-only RLVa preview restrictions, shared by every inventory floater.
    pub rlva_preview_mask: u8,
    pub owner: Uuid,
    pub root: Uuid,
    pub lib_root: Uuid,
    pub lib_owner: Uuid,
    pub folders: HashMap<Uuid, Folder>,
    pub items: items::Items,
    pub(crate) outgoing: outgoing::Outgoing,
    /// Folders waiting to be fetched (id, library).
    pub queue: Vec<(Uuid, bool)>,
    /// Background reads cooling down. Automatic UI requests cannot bypass
    /// their retry owner; that owner releases them when the retry is due.
    pub(crate) retrying_folders: HashSet<Uuid>,
    /// Folders asked for while the cache file is being read (`hold_requests`).
    pub held: Option<Vec<Uuid>>,
    /// Bumped whenever folders or items change (cached views rebuild).
    pub generation: u64,
    /// Link originals the server answered do not exist: broken links, which outfits skip instead
    /// of waiting for them (LLViewerInventoryItem::getIsBrokenLink).
    pub missing_items: HashSet<Uuid>,
    /// When an unreadable original exhausted its current retry round.
    pub gave_up_at: HashMap<Uuid, std::time::Instant>,
    /// How many times each unreadable item was given up this session
    /// ([`ITEM_GIVE_UP_ROUNDS`]).
    pub give_up_rounds: HashMap<Uuid, u32>,
    /// Folders the server refuses even at depth 0: over its per-folder reply
    /// limit (the Legacy Viewer's InventoryLimitReachedAIS). Their direct
    /// items cannot be read; search and refresh do not ask again in a loop.
    pub too_big: HashSet<Uuid>,
    /// When each folder's last read failed (`shown_needs_fetch`).
    pub(crate) failed_at: HashMap<Uuid, std::time::Instant>,
    /// Failed item reads (network, server error), by attempt.
    pub item_failures: HashMap<Uuid, u8>,
    pub(crate) background_idle: bool,
    pub(crate) accepted_snapshot_items: Vec<Uuid>,
    /// Failed item reads to ask for again (`take_item_retries`).
    pub item_retries: HashSet<Uuid>,
    /// Items not in the inventory with a read on its way, and until when
    /// (`claim_item_reads`): one read such an item at a time, so a failure
    /// counts once. Keyed with whether it is a worn attachment's read,
    /// whose answer is filed differently (`AttachmentInventoryItems`).
    pub items_reading: HashMap<(Uuid, bool), std::time::Instant>,
    /// The listing folders the Marketplace data holds in place, set by the
    /// listings every frame (`marketplace::Holds`).
    pub listing_holds: super::marketplace::Holds,
    pub(crate) repaired_parents: HashMap<Uuid, Uuid>,
    pub(crate) pending_parent_repairs: HashSet<Uuid>,
    pub(crate) deferred_parents: HashMap<Uuid, parents::ItemSource>,
    pub(crate) parent_retry_generation: u64,
    pub(crate) repaired_folder_parents: HashMap<Uuid, Uuid>,
    pub(crate) pending_folder_repairs: HashSet<Uuid>,
    pub(crate) hierarchy_repaired: bool,
    pub(crate) deferred_lost_folders: bool,
    pub(crate) lost_folder_retry_generation: u64,
    /// Undo and redo of the session's inventory changes, shared by every
    /// inventory window (`undo`); a new login starts it empty.
    pub undo: undo::History,
    /// The folder each entry of the Trash came from (`trash_origins`),
    /// loaded for the account at login.
    pub trash_origins: trash_origins::TrashOrigins,
}

/// Failed read results before the automatic producer's retry budget ends.
/// Exhaustion does not establish that the original was deleted.
pub const ITEM_FETCH_ATTEMPTS: u8 = 3;
/// An item read unanswered this long no longer stops another read of it,
/// for each chunk of the read before the item's own: the chunks go one
/// after the other (session.rs `fetch_items`), each answered within its
/// timeout (AIS: AISAPI::HTTP_TIMEOUT, 180 s; FetchInventory2: 300 s), so
/// a claim outlasts its read and a failure counts once.
pub const ITEM_READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(320);
/// An item unreadable after its attempts (never answered missing) is read
/// again after this pause.
pub const ITEM_GIVE_UP: std::time::Duration = std::time::Duration::from_secs(600);
/// How many times a session an unreadable item is given up and then read
/// again; after the last, it stays a broken link for the session.
pub const ITEM_GIVE_UP_ROUNDS: u32 = 6;

/// The pause before a folder shown open whose read failed is asked for
/// again (`Inventory::shown_needs_fetch`).
pub const SHOWN_RETRY: std::time::Duration = std::time::Duration::from_secs(10);

// v3 could store a no-copy original as removed even when the offer never
// reached a simulator. The folder version alone cannot distinguish that cache.
const CACHE_VERSION: i32 = 4;

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
        self.repaired_parents.remove(&id);
        self.pending_parent_repairs.remove(&id);
        self.deferred_parents.remove(&id);
        if let Some(item) = self.items.remove(&id) {
            if let Some(folder) = self.folders.get_mut(&item.parent) {
                folder.items.retain(|other| *other != id);
            }
            self.generation += 1;
        }
    }
    pub fn from_login(raw: &Llsd) -> Inventory {
        let mut inv = Inventory {
            owner: raw["agent_id"].as_uuid(),
            root: first_id(&raw["inventory-root"], "folder_id"),
            lib_root: first_id(&raw["inventory-lib-root"], "folder_id"),
            lib_owner: first_id(&raw["inventory-lib-owner"], "agent_id"),
            ..Default::default()
        };
        inv.add_folders(parse_skeleton(&raw["inventory-skeleton"]), false);
        inv.add_folders(parse_skeleton(&raw["inventory-skel-lib"]), true);
        inv
    }

    pub(crate) fn add_folders(&mut self, list: Vec<InvFolder>, library: bool) {
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
                // Neither can a skeleton, a legacy listing or a UDP update
                // say the thumbnail is gone: they do not carry it (#2716).
                if !f.thumbnail_known && f.thumbnail.is_nil() {
                    f.thumbnail = existing.info.thumbnail;
                    f.thumbnail_known = existing.info.thumbnail_known;
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
            (!sys, f.map(|f| crate::natural_order::key(&f.info.name)).unwrap_or_default())
        });
        let mut items = f.items.clone();
        seen.clear();
        // FetchInventoryDescendents2 may include linked originals from other
        // folders. LLInventoryModel::updateItem indexes each by its parent.
        items.retain(|item| self.items.get(item).is_some_and(|it| it.parent == id) && seen.insert(*item));
        items.sort_by_cached_key(|i| self.items.get(i).map(|it| crate::natural_order::key(&it.name)).unwrap_or_default());
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

    /// Folder requests wait for the cache file to be read.
    pub fn requests_held(&self) -> bool {
        self.held.is_some()
    }

    /// Automatic demand: failures are owned by recovery, not by frame-by-frame UI callers.
    pub fn request(&mut self, id: Uuid) {
        if self.retrying_folders.contains(&id) || self.too_big.contains(&id) {
            return;
        }
        if let Some(held) = &mut self.held {
            if !held.contains(&id) {
                held.push(id);
            }
            return;
        }
        if let Some(f) = self.folders.get_mut(&id)
            && f.state == FetchState::Unknown
        {
            f.state = FetchState::Fetching;
            self.queue.push((id, f.library));
            // (a folder changed state: the background walk's idle sweep
            // looks again and times this read, even when its pump did not
            // run before the app took the queue, e.g. while the region has
            // no capabilities yet)
            self.generation += 1;
        }
    }

    /// Set folders back to unread, to be read again. Changes the
    /// generation when one changes, so the background walk's idle sweep
    /// looks at them again, like a read the user asks for
    /// ([`Self::request`]).
    pub fn mark_unread(&mut self, ids: impl IntoIterator<Item = Uuid>) {
        let mut changed = false;
        for id in ids {
            if let Some(f) = self.folders.get_mut(&id)
                && f.state != FetchState::Unknown
            {
                f.state = FetchState::Unknown;
                changed = true;
            }
        }
        if changed {
            self.generation += 1;
        }
    }

    /// An explicit recovery request (e.g. new region capabilities). It may
    /// re-arm a failed folder, but cannot steal an existing retry owner.
    pub fn retry(&mut self, id: Uuid) {
        if self.retrying_folders.contains(&id) || self.too_big.contains(&id) {
            return;
        }
        if let Some(folder) = self.folders.get_mut(&id)
            && folder.state == FetchState::Failed
        {
            folder.state = FetchState::Unknown;
        }
        self.request(id);
    }

    /// A deliberate open can take over a failed background read. Leave the
    /// retry owner in place so it can reconcile the interactive result.
    pub fn open_folder(&mut self, id: Uuid) {
        if self.too_big.contains(&id) || self.requests_held() {
            self.request(id);
            return;
        }
        if let Some(folder) = self.folders.get_mut(&id)
            && matches!(folder.state, FetchState::Unknown | FetchState::Failed)
        {
            folder.state = FetchState::Fetching;
            self.queue.push((id, folder.library));
            self.generation += 1;
        }
    }

    /// A shown folder keeps main's paced retry without bypassing background
    /// ownership. Other automatic producers still only request unknown folders.
    pub fn request_shown(&mut self, id: Uuid, now: std::time::Instant) {
        if self.shown_needs_fetch(id, now) {
            self.retry(id);
        }
    }

    /// Whether a folder shown open should be asked for now: one never read,
    /// or one whose last read failed `SHOWN_RETRY` ago or more. The windows
    /// ask every frame; without the pause, a folder the server is busy with
    /// (a large link creation still running) or refuses at once was asked
    /// again and again, and stayed "loading" for good.
    pub fn shown_needs_fetch(&self, id: Uuid, now: std::time::Instant) -> bool {
        if self.retrying_folders.contains(&id) || self.too_big.contains(&id) {
            return false;
        }
        match self.folders.get(&id).map(|f| f.state) {
            Some(FetchState::Unknown) => true,
            Some(FetchState::Failed) => self.failed_at.get(&id).is_none_or(|at| now.duration_since(*at) >= SHOWN_RETRY),
            _ => false,
        }
    }

    /// Apply folder snapshots. A snapshot older than the contents already
    /// confirmed locally is not applied: the folder is queued again, like
    /// AISUpdate::parseCategory's stale-folder refetch (llaisapi.cpp).
    /// Returns the folders that were stale and queued.
    pub fn apply(&mut self, contents: Vec<FolderContents>) -> Vec<Uuid> {
        self.apply_snapshots(contents, false)
    }

    pub(super) fn apply_snapshots(&mut self, contents: Vec<FolderContents>, acknowledge: bool) -> Vec<Uuid> {
        if acknowledge {
            self.accepted_snapshot_items.clear();
        }
        self.generation += 1;
        let mut refreshed: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
        let mut stale = Vec::new();
        for mut c in contents {
            // Inventory versions advance as copies arrive (Firestorm's
            // LLInventoryModel::accountForUpdate). A delayed pre-copy HTTP
            // snapshot must not replace the newer, populated folder.
            if self
                .folders
                .get(&c.folder_id)
                .is_some_and(|f| f.state == FetchState::Fetched && c.version < f.info.version)
            {
                stale.push(c.folder_id);
                continue;
            }
            let library = self.folders.get(&c.folder_id).map(|f| f.library).unwrap_or(false);
            let child_ids: Vec<Uuid> = c.folders.iter().map(|f| f.id).collect();
            for mut f in c.folders {
                if let Some(parent) = self.repaired_folder_parents.get(&f.id).copied() {
                    if f.parent == parent && !self.pending_folder_repairs.contains(&f.id) {
                        self.repaired_folder_parents.remove(&f.id);
                    } else {
                        f.parent = parent;
                    }
                }
                self.upsert_folder(f, library);
            }
            let mut child_ids = child_ids;
            child_ids.extend(
                self.repaired_folder_parents
                    .iter()
                    .filter_map(|(id, parent)| (*parent == c.folder_id).then_some(*id)),
            );
            c.items.retain(|item| {
                item.parent.is_nil()
                    || item.parent == parents::LOST_MARKER
                    || self.repaired_parents.get(&item.id).is_none_or(|parent| *parent == item.parent)
            });
            let mut item_ids: Vec<Uuid> = c.items.iter().map(|i| i.id).collect();
            for item in &c.items {
                if self.repaired_parents.get(&item.id) == Some(&item.parent) && !self.pending_parent_repairs.contains(&item.id) {
                    self.repaired_parents.remove(&item.id);
                    self.pending_parent_repairs.remove(&item.id);
                }
            }
            // A parent read started before its reliable repair can still
            // omit the item. Retain only those explicitly pending repairs.
            item_ids.extend(self.repaired_parents.iter().filter_map(|(item, parent)| {
                (*parent == c.folder_id && self.items.get(item).is_some_and(|it| it.parent == *parent)).then_some(*item)
            }));
            if c.version >= 0 {
                refreshed.insert(c.folder_id, item_ids.iter().copied().collect());
            }
            if let Some(f) = self.folders.get_mut(&c.folder_id) {
                f.children = child_ids;
                f.items = item_ids;
                f.state = FetchState::Fetched;
                f.info.version = c.version;
            }
            self.retrying_folders.remove(&c.folder_id);
            // A folder that fits again (the resident split it, or a later read
            // succeeded) is no longer over the limit.
            self.too_big.remove(&c.folder_id);
            self.repair_item_parents(&mut c.items, parents::ItemSource::Snapshot);
            if acknowledge && !library {
                self.accepted_snapshot_items.extend(c.items.iter().map(|item| item.id));
                for item in &c.items {
                    self.items_reading.remove(&(item.id, false));
                }
            }
            self.add_items_inner(c.items);
            self.sort_children(c.folder_id);
        }
        if !refreshed.is_empty() {
            // AIS SlamFolder replaces link UUIDs. Like AISUpdate::doUpdate's
            // onObjectDeletedFromServer (indra/newview/llaisapi.cpp), retire
            // the old records as well as folder membership: inventory views
            // index items by parent, so forgotten links stay visible (#1515).
            // Reconcile once per batch, including links already absent from
            // the child lists; originals and other folders remain untouched.
            // The links come from the parent index, not a scan of every
            // record: ~6 ms per readback with 220,000 items (#1963).
            for (folder, listed) in &refreshed {
                for id in self.items.links_in(*folder) {
                    if !listed.contains(&id) {
                        self.items.remove(&id);
                    }
                }
            }
            for id in refreshed.keys() {
                if let Some(folder) = self.folders.get_mut(id) {
                    folder.items.retain(|item| self.items.contains_key(item));
                }
            }
            self.repaired_parents.retain(|id, _| self.items.contains_key(id));
            self.pending_parent_repairs.retain(|id| self.repaired_parents.contains_key(id));
            self.deferred_parents.retain(|id, _| self.items.contains_key(id));
        }
        for id in &stale {
            if let Some(folder) = self.folders.get_mut(id) {
                folder.state = FetchState::Unknown;
            }
            self.request(*id);
        }
        stale
    }

    /// Items fetched by id (FetchInventory2), listed in their folder when
    /// that folder is loaded.
    ///
    /// A moved or renamed item is taken out of its old place and inserted at
    /// its sorted place: a folder is not sorted again for each item, which
    /// cost ~1 ms per move into a 5,000-item Trash, twice per moved item
    /// (#1580). Lists stay ordered like `sort_children`'s.
    pub fn add_items(&mut self, mut items: Vec<InvItem>) {
        self.repair_item_parents(&mut items, parents::ItemSource::Fetch);
        self.add_items_inner(items);
    }

    /// A metadata read recovering a newly script-attached original retains
    /// LLInventoryModel::updateItem's creation rules across HTTP delays.
    pub fn add_attachment_items(&mut self, mut items: Vec<InvItem>) {
        self.repair_item_parents(&mut items, parents::ItemSource::Attachment);
        self.add_items_inner(items);
    }

    pub fn add_orphan_items(&mut self, mut items: Vec<InvItem>) {
        self.repair_item_parents(&mut items, parents::ItemSource::Orphans);
        self.add_items_inner(items);
    }

    fn add_items_inner(&mut self, items: Vec<InvItem>) {
        self.generation += 1;
        for mut it in items {
            self.item_failures.remove(&it.id);
            self.item_retries.remove(&it.id);
            self.missing_items.remove(&it.id);
            self.gave_up_at.remove(&it.id);
            self.give_up_rounds.remove(&it.id);
            self.preserve_favorite(&mut it);
            calling_card_target(&mut it);
            if !it.parent.is_nil() && self.repaired_parents.get(&it.id).is_some_and(|parent| *parent != it.parent) {
                self.repaired_parents.remove(&it.id);
                self.pending_parent_repairs.remove(&it.id);
            }
            if let Some(old) = self.items.get(&it.id)
                && (old.parent != it.parent || old.name != it.name)
                && let Some(folder) = self.folders.get_mut(&old.parent)
            {
                folder.items.retain(|id| *id != it.id);
            }
            let items = &self.items;
            if let Some(f) = self.folders.get_mut(&it.parent)
                && (f.state == FetchState::Fetched || self.repaired_parents.get(&it.id) == Some(&it.parent))
                && !f.items.contains(&it.id)
            {
                let key = it.name.to_lowercase();
                let at = f
                    .items
                    .partition_point(|other| items.get(other).is_some_and(|o| o.name.to_lowercase() <= key));
                f.items.insert(at, it.id);
            }
            self.items.insert(it.id, it);
        }
    }

    /// A BulkUpdateInventory (a delivery, a gift, an unpack): applied like
    /// any server change, then each of its folders not read yet is read
    /// (#3025). The Legacy Viewer dates a folder by its loaded children
    /// (llfolderviewmodelinventory.cpp) and, in
    /// LLInventoryModel::processBulkUpdateInventory, only fetches a known
    /// folder whose version is unknown or a new folder's parent, so a
    /// delivered folder waits for an expand there before it sorts by date.
    /// Aurora reads the delivered folder at once so that it sorts with its
    /// contents: one FetchInventoryDescendents2 per delivered folder, a
    /// small and deliberate difference in what is sent. Folders created,
    /// moved or recovered through `server_updated` are not read.
    pub fn bulk_updated(&mut self, folders: Vec<InvFolder>, items: Vec<InvItem>) {
        let delivered: Vec<Uuid> = folders.iter().map(|folder| folder.id).collect();
        self.server_updated(folders, items);
        for id in delivered {
            if self.folders.get(&id).is_some_and(|f| !matches!(f.state, FetchState::Fetched)) {
                self.request(id);
            }
        }
    }

    /// LLInventoryModel::accountForUpdate / updateItem / updateCategory:
    /// apply server changes immediately, and advance each loaded parent's
    /// version once per update. Repeated records do not advance it again.
    pub fn server_updated(&mut self, folders: Vec<InvFolder>, mut items: Vec<InvItem>) {
        items.retain(|item| !item.id.is_nil());
        for item in &mut items {
            self.preserve_favorite(item);
            // before the comparison below: a card's record is otherwise
            // always "changed"
            calling_card_target(item);
        }
        let mut changed = HashSet::new();
        // Folders whose subfolders changed, sorted again below; moved items
        // take their sorted place in `add_items`.
        let mut resort = HashSet::new();
        for folder in folders {
            let id = folder.id;
            let parent = folder.parent;
            if self.repaired_folder_parents.get(&id).is_some_and(|known| *known != parent) {
                self.repaired_folder_parents.remove(&id);
                self.pending_folder_repairs.remove(&id);
            }
            if let Some(old) = self.folders.get(&id) {
                if old.info.parent != parent || old.info.name != folder.name {
                    changed.insert(old.info.parent);
                    changed.insert(parent);
                    resort.extend([old.info.parent, parent]);
                }
                let old_parent = old.info.parent;
                if old_parent != parent
                    && let Some(old) = self.folders.get_mut(&old_parent)
                {
                    old.children.retain(|child| *child != id);
                }
            } else {
                changed.insert(parent);
                resort.insert(parent);
            }
            self.upsert_folder(folder, false);
            if let Some(parent) = self.folders.get_mut(&parent)
                && !parent.children.contains(&id)
            {
                parent.children.push(id);
            }
        }
        self.repair_item_parents(&mut items, parents::ItemSource::Simulator);
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
        self.add_items_inner(items);
        for id in resort {
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

    pub fn take_accepted_snapshot_items(&mut self) -> Vec<Uuid> {
        std::mem::take(&mut self.accepted_snapshot_items)
    }

    /// Skip confirmed missing originals and unreadable originals that
    /// exhausted this retry round. The latter may resume after
    /// [`ITEM_GIVE_UP`] and are never classified as broken.
    pub fn drop_broken(&mut self, items: &mut Vec<Uuid>) {
        self.expire_given_up(std::time::Instant::now());
        items.retain(|id| !self.broken(id) && !self.read_exhausted(id));
    }

    /// The items given up past [`ITEM_GIVE_UP`] are no longer broken links:
    /// the outfits and attachments that need them ask again (the
    /// generation changes, so their views look again). Called every frame
    /// by the inventory's pump; nothing to do most of the time.
    pub fn expire_given_up(&mut self, now: std::time::Instant) {
        if self.gave_up_at.is_empty() {
            return;
        }
        let expired: Vec<Uuid> = self
            .gave_up_at
            .iter()
            .filter(|(_, at)| now.saturating_duration_since(**at) >= ITEM_GIVE_UP)
            .map(|(id, _)| *id)
            .collect();
        if expired.is_empty() {
            return;
        }
        for id in expired {
            self.gave_up_at.remove(&id);
            *self.give_up_rounds.entry(id).or_insert(0) += 1;
            self.item_failures.remove(&id);
            if !self.broken(&id) && !self.items.contains_key(&id) {
                self.item_retries.insert(id);
            }
        }
        self.generation += 1;
    }

    /// Leaves out of an item read the broken links' originals and the
    /// items not in the inventory already on their way (asked again by
    /// another window or the outfit code before the first answer came: on a
    /// huge inventory the same dead link was read twenty times at once, each
    /// failure counted as an attempt), and marks those as on their way. An
    /// item already in the inventory is a refresh (a script saved, a gesture
    /// or an AO item changed): it is always sent, as on main (two such reads
    /// are sent apart, so their replies may land in either order, as there).
    /// A read never answered frees its item after [`ITEM_READ_DEADLINE`]
    /// for each chunk up to its own.
    pub fn claim_item_reads(&mut self, items: &mut Vec<Uuid>, attachment: bool, now: std::time::Instant) {
        self.drop_broken(items);
        let mut seen = HashSet::new();
        items.retain(|id| seen.insert(*id) && (self.items.contains_key(id) || !self.item_reading(*id, attachment, now)));
        let chunk = aurora_net::inventory::ais::ITEM_CHUNK;
        for (index, id) in items.iter().enumerate() {
            if !self.items.contains_key(id) {
                let chunks = u32::try_from(index / chunk + 1).unwrap_or(u32::MAX);
                self.items_reading
                    .insert((*id, attachment), now + ITEM_READ_DEADLINE.saturating_mul(chunks));
            }
        }
    }

    /// A read of this item is on its way (and not past its deadline).
    pub fn item_reading(&self, id: Uuid, attachment: bool, now: std::time::Instant) -> bool {
        self.items_reading.get(&(id, attachment)).is_some_and(|until| now < *until)
    }

    /// These items' reads of this kind answered (or failed): they may be
    /// asked again. The other kind's claim stays: its read is its own.
    pub fn items_answered(&mut self, ids: &[Uuid], attachment: bool) {
        for id in ids {
            self.items_reading.remove(&(*id, attachment));
        }
    }

    /// Links' originals unreadable after their attempts, not answered gone
    /// (Dev Toolbox N4); the confirmed-gone ones are counted apart.
    pub fn unreadable_items(&self) -> usize {
        self.item_failures.keys().filter(|id| self.read_exhausted(id)).count()
    }

    /// A broken link's original: confirmed gone, and not received since.
    pub fn broken(&self, id: &Uuid) -> bool {
        self.missing_items.contains(id) && !self.items.contains_key(id)
    }

    pub(crate) fn set_background_idle(&mut self, idle: bool) {
        if self.background_idle != idle {
            self.background_idle = idle;
            self.generation += 1;
        }
    }

    /// Only a wear may skip an exhausted original. Other consumers still
    /// distinguish unknown metadata from a confirmed missing record.
    pub(crate) fn can_skip_wear_original(&self, id: &Uuid) -> bool {
        self.background_idle && self.read_exhausted(id)
    }

    pub(super) fn read_exhausted(&self, id: &Uuid) -> bool {
        !self.items.contains_key(id) && self.item_failures.get(id).is_some_and(|attempts| *attempts >= ITEM_FETCH_ATTEMPTS)
    }

    /// The server answered that these items do not exist.
    pub fn items_missing(&mut self, ids: Vec<Uuid>) {
        self.generation += 1;
        for id in ids {
            if self.items.contains_key(&id) {
                continue;
            }
            self.item_failures.remove(&id);
            self.item_retries.remove(&id);
            self.gave_up_at.remove(&id);
            self.give_up_rounds.remove(&id);
            self.missing_items.insert(id);
        }
    }

    /// These item reads failed: asked again, up to `ITEM_FETCH_ATTEMPTS`.
    pub fn items_failed(&mut self, ids: &[Uuid]) {
        self.generation += 1;
        let mut exhausted = 0;
        for id in ids {
            if self.items.contains_key(id) || self.broken(id) || self.read_exhausted(id) {
                continue;
            }
            let attempts = self.item_failures.entry(*id).or_insert(0);
            *attempts = attempts.saturating_add(1).min(ITEM_FETCH_ATTEMPTS);
            if *attempts >= ITEM_FETCH_ATTEMPTS {
                exhausted += 1;
                self.item_retries.remove(id);
                // Retry rounds preserve the distinction between unreadable
                // and confirmed missing, even after their budget is used.
                if self.give_up_rounds.get(id).copied().unwrap_or(0) < ITEM_GIVE_UP_ROUNDS {
                    self.gave_up_at.insert(*id, std::time::Instant::now());
                }
            } else {
                self.item_retries.insert(*id);
            }
        }
        if exhausted > 0 {
            log::debug!("{exhausted} inventory originals still unreadable after {ITEM_FETCH_ATTEMPTS} attempts");
        }
    }

    /// The failed item reads to send again, once each.
    pub fn take_item_retries(&mut self) -> HashSet<Uuid> {
        std::mem::take(&mut self.item_retries)
    }

    pub fn failed(&mut self, folders: &[Uuid]) {
        self.failed_at_time(folders, std::time::Instant::now());
    }

    fn failed_at_time(&mut self, folders: &[Uuid], now: std::time::Instant) {
        self.generation += 1;
        for id in folders {
            if let Some(f) = self.folders.get_mut(id)
                && f.state != FetchState::Fetched
            {
                f.state = FetchState::Failed;
                self.failed_at.insert(*id, now);
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
            self.retry(id);
        }
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Loaded items and folders, including both roots, the Library and links.
    /// Follows LLPanelMainInventory::updateItemcountText and
    /// LLInventoryModel::{getItemCount,getCategoryCount} (indra/newview).
    pub fn total_count(&self) -> usize {
        self.item_count() + self.folders.len()
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
        self.repaired_parents
            .retain(|id, parent| self.items.contains_key(id) && !removed.contains(parent));
        self.pending_parent_repairs.retain(|id| self.repaired_parents.contains_key(id));
        self.deferred_parents.retain(|id, _| self.items.contains_key(id));
        self.repaired_folder_parents
            .retain(|id, parent| !removed.contains(id) && !removed.contains(parent));
        self.pending_folder_repairs
            .retain(|id| self.repaired_folder_parents.contains_key(id));
        self.folders.retain(|id, _| !removed.contains(id));
        self.sort_all();
        self.generation += 1;
    }

    /// Write fetched folders and their items (LLInventoryModel::saveToFile).
    pub fn save_cache(&self, path: &std::path::Path) {
        use aurora_llsd::llsd_map;
        let mut folders = Vec::new();
        let mut items = Vec::new();
        let excluded = self.outgoing.uncached_folders(&self.items);
        for f in self
            .folders
            .values()
            .filter(|f| f.state == FetchState::Fetched && !excluded.contains(&f.info.id))
        {
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
        if folders.is_empty() && excluded.is_empty() {
            return;
        }
        let doc = llsd_map! {
            "version" => CACHE_VERSION,
            "folders" => Llsd::Array(folders),
            "items" => Llsd::Array(items),
        };
        // The document is the snapshot; its encoding, sealing and writing
        // go to a save thread (saves.rs): a large inventory on a slow disk
        // held the frame for over a minute at exit (#3466). The exit waits
        // for it with a bound, and the next login's read waits for it.
        let path = path.to_path_buf();
        crate::saves::replace(path.clone(), move || {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("tmp");
            if crate::cache::write(&tmp, &aurora_llsd::to_binary(&doc)).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        });
    }

    /// Restore cached folders whose version still matches the login
    /// skeleton (LLInventoryModel::loadSkeleton): they need no fetch. The
    /// file was read by [`read_cache`]; what is left here is cheap. What
    /// arrived while the file was being read is newer than the cache: an
    /// item already known is kept, and a folder that received one is left
    /// to the fetch.
    pub fn apply_cache(&mut self, cache: CachedInventory) -> usize {
        let touched: HashSet<Uuid> = self.items.values().map(|it| it.parent).collect();
        let excluded = self.outgoing.uncached_folders(&self.items);
        let mut valid = 0;
        for c in cache.folders {
            if excluded.contains(&c.id) {
                continue;
            }
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
            for mut it in c.items {
                // caches written before the cards' target was read
                calling_card_target(&mut it);
                if !self.items.contains_key(&it.id) {
                    listed.push(it.id);
                    self.items.insert(it.id, it);
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

/// LLInventoryModel::updateItem (indra/newview/llinventorymodel.cpp): a
/// calling card made by a viewer (create_inventory_callingcard, a resident
/// dragged into the inventory) is created by its owner and names its resident
/// by key in its description. That key becomes the card's creator, which
/// every calling card action reads: without it, opening the card showed our
/// own profile. The friendship cards made by the server have no key there.
fn calling_card_target(item: &mut InvItem) {
    // LLUUID::set accepts the 36-character hyphenated form
    if item.asset_type == 2
        && item.desc.len() == 36
        && let Ok(id) = Uuid::try_parse(&item.desc)
    {
        item.creator = id;
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
    read_cache_with_timeout(path, std::time::Duration::from_secs(10))
}

fn read_cache_with_timeout(path: &std::path::Path, timeout: std::time::Duration) -> Option<CachedInventory> {
    // A slow logout save may still be replacing a snapshot with different
    // no-copy originals. The login can fetch these folders from the server.
    if !crate::saves::wait_for(path, timeout) {
        return None;
    }
    let bytes = crate::cache::read_touch(path).ok()?;
    let (doc, _) = aurora_llsd::from_binary_local(&bytes).ok()?;
    if doc["version"].as_i32() != CACHE_VERSION {
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
            items.sort_by_cached_key(|it| crate::natural_order::key(&it.name));
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

    #[test]
    fn recovery_transient_item_failures_are_not_confirmation_of_a_broken_link() {
        let mut inv = Inventory::default();
        let item = Uuid::from_u128(2);
        for _ in 0..300 {
            inv.items_failed(&[item]);
        }
        assert!(!inv.broken(&item));
        assert!(!inv.missing_items.contains(&item));
        assert!(
            inv.take_item_retries().is_empty(),
            "retry budget is finite without declaring deletion"
        );
    }

    #[test]
    fn recovery_a_late_item_failure_does_not_override_a_successful_read() {
        let mut inv = Inventory::default();
        let id = Uuid::from_u128(2);
        inv.add_items(vec![InvItem {
            id,
            name: "Recovered".into(),
            ..Default::default()
        }]);
        inv.items_failed(&[id]);
        assert!(!inv.item_failures.contains_key(&id));
        assert!(inv.take_item_retries().is_empty());
    }

    #[test]
    fn inventory_total_counts_folders_library_trash_and_links() {
        let id = Uuid::from_u128;
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(2),
            ..Default::default()
        };
        assert_eq!(inv.total_count(), 0);
        for (n, parent, kind, library) in [
            (1, 0, 8, false),
            (2, 0, 8, true),
            (3, 1, 14, false),
            (4, 2, -1, true),
            (5, 1, -1, false),
        ] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent: id(parent),
                        type_default: kind,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state: FetchState::Unknown,
                    library,
                },
            );
        }
        inv.add_items(
            [
                (100, 1, 6, 0),
                (101, 4, 6, 0),
                (102, 1, 24, 100),
                (103, 1, 24, 999),
                (104, 1, 25, 5),
                (105, 3, 6, 0),
            ]
            .into_iter()
            .map(|(n, parent, asset_type, target)| InvItem {
                id: id(n),
                parent: id(parent),
                asset_type,
                asset_id: id(target),
                ..Default::default()
            })
            .collect(),
        );
        inv.items_missing(vec![id(999)]);
        assert_eq!(inv.item_count(), 6, "item-only consumers retain their count");
        assert_eq!(
            inv.total_count(),
            11,
            "five folders plus six records, including aliases and a broken link"
        );
        // Moving a record or receiving it again never counts it twice.
        let mut moved = inv.items[&id(100)].clone();
        moved.parent = id(5);
        inv.add_items(vec![moved.clone(), moved]);
        assert_eq!(inv.total_count(), 11);
        inv.remove(&[id(3)]);
        assert_eq!(inv.total_count(), 9, "removing Trash removes its category and contained item");
        let folder = InvFolder {
            id: id(6),
            parent: inv.root,
            ..Default::default()
        };
        inv.created_folders(vec![folder.clone(), folder]);
        assert_eq!(inv.total_count(), 10, "a new empty folder counts once");
    }

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
    fn folder_readback_removes_obsolete_links_only_after_an_accepted_snapshot() {
        let mut inv = fixture();
        let folder = Uuid::from_u128(704);
        let links = inv.folders[&folder].items.clone();
        let original = inv.items[&inv.items[&links[0]].asset_id].clone();
        let others = inv.folders[&Uuid::from_u128(701)].items.clone();
        let snapshot = FolderContents {
            folder_id: folder,
            owner_id: Uuid::from_u128(1),
            version: 10,
            folders: Vec::new(),
            items: links.iter().map(|id| inv.items[id].clone()).chain([original.clone()]).collect(),
        };
        inv.apply(vec![snapshot.clone()]);
        inv.apply(vec![FolderContents {
            version: 9,
            items: Vec::new(),
            ..snapshot.clone()
        }]);
        assert!(links.iter().all(|id| inv.items.contains_key(id)), "ignore older readback");
        inv.apply(vec![FolderContents {
            version: 11,
            items: Vec::new(),
            ..snapshot
        }]);
        assert!(
            links.iter().all(|id| !inv.items.contains_key(id)),
            "remove the obsolete link records"
        );
        assert!(inv.folders[&folder].items.is_empty());
        assert_eq!(
            inv.items[&original.id].parent, original.parent,
            "keep linked originals in their own folder"
        );
        assert!(
            others.iter().all(|id| inv.items.contains_key(id)),
            "keep other outfits' links to the same originals"
        );
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

    /// #3025: a folder delivered by BulkUpdate must be fetched so its date
    /// (newest descendant) sorts it with its contents, without waiting for expand.
    #[test]
    fn bulk_updated_folders_are_queued_for_fetch() {
        let mut inv = fixture();
        let id = Uuid::from_u128(9801);
        inv.bulk_updated(
            vec![InvFolder {
                id,
                parent: inv.root,
                name: "Gift folder".into(),
                type_default: -1,
                version: -1,
                ..Default::default()
            }],
            Vec::new(),
        );
        assert_eq!(inv.folders[&id].state, FetchState::Fetching);
        assert!(
            inv.queue.iter().any(|(queued, _)| *queued == id),
            "descendent fetch queued: {:?}",
            inv.queue
        );
    }

    /// Only a BulkUpdateInventory reads its folders: a folder AIS just
    /// created, one announced during a copy and a folder moved elsewhere
    /// send no FetchInventoryDescendents2, as in the Legacy Viewer.
    #[test]
    fn created_and_moved_folders_are_not_queued_for_fetch() {
        let mut inv = fixture();
        let folder = |id: u128, parent: Uuid| InvFolder {
            id: Uuid::from_u128(id),
            parent,
            name: format!("Folder {id}"),
            type_default: -1,
            version: 1,
            ..Default::default()
        };
        let root = inv.root;
        inv.created_folders(vec![folder(9811, root)]);
        assert!(inv.queue.is_empty(), "created folder: {:?}", inv.queue);
        assert_eq!(inv.folders[&Uuid::from_u128(9811)].state, FetchState::Fetched);

        // copied without AIS (announce_folder) and never opened
        inv.server_updated(vec![folder(9812, root)], Vec::new());
        assert!(inv.queue.is_empty(), "announced folder: {:?}", inv.queue);

        // moved under another folder
        let mut moved = folder(9812, Uuid::from_u128(9811));
        moved.version = -1;
        inv.server_updated(vec![moved], Vec::new());
        assert!(inv.queue.is_empty(), "moved folder: {:?}", inv.queue);
        assert!(inv.folders[&Uuid::from_u128(9811)].children.contains(&Uuid::from_u128(9812)));
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

    fn outfit(id: u128, thumbnail: u128, known: bool) -> InvFolder {
        InvFolder {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(48),
            name: "Outfit".into(),
            type_default: 47,
            version: 3,
            thumbnail: Uuid::from_u128(thumbnail),
            thumbnail_known: known,
            ..Default::default()
        }
    }

    /// A legacy listing of My Outfits, as saving or renaming an outfit reads
    /// it, says nothing of thumbnails: its children keep theirs (#2716).
    #[test]
    fn a_record_without_a_thumbnail_keeps_the_one_known() {
        let mut inv = Inventory::default();
        inv.add_folders(vec![outfit(1, 9, true)], false);
        let picture = Uuid::from_u128(9);
        inv.add_folders(vec![outfit(1, 0, false)], false);
        assert_eq!(inv.folders[&Uuid::from_u128(1)].info.thumbnail, picture);
        // and a new one replaces it, known or not
        inv.add_folders(vec![outfit(1, 8, false)], false);
        assert_eq!(inv.folders[&Uuid::from_u128(1)].info.thumbnail, Uuid::from_u128(8));
    }

    /// A complete AIS record, or the removal of the picture, does clear it.
    #[test]
    fn a_complete_record_without_a_thumbnail_clears_it() {
        let mut inv = Inventory::default();
        inv.add_folders(vec![outfit(1, 9, true)], false);
        inv.add_folders(vec![outfit(1, 0, true)], false);
        assert!(inv.folders[&Uuid::from_u128(1)].info.thumbnail.is_nil());
    }
}
