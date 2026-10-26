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
    /// Since when the current walk sends requests, and how many (logged
    /// when it ends).
    busy: Option<(Instant, usize)>,
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
        }
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.in_flight.is_empty()
    }

    /// LLInventoryModelBackgroundFetch::bulkFetchViaAis: the reads to send
    /// this frame, never more than [`MAX_IN_FLIGHT`] at once.
    pub fn pump(&mut self, inv: &mut Inventory, agent: Uuid) -> Vec<NetCommand> {
        let mut out = Vec::new();
        let mut steps = 0;
        while self.in_flight.len() < MAX_IN_FLIGHT && steps < STEPS_PER_FRAME {
            let Some((id, kind)) = self.queue.pop_front() else {
                break;
            };
            steps += 1;
            self.step(inv, agent, id, kind, &mut out);
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

    fn step(&mut self, inv: &mut Inventory, agent: Uuid, id: Uuid, kind: Kind, out: &mut Vec<NetCommand>) {
        let Some(folder) = inv.folders.get(&id) else {
            return;
        };
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
