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
//! clothing worn after it. A folder not found, or a copy that never made its
//! folder, leaves the outfit as it is, with a line in the log. A copy that
//! ends in error once its folder exists wears what arrived
//! (LLWearCategoryAfterCopy wears whatever the category holds); a command not
//! done within a minute is dropped, and its late copy is finished but never
//! worn. At most eight commands wait.

use super::*;
use crate::slapp::{FolderAction, FolderCommand};
use crate::world::appearance::{FT_CLOTHING, WearMode};
use crate::world::inventory::{FetchState, Inventory, actions as rules};
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation, Reply};
use aurora_net::outfits::OutfitLink;
use std::collections::VecDeque;
use uuid::Uuid;

/// A command waits this long for its folder, its copy and the outfit
/// window, then gives up.
const TIMEOUT: Duration = Duration::from_secs(60);
/// Once the copy is on its way, the network layer's own batch timer (also a
/// minute, started a little later) gets to end it first, so that what did
/// arrive is worn.
const COPY_GRACE: Duration = Duration::from_secs(10);
/// The page sends two commands a choice: eight is four choices in a row.
const QUEUE_CAP: usize = 8;

#[derive(Default)]
pub(super) struct WelcomeWears {
    queue: VecDeque<FolderCommand>,
    job: Option<Job>,
    /// Copies whose job timed out: `(request, folder)`. Their late reply is
    /// still applied and finished, like ContentsCopies' `timed_out`, so that
    /// the folder is not left empty; they never wear anything.
    late: Vec<(Uuid, Uuid)>,
}

struct Job {
    command: FolderCommand,
    since: Instant,
    stage: Stage,
}

enum Stage {
    /// Finding the folder, and loading a Library one before its copy.
    Find,
    /// The copy on its way: `root` is the new folder (the asked id, then the
    /// server's once `created`).
    Copy { request: Uuid, root: Uuid, created: bool },
    /// Waiting for the folder's links to resolve, then for the outfit window.
    Wear { folder: Uuid },
}

impl WelcomeWears {
    /// Queues the commands, at most `QUEUE_CAP` in all. The queue is not
    /// emptied when the window closes: that would lose a body's `add_folder`.
    pub fn push(&mut self, commands: Vec<FolderCommand>) {
        for command in commands {
            if self.queue.len() >= QUEUE_CAP {
                log::warn!("avatar welcome pack: too many folder commands queued, {:?} dropped", command.name);
                continue;
            }
            self.queue.push_back(command);
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
pub(super) fn library_folder(inv: &Inventory, name: &str) -> Option<Uuid> {
    let wanted = name.to_lowercase();
    let mut stack: Vec<Uuid> = inv.folders.get(&inv.lib_root)?.children.iter().rev().copied().collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else {
            continue;
        };
        if folder.info.name.to_lowercase() == wanted {
            return Some(id);
        }
        stack.extend(folder.children.iter().rev());
    }
    None
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
        // fetch the subtree and the link targets until the job can go on
        if let Some(folder) = job_folder(&self.world.inventory, &job) {
            for command in self
                .appearance_ui
                .prepare_contents(&mut self.world.inventory, self.world.agent_id, folder)
            {
                self.send(command);
            }
        }
        let step = welcome_step(&mut self.world.inventory, &mut job, busy);
        match step {
            Step::Keep => self.welcome_wears.job = Some(job),
            Step::Done => {}
            Step::Send(command) => {
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
    let Some(command) = wears.queue.pop_front() else {
        return;
    };
    log::info!("avatar welcome pack: {:?} {:?} {:?}", command.action, command.name, command.id);
    wears.job = Some(Job {
        command,
        since: now,
        stage: Stage::Find,
    });
}

/// Ends the job that waited too long. A copy still on its way is kept in
/// `late`, so that its reply is finished (the folder is not left empty) but
/// never worn.
fn expire(wears: &mut WelcomeWears, now: Instant) {
    let Some(job) = wears.job.as_ref() else {
        return;
    };
    let limit = match job.stage {
        Stage::Copy { created: true, .. } => TIMEOUT + COPY_GRACE,
        _ => TIMEOUT,
    };
    if now.saturating_duration_since(job.since) <= limit {
        return;
    }
    log::warn!(
        "avatar welcome pack: {:?} not done within a minute, outfit unchanged",
        job.command.name
    );
    if let Some(Job {
        stage: Stage::Copy { request, root, .. },
        ..
    }) = wears.job.take()
    {
        if wears.late.len() >= QUEUE_CAP {
            wears.late.remove(0);
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

/// One answer of the copy: returns the command to send, if any. Like the
/// Legacy Viewer's LLWearCategoryAfterCopy (indra/newview/llappearancemgr.cpp),
/// a copy that ends in error after its folder exists wears what the folder
/// holds; one with no folder, or one that timed out, wears nothing.
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
    let Stage::Copy {
        request: asked,
        root,
        created,
    } = job.stage
    else {
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
                job.stage = Stage::Copy {
                    request,
                    root: folder,
                    created: true,
                };
                Some(NetCommand::FinishInventoryCopies { request, reply })
            }
        }
        Err(reason) if created => {
            log::warn!("avatar welcome pack: copy incomplete ({reason}), wearing what arrived");
            job.stage = Stage::Wear { folder: root };
            job.since = now;
            None
        }
        Err(reason) => {
            log::warn!("avatar welcome pack: copy failed: {reason}");
            wears.job = None;
            None
        }
    }
}

/// The agent's folder a job works on once it is in the agent's inventory:
/// the folder to take off, or the (copied) folder to wear.
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
                return if busy || crate::world::appearance::wearable_contents(inv, folder).is_err() {
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
                    job.stage = Stage::Copy {
                        request,
                        root,
                        created: false,
                    };
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
            if busy {
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
    /// (20) and a link (21); the agent's root (1) with Clothing (2).
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

    /// A job copying folder 100, sent at `since`.
    fn copying(request: Uuid, since: Instant) -> WelcomeWears {
        WelcomeWears {
            job: Some(Job {
                command: command(FolderAction::Wear),
                since,
                stage: Stage::Copy {
                    request,
                    root: id(100),
                    created: false,
                },
            }),
            ..Default::default()
        }
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

    fn wearing(wears: &mut WelcomeWears, inv: &mut Inventory) -> bool {
        let job = wears.job.as_mut().expect("job");
        matches!(
            welcome_step(inv, job, false),
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
        assert!(matches!(wears.job.as_ref().map(|j| &j.stage), Some(Stage::Copy { root, created: true, .. }) if *root == id(101)));
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
    fn a_copy_that_made_no_folder_wears_nothing() {
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let mut wears = copying(request, Instant::now());
        assert!(copy_reply(&mut wears, &mut inv, request, Err("refused".into()), Instant::now()).is_none());
        assert!(wears.job.is_none() && !wears.owns(request));
    }

    #[test]
    fn what_arrived_of_a_failed_copy_is_worn() {
        let mut inv = inventory();
        let request = Uuid::new_v4();
        let mut wears = copying(request, Instant::now());
        copy_reply(&mut wears, &mut inv, request, Ok(created_reply(one_copy())), Instant::now());
        // the network layer reports the items it did get, then the failure
        arrive(&mut inv);
        let later = Instant::now() + Duration::from_secs(50);
        assert!(copy_reply(&mut wears, &mut inv, request, Err("unconfirmed".into()), later).is_none());
        assert!(wears.job.as_ref().is_some_and(|j| j.since == later), "the stage's timer restarts");
        assert!(wearing(&mut wears, &mut inv));
        // nothing wearable arrived: nothing is worn
        let mut inv = inventory();
        let mut wears = copying(request, Instant::now());
        copy_reply(&mut wears, &mut inv, request, Ok(created_reply(one_copy())), Instant::now());
        inv.folders.get_mut(&id(101)).expect("copy").state = FetchState::Fetched;
        copy_reply(&mut wears, &mut inv, request, Err("unconfirmed".into()), Instant::now());
        let job = wears.job.as_mut().expect("job");
        assert!(matches!(welcome_step(&mut inv, job, false), Step::Done));
    }

    #[test]
    fn commands_run_one_after_the_other() {
        let mut wears = WelcomeWears::default();
        wears.push(vec![command(FolderAction::Wear), command(FolderAction::Add)]);
        let now = Instant::now();
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
    fn the_queue_holds_eight_commands() {
        let mut wears = WelcomeWears::default();
        wears.push((0..6).map(|_| command(FolderAction::Wear)).collect());
        wears.push((0..6).map(|_| command(FolderAction::Add)).collect());
        assert_eq!(wears.queue.len(), QUEUE_CAP);
        assert_eq!(
            wears.queue.iter().filter(|c| c.action == FolderAction::Add).count(),
            2,
            "the excess is dropped"
        );
    }
}
