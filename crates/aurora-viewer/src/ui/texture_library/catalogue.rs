//! A generation-cached view of the library's inventory subtree, including
//! folder and item links. Never walk the bridge or unrelated inventory contents.

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
    /// Keep the inventory link as cargo so permissions are refreshed at drop.
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
        let aurora = inv.folders[&root].info.parent;
        let mut queue = VecDeque::from([(root, root, root, Vec::new(), String::new())]);
        let mut visited = HashSet::new();
        let mut fetches = 0;
        while let Some((target, logical, collection, mut ancestry, path)) = queue.pop_front() {
            // A linked folder can point back at its source or another link.
            if !visited.insert((target, collection)) || ancestry.len() > 64 {
                continue;
            }
            ancestry.push(logical);
            let Some(folder) = inv.folders.get(&target) else { continue };
            if folder.state != FetchState::Fetched {
                if folder.state == FetchState::Failed {
                    self.failed.push(target);
                } else {
                    self.loading = true;
                }
                if fetches < 32 && folder.state == FetchState::Unknown {
                    inv.request(target);
                    fetches += 1;
                }
                continue;
            }
            let folder = &inv.folders[&target];
            let mut descendants: Vec<(Uuid, Uuid, String)> = folder
                .children
                .iter()
                .filter_map(|id| {
                    let f = inv.folders.get(id)?;
                    Some((*id, *id, f.info.name.clone()))
                })
                .collect();
            for id in &folder.items {
                let Some(item) = inv.items.get(id) else { continue };
                if item.asset_type == 25 {
                    if let Some(original) = actions::original(inv, *id).filter(|id| inv.folders.contains_key(id)) {
                        descendants.push((*id, original, item.name.clone()));
                    }
                    continue;
                }
                let Some(original) = resource(inv, *id) else { continue };
                let asset = asset(original);
                if asset.is_nil() {
                    continue;
                }
                let name = item.name.clone();
                self.entries.push(Entry {
                    id: *id,
                    original: original.id,
                    asset,
                    material: original.asset_type == 57,
                    thumbnail: original.thumbnail,
                    search: folded(&format!("{name} {path}")),
                    name,
                    path: path.clone(),
                    ancestors: ancestry.clone(),
                });
            }
            descendants.sort_by_key(|(_, _, name)| folded(name));
            for (id, target, name) in descendants {
                if actions::under(inv, target, aurora) && !actions::under(inv, target, root) {
                    continue;
                }
                let label = if path.is_empty() { name } else { format!("{path} / {name}") };
                self.folders.push(Folder { id, label: label.clone() });
                queue.push_back((target, id, if logical == root { id } else { collection }, ancestry.clone(), label));
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
    fn folder_links_include_descendants_and_stop_cycles() {
        let (mut inv, root) = fixture();
        let parent = inv.root;
        let source = folder(&mut inv, 4, parent, "Source");
        let nested = folder(&mut inv, 5, source, "Nested");
        item(&mut inv, 20, root, 25, "Stone", source);
        item(&mut inv, 21, nested, 25, "Loop", source);
        item(&mut inv, 22, nested, 57, "Marble", Uuid::from_u128(122));
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 1);
        assert!(catalogue.entries[0].matches("marble", Some(Uuid::from_u128(20))));
        assert!(catalogue.entries[0].matches("nested", Some(nested)));
    }
    #[test]
    fn broken_item_links_are_excluded_and_blank_materials_are_kept() {
        let (mut inv, root) = fixture();
        item(&mut inv, 30, root, 24, "Missing", Uuid::from_u128(999));
        item(&mut inv, 31, root, 57, "Blank", Uuid::nil());
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 1);
        assert_eq!(catalogue.entries[0].asset, BLANK_MATERIAL);
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
