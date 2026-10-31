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

type LockedFolders = HashMap<(Uuid, String), Vec<Uuid>>;

/// The folders of every "…this" lock, shared by the checks of one pass (the
/// watchdog's attachments, a folder tree). Resolving them walks the whole
/// inventory, so the result is kept across passes while what it reads stays
/// the same ([`LockInputs`]), as RlvFolderLocks keeps its lookups until
/// onNeedsLookupRefresh marks them dirty (rlvlocks.cpp).
#[derive(Default)]
pub(crate) struct FolderLocks(std::cell::OnceCell<Arc<LockedFolders>>);

impl FolderLocks {
    fn folders(&self, world: &World, source: Uuid, option: &str) -> &[Uuid] {
        let all = self.0.get_or_init(|| world.rlva_locked_folders());
        all.get(&(source, option.to_owned())).map_or(&[], Vec::as_slice)
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
        let folders = Arc::new(
            self.rlva_context_folders_of(
                self.rlva
                    .rules
                    .iter()
                    .flat_map(|(source, rules)| rules.iter().map(move |r| (*source, r)))
                    .filter(|(_, r)| r.behaviour.contains("this"))
                    .map(|(source, r)| (source, r.option.as_str())),
            ),
        );
        *cache.lock().unwrap_or_else(PoisonError::into_inner) = Some((inputs, folders.clone()));
        folders
    }
}

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

/// rlvAttachGroupFromIndex; body groups in avatar_lad.xml are zero-based.
/// Deliberate correction: HUDs use the native HUD flag below. The reference's
/// pre-Bento group 8 is now ordinary Bento points; actual HUDs use group 9.
fn group_name(group: u8) -> Option<&'static str> {
    Some(match group {
        0 | 1 | 3 | 4 => "arms",
        2 => "head",
        5 | 7 => "legs",
        6 => "torso",
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
    "hind left foot",
    "hind right foot",
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
    let candidates = inv.folders.get(&inv.root).map(|f| f.children.clone()).unwrap_or_else(|| {
        let mut ids: Vec<_> = inv.folders.keys().copied().collect();
        ids.sort();
        ids
    });
    let mut empty = None;
    for id in candidates {
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

/// The rules `rlva_folder_signature_in` can match: the global shared and
/// unshared wear locks, and the `attach…this` / `detach…this` folder locks.
fn folder_rule(name: &str) -> bool {
    matches!(name, "sharedwear" | "sharedunwear" | "unsharedwear" | "unsharedunwear")
        || ((name.starts_with("attach") || name.starts_with("detach")) && name.contains("this"))
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
                        .is_some_and(|f| visible(&f.info.name) && f.info.name.to_lowercase().contains(&component.to_lowercase()))
                })
            })?;
    }
    Some(id)
}

impl World {
    fn rlva_collect(&mut self, base: Uuid, all: bool, attach: bool, replace: bool) -> Option<Vec<super::collection::Entry>> {
        let collection = super::collection::collect(&self.inventory, base, all, attach, replace, &self.rlva.preferences);
        let mut ready = true;
        for id in collection.missing {
            if self.inventory.folders.contains_key(&id) {
                self.inventory.request(id);
                ready = false;
            } else {
                if self.rlva_inventory_fetch_waiting(id) {
                    ready = false;
                }
                if self.rlva_inventory_fetch_due(id) {
                    self.rlva.out.push(aurora_net::NetCommand::FetchInventory {
                        folders: vec![id],
                        owner: self.agent_id,
                        library: false,
                    });
                }
            }
        }
        for id in collection.missing_items {
            if self.rlva_inventory_fetch_waiting(id) {
                ready = false;
            }
            if self.rlva_inventory_fetch_due(id) {
                self.rlva.out.push(aurora_net::NetCommand::FetchItems {
                    items: vec![id],
                    owner: self.agent_id,
                });
            }
        }
        ready.then_some(collection.items)
    }

    fn rlva_inventory_fetch_waiting(&self, id: Uuid) -> bool {
        self.rlva
            .inventory_fetches
            .get(&id)
            .is_none_or(|(at, tries)| *tries < 3 || at.elapsed().as_secs() < 10)
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
        let point = self.avatar_lib.attach_points.get(&point)?;
        if point.hud { Some("hud") } else { group_name(point.group) }
    }
    /// The folders of the "…this" locks of `keys` (issuer, option), sorted.
    /// An option naming items (none: the issuer's own; a point, a layer, an
    /// object) locks the folders of those items: shared originals can live
    /// elsewhere, so every shared link counts, and attachment-point folders
    /// fold into their parent. Finding the links walks the whole inventory:
    /// one walk for all the keys.
    fn rlva_context_folders_of<'a>(&self, keys: impl IntoIterator<Item = (Uuid, &'a str)>) -> HashMap<(Uuid, String), Vec<Uuid>> {
        let worn = self.rlva_worn_attachment_points();
        let mut cof = None;
        let mut out = HashMap::new();
        // item → the keys locking its folders
        let mut wanted = HashMap::<Uuid, Vec<(Uuid, String)>>::new();
        for (source, option) in keys {
            let key = (source, option.to_owned());
            if out.contains_key(&key) {
                continue;
            }
            let items: Vec<_> = if option.is_empty() {
                self.rlva_attachment_item(source).into_iter().collect()
            } else if let Some(p) = point(option) {
                worn.iter().filter(|(_, at)| **at == p).map(|(id, _)| *id).collect()
            } else if let Some(kind) = layer(option) {
                cof.get_or_insert_with(|| {
                    appearance::cof(&self.inventory)
                        .map(|id| appearance::folder_links(&self.inventory, id))
                        .unwrap_or_default()
                })
                .iter()
                .filter_map(|l| self.inventory.items.get(&l.target))
                .filter(|it| matches!(it.asset_type, 5 | 13) && it.flags & 0xff == u32::from(kind))
                .map(|it| it.id)
                .collect()
            } else if let Ok(id) = Uuid::parse_str(option) {
                self.rlva_attachment_item(id).into_iter().collect()
            } else {
                out.insert(key, resolve(&self.inventory, option).into_iter().collect());
                continue;
            };
            for item in items {
                wanted.entry(item).or_default().push(key.clone());
            }
            out.insert(key, Vec::new());
        }
        if !wanted.is_empty() {
            // a few items among the whole inventory: a sorted list tells most
            // of them apart faster than hashing every id
            let mut targets: Vec<_> = wanted.keys().copied().collect();
            targets.sort_unstable();
            for it in self.inventory.items.values() {
                let mut add = |item: &Uuid| {
                    if targets.binary_search(item).is_err() {
                        return;
                    }
                    let Some(keys) = wanted.get(item) else { return };
                    let folder = folded_parent(&self.inventory, it.parent, self.rlva.preferences.legacy_naming);
                    if path(&self.inventory, folder).is_some() {
                        for key in keys {
                            out.entry(key.clone()).or_default().push(folder);
                        }
                    }
                };
                add(&it.id);
                if it.asset_type == 24 && it.asset_id != it.id {
                    add(&it.asset_id);
                }
            }
        }
        for folders in out.values_mut() {
            folders.sort();
            folders.dedup();
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
            cof.iter()
                .filter(|link| worn.get(&link.target) == Some(&point))
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
        } else if option.is_empty() || Uuid::parse_str(option).is_ok() {
            let object = if option.is_empty() {
                source
            } else {
                Uuid::parse_str(option).unwrap_or_default()
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
        // The nearest rule of each issuer wins; an exception granted by one
        // issuer cannot release another issuer's subtree lock.
        let mut nearest = std::collections::BTreeMap::<Uuid, (usize, bool, Vec<(String, String)>)>::new();
        for (source, rules) in &self.rlva.rules {
            if *source == except {
                continue;
            }
            for rule in rules {
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
                    let entry = nearest.entry(*source).or_insert((usize::MAX, true, Vec::new()));
                    if distance < entry.0 {
                        *entry = (distance, allow, Vec::new());
                    } else if distance == entry.0 {
                        entry.1 &= allow;
                    }
                    if distance == entry.0 && !allow {
                        entry.2.push((rule.behaviour.clone(), rule.option.clone()));
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
        let signature = |id, add| {
            self.rlva_folder_signature_in(&locks, id, add, Uuid::nil())
                .into_iter()
                .filter(|(_, name, _)| folder || name.contains("this"))
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
            if !appearance::loaded(&self.inventory, id)
                || [false, true].into_iter().any(|add| {
                    self.rlva_folder_signature_in(&locks, id, add, Uuid::nil())
                        .iter()
                        .any(|(_, name, _)| name.contains("this"))
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
                    (!moved || self.rlva_inventory_move_allowed(*id, body["parent_id"].as_uuid(), actor))
                        && (!renamed || !*folder || !self.rlva_explicit_folder_descendant(&FolderLocks::default(), *id, actor))
                }
                Operation::Delete { id, folder: true } | Operation::Purge(id) => self.rlva_inventory_tree_unlocked(*id, actor),
                Operation::Delete { id, folder: false } => {
                    let target = super::super::inventory::actions::original(&self.inventory, *id).unwrap_or(*id);
                    let worn = self.worn_attachment_items().contains(&target)
                        || appearance::cof(&self.inventory)
                            .is_some_and(|id| appearance::folder_links(&self.inventory, id).iter().any(|l| l.target == target));
                    self.inventory.items.get(id).is_some_and(|it| {
                        [false, true].into_iter().all(|add| {
                            self.rlva_folder_signature(it.parent, add, actor)
                                .iter()
                                .all(|(_, name, _)| !name.contains("this"))
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

    fn rlva_shared_ready(&mut self) -> bool {
        let Some(root) = shared_root(&self.inventory) else {
            let loaded = appearance::loaded(&self.inventory, self.inventory.root);
            if !loaded {
                self.inventory.request(self.inventory.root);
            }
            return loaded;
        };
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut ready = true;
        let mut missing = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if !appearance::loaded(&self.inventory, id) {
                self.inventory.request(id);
                ready = false;
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
        if !missing.is_empty() {
            let mut request = Vec::new();
            for id in missing {
                if self.rlva_inventory_fetch_waiting(id) {
                    ready = false;
                }
                if self.rlva_inventory_fetch_due(id) {
                    request.push(id);
                }
            }
            if !request.is_empty() {
                self.rlva.out.push(aurora_net::NetCommand::FetchItems {
                    items: request,
                    owner: self.agent_id,
                });
            }
        }
        ready
    }
    /// Resolve forced wear through the same COF planner as the native UI.
    /// Requests remain queued while folders or originals are still loading.
    pub fn rlva_inventory_force(&mut self, source: Uuid, c: &Command) -> bool {
        let Some(name) = force_kind(&c.behaviour) else { return false };
        let context = c.behaviour.contains("this");
        if context && c.option.is_empty() && self.objects.index_of_uuid(&source).is_none() {
            self.rlva.relative.wait(source, c.clone(), std::time::Instant::now());
            return true;
        }
        if !context && c.option.is_empty() && matches!(name, "attach" | "attachover" | "attachall" | "attachallover" | "detachall") {
            return true;
        }
        if self.rlva.appearance.len() >= 128 {
            return true;
        }
        if appearance::cof(&self.inventory).is_none_or(|cof| !appearance::complete(&self.inventory, cof)) {
            let worn = self.outfit_attachment_items();
            self.rlva.out.extend(appearance::prepare(&mut self.inventory, self.agent_id, &worn));
            return false;
        }
        let worn = self.rlva_worn_attachment_points();
        let group = GROUPS.contains(&c.option.as_str()) && resolve(&self.inventory, &c.option).is_none();
        let action = if name == "detachme" {
            self.rlva_attachment_item(source).map(|item| {
                self.rlva.self_detach = Some((source, item));
                appearance::Action::Remove(item)
            })
        } else if !context
            && matches!(name, "detach" | "remattach")
            && (c.option.is_empty() || point(&c.option).is_some() || Uuid::parse_str(&c.option).is_ok() || group)
        {
            let target = Uuid::parse_str(&c.option).ok().and_then(|id| self.rlva_attachment_item(id));
            let ids = worn
                .iter()
                .filter(|(id, at)| {
                    c.option.is_empty()
                        || point(&c.option) == Some(**at)
                        || target == Some(**id)
                        || (group && self.rlva_attachment_group(**at) == Some(c.option.as_str()))
                })
                .map(|(id, _)| *id)
                .filter(|id| self.rlva_can_remove(*id, source))
                .collect();
            Some(appearance::Action::RemoveItems(ids))
        } else if name == "remoutfit" && (c.option.is_empty() || layer(&c.option).is_some()) {
            let ids = appearance::cof(&self.inventory)
                .map(|cof| appearance::folder_links(&self.inventory, cof))
                .unwrap_or_default()
                .iter()
                .filter_map(|l| self.inventory.items.get(&l.target))
                .filter(|it| it.asset_type == 5 && (c.option.is_empty() || layer(&c.option) == Some((it.flags & 0xff) as u8)))
                .filter(|it| self.rlva_can_remove(it.id, source))
                .map(|it| it.id)
                .collect();
            Some(appearance::Action::RemoveItems(ids))
        } else {
            if !self.rlva_shared_ready() {
                return false;
            }
            let bases = if context {
                self.rlva_path_folders(source, &c.option)
            } else {
                resolve(&self.inventory, &c.option)
                    .filter(|id| Some(*id) != shared_root(&self.inventory))
                    .into_iter()
                    .collect()
            };
            let attach = !matches!(name, "detach" | "detachall" | "remattach" | "remoutfit");
            let all = matches!(name, "attachall" | "attachallover" | "detachall");
            let replace = !matches!(name, "attachover" | "attachallover");
            let mut ids = Vec::new();
            let mut points = std::collections::HashMap::new();
            let mut modes = std::collections::HashMap::new();
            let shared = shared_root(&self.inventory);
            for base in bases {
                if Some(base) == shared {
                    continue;
                }
                let Some(entries) = self.rlva_collect(base, all, attach, replace) else {
                    return false;
                };
                for entry in entries {
                    ids.push(entry.item);
                    points.insert(entry.item, entry.point);
                    modes.insert(entry.item, entry.replace);
                }
            }
            ids.sort();
            ids.dedup();
            if matches!(name, "detach" | "detachall" | "remattach" | "remoutfit") {
                ids.retain(|id| self.inventory.items.get(id).is_some_and(|it| it.asset_type != 13) && self.rlva_can_remove(*id, source));
                Some(appearance::Action::RemoveItems(ids))
            } else {
                ids.sort_by(|a, b| {
                    modes
                        .get(b)
                        .cmp(&modes.get(a))
                        .then_with(|| {
                            self.inventory.items[a]
                                .name
                                .to_lowercase()
                                .cmp(&self.inventory.items[b].name.to_lowercase())
                        })
                        .then_with(|| a.cmp(b))
                });
                let mut replaced = HashSet::new();
                let current = appearance::cof(&self.inventory)
                    .map(|id| appearance::folder_links(&self.inventory, id))
                    .unwrap_or_default();
                for item in ids.into_iter().take(128) {
                    let it = &self.inventory.items[&item];
                    let point = points.get(&item).copied().unwrap_or(0);
                    let key = (it.asset_type, if it.asset_type == 6 { point as u32 } else { it.flags & 0xff });
                    let replace_item = it.asset_type != 21
                        && (modes.get(&item).copied().unwrap_or(replace) || it.asset_type == 13)
                        && !replaced.contains(&key);
                    if !self.rlva_can_add(item, point, source) {
                        continue;
                    }
                    if replace_item
                        && (it.asset_type != 6 || !self.rlva.preferences.replace_unlocked)
                        && current
                            .iter()
                            .filter(|l| l.target != item)
                            .filter_map(|l| self.inventory.items.get(&l.target))
                            .any(|old| {
                                let same_slot = old.asset_type == it.asset_type
                                    && if it.asset_type == 6 {
                                        point != 0 && worn.get(&old.id).copied().unwrap_or((old.flags & 0xff) as u8) == point
                                    } else {
                                        old.flags & 0xff == it.flags & 0xff
                                    };
                                same_slot && !self.rlva_can_remove(old.id, source)
                            })
                    {
                        continue;
                    }
                    replaced.insert(key);
                    self.rlva.appearance.push((
                        source,
                        appearance::Action::WearItem {
                            item,
                            replace: replace_item,
                            point,
                        },
                    ));
                }
                None
            }
        };
        if let Some(action) = action {
            self.rlva.appearance.push((source, action));
        }
        true
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
            return self.rlva_temporary_removable(item, actor) || self.rlva_unloaded_attachment_removable(item, actor);
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
        self.rlva_folder_allowed(it.parent, false, actor)
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
            let at = if at == 0 { (it.flags & 0xff) as u8 } else { at };
            if !self.rlva_point_addable(at) {
                return false;
            }
        }
        if matches!(it.asset_type, 5 | 13) && self.rlva_layer_locked("addoutfit", (it.flags & 0xff) as u8, actor) {
            return false;
        }
        self.rlva_folder_allowed_in(locks, it.parent, true, actor)
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
    fn rlva_layer_locked(&self, name: &str, kind: u8, _actor: Uuid) -> bool {
        self.rlva
            .rules_for(name, Uuid::nil())
            .any(|(_, option)| option.is_empty() || layer(option) == Some(kind))
    }

    /// LLAppearanceMgr::updateCOF and removeItemsFromAvatar with RLVa
    /// (llappearancemgr.cpp): a lock never refuses an outfit change as a
    /// whole. Locked worn items (attachments, clothing, body parts) stay in
    /// the COF, new items that cannot be worn are left out, the rest goes
    /// through. A new body part does not displace a locked one of its type.
    /// With RLVaWearReplaceUnlocked off, an attachment replacing a locked one
    /// on its point is left out: RlvAttachmentLockWatchdog::onAttach
    /// (rlvlocks.cpp) would detach it again.
    pub fn rlva_prepare_outfit(&self, command: &mut aurora_net::NetCommand, actor: Uuid) {
        if !self.rlva.enabled() || actor == super::watchdog::ACTOR {
            return;
        }
        let aurora_net::NetCommand::UpdateOutfit { change, .. } = command else {
            return;
        };
        let Some(cof) = appearance::cof(&self.inventory) else {
            return;
        };
        let links = if change.folder == cof {
            &mut change.links
        } else if let Some((id, links)) = &mut change.cof {
            if *id != cof {
                return;
            }
            links
        } else {
            return;
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
        links.retain(|link| {
            if link.folder || before.iter().any(|old| old.target == link.target) {
                return true;
            }
            let Some(it) = self.inventory.items.get(&link.target) else {
                return true;
            };
            let slot = it.flags & 0xff;
            self.rlva_can_add(it.id, 0, actor)
                && match it.asset_type {
                    13 => !kept_slot(13, slot),
                    6 => self.rlva.preferences.replace_unlocked || slot == 0 || !kept_slot(6, slot),
                    _ => true,
                }
        });
        links.extend(kept);
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
        self.inventory
            .items
            .get(&item)
            .is_none_or(|it| self.rlva_folder_signature_except(it.parent, false, except).is_empty())
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
        let list_layers = |mode: &str| {
            LAYERS
                .iter()
                .enumerate()
                .filter(|(i, _)| match mode {
                    "getoutfitnames" => worn(*i),
                    "getaddoutfitnames" => {
                        !self.rlva_layer_locked("addoutfit", *i as u8, Uuid::nil())
                            && !(*i < 4 && worn(*i) && self.rlva_layer_locked("remoutfit", *i as u8, Uuid::nil()))
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
        let visible_point = |point: u8| {
            worn_points.iter().any(|(item, at)| {
                *at == point && (!self.rlva.preferences.hide_locked_attachments || self.rlva_query_removable(*item, source, &worn_points))
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
                            "getattachnames" => worn_points.values().any(|p| *p == *i as u8),
                            "getaddattachnames" => self.rlva_point_addable(*i as u8),
                            _ => worn_points
                                .iter()
                                .any(|(id, at)| *at == *i as u8 && self.rlva_can_remove(*id, source)),
                        })
                        .map(|(i, name)| self.avatar_lib.attach_names.get(&(i as u8)).map_or(*name, String::as_str))
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            "getpath" | "getpathnew" => {
                if !self.rlva_shared_ready() {
                    return None;
                }
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
        if !self.rlva_shared_ready() {
            return None;
        }
        let Some(root) = shared_root(&self.inventory) else {
            let loaded = appearance::loaded(&self.inventory, self.inventory.root);
            self.inventory.request(self.inventory.root);
            return loaded.then(String::new);
        };
        // A shared-tree query cannot treat unloaded descendants as empty.
        // Request only #RLV, never unrelated inventory or the Library.
        let mut stack = vec![root];
        let mut seen = HashSet::new();
        let mut ready = true;
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if !appearance::loaded(&self.inventory, id) {
                self.inventory.request(id);
                ready = false;
            }
            if let Some(f) = self.inventory.folders.get(&id) {
                stack.extend(f.children.iter().copied());
            }
        }
        if !ready {
            return None;
        }
        if matches!(c.behaviour.as_str(), "findfolder" | "findfolders") {
            let terms: Vec<_> = c
                .option
                .split("&&")
                .filter(|term| !term.is_empty())
                .map(str::to_lowercase)
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
                let name = folder.info.name.to_lowercase();
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
        if !appearance::loaded(&self.inventory, folder) {
            self.inventory.request(folder);
            return None;
        }
        let collected = if c.behaviour == "getinvworn" {
            self.rlva_collect(folder, true, true, true)?
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
