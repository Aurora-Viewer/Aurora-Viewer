//! Undo and redo of the inventory's own changes, as Windows Explorer's
//! Ctrl+Z / Ctrl+Y (#2520). Aurora difference: the Legacy Viewer's
//! inventory has no undo at all.
//!
//! One history for the whole session, shared by every inventory window and
//! pane (Explorer's is shared by all its windows): a drag from one window
//! to another is one step, whichever window undoes it. It holds the moves
//! (drag, Cut / Paste, Move to…), the moves to the Trash (Delete), the
//! renames and what a change made (a new folder, pasted or dragged copies
//! and links), at most [`LIMIT`] steps. Nothing else is recorded: a purge
//! cannot come back, and the outfits, the current outfit and the
//! Marketplace keep their own rules.
//!
//! Second Life's rules win. A step is undone by sending its inverse
//! through the same rules as the user's own change (`actions::move_selection`,
//! `actions::patch`): a rule that refuses (a worn item, a protected folder)
//! leaves the history as it is and says why. What the step made is never
//! purged: undone, it goes to the Trash, and its redo brings it back, so the
//! ids stay the same. Each entry is only taken back from where the step left
//! it: what was moved since, or is gone, is left alone, and a step with
//! nothing left is dropped. A change the server refused never enters the
//! history, and an undo the server refuses goes back where it was.

use super::Inventory;
use super::actions as rules;
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{Mutation, Operation};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

/// The steps kept, as Explorer's own history.
pub const LIMIT: usize = 50;

/// One entry moved by a step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Moved {
    pub id: Uuid,
    pub from: Uuid,
    pub to: Uuid,
    /// `from` and `to` were the Trash or one of its folders when the step
    /// was recorded: only then may an undo or a redo send the entry to the
    /// Trash (a folder deleted since is not a reason to delete what it held).
    pub from_trash: bool,
    pub to_trash: bool,
}

/// What a step made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Made {
    Folder,
    Copy,
    Link,
}

/// One change of the history, as it stands once the server accepted it.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Moved entries; all into the Trash for a delete.
    Move {
        moves: Vec<Moved>,
        trash: bool,
    },
    Rename {
        id: Uuid,
        old: String,
        new: String,
    },
    /// The entries a change made in `parent` (the top ones: a copied
    /// folder's contents follow it).
    Created {
        made: Made,
        parent: Uuid,
        ids: Vec<Uuid>,
    },
}

/// Why a step cannot be taken back or done again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Nothing of the step is left where it was: the step is dropped.
    Gone,
    /// A rule refuses it (its message); the step stays.
    Refused(String),
}

/// The rules' context: the user's locked folders and what is worn.
pub struct Rules<'a> {
    pub protected: &'a HashSet<Uuid>,
    pub worn: &'a HashSet<Uuid>,
}

impl Step {
    /// The entries the step names.
    fn count(&self) -> usize {
        match self {
            Step::Move { moves, .. } => moves.len(),
            Step::Rename { .. } => 1,
            Step::Created { ids, .. } => ids.len(),
        }
    }

    /// What the step did, for "Undo …" and "Redo …": "move of 3 items".
    pub fn what(&self) -> String {
        let count = self.count() as i64;
        match self {
            Step::Move { trash: true, .. } => aurora_i18n::tr!("inventory-history-delete", count = count),
            Step::Move { .. } => aurora_i18n::tr!("inventory-history-move", count = count),
            Step::Rename { .. } => aurora_i18n::tr!("inventory-history-rename").to_owned(),
            Step::Created { made: Made::Folder, .. } => aurora_i18n::tr!("inventory-history-new-folder").to_owned(),
            Step::Created { made: Made::Copy, .. } => aurora_i18n::tr!("inventory-history-copy", count = count),
            Step::Created { made: Made::Link, .. } => aurora_i18n::tr!("inventory-history-link", count = count),
        }
    }

    /// The change that takes the step back (`undo`) or does it again, and
    /// the step as it then stands: only the entries still where the step
    /// (or its undo) left them.
    pub fn plan(&self, inv: &Inventory, undo: bool, rules_of: &Rules<'_>) -> Result<(Mutation, Step), Refusal> {
        match self {
            Step::Move { moves, trash } => {
                let mut change = Mutation::default();
                let mut kept = Vec::new();
                for m in moves {
                    let (at, back) = if undo { (m.to, m.from) } else { (m.from, m.to) };
                    if rules::parent(inv, m.id) != Some(at) {
                        continue;
                    }
                    let back_in_trash = if undo { m.from_trash } else { m.to_trash };
                    let trash_root = rules::system(inv, 14);
                    let back = if rules::destination(inv, back) {
                        back
                    } else if undo && *trash {
                        // its folder is gone or in the Trash itself: Restore's place
                        // (LLItemBridge::restoreItem, the type's folder)
                        match rules::default_folder(inv, m.id) {
                            Some(folder) => folder,
                            None => continue,
                        }
                    } else if let Some(root) = trash_root.filter(|_| back_in_trash) {
                        // it came out of the Trash (a Restore, a drag out of
                        // it), or a delete is done again: back to the Trash's
                        // top, through the rules of a delete (`removable`,
                        // worn items). Decided when the step was recorded: a
                        // folder deleted since sends nothing to the Trash.
                        root
                    } else {
                        // its folder is gone, or in the Trash since
                        continue;
                    };
                    let one = rules::move_selection(inv, &[m.id], back, rules_of.protected, rules_of.worn).map_err(Refusal::Refused)?;
                    change.operations.extend(one.operations);
                    change.refresh.extend(one.refresh);
                    kept.push(if undo {
                        Moved { from: back, ..*m }
                    } else {
                        Moved { to: back, ..*m }
                    });
                }
                if kept.is_empty() {
                    return Err(Refusal::Gone);
                }
                finish(&mut change);
                Ok((
                    change,
                    Step::Move {
                        moves: kept,
                        trash: *trash,
                    },
                ))
            }
            Step::Rename { id, old, new } => {
                let (current, wanted) = if undo { (new, old) } else { (old, new) };
                if !(inv.items.contains_key(id) || inv.folders.contains_key(id)) || rules::name(inv, *id) != *current {
                    return Err(Refusal::Gone);
                }
                if !rules::renameable(inv, *id) {
                    return Err(Refusal::Refused(
                        aurora_i18n::tr!("world-inventory-actions-the-selection-contains-a-protected-or-worn-item").into(),
                    ));
                }
                let change = rules::patch(inv, *id, llsd_map! { "name" => wanted.as_str() }).map_err(Refusal::Refused)?;
                Ok((change, self.clone()))
            }
            Step::Created { made, parent, ids } => {
                let trash =
                    rules::system(inv, 14).ok_or_else(|| Refusal::Refused(aurora_i18n::tr!("common-trash-folder-not-found").into()))?;
                // undone: from its folder to the Trash; done again: back
                let kept: Vec<Uuid> = ids
                    .iter()
                    .copied()
                    .filter(|id| {
                        if undo {
                            rules::parent(inv, *id) == Some(*parent)
                        } else {
                            rules::parent(inv, *id) == Some(trash)
                        }
                    })
                    .collect();
                if kept.is_empty() {
                    return Err(Refusal::Gone);
                }
                // done again only into a folder that still takes entries
                if !undo && !rules::destination(inv, *parent) {
                    return Err(Refusal::Gone);
                }
                // a new folder holds only what was put there since, maybe
                // by a route the history does not record (an offer, an
                // outfit saved): not sent to the Trash with it
                if undo
                    && *made == Made::Folder
                    && kept
                        .iter()
                        .any(|id| inv.folders.get(id).is_some_and(|f| !f.items.is_empty() || !f.children.is_empty()))
                {
                    return Err(Refusal::Refused(aurora_i18n::tr!("inventory-undo-folder-not-empty").into()));
                }
                let to = if undo { trash } else { *parent };
                let change = rules::move_selection(inv, &kept, to, rules_of.protected, rules_of.worn).map_err(Refusal::Refused)?;
                Ok((
                    change,
                    Step::Created {
                        made: *made,
                        parent: *parent,
                        ids: kept,
                    },
                ))
            }
        }
    }
}

fn finish(change: &mut Mutation) {
    change.refresh.sort();
    change.refresh.dedup();
}

/// What a copy will look like: its name, its type, its creator and its
/// base rights, all of which a copy keeps (an unrelated item that lands in
/// the folder meanwhile is not taken for it).
#[derive(Clone, Debug, PartialEq)]
pub struct Copied {
    name: String,
    asset_type: i32,
    inv_type: i32,
    creator: Uuid,
    base_mask: u32,
}

impl Copied {
    fn is(&self, it: &aurora_net::inventory::InvItem) -> bool {
        it.name == self.name
            && it.asset_type == self.asset_type
            && it.inv_type == self.inv_type
            && it.creator == self.creator
            && it.base_mask == self.base_mask
    }
}

/// A change on its way, waiting for the server to accept it.
#[derive(Clone, Debug, PartialEq)]
pub enum Draft {
    /// A move or a rename: known before the reply.
    Ready(Step),
    /// What a change makes: its ids come with the reply (the new folders)
    /// or with the folder's contents (links and copies, found by their
    /// target or their name among the entries `parent` did not hold).
    Made {
        made: Made,
        parent: Uuid,
        /// The new top folders, by the ids the change gave them.
        folders: Vec<Uuid>,
        /// The originals linked.
        links: Vec<Uuid>,
        /// The items copied, as their copies will be.
        copies: Vec<Copied>,
        /// What `parent` held before.
        before: HashSet<Uuid>,
        /// The server's ids of the new folders, from the first reply of a
        /// change whose item copies finish later.
        created: HashMap<Uuid, Uuid>,
    },
}

/// Whether `change` touches the current outfit or the Marketplace listings,
/// which keep their own rules (the outfit's links, the merchant's
/// questions, `listings::guard_inventory_change`): never recorded, and
/// never undone there either when the entries went there since.
fn kept_out(inv: &Inventory, change: &Mutation) -> bool {
    let cof = super::super::appearance::cof(inv);
    let market = super::super::marketplace::root(inv);
    let touched = |id: Uuid| Some(id) == cof || market.is_some_and(|root| id == root || rules::under(inv, id, root));
    change.refresh.iter().any(|id| touched(*id))
        || change.operations.iter().any(|op| match op {
            Operation::Patch { id, .. } => rules::parent(inv, *id).is_some_and(touched) || touched(*id),
            _ => false,
        })
}

/// The step a change will be, when the history keeps it: moves, a rename,
/// or a change that only makes folders, links and copies in one folder.
/// None for anything else, and for what touches the current outfit or the
/// Marketplace.
pub fn draft(inv: &Inventory, change: &Mutation) -> Option<Draft> {
    if change.operations.is_empty() && change.copies.is_empty() || kept_out(inv, change) {
        return None;
    }
    if change.move_only() {
        let trash = rules::system(inv, 14);
        let in_trash = |folder: Uuid| trash.is_some_and(|trash| folder == trash || rules::under(inv, folder, trash));
        let moves = change
            .operations
            .iter()
            .map(|op| match op {
                Operation::Patch { id, body, .. } => {
                    let (from, to) = (rules::parent(inv, *id)?, body["parent_id"].as_uuid());
                    Some(Moved {
                        id: *id,
                        from,
                        to,
                        from_trash: in_trash(from),
                        to_trash: in_trash(to),
                    })
                }
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let trash = trash.is_some_and(|trash| moves.iter().all(|m| m.to == trash));
        return Some(Draft::Ready(Step::Move { moves, trash }));
    }
    if let [Operation::Patch { id, body, .. }] = change.operations.as_slice()
        && change.copies.is_empty()
        && body.as_map().is_some_and(|m| m.len() == 1 && m.contains_key("name"))
    {
        let old = rules::name(inv, *id);
        let new = body["name"].to_string_value();
        return (old != new).then_some(Draft::Ready(Step::Rename { id: *id, old, new }));
    }
    // made: folders, links and copies only, their tops in one folder
    let local: HashSet<Uuid> = change
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Folder { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    let mut parent = None;
    let mut same = |p: Uuid| match parent {
        None => {
            parent = Some(p);
            true
        }
        Some(q) => q == p,
    };
    let (mut folders, mut links, mut copies) = (Vec::new(), Vec::new(), Vec::new());
    for op in &change.operations {
        match op {
            Operation::Folder { id, parent: p, .. } if !local.contains(p) => {
                if !same(*p) {
                    return None;
                }
                folders.push(*id);
            }
            Operation::Folder { .. } => {}
            Operation::Links { parent: p, links: made } if !local.contains(p) => {
                if !same(*p) {
                    return None;
                }
                links.extend(made.iter().map(|l| l.target));
            }
            Operation::Links { .. } => {}
            _ => return None,
        }
    }
    for copy in change.copies.iter().filter(|c| !local.contains(&c.parent)) {
        if !same(copy.parent) {
            return None;
        }
        let original = inv.items.get(&copy.item)?;
        copies.push(Copied {
            name: if copy.name.is_empty() {
                original.name.clone()
            } else {
                copy.name.clone()
            },
            asset_type: original.asset_type,
            inv_type: original.inv_type,
            creator: original.creator,
            base_mask: original.base_mask,
        });
    }
    let parent = parent?;
    // links and copies are found among the folder's new entries: a folder
    // not read yet has no "before" to tell them from what it held
    if (!links.is_empty() || !copies.is_empty()) && inv.folders.get(&parent).is_none_or(|f| f.state != super::FetchState::Fetched) {
        return None;
    }
    let made = if change.operations.len() == 1 && folders.len() == 1 && copies.is_empty() {
        Made::Folder
    } else if folders.is_empty() && copies.is_empty() {
        Made::Link
    } else {
        Made::Copy
    };
    let before = inv
        .folders
        .get(&parent)
        .map(|f| f.children.iter().chain(&f.items).copied().collect())
        .unwrap_or_default();
    Some(Draft::Made {
        made,
        parent,
        folders,
        links,
        copies,
        before,
        created: HashMap::new(),
    })
}

impl Draft {
    /// The step, once the server accepted the change and the inventory
    /// holds what it made; None when nothing it made is found.
    pub fn finish(self, inv: &Inventory, created: &HashMap<Uuid, Uuid>, claimed: &HashSet<Uuid>) -> Option<Step> {
        match self {
            Draft::Ready(step) => Some(step),
            Draft::Made {
                made,
                parent,
                folders,
                mut links,
                mut copies,
                before,
                created: first,
            } => {
                let at = |id: &Uuid| created.get(id).or_else(|| first.get(id)).copied().unwrap_or(*id);
                let mut ids: Vec<Uuid> = folders.iter().map(at).filter(|id| inv.folders.contains_key(id)).collect();
                if let Some(folder) = inv.folders.get(&parent) {
                    // not what another step already named: two pastes of
                    // the same name into one folder each get their copy
                    for id in folder.items.iter().filter(|id| !before.contains(id) && !claimed.contains(id)) {
                        let Some(it) = inv.items.get(id) else { continue };
                        let found = if matches!(it.asset_type, 24 | 25) {
                            links.iter().position(|target| *target == it.asset_id).map(|i| links.swap_remove(i))
                        } else {
                            copies
                                .iter()
                                .position(|copied| copied.is(it))
                                .map(|i| copies.swap_remove(i))
                                .map(|_| *id)
                        };
                        if found.is_some() {
                            ids.push(*id);
                        }
                    }
                }
                (!ids.is_empty()).then_some(Step::Created { made, parent, ids })
            }
        }
    }
}

/// A request the history waits on.
#[derive(Clone, Debug, PartialEq)]
enum Flight {
    Record(Draft),
    Undo(Step),
    Redo(Step),
}

/// The menu's step of Undo and of Redo (`History::shown`), by the inventory
/// generation and the lengths of both lists it was found for.
type Shown = [Option<((u64, usize, usize), Option<usize>)>; 2];

/// The session's history (see the module).
#[derive(Default, Debug)]
pub struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    flights: HashMap<Uuid, Flight>,
    /// The world is shared with other threads: a lock, not a `Cell`.
    shown: std::sync::Mutex<Shown>,
}

impl History {
    /// The step at the top of Undo (the menu names `shown`'s).
    #[cfg(test)]
    pub fn next_undo(&self) -> Option<&Step> {
        self.undo.back()
    }

    /// The step at the top of Redo.
    #[cfg(test)]
    pub fn next_redo(&self) -> Option<&Step> {
        self.redo.last()
    }

    /// An undo, a redo or a recorded change is on its way: the history
    /// waits for it, so that its steps stay in order (a paste's copies may
    /// arrive long after the inventory's own lock is free; an undo then
    /// would take the step before, and the paste's arrival would wipe Redo).
    pub fn busy(&self) -> bool {
        !self.flights.is_empty()
    }

    /// A new change of the user's: kept, and the steps undone are gone.
    fn push(&mut self, step: Step) {
        self.undo.push_back(step);
        while self.undo.len() > LIMIT {
            self.undo.pop_front();
        }
        self.redo.clear();
    }

    /// The user's change `request` left: its step is kept once the server
    /// accepts it.
    pub fn record(&mut self, request: Uuid, draft: Draft) {
        self.flights.insert(request, Flight::Record(draft));
    }

    /// The change that takes back the last step (`undo`) or does the last
    /// one undone again. The steps with nothing left are dropped on the way;
    /// a step a rule refuses stays, with the rule's message.
    ///
    /// A step whose entries all moved or are gone is dropped. From the keys
    /// (`named` false) the undo then stops there, saying so: the user did
    /// not see which step comes next. From the menu, which named the step
    /// it would take (`shown`), it goes on to that one.
    pub fn plan(&mut self, inv: &Inventory, undo: bool, rules_of: &Rules<'_>, named: bool) -> Result<(Mutation, Step), String> {
        let mut dropped = false;
        loop {
            let next = if undo { self.undo.back() } else { self.redo.last() };
            let Some(step) = next else {
                return Err(match (undo, dropped) {
                    (true, true) => aurora_i18n::tr!("inventory-undo-nothing"),
                    (false, true) => aurora_i18n::tr!("inventory-redo-nothing"),
                    (true, false) => aurora_i18n::tr!("inventory-undo-empty"),
                    (false, false) => aurora_i18n::tr!("inventory-redo-empty"),
                }
                .into());
            };
            if dropped && !named {
                return Err(if undo {
                    aurora_i18n::tr!("inventory-undo-dropped")
                } else {
                    aurora_i18n::tr!("inventory-redo-dropped")
                }
                .into());
            }
            match step.plan(inv, undo, rules_of) {
                // what went to the outfit or the Marketplace since keeps
                // their rules
                Ok((change, _)) if kept_out(inv, &change) => {
                    return Err(aurora_i18n::tr!("inventory-undo-kept-out").into());
                }
                Ok(plan) => return Ok(plan),
                Err(Refusal::Refused(reason)) => return Err(reason),
                Err(Refusal::Gone) => {
                    dropped = true;
                    if undo {
                        self.undo.pop_back();
                    } else {
                        self.redo.pop();
                    }
                }
            }
        }
    }

    /// The step an Undo (or Redo) would take now, for its menu entry: the
    /// newest whose entries are still where it left them (`plan` drops the
    /// others first).
    /// Planning a step (a delete of thousands of entries) is not redone
    /// every frame the menu is open: it is kept until the inventory or the
    /// history changes.
    pub fn shown(&self, inv: &Inventory, undo: bool) -> Option<&Step> {
        let key = (inv.generation, self.undo.len(), self.redo.len());
        let mut cache = self.shown.lock().map(|cache| *cache).unwrap_or_default();
        let slot = &mut cache[usize::from(!undo)];
        let back = match *slot {
            Some((at, back)) if at == key => back,
            _ => {
                let none = HashSet::new();
                let rules_of = Rules {
                    protected: &none,
                    worn: &none,
                };
                let mut steps: Box<dyn Iterator<Item = &Step>> = if undo {
                    Box::new(self.undo.iter().rev())
                } else {
                    Box::new(self.redo.iter().rev())
                };
                let back = steps.position(|step| !matches!(step.plan(inv, undo, &rules_of), Err(Refusal::Gone)));
                *slot = Some((key, back));
                if let Ok(mut kept) = self.shown.lock() {
                    *kept = cache;
                }
                back
            }
        };
        let back = back?;
        if undo {
            self.undo.iter().rev().nth(back)
        } else {
            self.redo.iter().rev().nth(back)
        }
    }

    /// The entries the steps made: a copy found for one is not another's.
    fn claimed(&self) -> HashSet<Uuid> {
        self.undo
            .iter()
            .chain(&self.redo)
            .filter_map(|step| match step {
                Step::Created { ids, .. } => Some(ids),
                _ => None,
            })
            .flatten()
            .copied()
            .collect()
    }

    /// The undo or redo `request` left with the change `plan` gave: the
    /// step waits there until the server answers.
    pub fn begin(&mut self, request: Uuid, undo: bool, step: Step) {
        if undo {
            self.undo.pop_back();
            self.flights.insert(request, Flight::Undo(step));
        } else {
            self.redo.pop();
            self.flights.insert(request, Flight::Redo(step));
        }
    }

    /// The first reply of a change whose item copies finish later: the ids
    /// of the folders it made, kept for its end.
    pub fn created(&mut self, request: Uuid, ids: &HashMap<Uuid, Uuid>) {
        if let Some(Flight::Record(Draft::Made { created, .. })) = self.flights.get_mut(&request) {
            created.extend(ids.iter().map(|(k, v)| (*k, *v)));
        }
    }

    /// The server answered `request`. A refused change is no step; a
    /// refused undo or redo puts its step back.
    pub fn finish(&mut self, request: Uuid, inv: &Inventory, accepted: bool, created: &HashMap<Uuid, Uuid>) {
        let Some(flight) = self.flights.remove(&request) else { return };
        match (flight, accepted) {
            (Flight::Record(draft), true) => {
                if let Some(step) = draft.finish(inv, created, &self.claimed()) {
                    self.push(step);
                }
            }
            (Flight::Record(_), false) => {}
            (Flight::Undo(step), true) => self.redo.push(step),
            (Flight::Undo(step), false) => self.undo.push_back(step),
            (Flight::Redo(step), true) => self.undo.push_back(step),
            (Flight::Redo(step), false) => self.redo.push(step),
        }
    }
}

impl Inventory {
    /// The user's `change` leaves as `request`: its step waits for the
    /// server (`History::record`), when the history keeps such a change.
    pub fn record_change(&mut self, request: Uuid, change: &Mutation) {
        if let Some(draft) = draft(self, change) {
            self.undo.record(request, draft);
        }
    }

    /// The server answered `request`: the history and the Trash's origins
    /// keep what it accepted.
    pub fn changed(&mut self, request: Uuid, accepted: bool, created: &HashMap<Uuid, Uuid>) {
        let mut history = std::mem::take(&mut self.undo);
        history.finish(request, self, accepted, created);
        self.undo = history;
        let mut origins = std::mem::take(&mut self.trash_origins);
        origins.finish(request, self, accepted);
        self.trash_origins = origins;
    }

    /// The change that undoes (or redoes) the history's next step: see
    /// `History::plan`.
    pub fn plan_history(&mut self, undo: bool, rules_of: &Rules<'_>, named: bool) -> Result<(Mutation, Step), String> {
        let mut history = std::mem::take(&mut self.undo);
        let plan = history.plan(self, undo, rules_of, named);
        self.undo = history;
        plan
    }

    /// Demo (`AURORA_DEMO_INVENTORY=aurora-explorer-undo`): the right-click
    /// tests' notecard deleted (Undo, and the Trash's Restore to its
    /// folder), then the animation renamed and that rename undone (Redo),
    /// each through the same path as the user's own changes.
    pub fn demo_history(&mut self, agent: Uuid) {
        let none = HashSet::new();
        let rules_of = Rules {
            protected: &none,
            worn: &none,
        };
        let Some(trash) = rules::system(self, 14) else { return };
        let (notecard, animation) = (Uuid::from_u128(8103), Uuid::from_u128(8101));
        let renamed = format!("{} 2", rules::name(self, animation));
        let changes = [
            rules::move_selection(self, &[notecard], trash, &none, &none).ok(),
            rules::patch(self, animation, llsd_map! { "name" => renamed }).ok(),
        ];
        for change in changes.into_iter().flatten() {
            self.demo_send(agent, change, |inv, request, change| inv.record_change(request, change));
        }
        if let Ok((change, step)) = self.plan_history(true, &rules_of, true) {
            self.demo_send(agent, change, |inv, request, _| inv.undo.begin(request, true, step));
        }
    }

    fn demo_send(&mut self, agent: Uuid, change: Mutation, begin: impl FnOnce(&mut Inventory, Uuid, &Mutation)) {
        let request = Uuid::new_v4();
        let moves = super::trash_origins::TrashOrigins::moves(self, &change);
        self.trash_origins.begin(request, moves);
        begin(self, request, &change);
        match rules::demo_mutate(self, agent, change) {
            Ok(reply) => self.changed(request, true, &reply.created),
            Err(error) => {
                log::warn!("demo history: {error}");
                self.changed(request, false, &HashMap::new());
            }
        }
    }
}

#[cfg(test)]
#[path = "undo_tests.rs"]
mod tests;

/// A small inventory of the agent's own for the history's tests and the
/// Trash's origins.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};

    pub(crate) struct Ids {
        pub agent: Uuid,
        pub trash: Uuid,
        pub trash_sub: Uuid,
        pub clothes: Uuid,
        pub objects: Uuid,
        pub shirt: Uuid,
        pub hat: Uuid,
        /// No copy, no modify.
        pub sword: Uuid,
    }

    pub(crate) fn folder(inv: &mut Inventory, id: Uuid, parent: Uuid, name: &str, kind: i32) {
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: kind,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(p) = inv.folders.get_mut(&parent) {
            p.children.push(id);
        }
    }

    /// Root › Clothes (Shirt, Hat), Objects (Sword), Trash › Old.
    pub(crate) fn fixture() -> (Inventory, Ids) {
        let n = Uuid::from_u128;
        let ids = Ids {
            agent: n(2),
            trash: n(10),
            trash_sub: n(11),
            clothes: n(20),
            objects: n(21),
            shirt: n(30),
            hat: n(31),
            sword: n(32),
        };
        let mut inv = Inventory {
            root: n(1),
            owner: ids.agent,
            ..Default::default()
        };
        let root = inv.root;
        folder(&mut inv, root, Uuid::nil(), "My Inventory", 8);
        folder(&mut inv, ids.trash, root, "Trash", 14);
        folder(&mut inv, ids.trash_sub, ids.trash, "Old", -1);
        folder(&mut inv, ids.clothes, root, "Clothes", -1);
        folder(&mut inv, ids.objects, root, "Objects", -1);
        let item = |id, parent, name: &str, mask: u32| InvItem {
            id,
            parent,
            owner: ids.agent,
            creator: ids.agent,
            name: name.into(),
            asset_type: 6,
            inv_type: 6,
            asset_id: Uuid::new_v4(),
            owner_mask: mask,
            ..Default::default()
        };
        inv.add_items(vec![
            item(ids.shirt, ids.clothes, "Shirt", 0x7fff_ffff),
            item(ids.hat, ids.clothes, "Hat", 0x7fff_ffff),
            item(ids.sword, ids.objects, "Sword", rules::TRANSFER | rules::MOVE),
        ]);
        inv.generation += 1;
        (inv, ids)
    }
}
