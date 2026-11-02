//! System-folder lookup shared by inventory and appearance rules.
//!
//! Preserve LLInventoryModel::findCategoryUUIDForTypeInRoot (llinventorymodel.cpp):
//! prefer a listed root child, with the smallest UUID for duplicate types.
//! Keep the existing fallback for an incomplete root listing. A large root can
//! contain thousands of ordinary folders; walking it for every visible row
//! defeats the inventory and Appearance list virtualization (#1963).

use super::Inventory;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Cache {
    snapshot: Mutex<Option<Snapshot>>,
    #[cfg(test)]
    builds: std::sync::atomic::AtomicUsize,
}

struct Snapshot {
    key: (Uuid, u64, usize, Option<usize>),
    folders: HashMap<i32, Uuid>,
}

impl Inventory {
    pub fn system_folder(&self, kind: i32) -> Option<Uuid> {
        let root = self.folders.get(&self.root);
        let key = (self.root, self.generation, self.folders.len(), root.map(|f| f.children.len()));
        let mut cached = self.system_folders.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
        if cached.as_ref().is_none_or(|snapshot| snapshot.key != key) {
            #[cfg(test)]
            self.system_folders.builds.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut folders = HashMap::new();
            for f in root
                .into_iter()
                .flat_map(|root| &root.children)
                .filter_map(|id| self.folders.get(id))
            {
                if f.info.parent == self.root && !f.library {
                    folders
                        .entry(f.info.type_default)
                        .and_modify(|id: &mut Uuid| *id = (*id).min(f.info.id))
                        .or_insert(f.info.id);
                }
            }
            // HashMap iteration order intentionally matches the previous fallback.
            for f in self.folders.values().filter(|f| !f.library) {
                folders.entry(f.info.type_default).or_insert(f.info.id);
            }
            *cached = Some(Snapshot { key, folders });
        }
        cached.as_ref().and_then(|snapshot| snapshot.folders.get(&kind)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::InvFolder;
    use std::sync::atomic::Ordering;

    fn folder(id: u128, parent: Uuid, kind: i32, library: bool) -> Folder {
        Folder {
            info: InvFolder {
                id: Uuid::from_u128(id),
                parent,
                type_default: kind,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library,
        }
    }

    fn reference(inv: &Inventory, kind: i32) -> Option<Uuid> {
        let own = |f: &&Folder| f.info.type_default == kind && !f.library;
        inv.folders
            .get(&inv.root)
            .into_iter()
            .flat_map(|f| &f.children)
            .filter_map(|id| inv.folders.get(id))
            .filter(|f| f.info.parent == inv.root)
            .filter(own)
            .map(|f| f.info.id)
            .min()
            .or_else(|| inv.folders.values().find(own).map(|f| f.info.id))
    }

    #[test]
    fn system_folder_index_preserves_duplicates_library_and_incomplete_root_rules() {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        for (id, parent, kind, library) in [
            (1, Uuid::nil(), 8, false),
            (2, inv.root, 46, false),
            (3, inv.root, 46, false),
            (4, inv.root, 46, true),
            (5, Uuid::nil(), 46, false),
            (6, inv.root, 50, false),
        ] {
            inv.folders.insert(Uuid::from_u128(id), folder(id, parent, kind, library));
        }
        inv.folders.get_mut(&inv.root).expect("root").children =
            vec![Uuid::from_u128(3), Uuid::from_u128(4), Uuid::from_u128(5), Uuid::from_u128(2)];
        for kind in -1..=60 {
            assert_eq!(inv.system_folder(kind), reference(&inv, kind), "kind {kind}");
        }
        assert_eq!(inv.system_folder(46), Some(Uuid::from_u128(2)));
        assert_eq!(inv.system_folder(50), Some(Uuid::from_u128(6)), "unlisted folder fallback");
        assert_eq!(
            inv.system_folders.builds.load(Ordering::Relaxed),
            1,
            "all types and missing types share one scan"
        );
    }

    #[test]
    fn system_folder_index_rechecks_missing_types_library_changes_and_deletions() {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        inv.folders.insert(inv.root, folder(1, Uuid::nil(), 8, false));
        inv.folders.insert(Uuid::from_u128(2), folder(2, inv.root, -1, false));
        inv.folders.get_mut(&inv.root).expect("root").children.push(Uuid::from_u128(2));
        assert_eq!(inv.system_folder(46), None);
        inv.folders.get_mut(&Uuid::from_u128(2)).expect("folder").info.type_default = 46;
        inv.generation += 1;
        assert_eq!(inv.system_folder(46), Some(Uuid::from_u128(2)));
        inv.folders.get_mut(&Uuid::from_u128(2)).expect("folder").library = true;
        inv.generation += 1;
        assert_eq!(inv.system_folder(46), None);
        inv.folders.remove(&Uuid::from_u128(2));
        inv.generation += 1;
        assert_eq!(inv.system_folder(46), None);
        assert_eq!(inv.system_folders.builds.load(Ordering::Relaxed), 4);
    }
}
