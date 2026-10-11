//! AOEngine::{tick, update, reloadStateAnimations, parseNotecard, saveSet,
//! saveState, addAnimation}: Firestorm's #Firestorm/#AO folders and links.

use super::*;
use crate::world::inventory::{FetchState, Inventory, actions};
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use std::collections::HashSet;

#[derive(Default)]
pub struct Catalog {
    pub root: Option<Uuid>,
    pub sets: Vec<Set>,
    pub loading: bool,
    pub failed: bool,
    pub missing: Vec<Uuid>,
    generation: Option<u64>,
    waiting: Vec<(Uuid, FetchState)>,
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
    pub fn invalidate(&mut self) {
        self.generation = None;
    }

    /// Only rebuild after inventory events. Keep a previously loaded set
    /// during partial fetches, so loading another folder never stops the AO.
    pub fn refresh(&mut self, inv: &mut Inventory) {
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
        let Some(fs) = child(inv, inv.root, "#Firestorm") else {
            self.root = None;
            self.sets.clear();
            return;
        };
        self.loading |= !fetch(inv, fs, &mut self.waiting, &mut self.failed);
        self.root = child(inv, fs, "#AO");
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
                config.tracks.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
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
        let root = self.root;
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
        for name in ["#Firestorm", "#AO"] {
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

/// Build the entire parent-before-child mutation before sending it. AIS
/// remaps every new folder UUID, and reads the contents back before success.
pub fn create(inv: &Inventory, catalog: &Catalog, set: &Set) -> Result<(Mutation, Uuid), String> {
    valid_name(&set.name)?;
    if catalog.loading || inv.root.is_nil() || catalog.sets.iter().any(|s| s.name == set.name) {
        return Err(aurora_i18n::tr!("ao-name-or-loading").into());
    }
    let mut change = Mutation::default();
    let fs = child(inv, inv.root, "#Firestorm").unwrap_or_else(|| folder(&mut change, inv.root, "#Firestorm".into()));
    let root = catalog.root.unwrap_or_else(|| folder(&mut change, fs, "#AO".into()));
    let id = folder(&mut change, root, set.folder_name());
    for state in State::ALL {
        let Some(config) = set.states.get(&state) else {
            continue;
        };
        let parent = folder(&mut change, id, format!("{}{}", state.name(), config.cycle.suffix()));
        for (order, step) in config.steps.iter().enumerate() {
            if step.group {
                let group = folder(&mut change, parent, format!("{}:OR{order}", valid_name(&step.name)?));
                links(&mut change, group, &step.members);
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
        for track in &config.tracks {
            let parent = folder(
                &mut change,
                parent,
                format!("{}:TR{}", valid_name(&track.name)?, track.cycle.suffix()),
            );
            links(&mut change, parent, &track.members);
        }
    }
    if change.operations.len() > 2048 {
        return Err(aurora_i18n::tr!("ao-too-large").into());
    }
    Ok((finish(change), id))
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

pub fn new_container(inv: &Inventory, set: &Set, state: State, name: &str, track: bool) -> Result<Mutation, String> {
    let mut change = Mutation::default();
    let config = set.states.get(&state);
    let name = valid_name(name)?;
    if config.is_some_and(|c| c.steps.iter().any(|s| s.group && s.name == name) || c.tracks.iter().any(|t| t.name == name)) {
        return Err(aurora_i18n::tr!("ao-name-or-loading").into());
    }
    if let Some(config) = config {
        normalize_steps(inv, &mut change, &config.steps)?;
    }
    let parent = config
        .map(|c| c.folder)
        .unwrap_or_else(|| folder(&mut change, set.id, state.name().into()));
    let suffix = if track {
        ":TR".into()
    } else {
        format!(":OR{}", config.map_or(0, |c| c.steps.len()))
    };
    folder(&mut change, parent, format!("{name}{suffix}"));
    Ok(finish(change))
}

pub fn order(inv: &Inventory, config: &StateConfig, index: usize, down: bool) -> Result<Mutation, String> {
    let mut steps = config.steps.clone();
    let other = if down { index.checked_add(1) } else { index.checked_sub(1) }.filter(|i| *i < steps.len());
    let Some(other) = other.filter(|_| index < steps.len()) else {
        return Err(aurora_i18n::tr!("ao-not-found").into());
    };
    steps.swap(index, other);
    let mut change = Mutation::default();
    normalize_steps(inv, &mut change, &steps)?;
    change.refresh.push(config.folder);
    Ok(finish(change))
}

fn normalize_steps(inv: &Inventory, change: &mut Mutation, steps: &[Step]) -> Result<(), String> {
    for (order, step) in steps.iter().enumerate() {
        let body = if step.group {
            llsd_map! { "name" => format!("{}:OR{order}", step.name) }
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

pub fn member_order(inv: &Inventory, members: &[Animation], index: usize, down: bool) -> Result<Mutation, String> {
    let mut members = members.to_vec();
    let other = if down { index.checked_add(1) } else { index.checked_sub(1) }.filter(|i| *i < members.len());
    let Some(other) = other.filter(|_| index < members.len()) else {
        return Err(aurora_i18n::tr!("ao-not-found").into());
    };
    members.swap(index, other);
    let mut change = Mutation::default();
    normalize_members(inv, &mut change, &members)?;
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
