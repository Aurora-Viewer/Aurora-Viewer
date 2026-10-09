//! Client-side inventory model: login skeleton + lazily fetched folders.

use aurora_llsd::Llsd;
use aurora_net::inventory::{FolderContents, InvFolder, InvItem, parse_skeleton};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
#[cfg(test)]
#[path = "inventory/tests.rs"]
mod action_tests;
pub mod actions;
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
    pub root: Uuid,
    pub lib_root: Uuid,
    pub lib_owner: Uuid,
    pub folders: HashMap<Uuid, Folder>,
    pub items: HashMap<Uuid, InvItem>,
    /// Folders waiting to be fetched (id, library).
    pub queue: Vec<(Uuid, bool)>,
    /// Bumped whenever folders or items change (cached views rebuild).
    pub generation: u64,
}

fn first_id(v: &Llsd, key: &str) -> Uuid {
    v.at(0)[key].as_uuid()
}

impl Inventory {
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

    fn upsert_folder(&mut self, f: InvFolder, library: bool) {
        match self.folders.get_mut(&f.id) {
            Some(existing) => existing.info = f,
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
        kids.sort_by(|a, b| {
            let fa = self.folders.get(a);
            let fb = self.folders.get(b);
            let sys = |f: Option<&Folder>| f.map(|f| f.info.type_default >= 0 && f.info.type_default != 8).unwrap_or(false);
            let name = |f: Option<&Folder>| f.map(|f| f.info.name.to_lowercase()).unwrap_or_default();
            sys(fb).cmp(&sys(fa)).then_with(|| name(fa).cmp(&name(fb)))
        });
        let mut items = f.items.clone();
        seen.clear();
        // FetchInventoryDescendents2 may include linked originals from other
        // folders. LLInventoryModel::updateItem indexes each by its parent.
        items.retain(|item| self.items.get(item).is_some_and(|it| it.parent == id) && seen.insert(*item));
        items.sort_by_key(|i| self.items.get(i).map(|it| it.name.to_lowercase()).unwrap_or_default());
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
        for it in items {
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

    pub fn failed(&mut self, folders: &[Uuid]) {
        for id in folders {
            if let Some(f) = self.folders.get_mut(id) {
                f.state = FetchState::Failed;
            }
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
            "version" => 2,
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
    /// skeleton (LLInventoryModel::loadSkeleton): they need no fetch.
    pub fn load_cache(&mut self, path: &std::path::Path) -> usize {
        let Ok(bytes) = crate::cache::read_touch(path) else {
            return 0;
        };
        let Ok((doc, _)) = aurora_llsd::from_binary(&bytes) else {
            return 0;
        };
        if doc["version"].as_i32() != 2 {
            return 0;
        }
        let mut valid = std::collections::HashSet::new();
        for f in doc["folders"].as_array() {
            let id = f["id"].as_uuid();
            if let Some(cur) = self.folders.get_mut(&id)
                && cur.state == FetchState::Unknown
                && cur.info.version == f["version"].as_i32()
                && cur.info.version >= 0
            {
                valid.insert(id);
                cur.info.favorite = f["favorite"].as_bool();
                cur.info.thumbnail = f["thumbnail"].as_uuid();
            }
        }
        let mut by_parent: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for v in doc["items"].as_array() {
            let parent = v["parent"].as_uuid();
            if !valid.contains(&parent) {
                continue;
            }
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
            if it.id.is_nil() {
                continue;
            }
            by_parent.entry(parent).or_default().push(it.id);
            self.items.insert(it.id, it);
        }
        for id in &valid {
            if let Some(f) = self.folders.get_mut(id) {
                f.items = by_parent.remove(id).unwrap_or_default();
                f.state = FetchState::Fetched;
            }
        }
        let ids: Vec<Uuid> = valid.iter().copied().collect();
        for id in ids {
            self.sort_children(id);
        }
        self.generation += 1;
        valid.len()
    }
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
                    name: "Inventaire".into(),
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
        assert!(restored.load_cache(&path) > 0);
        assert!(restored.folders[&Uuid::from_u128(704)].info.favorite);
        assert_eq!(restored.folders[&Uuid::from_u128(704)].info.thumbnail, Uuid::from_u128(999));
        std::fs::remove_file(path).expect("synthetic cache cleanup");
        assert!(!restored.folders[&Uuid::from_u128(704)].items.contains(&item));
        assert_eq!(restored.folders[&restored.root].items.iter().filter(|id| **id == item).count(), 1);
    }
}
