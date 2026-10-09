//! The environment selector's lists: every sky, water and day cycle
//! settings item of the inventory and the library, and the folder fetches
//! that load them.
//!
//! Port of FloaterQuickPrefs::loadPresets / FSSettingsCollector /
//! stepComboBox / isValidPreset (newview/quickprefs.cpp, Firestorm,
//! originally LGPL 2.1) and LLSettingsType::fromInventoryFlags
//! (llinventory/llinventorysettings.cpp). Firestorm lists what its background
//! inventory fetch has loaded; Aurora fetches on demand, when the selector is
//! first opened: the library's "Environments" folders first, then the user's
//! Settings folder, then the rest of the inventory (Trash and Marketplace
//! listings excluded, as the collector excludes their items).

use super::inventory::{FetchState, Inventory};
use aurora_net::inventory::InvItem;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// LLAssetType::AT_SETTINGS.
pub const AT_SETTINGS: i32 = 56;
/// LLInventoryItemFlags::II_FLAGS_SUBTYPE_MASK: the settings type of an item.
const II_FLAGS_SUBTYPE_MASK: u32 = 0xff;
/// LLFolderType values used to order and filter the fetches.
const FT_TRASH: i32 = 14;
const FT_MARKETPLACE_LISTINGS: i32 = 53;
const FT_SETTINGS: i32 = 56;
/// Name of the library folders holding Linden's environments.
const LIBRARY_ENVIRONMENTS: &str = "environments";
/// Folders requested at once, and in flight at most (FetchInventoryDescendents2
/// sends them by 16).
const FETCH_BATCH: usize = 16;
const MAX_IN_FLIGHT: usize = 48;

/// LLSettingsType::type_e of a settings item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsKind {
    Sky,
    Water,
    Day,
}

impl SettingsKind {
    /// LLSettingsType::fromInventoryFlags (ST_SKY 0, ST_WATER 1, ST_DAYCYCLE 2;
    /// anything else is ST_INVALID).
    pub fn from_flags(flags: u32) -> Option<SettingsKind> {
        match flags & II_FLAGS_SUBTYPE_MASK {
            0 => Some(SettingsKind::Sky),
            1 => Some(SettingsKind::Water),
            2 => Some(SettingsKind::Day),
            _ => None,
        }
    }
}

/// One entry of a list: the item name and the settings asset it applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsChoice {
    pub name: String,
    pub asset: Uuid,
}

/// The three lists of the selector.
#[derive(Debug, Clone, Default)]
pub struct SettingsCatalog {
    pub sky: Vec<SettingsChoice>,
    pub water: Vec<SettingsChoice>,
    pub day: Vec<SettingsChoice>,
}

/// Where a folder sits, for the fetch order and the item filter.
#[derive(Debug, Clone, Copy, Default)]
struct Place {
    /// In Trash or in the Marketplace listings: its items are not listed.
    excluded: bool,
    /// In a library "Environments" folder.
    library_environments: bool,
    /// In the user's Settings folder.
    settings: bool,
    library: bool,
}

fn places(inv: &Inventory) -> HashMap<Uuid, Place> {
    fn resolve(inv: &Inventory, id: Uuid, out: &mut HashMap<Uuid, Place>, depth: u32) -> Place {
        if let Some(p) = out.get(&id) {
            return *p;
        }
        let Some(f) = inv.folders.get(&id) else {
            return Place::default();
        };
        // a broken parent chain (loop) ends after a sane depth
        let parent = if f.info.parent.is_nil() || f.info.parent == id || depth > 64 {
            Place::default()
        } else {
            resolve(inv, f.info.parent, out, depth + 1)
        };
        let t = f.info.type_default;
        let p = Place {
            excluded: parent.excluded || (!f.library && (t == FT_TRASH || t == FT_MARKETPLACE_LISTINGS)),
            library_environments: parent.library_environments
                || (f.library && f.info.name.trim().eq_ignore_ascii_case(LIBRARY_ENVIRONMENTS)),
            settings: parent.settings || (!f.library && t == FT_SETTINGS),
            library: f.library,
        };
        out.insert(id, p);
        p
    }
    let mut out = HashMap::with_capacity(inv.folders.len());
    for id in inv.folders.keys() {
        resolve(inv, *id, &mut out, 0);
    }
    out
}

impl SettingsCatalog {
    pub fn list(&self, kind: SettingsKind) -> &[SettingsChoice] {
        match kind {
            SettingsKind::Sky => &self.sky,
            SettingsKind::Water => &self.water,
            SettingsKind::Day => &self.day,
        }
    }

    pub fn name_of(&self, kind: SettingsKind, asset: Uuid) -> Option<&str> {
        self.list(kind).iter().find(|c| c.asset == asset).map(|c| c.name.as_str())
    }

    /// The lists from the loaded inventory and library (loadPresets).
    pub fn build(inv: &Inventory) -> SettingsCatalog {
        let places = places(inv);
        let mut items: Vec<(&InvItem, bool)> = inv
            .items
            .values()
            .filter(|it| it.asset_type == AT_SETTINGS)
            .filter_map(|it| {
                let p = places.get(&it.parent).copied().unwrap_or_default();
                (!p.excluded).then_some((it, p.library))
            })
            .collect();
        // a stable traversal: the user's items before the library's (the
        // first item of an asset gives its name), then by name and id
        items.sort_by(|(a, la), (b, lb)| la.cmp(lb).then_with(|| a.name.cmp(&b.name)).then_with(|| a.id.cmp(&b.id)));
        Self::from_items(items.into_iter().map(|(it, _)| it))
    }

    /// FSSettingsCollector + the three multimaps of loadPresets: one entry
    /// per settings asset (the first item seen), items of an invalid settings
    /// type dropped, each list sorted by name. Firestorm's std::multimap
    /// compares names byte by byte (case-sensitive: "Zebra" before "apple"),
    /// equal names keeping their order.
    pub fn from_items<'a>(items: impl IntoIterator<Item = &'a InvItem>) -> SettingsCatalog {
        let mut seen = HashSet::new();
        let mut out = SettingsCatalog::default();
        for it in items {
            if it.asset_type != AT_SETTINGS || !seen.insert(it.asset_id) {
                continue;
            }
            // isValidPreset refuses a null asset and loadXPresets an empty
            // name: such entries could never be applied
            if it.asset_id.is_nil() || it.name.is_empty() {
                continue;
            }
            let Some(kind) = SettingsKind::from_flags(it.flags) else {
                log::warn!("found invalid environment setting: {}", it.name);
                continue;
            };
            let choice = SettingsChoice {
                name: it.name.clone(),
                asset: it.asset_id,
            };
            match kind {
                SettingsKind::Sky => out.sky.push(choice),
                SettingsKind::Water => out.water.push(choice),
                SettingsKind::Day => out.day.push(choice),
            }
        }
        for list in [&mut out.sky, &mut out.water, &mut out.day] {
            list.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        }
        out
    }
}

/// Queue the next folder fetches of the selector's scan; returns how many
/// folders are still to load (0: the lists are complete).
pub fn scan_step(inv: &mut Inventory) -> usize {
    let places = places(inv);
    let mut wanted: Vec<(u8, Uuid)> = Vec::new();
    let mut in_flight = 0;
    for (id, f) in &inv.folders {
        match f.state {
            FetchState::Fetching => in_flight += 1,
            FetchState::Unknown => {
                let p = places.get(id).copied().unwrap_or_default();
                let rank = if p.library_environments {
                    0
                } else if p.library || p.excluded {
                    continue;
                } else if p.settings {
                    1
                } else {
                    2
                };
                wanted.push((rank, *id));
            }
            FetchState::Fetched | FetchState::Failed => {}
        }
    }
    let left = wanted.len() + in_flight;
    if in_flight < MAX_IN_FLIGHT {
        wanted.sort();
        for (_, id) in wanted.into_iter().take(FETCH_BATCH.min(MAX_IN_FLIGHT - in_flight)) {
            inv.request(id);
        }
    }
    left
}

/// What a list shows (setSelectedEnvironment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// "Environnement partagé" (PRESET_NAME_REGION_DEFAULT).
    Shared,
    /// Sky and water lists: "Basé sur le cycle du jour" (PRESET_NAME_DAY_CYCLE);
    /// the day list: "Rien" (PRESET_NAME_NONE).
    DayBased,
    Item(Uuid),
    /// Aurora's local time of day preset (Monde > Environnement).
    Preset(u8),
    /// Edited in « Éclairage personnel ».
    Personal,
}

/// Index of what a list shows among its entries: 0 and 1 are the two fixed
/// entries, then the items. Something that is not in the list (a preset, a
/// personal sky, an item not loaded yet) leaves the combo on its first entry,
/// as LLComboBox::selectByValue does when the value is missing.
pub fn shown_index(list: &[SettingsChoice], shown: Shown) -> usize {
    match shown {
        Shown::DayBased => 1,
        Shown::Item(id) => list.iter().position(|c| c.asset == id).map_or(0, |i| i + 2),
        Shown::Shared | Shown::Preset(_) | Shown::Personal => 0,
    }
}

/// The < > arrows (stepComboBox): the next item in that direction, wrapping
/// around and skipping the two fixed entries, which cannot be selected.
pub fn step(list: &[SettingsChoice], current: usize, forward: bool) -> Option<Uuid> {
    let count = list.len() + 2;
    let start = current.min(count - 1);
    let mut i = start;
    loop {
        i = if forward { (i + 1) % count } else { (i + count - 1) % count };
        if i >= 2 {
            return Some(list[i - 2].asset);
        }
        if i == start {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvFolder;

    fn item(name: &str, flags: u32, asset: u128, parent: Uuid) -> InvItem {
        InvItem {
            id: Uuid::new_v4(),
            parent,
            name: name.into(),
            desc: String::new(),
            asset_type: AT_SETTINGS,
            inv_type: 25,
            asset_id: Uuid::from_u128(asset),
            flags,
            favorite: false,
            creator: Uuid::nil(),
            created_at: 0,
            owner: Uuid::nil(),
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
        }
    }

    #[test]
    fn settings_type_comes_from_the_item_flags() {
        assert_eq!(SettingsKind::from_flags(0), Some(SettingsKind::Sky));
        assert_eq!(SettingsKind::from_flags(1), Some(SettingsKind::Water));
        assert_eq!(SettingsKind::from_flags(2), Some(SettingsKind::Day));
        // only the low byte holds the subtype
        assert_eq!(SettingsKind::from_flags(0x0100_0002), Some(SettingsKind::Day));
        assert_eq!(SettingsKind::from_flags(3), None);
        assert_eq!(SettingsKind::from_flags(0xff), None);
    }

    #[test]
    fn lists_are_deduplicated_and_sorted_like_firestorm() {
        let f = Uuid::nil();
        let items = [
            item("Midday", 0, 1, f),
            item("A-6PM", 0, 2, f),
            item("annan sky", 0, 3, f),
            item("Coastal Sunset", 0, 4, f),
            // a second item of the same asset is not listed again
            item("Midday copy", 0, 1, f),
            item("Murky", 1, 5, f),
            item("Default Water", 1, 6, f),
            item("Dynamic", 2, 7, f),
            item("Broken", 9, 8, f),
            item("No asset", 0, 0, f),
            item("Same", 0, 10, f),
            item("Same", 0, 9, f),
        ];
        let c = SettingsCatalog::from_items(items.iter());
        let names = |l: &[SettingsChoice]| l.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
        // byte order: capitals before lower case, equal names in order
        assert_eq!(names(&c.sky), ["A-6PM", "Coastal Sunset", "Midday", "Same", "Same", "annan sky"]);
        assert_eq!(c.sky[3].asset, Uuid::from_u128(10));
        assert_eq!(names(&c.water), ["Default Water", "Murky"]);
        assert_eq!(names(&c.day), ["Dynamic"]);
        assert_eq!(c.name_of(SettingsKind::Sky, Uuid::from_u128(1)), Some("Midday"));
    }

    fn folder(inv: &mut Inventory, id: u128, parent: u128, name: &str, t: i32, library: bool) {
        inv.folders.insert(
            Uuid::from_u128(id),
            super::super::inventory::Folder {
                info: InvFolder {
                    id: Uuid::from_u128(id),
                    parent: Uuid::from_u128(parent),
                    name: name.into(),
                    type_default: t,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Unknown,
                library,
            },
        );
    }

    fn test_inventory() -> Inventory {
        let mut inv = Inventory::default();
        folder(&mut inv, 1, 0, "My Inventory", 8, false);
        folder(&mut inv, 2, 1, "Settings", FT_SETTINGS, false);
        folder(&mut inv, 3, 1, "Trash", FT_TRASH, false);
        folder(&mut inv, 4, 3, "Old", -1, false);
        folder(&mut inv, 5, 1, "Objects", 6, false);
        folder(&mut inv, 6, 1, "Marketplace listings", FT_MARKETPLACE_LISTINGS, false);
        folder(&mut inv, 10, 0, "Library", 8, true);
        folder(&mut inv, 11, 10, "Environments", -1, true);
        folder(&mut inv, 12, 11, "Skies", -1, true);
        folder(&mut inv, 13, 10, "Clothing", 5, true);
        inv
    }

    #[test]
    fn catalog_skips_trash_and_marketplace_items() {
        let mut inv = test_inventory();
        for (i, parent) in [2u128, 4, 5, 6, 12, 13].into_iter().enumerate() {
            let it = item(&format!("Sky {i}"), 0, 100 + i as u128, Uuid::from_u128(parent));
            inv.items.insert(it.id, it);
        }
        let c = SettingsCatalog::build(&inv);
        let names: Vec<&str> = c.sky.iter().map(|c| c.name.as_str()).collect();
        // Settings, Objects, library Environments and any other library folder
        assert_eq!(names, ["Sky 0", "Sky 2", "Sky 4", "Sky 5"]);
    }

    #[test]
    fn scan_fetches_library_environments_then_settings_then_the_rest() {
        let mut inv = test_inventory();
        let left = scan_step(&mut inv);
        // library Environments (2), Settings, My Inventory, Objects; never
        // Trash, Marketplace or the rest of the library
        assert_eq!(left, 5);
        let order: Vec<u128> = inv.queue.iter().map(|(id, _)| id.as_u128()).collect();
        assert_eq!(&order[..3], &[11, 12, 2]);
        assert_eq!(order.len(), 5);
        assert!(inv.queue.iter().take(2).all(|(_, lib)| *lib));
        // everything in flight: nothing new is queued, nothing left once fetched
        inv.queue.clear();
        assert_eq!(scan_step(&mut inv), 5);
        assert!(inv.queue.is_empty());
        for f in inv.folders.values_mut() {
            if f.state == FetchState::Fetching {
                f.state = FetchState::Fetched;
            }
        }
        assert_eq!(scan_step(&mut inv), 0);
    }

    #[test]
    fn arrows_skip_the_fixed_entries_and_wrap() {
        let list: Vec<SettingsChoice> = (1..=3)
            .map(|i| SettingsChoice {
                name: format!("S{i}"),
                asset: Uuid::from_u128(i),
            })
            .collect();
        let id = Uuid::from_u128;
        // from "Environnement partagé": > gives the first item, < the last
        assert_eq!(step(&list, shown_index(&list, Shown::Shared), true), Some(id(1)));
        assert_eq!(step(&list, shown_index(&list, Shown::Shared), false), Some(id(3)));
        assert_eq!(step(&list, shown_index(&list, Shown::DayBased), false), Some(id(3)));
        let at = |n| shown_index(&list, Shown::Item(id(n)));
        assert_eq!(step(&list, at(2), true), Some(id(3)));
        assert_eq!(step(&list, at(3), true), Some(id(1)), "wraps past the fixed entries");
        assert_eq!(step(&list, at(1), false), Some(id(3)));
        // an item that is not listed (yet) counts as the first entry
        assert_eq!(shown_index(&list, Shown::Item(id(9))), 0);
        assert_eq!(shown_index(&list, Shown::Preset(2)), 0);
        // an empty list: nothing to select (NoValidEnvSettingFound)
        assert_eq!(step(&[], 0, true), None);
        assert_eq!(step(&[], 1, false), None);
    }
}
