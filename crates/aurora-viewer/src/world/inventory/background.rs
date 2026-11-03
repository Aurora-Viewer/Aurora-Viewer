//! Fetch of the whole inventory in the background, from login on, so that
//! folders are loaded before they are opened.
//!
//! Port of LLInventoryModelBackgroundFetch's AIS walk (start, bulkFetchViaAis,
//! onAISFolderCalback, onAISContentCalback in
//! indra/newview/llinventorymodelbackgroundfetch.cpp, originally LGPL 2.1):
//! the inventory root alone, then its folders by batches of 20, each read
//! with its whole subtree; every reply is then walked again to fetch what the
//! depth limit or a failure left. The library is read from its root in one
//! recursive request. Folders opened by the user still go through
//! [`Inventory::request`], ahead of this queue.
//!
//! A grid without AIS (OpenSim) has only `FetchInventoryDescendents2`, which
//! reads one level whatever the depth asked: the walk then follows bulkFetch
//! instead, any [`LEGACY_BATCH`] queued folders per read, whatever their
//! parent, at most [`LEGACY_IN_FLIGHT`] reads at once.

use super::{FetchState, Inventory};
use aurora_net::NetCommand;
use aurora_net::inventory::ais::MAX_FOLDER_DEPTH;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;
use uuid::Uuid;

/// PoolSizeAIS (20) less the request it keeps for the user's own actions.
const MAX_IN_FLIGHT: usize = 19;
/// BatchSizeAIS3: folders per subset read.
const BATCH: usize = 20;
/// bulkFetch's max_batch_size: folders per `FetchInventoryDescendents2` read.
const LEGACY_BATCH: usize = 10;
/// bulkFetch's max_concurrent_fetches.
const LEGACY_IN_FLIGHT: usize = 12;
/// LLFolderType::FT_MARKETPLACE_LISTINGS, loaded by its own window.
const MARKETPLACE_LISTINGS: i32 = 53;
/// Queue entries looked at per frame (bulkFetch stops after 6 ms).
const STEPS_PER_FRAME: usize = 512;

/// LLInventoryModelBackgroundFetch::EFetchType.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// FT_RECURSIVE: the folder with its subtree, in one read.
    Recursive,
    /// FT_FOLDER_AND_CONTENT: the folder alone, then its folders.
    FolderAndContent,
    /// FT_CONTENT_RECURSIVE: the folder's unfetched folders, by batches.
    ContentRecursive,
}

struct Request {
    kind: Kind,
    folder: Uuid,
    /// The batch of a subset read; empty when `folder` itself was read.
    children: Vec<Uuid>,
}

#[derive(Default)]
pub struct BackgroundFetch {
    queue: VecDeque<(Uuid, Kind)>,
    in_flight: HashMap<u64, Request>,
    /// The folders those reads are fetching: their replies walk them.
    pending: HashSet<Uuid>,
    /// Folders the user's own reads are fetching, looked at again next frame.
    waiting: Vec<(Uuid, Kind)>,
    next_request: u64,
    started: bool,
    orphans_requested: bool,
    orphans_pending: Option<u64>,
    orphans_failed: bool,
    /// Since when the current walk sends requests, and how many (logged
    /// when it ends).
    busy: Option<(Instant, usize)>,
    /// The agent's region lacks InventoryAPIv3 / LibraryAPIv3: that tree is
    /// read level by level through the legacy capability.
    legacy: [bool; 2],
    /// The folders of each tree's next legacy read, sent when full or at the
    /// end of the frame.
    legacy_batch: [Vec<Uuid>; 2],
}

impl BackgroundFetch {
    /// LLInventoryModelBackgroundFetch::start(): the root alone first (it
    /// can be huge and its system folders are asked for on their own), the
    /// library with its subtree.
    pub fn start(&mut self, inv: &Inventory) {
        if self.started {
            return;
        }
        self.started = true;
        self.queue.push_front((inv.root, Kind::FolderAndContent));
        self.queue.push_back((inv.lib_root, Kind::Recursive));
    }

    /// Walk the tree again once the agent's region has its capabilities:
    /// reads sent on the region just left fail, and what they left is
    /// fetched then (backgroundFetch waits for capabilitiesReceived).
    pub fn resume(&mut self, inv: &Inventory) {
        if self.started {
            self.queue.push_back((inv.root, Kind::Recursive));
            self.queue.push_back((inv.lib_root, Kind::Recursive));
            if self.orphans_failed {
                self.orphans_requested = false;
            }
        }
    }

    /// Which trees the agent's region can only read without AIS
    /// (LLInventoryModelBackgroundFetch::backgroundFetch picks bulkFetch
    /// when AISAPI::isAvailable is false).
    pub fn set_legacy(&mut self, inventory: bool, library: bool) {
        self.legacy = [inventory, library];
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.in_flight.is_empty() && self.orphans_pending.is_none()
    }

    /// LLInventoryModelBackgroundFetch::bulkFetchViaAis: the reads to send
    /// this frame, never more than [`MAX_IN_FLIGHT`] at once.
    pub fn pump(&mut self, inv: &mut Inventory, agent: Uuid) -> Vec<NetCommand> {
        let mut out = Vec::new();
        // llstartup::STATE_INVENTORY_SEND calls findLostItems once per
        // session, including when the regular folders came from cache.
        if self.started && !self.orphans_requested {
            let request = self.next_request;
            self.next_request += 1;
            self.orphans_requested = true;
            self.orphans_pending = Some(request);
            self.orphans_failed = false;
            out.push(NetCommand::FetchInventoryOrphans { request });
        }
        let max = if self.legacy[0] { LEGACY_IN_FLIGHT } else { MAX_IN_FLIGHT };
        let mut steps = 0;
        while self.in_flight.len() + usize::from(self.orphans_pending.is_some()) < max && steps < STEPS_PER_FRAME {
            let Some((id, kind)) = self.queue.pop_front() else {
                break;
            };
            steps += 1;
            self.step(inv, agent, id, kind, &mut out);
        }
        for library in [false, true] {
            self.send_legacy(inv, agent, library, &mut out);
        }
        self.queue.extend(self.waiting.drain(..));
        if self.is_idle()
            && let Some((since, requests)) = self.busy.take()
        {
            let unfetched = inv.folders.values().filter(|f| f.state != FetchState::Fetched).count();
            log::info!(
                "inventory background fetch: {} folders and {} items in {:.1} s ({requests} requests, {unfetched} folders left)",
                inv.folders.len() - unfetched,
                inv.items.len(),
                since.elapsed().as_secs_f32()
            );
        }
        out
    }

    pub fn on_orphans_reply(&mut self, request: u64, success: bool) -> bool {
        if self.orphans_pending != Some(request) {
            return false;
        }
        self.orphans_pending = None;
        self.orphans_failed = !success;
        true
    }

    fn step(&mut self, inv: &mut Inventory, agent: Uuid, id: Uuid, kind: Kind, out: &mut Vec<NetCommand>) {
        let Some(folder) = inv.folders.get(&id) else {
            return;
        };
        if self.legacy[usize::from(folder.library)] {
            return self.step_legacy(inv, agent, id, kind, out);
        }
        match kind {
            Kind::Recursive | Kind::FolderAndContent => match folder.state {
                FetchState::Unknown | FetchState::Failed => {
                    let depth = if kind == Kind::Recursive { MAX_FOLDER_DEPTH } else { 0 };
                    self.send(inv, agent, kind, id, Vec::new(), depth, out);
                }
                FetchState::Fetching if self.pending.contains(&id) => {}
                // asked for by the user: walked once it has arrived
                FetchState::Fetching => self.waiting.push((id, kind)),
                // the folder alone is already there (the login's own read of
                // the root, or the cache): its folders by batches, as after
                // that read in onAISFolderCalback, not one read each
                FetchState::Fetched if kind == Kind::FolderAndContent => self.queue.push_front((id, Kind::ContentRecursive)),
                // already fetched: check what is inside
                FetchState::Fetched => {
                    let children = folder.children.clone();
                    self.queue.extend(children.into_iter().map(|child| (child, Kind::Recursive)));
                }
            },
            Kind::ContentRecursive => {
                let mut batch = Vec::new();
                let mut later = Vec::new();
                let mut done = true;
                for child in &folder.children {
                    let Some(c) = inv.folders.get(child) else {
                        continue;
                    };
                    match c.state {
                        _ if c.info.type_default == MARKETPLACE_LISTINGS => {}
                        FetchState::Unknown | FetchState::Failed if batch.len() < BATCH => batch.push(*child),
                        FetchState::Unknown | FetchState::Failed => done = false,
                        FetchState::Fetching if self.pending.contains(child) => {}
                        // a folder the user opened meanwhile is walked once
                        // it has arrived, like a fetched one
                        FetchState::Fetching | FetchState::Fetched => later.push(*child),
                    }
                }
                if !batch.is_empty() {
                    self.send(inv, agent, kind, id, batch, MAX_FOLDER_DEPTH, out);
                }
                if done {
                    self.queue.extend(later.into_iter().map(|child| (child, Kind::Recursive)));
                } else {
                    self.queue.push_back((id, Kind::ContentRecursive));
                }
            }
        }
    }

    /// bulkFetch's queue walk: unread folders join the next read, the
    /// folders of read ones are queued. Kinds keep their fallback meaning:
    /// [`Kind::Recursive`] is read in a batch, [`Kind::FolderAndContent`]
    /// alone after its batch failed, [`Kind::ContentRecursive`] only queues
    /// the known folders of one that failed alone too.
    fn step_legacy(&mut self, inv: &mut Inventory, agent: Uuid, id: Uuid, kind: Kind, out: &mut Vec<NetCommand>) {
        let Some(folder) = inv.folders.get(&id) else {
            return;
        };
        if folder.info.type_default == MARKETPLACE_LISTINGS {
            return;
        }
        let library = usize::from(folder.library);
        match (kind, folder.state) {
            (Kind::ContentRecursive, _) | (_, FetchState::Fetched) => {
                let children = folder.children.clone();
                self.queue.extend(children.into_iter().map(|child| (child, Kind::Recursive)));
            }
            (_, FetchState::Fetching) if self.pending.contains(&id) => {}
            (_, FetchState::Fetching) => self.waiting.push((id, kind)),
            (Kind::FolderAndContent, _) => self.send(inv, agent, kind, id, Vec::new(), 0, out),
            (Kind::Recursive, _) => {
                if !self.legacy_batch[library].contains(&id) {
                    self.legacy_batch[library].push(id);
                }
                if self.legacy_batch[library].len() >= LEGACY_BATCH {
                    self.send_legacy(inv, agent, library == 1, out);
                }
            }
        }
    }

    /// One read of the folders waiting in `library`'s legacy batch. Its
    /// first folder stands for the read (the legacy capability ignores it).
    fn send_legacy(&mut self, inv: &mut Inventory, agent: Uuid, library: bool, out: &mut Vec<NetCommand>) {
        let batch = std::mem::take(&mut self.legacy_batch[usize::from(library)]);
        let Some(&first) = batch.first() else {
            return;
        };
        self.send(inv, agent, Kind::Recursive, first, batch, 0, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn send(
        &mut self,
        inv: &mut Inventory,
        agent: Uuid,
        kind: Kind,
        folder: Uuid,
        children: Vec<Uuid>,
        depth: i32,
        out: &mut Vec<NetCommand>,
    ) {
        let library = inv.folders.get(&folder).is_some_and(|f| f.library);
        let targets = if children.is_empty() {
            std::slice::from_ref(&folder)
        } else {
            &children[..]
        };
        for id in targets {
            if let Some(f) = inv.folders.get_mut(id) {
                f.state = FetchState::Fetching;
            }
            self.pending.insert(*id);
        }
        let request = self.next_request;
        self.next_request += 1;
        log::debug!(
            "inventory background read {request}: {kind:?} {folder}, {} folders, depth {depth}",
            children.len()
        );
        out.push(NetCommand::FetchInventoryTree {
            request,
            parent: folder,
            children: children.clone(),
            depth,
            owner: if library { inv.lib_owner } else { agent },
            library,
        });
        self.in_flight.insert(request, Request { kind, folder, children });
        self.busy.get_or_insert_with(|| (Instant::now(), 0)).1 += 1;
    }

    /// onAISFolderCalback / onAISContentCalback, once the reply's snapshots
    /// are applied. A requested folder the reply did not bring failed.
    pub fn on_reply(&mut self, inv: &mut Inventory, request: u64) {
        let Some(Request { kind, folder, children }) = self.in_flight.remove(&request) else {
            return;
        };
        let targets = if children.is_empty() { vec![folder] } else { children };
        for id in targets {
            self.pending.remove(&id);
            let Some(state) = inv.folders.get(&id).map(|f| f.state) else {
                continue;
            };
            let fetched = state == FetchState::Fetched;
            if !fetched && state == FetchState::Fetching {
                inv.failed(&[id]);
            }
            match (kind, fetched) {
                // got the folder and its content: verify the content, in
                // case the depth limit or a failure left some of it
                (Kind::Recursive | Kind::ContentRecursive, true) => {
                    let children = inv.folders.get(&id).map(|f| f.children.clone()).unwrap_or_default();
                    self.queue.extend(children.into_iter().map(|child| (child, Kind::Recursive)));
                }
                // a full recursive read failed: the folder and its content
                // separately
                (Kind::Recursive, false) => self.queue.push_back((id, Kind::FolderAndContent)),
                (Kind::FolderAndContent, true) => self.queue.push_front((id, Kind::ContentRecursive)),
                // even the folder alone failed: its known folders
                (Kind::FolderAndContent, false) => self.queue.push_back((id, Kind::ContentRecursive)),
                // a batch failed: each folder on its own
                (Kind::ContentRecursive, false) => self.queue.push_back((id, Kind::Recursive)),
            }
        }
    }
}

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;
