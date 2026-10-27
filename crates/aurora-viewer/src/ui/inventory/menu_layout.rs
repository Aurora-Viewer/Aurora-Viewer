//! Inventory menu arrangements, independent of labels, accounts and live permissions.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub(crate) const ICON_SLOTS: usize = 8;
pub(crate) const TEXT: &str = "text";
pub(crate) const HIDDEN: &str = "hidden";
pub(crate) const LOCK: &str = "lock";
pub(crate) const SHARED: &str = "shared";
pub(crate) fn children(id: &str) -> String {
    format!("children:{id}")
}
pub(crate) fn icon_zone(id: &str) -> String {
    match id {
        "bar:top" => "top".into(),
        "bar:bottom" => "bottom".into(),
        _ => format!("icons:{id}"),
    }
}
pub(crate) fn bar(zone: &str) -> bool {
    matches!(zone, "top" | "bottom") || zone.starts_with("icons:bar:custom:")
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Direction {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    Action(usize),
    Lock,
    Group { direction: Direction, open: bool },
    IconBar,
    Separator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub kind: Kind,
    pub zone: String,
    pub slot: Option<usize>,
}
impl Entry {
    pub fn group(&self) -> bool {
        matches!(self.kind, Kind::Group { .. })
    }
    pub fn is_separator(&self) -> bool {
        self.kind == Kind::Separator
    }
    pub fn is_icon_bar(&self) -> bool {
        self.kind == Kind::IconBar
    }
    fn owned_zone(&self) -> Option<String> {
        if self.group() {
            Some(children(&self.id))
        } else if self.is_icon_bar() {
            Some(icon_zone(&self.id))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Group {
    pub name: String,
    pub direction: Direction,
    /// A newly created, unused group remains until explicitly removed.
    pub keep_empty: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Layout {
    pub placements: BTreeMap<String, String>,
    pub orders: BTreeMap<String, Vec<String>>,
    pub slots: BTreeMap<String, BTreeMap<String, usize>>,
    pub groups: BTreeMap<String, Group>,
    pub icon_bars: BTreeSet<String>,
    pub directions: BTreeMap<String, Direction>,
    pub separators: BTreeMap<String, String>,
    pub removed: BTreeSet<String>,
    pub filters: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Profile {
    pub layout: Layout,
    pub open: BTreeMap<String, bool>,
    pub locked: bool,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            layout: Layout {
                placements: [
                    ("asset-uuid", TEXT),
                    ("image", "children:organize"),
                    ("move", "children:organize"),
                    ("folder-uuid", HIDDEN),
                ]
                .map(|(id, zone)| (id.into(), zone.into()))
                .into(),
                orders: [
                    (
                        children("organize"),
                        [
                            "image",
                            "copy-name",
                            "move",
                            "default-folder",
                            "group-selection",
                            "ungroup",
                            "separator:links",
                            "paste-link",
                            "find-links",
                            "replace-links",
                            "separator:marketplace",
                            "marketplace-copy",
                            "marketplace-move",
                            "separator:uploads",
                            "upload-folder",
                            "asset-uuid",
                        ]
                        .map(str::to_owned)
                        .into(),
                    ),
                    (HIDDEN.into(), vec!["move".into()]),
                    (
                        TEXT.into(),
                        [
                            "bar:top",
                            "restore",
                            "recover-folder",
                            "original",
                            "open",
                            "save-textures",
                            "card-im",
                            "card-offer-teleport",
                            "card-request-teleport",
                            "card-conference",
                            "folder-im-all",
                            "about-landmark",
                            "copy-landmark-slurl",
                            "show-landmark-on-map",
                            "rez-previous",
                            "play-world",
                            "play-local",
                            "play-sound",
                            "apply-environment",
                            "apply-parcel",
                            "gesture-active",
                            "gesture-deactivate",
                            "gesture-play",
                            "edit-worn",
                            "refresh-attachment-textures",
                            "touch-worn",
                            "attachments",
                            "copy-outfit-list",
                            "separator:metadata",
                            "properties",
                            "bulk-permissions",
                            "rename",
                            "share",
                            "asset-uuid",
                            "protect-folder",
                            "organize",
                            "listing",
                            "separator:image",
                            "separator:folder-navigation",
                            "reload",
                            "clean-broken-links",
                            "show-main",
                            "show-window",
                            "open-window",
                            "open-current",
                            "separator:purge",
                            "purge",
                            "empty-trash",
                            "empty-lost-found",
                            "bar:bottom",
                        ]
                        .map(str::to_owned)
                        .into(),
                    ),
                ]
                .into(),
                ..Layout::default()
            },
            open: [("attachments".into(), true), ("organize".into(), true)].into(),
            locked: true,
        }
    }
}

impl Profile {
    pub(crate) fn reset(&mut self) -> Layout {
        let defaults = Self::default();
        self.open = defaults.open;
        std::mem::replace(&mut self.layout, defaults.layout)
    }

    fn customized(&self) -> bool {
        let mut baseline = Self::default();
        for id in [
            "folder-uuid",
            "save-textures",
            "card-im",
            "card-offer-teleport",
            "card-request-teleport",
            "card-conference",
            "folder-im-all",
            "apply-parcel",
            "gesture-deactivate",
            "refresh-attachment-textures",
            "copy-outfit-list",
            "listing",
            "clean-broken-links",
            "show-main",
            "open-current",
            "empty-lost-found",
            "recover-folder",
            "copy-name",
            "new-outfit",
        ] {
            let known = self.layout.placements.contains_key(id)
                || self.layout.removed.contains(id)
                || self.layout.orders.values().any(|order| order.iter().any(|key| key == id));
            if !known {
                baseline.layout.placements.remove(id);
                for order in baseline.layout.orders.values_mut() {
                    order.retain(|key| key != id);
                }
            }
        }
        self != &baseline && !(self.layout == Layout::default() && self.open.is_empty() && self.locked)
    }
}

/// Stored globally: one arrangement, resolved against each item's live commands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub(crate) profiles: BTreeMap<String, Profile>,
}

impl Preferences {
    pub(crate) fn unify(&mut self) {
        // Choose the pre-migration user arrangement before adding defaults;
        // a previously blank profile must not become "customized" merely
        // because the folder UUID is now hidden there.
        if self.profiles.len() != 1 || !self.profiles.contains_key(SHARED) {
            let profile = self
                .profiles
                .remove(SHARED)
                .or_else(|| {
                    let key = ["type:6", "folders"]
                        .into_iter()
                        .find(|key| self.profiles.get(*key).is_some_and(Profile::customized))
                        .map(str::to_owned)
                        .or_else(|| self.profiles.iter().find(|(_, p)| p.customized()).map(|(key, _)| key.clone()));
                    key.and_then(|key| self.profiles.remove(&key))
                })
                .unwrap_or_default();
            self.profiles.clear();
            self.profiles.insert(SHARED.into(), profile);
        }
        // Older saved arrangements have not seen these destination commands.
        // Insert them beside About without moving or restoring edited entries.
        for profile in self.profiles.values_mut() {
            let known = |id: &str| {
                profile.layout.placements.contains_key(id)
                    || profile.layout.removed.contains(id)
                    || profile.layout.orders.values().any(|order| order.iter().any(|key| key == id))
            };
            if !known("folder-uuid") {
                profile.layout.placements.insert("folder-uuid".into(), HIDDEN.into());
            }
            for (id, after, zone) in [
                ("save-textures", "open", TEXT),
                ("card-im", "save-textures", TEXT),
                ("card-offer-teleport", "card-im", TEXT),
                ("card-request-teleport", "card-offer-teleport", TEXT),
                ("card-conference", "card-request-teleport", TEXT),
                ("folder-im-all", "card-conference", TEXT),
                ("apply-parcel", "apply-environment", TEXT),
                ("gesture-deactivate", "gesture-active", TEXT),
                ("refresh-attachment-textures", "edit-worn", TEXT),
                ("copy-outfit-list", "attachments", TEXT),
                ("listing", "organize", TEXT),
                ("clean-broken-links", "reload", TEXT),
                ("show-main", "clean-broken-links", TEXT),
                ("open-current", "open-window", TEXT),
                ("empty-lost-found", "empty-trash", TEXT),
                ("recover-folder", "restore", TEXT),
                ("copy-name", "image", "children:organize"),
            ] {
                let known = profile.layout.placements.contains_key(id)
                    || profile.layout.removed.contains(id)
                    || profile.layout.orders.values().any(|order| order.iter().any(|key| key == id));
                if !known
                    && let Some(order) = profile.layout.orders.get_mut(zone)
                    && let Some(index) = order.iter().position(|key| key == after)
                {
                    order.insert(index + 1, id.into());
                }
            }
            let mut after = "about-landmark";
            for id in ["copy-landmark-slurl", "show-landmark-on-map"] {
                let known = profile.layout.placements.contains_key(id)
                    || profile.layout.removed.contains(id)
                    || profile.layout.orders.values().any(|order| order.iter().any(|key| key == id));
                if !known
                    && let Some(order) = profile.layout.orders.get_mut(TEXT)
                    && let Some(index) = order.iter().position(|key| key == after)
                {
                    order.insert(index + 1, id.into());
                }
                after = id;
            }
        }
    }
}

/// Presentation-only filtering; stored dividers retain their independent identities.
pub(crate) fn visible_rows<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Vec<&'a Entry> {
    let mut rows = Vec::new();
    let mut divider = None;
    for entry in entries {
        if entry.is_separator() {
            if !rows.is_empty() {
                divider = Some(entry);
            }
        } else {
            if let Some(divider) = divider.take() {
                rows.push(divider);
            }
            rows.push(entry);
        }
    }
    rows
}

pub(crate) type Resolved = BTreeMap<String, Vec<Entry>>;

/// Locked menus omit groups without live descendants, without changing saved layout.
/// Disabled actions and the layout lock still count; dividers and empty bars do not.
pub(crate) fn hide_empty_groups(resolved: &mut Resolved) {
    fn has_options(entry: &Entry, resolved: &Resolved, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        match entry.kind {
            Kind::Action(_) | Kind::Lock => true,
            Kind::Separator => false,
            Kind::Group { .. } | Kind::IconBar => entry
                .owned_zone()
                .and_then(|zone| resolved.get(&zone))
                .is_some_and(|entries| entries.iter().any(|e| has_options(e, resolved, depth + 1))),
        }
    }
    let empty: BTreeSet<_> = resolved
        .values()
        .flatten()
        .filter(|entry| entry.group() && !has_options(entry, resolved, 0))
        .map(|entry| entry.id.clone())
        .collect();
    for entries in resolved.values_mut() {
        entries.retain(|entry| !empty.contains(&entry.id));
    }
}

fn order_key(ranks: &BTreeMap<&str, usize>, id: &str) -> usize {
    // Pre-toolbar arrangements only ordered text commands.
    ranks
        .get(id)
        .map_or_else(|| if id == "bar:top" { 0 } else { usize::MAX }, |rank| rank.saturating_add(1))
}
pub(crate) fn hidden_zone(resolved: &Resolved, zone: &str) -> bool {
    let mut zone = zone;
    for _ in 0..25 {
        if zone == HIDDEN {
            return true;
        }
        let Some(parent) = resolved.iter().find_map(|(parent, entries)| {
            entries
                .iter()
                .any(|e| e.owned_zone().as_deref() == Some(zone))
                .then_some(parent.as_str())
        }) else {
            return false;
        };
        zone = parent;
    }
    false
}
/// A container cannot be hidden if that would also hide the layout lock.
pub(crate) fn can_hide(resolved: &Resolved, id: &str) -> bool {
    if id == LOCK {
        return false;
    }
    let Some((mut zone, _)) = resolved.iter().find(|(_, entries)| entries.iter().any(|e| e.id == LOCK)) else {
        return true;
    };
    for _ in 0..25 {
        let Some((parent, owner)) = resolved.iter().find_map(|(parent, entries)| {
            entries
                .iter()
                .find(|e| e.owned_zone().as_deref() == Some(zone))
                .map(|owner| (parent, owner))
        }) else {
            return true;
        };
        if owner.id == id {
            return false;
        }
        zone = parent;
    }
    false
}
impl Layout {
    pub fn resolve(&self, available: &[Entry]) -> Resolved {
        let mut entries: Vec<_> = available
            .iter()
            .filter(|e| !(self.removed.contains(&e.id) && (e.group() || e.is_separator())))
            .cloned()
            .collect();
        // Bound manually edited settings before building any recursive UI.
        for (id, g) in self.groups.iter().take(64) {
            if !id.starts_with("custom:") || g.name.trim().is_empty() {
                continue;
            }
            entries.push(Entry {
                id: id.clone(),
                label: g.name.chars().take(80).collect(),
                icon: "folder-filled".into(),
                enabled: true,
                checked: None,
                kind: Kind::Group {
                    direction: g.direction,
                    open: true,
                },
                zone: TEXT.into(),
                slot: None,
            });
        }
        for (id, zone) in self.separators.iter().take(128) {
            if id.starts_with("separator:custom:") && !self.removed.contains(id) {
                entries.push(Entry {
                    id: id.clone(),
                    label: String::new(),
                    icon: String::new(),
                    enabled: true,
                    checked: None,
                    kind: Kind::Separator,
                    zone: zone.clone(),
                    slot: None,
                });
            }
        }
        let mut bars = vec!["bar:top".to_owned()];
        bars.extend(
            self.icon_bars
                .iter()
                .filter(|id| id.starts_with("bar:custom:") && id.len() <= 80)
                .take(16)
                .cloned(),
        );
        bars.push("bar:bottom".into());
        for id in bars.into_iter().filter(|id| !self.removed.contains(id)) {
            let entry = Entry {
                id: id.clone(),
                label: String::new(),
                icon: "grip-vertical".into(),
                enabled: true,
                checked: None,
                kind: Kind::IconBar,
                zone: TEXT.into(),
                slot: None,
            };
            if id == "bar:top" {
                entries.insert(0, entry);
            } else {
                entries.push(entry);
            }
        }
        let owners: BTreeMap<_, _> = entries
            .iter()
            .filter_map(|e| e.owned_zone().map(|zone| (zone, e.id.clone())))
            .collect();
        let mut result: Resolved = [TEXT, HIDDEN].into_iter().map(|z| (z.into(), Vec::new())).collect();
        for zone in owners.keys() {
            result.insert(zone.clone(), Vec::new());
        }
        let mut located = BTreeMap::new();
        for e in &entries {
            let mut zone = self.placements.get(&e.id).unwrap_or(&e.zone).clone();
            // Arrangements saved before the movable lock had a fixed delete footer.
            if zone == "footer" && e.id == "delete" {
                zone = "bottom".into();
            }
            if !result.contains_key(&zone) || ((e.is_separator() || e.is_icon_bar()) && bar(&zone)) {
                zone = TEXT.into();
            }
            located.insert(e.id.clone(), zone);
        }
        for e in &mut entries {
            let mut zone = located[&e.id].clone();
            let mut cursor = zone.clone();
            let mut seen = BTreeSet::from([e.id.clone()]);
            while let Some(owner) = owners.get(&cursor) {
                if !seen.insert(owner.clone()) || seen.len() > 24 {
                    zone = TEXT.into();
                    break;
                }
                cursor = located.get(owner).cloned().unwrap_or_default();
            }
            if let Kind::Group { direction, .. } = &mut e.kind
                && let Some(saved) = self.directions.get(&e.id)
            {
                *direction = *saved;
            }
            result.entry(zone).or_default().push(e.clone());
        }
        for (zone, list) in &mut result {
            if let Some(order) = self.orders.get(zone) {
                let ranks: BTreeMap<_, _> = order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
                list.sort_by_key(|e| order_key(&ranks, &e.id));
            }
        }
        // Old/corrupt arrangements must never hide a ninth action.
        let bars: Vec<_> = result.keys().filter(|zone| bar(zone)).cloned().collect();
        for zone in bars {
            let list = result.get_mut(&zone).unwrap();
            if list.len() > ICON_SLOTS {
                let overflow = list.split_off(ICON_SLOTS);
                result.get_mut(TEXT).unwrap().extend(overflow);
            }
        }
        if let Some(zone) = result
            .iter()
            .find_map(|(zone, entries)| (entries.iter().any(|e| e.id == LOCK) && hidden_zone(&result, zone)).then_some(zone.clone()))
        {
            let entries = result.get_mut(&zone).unwrap();
            if let Some(at) = entries.iter().position(|e| e.id == LOCK) {
                let lock = entries.remove(at);
                result.get_mut(TEXT).unwrap().push(lock);
            }
        }
        result
    }

    pub fn icon_slots(&self, zone: &str, entries: &[Entry]) -> [Option<String>; ICON_SLOTS] {
        let mut cells = std::array::from_fn(|_| None);
        let saved = self.slots.get(zone);
        // Saved absolute cells take priority, including deliberate empty cells.
        for e in entries {
            if let Some(slot) = saved.and_then(|m| m.get(&e.id)).copied().filter(|s| *s < ICON_SLOTS)
                && cells[slot].is_none()
            {
                cells[slot] = Some(e.id.clone());
            }
        }
        for e in entries {
            if cells.iter().any(|c| c.as_ref() == Some(&e.id)) {
                continue;
            }
            let preferred = e
                .slot
                .unwrap_or_else(|| ICON_SLOTS.saturating_sub(entries.len()))
                .min(ICON_SLOTS - 1);
            if let Some(slot) = (preferred..ICON_SLOTS).chain(0..preferred).find(|s| cells[*s].is_none()) {
                cells[slot] = Some(e.id.clone());
            }
        }
        cells
    }

    fn order(&mut self, zone: &str, visible: Vec<String>) {
        let hidden: Vec<_> = self
            .orders
            .get(zone)
            .into_iter()
            .flatten()
            .filter(|id| !visible.contains(id))
            .cloned()
            .collect();
        self.orders.insert(zone.into(), visible.into_iter().chain(hidden).collect());
    }

    pub fn move_entry(&mut self, available: &[Entry], id: &str, zone: &str, before: Option<&str>, slot: Option<usize>) -> bool {
        let current = self.resolve(available);
        let Some((source, entry)) = current
            .iter()
            .find_map(|(z, list)| list.iter().find(|e| e.id == id).map(|e| (z.clone(), e)))
        else {
            return false;
        };
        if !current.contains_key(zone)
            || ((entry.is_separator() || entry.is_icon_bar()) && bar(zone))
            || (!can_hide(&current, id) && hidden_zone(&current, zone))
            || before == Some(id)
        {
            return false;
        }
        if bar(zone) && current[zone].iter().filter(|e| e.id != id).count() >= ICON_SLOTS {
            return false;
        }
        let owners: BTreeMap<_, _> = current
            .iter()
            .flat_map(|(z, entries)| {
                entries
                    .iter()
                    .filter_map(move |e| e.owned_zone().map(|zone| (zone, (e.id.clone(), z.clone()))))
            })
            .collect();
        let mut cursor = zone.to_owned();
        let mut seen = BTreeSet::new();
        while let Some((owner, parent)) = owners.get(&cursor) {
            if owner == id || !seen.insert(owner) {
                return false;
            }
            cursor = parent.clone();
        }
        let mut target: Vec<_> = current[zone].iter().filter(|e| e.id != id).map(|e| e.id.clone()).collect();
        let index = match before {
            Some(b) => match target.iter().position(|e| e == b) {
                Some(i) => i,
                None => return false,
            },
            None => target.len(),
        };
        let mut cells = self.icon_slots(zone, &current[zone]);
        if let Some(at) = slot {
            if !bar(zone) || at >= ICON_SLOTS {
                return false;
            }
            for c in &mut cells {
                if c.as_deref() == Some(id) {
                    *c = None;
                }
            }
            if let Some(free) = (at..ICON_SLOTS).find(|i| cells[*i].is_none()) {
                for i in (at..free).rev() {
                    cells[i + 1] = cells[i].take();
                }
            } else if let Some(free) = (0..at).rev().find(|i| cells[*i].is_none()) {
                for i in free..at {
                    cells[i] = cells[i + 1].take();
                }
            } else {
                return false;
            }
            cells[at] = Some(id.into());
        }
        self.placements.insert(id.into(), zone.into());
        for saved in self.slots.values_mut() {
            saved.remove(id);
        }
        self.order(
            &source,
            current[&source].iter().filter(|e| e.id != id).map(|e| e.id.clone()).collect(),
        );
        target.insert(index, id.into());
        self.order(zone, target);
        if slot.is_some() {
            let saved = self.slots.entry(zone.into()).or_default();
            for (slot, id) in cells.into_iter().enumerate().filter_map(|(i, id)| id.map(|id| (i, id))) {
                saved.insert(id, slot);
            }
        }
        self.prune(available, &current);
        true
    }

    pub fn create_group(&mut self, available: &[Entry], name: &str, direction: Direction, member: Option<&str>) -> bool {
        let name = name.trim().chars().take(80).collect::<String>();
        if name.is_empty() || self.groups.len() >= 64 {
            return false;
        }
        let current = self.resolve(available);
        let location = member.and_then(|id| {
            current.iter().find_map(|(z, list)| {
                list.iter()
                    .find(|e| e.id == id && matches!(e.kind, Kind::Action(_) | Kind::Lock))
                    .map(|_| z.clone())
            })
        });
        if member.is_some() && location.is_none() {
            return false;
        }
        let id = format!("custom:{}", uuid::Uuid::new_v4());
        let zone = location.filter(|z| !bar(z)).unwrap_or_else(|| TEXT.into());
        self.groups.insert(
            id.clone(),
            Group {
                name,
                direction,
                keep_empty: member.is_none(),
            },
        );
        self.placements.insert(id.clone(), zone.clone());
        let mut order: Vec<_> = current[&zone].iter().map(|e| e.id.clone()).collect();
        let at = member.and_then(|m| order.iter().position(|e| e == m)).unwrap_or(order.len());
        order.retain(|e| Some(e.as_str()) != member);
        order.insert(at.min(order.len()), id.clone());
        self.order(&zone, order);
        if let Some(member) = member {
            self.placements.insert(member.into(), children(&id));
            self.orders.insert(children(&id), vec![member.into()]);
            for slots in self.slots.values_mut() {
                slots.remove(member);
            }
        }
        self.prune(available, &current);
        true
    }

    pub fn remove_container(&mut self, available: &[Entry], id: &str) -> bool {
        let current = self.resolve(available);
        let Some(entry) = current.values().flatten().find(|e| e.id == id && (e.group() || e.is_icon_bar())) else {
            return false;
        };
        let zone = entry.owned_zone().unwrap();
        let Some(members) = current.get(&zone) else {
            return false;
        };
        let mut all: Vec<_> = if entry.is_icon_bar() {
            self.icon_slots(&zone, members).into_iter().flatten().collect()
        } else {
            members.iter().map(|e| e.id.clone()).collect()
        };
        for member in self
            .orders
            .get(&zone)
            .into_iter()
            .flatten()
            .chain(self.placements.iter().filter(|(_, z)| **z == zone).map(|(id, _)| id))
        {
            if !all.contains(member) && self.placements.get(member).is_none_or(|z| *z == zone) {
                all.push(member.clone());
            }
        }
        for member in &all {
            self.placements.insert(member.clone(), TEXT.into());
        }
        let mut root: Vec<_> = current[TEXT]
            .iter()
            .filter(|e| e.id != id && !all.contains(&e.id))
            .map(|e| e.id.clone())
            .collect();
        // Removing a container must preserve the other root positions even
        // when permissions or selection temporarily omit some commands.
        if let Some(order) = self.orders.get(TEXT) {
            for member in order.iter().filter(|member| member.as_str() != id && !all.contains(member)) {
                if !root.contains(member) {
                    root.push(member.clone());
                }
            }
            let ranks: BTreeMap<_, _> = order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
            root.sort_by_key(|id| order_key(&ranks, id));
        }
        root.extend(all);
        self.forget(id);
        self.orders.remove(&zone);
        self.slots.remove(&zone);
        if self.groups.remove(id).is_none() && !self.icon_bars.remove(id) {
            self.removed.insert(id.into());
        }
        self.order(TEXT, root);
        self.prune(available, &current);
        true
    }

    pub fn add_icon_bar(&mut self, available: &[Entry]) -> bool {
        if self.icon_bars.len() >= 16 {
            return false;
        }
        let current = self.resolve(available);
        let id = format!("bar:custom:{}", uuid::Uuid::new_v4());
        self.icon_bars.insert(id.clone());
        let mut order: Vec<_> = current[TEXT].iter().map(|e| e.id.clone()).collect();
        order.push(id);
        self.order(TEXT, order);
        true
    }

    pub fn add_separator(&mut self, available: &[Entry], zone: &str, before: Option<&str>) -> bool {
        let current = self.resolve(available);
        if bar(zone) || !current.contains_key(zone) || self.separators.len() >= 128 {
            return false;
        }
        let mut order: Vec<_> = current[zone].iter().map(|e| e.id.clone()).collect();
        let at = match before {
            Some(id) => match order.iter().position(|e| e == id) {
                Some(i) => i,
                None => return false,
            },
            None => order.len(),
        };
        let id = format!("separator:custom:{}", uuid::Uuid::new_v4());
        self.separators.insert(id.clone(), zone.into());
        self.placements.insert(id.clone(), zone.into());
        order.insert(at, id);
        self.order(zone, order);
        true
    }
    pub fn remove_separator(&mut self, available: &[Entry], id: &str) {
        let current = self.resolve(available);
        self.forget(id);
        if self.separators.remove(id).is_none() {
            self.removed.insert(id.into());
        }
        self.prune(available, &current);
    }
    fn forget(&mut self, id: &str) {
        self.placements.remove(id);
        self.filters.remove(id);
        self.directions.remove(id);
        for order in self.orders.values_mut() {
            order.retain(|e| e != id);
        }
        for slots in self.slots.values_mut() {
            slots.remove(id);
        }
    }
    fn prune(&mut self, available: &[Entry], previous: &Resolved) {
        // Only emptying a populated bar removes it. Newly created empty bars
        // and bars whose commands are temporarily unavailable must survive.
        let populated: Vec<_> = previous
            .iter()
            .filter(|(zone, entries)| bar(zone) && !entries.is_empty())
            .map(|(zone, _)| zone.clone())
            .collect();
        loop {
            let mut empty = Vec::new();
            for (id, group) in &mut self.groups {
                let zone = children(id);
                if self.placements.iter().any(|(id, z)| *z == zone && !id.starts_with("separator:")) {
                    group.keep_empty = false;
                } else if !group.keep_empty {
                    empty.push(id.clone());
                }
            }
            let mut changed = !empty.is_empty();
            for id in empty {
                let zone = children(&id);
                let separators: Vec<_> = self
                    .placements
                    .iter()
                    .filter(|(id, z)| **z == zone && id.starts_with("separator:"))
                    .map(|(id, _)| id.clone())
                    .collect();
                for sep in separators {
                    self.forget(&sep);
                    self.separators.remove(&sep);
                    self.removed.insert(sep);
                }
                self.groups.remove(&id);
                self.forget(&id);
                self.orders.remove(&zone);
            }
            let current = self.resolve(available);
            for zone in &populated {
                if current.get(zone).is_none_or(|entries| !entries.is_empty()) {
                    continue;
                }
                let stored_member = self
                    .placements
                    .iter()
                    .any(|(id, z)| z == zone || (id == "delete" && z == "footer" && zone == "bottom"))
                    || self
                        .orders
                        .get(zone)
                        .into_iter()
                        .flatten()
                        .chain(self.slots.get(zone).into_iter().flat_map(|slots| slots.keys()))
                        .any(|id| self.placements.get(id).is_none_or(|z| z == zone));
                if stored_member {
                    continue;
                }
                if let Some(entry) = current.values().flatten().find(|e| e.is_icon_bar() && icon_zone(&e.id) == *zone) {
                    if !self.icon_bars.remove(&entry.id) {
                        self.removed.insert(entry.id.clone());
                    }
                    self.forget(&entry.id);
                    self.orders.remove(zone);
                    self.slots.remove(zone);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn folder_uuid_migration_starts_hidden_and_preserves_a_later_user_choice() {
        let mut profile = Profile::default();
        profile.layout.placements.remove("folder-uuid");
        profile.layout.orders.get_mut(TEXT).expect("text order").rotate_left(3);
        let orders = profile.layout.orders.clone();
        let mut preferences = Preferences {
            profiles: [(SHARED.into(), profile)].into(),
        };
        preferences.unify();
        let profile = preferences.profiles.get_mut(SHARED).expect("shared profile");
        assert_eq!(profile.layout.placements.get("folder-uuid").map(String::as_str), Some(HIDDEN));
        assert_eq!(profile.layout.orders, orders);
        profile.layout.placements.insert("folder-uuid".into(), TEXT.into());
        preferences.unify();
        assert_eq!(
            preferences.profiles[SHARED]
                .layout
                .placements
                .get("folder-uuid")
                .map(String::as_str),
            Some(TEXT)
        );
        assert_eq!(preferences.profiles[SHARED].layout.orders, orders);
    }
    #[test]
    fn saved_menus_gain_landmark_destinations_without_moving_customized_entries() {
        let mut profile = Profile::default();
        let order = profile.layout.orders.get_mut(TEXT).expect("text order");
        order.retain(|id| !matches!(id.as_str(), "copy-landmark-slurl" | "show-landmark-on-map"));
        let previous = order.clone();
        let mut prefs = Preferences {
            profiles: [(SHARED.into(), profile)].into(),
        };
        prefs.unify();
        let profile = prefs.profiles.get_mut(SHARED).expect("shared profile");
        let order = profile.layout.orders.get_mut(TEXT).expect("text order");
        let about = order.iter().position(|id| id == "about-landmark").expect("About");
        assert_eq!(&order[about + 1..about + 3], ["copy-landmark-slurl", "show-landmark-on-map"]);
        assert_eq!(
            order
                .iter()
                .filter(|id| !matches!(id.as_str(), "copy-landmark-slurl" | "show-landmark-on-map"))
                .cloned()
                .collect::<Vec<_>>(),
            previous
        );
        order.retain(|id| id != "copy-landmark-slurl");
        profile.layout.placements.insert("copy-landmark-slurl".into(), HIDDEN.into());
        let customized = prefs.clone();
        prefs.unify();
        assert_eq!(prefs, customized, "a user's hidden or reordered action remains unchanged");
    }
    use super::*;
    #[test]
    fn locked_presentation_omits_recursively_empty_groups_but_keeps_disabled_actions_and_the_lock() {
        fn group(id: &str, zone: &str) -> Entry {
            let mut entry = action(id, zone);
            entry.kind = Kind::Group {
                direction: Direction::Horizontal,
                open: false,
            };
            entry
        }
        let mut divider = action("divider", &children("dividers-only"));
        divider.kind = Kind::Separator;
        let mut empty_bar = action("bar:custom:empty", &children("bar-only"));
        empty_bar.kind = Kind::IconBar;
        let mut disabled = action("disabled", &children("with-action"));
        disabled.enabled = false;
        let mut layout_lock = lock();
        layout_lock.zone = children("with-lock");
        let available = vec![
            group("empty", TEXT),
            group("parent", TEXT),
            group("empty-child", &children("parent")),
            group("dividers-only", TEXT),
            divider,
            group("bar-only", TEXT),
            empty_bar,
            group("with-action", TEXT),
            disabled,
            group("with-lock", TEXT),
            layout_lock,
            group("empty-icon", "top"),
            group("filled-icon", "top"),
            action("icon-action", &children("filled-icon")),
        ];
        let mut layout = Layout::default();
        layout
            .slots
            .insert("top".into(), [("empty-icon".into(), 2), ("filled-icon".into(), 5)].into());
        let original = layout.clone();
        let mut locked = layout.resolve(&available);
        hide_empty_groups(&mut locked);
        assert_eq!(ids(&locked, TEXT), ["with-action", "with-lock"]);
        assert!(!locked.values().flatten().any(|entry| entry.id == "empty-child"));
        assert!(!locked[&children("with-action")][0].enabled);
        assert!(locked[&children("with-lock")].iter().any(|entry| entry.kind == Kind::Lock));
        let slots = layout.icon_slots("top", &locked["top"]);
        assert!(slots[2].is_none());
        assert_eq!(slots[5].as_deref(), Some("filled-icon"));
        assert_eq!(layout, original);
        let editing = layout.resolve(&available);
        for id in ["empty", "parent", "empty-child", "dividers-only", "bar-only", "empty-icon"] {
            assert!(editing.values().flatten().any(|entry| entry.id == id));
        }
    }

    #[test]
    fn divider_display_collapses_runs_and_omits_edges_without_changing_storage() {
        let mut entries: Vec<_> = ["leading", "a", "one", "two", "b", "trailing"]
            .into_iter()
            .map(|id| action(id, TEXT))
            .collect();
        for at in [0, 2, 3, 5] {
            entries[at].kind = Kind::Separator;
        }
        for (a, b, expected) in [
            (true, true, vec!["a", "two", "b"]),
            (true, false, vec!["a"]),
            (false, true, vec!["b"]),
            (false, false, vec![]),
        ] {
            let rows = visible_rows(
                entries
                    .iter()
                    .filter(|e| e.is_separator() || (e.id == "a" && a) || (e.id == "b" && b)),
            );
            assert_eq!(rows.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), expected);
            assert_eq!(entries.iter().filter(|e| e.is_separator()).count(), 4);
        }
    }

    #[test]
    fn a_single_shared_profile_migrates_edited_legacy_layouts_without_combining_conflicts() {
        let mut prefs = Preferences::default();
        let mut object = Profile::default();
        object.layout.filters.insert("attach-body".into());
        object.layout.placements.insert("copy".into(), TEXT.into());
        prefs.profiles.insert("type:6".into(), object.clone());
        prefs.profiles.insert("folders".into(), Profile::default());
        prefs.unify();
        assert_eq!(prefs.profiles, BTreeMap::from([(SHARED.into(), object)]));
        let saved = serde_json::to_string(&prefs).unwrap();
        let mut reloaded: Preferences = serde_json::from_str(&saved).unwrap();
        reloaded.unify();
        assert_eq!(reloaded, prefs);
        reloaded.profiles.insert("type:0".into(), Profile::default());
        reloaded.unify();
        assert_eq!(reloaded, prefs);
    }

    #[test]
    fn new_defaults_preserve_existing_shared_and_skip_old_blank_type_profiles_during_migration() {
        let blank = Profile {
            layout: Layout::default(),
            open: BTreeMap::new(),
            locked: true,
        };
        let mut edited = blank.clone();
        edited.layout.placements.insert("copy".into(), TEXT.into());
        let mut prefs = Preferences {
            profiles: [("type:6".into(), blank.clone()), ("folders".into(), edited.clone())].into(),
        };
        prefs.unify();
        edited.layout.placements.insert("folder-uuid".into(), HIDDEN.into());
        assert_eq!(prefs.profiles[SHARED], edited);
        let mut prefs = Preferences {
            profiles: [(SHARED.into(), blank.clone()), ("folders".into(), edited)].into(),
        };
        prefs.unify();
        let mut migrated_blank = blank;
        migrated_blank.layout.placements.insert("folder-uuid".into(), HIDDEN.into());
        assert_eq!(
            prefs.profiles[SHARED], migrated_blank,
            "existing shared settings are never replaced by new defaults"
        );
    }

    #[test]
    fn an_untouched_older_type_profile_does_not_displace_a_customized_folder_profile() {
        let mut older = Profile::default();
        let introduced = [
            "folder-uuid",
            "save-textures",
            "card-im",
            "card-offer-teleport",
            "card-request-teleport",
            "card-conference",
            "folder-im-all",
            "apply-parcel",
            "gesture-deactivate",
            "refresh-attachment-textures",
            "copy-outfit-list",
            "listing",
            "clean-broken-links",
            "show-main",
            "open-current",
            "empty-lost-found",
            "recover-folder",
            "copy-name",
            "new-outfit",
        ];
        older.layout.placements.retain(|key, _| !introduced.contains(&key.as_str()));
        for order in older.layout.orders.values_mut() {
            order.retain(|key| !introduced.contains(&key.as_str()));
        }
        assert!(!older.customized());
        let mut folders = older.clone();
        folders.layout.placements.insert("copy".into(), TEXT.into());
        let mut prefs = Preferences {
            profiles: [("type:6".into(), older), ("folders".into(), folders)].into(),
        };
        prefs.unify();
        assert_eq!(prefs.profiles[SHARED].layout.placements["copy"], TEXT);
        assert_eq!(prefs.profiles[SHARED].layout.placements["folder-uuid"], HIDDEN);
    }
    fn action(id: &str, zone: &str) -> Entry {
        Entry {
            id: id.into(),
            label: format!("label {id}"),
            icon: "copy-filled".into(),
            enabled: true,
            checked: None,
            kind: Kind::Action(0),
            zone: zone.into(),
            slot: None,
        }
    }
    fn ids(resolved: &Resolved, zone: &str) -> Vec<String> {
        resolved[zone].iter().filter(|e| !e.is_icon_bar()).map(|e| e.id.clone()).collect()
    }

    fn lock() -> Entry {
        let mut entry = action(LOCK, "bottom");
        entry.kind = Kind::Lock;
        entry.slot = Some(0);
        entry
    }

    #[test]
    fn lock_and_legacy_delete_share_the_bottom_bar_and_remain_movable_after_reload() {
        let mut delete = action("delete", "bottom");
        delete.slot = Some(7);
        let entries = [lock(), delete, action("open", TEXT)];
        let mut layout: Layout = serde_json::from_str(r#"{"placements":{"delete":"footer"}}"#).unwrap();
        let resolved = layout.resolve(&entries);
        let cells = layout.icon_slots("bottom", &resolved["bottom"]);
        assert_eq!(cells[0].as_deref(), Some(LOCK));
        assert_eq!(cells[7].as_deref(), Some("delete"));
        let mut partial = layout.clone();
        assert!(partial.move_entry(&entries[..1], LOCK, TEXT, None, None));
        assert!(
            partial.resolve(&entries).contains_key("bottom"),
            "an unavailable legacy Delete still belongs to the bar"
        );
        assert!(layout.move_entry(&entries, LOCK, TEXT, Some("open"), None));
        assert_eq!(ids(&layout.resolve(&entries), TEXT), [LOCK, "open"]);
        assert!(layout.move_entry(&entries, LOCK, "top", None, Some(3)));
        let mut saved: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(saved.icon_slots("top", &saved.resolve(&entries)["top"])[3].as_deref(), Some(LOCK));
        assert!(saved.remove_container(&entries, "bar:top"));
        assert!(saved.resolve(&entries)[TEXT].iter().any(|e| e.id == LOCK && e.kind == Kind::Lock));
    }

    #[test]
    fn the_lock_and_all_its_containers_reject_hidden_destinations_without_changing_the_layout() {
        let entries = [lock(), action("open", TEXT)];
        let mut layout = Layout::default();
        assert!(layout.create_group(&entries, "Inner", Direction::Horizontal, Some(LOCK)));
        let inner = layout.groups.keys().next().unwrap().clone();
        layout.add_icon_bar(&entries);
        let icon_bar = layout.icon_bars.iter().next().unwrap().clone();
        assert!(layout.move_entry(&entries, &inner, &icon_zone(&icon_bar), None, Some(4)));
        layout.create_group(&entries, "Outer", Direction::Vertical, None);
        let outer = layout.groups.iter().find(|(_, g)| g.name == "Outer").unwrap().0.clone();
        assert!(layout.move_entry(&entries, &icon_bar, &children(&outer), None, None));
        layout.create_group(&entries, "Hidden", Direction::Vertical, None);
        let hidden = layout.groups.iter().find(|(_, g)| g.name == "Hidden").unwrap().0.clone();
        assert!(layout.move_entry(&entries, &hidden, HIDDEN, None, None));
        let before = layout.clone();
        for id in [LOCK, &inner, &icon_bar, &outer] {
            assert!(!can_hide(&layout.resolve(&entries), id));
            for destination in [HIDDEN.to_owned(), children(&hidden)] {
                assert!(!layout.move_entry(&entries, id, &destination, None, None));
                assert_eq!(layout, before);
            }
        }
        assert!(layout.remove_container(&entries, &icon_bar));
        assert!(layout.remove_container(&entries, &inner));
        assert!(layout.resolve(&entries)[TEXT].iter().any(|e| e.id == LOCK));
    }

    #[test]
    fn saved_hidden_or_removed_lock_settings_always_recover_a_visible_lock() {
        let entries = [lock(), action("open", TEXT)];
        for placement in [HIDDEN, "bottom"] {
            let mut layout = Layout::default();
            layout.placements.insert(LOCK.into(), placement.into());
            layout.placements.insert("bar:bottom".into(), HIDDEN.into());
            layout.removed.insert(LOCK.into());
            let resolved = layout.resolve(&entries);
            assert!(resolved[TEXT].iter().any(|e| e.id == LOCK && e.enabled && e.kind == Kind::Lock));
            assert!(!resolved[HIDDEN].iter().any(|e| e.id == LOCK));
            assert!(can_hide(&resolved, "bar:bottom"));
        }
    }

    #[test]
    fn legacy_text_order_and_absolute_cells_gain_movable_default_bars_without_rearranging_commands() {
        let entries = [action("a", TEXT), action("b", TEXT), action("c", "top")];
        let layout: Layout = serde_json::from_str(r#"{"orders":{"text":["b","a"]},"slots":{"top":{"c":7}}}"#).unwrap();
        let resolved = layout.resolve(&entries);
        assert_eq!(resolved[TEXT].first().unwrap().id, "bar:top");
        assert_eq!(resolved[TEXT].last().unwrap().id, "bar:bottom");
        assert_eq!(ids(&resolved, TEXT), ["b", "a"]);
        assert_eq!(layout.icon_slots("top", &resolved["top"])[7].as_deref(), Some("c"));
        let mut edited = layout.clone();
        assert!(edited.remove_container(&entries, "bar:bottom"));
        assert_eq!(edited.resolve(&entries)[TEXT].first().unwrap().id, "bar:top");
        assert_eq!(ids(&edited.resolve(&entries), TEXT), ["b", "a"]);
    }

    #[test]
    fn deleting_bars_lifts_visible_and_temporarily_missing_members_without_losing_permissions() {
        let mut disabled = action("disabled", TEXT);
        disabled.enabled = false;
        let entries = [action("a", "top"), action("b", TEXT), disabled];
        let mut layout = Layout::default();
        layout.add_icon_bar(&entries);
        let id = layout.icon_bars.iter().next().unwrap().clone();
        let zone = icon_zone(&id);
        assert!(layout.move_entry(&entries, "a", &zone, None, Some(7)));
        assert!(layout.move_entry(&entries, "disabled", &zone, None, Some(1)));
        assert!(layout.move_entry(&entries, "b", &zone, None, Some(3)));
        let saved: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(saved.icon_slots(&zone, &saved.resolve(&entries)[&zone])[7].as_deref(), Some("a"));
        let mut partial = saved.clone();
        assert!(partial.remove_container(&entries[..1], &id));
        let mut recovered = ids(&partial.resolve(&entries), TEXT);
        recovered.sort();
        assert_eq!(recovered, ["a", "b", "disabled"]);
        assert!(layout.remove_container(&entries, &id));
        assert_eq!(ids(&layout.resolve(&entries), TEXT), ["disabled", "b", "a"]);
        assert!(!layout.resolve(&entries)[TEXT].iter().find(|e| e.id == "disabled").unwrap().enabled);
        assert!(!layout.resolve(&entries).contains_key(&zone));
        assert!(
            layout.removed.contains("bar:top"),
            "moving its last icon already removed the top bar"
        );
        assert!(layout.remove_container(&entries[..1], "bar:bottom"));
        assert!(!layout.resolve(&entries).keys().any(|zone| bar(zone)));
        assert_eq!(ids(&layout.resolve(&entries), TEXT), ["disabled", "b", "a"]);
    }

    #[test]
    fn bars_can_move_and_hide_as_a_unit_but_cannot_nest_in_bars_or_create_cycles() {
        let entries = [action("a", TEXT), action("b", TEXT)];
        let mut layout = Layout::default();
        layout.add_icon_bar(&entries);
        let id = layout.icon_bars.iter().next().unwrap().clone();
        let zone = icon_zone(&id);
        assert!(layout.move_entry(&entries, "a", &zone, None, Some(4)));
        assert!(layout.move_entry(&entries, &id, HIDDEN, None, None));
        assert!(hidden_zone(&layout.resolve(&entries), &zone));
        assert!(!layout.move_entry(&entries, &id, &zone, None, Some(0)));
        assert!(!layout.move_entry(&entries, "bar:top", &zone, None, Some(0)));
        layout.create_group(&entries, "Tools", Direction::Vertical, Some("b"));
        let group = layout.groups.keys().next().unwrap().clone();
        assert!(layout.move_entry(&entries, &id, &children(&group), None, None));
        assert!(!layout.move_entry(&entries, &group, &zone, None, Some(0)));
        assert!(layout.move_entry(&entries, &id, TEXT, Some(&group), None));
        assert!(!hidden_zone(&layout.resolve(&entries), &zone));
        assert!(layout.remove_container(&entries, &id));
        assert_eq!(ids(&layout.resolve(&entries), TEXT).last().map(String::as_str), Some("a"));
    }

    #[test]
    fn emptying_a_used_bar_removes_it_after_reload_but_leaves_unused_bars_available() {
        for default in [Some("bar:top"), Some("bar:bottom"), None] {
            let entries = [action("a", TEXT), action("b", TEXT)];
            let mut layout = Layout::default();
            assert!(layout.add_icon_bar(&entries));
            let unused = layout.icon_bars.iter().next().unwrap().clone();
            let id = if let Some(id) = default {
                id.to_owned()
            } else {
                assert!(layout.add_icon_bar(&entries));
                layout.icon_bars.iter().find(|id| **id != unused).unwrap().clone()
            };
            let zone = icon_zone(&id);
            assert!(layout.move_entry(&entries, "a", &zone, None, Some(7)));
            assert!(layout.move_entry(&entries, "b", &zone, None, Some(1)));
            assert!(layout.move_entry(&entries, "a", &zone, None, Some(4)));
            assert_eq!(ids(&layout.resolve(&entries), &zone).len(), 2);
            assert!(layout.move_entry(&entries, "b", HIDDEN, None, None));
            let mut saved: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
            assert!(saved.move_entry(&entries, "a", TEXT, None, None));
            let resolved = saved.resolve(&entries);
            assert!(!resolved.contains_key(&zone));
            assert!(!saved.icon_bars.contains(&id));
            assert!(!saved.orders.contains_key(&zone));
            assert!(!saved.slots.contains_key(&zone));
            assert!(!saved.placements.contains_key(&id));
            assert_eq!(ids(&resolved, TEXT), ["a"]);
            assert_eq!(ids(&resolved, HIDDEN), ["b"]);
            assert!(saved.icon_bars.contains(&unused));
            assert!(resolved[&icon_zone(&unused)].is_empty());
        }
    }

    #[test]
    fn unavailable_icons_keep_both_default_and_added_bars_until_actually_moved_out() {
        for custom in [false, true] {
            let entries = [action("a", "top"), action("b", "top"), action("other", TEXT)];
            let mut layout = Layout::default();
            let id = if custom {
                layout.add_icon_bar(&entries);
                layout.icon_bars.iter().next().unwrap().clone()
            } else {
                "bar:top".into()
            };
            let zone = icon_zone(&id);
            assert!(layout.move_entry(&entries, "a", &zone, None, Some(7)));
            assert!(layout.move_entry(&entries, "b", &zone, None, Some(3)));
            let mut saved: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
            assert!(saved.move_entry(&entries[..1], "a", TEXT, None, None));
            assert!(saved.resolve(&entries[..1]).contains_key(&zone));
            assert!(saved.move_entry(&entries[2..], "other", HIDDEN, None, None));
            let restored = saved.resolve(&entries);
            assert_eq!(ids(&restored, &zone), ["b"]);
            assert_eq!(saved.icon_slots(&zone, &restored[&zone])[3].as_deref(), Some("b"));
            assert!(saved.move_entry(&entries, "b", TEXT, None, None));
            assert!(!saved.resolve(&entries).contains_key(&zone));
            assert_eq!(ids(&saved.resolve(&entries), TEXT), ["a", "b"]);
        }
    }

    #[test]
    fn wrapping_the_last_icon_or_removing_its_group_cleans_up_empty_bars_and_parents() {
        let entries = [action("a", TEXT), action("b", TEXT)];
        let mut layout = Layout::default();
        layout.add_icon_bar(&entries);
        let id = layout.icon_bars.iter().next().unwrap().clone();
        let zone = icon_zone(&id);
        layout.create_group(&entries, "Outer", Direction::Vertical, None);
        let outer = layout.groups.keys().next().unwrap().clone();
        assert!(layout.move_entry(&entries, &id, &children(&outer), None, None));
        assert!(layout.move_entry(&entries, "a", &zone, None, Some(7)));
        assert!(layout.create_group(&entries, "Inner", Direction::Horizontal, Some("a")));
        assert!(!layout.icon_bars.contains(&id));
        assert!(!layout.groups.contains_key(&outer));
        let inner = layout.groups.keys().next().unwrap().clone();
        assert_eq!(ids(&layout.resolve(&entries), &children(&inner)), ["a"]);
        layout.add_icon_bar(&entries);
        let id = layout.icon_bars.iter().next().unwrap().clone();
        assert!(layout.move_entry(&entries, &inner, &icon_zone(&id), None, Some(5)));
        assert!(layout.remove_container(&entries, &inner));
        assert!(!layout.icon_bars.contains(&id));
        assert!(layout.groups.is_empty());
        assert_eq!(ids(&layout.resolve(&entries), TEXT), ["b", "a"]);
    }

    #[test]
    fn absolute_icon_cells_keep_holes_and_shift_only_to_the_nearest_hole() {
        let entries = [action("a", "top"), action("b", "top"), action("c", TEXT)];
        let mut layout = Layout::default();
        assert!(layout.move_entry(&entries, "a", "top", None, Some(0)));
        assert!(layout.move_entry(&entries, "b", "top", None, Some(5)));
        let saved: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        let cells = saved.icon_slots("top", &saved.resolve(&entries)["top"]);
        assert_eq!(cells[0].as_deref(), Some("a"));
        assert_eq!(cells[5].as_deref(), Some("b"));
        assert!(cells[1..5].iter().all(Option::is_none));
        assert!(layout.move_entry(&entries, "c", "top", None, Some(5)));
        let cells = layout.icon_slots("top", &layout.resolve(&entries)["top"]);
        assert_eq!(cells[5].as_deref(), Some("c"));
        assert_eq!(cells[6].as_deref(), Some("b"));
        assert_eq!(cells[0].as_deref(), Some("a"));
        assert!(layout.move_entry(&entries, "a", "top", None, Some(7)));
        assert!(layout.move_entry(&entries, "c", "top", None, Some(7)));
        let cells = layout.icon_slots("top", &layout.resolve(&entries)["top"]);
        assert_eq!(cells[7].as_deref(), Some("c"));
        assert_eq!(cells[6].as_deref(), Some("a"));
        assert_eq!(cells[5].as_deref(), Some("b"));
    }

    #[test]
    fn hidden_groups_keep_children_and_permissions_across_settings_roundtrips() {
        let mut disabled = action("a", "top");
        disabled.enabled = false;
        let entries = [disabled, action("b", TEXT)];
        let mut layout = Layout::default();
        layout.create_group(&entries, "Less used", Direction::Vertical, Some("a"));
        let group = layout.groups.keys().next().unwrap().clone();
        assert!(layout.move_entry(&entries, &group, HIDDEN, None, None));
        let mut saved: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        let resolved = saved.resolve(&entries);
        assert_eq!(ids(&resolved, HIDDEN).as_slice(), std::slice::from_ref(&group));
        assert!(hidden_zone(&resolved, &children(&group)));
        assert!(!resolved[&children(&group)][0].enabled);
        assert!(saved.move_entry(&entries, &group, TEXT, None, None));
        assert!(!hidden_zone(&saved.resolve(&entries), &children(&group)));
        assert_eq!(ids(&saved.resolve(&entries), &children(&group)), ["a"]);
        assert!(Profile::default().locked);
    }
    #[test]
    fn a_full_bar_rejects_a_ninth_entry_without_losing_it() {
        let entries: Vec<_> = (0..9).map(|i| action(&i.to_string(), if i == 8 { TEXT } else { "top" })).collect();
        let mut layout = Layout::default();
        let before = layout.clone();
        assert!(!layout.move_entry(&entries, "8", "top", None, Some(3)));
        assert_eq!(layout, before);
        assert_eq!(layout.resolve(&entries)["top"].len(), ICON_SLOTS);
        assert_eq!(ids(&layout.resolve(&entries), TEXT), ["8"]);
    }
    #[test]
    fn moving_an_action_does_not_move_its_preceding_separator() {
        let mut separator = action("separator:links", TEXT);
        separator.kind = Kind::Separator;
        let entries = [action("copy", TEXT), separator, action("paste-link", TEXT), action("rename", TEXT)];
        let mut layout = Layout::default();
        assert!(layout.move_entry(&entries, "paste-link", TEXT, Some("copy"), None));
        assert_eq!(
            ids(&layout.resolve(&entries), TEXT),
            ["paste-link", "copy", "separator:links", "rename"]
        );
        assert!(layout.add_separator(&entries, TEXT, Some("copy")));
        assert_eq!(layout.resolve(&entries)[TEXT].iter().filter(|e| e.is_separator()).count(), 2);
        layout.remove_separator(&entries, "separator:links");
        assert!(!layout.resolve(&entries)[TEXT].iter().any(|e| e.id == "separator:links"));
        assert!(!layout.move_entry(
            &entries,
            layout.separators.keys().next().cloned().unwrap().as_str(),
            "top",
            None,
            Some(0)
        ));
    }
    #[test]
    fn new_empty_groups_persist_but_used_groups_disappear_after_the_last_member_leaves() {
        let entries = [action("a", TEXT), action("b", TEXT)];
        let mut layout = Layout::default();
        assert!(layout.create_group(&entries, "  Personal tools  ", Direction::Vertical, None));
        let group = layout.groups.keys().next().unwrap().clone();
        assert!(layout.move_entry(&entries, "b", TEXT, Some("a"), None));
        assert!(layout.groups.contains_key(&group));
        assert!(layout.move_entry(&entries, "a", &children(&group), None, None));
        assert!(!layout.groups[&group].keep_empty);
        assert!(layout.move_entry(&entries, "a", TEXT, None, None));
        assert!(!layout.groups.contains_key(&group));
    }
    #[test]
    fn a_temporarily_unavailable_command_keeps_its_group_and_reappears_in_its_saved_position() {
        let entries = [action("wear", TEXT), action("detach", TEXT)];
        let mut layout = Layout::default();
        assert!(layout.create_group(&entries, "Appearance", Direction::Horizontal, Some("detach")));
        let group = layout.groups.keys().next().unwrap().clone();
        assert!(layout.move_entry(&entries[..1], "wear", TEXT, None, None));
        assert!(layout.groups.contains_key(&group));
        assert_eq!(ids(&layout.resolve(&entries), &children(&group)), ["detach"]);
        assert!(layout.remove_container(&entries[..1], &group));
        assert_eq!(ids(&layout.resolve(&entries), TEXT), ["wear", "detach"]);
    }
    #[test]
    fn group_removal_lifts_children_and_forbids_cycles_and_group_wrapping() {
        let entries = [action("a", TEXT), action("b", TEXT), action("c", TEXT)];
        let mut layout = Layout::default();
        layout.create_group(&entries, "One", Direction::Vertical, Some("a"));
        let one = layout.groups.keys().next().unwrap().clone();
        layout.create_group(&entries, "Two", Direction::Horizontal, Some("b"));
        let two = layout.groups.keys().find(|id| **id != one).unwrap().clone();
        assert!(!layout.create_group(&entries, "Forbidden", Direction::Vertical, Some(&one)));
        assert!(layout.move_entry(&entries, &two, &children(&one), None, None));
        assert!(!layout.move_entry(&entries, &one, &children(&two), None, None));
        assert!(layout.remove_container(&entries, &one));
        assert_eq!(ids(&layout.resolve(&entries), TEXT), ["c", "a", two.as_str()]);
        assert_eq!(ids(&layout.resolve(&entries), &children(&two)), ["b"]);
    }
    #[test]
    fn corrupt_cycles_and_overfull_saved_bars_preserve_every_available_command() {
        let mut entries: Vec<_> = (0..12).map(|i| action(&i.to_string(), "top")).collect();
        let mut layout = Layout::default();
        layout.create_group(&entries, "One", Direction::Vertical, None);
        let one = layout.groups.keys().next().unwrap().clone();
        layout.create_group(&entries, "Two", Direction::Vertical, None);
        let two = layout.groups.keys().find(|id| **id != one).unwrap().clone();
        layout.placements.insert(one.clone(), children(&two));
        layout.placements.insert(two.clone(), children(&one));
        let mut inactive = action("inactive", TEXT);
        inactive.enabled = false;
        entries.push(inactive.clone());
        let resolved = layout.resolve(&entries);
        assert_eq!(resolved["top"].len(), 8);
        assert_eq!(resolved.values().flatten().filter(|e| !e.is_icon_bar()).count(), 15);
        assert!(resolved[TEXT].iter().any(|e| e.id == one));
        assert_eq!(resolved[TEXT].iter().find(|e| e.id == "inactive").unwrap(), &inactive);
    }
}
