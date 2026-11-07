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
//! clothing worn after it. A folder not found, a copy refused or not done
//! within a minute leaves the outfit as it is, with a line in the log, as
//! the Legacy Viewer does.

use super::*;
use crate::slapp::{FolderAction, FolderCommand};
use crate::world::appearance::WearMode;
use crate::world::inventory::{FetchState, Inventory, actions as rules};
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use std::collections::VecDeque;
use uuid::Uuid;

/// The Clothing system folder: the copies go under it.
const FT_CLOTHING: i32 = 5;
/// A command waits this long for its folder, its copy and the outfit
/// window, then gives up.
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct WelcomeWears {
    queue: VecDeque<FolderCommand>,
    job: Option<Job>,
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
    /// server's), `count` the items it will hold.
    Copy { request: Uuid, root: Uuid, count: usize },
    /// Waiting for `count` items in `folder`, then for the outfit window.
    Wear { folder: Uuid, count: usize },
}

impl WelcomeWears {
    pub fn push(&mut self, commands: Vec<FolderCommand>) {
        self.queue.extend(commands);
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.job
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
/// Returns the new folder's asked id, the change and the items it holds.
pub(super) fn copy_plan(inv: &Inventory, folder: Uuid, to: Uuid) -> Result<(Uuid, Mutation, usize), String> {
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
    let count = change.copies.len() + links.len();
    if !links.is_empty() {
        change.operations.push(Operation::Links { parent: root, links });
    }
    Ok((root, change, count))
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
        let mut job = match self.welcome_wears.job.take() {
            Some(job) => job,
            None => {
                let Some(command) = self.welcome_wears.queue.pop_front() else {
                    return;
                };
                log::info!("avatar welcome pack: {:?} {:?} {:?}", command.action, command.name, command.id);
                Job {
                    command,
                    since: now,
                    stage: Stage::Find,
                }
            }
        };
        if now.saturating_duration_since(job.since) > TIMEOUT {
            log::warn!(
                "avatar welcome pack: {:?} not done within a minute, outfit unchanged",
                job.command.name
            );
            return;
        }
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
    pub(super) fn welcome_wear_result(&mut self, request: Uuid, result: Result<aurora_net::inventory::operations::Reply, String>) {
        let Some(mut job) = self.welcome_wears.job.take() else {
            return;
        };
        let Stage::Copy { root, count, .. } = job.stage else {
            self.welcome_wears.job = Some(job);
            return;
        };
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents.clone());
                self.world.inventory.created_folders(reply.folders.clone());
                let folder = reply.created.get(&root).copied().unwrap_or(root);
                if reply.copies.is_empty() {
                    job.stage = Stage::Wear { folder, count };
                    self.welcome_wears.job = Some(job);
                } else {
                    job.stage = Stage::Copy {
                        request,
                        root: folder,
                        count,
                    };
                    self.welcome_wears.job = Some(job);
                    self.send(NetCommand::FinishInventoryCopies { request, reply });
                }
            }
            Err(reason) => log::warn!("avatar welcome pack: copy failed: {reason}"),
        }
    }
}

/// The agent's folder a job works on once it is in the agent's inventory:
/// the folder to take off, or the (copied) folder to wear.
fn job_folder(inv: &Inventory, job: &Job) -> Option<Uuid> {
    match job.stage {
        Stage::Find if job.command.action == FolderAction::Remove => folder_of(inv, &job.command),
        Stage::Wear { folder, .. } => Some(folder),
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
                job.stage = Stage::Wear { folder, count: 0 };
                return Step::Keep;
            }
            if inv.folders.get(&folder).is_some_and(|f| f.state != FetchState::Fetched) {
                inv.request(folder);
                return Step::Keep;
            }
            let to = rules::system(inv, FT_CLOTHING).unwrap_or(inv.root);
            match copy_plan(inv, folder, to) {
                Ok((root, change, count)) => {
                    let request = Uuid::new_v4();
                    job.stage = Stage::Copy { request, root, count };
                    Step::Send(NetCommand::EditInventory { request, change })
                }
                Err(reason) => {
                    log::warn!("avatar welcome pack: copy not made: {reason}");
                    Step::Done
                }
            }
        }
        Stage::Copy { .. } => Step::Keep,
        Stage::Wear { folder, count } => {
            let arrived = inv.folders.get(&folder).map_or(0, |f| f.items.len());
            // recursive: sub-folders and the originals of their links, not
            // only the root folder (LLWearFolderHandler waits for the whole
            // category's descendants)
            let complete = crate::world::appearance::wearable_contents(inv, folder).is_ok();
            if arrived >= count && complete && !busy {
                let mode = match job.command.action {
                    FolderAction::Add => WearMode::Append,
                    _ => WearMode::ReplaceOutfit,
                };
                log::info!("avatar welcome pack: wearing the folder ({mode:?})");
                Step::Appearance(crate::world::appearance::Action::WearContents { folder, mode })
            } else {
                Step::Keep
            }
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
        let (root, change, count) = copy_plan(&inv, id(12), id(2)).expect("copy");
        assert_eq!(count, 2);
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

    #[test]
    fn commands_run_one_after_the_other() {
        let mut wears = WelcomeWears::default();
        let command = |action| FolderCommand {
            action,
            name: Some("x".into()),
            id: None,
        };
        wears.push(vec![command(FolderAction::Wear), command(FolderAction::Add)]);
        assert_eq!(wears.queue.pop_front().map(|c| c.action), Some(FolderAction::Wear));
        assert_eq!(wears.queue.pop_front().map(|c| c.action), Some(FolderAction::Add));
        assert!(!wears.owns(Uuid::new_v4()));
    }
}
