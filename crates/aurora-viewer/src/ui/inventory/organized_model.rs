//! Local Explorer navigation. Nothing in this module writes to the inventory model.
//! Persist one Preferences value per inventory root and never discard IDs on a partial fetch.

use crate::world::inventory::{Inventory, actions};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Location {
    // Through v3 the serialized Home value meant the list of personal root entries.
    #[serde(rename = "Overview")]
    Home,
    #[serde(rename = "Home")]
    MyFolders,
    Library,
    Group(Group),
    Folder(Uuid),
    Custom(Uuid),
    FullTree,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Group {
    Appearance,
    TexturesMaterials,
    MotionSound,
    ObjectsScripts,
    PlacesPeople,
    System,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectoryView {
    pub layout: ViewLayout,
    pub sort: SortKey,
    pub descending: bool,
    pub group: GroupBy,
    pub group_descending: bool,
    /// Type, date and location widths, in UI points. Name fills the remainder.
    /// Normal folders and search results keep independent measurements.
    #[serde(skip_serializing_if = "empty_column_widths")]
    pub column_widths: [[u16; 3]; 2],
}

impl DirectoryView {
    /// Home contains only destinations, with no creation dates. A Home search
    /// uses the stored item sort and grouping again.
    pub(super) fn for_dashboard(mut self) -> Self {
        if self.sort == SortKey::Date {
            self.sort = SortKey::Name;
        }
        self.group = GroupBy::None;
        self
    }
}

fn empty_column_widths(widths: &[[u16; 3]; 2]) -> bool {
    *widths == [[0; 3]; 2]
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum ViewLayout {
    #[default]
    Details,
    List,
    Icons(IconSize),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum IconSize {
    Small,
    #[default]
    Medium,
    Large,
    ExtraLarge,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Type,
    Date,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum GroupBy {
    #[default]
    None,
    Type,
    Name,
    Date,
}

/// Locale-independent identifiers also distinguish Home sections that have the same visible title.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub(super) enum GroupKey {
    Folder,
    VirtualFolder,
    Location,
    Item(super::aurora::rows::ItemType),
    Name(NameBucket),
    Date(DateBucket),
    HomeCategories,
    HomeUtilities,
    HomePinned,
    HomePinGroup(Uuid),
    HomeOther,
    HomeFolders,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub(super) enum NameBucket {
    Symbols,
    Numbers,
    AtoH,
    ItoP,
    QtoZ,
    Initial(char),
}

/// Ordered from oldest to newest. Unknown remains last in either direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub(super) enum DateBucket {
    Older,
    LastYear,
    ThisYear,
    LastMonth,
    ThisMonth,
    LastWeek,
    ThisWeek,
    Yesterday,
    Today,
    Future,
    Unknown,
}

// JSON object keys cannot represent a tagged Location such as Folder(uuid).
// Keep ordered lookups in memory and serialize location/view pairs losslessly.
mod view_locations {
    use super::Location;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;
    pub fn serialize<S: Serializer, T: Serialize>(views: &BTreeMap<Location, T>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(views.iter())
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<BTreeMap<Location, T>, D::Error> {
        Ok(Vec::<(Location, T)>::deserialize(deserializer)?.into_iter().collect())
    }
}

impl Group {
    pub const ALL: &'static [Self] = &[
        Self::Appearance,
        Self::TexturesMaterials,
        Self::MotionSound,
        Self::ObjectsScripts,
        Self::PlacesPeople,
        Self::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => aurora_i18n::tr!("explorer-model-appearance"),
            Self::TexturesMaterials => aurora_i18n::tr!("explorer-model-textures-materials"),
            Self::MotionSound => aurora_i18n::tr!("explorer-model-motion-sound"),
            Self::ObjectsScripts => aurora_i18n::tr!("explorer-model-objects-scripts"),
            Self::PlacesPeople => aurora_i18n::tr!("explorer-model-places-people"),
            Self::System => aurora_i18n::tr!("explorer-model-system"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Appearance => "shirt-filled",
            Self::TexturesMaterials => "palette-filled",
            Self::MotionSound => "music",
            Self::ObjectsScripts => "cube",
            Self::PlacesPeople => "users",
            Self::System => "settings-filled",
        }
    }
}

// These are preferred folder types (LLFolderType), not item or asset types.
// Descriptions apply only to direct children of this resident's root.
fn system_info(kind: i32) -> Option<(Option<Group>, &'static str, &'static str)> {
    use Group::*;
    Some(match kind {
        46 => (Some(Appearance), "explorer-model-current-outfit", "hanger-2-filled"),
        48 => (Some(Appearance), "explorer-model-saved-outfits", "shirt-filled"),
        5 => (Some(Appearance), "explorer-model-clothing", "jacket"),
        13 => (Some(Appearance), "explorer-model-body-parts", "user"),
        0 => (Some(TexturesMaterials), "explorer-model-textures", "photo-filled"),
        57 => (Some(TexturesMaterials), "explorer-model-materials", "sphere"),
        20 => (Some(MotionSound), "explorer-model-animations", "walk"),
        21 => (Some(MotionSound), "explorer-model-gestures", "hand-stop"),
        1 => (Some(MotionSound), "explorer-model-sounds", "volume"),
        6 => (Some(ObjectsScripts), "explorer-model-objects", "cube"),
        10 => (Some(ObjectsScripts), "explorer-model-scripts", "code"),
        3 => (Some(PlacesPeople), "explorer-model-saved-places", "map-pin-filled"),
        23 => (Some(PlacesPeople), "explorer-model-favorite-places", "star"),
        2 => (Some(PlacesPeople), "explorer-model-contact-cards", "id-filled"),
        15 => (None, "explorer-model-photos", "Inv_Snapshot"),
        7 => (None, "explorer-model-notecards", "Inv_Notecard"),
        56 => (None, "explorer-model-environments", "Inv_Settings"),
        50 => (None, "explorer-model-marketplace-deliveries", "inbox"),
        53 => (None, "explorer-model-marketplace-listings", "building-store"),
        16 => (None, "explorer-model-lost-found", "archive-filled"),
        14 => (None, "explorer-model-trash", "Inv_TrashClosed"),
        _ => return None,
    })
}

// The shared viewer cache can return a duplicate or a nested fallback. An
// Explorer system shortcut must resolve to an owned direct child of the
// requested type, and must never consume a reserved client folder's UUID.
pub(super) fn preferred_root(inv: &Inventory, kind: i32) -> Option<Uuid> {
    let cached = inv.system_folder(kind)?;
    let eligible = |&id: &Uuid| {
        inv.folders.get(&id).is_some_and(|f| {
            !f.library
                && id != inv.lib_root
                && id != inv.root
                && f.info.parent == inv.root
                && f.info.type_default == kind
                && !actions::pinned(inv, id)
        })
    };
    if eligible(&cached) {
        return Some(cached);
    }
    inv.folders.get(&inv.root)?.children.iter().filter(|id| eligible(id)).min().copied()
}

// RLVa itself accepts only a direct #RLV child of the resident's root.
// Explorer also gives a unique, misplaced Photos/#RLV a display location.
// A direct shared root takes precedence, and an ambiguous pair stays where
// it is. Only inspect the loaded contents of the actual snapshot folder.
fn misplaced_rlv(inv: &Inventory) -> Option<Uuid> {
    let photos = preferred_root(inv, 15)?;
    let folder = inv.folders.get(&photos)?;
    let mut matches = folder.children.iter().copied().filter(|id| {
        inv.folders
            .get(id)
            .is_some_and(|f| !f.library && f.info.parent == photos && f.info.name == "#RLV")
    });
    let candidate = matches.next()?;
    if matches.any(|id| id != candidate) {
        return None;
    }
    let direct = inv
        .folders
        .values()
        .any(|f| !f.library && f.info.parent == inv.root && f.info.name == "#RLV");
    (!direct).then_some(candidate)
}

fn root_info(inv: &Inventory, id: Uuid) -> Option<(Option<Group>, &'static str, &'static str)> {
    let folder = inv.folders.get(&id)?;
    if folder.library || id == inv.lib_root || id == inv.root {
        return None;
    }
    if folder.info.parent == inv.root {
        // Aurora's own reserved-folder and RLVa checks use the literal name
        // and parent. Some imported or legacy folders carry a conflicting type.
        if actions::pinned(inv, id) {
            return Some((Some(Group::System), "explorer-model-system", "settings-filled"));
        }
        return system_info(folder.info.type_default);
    }
    if folder.info.name == "#RLV"
        && inv
            .folders
            .get(&folder.info.parent)
            .is_some_and(|f| f.info.parent == inv.root && f.info.type_default == 15 && !f.library)
        && misplaced_rlv(inv) == Some(id)
    {
        return Some((Some(Group::System), "explorer-model-system", "settings-filled"));
    }
    None
}

/// True for the curated entry points that are available under All Locations.
#[cfg(test)]
pub fn is_managed_root(inv: &Inventory, id: Uuid) -> bool {
    root_info(inv, id).is_some()
}

pub(super) fn folder_symbol(inv: &Inventory, id: Uuid) -> Option<&'static str> {
    root_info(inv, id).map(|(_, _, glyph)| glyph)
}

pub fn label(inv: &Inventory, id: Uuid) -> &str {
    if id == inv.lib_root && !id.is_nil() {
        return aurora_i18n::tr!("explorer-model-second-life-library");
    }
    root_info(inv, id)
        .map(|(group, key, _)| {
            if group == Some(Group::System) {
                inv.folders[&id].info.name.as_str()
            } else {
                aurora_i18n::text(key)
            }
        })
        .or_else(|| inv.folders.get(&id).map(|folder| folder.info.name.as_str()))
        .or_else(|| inv.items.get(&id).map(|item| item.name.as_str()))
        .unwrap_or("")
}

/// A Home folder container belongs to this account's personal root. A same-named
/// folder, a former container moved to Trash, and a root renamed to #RLV
/// must never become destinations for new inventory.
pub(super) fn home_folder(inv: &Inventory, layout: &Preferences) -> Option<Uuid> {
    let id = layout.home_folder?;
    let folder = inv.folders.get(&id)?;
    (id != inv.root
        && id != inv.lib_root
        && !inv.root.is_nil()
        && !folder.library
        && folder.info.parent == inv.root
        && folder.info.type_default == -1
        && !actions::pinned(inv, id))
    .then_some(id)
}

fn natural_parent(inv: &Inventory, layout: &Preferences, id: Uuid) -> Location {
    if let Some((group, _, _)) = root_info(inv, id) {
        return group.map_or(Location::Home, Location::Group);
    }
    let parent = actions::parent(inv, id);
    if parent.is_some() && parent == home_folder(inv, layout) {
        // Other clients can place loose items inside the backing folder.
        return if inv.folders.contains_key(&id) {
            Location::Home
        } else {
            Location::MyFolders
        };
    }
    match parent {
        Some(id) if !id.is_nil() && id == inv.lib_root => Location::Library,
        Some(id) if !id.is_nil() && id != inv.root => Location::Folder(id),
        _ => Location::MyFolders,
    }
}

/// Canonical viewer-managed content, including the uniquely recognized misplaced RLV root.
pub(super) fn client_managed(inv: &Inventory, id: Uuid) -> bool {
    actions::ancestors(inv, id).any(|id| matches!(root_info(inv, id), Some((Some(Group::System), _, _))))
}

/// Local aliases may refer to ordinary owned rows. System entry points keep
/// their stable home, and trashed rows cannot leak back into a personal view.
pub fn can_organize(inv: &Inventory, id: Uuid) -> bool {
    if id.is_nil() || id == inv.root || id == inv.lib_root {
        return false;
    }
    if let Some(folder) = inv.folders.get(&id) {
        if folder.info.type_default != -1 {
            return false;
        }
    } else if !inv.items.contains_key(&id) {
        return false;
    }
    let parent = if inv.folders.contains_key(&id) {
        Some(id)
    } else {
        actions::parent(inv, id)
    };
    parent.is_some_and(|folder| personal_contents(inv, folder))
}

/// A complete personal ancestry, excluding stock, protected and viewer-managed locations.
pub(super) fn personal_contents(inv: &Inventory, id: Uuid) -> bool {
    let mut cursor = Some(id);
    let mut visited = HashSet::new();
    while let Some(current) = cursor {
        if current == inv.root {
            return !inv.root.is_nil();
        }
        let Some(folder) = inv.folders.get(&current) else { return false };
        if !visited.insert(current)
            || folder.library
            || current == inv.lib_root
            || matches!(folder.info.type_default, 14 | 46 | 50 | 53)
            || matches!(root_info(inv, current), Some((Some(Group::System), _, _)))
        {
            return false;
        }
        cursor = Some(folder.info.parent);
    }
    false
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Pin {
    Place(Location),
    /// Default pins resolve when their own system folder arrives. Unpinning
    /// one before a logout remains effective even while the next login loads.
    SystemFolder(i32),
}

impl Pin {
    fn resolve(self, inv: &Inventory) -> Option<Location> {
        match self {
            Self::Place(place) => Some(place),
            Self::SystemFolder(kind) => preferred_root(inv, kind).map(Location::Folder),
        }
    }
}

// Earlier layouts stored bare Pin values. New records keep their slot and
// group even when unpinned, including unresolved system-folder shortcuts.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SidebarPin {
    #[serde(flatten)]
    target: Pin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<Uuid>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hidden: bool,
}

impl From<Pin> for SidebarPin {
    fn from(target: Pin) -> Self {
        Self {
            target,
            group: None,
            hidden: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct PinGroup {
    pub id: Uuid,
    pub name: String,
    pub collapsed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct LocalFolder {
    name: String,
    parent: Location,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    #[serde(default)]
    pin_width: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    home_folder: Option<Uuid>,
    shortcuts: Vec<SidebarPin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pin_groups: Vec<PinGroup>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    ungrouped_collapsed: bool,
    placements: BTreeMap<Uuid, Location>,
    folders: BTreeMap<Uuid, LocalFolder>,
    #[serde(default, with = "view_locations", skip_serializing_if = "BTreeMap::is_empty")]
    views: BTreeMap<Location, DirectoryView>,
    #[serde(default, with = "view_locations", skip_serializing_if = "BTreeMap::is_empty")]
    collapsed_groups: BTreeMap<Location, BTreeSet<GroupKey>>,
    #[serde(default, with = "view_locations", skip_serializing_if = "BTreeMap::is_empty")]
    folder_symbols: BTreeMap<Location, String>,
    #[serde(skip)]
    pub revision: u64,
    #[serde(skip)]
    collapse_revision: u64,
}

impl Default for Preferences {
    fn default() -> Self {
        let shortcuts = Self::default_shortcuts().map(SidebarPin::from).collect();
        Self {
            pin_width: None,
            home_folder: None,
            shortcuts,
            pin_groups: Vec::new(),
            ungrouped_collapsed: false,
            placements: Default::default(),
            folders: Default::default(),
            views: Default::default(),
            collapsed_groups: Default::default(),
            folder_symbols: Default::default(),
            revision: 0,
            collapse_revision: 0,
        }
    }
}

impl Preferences {
    fn default_shortcuts() -> impl Iterator<Item = Pin> {
        Group::ALL
            .iter()
            .filter(|g| **g != Group::System)
            .map(|g| Pin::Place(Location::Group(*g)))
            .chain([15, 7, 56, 50, 53].map(Pin::SystemFolder))
            .chain([
                Pin::Place(Location::Library),
                Pin::Place(Location::Group(Group::System)),
                Pin::SystemFolder(16),
                Pin::SystemFolder(14),
            ])
    }
    pub(super) fn folder_icon(&self, place: Location) -> Option<&str> {
        self.folder_symbols.get(&place).map(String::as_str)
    }

    pub(super) fn set_folder_icon(&mut self, place: Location, symbol: Option<&str>) {
        if self.folder_icon(place) != symbol {
            if let Some(name) = symbol {
                self.folder_symbols.insert(place, name.to_owned());
            } else {
                self.folder_symbols.remove(&place);
            }
            self.revision += 1;
        }
    }

    pub(super) fn set_home_folder(&mut self, id: Uuid) {
        if self.home_folder != Some(id) {
            self.home_folder = Some(id);
            self.revision += 1;
        }
    }

    pub(super) fn is_home_child(&self, inv: &Inventory, id: Uuid) -> bool {
        inv.folders.get(&id).is_some_and(|f| Some(f.info.parent) == home_folder(inv, self)) && self.parent(inv, id) == Location::Home
    }

    pub(super) fn home_children(&self, inv: &Inventory) -> Vec<Location> {
        let Some(folder) = home_folder(inv, self).and_then(|id| inv.folders.get(&id)) else {
            return Vec::new();
        };
        folder
            .children
            .iter()
            .copied()
            .filter(|id| self.is_home_child(inv, *id))
            .map(Location::Folder)
            .collect()
    }

    pub(super) fn pin_width(&self) -> Option<u16> {
        self.pin_width
    }

    pub(super) fn set_pin_width(&mut self, width: u16) {
        if self.pin_width != Some(width) {
            self.pin_width = Some(width);
            self.revision += 1;
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Collapsing a heading changes presentation, but not which rows match or their order.
    pub(super) fn ordering_revision(&self) -> u64 {
        self.revision - self.collapse_revision
    }

    fn default_view(location: Location) -> DirectoryView {
        if location == Location::Home {
            DirectoryView {
                layout: ViewLayout::Icons(IconSize::Large),
                ..DirectoryView::default()
            }
        } else {
            DirectoryView::default()
        }
    }

    pub fn view(&self, location: Location) -> DirectoryView {
        self.views.get(&location).copied().unwrap_or_else(|| Self::default_view(location))
    }

    pub(super) fn migrate_group_direction(&mut self) {
        for view in self.views.values_mut() {
            view.group_descending = view.group == GroupBy::Type && view.sort == SortKey::Type && view.descending;
        }
    }

    pub(super) fn group_collapsed(&self, location: Location, key: &GroupKey) -> bool {
        self.collapsed_groups.get(&location).is_some_and(|keys| keys.contains(key))
    }

    pub(super) fn set_group_collapsed(&mut self, location: Location, key: &GroupKey, collapsed: bool) -> bool {
        self.set_groups_collapsed(location, [key], collapsed)
    }

    pub(super) fn set_groups_collapsed<'a>(
        &mut self,
        location: Location,
        keys: impl IntoIterator<Item = &'a GroupKey>,
        collapsed: bool,
    ) -> bool {
        let mut changed = false;
        for key in keys {
            if collapsed {
                changed |= self.collapsed_groups.entry(location).or_default().insert(*key);
            } else if let Some(stored) = self.collapsed_groups.get_mut(&location) {
                changed |= stored.remove(key);
            }
        }
        if changed {
            if self.collapsed_groups.get(&location).is_some_and(BTreeSet::is_empty) {
                self.collapsed_groups.remove(&location);
            }
            self.revision += 1;
            self.collapse_revision += 1;
        }
        changed
    }

    pub fn set_view(&mut self, location: Location, view: DirectoryView) -> bool {
        if self.view(location) == view {
            return false;
        }
        if view == Self::default_view(location) {
            self.views.remove(&location);
        } else {
            self.views.insert(location, view);
        }
        self.revision += 1;
        true
    }

    pub fn available(&self, inv: &Inventory, place: Location) -> bool {
        match place {
            Location::Home | Location::MyFolders | Location::FullTree | Location::Group(_) => true,
            Location::Library => !inv.lib_root.is_nil() && inv.folders.contains_key(&inv.lib_root),
            Location::Folder(id) => !id.is_nil() && inv.folders.contains_key(&id),
            Location::Custom(id) => self.folders.contains_key(&id),
        }
    }

    pub fn location_label<'a>(&'a self, inv: &'a Inventory, place: Location) -> &'a str {
        match place {
            Location::Home => aurora_i18n::tr!("common-home"),
            Location::MyFolders => aurora_i18n::tr!("explorer-general-inventory"),
            Location::FullTree => aurora_i18n::tr!("explorer-model-all-inventory"),
            Location::Library => aurora_i18n::tr!("explorer-model-second-life-library"),
            Location::Folder(id) => label(inv, id),
            Location::Group(group) => group.label(),
            Location::Custom(id) => self.folders.get(&id).map_or("", |folder| &folder.name),
        }
    }

    fn pin_position(&self, inv: &Inventory, place: Location) -> Option<usize> {
        self.shortcuts.iter().position(|pin| pin.target.resolve(inv) == Some(place))
    }

    /// Every built-in shortcut that can currently be pinned, including ones
    /// removed from Quick Access. The stock Library is a separate root.
    pub fn catalog(&self, inv: &Inventory) -> Vec<Location> {
        Self::default_shortcuts()
            .filter_map(|pin| pin.resolve(inv))
            .filter(|place| self.available(inv, *place))
            .collect()
    }

    /// All built-in entry points on Home, independent of pin visibility. Some inventories
    /// contain several real direct folders of the same preferred type; each has its own ID.
    pub(super) fn home_roots(&self, inv: &Inventory) -> Vec<Location> {
        let mut roots = self.catalog(inv);
        let mut known: HashSet<_> = roots.iter().copied().collect();
        if let Some(root) = inv.folders.get(&inv.root) {
            for &id in &root.children {
                let place = Location::Folder(id);
                if matches!(root_info(inv, id), Some((None, _, _))) && known.insert(place) {
                    roots.push(place);
                }
            }
        }
        roots
    }

    fn visible_pins<'a>(&'a self, inv: &'a Inventory) -> impl Iterator<Item = (Option<Uuid>, Location)> + 'a {
        let mut seen = HashSet::new();
        self.shortcuts.iter().filter(|pin| !pin.hidden).filter_map(move |pin| {
            let place = pin.target.resolve(inv)?;
            (self.available(inv, place) && seen.insert(place)).then_some((pin.group, place))
        })
    }

    pub fn pins(&self, inv: &Inventory) -> Vec<Location> {
        self.visible_pins(inv).map(|(_, place)| place).collect()
    }

    pub(super) fn pin_sections(&self, inv: &Inventory) -> Vec<(Option<PinGroup>, Vec<Location>)> {
        let mut sections: Vec<_> = self.pin_groups.iter().cloned().map(|g| (Some(g), Vec::new())).collect();
        let index: BTreeMap<_, _> = self.pin_groups.iter().enumerate().map(|(i, group)| (group.id, i)).collect();
        let ungrouped = sections.len();
        sections.push((None, Vec::new()));
        for (group, place) in self.visible_pins(inv) {
            let section = group.and_then(|g| index.get(&g).copied()).unwrap_or(ungrouped);
            sections[section].1.push(place);
        }
        sections
    }

    pub(super) fn pin_groups(&self) -> &[PinGroup] {
        &self.pin_groups
    }

    pub(super) fn ungrouped_collapsed(&self) -> bool {
        self.ungrouped_collapsed
    }

    pub(super) fn create_pin_group(&mut self, name: &str) -> Option<Uuid> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let id = Uuid::new_v4();
        self.pin_groups.push(PinGroup {
            id,
            name: name.into(),
            collapsed: false,
        });
        self.revision += 1;
        Some(id)
    }

    pub(super) fn rename_pin_group(&mut self, id: Uuid, name: &str) -> bool {
        let Some(group) = self.pin_groups.iter_mut().find(|g| g.id == id) else {
            return false;
        };
        let name = name.trim();
        if name.is_empty() || group.name == name {
            return false;
        }
        group.name = name.into();
        self.revision += 1;
        true
    }

    pub(super) fn toggle_pin_group(&mut self, id: Option<Uuid>) {
        let collapsed = match id {
            Some(id) => {
                let Some(group) = self.pin_groups.iter_mut().find(|g| g.id == id) else {
                    return;
                };
                &mut group.collapsed
            }
            None => &mut self.ungrouped_collapsed,
        };
        *collapsed = !*collapsed;
        self.revision += 1;
    }

    pub(super) fn shift_pin_group(&mut self, id: Uuid, up: bool) -> bool {
        let Some(index) = self.pin_groups.iter().position(|g| g.id == id) else {
            return false;
        };
        let other = if up {
            index.checked_sub(1)
        } else {
            (index + 1 < self.pin_groups.len()).then_some(index + 1)
        };
        let Some(other) = other else { return false };
        self.pin_groups.swap(index, other);
        self.revision += 1;
        true
    }

    pub(super) fn remove_pin_group(&mut self, id: Uuid) -> bool {
        let Some(index) = self.pin_groups.iter().position(|g| g.id == id) else {
            return false;
        };
        self.pin_groups.remove(index);
        if let Some(keys) = self.collapsed_groups.get_mut(&Location::Home) {
            keys.remove(&GroupKey::HomePinGroup(id));
            if keys.is_empty() {
                self.collapsed_groups.remove(&Location::Home);
            }
        }
        for pin in self.shortcuts.iter_mut().filter(|p| p.group == Some(id)) {
            pin.group = None;
        }
        if self.pin_groups.is_empty() {
            self.ungrouped_collapsed = false;
        }
        self.revision += 1;
        true
    }

    pub fn pin_location(&mut self, inv: &Inventory, place: Location) -> bool {
        if !self.available(inv, place)
            || matches!(place, Location::Home | Location::FullTree)
            || matches!(place, Location::Folder(id) if id == inv.root || id == inv.lib_root)
        {
            return false;
        }
        if let Some(index) = self.pin_position(inv, place) {
            if !self.shortcuts[index].hidden {
                return false;
            }
            self.shortcuts[index].hidden = false;
        } else {
            let pin = Self::default_shortcuts()
                .find(|pin| pin.resolve(inv) == Some(place))
                .unwrap_or(Pin::Place(place));
            self.shortcuts.push(pin.into());
        }
        self.revision += 1;
        true
    }

    pub fn unpin_location(&mut self, inv: &Inventory, place: Location) -> bool {
        let Some(index) = self.pin_position(inv, place) else { return false };
        if self.shortcuts[index].hidden {
            return false;
        }
        self.shortcuts[index].hidden = true;
        self.revision += 1;
        true
    }

    pub(super) fn move_pin_to_group(&mut self, inv: &Inventory, place: Location, group: Option<Uuid>) -> bool {
        if group.is_some_and(|id| !self.pin_groups.iter().any(|g| g.id == id)) {
            return false;
        }
        let Some(index) = self.pin_position(inv, place) else { return false };
        if self.shortcuts[index].hidden || self.shortcuts[index].group == group {
            return false;
        }
        let mut pin = self.shortcuts.remove(index);
        pin.group = group;
        self.shortcuts.push(pin);
        self.revision += 1;
        true
    }

    pub fn shift_location(&mut self, inv: &Inventory, place: Location, up: bool) -> bool {
        let Some(index) = self.pin_position(inv, place) else { return false };
        let group = self.shortcuts[index].group;
        let neighbor = {
            let mut pins = self.visible_pins(inv).filter(|(g, _)| *g == group).map(|(_, p)| p);
            if up {
                pins.take_while(|&p| p != place).last()
            } else {
                pins.find(|&p| p == place).and_then(|_| pins.next())
            }
        };
        neighbor.is_some_and(|next| self.reorder_pin(inv, place, next, up))
    }

    pub fn reorder_pin(&mut self, inv: &Inventory, source: Location, target: Location, before: bool) -> bool {
        let (Some(from), Some(to)) = (self.pin_position(inv, source), self.pin_position(inv, target)) else {
            return false;
        };
        if from == to || self.shortcuts[from].hidden || self.shortcuts[to].hidden {
            return false;
        }
        let group = self.shortcuts[to].group;
        let insert = to + usize::from(!before) - usize::from(from < to);
        if insert == from && self.shortcuts[from].group == group {
            return false;
        }
        let mut pin = self.shortcuts.remove(from);
        pin.group = group;
        self.shortcuts.insert(insert, pin);
        self.revision += 1;
        true
    }

    /// A category or ordinary content folder may receive local placements.
    /// Viewer-managed, read-only and action-specific locations keep their contents.
    pub fn destination(&self, inv: &Inventory, to: Location) -> bool {
        match to {
            Location::MyFolders => true,
            Location::Custom(id) => self.folders.contains_key(&id),
            Location::Group(group) => group != Group::System,
            Location::Folder(id) => inv.folders.get(&id).is_some_and(|folder| {
                if folder.info.type_default == -1 {
                    return can_organize(inv, id);
                }
                // These SL folders hold ordinary content. Their entry points
                // are fixed but a *local* placement inside them is harmless.
                !folder.library
                    && id != inv.lib_root
                    && folder.info.parent == inv.root
                    && !actions::pinned(inv, id)
                    && matches!(
                        folder.info.type_default,
                        0 | 1 | 2 | 3 | 5 | 6 | 7 | 10 | 13 | 15 | 20 | 21 | 23 | 56 | 57
                    )
            }),
            _ => false,
        }
    }

    // Single projected step, *without* cycle checking: the walker below
    // validates the complete path, including native edges changed in another viewer.
    fn step(&self, inv: &Inventory, place: Location) -> Option<Location> {
        match place {
            Location::Folder(id) => Some(
                self.placements
                    .get(&id)
                    .copied()
                    .filter(|p| can_organize(inv, id) && self.destination(inv, *p))
                    .unwrap_or_else(|| natural_parent(inv, self, id)),
            ),
            Location::Custom(id) => self.folders.get(&id).map(|folder| {
                if self.destination(inv, folder.parent) {
                    folder.parent
                } else {
                    Location::MyFolders
                }
            }),
            Location::Group(_) | Location::Library | Location::FullTree | Location::MyFolders => Some(Location::Home),
            Location::Home => None,
        }
    }

    fn acyclic(&self, inv: &Inventory, start: Location, excluded: Location) -> bool {
        let mut seen = HashSet::new();
        let mut current = Some(start);
        while let Some(place) = current {
            if place == excluded || !seen.insert(place) {
                return false;
            }
            current = self.step(inv, place);
        }
        true
    }

    pub fn parent(&self, inv: &Inventory, id: Uuid) -> Location {
        if let Some(&parent) = self.placements.get(&id)
            && can_organize(inv, id)
            && self.destination(inv, parent)
            && self.acyclic(inv, parent, Location::Folder(id))
        {
            return parent;
        }
        natural_parent(inv, self, id)
    }

    pub fn location_parent(&self, inv: &Inventory, place: Location) -> Option<Location> {
        match place {
            Location::Folder(id) => Some(self.parent(inv, id)),
            Location::Custom(_) => self
                .step(inv, place)
                .map(|p| if self.acyclic(inv, p, place) { p } else { Location::MyFolders }),
            _ => self.step(inv, place),
        }
    }

    /// Validate against the complete selected forest before writing. If
    /// a folder and a descendant are both selected, move the ancestor and
    /// preserve the descendant's place, including its existing local override.
    fn move_roots(&self, inv: &Inventory, ids: &[Uuid], customs: &[Uuid], to: Location) -> Option<(Vec<Uuid>, Vec<Uuid>)> {
        if (ids.is_empty() && customs.is_empty())
            || !self.destination(inv, to)
            || !ids.iter().all(|id| can_organize(inv, *id))
            || !customs.iter().all(|id| self.folders.contains_key(id))
        {
            return None;
        }
        let containers: HashSet<_> = ids
            .iter()
            .filter(|id| inv.folders.contains_key(id))
            .copied()
            .map(Location::Folder)
            .chain(customs.iter().copied().map(Location::Custom))
            .collect();
        let mut here = Some(to);
        let mut seen = HashSet::new();
        while let Some(loc) = here {
            if containers.contains(&loc) || !seen.insert(loc) {
                return None;
            }
            here = self.step(inv, loc);
        }
        let mut top_level = |mut parent: Option<Location>| {
            seen.clear();
            while let Some(loc) = parent {
                if containers.contains(&loc) || !seen.insert(loc) {
                    return false;
                }
                parent = self.location_parent(inv, loc);
            }
            true
        };
        let mut distinct = HashSet::new();
        let rows: Vec<_> = ids
            .iter()
            .copied()
            .filter(|id| {
                if !distinct.insert(*id) {
                    return false;
                }
                let parent = self.parent(inv, *id);
                parent != to && top_level(Some(parent))
            })
            .collect();
        distinct.clear();
        let locals: Vec<_> = customs
            .iter()
            .copied()
            .filter(|id| {
                if !distinct.insert(*id) {
                    return false;
                }
                let parent = self.location_parent(inv, Location::Custom(*id));
                parent != Some(to) && top_level(parent)
            })
            .collect();
        (!rows.is_empty() || !locals.is_empty()).then_some((rows, locals))
    }

    pub fn can_move_entries(&self, inv: &Inventory, ids: &[Uuid], customs: &[Uuid], to: Location) -> bool {
        self.move_roots(inv, ids, customs, to).is_some()
    }

    pub fn move_entries(&mut self, inv: &Inventory, ids: &[Uuid], customs: &[Uuid], to: Location) -> bool {
        let Some((rows, locals)) = self.move_roots(inv, ids, customs, to) else {
            return false;
        };
        for id in rows {
            if natural_parent(inv, self, id) == to {
                self.placements.remove(&id);
            } else {
                self.placements.insert(id, to);
            }
        }
        for id in locals {
            self.folders.get_mut(&id).unwrap().parent = to;
        }
        self.revision += 1;
        true
    }

    #[cfg(test)]
    pub fn place(&mut self, inv: &Inventory, id: Uuid, to: Location) -> bool {
        self.move_entries(inv, &[id], &[], to)
    }

    pub fn restore(&mut self, id: Uuid) -> bool {
        if self.placements.remove(&id).is_some() {
            self.revision += 1;
            return true;
        }
        false
    }

    pub fn is_placed(&self, id: Uuid) -> bool {
        self.placements.contains_key(&id)
    }

    #[cfg(test)]
    pub fn create_custom(&mut self, inv: &Inventory, parent: Location, name: &str) -> Option<Location> {
        let name = name.trim();
        if name.is_empty() || !self.destination(inv, parent) {
            return None;
        }
        let id = Uuid::new_v4();
        self.folders.insert(
            id,
            LocalFolder {
                name: name.to_owned(),
                parent,
            },
        );
        self.revision += 1;
        Some(Location::Custom(id))
    }

    pub fn remove_custom(&mut self, id: Uuid) -> bool {
        let Some(folder) = self.folders.remove(&id) else { return false };
        let old = Location::Custom(id);
        self.views.remove(&old);
        self.collapsed_groups.remove(&old);
        self.folder_symbols.remove(&old);
        self.shortcuts.retain(|pin| pin.target != Pin::Place(old));
        for child in self.folders.values_mut().filter(|child| child.parent == old) {
            child.parent = folder.parent;
        }
        for parent in self.placements.values_mut().filter(|parent| **parent == old) {
            *parent = folder.parent;
        }
        self.revision += 1;
        true
    }

    pub fn custom_locations(&self) -> impl Iterator<Item = Location> + '_ {
        self.folders.keys().copied().map(Location::Custom)
    }

    pub fn virtual_children(&self, inv: &Inventory, place: Location) -> Vec<Location> {
        let mut local: Vec<_> = self
            .folders
            .iter()
            .filter(|(id, _)| self.location_parent(inv, Location::Custom(**id)) == Some(place))
            .collect();
        local.sort_by_cached_key(|(id, folder)| (folder.name.to_lowercase(), **id));
        let mut children: Vec<_> = local.into_iter().map(|(id, _)| Location::Custom(*id)).collect();
        if place == Location::Home {
            children.push(Location::MyFolders);
            children.extend(Group::ALL.iter().copied().map(Location::Group));
        }
        children
    }

    /// Ordinary browsing only visits direct children and explicit placements.
    /// Opening System also reconciles reserved folder records with stale child lists.
    pub fn children(&self, inv: &Inventory, place: Location) -> Vec<Uuid> {
        let canonical = match place {
            Location::Home | Location::MyFolders | Location::Group(_) | Location::FullTree => Some(inv.root),
            Location::Library => Some(inv.lib_root),
            Location::Folder(id) => Some(id),
            Location::Custom(_) => None,
        };
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        let backing = home_folder(inv, self);
        let projected = matches!(place, Location::Home | Location::MyFolders).then_some(backing).flatten();
        for source in canonical.into_iter().chain(projected) {
            let Some(folder) = inv.folders.get(&source) else { continue };
            for &id in folder.children.iter().chain(&folder.items) {
                if (Some(id) == backing && place != Location::FullTree) || actions::parent(inv, id) != Some(source) {
                    continue;
                }
                if (place == Location::FullTree || self.parent(inv, id) == place) && seen.insert(id) {
                    result.push(id);
                }
            }
        }
        if place == Location::Group(Group::System) {
            // A stale child index can omit the canonical client root even
            // while its live record already identifies the correct parent.
            for folder in inv
                .folders
                .values()
                .filter(|f| f.info.parent == inv.root && actions::pinned(inv, f.info.id))
            {
                if seen.insert(folder.info.id) {
                    result.push(folder.info.id);
                }
            }
            if let Some(id) = misplaced_rlv(inv)
                && seen.insert(id)
            {
                result.push(id);
            }
        }
        if place != Location::FullTree {
            for (&id, &parent) in &self.placements {
                if Some(id) != backing && parent == place && can_organize(inv, id) && self.parent(inv, id) == place && seen.insert(id) {
                    result.push(id);
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(inv: &mut Inventory, n: u128, p: u128, kind: i32, name: &str, library: bool) {
        let key = id(n);
        let parent = id(p);
        let info = InvFolder {
            id: key,
            parent,
            name: name.into(),
            type_default: kind,
            ..Default::default()
        };
        inv.folders.insert(
            key,
            Folder {
                info,
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library,
            },
        );
        if let Some(parent) = inv.folders.get_mut(&parent) {
            parent.children.push(key);
        }
        inv.generation += 1;
    }

    fn fixture() -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(2),
            ..Default::default()
        };
        folder(&mut inv, 1, 0, 8, "Inventory", false);
        folder(&mut inv, 2, 0, 8, "Library", true);
        folder(&mut inv, 3, 1, 6, "Objets", false);
        folder(&mut inv, 4, 2, 6, "Objects", true);
        folder(&mut inv, 5, 1, -1, "Projects", false);
        folder(&mut inv, 6, 1, -1, "#Aurora", false);
        folder(&mut inv, 7, 5, -1, "#Aurora", false);
        folder(&mut inv, 8, 1, -1, "#Brand", false);
        folder(&mut inv, 9, 1, -1, "MarketPlace", false);
        folder(&mut inv, 10, 1, 53, "MarketPlace", false);
        folder(&mut inv, 11, 1, 14, "Corbeille", false);
        folder(&mut inv, 12, 1, -1, "Objects", false);
        let item = InvItem {
            id: id(13),
            parent: id(5),
            name: "No-copy original".into(),
            owner_mask: 0,
            ..Default::default()
        };
        inv.folders.get_mut(&item.parent).unwrap().items.push(item.id);
        inv.items.insert(item.id, item);
        inv
    }

    #[test]
    fn organized_v3_home_layout_placements_and_local_folders_belong_to_personal_root() {
        let inv = fixture();
        let custom = Uuid::new_v4();
        let ordinary = id(5);
        let data = serde_json::json!({
            "views": [["Home", { "layout": "List" }]],
            "placements": {ordinary.to_string(): "Home"},
            "folders": {custom.to_string(): { "name": "Older local folder", "parent": "Home" }}
        });
        let mut layout: Preferences = serde_json::from_value(data).unwrap();
        assert_eq!(layout.view(Location::Home).layout, ViewLayout::Icons(IconSize::Large));
        assert_eq!(layout.view(Location::MyFolders).layout, ViewLayout::List);
        assert_eq!(layout.parent(&inv, ordinary), Location::MyFolders);
        assert_eq!(layout.location_parent(&inv, Location::Custom(custom)), Some(Location::MyFolders));
        assert!(layout.set_view(Location::Home, DirectoryView::default()));
        let reopened: Preferences = serde_json::from_slice(&serde_json::to_vec(&layout).unwrap()).unwrap();
        assert_eq!(reopened.view(Location::Home).layout, ViewLayout::Details);
        assert_eq!(reopened.view(Location::MyFolders).layout, ViewLayout::List);
        assert_eq!(reopened.location_parent(&inv, Location::MyFolders), Some(Location::Home));
        assert!(!reopened.destination(&inv, Location::Home));
    }

    #[test]
    fn classification_uses_owner_type_and_exact_root_names() {
        let inv = fixture();
        let layout = Preferences::default();
        assert_eq!(layout.children(&inv, Location::Group(Group::ObjectsScripts)), [id(3)]);
        assert_eq!(label(&inv, id(3)), "Objects");
        assert_eq!(layout.children(&inv, Location::Group(Group::System)), [id(6)]);
        assert_eq!(layout.children(&inv, Location::Library), [id(4)]);
        assert!(!can_organize(&inv, id(4)));
        assert!(!can_organize(&inv, id(6)));
        assert!(can_organize(&inv, id(7)));
        assert_eq!(label(&inv, id(9)), "MarketPlace");
        assert!(can_organize(&inv, id(9)));
        assert_eq!(label(&inv, id(10)), "Marketplace Listings");
        assert!(!can_organize(&inv, id(10)));
        assert!(layout.children(&inv, Location::MyFolders).contains(&id(12)));
    }

    #[test]
    fn protects_real_client_subtrees_but_allows_ordinary_descendants_of_clothing() {
        let mut inv = fixture();
        folder(&mut inv, 14, 6, -1, "Storage", false);
        folder(&mut inv, 15, 1, 5, "Clothing", false);
        folder(&mut inv, 16, 15, -1, "Costumes", false);
        let mut layout = Preferences::default();
        assert!(is_managed_root(&inv, id(6)));
        assert!(!is_managed_root(&inv, id(7)));
        assert!(!can_organize(&inv, id(14)));
        assert!(!layout.destination(&inv, Location::Folder(id(14))));
        assert!(can_organize(&inv, id(16)));
        assert!(layout.place(&inv, id(13), Location::Folder(id(16))));
        inv.items.get_mut(&id(13)).unwrap().parent = id(14);
        assert!(!can_organize(&inv, id(13)));
        assert_eq!(layout.parent(&inv, id(13)), Location::Folder(id(14)));
        inv.folders.get_mut(&id(16)).unwrap().info.parent = id(16);
        assert!(!can_organize(&inv, id(16)));
    }

    #[test]
    fn sidebar_groups_preserve_membership_order_and_hidden_slots() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        let a = Location::Group(Group::Appearance);
        let b = Location::Library;
        let c = Location::Folder(id(7));
        assert!(layout.pin_location(&inv, c));
        assert_eq!(layout.pin_sections(&inv), [(None, layout.pins(&inv))]);
        assert_eq!(layout.create_pin_group(" "), None);
        let work = layout.create_pin_group(" Work ").unwrap();
        let refs = layout.create_pin_group("Refs").unwrap();
        for pin in [a, b, c] {
            assert!(layout.move_pin_to_group(&inv, pin, Some(work)));
        }
        assert!(layout.reorder_pin(&inv, c, a, true));
        assert!(layout.unpin_location(&inv, a));
        assert!(layout.rename_pin_group(work, " Projects "));
        assert!(layout.shift_pin_group(refs, true));
        layout.toggle_pin_group(Some(work));
        layout.toggle_pin_group(None);
        let json = serde_json::to_vec(&layout).unwrap();
        let mut layout: Preferences = serde_json::from_slice(&json).unwrap();
        let groups = layout.pin_groups();
        assert_eq!(
            (groups[0].id, groups[1].id, groups[1].name.as_str(), groups[1].collapsed),
            (refs, work, "Projects", true)
        );
        assert!(layout.ungrouped_collapsed());
        let missing = inv.folders.remove(&id(7)).unwrap();
        assert_eq!(layout.pin_sections(&inv)[1].1, [b]);
        inv.folders.insert(id(7), missing);
        assert!(layout.pin_location(&inv, a));
        assert_eq!(layout.pin_sections(&inv)[1].1, [c, a, b]);
        assert!(layout.move_pin_to_group(&inv, b, Some(refs)));
        assert!(layout.reorder_pin(&inv, a, b, false));
        assert_eq!(layout.pin_sections(&inv)[0].1, [b, a]);
        assert!(layout.shift_location(&inv, a, true));
        assert_eq!(layout.pin_sections(&inv)[0].1, [a, b]);
        assert!(layout.move_pin_to_group(&inv, b, None));
        assert!(layout.unpin_location(&inv, a));
        assert!(layout.remove_pin_group(refs));
        assert!(layout.pin_location(&inv, a));
        assert!(layout.remove_pin_group(work));
        assert!(!layout.ungrouped_collapsed());
        let pins = layout.pins(&inv);
        for pin in [a, b, c] {
            assert_eq!(pins.iter().filter(|p| **p == pin).count(), 1);
        }
        assert_eq!(layout.pin_sections(&inv), [(None, pins)]);
        assert!(layout.placements.is_empty() && layout.folders.is_empty());
    }

    #[test]
    fn pins_survive_restart_and_missing_data_including_default_pins() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        assert!(layout.pins(&inv).contains(&Location::Folder(id(11))));
        assert!(layout.unpin_location(&inv, Location::Folder(id(11))));
        assert!(layout.unpin_location(&inv, Location::Library));
        assert!(layout.pin_location(&inv, Location::Folder(id(7))));
        assert!(layout.reorder_pin(&inv, Location::Folder(id(7)), Location::Group(Group::Appearance), true));
        let json = serde_json::to_string(&layout).unwrap();
        let layout: Preferences = serde_json::from_str(&json).unwrap();
        assert_eq!(layout.pins(&inv)[0], Location::Folder(id(7)));
        assert!(!layout.pins(&inv).contains(&Location::Library));
        assert!(!layout.pins(&inv).contains(&Location::Folder(id(11))));
        let removed = inv.folders.remove(&id(7)).unwrap();
        assert!(!layout.pins(&inv).contains(&Location::Folder(id(7))));
        inv.folders.insert(id(7), removed);
        assert_eq!(layout.pins(&inv)[0], Location::Folder(id(7)));
    }

    #[test]
    fn local_bulk_move_is_atomic_and_restorable_even_for_no_copy_items() {
        let inv = fixture();
        let mut layout = Preferences::default();
        let target = layout.create_custom(&inv, Location::MyFolders, "Builds").unwrap();
        assert!(!layout.move_entries(&inv, &[id(13), id(6)], &[], target));
        assert!(layout.children(&inv, target).is_empty());
        assert!(layout.move_entries(&inv, &[id(8), id(13)], &[], target));
        assert_eq!(inv.items[&id(13)].parent, id(5));
        assert_eq!(layout.children(&inv, target), [id(8), id(13)]);
        assert!(!layout.children(&inv, Location::Folder(id(5))).contains(&id(13)));
        assert!(layout.restore(id(13)));
        assert!(layout.children(&inv, Location::Folder(id(5))).contains(&id(13)));
        assert!(layout.children(&inv, Location::FullTree).contains(&id(8)));
    }

    #[test]
    fn external_reparent_trash_and_virtual_cycles_remain_recoverable() {
        let mut inv = fixture();
        let mut layout = Preferences::default();
        assert!(!layout.place(&inv, id(5), Location::Folder(id(7))));
        let a = layout.create_custom(&inv, Location::MyFolders, "A").unwrap();
        let b = layout.create_custom(&inv, a, "B").unwrap();
        let Location::Custom(aid) = a else { unreachable!() };
        let Location::Custom(bid) = b else { unreachable!() };
        assert!(!layout.move_entries(&inv, &[], &[aid], b));
        assert!(layout.place(&inv, id(5), b));
        assert!(!layout.move_entries(&inv, &[], &[bid], Location::Folder(id(7))));
        assert!(layout.remove_custom(bid));
        assert_eq!(layout.parent(&inv, id(5)), a);
        assert!(layout.place(&inv, id(5), Location::Folder(id(8))));
        // An external viewer moves 8 into 5. The cached 5 -> 8 overlay now
        // forms a cycle, so 5 becomes reachable at its actual parent again.
        inv.folders.get_mut(&id(8)).unwrap().info.parent = id(5);
        inv.folders.get_mut(&id(5)).unwrap().children.push(id(8));
        assert_eq!(layout.parent(&inv, id(5)), Location::MyFolders);
        assert!(layout.children(&inv, Location::MyFolders).contains(&id(5)));
        inv.folders.get_mut(&id(8)).unwrap().info.parent = id(11);
        assert_eq!(layout.parent(&inv, id(5)), Location::MyFolders);
    }

    #[test]
    fn exact_client_name_precedes_conflicting_type_and_old_photo_placement() {
        let mut inv = fixture();
        folder(&mut inv, 20, 1, 15, "#RLV", false);
        folder(&mut inv, 21, 1, 15, "Fotografías", false);
        folder(&mut inv, 22, 21, -1, "#RLV", false);
        folder(&mut inv, 23, 1, 56, "#Firestorm", false);
        let old = serde_json::json!({"placements": {id(20).to_string(): {"Folder": id(21)}}});
        let layout: Preferences = serde_json::from_value(old).unwrap();
        let photos = Location::Folder(id(21));
        let system = Location::Group(Group::System);
        // The generic inventory lookup picks the smaller, conflicting id.
        assert_eq!(inv.system_folder(15), Some(id(20)));
        assert_eq!(preferred_root(&inv, 15), Some(id(21)));
        assert!(layout.pins(&inv).contains(&photos));
        assert!(!layout.pins(&inv).contains(&Location::Folder(id(20))));
        assert!(layout.catalog(&inv).contains(&photos));
        assert!(!layout.catalog(&inv).contains(&Location::Folder(id(23))));
        assert_eq!(label(&inv, id(20)), "#RLV");
        assert_eq!(layout.parent(&inv, id(20)), system);
        assert!(layout.children(&inv, system).contains(&id(20)));
        assert!(!layout.children(&inv, photos).contains(&id(20)));
        assert_eq!(layout.parent(&inv, id(22)), photos);
        assert!(can_organize(&inv, id(22)));
        assert!(!can_organize(&inv, id(20)));
        assert_eq!(inv.folders[&id(20)].info.parent, inv.root);
        // Known direct folders remain discoverable through System even if a
        // partially fetched folder list lacks their id.
        inv.folders.get_mut(&id(1)).unwrap().children.retain(|n| *n != id(20));
        assert!(layout.children(&inv, system).contains(&id(20)));
        assert_eq!(layout.parent(&inv, id(22)), photos);
    }

    #[test]
    fn only_unique_misplaced_rlv_in_the_real_snapshots_folder_is_projected() {
        let mut inv = fixture();
        folder(&mut inv, 20, 1, 15, "Instantanés", false);
        folder(&mut inv, 21, 20, -1, "#RLV", false);
        folder(&mut inv, 22, 21, -1, "Wardrobe", false);
        folder(&mut inv, 23, 5, -1, "#RLV", false);
        folder(&mut inv, 24, 1, -1, "Photos", false);
        folder(&mut inv, 25, 24, -1, "#RLV", false);
        folder(&mut inv, 26, 20, -1, "#rlv", false);
        folder(&mut inv, 27, 2, 15, "Snapshots", true);
        folder(&mut inv, 28, 27, -1, "#RLV", true);
        let layout = Preferences::default();
        let photos = Location::Folder(id(20));
        let system = Location::Group(Group::System);
        assert_eq!(layout.parent(&inv, id(21)), system);
        assert!(layout.children(&inv, system).contains(&id(21)));
        assert!(!layout.children(&inv, photos).contains(&id(21)));
        assert_eq!(inv.folders[&id(20)].children[0], id(21));
        assert_eq!(inv.folders[&id(21)].info.parent, id(20));
        assert!(!can_organize(&inv, id(22)));
        for (folder, parent) in [(23, 5), (25, 24), (26, 20), (28, 27)] {
            assert_eq!(layout.parent(&inv, id(folder)), Location::Folder(id(parent)));
        }
        folder(&mut inv, 29, 20, -1, "#RLV", false);
        for candidate in [21, 29] {
            assert_eq!(layout.parent(&inv, id(candidate)), photos);
            assert!(!layout.children(&inv, system).contains(&id(candidate)));
        }
        inv.folders.remove(&id(29));
        folder(&mut inv, 30, 1, -1, "#RLV", false);
        assert_eq!(layout.parent(&inv, id(21)), photos);
        assert_eq!(layout.parent(&inv, id(30)), system);
    }

    #[test]
    fn mixed_selection_is_atomic_and_preserves_real_and_virtual_descendants() {
        let mut inv = fixture();
        folder(&mut inv, 20, 1, 15, "Snapshots", false);
        folder(&mut inv, 21, 1, 5, "Clothing", false);
        folder(&mut inv, 22, 1, 1, "Sounds", false);
        folder(&mut inv, 23, 1, 46, "Current Outfit", false);
        folder(&mut inv, 24, 1, 50, "Received Items", false);
        folder(&mut inv, 25, 23, -1, "Nested", false);
        let mut layout = Preferences::default();
        let Location::Custom(a) = layout.create_custom(&inv, Location::Folder(id(5)), "Local").unwrap() else {
            panic!()
        };
        let Location::Custom(b) = layout.create_custom(&inv, Location::Custom(a), "Nested local").unwrap() else {
            panic!()
        };
        let Location::Custom(c) = layout.create_custom(&inv, Location::MyFolders, "Independent").unwrap() else {
            panic!()
        };
        assert!(layout.place(&inv, id(8), Location::Custom(a)));
        let ids = [id(5), id(7), id(13), id(8)];
        let custom = [a, b, c];
        for bad in [
            Location::Group(Group::System),
            Location::Library,
            Location::Folder(id(6)),
            Location::Folder(id(10)),
            Location::Folder(id(11)),
            Location::Folder(id(23)),
            Location::Folder(id(24)),
            Location::Folder(id(25)),
            Location::Custom(b),
            Location::Folder(id(7)),
        ] {
            let revision = layout.revision();
            assert!(!layout.can_move_entries(&inv, &ids, &custom, bad));
            assert!(!layout.move_entries(&inv, &ids, &custom, bad));
            assert_eq!(layout.revision(), revision);
        }
        let photos = Location::Folder(id(20));
        let group = Location::Group(Group::MotionSound);
        let revision = layout.revision();
        assert!(layout.can_move_entries(&inv, &ids, &custom, photos));
        assert!(layout.move_entries(&inv, &ids, &custom, photos));
        assert_eq!(layout.revision(), revision + 1);
        assert_eq!(layout.parent(&inv, id(5)), photos);
        assert_eq!(layout.location_parent(&inv, Location::Custom(c)), Some(photos));
        assert_eq!(layout.location_parent(&inv, Location::Custom(a)), Some(Location::Folder(id(5))));
        assert_eq!(layout.location_parent(&inv, Location::Custom(b)), Some(Location::Custom(a)));
        assert_eq!(layout.parent(&inv, id(7)), Location::Folder(id(5)));
        assert_eq!(layout.parent(&inv, id(13)), Location::Folder(id(5)));
        assert_eq!(layout.parent(&inv, id(8)), Location::Custom(a));
        assert!(!layout.move_entries(&inv, &ids, &custom, photos));
        assert!(!layout.move_entries(&inv, &[id(5), id(6)], &[c], group));
        assert_eq!(layout.parent(&inv, id(5)), photos);
        assert!(layout.move_entries(&inv, &[id(5)], &[c], group));
        assert!(layout.children(&inv, group).contains(&id(5)));
        assert!(layout.virtual_children(&inv, group).contains(&Location::Custom(c)));
        for kind in [20, 21, 22] {
            assert!(layout.destination(&inv, Location::Folder(id(kind))));
        }
        assert_eq!(inv.folders[&id(5)].info.parent, inv.root);
        assert_eq!(inv.items[&id(13)].parent, id(5));
    }

    #[test]
    fn explorer_collapsed_keys_follow_locations_and_renamed_home_pin_groups() {
        let inv = fixture();
        let mut layout = Preferences::default();
        let place = layout.create_custom(&inv, Location::MyFolders, "Project").unwrap();
        let group = layout.create_pin_group("Home folders").unwrap();
        let pinned = GroupKey::HomePinGroup(group);
        let name = GroupKey::Name(NameBucket::ItoP);
        let date = GroupKey::Date(DateBucket::Yesterday);
        let revision = layout.revision;
        assert!(layout.set_groups_collapsed(place, [&name, &date], true));
        assert_eq!(layout.revision, revision + 1);
        assert!(!layout.set_group_collapsed(place, &date, true));
        assert_eq!(layout.revision, revision + 1);
        assert!(!layout.group_collapsed(Location::FullTree, &date));
        assert!(layout.set_group_collapsed(Location::Home, &pinned, true));
        assert!(!layout.group_collapsed(Location::Home, &GroupKey::HomeFolders));
        assert!(layout.rename_pin_group(group, "Work"));
        let mut restored: Preferences = serde_json::from_slice(&serde_json::to_vec(&layout).unwrap()).unwrap();
        assert!(restored.group_collapsed(Location::Home, &pinned));
        assert!(restored.group_collapsed(place, &name));
        assert!(restored.group_collapsed(place, &date));
        // Expanding the sections in one filtered result leaves unrelated keys alone.
        assert!(restored.set_groups_collapsed(place, [&name], false));
        assert!(restored.group_collapsed(place, &date));
        assert!(restored.remove_pin_group(group));
        assert!(!restored.group_collapsed(Location::Home, &pinned));
        let Location::Custom(v) = place else { unreachable!() };
        assert!(restored.remove_custom(v));
        assert!(!restored.group_collapsed(place, &date));
        assert!(serde_json::to_value(restored).unwrap().get("collapsed_groups").is_none());
    }

    #[test]
    fn old_preferences_load_and_views_round_trip_per_location() {
        let inv = fixture();
        let old = serde_json::json!({"shortcuts":[{"Place":{"Group":"System"}}],"placements":{},"folders":{}});
        let mut layout: Preferences = serde_json::from_value(old).unwrap();
        assert_eq!(layout.pins(&inv), [Location::Group(Group::System)]);
        assert_eq!(layout.view(Location::MyFolders), DirectoryView::default());
        let local = layout.create_custom(&inv, Location::MyFolders, "Offline").unwrap();
        let places = [
            Location::MyFolders,
            Location::Folder(id(5)),
            Location::Group(Group::Appearance),
            Location::Library,
            local,
            Location::FullTree,
        ];
        let layouts = [
            ViewLayout::Icons(IconSize::Small),
            ViewLayout::Icons(IconSize::Medium),
            ViewLayout::Icons(IconSize::Large),
            ViewLayout::Icons(IconSize::ExtraLarge),
            ViewLayout::List,
            ViewLayout::Details,
        ];
        for (i, &location) in places.iter().enumerate() {
            let view = DirectoryView {
                layout: layouts[i],
                sort: if i % 2 == 0 { SortKey::Date } else { SortKey::Type },
                descending: i % 2 == 1,
                group: GroupBy::Type,
                ..Default::default()
            };
            assert!(layout.set_view(location, view));
            assert!(!layout.set_view(location, view));
        }
        let mut restored: Preferences = serde_json::from_slice(&serde_json::to_vec(&layout).unwrap()).unwrap();
        for location in places {
            assert_eq!(restored.view(location), layout.view(location));
        }
        assert_eq!(restored.revision(), 0);
        assert!(restored.set_view(Location::Library, DirectoryView::default()));
        assert_eq!(restored.view(Location::Library), DirectoryView::default());
        let Location::Custom(v) = local else { unreachable!() };
        assert!(restored.remove_custom(v));
        assert_eq!(restored.view(local), DirectoryView::default());
    }
}
