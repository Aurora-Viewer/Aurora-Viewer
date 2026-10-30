//! Parent repair following AISUpdate::doUpdate and LLInventoryModel::updateItem
//! (indra/newview/llaisapi.cpp, llinventorymodel.cpp, LGPL 2.1).

use super::{FetchState, Inventory};
use aurora_llsd::llsd_map;
use aurora_net::inventory::{
    InvItem,
    operations::{Mutation, Operation},
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemSource {
    Fetch,
    Simulator,
    Attachment,
    Orphans,
    Cache,
    Snapshot,
}

// CATEGORIZE_LOST_AND_FOUND_ID (indra/llmessage/llassetstorage.cpp).
pub(super) const LOST_MARKER: uuid::Uuid = uuid::Uuid::from_u128(0x10);

impl Inventory {
    pub(super) fn repair_item_parents(&mut self, items: &mut [InvItem], source: ItemSource) {
        if self.owner.is_nil()
            || !items.iter().any(|it| {
                !it.id.is_nil()
                    && it.owner == self.owner
                    && (it.parent.is_nil() || it.parent == LOST_MARKER || !self.folders.contains_key(&it.parent))
            })
        {
            return;
        }
        let mut system = HashMap::new();
        for f in self
            .folders
            .values()
            .filter(|f| !f.library && !f.info.id.is_nil() && f.info.type_default >= 0 && f.info.parent == self.root)
        {
            // findCategoryUUIDForTypeInRoot picks the smallest UUID when a
            // corrupt skeleton contains duplicate system folders.
            system
                .entry(f.info.type_default)
                .and_modify(|id: &mut uuid::Uuid| *id = (*id).min(f.info.id))
                .or_insert(f.info.id);
        }
        for item in items {
            if item.id.is_nil() || item.owner != self.owner {
                continue;
            }
            let marker = item.parent == LOST_MARKER;
            let unknown = !item.parent.is_nil() && !self.folders.contains_key(&item.parent);
            if !marker && !item.parent.is_nil() && (!unknown || source == ItemSource::Simulator && self.items.contains_key(&item.id)) {
                continue;
            }
            self.deferred_parents.remove(&item.id);
            let previous = self
                .items
                .get(&item.id)
                .map(|it| it.parent)
                .filter(|id| !id.is_nil() && self.folders.get(id).is_some_and(|f| !f.library));
            // processUpdateCreateInventoryItem -> updateItem: preserve the
            // known parent or file a new object in FT_OBJECT. The attachment
            // recovery read keeps that context even after a detach/teleport.
            // AIS orphan repair is a separate path; a delayed metadata read
            // cannot undo an already pending normal parent assignment.
            let pending = previous.filter(|parent| self.repaired_parents.get(&item.id) == Some(parent));
            let lost = marker
                || unknown
                || matches!(
                    source,
                    ItemSource::Fetch | ItemSource::Snapshot | ItemSource::Orphans | ItemSource::Cache
                )
                || source == ItemSource::Simulator && self.items.get(&item.id).is_some_and(|it| it.parent.is_nil());
            let parent = pending
                .filter(|_| !marker)
                .or_else(|| if lost { system.get(&16).copied() } else { previous })
                .or_else(|| (!lost).then(|| system.get(&item.asset_type).copied()).flatten());
            let Some(parent) = parent else {
                self.deferred_parents.insert(item.id, source);
                continue;
            };
            item.parent = parent;
            if marker && source != ItemSource::Cache {
                // The simulator has already categorized this return. Legacy
                // translates the sentinel locally, without sending a move.
                self.repaired_parents.insert(item.id, parent);
                self.pending_parent_repairs.remove(&item.id);
                if !matches!(source, ItemSource::Simulator | ItemSource::Snapshot)
                    && previous != Some(parent)
                    && let Some(folder) = self.folders.get_mut(&parent)
                    && folder.state == FetchState::Fetched
                    && folder.info.version >= 0
                {
                    folder.info.version = folder.info.version.saturating_add(1);
                }
                self.request(parent);
                continue;
            }
            if self.repaired_parents.insert(item.id, parent) != Some(parent) {
                self.pending_parent_repairs.insert(item.id);
                if source != ItemSource::Simulator
                    && previous != Some(parent)
                    && let Some(folder) = self.folders.get_mut(&parent)
                    && folder.state == FetchState::Fetched
                    && folder.info.version >= 0
                {
                    folder.info.version = folder.info.version.saturating_add(1);
                }
            }
            self.request(parent);
        }
    }

    /// Keep a burst of orphaned records from flooding the simulator.
    pub fn take_parent_repairs(&mut self) -> Option<Mutation> {
        if self.deferred_lost_folders && self.lost_folder_retry_generation != self.generation {
            self.repair_lost_folders();
            self.lost_folder_retry_generation = self.generation;
        }
        self.retry_deferred_parents();
        let ids: Vec<_> = self.pending_parent_repairs.iter().take(32).copied().collect();
        let mut operations = Vec::new();
        let mut refresh = HashSet::new();
        for id in ids {
            self.pending_parent_repairs.remove(&id);
            let Some(parent) = self.repaired_parents.get(&id).copied() else {
                continue;
            };
            if !self.items.get(&id).is_some_and(|it| it.owner == self.owner && it.parent == parent) {
                continue;
            }
            operations.push(Operation::Patch {
                id,
                folder: false,
                body: llsd_map! { "parent_id" => parent },
            });
            refresh.insert(parent);
        }
        let folders: Vec<_> = self.pending_folder_repairs.iter().take(32 - operations.len()).copied().collect();
        for id in folders {
            self.pending_folder_repairs.remove(&id);
            let Some(parent) = self.repaired_folder_parents.get(&id).copied() else {
                continue;
            };
            if !self.folders.get(&id).is_some_and(|f| !f.library && f.info.parent == parent) {
                continue;
            }
            operations.push(Operation::Patch {
                id,
                folder: true,
                body: llsd_map! { "parent_id" => parent },
            });
            refresh.insert(parent);
        }
        (!operations.is_empty()).then_some(Mutation {
            operations,
            refresh: refresh.into_iter().collect(),
            ..Default::default()
        })
    }

    fn retry_deferred_parents(&mut self) {
        if self.parent_retry_generation == self.generation || self.deferred_parents.is_empty() {
            return;
        }
        self.parent_retry_generation = self.generation;
        let before = self.generation;
        let mut progressed = false;
        let pending: Vec<_> = self.deferred_parents.iter().take(32).map(|(id, source)| (*id, *source)).collect();
        for (id, source) in pending {
            self.deferred_parents.remove(&id);
            if let Some(mut item) = self.items.get(&id).cloned() {
                let old_parent = item.parent;
                self.repair_item_parents(std::slice::from_mut(&mut item), source);
                progressed |= item.parent != old_parent;
                self.add_items_inner(vec![item]);
            }
        }
        self.parent_retry_generation = if progressed { before } else { self.generation };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::{FolderContents, InvFolder};
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn fixture(state: FetchState) -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            owner: id(10),
            ..Default::default()
        };
        for (n, kind) in [(1, 8), (2, 16), (3, 6)] {
            inv.folders.insert(
                id(n),
                Folder {
                    info: InvFolder {
                        id: id(n),
                        parent: if n == 1 { Uuid::nil() } else { id(1) },
                        type_default: kind,
                        version: 5,
                        name: format!("Folder {n}"),
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: Vec::new(),
                    state,
                    library: false,
                },
            );
        }
        inv
    }

    fn object(n: u128, parent: Uuid) -> InvItem {
        InvItem {
            id: id(n),
            owner: id(10),
            parent,
            name: "Script-attached object".to_owned(),
            asset_type: 6,
            inv_type: 6,
            ..Default::default()
        }
    }

    fn snapshot(parent: Uuid, items: Vec<InvItem>) -> FolderContents {
        FolderContents {
            folder_id: parent,
            owner_id: id(10),
            version: 9,
            folders: Vec::new(),
            items,
        }
    }

    #[test]
    fn fetched_parentless_items_are_visible_in_lost_and_found_and_queue_the_original_id() {
        let mut inv = fixture(FetchState::Fetched);
        inv.add_items(vec![object(20, Uuid::nil())]);
        assert_eq!(inv.items[&id(20)].parent, id(2));
        assert_eq!(inv.folders[&id(2)].items, vec![id(20)]);
        let change = inv.take_parent_repairs().expect("parent repair");
        assert!(change.move_only());
        assert!(
            matches!(&change.operations[..], [Operation::Patch { id: item, folder: false, body }] if *item == id(20) && body["parent_id"].as_uuid() == id(2))
        );
        assert!(inv.take_parent_repairs().is_none());
        inv.add_items(vec![object(20, Uuid::nil())]);
        assert!(inv.take_parent_repairs().is_none(), "no repeated moves while readback is pending");
    }

    #[test]
    fn simulator_return_marker_is_translated_without_sending_a_redundant_move() {
        let mut inv = fixture(FetchState::Fetched);
        inv.server_updated(Vec::new(), vec![object(20, LOST_MARKER)]);
        assert_eq!(inv.items[&id(20)].parent, id(2));
        assert_eq!(inv.folders[&id(2)].items, vec![id(20)]);
        assert_eq!(inv.folders[&id(2)].info.version, 6);
        assert!(inv.take_parent_repairs().is_none());
        let mut returned = object(21, LOST_MARKER);
        inv.add_attachment_items(vec![object(21, Uuid::nil())]);
        assert_eq!(inv.items[&id(21)].parent, id(3));
        returned.owner_mask = 0x1234;
        inv.apply(vec![snapshot(id(2), vec![inv.items[&id(20)].clone(), returned])]);
        assert_eq!(inv.items[&id(21)].parent, id(2));
        assert_eq!(inv.items[&id(21)].owner_mask, 0x1234);
        assert_eq!(
            inv.folders[&id(2)].info.version,
            9,
            "the snapshot already accounts for its returned item"
        );
        assert!(inv.take_parent_repairs().is_none());
    }

    #[test]
    fn invalid_new_parents_and_existing_null_parents_follow_legacy_recovery() {
        let mut inv = fixture(FetchState::Fetched);
        inv.server_updated(Vec::new(), vec![object(20, id(999))]);
        assert_eq!(inv.items[&id(20)].parent, id(2));
        assert!(inv.take_parent_repairs().is_some());
        // A cached, already known null-parent record is lost; a genuinely
        // new simulator-created record still goes to its type folder.
        inv.items.insert(id(21), object(21, Uuid::nil()));
        inv.server_updated(Vec::new(), vec![object(21, Uuid::nil()), object(22, Uuid::nil())]);
        assert_eq!(inv.items[&id(21)].parent, id(2));
        assert_eq!(inv.items[&id(22)].parent, id(3));
        assert_eq!(inv.take_parent_repairs().expect("two repairs").operations.len(), 2);
    }

    #[test]
    fn missing_lost_folder_defers_recovery_until_the_real_folder_arrives() {
        let mut inv = fixture(FetchState::Fetched);
        inv.folders.remove(&id(2));
        inv.add_orphan_items((20..55).map(|n| object(n, Uuid::nil())).collect());
        assert!(inv.take_parent_repairs().is_none());
        assert!(inv.items[&id(20)].parent.is_nil());
        inv.server_updated(
            vec![InvFolder {
                id: id(2),
                parent: inv.root,
                type_default: 16,
                ..Default::default()
            }],
            Vec::new(),
        );
        let mut moved = 0;
        for _ in 0..3 {
            moved += inv.take_parent_repairs().map_or(0, |change| change.operations.len());
        }
        assert_eq!(moved, 35);
        assert!(inv.items.values().all(|it| it.parent == id(2)));
    }

    #[test]
    fn attachment_reads_preserve_known_folders_and_use_objects_for_new_originals() {
        let mut inv = fixture(FetchState::Fetched);
        let custom = id(4);
        let mut folder = inv.folders[&id(3)].clone();
        folder.info.id = custom;
        folder.info.type_default = -1;
        inv.folders.insert(custom, folder);
        inv.add_items(vec![object(20, custom)]);
        inv.add_attachment_items(vec![object(20, Uuid::nil()), object(21, Uuid::nil()), object(22, custom)]);
        assert_eq!(inv.items[&id(20)].parent, custom);
        assert_eq!(inv.items[&id(21)].parent, id(3));
        assert_eq!(inv.items[&id(22)].parent, custom);
        assert!(
            inv.folders[&id(2)].items.is_empty(),
            "ordinary attachments never use Lost And Found"
        );
        let change = inv.take_parent_repairs().expect("only parentless originals need repair");
        assert_eq!(change.operations.len(), 2);
        assert!(!inv.repaired_parents.contains_key(&id(22)), "a supplied valid parent needs no move");
    }

    #[test]
    fn attachment_parent_assignment_never_targets_lost_and_found_when_objects_is_unavailable() {
        let mut inv = fixture(FetchState::Fetched);
        inv.folders.remove(&id(3));
        inv.add_attachment_items(vec![object(20, Uuid::nil())]);
        assert!(inv.items[&id(20)].parent.is_nil());
        assert!(
            inv.take_parent_repairs().is_none(),
            "wait for the actual system folder instead of filing at root"
        );
        inv.server_updated(
            vec![InvFolder {
                id: id(3),
                parent: inv.root,
                type_default: 6,
                ..Default::default()
            }],
            Vec::new(),
        );
        assert!(inv.take_parent_repairs().is_some());
        assert_eq!(inv.items[&id(20)].parent, id(3));
        assert!(inv.folders[&id(2)].items.is_empty());
        let mut foreign = object(21, Uuid::nil());
        foreign.owner = id(99);
        inv.add_attachment_items(vec![foreign]);
        assert!(inv.items[&id(21)].parent.is_nil());
    }

    #[test]
    fn server_parentless_updates_keep_the_known_folder_or_use_the_asset_system_folder() {
        let mut inv = fixture(FetchState::Fetched);
        inv.add_items(vec![object(20, id(3))]);
        inv.server_updated(Vec::new(), vec![object(20, Uuid::nil()), object(21, Uuid::nil())]);
        assert_eq!(inv.items[&id(20)].parent, id(3));
        assert_eq!(inv.items[&id(21)].parent, id(3));
        assert!(inv.folders[&id(3)].items.contains(&id(20)) && inv.folders[&id(3)].items.contains(&id(21)));
        assert_eq!(inv.folders[&id(3)].info.version, 6, "account for the server update once");
        let change = inv.take_parent_repairs().expect("repair both records");
        assert_eq!(change.operations.len(), 2);
        assert!(change.move_only());
    }

    #[test]
    fn parent_repair_survives_an_older_folder_listing_and_finishes_on_confirmed_readback() {
        let mut inv = fixture(FetchState::Unknown);
        inv.add_items(vec![object(20, Uuid::nil())]);
        assert_eq!(inv.folders[&id(2)].state, FetchState::Fetching);
        assert!(inv.folders[&id(2)].items.contains(&id(20)));
        inv.take_parent_repairs().expect("move queued");
        inv.apply(vec![snapshot(id(2), Vec::new())]);
        assert!(inv.folders[&id(2)].items.contains(&id(20)));
        assert!(inv.repaired_parents.contains_key(&id(20)));
        inv.apply(vec![snapshot(id(2), vec![object(20, id(2))])]);
        assert_eq!(inv.folders[&id(2)].items, vec![id(20)]);
        assert!(!inv.repaired_parents.contains_key(&id(20)));
    }

    #[test]
    fn an_old_source_folder_cannot_undo_a_pending_parent_repair() {
        let mut inv = fixture(FetchState::Fetched);
        inv.add_items(vec![object(20, id(3))]);
        inv.add_items(vec![object(20, Uuid::nil())]);
        inv.apply(vec![snapshot(id(3), vec![object(20, id(3))])]);
        assert_eq!(inv.items[&id(20)].parent, id(2));
        assert!(!inv.folders[&id(3)].items.contains(&id(20)));
        assert!(inv.take_parent_repairs().is_some());
        // An explicit server move, unlike an older HTTP listing, wins.
        inv.server_updated(Vec::new(), vec![object(20, id(3))]);
        assert_eq!(inv.items[&id(20)].parent, id(3));
        assert!(!inv.repaired_parents.contains_key(&id(20)));
    }

    #[test]
    fn early_readback_does_not_cancel_a_repair_before_it_is_dispatched() {
        let mut inv = fixture(FetchState::Fetched);
        inv.add_items(vec![object(20, Uuid::nil())]);
        inv.apply(vec![snapshot(id(2), vec![object(20, id(2))])]);
        assert!(inv.take_parent_repairs().is_some());
    }

    #[test]
    fn foreign_library_and_unidentified_owners_never_authorize_parent_repair() {
        let mut inv = fixture(FetchState::Fetched);
        let mut foreign = object(20, Uuid::nil());
        foreign.owner = id(99);
        inv.add_items(vec![foreign]);
        assert!(inv.items[&id(20)].parent.is_nil());
        assert!(inv.take_parent_repairs().is_none());
        inv.owner = Uuid::nil();
        inv.add_items(vec![object(21, Uuid::nil())]);
        assert!(inv.items[&id(21)].parent.is_nil());
        assert!(inv.take_parent_repairs().is_none());
        inv.owner = id(10);
        for folder in inv.folders.values_mut() {
            folder.library = true;
        }
        inv.add_items(vec![object(22, Uuid::nil())]);
        assert!(inv.items[&id(22)].parent.is_nil());
        assert!(
            inv.take_parent_repairs().is_none(),
            "never file an owned record into library folders"
        );
    }

    #[test]
    fn consumed_parentless_records_never_send_a_late_parent_move() {
        let mut inv = fixture(FetchState::Fetched);
        inv.add_items(vec![object(20, Uuid::nil())]);
        inv.consume_item(id(20));
        assert!(!inv.items.contains_key(&id(20)));
        assert!(!inv.repaired_parents.contains_key(&id(20)));
        assert!(inv.take_parent_repairs().is_none());
    }

    #[test]
    fn orphan_repair_dispatch_is_bounded_and_deleted_records_are_not_sent() {
        let mut inv = fixture(FetchState::Fetched);
        inv.add_items((20..53).map(|n| object(n, Uuid::nil())).collect());
        let first = inv.take_parent_repairs().expect("first batch");
        assert_eq!(first.operations.len(), 32);
        let second = inv.take_parent_repairs().expect("last record");
        assert_eq!(second.operations.len(), 1);
        assert!(inv.take_parent_repairs().is_none());
        inv.add_items(vec![object(100, Uuid::nil())]);
        inv.remove(&[id(100)]);
        assert!(inv.take_parent_repairs().is_none());
    }
}
