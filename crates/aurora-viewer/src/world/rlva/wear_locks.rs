//! What the wear menus grey out under RLVa locks: the ERlvWearMask answers of
//! RlvAttachmentLocks::canAttach / canDetach and RlvWearableLocks::canWear /
//! canRemove (rlvlocks.h), as LLObjectBridge, LLWearableBridge and
//! LLAppearanceMgr::getCanAddToCOF / getCanRemoveFromCOF / getCanReplaceCOF
//! use them (llinventorybridge.cpp, llappearancemgr.cpp, originally LGPL 2.1).
//! The menus see the inventory, not the world: one snapshot answers them,
//! kept while what it was built from stays the same ([`LockInputs`]), as
//! RlvFolderLocks keeps its lookups until something marks them dirty. A
//! window that asks every frame then costs one comparison, not a rebuild.

use super::super::{World, appearance};
use super::inventory::{FolderLocks, LAYERS, LockInputs};
use crate::world::inventory::Inventory;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use uuid::Uuid;

/// What a lock leaves of wearing an item: adding it beside what is worn,
/// replacing what is worn in its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wear {
    pub add: bool,
    pub replace: bool,
}

impl Wear {
    const ALL: Self = Self { add: true, replace: true };
    const LOCKED: Self = Self {
        add: false,
        replace: false,
    };
}

#[derive(Default)]
struct FolderSets {
    /// Folders whose items cannot be worn (RLV_LOCK_ADD).
    no_add: HashSet<Uuid>,
    /// Folders whose items cannot be taken off (RLV_LOCK_REMOVE).
    no_remove: HashSet<Uuid>,
}

/// The last snapshot, what it was built from, and the revision of the last
/// seen points it copied.
#[derive(Default)]
pub(crate) struct WearLocksCache(Mutex<Option<(LockInputs, u64, Arc<WearLocks>)>>);

/// No wear lock: everything is allowed.
static UNLOCKED: LazyLock<Arc<WearLocks>> = LazyLock::new(Arc::default);

#[derive(Default)]
pub struct WearLocks {
    /// RLVa is on and a wear lock is set; otherwise everything is allowed.
    active: bool,
    legacy_naming: bool,
    /// Worn items (attachments, clothing, body parts) that cannot come off.
    fixed: HashSet<Uuid>,
    /// Attachment points nothing can be attached to.
    closed_points: HashSet<u8>,
    /// Points holding an attachment that cannot come off: add only.
    held_points: HashSet<u8>,
    /// @addattach with no point: nothing can be attached.
    attach_closed: bool,
    /// Wearable types nothing can be worn on.
    closed_layers: HashSet<u8>,
    /// Wearable types worn by an item that cannot come off: add only.
    held_layers: HashSet<u8>,
    folders: FolderSets,
    /// The point each object was last seen attached to (`LastPoints`).
    last_points: std::collections::HashMap<Uuid, u8>,
}

impl WearLocks {
    /// Whether the worn item `item` (an original, not a link) can be taken off.
    pub fn can_remove(&self, item: Uuid) -> bool {
        !self.active || !self.fixed.contains(&item)
    }

    /// The folder's own Add / Replace (RLV_LOCK_ADD) and Take Off
    /// (RLV_LOCK_REMOVE) locks.
    pub fn folder_allows(&self, folder: Uuid, add: bool) -> bool {
        !self.active
            || !if add {
                self.folders.no_add.contains(&folder)
            } else {
                self.folders.no_remove.contains(&folder)
            }
    }

    /// Wearing `entry` (an item, or a link to it) at `point` (0: its own
    /// point). The add lock is the one of the entry's own folder
    /// (RlvAttachmentLocks::canAttach, RlvWearableLocks::canWear on the item
    /// clicked, rlvlocks.h): a link in a locked folder cannot be worn, its
    /// original elsewhere can.
    pub fn wear(&self, inv: &Inventory, entry: Uuid, point: u8) -> Wear {
        if !self.active {
            return Wear::ALL;
        }
        let item = crate::world::inventory::actions::original(inv, entry).unwrap_or(entry);
        let Some(it) = inv.items.get(&item) else {
            return Wear::ALL;
        };
        let folder = inv.items.get(&entry).map_or(it.parent, |e| e.parent);
        if self.folders.no_add.contains(&folder) {
            return Wear::LOCKED;
        }
        match it.asset_type {
            6 => {
                if self.attach_closed {
                    return Wear::LOCKED;
                }
                // RlvAttachPtLookup::getAttachPoint: the point named by the
                // item or its folder. Deliberate difference (human decision
                // of 2026-10-31): without one, the point it was last worn
                // on, where Wear sends it, instead of RLV_WEAR; Wear is then
                // greyed when a locked attachment holds that point. The
                // inventory often has no saved point (flags 0): the point
                // it was last seen on (kept per account) stands in.
                let at = if point != 0 {
                    Some(point)
                } else {
                    super::collection::attachment_point(inv, entry, self.legacy_naming)
                        .or(Some((it.flags & 0xff) as u8).filter(|p| *p != 0))
                        .or_else(|| self.last_points.get(&item).copied())
                };
                match at {
                    Some(p) if self.closed_points.contains(&p) => Wear::LOCKED,
                    Some(p) => Wear {
                        add: true,
                        replace: !self.held_points.contains(&p),
                    },
                    None => Wear::ALL,
                }
            }
            5 | 13 => {
                let layer = (it.flags & 0xff) as u8;
                if self.closed_layers.contains(&layer) {
                    Wear::LOCKED
                } else if self.held_layers.contains(&layer) {
                    // Clothing layers stack; a body part only replaces.
                    Wear {
                        add: it.asset_type == 5,
                        replace: false,
                    }
                } else {
                    Wear::ALL
                }
            }
            _ => Wear::ALL,
        }
    }
}

impl World {
    pub fn rlva_wear_locks(&self) -> Arc<WearLocks> {
        if !self.rlva.inventory_locked() {
            return UNLOCKED.clone();
        }
        let worn = self.rlva_worn_attachment_points();
        let inputs = self.rlva_lock_inputs(worn.iter().map(|(item, at)| (*item, *at)));
        let points = self.rlva.last_points.revision();
        let cache = &self.rlva.wear_locks.0;
        if let Some((cached, seen, locks)) = cache.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            && *cached == inputs
            && *seen == points
        {
            return locks.clone();
        }
        let nil = Uuid::nil();
        let mut locks = WearLocks {
            active: true,
            legacy_naming: self.rlva.preferences.legacy_naming,
            attach_closed: self.rlva.rules_for("addattach", nil).any(|(_, option)| option.is_empty()),
            last_points: self.rlva.last_points.snapshot(),
            ..Default::default()
        };
        for (item, at) in &worn {
            if !self.rlva_can_remove(*item, nil) {
                locks.fixed.insert(*item);
                locks.held_points.insert(*at);
            }
        }
        let cof = appearance::cof(&self.inventory)
            .map(|cof| appearance::folder_links(&self.inventory, cof))
            .unwrap_or_default();
        for it in cof.iter().filter_map(|l| self.inventory.items.get(&l.target)) {
            if matches!(it.asset_type, 5 | 13) && !self.rlva_can_remove(it.id, nil) {
                locks.fixed.insert(it.id);
                locks.held_layers.insert((it.flags & 0xff) as u8);
            }
        }
        locks.closed_points = (1..=u8::MAX).filter(|p| !self.rlva_point_addable(*p)).collect();
        locks.closed_layers = (0..LAYERS.len() as u8)
            .filter(|kind| self.rlva_layer_locked("addoutfit", *kind, nil))
            .collect();
        locks.folders = self.rlva_folder_sets();
        let locks = Arc::new(locks);
        *cache.lock().unwrap_or_else(PoisonError::into_inner) = Some((inputs, points, locks.clone()));
        locks
    }

    /// The folders with an add or a remove lock. A "…this" lock reaches only
    /// the folders it names and what is under them, so only those are
    /// checked, not every folder of the inventory. @sharedwear, @sharedunwear
    /// and their unshared twins lock by where a folder is, whatever it is:
    /// with one of them set, every folder is checked.
    fn rlva_folder_sets(&self) -> FolderSets {
        let mut sets = FolderSets::default();
        if !self.rlva_has_folder_rule() {
            return sets;
        }
        let shared = FolderLocks::default();
        let mut check = |id: Uuid| {
            if !self.rlva_folder_allowed_in(&shared, id, true, Uuid::nil()) {
                sets.no_add.insert(id);
            }
            if !self.rlva_folder_allowed_in(&shared, id, false, Uuid::nil()) {
                sets.no_remove.insert(id);
            }
        };
        if self.rlva_has_placement_rule() {
            for (id, _) in self.inventory.folders.iter().filter(|(_, f)| !f.library) {
                check(*id);
            }
            return sets;
        }
        let mut seen = HashSet::new();
        let mut stack = shared.roots(self);
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(f) = self.inventory.folders.get(&id).filter(|f| !f.library) else {
                continue;
            };
            stack.extend(f.children.iter().copied());
            check(id);
        }
        sets
    }
}
