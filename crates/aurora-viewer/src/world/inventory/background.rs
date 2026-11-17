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
use aurora_net::inventory::ais::{FetchError, FetchFailure, MAX_FOLDER_DEPTH};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;
use uuid::Uuid;

/// PoolSizeAIS (20) less the request it keeps for the user's own actions.
const MAX_IN_FLIGHT: usize = 19;
/// BatchSizeAIS3: folders per subset read.
const BATCH: usize = 20;
/// How deep a read goes while the walk is narrow (its queued and sent
/// reads fill fewer than half the slots): a huge inventory's first folders read 50
/// levels deep each held all the rest in one reply, near the timeout, one
/// slot of 19 in use. A few levels at a time, the next folders are known at
/// once and read side by side; once the slots are busy, reads go
/// `MAX_FOLDER_DEPTH` deep again, as the Legacy Viewer's always do. The
/// same reads, the same batches and pool: only their depth changes.
const NARROW_DEPTH: i32 = 3;
/// bulkFetch's max_batch_size: folders per `FetchInventoryDescendents2` read.
const LEGACY_BATCH: usize = 10;
/// bulkFetch's max_concurrent_fetches.
const LEGACY_IN_FLIGHT: usize = 12;
/// How long a read may stay unanswered before its slot is freed and it is
/// tried again like a throttled one: beyond the HTTP layer's own worst case
/// (ais.rs: a folder's 60 s, five times, with its backoff; a subtree read
/// has its own, [`TREE_DEADLINE`]). A reply that
/// never comes (a task lost, a connection dropped without an error) never
/// holds one of the pool's slots for the rest of the session.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(900);
/// A subtree read's deadline: one 180 s try (ais.rs `TREE_TIMEOUT`, no
/// second one) and a throttle's waits; past it the read is split.
const TREE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(240);
/// An idle walk with folders left unread looks at them again this often:
/// what a lost reply or a dropped retry left is asked for in the walk's own
/// batches, at its own pace.
const WATCHDOG: std::time::Duration = std::time::Duration::from_secs(30);
/// Folders the walk gave up on (failed after their retries) are asked for
/// again this often, a few times a session, never in a loop.
const FAILED_AGAIN: std::time::Duration = std::time::Duration::from_secs(600);
const FAILED_ROUNDS: u32 = 6;
/// LLFolderType::FT_MARKETPLACE_LISTINGS, loaded by its own window.
const MARKETPLACE_LISTINGS: i32 = 53;
/// Queue entries looked at per frame (bulkFetch stops after 6 ms).
const STEPS_PER_FRAME: usize = 512;
/// Reads the HTTP layer's own retries (ais.rs) left failed are asked again
/// later, so a throttled or briefly unavailable AIS server does not end the
/// walk with folders missing.
const BACKGROUND_RETRIES: u32 = 3;
const BACKGROUND_BACKOFF: [std::time::Duration; 3] = [
    std::time::Duration::from_secs(15),
    std::time::Duration::from_secs(60),
    std::time::Duration::from_secs(300),
];

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
    depth: i32,
    attempt: u32,
    /// When it was sent ([`READ_DEADLINE`]).
    sent: Instant,
}

/// A read waiting for its next attempt after a transient failure.
struct Retry {
    due: Instant,
    kind: Kind,
    folder: Uuid,
    children: Vec<Uuid>,
    depth: i32,
    attempt: u32,
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
    /// When the orphans request went: one never answered is given up after
    /// [`READ_DEADLINE`], never holding a slot for the session.
    orphans_sent: Option<Instant>,
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
    /// Reads waiting for their next attempt (a transient failure).
    retries: Vec<Retry>,
    /// The too-big folders not announced to the user yet.
    too_big_new: Vec<Uuid>,
    /// The first too-big folder has been announced (the Legacy Viewer's
    /// InventoryLimitReachedAISAlert is shown once).
    too_big_announced: bool,
    /// The HTTP status of the refusal that found the first over-limit folder.
    too_big_status: Option<u16>,
    /// The clock of the current pump: when its reads are sent.
    clock: Option<Instant>,
    /// When the idle walk last looked for folders left unread.
    swept: Option<Instant>,
    /// When the failed folders were last asked again, and how many times.
    failed_round: Option<Instant>,
    failed_rounds: u32,
    /// For the loading report (Dev Toolbox N4): the last reply, and how
    /// often the deadline and the watchdog fired.
    last_reply: Option<Instant>,
    expired_total: usize,
    watchdog_total: usize,
    /// Folders seen fetching with no read of this walk, and since when.
    orphaned: HashMap<Uuid, Instant>,
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

    /// Read what is left below these folders' parents, once the walk is
    /// idle (LLInventoryModelBackgroundFetch::start(cat_id, recursive), as
    /// Auto-sort's layout to share needs the whole tree): each parent's
    /// unread folders in the walk's own batches (20 subtrees a read, 19 in
    /// flight, Marketplace Listings left to its window), never one read a
    /// folder, never past a folder waiting for its retry.
    pub fn fetch_subtrees(&mut self, inv: &Inventory, folders: &[Uuid]) {
        if !self.started {
            self.start(inv);
        }
        if !self.is_idle() {
            return;
        }
        let mut parents: Vec<Uuid> = folders
            .iter()
            .filter_map(|id| inv.folders.get(id).map(|f| f.info.parent))
            .filter(|p| inv.folders.contains_key(p))
            .collect();
        parents.sort_unstable();
        parents.dedup();
        for parent in parents {
            self.queue.push_back((parent, Kind::ContentRecursive));
        }
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.in_flight.is_empty() && self.orphans_pending.is_none() && self.retries.is_empty()
    }

    /// The folders the server refuses even at depth 0, not announced yet,
    /// and the HTTP status of the refusal.
    pub fn take_too_big(&mut self) -> (Vec<Uuid>, Option<u16>) {
        (std::mem::take(&mut self.too_big_new), self.too_big_status.take())
    }

    /// Record folders the server refuses even at depth 0 (from the background
    /// walk or a folder the user opened) and announce the first one once.
    pub fn note_too_big(&mut self, inv: &mut Inventory, folders: &[Uuid], status: Option<u16>) {
        if self.too_big_status.is_none() {
            self.too_big_status = status;
        }
        for &id in folders {
            if inv.too_big.insert(id) {
                log::warn!("inventory folder {id} is over the server's limit; its direct items cannot be loaded");
            }
        }
        if !self.too_big_announced && !folders.is_empty() {
            self.too_big_announced = true;
            self.too_big_new.extend(folders);
        }
    }

    /// LLInventoryModelBackgroundFetch::bulkFetchViaAis: the reads to send
    /// this frame, never more than [`MAX_IN_FLIGHT`] at once.
    pub fn pump(&mut self, inv: &mut Inventory, agent: Uuid) -> Vec<NetCommand> {
        self.pump_at(inv, agent, Instant::now())
    }

    /// [`pump`] with the clock given (the tests drive the retries without
    /// waiting for the real backoff).
    pub fn pump_at(&mut self, inv: &mut Inventory, agent: Uuid, now: Instant) -> Vec<NetCommand> {
        let mut out = Vec::new();
        self.clock = Some(now);
        // a read unanswered past its deadline frees its slot and is tried
        // again like a throttled one
        let expired: Vec<u64> = self
            .in_flight
            .iter()
            .filter(|(_, r)| now.saturating_duration_since(r.sent) >= if r.depth > 0 { TREE_DEADLINE } else { READ_DEADLINE })
            .map(|(request, _)| *request)
            .collect();
        if !expired.is_empty() {
            log::warn!(
                "inventory background fetch: {} reads unanswered past their deadline ({} s a subtree, {} s a folder), asked again",
                expired.len(),
                TREE_DEADLINE.as_secs(),
                READ_DEADLINE.as_secs()
            );
            self.expired_total += expired.len();
            for request in expired {
                self.on_reply_at(inv, request, &[], Err(FetchError::new(FetchFailure::Transient, None)), now);
            }
        }
        // llstartup::STATE_INVENTORY_SEND calls findLostItems once per
        // session, including when the regular folders came from cache.
        if self.started && !self.orphans_requested {
            let request = self.next_request;
            self.next_request += 1;
            self.orphans_requested = true;
            self.orphans_pending = Some(request);
            self.orphans_sent = Some(now);
            self.orphans_failed = false;
            out.push(NetCommand::FetchInventoryOrphans { request });
        }
        let max = if self.legacy[0] { LEGACY_IN_FLIGHT } else { MAX_IN_FLIGHT };
        // Reads that failed transiently come back before new ones: they are
        // already known to the model.
        let mut retries = std::mem::take(&mut self.retries);
        let mut waiting = Vec::new();
        for retry in retries.drain(..) {
            if retry.due > now || self.in_flight.len() + usize::from(self.orphans_pending.is_some()) >= max {
                waiting.push(retry);
            } else {
                self.send(
                    inv,
                    agent,
                    retry.kind,
                    retry.folder,
                    retry.children,
                    retry.depth,
                    &mut out,
                    retry.attempt,
                );
            }
        }
        self.retries = waiting;
        if self.orphans_pending.is_some()
            && self
                .orphans_sent
                .is_some_and(|at| now.saturating_duration_since(at) >= READ_DEADLINE)
        {
            log::warn!("inventory background fetch: the lost items request was never answered, given up");
            self.orphans_pending = None;
            self.orphans_failed = true;
        }
        self.watch(inv, now);
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
        // (a folder no longer fetching keeps no old clock)
        self.orphaned
            .retain(|id, _| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetching));
        // A folder the user's own read holds (`waiting`) whose answer never
        // comes is read again after the deadline: it never keeps the walk
        // busy for the session.
        for (id, _) in &self.waiting {
            if inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetching) && !inv.queue.iter().any(|(q, _)| q == id) {
                let since = *self.orphaned.entry(*id).or_insert(now);
                if now.saturating_duration_since(since) >= READ_DEADLINE {
                    log::warn!(
                        "inventory background fetch: {id} asked by the user {} s ago never arrived, asked again",
                        READ_DEADLINE.as_secs()
                    );
                    if let Some(f) = inv.folders.get_mut(id) {
                        f.state = FetchState::Unknown;
                    }
                    self.orphaned.remove(id);
                }
            }
        }
        self.queue.extend(self.waiting.drain(..));
        if self.is_idle()
            && let Some((since, requests)) = self.busy.take()
        {
            let unfetched = inv.folders.values().filter(|f| f.state != FetchState::Fetched).count();
            log::info!(
                "inventory background fetch: {} folders and {} items in {:.1} s ({requests} requests, {unfetched} folders left, {} over the server's limit)",
                inv.folders.len() - unfetched,
                inv.items.len(),
                since.elapsed().as_secs_f32(),
                inv.too_big.len()
            );
        }
        out
    }

    /// The stall watchdog: an idle walk that left folders unread (a reply
    /// lost, a retry dropped) asks for them again in its own batches, each
    /// unread folder's parent read for its unfetched folders; the folders it
    /// gave up on are asked again a few times a session. Never a folder over
    /// the server's limit, nor Marketplace Listings.
    fn watch(&mut self, inv: &mut Inventory, now: Instant) {
        if !self.started || !self.is_idle() || self.swept.is_some_and(|at| now.saturating_duration_since(at) < WATCHDOG) {
            return;
        }
        self.swept = Some(now);
        let failed_due =
            self.failed_rounds < FAILED_ROUNDS && self.failed_round.is_none_or(|at| now.saturating_duration_since(at) >= FAILED_AGAIN);
        let mut parents: Vec<Uuid> = Vec::new();
        let mut failed: Vec<Uuid> = Vec::new();
        let mut lost: Vec<Uuid> = Vec::new();
        let mut stack = vec![inv.root, inv.lib_root];
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(f) = inv.folders.get(&id) else { continue };
            if f.info.type_default == MARKETPLACE_LISTINGS || inv.too_big.contains(&id) {
                continue;
            }
            match f.state {
                FetchState::Fetched => stack.extend(f.children.iter().copied()),
                FetchState::Unknown => {
                    if !parents.contains(&f.info.parent) {
                        parents.push(f.info.parent);
                    }
                }
                FetchState::Failed => failed.push(id),
                // (a read neither this walk holds nor the user's own queue:
                // its answer may be lost; once past the deadline it is read
                // again)
                FetchState::Fetching if !self.pending.contains(&id) && !inv.queue.iter().any(|(q, _)| *q == id) => {
                    let since = *self.orphaned.entry(id).or_insert(now);
                    if now.saturating_duration_since(since) >= READ_DEADLINE {
                        lost.push(id);
                    }
                }
                FetchState::Fetching => {}
            }
        }
        self.orphaned
            .retain(|id, _| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetching));
        if !lost.is_empty() {
            log::warn!(
                "inventory background fetch: {} folders asked {} s ago never arrived, asked again",
                lost.len(),
                READ_DEADLINE.as_secs()
            );
            for id in &lost {
                self.orphaned.remove(id);
                if let Some(f) = inv.folders.get_mut(id) {
                    f.state = FetchState::Unknown;
                }
            }
            self.watchdog_total += 1;
            self.queue.extend(lost.into_iter().map(|id| (id, Kind::FolderAndContent)));
        }
        // (the first walk ends on its own; a folder the user's own read
        // fails stays for the failed rounds)
        parents.retain(|p| inv.folders.get(p).is_some_and(|f| f.state == FetchState::Fetched));
        if !parents.is_empty() {
            self.watchdog_total += 1;
            log::warn!(
                "inventory background fetch: idle with unread folders under {} folders, asked again",
                parents.len()
            );
            self.queue.extend(parents.into_iter().map(|p| (p, Kind::ContentRecursive)));
        }
        if failed_due && !failed.is_empty() {
            self.failed_round = Some(now);
            self.failed_rounds += 1;
            self.watchdog_total += 1;
            log::warn!(
                "inventory background fetch: {} folders that failed asked again (round {} of {FAILED_ROUNDS})",
                failed.len(),
                self.failed_rounds
            );
            self.queue.extend(failed.into_iter().map(|id| (id, Kind::Recursive)));
        }
    }

    /// The loader's state in a few lines (Dev Toolbox N4): what a tester
    /// pastes when the inventory stops loading, no log needed.
    pub fn report(&self, inv: &Inventory, items_unreadable: usize, now: Instant) -> String {
        let mut counts = [0usize; 4];
        for f in inv.folders.values().filter(|f| !f.library) {
            counts[match f.state {
                FetchState::Fetched => 0,
                FetchState::Fetching => 1,
                FetchState::Failed => 2,
                FetchState::Unknown => 3,
            }] += 1;
        }
        let ago = |at: Option<Instant>| {
            at.map_or_else(
                || "never".to_owned(),
                |at| format!("{} s ago", now.saturating_duration_since(at).as_secs()),
            )
        };
        let oldest = self
            .in_flight
            .values()
            .map(|r| now.saturating_duration_since(r.sent).as_secs())
            .max();
        let next_retry = self.retries.iter().map(|r| r.due.saturating_duration_since(now).as_secs()).min();
        format!(
            "Inventory loading\n\
             folders: {} known, {} read, {} reading, {} failed, {} not read yet, {} over the server's limit\n\
             items: {} known, {items_unreadable} links' originals unreadable\n\
             walk: {}, {} reads in flight (oldest {}), {} queued, {} waiting for a retry (next in {}), {} more asked by the user\n\
             last answer: {}; unanswered reads freed: {}; watchdog restarts: {}; failed rounds: {} of {FAILED_ROUNDS}\n\
             AIS: inventory {}, library {}",
            counts.iter().sum::<usize>(),
            counts[0],
            counts[1],
            counts[2],
            counts[3],
            inv.too_big.len(),
            inv.items.len(),
            if !self.started {
                "not started"
            } else if self.is_idle() {
                "idle"
            } else {
                "busy"
            },
            self.in_flight.len(),
            oldest.map_or_else(|| "none".to_owned(), |s| format!("{s} s")),
            self.queue.len(),
            self.retries.len(),
            next_retry.map_or_else(|| "none".to_owned(), |s| format!("{s} s")),
            self.waiting.len(),
            ago(self.last_reply),
            self.expired_total,
            self.watchdog_total,
            self.failed_rounds,
            if self.legacy[0] { "no (legacy reads)" } else { "yes" },
            if self.legacy[1] { "no (legacy reads)" } else { "yes" },
        )
    }

    /// A subtree read's depth: [`NARROW_DEPTH`] while the walk is narrow.
    fn depth(&self) -> i32 {
        if self.queue.len() + self.in_flight.len() < MAX_IN_FLIGHT / 2 {
            NARROW_DEPTH
        } else {
            MAX_FOLDER_DEPTH
        }
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
        // A folder over the server's limit is not read again, but its known
        // subfolders are still walked.
        if inv.too_big.contains(&id) && kind != Kind::ContentRecursive {
            return;
        }
        if self.legacy[usize::from(folder.library)] {
            return self.step_legacy(inv, agent, id, kind, out);
        }
        match kind {
            Kind::Recursive | Kind::FolderAndContent => match folder.state {
                FetchState::Unknown | FetchState::Failed => {
                    let depth = if kind == Kind::Recursive { self.depth() } else { 0 };
                    self.send(inv, agent, kind, id, Vec::new(), depth, out, 0);
                }
                FetchState::Fetching if self.pending.contains(&id) => {}
                // asked for by the user: walked once it has arrived
                FetchState::Fetching => self.waiting.push((id, kind)),
                // the folder alone is already there (the login's own read of
                // the root, or the cache): its folders by batches, as after
                // that read in onAISFolderCalback, not one read each
                FetchState::Fetched if kind == Kind::FolderAndContent => self.queue.push_front((id, Kind::ContentRecursive)),
                // already fetched: check what is inside, its unread folders
                // by batches (a read cut by its depth left them)
                FetchState::Fetched => self.queue.push_back((id, Kind::ContentRecursive)),
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
                    let depth = self.depth();
                    self.send(inv, agent, kind, id, batch, depth, out, 0);
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
        if (inv.too_big.contains(&id) && kind != Kind::ContentRecursive) || folder.info.type_default == MARKETPLACE_LISTINGS {
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
            (Kind::FolderAndContent, _) => self.send(inv, agent, kind, id, Vec::new(), 0, out, 0),
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
        self.send(inv, agent, Kind::Recursive, first, batch, 0, out, 0);
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
        attempt: u32,
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
        self.in_flight.insert(
            request,
            Request {
                kind,
                folder,
                children,
                depth,
                attempt,
                sent: self.clock.unwrap_or_else(Instant::now),
            },
        );
        self.busy.get_or_insert_with(|| (Instant::now(), 0)).1 += 1;
    }

    /// onAISFolderCalback / onAISContentCalback, once the reply's snapshots
    /// are applied. `stale` are the folders `Inventory::apply` queued again;
    /// `outcome` is why the read failed, when it did.
    pub fn on_reply(&mut self, inv: &mut Inventory, request: u64, stale: &[Uuid], outcome: Result<(), FetchError>) {
        self.on_reply_at(inv, request, stale, outcome, Instant::now())
    }

    /// [`on_reply`] with the clock given (the tests drive the retries).
    pub fn on_reply_at(&mut self, inv: &mut Inventory, request: u64, stale: &[Uuid], outcome: Result<(), FetchError>, now: Instant) {
        let Some(Request {
            kind,
            folder,
            children,
            depth,
            attempt,
            ..
        }) = self.in_flight.remove(&request)
        else {
            return;
        };
        self.last_reply = Some(now);
        let single = children.is_empty();
        let targets = if single { vec![folder] } else { children.clone() };
        if let Err(error) = outcome {
            // A throttled or briefly unavailable server: ask again later
            // instead of ending the walk with this folder missing. The HTTP
            // layer already retried a few times (ais.rs).
            // (a read that timed out, no status, over a subtree is never
            // repeated as is: it is read in smaller parts at once, as
            // LLInventoryModelBackgroundFetch::onAISFolderCalback falls back
            // on a failed recursive read; only a refusal, 429 or 5xx, or a
            // folder alone waits for its retry)
            // (a gateway's timeout, 502 to 504, is a timeout too)
            // (a 503 is an overload: it waits for its retry like a 429)
            let timed_out =
                error.kind == FetchFailure::Transient && depth > 0 && error.status.is_none_or(|status| matches!(status, 502 | 504));
            if timed_out {
                log::warn!(
                    "background inventory read of {folder} timed out ({} folders, depth {depth}): read in parts",
                    targets.len()
                );
                for id in &targets {
                    self.pending.remove(id);
                    if let Some(f) = inv.folders.get_mut(id)
                        && f.state == FetchState::Fetching
                    {
                        f.state = FetchState::Unknown;
                    }
                }
                // (the folder alone, then its folders, each read the same way)
                self.queue.extend(targets.iter().map(|id| (*id, Kind::FolderAndContent)));
                return;
            }
            if error.kind == FetchFailure::Transient && attempt < BACKGROUND_RETRIES {
                for id in &targets {
                    self.pending.remove(id);
                    if let Some(f) = inv.folders.get_mut(id)
                        && f.state == FetchState::Fetching
                    {
                        f.state = FetchState::Unknown;
                    }
                }
                let delay = BACKGROUND_BACKOFF[attempt.min(BACKGROUND_BACKOFF.len() as u32 - 1) as usize];
                log::debug!(
                    "background inventory read of {folder} failed transiently; retry {} in {delay:?}",
                    attempt + 1
                );
                self.retries.push(Retry {
                    due: now + delay,
                    kind,
                    folder,
                    children,
                    depth,
                    attempt: attempt + 1,
                });
                return;
            }
            // A folder the server refuses even at depth 0 is over its reply
            // limit (the Legacy Viewer's InventoryLimitReachedAIS): record it,
            // announce it once, and still walk its subfolders.
            if error.kind == FetchFailure::TooBig && kind == Kind::FolderAndContent && single {
                self.note_too_big(inv, &[folder], error.status);
                for id in &targets {
                    self.pending.remove(id);
                }
                if let Some(f) = inv.folders.get_mut(&folder) {
                    f.state = FetchState::Failed;
                }
                self.queue.push_back((folder, Kind::ContentRecursive));
                return;
            }
        }
        for id in targets {
            self.pending.remove(&id);
            // Inventory::apply queued this stale folder again; it is being
            // read by the user path, so this reply must not fail it.
            if stale.contains(&id) {
                continue;
            }
            let Some(state) = inv.folders.get(&id).map(|f| f.state) else {
                continue;
            };
            let fetched = state == FetchState::Fetched;
            if !fetched && state == FetchState::Fetching {
                inv.failed(&[id]);
            }
            match (kind, fetched) {
                // got the folder and its content: verify the content, in
                // case the depth limit or a failure left some of it (its
                // unread folders by batches, never one read each)
                (Kind::Recursive | Kind::ContentRecursive, true) => self.queue.push_back((id, Kind::ContentRecursive)),
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
