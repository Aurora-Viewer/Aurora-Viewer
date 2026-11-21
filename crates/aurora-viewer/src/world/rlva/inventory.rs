//! Shared inventory paths and wear locks, following RlvInventory,
//! RlvAttachmentLocks and RlvWearableLocks (rlvinventory.cpp, rlvlocks.cpp).

use super::super::{
    World, appearance,
    inventory::{Inventory, actions},
};
use super::Command;
use aurora_net::outfits::OutfitLink;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use uuid::Uuid;

/// The folders of every "…this" lock, and the folders of the links under
/// #RLV, by the item they point to.
#[derive(Default)]
pub(crate) struct LockedFolders {
    rules: HashMap<(Uuid, String), Vec<Uuid>>,
    links: HashMap<Uuid, Vec<Uuid>>,
}

/// The folders of every "…this" lock, shared by the checks of one pass (the
/// watchdog's attachments, a folder tree). Resolving them walks the shared
/// folder, so the result is kept across passes while what it reads stays
/// the same ([`LockInputs`]), as RlvFolderLocks keeps its lookups until
/// onNeedsLookupRefresh marks them dirty (rlvlocks.cpp).
#[derive(Default)]
pub(crate) struct FolderLocks(std::cell::OnceCell<Arc<LockedFolders>>);

impl FolderLocks {
    fn resolved(&self, world: &World) -> &LockedFolders {
        self.0.get_or_init(|| world.rlva_locked_folders())
    }

    fn folders(&self, world: &World, source: Uuid, option: &str) -> &[Uuid] {
        self.resolved(world)
            .rules
            .get(&(source, option.to_owned()))
            .map_or(&[], Vec::as_slice)
    }

    /// The (folded) folders of the links to `item` under #RLV.
    fn link_folders(&self, world: &World, item: Uuid) -> &[Uuid] {
        self.resolved(world).links.get(&item).map_or(&[], Vec::as_slice)
    }

    /// Every folder a "…this" lock or exception names.
    pub(super) fn roots(&self, world: &World) -> Vec<Uuid> {
        self.resolved(world).rules.values().flatten().copied().collect()
    }
}

/// What the lock checks read besides the time: the rules, the inventory,
/// the folded-folder naming, the worn attachments (item and point) and the
/// issuers resolved to their items. Equal inputs give equal answers.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct LockInputs {
    rules: u64,
    inventory: u64,
    legacy_naming: bool,
    worn: Vec<(Uuid, u8)>,
    sources: Vec<(Uuid, Uuid)>,
}

/// The last resolution of [`FolderLocks`] and its inputs.
#[derive(Default)]
pub(crate) struct LockedFoldersCache(pub(super) Mutex<Option<(LockInputs, Arc<LockedFolders>)>>);

impl World {
    pub(super) fn rlva_lock_inputs(&self, worn: impl IntoIterator<Item = (Uuid, u8)>) -> LockInputs {
        let mut worn: Vec<_> = worn.into_iter().collect();
        worn.sort_unstable();
        LockInputs {
            rules: self.rlva.revision,
            inventory: self.inventory.generation,
            legacy_naming: self.rlva.preferences.legacy_naming,
            worn,
            sources: self.rlva.watchdog.sources.iter().map(|(source, item)| (*source, *item)).collect(),
        }
    }

    fn rlva_locked_folders(&self) -> Arc<LockedFolders> {
        let inputs = self.rlva_lock_inputs(self.rlva_worn_attachment_points());
        let cache = &self.rlva.locked_folders.0;
        if let Some((cached, folders)) = cache.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            && *cached == inputs
        {
            return folders.clone();
        }
        let folders = Arc::new(LockedFolders {
            rules: self.rlva_context_folders_of(
                self.rlva
                    .rules
                    .iter()
                    .flat_map(|(source, rules)| rules.iter().map(move |r| (*source, r)))
                    .filter(|(_, r)| r.behaviour.contains("this"))
                    .map(|(source, r)| (source, r.option.as_str())),
            ),
            links: self.rlva_shared_link_folders(),
        });
        *cache.lock().unwrap_or_else(PoisonError::into_inner) = Some((inputs, folders.clone()));
        folders
    }

    /// Every link under #RLV, by the item it points to: the folded folders
    /// holding a link to it. The walk of RlvFolderLocks::getLockedItems
    /// (collectDescendentsIf from the shared root, LLLinkedItemIDMatches),
    /// done once for every item.
    fn rlva_shared_link_folders(&self) -> HashMap<Uuid, Vec<Uuid>> {
        let mut links = HashMap::<Uuid, Vec<Uuid>>::new();
        let Some(root) = shared_root(&self.inventory) else {
            return links;
        };
        let legacy = self.rlva.preferences.legacy_naming;
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(f) = self.inventory.folders.get(&id) else { continue };
            stack.extend(f.children.iter().copied());
            for it in f.items.iter().filter_map(|id| self.inventory.items.get(id)) {
                if it.asset_type == 24 && it.asset_id != it.id {
                    links
                        .entry(it.asset_id)
                        .or_default()
                        .push(folded_parent(&self.inventory, it.parent, legacy));
                }
            }
        }
        for folders in links.values_mut() {
            folders.sort_unstable();
            folders.dedup();
        }
        links
    }

    /// The folder remove locks of a worn item, `except` one issuer's:
    /// RlvFolderLocks::getLockedItems (rlvlocks.cpp) locks a worn item when
    /// the folder of the item itself is locked, or the folder of any of its
    /// links under #RLV. An item worn from a link in a locked folder stays on.
    /// Every check of a worn item asks here (taking it off, the watchdog,
    /// the locked HUDs, Release Keys, the presence queries), so that none
    /// forgets the links.
    ///
    /// Deliberately stricter than the Legacy Viewer: an item whose own
    /// record is not loaded yet is still held by its links under #RLV. The
    /// reference only walks the worn items its inventory holds, and would
    /// let such an item go until its folder arrives.
    pub(super) fn rlva_item_folders_removable_in(&self, locks: &FolderLocks, item: Uuid, except: Uuid) -> bool {
        if !self.rlva_has_folder_rule() {
            return true;
        }
        let own = self.inventory.items.get(&item).map(|it| it.parent);
        own.iter()
            .chain(locks.link_folders(self, item))
            .all(|folder| self.rlva_folder_signature_in(locks, *folder, false, except).is_empty())
    }
}

/// How long a wear stands for the entry it was asked from (`rlva_note_wear_entry`).
const WEAR_ENTRY_TIME: std::time::Duration = std::time::Duration::from_secs(60);

/// The forced batches queued at most (`rlva_inventory_force`).
const FORCE_QUEUE_LIMIT: usize = 4096;

pub const LAYERS: &[&str] = &[
    "shape",
    "skin",
    "hair",
    "eyes",
    "shirt",
    "pants",
    "shoes",
    "socks",
    "jacket",
    "gloves",
    "undershirt",
    "underpants",
    "skirt",
    "alpha",
    "tattoo",
    "physics",
    "universal",
];
// RlvHandler::onGetOutfit uses the historical protocol order, independent
// of LLWearableType's numeric order used by flags and the name queries.
const OUTFIT_ORDER: &[usize] = &[9, 8, 5, 4, 6, 12, 7, 11, 10, 1, 3, 2, 0, 13, 14, 15, 16];
const GROUPS: &[&str] = &["arms", "head", "legs", "torso", "hud"];

/// rlvAttachGroupFromIndex (rlvhelper.cpp), on the group of avatar_lad.xml:
/// its "hud" is group 8, which avatar_lad.xml gives the Bento points (tail,
/// wings, ring fingers, hind feet…); the HUD points (group 9) are in no
/// group, as in the reference.
fn group_name(group: u8) -> Option<&'static str> {
    Some(match group {
        0 | 1 | 3 | 4 => "arms",
        2 => "head",
        5 | 7 => "legs",
        6 => "torso",
        8 => "hud",
        _ => return None,
    })
}
pub const POINTS: &[&str] = &[
    "",
    "chest",
    "skull",
    "left shoulder",
    "right shoulder",
    "left hand",
    "right hand",
    "left foot",
    "right foot",
    "spine",
    "pelvis",
    "mouth",
    "chin",
    "left ear",
    "right ear",
    "left eyeball",
    "right eyeball",
    "nose",
    "r upper arm",
    "r forearm",
    "l upper arm",
    "l forearm",
    "right hip",
    "r upper leg",
    "r lower leg",
    "left hip",
    "l upper leg",
    "l lower leg",
    "stomach",
    "left pec",
    "right pec",
    "center 2",
    "top right",
    "top",
    "top left",
    "center",
    "bottom left",
    "bottom",
    "bottom right",
    "neck",
    "avatar center",
    "left ring finger",
    "right ring finger",
    "tail base",
    "tail tip",
    "left wing",
    "right wing",
    "jaw",
    "alt left ear",
    "alt right ear",
    "alt left eye",
    "alt right eye",
    "tongue",
    "groin",
    "left hind foot",
    "right hind foot",
];

pub fn point(name: &str) -> Option<u8> {
    if name.eq_ignore_ascii_case("root") {
        return Some(40);
    }
    POINTS
        .iter()
        .position(|p| !p.is_empty() && p.eq_ignore_ascii_case(name))
        .map(|p| p as u8)
}
pub fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().position(|l| l.eq_ignore_ascii_case(name)).map(|l| l as u8)
}

pub(super) fn shared_root(inv: &Inventory) -> Option<Uuid> {
    // RlvInventory::getSharedRootID prefers the first populated #RLV root,
    // otherwise the last empty one, in native inventory child order.
    let sorted;
    let candidates = match inv.folders.get(&inv.root) {
        Some(f) => f.children.as_slice(),
        None => {
            let mut ids: Vec<_> = inv.folders.keys().copied().collect();
            ids.sort();
            sorted = ids;
            sorted.as_slice()
        }
    };
    let mut empty = None;
    for id in candidates.iter().copied() {
        if let Some(f) = inv
            .folders
            .get(&id)
            .filter(|f| !f.library && f.info.parent == inv.root && f.info.name == "#RLV")
        {
            if !f.children.is_empty() {
                return Some(id);
            }
            empty = Some(id);
        }
    }
    empty
}

fn visible(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains('/')
}

fn folded(name: &str, legacy: bool) -> bool {
    super::naming::folded(name, legacy)
}

/// The locks that go by where a folder is (under #RLV or not), whatever the
/// folder: they reach the whole inventory.
fn placement_rule(name: &str) -> bool {
    matches!(name, "sharedwear" | "sharedunwear" | "unsharedwear" | "unsharedunwear")
}

/// The rules that lock the inventory root (ST_ROOTFOLDER in RlvFolderLocks,
/// rlvlocks.cpp), with #RLV as their exception.
fn root_lock(name: &str) -> bool {
    matches!(name, "unsharedwear" | "unsharedunwear")
}

/// The rules `rlva_folder_signature_in` can match: the global shared and
/// unshared wear locks, and the `attach…this` / `detach…this` folder locks.
fn folder_rule(name: &str) -> bool {
    placement_rule(name) || ((name.starts_with("attach") || name.starts_with("detach")) && name.contains("this"))
}

fn folded_parent(inv: &Inventory, mut id: Uuid, legacy: bool) -> Uuid {
    let mut seen = HashSet::new();
    while seen.insert(id) {
        let Some(f) = inv.folders.get(&id).filter(|f| folded(&f.info.name, legacy)) else {
            break;
        };
        id = f.info.parent;
    }
    id
}

pub fn force_kind(name: &str) -> Option<&'static str> {
    if !super::FORCES.contains(&name) {
        return None;
    }
    if matches!(name, "detachme" | "remattach" | "remoutfit") {
        return Some(match name {
            "detachme" => "detachme",
            "remattach" => "remattach",
            _ => "remoutfit",
        });
    }
    if name.starts_with("detach") {
        return Some(if name.contains("all") { "detachall" } else { "detach" });
    }
    if !(name.starts_with("attach") || name.starts_with("addoutfit")) {
        return None;
    }
    Some(match (name.contains("all"), name.ends_with("over")) {
        (true, true) => "attachallover",
        (true, false) => "attachall",
        (false, true) => "attachover",
        _ => "attach",
    })
}

pub(super) fn path(inv: &Inventory, mut id: Uuid) -> Option<String> {
    let root = shared_root(inv)?;
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    while id != root && seen.insert(id) {
        let f = inv.folders.get(&id)?;
        names.push(f.info.name.clone());
        id = f.info.parent;
    }
    if id != root {
        return None;
    }
    names.reverse();
    Some(names.join("/"))
}

fn resolve(inv: &Inventory, name: &str) -> Option<Uuid> {
    let mut id = shared_root(inv)?;
    if name.is_empty() {
        return Some(id);
    }
    for component in name.split('/').filter(|v| !v.is_empty()) {
        let f = inv.folders.get(&id)?;
        id = *f
            .children
            .iter()
            .find(|child| inv.folders.get(child).is_some_and(|f| f.info.name.eq_ignore_ascii_case(component)))
            .or_else(|| {
                f.children.iter().find(|child| {
                    inv.folders
                        .get(child)
                        // boost::icontains (RlvInventory::getSharedFolder):
                        // the ASCII letters only (see `findfolder` below)
                        .is_some_and(|f| {
                            visible(&f.info.name) && f.info.name.to_ascii_lowercase().contains(&component.to_ascii_lowercase())
                        })
                })
            })?;
    }
    Some(id)
}

/// A path names a folder under #RLV (RlvInventory::getSharedFolder).
pub(crate) fn shared_folder_exists(inv: &Inventory, path: &str) -> bool {
    resolve(inv, path).is_some()
}

impl World {
    /// The entries of a folder force, with what is loaded: RlvForceWear::
    /// forceFolder collects from gInventory at once (rlvhelper.cpp). What is
    /// missing is fetched for the next command, as RlvInventory::
    /// fetchSharedInventory and fetchSharedLinks do at login.
    fn rlva_collect(&mut self, base: Uuid, all: bool, attach: bool, replace: bool) -> Vec<super::collection::Entry> {
        let collection = super::collection::collect(&self.inventory, base, all, attach, replace, &self.rlva.preferences);
        for id in collection.missing {
            if self.inventory.folders.contains_key(&id) {
                self.inventory.request(id);
            } else if self.rlva_inventory_fetch_due(id) {
                self.rlva.out.push(aurora_net::NetCommand::FetchInventory {
                    folders: vec![id],
                    owner: self.agent_id,
                    library: false,
                });
            }
        }
        for id in collection.missing_items {
            if self.rlva_inventory_fetch_due(id) {
                self.rlva.out.push(aurora_net::NetCommand::FetchItems {
                    items: vec![id],
                    owner: self.agent_id,
                });
            }
        }
        collection.items
    }

    pub(super) fn rlva_inventory_fetch_due(&mut self, id: Uuid) -> bool {
        let now = std::time::Instant::now();
        let entry = self.rlva.inventory_fetches.entry(id).or_insert((now, 0));
        if entry.1 < 3 && (entry.1 == 0 || now.saturating_duration_since(entry.0).as_secs() >= 10) {
            *entry = (now, entry.1 + 1);
            true
        } else {
            false
        }
    }

    fn rlva_attachment_group(&self, point: u8) -> Option<&'static str> {
        group_name(self.avatar_lib.attach_points.get(&point)?.group)
    }
    /// The folders of the "…this" locks of `keys` (issuer, option), sorted:
    /// RlvFolderLocks::getLockedFolders (rlvlocks.cpp). No option
    /// (ST_ATTACHMENT): the folder of the issuer's own item, when it is a
    /// shared folder other than #RLV, as it is (a folded folder's lock then
    /// reaches nothing, isLockedFolder folding the folder it is asked about).
    /// A point or a layer: RlvInventory::getPath of the items it names, one
    /// folded folder up. A path: the shared folder it names now.
    fn rlva_context_folders_of<'a>(&self, keys: impl IntoIterator<Item = (Uuid, &'a str)>) -> HashMap<(Uuid, String), Vec<Uuid>> {
        let root = shared_root(&self.inventory);
        let mut out = HashMap::new();
        for (source, option) in keys {
            let key = (source, option.to_owned());
            if out.contains_key(&key) {
                continue;
            }
            let mut folders: Vec<Uuid> = if option.is_empty() {
                // gObjectList.findObject(issuer)->getAttachmentItemID():
                // a child prim holds no item of its own (only the root gets
                // AttachItemID, LLVOAvatar::addChild), so its lock names no
                // folder. An issuer not loaded yet keeps its last known item.
                let item = match self.objects.index_of_uuid(&source).and_then(|i| self.objects.get(i)) {
                    Some(object) => object.attachment_item_id().filter(|id| !id.is_nil()),
                    None => self.rlva_attachment_item(source),
                };
                item.and_then(|item| self.inventory.items.get(&item))
                    .map(|it| it.parent)
                    .filter(|parent| root.is_some_and(|root| *parent != root && actions::under(&self.inventory, *parent, root)))
                    .into_iter()
                    .collect()
            } else if point(option).is_some() || layer(option).is_some() {
                self.rlva_path_folders(source, option)
            } else if super::numeric::uuid_set(option).is_some() {
                Vec::new()
            } else {
                resolve(&self.inventory, option).into_iter().collect()
            };
            folders.sort();
            folders.dedup();
            out.insert(key, folders);
        }
        out
    }

    /// RlvCommandOptionGetPath and RlvInventory::getPath use original COF
    /// item order and fold one containing folder. Folder-lock resolution is
    /// separate: a link outside #RLV must not manufacture a query path.
    fn rlva_path_folders(&self, source: Uuid, option: &str) -> Vec<Uuid> {
        let worn = self.rlva_worn_attachment_points();
        let cof = appearance::cof(&self.inventory)
            .map(|id| appearance::folder_links(&self.inventory, id))
            .unwrap_or_default();
        let items = if let Some(point) = point(option) {
            // RlvFindAttachmentsOnPoint (rlvinventory.cpp): an object worn
            // on the point, or one whose name or folder names it
            let legacy = self.rlva.preferences.legacy_naming;
            cof.iter()
                .filter(|link| {
                    worn.get(&link.target) == Some(&point)
                        || (self.inventory.items.get(&link.target).is_some_and(|it| it.asset_type == 6)
                            && super::collection::attachment_point(&self.inventory, link.target, legacy) == Some(point))
                })
                .map(|link| link.target)
                .collect::<Vec<_>>()
        } else if let Some(layer) = layer(option) {
            cof.iter()
                .filter(|link| {
                    self.inventory
                        .items
                        .get(&link.target)
                        .is_some_and(|item| matches!(item.asset_type, 5 | 13) && item.flags & 0xff == u32::from(layer))
                })
                .map(|link| link.target)
                .collect()
        } else if option.is_empty() || super::numeric::uuid_set(option).is_some() {
            let object = if option.is_empty() {
                source
            } else {
                super::numeric::uuid_set(option).unwrap_or_default()
            };
            self.rlva_own_attachment_root(object)
                .and_then(|root| self.rlva_attachment_item(root))
                .into_iter()
                .collect()
        } else {
            Vec::new()
        };
        let Some(root) = shared_root(&self.inventory) else {
            return Vec::new();
        };
        items
            .into_iter()
            .filter_map(|id| {
                let item = self.inventory.items.get(&id)?;
                if !actions::under(&self.inventory, item.parent, root) {
                    return None;
                }
                let mut parent = self.inventory.folders.get(&item.parent)?;
                if folded(&parent.info.name, self.rlva.preferences.legacy_naming) {
                    parent = self.inventory.folders.get(&parent.info.parent)?;
                }
                path(&self.inventory, parent.info.id).map(|_| parent.info.id)
            })
            .collect()
    }

    /// One folder check alone; the item checks go through
    /// `rlva_item_folders_removable_in`, which counts the item's links too.
    #[cfg(test)]
    pub(super) fn rlva_folder_allowed(&self, folder: Uuid, add: bool, actor: Uuid) -> bool {
        self.rlva_folder_allowed_in(&FolderLocks::default(), folder, add, actor)
    }

    pub(super) fn rlva_folder_allowed_in(&self, locks: &FolderLocks, folder: Uuid, add: bool, _actor: Uuid) -> bool {
        self.rlva_folder_signature_in(locks, folder, add, Uuid::nil()).is_empty()
    }

    fn rlva_folder_signature(&self, folder: Uuid, add: bool, _actor: Uuid) -> std::collections::BTreeSet<(Uuid, String, String)> {
        self.rlva_folder_signature_except(folder, add, Uuid::nil())
    }

    fn rlva_folder_signature_except(&self, folder: Uuid, add: bool, except: Uuid) -> std::collections::BTreeSet<(Uuid, String, String)> {
        self.rlva_folder_signature_in(&FolderLocks::default(), folder, add, except)
    }

    fn rlva_folder_signature_in(
        &self,
        locks: &FolderLocks,
        folder: Uuid,
        add: bool,
        except: Uuid,
    ) -> std::collections::BTreeSet<(Uuid, String, String)> {
        // The watchdog asks this for every worn attachment, twice a frame:
        // with no folder rule at all (RLVa on, nothing locked) the answer is
        // empty, without walking the folder's ancestors.
        if !self
            .rlva
            .rules
            .iter()
            .any(|(source, rules)| *source != except && rules.iter().any(|r| folder_rule(&r.behaviour)))
        {
            return Default::default();
        }
        let folder = folded_parent(&self.inventory, folder, self.rlva.preferences.legacy_naming);
        let mut ancestors = Vec::new();
        let mut id = folder;
        while !ancestors.contains(&id) {
            ancestors.push(id);
            let Some(f) = self.inventory.folders.get(&id) else { break };
            id = f.info.parent;
        }
        // Under #RLV exactly when `path` would name it, without building the
        // path: the walk above reaches the shared root, or stops before it.
        let shared = shared_root(&self.inventory).is_some_and(|root| ancestors.contains(&root));
        // RlvFolderLocks::isLockedFolder (rlvlocks.cpp) walks up from the
        // folder: the nearest rule of each issuer wins, and among the rules
        // of one folder the one issued first (m_FolderLocks keeps the order):
        // an exception skips that issuer's later rules, a lock found first
        // holds. An exception granted by one issuer cannot release another
        // issuer's subtree lock.
        let mut nearest = std::collections::BTreeMap::<Uuid, (usize, bool, Vec<(String, String)>)>::new();
        for (source, rules) in &self.rlva.rules {
            if *source == except {
                continue;
            }
            let ordered: Vec<&super::Rule> = match self.rlva.issued.get(source) {
                Some(issued) if issued.len() == rules.len() => issued.iter().collect(),
                _ => rules.iter().collect(),
            };
            for rule in ordered {
                let name = rule.behaviour.as_str();
                let global = name
                    == match (shared, add) {
                        (true, true) => "sharedwear",
                        (true, false) => "sharedunwear",
                        (false, true) => "unsharedwear",
                        _ => "unsharedunwear",
                    };
                if !global
                    && !(if add {
                        name.starts_with("attach")
                    } else {
                        name.starts_with("detach")
                    })
                {
                    continue;
                }
                if !global && !name.contains("this") {
                    continue;
                }
                let allow = name.ends_with("_except");
                let distance = if global {
                    Some(usize::MAX)
                } else {
                    locks
                        .folders(self, *source, &rule.option)
                        .iter()
                        .filter_map(|root| ancestors.iter().position(|id| id == root))
                        .filter(|d| *d == 0 || name.contains("all"))
                        .min()
                };
                if let Some(distance) = distance {
                    let denied = if allow {
                        Vec::new()
                    } else {
                        vec![(rule.behaviour.clone(), rule.option.clone())]
                    };
                    match nearest.entry(*source) {
                        std::collections::btree_map::Entry::Vacant(slot) => {
                            slot.insert((distance, allow, denied));
                        }
                        std::collections::btree_map::Entry::Occupied(mut slot) => {
                            let entry = slot.get_mut();
                            if distance < entry.0 {
                                *entry = (distance, allow, denied);
                            } else if distance == entry.0 && !entry.1 {
                                entry.2.extend(denied);
                            }
                        }
                    }
                }
            }
        }
        nearest
            .into_iter()
            .filter(|(_, (_, allow, _))| !allow)
            .flat_map(|(source, (_, _, rules))| rules.into_iter().map(move |(name, option)| (source, name, option)))
            .collect()
    }

    pub fn rlva_inventory_move_allowed(&self, item: Uuid, destination: Uuid, actor: Uuid) -> bool {
        if !self.rlva.enabled() || !self.rlva.inventory_locked() {
            return true;
        }
        let Some(parent) = self
            .inventory
            .items
            .get(&item)
            .map(|it| it.parent)
            .or_else(|| self.inventory.folders.get(&item).map(|f| f.info.parent))
        else {
            return false;
        };
        let folder = self.inventory.folders.contains_key(&item);
        let locks = FolderLocks::default();
        // RlvFolderLocks::canMoveItem (rlvlocks.h) compares every lock of
        // the two folders but the inventory root's (ST_ROOTFOLDER, set by
        // @unsharedwear / @unsharedunwear): @sharedwear and @sharedunwear
        // keep an item in #RLV or out of it. canMoveFolder counts them all.
        let signature = |id, add| {
            self.rlva_folder_signature_in(&locks, id, add, Uuid::nil())
                .into_iter()
                .filter(|(_, name, _)| folder || !root_lock(name))
                .collect::<std::collections::BTreeSet<_>>()
        };
        [false, true]
            .into_iter()
            .all(|add| signature(if folder { item } else { parent }, add) == signature(destination, add))
            && (!folder || !self.rlva_explicit_folder_descendant(&locks, item, actor))
    }

    fn rlva_explicit_folder_descendant(&self, locks: &FolderLocks, folder: Uuid, actor: Uuid) -> bool {
        self.rlva
            .rules
            .iter()
            .filter(|(source, _)| **source != actor)
            .any(|(source, rules)| {
                rules.iter().filter(|r| r.behaviour.contains("this")).any(|rule| {
                    locks.folders(self, *source, &rule.option).iter().copied().any(|mut root| {
                        let mut seen = HashSet::new();
                        while seen.insert(root) {
                            if root == folder {
                                return true;
                            }
                            let Some(f) = self.inventory.folders.get(&root) else { break };
                            root = f.info.parent;
                        }
                        false
                    })
                })
            })
    }

    fn rlva_inventory_tree_unlocked(&self, folder: Uuid, actor: Uuid) -> bool {
        let locks = FolderLocks::default();
        if self.rlva_explicit_folder_descendant(&locks, folder, actor) {
            return false;
        }
        let mut stack = vec![folder];
        let mut seen = HashSet::new();
        let worn = self.worn_attachment_items();
        let cof = appearance::cof(&self.inventory)
            .map(|id| appearance::folder_links(&self.inventory, id))
            .unwrap_or_default();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(f) = self.inventory.folders.get(&id) else {
                return false;
            };
            // RlvFolderLocks::canRemoveFolder (rlvlocks.h) counts every lock
            // but the inventory root's (ST_ROOTFOLDER): @sharedwear and
            // @sharedunwear, held on #RLV, keep it and its folders as well.
            if !appearance::loaded(&self.inventory, id)
                || [false, true].into_iter().any(|add| {
                    self.rlva_folder_signature_in(&locks, id, add, Uuid::nil())
                        .iter()
                        .any(|(_, name, _)| name.contains("this") || matches!(name.as_str(), "sharedwear" | "sharedunwear"))
                })
            {
                return false;
            }
            for id in &f.items {
                let target = super::super::inventory::actions::original(&self.inventory, *id).unwrap_or(*id);
                if (worn.contains(&target) || cof.iter().any(|l| l.target == target)) && !self.rlva_can_remove(target, actor) {
                    return false;
                }
            }
            stack.extend(f.children.iter().copied());
        }
        true
    }

    pub fn rlva_inventory_mutation_allowed(&self, change: &aurora_net::inventory::operations::Mutation, actor: Uuid) -> bool {
        use aurora_net::inventory::operations::Operation;
        if !self.rlva.enabled() || !self.rlva.inventory_locked() {
            return true;
        }
        change
            .copies
            .iter()
            .all(|copy| self.rlva_inventory_move_allowed(copy.item, copy.parent, actor))
            && change.operations.iter().all(|operation| match operation {
                Operation::Patch { id, folder, body } => {
                    let moved = body.as_map().is_some_and(|m| m.contains_key("parent_id"));
                    let renamed = body.as_map().is_some_and(|m| m.contains_key("name"));
                    // RlvFolderLocks::canRenameFolder (rlvlocks.h): #RLV
                    // carries the shared path lock of the four placement
                    // rules (their deny, or the exception of the unshared
                    // twins), so it keeps its name while one is set.
                    (!moved || self.rlva_inventory_move_allowed(*id, body["parent_id"].as_uuid(), actor))
                        && (!renamed
                            || !*folder
                            || (!self.rlva_explicit_folder_descendant(&FolderLocks::default(), *id, actor)
                                && !(Some(*id) == shared_root(&self.inventory) && self.rlva_has_placement_rule())))
                }
                // LLInventoryModel::emptyFolderType does not apply RLVa locks.
                // The Trash dialog still refuses to purge worn items.
                Operation::Purge(id) if self.inventory.folders.get(id).is_some_and(|folder| folder.info.type_default == 14) => true,
                Operation::Delete { id, folder: true } | Operation::Purge(id) => self.rlva_inventory_tree_unlocked(*id, actor),
                Operation::Delete { id, folder: false } => {
                    let target = super::super::inventory::actions::original(&self.inventory, *id).unwrap_or(*id);
                    let worn = self.worn_attachment_items().contains(&target)
                        || appearance::cof(&self.inventory)
                            .is_some_and(|id| appearance::folder_links(&self.inventory, id).iter().any(|l| l.target == target));
                    // RlvFolderLocks::canRemoveItem (rlvlocks.h): any lock
                    // of the item's folder but the inventory root's.
                    self.inventory.items.get(id).is_some_and(|it| {
                        [false, true].into_iter().all(|add| {
                            self.rlva_folder_signature(it.parent, add, actor)
                                .iter()
                                .all(|(_, name, _)| root_lock(name))
                        })
                    }) && (!worn || self.rlva_can_remove(target, actor))
                }
                Operation::Links { parent, links } if Some(*parent) == appearance::cof(&self.inventory) => {
                    let before = appearance::folder_links(&self.inventory, *parent);
                    let after: Vec<_> = before.iter().chain(links).cloned().collect();
                    self.rlva_outfit_allowed(&before, &after, actor)
                }
                _ => true,
            })
    }

    /// Asks for what is not loaded yet under #RLV, the folders and the
    /// originals of their links, as RlvInventory::fetchSharedInventory and
    /// fetchSharedLinks do once logged in (rlvinventory.cpp). Nothing waits
    /// for it: RlvHandler runs every command at once once the login is
    /// complete (processCommand, onLoginComplete; rlvhandler.cpp), with what
    /// the inventory holds.
    fn rlva_shared_fetch(&mut self) {
        let Some(root) = shared_root(&self.inventory) else {
            if !appearance::loaded(&self.inventory, self.inventory.root) {
                self.inventory.request(self.inventory.root);
            }
            return;
        };
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut missing = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if !appearance::loaded(&self.inventory, id) {
                self.inventory.request(id);
            }
            if let Some(f) = self.inventory.folders.get(&id) {
                stack.extend(f.children.iter().copied());
                for id in &f.items {
                    if let Some(it) = self.inventory.items.get(id)
                        && it.asset_type == 24
                        && !self.inventory.items.contains_key(&it.asset_id)
                    {
                        missing.insert(it.asset_id);
                    }
                }
            }
        }
        let request: Vec<Uuid> = missing.into_iter().filter(|id| self.rlva_inventory_fetch_due(*id)).collect();
        if !request.is_empty() {
            self.rlva.out.push(aurora_net::NetCommand::FetchItems {
                items: request,
                owner: self.agent_id,
            });
        }
    }
    /// Resolve forced wear through the same COF planner as the native UI.
    /// A request waits only for the outfit (COF) to be known, which the
    /// reference has once logged in; a folder force runs at once with what
    /// is loaded under #RLV.
    /// The removals and wears join the source's open `Action::Force` batch
    /// (`force_wear`), applied as one COF update.
    pub fn rlva_inventory_force(&mut self, source: Uuid, c: &Command) -> bool {
        let Some(name) = force_kind(&c.behaviour) else { return false };
        let context = c.behaviour.contains("this");
        if context && c.option.is_empty() && self.objects.index_of_uuid(&source).is_none() {
            self.rlva.relative.wait(source, c.clone(), std::time::Instant::now());
            return true;
        }
        // RlvForceHandler<RLV_BHVR_DETACHME> fails on any option.
        if name == "detachme" && !c.option.is_empty() {
            return true;
        }
        // Not a limit of the reference, which runs each force at once: the
        // queue waits for the outfit, and a hostile object alternating with
        // another one must not grow it without end. Far above any real
        // batch (one per object and command run).
        if self.rlva.appearance.len() >= FORCE_QUEUE_LIMIT {
            return true;
        }
        if appearance::cof(&self.inventory).is_none_or(|cof| !appearance::complete(&self.inventory, cof)) {
            let worn = self.outfit_attachment_items();
            self.rlva.out.extend(appearance::prepare(&mut self.inventory, self.agent_id, &worn));
            return false;
        }
        let worn = self.rlva_worn_attachment_points();
        // RlvCommandOptionHelper::parseOption<RlvCommandOptionGeneric>
        // (rlvhelper.cpp) reads a wearable type, then a point, then a UUID,
        // then a shared folder, then a point group: a folder only counts
        // when the option is none of the first three.
        let folder_option = !c.option.is_empty()
            && layer(&c.option).is_none()
            && point(&c.option).is_none()
            && super::numeric::uuid_set(&c.option).is_none();
        if !context && folder_option {
            self.rlva_shared_fetch();
        }
        let folder = if folder_option { resolve(&self.inventory, &c.option) } else { None };
        let group = folder.is_none() && GROUPS.contains(&c.option.as_str());
        let removal = if name == "detachme" {
            // LLVOAvatarSelf::detachAttachmentIntoInventory
            // (llvoavatarself.cpp) needs the inventory item: a temporary
            // attachment has none and stays on.
            self.rlva_attachment_item(source)
                .filter(|item| self.inventory.items.contains_key(item))
                .map(|item| {
                    self.rlva.self_detach = Some((source, item));
                    vec![item]
                })
        } else if !context && matches!(name, "detach" | "remattach") && folder.is_none() {
            // RlvForceRemAttachHandler: a point, a group or nothing, a UUID
            let target = super::numeric::uuid_set(&c.option)
                .filter(|id| !id.is_nil())
                .and_then(|id| self.rlva_attachment_item(id));
            let known = c.option.is_empty() || point(&c.option).is_some() || group || target.is_some();
            let ids = worn
                .iter()
                .filter(|(id, at)| {
                    known
                        && (c.option.is_empty()
                            || point(&c.option) == Some(**at)
                            || target == Some(**id)
                            || (group && self.rlva_attachment_group(**at) == Some(c.option.as_str())))
                })
                // RlvForceWear::isForceDetachable: a script never takes
                // the viewer's own bridge off.
                .filter(|(id, at)| !crate::world::bridge::viewer_bridge(self, **id, **at))
                .map(|(id, _)| *id)
                .filter(|id| self.rlva_can_remove(*id, source))
                .collect();
            Some(ids)
        } else if !context && name == "remoutfit" && folder.is_none() {
            // RlvForceHandler<RLV_BHVR_REMOUTFIT>: a layer or nothing
            let kind = layer(&c.option);
            let ids = if c.option.is_empty() || kind.is_some() {
                appearance::cof(&self.inventory)
                    .map(|cof| appearance::folder_links(&self.inventory, cof))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|l| self.inventory.items.get(&l.target))
                    .filter(|it| it.asset_type == 5 && (kind.is_none() || kind == Some((it.flags & 0xff) as u8)))
                    .filter(|it| self.rlva_can_remove(it.id, source))
                    .map(|it| it.id)
                    .collect()
            } else {
                Vec::new()
            };
            Some(ids)
        } else {
            self.rlva_shared_fetch();
            let bases = if context {
                self.rlva_path_folders(source, &c.option)
            } else {
                // RLV_CMD_FORCEWEAR with FORCEWEAR_CONTEXT_NONE:
                // VERIFY_OPTION(isSharedFolder())
                folder.filter(|id| Some(*id) != shared_root(&self.inventory)).into_iter().collect()
            };
            let attach = !matches!(name, "detach" | "detachall" | "remattach" | "remoutfit");
            let all = matches!(name, "attachall" | "attachallover" | "detachall");
            let replace = !matches!(name, "attachover" | "attachallover");
            let shared = shared_root(&self.inventory);
            let mut collected = Vec::new();
            for base in bases {
                if Some(base) == shared {
                    continue;
                }
                collected.push(self.rlva_collect(base, all, attach, replace));
            }
            if !attach {
                // RlvForceWear::forceFolder, ACTION_DETACH: the worn clothing
                // and attachments that can be forced off (isForceRemovable,
                // isForceDetachable), and the gestures
                let mut ids: Vec<Uuid> = Vec::new();
                for entry in collected.into_iter().flatten() {
                    let Some(it) = self.inventory.items.get(&entry.item) else {
                        continue;
                    };
                    let bridge = it.asset_type == 6
                        && worn
                            .get(&entry.item)
                            .is_some_and(|at| crate::world::bridge::viewer_bridge(self, entry.item, *at));
                    if it.asset_type != 13 && !bridge && !ids.contains(&entry.item) && self.rlva_can_remove(entry.item, source) {
                        ids.push(entry.item);
                    }
                }
                Some(ids)
            } else {
                for entries in collected {
                    self.rlva_force_folder(source, entries);
                }
                None
            }
        };
        if let Some(items) = removal {
            self.rlva_force_remove(source, items);
        }
        true
    }

    /// RlvForceWear::forceFolder for a wear (rlvhelper.cpp): the entries in
    /// the collector's order, each with its folder's action. The first
    /// clothing of a type takes that action, the next ones of the type add
    /// (fSeenWType). What a lock leaves of an entry (`rlva_force_wear_mask`)
    /// decides whether it is worn at all.
    fn rlva_force_folder(&mut self, source: Uuid, entries: Vec<super::collection::Entry>) {
        let mut seen_types = HashSet::new();
        let current = appearance::cof(&self.inventory)
            .map(|id| appearance::folder_links(&self.inventory, id))
            .unwrap_or_default();
        for entry in entries {
            let Some(it) = self.inventory.items.get(&entry.item) else {
                continue;
            };
            let (kind, layer) = (it.asset_type, (it.flags & 0xff) as u8);
            let mut replace = entry.replace;
            match kind {
                5 | 13 => {
                    if seen_types.contains(&layer) {
                        replace = false;
                    }
                    let mask = self.rlva_force_wear_mask(&entry);
                    if (replace && mask.replace) || (!replace && mask.add) {
                        if !self.rlva_force_pending(source, entry.item) {
                            // RlvForceWear::addWearable: a worn item replaces nothing
                            let worn = current.iter().any(|l| l.target == entry.item);
                            self.rlva_note_wear_entry(entry.item, entry.entry);
                            self.rlva_force_wear(
                                source,
                                appearance::WearRequest {
                                    item: entry.item,
                                    replace: replace && !worn,
                                    point: 0,
                                },
                            );
                        }
                        seen_types.insert(layer);
                    }
                }
                6 => {
                    let mask = self.rlva_force_wear_mask(&entry);
                    if ((replace && mask.replace) || (!replace && mask.add)) && !self.rlva_force_pending(source, entry.item) {
                        self.rlva_note_wear_entry(entry.item, entry.entry);
                        self.rlva_force_wear(
                            source,
                            appearance::WearRequest {
                                item: entry.item,
                                replace,
                                point: entry.point.unwrap_or(0),
                            },
                        );
                    }
                }
                21 if !self.rlva_force_pending(source, entry.item) => {
                    self.rlva_force_wear(
                        source,
                        appearance::WearRequest {
                            item: entry.item,
                            replace: false,
                            point: 0,
                        },
                    );
                }
                _ => {}
            }
        }
    }

    /// The ERlvWearMask RlvForceWear::forceFolder reads for an entry:
    /// RlvAttachmentLocks::canAttach and RlvWearableLocks::canWear
    /// (rlvlocks.h) on the inventory entry itself, so that a link counts by
    /// its own folder. A worn item that is locked holds its point or type
    /// (add only); nostrip does not, it only keeps the item on.
    fn rlva_force_wear_mask(&self, entry: &super::collection::Entry) -> super::wear_locks::Wear {
        use super::wear_locks::Wear;
        let locked = Wear {
            add: false,
            replace: false,
        };
        let all = Wear { add: true, replace: true };
        let nil = Uuid::nil();
        if !self.rlva.inventory_locked() {
            return all;
        }
        let Some(it) = self.inventory.items.get(&entry.item) else {
            return locked;
        };
        let folder = self.inventory.items.get(&entry.entry).map_or(it.parent, |e| e.parent);
        if !self.rlva_folder_allowed_in(&FolderLocks::default(), folder, true, nil) {
            return locked;
        }
        let worn = self.rlva_worn_attachment_points();
        match it.asset_type {
            6 => {
                if self.rlva.rules_for("addattach", nil).any(|(_, option)| option.is_empty()) {
                    return locked;
                }
                let Some(at) = entry.point else { return all };
                if !self.rlva_point_addable(at) {
                    return locked;
                }
                Wear {
                    add: true,
                    replace: worn
                        .iter()
                        .filter(|(_, p)| **p == at)
                        .all(|(old, _)| self.rlva_can_remove(*old, nil)),
                }
            }
            5 | 13 => {
                let layer = (it.flags & 0xff) as u8;
                if self.rlva_layer_locked("addoutfit", layer, nil) {
                    return locked;
                }
                let held = appearance::cof(&self.inventory)
                    .map(|cof| appearance::folder_links(&self.inventory, cof))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|l| self.inventory.items.get(&l.target))
                    .any(|old| matches!(old.asset_type, 5 | 13) && (old.flags & 0xff) as u8 == layer && !self.rlva_can_remove(old.id, nil));
                if !held {
                    all
                } else if it.asset_type == 5 {
                    // gAgentWearables.canAddWearable: clothing layers stack
                    Wear { add: true, replace: false }
                } else {
                    locked
                }
            }
            _ => all,
        }
    }

    /// RlvForceWear::isAddAttachment / isAddWearable / the gesture list: the
    /// item is already in the source's open batch.
    fn rlva_force_pending(&self, source: Uuid, item: Uuid) -> bool {
        matches!(self.rlva.appearance.last(), Some((s, appearance::Action::Force { wear, .. })) if *s == source && wear.iter().any(|w| w.item == item))
    }

    pub fn rlva_can_remove(&self, item: Uuid, actor: Uuid) -> bool {
        let r = &self.rlva;
        if r.enabled() && actor == super::watchdog::ACTOR {
            return r.watchdog.rejected.contains(&item);
        }
        if !r.enabled() || (actor.is_nil() && !r.inventory_locked()) {
            return true;
        }
        let Some(it) = self.inventory.items.get(&item) else {
            // no record of its own: its links under #RLV still hold it
            return (self.rlva_temporary_removable(item, actor) || self.rlva_unloaded_attachment_removable(item, actor))
                && self.rlva_item_folders_removable_in(&FolderLocks::default(), item, Uuid::nil());
        };
        // Nostrip is the user's override of scripted undressing. It does not
        // prevent the user manually removing an item.
        if !actor.is_nil()
            && self.rlva.self_detach != Some((actor, item))
            && !super::collection::strippable(&self.inventory, item, self.rlva.preferences.legacy_naming)
        {
            return false;
        }
        if it.asset_type == 6 {
            let p = self
                .worn_attachment_points()
                .get(&item)
                .copied()
                .or_else(|| r.watchdog.point(item))
                .unwrap_or((it.flags & 0xff) as u8);
            if !self.rlva_attachment_remove_at(item, p, actor) {
                return false;
            }
        } else if matches!(it.asset_type, 5 | 13) && self.rlva_layer_locked("remoutfit", (it.flags & 0xff) as u8, actor) {
            return false;
        }
        self.rlva_item_folders_removable_in(&FolderLocks::default(), item, Uuid::nil())
    }

    pub fn rlva_can_add(&self, item: Uuid, at: u8, actor: Uuid) -> bool {
        self.rlva_can_add_in(&FolderLocks::default(), item, at, actor)
    }

    pub(super) fn rlva_can_add_in(&self, locks: &FolderLocks, item: Uuid, at: u8, actor: Uuid) -> bool {
        let r = &self.rlva;
        if r.enabled() && actor == super::watchdog::ACTOR {
            return r.watchdog.restore_allowed(item, at);
        }
        if r.enabled() && at != 0 && r.watchdog.pending.iter().any(|(id, p)| *id != item && p.attachment.point == at) {
            return false;
        }
        if !r.enabled() || !r.inventory_locked() {
            return true;
        }
        let Some(it) = self.inventory.items.get(&item) else {
            return self.rlva_temporary_addable(item, at, actor);
        };
        if it.asset_type == 6 {
            // RlvAttachmentLocks::canAttach(const LLInventoryItem*)
            // (rlvlocks.h): with no point asked, the one the names give
            // (RlvAttachPtLookup::getAttachPoint), else RLV_WEAR; a point
            // saved on the item does not count (point 0: only an @addattach
            // with no point refuses it)
            let at = if at == 0 {
                super::collection::attachment_point(&self.inventory, item, self.rlva.preferences.legacy_naming).unwrap_or(0)
            } else {
                at
            };
            if !self.rlva_point_addable(at) {
                return false;
            }
        }
        if matches!(it.asset_type, 5 | 13) && self.rlva_layer_locked("addoutfit", (it.flags & 0xff) as u8, actor) {
            return false;
        }
        self.rlva_folder_allowed_in(locks, self.rlva_wear_folder(item).unwrap_or(it.parent), true, actor)
    }

    /// The point a Wear or an Add from the inventory asks for
    /// (rez_attachment, llinventorybridge.cpp): with RLVa on, no point chosen
    /// and a point locked in any way (RlvAttachmentLocks::
    /// hasLockedAttachmentPoint(RLV_LOCK_ANY), rlvlocks.h), the point the
    /// item or its folder names (RlvAttachPtLookup::getAttachPoint), else
    /// the default point. RLVaDebugDeprecateExplicitPoint skips the lookup.
    pub fn rlva_rez_point(&self, item: Uuid, point: u8) -> u8 {
        if point != 0 || !self.rlva.enabled() || self.rlva.preferences.debug_deprecate_explicit_point {
            return point;
        }
        let any_lock = self.rlva_has_locked_attachment() || self.rlva.rules_for("addattach", Uuid::nil()).next().is_some();
        if !any_lock {
            return point;
        }
        super::collection::attachment_point(&self.inventory, item, self.rlva.preferences.legacy_naming).unwrap_or(0)
    }

    /// The folder of the entry an item was last asked to be worn from, while
    /// that request stands (`WEAR_ENTRY_TIME`).
    fn rlva_wear_folder(&self, item: Uuid) -> Option<Uuid> {
        self.rlva
            .wear_entries
            .get(&item)
            .filter(|(_, at)| at.elapsed() < WEAR_ENTRY_TIME)
            .map(|(folder, _)| *folder)
    }

    /// RlvAttachmentLocks::canAttach and RlvWearableLocks::canWear
    /// (rlvlocks.h) check the add lock of the folder holding the inventory
    /// entry that is worn: a link is worn by its own folder, not by its
    /// original's. The entry `entry` (a link or `item` itself) is asked to be
    /// worn for `item`: until the wear is done, every check of it (the outfit
    /// write, the rez, the watchdog) reads that folder.
    pub(crate) fn rlva_note_wear_entry(&mut self, item: Uuid, entry: Uuid) {
        self.rlva.wear_entries.retain(|_, (_, at)| at.elapsed() < WEAR_ENTRY_TIME);
        match self.inventory.items.get(&entry) {
            Some(it) if entry != item => {
                self.rlva.wear_entries.insert(item, (it.parent, std::time::Instant::now()));
            }
            _ => {
                self.rlva.wear_entries.remove(&item);
            }
        }
    }

    /// The entries of a wear asked through the interface (`Action::WearItem`,
    /// `WearItems`, `Add`): items or links.
    pub fn rlva_note_wear_request(&mut self, action: &appearance::Action) {
        let entries: Vec<Uuid> = match action {
            appearance::Action::WearItem { item, .. } | appearance::Action::Add(item) => vec![*item],
            appearance::Action::WearItems { items, .. } => items.clone(),
            _ => return,
        };
        for entry in entries {
            if let Some(item) = actions::original(&self.inventory, entry) {
                self.rlva_note_wear_entry(item, entry);
            }
        }
    }

    /// The avatar's `#RLV` folder (RlvInventory::getSharedRootID), if it has one.
    pub fn rlva_shared_root(&self) -> Option<Uuid> {
        shared_root(&self.inventory)
    }

    pub fn rlva_attachment_item(&self, object: Uuid) -> Option<Uuid> {
        if let Some(mut index) = self.objects.index_of_uuid(&object) {
            for _ in 0..64 {
                let Some(o) = self.objects.get(index) else {
                    break;
                };
                if o.attachment_point() != 0
                    && self
                        .objects
                        .parent_of(o)
                        .and_then(|i| self.objects.get(i))
                        .is_some_and(|p| p.is_avatar())
                {
                    return Some(o.attachment_item_id().filter(|item| !item.is_nil()).unwrap_or(o.full_id));
                }
                if let Some(item) = o.attachment_item_id().filter(|item| !item.is_nil()) {
                    return Some(item);
                }
                let Some(parent) = self.objects.parent_of(o) else {
                    break;
                };
                index = parent;
            }
        }
        self.rlva.watchdog.sources.get(&object).copied()
    }

    pub(super) fn rlva_point_locked(&self, name: &str, p: u8, _actor: Uuid) -> bool {
        self.rlva
            .rules_for(name, Uuid::nil())
            .any(|(_, option)| option.is_empty() || point(option) == Some(p))
    }
    pub(super) fn rlva_point_addable(&self, p: u8) -> bool {
        !self.rlva_point_locked("addattach", p, Uuid::nil())
            && !self
                .rlva
                .rules_for("detach", Uuid::nil())
                .any(|(_, option)| !option.is_empty() && point(option) == Some(p))
    }
    pub(super) fn rlva_layer_locked(&self, name: &str, kind: u8, _actor: Uuid) -> bool {
        self.rlva
            .rules_for(name, Uuid::nil())
            .any(|(_, option)| option.is_empty() || layer(option) == Some(kind))
    }
    /// A rule a folder lock check can match (`folder_rule`) is set.
    pub(super) fn rlva_has_folder_rule(&self) -> bool {
        self.rlva
            .rules
            .values()
            .any(|rules| rules.iter().any(|r| folder_rule(&r.behaviour)))
    }
    /// A lock on every shared or every unshared folder (`placement_rule`) is set.
    pub(super) fn rlva_has_placement_rule(&self) -> bool {
        self.rlva
            .rules
            .values()
            .any(|rules| rules.iter().any(|r| placement_rule(&r.behaviour)))
    }

    /// LLAppearanceMgr::updateCOF and removeItemsFromAvatar with RLVa
    /// (llappearancemgr.cpp): a lock never refuses an outfit change as a
    /// whole. Locked worn items (attachments, clothing, body parts) stay in
    /// the COF, new items that cannot be worn are left out, the rest goes
    /// through. A new body part does not displace a locked one of its type.
    /// With RLVaWearReplaceUnlocked off, an attachment replacing a locked one
    /// on its point is left out: RlvAttachmentLockWatchdog::onAttach
    /// (rlvlocks.cpp) would detach it again. Returns the new items left out,
    /// so that the attachments rezzed after the change leave them out too.
    pub fn rlva_prepare_outfit(&self, command: &mut aurora_net::NetCommand, actor: Uuid) -> Vec<Uuid> {
        if !self.rlva.enabled() || actor == super::watchdog::ACTOR {
            return Vec::new();
        }
        let aurora_net::NetCommand::UpdateOutfit { change, .. } = command else {
            return Vec::new();
        };
        let Some(cof) = appearance::cof(&self.inventory) else {
            return Vec::new();
        };
        let links = if change.folder == cof {
            &mut change.links
        } else if let Some((id, links)) = &mut change.cof {
            if *id != cof {
                return Vec::new();
            }
            links
        } else {
            return Vec::new();
        };
        let before = appearance::folder_links(&self.inventory, cof);
        let kept: Vec<_> = before
            .iter()
            .filter(|old| !old.folder && !links.iter().any(|link| link.target == old.target) && !self.rlva_can_remove(old.target, actor))
            .cloned()
            .collect();
        let worn = self.worn_attachment_points();
        let kept_slot = |kind: i32, slot: u32| {
            kept.iter().filter_map(|old| self.inventory.items.get(&old.target)).any(|old| {
                old.asset_type == kind
                    && if kind == 6 {
                        worn.get(&old.id).map_or(old.flags & 0xff, |p| u32::from(*p)) == slot
                    } else {
                        old.flags & 0xff == slot
                    }
            })
        };
        let mut dropped = Vec::new();
        links.retain(|link| {
            if link.folder || before.iter().any(|old| old.target == link.target) {
                return true;
            }
            let Some(it) = self.inventory.items.get(&link.target) else {
                return true;
            };
            let slot = it.flags & 0xff;
            let wearable = self.rlva_can_add(it.id, 0, actor)
                && match it.asset_type {
                    13 => !kept_slot(13, slot),
                    6 => self.rlva.preferences.replace_unlocked || slot == 0 || !kept_slot(6, slot),
                    _ => true,
                };
            if !wearable {
                dropped.push(it.id);
            }
            wearable
        });
        links.extend(kept);
        dropped
    }

    pub fn rlva_outfit_allowed(&self, before: &[OutfitLink], after: &[OutfitLink], actor: Uuid) -> bool {
        before
            .iter()
            .filter(|l| !l.folder && !after.iter().any(|n| n.target == l.target))
            .all(|l| self.rlva_can_remove(l.target, actor))
            && after
                .iter()
                .filter(|l| !l.folder && !before.iter().any(|n| n.target == l.target))
                .all(|l| self.rlva_can_add(l.target, 0, actor))
    }

    /// getattach/getoutfit optionally hide locked slots, exempting the query's
    /// issuer as the reference does. Forced removals still obey every lock.
    fn rlva_query_removable(&self, item: Uuid, except: Uuid, worn: &std::collections::HashMap<Uuid, u8>) -> bool {
        if !super::collection::strippable(&self.inventory, item, self.rlva.preferences.legacy_naming) {
            return false;
        }
        if let Some(point) = worn.get(&item) {
            if self
                .rlva
                .rules_for("remattach", except)
                .any(|(_, option)| option.is_empty() || super::inventory::point(option) == Some(*point))
            {
                return false;
            }
            if self.rlva.rules_for("detach", except).any(|(source, option)| {
                if option.is_empty() {
                    self.rlva_attachment_item(source) == Some(item)
                } else {
                    super::inventory::point(option) == Some(*point)
                }
            }) {
                return false;
            }
        } else if let Some(it) = self.inventory.items.get(&item)
            && self
                .rlva
                .rules_for("remoutfit", except)
                .any(|(_, option)| option.is_empty() || layer(option) == Some((it.flags & 0xff) as u8))
        {
            return false;
        }
        // isLockedAttachmentExcept and isLockedWearableExcept (rlvlocks.cpp)
        // leave the folder locks out once an issuer is exempted, which the
        // queries always do (they pass the asking object).
        !except.is_nil() || self.rlva_item_folders_removable_in(&FolderLocks::default(), item, except)
    }

    pub fn rlva_inventory_query(&mut self, c: &Command, source: Uuid) -> Option<String> {
        let worn_points = self.rlva_worn_attachment_points();
        let cof = appearance::cof(&self.inventory)
            .map(|id| appearance::folder_links(&self.inventory, id))
            .unwrap_or_default();
        let worn = |kind: usize| {
            cof.iter().any(|l| {
                self.inventory
                    .items
                    .get(&l.target)
                    .is_some_and(|it| matches!(it.asset_type, 5 | 13) && it.flags & 0xff == kind as u32)
            })
        };
        // LLWearableData::canAddWearable (indra/llappearance/llwearabledata.cpp):
        // a body part or the physics layer only when none is worn, other
        // clothing below MAX_CLOTHING_LAYERS in all.
        let clothing = cof
            .iter()
            .filter_map(|l| self.inventory.items.get(&l.target))
            .filter(|it| it.asset_type == 5)
            .count();
        let can_add_wearable = |kind: usize| if kind < 4 || kind == 15 { !worn(kind) } else { clothing < 60 };
        let list_layers = |mode: &str| {
            LAYERS
                .iter()
                .enumerate()
                .filter(|(i, _)| match mode {
                    "getoutfitnames" => worn(*i),
                    // RlvWearableLocks::canWear(LLWearableType::EType)
                    // (rlvlocks.h): an add-locked type is closed; a type
                    // that holds a locked wearable (type or folder lock)
                    // only takes one more when
                    // LLWearableData::canAddWearable allows it.
                    "getaddoutfitnames" => {
                        let held = cof.iter().filter_map(|l| self.inventory.items.get(&l.target)).any(|it| {
                            matches!(it.asset_type, 5 | 13) && it.flags & 0xff == *i as u32 && !self.rlva_can_remove(it.id, Uuid::nil())
                        });
                        !self.rlva_layer_locked("addoutfit", *i as u8, Uuid::nil()) && (!held || can_add_wearable(*i))
                    }
                    _ => cof
                        .iter()
                        .filter_map(|l| self.inventory.items.get(&l.target))
                        .any(|it| it.asset_type == 5 && it.flags & 0xff == *i as u32 && self.rlva_can_remove(it.id, source)),
                })
                .map(|(_, n)| *n)
                .collect::<Vec<_>>()
                .join(",")
        };
        let visible_layer = |kind: usize| {
            worn(kind)
                && (!self.rlva.preferences.hide_locked_layers
                    || kind < 4
                    || cof.iter().any(|link| {
                        self.inventory.items.get(&link.target).is_some_and(|it| {
                            it.asset_type == 5 && it.flags & 0xff == kind as u32 && self.rlva_query_removable(it.id, source, &worn_points)
                        })
                    }))
        };
        // RlvHandler::onGetAttach and onGetAttachNames leave the viewer's
        // own bridge out of the Center 2 count, and it is never force
        // detachable (RlvForceWear::isForceDetachable).
        let counted = |item: &Uuid, at: u8| !crate::world::bridge::viewer_bridge(self, *item, at);
        let visible_point = |point: u8| {
            worn_points.iter().any(|(item, at)| {
                *at == point
                    && counted(item, *at)
                    && (!self.rlva.preferences.hide_locked_attachments || self.rlva_query_removable(*item, source, &worn_points))
            })
        };
        match c.behaviour.as_str() {
            "getattach" => {
                return Some(if c.option.is_empty() {
                    (0..POINTS.len())
                        .map(|p| if p != 0 && visible_point(p as u8) { '1' } else { '0' })
                        .collect()
                } else {
                    point(&c.option).map_or(String::new(), |p| if visible_point(p) { "1" } else { "0" }.to_owned())
                });
            }
            "getoutfit" => {
                return Some(if c.option.is_empty() {
                    OUTFIT_ORDER.iter().map(|i| if visible_layer(*i) { '1' } else { '0' }).collect()
                } else {
                    layer(&c.option).map_or(String::new(), |i| if visible_layer(i as usize) { "1" } else { "0" }.to_owned())
                });
            }
            "getoutfitnames" | "getaddoutfitnames" | "getremoutfitnames" => {
                return Some(if c.option.is_empty() {
                    list_layers(&c.behaviour)
                } else {
                    String::new()
                });
            }
            "getattachnames" | "getaddattachnames" | "getremattachnames" => {
                return Some(
                    POINTS
                        .iter()
                        .enumerate()
                        .skip(1)
                        .filter(|(i, _)| {
                            let group = self.rlva_attachment_group(*i as u8);
                            !GROUPS.contains(&c.option.as_str()) || group == Some(c.option.as_str())
                        })
                        .filter(|(i, _)| match c.behaviour.as_str() {
                            "getattachnames" => worn_points.iter().any(|(id, p)| *p == *i as u8 && counted(id, *p)),
                            "getaddattachnames" => self.rlva_point_addable(*i as u8),
                            _ => worn_points
                                .iter()
                                .any(|(id, at)| *at == *i as u8 && counted(id, *at) && self.rlva_can_remove(*id, source)),
                        })
                        .map(|(i, name)| self.avatar_lib.attach_names.get(&(i as u8)).map_or(*name, String::as_str))
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            "getpath" | "getpathnew" => {
                self.rlva_shared_fetch();
                let paths: Vec<_> = self
                    .rlva_path_folders(source, &c.option)
                    .into_iter()
                    .filter_map(|id| path(&self.inventory, id))
                    .collect();
                return Some(if c.behaviour == "getpath" {
                    paths.into_iter().next().unwrap_or_default()
                } else {
                    paths.join(",")
                });
            }
            _ => {}
        }
        // The shared-tree queries answer at once with what is loaded
        // (RlvHandler::onGetInv, onGetInvWorn, onFindFolder): what is not
        // loaded yet is asked for, under #RLV only.
        self.rlva_shared_fetch();
        let Some(root) = shared_root(&self.inventory) else {
            return Some(String::new());
        };
        if matches!(c.behaviour.as_str(), "findfolder" | "findfolders") {
            let terms: Vec<_> = c
                .option
                .split("&&")
                .filter(|term| !term.is_empty())
                .map(str::to_ascii_lowercase)
                .collect();
            if terms.is_empty() {
                return Some(String::new());
            }
            let mut matches = Vec::new();
            let mut stack: Vec<_> = self
                .inventory
                .folders
                .get(&root)
                .into_iter()
                .flat_map(|f| f.children.iter().rev().copied())
                .collect();
            let mut seen = HashSet::new();
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(folder) = self.inventory.folders.get(&id) else {
                    continue;
                };
                stack.extend(folder.children.iter().rev().copied());
                // RlvCriteriaCategoryCollector (rlvinventory.h) lowers the
                // name with LLStringUtil::toLower, byte by byte: the ASCII
                // letters only on a UTF-8 locale. A Windows code page would
                // also change bytes of a multi-byte character, which breaks
                // the UTF-8: never copied.
                let name = folder.info.name.to_ascii_lowercase();
                if visible(&name)
                    && !name.starts_with('~')
                    && terms.iter().all(|term| name.contains(term))
                    && let Some(path) = path(&self.inventory, id)
                {
                    matches.push(path);
                }
            }
            if c.behaviour == "findfolder" {
                // onFindFolder chooses the deepest path and preserves the
                // first native traversal result when depths are equal.
                let mut best = None;
                let mut depth = 0;
                for path in matches {
                    let current = path.bytes().filter(|b| *b == b'/').count();
                    if best.is_none() || current > depth {
                        depth = current;
                        best = Some(path);
                    }
                }
                matches = best.into_iter().collect();
            }
            return Some(matches.join(","));
        }
        let Some(folder) = resolve(&self.inventory, &c.option) else {
            return Some(String::new());
        };
        let collected = if c.behaviour == "getinvworn" {
            self.rlva_collect(folder, true, true, true)
        } else {
            Vec::new()
        };
        let Some(folder) = self.inventory.folders.get(&folder) else {
            return Some(String::new());
        };
        let mut children: Vec<_> = folder
            .children
            .iter()
            .filter_map(|id| self.inventory.folders.get(id))
            .filter(|f| {
                !f.info.name.is_empty()
                    && !f.info.name.starts_with('.')
                    && (c.behaviour == "getinvworn" || !f.info.name.contains('/'))
                    && !folded(&f.info.name, self.rlva.preferences.legacy_naming)
            })
            .collect();
        if c.behaviour == "getinv" {
            return Some(children.iter().map(|f| f.info.name.as_str()).collect::<Vec<_>>().join(","));
        }
        let worn: HashSet<_> = cof
            .iter()
            .map(|l| l.target)
            .chain(worn_points.keys().copied())
            .chain(self.active_gestures.iter().copied())
            .collect();
        // One ancestry walk per collected item, shared by all returned rows.
        // The reference's getinvworn map orders child folders by UUID.
        children.sort_by_key(|folder| folder.info.id);
        let mut counts = std::collections::HashMap::<(Uuid, bool), (usize, usize)>::new();
        let mut ancestors = Vec::new();
        for entry in &collected {
            let worn = usize::from(worn.contains(&entry.item));
            let direct = counts.entry((entry.folder, false)).or_default();
            direct.0 += 1;
            direct.1 += worn;
            ancestors.clear();
            let mut id = entry.folder;
            while id != folder.info.id && !ancestors.contains(&id) {
                ancestors.push(id);
                let Some(parent) = self.inventory.folders.get(&id).map(|folder| folder.info.parent) else {
                    break;
                };
                id = parent;
                let descendant = counts.entry((id, true)).or_default();
                descendant.0 += 1;
                descendant.1 += worn;
            }
        }
        let status = |id, descendants| {
            let (total, worn) = counts.get(&(id, descendants)).copied().unwrap_or_default();
            if total == 0 {
                0
            } else if worn == 0 {
                1
            } else if worn == total {
                3
            } else {
                2
            }
        };
        Some(
            std::iter::once(format!("|{}{}", status(folder.info.id, false), status(folder.info.id, true)))
                .chain(
                    children
                        .iter()
                        .map(|f| format!("{}|{}{}", f.info.name, status(f.info.id, false), status(f.info.id, true))),
                )
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}
