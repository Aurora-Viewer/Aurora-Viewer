//! Inventory sorting and filtering rules from Firestorm LLInventorySort,
//! LLInventoryFilter and LLFolderBridge::getSortGroup (indra/newview,
//! originally LGPL 2.1), including SHOW_NON_EMPTY_FOLDERS used by
//! LLPanelMainInventory. Views are cached by inventory generation and inputs;
//! parallel sorting/filtering never changes server folder membership.

use super::{Facts, Inventory, folder_name, rules};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const TYPES: &[(i32, &str)] = &[
    (19, "Animations"),
    (2, "Cartes de visite"),
    (18, "Vêtements et corps"),
    (20, "Gestes"),
    (3, "Repères"),
    (26, "Matériaux"),
    (7, "Notes"),
    (6, "Objets"),
    (10, "Scripts"),
    (1, "Sons"),
    (0, "Textures"),
    (15, "Photos"),
    (25, "Paramètres EEP"),
    (22, "Maillages"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sort {
    pub by_date: bool,
    pub folders_by_name: bool,
    pub system_first: bool,
}
impl Default for Sort {
    fn default() -> Self {
        Self {
            by_date: true,
            folders_by_name: true,
            system_first: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchField {
    #[default]
    Name,
    Description,
    Creator,
    Uuid,
    All,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Links {
    #[default]
    Include,
    Only,
    Exclude,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Creator {
    #[default]
    All,
    Me,
    Others,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Age {
    #[default]
    Any,
    SinceLogout,
    Newer,
    Older,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub types: u32,
    pub permissions: u32,
    pub links: Links,
    pub creator: Creator,
    pub age: Age,
    pub hours: u32,
    pub always_folders: bool,
    pub coalesced: bool,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            types: u32::MAX,
            permissions: 0,
            links: Links::Include,
            creator: Creator::All,
            age: Age::Any,
            hours: 24,
            always_folders: false,
            coalesced: false,
        }
    }
}
impl Filters {
    pub fn active(&self) -> bool {
        self.types != u32::MAX
            || self.permissions != 0
            || self.links != Links::Include
            || self.creator != Creator::All
            || self.age != Age::Any
            || self.always_folders
            || self.coalesced
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InventoryPreferences {
    pub protected: HashSet<Uuid>,
    pub upload_folders: [Uuid; 5],
    pub sort: Sort,
    pub search_field: SearchField,
    pub search_trash: bool,
    pub search_library: bool,
    pub search_outfits: bool,
    pub show_library: bool,
    pub show_recent: bool,
    pub show_worn: bool,
    pub show_favorites: bool,
    pub separate_searches: bool,
    pub double_click_add_objects: bool,
    pub double_click_add_clothes: bool,
    pub filter_defaults: Filters,
    pub last_logout: i64,
}
impl Default for InventoryPreferences {
    fn default() -> Self {
        Self {
            protected: HashSet::new(),
            upload_folders: [Uuid::nil(); 5],
            sort: Sort::default(),
            search_field: SearchField::Name,
            search_trash: false,
            search_library: true,
            search_outfits: true,
            show_library: true,
            show_recent: true,
            show_worn: true,
            show_favorites: true,
            separate_searches: false,
            double_click_add_objects: false,
            double_click_add_clothes: false,
            filter_defaults: Filters::default(),
            last_logout: 0,
        }
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

struct Entry {
    id: Uuid,
    parent: Uuid,
    name: String,
    desc: String,
    date: i64,
    group: u8,
    folder: bool,
    trash: bool,
    outfit: bool,
    library: bool,
}

#[derive(PartialEq, Eq)]
struct Query {
    root: Uuid,
    text: String,
    field: SearchField,
    trash: bool,
    library: bool,
    outfits: bool,
    filters: Filters,
    tab: usize,
    target: Option<Uuid>,
    agent: Uuid,
    cutoff: i64,
    worn: HashSet<Uuid>,
    names: HashMap<Uuid, String>,
}

#[derive(Default)]
pub struct ViewCache {
    version: Option<(Uuid, Uuid, u64, Sort)>,
    entries: Vec<Entry>,
    children: HashMap<Uuid, Vec<Uuid>>,
    result_children: HashMap<Uuid, Vec<Uuid>>,
    folders: HashSet<Uuid>,
    revision: u64,
    query: Option<Query>,
    pub results: Vec<Uuid>,
}

impl ViewCache {
    pub fn prepare(&mut self, inv: &Inventory, sort: Sort) {
        let version = (inv.root, inv.lib_root, inv.generation, sort);
        if self.version == Some(version) {
            return;
        }
        // A folder has no server creation date. Firestorm uses its newest
        // descendant. A guarded ancestor walk also handles malformed cycles.
        let mut dates: HashMap<Uuid, i64> = HashMap::new();
        for item in inv.items.values() {
            let mut parent = item.parent;
            let mut seen = HashSet::new();
            while seen.insert(parent) {
                let Some(folder) = inv.folders.get(&parent) else {
                    break;
                };
                dates
                    .entry(parent)
                    .and_modify(|d| *d = (*d).max(item.created_at))
                    .or_insert(item.created_at);
                parent = folder.info.parent;
            }
        }
        let trash = rules::system(inv, 14);
        let outfits = [rules::system(inv, 46), rules::system(inv, 48)];
        let metadata = |id: Uuid, parent, name, desc, date, group, folder| Entry {
            id,
            parent,
            name,
            desc,
            date,
            group,
            folder,
            trash: trash.is_some_and(|root| rules::under(inv, id, root)),
            outfit: outfits.iter().flatten().any(|root| rules::under(inv, id, *root)),
            library: rules::library(inv, id),
        };
        self.entries = inv
            .folders
            .par_iter()
            .map(|(id, f)| {
                let kind = f.info.type_default;
                // FT_OUTFIT, merchant folders, obsolete outbox and ensembles are
                // ordinary editable folders, despite having a nonnegative type.
                let protected = matches!(kind, 0..=3 | 5..=7 | 8 | 10 | 13 | 15 | 16 | 20 | 21 | 23 | 46 | 48 | 49 | 50 | 52 | 56 | 57);
                let group = if *id == inv.lib_root {
                    4
                } else if kind == 14 {
                    1
                } else if protected {
                    0
                } else {
                    2
                };
                metadata(
                    *id,
                    f.info.parent,
                    folder_name(inv, *id).to_lowercase(),
                    String::new(),
                    dates.get(id).copied().unwrap_or(0),
                    group,
                    true,
                )
            })
            .chain(inv.items.par_iter().map(|(id, it)| {
                metadata(
                    *id,
                    it.parent,
                    it.name.to_lowercase(),
                    it.desc.to_lowercase(),
                    it.created_at,
                    3,
                    false,
                )
            }))
            .collect();
        self.entries.par_sort_unstable_by(|a, b| compare(a, b, sort));
        self.folders = inv.folders.keys().copied().collect();
        self.children.clear();
        for entry in &self.entries {
            self.children.entry(entry.parent).or_default().push(entry.id);
        }
        // The library stays last, without modifying its server parent or
        // re-sorting the system folders alphabetically with ordinary folders.
        if !inv.lib_root.is_nil() && inv.lib_root != inv.root && inv.folders.contains_key(&inv.lib_root) {
            let root = self.children.entry(inv.root).or_default();
            root.retain(|id| *id != inv.lib_root);
            root.push(inv.lib_root);
        }
        self.version = Some(version);
        self.query = None;
    }

    pub fn children(&self, parent: Uuid, prefs: &InventoryPreferences, lib_root: Uuid) -> Vec<Uuid> {
        self.children
            .get(&parent)
            .into_iter()
            .flatten()
            .filter(|id| prefs.show_library || **id != lib_root)
            .copied()
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &mut self,
        inv: &Inventory,
        root: Uuid,
        text: &str,
        prefs: &InventoryPreferences,
        filters: &Filters,
        tab: usize,
        target: Option<Uuid>,
        facts: &Facts,
        agent: Uuid,
        time: i64,
    ) {
        self.prepare(inv, prefs.sort);
        let age = if tab == 2 && filters.age == Age::Any {
            Age::SinceLogout
        } else {
            filters.age
        };
        let cutoff = match age {
            Age::Any => 0,
            Age::SinceLogout if prefs.last_logout > 0 => prefs.last_logout,
            Age::SinceLogout => time / 60 * 60 - 86400,
            // Refresh relative dates once a minute, not once per frame.
            _ => time / 60 * 60 - i64::from(filters.hours.min(876_000)) * 3600,
        };
        let text = text.trim().to_lowercase();
        let uses_creator = matches!(prefs.search_field, SearchField::Creator | SearchField::All) && !text.is_empty();
        let names = if uses_creator { facts.names.clone() } else { HashMap::new() };
        let worn = if tab == 3 { facts.worn.clone() } else { HashSet::new() };
        let query = Query {
            root,
            text,
            field: prefs.search_field,
            trash: prefs.search_trash,
            library: prefs.search_library,
            outfits: prefs.search_outfits,
            filters: filters.clone(),
            tab,
            target,
            agent,
            cutoff,
            worn,
            names,
        };
        if self.query.as_ref() == Some(&query) {
            return;
        }
        self.results = self
            .entries
            .par_iter()
            .filter(|e| {
                if root == inv.root && !rules::under(inv, e.id, inv.root) && !rules::under(inv, e.id, inv.lib_root) {
                    return false;
                }
                if e.id == root || e.id == inv.lib_root || (root != inv.root && !rules::under(inv, e.id, root)) {
                    return false;
                }
                if (!query.trash && e.trash) || (!query.library && e.library) || (!query.outfits && e.outfit) {
                    return false;
                }
                if e.folder {
                    if query.target.is_some() {
                        return false;
                    }
                    if filters.always_folders {
                        return true;
                    }
                    if matches!(tab, 2 | 3) || (tab == 4 && !inv.folders.get(&e.id).is_some_and(|f| f.info.favorite)) {
                        return false;
                    }
                    return !filters.active()
                        && match query.field {
                            SearchField::Name => matches_text(&e.name, &query.text),
                            SearchField::Uuid => matches_text(&e.id.to_string(), &query.text),
                            SearchField::All => matches_text(&format!("{} {}", e.name, e.id), &query.text),
                            SearchField::Description | SearchField::Creator => query.text.is_empty(),
                        };
                }
                let Some(it) = inv.items.get(&e.id) else {
                    return false;
                };
                let linked = matches!(it.asset_type, 24 | 25);
                if (filters.links == Links::Only && !linked) || (filters.links == Links::Exclude && linked) {
                    return false;
                }
                if query.target.is_some_and(|target| !linked || it.asset_id != target) {
                    return false;
                }
                let original_id = rules::original(inv, e.id).unwrap_or(e.id);
                let original = inv.items.get(&original_id).unwrap_or(it);
                let kind = if original.inv_type == 17 { 6 } else { original.inv_type };
                if filters.types != u32::MAX && !(0..32).contains(&kind) {
                    return false;
                }
                if filters.types != u32::MAX && filters.types & (1_u32 << kind) == 0 {
                    return false;
                }
                if original.owner_mask & filters.permissions != filters.permissions {
                    return false;
                }
                if filters.coalesced && (original.asset_type != 6 || original.flags & 0x200000 == 0) {
                    return false;
                }
                if (filters.creator == Creator::Me && original.creator != agent)
                    || (filters.creator == Creator::Others && original.creator == agent)
                {
                    return false;
                }
                if age != Age::Any && (e.date <= 0 || if age == Age::Older { e.date > cutoff } else { e.date < cutoff }) {
                    return false;
                }
                if tab == 3 && (!query.worn.contains(&original_id) || e.outfit) {
                    return false;
                }
                if tab == 4 && !it.favorite {
                    return false;
                }
                let creator = || query.names.get(&original.creator).map(|s| s.to_lowercase()).unwrap_or_default();
                match query.field {
                    SearchField::Name => matches_text(&e.name, &query.text),
                    SearchField::Description => matches_text(&e.desc, &query.text),
                    SearchField::Creator => matches_text(&creator(), &query.text),
                    SearchField::Uuid => matches_text(&e.id.to_string(), &query.text),
                    SearchField::All => matches_text(&format!("{} {} {} {}", e.name, e.desc, creator(), e.id), &query.text),
                }
            })
            .map(|e| e.id)
            .collect();
        // Every matching item retains its real ancestors. Folder visibility
        // is independent of whether the folder itself passes item filters.
        let mut included = HashSet::new();
        for id in &self.results {
            let mut parent = *id;
            let mut seen = HashSet::new();
            while parent != root && seen.insert(parent) {
                included.insert(parent);
                if parent == inv.lib_root && root == inv.root {
                    break;
                }
                let Some(next) = rules::parent(inv, parent) else { break };
                parent = next;
            }
        }
        self.result_children.clear();
        for (parent, children) in &self.children {
            let children: Vec<_> = children.iter().filter(|id| included.contains(id)).copied().collect();
            if !children.is_empty() {
                self.result_children.insert(*parent, children);
            }
        }
        self.revision = self.revision.wrapping_add(1);
        self.query = Some(query);
    }
}

#[derive(Clone, Copy)]
pub struct TreeRow {
    pub id: Uuid,
    pub depth: usize,
    pub folder: bool,
    pub open: bool,
}

/// Flatten only the expanded branches for virtual drawing. Collapsed folders
/// belong to each tab, independently of the unfiltered inventory tree.
#[derive(Default)]
pub struct TreeState {
    key: Option<(u64, Uuid, usize, bool)>,
    closed: [HashSet<Uuid>; 5],
    pub rows: Vec<TreeRow>,
    pub order: Vec<Uuid>,
}
impl TreeState {
    pub fn prepare(&mut self, view: &ViewCache, root: Uuid, tab: usize, show_root: bool) -> bool {
        let tab = tab.min(4);
        let key = (view.revision, root, tab, show_root);
        if self.key == Some(key) {
            return false;
        }
        self.rows.clear();
        self.order.clear();
        let mut todo = vec![(root, 0)];
        let mut seen = HashSet::new();
        while let Some((id, depth)) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            let folder = view.folders.contains(&id);
            let open = folder && !self.closed[tab].contains(&id);
            if id != root || show_root {
                self.rows.push(TreeRow { id, depth, folder, open });
                self.order.push(id);
            }
            if (open || (id == root && !show_root))
                && let Some(children) = view.result_children.get(&id)
            {
                let depth = depth + usize::from(id != root || show_root);
                todo.extend(children.iter().rev().map(|id| (*id, depth)));
            }
        }
        if view.results.is_empty() {
            self.rows.clear();
            self.order.clear();
        }
        self.key = Some(key);
        true
    }

    pub fn toggle(&mut self, id: Uuid, tab: usize) {
        let closed = &mut self.closed[tab.min(4)];
        if !closed.remove(&id) {
            closed.insert(id);
        }
        self.key = None;
    }

    pub fn reveal(&mut self, inv: &Inventory, id: Uuid, tab: usize) {
        let closed = &mut self.closed[tab.min(4)];
        let mut parent = rules::parent(inv, id);
        let mut seen = HashSet::new();
        let mut changed = false;
        while let Some(id) = parent {
            if !seen.insert(id) {
                break;
            }
            changed |= closed.remove(&id);
            parent = rules::parent(inv, id);
        }
        if changed {
            self.key = None;
        }
    }

    pub fn set_all(&mut self, view: &ViewCache, root: Uuid, tab: usize, open: bool) {
        let closed = &mut self.closed[tab.min(4)];
        closed.clear();
        if !open {
            closed.extend(view.result_children.keys().filter(|id| **id != root).copied());
        }
        self.key = None;
    }
}

fn compare(a: &Entry, b: &Entry, sort: Sort) -> Ordering {
    if a.group == 4 || b.group == 4 {
        return a.group.cmp(&b.group).then_with(|| a.id.cmp(&b.id));
    }
    let groups = if sort.system_first {
        a.group.cmp(&b.group)
    } else if sort.by_date && !sort.folders_by_name {
        (a.group == 1).cmp(&(b.group == 1))
    } else {
        b.folder.cmp(&a.folder)
    };
    let by_name = !sort.by_date || (sort.folders_by_name && a.folder && b.folder);
    groups
        .then_with(|| {
            if by_name {
                natural_cmp(&a.name, &b.name).then_with(|| b.date.cmp(&a.date))
            } else {
                b.date.cmp(&a.date).then_with(|| natural_cmp(&a.name, &b.name))
            }
        })
        .then_with(|| a.id.cmp(&b.id))
}

/// Dictionary order: numeric runs compare numerically ("2" before "10").
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while let (Some(&x), Some(&y)) = (a.first(), b.first()) {
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let (an, bn) = (
                a.iter().take_while(|c| c.is_ascii_digit()).count(),
                b.iter().take_while(|c| c.is_ascii_digit()).count(),
            );
            let (av, bv) = (&a[..an], &b[..bn]);
            let (az, bz) = (
                av.iter().take_while(|c| **c == b'0').count(),
                bv.iter().take_while(|c| **c == b'0').count(),
            );
            let order = (an - az)
                .cmp(&(bn - bz))
                .then_with(|| av[az..].cmp(&bv[bz..]))
                .then_with(|| an.cmp(&bn));
            if order != Ordering::Equal {
                return order;
            }
            a = &a[an..];
            b = &b[bn..];
        } else {
            let order = x.cmp(&y);
            if order != Ordering::Equal {
                return order;
            }
            a = &a[1..];
            b = &b[1..];
        }
    }
    a.len().cmp(&b.len())
}

fn matches_text(value: &str, query: &str) -> bool {
    if let Some(exact) = query.strip_prefix('"').and_then(|q| q.strip_suffix('"')) {
        return value.split_whitespace().any(|word| word == exact);
    }
    query.split('+').all(|token| value.contains(token.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn folder(inv: &mut Inventory, n: u128, parent: Uuid, kind: i32, name: &str) {
        inv.folders.insert(
            id(n),
            Folder {
                info: InvFolder {
                    id: id(n),
                    parent,
                    type_default: kind,
                    name: name.into(),
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: n == 3,
            },
        );
        inv.generation += 1;
    }
    fn item(n: u128, parent: Uuid, name: &str, date: i64) -> InvItem {
        InvItem {
            id: id(n),
            parent,
            name: name.into(),
            created_at: date,
            creator: id(99),
            asset_type: 6,
            inv_type: 6,
            owner_mask: rules::COPY | rules::MODIFY | rules::TRANSFER,
            ..Default::default()
        }
    }
    fn fixture() -> (Inventory, Facts) {
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(3),
            ..Default::default()
        };
        for (n, kind, name) in [
            (1, 8, "root"),
            (2, -1, "A personal"),
            (3, 8, "library"),
            (4, 0, "Textures"),
            (5, 14, "Trash"),
            (6, 47, "A outfit"),
        ] {
            let parent = if n == 1 || n == 3 { Uuid::nil() } else { inv.root };
            folder(&mut inv, n, parent, kind, name);
        }
        inv.add_items(vec![
            item(10, id(2), "A older", 100),
            item(11, id(2), "Z newest", 300),
            item(12, id(2), "Unknown date", 0),
        ]);
        let facts = Facts {
            agent: id(99),
            worn: HashSet::new(),
            worn_labels: Default::default(),
            points: vec![],
            appearance_busy: false,
            names: HashMap::new(),
        };
        (inv, facts)
    }
    fn search(inv: &Inventory, facts: &Facts, prefs: &InventoryPreferences, filters: &Filters, tab: usize) -> Vec<Uuid> {
        let mut cache = ViewCache::default();
        cache.search(inv, inv.root, "", prefs, filters, tab, None, facts, facts.agent, 1000);
        cache.results
    }

    #[test]
    fn defaults_keep_system_folders_first_and_items_newest_even_with_library() {
        let (inv, _) = fixture();
        let prefs = InventoryPreferences::default();
        let mut cache = ViewCache::default();
        cache.prepare(&inv, prefs.sort);
        assert_eq!(cache.children(inv.root, &prefs, inv.lib_root), [id(4), id(5), id(6), id(2), id(3)]);
        assert_eq!(cache.children(id(2), &prefs, inv.lib_root), [id(11), id(10), id(12)]);
        let hidden = InventoryPreferences {
            show_library: false,
            ..prefs
        };
        assert!(!cache.children(inv.root, &hidden, inv.lib_root).contains(&id(3)));
    }

    #[test]
    fn changing_sort_and_inventory_invalidates_the_cached_order() {
        let (mut inv, _) = fixture();
        let mut prefs = InventoryPreferences::default();
        let mut cache = ViewCache::default();
        cache.prepare(&inv, prefs.sort);
        prefs.sort.by_date = false;
        cache.prepare(&inv, prefs.sort);
        assert_eq!(cache.children(id(2), &prefs, inv.lib_root), [id(10), id(12), id(11)]);
        prefs.sort.by_date = true;
        inv.add_items(vec![item(10, id(2), "A older", 400)]);
        cache.prepare(&inv, prefs.sort);
        assert_eq!(cache.children(id(2), &prefs, inv.lib_root)[0], id(10));
        inv.remove(&[id(10)]);
        cache.prepare(&inv, prefs.sort);
        assert!(!cache.children(id(2), &prefs, inv.lib_root).contains(&id(10)));
    }

    #[test]
    fn folder_dates_use_the_newest_nested_descendant() {
        let (mut inv, _) = fixture();
        folder(&mut inv, 7, id(6), -1, "nested");
        inv.add_items(vec![item(13, id(7), "newer nested", 500)]);
        let prefs = InventoryPreferences {
            sort: Sort {
                folders_by_name: false,
                ..Sort::default()
            },
            ..Default::default()
        };
        let mut cache = ViewCache::default();
        cache.prepare(&inv, prefs.sort);
        let root = cache.children(inv.root, &prefs, inv.lib_root);
        assert!(root.iter().position(|n| *n == id(6)) < root.iter().position(|n| *n == id(2)));
        assert!(natural_cmp("folder 2", "folder 10").is_lt());
    }

    #[test]
    fn type_permissions_creator_and_coalesced_filters_use_link_originals() {
        let (mut inv, facts) = fixture();
        let mut original = item(11, id(2), "Z newest", 300);
        original.flags = 0x200000;
        original.owner_mask = rules::COPY;
        inv.add_items(vec![
            original,
            InvItem {
                id: id(14),
                parent: id(2),
                asset_type: 24,
                inv_type: 6,
                asset_id: id(11),
                name: "link".into(),
                ..Default::default()
            },
        ]);
        let prefs = InventoryPreferences::default();
        let mut filters = Filters {
            types: 1 << 6,
            permissions: rules::COPY,
            links: Links::Only,
            creator: Creator::Me,
            coalesced: true,
            ..Default::default()
        };
        assert_eq!(search(&inv, &facts, &prefs, &filters, 0), [id(14)]);
        filters.permissions |= rules::MODIFY;
        assert!(search(&inv, &facts, &prefs, &filters, 0).is_empty());
        filters.permissions = 0;
        filters.creator = Creator::Others;
        assert!(search(&inv, &facts, &prefs, &filters, 0).is_empty());
        filters.creator = Creator::Me;
        filters.types = 1 << 0;
        assert!(search(&inv, &facts, &prefs, &filters, 0).is_empty());
    }

    #[test]
    fn recent_and_age_filters_exclude_unknown_dates_and_use_last_logout() {
        let (inv, facts) = fixture();
        let prefs = InventoryPreferences {
            last_logout: 200,
            ..Default::default()
        };
        assert_eq!(search(&inv, &facts, &prefs, &Filters::default(), 2), [id(11)]);
        let filters = Filters {
            age: Age::Older,
            hours: 0,
            ..Default::default()
        };
        assert_eq!(search(&inv, &facts, &prefs, &filters, 0), [id(11), id(10)]);
    }

    #[test]
    fn filtered_items_keep_all_ancestors_without_unrelated_items() {
        let (mut inv, facts) = fixture();
        folder(&mut inv, 7, id(2), -1, "nested textures");
        let mut texture = item(20, id(7), "texture", 500);
        texture.inv_type = 0;
        texture.asset_type = 0;
        inv.add_items(vec![texture]);
        let prefs = InventoryPreferences::default();
        let filters = Filters {
            types: 1,
            ..Default::default()
        };
        let mut view = ViewCache::default();
        view.search(&inv, inv.root, "", &prefs, &filters, 0, None, &facts, facts.agent, 1000);
        let mut tree = TreeState::default();
        tree.prepare(&view, inv.root, 0, true);
        assert_eq!(view.results, [id(20)]);
        assert_eq!(tree.order, [id(1), id(2), id(7), id(20)]);
        assert_eq!(tree.rows.last().unwrap().depth, 3);
        tree.set_all(&view, inv.root, 0, false);
        tree.prepare(&view, inv.root, 0, true);
        assert_eq!(tree.order, [id(1), id(2)]);
        tree.toggle(id(2), 0);
        tree.prepare(&view, inv.root, 0, true);
        assert_eq!(tree.order, [id(1), id(2), id(7)]);
        tree.reveal(&inv, id(20), 0);
        tree.prepare(&view, inv.root, 0, true);
        assert!(tree.order.contains(&id(20)));
        tree.prepare(&view, id(2), 0, false);
        assert_eq!(tree.order, [id(7), id(20)]);
    }

    #[test]
    fn recent_and_worn_use_real_folders_and_separate_collapse_states() {
        let (inv, mut facts) = fixture();
        let prefs = InventoryPreferences {
            last_logout: 200,
            ..Default::default()
        };
        let mut view = ViewCache::default();
        let mut tree = TreeState::default();
        view.search(&inv, inv.root, "", &prefs, &Filters::default(), 2, None, &facts, facts.agent, 1000);
        tree.prepare(&view, inv.root, 2, true);
        assert_eq!(tree.order, [id(1), id(2), id(11)]);
        tree.toggle(id(2), 2);
        tree.prepare(&view, inv.root, 2, true);
        assert_eq!(tree.order, [id(1), id(2)]);
        facts.worn.insert(id(10));
        view.search(&inv, inv.root, "", &prefs, &Filters::default(), 3, None, &facts, facts.agent, 1000);
        tree.prepare(&view, inv.root, 3, true);
        assert_eq!(tree.order, [id(1), id(2), id(10)]);
        assert!(!tree.rows.last().unwrap().folder);
    }

    #[test]
    fn scopes_exclude_trash_library_and_outfit_links_and_limit_folder_search() {
        let (mut inv, mut facts) = fixture();
        let root = inv.root;
        folder(&mut inv, 8, root, 48, "Outfits");
        inv.add_items(vec![
            item(15, id(5), "trash", 500),
            item(16, id(3), "library", 600),
            item(17, id(8), "outfit link", 700),
        ]);
        let prefs = InventoryPreferences {
            search_library: false,
            search_outfits: false,
            ..Default::default()
        };
        let filters = Filters {
            types: 1 << 6,
            ..Default::default()
        };
        assert_eq!(search(&inv, &facts, &prefs, &filters, 0), [id(11), id(10), id(12)]);
        let mut cache = ViewCache::default();
        cache.search(
            &inv,
            id(2),
            "",
            &InventoryPreferences::default(),
            &filters,
            0,
            None,
            &facts,
            facts.agent,
            1000,
        );
        assert_eq!(cache.results, [id(11), id(10), id(12)]);
        facts.worn.extend([id(11), id(17)]);
        assert_eq!(
            search(&inv, &facts, &InventoryPreferences::default(), &Filters::default(), 3),
            [id(11)]
        );
    }

    #[test]
    fn search_fields_tokens_and_name_arrival_invalidate_results() {
        let (mut inv, mut facts) = fixture();
        let mut original = inv.items[&id(11)].clone();
        original.desc = "a violet jacket".into();
        inv.add_items(vec![original]);
        let mut prefs = InventoryPreferences {
            search_field: SearchField::Description,
            ..Default::default()
        };
        let mut cache = ViewCache::default();
        cache.search(
            &inv,
            inv.root,
            "violet+jacket",
            &prefs,
            &Filters::default(),
            0,
            None,
            &facts,
            facts.agent,
            1000,
        );
        assert_eq!(cache.results, [id(11)]);
        prefs.search_field = SearchField::Creator;
        cache.search(
            &inv,
            inv.root,
            "alice",
            &prefs,
            &Filters::default(),
            0,
            None,
            &facts,
            facts.agent,
            1000,
        );
        assert!(cache.results.is_empty());
        facts.names.insert(id(99), "Alice Doe".into());
        cache.search(
            &inv,
            inv.root,
            "alice",
            &prefs,
            &Filters::default(),
            0,
            None,
            &facts,
            facts.agent,
            1000,
        );
        assert_eq!(cache.results, [id(11), id(10), id(12)]);
        for field in [SearchField::Uuid, SearchField::All] {
            prefs.search_field = field;
            cache.search(
                &inv,
                inv.root,
                &id(2).to_string(),
                &prefs,
                &Filters::default(),
                0,
                None,
                &facts,
                facts.agent,
                1000,
            );
            assert_eq!(cache.results, [id(2)]);
        }
        let filters = Filters {
            always_folders: true,
            types: 0,
            ..Default::default()
        };
        for tab in [2, 3, 4] {
            assert!(search(&inv, &facts, &prefs, &filters, tab).contains(&id(2)));
        }
        assert!(matches_text("a coat", "\"coat\""));
        assert!(!matches_text("a raincoat", "\"coat\""));
    }

    #[test]
    fn every_match_is_available_beyond_the_old_500_result_limit() {
        let (mut inv, facts) = fixture();
        inv.add_items((100..1600).map(|n| item(n, id(2), "matching item", n as i64)).collect());
        let mut cache = ViewCache::default();
        cache.search(
            &inv,
            inv.root,
            "matching",
            &InventoryPreferences::default(),
            &Filters::default(),
            0,
            None,
            &facts,
            facts.agent,
            2000,
        );
        assert_eq!(cache.results.len(), 1500);
        assert_eq!(cache.results.first(), Some(&id(1599)));
        assert_eq!(cache.results.last(), Some(&id(100)));
    }

    #[test]
    fn old_preferences_keep_protected_and_upload_folders_and_get_new_defaults() {
        let prefs: InventoryPreferences =
            serde_json::from_str(r#"{"protected":["00000000-0000-0000-0000-000000000002"]}"#).expect("old preferences");
        assert!(prefs.protected.contains(&id(2)));
        assert_eq!(prefs.sort, Sort::default());
        let changed = InventoryPreferences {
            sort: Sort {
                by_date: false,
                system_first: false,
                folders_by_name: false,
            },
            filter_defaults: Filters {
                links: Links::Exclude,
                age: Age::Newer,
                hours: 48,
                ..Default::default()
            },
            ..prefs
        };
        let json = serde_json::to_string(&changed).expect("serialize preferences");
        assert_eq!(
            serde_json::from_str::<InventoryPreferences>(&json).expect("reload preferences"),
            changed
        );
    }
}
