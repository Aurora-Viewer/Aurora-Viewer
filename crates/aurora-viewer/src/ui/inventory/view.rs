//! Inventory sorting and filtering rules from Firestorm LLInventorySort,
//! LLInventoryFilter and LLFolderBridge::getSortGroup (indra/newview,
//! originally LGPL 2.1), including SHOW_NON_EMPTY_FOLDERS used by
//! LLPanelMainInventory. Views are cached by inventory generation and inputs;
//! parallel sorting/filtering never changes server folder membership.

use super::smart::{Field as SmartField, Matcher, Subject};
use super::{Facts, Inventory, folder_name, rules};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[test]
fn received_filing_view_pins_marketplace_and_scopes_sender_search_without_server_moves() {
    use aurora_net::inventory::{InvFolder, InvItem};
    let agent = Uuid::from_u128(1);
    let mut inv = Inventory {
        root: Uuid::from_u128(90_000),
        ..Default::default()
    };
    inv.created_folders(vec![InvFolder {
        id: inv.root,
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    crate::world::inventory::demo::seed(&mut inv, agent);
    crate::world::inventory::demo::seed_received(&mut inv, agent);
    let inbox = rules::system(&inv, 50).unwrap();
    let base = Uuid::from_u128(90_001);
    let received = Uuid::from_u128(90_002);
    let sender = Uuid::from_u128(90_003);
    let gift = Uuid::from_u128(90_004);
    inv.created_folders(vec![
        InvFolder {
            id: base,
            parent: inv.root,
            name: "#Aurora".into(),
            type_default: -1,
            ..Default::default()
        },
        InvFolder {
            id: received,
            parent: base,
            name: "Received items".into(),
            type_default: -1,
            ..Default::default()
        },
        InvFolder {
            id: sender,
            parent: received,
            name: "Aaa Resident".into(),
            type_default: -1,
            ..Default::default()
        },
    ]);
    inv.add_items(vec![InvItem {
        id: gift,
        parent: sender,
        name: "Unique received texture".into(),
        asset_type: 0,
        inv_type: 0,
        ..Default::default()
    }]);
    for by_date in [false, true] {
        for system_first in [false, true] {
            let prefs = InventoryPreferences {
                received_sort: Sort {
                    by_date,
                    folders_by_name: !by_date,
                    system_first,
                },
                ..Default::default()
            };
            for query in ["", "Unique received texture", "no matching gift"] {
                let mut view = ViewCache::default();
                view.search(
                    &inv,
                    inv.root,
                    query,
                    &prefs,
                    &Filters::default(),
                    super::RECEIVED_TAB,
                    None,
                    &Facts::default(),
                    agent,
                    now(),
                );
                let mut tree = TreeState::default();
                tree.prepare(&view, inv.root, super::RECEIVED_TAB, false);
                assert_eq!(tree.rows[0].id, inbox, "Marketplace stays first for every sort and search");
                assert!(!tree.order.contains(&base));
                assert!(!tree.order.contains(&received));
                assert!(!tree.order.contains(&Uuid::from_u128(8100)), "ordinary inventory is excluded");
                assert_eq!(tree.order.contains(&gift), query != "no matching gift");
                if query != "no matching gift" {
                    assert!(tree.order.contains(&sender));
                }
                assert_eq!(inv.folders[&inbox].info.parent, inv.root);
                assert_eq!(inv.folders[&sender].info.parent, received);
            }
        }
    }
}

pub fn types() -> &'static Vec<(i32, &'static str)> {
    static TABLE: aurora_i18n::Localized<Vec<(i32, &'static str)>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            (19, aurora_i18n::tr!("common-animations")),
            (2, aurora_i18n::tr!("ui-inventory-view-calling-cards")),
            (18, aurora_i18n::tr!("ui-inventory-view-clothing-and-body")),
            (20, aurora_i18n::tr!("ui-inventory-view-gestures")),
            (3, aurora_i18n::tr!("common-landmarks")),
            (26, aurora_i18n::tr!("ui-inventory-view-materials")),
            (7, aurora_i18n::tr!("common-notes")),
            (6, aurora_i18n::tr!("common-objects")),
            (10, aurora_i18n::tr!("common-scripts")),
            (1, aurora_i18n::tr!("common-sounds")),
            (0, aurora_i18n::tr!("common-textures")),
            (15, aurora_i18n::tr!("ui-inventory-view-snapshots")),
            (25, aurora_i18n::tr!("ui-inventory-view-eep-settings")),
            (22, aurora_i18n::tr!("ui-inventory-view-meshes")),
        ]
    })
}

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

/// The main inventory's saved order is published once per UI frame. All
/// inventory selectors inherit it, as LLInventoryPanel's DEFAULT_SORT_ORDER
/// does (indra/newview/llinventorypanel.cpp).
pub fn set_picker_sort(ctx: &egui::Context, sort: Sort) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("inventory_picker_sort"), sort));
}

pub fn picker_sort(ctx: &egui::Context) -> Sort {
    ctx.data(|data| data.get_temp(egui::Id::unique("inventory_picker_sort")))
        .unwrap_or_default()
}

/// A shared, immutable projection for selectors that display a flat list or
/// browse folders themselves. Keep the inventory's comparator, folder dates,
/// support folders and Library placement in one place.
pub struct PickerOrder {
    version: (Uuid, Uuid, u64, Sort, &'static str),
    ranks: HashMap<Uuid, usize>,
    children: IdMap<Vec<Uuid>>,
}

impl PickerOrder {
    pub fn rank(&self, id: Uuid) -> usize {
        self.ranks.get(&id).copied().unwrap_or(usize::MAX)
    }

    pub fn children(&self, id: Uuid) -> &[Uuid] {
        self.children.get(&id).map_or(&[], Vec::as_slice)
    }
}

pub fn picker_order(ctx: &egui::Context, inv: &Inventory) -> std::sync::Arc<PickerOrder> {
    let key = egui::Id::unique("inventory_picker_order");
    let version = (inv.root, inv.lib_root, inv.generation, picker_sort(ctx), aurora_i18n::language());
    if let Some(order) = ctx.data(|data| data.get_temp::<std::sync::Arc<PickerOrder>>(key))
        && order.version == version
    {
        return order;
    }
    let mut view = ViewCache::default();
    view.prepare(inv, version.3);
    // A flattened selector still keeps the user's items before Library.
    let ranks = [false, true]
        .into_iter()
        .flat_map(|library| view.entries.iter().filter(move |entry| entry.library == library))
        .enumerate()
        .map(|(rank, entry)| (entry.id, rank))
        .collect();
    let order = std::sync::Arc::new(PickerOrder {
        version,
        ranks,
        children: view.children,
    });
    ctx.data_mut(|data| data.insert_temp(key, order.clone()));
    order
}

pub fn picker_name_cmp(a: &str, b: &str) -> Ordering {
    natural_cmp(&a.to_lowercase(), &b.to_lowercase())
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
    /// Advanced search: the rule tree that replaces the text and the
    /// filters above (all but always_folders) while it is set.
    pub smart: Option<super::smart::SmartQuery>,
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
            smart: None,
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
            || self.smart.as_ref().is_some_and(|q| !q.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InventoryPreferences {
    pub creation_permissions: crate::creation_permissions::Defaults,
    pub explorer: ExplorerPreferences,
    /// One menu arrangement is shared by all inventory types and accounts.
    pub context_menus: super::menu_layout::Preferences,
    pub notecard_view: super::markdown::View,
    pub protected: HashSet<Uuid>,
    pub upload_folders: [Uuid; 5],
    pub sort: Sort,
    pub received_sort: Sort,
    pub recent_sort: Sort,
    pub search_field: SearchField,
    pub search_trash: bool,
    pub search_library: bool,
    pub search_outfits: bool,
    pub show_library: bool,
    pub show_recent: bool,
    pub show_worn: bool,
    pub show_favorites: bool,
    pub thumbnail_tooltips: bool,
    pub separate_searches: bool,
    pub double_click_add_objects: bool,
    pub double_click_add_clothes: bool,
    pub filter_defaults: Filters,
    pub last_logout: i64,
    /// How far back the Recent tab goes, in hours; 0: since the last logout.
    /// The Recent Items panel's own filter keeps its "Since Logoff" or its
    /// days and hours between sessions (LLPanelMainInventory, filters.xml).
    pub recent_hours: u32,
    pub warn_task_move: bool,
    pub warn_scripted_task_move: bool,
    /// The rights Aurora's rows show (its "CMT" menu).
    pub rights: super::permissions::Rights,
    /// Aurora's window shows the selection's details: the card at the
    /// bottom of the narrow and medium widths, the pane of the wide one.
    pub show_details: bool,
}
impl Default for InventoryPreferences {
    fn default() -> Self {
        Self {
            explorer: ExplorerPreferences::default(),
            creation_permissions: Default::default(),
            context_menus: Default::default(),
            notecard_view: Default::default(),
            protected: HashSet::new(),
            upload_folders: [Uuid::nil(); 5],
            sort: Sort::default(),
            recent_sort: Sort::default(),
            received_sort: Sort {
                folders_by_name: false,
                ..Sort::default()
            },
            search_field: SearchField::Name,
            search_trash: false,
            search_library: true,
            search_outfits: true,
            show_library: true,
            show_recent: true,
            show_worn: true,
            show_favorites: true,
            thumbnail_tooltips: true,
            separate_searches: false,
            double_click_add_objects: false,
            double_click_add_clothes: false,
            filter_defaults: Filters::default(),
            last_logout: 0,
            recent_hours: 0,
            warn_task_move: true,
            warn_scripted_task_move: true,
            rights: Default::default(),
            show_details: true,
        }
    }
}

/// Expanded inventory geometry in logical points. Zero widths inherit the
/// layout's defaults; resizing a small window never overwrites saved widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExplorerPreferences {
    pub sidebar_width: u16,
    pub kind_width: u16,
    pub rights_width: u16,
    pub date_width: u16,
    pub show_kind: bool,
    pub show_rights: bool,
    pub show_date: bool,
}

impl Default for ExplorerPreferences {
    fn default() -> Self {
        Self {
            sidebar_width: 0,
            kind_width: 0,
            rights_width: 0,
            date_width: 0,
            show_kind: true,
            show_rights: true,
            show_date: true,
        }
    }
}

impl InventoryPreferences {
    // LLPanelMainInventory::setSortBy saves the active panel's order
    // (indra/newview/llpanelmaininventory.cpp). Recent and Received each
    // keep their own order; other tabs share the main-inventory preference.
    pub(super) fn sort_for(&self, tab: usize) -> Sort {
        if tab == super::RECEIVED_TAB {
            self.received_sort
        } else if tab == 2 {
            self.recent_sort
        } else {
            self.sort
        }
    }

    pub(super) fn sort_for_mut(&mut self, tab: usize) -> &mut Sort {
        if tab == super::RECEIVED_TAB {
            &mut self.received_sort
        } else if tab == 2 {
            &mut self.recent_sort
        } else {
            &mut self.sort
        }
    }
}

/// The longest Recent period: the 10000 days of spin_days_ago
/// (floater_inventory_view_finder.xml), plus a day of hours.
pub const RECENT_HOURS_MAX: u32 = 10_000 * 24 + 23;

/// Where "Recent" starts: the period chosen, else the last logout, or a day
/// ago before the first one (relative dates refreshed once a minute).
pub(super) fn recent_cutoff(prefs: &InventoryPreferences, time: i64) -> i64 {
    if prefs.recent_hours > 0 {
        time / 60 * 60 - i64::from(prefs.recent_hours.min(RECENT_HOURS_MAX)) * 3600
    } else if prefs.last_logout > 0 {
        prefs.last_logout
    } else {
        time / 60 * 60 - 86400
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
    children: IdMap<Vec<Uuid>>,
    result_children: IdMap<Vec<Uuid>>,
    folders: IdSet,
    /// A folder's date: its newest descendant's (see `prepare`).
    folder_dates: IdMap<i64>,
    revision: u64,
    /// Bumped by every rebuild of the sorted entries (`prepare`).
    built: u64,
    query: Option<Query>,
    pub results: Vec<Uuid>,
}

impl ViewCache {
    pub fn prepare(&mut self, inv: &Inventory, sort: Sort) {
        let version = (inv.root, inv.lib_root, inv.generation, sort);
        if self.version == Some(version) {
            return;
        }
        // Trash and outfits by folder, once each; an item is where its folder is.
        let roots = |kinds: &[i32]| kinds.iter().filter_map(|kind| rules::system(inv, *kind)).collect::<Vec<_>>();
        let trash = within(inv, &roots(&[14]));
        let outfits = within(inv, &roots(&[46, 48]));
        let metadata = |id: Uuid, parent, name, desc, date, group, folder: bool| {
            let at = if folder { id } else { parent };
            Entry {
                id,
                parent,
                name,
                desc,
                date,
                group,
                folder,
                trash: trash.contains(&at),
                outfit: outfits.contains(&at),
                library: rules::library(inv, id),
            }
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
                // dated below, from their items
                metadata(
                    *id,
                    f.info.parent,
                    folder_name(inv, *id).to_lowercase(),
                    String::new(),
                    0,
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
        // A folder has no server creation date: the Legacy Viewer uses its
        // newest descendant. Each folder's newest item climbs its ancestors
        // until one is already as new (they are then all as new above it).
        // The bound also stops malformed parent cycles.
        let mut newest: IdMap<i64> = IdMap::default();
        for entry in self.entries.iter().filter(|e| !e.folder && inv.folders.contains_key(&e.parent)) {
            newest
                .entry(entry.parent)
                .and_modify(|d| *d = (*d).max(entry.date))
                .or_insert(entry.date);
        }
        let mut dates: IdMap<i64> = IdMap::default();
        for (folder, date) in newest {
            let mut id = folder;
            for _ in 0..=inv.folders.len() {
                let Some(f) = inv.folders.get(&id) else { break };
                match dates.get_mut(&id) {
                    Some(d) if *d >= date => break,
                    Some(d) => *d = date,
                    None => {
                        dates.insert(id, date);
                    }
                }
                id = f.info.parent;
            }
        }
        for entry in self.entries.iter_mut().filter(|e| e.folder) {
            entry.date = dates.get(&entry.id).copied().unwrap_or(0);
        }
        self.folder_dates = dates;
        self.entries.par_sort_unstable_by(|a, b| compare(a, b, sort));
        self.folders = inv.folders.keys().copied().collect();
        // Keep the lists' memory: a folder gone keeps an empty list.
        self.children.values_mut().for_each(Vec::clear);
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
        self.built = self.built.wrapping_add(1);
        self.query = None;
    }

    /// Changes whenever the sorted children change (see `prepare`).
    pub fn built(&self) -> u64 {
        self.built
    }

    /// The date a folder sorts by: its newest descendant's, 0 when it holds
    /// no dated item.
    pub fn folder_date(&self, id: Uuid) -> i64 {
        self.folder_dates.get(&id).copied().unwrap_or(0)
    }

    #[cfg(test)]
    pub fn children(&self, parent: Uuid, prefs: &InventoryPreferences, lib_root: Uuid) -> Vec<Uuid> {
        self.children_iter(parent, prefs.show_library, lib_root).collect()
    }

    pub fn children_iter(&self, parent: Uuid, show_library: bool, lib_root: Uuid) -> impl Iterator<Item = Uuid> + '_ {
        self.children
            .get(&parent)
            .into_iter()
            .flatten()
            .filter(move |id| show_library || **id != lib_root)
            .copied()
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
        self.prepare(inv, prefs.sort_for(tab));
        let age = if tab == 2 && filters.age == Age::Any {
            Age::SinceLogout
        } else {
            filters.age
        };
        let cutoff = match age {
            Age::Any => 0,
            Age::SinceLogout => recent_cutoff(prefs, time),
            // Refresh relative dates once a minute, not once per frame.
            _ => time / 60 * 60 - i64::from(filters.hours.min(876_000)) * 3600,
        };
        let text = text.trim().to_lowercase();
        let smart = filters.smart.as_ref();
        // Advanced Date rules read the clock (`Matcher` rounds it to the
        // minute): the key follows it, as the age filter's cutoff does.
        let cutoff = match smart {
            Some(q) if q.uses(SmartField::Date) => time / 60 * 60,
            _ => cutoff,
        };
        let uses_creator = match smart {
            Some(q) => q.uses(SmartField::Creator),
            None => matches!(prefs.search_field, SearchField::Creator | SearchField::All) && !text.is_empty(),
        };
        let names = if uses_creator { facts.names.clone() } else { HashMap::new() };
        let worn = if tab == 3 || smart.is_some_and(|q| q.uses(SmartField::Worn)) {
            facts.worn.clone()
        } else {
            HashSet::new()
        };
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
        // The scope by folder, once (`rules::under` for each entry walked its
        // ancestors again); an item is where its folder is.
        let received = tab == super::RECEIVED_TAB;
        let received_roots = if received {
            crate::world::received_items::roots(inv)
        } else {
            Vec::new()
        };
        let received_root = received.then(|| crate::world::received_items::root(inv)).flatten();
        let scope = if received {
            within(inv, &received_roots)
        } else if root == inv.root {
            within(inv, &[inv.root, inv.lib_root])
        } else {
            within(inv, &[root])
        };
        // A favorite folder shows its whole content, favorite or not
        // (LLInventoryFavoritesItemsPanel::findAndInitRootContent builds the
        // full tree under each favorite folder, llinventorypanel.cpp).
        let favorites: Vec<Uuid> = if tab == 4 {
            inv.folders.values().filter(|f| f.info.favorite).map(|f| f.info.id).collect()
        } else {
            Vec::new()
        };
        let favored = if favorites.is_empty() {
            IdSet::default()
        } else {
            within(inv, &favorites)
        };
        // Advanced search: patterns and creator names resolved once, then
        // shared by the parallel walk. Its Date rules keep their own clock;
        // the Recent tab still limits to since logout.
        let recent = recent_cutoff(prefs, time);
        let matcher = smart.map(|q| Matcher::new(q, &query.names, recent, time));
        let folder_names: HashMap<Uuid, &str> = if smart.is_some_and(|q| q.uses(SmartField::Folder)) {
            self.entries.iter().filter(|e| e.folder).map(|e| (e.id, e.name.as_str())).collect()
        } else {
            HashMap::new()
        };
        self.results = self
            .entries
            .par_iter()
            .filter(|e| {
                if !scope.contains(if e.folder { &e.id } else { &e.parent }) {
                    return false;
                }
                if e.id == root || e.id == inv.lib_root {
                    return false;
                }
                if received && Some(e.id) == received_root {
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
                    if matches!(tab, 2 | 3) || (tab == 4 && !favored.contains(&e.id)) {
                        return false;
                    }
                    if let Some(matcher) = &matcher {
                        return matcher.matches(&Subject {
                            id: e.id,
                            folder: true,
                            name: &e.name,
                            desc: "",
                            parent_name: folder_names.get(&e.parent).copied().unwrap_or(""),
                            creator: Uuid::nil(),
                            kind: -1,
                            owner_mask: 0,
                            worn: false,
                            linked: false,
                            coalesced: false,
                            date: e.date,
                        });
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
                if let Some(matcher) = &matcher {
                    if query.target.is_some_and(|target| !linked || it.asset_id != target) {
                        return false;
                    }
                    let original_id = rules::original(inv, e.id).unwrap_or(e.id);
                    let original = inv.items.get(&original_id).unwrap_or(it);
                    if tab == 2 && (e.date <= 0 || e.date < recent) {
                        return false;
                    }
                    if tab == 3 && (!query.worn.contains(&original_id) || e.outfit) {
                        return false;
                    }
                    if tab == 4 && !favored.contains(&e.parent) && !rules::is_favorite(inv, e.id) {
                        return false;
                    }
                    return matcher.matches(&Subject {
                        id: e.id,
                        folder: false,
                        name: &e.name,
                        desc: &e.desc,
                        parent_name: folder_names.get(&e.parent).copied().unwrap_or(""),
                        creator: original.creator,
                        kind: if original.inv_type == 17 { 6 } else { original.inv_type },
                        owner_mask: original.owner_mask,
                        worn: query.worn.contains(&original_id),
                        linked,
                        coalesced: original.asset_type == 6 && original.flags & super::OBJECT_HAS_MULTIPLE_ITEMS != 0,
                        date: e.date,
                    });
                }
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
                if filters.coalesced && (original.asset_type != 6 || original.flags & super::OBJECT_HAS_MULTIPLE_ITEMS == 0) {
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
                if tab == 4 && !favored.contains(&e.parent) && !rules::is_favorite(inv, e.id) {
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
        // Keep the Marketplace container even when it is empty or filtered.
        // These are presentation parents only; server membership stays intact.
        let inbox = received.then(|| rules::system(inv, 50)).flatten();
        // Every matching item retains its real ancestors. Folder visibility
        // is independent of whether the folder itself passes item filters.
        // A walk stops at an ancestor already in: the ones above it are too.
        let mut included = IdSet::default();
        for id in &self.results {
            let mut parent = *id;
            while parent != root && included.insert(parent) {
                if received && received_roots.contains(&parent) {
                    break;
                }
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
        if received {
            let mut top: Vec<Uuid> = inbox.into_iter().collect();
            if let Some(received_root) = received_root {
                top.extend(
                    self.children
                        .get(&received_root)
                        .into_iter()
                        .flatten()
                        .filter(|id| included.contains(id))
                        .copied(),
                );
            }
            if !top.is_empty() {
                self.result_children.insert(root, top);
                if let Some(inbox) = inbox
                    && !self.results.contains(&inbox)
                {
                    self.results.push(inbox);
                }
            }
        }
        self.revision = self.revision.wrapping_add(1);
        self.query = Some(query);
    }
}

impl ViewCache {
    /// The folders that hold results (the branches of the result tree).
    pub fn result_folders(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.result_children.keys().copied()
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
    pub fn collapse_folders(&mut self, folders: impl IntoIterator<Item = Uuid>, tab: usize) {
        self.closed[tab.min(4)].extend(folders);
        self.key = None;
    }

    pub fn expand_folder(&mut self, id: Uuid, tab: usize) {
        if self.closed[tab.min(4)].remove(&id) {
            self.key = None;
        }
    }

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

    pub fn is_open(&self, id: Uuid, tab: usize) -> bool {
        !self.closed[tab.min(4)].contains(&id)
    }

    pub fn set_open(&mut self, id: Uuid, tab: usize, open: bool) {
        if self.is_open(id, tab) != open {
            self.toggle(id, tab);
        }
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

/// The view's maps by inventory id: no protection against chosen keys is
/// needed, so the halves are folded with FxHasher's multiply and finished
/// with MurmurHash3's mixer (synthetic ids differ only in a few bytes), for a
/// fraction of SipHash's cost (a third of a rebuild went to hashing).
#[derive(Default, Clone, Copy)]
pub(super) struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(word)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    fn finish(&self) -> u64 {
        let mut h = self.0;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        h ^ (h >> 33)
    }
}

type IdMap<V> = HashMap<Uuid, V, std::hash::BuildHasherDefault<IdHasher>>;
pub(super) type IdSet = HashSet<Uuid, std::hash::BuildHasherDefault<IdHasher>>;

/// The folders under one of `roots` (`rules::under`, the roots included),
/// each ancestor chain walked once.
pub(super) fn within(inv: &Inventory, roots: &[Uuid]) -> IdSet {
    let mut known: IdMap<bool> = IdMap::default();
    known.reserve(inv.folders.len());
    let mut path = Vec::new();
    for start in inv.folders.keys() {
        let mut id = *start;
        let found = loop {
            if roots.contains(&id) {
                break true;
            }
            if let Some(found) = known.get(&id) {
                break *found;
            }
            if path.contains(&id) {
                break false;
            }
            path.push(id);
            match rules::parent(inv, id) {
                Some(parent) => id = parent,
                None => break false,
            }
        };
        for id in path.drain(..) {
            known.insert(id, found);
        }
    }
    known
        .into_iter()
        .filter_map(|(id, found)| found.then_some(id))
        .chain(roots.iter().copied())
        .collect()
}

fn compare(a: &Entry, b: &Entry, sort: Sort) -> Ordering {
    // Viewer support folders stay discoverable above system categories,
    // including in date order, as for Firestorm's support folder. #RLV
    // joins them like the Legacy Viewer's FT_RLV (LLFolderBridge::getSortGroup
    // with FSSortFSFoldersToTop, llinventorybridge.cpp).
    let support = |e: &Entry| e.folder && !e.library && matches!(e.name.as_str(), "#aurora" | "#firestorm" | "#rlv");
    let pinned = support(b).cmp(&support(a));
    if pinned != Ordering::Equal {
        return pinned;
    }
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

    #[test]
    fn outfit_readback_replaces_visible_links_across_repeated_attach_detach_cycles() {
        use crate::world::appearance;
        use aurora_net::inventory::FolderContents;

        let (mut inv, _) = fixture();
        let agent = id(99);
        appearance::seed_demo(&mut inv, agent);
        let cof = appearance::cof(&inv).expect("COF");
        let target = id(717);
        let saved = inv.folders[&id(704)].items.clone();
        // An older viewer has already lost these links from the folder's
        // child list, while the visible inventory still indexes their parent.
        let old = inv.folders[&cof]
            .items
            .iter()
            .find_map(|id| inv.items.get(id).filter(|it| it.asset_id == target))
            .expect("worn attachment link")
            .clone();
        for _ in 0..2 {
            let ghost = InvItem {
                id: Uuid::new_v4(),
                ..old.clone()
            };
            inv.items.insert(ghost.id, ghost);
        }
        let mut view = ViewCache::default();
        for worn in [false, true, false, true, false, true] {
            let action = if worn {
                appearance::Action::Add(target)
            } else {
                appearance::Action::Remove(target)
            };
            let (change, _) = appearance::plan(&inv, action, &HashMap::new()).expect("outfit action");
            // Real AIS readback replaces link UUIDs. Do not use demo_replace,
            // which used to delete the old records before applying the reply.
            let items = change
                .links
                .iter()
                .map(|link| InvItem {
                    id: Uuid::new_v4(),
                    parent: cof,
                    name: link.name.clone(),
                    desc: link.desc.clone(),
                    asset_type: if link.folder { 25 } else { 24 },
                    inv_type: link.inv_type,
                    asset_id: link.target,
                    owner: agent,
                    ..Default::default()
                })
                .collect();
            inv.apply(vec![FolderContents {
                folder_id: cof,
                owner_id: agent,
                version: inv.folders[&cof].info.version + 1,
                folders: Vec::new(),
                items,
            }]);
            view.prepare(&inv, Sort::default());
            let rows: Vec<_> = view.children_iter(cof, true, inv.lib_root).collect();
            assert_eq!(
                rows.len(),
                inv.folders[&cof].items.len(),
                "the tree must reflect the current readback"
            );
            assert_eq!(
                rows.iter().filter(|id| inv.items[*id].asset_id == target).count(),
                usize::from(worn)
            );
            assert!(inv.items.contains_key(&target), "keep the inventory original");
            assert_eq!(inv.folders[&id(704)].items, saved, "keep the saved outfit");
            assert!(saved.iter().all(|id| inv.items.contains_key(id)));
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
            worn_at: Default::default(),
            points: vec![],
            appearance_busy: false,
            active_gestures: Default::default(),
            touchable: Default::default(),
            names: HashMap::new(),
            ..Default::default()
        };
        (inv, facts)
    }
    fn search(inv: &Inventory, facts: &Facts, prefs: &InventoryPreferences, filters: &Filters, tab: usize) -> Vec<Uuid> {
        let mut cache = ViewCache::default();
        cache.search(inv, inv.root, "", prefs, filters, tab, None, facts, facts.agent, 1000);
        cache.results
    }

    #[test]
    fn inventory_favorites_filter_links_by_the_original_state() {
        let (mut inv, facts) = fixture();
        inv.items.get_mut(&id(10)).expect("original").favorite = true;
        let mut link = item(13, id(2), "Link", 100);
        link.asset_type = 24;
        link.asset_id = id(10);
        inv.add_items(vec![link]);
        let prefs = InventoryPreferences::default();
        let filters = Filters::default();
        let results = search(&inv, &facts, &prefs, &filters, 4);
        assert!(results.contains(&id(10)));
        assert!(results.contains(&id(13)));
        assert!(!results.contains(&id(11)));
        inv.items.get_mut(&id(10)).expect("original").favorite = false;
        inv.items.get_mut(&id(13)).expect("link").favorite = true;
        inv.generation += 1;
        let results = search(&inv, &facts, &prefs, &filters, 4);
        assert!(!results.contains(&id(10)));
        assert!(!results.contains(&id(13)), "ignore a stale favorite on the link itself");
    }

    #[test]
    fn inventory_favorite_folders_show_their_whole_content() {
        let (mut inv, facts) = fixture();
        folder(&mut inv, 20, id(2), -1, "Sub");
        folder(&mut inv, 21, id(20), -1, "Empty");
        inv.add_items(vec![item(22, id(20), "Deep", 100)]);
        inv.folders.get_mut(&id(2)).expect("folder").info.favorite = true;
        let prefs = InventoryPreferences::default();
        let filters = Filters::default();
        let results = search(&inv, &facts, &prefs, &filters, 4);
        for n in [2, 10, 11, 12, 20, 21, 22] {
            assert!(results.contains(&id(n)), "{n} sits in a favorite folder");
        }
        assert!(!results.contains(&id(4)), "a folder outside stays hidden");
        inv.folders.get_mut(&id(2)).expect("folder").info.favorite = false;
        inv.folders.get_mut(&id(20)).expect("folder").info.favorite = true;
        inv.generation += 1;
        let results = search(&inv, &facts, &prefs, &filters, 4);
        assert!(results.contains(&id(22)) && results.contains(&id(21)));
        assert!(!results.contains(&id(10)), "the parent of a favorite folder shows only its path");
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
    fn picker_order_tracks_every_inventory_sort_and_invalidates_only_on_changes() {
        let ctx = egui::Context::default();
        let (mut inv, _) = fixture();
        let root = inv.root;
        folder(&mut inv, 30, root, -1, "#Aurora");
        folder(&mut inv, 31, id(2), -1, "Folder 10");
        folder(&mut inv, 32, id(2), -1, "folder 2");
        inv.add_items(vec![
            item(13, id(31), "Nested new item", 700),
            item(14, id(2), "Texture 10", 500),
            item(15, id(2), "texture 2", 400),
            item(16, inv.lib_root, "AAA Library", 900),
        ]);
        for by_date in [false, true] {
            for folders_by_name in [false, true] {
                for system_first in [false, true] {
                    let sort = Sort {
                        by_date,
                        folders_by_name,
                        system_first,
                    };
                    set_picker_sort(&ctx, sort);
                    let order = picker_order(&ctx, &inv);
                    let mut inventory = ViewCache::default();
                    inventory.prepare(&inv, sort);
                    for folder in inv.folders.keys() {
                        let expected: Vec<_> = inventory.children_iter(*folder, true, inv.lib_root).collect();
                        assert_eq!(order.children(*folder), expected, "{sort:?}, {folder}");
                        let mut flat = expected.clone();
                        flat.sort_by_key(|id| order.rank(*id));
                        assert_eq!(flat, expected);
                    }
                    assert_eq!(order.children(root).first(), Some(&id(30)));
                    assert_eq!(order.children(root).last(), Some(&inv.lib_root));
                    assert!(order.rank(id(15)) < order.rank(id(16)), "Library stays after personal items");
                    assert_eq!(order.rank(id(14)) < order.rank(id(15)), by_date);
                    assert!(
                        std::sync::Arc::ptr_eq(&order, &picker_order(&ctx, &inv)),
                        "reuse the cached projection"
                    );
                }
            }
        }
        let before = picker_order(&ctx, &inv);
        inv.add_items(vec![item(10, id(2), "A older", 1000)]);
        let after = picker_order(&ctx, &inv);
        assert!(!std::sync::Arc::ptr_eq(&before, &after));
        assert!(after.rank(id(10)) < after.rank(id(14)));
        assert!(picker_name_cmp("texture 2", "Texture 10").is_lt());
    }

    #[test]
    fn viewer_support_folders_stay_above_system_folders_with_every_sort() {
        let (mut inv, _) = fixture();
        let root = inv.root;
        folder(&mut inv, 30, root, -1, "#Aurora");
        folder(&mut inv, 31, root, -1, "#Firestorm");
        folder(&mut inv, 32, root, -1, "#RLV");
        for by_date in [false, true] {
            for system_first in [false, true] {
                let prefs = InventoryPreferences {
                    sort: Sort {
                        by_date,
                        system_first,
                        folders_by_name: false,
                    },
                    ..Default::default()
                };
                let mut cache = ViewCache::default();
                cache.prepare(&inv, prefs.sort);
                assert_eq!(&cache.children(root, &prefs, inv.lib_root)[..3], &[id(30), id(31), id(32)]);
            }
        }
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
    fn folder_dates_climb_past_older_branches_and_stop_on_parent_cycles() {
        let (mut inv, _) = fixture();
        // 2 > 7 > 8: an old item deep down, a newer one in 7, the newest in 8.
        folder(&mut inv, 7, id(2), -1, "nested");
        folder(&mut inv, 8, id(7), -1, "deeper");
        inv.add_items(vec![item(13, id(8), "deep", 900), item(14, id(7), "middle", 50)]);
        // 30 <-> 31, a malformed cycle, with an item.
        folder(&mut inv, 30, id(31), -1, "cycle a");
        folder(&mut inv, 31, id(30), -1, "cycle b");
        inv.add_items(vec![item(15, id(30), "in a cycle", 70)]);
        let mut cache = ViewCache::default();
        cache.prepare(&inv, Sort::default());
        let date = |n| cache.entries.iter().find(|e| e.id == id(n)).map(|e| e.date);
        assert_eq!([date(2), date(7), date(8)], [Some(900); 3]);
        assert_eq!(date(1), Some(900), "the root holds the newest of all");
        assert_eq!([date(30), date(31)], [Some(70); 2]);
        assert_eq!(date(4), Some(0), "an empty folder has no date");
    }

    #[test]
    fn scope_sets_follow_parents_like_under_including_cycles() {
        let (mut inv, _) = fixture();
        folder(&mut inv, 7, id(2), -1, "nested");
        folder(&mut inv, 30, id(31), -1, "cycle a");
        folder(&mut inv, 31, id(30), -1, "cycle b");
        let set = within(&inv, &[id(2)]);
        for n in inv.folders.keys() {
            assert_eq!(set.contains(n), rules::under(&inv, *n, id(2)), "{n}");
        }
        assert!(set.contains(&id(7)) && !set.contains(&id(30)));
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
    fn recent_period_replaces_the_last_logout() {
        let now = 10 * 86400 + 30;
        let week = InventoryPreferences {
            last_logout: 200,
            recent_hours: 7 * 24,
            ..Default::default()
        };
        assert_eq!(recent_cutoff(&week, now), 3 * 86400);
        let since_logout = InventoryPreferences {
            recent_hours: 0,
            ..week.clone()
        };
        assert_eq!(recent_cutoff(&since_logout, now), 200);
        let first_launch = InventoryPreferences::default();
        assert_eq!(recent_cutoff(&first_launch, now), 9 * 86400);
        // the Recent tab of the search follows the period: both items are
        // within the last hour of the fixture's clock (1000 s)
        let (inv, facts) = fixture();
        let hour = InventoryPreferences { recent_hours: 1, ..week };
        assert_eq!(search(&inv, &facts, &hour, &Filters::default(), 2), [id(11), id(10)]);
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
        assert_eq!(prefs.recent_sort, Sort::default());
        assert!(prefs.received_sort.by_date);
        assert!(!prefs.received_sort.folders_by_name);
        assert!(prefs.received_sort.system_first);
        assert!(prefs.show_details, "the details stay shown until hidden");
        let changed = InventoryPreferences {
            sort: Sort {
                by_date: false,
                system_first: false,
                folders_by_name: false,
            },
            received_sort: Sort {
                by_date: true,
                folders_by_name: true,
                system_first: false,
            },
            filter_defaults: Filters {
                links: Links::Exclude,
                age: Age::Newer,
                hours: 48,
                ..Default::default()
            },
            show_details: false,
            ..prefs
        };
        let json = serde_json::to_string(&changed).expect("serialize preferences");
        assert_eq!(
            serde_json::from_str::<InventoryPreferences>(&json).expect("reload preferences"),
            changed
        );
    }

    #[test]
    fn received_sort_migration_preserves_the_saved_main_inventory_order() {
        let prefs: InventoryPreferences = serde_json::from_str(r#"{"sort":{"by_date":false,"folders_by_name":true,"system_first":false}}"#)
            .expect("existing preferences");
        assert_eq!(
            prefs.sort,
            Sort {
                by_date: false,
                folders_by_name: true,
                system_first: false
            }
        );
        assert_eq!(
            prefs.received_sort,
            Sort {
                by_date: true,
                folders_by_name: false,
                system_first: true
            }
        );
    }

    #[test]
    fn recent_sort_is_independent_and_survives_reload() {
        let mut prefs: InventoryPreferences =
            serde_json::from_str(r#"{"sort":{"by_date":false,"folders_by_name":false,"system_first":false}}"#).expect("old preferences");
        assert_eq!(prefs.recent_sort, Sort::default());
        let main = prefs.sort;
        let received = prefs.received_sort;
        *prefs.sort_for_mut(2) = Sort {
            by_date: false,
            folders_by_name: true,
            system_first: false,
        };
        let saved = serde_json::to_string(&prefs).expect("save");
        let restored: InventoryPreferences = serde_json::from_str(&saved).expect("reload");
        assert_eq!(restored, prefs);
        assert_eq!(restored.sort_for(2), prefs.recent_sort);
        assert_eq!(restored.sort, main);
        assert_eq!(restored.received_sort, received);
    }

    #[test]
    fn recent_sort_refreshes_results_without_changing_other_views() {
        let (inv, facts) = fixture();
        let mut prefs = InventoryPreferences {
            last_logout: 1,
            ..Default::default()
        };
        let mut cache = ViewCache::default();
        let run = |cache: &mut ViewCache, prefs: &InventoryPreferences, tab| {
            cache.search(&inv, inv.root, "", prefs, &Filters::default(), tab, None, &facts, facts.agent, 1000);
            cache
                .results
                .iter()
                .copied()
                .filter(|id| [self::id(10), self::id(11)].contains(id))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(&mut cache, &prefs, 2), [id(11), id(10)]);
        prefs.recent_sort.by_date = false;
        assert_eq!(run(&mut cache, &prefs, 2), [id(10), id(11)]);
        assert_eq!(run(&mut cache, &prefs, 0), [id(11), id(10)]);
        let built = cache.built;
        prefs.recent_sort.system_first = false;
        assert_eq!(run(&mut cache, &prefs, 0), [id(11), id(10)]);
        assert_eq!(cache.built, built, "Recent changes do not rebuild All");
        assert_eq!(run(&mut cache, &prefs, 2), [id(10), id(11)]);
        let built = cache.built;
        prefs.sort.by_date = false;
        prefs.received_sort.by_date = false;
        assert_eq!(run(&mut cache, &prefs, 2), [id(10), id(11)]);
        assert_eq!(cache.built, built, "other tabs do not rebuild Recent");
    }

    #[test]
    fn received_sort_uses_its_own_folder_dates_and_cache_invalidation() {
        let (mut inv, facts) = fixture();
        let root = inv.root;
        folder(&mut inv, 20, root, 50, "Marketplace");
        folder(&mut inv, 21, id(20), -1, "A older delivery");
        folder(&mut inv, 22, id(20), -1, "Z newer delivery");
        inv.add_items(vec![item(23, id(21), "A gift", 100), item(24, id(22), "Z gift", 300)]);
        let mut prefs = InventoryPreferences::default();
        let mut cache = ViewCache::default();
        let run = |cache: &mut ViewCache, prefs: &InventoryPreferences, tab| {
            cache.search(&inv, root, "", prefs, &Filters::default(), tab, None, &facts, facts.agent, 1000);
            cache
                .results
                .iter()
                .copied()
                .filter(|entry| [id(21), id(22)].contains(entry))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(&mut cache, &prefs, 0), [id(21), id(22)]);
        assert_eq!(run(&mut cache, &prefs, super::super::RECEIVED_TAB), [id(22), id(21)]);
        let built = cache.built;
        prefs.sort.by_date = false;
        prefs.sort.system_first = false;
        assert_eq!(run(&mut cache, &prefs, super::super::RECEIVED_TAB), [id(22), id(21)]);
        assert_eq!(cache.built, built, "main-inventory changes do not rebuild Received");
        prefs.received_sort.folders_by_name = true;
        assert_eq!(run(&mut cache, &prefs, super::super::RECEIVED_TAB), [id(21), id(22)]);
        assert_ne!(cache.built, built, "Received changes refresh the cached order");
        assert_eq!(run(&mut cache, &prefs, 0), [id(21), id(22)]);
        let built = cache.built;
        prefs.received_sort.by_date = false;
        prefs.received_sort.system_first = false;
        assert_eq!(run(&mut cache, &prefs, 0), [id(21), id(22)]);
        assert_eq!(cache.built, built, "Received changes do not rebuild the main inventory");
    }

    #[test]
    fn received_sort_demo_reverses_real_purchase_folders_when_the_option_changes() {
        let (mut inv, facts) = fixture();
        crate::world::inventory::demo::seed_received_sort(&mut inv, facts.agent);
        let inbox = rules::system(&inv, 50).expect("demo inbox");
        let mut cache = ViewCache::default();
        let mut prefs = InventoryPreferences::default();
        for folders_by_name in [false, true, false] {
            prefs.received_sort.folders_by_name = folders_by_name;
            cache.search(
                &inv,
                inv.root,
                "",
                &prefs,
                &Filters::default(),
                super::super::RECEIVED_TAB,
                None,
                &facts,
                facts.agent,
                now(),
            );
            let expected = if folders_by_name {
                [id(8_300_001), id(8_300_010), id(8_300_020)]
            } else {
                [id(8_300_020), id(8_300_010), id(8_300_001)]
            };
            assert_eq!(cache.result_children[&inbox], expected, "the actual demo must visibly change order");
            assert_eq!(cache.result_children[&inv.root][0], inbox, "Marketplace stays above sender folders");
            assert!(expected.iter().all(|id| cache.folder_date(*id) > 0));
        }
    }

    fn smart(nodes: Vec<super::super::smart::Node>, any: bool) -> Filters {
        use super::super::smart::{Group, SmartQuery};
        Filters {
            smart: Some(SmartQuery {
                root: Group { any, not: false, nodes },
            }),
            ..Filters::default()
        }
    }

    #[test]
    fn advanced_search_matches_quick_search_for_a_single_and() {
        use super::super::smart::{Field, Node, Rule, SmartQuery};
        let (inv, facts) = fixture();
        let prefs = InventoryPreferences::default();
        for text in ["older", "a", "date", "zz"] {
            let mut cache = ViewCache::default();
            cache.search(
                &inv,
                inv.root,
                text,
                &prefs,
                &Filters::default(),
                0,
                None,
                &facts,
                facts.agent,
                1000,
            );
            let quick = cache.results.clone();
            let filters = Filters {
                smart: Some(SmartQuery::from_quick(text, SearchField::Name)),
                ..Filters::default()
            };
            assert_eq!(search(&inv, &facts, &prefs, &filters, 0), quick, "{text}");
        }
        // The legacy type filter, as a rule.
        let mut types = Rule::of(Field::Type);
        types.bits = 1 << 6;
        let legacy = Filters {
            types: 1 << 6,
            ..Filters::default()
        };
        assert_eq!(
            search(&inv, &facts, &prefs, &smart(vec![Node::Rule(types)], false), 0),
            search(&inv, &facts, &prefs, &legacy, 0)
        );
    }

    #[test]
    fn advanced_search_ors_across_fields_and_negates() {
        use super::super::smart::{Field, Node, Op, Rule};
        let (mut inv, facts) = fixture();
        let mut gesture = item(13, id(2), "Wave", 200);
        gesture.inv_type = 20;
        gesture.asset_type = 21;
        inv.add_items(vec![gesture]);
        let prefs = InventoryPreferences::default();
        let mut gestures = Rule::of(Field::Type);
        gestures.bits = 1 << 20;
        let either = smart(
            vec![
                Node::Rule(gestures.clone()),
                Node::Rule(Rule::text(Field::Name, Op::StartsWith, "z ")),
            ],
            true,
        );
        let results = search(&inv, &facts, &prefs, &either, 0);
        assert!(results.contains(&id(13)) && results.contains(&id(11)));
        assert!(!results.contains(&id(10)) && !results.contains(&id(2)), "{results:?}");
        gestures.not = true;
        let not_gesture = search(&inv, &facts, &prefs, &smart(vec![Node::Rule(gestures)], false), 0);
        assert!(!not_gesture.contains(&id(13)) && not_gesture.contains(&id(10)));
        assert!(!not_gesture.contains(&id(2)), "folders do not match item-only rules");
        let in_folder = smart(vec![Node::Rule(Rule::text(Field::Folder, Op::Is, "personal"))], false);
        let results = search(&inv, &facts, &prefs, &in_folder, 0);
        assert!(results.contains(&id(10)) && results.contains(&id(13)));
    }

    #[test]
    fn advanced_search_refreshes_when_rules_or_creator_names_change() {
        use super::super::smart::{Field, Node, Op, Rule};
        let (inv, mut facts) = fixture();
        let prefs = InventoryPreferences::default();
        let mut cache = ViewCache::default();
        let mut run = |filters: &Filters, facts: &Facts| {
            cache.search(&inv, inv.root, "", &prefs, filters, 0, None, facts, facts.agent, 1000);
            cache.results.clone()
        };
        let by_alice = smart(vec![Node::Rule(Rule::text(Field::Creator, Op::Contains, "alice"))], false);
        assert!(run(&by_alice, &facts).is_empty());
        facts.names.insert(id(99), "Alice Doe".into());
        assert!(run(&by_alice, &facts).contains(&id(10)), "a name arriving refreshes the results");
        let by_bob = smart(vec![Node::Rule(Rule::text(Field::Creator, Op::Contains, "bob"))], false);
        assert!(run(&by_bob, &facts).is_empty(), "a changed rule refreshes the results");
    }
}
