//! AOEngine::{tick, update, reloadStateAnimations, parseNotecard, saveSet,
//! saveState, addAnimation}: Aurora-owned AO folders using the legacy format.

use super::*;

pub mod editor;
use crate::world::inventory::{FetchState, Inventory, actions};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use std::collections::HashSet;

pub const FOLDER: &str = "Aurora AO";
pub(super) const OLD_FOLDER: &str = "AO";
const STAGING: &str = "Aurora AO (Legacy Viewer import)";
// Existing interrupted imports must still prevent an automatic second copy.
const OLD_STAGING: &str = "AO (Firestorm import)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    pub source: Uuid,
    pub root: Option<Uuid>,
    pub sets: usize,
    pub duplicate: bool,
}

/// Incomplete migration folders are surfaced only after the brand folder was
/// fetched, so the AO window can offer an explicit, reviewable recovery.
pub fn recoveries(inv: &mut Inventory) -> Vec<Recovery> {
    let Some(brand) = child(inv, inv.root, "#Aurora") else {
        return Vec::new();
    };
    let Some(folder) = inv.folders.get(&brand) else { return Vec::new() };
    let roots: Vec<_> = folder
        .children
        .iter()
        .filter_map(|id| {
            inv.folders
                .get(id)
                .filter(|f| f.info.parent == brand && f.info.name == FOLDER)
                .map(|_| *id)
        })
        .collect();
    let root = native_root(inv, brand);
    let mut out = Vec::new();
    let mut sources: Vec<_> = folder
        .children
        .iter()
        .copied()
        .filter(|id| {
            inv.folders
                .get(id)
                .is_some_and(|f| [STAGING, OLD_STAGING].contains(&f.info.name.as_str()))
        })
        .collect();
    if roots.len() > 1 {
        sources.extend(roots.into_iter().filter(|id| Some(*id) != root));
    }
    for id in sources {
        if inv.folders.get(&id).is_some_and(|f| f.state != FetchState::Fetched) {
            inv.request(id);
            continue;
        }
        let sets = inv.folders.get(&id).map_or(0, |f| f.children.len());
        let duplicate = inv.folders.get(&id).is_some_and(|f| f.info.name == FOLDER);
        out.push(Recovery {
            source: id,
            root,
            sets,
            duplicate,
        });
    }
    out
}

/// Merge staged set folders into the selected native root, preserving every
/// existing set. The source is moved to Trash after its children.
pub fn finish_recovery(inv: &Inventory, recovery: Recovery) -> Result<Mutation, String> {
    let source = inv.folders.get(&recovery.source).ok_or(aurora_i18n::tr!("common-item-not-found"))?;
    let Some(root) = recovery.root else {
        return actions::patch(inv, recovery.source, llsd_map! { "name" => FOLDER });
    };
    let mut change = Mutation::default();
    let existing: HashSet<_> = inv
        .folders
        .get(&root)
        .into_iter()
        .flat_map(|f| f.children.iter())
        .filter_map(|id| inv.folders.get(id).map(|f| f.info.name.clone()))
        .collect();
    let mut reserved = existing;
    for id in &source.children {
        let Some(set) = inv.folders.get(id) else { continue };
        let mut name = set.info.name.clone();
        if reserved.contains(&name) {
            let suffix = " (imported)";
            let base = name.strip_suffix(suffix).unwrap_or(&name).to_owned();
            name = encoded_name(&base, suffix);
            let mut ordinal = 2;
            while reserved.contains(&name) {
                let extra = format!(" {ordinal}{suffix}");
                name = encoded_name(&base, &extra);
                ordinal += 1;
            }
            change.operations.push(Operation::Patch {
                id: *id,
                folder: true,
                body: llsd_map! { "name" => name.clone() },
            });
        }
        reserved.insert(name);
        change.operations.push(Operation::Patch {
            id: *id,
            folder: true,
            body: llsd_map! { "parent_id" => root },
        });
    }
    let trash = actions::system(inv, 14).ok_or(aurora_i18n::tr!("ao-no-trash"))?;
    let trash_move = actions::move_selection(inv, &[recovery.source], trash, &HashSet::new(), &HashSet::new())?;
    change.operations.extend(trash_move.operations);
    change.refresh.extend(trash_move.refresh);
    change.refresh.extend([root, source.info.parent]);
    Ok(finish(change))
}
/// ROOT_AO_FOLDER (Alchemy indra/newview/aoengine.h): the same set format as
/// the Legacy Viewer, directly under the inventory root.
pub const ALCHEMY_FOLDER: &str = "Animation Overrides";

/// Where a catalogue reads its sets. Imports are read-only sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Native,
    Legacy,
    Alchemy,
}

impl Source {
    /// Inventory path shown to the user; folder names are not translated.
    pub fn path(self) -> &'static str {
        match self {
            Self::Native => "#Aurora/Aurora AO",
            Self::Legacy => "#Firestorm/#AO",
            Self::Alchemy => ALCHEMY_FOLDER,
        }
    }

    fn root(self, inv: &Inventory) -> Option<Uuid> {
        match self {
            Self::Native => child(inv, inv.root, "#Aurora").and_then(|brand| native_root(inv, brand)),
            Self::Legacy => child(inv, inv.root, "#Firestorm").and_then(|fs| child(inv, fs, "#AO")),
            Self::Alchemy => child(inv, inv.root, ALCHEMY_FOLDER),
        }
    }
}

#[derive(Default)]
pub struct Catalog {
    pub root: Option<Uuid>,
    pub sets: Vec<Set>,
    pub loading: bool,
    pub failed: bool,
    pub missing: Vec<Uuid>,
    generation: Option<u64>,
    waiting: Vec<(Uuid, FetchState)>,
    source: Source,
}

pub fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    inv.folders
        .get(&parent)?
        .children
        .iter()
        .copied()
        .filter(|id| {
            inv.folders
                .get(id)
                .is_some_and(|f| !f.library && f.info.parent == parent && f.info.name == name)
        })
        .min()
}

fn native_root(inv: &Inventory, brand: Uuid) -> Option<Uuid> {
    let mut roots: Vec<_> = inv
        .folders
        .get(&brand)
        .into_iter()
        .flat_map(|f| f.children.iter())
        .filter_map(|id| {
            inv.folders
                .get(id)
                .filter(|f| !f.library && f.info.parent == brand && f.info.name == FOLDER)
                .map(|f| (f.children.len(), *id))
        })
        .collect();
    roots.sort_by_key(|(children, id)| (std::cmp::Reverse(*children), *id));
    roots.first().map(|(_, id)| *id).or_else(|| child(inv, brand, OLD_FOLDER))
}

/// Inventory names include Aurora's ordering and option suffixes in the same
/// 63-byte limit enforced by AIS. Trim only the base, on a UTF-8 boundary.
pub(super) fn encoded_name(base: &str, suffix: &str) -> String {
    let max_base = 63usize.saturating_sub(suffix.len());
    let mut end = base.len().min(max_base);
    while !base.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &base[..end])
}

fn fetch(inv: &mut Inventory, id: Uuid, waiting: &mut Vec<(Uuid, FetchState)>, failed: &mut bool) -> bool {
    if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Fetched) {
        return true;
    }
    // Failure is displayed until the user requests a reload; no retry loop.
    if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Unknown) {
        inv.request(id);
    }
    if let Some(folder) = inv.folders.get(&id) {
        *failed |= folder.state == FetchState::Failed;
        waiting.push((id, folder.state));
    }
    false
}

pub fn animation(inv: &Inventory, id: Uuid) -> Option<Animation> {
    let original = actions::original(inv, id)?;
    let item = inv.items.get(&original)?;
    if item.asset_type != 20 || item.asset_id.is_nil() || actions::in_type(inv, original, 14) {
        return None;
    }
    let link = inv.items.get(&id)?;
    Some(Animation {
        link: id,
        original,
        asset: item.asset_id,
        name: item.name.clone(),
        order: link.desc.parse::<i32>().ok().filter(|n| *n >= 0).unwrap_or(i32::MAX),
    })
}

fn members(inv: &Inventory, folder: Uuid, missing: &mut Vec<Uuid>) -> Vec<Animation> {
    let Some(f) = inv.folders.get(&folder) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in &f.items {
        let Some(link) = inv.items.get(id).filter(|it| it.asset_type == 24 && it.inv_type == 19) else {
            continue;
        };
        if let Some(a) = animation(inv, *id) {
            out.push(a);
        } else if !link.asset_id.is_nil() && !inv.items.contains_key(&link.asset_id) {
            missing.push(link.asset_id);
        }
    }
    out.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.link.cmp(&b.link))
    });
    out
}

impl Catalog {
    pub fn swimming_ready(&self, selected: Uuid) -> bool {
        self.source == Source::Native && !self.loading && !self.failed && self.selected(selected).is_some_and(|set| set.swimming_ready())
    }
    pub fn invalidate(&mut self) {
        self.generation = None;
    }

    /// Only rebuild after inventory events. Keep a previously loaded set
    /// during partial fetches, so loading another folder never stops the AO.
    pub fn refresh(&mut self, inv: &mut Inventory) {
        self.refresh_source(inv, Source::Native);
    }

    pub(super) fn refresh_firestorm(&mut self, inv: &mut Inventory) {
        self.refresh_source(inv, Source::Legacy);
    }

    pub(super) fn refresh_source(&mut self, inv: &mut Inventory, source: Source) {
        if self.source != source {
            *self = Self {
                source,
                ..Default::default()
            };
        }
        if self.generation == Some(inv.generation)
            && self
                .waiting
                .iter()
                .all(|(id, state)| inv.folders.get(id).is_some_and(|f| f.state == *state))
        {
            return;
        }
        self.generation = Some(inv.generation);
        self.waiting.clear();
        self.failed = false;
        self.missing.clear();
        self.loading = !fetch(inv, inv.root, &mut self.waiting, &mut self.failed);
        // The brand folder must be fetched before its AO child can be found.
        let brand = match source {
            Source::Native => Some("#Aurora"),
            Source::Legacy => Some("#Firestorm"),
            Source::Alchemy => None,
        };
        if let Some(name) = brand {
            let Some(fs) = child(inv, inv.root, name) else {
                self.root = None;
                self.sets.clear();
                return;
            };
            self.loading |= !fetch(inv, fs, &mut self.waiting, &mut self.failed);
            if source == Source::Native {
                let roots: Vec<_> = inv
                    .folders
                    .get(&fs)
                    .into_iter()
                    .flat_map(|f| f.children.iter())
                    .copied()
                    .filter(|id| {
                        inv.folders
                            .get(id)
                            .is_some_and(|f| !f.library && f.info.parent == fs && f.info.name == FOLDER)
                    })
                    .collect();
                for id in roots {
                    self.loading |= !fetch(inv, id, &mut self.waiting, &mut self.failed);
                }
            }
        }
        self.root = source.root(inv);
        let Some(root) = self.root else {
            self.sets.clear();
            return;
        };
        self.loading |= !fetch(inv, root, &mut self.waiting, &mut self.failed);
        let folders = inv.folders.get(&root).map(|f| f.children.clone()).unwrap_or_default();
        let old = std::mem::take(&mut self.sets);
        for id in folders {
            let Some(folder) = inv.folders.get(&id).filter(|f| f.info.parent == root && !f.library) else {
                continue;
            };
            let params: Vec<_> = folder.info.name.split(':').collect();
            let Some(name) = params.first().filter(|name| !name.is_empty()) else {
                continue;
            };
            let mut set = Set {
                id,
                name: (*name).into(),
                sits: params.contains(&"SO"),
                smart: params.contains(&"SM"),
                mouselook: params.contains(&"DM"),
                default: params.contains(&"**"),
                states: HashMap::new(),
            };
            let mut complete = fetch(inv, id, &mut self.waiting, &mut self.failed);
            let states = inv.folders.get(&id).map(|f| f.children.clone()).unwrap_or_default();
            for folder_id in states {
                let Some(folder) = inv.folders.get(&folder_id).filter(|f| f.info.parent == id) else {
                    continue;
                };
                let params: Vec<_> = folder.info.name.split(':').collect();
                let Some(state) = State::parse(params[0]) else {
                    continue;
                };
                // Read actual legacy swim slots, including their cycles and
                // originals. Aurora-only water slots are not source mappings.
                if source != Source::Native && state.swimming() && !State::LEGACY_VIEWER_SWIMMING.contains(&state) {
                    continue;
                }
                let mut config = StateConfig {
                    folder: folder_id,
                    cycle: Cycle::parse(&params),
                    ..Default::default()
                };
                complete &= fetch(inv, folder_id, &mut self.waiting, &mut self.failed);
                for a in members(inv, folder_id, &mut self.missing) {
                    config.steps.push(Step {
                        id: a.link,
                        name: a.name.clone(),
                        order: a.order,
                        group: false,
                        members: vec![a],
                    });
                }
                let children = inv.folders.get(&folder_id).map(|f| f.children.clone()).unwrap_or_default();
                for group in children {
                    let Some(folder) = inv.folders.get(&group).filter(|f| f.info.parent == folder_id) else {
                        continue;
                    };
                    let params: Vec<_> = folder.info.name.split(':').collect();
                    let name = params[0].to_owned();
                    let cycle = Cycle::parse(&params);
                    let track = params.contains(&"TR");
                    let order = params
                        .iter()
                        .find_map(|p| p.strip_prefix("OR").and_then(|n| n.parse::<i32>().ok()))
                        .unwrap_or(i32::MAX);
                    complete &= fetch(inv, group, &mut self.waiting, &mut self.failed);
                    let members = members(inv, group, &mut self.missing);
                    if track {
                        config.tracks.push(Track {
                            id: group,
                            name,
                            cycle,
                            members,
                        });
                    } else {
                        config.steps.push(Step {
                            id: group,
                            name,
                            order,
                            group: true,
                            members,
                        });
                    }
                }
                config.steps.sort_by(|a, b| {
                    a.order
                        .cmp(&b.order)
                        .then_with(|| a.name.cmp(&b.name))
                        .then_with(|| a.id.cmp(&b.id))
                });
                config.tracks.sort_by(|a, b| {
                    editor::folder_order(inv, a.id)
                        .cmp(&editor::folder_order(inv, b.id))
                        .then_with(|| a.name.cmp(&b.name))
                        .then_with(|| a.id.cmp(&b.id))
                });
                set.states.insert(state, config);
            }
            if complete {
                self.sets.push(set);
            } else {
                self.loading = true;
                if let Some(set) = old.iter().find(|s| s.id == id) {
                    self.sets.push(set.clone());
                }
            }
        }
        self.sets.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        self.missing.sort();
        self.missing.dedup();
    }

    pub fn selected(&self, id: Uuid) -> Option<&Set> {
        self.sets
            .iter()
            .find(|s| s.id == id)
            .or_else(|| self.sets.iter().find(|s| s.default))
            .or_else(|| self.sets.first())
    }

    pub fn reload(&mut self, inv: &mut Inventory) {
        let root = self.root.or_else(|| match self.source {
            Source::Native => Source::Legacy.root(inv),
            source => source.root(inv),
        });
        let ids: Vec<_> = inv
            .folders
            .keys()
            .copied()
            .filter(|id| root.is_some_and(|root| actions::under(inv, *id, root)))
            .collect();
        for id in ids {
            if let Some(f) = inv.folders.get_mut(&id) {
                f.state = FetchState::Unknown;
            }
        }
        for name in ["#Aurora", FOLDER, OLD_FOLDER, "#Firestorm", "#AO", ALCHEMY_FOLDER] {
            let ids: Vec<_> = inv
                .folders
                .values()
                .filter(|f| !f.library && f.info.name == name)
                .map(|f| f.info.id)
                .collect();
            for id in ids {
                if let Some(f) = inv.folders.get_mut(&id) {
                    f.state = FetchState::Unknown;
                }
            }
        }
        if let Some(f) = inv.folders.get_mut(&inv.root) {
            f.state = FetchState::Unknown;
        }
        self.invalidate();
    }
}

pub fn valid_name(name: &str) -> Result<String, String> {
    crate::text_validation::require(crate::text_validation::Policy::AoName, name)?;
    let name = name.trim();
    if name.is_empty() || name.len() > 63 || name.contains(':') || name.chars().any(char::is_control) {
        return Err(aurora_i18n::tr!("ao-invalid-name").into());
    }
    Ok(name.into())
}

fn folder(change: &mut Mutation, parent: Uuid, name: String) -> Uuid {
    let id = Uuid::new_v4();
    change.operations.push(Operation::Folder {
        id,
        parent,
        name,
        type_default: -1,
    });
    change.refresh.extend([parent, id]);
    id
}

fn links(change: &mut Mutation, parent: Uuid, members: &[Animation]) {
    if members.is_empty() {
        return;
    }
    change.operations.push(Operation::Links {
        parent,
        links: members
            .iter()
            .enumerate()
            .map(|(order, a)| OutfitLink {
                target: a.original,
                name: a.name.clone(),
                desc: order.to_string(),
                folder: false,
                inv_type: 19,
            })
            .collect(),
    });
    change.refresh.push(parent);
}

fn finish(mut change: Mutation) -> Mutation {
    change.refresh.sort();
    change.refresh.dedup();
    change
}

/// Refresh lists contain parents, so they alone cannot protect an element
/// being renamed or removed. Check operation targets and destinations too.
pub fn editable(inv: &Inventory, change: &Mutation, protected: &HashSet<Uuid>) -> bool {
    // AO startup and subsequent edits can read/link Firestorm originals, but
    // can never write into, rename, move or remove any part of its inventory.
    // The Alchemy import source is protected the same way.
    let sources = [child(inv, inv.root, "#Firestorm"), Source::Alchemy.root(inv)];
    // a locked folder holds only itself: never moved nor removed (`movable`)
    let unlocks = change.operations.iter().any(|op| match op {
        Operation::Patch { id, body, .. } => protected.contains(id) && body.as_map().is_some_and(|m| m.contains_key("parent_id")),
        Operation::Delete { id, .. } | Operation::Purge(id) => protected.contains(id),
        Operation::Folder { .. } | Operation::Links { .. } => false,
    });
    !unlocks
        && change
            .operations
            .iter()
            .flat_map(|op| match op {
                Operation::Folder { parent, .. } | Operation::Links { parent, .. } => [Some(*parent), None],
                Operation::Patch { id, body, .. } => [Some(*id), body.as_map().and_then(|m| m.get("parent_id")).map(|p| p.as_uuid())],
                Operation::Delete { id, .. } | Operation::Purge(id) => [Some(*id), None],
            })
            .flatten()
            .chain(change.copies.iter().map(|copy| copy.parent))
            .chain(change.refresh.iter().copied())
            .filter(|id| inv.folders.contains_key(id) || inv.items.contains_key(id))
            .all(|id| !sources.iter().flatten().any(|root| actions::under(inv, id, *root)) && actions::mutable(inv, id))
}

/// Build the entire parent-before-child mutation before sending it. AIS
/// remaps every new folder UUID, and reads the contents back before success.
pub fn create(inv: &Inventory, catalog: &Catalog, set: &Set) -> Result<(Mutation, Uuid), String> {
    valid_name(&set.name)?;
    if catalog.loading || inv.root.is_nil() || catalog.sets.iter().any(|s| s.name == set.name) {
        return Err(aurora_i18n::tr!("ao-name-or-loading").into());
    }
    let mut change = Mutation::default();
    let fs = child(inv, inv.root, "#Aurora").unwrap_or_else(|| folder(&mut change, inv.root, "#Aurora".into()));
    let root = catalog.root.unwrap_or_else(|| folder(&mut change, fs, FOLDER.into()));
    let id = append_set(&mut change, root, set)?;
    Ok((finish(change), id))
}

fn append_set(change: &mut Mutation, root: Uuid, set: &Set) -> Result<Uuid, String> {
    valid_name(&set.name)?;
    let id = folder(change, root, set.folder_name());
    for state in State::ALL.into_iter().chain(State::LEGACY_SWIMMING) {
        let Some(config) = set.states.get(&state) else {
            continue;
        };
        let parent = folder(change, id, format!("{}{}", state.name(), config.cycle.suffix()));
        for (order, step) in config.steps.iter().enumerate() {
            if step.group {
                let group = folder(change, parent, encoded_name(&valid_name(&step.name)?, &format!(":OR{order}")));
                links(change, group, &step.members);
            } else if let Some(a) = step.members.first() {
                change.operations.push(Operation::Links {
                    parent,
                    links: vec![OutfitLink {
                        target: a.original,
                        name: a.name.clone(),
                        desc: order.to_string(),
                        folder: false,
                        inv_type: 19,
                    }],
                });
            }
        }
        for (order, track) in config.tracks.iter().enumerate() {
            let parent = folder(
                change,
                parent,
                encoded_name(&valid_name(&track.name)?, &format!(":TR:OR{order}{}", track.cycle.suffix())),
            );
            links(change, parent, &track.members);
        }
    }
    if change.operations.len() > 2048 {
        return Err(aurora_i18n::tr!("ao-too-large").into());
    }
    Ok(id)
}

/// Publish only after every set/link was acknowledged. A partially failed
/// import stays separate and is never blindly replayed over user AO data.
pub(super) fn reconstruct(inv: &Inventory, source: &Catalog, selected: Uuid) -> Result<(Mutation, Option<Uuid>), String> {
    let brand = child(inv, inv.root, "#Aurora").ok_or(aurora_i18n::tr!("ao-name-or-loading"))?;
    if let Some(name) = [STAGING, OLD_STAGING]
        .iter()
        .find(|name| child(inv, brand, name).is_some())
        .copied()
    {
        return Err(aurora_i18n::tr!("ao-bootstrap-interrupted", folder = name));
    }
    if native_root(inv, brand).is_some() {
        return Err(aurora_i18n::tr!("ao-native-already-exists").into());
    }
    let mut change = Mutation::default();
    let root = folder(&mut change, brand, STAGING.into());
    let preferred = source.selected(selected).map(|s| s.id);
    let mut select = None;
    for set in &source.sets {
        let mut completed = set.clone();
        super::completion::swimming(&mut completed);
        let id = append_set(&mut change, root, &completed)?;
        if Some(set.id) == preferred {
            select = Some(id);
        }
    }
    change.operations.push(Operation::Patch {
        id: root,
        folder: true,
        body: llsd_map! { "name" => FOLDER },
    });
    change.refresh.push(brand);
    if change.operations.len() > 2048 {
        return Err(aurora_i18n::tr!("ao-too-large").into());
    }
    Ok((finish(change), select))
}

pub fn save_set(inv: &Inventory, set: &Set) -> Result<Mutation, String> {
    valid_name(&set.name)?;
    actions::patch(inv, set.id, llsd_map! { "name" => set.folder_name() })
}

pub fn save_cycle(inv: &Inventory, state: State, config: &StateConfig) -> Result<Mutation, String> {
    actions::patch(
        inv,
        config.folder,
        llsd_map! { "name" => format!("{}{}", state.name(), config.cycle.suffix()) },
    )
}

pub fn add(inv: &Inventory, set: &Set, state: State, item: Uuid, container: Option<Uuid>) -> Result<Mutation, String> {
    let a = animation(inv, item).ok_or(aurora_i18n::tr!("ao-not-animation"))?;
    let mut change = Mutation::default();
    let config = set.states.get(&state);
    let parent = config
        .map(|c| c.folder)
        .unwrap_or_else(|| folder(&mut change, set.id, state.name().into()));
    let parent = container.unwrap_or(parent);
    // Containers must belong to this state, never an arbitrary inventory folder.
    let count = if let Some(container) = container {
        let config = config.ok_or(aurora_i18n::tr!("ao-not-found"))?;
        if let Some(step) = config.steps.iter().find(|s| s.group && s.id == container) {
            normalize_members(inv, &mut change, &step.members)?;
            step.members.len()
        } else if let Some(track) = config.tracks.iter().find(|t| t.id == container) {
            normalize_members(inv, &mut change, &track.members)?;
            track.members.len()
        } else {
            return Err(aurora_i18n::tr!("ao-not-found").into());
        }
    } else {
        if let Some(config) = config {
            normalize_steps(inv, &mut change, &config.steps)?;
        }
        config.map_or(0, |c| c.steps.len())
    };
    change.operations.push(Operation::Links {
        parent,
        links: vec![OutfitLink {
            target: a.original,
            name: a.name,
            desc: count.to_string(),
            folder: false,
            inv_type: 19,
        }],
    });
    change.refresh.push(parent);
    Ok(finish(change))
}

fn normalize_steps(inv: &Inventory, change: &mut Mutation, steps: &[Step]) -> Result<(), String> {
    for (order, step) in steps.iter().enumerate() {
        let body = if step.group {
            llsd_map! { "name" => encoded_name(&step.name, &format!(":OR{order}")) }
        } else {
            llsd_map! { "desc" => order.to_string() }
        };
        let patch = actions::patch(inv, step.id, body)?;
        change.operations.extend(patch.operations);
        change.refresh.extend(patch.refresh);
    }
    Ok(())
}

fn normalize_members(inv: &Inventory, change: &mut Mutation, members: &[Animation]) -> Result<(), String> {
    for (order, a) in members.iter().enumerate() {
        let patch = actions::patch(inv, a.link, llsd_map! { "desc" => order.to_string() })?;
        change.operations.extend(patch.operations);
        change.refresh.extend(patch.refresh);
    }
    Ok(())
}

/// Consolidate the former style slots through acknowledged inventory edits.
/// Move only AO links/containers; originals and unrecognized contents survive.
pub fn consolidate_swimming(inv: &Inventory, set: &Set) -> Result<Mutation, String> {
    if !set.legacy_swimming_pending() {
        return Ok(Mutation::default());
    }
    let mut change = Mutation::default();
    let config = set.states.get(&State::SwimmingForward);
    let parent = config.map_or_else(|| folder(&mut change, set.id, State::SwimmingForward.name().into()), |c| c.folder);
    let mut step_order = config.map_or(0, |c| c.steps.len());
    let mut track_order = config.map_or(0, |c| c.tracks.len());
    if let Some(config) = config {
        normalize_steps(inv, &mut change, &config.steps)?;
        for (order, track) in config.tracks.iter().enumerate() {
            let part = actions::patch(
                inv,
                track.id,
                llsd_map! { "name" => encoded_name(&track.name, &format!(":TR:OR{order}{}", track.cycle.suffix())) },
            )?;
            change.operations.extend(part.operations);
            change.refresh.extend(part.refresh);
        }
    }
    for state in State::LEGACY_SWIMMING {
        let Some(old) = set.states.get(&state) else { continue };
        for step in &old.steps {
            if step.members.is_empty() {
                continue;
            }
            let body = if step.group {
                llsd_map! { "parent_id" => parent, "name" => encoded_name(&step.name, &format!(":OR{step_order}")) }
            } else {
                llsd_map! { "parent_id" => parent, "desc" => step_order.to_string() }
            };
            let part = actions::patch(inv, step.id, body)?;
            change.operations.extend(part.operations);
            change.refresh.extend(part.refresh);
            step_order += 1;
        }
        for track in &old.tracks {
            if track.members.is_empty() {
                continue;
            }
            let part = actions::patch(
                inv,
                track.id,
                llsd_map! {
                    "parent_id" => parent,
                    "name" => encoded_name(&track.name, &format!(":TR:OR{track_order}{}", track.cycle.suffix()))
                },
            )?;
            change.operations.extend(part.operations);
            change.refresh.extend(part.refresh);
            track_order += 1;
        }
        // Keep source folders and all unknown contents intact. A partial move
        // can be reconciled from a fresh read without copying links twice or
        // hiding the remaining assignments behind a renamed/deleted state.
    }
    change.refresh.extend([parent, set.id]);
    if change.operations.len() > 2048 {
        return Err(aurora_i18n::tr!("ao-too-large").into());
    }
    Ok(finish(change))
}

/// Link deletion never deletes the original asset. Folders are moved to
/// Trash, preserving any foreign content somebody put inside an AO folder.
pub fn remove(inv: &Inventory, id: Uuid) -> Result<Mutation, String> {
    if inv.folders.contains_key(&id) {
        let trash = actions::system(inv, 14).ok_or(aurora_i18n::tr!("ao-no-trash"))?;
        actions::patch(inv, id, llsd_map! { "parent_id" => trash })
    } else if let Some(item) = inv.items.get(&id).filter(|it| it.asset_type == 24) {
        Ok(Mutation {
            operations: vec![Operation::Delete { id, folder: false }],
            refresh: vec![item.parent],
            removed: vec![id],
            ..Default::default()
        })
    } else {
        Err(aurora_i18n::tr!("ao-not-found").into())
    }
}

pub fn import(inv: &Inventory, item: Uuid, text: &str) -> Result<(Set, Vec<String>), String> {
    if text.len() > 1024 * 1024 {
        return Err(aurora_i18n::tr!("ao-too-large").into());
    }
    let note = inv
        .items
        .get(&item)
        .filter(|it| it.asset_type == 7)
        .ok_or(aurora_i18n::tr!("ao-not-found"))?;
    if !inv.folders.get(&note.parent).is_some_and(|f| f.state == FetchState::Fetched) {
        return Err(aurora_i18n::tr!("ao-name-or-loading").into());
    }
    let mut set = Set::new(valid_name(&note.name)?);
    let mut by_name = HashMap::new();
    if let Some(folder) = inv.folders.get(&note.parent) {
        for id in &folder.items {
            if let Some(a) = animation(inv, *id) {
                by_name.insert(a.name.clone(), a);
            }
        }
    }
    let mut warnings = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, body)) = line.strip_prefix('[').and_then(|l| l.split_once(']')) else {
            warnings.push(aurora_i18n::tr!(
                "ao-import-line",
                line = (line_no + 1).to_string(),
                name = line.to_owned()
            ));
            continue;
        };
        let Some(state) = State::parse(name.trim()) else {
            warnings.push(aurora_i18n::tr!(
                "ao-import-line",
                line = (line_no + 1).to_string(),
                name = name.to_owned()
            ));
            continue;
        };
        let config = set.states.entry(state).or_default();
        for step in body.split('|') {
            let mut members = Vec::new();
            for name in step.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                if let Some(a) = by_name.get(name) {
                    members.push(a.clone());
                } else {
                    warnings.push(aurora_i18n::tr!(
                        "ao-import-line",
                        line = (line_no + 1).to_string(),
                        name = name.to_owned()
                    ));
                }
            }
            if members.is_empty() {
                continue;
            }
            let order = config.steps.len() as i32;
            let group = members.len() > 1;
            config.steps.push(Step {
                id: Uuid::new_v4(),
                name: if group {
                    format!("Group {}", order + 1)
                } else {
                    members[0].name.clone()
                },
                order,
                group,
                members,
            });
        }
    }
    if set.states.values().flat_map(|c| &c.steps).map(|s| s.members.len()).sum::<usize>() > 2048 {
        return Err(aurora_i18n::tr!("ao-too-large").into());
    }
    warnings.truncate(200);
    if set.states.values().all(|c| c.steps.is_empty()) {
        return Err(aurora_i18n::tr!("ao-invalid-note").into());
    }
    // Reject duplicates within a simultaneous group, which cannot be played twice.
    for config in set.states.values_mut() {
        for step in &mut config.steps {
            let mut seen = HashSet::new();
            step.members.retain(|a| seen.insert(a.asset));
        }
    }
    Ok((set, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::InvFolder;

    fn add_folder(inv: &mut Inventory, parent: Uuid, id: u128, name: &str) -> Uuid {
        let id = Uuid::from_u128(id);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&parent).expect("parent").children.push(id);
        id
    }

    #[test]
    fn generated_ao_folder_names_respect_the_ais_byte_limit() {
        let mut inv = Inventory::default();
        let selected = super::super::demo::seed(&mut inv, Uuid::from_u128(1));
        let mut catalog = Catalog::default();
        catalog.refresh(&mut inv);
        let mut set = catalog.selected(selected).expect("demo set").clone();
        set.name = "S".repeat(63);
        set.sits = true;
        set.smart = true;
        set.mouselook = true;
        set.default = true;
        let config = set.states.get_mut(&State::Standing).expect("standing");
        let mut group = config.steps[0].clone();
        group.name = "G".repeat(62);
        group.group = true;
        config.steps.resize_with(13, || group.clone());
        let (change, _) = create(&inv, &catalog, &set).expect("bounded inventory names");
        let names = change.operations.iter().filter_map(|op| match op {
            Operation::Folder { name, .. } => Some(name.as_str()),
            Operation::Patch { body, .. } => Some(body.as_map()?.get("name")?.as_str()),
            _ => None,
        });
        for name in names {
            assert!(name.len() <= 63, "{name:?} is {} bytes", name.len());
        }
        assert!(set.folder_name().len() <= 63);
        assert_eq!(encoded_name(&"é".repeat(40), ":OR12").len(), 63);
    }

    #[test]
    fn recovery_renames_staging_when_native_root_is_absent() {
        let mut inv = Inventory::default();
        let root = Uuid::from_u128(9);
        inv.root = root;
        inv.folders.insert(
            root,
            Folder {
                info: InvFolder {
                    id: root,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        let brand = Uuid::from_u128(10);
        inv.folders.insert(
            brand,
            Folder {
                info: InvFolder {
                    id: brand,
                    parent: root,
                    name: "#Aurora".into(),
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&root).expect("root").children.push(brand);
        let source = Uuid::from_u128(11);
        inv.folders.insert(
            source,
            Folder {
                info: InvFolder {
                    id: source,
                    parent: brand,
                    name: STAGING.into(),
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&brand).expect("brand").children.push(source);
        let recovery = recoveries(&mut inv).pop().expect("staging recovery");
        let change = finish_recovery(&inv, recovery).expect("rename staging");
        assert!(
            matches!(change.operations.as_slice(), [Operation::Patch { id, body, .. }] if *id == source && body.as_map().and_then(|m| m.get("name")).map(|v| v.as_str()) == Some(FOLDER))
        );
        let refusal = reconstruct(&inv, &Catalog::default(), Uuid::nil()).expect_err("an unfinished import prevents another migration");
        assert!(refusal.contains(STAGING), "{refusal}");
    }

    #[test]
    fn recovery_merges_sets_renames_clashes_and_moves_staging_to_trash() {
        let mut inv = Inventory::default();
        let owner = Uuid::from_u128(1);
        super::super::demo::seed(&mut inv, owner);
        let brand = child(&inv, inv.root, "#Aurora").expect("brand");
        let root = child(&inv, brand, FOLDER).expect("native AO");
        let staging = add_folder(&mut inv, brand, 100, STAGING);
        let clash = add_folder(&mut inv, staging, 101, "Recover me");
        let unique = add_folder(&mut inv, staging, 102, "A unique import");
        add_folder(&mut inv, root, 103, "Recover me");
        let trash = actions::system(&inv, 14).expect("Trash");
        let recovery = recoveries(&mut inv)
            .into_iter()
            .find(|r| r.source == staging)
            .expect("staging recovery");
        let change = finish_recovery(&inv, recovery).expect("merge recovery");
        assert!(change.operations.iter().any(|op| matches!(op, Operation::Patch { id, body, .. } if *id == clash && body.as_map().and_then(|m| m.get("name")).is_some_and(|v| v.as_str() == "Recover me (imported)"))));
        assert!(change.operations.iter().any(|op| matches!(op, Operation::Patch { id, body, .. } if *id == clash && body.as_map().and_then(|m| m.get("parent_id")).is_some_and(|v| v.as_uuid() == root))));
        assert!(change.operations.iter().any(|op| matches!(op, Operation::Patch { id, body, .. } if *id == unique && body.as_map().and_then(|m| m.get("parent_id")).is_some_and(|v| v.as_uuid() == root))));
        assert!(change.operations.iter().any(|op| matches!(op, Operation::Patch { id, body, .. } if *id == staging && body.as_map().and_then(|m| m.get("parent_id")).is_some_and(|v| v.as_uuid() == trash))));
        let names = change.operations.iter().filter_map(|op| match op {
            Operation::Patch { body, .. } => body.as_map()?.get("name").map(|v| v.as_str()),
            _ => None,
        });
        assert!(names.into_iter().all(|name| name.len() <= 63));
        for op in &change.operations {
            let Operation::Patch { id, body, .. } = op else { continue };
            let map = body.as_map().expect("patch body");
            if let Some(name) = map.get("name") {
                inv.folders.get_mut(id).expect("patched folder").info.name = name.as_str().to_owned();
            }
            if let Some(parent) = map.get("parent_id").map(|value| value.as_uuid()) {
                let old = inv.folders[id].info.parent;
                inv.folders.get_mut(&old).expect("old parent").children.retain(|child| child != id);
                inv.folders.get_mut(&parent).expect("new parent").children.push(*id);
                inv.folders.get_mut(id).expect("moved folder").info.parent = parent;
            }
        }
        inv.generation = inv.generation.wrapping_add(1);
        let mut catalog = Catalog::default();
        catalog.refresh(&mut inv);
        assert!(catalog.sets.iter().any(|set| set.name == "Recover me (imported)"));
        assert!(catalog.sets.iter().any(|set| set.name == "A unique import"));
        assert!(inv.folders[&trash].children.contains(&staging));
    }

    #[test]
    fn duplicate_native_roots_prefer_the_one_with_sets_and_offer_the_other_for_merge() {
        let mut inv = Inventory::default();
        super::super::demo::seed(&mut inv, Uuid::from_u128(1));
        let brand = child(&inv, inv.root, "#Aurora").expect("brand");
        let chosen = native_root(&inv, brand).expect("seeded root");
        let duplicate = add_folder(&mut inv, brand, 201, FOLDER);
        let _set = add_folder(&mut inv, duplicate, 202, "Duplicate set");
        // Make the original root empty so the duplicate has more set folders.
        inv.folders.get_mut(&chosen).expect("root").children.clear();
        assert_eq!(native_root(&inv, brand), Some(duplicate));
        assert!(recoveries(&mut inv).iter().any(|r| r.source == chosen && r.root == Some(duplicate)));
    }
}
