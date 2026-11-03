//! Startup repair of the authoritative skeleton and cached inventory, following
//! LLInventoryModel::buildParentChildMap (llinventorymodel.cpp, LGPL 2.1).
use super::{Inventory, parents::ItemSource};

impl Inventory {
    pub fn repair_lost_hierarchy(&mut self) {
        if self.hierarchy_repaired || self.owner.is_nil() || self.root.is_nil() {
            return;
        }
        self.hierarchy_repaired = true;
        self.repair_lost_folders();
        let mut items: Vec<_> = self
            .items
            .values()
            .filter(|it| it.owner == self.owner && (it.parent.is_nil() || !self.folders.contains_key(&it.parent)))
            .cloned()
            .collect();
        self.repair_item_parents(&mut items, ItemSource::Cache);
        self.add_items_inner(items);
        for folder in self.folders.values_mut().filter(|f| !f.library) {
            folder.items.clear();
        }
        for item in self.items.values() {
            if let Some(folder) = self.folders.get_mut(&item.parent).filter(|f| !f.library) {
                folder.items.push(item.id);
            }
        }
        self.sort_all();
    }

    pub fn repair_lost_folders(&mut self) {
        if self.owner.is_nil() || self.root.is_nil() {
            return;
        }
        let lost = self
            .folders
            .values()
            .filter(|f| !f.library && f.info.type_default == 16)
            .map(|f| f.info.id)
            .min();
        self.deferred_lost_folders = lost.is_none()
            && self.folders.values().any(|f| {
                !f.library
                    && f.info.id != self.root
                    && f.info.type_default < 0
                    && (f.info.parent.is_nil() || !self.folders.contains_key(&f.info.parent))
            });
        let repairs: Vec<_> = self
            .folders
            .values()
            .filter(|f| !f.library && f.info.id != self.root && (f.info.parent.is_nil() || !self.folders.contains_key(&f.info.parent)))
            .filter_map(|f| {
                let parent = if f.info.type_default >= 0 { Some(self.root) } else { lost }?;
                (parent != f.info.id).then_some((f.info.id, parent))
            })
            .collect();
        let changed = !repairs.is_empty();
        for (id, parent) in repairs {
            if let Some(folder) = self.folders.get_mut(&id) {
                folder.info.parent = parent;
            }
            if let Some(folder) = self.folders.get_mut(&parent) {
                folder.children.push(id);
            }
            self.repaired_folder_parents.insert(id, parent);
            self.pending_folder_repairs.insert(id);
        }
        // buildParentChildMap reconstructs the complete tree, including a
        // valid child whose newly received parent was inserted later.
        let children: Vec<_> = self
            .folders
            .values()
            .filter(|f| !f.library)
            .map(|f| (f.info.id, f.info.parent))
            .collect();
        for folder in self.folders.values_mut().filter(|f| !f.library) {
            folder.children.clear();
        }
        for (id, parent) in children {
            if let Some(folder) = self.folders.get_mut(&parent).filter(|f| !f.library) {
                folder.children.push(id);
            }
        }
        self.sort_all();
        if changed {
            self.generation += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{FolderContents, InvFolder, InvItem};
    use uuid::Uuid;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn folder(n: u128, parent: u128, kind: i32, library: bool) -> Folder {
        Folder {
            info: InvFolder {
                id: id(n),
                parent: id(parent),
                type_default: kind,
                version: 5,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library,
        }
    }

    #[test]
    fn startup_recovers_items_and_folders_but_keeps_system_roots_and_library_intact() {
        let mut inv = Inventory {
            owner: id(99),
            root: id(1),
            lib_root: id(90),
            ..Default::default()
        };
        for f in [
            folder(1, 0, 8, false),
            folder(2, 999, 16, false),
            folder(3, 0, 6, false),
            folder(4, 999, -1, false),
            folder(5, 4, -1, false),
            folder(90, 0, 8, true),
            folder(91, 999, -1, true),
        ] {
            inv.folders.insert(f.info.id, f);
        }
        for (n, parent, owner) in [(20, 999, 99), (21, 0, 99), (22, 4, 99), (23, 999, 100)] {
            inv.items.insert(
                id(n),
                InvItem {
                    id: id(n),
                    parent: id(parent),
                    owner: id(owner),
                    asset_type: 6,
                    ..Default::default()
                },
            );
        }
        inv.repair_lost_hierarchy();
        assert_eq!(inv.folders[&id(2)].info.parent, inv.root);
        assert_eq!(inv.folders[&id(3)].info.parent, inv.root);
        assert_eq!(inv.folders[&id(4)].info.parent, id(2));
        assert_eq!(inv.folders[&id(5)].info.parent, id(4));
        assert!(inv.folders[&id(4)].children.contains(&id(5)));
        assert!(inv.folders[&id(90)].info.parent.is_nil());
        assert_eq!(inv.folders[&id(91)].info.parent, id(999));
        assert_eq!(inv.items[&id(20)].parent, id(2));
        assert_eq!(inv.items[&id(21)].parent, id(2));
        assert_eq!(inv.items[&id(22)].parent, id(4));
        assert!(inv.folders[&id(4)].items.contains(&id(22)));
        assert_eq!(inv.items[&id(23)].parent, id(999));
        let moves = inv.take_parent_repairs().expect("repairs");
        assert_eq!(moves.operations.len(), 5);
        inv.repair_lost_hierarchy();
        assert!(inv.take_parent_repairs().is_none(), "once per authoritative skeleton/cache load");
        // Older root listings cannot remove a repaired system folder or
        // restore the old parent of a recovered user folder.
        inv.apply(vec![FolderContents {
            folder_id: id(1),
            owner_id: id(99),
            version: 99,
            folders: Vec::new(),
            items: Vec::new(),
        }]);
        assert!(inv.folders[&id(1)].children.contains(&id(2)));
        assert!(inv.folders[&id(1)].children.contains(&id(3)));
    }
}
