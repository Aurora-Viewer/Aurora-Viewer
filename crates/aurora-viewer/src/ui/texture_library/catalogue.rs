//! A generation-cached view of actual texture/material items and folders in
//! the library. Links never extend this subtree into other inventory contents.

use crate::world::inventory::{FetchState, Inventory, actions};
use aurora_net::inventory::InvItem;
use std::collections::{HashSet, VecDeque};
use uuid::Uuid;

pub const AURORA: &str = "#Aurora";
pub const LIBRARY: &str = "Textures Library";
pub const BLANK_MATERIAL: Uuid = Uuid::from_u128(0x968cbad0_4dad_d64e_71b5_72bf13ad051a);

pub fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    inv.folders
        .values()
        .filter(|f| !f.library && f.info.parent == parent && f.info.name == name)
        .map(|f| f.info.id)
        .min()
}
pub fn root(inv: &Inventory) -> Option<Uuid> {
    child(inv, child(inv, inv.root, AURORA)?, LIBRARY)
}
pub fn resource(inv: &Inventory, id: Uuid) -> Option<&InvItem> {
    let original = actions::original(inv, id)?;
    inv.items.get(&original).filter(|item| matches!(item.asset_type, 0 | 57))
}
pub fn asset(item: &InvItem) -> Uuid {
    if item.asset_type == 57 && item.asset_id.is_nil() {
        BLANK_MATERIAL
    } else {
        item.asset_id
    }
}
pub fn folded(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            _ => c,
        })
        .collect()
}

#[derive(Clone)]
pub struct Folder {
    pub id: Uuid,
    pub label: String,
}
#[derive(Clone)]
pub struct Entry {
    /// Carry the actual inventory item so permissions are refreshed at drop.
    pub id: Uuid,
    pub original: Uuid,
    pub asset: Uuid,
    pub name: String,
    pub path: String,
    pub material: bool,
    pub thumbnail: Uuid,
    pub ancestors: Vec<Uuid>,
    search: String,
}
impl Entry {
    pub fn matches(&self, query: &str, folder: Option<Uuid>) -> bool {
        self.search.contains(query) && folder.is_none_or(|id| self.ancestors.contains(&id))
    }
}

#[derive(Default)]
pub struct Catalogue {
    key: Option<(Uuid, u64)>,
    pub root: Option<Uuid>,
    pub folders: Vec<Folder>,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub failed: Vec<Uuid>,
}
impl Catalogue {
    pub fn invalidate(&mut self) {
        self.key = None;
    }
    pub fn prepare(&mut self, inv: &mut Inventory) {
        if self.key == Some((inv.root, inv.generation)) {
            return;
        }
        self.key = Some((inv.root, inv.generation));
        self.root = root(inv);
        self.folders.clear();
        self.entries.clear();
        self.loading = false;
        self.failed.clear();
        let Some(root) = self.root else { return };
        let mut queue = VecDeque::from([(root, Vec::new(), String::new())]);
        let mut visited = HashSet::new();
        let mut fetches = 0;
        while let Some((id, mut ancestry, path)) = queue.pop_front() {
            if !visited.insert(id) || ancestry.len() > 64 {
                continue;
            }
            ancestry.push(id);
            let Some(folder) = inv.folders.get(&id) else { continue };
            if folder.state != FetchState::Fetched {
                if folder.state == FetchState::Failed {
                    self.failed.push(id);
                } else {
                    self.loading = true;
                }
                if fetches < 32 && folder.state == FetchState::Unknown {
                    inv.request(id);
                    fetches += 1;
                }
                continue;
            }
            for item in folder.items.iter().filter_map(|item| inv.items.get(item)) {
                // Inventory links are not library content, even when their
                // targets happen to be textures or materials inside the tree.
                if item.parent != id || !matches!(item.asset_type, 0 | 57) {
                    continue;
                }
                let asset = asset(item);
                if asset.is_nil() {
                    continue;
                }
                let name = item.name.clone();
                self.entries.push(Entry {
                    id: item.id,
                    original: item.id,
                    asset,
                    material: item.asset_type == 57,
                    thumbnail: item.thumbnail,
                    search: folded(&format!("{name} {path}")),
                    name,
                    path: path.clone(),
                    ancestors: ancestry.clone(),
                });
            }
            for child in folder.children.iter().filter_map(|child| inv.folders.get(child)) {
                if child.library || child.info.parent != id || visited.contains(&child.info.id) {
                    continue;
                }
                let label = if path.is_empty() {
                    child.info.name.clone()
                } else {
                    format!("{path} / {}", child.info.name)
                };
                self.folders.push(Folder {
                    id: child.info.id,
                    label: label.clone(),
                });
                queue.push_back((child.info.id, ancestry.clone(), label));
            }
        }
        self.entries
            .sort_by(|a, b| folded(&a.name).cmp(&folded(&b.name)).then(a.id.cmp(&b.id)));
        self.folders
            .sort_by(|a, b| folded(&a.label).cmp(&folded(&b.label)).then(a.id.cmp(&b.id)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder as InvFolder;
    use aurora_net::inventory::InvFolder as Info;
    fn folder(inv: &mut Inventory, id: u128, parent: Uuid, name: &str) -> Uuid {
        let id = Uuid::from_u128(id);
        inv.folders.insert(
            id,
            InvFolder {
                info: Info {
                    id,
                    parent,
                    name: name.into(),
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(f) = inv.folders.get_mut(&parent) {
            f.children.push(id);
        }
        id
    }
    fn fixture() -> (Inventory, Uuid) {
        let mut inv = Inventory::default();
        inv.root = folder(&mut inv, 1, Uuid::nil(), "Inventory");
        let parent = inv.root;
        let aurora = folder(&mut inv, 2, parent, AURORA);
        let root = folder(&mut inv, 3, aurora, LIBRARY);
        (inv, root)
    }
    fn item(inv: &mut Inventory, id: u128, parent: Uuid, kind: i32, name: &str, target: Uuid) {
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(id),
            parent,
            asset_id: target,
            asset_type: kind,
            name: name.into(),
            ..Default::default()
        }]);
    }
    #[test]
    fn library_ignores_bridge_and_unrelated_inventory() {
        let (mut inv, root) = fixture();
        let aurora = inv.folders[&root].info.parent;
        item(&mut inv, 10, aurora, 0, "Bridge texture", Uuid::from_u128(110));
        let bridge = folder(&mut inv, 4, aurora, "Bridge data");
        item(&mut inv, 13, bridge, 0, "Bridge only", Uuid::from_u128(113));
        item(&mut inv, 14, root, 25, "Linked bridge", bridge);
        item(&mut inv, 11, root, 0, "Béton brut", Uuid::from_u128(111));
        item(&mut inv, 12, root, 6, "An object", Uuid::from_u128(112));
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 1);
        assert!(catalogue.entries[0].matches(&folded("BETON"), None));
    }
    #[test]
    fn actual_nested_folders_are_included_and_folder_links_are_ignored() {
        let (mut inv, root) = fixture();
        let source = folder(&mut inv, 4, root, "Stone");
        let nested = folder(&mut inv, 5, source, "Nested");
        let parent = inv.root;
        let outside = folder(&mut inv, 6, parent, "Outside");
        // Neither external links nor links back into the tree add folders.
        item(&mut inv, 20, root, 25, "External link", outside);
        item(&mut inv, 21, nested, 25, "Loop link", source);
        item(&mut inv, 22, nested, 57, "Marble", Uuid::from_u128(122));
        item(&mut inv, 23, outside, 0, "External texture", Uuid::from_u128(123));
        // Ignore stale child lists, including a cycle, using actual parents.
        inv.folders.get_mut(&nested).unwrap().children.extend([root, outside]);
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 1);
        assert_eq!(
            catalogue.folders.iter().map(|f| f.id).collect::<HashSet<_>>(),
            HashSet::from([source, nested])
        );
        assert!(catalogue.entries[0].matches("marble", Some(source)));
        assert!(catalogue.entries[0].matches("nested", Some(nested)));
        assert_eq!(catalogue.entries[0].path, "Stone / Nested");
    }
    #[test]
    fn valid_and_broken_item_links_are_excluded_and_blank_materials_are_kept() {
        let (mut inv, root) = fixture();
        item(&mut inv, 30, root, 24, "Missing", Uuid::from_u128(999));
        item(&mut inv, 31, root, 57, "Blank", Uuid::nil());
        let parent = inv.root;
        item(&mut inv, 32, parent, 0, "External texture", Uuid::from_u128(132));
        item(&mut inv, 33, root, 24, "Valid texture link", Uuid::from_u128(32));
        item(&mut inv, 34, root, 24, "Valid material link", Uuid::from_u128(31));
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 1);
        assert_eq!(catalogue.entries[0].asset, BLANK_MATERIAL);
    }
    #[test]
    fn moving_actual_items_in_and_out_updates_the_library() {
        let (mut inv, root) = fixture();
        let outside = inv.root;
        item(&mut inv, 50, outside, 0, "Texture", Uuid::from_u128(150));
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert!(catalogue.entries.is_empty());
        let mut actual = inv.items[&Uuid::from_u128(50)].clone();
        actual.parent = root;
        inv.server_updated(Vec::new(), vec![actual.clone()]);
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 1);
        assert_eq!(catalogue.entries[0].id, actual.id);
        assert_eq!(catalogue.entries[0].asset, actual.asset_id);
        actual.parent = outside;
        inv.server_updated(Vec::new(), vec![actual]);
        catalogue.prepare(&mut inv);
        assert!(catalogue.entries.is_empty());
    }
    #[test]
    fn moving_a_real_folder_updates_all_its_nested_resources() {
        let (mut inv, root) = fixture();
        let outside = inv.root;
        let moved = folder(&mut inv, 7, outside, "Stone");
        let nested = folder(&mut inv, 8, moved, "Polished");
        item(&mut inv, 60, nested, 57, "Material", Uuid::from_u128(160));
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert!(catalogue.entries.is_empty());
        let mut info = inv.folders[&moved].info.clone();
        info.parent = root;
        inv.server_updated(vec![info.clone()], Vec::new());
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 1);
        assert!(catalogue.entries[0].matches("polished", Some(moved)));
        assert!(catalogue.entries[0].material);
        info.parent = outside;
        inv.server_updated(vec![info], Vec::new());
        catalogue.prepare(&mut inv);
        assert!(catalogue.entries.is_empty());
        assert!(catalogue.folders.is_empty());
    }
    #[test]
    fn an_unchanged_inventory_reuses_the_catalogue() {
        let (mut inv, root) = fixture();
        item(&mut inv, 40, root, 0, "Texture", Uuid::from_u128(140));
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        let storage = catalogue.entries.as_ptr();
        catalogue.prepare(&mut inv);
        assert_eq!(storage, catalogue.entries.as_ptr());
        inv.remove(&[Uuid::from_u128(40)]);
        catalogue.prepare(&mut inv);
        assert!(catalogue.entries.is_empty());
    }
}
