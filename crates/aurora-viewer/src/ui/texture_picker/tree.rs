//! LLFloaterTexturePicker's filtered inventory hierarchy, using the same
//! cached sorting, ancestor filtering and virtual rows as the inventory.
//! Fetches follow LLInventoryModelBackgroundFetch: bounded batches, including
//! nested texture folders, rather than only the system Textures folder.

use super::{INV_MATERIAL, INV_SNAPSHOT, INV_TEXTURE, inventory_asset};
use crate::ui::inventory::{Facts, InventoryPreferences, view};
use crate::world::inventory::{FetchState, Inventory};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Tree {
    pub view: view::ViewCache,
    pub state: view::TreeState,
    known_generation: Option<u64>,
    known_folders: HashSet<Uuid>,
    initialized_roots: HashSet<Uuid>,
    reveal: Option<Uuid>,
    reveal_generation: Option<u64>,
    scan_generation: Option<u64>,
    pending: Vec<Uuid>,
    pub loading: bool,
}

impl Tree {
    pub fn opened(&mut self, current: Uuid) {
        self.reveal = Some(current);
        self.reveal_generation = None;
        self.scan_generation = None;
    }

    #[cfg(test)]
    pub fn prepare(&mut self, inv: &mut Inventory, agent: Uuid, materials: bool, query: &str) {
        self.prepare_root(inv, agent, materials, query, inv.root, view::Sort::default());
    }

    pub fn prepare_root(&mut self, inv: &mut Inventory, agent: Uuid, materials: bool, query: &str, root: Uuid, sort: view::Sort) {
        self.fetch(inv);
        let prefs = InventoryPreferences {
            sort,
            ..Default::default()
        };
        let filters = view::Filters {
            types: if materials {
                1 << INV_MATERIAL
            } else {
                (1 << INV_TEXTURE) | (1 << INV_SNAPSHOT)
            },
            ..Default::default()
        };
        self.view
            .search(inv, root, query, &prefs, &filters, 0, None, &Facts::default(), agent, 0);
        if self.known_generation != Some(inv.generation) {
            // The initial tree opens the selected texture's ancestors only.
            // Search has separate expansion state so clearing it restores
            // the branches the user was browsing, as in LLTextureCtrl.
            self.state
                .collapse_folders(inv.folders.keys().filter(|id| !self.known_folders.contains(id)).copied(), 0);
            self.known_folders.extend(inv.folders.keys().copied());
            self.known_generation = Some(inv.generation);
        }
        if self.initialized_roots.insert(root) {
            self.state.expand_folder(root, 0);
        }
        if let Some(asset) = self.reveal
            && self.reveal_generation != Some(inv.generation)
        {
            self.reveal_generation = Some(inv.generation);
            let item = inv
                .items
                .values()
                .filter(|it| inventory_asset(inv, it.id, materials) == Some(asset))
                .min_by_key(|it| it.id);
            if let Some(item) = item {
                let mut id = item.parent;
                let mut seen = HashSet::new();
                while seen.insert(id) {
                    self.state.expand_folder(id, 0);
                    let Some(parent) = inv.folders.get(&id) else { break };
                    id = parent.info.parent;
                }
                self.reveal = None;
            }
        }
        self.state.prepare(&self.view, root, usize::from(!query.trim().is_empty()), true);
    }

    fn fetch(&mut self, inv: &mut Inventory) {
        if self.scan_generation == Some(inv.generation) && !self.loading && self.pending.is_empty() {
            return;
        }
        if self.scan_generation != Some(inv.generation) {
            self.pending = inv
                .folders
                .iter()
                .filter(|(_, f)| f.state == FetchState::Unknown)
                .map(|(&id, _)| id)
                .collect();
            // Load the roots and system texture folders first. Requests for
            // new descendants are picked up on the next inventory update.
            self.pending.sort_by_key(|id| {
                let folder = &inv.folders[id];
                let priority = if *id == inv.root || *id == inv.lib_root {
                    0
                } else if matches!(folder.info.type_default, 0 | 15 | 57) {
                    1
                } else {
                    2
                };
                (std::cmp::Reverse(priority), *id)
            });
            self.scan_generation = Some(inv.generation);
        }
        let mut in_flight = inv.folders.values().filter(|f| f.state == FetchState::Fetching).count();
        let mut requested = 0;
        while in_flight < 32 && requested < 8 {
            let Some(id) = self.pending.pop() else { break };
            if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Unknown) {
                inv.request(id);
                in_flight += 1;
                requested += 1;
            }
        }
        self.loading = !self.pending.is_empty() || in_flight != 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn folder(inv: &mut Inventory, n: u128, parent: u128, name: &str) {
        inv.folders.insert(
            id(n),
            Folder {
                info: InvFolder {
                    id: id(n),
                    parent: id(parent),
                    type_default: -1,
                    name: name.into(),
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: n >= 90,
            },
        );
        inv.generation += 1;
    }
    fn fixture() -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(90),
            ..Default::default()
        };
        for (n, parent, name) in [
            (1, 0, "Inventory"),
            (2, 1, "Textures"),
            (3, 2, "Wood"),
            (4, 1, "Project"),
            (90, 0, "Library"),
            (91, 90, "Stone"),
        ] {
            folder(&mut inv, n, parent, name);
        }
        for (n, parent, name, kind) in [
            (10, 3, "Oak very long descriptive texture name", INV_TEXTURE),
            (11, 4, "Oak project texture", INV_SNAPSHOT),
            (12, 4, "Material", INV_MATERIAL),
            (92, 91, "Granite", INV_TEXTURE),
        ] {
            inv.items.insert(
                id(n),
                InvItem {
                    id: id(n),
                    asset_id: id(n + 100),
                    parent: id(parent),
                    name: name.into(),
                    inv_type: kind,
                    ..Default::default()
                },
            );
        }
        inv
    }
    fn rows(tree: &Tree) -> Vec<Uuid> {
        tree.state.rows.iter().map(|r| r.id).collect()
    }

    #[test]
    fn picker_opens_selected_ancestors_and_search_preserves_real_folders() {
        let mut inv = fixture();
        let mut tree = Tree::default();
        tree.opened(id(110));
        tree.prepare(&mut inv, id(99), false, "");
        assert_eq!(rows(&tree), vec![id(1), id(4), id(2), id(3), id(10), id(90)]);
        tree.prepare(&mut inv, id(99), false, "oak");
        assert!(rows(&tree).contains(&id(11)));
        assert_eq!(tree.state.rows.iter().find(|r| r.id == id(10)).unwrap().depth, 3);
        assert!(!rows(&tree).contains(&id(12)));
        tree.prepare(&mut inv, id(99), false, "");
        assert!(!rows(&tree).contains(&id(11)), "search must not expand normal browsing");
        tree.state.toggle(id(90), 0);
        tree.state.expand_folder(id(91), 0);
        tree.prepare(&mut inv, id(99), false, "");
        assert!(rows(&tree).contains(&id(92)), "library textures keep their hierarchy");
    }

    #[test]
    fn picker_filters_materials_and_retains_a_new_folders_collapsed_state() {
        let mut inv = fixture();
        let mut tree = Tree::default();
        tree.opened(id(112));
        tree.prepare(&mut inv, id(99), true, "");
        assert!(rows(&tree).contains(&id(12)));
        assert!(!rows(&tree).contains(&id(10)));
        folder(&mut inv, 5, 4, "New nested folder");
        inv.add_items(vec![InvItem {
            id: id(15),
            parent: id(5),
            asset_id: id(115),
            inv_type: INV_MATERIAL,
            name: "Material 2".into(),
            ..Default::default()
        }]);
        tree.prepare(&mut inv, id(99), true, "");
        assert!(rows(&tree).contains(&id(5)));
        assert!(!rows(&tree).contains(&id(15)));
    }

    #[test]
    fn inventory_root_stays_collapsed_across_frames_and_inventory_updates() {
        let mut inv = fixture();
        let mut tree = Tree::default();
        tree.opened(id(110));
        tree.prepare(&mut inv, id(99), false, "");
        tree.state.toggle(inv.root, 0);
        for _ in 0..3 {
            tree.prepare(&mut inv, id(99), false, "");
            assert_eq!(rows(&tree), vec![inv.root]);
            inv.generation += 1;
        }
        tree.state.toggle(inv.root, 0);
        tree.prepare(&mut inv, id(99), false, "");
        assert!(rows(&tree).contains(&id(10)));
    }

    #[test]
    fn picker_fetch_batches_are_bounded_and_do_not_retry_failed_folders() {
        let mut inv = fixture();
        for n in 1000..1100 {
            folder(&mut inv, n, 1, "Pending");
            inv.folders.get_mut(&id(n)).unwrap().state = FetchState::Unknown;
        }
        inv.folders.get_mut(&id(1099)).unwrap().state = FetchState::Failed;
        let mut tree = Tree::default();
        tree.opened(id(110));
        for frame in 0..10 {
            let before = inv.queue.len();
            tree.prepare(&mut inv, id(99), false, "");
            assert!(inv.queue.len() - before <= 8, "frame {frame}");
            assert!(inv.queue.len() <= 32);
        }
        assert_eq!(inv.queue.len(), 32);
        assert!(!inv.queue.iter().any(|(n, _)| *n == id(1099)));
        assert!(tree.loading);
    }
}
