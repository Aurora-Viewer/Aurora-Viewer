//! A generation-cached view of actual supported inventory items and folders in
//! the library. Links never extend this subtree into other inventory contents.

use crate::world::inventory::{FetchState, Inventory, actions};
use aurora_net::inventory::InvItem;
use std::collections::{HashSet, VecDeque};
use uuid::Uuid;

pub const AURORA: &str = "#Aurora";
pub const LIBRARY: &str = "Aurora Library";
pub const LEGACY_TEXTURES: &str = "Textures Library";
pub const OBJECTS_LIBRARY: &str = "Objects Library";
pub const DEFAULT_FOLDERS: [&str; 8] = [
    "Textures",
    "Materials",
    "Objects",
    "Animations",
    "Sounds",
    "Notecards",
    "Landmarks",
    "Scripts",
];
pub const BLANK_MATERIAL: Uuid = Uuid::from_u128(0x968cbad0_4dad_d64e_71b5_72bf13ad051a);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Texture,
    Material,
    Object,
    ObjectGroup,
    Sound,
    Notecard,
    Landmark,
    Animation,
    Script,
}
impl ResourceKind {
    fn item(item: &InvItem) -> Option<Self> {
        match item.asset_type {
            0 => Some(Self::Texture),
            57 => Some(Self::Material),
            1 => Some(Self::Sound),
            7 => Some(Self::Notecard),
            3 => Some(Self::Landmark),
            20 => Some(Self::Animation),
            10 => Some(Self::Script),
            // LLInventoryIcon::getIconName, using
            // II_FLAGS_OBJECT_HAS_MULTIPLE_ITEMS (Firestorm, LGPL 2.1).
            6 if item.flags & super::super::inventory::OBJECT_HAS_MULTIPLE_ITEMS != 0 => Some(Self::ObjectGroup),
            6 => Some(Self::Object),
            _ => None,
        }
    }
    pub const TYPES: [Self; 8] = [
        Self::Texture,
        Self::Material,
        Self::Object,
        Self::Animation,
        Self::Sound,
        Self::Notecard,
        Self::Landmark,
        Self::Script,
    ];
    pub fn category(self) -> Self {
        if self == Self::ObjectGroup { Self::Object } else { self }
    }
    pub fn surface(self) -> bool {
        matches!(self, Self::Texture | Self::Material)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Texture => aurora_i18n::tr!("texture-library-textures"),
            Self::Material => aurora_i18n::tr!("texture-library-pbr"),
            Self::Object | Self::ObjectGroup => aurora_i18n::tr!("texture-library-objects"),
            Self::Animation => aurora_i18n::tr!("texture-library-animations"),
            Self::Sound => aurora_i18n::tr!("texture-library-sounds"),
            Self::Notecard => aurora_i18n::tr!("texture-library-notecards"),
            Self::Landmark => aurora_i18n::tr!("texture-library-landmarks"),
            Self::Script => aurora_i18n::tr!("texture-library-scripts"),
        }
    }
    pub fn object(self) -> bool {
        matches!(self, Self::Object | Self::ObjectGroup)
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Texture => "Inv_Texture",
            Self::Material => "Inv_Material",
            Self::Object => "Inv_Object",
            Self::ObjectGroup => "Inv_Object_Multi",
            Self::Sound => "Inv_Sound",
            Self::Notecard => "Inv_Notecard",
            Self::Landmark => "Inv_Landmark",
            Self::Animation => "Inv_Animation",
            Self::Script => "Inv_Script",
        }
    }
}

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
/// Follow actual parent IDs; folder names inside the library have no type meaning.
pub fn contains(inv: &Inventory, item: Uuid) -> bool {
    let Some(root) = root(inv) else { return false };
    let Some(mut parent) = inv.items.get(&item).map(|it| it.parent) else {
        return false;
    };
    for _ in 0..64 {
        if parent == root {
            return true;
        }
        let Some(folder) = inv.folders.get(&parent) else { return false };
        parent = folder.info.parent;
    }
    false
}
/// Surface resources only: object cargo must reach the inventory rez path.
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
    pub kind: ResourceKind,
    pub thumbnail: Uuid,
    pub ancestors: Vec<Uuid>,
    search: String,
}
impl Entry {
    pub fn image(&self) -> Uuid {
        if self.kind == ResourceKind::Texture {
            self.asset
        } else {
            self.thumbnail
        }
    }
    pub fn matches(&self, query: &str, folder: Option<Uuid>) -> bool {
        self.search.contains(query) && folder.is_none_or(|id| self.ancestors.contains(&id))
    }
}

#[derive(Default)]
pub struct Catalogue {
    key: Option<(Uuid, u64)>,
    pub parent: Option<Uuid>,
    pub roots: Vec<Uuid>,
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
        self.parent = root(inv);
        self.roots.clear();
        if let Some(parent) = self.parent {
            self.roots.push(parent);
        }
        self.folders.clear();
        self.entries.clear();
        self.loading = false;
        self.failed.clear();
        let mut queue = VecDeque::new();
        for root in &self.roots {
            let Some(folder) = inv.folders.get(root) else { continue };
            let label = folder.info.name.clone();
            self.folders.push(Folder {
                id: *root,
                label: label.clone(),
            });
            queue.push_back((*root, Vec::new(), label));
        }
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
                // targets happen to be supported resources inside the tree.
                if item.parent != id {
                    continue;
                }
                let Some(kind) = ResourceKind::item(item) else { continue };
                let asset = asset(item);
                // Object asset UUIDs may be hidden by the simulator. Rez uses
                // the inventory item, and its thumbnail is an independent asset.
                if asset.is_nil() && kind == ResourceKind::Texture {
                    continue;
                }
                let name = item.name.clone();
                self.entries.push(Entry {
                    id: item.id,
                    original: item.id,
                    asset,
                    kind,
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
        assert_eq!(catalogue.entries.len(), 2);
        assert!(catalogue.entries.iter().any(|entry| entry.matches(&folded("BETON"), None)));
    }
    #[test]
    fn every_type_is_indexed_by_metadata_in_arbitrary_folders() {
        let (mut inv, root) = fixture();
        let objects = folder(&mut inv, 4, root, "Objects");
        let custom = folder(&mut inv, 5, objects, "Custom");
        for (n, parent) in [root, custom].into_iter().enumerate() {
            for (k, kind) in [0, 57, 6, 1, 7, 3, 20, 10].into_iter().enumerate() {
                let number = 100 + n as u128 * 10 + k as u128;
                item(
                    &mut inv,
                    number,
                    parent,
                    kind,
                    &format!("Resource {number}"),
                    Uuid::from_u128(200 + k as u128),
                );
            }
        }
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.parent, Some(root));
        assert_eq!(catalogue.roots, vec![root]);
        assert_eq!(catalogue.entries.len(), 16);
        assert_eq!(
            catalogue
                .entries
                .iter()
                .filter(|entry| entry.matches("resource", Some(objects)))
                .count(),
            8
        );
        assert!(catalogue.entries.iter().all(|entry| contains(&inv, entry.id)));
        assert!(
            catalogue
                .entries
                .iter()
                .all(|entry| super::super::super::icons::embedded(entry.kind.icon()))
        );
        let mut moved = inv.items[&Uuid::from_u128(100)].clone();
        moved.parent = custom;
        inv.server_updated(Vec::new(), vec![moved]);
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 16);
        let entry = catalogue.entries.iter().find(|entry| entry.id == Uuid::from_u128(100)).unwrap();
        assert_eq!(entry.kind, ResourceKind::Texture);
        assert!(entry.matches("objects", Some(objects)));
        inv.folders.get_mut(&custom).unwrap().state = FetchState::Unknown;
        inv.generation += 1;
        catalogue.prepare(&mut inv);
        assert!(catalogue.loading);
        assert!(inv.queue.iter().any(|(id, _)| *id == custom));
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
            HashSet::from([root, source, nested])
        );
        assert!(catalogue.entries[0].matches("marble", Some(source)));
        assert!(catalogue.entries[0].matches("nested", Some(nested)));
        assert_eq!(catalogue.entries[0].path, "Aurora Library / Stone / Nested");
    }
    #[test]
    fn library_objects_use_thumbnails_and_distinct_single_and_group_fallbacks() {
        let (mut inv, root) = fixture();
        let nested = folder(&mut inv, 4, root, "Objects");
        for (n, flags, thumbnail) in [(70, 0, 170), (71, 0, 0), (72, 0x200000, 0), (73, 0x200000, 173)] {
            inv.add_items(vec![InvItem {
                id: Uuid::from_u128(n),
                parent: nested,
                asset_type: 6,
                // A hidden asset UUID must not hide a valid rez source.
                asset_id: Uuid::nil(),
                thumbnail: Uuid::from_u128(thumbnail),
                flags,
                name: format!("Object {n}"),
                ..Default::default()
            }]);
        }
        item(&mut inv, 74, root, 24, "Object link", Uuid::from_u128(70));
        let mut catalogue = Catalogue::default();
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries.len(), 4);
        for (entry, image, icon) in [
            (&catalogue.entries[0], 170, "Inv_Object"),
            (&catalogue.entries[1], 0, "Inv_Object"),
            (&catalogue.entries[2], 0, "Inv_Object_Multi"),
            (&catalogue.entries[3], 173, "Inv_Object_Multi"),
        ] {
            assert!(entry.kind.object());
            assert!(entry.matches("object", Some(nested)));
            assert_eq!(entry.image(), Uuid::from_u128(image));
            assert_eq!(entry.kind.icon(), icon);
            assert!(super::super::super::icons::embedded(icon));
            assert!(resource(&inv, entry.id).is_none());
        }
        // Inventory acknowledgements update the image and group flag in place.
        let mut updated = inv.items[&Uuid::from_u128(71)].clone();
        updated.thumbnail = Uuid::from_u128(171);
        updated.flags = 0x200000;
        inv.server_updated(Vec::new(), vec![updated]);
        catalogue.prepare(&mut inv);
        assert_eq!(catalogue.entries[1].image(), Uuid::from_u128(171));
        assert_eq!(catalogue.entries[1].kind, ResourceKind::ObjectGroup);
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
        assert_eq!(catalogue.entries[0].kind, ResourceKind::Material);
        info.parent = outside;
        inv.server_updated(vec![info], Vec::new());
        catalogue.prepare(&mut inv);
        assert!(catalogue.entries.is_empty());
        assert_eq!(catalogue.folders.len(), 1);
        assert_eq!(catalogue.folders[0].id, root);
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
