//! AOEngine::getGroupFolderName / getTrackFolderName and saveGroup / saveTrack
//! (Firestorm indra/newview/aoengine.cpp, originally LGPL 2.1).
//! Guided native combinations preserve these folder formats.
//! Destination links are acknowledged before old AO links are removed. Originals
//! are never moved, renamed or deleted; failures are reconciled, never replayed.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Main,
    Tracks,
    Members(Uuid),
}

fn missing() -> String {
    aurora_i18n::tr!("ao-not-found").into()
}

fn config<'a>(inv: &Inventory, set: &'a Set, state: State) -> Result<&'a StateConfig, String> {
    let c = set.states.get(&state).ok_or_else(missing)?;
    if !inv
        .folders
        .get(&c.folder)
        .is_some_and(|f| f.info.parent == set.id && f.state == FetchState::Fetched)
    {
        return Err(aurora_i18n::tr!("ao-name-or-loading").into());
    }
    Ok(c)
}

fn name_available(c: Option<&StateConfig>, name: &str, except: Option<Uuid>) -> Result<String, String> {
    let name = valid_name(name)?;
    if c.is_some_and(|c| {
        c.steps.iter().any(|s| s.group && Some(s.id) != except && s.name == name)
            || c.tracks.iter().any(|t| Some(t.id) != except && t.name == name)
    }) {
        return Err(aurora_i18n::tr!("ao-combine-name-exists").into());
    }
    Ok(name)
}

pub fn folder_order(inv: &Inventory, id: Uuid) -> i32 {
    inv.folders
        .get(&id)
        .and_then(|f| f.info.name.split(':').find_map(|p| p.strip_prefix("OR")?.parse::<i32>().ok()))
        .filter(|n| *n >= 0)
        .unwrap_or(i32::MAX)
}

pub fn save_track(inv: &Inventory, track: &Track) -> Result<Mutation, String> {
    actions::patch(
        inv,
        track.id,
        llsd_map! { "name" => format!("{}:TR:OR{}{}", valid_name(&track.name)?, folder_order(inv, track.id), track.cycle.suffix()) },
    )
}

fn append(change: &mut Mutation, part: Mutation) {
    change.operations.extend(part.operations);
    change.refresh.extend(part.refresh);
    change.removed.extend(part.removed);
}

fn step_order(inv: &Inventory, change: &mut Mutation, step: &Step, order: usize) -> Result<(), String> {
    let body = if step.group {
        llsd_map! { "name" => format!("{}:OR{order}", step.name) }
    } else {
        llsd_map! { "desc" => order.to_string() }
    };
    append(change, actions::patch(inv, step.id, body)?);
    Ok(())
}

fn bounded(change: Mutation) -> Result<Mutation, String> {
    if change.operations.len() > 2048 {
        return Err(aurora_i18n::tr!("ao-too-large").into());
    }
    Ok(finish(change))
}

pub fn combine(
    inv: &Inventory,
    set: &Set,
    state: State,
    name: &str,
    track: bool,
    selected: &HashSet<Uuid>,
) -> Result<(Mutation, Uuid), String> {
    let c = set.states.get(&state);
    if c.is_some() {
        config(inv, set, state)?;
    }
    let name = name_available(c, name, None)?;
    let chosen: Vec<_> = c
        .into_iter()
        .flat_map(|c| &c.steps)
        .filter(|s| selected.contains(&s.id) && !s.group && s.members.len() == 1)
        .collect();
    if chosen.len() != selected.len() {
        return Err(missing());
    }
    let mut change = Mutation::default();
    let parent = c.map_or_else(|| folder(&mut change, set.id, state.name().into()), |c| c.folder);
    let position = c.map_or(0, |c| {
        c.steps.iter().position(|s| selected.contains(&s.id)).unwrap_or(c.steps.len())
    });
    let suffix = if track {
        format!(":TR:OR{}:CT10.00:CY", c.map_or(0, |c| c.tracks.len()))
    } else {
        format!(":OR{position}")
    };
    let id = folder(&mut change, parent, format!("{name}{suffix}"));
    let members: Vec<_> = chosen.iter().flat_map(|s| &s.members).cloned().collect();
    links(&mut change, id, &members);
    // Only obsolete Aurora link items are deleted, after destination creation.
    for step in &chosen {
        if !inv.items.get(&step.id).is_some_and(|a| a.parent == parent && a.asset_type == 24) {
            return Err(missing());
        }
        append(&mut change, remove(inv, step.id)?);
    }
    if let Some(c) = c {
        let mut order = 0;
        for step in c.steps.iter().filter(|s| !selected.contains(&s.id)) {
            if !track && order == position {
                order += 1;
            }
            step_order(inv, &mut change, step, order)?;
            order += 1;
        }
    }
    change.refresh.push(parent);
    Ok((bounded(change)?, id))
}

pub fn rename_container(inv: &Inventory, set: &Set, state: State, id: Uuid, name: &str) -> Result<Mutation, String> {
    let c = config(inv, set, state)?;
    let name = name_available(Some(c), name, Some(id))?;
    if let Some((order, _)) = c.steps.iter().enumerate().find(|(_, s)| s.group && s.id == id) {
        actions::patch(inv, id, llsd_map! { "name" => format!("{name}:OR{order}") })
    } else if let Some(track) = c.tracks.iter().find(|t| t.id == id) {
        let mut track = track.clone();
        track.name = name;
        save_track(inv, &track)
    } else {
        Err(missing())
    }
}

pub fn dissolve(inv: &Inventory, set: &Set, state: State, id: Uuid) -> Result<Mutation, String> {
    let c = config(inv, set, state)?;
    let group = c.steps.iter().enumerate().find(|(_, s)| s.group && s.id == id);
    let (position, members) = if let Some((index, step)) = group {
        (index, &step.members)
    } else {
        (c.steps.len(), &c.tracks.iter().find(|t| t.id == id).ok_or_else(missing)?.members)
    };
    if !inv
        .folders
        .get(&id)
        .is_some_and(|f| f.info.parent == c.folder && f.state == FetchState::Fetched)
    {
        return Err(missing());
    }
    let mut change = Mutation::default();
    if !members.is_empty() {
        change.operations.push(Operation::Links {
            parent: c.folder,
            links: members
                .iter()
                .enumerate()
                .map(|(i, a)| OutfitLink {
                    target: a.original,
                    name: a.name.clone(),
                    desc: (position + i).to_string(),
                    folder: false,
                    inv_type: 19,
                })
                .collect(),
        });
    }
    let mut order = 0;
    for step in c.steps.iter().filter(|s| s.id != id) {
        if order == position {
            order += members.len();
        }
        step_order(inv, &mut change, step, order)?;
        order += 1;
    }
    // Keep the old folder and any foreign content recoverable in Trash.
    append(&mut change, remove(inv, id)?);
    change.refresh.push(c.folder);
    bounded(change)
}

pub fn reorder(inv: &Inventory, set: &Set, state: State, list: List, source: Uuid, target: Uuid, after: bool) -> Result<Mutation, String> {
    let c = config(inv, set, state)?;
    let mut ids: Vec<_> = match list {
        List::Main => c.steps.iter().map(|s| s.id).collect(),
        List::Tracks => c.tracks.iter().map(|t| t.id).collect(),
        List::Members(id) => container_members(c, id)?.iter().map(|a| a.link).collect(),
    };
    let from = ids.iter().position(|id| *id == source).ok_or_else(missing)?;
    let to = ids.iter().position(|id| *id == target).ok_or_else(missing)? + usize::from(after);
    let to = to - usize::from(from < to);
    if source == target || from == to {
        return Ok(Mutation::default());
    }
    ids.remove(from);
    ids.insert(to, source);
    let mut change = Mutation::default();
    for (order, id) in ids.into_iter().enumerate() {
        match list {
            List::Main => step_order(inv, &mut change, c.steps.iter().find(|s| s.id == id).ok_or_else(missing)?, order)?,
            List::Tracks => {
                let t = c.tracks.iter().find(|t| t.id == id).ok_or_else(missing)?;
                append(
                    &mut change,
                    actions::patch(
                        inv,
                        id,
                        llsd_map! { "name" => format!("{}:TR:OR{order}{}", t.name, t.cycle.suffix()) },
                    )?,
                );
            }
            List::Members(_) => append(&mut change, actions::patch(inv, id, llsd_map! { "desc" => order.to_string() })?),
        }
    }
    change.refresh.push(c.folder);
    bounded(change)
}

pub fn container_members(c: &StateConfig, id: Uuid) -> Result<&[Animation], String> {
    c.steps
        .iter()
        .find(|s| s.group && s.id == id)
        .map(|s| s.members.as_slice())
        .or_else(|| c.tracks.iter().find(|t| t.id == id).map(|t| t.members.as_slice()))
        .ok_or_else(missing)
}

#[cfg(test)]
mod tests;
