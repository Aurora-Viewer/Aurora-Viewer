//! What the wear menus grey out under RLVa locks: the ERlvWearMask answers of
//! RlvAttachmentLocks::canAttach / canDetach and RlvWearableLocks::canWear /
//! canRemove (rlvlocks.h), as LLObjectBridge, LLWearableBridge and
//! LLAppearanceMgr::getCanAddToCOF / getCanRemoveFromCOF / getCanReplaceCOF
//! use them (llinventorybridge.cpp, llappearancemgr.cpp, originally LGPL 2.1).
//! The menus see the inventory, not the world: one snapshot a frame answers
//! them, and the folder locks it holds are kept while their inputs stay equal.

use super::super::{World, appearance};
use super::inventory::{FolderLocks, LAYERS, LockInputs};
use crate::world::inventory::Inventory;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
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
pub(crate) struct FolderSets {
    /// Folders whose items cannot be worn (RLV_LOCK_ADD).
    no_add: HashSet<Uuid>,
    /// Folders whose items cannot be taken off (RLV_LOCK_REMOVE).
    no_remove: HashSet<Uuid>,
}

/// The last folder resolution and its inputs.
#[derive(Default)]
pub(crate) struct FolderSetsCache(Mutex<Option<(LockInputs, Arc<FolderSets>)>>);

#[derive(Clone, Default)]
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
    folders: Arc<FolderSets>,
    /// The point each object was last seen attached to this session.
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

    /// Wearing `item` (an original) at `point` (0: its own point).
    pub fn wear(&self, inv: &Inventory, item: Uuid, point: u8) -> Wear {
        if !self.active {
            return Wear::ALL;
        }
        let Some(it) = inv.items.get(&item) else {
            return Wear::ALL;
        };
        if self.folders.no_add.contains(&it.parent) {
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
                // it was last seen on this session stands in.
                let at = if point != 0 {
                    Some(point)
                } else {
                    super::collection::attachment_point(inv, item, self.legacy_naming)
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
    pub fn rlva_wear_locks(&self) -> WearLocks {
        if !self.rlva.inventory_locked() {
            return WearLocks::default();
        }
        let nil = Uuid::nil();
        let mut locks = WearLocks {
            active: true,
            legacy_naming: self.rlva.preferences.legacy_naming,
            attach_closed: self.rlva.rules_for("addattach", nil).any(|(_, option)| option.is_empty()),
            last_points: self.rlva.last_points.snapshot(),
            ..Default::default()
        };
        let worn = self.rlva_worn_attachment_points();
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
        locks.folders = self.rlva_folder_sets(worn);
        locks
    }

    /// Every folder's add and remove lock, kept while the rules, the
    /// inventory and the worn attachments stay the same.
    fn rlva_folder_sets(&self, worn: std::collections::HashMap<Uuid, u8>) -> Arc<FolderSets> {
        if !self.rlva_has_folder_rule() {
            return Arc::default();
        }
        let inputs = self.rlva_lock_inputs(worn);
        let cache = &self.rlva.wear_folders.0;
        if let Some((cached, sets)) = cache.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            && *cached == inputs
        {
            return sets.clone();
        }
        let shared = FolderLocks::default();
        let mut sets = FolderSets::default();
        for (id, _) in self.inventory.folders.iter().filter(|(_, f)| !f.library) {
            if !self.rlva_folder_allowed_in(&shared, *id, true, Uuid::nil()) {
                sets.no_add.insert(*id);
            }
            if !self.rlva_folder_allowed_in(&shared, *id, false, Uuid::nil()) {
                sets.no_remove.insert(*id);
            }
        }
        let sets = Arc::new(sets);
        *cache.lock().unwrap_or_else(PoisonError::into_inner) = Some((inputs, sets.clone()));
        sets
    }
}
