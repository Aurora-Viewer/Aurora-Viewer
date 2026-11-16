//! The folder commands of the Avatar Welcome Pack's trusted page
//! (`ui/avatar_welcome.rs`): port of LLWearFolderHandler, LLAddFolderHandler,
//! LLRemoveFolderHandler and wear_category (indra/newview/llappearancemgr.cpp,
//! originally LGPL 2.1).
//!
//! - `wear_folder` / `add_folder`: the folder named `folder_name` in the
//!   Library (findDescendentCategoryIDByName: the first one of that name,
//!   case-insensitive), else the `folder_id` one. A Library folder is copied
//!   first, its own items only, into the Clothing folder
//!   (wearInventoryCategory: AISAPI::CopyLibraryCategory without subfolders,
//!   or wearCategoryFinal's createNewCategory and item copies), then the
//!   copy is worn: replacing the outfit, or added to it
//!   (LLWearCategoryAfterCopy → wearInventoryCategoryOnAvatar). A folder of
//!   the agent's inventory is worn as it is.
//! - `remove_folder`: what the `folder_id` folder holds is taken off
//!   (takeOffOutfit).
//!
//! The page sends the body (`wear_folder`) then the clothing (`add_folder`)
//! two and a half seconds later: the commands run one after the other, in
//! the order sent, so that a slow copy of the body never replaces the
//! clothing worn after it. A folder not found, a copy that fails (one lost
//! item fails the whole copy: the Legacy Viewer's CopyLibraryCategory is one
//! server request, ours is many item copies) or a stage that makes no
//! progress for a minute leaves the outfit as it is, with a line in the log,
//! and the copied folder stays in Clothing, as the Legacy Viewer does on a
//! failed copy (wearInventoryCategoryOnAvatar returns on a null category,
//! llappearancemgr.cpp). When a `wear_folder` fails, the `add_folder` of the
//! same choice is skipped, so that the clothing is not layered on the old
//! outfit: each `add_folder` belongs to the oldest `wear_folder` sent less
//! than `PAIR_WINDOW` before it that has none yet, so that two choices made
//! in a row (body A, body B, clothing A, clothing B) are told apart. A copy
//! that times out is still finished when its reply comes, but never worn. At
//! most eight commands wait.
//!
//! A new account's first login wears its registration outfit the same way
//! (`initial_command`, LLStartUp::loadInitialOutfit: a My Outfits folder as
//! it is, else the Library's copied). A wear waits for the Current Outfit
//! folder to be loaded, which the first login does not restore
//! (callAfterCOFFetch before wearInventoryCategory).

use super::*;
use crate::slapp::{FolderAction, FolderCommand};
use crate::world::appearance::{FT_CLOTHING, WearMode};
use crate::world::inventory::{FetchState, Inventory, actions as rules};
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation, Reply};
use aurora_net::outfits::OutfitLink;
use std::collections::VecDeque;
use uuid::Uuid;

/// A stage (finding the folder, the copy, the wait for the outfit window)
/// that makes no progress for this long ends its command.
const TIMEOUT: Duration = Duration::from_secs(60);
/// The page sends two commands a choice: eight is four choices in a row.
const QUEUE_CAP: usize = 8;
/// Timed-out copies still waiting for their reply. A bound, not a guarantee:
/// past it the oldest is forgotten and its folder stays as it is.
const LATE_CAP: usize = 4;
/// The page sends a choice's `add_folder` two and a half seconds after its
/// `wear_folder`: an `add_folder` sent later than this after a `wear_folder`
/// is not that choice's (a choice without clothing, say).
const PAIR_WINDOW: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct WelcomeWears {
    queue: VecDeque<Queued>,
    job: Option<Job>,
    /// Copies whose job timed out: `(request, folder)`. Their late reply is
    /// still applied and finished, like ContentsCopies' `timed_out`, so that
    /// the folder is not left empty; they never wear anything.
    late: Vec<(Uuid, Uuid)>,
    /// The `wear_folder`s of the last `PAIR_WINDOW` still waiting for their
    /// `add_folder`, oldest first.
    unpaired: VecDeque<Unpaired>,
    /// The number of the next `wear_folder`.
    next_wear: u64,
}

/// A queued command and its choice: for a `wear_folder` its own number, for
/// an `add_folder` the number of the `wear_folder` it belongs to.
struct Queued {
    command: FolderCommand,
    wear: Option<u64>,
}

struct Unpaired {
    wear: u64,
    at: Instant,
    /// Its `add_folder` is skipped when it comes.
    failed: bool,
}

struct Job {
    command: FolderCommand,
    wear: Option<u64>,
    since: Instant,
    stage: Stage,
}

#[derive(Debug)]
enum Stage {
    /// Finding the folder, and loading a Library one before its copy.
    Find,
    /// The copy on its way: `root` is the new folder (the asked id, then the
    /// server's once its folder is created).
    Copy { request: Uuid, root: Uuid },
    /// Waiting for the folder's links to resolve, then for the outfit window.
    Wear { folder: Uuid },
}

impl WelcomeWears {
    /// Queues the commands, at most `QUEUE_CAP` in all. The queue is not
    /// emptied when the window closes: that would lose a body's `add_folder`.
    pub fn push(&mut self, commands: Vec<FolderCommand>, now: Instant) {
        self.unpaired.retain(|body| now.saturating_duration_since(body.at) <= PAIR_WINDOW);
        for command in commands {
            let full = self.queue.len() >= QUEUE_CAP;
            let wear = match command.action {
                FolderAction::Wear => {
                    let wear = self.next_wear;
                    self.next_wear += 1;
                    // a body dropped from the queue is not worn: nor is its clothing
                    self.unpaired.push_back(Unpaired {
                        wear,
                        at: now,
                        failed: full,
                    });
                    Some(wear)
                }
                FolderAction::Add => match self.unpaired.pop_front() {
                    Some(body) if body.failed => {
                        log::warn!(
                            "avatar welcome pack: add_folder {:?} skipped, the wear_folder before it failed",
                            command.name
                        );
                        continue;
                    }
                    body => body.map(|body| body.wear),
                },
                FolderAction::Remove => None,
            };
            if full {
                log::warn!("avatar welcome pack: too many folder commands queued, {:?} dropped", command.name);
                continue;
            }
            self.queue.push_back(Queued { command, wear });
        }
    }

    /// A command ended without wearing: when it is a `wear_folder`, the
    /// `add_folder` of its choice is skipped, queued or still to come.
    fn failed(&mut self, action: FolderAction, wear: Option<u64>) {
        let (FolderAction::Wear, Some(wear)) = (action, wear) else {
            return;
        };
        let theirs = |q: &Queued| q.command.action == FolderAction::Add && q.wear == Some(wear);
        if let Some(at) = self.queue.iter().position(theirs) {
            if let Some(skipped) = self.queue.remove(at) {
                log::warn!(
                    "avatar welcome pack: add_folder {:?} skipped, the wear_folder before it failed",
                    skipped.command.name
                );
            }
        } else if let Some(body) = self.unpaired.iter_mut().find(|body| body.wear == wear) {
            body.failed = true;
        }
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.late.iter().any(|(r, _)| *r == request)
            || self
                .job
                .as_ref()
                .is_some_and(|job| matches!(job.stage, Stage::Copy { request: r, .. } if r == request))
    }
}

/// findDescendentCategoryIDByName under the Library: the first folder named
/// `name`, case-insensitive, depth first in the folders' order.
/// The `wear_folder` that dresses a new account: its My Outfits folder by
/// id, else the Library's by name (copied first), replacing the outfit.
pub(super) fn initial_command(outfit: crate::world::outfit::InitialOutfit) -> FolderCommand {
    use crate::world::outfit::InitialOutfit;
    let (name, id) = match outfit {
        InitialOutfit::Mine(folder) => (None, Some(folder)),
        InitialOutfit::Library(name) => (Some(name), None),
    };
    FolderCommand {
        action: FolderAction::Wear,
        name,
        id,
    }
}

pub(super) fn library_folder(inv: &Inventory, name: &str) -> Option<Uuid> {
    crate::world::appearance::descendant_named(inv, inv.lib_root, name)
}

/// wear_category's folder: the Library one of the name, else the id's.
pub(super) fn folder_of(inv: &Inventory, command: &FolderCommand) -> Option<Uuid> {
    command
        .name
        .as_deref()
        .and_then(|name| library_folder(inv, name))
        .or_else(|| command.id.filter(|id| inv.folders.contains_key(id)))
}

/// The copy of a loaded Library folder into `to`: a folder of the same
/// name, the folder's own items copied into it (its subfolders are not:
/// CopyLibraryCategory's copy_subfolders=false), a link linked again.
/// Returns the new folder's asked id and the change.
pub(super) fn copy_plan(inv: &Inventory, folder: Uuid, to: Uuid) -> Result<(Uuid, Mutation), String> {
    let f = inv.folders.get(&folder).ok_or(aurora_i18n::tr!("common-folder-not-found"))?;
    if f.state != FetchState::Fetched {
        return Err(aurora_i18n::tr!("world-appearance-this-outfit-s-inventory-is-still-loading").into());
    }
    let root = Uuid::new_v4();
    let mut change = Mutation {
        operations: vec![Operation::Folder {
            id: root,
            parent: to,
            name: f.info.name.clone(),
            type_default: -1,
        }],
        refresh: vec![to, root],
        ..Default::default()
    };
    let mut links = Vec::new();
    for item in f.items.iter().filter_map(|id| inv.items.get(id)) {
        if matches!(item.asset_type, 24 | 25) {
            links.push(OutfitLink {
                target: item.asset_id,
                name: item.name.clone(),
                desc: item.desc.clone(),
                folder: item.asset_type == 25,
                inv_type: item.inv_type,
            });
        } else {
            change.copies.push(CopyItem {
                item: item.id,
                owner: if rules::library(inv, item.id) { inv.lib_owner } else { item.owner },
                parent: root,
                name: String::new(),
            });
        }
    }
    if !links.is_empty() {
        change.operations.push(Operation::Links { parent: root, links });
    }
    Ok((root, change))
}

/// What a step of a command asks the app to do.
enum Step {
    Keep,
    Done,
    Send(NetCommand),
    Appearance(crate::world::appearance::Action),
}

impl App {
    /// One step of the current folder command, once per frame.
    pub(super) fn welcome_wear_tick(&mut self, now: Instant) {
        expire(&mut self.welcome_wears, now);
        start_next(&mut self.welcome_wears, now);
        let Some(mut job) = self.welcome_wears.job.take() else {
            return;
        };
        // one outfit change at a time: a wear sent meanwhile would be dropped
        let busy = self.appearance_ui.pending.is_some();
        // fetch the subtree and the link targets until the job can go on,
        // and the Current Outfit folder the change rewrites (not restored at
        // a first login)
        if let Some(folder) = job_folder(&self.world.inventory, &job) {
            let cof = crate::world::appearance::cof(&self.world.inventory);
            for folder in std::iter::once(folder).chain(cof) {
                for command in self
                    .appearance_ui
                    .prepare_contents(&mut self.world.inventory, self.world.agent_id, folder)
                {
                    self.send(command);
                }
            }
        }
        let step = welcome_step(&mut self.world.inventory, &mut job, busy);
        match step {
            Step::Keep => self.welcome_wears.job = Some(job),
            Step::Done => self.welcome_wears.failed(job.command.action, job.wear),
            Step::Send(command) => {
                // the copy's budget starts now, not at the Library fetch
                job.since = now;
                self.welcome_wears.job = Some(job);
                self.send(command);
            }
            Step::Appearance(action) => self.apply_appearance_action(action),
        }
    }

    /// The copy's answers: the folders first, then (with items to copy) the
    /// items once the server confirmed them all.
    pub(super) fn welcome_wear_result(&mut self, request: Uuid, result: Result<Reply, String>) {
        if let Some(command) = copy_reply(&mut self.welcome_wears, &mut self.world.inventory, request, result, Instant::now()) {
            self.send(command);
        }
    }
}

/// The next queued command becomes the job, when there is none.
fn start_next(wears: &mut WelcomeWears, now: Instant) {
    if wears.job.is_some() {
        return;
    }
    let Some(Queued { command, wear }) = wears.queue.pop_front() else {
        return;
    };
    log::info!("avatar welcome pack: {:?} {:?} {:?}", command.action, command.name, command.id);
    wears.job = Some(Job {
        command,
        wear,
        since: now,
        stage: Stage::Find,
    });
}

/// Ends the job whose stage made no progress for a minute. A copy still on
/// its way is kept in `late`, so that its reply is finished (the folder is not
/// left empty) but never worn.
fn expire(wears: &mut WelcomeWears, now: Instant) {
    let Some(job) = wears.job.as_ref() else {
        return;
    };
    if now.saturating_duration_since(job.since) <= TIMEOUT {
        return;
    }
    log::warn!(
        "avatar welcome pack: {:?} {:?} made no progress for a minute in stage {:?}, outfit unchanged",
        job.command.action,
        job.command.name,
        job.stage
    );
    let Some(job) = wears.job.take() else {
        return;
    };
    wears.failed(job.command.action, job.wear);
    if let Stage::Copy { request, root } = job.stage {
        if wears.late.len() >= LATE_CAP {
            let (dropped, _) = wears.late.remove(0);
            log::warn!("avatar welcome pack: too many late copies, the reply of {dropped} will not be finished");
        }
        wears.late.push((request, root));
    }
}

/// Applies a reply of the copy to the inventory and returns the folder the
/// server made for `root`.
fn apply_reply(inv: &mut Inventory, reply: &Reply, root: Uuid) -> Uuid {
    inv.remove(&reply.removed);
    inv.apply(reply.contents.clone());
    inv.created_folders(reply.folders.clone());
    reply.created.get(&root).copied().unwrap_or(root)
}

/// One answer of the copy: returns the command to send, if any. A copy that
/// ends in error wears nothing and keeps the old outfit, as the Legacy Viewer
/// does when CopyLibraryCategory fails (wearInventoryCategoryOnAvatar returns
/// on a null category, indra/newview/llappearancemgr.cpp): the folder it made
/// stays in Clothing. An answer for another request is ignored.
fn copy_reply(
    wears: &mut WelcomeWears,
    inv: &mut Inventory,
    request: Uuid,
    result: Result<Reply, String>,
    now: Instant,
) -> Option<NetCommand> {
    if let Some(at) = wears.late.iter().position(|(r, _)| *r == request) {
        let root = wears.late[at].1;
        match result {
            Ok(reply) => {
                let folder = apply_reply(inv, &reply, root);
                if reply.copies.is_empty() {
                    wears.late.remove(at);
                    return None;
                }
                wears.late[at].1 = folder;
                return Some(NetCommand::FinishInventoryCopies { request, reply });
            }
            Err(reason) => {
                log::warn!("avatar welcome pack: late copy failed: {reason}");
                wears.late.remove(at);
                return None;
            }
        }
    }
    let job = wears.job.as_mut()?;
    let Stage::Copy { request: asked, root } = job.stage else {
        return None;
    };
    if asked != request {
        return None;
    }
    match result {
        Ok(reply) => {
            let folder = apply_reply(inv, &reply, root);
            job.since = now;
            if reply.copies.is_empty() {
                job.stage = Stage::Wear { folder };
                None
            } else {
                job.stage = Stage::Copy { request, root: folder };
                Some(NetCommand::FinishInventoryCopies { request, reply })
            }
        }
        Err(reason) => {
            log::warn!("avatar welcome pack: copy failed ({reason}), outfit unchanged");
            let (action, wear) = (job.command.action, job.wear);
            wears.job = None;
            wears.failed(action, wear);
            None
        }
    }
}

/// The agent's folder a job works on once it is in the agent's inventory:
/// the folder to take off, or the (copied) folder to wear.
/// The Current Outfit folder and its links' items are loaded: an outfit
/// change plans from them (`appearance::require_complete`), and the first
/// login does not fetch them (callAfterCOFFetch).
fn cof_loaded(inv: &Inventory) -> bool {
    crate::world::appearance::cof(inv).is_some_and(|cof| crate::world::appearance::complete(inv, cof))
}

fn job_folder(inv: &Inventory, job: &Job) -> Option<Uuid> {
    match job.stage {
        Stage::Find if job.command.action == FolderAction::Remove => folder_of(inv, &job.command),
        Stage::Wear { folder } => Some(folder),
        _ => None,
    }
}

/// The next step of `job` (LLWearFolderHandler / LLAddFolderHandler /
/// LLRemoveFolderHandler, wear_category).
fn welcome_step(inv: &mut Inventory, job: &mut Job, busy: bool) -> Step {
    match job.stage {
        Stage::Find => {
            let Some(folder) = folder_of(inv, &job.command) else {
                // wear_category: "Couldn't find … in the Library"
                log::warn!(
                    "avatar welcome pack: couldn't find {:?} {:?} in the inventory",
                    job.command.name,
                    job.command.id
                );
                return Step::Done;
            };
            if job.command.action == FolderAction::Remove {
                // the whole subtree and its link targets must have arrived
                // (wearable_contents is what the take-off plans from)
                return if busy || !cof_loaded(inv) || crate::world::appearance::wearable_contents(inv, folder).is_err() {
                    Step::Keep
                } else {
                    Step::Appearance(crate::world::appearance::Action::RemoveOutfit(folder))
                };
            }
            if !rules::library(inv, folder) {
                job.stage = Stage::Wear { folder };
                return Step::Keep;
            }
            if inv.folders.get(&folder).is_some_and(|f| f.state != FetchState::Fetched) {
                inv.request(folder);
                return Step::Keep;
            }
            let to = rules::system(inv, FT_CLOTHING).unwrap_or(inv.root);
            match copy_plan(inv, folder, to) {
                Ok((root, change)) => {
                    let request = Uuid::new_v4();
                    job.stage = Stage::Copy { request, root };
                    Step::Send(NetCommand::EditInventory { request, change })
                }
                Err(reason) => {
                    log::warn!("avatar welcome pack: copy not made: {reason}");
                    Step::Done
                }
            }
        }
        Stage::Copy { .. } => Step::Keep,
        Stage::Wear { folder } => {
            // recursive: sub-folders and the originals of their links, not
            // only the root folder (LLWearFolderHandler waits for the whole
            // category's descendants)
            let Ok(links) = crate::world::appearance::wearable_contents(inv, folder) else {
                return Step::Keep;
            };
            if links.is_empty() {
                log::warn!("avatar welcome pack: the folder holds nothing wearable, outfit unchanged");
                return Step::Done;
            }
            if busy || !cof_loaded(inv) {
                return Step::Keep;
            }
            let mode = match job.command.action {
                FolderAction::Add => WearMode::Append,
                _ => WearMode::ReplaceOutfit,
            };
            log::info!("avatar welcome pack: wearing the folder ({mode:?})");
            Step::Appearance(crate::world::appearance::Action::WearContents { folder, mode })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::Folder;
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn folder(inv: &mut Inventory, n: u128, parent: Uuid, name: &str, library: bool) {
        inv.folders.insert(
            id(n),
            Folder {
                info: InvFolder {
                    id: id(n),
                    parent,
                    name: name.into(),
                    type_default: -1,
                    version: 1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library,
            },
        );
        if let Some(p) = inv.folders.get_mut(&parent) {
            p.children.push(id(n));
        }
    }

    /// Library (10) › Clothing (11) › Female body - A (12) holding a shape
    /// (20) and a link (21); the agent's root (1) with Clothing (2) and the
    /// Current Outfit folder (3).
    fn inventory() -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            lib_root: id(10),
            lib_owner: id(99),
            ..Default::default()
        };
        folder(&mut inv, 1, Uuid::nil(), "My Inventory", false);
        folder(&mut inv, 10, Uuid::nil(), "Library", true);
        folder(&mut inv, 2, id(1), "Clothing", false);
        inv.folders.get_mut(&id(2)).expect("clothing").info.type_default = FT_CLOTHING;
        folder(&mut inv, 3, id(1), "Current Outfit", false);
        inv.folders.get_mut(&id(3)).expect("cof").info.type_default = crate::world::outfit::FT_CURRENT_OUTFIT;
        folder(&mut inv, 11, id(10), "Clothing", true);
        folder(&mut inv, 12, id(11), "Female body - A", true);
        folder(&mut inv, 13, id(12), "Spare", true);
        inv.add_items(vec![
            InvItem {
                id: id(20),
                parent: id(12),
                owner: id(99),
                name: "Shape".into(),
                asset_type: 13,
                inv_type: 18,
                ..Default::default()
            },
            InvItem {
                id: id(21),
                parent: id(12),
                name: "Hair".into(),
                asset_type: 24,
                asset_id: id(30),
                inv_type: 18,
                ..Default::default()
            },
        ]);
        inv.folders.get_mut(&id(12)).expect("body").items = vec![id(20), id(21)];
        inv
    }

    #[test]
    fn the_library_folder_is_found_by_its_name_then_the_id() {
        let inv = inventory();
        assert_eq!(library_folder(&inv, "female BODY - a"), Some(id(12)));
        assert_eq!(library_folder(&inv, "Male body - A"), None);
        // an agent's folder of the same name is not the Library's
        let named = |name: Option<&str>, folder: Option<Uuid>| FolderCommand {
            action: FolderAction::Wear,
            name: name.map(str::to_owned),
            id: folder,
        };
        assert_eq!(folder_of(&inv, &named(Some("Clothing"), None)), Some(id(11)));
        assert_eq!(folder_of(&inv, &named(Some("Nowhere"), Some(id(2)))), Some(id(2)));
        assert_eq!(folder_of(&inv, &named(None, Some(id(77)))), None);
    }

    #[test]
    fn a_library_folder_is_copied_without_its_subfolders() {
        let inv = inventory();
        let (root, change) = copy_plan(&inv, id(12), id(2)).expect("copy");
        assert!(
            matches!(&change.operations[0], Operation::Folder { id: folder, parent, name, type_default: -1 }
            if *folder == root && *parent == id(2) && name == "Female body - A")
        );
        assert!(matches!(&change.operations[1], Operation::Links { parent, links }
            if *parent == root && links.len() == 1 && links[0].target == id(30)));
        assert_eq!(change.operations.len(), 2, "the subfolder stays in the Library");
        assert!(matches!(change.copies.as_slice(), [CopyItem { item, owner, parent, .. }]
            if *item == id(20) && *owner == id(99) && *parent == root));
        // a folder still loading is not copied
        let mut loading = inventory();
        loading.folders.get_mut(&id(12)).expect("body").state = FetchState::Fetching;
        assert!(copy_plan(&loading, id(12), id(2)).is_err());
    }

    #[test]
    fn a_take_off_waits_for_the_subfolders_and_their_link_targets() {
        let mut inv = inventory();
        folder(&mut inv, 40, id(1), "Outfit", false);
        folder(&mut inv, 41, id(40), "Layers", false);
        // a link in the sub-folder whose original has not arrived
        inv.add_items(vec![InvItem {
            id: id(50),
            parent: id(41),
            name: "Jacket link".into(),
            asset_type: 24,
            asset_id: id(51),
            inv_type: 18,
            ..Default::default()
        }]);
        inv.folders.get_mut(&id(41)).expect("layers").items = vec![id(50)];
        let mut job = Job {
            command: FolderCommand {
                action: FolderAction::Remove,
                name: None,
                id: Some(id(40)),
            },
            wear: None,
            since: Instant::now(),
            stage: Stage::Find,
        };
        assert_eq!(job_folder(&inv, &job), Some(id(40)));
        assert!(matches!(welcome_step(&mut inv, &mut job, false), Step::Keep));
        // the sub-folder still loading
        inv.folders.get_mut(&id(41)).expect("layers").state = FetchState::Fetching;
        assert!(matches!(welcome_step(&mut inv, &mut job, false), Step::Keep));
        inv.folders.get_mut(&id(41)).expect("layers").state = FetchState::Fetched;
        // the original arrives
        inv.add_items(vec![InvItem {
            id: id(51),
            parent: id(2),
            owner: id(1),
            name: "Jacket".into(),
            asset_type: 5,
            inv_type: 18,
            ..Default::default()
        }]);
        assert!(matches!(welcome_step(&mut inv, &mut job, false), Step::Appearance(_)));
    }

    fn command(action: FolderAction) -> FolderCommand {
        FolderCommand {
            action,
            name: Some("x".into()),
            id: None,
        }
    }

    /// A reply that creates the copy's folder (asked 100, made 101).
    fn created_reply(copies: Vec<CopyItem>) -> Reply {
        Reply {
            moved_items: Vec::new(),
            moved_folders: Vec::new(),
            contents: Vec::new(),
            folders: vec![InvFolder {
                id: id(101),
                parent: id(2),
                name: "Female body - A".into(),
                type_default: -1,
                version: 1,
                ..Default::default()
            }],
            moves_queued: false,
            refresh: Vec::new(),
            copies,
            removed: Vec::new(),
            created: std::collections::HashMap::from([(id(100), id(101))]),
        }
    }

    fn one_copy() -> Vec<CopyItem> {
        vec![CopyItem {
            item: id(20),
            owner: id(99),
            parent: id(101),
            name: String::new(),
        }]
    }

    fn empty_reply() -> Reply {
        Reply {
            folders: Vec::new(),
            created: Default::default(),
            ..created_reply(Vec::new())
        }
    }

    /// A job of `action` copying folder 100, sent at `since`.
    fn copying_as(action: FolderAction, request: Uuid, since: Instant) -> WelcomeWears {
        WelcomeWears {
            job: Some(Job {
                command: command(action),
                wear: None,
                since,
                stage: Stage::Copy { request, root: id(100) },
            }),
            ..Default::default()
        }
    }

    fn copying(request: Uuid, since: Instant) -> WelcomeWears {
        copying_as(FolderAction::Wear, request, since)
    }

    /// A shape (60) worn through a link (61) in the copied folder 101.
    fn arrive(inv: &mut Inventory) {
        inv.folders.get_mut(&id(101)).expect("copy").state = FetchState::Fetched;
        inv.add_items(vec![
            InvItem {
                id: id(60),
                parent: id(2),
                owner: id(1),
                name: "Shape".into(),
                asset_type: 13,
                inv_type: 18,
                ..Default::default()
            },
            InvItem {
                id: id(61),
                parent: id(101),
                name: "Shape link".into(),
                asset_type: 24,
                asset_id: id(60),
                inv_type: 18,
                ..Default::default()
            },
        ]);
        inv.folders.get_mut(&id(101)).expect("copy").items = vec![id(61)];
    }

    fn wear_step(wears: &mut WelcomeWears, inv: &mut Inventory) -> Step {
        let job = wears.job.as_mut().expect("job");
        welcome_step(inv, job, false)
    }

    fn wearing(wears: &mut WelcomeWears, inv: &mut Inventory) -> bool {
        matches!(
            wear_step(wears, inv),
            Step::Appearance(crate::world::appearance::Action::WearContents { folder, .. }) if folder == id(101)
        )
    }

    #[test]
    fn a_copy_is_finished_then_worn_once_its_items_are_in() {
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let mut wears = copying(request, Instant::now());
        let sent = copy_reply(&mut wears, &mut inv, request, Ok(created_reply(one_copy())), Instant::now());
        assert!(matches!(sent, Some(NetCommand::FinishInventoryCopies { request: r, .. }) if r == request));
        assert!(matches!(wears.job.as_ref().map(|j| &j.stage), Some(Stage::Copy { root, .. }) if *root == id(101)));
        assert!(wears.owns(request));
        // the items are confirmed: the empty reply ends the copy
        assert!(copy_reply(&mut wears, &mut inv, request, Ok(empty_reply()), Instant::now()).is_none());
        assert!(matches!(wears.job.as_ref().map(|j| &j.stage), Some(Stage::Wear { folder }) if *folder == id(101)));
        assert!(!wears.owns(request));
        arrive(&mut inv);
        assert!(wearing(&mut wears, &mut inv));
    }

    #[test]
    fn a_copy_that_timed_out_is_still_finished_but_never_worn() {
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let since = Instant::now();
        let mut wears = copying(request, since);
        // not yet: the limit is a full minute
        expire(&mut wears, since + TIMEOUT);
        assert!(wears.job.is_some());
        expire(&mut wears, since + TIMEOUT + Duration::from_secs(1));
        assert!(wears.job.is_none());
        assert!(wears.owns(request), "its late reply is still ours");
        let sent = copy_reply(&mut wears, &mut inv, request, Ok(created_reply(one_copy())), Instant::now());
        assert!(matches!(sent, Some(NetCommand::FinishInventoryCopies { .. })));
        assert!(wears.job.is_none(), "nothing is worn");
        assert!(wears.owns(request), "the items' reply is still to come");
        assert!(copy_reply(&mut wears, &mut inv, request, Ok(empty_reply()), Instant::now()).is_none());
        assert!(!wears.owns(request) && wears.job.is_none());
        // an error drops the entry too
        let mut wears = copying(request, since);
        expire(&mut wears, since + TIMEOUT + Duration::from_secs(1));
        assert!(copy_reply(&mut wears, &mut inv, request, Err("no".into()), Instant::now()).is_none());
        assert!(!wears.owns(request));
    }

    #[test]
    fn a_timeout_after_the_folder_was_made_keeps_the_servers_folder() {
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let since = Instant::now();
        let mut wears = copying(request, since);
        copy_reply(&mut wears, &mut inv, request, Ok(created_reply(one_copy())), since);
        expire(&mut wears, since + TIMEOUT + Duration::from_secs(1));
        assert_eq!(wears.late, vec![(request, id(101))], "the folder the server made, not the asked id");
        assert!(wears.job.is_none());
    }

    #[test]
    fn the_late_copies_are_bounded_and_the_oldest_goes_first() {
        let mut wears = WelcomeWears::default();
        let since = Instant::now();
        let requests: Vec<Uuid> = (0..=LATE_CAP).map(|_| Uuid::new_v4()).collect();
        for request in &requests {
            wears.job = copying(*request, since).job;
            expire(&mut wears, since + TIMEOUT + Duration::from_secs(1));
        }
        assert_eq!(wears.late.len(), LATE_CAP);
        assert!(!wears.owns(requests[0]), "the oldest was forgotten");
        assert!(requests[1..].iter().all(|r| wears.owns(*r)));
    }

    #[test]
    fn a_stage_that_is_not_a_copy_times_out_without_a_late_entry() {
        let since = Instant::now();
        for stage in [Stage::Find, Stage::Wear { folder: id(101) }] {
            let mut wears = WelcomeWears {
                job: Some(Job {
                    command: command(FolderAction::Add),
                    wear: None,
                    since,
                    stage,
                }),
                ..Default::default()
            };
            expire(&mut wears, since + TIMEOUT + Duration::from_secs(1));
            assert!(wears.job.is_none());
            assert!(wears.late.is_empty());
        }
    }

    #[test]
    fn an_answer_for_another_request_is_ignored() {
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let mut wears = copying(request, Instant::now());
        let other = Uuid::new_v4();
        assert!(copy_reply(&mut wears, &mut inv, other, Ok(created_reply(one_copy())), Instant::now()).is_none());
        assert!(copy_reply(&mut wears, &mut inv, other, Err("no".into()), Instant::now()).is_none());
        assert!(
            matches!(wears.job.as_ref().map(|j| &j.stage), Some(Stage::Copy { request: r, root }) if *r == request && *root == id(100))
        );
        assert!(!inv.folders.contains_key(&id(101)), "nothing of it was applied");
    }

    #[test]
    fn a_failed_copy_wears_nothing_and_keeps_the_old_outfit() {
        let now = Instant::now();
        // no folder made
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let mut wears = copying(request, now);
        assert!(copy_reply(&mut wears, &mut inv, request, Err("refused".into()), now).is_none());
        assert!(wears.job.is_none() && !wears.owns(request));
        // the folder was made and some items arrived, then the copy failed
        let mut inv = inventory();
        let mut wears = copying(request, now);
        copy_reply(&mut wears, &mut inv, request, Ok(created_reply(one_copy())), now);
        arrive(&mut inv);
        assert!(copy_reply(&mut wears, &mut inv, request, Err("unconfirmed".into()), now).is_none());
        assert!(wears.job.is_none(), "no wear stage");
        assert!(inv.folders.contains_key(&id(101)), "the copied folder stays");
    }

    fn named(action: FolderAction, name: &str) -> FolderCommand {
        FolderCommand {
            action,
            name: Some(name.into()),
            id: None,
        }
    }

    /// Starts the next queued command and puts it at its copy.
    fn start_copy(wears: &mut WelcomeWears, request: Uuid, now: Instant) {
        start_next(wears, now);
        wears.job.as_mut().expect("job").stage = Stage::Copy { request, root: id(100) };
    }

    fn refuse(wears: &mut WelcomeWears, inv: &mut Inventory, request: Uuid, now: Instant) {
        assert!(copy_reply(wears, inv, request, Err("refused".into()), now).is_none());
        assert!(wears.job.is_none());
    }

    fn queued(wears: &WelcomeWears) -> Vec<(FolderAction, String)> {
        let pair = |q: &Queued| (q.command.action, q.command.name.clone().expect("name"));
        wears.queue.iter().map(pair).collect()
    }

    fn at(now: Instant, millis: u64) -> Instant {
        now + Duration::from_millis(millis)
    }

    #[test]
    fn a_failed_wear_folder_skips_its_own_add_folder_only() {
        let now = Instant::now();
        let mut inv = inventory();
        let request = Uuid::new_v4();

        // the page's add_folder is already queued when the copy fails
        let mut wears = WelcomeWears::default();
        wears.push(vec![named(FolderAction::Wear, "A")], now);
        start_copy(&mut wears, request, now);
        wears.push(vec![named(FolderAction::Add, "A")], at(now, 2500));
        refuse(&mut wears, &mut inv, request, at(now, 3000));
        assert!(queued(&wears).is_empty(), "the queued add_folder was skipped");
        // a later add_folder of no choice runs
        wears.push(vec![named(FolderAction::Add, "other")], at(now, 4000));
        assert_eq!(queued(&wears), [(FolderAction::Add, "other".into())]);

        // the copy fails before the page's add_folder comes
        let mut wears = WelcomeWears::default();
        wears.push(vec![named(FolderAction::Wear, "A")], now);
        start_copy(&mut wears, request, now);
        refuse(&mut wears, &mut inv, request, at(now, 1000));
        wears.push(vec![named(FolderAction::Add, "A")], at(now, 2500));
        assert!(queued(&wears).is_empty(), "skipped when it comes");
        wears.push(vec![named(FolderAction::Add, "other")], at(now, 3000));
        assert_eq!(queued(&wears).len(), 1, "one only");

        // a wear_folder that times out skips its add_folder too
        let mut wears = WelcomeWears::default();
        wears.push(vec![named(FolderAction::Wear, "A"), named(FolderAction::Add, "A")], now);
        start_copy(&mut wears, request, now);
        expire(&mut wears, now + TIMEOUT + Duration::from_secs(1));
        assert!(queued(&wears).is_empty());

        // a failed add_folder skips nothing
        let mut wears = WelcomeWears::default();
        wears.push(vec![named(FolderAction::Wear, "A"), named(FolderAction::Add, "A")], now);
        start_next(&mut wears, now);
        wears.job = None; // the body is worn
        start_copy(&mut wears, request, now);
        refuse(&mut wears, &mut inv, request, now);
        wears.push(vec![named(FolderAction::Wear, "B"), named(FolderAction::Add, "B")], now);
        assert_eq!(queued(&wears).len(), 2);
    }

    #[test]
    fn two_choices_in_a_row_skip_the_failed_ones_clothing_only() {
        // body A, body B a second later, then clothing A and clothing B
        let now = Instant::now();
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let mut wears = WelcomeWears::default();
        wears.push(vec![named(FolderAction::Wear, "A")], now);
        start_copy(&mut wears, request, now);
        wears.push(vec![named(FolderAction::Wear, "B")], at(now, 1000));
        refuse(&mut wears, &mut inv, request, at(now, 1500));
        start_next(&mut wears, at(now, 1500));
        assert_eq!(wears.job.as_ref().and_then(|j| j.command.name.clone()), Some("B".into()));
        wears.push(vec![named(FolderAction::Add, "A")], at(now, 2500));
        wears.push(vec![named(FolderAction::Add, "B")], at(now, 3500));
        assert_eq!(queued(&wears), [(FolderAction::Add, "B".into())], "A's clothing is not added to B");
    }

    #[test]
    fn a_choice_without_clothing_does_not_take_the_next_ones() {
        // body A fails and its page sends no add_folder; body B comes later
        let now = Instant::now();
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let mut wears = WelcomeWears::default();
        wears.push(vec![named(FolderAction::Wear, "A")], now);
        start_copy(&mut wears, request, now);
        refuse(&mut wears, &mut inv, request, at(now, 1000));
        let later = now + PAIR_WINDOW + Duration::from_secs(1);
        wears.push(vec![named(FolderAction::Wear, "B")], later);
        wears.push(vec![named(FolderAction::Add, "B")], at(later, 2500));
        assert_eq!(
            queued(&wears),
            [(FolderAction::Wear, "B".into()), (FolderAction::Add, "B".into())],
            "B's clothing is worn"
        );
    }

    #[test]
    fn a_body_dropped_from_a_full_queue_drops_its_clothing() {
        let now = Instant::now();
        let mut wears = WelcomeWears::default();
        wears.push((0..QUEUE_CAP).map(|n| named(FolderAction::Remove, &n.to_string())).collect(), now);
        wears.push(vec![named(FolderAction::Wear, "A")], now);
        start_next(&mut wears, now);
        assert_eq!(wears.queue.len(), QUEUE_CAP - 1);
        wears.push(vec![named(FolderAction::Add, "A")], at(now, 2500));
        assert_eq!(wears.queue.len(), QUEUE_CAP - 1, "A's clothing is not queued without its body");
    }

    #[test]
    fn a_wear_waits_for_the_current_outfit_folder() {
        // a first login: the Current Outfit folder is not loaded yet
        let mut inv = inventory();
        inv.folders.get_mut(&id(3)).expect("cof").state = FetchState::Fetching;
        let request = Uuid::new_v4();
        let mut wears = copying(request, Instant::now());
        copy_reply(&mut wears, &mut inv, request, Ok(created_reply(Vec::new())), Instant::now());
        arrive(&mut inv);
        assert!(matches!(wear_step(&mut wears, &mut inv), Step::Keep));
        inv.folders.get_mut(&id(3)).expect("cof").state = FetchState::Fetched;
        assert!(wearing(&mut wears, &mut inv));
    }

    #[test]
    fn a_new_accounts_outfit_is_worn_from_my_outfits_or_copied_from_the_library() {
        use crate::world::outfit::InitialOutfit;
        let mine = initial_command(InitialOutfit::Mine(id(40)));
        assert_eq!((mine.action, mine.name, mine.id), (FolderAction::Wear, None, Some(id(40))));
        let library = initial_command(InitialOutfit::Library("Female body - A".into()));
        assert_eq!(
            (library.action, library.name.as_deref(), library.id),
            (FolderAction::Wear, Some("Female body - A"), None)
        );
        // the Library one is copied first, then worn replacing the outfit
        let mut inv = inventory();
        let mut job = Job {
            command: library,
            wear: None,
            since: Instant::now(),
            stage: Stage::Find,
        };
        assert!(matches!(
            welcome_step(&mut inv, &mut job, false),
            Step::Send(NetCommand::EditInventory { .. })
        ));
    }

    #[test]
    fn the_folder_is_worn_replacing_or_added_by_the_command() {
        for (action, replaces) in [(FolderAction::Wear, true), (FolderAction::Add, false)] {
            let mut inv = inventory();
            let request = Uuid::new_v4();
            let mut wears = copying_as(action, request, Instant::now());
            copy_reply(&mut wears, &mut inv, request, Ok(created_reply(Vec::new())), Instant::now());
            arrive(&mut inv);
            let Step::Appearance(crate::world::appearance::Action::WearContents { mode, .. }) = wear_step(&mut wears, &mut inv) else {
                panic!("wearing");
            };
            assert_eq!(matches!(mode, WearMode::ReplaceOutfit), replaces);
            assert_eq!(matches!(mode, WearMode::Append), !replaces);
        }
    }

    #[test]
    fn commands_run_one_after_the_other() {
        let mut wears = WelcomeWears::default();
        let now = Instant::now();
        wears.push(vec![command(FolderAction::Wear), command(FolderAction::Add)], now);
        start_next(&mut wears, now);
        assert_eq!(wears.job.as_ref().map(|j| j.command.action), Some(FolderAction::Wear));
        start_next(&mut wears, now);
        assert_eq!(
            wears.job.as_ref().map(|j| j.command.action),
            Some(FolderAction::Wear),
            "the second waits"
        );
        wears.job = None;
        start_next(&mut wears, now);
        assert_eq!(wears.job.as_ref().map(|j| j.command.action), Some(FolderAction::Add));
        assert!(!wears.owns(Uuid::new_v4()));
    }

    #[test]
    fn the_queue_keeps_the_first_eight_in_order() {
        let mut wears = WelcomeWears::default();
        let now = Instant::now();
        let named = |n: usize| FolderCommand {
            action: FolderAction::Wear,
            name: Some(n.to_string()),
            id: None,
        };
        wears.push((0..6).map(named).collect(), now);
        wears.push((6..12).map(named).collect(), now);
        let kept: Vec<_> = wears.queue.iter().map(|q| q.command.name.clone().expect("name")).collect();
        assert_eq!(kept, ["0", "1", "2", "3", "4", "5", "6", "7"]);
    }
}
