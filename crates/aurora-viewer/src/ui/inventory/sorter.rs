//! The Auto-sort window: the rules of `world::inventory::sorter`, a preview of
//! where every entry goes, the user's own rules, and the sort itself, sent as
//! bounded inventory mutations one after another through the ordinary
//! pending edit (like Bulk permissions), so that nothing else races it.
//!
//! It opens three ways: the Auto-sort button of the inventory window (on the
//! folder shown or selected, Objects otherwise), a folder's right-click (its
//! contents) and an entry's right-click, which sorts the selected entries at
//! once. "Whole inventory" reads My Inventory and its system folders, and
//! asks twice before it moves anything. Pre-Trash rules collect entries in
//! a Pre-Trash folder; they reach the Trash only from its own confirmation,
//! through the ordinary delete path.

use super::*;
use crate::world::inventory::sorter::{self, Age, CleanupPlan, Find, Kind, Match, Move, Plan, Rule, Step};
use std::collections::VecDeque;
use std::time::Instant;

/// Entries sent to the Trash in one delete request.
const TRASH_BATCH: usize = 200;

/// What the window sorts.
#[derive(Debug, Clone, PartialEq)]
enum Source {
    /// The direct contents of a folder.
    Folder(Uuid),
    /// Entries chosen in the inventory, sorted once their preview is seen.
    Entries(Vec<Uuid>),
    /// My Inventory's loose items, Objects and the user's own folders
    /// (`sorter::whole_sources`), confirmed before it starts.
    Whole,
    /// Saved moves: Undo, a snapshot's layout, a run to resume.
    Saved(std::sync::Arc<Vec<Move>>, Restoring),
}

/// Where saved moves come from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Restoring {
    Undo,
    Snapshot(i64),
    Resume,
}

/// The time now, in seconds since 1970.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A confirmation on top of the window.
#[derive(Debug, Clone, PartialEq)]
enum Confirm {
    Whole,
    /// Pre-Trash's entries and the folder they wait in.
    /// Shared, never copied per frame while the confirmation is open.
    PreTrash(std::sync::Arc<[(Uuid, Uuid)]>),
    /// Clean up's entries, straight to the Trash.
    Trash(std::sync::Arc<[(Uuid, Uuid)]>),
    /// Imported settings replacing the user's rules and store hints.
    Replace,
    /// Allow Auto-sort into a default folder (its id and name), or Lost
    /// and Found's switch (None).
    Unlock(Option<Uuid>, String),
    /// Empty Pre-Trash or the Trash for good in small steps: the folder,
    /// its name and the entries it holds.
    Purge(Uuid, String, usize),
}

/// The folder picker: for the folder to sort or for a rule's destination.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pick {
    Source,
    Rule(usize),
    Exclude,
    Own,
}

struct Picker {
    pick: Pick,
    filter: String,
    /// Every eligible folder, its path and the path in lowercase, built
    /// again after an inventory change at most once a second.
    rows: Vec<(Uuid, String, String)>,
    generation: u64,
    built: Option<Instant>,
}

/// The layout saved to the Desktop to share (`Save my inventory layout`).
enum Share {
    /// The whole inventory is read first: since when, how far, and when it
    /// last moved on.
    Loading {
        since: Instant,
        progress: sorter::ReadProgress,
        moved: Instant,
        /// Saved now with what is read (`Save what's loaded now`).
        now: bool,
        /// When the progress was last read (twice a second, never each
        /// frame: two walks of 40,000 folders).
        checked: Option<Instant>,
    },
    /// Written on another thread.
    Saving(std::sync::mpsc::Receiver<Result<(std::path::PathBuf, usize), String>>),
    /// Its path, and how many folders could not be read.
    Saved(std::path::PathBuf, usize),
    Failed(String),
}

#[derive(Default)]
pub(super) struct State {
    /// The layout saved to share, on its way or done.
    shared: Option<Share>,
    source: Option<Source>,
    open: bool,
    tab: usize,
    /// The preview, kept until the inventory or the options change.
    preview: Option<(u64, sorter::Preferences, Plan)>,
    job: Option<Job>,
    /// A batch is on its way; its reply comes through `finish_sorter`.
    submitted: bool,
    status: String,
    /// Start once the preview is ready: a right-click on entries, or
    /// `AURORA_DEMO_INVENTORY=sorter-run`.
    autostart: bool,
    /// The toolbar button: the folder is chosen where the inventory is known.
    requested: Option<Uuid>,
    picker: Option<Picker>,
    confirm: Option<Confirm>,
    /// The whole-inventory confirmation's "I understand" box.
    understood: bool,
    /// Clean up's list, kept until the inventory, its choices, the source or
    /// the day change.
    cleanup: Option<(u64, sorter::Cleanup, Uuid, i64, CleanupPlan)>,
    /// The excluded folders Clean up's list was made with: an exclusion
    /// since makes it again.
    cleanup_excluded: Vec<Uuid>,
    /// My folders when Clean up's list was made.
    cleanup_own: Vec<Uuid>,
    /// Clean up's rows unchecked: they stay.
    skipped: HashSet<Uuid>,
    /// Clean up's entries on their way to the Trash, a batch at a time.
    trash: Option<TrashJob>,
    /// Pre-Trash or the Trash emptied for good in small steps.
    purge: Option<PurgeJob>,
    /// A Clean up button pressed this frame.
    cleanup_action: Option<CleanupAction>,
    /// The Share window and what is ticked in it.
    share: Option<sorter::share::Share>,
    /// The Import window.
    import: Option<ImportWindow>,
    /// `AURORA_DEMO_INVENTORY=sorter-vocab`: the golden corpus sorted with
    /// the vocabulary in use, expected against result.
    vocab_check: Option<Vec<sorter::vocab::CorpusRow>>,
    /// The account whose journal, snapshots and unfinished run are loaded.
    account: Option<Uuid>,
    /// The last runs' confirmed moves, for Undo (saved per account).
    journal: SharedJournal,
    /// A run the viewer stopped before its end, to resume.
    unfinished: Option<sorter::journal::Unfinished>,
    /// That run being read off the frame (a stopped 400,000-move run is a
    /// large file), when the window first opens.
    unfinished_loading: Option<std::sync::mpsc::Receiver<Option<sorter::journal::Unfinished>>>,
    snapshots: Vec<(i64, std::path::PathBuf)>,
    /// Batches confirmed since the journal was last written.
    unsaved: usize,
    /// Batches are held: a teleport runs.
    held: bool,
    /// The inventory's generation last seen, and since when it has not changed.
    seen_generation: u64,
    quiet_since: Option<Instant>,
    /// When the preview and Clean up's list were last made.
    planned_at: Option<Instant>,
    cleaned_at: Option<Instant>,
    /// The preview waits for the inventory to settle before it is made again.
    updating: bool,
    /// Folders the plans would read, asked for a few at a time.
    to_request: Vec<Uuid>,
    /// What waits in Pre-Trash and where, at an inventory generation.
    waiting: Option<(u64, std::sync::Arc<Vec<Uuid>>, Option<Uuid>)>,
    /// Whether Pre-Trash is loaded with all it holds, at a generation.
    pre_trash_loaded: Option<(u64, bool)>,
    /// Empty in small steps, per folder: read at that generation, when, all
    /// it holds loaded, and its entries.
    purge_seen: std::collections::HashMap<Uuid, (u64, Instant, bool, usize)>,
    /// When a saved source's folders were last looked at while loading.
    source_checked: Option<Instant>,
    /// The preview and Clean up's list being made off the frame, for a
    /// large inventory.
    planning: Option<Planning<(Plan, sorter::journal::Snapshot)>>,
    /// The layout the last background plan read, at its inventory
    /// generation: a run that starts on that same inventory saves it as its
    /// snapshot, nothing taken on the frame.
    layout: Option<(u64, sorter::journal::Snapshot)>,
    /// The Undo button's run (its date and size), at an inventory generation
    /// and a journal version: searched again only when either changes.
    undo_label: Option<((u64, (u64, usize, usize)), Option<(i64, usize)>)>,
    /// Undo was pressed while the folders its runs put entries in were
    /// still loading: it opens its preview once they are (checked again
    /// when the inventory changes from this generation).
    undo_waiting: Option<u64>,
    /// The folders Undo waits for (gathered once at the click).
    undo_folders: Vec<Uuid>,
    /// What is worn and locked now, for Undo and a snapshot's restore (the
    /// Options tab is drawn without them): an entry that cannot go back is
    /// never what Undo picks.
    worn_now: HashSet<Uuid>,
    protected_now: HashSet<Uuid>,
    /// A snapshot being read off the frame (a whole inventory's layout is a
    /// large file): its date and the file once read.
    snapshot_loading: Option<(
        i64,
        std::path::PathBuf,
        std::sync::mpsc::Receiver<std::io::Result<sorter::journal::Snapshot>>,
    )>,
    /// Whether all a saved or whole-inventory source reads is loaded, at an
    /// inventory generation: checked again only when the inventory changes.
    source_loaded: Option<(u64, bool)>,
    /// The exclusions `source_loaded` was worked out with.
    source_excluded: Vec<Uuid>,
    cleaning: Option<Planning<CleanupPlan>>,
    /// The run an Undo puts back (its id in the journal).
    undoing: Option<u64>,
    /// This window, for the account's one run at a time.
    window: Uuid,
    /// When the journal was last written: written again every few seconds
    /// while a run confirms moves (a crash loses little).
    saved_at: Option<Instant>,
    /// A sort job ended this frame: its save removes the run to resume.
    /// No other save (a Trash run's, another window's) touches that file.
    job_just_ended: bool,
    /// The rules and hints the Share window's boxes were ticked for: when
    /// they change while it is open, its boxes are unticked again.
    shared_for: Option<(Vec<Rule>, Vec<sorter::StoreHint>)>,
}

/// A plan made on another thread from a copy of the inventory
/// (`sorter::snapshot`): what it was made for, and where it arrives.
struct Planning<T> {
    generation: u64,
    options: sorter::Preferences,
    source: Source,
    cleanup: Option<(sorter::Cleanup, Uuid, i64)>,
    result: std::sync::mpsc::Receiver<T>,
}

impl<T: Send + 'static> Planning<T> {
    /// Copy what the plan reads and make it on another thread. The copy is
    /// the only work left on the frame (about 60 ms for 400,000 items,
    /// against about a second for the plan itself).
    fn spawn(
        inv: &Inventory,
        options: &sorter::Preferences,
        source: &Source,
        cleanup: Option<(sorter::Cleanup, Uuid, i64)>,
        make: impl FnOnce(&Inventory) -> T + Send + 'static,
    ) -> Self {
        // one copy per inventory change, shared by every plan made of it (the
        // preview's and Clean up's, every window's): a 400,000-item
        // inventory is copied once, not once per planner. Held only while a
        // planner reads it.
        type Shared = Option<(usize, u64, std::sync::Weak<Inventory>)>;
        static LAST: std::sync::Mutex<Shared> = std::sync::Mutex::new(None);
        let key = (std::ptr::from_ref(inv) as usize, inv.generation);
        let copy = {
            let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match last
                .as_ref()
                .filter(|(at, generation, _)| (*at, *generation) == key)
                .and_then(|(_, _, copy)| copy.upgrade())
            {
                Some(copy) => copy,
                None => {
                    let copy = std::sync::Arc::new(sorter::snapshot(inv));
                    *last = Some((key.0, key.1, std::sync::Arc::downgrade(&copy)));
                    copy
                }
            }
        };
        let (send, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // the window may be gone by now: nobody to tell
            let _ = send.send(make(&copy));
        });
        Self {
            generation: inv.generation,
            options: options.clone(),
            source: source.clone(),
            cleanup,
            result,
        }
    }

    /// The plan once it is made; None while it is being made. A planner
    /// that died (a bug) is dropped and logged, and the next frame plans
    /// again.
    fn take(slot: &mut Option<Self>) -> Option<(Self, T)> {
        let task = slot.as_ref()?;
        match task.result.try_recv() {
            Ok(plan) => slot.take().map(|task| (task, plan)),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                log::warn!("auto-sort: a background plan stopped without a result");
                *slot = None;
                None
            }
        }
    }
}

/// The journal of an account's runs, one for every inventory window: a run
/// in one window and Undo in another see the same runs, and a save never
/// writes one window's older copy over another's.
#[derive(Clone, Default)]
struct SharedJournal(
    std::sync::Arc<std::sync::Mutex<sorter::journal::Journal>>,
    /// The window whose run or Trash run is going, and when it last said so:
    /// one at a time per account, so two windows never mix their runs.
    std::sync::Arc<std::sync::Mutex<Option<(Uuid, Instant)>>>,
    /// Changed since its file was last written: a save queued while an
    /// earlier one waits is done by that one (it reads the journal when it
    /// runs), never twice.
    std::sync::Arc<std::sync::atomic::AtomicBool>,
);

impl SharedJournal {
    /// The account's journal, read from its file the first time, on a
    /// thread of its own: until it is read the journal is `loading`.
    fn of(account: Uuid, read: impl FnOnce() -> sorter::journal::Journal + Send + 'static) -> Self {
        static JOURNALS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<Uuid, SharedJournal>>> = std::sync::OnceLock::new();
        let journals = JOURNALS.get_or_init(Default::default);
        let mut journals = journals.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        journals
            .entry(account)
            .or_insert_with(|| {
                let shared = Self(
                    std::sync::Arc::new(std::sync::Mutex::new(sorter::journal::Journal {
                        loading: true,
                        read_only: true,
                        ..Default::default()
                    })),
                    Default::default(),
                    Default::default(),
                );
                let slot = shared.clone();
                let load = move || {
                    // a reader that fails badly (a bug) never leaves the
                    // history loading for the whole session
                    let mut journal = std::panic::catch_unwind(std::panic::AssertUnwindSafe(read)).unwrap_or_else(|_| {
                        log::warn!("auto-sort: the history could not be read: Undo starts empty");
                        sorter::journal::Journal {
                            read_only: true,
                            ..Default::default()
                        }
                    });
                    journal.number_runs();
                    slot.with(|j| *j = journal);
                };
                // read at once in the tests, which use the journal right away
                #[cfg(test)]
                load();
                #[cfg(not(test))]
                if let Err(error) = std::thread::Builder::new().name("auto-sort history".into()).spawn(load) {
                    log::warn!("auto-sort: the history could not be read ({error}): Undo starts empty");
                    shared.with(|j| j.loading = false);
                }
                shared
            })
            .clone()
    }

    fn runner(&self) -> std::sync::MutexGuard<'_, Option<(Uuid, Instant)>> {
        self.1.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Another window's run is going on this account.
    fn busy_elsewhere(&self, window: Uuid) -> bool {
        self.runner()
            .is_some_and(|(other, seen)| other != window && seen.elapsed() < RUNNER_LIMIT)
    }

    /// This window's run is going (said every frame while it is).
    fn claim(&self, window: Uuid) {
        let mut runner = self.runner();
        if runner.is_none_or(|(other, seen)| other == window || seen.elapsed() >= RUNNER_LIMIT) {
            *runner = Some((window, Instant::now()));
        }
    }

    fn release(&self, window: Uuid) {
        let mut runner = self.runner();
        if runner.is_some_and(|(other, _)| other == window) {
            *runner = None;
        }
    }

    #[cfg(test)]
    fn holds(&self, window: Uuid) -> bool {
        self.runner().is_some_and(|(other, _)| other == window)
    }

    /// What the writer saves: a copy taken on the writer's thread, never
    /// on the frame (a large run's journal is hundreds of thousands of
    /// entries), and only when it changed since the last save. A journal
    /// whose file could not be read is never written over it.
    fn for_save(&self) -> Option<sorter::journal::Journal> {
        if !self.2.swap(false, std::sync::atomic::Ordering::AcqRel) {
            return None;
        }
        self.with(|j| (!j.read_only).then(|| j.clone()))
    }

    fn with<R>(&self, f: impl FnOnce(&mut sorter::journal::Journal) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    fn record_in(&self, run: Option<u64>, entries: impl IntoIterator<Item = sorter::journal::Entry>) {
        if let Some(run) = run {
            self.with(|j| j.record_in(run, entries));
        }
    }

    fn begin(&self, kind: sorter::journal::RunKind, at: i64) -> u64 {
        self.with(|j| j.begin(kind, at))
    }
}

/// A window that stopped saying its run is going (closed, gone) for this
/// long no longer holds the account's journal.
const RUNNER_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Open Undo's preview: the newest run with something that can go back. A
/// run whose entries all stay (moved on by hand, in a place locked since)
/// never blocks an older one.
fn start_undo(inv: &Inventory, state: &mut State, worn: &HashSet<Uuid>, protected: &HashSet<Uuid>) {
    let found = state.journal.with(|j| {
        let mut stayed = 0;
        for run in j.runs.iter().rev().filter(|run| run.undoes.is_none() && !run.entries.is_empty()) {
            let (moves, skipped) = sorter::journal::undo_moves(inv, run, protected, worn);
            if !moves.is_empty() {
                return Some((run.id, moves, skipped + stayed));
            }
            stayed += skipped;
        }
        None
    });
    match found {
        Some((run, moves, skipped)) => {
            state.undoing = Some(run);
            state.set_source(Source::Saved(std::sync::Arc::new(moves), Restoring::Undo));
            state.tab = PREVIEW;
            if skipped > 0 {
                state.status = aurora_i18n::tr!("inventory-sorter-undo-stays", count = skipped);
            }
        }
        None => state.status = aurora_i18n::tr!("inventory-sorter-undo-nothing").into(),
    }
}

/// Ask for the folders not loaded yet among `ids`, and say whether they all
/// are. One the server failed to give counts as read (what it holds stays
/// where it is) and is never asked for again in a loop.
fn ask_for(inv: &mut Inventory, ids: impl IntoIterator<Item = Uuid>) -> bool {
    let mut unknown = Vec::new();
    let mut reading = 0;
    for id in ids {
        match inv.folders.get(&id).map(|f| f.state) {
            Some(FetchState::Unknown) => unknown.push(id),
            Some(FetchState::Fetching) => reading += 1,
            Some(FetchState::Fetched | FetchState::Failed) | None => {}
        }
    }
    // a few at a time: a deep tree is read as its folders answer (each
    // answer changes the inventory, and this is asked again), never
    // thousands of requests at once
    let loaded = unknown.is_empty() && reading == 0;
    for id in unknown.into_iter().take(REQUESTS_AT_ONCE.saturating_sub(reading)) {
        inv.request(id);
    }
    loaded
}

/// Wait (a short while at most) for Auto-sort's files queued so far to be
/// written: at quit, after the last flush.
pub(crate) fn wait_for_writes(limit: std::time::Duration) {
    let (done, wait) = std::sync::mpsc::channel();
    in_order(move || {
        let _ = done.send(());
    });
    let _ = wait.recv_timeout(limit);
}

/// Auto-sort's files (journal, run to resume, snapshots) are written by one
/// thread, one after the other: two saves never write the same file at
/// once, and a newer one is never overtaken by an older one.
fn in_order(job: impl FnOnce() + Send + 'static) {
    type Job = Box<dyn FnOnce() + Send>;
    static WRITER: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<Job>>> = std::sync::OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (send, jobs) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("auto-sort files".into())
            .spawn(move || {
                for job in jobs {
                    job();
                }
            })
            .map_err(|e| log::warn!("auto-sort: no writer thread ({e}): files are written on the frame"))
            .ok();
        std::sync::Mutex::new(send)
    });
    let sent = writer.lock().map(|send| send.send(Box::new(job)));
    match sent {
        Ok(Ok(())) => {}
        // no thread (it could not start): written here, late rather than never
        Ok(Err(std::sync::mpsc::SendError(job))) => job(),
        Err(_) => log::warn!("auto-sort: the writer is unavailable: a save was skipped"),
    }
}

/// From this many folders and items on, plans are made off the frame; a
/// smaller inventory plans in a few milliseconds, at once.
const PLAN_IN_BACKGROUND: usize = 20_000;

/// The inventory unchanged this long before a plan is made again.
const QUIET: std::time::Duration = std::time::Duration::from_millis(600);
/// A plan made again at most this often while the inventory changes.
const REPLAN: std::time::Duration = std::time::Duration::from_secs(2);
/// Folder requests the plans keep in the inventory's queue at most.
const REQUESTS_AT_ONCE: usize = 16;

/// The Import window: the pasted text, what it holds and what is ticked.
#[derive(Default)]
struct ImportWindow {
    text: String,
    /// The text `found` was read from.
    read: String,
    found: sorter::share::Imported,
    pick: sorter::share::Pick,
    done: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CleanupAction {
    PreTrash,
    Trash,
}

/// Empty in small steps: the pieces left, the one on its way and its
/// answer. Paced, one request at a time; a refused piece is split smaller.
struct PurgeJob {
    folder: Uuid,
    pieces: VecDeque<sorter::purge::Piece>,
    total: usize,
    done: usize,
    failed: usize,
    /// The piece on its way, the entries it held, frames since it was
    /// pushed, and since when the app took it.
    in_flight: Option<(sorter::purge::Piece, usize, u32, Option<Instant>)>,
    request: Option<Uuid>,
    /// Its answer: accepted or refused.
    answered: Option<bool>,
    paused: bool,
    next_at: Option<Instant>,
    /// Stop was pressed: nothing more is sent, not even a refused piece's
    /// smaller parts.
    stopping: bool,
    /// Pieces refused in a row: past [`PURGE_REFUSALS`] the purge stops.
    refused_in_row: u32,
}

/// Pieces refused in a row before a purge stops (a server or a network
/// that refuses everything is never asked again and again).
const PURGE_REFUSALS: u32 = 20;

/// A pause between two pieces: the server is never asked faster.
const PURGE_GAP: std::time::Duration = std::time::Duration::from_millis(400);
/// A piece taken and never answered is given up after this.
const PURGE_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// The next piece of a purge, or the answer of the one on its way.
fn send_purge_piece(inv: &Inventory, st: &mut InventoryUi, worn: &HashSet<Uuid>, actions: &mut Vec<InvAction>, free: bool) {
    use aurora_net::inventory::operations::{Mutation, Operation};
    let free = free
        && st.pending.is_none()
        && !actions
            .iter()
            .any(|a| matches!(a, InvAction::Edit(_) | InvAction::Delete(_) | InvAction::Drop(_)));
    let now = Instant::now();
    let Some(job) = st.sorter.purge.as_mut() else { return };
    if let Some((piece, count, frames, taken)) = job.in_flight.as_mut() {
        *frames += 1;
        if job.request.is_some() {
            taken.get_or_insert(now);
        }
        let answer = match job.answered {
            Some(answer) => Some(answer),
            // never taken (another change went first): asked again
            None if job.request.is_none() && *frames > 120 => {
                let piece = piece.clone();
                if !job.stopping {
                    job.pieces.push_front(piece);
                }
                job.in_flight = None;
                return;
            }
            // taken and never answered: refused
            None if taken.is_some_and(|at| now.duration_since(at) > PURGE_LIMIT) => Some(false),
            None => None,
        };
        let Some(accepted) = answer else { return };
        let (piece, count) = (piece.clone(), *count);
        job.in_flight = None;
        job.request = None;
        job.answered = None;
        job.next_at = Some(now + PURGE_GAP);
        if accepted {
            job.done += count;
            job.refused_in_row = 0;
            return;
        }
        job.refused_in_row += 1;
        match sorter::purge::split_piece(inv, &piece) {
            // refused (a 403 for too large a piece): smaller, at once
            Some(parts) if !job.stopping => {
                for part in parts.into_iter().rev() {
                    job.pieces.push_front(part);
                }
            }
            _ => job.failed += count,
        }
        if job.refused_in_row >= PURGE_REFUSALS {
            job.stopping = true;
            job.pieces.clear();
        }
        return;
    }
    if job.pieces.is_empty() {
        // (what is left is what the folder still holds, a folder never read
        // and what holds it included: never an entry counted twice)
        job.failed = sorter::purge::entries_below(inv, job.folder);
        st.sorter.status = if job.refused_in_row >= PURGE_REFUSALS {
            aurora_i18n::tr!("inventory-purge-refused", count = job.done, failed = job.failed)
        } else {
            aurora_i18n::tr!("inventory-purge-done", count = job.done, failed = job.failed)
        };
        st.sorter.purge = None;
        st.sorter.journal.release(st.sorter.window);
        return;
    }
    if job.paused || !free || job.next_at.is_some_and(|at| now < at) {
        return;
    }
    // Checked again now, never only when planned: what is gone, moved out
    // of the folder being emptied (by hand, by an Undo) or worn since is
    // skipped, never deleted.
    let root = job.folder;
    let inside = |id: Uuid| id != root && rules::under(inv, id, root);
    let holds_worn = |id: Uuid| worn.iter().any(|w| *w == id || rules::under(inv, *w, id));
    let piece = loop {
        let Some(piece) = job.pieces.pop_front() else { return };
        let piece = match piece {
            sorter::purge::Piece::Folder(id) | sorter::purge::Piece::Emptied(id)
                if inv.folders.contains_key(&id)
                    && inside(id)
                    && !holds_worn(id)
                    // (never a folder holding one never read: split instead)
                    && !sorter::purge::holds_blind(inv, id) =>
            {
                piece
            }
            sorter::purge::Piece::Items(ids) => {
                let ids: Vec<Uuid> = ids
                    .into_iter()
                    // (never an item of a folder not read now: a refresh
                    // since may have moved or changed it)
                    .filter(|id| {
                        // (the folder being emptied excepted: one over the
                        // server's limit gives up what is known of it)
                        inv.items
                            .get(id)
                            .is_some_and(|it| it.parent == root || !sorter::purge::blind(inv, it.parent))
                            && inside(*id)
                            && !worn.contains(id)
                    })
                    .collect();
                if ids.is_empty() {
                    continue;
                }
                sorter::purge::Piece::Items(ids)
            }
            // (a folder holding one not read now, a refresh meanwhile: its
            // seen parts go, smaller; never what was not read)
            sorter::purge::Piece::Folder(id)
                if inv.folders.contains_key(&id) && inside(id) && !holds_worn(id) && !sorter::purge::blind(inv, id) =>
            {
                if let Some(parts) = sorter::purge::split_piece(inv, &piece) {
                    for part in parts.into_iter().rev() {
                        job.pieces.push_front(part);
                    }
                }
                continue;
            }
            sorter::purge::Piece::Folder(_) | sorter::purge::Piece::Emptied(_) => continue,
        };
        break piece;
    };
    let (operations, removed): (Vec<Operation>, Vec<Uuid>) = match &piece {
        sorter::purge::Piece::Folder(id) | sorter::purge::Piece::Emptied(id) => {
            (vec![Operation::Delete { id: *id, folder: true }], vec![*id])
        }
        sorter::purge::Piece::Items(ids) => (
            ids.iter().map(|id| Operation::Delete { id: *id, folder: false }).collect(),
            ids.clone(),
        ),
    };
    let count = piece.entries(inv);
    // (no read-back of the folder being emptied: it is the one too large
    // to read; the delete's own answer settles the piece)
    actions.push(InvAction::Edit(Mutation {
        operations,
        refresh: Vec::new(),
        removed,
        ..Default::default()
    }));
    job.in_flight = Some((piece, count, 0, None));
}

struct TrashJob {
    /// Each entry and the folder the confirmed list saw it in.
    queue: VecDeque<(Uuid, Uuid)>,
    /// Entries dropped because they changed since the confirmation.
    skipped: usize,
    total: usize,
    /// Entries the delete requests carried, counted once each one is done.
    sent: usize,
    /// Entries a finished delete request left where they were.
    failed: usize,
    /// The batch on its way: its entries, frames since it was pushed, and
    /// whether the app took it (a delete pending).
    in_flight: Option<(Vec<(Uuid, Uuid)>, u32, bool)>,
    next_at: Option<Instant>,
    /// The journal and the snapshot of its start are taken.
    begun: bool,
    /// Entries of a finished delete request not seen in the Trash yet, and
    /// until when they are waited for (the inventory may show the move a
    /// little after the request is over).
    unsure: Vec<(Uuid, Uuid, Instant)>,
    /// The inventory generation `unsure` was last read at: read again only
    /// when it changed or an entry's wait is over.
    unsure_seen: Option<u64>,
    /// Its run in the journal.
    run: Option<u64>,
    /// Since when the batch on its way has been taken and its delete still
    /// pending: a delete that never ends does not hold the run forever.
    taken_at: Option<Instant>,
    /// The request that took the batch on its way, and whether it was
    /// answered (`finish_sorter`).
    request: Option<Uuid>,
    answered: bool,
    /// Entries of a refused or unconfirmed batch to show back where they
    /// were (the inventory shows a move as soon as it is sent).
    put_back: Vec<(Uuid, Uuid)>,
}

impl TrashJob {
    fn new(ids: Vec<(Uuid, Uuid)>) -> Self {
        Self {
            total: ids.len(),
            queue: ids.into(),
            sent: 0,
            failed: 0,
            skipped: 0,
            in_flight: None,
            next_at: None,
            begun: false,
            unsure: Vec::new(),
            unsure_seen: None,
            run: None,
            taken_at: None,
            request: None,
            answered: false,
            put_back: Vec::new(),
        }
    }
}

/// How long the moves of an unanswered or refused batch may still turn up
/// at their destination and count as done.
const UNSURE_LIMIT: std::time::Duration = std::time::Duration::from_secs(180);
/// How long a Trash batch's entries may take to show in the Trash.
const TRASH_SHOW_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The time between two batches of a job, after the server's reply: about
/// 200 moves a second at most, so a 400,000-item sort neither floods the
/// simulator nor the viewer's own inventory updates.
const PACE: std::time::Duration = std::time::Duration::from_secs(1);
/// A refused batch is tried again this many times, after 2, 4 and 8 s.
const RETRIES: u32 = 3;
/// A destination folder that does not load within this time: its move is
/// skipped and the job goes on.
const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A folder moved whole waits this long for its subfolders to load (a deep
/// tree of a large inventory loads level by level).
const LOADING_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// A sort or a move to Pre-Trash, sent as paced batches: one at a time,
/// counted only once the server confirmed it.
struct Job {
    queue: VecDeque<Move>,
    base: String,
    total: usize,
    /// Moves the server confirmed.
    done: usize,
    /// Entries dropped because they changed since the preview (moved,
    /// worn, deleted, now locked): never sent, reported at the end.
    skipped: usize,
    /// Moves still refused after their retries: they stayed where they were.
    failed: usize,
    /// The batch on its way, and the request it was sent as.
    in_flight: Option<InFlight>,
    retries: u32,
    /// No batch before this: the pace between batches, or a retry's wait.
    next_at: Option<Instant>,
    paused: bool,
    started: Instant,
    /// Since when the next move waits for its destination to load.
    waiting_since: Option<Instant>,
    /// Entries of a refused batch to show back in the folder they were in
    /// (id, folder, that folder): the server did not say where they are.
    put_back: Vec<(Uuid, bool, Uuid)>,
    /// The inventory generation `unsure` was last read at.
    settled_at: Option<u64>,
    /// The move that wait is for, and whether for its own subfolders: a
    /// new move or another kind of wait starts the clock again.
    waiting_for: Option<(Uuid, bool)>,
    /// Moves confirmed since the journal last took them.
    confirmed: Vec<sorter::journal::Entry>,
    /// Undo, a snapshot or a resumed run: what it ends.
    restoring: Option<Restoring>,
    /// The journal and the snapshot of its start are taken.
    begun: bool,
    /// Stop was pressed: the batch on its way ends the run.
    stopping: bool,
    /// The moves of a refused or unanswered batch, and whether they were
    /// counted as failed (else they went back to the queue): the server may
    /// still have applied them, late or in part. Each one found where it
    /// was sent counts as done and goes in the journal, for Undo.
    unsure: Vec<(Move, bool, Instant)>,
    /// Its run in the journal.
    run: Option<u64>,
    /// An Undo: the run it undoes, and the runs of the same Undo begun in
    /// an earlier session (stopped by the viewer, then resumed).
    undoes: Option<u64>,
    earlier_runs: Vec<u64>,
    /// The sort folder's id when the run began, for a resume.
    base_id: Option<Uuid>,
    /// How many moves the run-to-resume file held when last written (None:
    /// not written yet). It is written again only once the run shrank by a
    /// half: a large run's whole queue is not copied every few seconds,
    /// and a resume drops the moves already done.
    unfinished_len: Option<usize>,
}

struct InFlight {
    moves: Vec<Move>,
    /// The app's request id, set when it takes the batch
    /// (claim_sorter_request); a batch it never takes goes back in the
    /// queue.
    request: Option<Uuid>,
    frames: u32,
    /// When it left; an answer that never comes (a lost connection) times
    /// out and the batch is tried again, re-checked entry by entry.
    sent_at: Instant,
}

/// A batch with no answer after this is tried again.
const ANSWER_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

impl Job {
    fn new(queue: VecDeque<Move>, base: String) -> Self {
        Self {
            total: queue.len(),
            queue,
            base,
            done: 0,
            skipped: 0,
            failed: 0,
            in_flight: None,
            retries: 0,
            next_at: None,
            paused: false,
            started: Instant::now(),
            waiting_since: None,
            confirmed: Vec::new(),
            restoring: None,
            begun: false,
            stopping: false,
            unsure: Vec::new(),
            run: None,
            undoes: None,
            earlier_runs: Vec::new(),
            base_id: None,
            waiting_for: None,
            put_back: Vec::new(),
            settled_at: None,
            unfinished_len: None,
        }
    }

    /// The next move waits (`subtree`: for its own subfolders, else for its
    /// destination): the clock starts again for another move or wait.
    fn wait_for(&mut self, subtree: bool) {
        let key = self.queue.front().map(|m| (m.id, subtree));
        if self.waiting_for != key {
            self.waiting_for = key;
            self.waiting_since = None;
        }
    }

    /// Put a batch's moves back at the front of the queue, in order.
    fn requeue(&mut self, moves: Vec<Move>) {
        for m in moves.into_iter().rev() {
            self.queue.push_front(m);
        }
    }

    /// The server's answer to the batch on its way.
    /// `unread`: the moves the answer gives no place for; None (no answer,
    /// or a request that failed) is every move of the batch.
    fn finish(&mut self, error: Option<&str>, unread: Option<&[Uuid]>, now: Instant) -> Option<String> {
        let batch = self.in_flight.take()?;
        // the inventory moved them when they were sent: until the server
        // shows where they are, they are shown back where they were, so
        // nothing is counted from where it was only sent
        if error.is_some() {
            self.put_back.extend(
                batch
                    .moves
                    .iter()
                    .filter(|m| unread.is_none_or(|ids| ids.contains(&m.id)))
                    .map(|m| (m.id, m.folder, m.from)),
            );
        }
        if let Some(error) = error {
            // for a tester's report (Dev Toolbox J3): what was refused, why,
            // and which attempt; no name, no id
            let folders = batch.moves.iter().filter(|m| m.folder).count();
            log::warn!(
                "auto-sort: a batch of {} moves ({} folders) was refused, attempt {}: {error}",
                batch.moves.len(),
                folders,
                self.retries + 1
            );
        }
        match error {
            None => {
                // a batch that went through on a retry: no longer unsure
                self.unsure.retain(|(m, _, _)| !batch.moves.iter().any(|b| b.id == m.id));
                self.done += batch.moves.len();
                self.confirmed.extend(batch.moves.iter().map(|m| sorter::journal::Entry {
                    id: m.id,
                    folder: m.folder,
                    from: m.from,
                    name: m.name.clone(),
                    to: None,
                }));
                self.retries = 0;
                self.next_at = Some(now + PACE);
                None
            }
            Some(error) if self.retries < RETRIES && !self.stopping => {
                self.retries += 1;
                self.mark_unsure(&batch.moves, false, now);
                self.requeue(batch.moves);
                self.next_at = Some(now + std::time::Duration::from_secs(1 << self.retries));
                Some(aurora_i18n::tr!("inventory-sorter-retrying", error = error, attempt = self.retries))
            }
            Some(error) => {
                // never the whole job for one refused batch: it stays put
                self.failed += batch.moves.len();
                self.mark_unsure(&batch.moves, true, now);
                self.retries = 0;
                self.next_at = Some(now + PACE);
                Some(error.to_owned())
            }
        }
    }

    /// The moves of failed batches that happened after all (the server
    /// applied them late, or in part): done and journaled, out of the queue
    /// or of the failed count. The rest stay unsure until the job ends.
    fn settle(&mut self, inv: &Inventory) {
        // read again only when the inventory changed (an arrival is one)
        if self.unsure.is_empty() || self.settled_at == Some(inv.generation) {
            return;
        }
        self.settled_at = Some(inv.generation);
        let base = self.base.clone();
        let flying: HashSet<Uuid> = self.in_flight.iter().flat_map(|b| b.moves.iter().map(|m| m.id)).collect();
        let mut arrived = Vec::new();
        self.unsure.retain(|(m, failed, since)| {
            // the server answers late, not minutes later: past that, an
            // entry found at its destination was moved there by hand
            if since.elapsed() > UNSURE_LIMIT {
                return false;
            }
            // on its way again: its own answer counts it
            if flying.contains(&m.id) {
                return true;
            }
            let here = sorter::arrived(inv, &base, m);
            if here {
                arrived.push((m.clone(), *failed));
            }
            !here
        });
        // a requeued move waits at the front of the queue: only the front is
        // looked at, never the whole queue once per arrival
        let requeued: HashSet<Uuid> = arrived.iter().filter(|(_, failed)| !failed).map(|(m, _)| m.id).collect();
        if !requeued.is_empty() {
            let front = self.queue.len().min(sorter::BATCH * 4);
            let at: Vec<usize> = (0..front).filter(|&i| requeued.contains(&self.queue[i].id)).collect();
            for i in at.into_iter().rev() {
                self.queue.remove(i);
            }
        }
        for (m, failed) in arrived {
            if failed {
                self.failed = self.failed.saturating_sub(1);
            }
            self.done += 1;
            self.confirmed.push(sorter::journal::Entry {
                id: m.id,
                folder: m.folder,
                from: m.from,
                name: m.name.clone(),
                to: None,
            });
        }
    }

    /// The moves still unsure, as journal entries with no known place.
    fn unsure_entries(&mut self) -> Vec<sorter::journal::Entry> {
        std::mem::take(&mut self.unsure)
            .into_iter()
            .map(|(m, _, _)| sorter::journal::Entry {
                id: m.id,
                folder: m.folder,
                from: m.from,
                name: m.name,
                to: None,
            })
            .collect()
    }

    /// A failed batch's moves, unsure until found or forgotten: once each
    /// (a later failure of the same move replaces the earlier one).
    fn mark_unsure(&mut self, moves: &[Move], failed: bool, now: Instant) {
        self.settled_at = None;
        self.unsure.retain(|(m, _, _)| !moves.iter().any(|b| b.id == m.id));
        self.unsure.extend(moves.iter().cloned().map(|m| (m, failed, now)));
    }

    /// The time left, from the pace so far.
    fn eta(&self) -> Option<std::time::Duration> {
        let elapsed = self.started.elapsed().as_secs_f64();
        let left = self.queue.len();
        (self.done > 0 && left > 0).then(|| std::time::Duration::from_secs_f64(elapsed / self.done as f64 * left as f64))
    }
}

const PREVIEW: usize = 0;
const RULES: usize = 1;
const OPTIONS: usize = 2;
const CLEANUP: usize = 3;

/// Journal saves: at most this often while a run confirms batches, and at
/// a run's end.
const SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

impl State {
    fn account_dir(&self) -> Option<std::path::PathBuf> {
        self.account.map(|agent| crate::settings::account_dir(&agent))
    }

    /// Read the account's journal, unfinished run and snapshot list, once.
    fn load_account(&mut self, agent: Uuid) {
        if self.account == Some(agent) {
            return;
        }
        self.account = Some(agent);
        let dir = crate::settings::account_dir(&agent);
        use sorter::journal as j;
        let path = j::journal_path(&dir);
        self.journal = SharedJournal::of(agent, move || match j::load(&path) {
            Ok(journal) => journal,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(error)
                if !matches!(
                    error.kind(),
                    std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::Other
                ) =>
            {
                // busy or not allowed now: an empty journal for this session,
                // the file left as it is
                log::warn!("auto-sort: the journal could not be read now: {error}");
                j::Journal {
                    read_only: true,
                    ..Default::default()
                }
            }
            Err(error) => {
                // kept aside, never written over: the history stays for a
                // newer viewer or a look by hand
                let aside = path.with_extension(format!("bad-{}.json.gz", unix_now()));
                let kept = std::fs::rename(&path, &aside).is_ok();
                log::warn!("auto-sort: the journal could not be read ({error}); kept aside: {kept}");
                Default::default()
            }
        });
        let (send, read) = std::sync::mpsc::channel();
        let path = j::unfinished_path(&dir);
        let load = move || {
            let unfinished = j::load(&path).ok().filter(|u: &j::Unfinished| !u.moves.is_empty());
            let _ = send.send(unfinished);
        };
        if std::thread::Builder::new()
            .name("auto-sort run to resume".into())
            .spawn(load)
            .is_ok()
        {
            self.unfinished_loading = Some(read);
        }
        self.snapshots = j::snapshots(&dir);
    }

    /// Write the journal (and the run's moves still to send) off the frame.
    fn save_progress(&mut self) {
        self.unsaved = 0;
        let Some(dir) = self.account_dir() else { return };
        // a journal whose file could not be read is never written over it
        let journal = self.journal.clone();
        journal.2.store(true, std::sync::atomic::Ordering::Release);
        // only the window whose run is going (or just ended) writes or
        // removes the run to resume: another one's save leaves it alone
        let owns_run = self.job.is_some() || std::mem::take(&mut self.job_just_ended);
        self.saved_at = Some(Instant::now());
        // a new run's moves replace any older run to resume
        if self.job.as_ref().is_some_and(|job| job.restoring != Some(Restoring::Resume)) {
            self.unfinished = None;
        }
        // the run to resume is written at its start and again each time it
        // shrank by half, never its whole queue every few seconds (a dozen
        // copies over a 400,000-move run; a resume drops what was done)
        let mut keep_file = false;
        if let Some(job) = self.job.as_mut() {
            let remaining = job.queue.len() + job.in_flight.as_ref().map_or(0, |batch| batch.moves.len());
            if remaining > 0 && job.run.is_some() && job.unfinished_len.is_some_and(|last| remaining * 2 > last) {
                keep_file = true;
            } else {
                job.unfinished_len = Some(remaining);
            }
        }
        if keep_file {
            in_order(move || {
                if let Some(journal) = journal.for_save()
                    && let Err(error) = sorter::journal::save(&sorter::journal::journal_path(&dir), &journal)
                {
                    log::warn!("auto-sort: the journal could not be saved: {error}");
                }
            });
            return;
        }
        let unfinished = self.job.as_ref().map(|job| sorter::journal::Unfinished {
            base_id: job.base_id,
            undoes: job.undoes,
            runs: job.earlier_runs.iter().copied().chain(job.run).collect(),
            base: job.base.clone(),
            moves: job
                .in_flight
                .iter()
                .flat_map(|batch| batch.moves.iter())
                .chain(job.queue.iter())
                .cloned()
                .collect(),
        });
        in_order(move || {
            use sorter::journal as j;
            if let Some(journal) = journal.for_save()
                && let Err(error) = j::save(&j::journal_path(&dir), &journal)
            {
                log::warn!("auto-sort: the journal could not be saved: {error}");
            }
            if !owns_run {
                return;
            }
            let path = j::unfinished_path(&dir);
            match unfinished.filter(|u| !u.moves.is_empty()) {
                Some(unfinished) => {
                    if let Err(error) = j::save(&path, &unfinished) {
                        log::warn!("auto-sort: the run in progress could not be saved: {error}");
                    }
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        });
    }

    /// A snapshot of the layout before a sort or Trash run, kept with the
    /// account's last ones.
    fn snapshot(&mut self, inv: &Inventory) {
        let Some(dir) = self.account_dir() else { return };
        let at = unix_now();
        let Some(account) = self.account else { return };
        // a large inventory's layout was read off the frame with its
        // preview: used when nothing changed since, else read now
        let snapshot = match self.layout.take() {
            Some((generation, mut layout)) if generation == inv.generation => {
                layout.at = at;
                layout.account = account;
                layout
            }
            _ => sorter::journal::Snapshot::take(inv, at, account),
        };
        let path = sorter::journal::snapshot_path(&dir, at);
        // one in this second already: it is the layout before both runs
        if self.snapshots.iter().any(|(_, listed)| *listed == path) || path.exists() {
            return;
        }
        self.snapshots.insert(0, (at, path));
        self.snapshots.truncate(sorter::journal::SNAPSHOTS);
        in_order(move || {
            use sorter::journal as j;
            if let Err(error) = j::save(&j::snapshot_path(&dir, at), &snapshot) {
                log::warn!("auto-sort: the snapshot could not be saved: {error}");
            }
            j::prune_snapshots(&dir);
        });
    }

    /// What a run has confirmed so far is written before the inventory
    /// goes away (logout): Undo and Resume find it at the next session.
    fn flush(&mut self) {
        if let Some(job) = self.job.as_mut() {
            // (an Undo records only what went back: its run forgets what it
            // records from the run it undid)
            let unsure = job.unsure_entries();
            // the batch on its way may have been applied: journaled too, as
            // Undo leaves an entry still where it was
            let flying = job.in_flight.iter().flat_map(|b| b.moves.iter()).map(|m| sorter::journal::Entry {
                id: m.id,
                folder: m.folder,
                from: m.from,
                name: m.name.clone(),
                to: None,
            });
            let mut confirmed = std::mem::take(&mut job.confirmed);
            if job.undoes.is_none() {
                confirmed.extend(flying);
                confirmed.extend(unsure);
            }
            // the run to resume written with what is left now, not the copy
            // kept since the run was twice as long
            job.unfinished_len = None;
            // a move resent while unsure is both on its way and unsure
            let mut seen = HashSet::new();
            confirmed.retain(|e| seen.insert(e.id));
            if !confirmed.is_empty() {
                self.journal.record_in(job.run, confirmed);
                self.unsaved += 1;
            }
        }
        // a Trash run's entries on their way or not seen in the Trash yet:
        // journaled too, as Undo leaves an entry that is not in the Trash
        if let Some(trash) = self.trash.as_mut() {
            let flying: Vec<(Uuid, Uuid)> = trash
                .in_flight
                .take()
                .map(|(batch, _, _)| batch)
                .unwrap_or_default()
                .into_iter()
                .chain(trash.unsure.drain(..).map(|(id, from, _)| (id, from)))
                .collect();
            if !flying.is_empty() {
                self.journal.record_in(
                    trash.run,
                    flying.into_iter().map(|(id, from)| sorter::journal::Entry {
                        id,
                        folder: false,
                        from,
                        name: String::new(),
                        to: None,
                    }),
                );
                self.unsaved += 1;
            }
        }
        if self.unsaved > 0 || self.job.is_some() {
            self.save_progress();
        }
        // the window and its state go: its run no longer holds the account
        self.journal.release(self.window);
    }

    pub(super) fn active(&self) -> bool {
        self.open || self.job.is_some() || self.submitted || self.requested.is_some() || self.trash.is_some() || self.purge.is_some()
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.submitted || self.trash.is_some() || self.purge.is_some()
    }

    fn set_source(&mut self, source: Source) {
        self.open = true;
        if self.busy() {
            return;
        }
        if self.source.as_ref() != Some(&source) {
            self.preview = None;
            self.status.clear();
            self.source_loaded = None;
            // an Undo waiting for its folders is given up with its source
            self.undo_waiting = None;
            self.undo_folders.clear();
        }
        self.source = Some(source);
    }
}

impl InventoryUi {
    /// A folder's right-click: sort its contents.
    pub(crate) fn open_sorter(&mut self, source: Uuid) {
        self.sorter.set_source(Source::Folder(source));
        self.sorter.tab = PREVIEW;
    }

    /// "Sort by this creator": the hint is added (or the existing one for
    /// that creator kept, its name refreshed) and Options opens on it, for
    /// the user to name the store and choose the category.
    pub(crate) fn add_creator_hint(&mut self, prefs: &mut sorter::Preferences, hint: sorter::StoreHint) {
        match prefs.store_hints.iter_mut().find(|h| h.creator == hint.creator) {
            Some(mine) => {
                // a name still loading ("…") never replaces a known one
                if !hint.creator_name.is_empty() && !hint.creator_name.ends_with('…') {
                    mine.creator_name = hint.creator_name;
                }
            }
            None => {
                let mut hint = hint;
                if hint.creator_name.ends_with('…') {
                    hint.creator_name.clear();
                }
                prefs.store_hints.push(hint);
            }
        }
        self.sorter.open = true;
        if self.sorter.source.is_none() {
            // the window opens on its usual folder (the selected one or
            // Objects), never on nothing
            self.sorter.requested = Some(Uuid::nil());
        }
        if !self.sorter.busy() {
            self.sorter.tab = OPTIONS;
        }
    }

    /// An entry's right-click: the selected entries' preview, sorted once
    /// the user presses Sort (nothing moves before a preview is seen).
    pub(crate) fn sort_entries(&mut self, ids: Vec<Uuid>) {
        if self.sorter.busy() {
            self.sorter.open = true;
            return;
        }
        self.sorter.set_source(Source::Entries(ids));
        self.sorter.tab = PREVIEW;
    }

    /// The toolbar button: the window opens on `shown` (the folder the
    /// inventory shows), the selected folder or Objects.
    pub(crate) fn request_sorter(&mut self, shown: Uuid) {
        if self.sorter.open {
            self.sorter.open = false;
        } else {
            self.sorter.requested = Some(shown);
        }
    }

    /// The demo's unattended run: Auto-sort opens and starts by itself.
    pub(crate) fn demo_run_sorter(&mut self, source: Uuid) {
        self.open_sorter(source);
        self.sorter.autostart = true;
    }

    /// `AURORA_DEMO_INVENTORY=sorter-vocab`: the vocabulary check window.
    pub(crate) fn demo_vocab_check(&mut self) {
        self.sorter.vocab_check = Some(sorter::vocab::corpus());
    }

    /// `AURORA_DEMO_SORTER_TAB`: the window on one tab, window or
    /// confirmation, for the screenshots (docs/TESTING.md).
    pub(crate) fn demo_sorter_view(&mut self, view: &str, prefs: &mut sorter::Preferences) {
        let state = &mut self.sorter;
        match view {
            "rules" => state.tab = RULES,
            "options" => state.tab = OPTIONS,
            "cleanup" => {
                state.tab = CLEANUP;
                prefs.cleanup.age = Age::FiveYears;
            }
            "cleanup-custom" => {
                state.tab = CLEANUP;
                prefs.cleanup = sorter::Cleanup {
                    age: Age::Custom,
                    year: 2013,
                    month: 6,
                    kinds: vec![Kind::Notecards, Kind::Landmarks, Kind::Textures],
                    empty_folders: true,
                    ..Default::default()
                };
            }
            "cleanup-duplicates" => {
                state.tab = CLEANUP;
                prefs.cleanup = sorter::Cleanup {
                    find: Find::Duplicates,
                    ..Default::default()
                };
            }
            "share" => {
                state.tab = OPTIONS;
                state.share = Some(sorter::share::Share::all(prefs));
            }
            "import" => {
                state.tab = OPTIONS;
                // a friend's text: one rule already here, a new one, a store
                // hint and a broken line
                let mut theirs = prefs.clone();
                theirs.rules.push(Rule {
                    words: "wings halo".into(),
                    mode: Match::Any,
                    folder: "Accessories/Wings".into(),
                    ..Default::default()
                });
                // a new Pre-Trash rule (red, unticked) and two options that
                // differ (old → new; the sort folder name unticked)
                theirs.rules.push(Rule {
                    words: "old gift".into(),
                    mode: Match::All,
                    pre_trash: true,
                    ..Default::default()
                });
                theirs.base = "Tidy".into();
                theirs.stores = !prefs.stores;
                let text = sorter::share::export(&theirs, &sorter::share::Share::all(&theirs)) + "store: Somewhere = not-a-category\n";
                state.import = Some(ImportWindow {
                    text,
                    ..Default::default()
                });
            }
            "picker" => {
                state.tab = RULES;
                state.picker = Some(Picker::new(Pick::Rule(1)));
            }
            "confirm-whole" => {
                state.set_source(Source::Whole);
                state.understood = false;
                state.confirm = Some(Confirm::Whole);
            }
            _ => state.tab = PREVIEW,
        }
    }

    pub(super) fn sorter_open(&self) -> bool {
        self.sorter.open
    }

    /// The app takes an inventory change: when it is a sorter's batch on
    /// its way (recognised by its first move), that batch takes the request
    /// id, so only its own result completes it. True for a sorter's batch:
    /// its new folders are neither shown nor renamed like a "New folder".
    /// Before the inventory window state is dropped (logout): every
    /// window's Auto-sort run written to its journal.
    pub(crate) fn flush_sorter(&mut self) {
        self.sorter.flush();
        for window in &mut self.windows {
            window.state.sorter.flush();
        }
    }

    /// At quit, after the last flush: Auto-sort's files written (a few
    /// seconds at most).
    pub(crate) fn wait_for_sorter_writes(&self) {
        wait_for_writes(std::time::Duration::from_secs(5));
    }

    pub(crate) fn claim_sorter_request(&mut self, request: Uuid, change: &aurora_net::inventory::operations::Mutation) -> bool {
        fn claim(state: &mut State, request: Uuid, change: &aurora_net::inventory::operations::Mutation) -> bool {
            let Some(batch) = state.job.as_mut().and_then(|job| job.in_flight.as_mut()) else {
                return false;
            };
            let first = batch.moves.first().map(|m| m.id);
            let ours = batch.request.is_none()
                && change
                    .operations
                    .iter()
                    .any(|op| matches!(op, aurora_net::inventory::operations::Operation::Patch { id, .. } if Some(*id) == first));
            if ours {
                batch.request = Some(request);
            }
            ours
        }
        // a Clean up Trash batch: the run's own request, matched by its first
        // entry like a sort's
        fn claim_trash(state: &mut State, request: Uuid, change: &aurora_net::inventory::operations::Mutation) -> bool {
            let Some(trash) = state.trash.as_mut() else { return false };
            let Some(first) = trash.in_flight.as_ref().and_then(|(batch, _, _)| batch.first().map(|(id, _)| *id)) else {
                return false;
            };
            let ours = trash.request.is_none()
                && change
                    .operations
                    .iter()
                    .any(|op| matches!(op, aurora_net::inventory::operations::Operation::Patch { id, .. } if *id == first));
            if ours {
                trash.request = Some(request);
                trash.answered = false;
            }
            ours
        }
        // an Empty in small steps piece, matched by its first entry
        fn claim_purge(state: &mut State, request: Uuid, change: &aurora_net::inventory::operations::Mutation) -> bool {
            let Some(job) = state.purge.as_mut() else { return false };
            let Some(first) = job.in_flight.as_ref().and_then(|(piece, ..)| piece.first()) else {
                return false;
            };
            let ours = job.request.is_none()
                && change
                    .operations
                    .iter()
                    .any(|op| matches!(op, aurora_net::inventory::operations::Operation::Delete { id, .. } if *id == first));
            if ours {
                job.request = Some(request);
                job.answered = None;
            }
            ours
        }
        claim_purge(&mut self.sorter, request, change)
            || self.windows.iter_mut().any(|w| claim_purge(&mut w.state.sorter, request, change))
            || claim(&mut self.sorter, request, change)
            || self.windows.iter_mut().any(|w| claim(&mut w.state.sorter, request, change))
            || claim_trash(&mut self.sorter, request, change)
            || self.windows.iter_mut().any(|w| claim_trash(&mut w.state.sorter, request, change))
    }

    /// The result of `request`: completes the sorter batch that took it.
    /// `unread`: moves of the batch whose place the server's answer does not
    /// give; None (a failed request) is the whole batch.
    pub(crate) fn finish_sorter(&mut self, request: Uuid, error: Option<&str>, unread: Option<&[Uuid]>) {
        for state in std::iter::once(&mut self.sorter).chain(self.windows.iter_mut().map(|w| &mut w.state.sorter)) {
            if let Some(job) = state.purge.as_mut().filter(|j| j.request == Some(request)) {
                job.answered = Some(error.is_none());
            }
            if let Some(trash) = state.trash.as_mut().filter(|t| t.request == Some(request)) {
                trash.answered = true;
                // (refused, or not confirmed: shown back where they were, the
                // Trash wait then counts only what the server really moved)
                if let Some((batch, _, _)) = trash.in_flight.as_ref() {
                    let back = batch
                        .iter()
                        .filter(|(id, _)| error.is_some() && unread.is_none() || unread.is_some_and(|u| u.contains(id)));
                    trash.put_back.extend(back.copied());
                }
            }
        }
        fn finish(state: &mut State, request: Uuid, error: Option<&str>, unread: Option<&[Uuid]>) {
            let ours = state
                .job
                .as_ref()
                .and_then(|job| job.in_flight.as_ref())
                .is_some_and(|batch| batch.request == Some(request));
            if !ours {
                return;
            }
            state.submitted = false;
            state.preview = None;
            if let Some(job) = state.job.as_mut()
                && let Some(message) = job.finish(error, unread, Instant::now())
            {
                state.status = message;
            }
        }
        finish(&mut self.sorter, request, error, unread);
        for window in &mut self.windows {
            finish(&mut window.state.sorter, request, error, unread);
        }
    }
}
/// The folder the toolbar button sorts: the selected folder, else the folder
/// shown, else Objects, else My Inventory.
fn default_source(inv: &Inventory, st: &InventoryUi, shown: Uuid) -> Uuid {
    st.selected
        .filter(|id| inv.folders.contains_key(id))
        .into_iter()
        .chain([shown])
        .find(|id| *id != inv.root && sorter::source_allowed(inv, *id))
        .or_else(|| rules::system(inv, 6))
        .unwrap_or(inv.root)
}

/// Send the next batch when the inventory is free, then draw the window.
pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    worn: &HashSet<Uuid>,
    agent: Uuid,
    hold: bool,
    actions: &mut Vec<InvAction>,
) {
    st.sorter.held = hold;
    share_step(inv, &mut st.sorter, actions);
    // the default folders the user allowed, for the hands-off rule
    let unlocked = prefs.sorter.unlocked_set(inv);
    if inv.sorter_unlocked != unlocked {
        inv.sorter_unlocked = unlocked;
    }
    if st.sorter.open || st.sorter.job.is_some() || st.sorter.trash.is_some() {
        st.sorter.load_account(agent);
        // the stopped run, once read; a run started meanwhile replaced it
        if let Some(read) = &st.sorter.unfinished_loading
            && let Ok(unfinished) = read.try_recv()
        {
            st.sorter.unfinished_loading = None;
            if st.sorter.job.is_none() {
                st.sorter.unfinished = unfinished;
            }
        }
    }
    // one run at a time per account, across inventory windows
    if st.sorter.window.is_nil() {
        st.sorter.window = Uuid::new_v4();
    }
    if st.sorter.job.is_some() || st.sorter.trash.is_some() || st.sorter.purge.is_some() {
        st.sorter.journal.claim(st.sorter.window);
    } else {
        st.sorter.journal.release(st.sorter.window);
    }
    if let Some(shown) = st.sorter.requested.take() {
        let source = default_source(inv, st, shown);
        st.sorter.set_source(Source::Folder(source));
    }
    let free = st.pending.is_none()
        && !actions
            .iter()
            .any(|action| matches!(action, InvAction::Edit(_) | InvAction::Delete(_) | InvAction::Drop(_)));
    let now = Instant::now();
    let state = &mut st.sorter;
    // A batch the app never took (another inventory change went first in
    // that frame): back at the front of the queue, never counted.
    if let Some(job) = state.job.as_mut() {
        let dropped = job.in_flight.as_mut().is_some_and(|batch| {
            batch.frames += 1;
            batch.request.is_none() && batch.frames > 2
        });
        if dropped && let Some(batch) = job.in_flight.take() {
            if job.stopping {
                // stopped: what was never taken is not sent later, and is
                // skipped, not unsure any more
                job.skipped += batch.moves.len();
                job.unsure
                    .retain(|(m, failed, _)| *failed || !batch.moves.iter().any(|b| b.id == m.id));
            } else {
                job.requeue(batch.moves);
            }
            state.submitted = false;
        }
        let silent = job
            .in_flight
            .as_ref()
            .is_some_and(|batch| now.duration_since(batch.sent_at) > ANSWER_LIMIT);
        if silent {
            if let Some(message) = job.finish(Some(aurora_i18n::tr!("inventory-sorter-no-answer")), None, now) {
                state.status = message;
            }
            state.submitted = false;
        }
    }
    // a refused batch the server applied after all: done, not resent (before
    // the journal takes this frame's confirmed moves, so none is left out)
    if let Some(job) = state.job.as_mut() {
        // (a refused or partly confirmed batch's answer carries the server's
        // record of each entry it read, already applied; the others are
        // shown back where they were: an entry is never counted from where
        // it was only sent)
        for (id, folder, from) in std::mem::take(&mut job.put_back) {
            sorter::show_in(inv, id, folder, from);
        }
        job.settle(inv);
    }
    // The journal takes what the server confirmed; saved every few seconds
    // with the moves still to send, so a run the viewer stops can resume.
    let mut begin = false;
    if let Some(job) = state.job.as_mut() {
        if !job.confirmed.is_empty() {
            let confirmed = placed(inv, std::mem::take(&mut job.confirmed));
            state.journal.record_in(job.run, confirmed);
            state.unsaved += 1;
        }
        if job.done == 0 && job.in_flight.is_none() && !job.begun {
            job.begun = true;
            begin = true;
        }
    }
    if begin {
        // a run starts: its journal, and a snapshot of the layout first
        let undoes = state.job.as_ref().and_then(|job| job.undoes);
        let run = match undoes {
            Some(undone) => state.journal.with(|j| j.begin_undo(unix_now(), undone)),
            None => state.journal.begin(sorter::journal::RunKind::Move, unix_now()),
        };
        if let Some(job) = state.job.as_mut() {
            job.run = Some(run);
        }
        state.snapshot(inv);
    }
    if state.unsaved > 0 && state.saved_at.is_none_or(|at| at.elapsed() >= SAVE_EVERY) {
        state.save_progress();
    }
    // paused or held, a destination's wait starts again afterwards: the
    // pause is not counted against it
    if (hold || state.job.as_ref().is_some_and(|job| job.paused))
        && let Some(job) = state.job.as_mut()
    {
        job.waiting_since = None;
    }
    // while a teleport runs, nothing is sent and
    // no retry is spent: the job resumes on its own
    if free
        && !hold
        && !state.submitted
        && let Some(job) = state.job.as_mut()
        && !job.paused
        && job.next_at.is_none_or(|at| now >= at)
    {
        // a folder excluded during the run: its moves still queued stay
        // (the next batch's are checked, a few hundred at most); a folder
        // listed in My folders since, or holding one, is not moved whole
        // (the sort folder a first run makes: known once it exists, so an
        // exclusion below it counts)
        if job.base_id.is_none_or(|id| !inv.folders.contains_key(&id)) {
            job.base_id = sorter::child(inv, inv.root, &job.base);
        }
        let checks = !prefs.sorter.excluded.is_empty() || !prefs.sorter.own.is_empty();
        let own = if checks {
            sorter::own_folders(inv, &prefs.sorter)
        } else {
            Default::default()
        };
        // (the sort folder the moves really go into, as `step` finds it)
        let base = sorter::child(inv, inv.root, &job.base);
        let options = &prefs.sorter;
        // (each destination path read once a frame)
        let paths: std::cell::RefCell<std::collections::HashMap<Vec<String>, bool>> = Default::default();
        // a folder holding an excluded one stays too: it would carry it
        let refused = |inv: &Inventory, m: &Move| {
            if !checks {
                return false;
            }
            if !m.restore && m.folder && own.contains(&m.id) {
                return true;
            }
            // (a snapshot's restore honours exclusions like a sort)
            if m.restore && !m.snapshot {
                return false;
            }
            // (into an excluded folder neither: a rule's, or one on the way
            // below the sort folder)
            let into_excluded = || match &m.to {
                sorter::Dest::Folder(t) => options.excludes(inv, *t),
                sorter::Dest::Path(p) => base.is_some_and(|b| {
                    *paths
                        .borrow_mut()
                        .entry(p.clone())
                        .or_insert_with(|| sorter::path_excluded(inv, b, p, options))
                }),
                sorter::Dest::PreTrash => false,
            };
            options.excludes(inv, m.id)
                || m.folder
                    && options
                        .excluded
                        .iter()
                        .any(|e| crate::world::inventory::actions::under(inv, *e, m.id))
                || into_excluded()
        };
        match sorter::step_with(inv, &job.base, &mut job.queue, &prefs.protected, worn, &mut job.skipped, &refused) {
            Step::Change(change, moves) => {
                job.in_flight = Some(InFlight {
                    moves,
                    request: None,
                    frames: 0,
                    sent_at: now,
                });
                job.waiting_since = None;
                state.submitted = true;
                actions.push(InvAction::Edit(*change));
            }
            Step::Loading => {
                // a folder to move whole whose subfolders load slowly: only
                // that move is given up after a longer wait, never the moves
                // to the same place
                job.wait_for(true);
                let since = *job.waiting_since.get_or_insert(now);
                if now.duration_since(since) > LOADING_LIMIT {
                    if job.queue.pop_front().is_some() {
                        job.skipped += 1;
                        log::warn!(
                            "auto-sort: a folder's subfolders did not load in {} s: it stays",
                            LOADING_LIMIT.as_secs()
                        );
                    }
                    job.waiting_since = None;
                }
            }
            Step::Wait => {
                // a destination that never loads: its move is skipped
                job.wait_for(false);
                let since = *job.waiting_since.get_or_insert(now);
                if now.duration_since(since) > WAIT_LIMIT {
                    if let Some(first) = job.queue.pop_front() {
                        // every move to the same place waits on the same
                        // folder: they stay too, instead of 30 s each
                        let before = job.queue.len();
                        job.queue.retain(|m| m.to != first.to);
                        job.skipped += 1 + before - job.queue.len();
                        log::warn!(
                            "auto-sort: a destination folder did not load in {} s: {} entries stay",
                            WAIT_LIMIT.as_secs(),
                            1 + before - job.queue.len()
                        );
                    }
                    job.waiting_since = None;
                }
            }
            Step::Done => {
                // moves sent again after a refusal and never seen at their
                // place (a Stop cleared the queue): skipped, not done
                job.skipped += job.unsure.iter().filter(|(_, failed, _)| !failed).count();
                state.status = if job.failed > 0 {
                    aurora_i18n::tr!(
                        "inventory-sorter-done-failed",
                        count = job.done,
                        skipped = job.skipped,
                        failed = job.failed
                    )
                } else if job.skipped > 0 {
                    aurora_i18n::tr!("inventory-sorter-done-skipped", count = job.done, skipped = job.skipped)
                } else {
                    aurora_i18n::tr!("inventory-sorter-done", count = job.done)
                };
                let restoring = job.restoring;
                let done = job.done;
                let run = job.run;
                let undoes = job.undoes;
                let earlier = std::mem::take(&mut job.earlier_runs);
                // a refused batch's moves the server may have applied
                // after all, not seen yet: journaled too (Undo leaves an
                // entry still where it was), never lost from Undo
                let mut last = placed(inv, std::mem::take(&mut job.confirmed));
                // (a move still unsure at the end comes from a batch counted
                // as failed already). An Undo records only what went back:
                // its run forgets what it records from the run it undid
                // an Undo's unsure move that is back where it belongs went
                // back after all: journaled, so the run it undid forgets it
                let back: Vec<Uuid> = job
                    .unsure
                    .iter()
                    .filter(|(m, _, _)| matches!(m.to, sorter::Dest::Folder(t) if crate::world::inventory::actions::parent(inv, m.id) == Some(t)))
                    .map(|(m, _, _)| m.id)
                    .collect();
                let unsure = job.unsure_entries();
                if job.undoes.is_none() {
                    last.extend(unsure);
                } else {
                    last.extend(unsure.into_iter().filter(|e| back.contains(&e.id)));
                }
                state.journal.record_in(run, last);
                state.job = None;
                // only a run that moved something (or a resume) ends the run
                // to resume; one stopped at once leaves an older one alone
                state.job_just_ended = done > 0 || restoring == Some(Restoring::Resume);
                state.journal.release(state.window);
                state.preview = None;
                state.cleanup = None;
                if let Some(undone) = undoes {
                    // every Undo run of it goes (this session's, a stopped
                    // session's, one begun afresh), and from the run it undid
                    // only the entries they put back: what could not go back
                    // is still there to undo
                    let _ = (earlier, run);
                    state.journal.with(|j| j.forget_undos_of(undone));
                }
                if restoring == Some(Restoring::Resume) {
                    state.unfinished = None;
                }
                // the journal written, and no run left to resume
                state.save_progress();
            }
        }
    }
    if let Some(trash) = st.sorter.trash.as_mut() {
        for (id, from) in std::mem::take(&mut trash.put_back) {
            let folder = inv.folders.contains_key(&id);
            sorter::show_in(inv, id, folder, from);
        }
    }
    send_trash_batch(inv, st, prefs, worn, actions, free && !hold);
    send_purge_piece(inv, st, worn, actions, free && !hold);
    let state = &mut st.sorter;
    if !state.open {
        return;
    }
    let Some(source) = state.source.clone() else {
        state.open = false;
        return;
    };
    let busy = state.busy();
    if state.worn_now != *worn {
        state.worn_now.clone_from(worn);
    }
    if state.protected_now != prefs.protected {
        state.protected_now.clone_from(&prefs.protected);
    }
    let loaded = match &source {
        Source::Folder(id) => {
            // Loading the folder is all a preview needs: its direct contents.
            let loaded = inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetched);
            if !loaded && inv.folders.contains_key(id) {
                inv.request(*id);
            }
            loaded
        }
        Source::Entries(_) => true,
        // saved moves (Undo, a layout, a run to resume) are read where their
        // entries were: each such folder is loaded first, else a move whose
        // entry is not known yet would be dropped from the run
        // A folder moved whole is loaded with all it holds too, else it
        // would be kept as "not loaded". Read again only when the inventory
        // changes, never every frame.
        // (not looked at while a run goes: nothing is planned then)
        Source::Saved(..) if busy => true,
        // (once loaded, it stays so: a 400,000-move run is not read again at
        // every change)
        Source::Saved(moves, _) => match state.source_loaded {
            Some((generation, loaded))
                if (generation == inv.generation
                    || loaded
                    // (while its folders load, looked at once a second, never
                    // every frame: a 400,000-move run is a long read)
                    || state.source_checked.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1)))
                    && state.source_excluded == prefs.sorter.excluded =>
            {
                loaded
            }
            _ => {
                // (an exclusion removed since: its folders are read now)
                state.source_excluded.clone_from(&prefs.sorter.excluded);
                // a run's move in or of a folder excluded now is kept: its
                // folders are never read (an Undo puts back what it moved)
                let excluded =
                    |m: &Move| !(m.restore && !m.snapshot) && (prefs.sorter.excludes(inv, m.id) || prefs.sorter.excludes(inv, m.from));
                let mut read: HashSet<Uuid> = HashSet::new();
                for m in moves.iter().filter(|m| !excluded(m)) {
                    read.insert(m.from);
                    if m.folder {
                        let undo = m.restore && !m.snapshot;
                        read.extend(sorter::unloaded_below(inv, m.id, if undo { &[] } else { &prefs.sorter.excluded }));
                    }
                }
                let loaded = ask_for(inv, read);
                state.source_loaded = Some((inv.generation, loaded));
                state.source_checked = Some(Instant::now());
                loaded
            }
        },
        // the root's folders only on the frame: deeper ones are listed by
        // the plan (made off the frame) and asked for as it arrives
        Source::Whole if busy => true,
        Source::Whole => match state.source_loaded {
            Some((generation, loaded)) if generation == inv.generation => loaded,
            _ => {
                let root = inv.folders.get(&inv.root).map(|f| f.children.clone()).unwrap_or_default();
                let loaded = ask_for(inv, root);
                state.source_loaded = Some((inv.generation, loaded));
                loaded
            }
        },
    };
    // The inventory changes many times a second while it loads: a plan of a
    // large one is made again once it has been quiet for a moment, and not
    // more than every few seconds, so the frame never stalls on it. A
    // change of the options is followed at once.
    if state.seen_generation != inv.generation {
        state.seen_generation = inv.generation;
        state.quiet_since = Some(now);
    }
    // folders the plans would read, asked for a few at a time beside the
    // inventory's own loading (never thousands of requests at once)
    while inv.queue.len() < REQUESTS_AT_ONCE
        && let Some(id) = state.to_request.pop()
    {
        // (a folder excluded since it was listed is never read)
        // (an Undo reads what its run moved, excluded since or not)
        let undo = state.undo_folders.contains(&id);
        if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Unknown) && (undo || !prefs.sorter.excludes(inv, id)) {
            inv.request(id);
        }
    }
    let settled = state.quiet_since.is_none_or(|q| now.duration_since(q) >= QUIET)
        && state.planned_at.is_none_or(|at| now.duration_since(at) >= REPLAN);
    // A plan made off the frame arrives: kept when it is still for this
    // source (its options and inventory are compared below, like any
    // preview's, and a newer one is made when they moved on).
    if let Some((task, (plan, layout))) = Planning::take(&mut state.planning)
        && task.source == source
    {
        // (made with older options, an exclusion since among them: its
        // folders to read are not taken, the next plan lists its own)
        if task.options == prefs.sorter {
            state.to_request.extend(plan.unfetched.iter().copied());
        }
        state.layout = Some((task.generation, layout));
        state.preview = Some((task.generation, task.options, plan));
    }
    let options_changed = state.preview.as_ref().is_some_and(|(_, options, _)| *options != prefs.sorter);
    let inventory_changed = state
        .preview
        .as_ref()
        .is_some_and(|(generation, _, _)| *generation != inv.generation);
    state.updating = inventory_changed && !settled || state.planning.is_some();
    if loaded && !busy && state.planning.is_none() && (state.preview.is_none() || options_changed || inventory_changed && settled) {
        state.planned_at = Some(now);
        let make = {
            let (source, options, protected, worn) = (source.clone(), prefs.sorter.clone(), prefs.protected.clone(), worn.clone());
            move |inv: &Inventory| {
                let mut plan = match &source {
                    Source::Folder(id) => sorter::plan(inv, *id, &options, &protected, &worn),
                    Source::Entries(ids) => sorter::plan_entries(inv, ids, &options, &protected, &worn),
                    Source::Whole => sorter::plan_whole(inv, &options, &protected, &worn),
                    Source::Saved(moves, _) => sorter::plan_saved(inv, moves, &options, &protected, &worn),
                };
                plan.order();
                plan
            }
        };
        if inv.items.len() + inv.folders.len() >= PLAN_IN_BACKGROUND {
            // the layout is read on the same copy, for the run's snapshot
            let plan_and_layout = move |inv: &Inventory| (make(inv), sorter::journal::Snapshot::take(inv, 0, Uuid::nil()));
            state.planning = Some(Planning::spawn(inv, &prefs.sorter, &source, None, plan_and_layout));
            state.updating = true;
        } else {
            let plan = make(inv);
            // A Pre-Trash rule reads the whole folder sorted at any depth (and
            // the sort folder's tree): what is not loaded yet is asked for, and
            // the preview fills in as it arrives (it follows the inventory's
            // generation). Nothing waits for it.
            state.to_request.extend(plan.unfetched.iter().copied());
            state.preview = Some((inv.generation, prefs.sorter.clone(), plan));
        }
    }
    // Clean up's list, read on its tab only: once per inventory change (a
    // folder arriving included), choice, source or day.
    let cleanup_source = match &source {
        Source::Folder(id) => *id,
        Source::Entries(_) | Source::Whole | Source::Saved(..) => inv.root,
    };
    if let Some((task, plan)) = Planning::take(&mut state.cleaning)
        && let Some((choices, from, day)) = task.cleanup
        // (made with older options, an exclusion since among them: dropped,
        // and made again)
        && task.options == prefs.sorter
    {
        state.to_request.extend(plan.unfetched.iter().copied());
        state.cleanup = Some((task.generation, choices, from, day, plan));
        state.cleanup_excluded.clone_from(&prefs.sorter.excluded);
        state.cleanup_own.clone_from(&prefs.sorter.own);
    }
    if state.tab == CLEANUP && !busy && state.cleaning.is_none() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let day = now / 86_400;
        let instant = Instant::now();
        let settled = state.quiet_since.is_none_or(|q| instant.duration_since(q) >= QUIET)
            && state.cleaned_at.is_none_or(|at| instant.duration_since(at) >= REPLAN);
        let stale = state.cleanup.as_ref().is_none_or(|(generation, choices, from, at, _)| {
            *choices != prefs.sorter.cleanup || *from != cleanup_source || *at != day || *generation != inv.generation && settled
        }) || state.cleanup_excluded != prefs.sorter.excluded
            || state.cleanup_own != prefs.sorter.own;
        if stale {
            state.cleaned_at = Some(instant);
            let make = {
                let (options, protected, worn) = (prefs.sorter.clone(), prefs.protected.clone(), worn.clone());
                move |inv: &Inventory| match options.cleanup.find {
                    Find::Old => sorter::plan_cleanup(inv, &options, &options.cleanup, cleanup_source, now, &protected, &worn),
                    Find::Duplicates => sorter::plan_duplicates(inv, &options, &options.cleanup, cleanup_source, &protected, &worn),
                }
            };
            if inv.items.len() + inv.folders.len() >= PLAN_IN_BACKGROUND {
                let about = Some((prefs.sorter.cleanup.clone(), cleanup_source, day));
                state.cleaning = Some(Planning::spawn(inv, &prefs.sorter, &source, about, make));
            } else {
                let plan = make(inv);
                state.to_request.extend(plan.unfetched.iter().copied());
                state.cleanup = Some((inv.generation, prefs.sorter.cleanup.clone(), cleanup_source, day, plan));
                state.cleanup_excluded.clone_from(&prefs.sorter.excluded);
                state.cleanup_own.clone_from(&prefs.sorter.own);
            }
        }
    }
    // (kept until the history is read: a run never starts before it)
    if state.autostart && state.preview.is_some() && !state.journal.with(|j| j.loading) {
        state.autostart = false;
        start(state, inv, prefs);
    }
    let mut open = true;
    let mut go = false;
    let mut stop = false;
    let mut pause = false;
    // what waits in Pre-Trash, read again only when the inventory changes
    if state
        .waiting
        .as_ref()
        .is_none_or(|(generation, _, _)| *generation != inv.generation)
    {
        state.waiting = Some((
            inv.generation,
            std::sync::Arc::new(sorter::pre_trash_entries(inv)),
            sorter::pre_trash(inv),
        ));
    }
    let (waiting, waiting_in) = state
        .waiting
        .as_ref()
        .map_or((std::sync::Arc::default(), None), |(_, w, at)| (std::sync::Arc::clone(w), *at));
    // Pre-Trash is emptied once loaded with all it holds (every entry and
    // subfolder known), asked for when it is not yet
    // (read again only when the inventory changes, never every frame)
    let pre_trash_loaded = match state.pre_trash_loaded {
        Some((generation, loaded)) if generation == inv.generation => loaded,
        _ => {
            let loaded = waiting_in.is_none_or(|id| ask_for(inv, sorter::unloaded_below(inv, id, &prefs.sorter.excluded)));
            state.pre_trash_loaded = Some((inv.generation, loaded));
            loaded
        }
    };
    // Empty in small steps reads all a folder holds first, never deleting
    // what it has not seen (a worn item in a subfolder not loaded yet); read
    // again when the inventory changes, at most once a second while loading
    if state.tab == CLEANUP || !waiting.is_empty() {
        for folder in [sorter::pre_trash(inv), rules::system(inv, 14)].into_iter().flatten() {
            let stale = state
                .purge_seen
                .get(&folder)
                .is_none_or(|(generation, at, _, _)| *generation != inv.generation && at.elapsed() >= std::time::Duration::from_secs(1));
            if stale {
                let unloaded = sorter::unloaded_below(inv, folder, &[]);
                // (a folder over the server's limit is emptied piece by piece
                // from what is known of it; one that failed waits for the
                // walk's own retries, never asked here in a loop)
                let root = inv.folders.get(&folder).map(|f| f.state);
                let readable = root == Some(FetchState::Fetched) || inv.too_big.contains(&folder);
                let all_read = ask_for(inv, unloaded);
                let loaded = readable && all_read;
                if !readable {
                    ask_for(inv, [folder]);
                }
                // (what it would delete: never what a folder never read keeps)
                let count = if loaded { sorter::purge::deletable(inv, folder) } else { 0 };
                // (a folder whose own read failed says so, never "loading")
                let failed = root == Some(FetchState::Failed) && !inv.too_big.contains(&folder);
                state.purge_seen.insert(
                    folder,
                    (inv.generation, Instant::now(), loaded, if failed { usize::MAX } else { count }),
                );
            }
        }
    }
    // The tabs change the body's height all the time (rule cards, preview
    // groups opening): egui's frame followed it and kept the tallest it saw,
    // so the window grew down and its bottom edge snapped back when dragged.
    // `steady_size` lays the body out in the window's own rect, like the
    // World Map's side panel (#2815); the minimum keeps the tabs, a few rows
    // and the footer whole.
    let free_now = st.pending.is_none() && st.pending_delete.is_none();
    let window_id = format!("inventory_sorter_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-sorter-title"),
        ctx.content_rect().center() - Vec2::new(250.0, 290.0),
        Vec2::new(500.0, 580.0),
    )
    .steady_size();
    window.min_size = Vec2::new(420.0, 360.0);
    window.show(ctx, p, &mut open, |ui| {
        let state = &mut st.sorter;
        source_row(ui, p, inv, state, &source, busy);
        ui.add_space(6.0);
        let rule_count = prefs.sorter.rules.len();
        let rules_label = if rule_count == 0 {
            aurora_i18n::tr!("inventory-sorter-rules").to_owned()
        } else {
            format!("{}  ({rule_count})", aurora_i18n::tr!("inventory-sorter-rules"))
        };
        crate::ui::widgets::tabs(
            ui,
            p,
            &mut state.tab,
            &[
                (aurora_i18n::tr!("inventory-sorter-preview"), true),
                (&rules_label, true),
                (aurora_i18n::tr!("inventory-sorter-options"), true),
                (aurora_i18n::tr!("inventory-cleanup-tab"), true),
            ],
        );
        ui.add_space(6.0);
        // the body in exactly the room the footer leaves (its height is
        // measured, a larger text included), so its button is never cut
        let (mut body, foot) = crate::ui::widgets::above_footer(ui);
        let room = body.available_height();
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_body")
            .max_height(room.max(80.0))
            .auto_shrink([false, false])
            .show(&mut body, |ui| match state.tab {
                RULES => {
                    ui.add_enabled_ui(!busy, |ui| rules_editor(ui, p, inv, state, &mut prefs.sorter))
                        .inner
                }
                OPTIONS => ui.add_enabled_ui(!busy, |ui| options(ui, p, inv, state, &mut prefs.sorter)).inner,
                CLEANUP => cleanup_tab(
                    ui,
                    p,
                    inv,
                    state,
                    &mut prefs.sorter,
                    matches!(source, Source::Folder(_)),
                    busy,
                    free_now,
                ),
                _ => {
                    if !waiting.is_empty() {
                        pre_trash_card(ui, p, inv, state, (&waiting, waiting_in), busy, !pre_trash_loaded);
                        ui.add_space(6.0);
                    }
                    // the default folders the user opened, named in every preview
                    if !inv.sorter_unlocked.is_empty() {
                        let mut names: Vec<String> = inv.sorter_unlocked.iter().map(|id| rules::name(inv, *id)).collect();
                        names.sort();
                        let folders = names.join(", ");
                        ui.label(
                            RichText::new(aurora_i18n::tr!("inventory-sorter-unlocked-list", folders = folders.as_str()))
                                .small()
                                .color(p.warn),
                        );
                    }
                    if state.updating {
                        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-updating")).small().color(p.muted));
                    }
                    match state.preview.as_ref() {
                        _ if !loaded => {
                            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-loading")).color(p.muted));
                        }
                        Some((_, _, plan)) => preview(ui, p, plan),
                        None => {}
                    }
                }
            });
        foot.show(ui, |ui| {
            crate::ui::widgets::divider(ui, p);
            footer_row(ui, p, state, st.pending.is_none(), &mut go, &mut stop, &mut pause);
        });
    });
    // Clean up's buttons: Pre-Trash through the sort's own batches, the
    // Trash after its confirmation
    match st.sorter.cleanup_action.take() {
        Some(CleanupAction::PreTrash) => {
            let state = &mut st.sorter;
            if let Some((_, choices, .., plan)) = state.cleanup.as_ref()
                && *choices == prefs.sorter.cleanup
                && state.cleaning.is_none()
            {
                let moves = plan.moves(&state.skipped);
                if !moves.is_empty() && state.job.is_none() && !busy_elsewhere(state) {
                    state.status.clear();
                    state.job = Some(Job::new(moves.into(), sorter::base_name_in(inv, &prefs.sorter)));
                }
            }
        }
        Some(CleanupAction::Trash) => {
            let state = &mut st.sorter;
            // only the list the user sees, made for the choices shown
            if let Some((_, choices, .., plan)) = state.cleanup.as_ref()
                && *choices == prefs.sorter.cleanup
                && state.cleaning.is_none()
            {
                let ids: Vec<(Uuid, Uuid)> = plan
                    .rows
                    .iter()
                    .filter(|r| !state.skipped.contains(&r.id))
                    .map(|r| (r.id, r.from))
                    .collect();
                if !ids.is_empty() {
                    state.confirm = Some(Confirm::Trash(folders_first(inv, ids).into()));
                }
            }
        }
        None => {}
    }
    let state = &mut st.sorter;
    if go && state.job.is_none() {
        if source == Source::Whole {
            // the second step: nothing moves before the confirmation
            state.understood = false;
            state.confirm = Some(Confirm::Whole);
        } else {
            start(state, inv, prefs);
        }
    }
    if pause && let Some(job) = state.job.as_mut() {
        job.paused = !job.paused;
    }
    if stop && let Some(job) = state.job.as_mut() {
        job.paused = false;
        // the batch on its way finishes; nothing after it is sent, not even
        // its retries
        job.stopping = true;
        // what was waiting to be resent stays unsure (the server may have
        // applied it: counted once seen, journaled at the run's end); the
        // rest is skipped
        let unsure: HashSet<Uuid> = job.unsure.iter().map(|(m, _, _)| m.id).collect();
        job.skipped += job.queue.iter().filter(|m| !unsure.contains(&m.id)).count();
        job.queue.clear();
    }
    if let Some(job) = state.purge.as_mut() {
        if pause {
            job.paused = !job.paused;
        }
        if stop {
            // the piece on its way finishes; nothing after it is sent, not
            // even its smaller parts if it is refused
            job.stopping = true;
            job.pieces.clear();
        }
    }
    if stop && let Some(trash) = state.trash.as_mut() {
        // what Stop leaves is told in the summary
        trash.skipped += trash.queue.len();
        trash.queue.clear();
    }
    state.open = open;
    if !open {
        // (a paused run never outlives its window: a sort or an Undo goes
        // on to its end, journaled for Undo, and a purge stops; neither
        // holds the account for every other window)
        if let Some(job) = state.job.as_mut() {
            job.paused = false;
        }
        if let Some(purge) = state.purge.as_mut()
            && purge.paused
        {
            purge.paused = false;
            purge.stopping = true;
            purge.pieces.clear();
        }
        state.picker = None;
        state.confirm = None;
        state.share = None;
        state.shared_for = None;
        state.import = None;
        state.vocab_check = None;
    }
    picker(ctx, p, inv, st, prefs);
    share_window(ctx, p, st, &prefs.sorter);
    import_window(ctx, p, st, &mut prefs.sorter);
    vocab_window(ctx, p, st);
    confirm(ctx, p, inv, st, prefs);
}

/// The vocabulary check (`sorter-vocab` demo): each corpus name, where it
/// should go and where the vocabulary in use (`AURORA_SORTER_VOCAB` or the
/// built-in one) sends it; differences first, in the warning colour.
fn vocab_window(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi) {
    let Some(rows) = st.sorter.vocab_check.as_ref() else { return };
    let mut open = true;
    let window_id = format!("inventory_sorter_vocab_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-sorter-vocab-title"),
        ctx.content_rect().center() - Vec2::new(330.0, 260.0),
        Vec2::new(660.0, 520.0),
    )
    .steady_size();
    window.min_size = Vec2::new(420.0, 300.0);
    window.show(ctx, p, &mut open, |ui| {
        let ok = rows.iter().filter(|r| r.matches()).count();
        let colour = if ok == rows.len() { p.success } else { p.warn };
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-sorter-vocab-summary", ok = ok, total = rows.len()))
                .strong()
                .color(colour),
        );
        ui.add_space(4.0);
        let stays = aurora_i18n::tr!("inventory-sorter-vocab-stays");
        let shown = |p: &Option<String>| p.clone().unwrap_or_else(|| stays.to_owned());
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_vocab_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("inventory_sorter_vocab_grid")
                    .num_columns(3)
                    .striped(true)
                    .spacing(Vec2::new(12.0, 4.0))
                    .show(ui, |ui| {
                        for header in [
                            aurora_i18n::tr!("inventory-sorter-vocab-name"),
                            aurora_i18n::tr!("inventory-sorter-vocab-expected"),
                            aurora_i18n::tr!("inventory-sorter-vocab-got"),
                        ] {
                            ui.label(RichText::new(header).small().strong().color(p.muted));
                        }
                        ui.end_row();
                        // differences first
                        let mut order: Vec<&sorter::vocab::CorpusRow> = rows.iter().collect();
                        order.sort_by_key(|r| r.matches());
                        for row in order {
                            let colour = if row.matches() { p.ink } else { p.warn };
                            ui.label(RichText::new(&row.name).small().color(colour));
                            ui.label(RichText::new(shown(&row.expected)).small().color(p.muted));
                            ui.label(RichText::new(shown(&row.got)).small().color(colour));
                            ui.end_row();
                        }
                    });
            });
    });
    if !open {
        st.sorter.vocab_check = None;
    }
}

/// A rule as one line: its words and where it sends them.
fn rule_line(rule: &Rule) -> String {
    let to = if rule.pre_trash {
        sorter::pre_trash_name()
    } else if rule.target.is_some() {
        aurora_i18n::tr!("inventory-share-my-folder").to_owned()
    } else {
        rule.folder.trim().to_owned()
    };
    format!("{}  →  {to}", rule.words.trim())
}

/// A store hint in one line: `Store  →  Category`, or for a creator hint
/// `by Creator  →  Store  →  Category`.
fn hint_line(hint: &sorter::StoreHint) -> String {
    let to = match hint.store.trim() {
        "" => hint.category.folder().to_owned(),
        store => format!("{store}  →  {}", hint.category.folder()),
    };
    match hint.creator {
        Some(_) => {
            let name = hint.creator_name.trim();
            let name = if name.is_empty() {
                aurora_i18n::tr!("inventory-sorter-creator-unknown").to_owned()
            } else {
                name.to_owned()
            };
            format!("{}  →  {to}", aurora_i18n::tr!("inventory-sorter-by-creator", name = name))
        }
        None => to,
    }
}

/// Share…: what will be shared, ticked, then copied as text.
fn share_window(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &sorter::Preferences) {
    use crate::ui::widgets::{ActionKind, action_button, checkbox};
    let Some(share) = st.sorter.share.as_mut() else { return };
    // the boxes stand by row: rules or hints added, removed or moved while
    // Share is open would shift a tick onto another rule, so a change
    // unticks everything (nothing private goes out by mistake)
    let hints: Vec<sorter::StoreHint> = prefs
        .store_hints
        .iter()
        .map(|h| sorter::StoreHint {
            creator_name: String::new(),
            ..h.clone()
        })
        .collect();
    let now = (prefs.rules.clone(), hints);
    match &st.sorter.shared_for {
        Some(seen) if *seen == now => {}
        Some(_) => {
            share.rules = vec![false; prefs.rules.len()];
            share.hints = vec![false; prefs.store_hints.len()];
            st.sorter.shared_for = Some(now);
        }
        None => st.sorter.shared_for = Some(now),
    }
    share.rules.resize(prefs.rules.len(), false);
    share.hints.resize(prefs.store_hints.len(), false);
    let mut open = true;
    let mut copy = false;
    let window_id = format!("inventory_sorter_share_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-share-title"),
        ctx.content_rect().center() - Vec2::new(200.0, 240.0),
        Vec2::new(400.0, 480.0),
    )
    .steady_size();
    window.min_size = Vec2::new(320.0, 300.0);
    window.show(ctx, p, &mut open, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-share-hint")).small().color(p.muted));
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_share_list")
            .max_height((ui.available_height() - 44.0).max(60.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !prefs.rules.is_empty() {
                    let shareable: Vec<usize> = (0..prefs.rules.len())
                        .filter(|i| sorter::share::shareable(&prefs.rules[*i]))
                        .collect();
                    let mut all = !shareable.is_empty() && shareable.iter().all(|i| share.rules[*i]);
                    let before = all;
                    checkbox(ui, p, &mut all, RichText::new(aurora_i18n::tr!("inventory-sorter-rules")).strong());
                    if all != before {
                        for i in &shareable {
                            share.rules[*i] = all;
                        }
                    }
                    for (index, rule) in prefs.rules.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            if sorter::share::shareable(rule) {
                                checkbox(ui, p, &mut share.rules[index], rule_line(rule));
                            } else {
                                let mut never = false;
                                ui.add_enabled_ui(false, |ui| checkbox(ui, p, &mut never, rule_line(rule)));
                                ui.label(RichText::new(aurora_i18n::tr!("inventory-share-private")).small().color(p.muted));
                            }
                        });
                    }
                    ui.add_space(6.0);
                }
                if !prefs.store_hints.is_empty() {
                    let mut all = share.hints.iter().all(|t| *t);
                    let before = all;
                    checkbox(ui, p, &mut all, RichText::new(aurora_i18n::tr!("inventory-sorter-hints")).strong());
                    if all != before {
                        share.hints.iter_mut().for_each(|t| *t = all);
                    }
                    for (index, hint) in prefs.store_hints.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            checkbox(ui, p, &mut share.hints[index], hint_line(hint));
                        });
                    }
                    ui.add_space(6.0);
                }
                ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-options")).strong());
                if !prefs.base.trim().is_empty() {
                    checkbox(
                        ui,
                        p,
                        &mut share.base,
                        aurora_i18n::tr!("inventory-share-base", name = prefs.base.trim()),
                    );
                }
                checkbox(ui, p, &mut share.stores, aurora_i18n::tr!("inventory-sorter-by-store"));
                checkbox(ui, p, &mut share.folders, aurora_i18n::tr!("inventory-sorter-folders"));
                checkbox(ui, p, &mut share.kinds, aurora_i18n::tr!("inventory-sorter-kinds"));
                if !prefs.excluded.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let mut never = false;
                        let label = format!("{} ({})", aurora_i18n::tr!("inventory-sorter-excluded"), prefs.excluded.len());
                        ui.add_enabled_ui(false, |ui| checkbox(ui, p, &mut never, label));
                        ui.label(RichText::new(aurora_i18n::tr!("inventory-share-private")).small().color(p.muted));
                    });
                }
            });
        crate::ui::widgets::divider(ui, p);
        ui.horizontal(|ui| {
            copy = action_button(
                ui,
                p,
                "copy-filled",
                aurora_i18n::tr!("inventory-share-copy"),
                ActionKind::Primary,
                Vec2::new(0.0, 28.0),
            )
            .clicked();
            if !st.sorter.status.is_empty() {
                ui.label(RichText::new(&st.sorter.status).small().color(p.muted));
            }
        });
    });
    if copy && let Some(share) = st.sorter.share.as_ref() {
        ctx.copy_text(sorter::share::export(prefs, share));
        st.sorter.status = aurora_i18n::tr!("inventory-share-copied").to_owned();
    }
    if !open {
        st.sorter.share = None;
        st.sorter.shared_for = None;
    }
}

/// Import…: paste a shared text, tick what to take, then add or replace.
fn import_window(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &mut sorter::Preferences) {
    use crate::ui::widgets::{ActionKind, action_button};
    use sorter::share::OptionKind;
    let replace_open = st.sorter.confirm == Some(Confirm::Replace);
    let Some(import) = st.sorter.import.as_mut() else { return };
    if import.text != import.read {
        import.found = sorter::share::import(&import.text);
        import.pick = sorter::share::Pick::new(&import.found, prefs);
        import.read.clone_from(&import.text);
        import.done.clear();
    }
    let mut open = true;
    let (mut add, mut replace) = (false, false);
    let window_id = format!("inventory_sorter_import_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-import-title"),
        ctx.content_rect().center() - Vec2::new(210.0, 260.0),
        Vec2::new(420.0, 520.0),
    )
    .steady_size();
    window.min_size = Vec2::new(320.0, 340.0);
    window.show(ctx, p, &mut open, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-import-hint")).small().color(p.muted));
        // a long paste scrolls in its box instead of pushing the list away
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_import_text")
            .max_height(110.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut import.text)
                        // while Replace asks, the text it replaces with
                        // stays the one the user saw
                        .interactive(!replace_open)
                        .char_limit(sorter::share::MAX_TEXT)
                        .hint_text(sorter::share::HEADER)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
            });
        ui.add_space(4.0);
        let (dup_rules, dup_hints) = import.found.duplicates(prefs);
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_import_list")
            .max_height((ui.available_height() - 64.0).max(60.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let found = &import.found;
                if !import.text.trim().is_empty() && found.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-import-nothing")).color(p.muted));
                }
                let changed = found.defaults_changed(prefs);
                if changed > 0 {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("inventory-import-changes", count = changed))
                            .strong()
                            .color(p.warn),
                    );
                    ui.add_space(2.0);
                }
                // the options it changes first: what the user had, what the
                // file sets
                for (which, now, then) in found.changes(prefs) {
                    let (ticked, name) = match which {
                        OptionKind::Base => (&mut import.pick.base, aurora_i18n::tr!("inventory-sorter-base")),
                        OptionKind::Stores => (&mut import.pick.stores, aurora_i18n::tr!("inventory-sorter-by-store")),
                        OptionKind::Folders => (&mut import.pick.folders, aurora_i18n::tr!("inventory-import-folders")),
                        OptionKind::Kinds => (&mut import.pick.kinds, aurora_i18n::tr!("inventory-sorter-kinds")),
                    };
                    let none = aurora_i18n::tr!("inventory-import-none").to_string();
                    let shown = |s: &str| if s.is_empty() { none.clone() } else { s.to_owned() };
                    let label = RichText::new(format!("{name}:  {}  →  {}", shown(&now), shown(&then))).color(p.warn);
                    row(ui, p, ticked, label, false);
                }
                for (index, rule) in found.rules.iter().enumerate() {
                    let label = if rule.pre_trash {
                        // towards the Trash: red, and never ticked for the user
                        RichText::new(format!(
                            "{}  ·  {}",
                            rule_line(rule),
                            aurora_i18n::tr!("inventory-import-pre-trash")
                        ))
                        .color(p.danger)
                    } else {
                        RichText::new(rule_line(rule))
                    };
                    row(ui, p, &mut import.pick.rules[index], label, dup_rules[index]);
                }
                for (index, hint) in found.hints.iter().enumerate() {
                    row(
                        ui,
                        p,
                        &mut import.pick.hints[index],
                        RichText::new(hint_line(hint)),
                        dup_hints[index],
                    );
                }
                if found.skipped > 0 {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("inventory-import-skipped", count = found.skipped))
                            .small()
                            .color(p.warn),
                    );
                }
            });
        crate::ui::widgets::divider(ui, p);
        let pick = &import.pick;
        let option_ticked = import.found.changes(prefs).iter().any(|(which, _, _)| match which {
            OptionKind::Base => pick.base,
            OptionKind::Stores => pick.stores,
            OptionKind::Folders => pick.folders,
            OptionKind::Kinds => pick.kinds,
        });
        let any = pick.rules.iter().chain(&pick.hints).any(|t| *t) || option_ticked;
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 28.0);
            add = ui
                .add_enabled_ui(any && !replace_open, |ui| {
                    action_button(ui, p, "plus", aurora_i18n::tr!("inventory-import-add"), ActionKind::Primary, size)
                })
                .inner
                .clicked();
            replace = ui
                .add_enabled_ui(any && !replace_open, |ui| {
                    action_button(
                        ui,
                        p,
                        "replace-filled",
                        aurora_i18n::tr!("inventory-import-replace"),
                        ActionKind::Flat,
                        size,
                    )
                })
                .inner
                .clicked();
        });
        if !import.done.is_empty() {
            ui.label(RichText::new(&import.done).small().color(p.muted));
        }
    });
    if add {
        import.found.apply(prefs, &import.pick, false);
        import.done = aurora_i18n::tr!("inventory-import-done").to_owned();
        // what was added now reads "already in yours"
        import.pick = sorter::share::Pick::new(&import.found, prefs);
    }
    if replace {
        st.sorter.confirm = Some(Confirm::Replace);
    }
    if !open {
        st.sorter.import = None;
    }
}

/// An imported item and its box; one the user already has is shown
/// unticked and greyed, "already in yours".
fn row(ui: &mut egui::Ui, p: &Palette, ticked: &mut bool, label: RichText, mine: bool) {
    ui.horizontal(|ui| {
        if mine {
            let mut never = false;
            ui.add_enabled_ui(false, |ui| crate::ui::widgets::checkbox(ui, p, &mut never, label));
            ui.label(RichText::new(aurora_i18n::tr!("inventory-import-already")).small().color(p.muted));
        } else {
            crate::ui::widgets::checkbox(ui, p, ticked, label);
        }
    });
}

/// Clean up's Trash: one delete request of at most [`TRASH_BATCH`] entries
/// at a time, through the ordinary delete path (the Trash's own rules decide
/// what goes), the next one once the inventory is free again.
fn send_trash_batch(
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    worn: &HashSet<Uuid>,
    actions: &mut Vec<InvAction>,
    free: bool,
) {
    let free = free && !actions.iter().any(|a| matches!(a, InvAction::Delete(_) | InvAction::Drop(_)));
    let now = Instant::now();
    if st.sorter.trash.as_ref().is_some_and(|trash| !trash.begun) {
        // a Trash run starts: its journal, and a snapshot of the layout first
        let run = st.sorter.journal.begin(sorter::journal::RunKind::Trash, unix_now());
        st.sorter.snapshot(inv);
        if let Some(trash) = st.sorter.trash.as_mut() {
            trash.begun = true;
            trash.run = Some(run);
        }
    }
    let Some(trash) = st.sorter.trash.as_mut() else { return };
    // The batch on its way: done once the app took it and its delete is
    // over; never taken (another change went first), back in the queue.
    if let Some((batch, frames, taken)) = trash.in_flight.as_mut() {
        *frames += 1;
        *taken |= trash.request.is_some();
        // (answered: the request's reply came, `finish_sorter`)
        let deleting = *taken && !trash.answered;
        let stuck = deleting && trash.taken_at.get_or_insert(now).elapsed() > 4 * TRASH_SHOW_LIMIT;
        if stuck {
            log::warn!(
                "auto-sort: a Trash batch's delete did not end in {} s",
                (4 * TRASH_SHOW_LIMIT).as_secs()
            );
        }
        if *taken && !deleting || stuck {
            trash.taken_at = None;
            trash.request = None;
            trash.answered = false;
            // what the delete really did is read from the inventory: each
            // entry counts once it shows in the Trash (below), a little
            // after the request if need be
            trash
                .unsure
                .extend(batch.drain(..).map(|(id, from)| (id, from, now + TRASH_SHOW_LIMIT)));
            trash.unsure_seen = None;
            trash.in_flight = None;
            trash.next_at = Some(now + PACE);
        } else if !*taken && *frames > 2 {
            // not seen taken: the inventory window may have cleared its
            // pending delete before this frame (closed, another row clicked).
            // A delete is never sent twice: its entries are waited for in the
            // Trash like a finished one, and those that never show count as
            // failed (Clean up can be run again)
            trash
                .unsure
                .extend(batch.drain(..).map(|(id, from)| (id, from, now + TRASH_SHOW_LIMIT)));
            trash.unsure_seen = None;
            trash.in_flight = None;
            trash.next_at = Some(now + PACE);
        } else {
            return;
        }
    }
    // entries seen in the Trash: sent and journaled (for Undo); still
    // elsewhere once the wait is over: the delete left them, a failure
    let due = trash.unsure.iter().any(|(_, _, until)| now >= *until);
    if !trash.unsure.is_empty() && (due || trash.unsure_seen != Some(inv.generation)) {
        trash.unsure_seen = Some(inv.generation);
        let bin = rules::system(inv, 14);
        let (went, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut trash.unsure)
            .into_iter()
            .partition(|(id, _, _)| in_bin(inv, bin, *id));
        // each entry is waited for on its own: one left by its delete
        // counts as failed once its own wait is over
        let (over, waiting): (Vec<_>, Vec<_>) = rest.into_iter().partition(|(_, _, until)| now >= *until);
        if !over.is_empty() {
            log::warn!("auto-sort: a Trash batch left {} entries where they were", over.len());
            trash.failed += over.len();
        }
        trash.unsure = waiting;
        if !went.is_empty() {
            trash.sent += went.len();
            let run = trash.run;
            st.sorter.journal.record_in(
                run,
                went.iter().map(|(id, from, _)| sorter::journal::Entry {
                    id: *id,
                    folder: inv.folders.contains_key(id),
                    from: *from,
                    name: rules::name(inv, *id),
                    to: None,
                }),
            );
            st.sorter.unsaved += 1;
        }
    }
    let Some(trash) = st.sorter.trash.as_mut() else { return };
    // nothing left: the summary, whatever the inventory is doing (a delete
    // that never ends no longer holds the run)
    let over = trash.queue.is_empty() && trash.unsure.is_empty() && trash.in_flight.is_none();
    // one batch at a time: the next goes once the last one's entries are
    // all seen in the Trash or counted
    if !over && (!free || trash.next_at.is_some_and(|at| now < at) || !trash.unsure.is_empty()) {
        return;
    }
    if over {
        st.sorter.status = if trash.failed > 0 {
            aurora_i18n::tr!(
                "inventory-sorter-done-failed",
                count = trash.sent,
                skipped = trash.skipped,
                failed = trash.failed
            )
        } else if trash.skipped > 0 {
            aurora_i18n::tr!("inventory-sorter-done-skipped", count = trash.sent, skipped = trash.skipped)
        } else {
            aurora_i18n::tr!("inventory-cleanup-trash-done", count = trash.sent)
        };
        st.sorter.trash = None;
        st.sorter.cleanup = None;
        st.sorter.skipped.clear();
        // written now, like a sort's end: its window may close this frame
        st.sorter.save_progress();
        st.sorter.journal.release(st.sorter.window);
        return;
    }
    let take = trash.queue.len().min(TRASH_BATCH);
    let picked: Vec<(Uuid, Uuid)> = trash.queue.drain(..take).collect();
    // Re-checked now, like a sort's moves: still where the confirmed list
    // saw it, nowhere hands-off, nothing worn or locked. What changed is
    // skipped and reported, never sent.
    let holds_worn = crate::world::inventory::actions::worn_ancestors(inv, worn);
    // (a folder listed in My folders since, or holding one, never goes whole)
    let own = sorter::own_folders(inv, &prefs.sorter);
    let batch: Vec<Uuid> = picked
        .iter()
        .filter(|(id, from)| !own.contains(id) && sorter::trash_ready(inv, *id, *from, &prefs.sorter, &prefs.protected, &holds_worn))
        .map(|(id, _)| *id)
        .collect();
    trash.skipped += picked.len() - batch.len();
    if batch.is_empty() {
        return;
    }
    trash.in_flight = Some((picked.iter().filter(|(id, _)| batch.contains(id)).copied().collect(), 0, false));
    let change = rules::system(inv, 14)
        .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
        .and_then(|trash| rules::move_selection(inv, &batch, trash, &prefs.protected, worn));
    match change {
        // the ordinary delete path, never tied to the window's selection: a
        // long Clean up leaves the user's selection and scroll alone
        Ok(change) => actions.push(InvAction::Delete(Box::new(DeleteRequest::new(inv, st, &batch, change).detached()))),
        Err(reason) => {
            // this batch cannot go: counted as failed, the run goes on to
            // its summary
            log::warn!("auto-sort: a Trash batch could not be sent: {reason}");
            st.sorter.status = reason;
            if let Some(trash) = st.sorter.trash.as_mut() {
                trash.failed += batch.len();
                trash.in_flight = None;
                trash.next_at = Some(now + PACE);
            }
        }
    }
}

/// Each entry with the folder it is in now, where the run put it: Undo
/// leaves those the user moves on afterwards.
fn placed(inv: &Inventory, entries: Vec<sorter::journal::Entry>) -> Vec<sorter::journal::Entry> {
    entries
        .into_iter()
        .map(|mut e| {
            if e.to.is_none() {
                e.to = crate::world::inventory::actions::parent(inv, e.id);
            }
            e
        })
        .collect()
}

/// `id` is in the Trash (at any depth).
fn in_bin(inv: &Inventory, bin: Option<Uuid>, id: Uuid) -> bool {
    use crate::world::inventory::actions;
    actions::parent(inv, id).is_some_and(|p| bin.is_some_and(|t| p == t || actions::under(inv, p, t)))
}

/// What waits in Pre-Trash (its entries and the folder), and the step that
/// sends it to the Trash.
fn pre_trash_card(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    state: &mut State,
    waiting: (&[Uuid], Option<Uuid>),
    busy: bool,
    loading: bool,
) {
    let (waiting, waiting_in) = waiting;
    crate::ui::widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "trash-filled", p.warn, 16.0);
            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-pre-trash-waiting", count = waiting.len())).strong());
        });
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-sorter-pre-trash-hint"))
                .small()
                .color(p.muted),
        );
        if loading {
            ui.label(
                RichText::new(aurora_i18n::tr!("inventory-sorter-pre-trash-loading"))
                    .small()
                    .color(p.muted),
            );
        }
        ui.add_space(4.0);
        let send = ui.add_enabled_ui(!busy && !loading, |ui| {
            crate::ui::widgets::action_button(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("inventory-sorter-pre-trash-empty"),
                crate::ui::widgets::ActionKind::Danger,
                Vec2::new(0.0, 26.0),
            )
        });
        if send.inner.clicked()
            && let Some(folder) = waiting_in
        {
            state.confirm = Some(Confirm::PreTrash(
                folders_first(inv, waiting.iter().map(|id| (*id, folder)).collect()).into(),
            ));
        }
        purge_buttons(ui, p, inv, state, busy || loading);
    });
}

/// Empty in small steps, for Pre-Trash and the Trash: for a folder too
/// large to be emptied at once (the server refuses a huge Trash's purge).
fn purge_buttons(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, busy: bool) {
    let folders = [
        (sorter::pre_trash(inv), sorter::pre_trash_name()),
        (rules::system(inv, 14), aurora_i18n::tr!("inventory-purge-trash-name").to_owned()),
    ];
    ui.add_space(4.0);
    ui.label(RichText::new(aurora_i18n::tr!("inventory-purge-hint")).small().color(p.muted));
    ui.horizontal_wrapped(|ui| {
        for (folder, name) in folders {
            let Some(folder) = folder else { continue };
            let Some(&(_, _, loaded, count)) = state.purge_seen.get(&folder) else {
                continue;
            };
            // (nothing it could delete: no button)
            if loaded && count == 0 {
                continue;
            }
            let label = if count == usize::MAX {
                aurora_i18n::tr!("inventory-purge-unreadable", folder = name.as_str())
            } else if loaded {
                aurora_i18n::tr!("inventory-purge-steps", folder = name.as_str(), count = count)
            } else {
                aurora_i18n::tr!("inventory-purge-loading", folder = name.as_str())
            };
            let go = ui.add_enabled_ui(loaded && !busy && state.purge.is_none() && state.trash.is_none(), |ui| {
                crate::ui::widgets::action_button(
                    ui,
                    p,
                    "trash-x-filled",
                    &label,
                    crate::ui::widgets::ActionKind::Danger,
                    Vec2::new(0.0, 26.0),
                )
            });
            if go.inner.clicked() {
                state.confirm = Some(Confirm::Purge(folder, name, count));
            }
        }
    });
}

/// A Trash confirmation's entries, folders first (once, not per frame);
/// the order is otherwise kept.
fn folders_first(inv: &Inventory, mut ids: Vec<(Uuid, Uuid)>) -> Vec<(Uuid, Uuid)> {
    ids.sort_by_key(|(id, _)| !inv.folders.contains_key(id));
    ids
}

/// Rows shown in a Trash confirmation; the rest are counted.
const DOOMED_ROWS: usize = 300;

/// The entries a Trash confirmation sends, each with the folder it is in,
/// folders first, in a scrolled list.
fn doomed_list(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, ids: &[(Uuid, Uuid)]) {
    // folders come first in `ids` (the riskiest entries, `folders_first`
    // when the confirmation was made): the first rows of that order
    let mut rows: Vec<(bool, String, String)> = ids
        .iter()
        .take(DOOMED_ROWS)
        .map(|(id, from)| {
            let (folder, name) = match (inv.folders.get(id), inv.items.get(id)) {
                (Some(f), _) => (true, f.info.name.clone()),
                (None, Some(it)) => (false, it.name.clone()),
                _ => (false, aurora_i18n::tr!("inventory-sorter-trash-gone").to_owned()),
            };
            let place = inv.folders.get(from).map(|f| f.info.name.clone()).unwrap_or_default();
            (folder, name, place)
        })
        .collect();
    rows.sort_by_key(|(folder, name, _)| (!*folder, name.to_lowercase()));
    egui::ScrollArea::vertical()
        .id_salt("inventory_sorter_doomed")
        .max_height(220.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (folder, name, place) in &rows {
                ui.horizontal(|ui| {
                    let icon = if *folder { "folder-filled" } else { "file-description-filled" };
                    crate::ui::widgets::glyph(ui, icon, p.muted, 13.0);
                    ui.label(RichText::new(name).small().color(p.ink));
                    if !place.is_empty() {
                        ui.label(RichText::new(place).small().color(p.muted));
                    }
                });
            }
            if ids.len() > DOOMED_ROWS {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-sorter-trash-more", count = ids.len() - DOOMED_ROWS))
                        .small()
                        .color(p.muted),
                );
            }
        });
}

/// The confirmations: a whole-inventory sort (with its "I understand" box)
/// and Pre-Trash to Trash. Escape or Cancel closes them, nothing moved.
fn confirm(ctx: &egui::Context, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let Some(asked) = st.sorter.confirm.clone() else { return };
    let moves = st.sorter.preview.as_ref().map_or(0, |(_, _, plan)| plan.moves.len());
    let mut answer = None;
    crate::ui::widgets::Modal::new(egui::Id::unique(("inventory_sorter_confirm", st.ui_id))).show(ctx, |ui| {
        use crate::ui::widgets::{ActionKind, action_button};
        ui.set_max_width(400.0);
        let size = Vec2::new(0.0, 26.0);
        match &asked {
            Confirm::Whole => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-sorter-whole-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("inventory-sorter-whole-text", count = moves));
                ui.add_space(8.0);
                crate::ui::widgets::checkbox(ui, p, &mut st.sorter.understood, aurora_i18n::tr!("inventory-sorter-whole-check"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = aurora_i18n::tr!("inventory-sorter-start", count = moves);
                    let go = ui.add_enabled_ui(st.sorter.understood, |ui| {
                        action_button(ui, p, "check", &label, ActionKind::Primary, size)
                    });
                    if go.inner.clicked() {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            }
            Confirm::PreTrash(ids) | Confirm::Trash(ids) => {
                let (title, text) = if matches!(asked, Confirm::Trash(_)) {
                    (
                        aurora_i18n::tr!("inventory-cleanup-trash-title"),
                        aurora_i18n::tr!("inventory-cleanup-trash-text", count = ids.len()),
                    )
                } else {
                    (
                        aurora_i18n::tr!("inventory-sorter-pre-trash-title"),
                        aurora_i18n::tr!("inventory-sorter-pre-trash-text", count = ids.len()),
                    )
                };
                ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
                ui.add_space(8.0);
                ui.label(text);
                ui.add_space(6.0);
                // what goes, by name: nothing reaches the Trash unseen
                doomed_list(ui, p, inv, ids);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let free = st.pending.is_none() && st.pending_delete.is_none();
                    let go = ui.add_enabled_ui(free, |ui| {
                        action_button(
                            ui,
                            p,
                            "trash-filled",
                            aurora_i18n::tr!("inventory-sorter-pre-trash-empty"),
                            ActionKind::Danger,
                            size,
                        )
                    });
                    if go.inner.clicked() {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            }
            Confirm::Purge(_, name, count) => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-purge-title", folder = name.as_str()))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("inventory-purge-text", folder = name.as_str(), count = *count));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        p,
                        "trash-x-filled",
                        aurora_i18n::tr!("inventory-purge-go"),
                        ActionKind::Danger,
                        size,
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            }
            Confirm::Unlock(_, name) => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-sorter-unlock-title", folder = name.as_str()))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("inventory-sorter-unlock-text", folder = name.as_str()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        p,
                        "lock-open",
                        aurora_i18n::tr!("inventory-sorter-unlock-confirm"),
                        ActionKind::Danger,
                        size,
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            }
            Confirm::Replace => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-import-replace-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!(
                    "inventory-import-replace-text",
                    rules = prefs.sorter.rules.len(),
                    hints = prefs.sorter.store_hints.len()
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        p,
                        "replace-filled",
                        aurora_i18n::tr!("inventory-import-replace"),
                        ActionKind::Danger,
                        size,
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
    });
    let Some(answer) = answer else { return };
    st.sorter.confirm = None;
    if !answer {
        return;
    }
    match asked {
        Confirm::Whole => start(&mut st.sorter, inv, prefs),
        Confirm::Unlock(Some(id), _) => {
            if !prefs.sorter.unlocked.contains(&id) {
                prefs.sorter.unlocked.push(id);
            }
        }
        Confirm::Unlock(None, _) => prefs.sorter.include_lost_found = true,
        Confirm::Replace => {
            if let Some(import) = st.sorter.import.as_mut() {
                import.found.apply(&mut prefs.sorter, &import.pick, true);
                import.done = aurora_i18n::tr!("inventory-import-done").to_owned();
                import.pick = sorter::share::Pick::new(&import.found, &prefs.sorter);
            }
        }
        Confirm::Trash(ids) => {
            st.sorter.status.clear();
            if busy_elsewhere(&mut st.sorter) {
                return;
            }
            st.sorter.journal.claim(st.sorter.window);
            st.sorter.trash = Some(TrashJob::new(ids.to_vec()));
        }
        Confirm::Purge(folder, name, _) => {
            st.sorter.status.clear();
            if busy_elsewhere(&mut st.sorter) {
                return;
            }
            // (never a worn item: take it off first, as the Legacy Viewer's
            // Empty Trash asks)
            let worn_inside = st.sorter.worn_now.iter().any(|w| rules::under(inv, *w, folder));
            if worn_inside {
                st.sorter.status = aurora_i18n::tr!("inventory-purge-worn", folder = name.as_str());
                return;
            }
            let pieces: VecDeque<_> = sorter::purge::purge_pieces(inv, folder).into();
            let total = pieces.iter().map(|p| p.entries(inv)).sum();
            // one run at a time per account, like a sort or a Trash run
            st.sorter.journal.claim(st.sorter.window);
            st.sorter.purge = Some(PurgeJob {
                folder,
                pieces,
                total,
                done: 0,
                failed: 0,
                in_flight: None,
                request: None,
                answered: None,
                paused: false,
                next_at: None,
                stopping: false,
                refused_in_row: 0,
            });
        }
        Confirm::PreTrash(ids) => {
            // the same batches as Clean up's Trash: at most 200 per delete
            // request, each entry re-checked before it goes
            st.sorter.status.clear();
            if busy_elsewhere(&mut st.sorter) {
                return;
            }
            st.sorter.journal.claim(st.sorter.window);
            st.sorter.trash = Some(TrashJob::new(ids.to_vec()));
        }
    }
}

/// Another inventory window is running Auto-sort on this account: nothing
/// starts here meanwhile (its status says so).
fn busy_elsewhere(state: &mut State) -> bool {
    // the history still being read: nothing starts before it is (a run
    // would be journaled into a copy about to be replaced)
    if state.journal.with(|j| j.loading) {
        state.status = aurora_i18n::tr!("inventory-sorter-history-loading").into();
        return true;
    }
    let busy = state.journal.busy_elsewhere(state.window);
    if busy {
        state.status = aurora_i18n::tr!("inventory-sorter-busy-elsewhere").into();
    }
    busy
}

fn start(state: &mut State, inv: &Inventory, prefs: &InventoryPreferences) {
    if busy_elsewhere(state) {
        return;
    }
    let Some((_, options, plan)) = state.preview.as_ref() else { return };
    // never a run from a preview of other options, or while a newer one is
    // being made: the user sorts what they saw
    if state.planning.is_some() || *options != prefs.sorter {
        state.status = aurora_i18n::tr!("inventory-sorter-updating").into();
        return;
    }
    // a whole sort waits for every folder it opens to be read: a run never
    // starts on part of the inventory
    if matches!(state.source, Some(Source::Whole))
        // (a folder read since the plan was made counts too: the plan
        // made after it is waited for)
        && plan
            .unfetched
            .iter()
            .any(|id| inv.folders.get(id).is_some_and(|f| f.state != FetchState::Failed))
    {
        state.status = aurora_i18n::tr!("inventory-sorter-updating").into();
        return;
    }
    state.status.clear();
    if plan.moves.is_empty() {
        state.status = aurora_i18n::tr!("inventory-sorter-nothing").into();
        return;
    }
    // the preview's moves become the run's queue, not a copy of them (a
    // whole inventory's are hundreds of megabytes); the preview is made
    // again once the run ends
    let moves = state.preview.take().map(|(_, _, plan)| plan.moves).unwrap_or_default();
    let mut job = Job::new(moves.into(), sorter::base_name_in(inv, &prefs.sorter));
    job.base_id = sorter::base_folder(inv, &prefs.sorter);
    if let Some(Source::Saved(_, restoring)) = &state.source {
        job.restoring = Some(*restoring);
        if *restoring == Restoring::Undo {
            job.undoes = state.undoing.take();
        }
        if let (Restoring::Resume, Some(unfinished)) = (restoring, &state.unfinished) {
            // the sort folder as it is named now, else as it was
            let renamed = unfinished
                .base_id
                .and_then(|id| inv.folders.get(&id))
                .filter(|f| f.info.parent == inv.root && sorter::folder_open(inv, f.info.id))
                .map(|f| f.info.name.clone());
            job.base = renamed.unwrap_or_else(|| unfinished.base.clone());
            job.base_id = unfinished.base_id;
            job.undoes = unfinished.undoes;
            job.earlier_runs.clone_from(&unfinished.runs);
        }
    }
    // the account's run is this window's from now (not the next frame)
    state.journal.claim(state.window);
    state.job = Some(job);
}

/// A folder path that fits the width left: whole, else its first folder,
/// `…` and its last (`My Inventory / … / Velmora Hex Dress`); the label
/// shortens the end of what still does not fit.
fn fit_path(ui: &egui::Ui, p_ink: egui::Color32, path: &str) -> String {
    let width = ui.available_width();
    let wide = |text: &str| {
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.painter().layout_no_wrap(text.to_owned(), font, p_ink).size().x
    };
    if wide(path) <= width {
        return path.to_owned();
    }
    let parts: Vec<&str> = path.split(" / ").collect();
    match parts.as_slice() {
        [first, .., last] if parts.len() > 2 => format!("{first} / … / {last}"),
        _ => path.to_owned(),
    }
}

/// What is sorted: a folder (with Change…) or the selected entries.
fn source_row(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, source: &Source, busy: bool) {
    crate::ui::widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "folder-filled", p.violet_light, 16.0);
            // (the buttons first, on the right: a long path takes what is
            // left and is shortened, never runs under them)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-change")).clicked() {
                        state.picker = Some(Picker::new(Pick::Source));
                    }
                    if *source != Source::Whole
                        && crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-whole"))
                            .on_hover_text(aurora_i18n::tr!("inventory-sorter-whole-tip"))
                            .clicked()
                    {
                        state.set_source(Source::Whole);
                    }
                });
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-source")).small().color(p.muted));
                        let text = match source {
                            Source::Folder(id) => sorter::folder_path(inv, *id),
                            Source::Entries(ids) => aurora_i18n::tr!("inventory-sorter-selection", count = ids.len()),
                            Source::Whole => aurora_i18n::tr!("inventory-sorter-whole").to_owned(),
                            Source::Saved(_, Restoring::Undo) => aurora_i18n::tr!("inventory-sorter-undo-source").to_owned(),
                            Source::Saved(_, Restoring::Resume) => aurora_i18n::tr!("inventory-sorter-resume-source").to_owned(),
                            Source::Saved(_, Restoring::Snapshot(at)) => {
                                let date = snapshot_date(*at);
                                aurora_i18n::tr!("inventory-sorter-snapshot-source", date = date.as_str())
                            }
                        };
                        let shown = fit_path(ui, p.ink, &text);
                        let label = ui.add(egui::Label::new(RichText::new(shown.as_str()).strong()).truncate());
                        if shown != text {
                            label.on_hover_text(text);
                        }
                    });
                });
            });
        });
        let intro = if *source == Source::Whole {
            aurora_i18n::tr!("inventory-sorter-whole-intro")
        } else {
            aurora_i18n::tr!("inventory-sorter-intro")
        };
        ui.label(RichText::new(intro).small().color(p.muted));
    });
}

fn footer_row(ui: &mut egui::Ui, p: &Palette, state: &State, inventory_free: bool, go: &mut bool, stop: &mut bool, pause: &mut bool) {
    ui.add_space(4.0);
    if let Some(job) = &state.purge {
        let fraction = if job.total == 0 { 1.0 } else { job.done as f32 / job.total as f32 };
        let text = aurora_i18n::tr!("inventory-purge-progress", done = job.done, total = job.total);
        ui.horizontal(|ui| {
            crate::ui::widgets::progress(
                ui,
                p,
                "inventory_purge",
                fraction,
                (ui.available_width() - 210.0).max(60.0),
                &text,
                None,
            );
            let label = if job.paused {
                aurora_i18n::tr!("inventory-sorter-resume")
            } else {
                aurora_i18n::tr!("inventory-sorter-pause")
            };
            *pause = crate::ui::widgets::action_button(
                ui,
                p,
                if job.paused { "player-play-filled" } else { "player-pause-filled" },
                label,
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(96.0, 26.0),
            )
            .clicked();
            *stop = crate::ui::widgets::action_button(
                ui,
                p,
                "player-stop-filled",
                aurora_i18n::tr!("common-stop"),
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(96.0, 26.0),
            )
            .clicked();
        });
        return;
    }
    if let Some(trash) = &state.trash {
        let fraction = if trash.total == 0 {
            1.0
        } else {
            trash.sent as f32 / trash.total as f32
        };
        let text = aurora_i18n::tr!("inventory-cleanup-trash-progress", done = trash.sent, total = trash.total);
        ui.horizontal(|ui| {
            crate::ui::widgets::progress(
                ui,
                p,
                "inventory_cleanup_trash",
                fraction,
                (ui.available_width() - 110.0).max(60.0),
                &text,
                None,
            );
            *stop = crate::ui::widgets::action_button(
                ui,
                p,
                "player-stop-filled",
                aurora_i18n::tr!("common-stop"),
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(96.0, 26.0),
            )
            .clicked();
        });
        return;
    }
    if state.tab == CLEANUP && state.job.is_none() {
        // Clean up has its own buttons in the tab
        if !state.status.is_empty() {
            ui.label(RichText::new(&state.status).color(p.muted));
        }
        return;
    }
    if let Some(job) = &state.job {
        // confirmed moves only; the time left from the pace so far
        let handled = job.done + job.skipped + job.failed;
        let fraction = if job.total == 0 { 1.0 } else { handled as f32 / job.total as f32 };
        let mut text = aurora_i18n::tr!("inventory-sorter-progress", done = job.done, total = job.total);
        if state.held {
            text = aurora_i18n::tr!("inventory-sorter-held", done = job.done, total = job.total);
        } else if job.paused {
            text = aurora_i18n::tr!("inventory-sorter-paused", done = job.done, total = job.total);
        } else if let Some(left) = job.eta() {
            text.push_str(&format!(
                " · {}",
                aurora_i18n::tr!("inventory-sorter-eta", minutes = left.as_secs().div_ceil(60))
            ));
        }
        ui.horizontal(|ui| {
            crate::ui::widgets::progress(
                ui,
                p,
                "inventory_sorter_progress",
                fraction,
                (ui.available_width() - 220.0).max(60.0),
                &text,
                None,
            );
            let (icon, label) = if job.paused {
                ("player-play-filled", aurora_i18n::tr!("inventory-sorter-resume"))
            } else {
                ("player-pause-filled", aurora_i18n::tr!("inventory-sorter-pause"))
            };
            *pause = crate::ui::widgets::action_button(ui, p, icon, label, crate::ui::widgets::ActionKind::Flat, Vec2::new(104.0, 26.0))
                .clicked();
            *stop = crate::ui::widgets::action_button(
                ui,
                p,
                "player-stop-filled",
                aurora_i18n::tr!("common-stop"),
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(96.0, 26.0),
            )
            .clicked();
        });
        return;
    }
    let moves = state.preview.as_ref().map_or(0, |(_, _, plan)| plan.moves.len());
    ui.horizontal(|ui| {
        let label = aurora_i18n::tr!("inventory-sorter-start", count = moves);
        *go = ui
            .add_enabled_ui(moves > 0 && !state.submitted && inventory_free, |ui| {
                crate::ui::widgets::primary_button_sized(ui, p, &label, Vec2::new(170.0, 28.0))
            })
            .inner
            .clicked();
        if !state.status.is_empty() {
            ui.label(RichText::new(&state.status).color(p.muted));
        }
    });
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => aurora_i18n::tr!("inventory-cleanup-month-1"),
        2 => aurora_i18n::tr!("inventory-cleanup-month-2"),
        3 => aurora_i18n::tr!("inventory-cleanup-month-3"),
        4 => aurora_i18n::tr!("inventory-cleanup-month-4"),
        5 => aurora_i18n::tr!("inventory-cleanup-month-5"),
        6 => aurora_i18n::tr!("inventory-cleanup-month-6"),
        7 => aurora_i18n::tr!("inventory-cleanup-month-7"),
        8 => aurora_i18n::tr!("inventory-cleanup-month-8"),
        9 => aurora_i18n::tr!("inventory-cleanup-month-9"),
        10 => aurora_i18n::tr!("inventory-cleanup-month-10"),
        11 => aurora_i18n::tr!("inventory-cleanup-month-11"),
        _ => aurora_i18n::tr!("inventory-cleanup-month-12"),
    }
}

/// "1 March 2020", in the catalogue's order (UTC, like the cutoff).
fn date_text(at: i64) -> String {
    use chrono::Datelike;
    let Some(day) = chrono::DateTime::from_timestamp(at, 0).map(|t| t.date_naive()) else {
        return String::new();
    };
    // as text, so no number formatting groups the year's digits
    let (number, year) = (day.day().to_string(), day.year().to_string());
    aurora_i18n::tr!(
        "inventory-cleanup-date",
        day = number.as_str(),
        month = month_name(day.month()),
        year = year.as_str()
    )
}

/// A row's acquired date, `2014-06-03`.
fn short_date(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or_else(String::new, |t| t.date_naive().format("%Y-%m-%d").to_string())
}

/// The item kinds as two columns of boxes, with All and None: Clean up's
/// filter and the sort's.
fn kinds_grid(ui: &mut egui::Ui, p: &Palette, id: &str, kinds: &mut Vec<Kind>) {
    use crate::ui::widgets::{checkbox, flat_button};
    ui.horizontal(|ui| {
        if flat_button(ui, p, aurora_i18n::tr!("inventory-cleanup-all")).clicked() {
            *kinds = Kind::ALL.to_vec();
        }
        if flat_button(ui, p, aurora_i18n::tr!("inventory-cleanup-none")).clicked() {
            kinds.clear();
        }
    });
    egui::Grid::new(id).num_columns(2).spacing([18.0, 2.0]).show(ui, |ui| {
        for (index, kind) in Kind::ALL.iter().enumerate() {
            let was = kinds.contains(kind);
            let mut on = was;
            checkbox(ui, p, &mut on, kind.label());
            if on != was {
                if on {
                    kinds.push(*kind);
                    kinds.sort();
                } else {
                    kinds.retain(|k| k != kind);
                }
            }
            if index % 2 == 1 {
                ui.end_row();
            }
        }
    });
}

/// Rows drawn per group: the rest is counted, so a large cleanup stays
/// smooth (the groups open one at a time).
const CLEANUP_ROWS: usize = 300;

/// The Clean up tab: how old (chips, or a month and a year), which kinds,
/// where, then the list by kind with a box per row (unchecked rows stay) and
/// the two ways out: Pre-Trash, or the Trash after a confirmation.
#[allow(clippy::too_many_arguments)]
fn cleanup_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    state: &mut State,
    options: &mut sorter::Preferences,
    has_folder: bool,
    busy: bool,
    free: bool,
) {
    use crate::ui::widgets::{ActionKind, action_button, checkbox, collapsing, section_header, segmented, switch_labeled};
    let base = options.base_name();
    let cleanup = &mut options.cleanup;
    ui.add_enabled_ui(!busy, |ui| {
        let mut find = usize::from(cleanup.find == Find::Duplicates);
        let changed = ui.push_id("inventory_cleanup_find", |ui| {
            segmented(
                ui,
                p,
                &mut find,
                &[
                    (aurora_i18n::tr!("inventory-cleanup-find-old"), true),
                    (aurora_i18n::tr!("inventory-cleanup-find-duplicates"), true),
                ],
            )
        });
        if changed.inner {
            cleanup.find = if find == 1 { Find::Duplicates } else { Find::Old };
        }
    });
    let duplicates = cleanup.find == Find::Duplicates;
    let intro = if duplicates {
        aurora_i18n::tr!("inventory-cleanup-duplicates-intro")
    } else {
        aurora_i18n::tr!("inventory-cleanup-intro")
    };
    ui.label(RichText::new(intro).small().color(p.muted));
    ui.add_space(6.0);
    ui.add_enabled_ui(!busy, |ui| {
        if !duplicates {
            section_header(ui, p, "calendar-filled", aurora_i18n::tr!("inventory-cleanup-when"));
            const AGES: [Age; 5] = [Age::OneYear, Age::TwoYears, Age::ThreeYears, Age::FiveYears, Age::Custom];
            let mut age = AGES.iter().position(|a| *a == cleanup.age).unwrap_or(0);
            if segmented(
                ui,
                p,
                &mut age,
                &[
                    (aurora_i18n::tr!("inventory-cleanup-age-1"), true),
                    (aurora_i18n::tr!("inventory-cleanup-age-2"), true),
                    (aurora_i18n::tr!("inventory-cleanup-age-3"), true),
                    (aurora_i18n::tr!("inventory-cleanup-age-5"), true),
                    (aurora_i18n::tr!("inventory-cleanup-age-custom"), true),
                ],
            ) {
                cleanup.age = AGES[age.min(AGES.len() - 1)];
            }
            let now = chrono::Utc::now();
            if cleanup.age == Age::Custom {
                use chrono::Datelike;
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("inventory_cleanup_month")
                        .selected_text(month_name(cleanup.month))
                        .show_ui(ui, |ui| {
                            for month in 1..=12 {
                                ui.selectable_value(&mut cleanup.month, month, month_name(month));
                            }
                        });
                    egui::ComboBox::from_id_salt("inventory_cleanup_year")
                        .selected_text(cleanup.year.to_string())
                        .show_ui(ui, |ui| {
                            for year in (2003..=now.year()).rev() {
                                ui.selectable_value(&mut cleanup.year, year, year.to_string());
                            }
                        });
                });
            }
            if let Some(cutoff) = cleanup.cutoff(now.timestamp()) {
                ui.add_space(2.0);
                let date = date_text(cutoff);
                ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-before", date = date.as_str())).strong());
            }
            ui.add_space(8.0);
        }
        section_header(ui, p, "filter-filled", aurora_i18n::tr!("inventory-cleanup-types"));
        kinds_grid(ui, p, "inventory_cleanup_kinds", &mut cleanup.kinds);
        ui.add_space(8.0);
        section_header(ui, p, "folder-filled", aurora_i18n::tr!("inventory-cleanup-where"));
        let mut place = usize::from(!cleanup.whole && has_folder);
        let moved = ui.push_id("inventory_cleanup_where", |ui| {
            segmented(
                ui,
                p,
                &mut place,
                &[
                    (aurora_i18n::tr!("inventory-cleanup-whole"), true),
                    (aurora_i18n::tr!("inventory-cleanup-this-folder"), has_folder),
                ],
            )
        });
        if moved.inner {
            cleanup.whole = place == 0;
        }
        // Each switch under its own id: its slide follows that id, and an
        // id counted from the controls above (the custom date's dropdowns
        // appearing) would change and replay the slide.
        ui.push_id("inventory_cleanup_include_sorted", |ui| {
            switch_labeled(
                ui,
                p,
                &mut cleanup.include_sorted,
                aurora_i18n::tr!("inventory-cleanup-include-sorted", folder = base.as_str()),
            )
        });
        if duplicates {
            ui.push_id("inventory_cleanup_no_copy_duplicates", |ui| {
                switch_labeled(
                    ui,
                    p,
                    &mut cleanup.no_copy_duplicates,
                    aurora_i18n::tr!("inventory-cleanup-duplicates-no-copy"),
                )
            });
            ui.push_id("inventory_cleanup_landmark_duplicates", |ui| {
                switch_labeled(
                    ui,
                    p,
                    &mut cleanup.landmark_duplicates,
                    aurora_i18n::tr!("inventory-cleanup-duplicates-landmarks"),
                )
            });
            ui.push_id("inventory_cleanup_notecard_duplicates", |ui| {
                switch_labeled(
                    ui,
                    p,
                    &mut cleanup.notecard_duplicates,
                    aurora_i18n::tr!("inventory-cleanup-duplicates-notecards"),
                )
            });
            ui.label(
                RichText::new(aurora_i18n::tr!("inventory-cleanup-duplicates-whole-hint"))
                    .small()
                    .color(p.muted),
            );
        } else {
            ui.push_id("inventory_cleanup_empty_folders", |ui| {
                switch_labeled(
                    ui,
                    p,
                    &mut cleanup.empty_folders,
                    aurora_i18n::tr!("inventory-cleanup-empty-folders"),
                )
            });
        }
        ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-never")).small().color(p.muted));
    });
    purge_buttons(ui, p, inv, state, busy);
    crate::ui::widgets::divider(ui, p);
    let State {
        cleanup: list,
        skipped,
        cleanup_action,
        tab,
        ..
    } = state;
    let Some((.., plan)) = list.as_ref() else { return };
    // (the default folders still locked are never looked at: said, with
    // the way to Options, so a short list is never taken for the whole)
    if plan.locked_items > 0 {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("inventory-cleanup-locked", count = plan.locked_items))
                    .small()
                    .color(p.muted),
            );
            if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-cleanup-locked-options")).clicked() {
                *tab = OPTIONS;
            }
        });
        ui.add_space(4.0);
    }
    if !plan.unfetched.is_empty() {
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-cleanup-loading", count = plan.unfetched.len()))
                .small()
                .color(p.muted),
        );
    }
    let date = date_text(plan.cutoff);
    if plan.rows.is_empty() {
        let nothing = if duplicates {
            aurora_i18n::tr!("inventory-cleanup-duplicates-nothing").to_string()
        } else {
            aurora_i18n::tr!("inventory-cleanup-nothing", date = date.as_str()).to_string()
        };
        ui.label(RichText::new(nothing).color(p.muted));
        return;
    }
    // what goes: all rows but the few unticked (never a pass over every row
    // per frame)
    let kept: Vec<&sorter::CleanupRow> = plan.rows_of(skipped).collect();
    let going_count = plan.rows.len() - kept.len();
    let items: usize = plan.items - kept.iter().map(|r| r.items).sum::<usize>();
    let summary = if duplicates {
        aurora_i18n::tr!("inventory-cleanup-duplicates-summary", count = going_count, items = plan.duplicated)
    } else {
        aurora_i18n::tr!("inventory-cleanup-summary", count = items, date = date.as_str())
    };
    ui.label(RichText::new(summary).strong());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let size = Vec2::new(0.0, 26.0);
        let enabled = going_count > 0 && !busy && free;
        let label = aurora_i18n::tr!("inventory-cleanup-to-pre-trash", count = going_count);
        if ui
            .add_enabled_ui(enabled, |ui| {
                action_button(ui, p, "trash-filled", &label, ActionKind::Primary, size)
            })
            .inner
            .clicked()
        {
            *cleanup_action = Some(CleanupAction::PreTrash);
        }
        // (copies go to Pre-Trash only, to be looked at: never straight
        // to the Trash)
        let label = aurora_i18n::tr!("inventory-cleanup-to-trash", count = going_count);
        if !duplicates
            && ui
                .add_enabled_ui(enabled, |ui| action_button(ui, p, "trash-filled", &label, ActionKind::Danger, size))
                .inner
                .clicked()
        {
            *cleanup_action = Some(CleanupAction::Trash);
        }
    });
    ui.add_space(6.0);
    // folders first (they go whole), then each kind in the filter's order:
    // the plan's rows are in that order, each group one slice
    for rows in plan.groups() {
        let label = match (rows[0].folder, rows[0].kind) {
            (true, _) if rows[0].items > 0 => aurora_i18n::tr!("inventory-cleanup-folders").to_owned(),
            (true, _) => aurora_i18n::tr!("inventory-cleanup-empty-group").to_owned(),
            (false, Some(kind)) => kind.label().to_owned(),
            (false, None) => continue,
        };
        let title = format!("{label}  ({})", rows.len());
        collapsing(ui, p, ("inventory_cleanup_group", &label), title, false, |ui| {
            for row in rows.iter().take(CLEANUP_ROWS) {
                ui.horizontal(|ui| {
                    let mut on = !skipped.contains(&row.id);
                    checkbox(ui, p, &mut on, &row.name).on_hover_text(aurora_i18n::tr!("inventory-cleanup-keep-tip"));
                    if on {
                        skipped.remove(&row.id);
                    } else {
                        skipped.insert(row.id);
                    }
                    let mut when = short_date(row.created);
                    if row.folder {
                        when = format!("{when} · {}", aurora_i18n::tr!("inventory-cleanup-folder-items", count = row.items));
                    }
                    ui.label(RichText::new(when).small().color(p.muted));
                });
            }
            if rows.len() > CLEANUP_ROWS {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-cleanup-more", count = rows.len() - CLEANUP_ROWS))
                        .small()
                        .color(p.muted),
                );
            }
        });
    }
}

fn options(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, options: &mut sorter::Preferences) {
    crate::ui::widgets::section_header(ui, p, "folder-filled", aurora_i18n::tr!("inventory-sorter-base"));
    ui.add(
        egui::TextEdit::singleline(&mut options.base)
            .char_limit(63)
            .hint_text(aurora_i18n::tr!("inventory-sorter-default-base"))
            .desired_width(240.0),
    );
    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-base-hint")).small().color(p.muted));
    ui.add_space(10.0);
    crate::ui::widgets::switch_labeled(ui, p, &mut options.folders, aurora_i18n::tr!("inventory-sorter-folders"));
    ui.add_space(4.0);
    crate::ui::widgets::switch_labeled(ui, p, &mut options.stores, aurora_i18n::tr!("inventory-sorter-by-store"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-by-store-hint"))
            .small()
            .color(p.muted),
    );
    ui.add_space(10.0);
    crate::ui::widgets::section_header(ui, p, "filter-filled", aurora_i18n::tr!("inventory-sorter-kinds"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-kinds-hint"))
            .small()
            .color(p.muted),
    );
    kinds_grid(ui, p, "inventory_sorter_kinds", &mut options.kinds);
    ui.add_space(10.0);
    store_hints(ui, p, options);
    ui.add_space(10.0);
    crate::ui::widgets::section_header(ui, p, "eye-off", aurora_i18n::tr!("inventory-sorter-excluded"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-excluded-hint"))
            .small()
            .color(p.muted),
    );
    ui.add_space(4.0);
    let mut remove = None;
    for (index, id) in options.excluded.iter().enumerate() {
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "eye-off", p.muted, 14.0);
            let text = if inv.folders.contains_key(id) {
                sorter::folder_path(inv, *id)
            } else {
                aurora_i18n::tr!("inventory-sorter-folder-missing").to_owned()
            };
            ui.label(text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::ui::widgets::icon_button(ui, p, "x", aurora_i18n::tr!("inventory-sorter-exclude-remove"), true) {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        options.excluded.remove(index);
    }
    if crate::ui::widgets::action_button(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("inventory-sorter-exclude-add"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
    )
    .clicked()
    {
        state.picker = Some(Picker::new(Pick::Exclude));
    }
    ui.add_space(10.0);
    own_folders(ui, p, inv, state, options);
    ui.add_space(10.0);
    default_folders(ui, p, inv, state, options);
    ui.add_space(10.0);
    let worn = std::mem::take(&mut state.worn_now);
    undo_and_snapshots(ui, p, inv, state, &worn, options);
    state.worn_now = worn;
    ui.add_space(10.0);
    crate::ui::widgets::section_header(ui, p, "share", aurora_i18n::tr!("inventory-share-section"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-share-section-hint"))
            .small()
            .color(p.muted),
    );
    ui.horizontal(|ui| {
        use crate::ui::widgets::{ActionKind, action_button};
        let size = Vec2::new(0.0, 26.0);
        if action_button(ui, p, "share", aurora_i18n::tr!("inventory-share-open"), ActionKind::Flat, size).clicked() {
            state.status.clear();
            state.share = Some(sorter::share::Share::all(options));
        }
        if action_button(
            ui,
            p,
            "file-import",
            aurora_i18n::tr!("inventory-import-open"),
            ActionKind::Flat,
            size,
        )
        .clicked()
        {
            state.import = Some(ImportWindow::default());
        }
    });
}

/// The default folders Auto-sort keeps its hands off, and the user's
/// exceptions: Lost and Found's switch and an Allow per unlockable folder,
/// each behind a warning that names the folder. The Trash, Marketplace,
/// outfits, the Current Outfit, Received Items, Favorites, the Library and
/// the viewer folders are not listed: they can never be allowed.
fn default_folders(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, options: &mut sorter::Preferences) {
    crate::ui::widgets::section_header(ui, p, "lock-filled", aurora_i18n::tr!("inventory-sorter-defaults"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-defaults-hint"))
            .small()
            .color(p.muted),
    );
    let lost = crate::world::inventory::actions::system(inv, 16);
    let mut lost_on = options.include_lost_found;
    ui.push_id("inventory_sorter_lost_found", |ui| {
        crate::ui::widgets::switch_labeled(ui, p, &mut lost_on, aurora_i18n::tr!("inventory-sorter-lost-found"))
            .on_hover_text(aurora_i18n::tr!("inventory-sorter-lost-found-tip"));
    });
    if lost_on != options.include_lost_found {
        if lost_on {
            // turned on only after its warning
            let name = lost.map_or_else(
                || aurora_i18n::tr!("inventory-sorter-lost-found").to_owned(),
                |id| rules::name(inv, id),
            );
            state.confirm = Some(Confirm::Unlock(None, name));
        } else {
            options.include_lost_found = false;
        }
    }
    for id in sorter::unlockable_folders(inv) {
        if Some(id) == lost {
            continue;
        }
        let name = rules::name(inv, id);
        let allowed = options.unlocked.contains(&id);
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(
                ui,
                if allowed { "lock-open" } else { "lock-filled" },
                if allowed { p.warn } else { p.muted },
                14.0,
            );
            ui.label(&name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if allowed {
                    if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-unlock-remove")).clicked() {
                        options.unlocked.retain(|u| *u != id);
                    }
                } else if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-unlock")).clicked() {
                    state.confirm = Some(Confirm::Unlock(Some(id), name.clone()));
                }
            });
        });
    }
}

/// A snapshot's or a run's date, "2026-11-15 12:30" (UTC).
fn snapshot_date(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M").to_string())
}

/// How long the whole inventory is waited for before a layout to share is
/// saved with what is loaded (a folder the server never gives).
const SHARE_LOAD_LIMIT: std::time::Duration = std::time::Duration::from_secs(600);

/// How often the read's progress is looked at.
const SHARE_CHECK: std::time::Duration = std::time::Duration::from_millis(500);

/// How long the read may stand still (no folder read, failed or asked)
/// before the layout is saved with what is there.
const SHARE_STALL: std::time::Duration = std::time::Duration::from_secs(60);

/// The layout to share: the whole inventory read first (a few folders a
/// frame, like the inventory's own search), then written to the Desktop on
/// another thread. Folder and item names and where each is, never an asset
/// id; nothing moves.
fn share_step(inv: &mut Inventory, state: &mut State, actions: &mut Vec<InvAction>) {
    match state.shared.as_ref() {
        Some(Share::Loading {
            since,
            moved,
            now,
            checked,
            ..
        }) => {
            let (since, moved, now) = (*since, *moved, *now);
            if !now && checked.is_some_and(|at| at.elapsed() < SHARE_CHECK) {
                return;
            }
            // what is left goes to the inventory's background walk, as the
            // Legacy Viewer's search does (whole subtrees in its batches,
            // its own pace and retries), never a read a folder a frame
            // (only once the walk is idle: it reads the rest itself, at its
            // own pace and with its own retries)
            if !inv.walk_busy {
                let tops = sorter::unread_tops(inv, 400);
                if !tops.is_empty() {
                    actions.push(InvAction::FetchSubtrees(tops));
                }
            }
            // Done once nothing is left to come (every folder read, failed
            // or too large for the server), when nothing moved on for a
            // while, at the limit, or when the user asks. The same trees the
            // read walks: a folder it never asks for (the Trash's) never
            // holds it.
            let progress = sorter::read_progress(inv);
            // (a walk still reading, or retrying, is progress: the stall
            // counts only once nothing is on its way)
            let moved =
                if inv.walk_busy || matches!(state.shared.as_ref(), Some(Share::Loading { progress: last, .. }) if *last != progress) {
                    Instant::now()
                } else {
                    moved
                };
            let settled = progress.pending == 0 && !inv.walk_busy;
            let done = settled || now || moved.elapsed() >= SHARE_STALL || since.elapsed() >= SHARE_LOAD_LIMIT;
            if !done {
                state.shared = Some(Share::Loading {
                    since,
                    progress,
                    moved,
                    now,
                    checked: Some(Instant::now()),
                });
                return;
            }
            let at = unix_now();
            // (read on its own thread from a copy: the frame only copies,
            // and the journal's own writes never wait behind a 50 MB file)
            let copy = sorter::snapshot(inv);
            let when = chrono::DateTime::from_timestamp(at, 0).map_or_else(
                || at.to_string(),
                |t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H-%M").to_string(),
            );
            let dirs = directories::UserDirs::new();
            let path = sorter::journal::share_path(
                dirs.as_ref().and_then(|d| d.desktop_dir().map(std::path::Path::to_path_buf)),
                dirs.as_ref().and_then(|d| d.document_dir().map(std::path::Path::to_path_buf)),
                std::env::current_dir().unwrap_or_default(),
                &when,
            );
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let snapshot = sorter::journal::Snapshot::take_to_share(&copy, at);
                let unread = snapshot.unread.len();
                let result = sorter::journal::save_shared(&path, &snapshot)
                    .map(|()| (path, unread))
                    .map_err(|e| e.to_string());
                let _ = tx.send(result);
            });
            state.shared = Some(Share::Saving(rx));
        }
        Some(Share::Saving(rx)) => {
            // (a writer that died is a failure, never a wait forever)
            let result = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("the writer stopped".to_owned())),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                state.shared = Some(match result {
                    Ok((path, unread)) => Share::Saved(path, unread),
                    Err(e) => {
                        log::warn!("auto-sort: the layout to share could not be saved: {e}");
                        Share::Failed(e)
                    }
                });
            }
        }
        _ => {}
    }
}

/// The button that saves the layout to share, and where it went.
fn share_row(ui: &mut egui::Ui, p: &Palette, state: &mut State) {
    use crate::ui::widgets::{ActionKind, action_button};
    let size = Vec2::new(0.0, 26.0);
    let working = matches!(state.shared, Some(Share::Loading { .. } | Share::Saving(_)));
    ui.horizontal(|ui| {
        if ui
            .add_enabled_ui(!working, |ui| {
                action_button(
                    ui,
                    p,
                    "device-floppy-filled",
                    aurora_i18n::tr!("inventory-sorter-share-layout"),
                    ActionKind::Flat,
                    size,
                )
            })
            .inner
            .clicked()
        {
            let now = Instant::now();
            state.shared = Some(Share::Loading {
                since: now,
                progress: sorter::ReadProgress::default(),
                moved: now,
                now: false,
                checked: None,
            });
        }
    });
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-share-layout-hint"))
            .small()
            .color(p.muted),
    );
    let mut reveal = None;
    let mut save_now = false;
    match &state.shared {
        Some(Share::Loading { progress, .. }) => {
            ui.label(
                RichText::new(aurora_i18n::tr!("inventory-sorter-share-layout-loading"))
                    .small()
                    .color(p.muted),
            );
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "inventory-sorter-share-layout-progress",
                    read = progress.read,
                    total = progress.total,
                    failed = progress.failed
                ))
                .small()
                .color(p.muted),
            );
            if action_button(
                ui,
                p,
                "device-floppy-filled",
                aurora_i18n::tr!("inventory-sorter-share-layout-now"),
                ActionKind::Flat,
                size,
            )
            .clicked()
            {
                save_now = true;
            }
        }
        Some(Share::Saving(_)) => {
            ui.label(
                RichText::new(aurora_i18n::tr!("inventory-sorter-share-layout-saving"))
                    .small()
                    .color(p.muted),
            );
        }
        Some(Share::Saved(path, unread)) => {
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "inventory-sorter-share-layout-saved",
                    path = path.display().to_string()
                ))
                .small(),
            );
            if *unread > 0 {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-sorter-share-layout-unread", count = *unread))
                        .small()
                        .color(p.muted),
                );
            }
            if action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("inventory-sorter-share-layout-show"),
                ActionKind::Flat,
                size,
            )
            .clicked()
            {
                reveal = Some(path.clone());
            }
        }
        Some(Share::Failed(error)) => {
            ui.label(
                RichText::new(aurora_i18n::tr!("inventory-sorter-share-layout-failed", error = error.as_str()))
                    .small()
                    .color(p.warn),
            );
        }
        None => {}
    }
    if save_now && let Some(Share::Loading { now, .. }) = state.shared.as_mut() {
        *now = true;
    }
    if let Some(path) = reveal
        && let Some(folder) = path.parent()
    {
        crate::snapshot::gallery::reveal(folder, Some(&path));
    }
}

/// Undo the last run, put a snapshot's layout back, resume a stopped run,
/// take a snapshot: each opens its preview; nothing moves before Sort.
fn undo_and_snapshots(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    state: &mut State,
    worn: &HashSet<Uuid>,
    options: &sorter::Preferences,
) {
    use crate::ui::widgets::{ActionKind, action_button};
    let size = Vec2::new(0.0, 26.0);
    let protected = state.protected_now.clone();
    let busy = state.busy();
    crate::ui::widgets::section_header(ui, p, "history", aurora_i18n::tr!("inventory-sorter-undo-section"));
    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-undo-hint")).small().color(p.muted));
    if let Some(unfinished) = &state.unfinished {
        let label = aurora_i18n::tr!("inventory-sorter-resume-run", count = unfinished.moves.len());
        // (never while a run goes: it would only change the status)
        if ui
            .add_enabled_ui(!busy, |ui| {
                action_button(ui, p, "player-play-filled", &label, ActionKind::Primary, size)
            })
            .inner
            .clicked()
        {
            // gone: the run was ended by another window (never read again on
            // the frame, only looked for)
            let there = state
                .account_dir()
                .is_some_and(|dir| sorter::journal::unfinished_path(&dir).exists());
            if there {
                let moves = std::sync::Arc::new(unfinished.moves.clone());
                state.set_source(Source::Saved(moves, Restoring::Resume));
                state.tab = PREVIEW;
            } else {
                state.unfinished = None;
            }
        }
    }
    // an entry counts while it is known, or while the folder the run put it
    // in is not loaded yet (it is likely there, unseen)
    let unloaded = |id: Option<Uuid>| {
        id.is_some_and(|to| {
            inv.folders
                .get(&to)
                .is_some_and(|f| matches!(f.state, FetchState::Unknown | FetchState::Fetching))
        })
    };
    let key = (inv.generation, state.journal.with(|j| j.version()));
    let label = match &state.undo_label {
        Some((at, label)) if *at == key => *label,
        _ => {
            let exists = |e: &sorter::journal::Entry| inv.items.contains_key(&e.id) || inv.folders.contains_key(&e.id) || unloaded(e.to);
            let label = state
                .journal
                .with(|j| j.last_undoable(exists).map(|run| (run.at, run.entries.len())));
            state.undo_label = Some((key, label));
            label
        }
    };
    match label {
        Some((at, count)) => {
            let date = snapshot_date(at);
            let label = aurora_i18n::tr!("inventory-sorter-undo-run", count = count, date = date.as_str());
            // (never while a run goes)
            let clicked = ui
                .add_enabled_ui(!busy, |ui| action_button(ui, p, "arrow-back-up", &label, ActionKind::Flat, size))
                .inner
                .clicked();
            if !busy && (clicked || state.undo_waiting.is_some_and(|seen| seen != inv.generation)) {
                // the folders the runs put entries in are loaded first, else
                // an entry not known yet would be counted as staying
                // (gathered from the journal once, at the click; then only
                // the folders still loading are looked at again)
                if clicked {
                    state.undo_folders = state.journal.with(|j| {
                        j.runs
                            .iter()
                            .filter(|run| run.undoes.is_none())
                            .flat_map(|run| run.entries.iter().filter_map(|e| e.to))
                            .collect::<HashSet<_>>()
                            .into_iter()
                            .collect()
                    });
                }
                state.undo_folders.retain(|to| unloaded(Some(*to)));
                let loading: Vec<Uuid> = state
                    .undo_folders
                    .iter()
                    .copied()
                    .filter(|to| inv.folders.get(to).is_some_and(|f| f.state == FetchState::Unknown))
                    .collect();
                if state.undo_folders.is_empty() {
                    state.undo_waiting = None;
                    start_undo(inv, state, worn, &protected);
                } else {
                    if clicked {
                        state.status = aurora_i18n::tr!("inventory-sorter-undo-loading").into();
                    }
                    state.undo_waiting = Some(inv.generation);
                    state.to_request.extend(loading);
                }
            }
        }
        None => {
            // (nothing yet while the history is still being read)
            let text = if state.journal.with(|j| j.loading) {
                aurora_i18n::tr!("inventory-sorter-history-loading")
            } else {
                aurora_i18n::tr!("inventory-sorter-undo-none")
            };
            ui.label(RichText::new(text).small().color(p.muted));
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-snapshots")).strong());
        if action_button(
            ui,
            p,
            "camera-filled",
            aurora_i18n::tr!("inventory-sorter-snapshot-take"),
            ActionKind::Flat,
            size,
        )
        .clicked()
        {
            state.snapshot(inv);
        }
    });
    share_row(ui, p, state);
    let mut restore = None;
    for (at, path) in &state.snapshots {
        ui.horizontal(|ui| {
            ui.label(snapshot_date(*at));
            if ui
                .add_enabled_ui(!busy && state.snapshot_loading.is_none(), |ui| {
                    crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-snapshot-restore"))
                })
                .inner
                .clicked()
            {
                restore = Some((*at, path.clone()));
            }
        });
    }
    // the file is read on a thread; the restore is planned once it is
    if let Some((at, path)) = restore {
        let (send, read) = std::sync::mpsc::channel();
        let file = path.clone();
        let load = move || {
            let _ = send.send(sorter::journal::load::<sorter::journal::Snapshot>(&file));
        };
        match std::thread::Builder::new().name("auto-sort snapshot".into()).spawn(load) {
            Ok(_) => {
                state.snapshot_loading = Some((at, path, read));
                state.status = aurora_i18n::tr!("inventory-sorter-snapshot-loading").to_owned();
            }
            Err(error) => log::warn!("auto-sort: the snapshot could not be read ({error})"),
        }
    }
    let arrived = state.snapshot_loading.as_ref().and_then(|(_, _, read)| read.try_recv().ok());
    if let Some(result) = arrived
        && let Some((at, path, _)) = state.snapshot_loading.take()
    {
        match result {
            Ok(snapshot) => {
                let account = state.account.unwrap_or_default();
                let by_runs = state.journal.with(|j| sorter::journal::moved_since(j, snapshot.at));
                match sorter::journal::restore_moves(inv, &snapshot, account, &by_runs, &protected, worn, options) {
                    Ok((moves, summary)) => {
                        state.set_source(Source::Saved(std::sync::Arc::new(moves), Restoring::Snapshot(at)));
                        state.tab = PREVIEW;
                        // what stays, said before anything moves
                        state.status = aurora_i18n::tr!(
                            "inventory-sorter-restore-summary",
                            new = summary.new_since,
                            hand = summary.by_hand,
                            trashed = summary.trashed,
                            gone = summary.gone + summary.folder_gone,
                            kept = summary.kept
                        );
                    }
                    Err(sorter::journal::OtherAccount) => {
                        state.status = aurora_i18n::tr!("inventory-sorter-snapshot-other-account").to_owned();
                    }
                }
            }
            Err(error) => {
                log::warn!("auto-sort: the snapshot {} could not be read: {error}", path.display());
                state.status = aurora_i18n::tr!("inventory-sorter-snapshot-unreadable").to_owned();
            }
        }
    }
}

/// The folders the user lists as their own: opened and sorted, never moved
/// whole, whatever their name. One no longer allowed (made a default
/// folder, moved into the sort folder) is shown as such and ignored.
fn own_folders(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, options: &mut sorter::Preferences) {
    crate::ui::widgets::section_header(ui, p, "folder-open-filled", aurora_i18n::tr!("inventory-sorter-own"));
    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-own-hint")).small().color(p.muted));
    ui.add_space(4.0);
    let mut remove = None;
    for (index, id) in options.own.iter().enumerate() {
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "folder-open-filled", p.muted, 14.0);
            let text = if !inv.folders.contains_key(id) {
                aurora_i18n::tr!("inventory-sorter-folder-missing").to_owned()
            } else if sorter::own_allowed(inv, options, *id) {
                sorter::folder_path(inv, *id)
            } else {
                aurora_i18n::tr!("inventory-sorter-own-refused", path = sorter::folder_path(inv, *id))
            };
            ui.label(text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::ui::widgets::icon_button(ui, p, "x", aurora_i18n::tr!("inventory-sorter-own-remove"), true) {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        options.own.remove(index);
    }
    if crate::ui::widgets::action_button(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("inventory-sorter-own-add"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
    )
    .clicked()
    {
        state.picker = Some(Picker::new(Pick::Own));
    }
}

/// Store hints: a store whose names say nothing of what it sells, and the
/// category its items go to.
fn store_hints(ui: &mut egui::Ui, p: &Palette, options: &mut sorter::Preferences) {
    use sorter::{Category, StoreHint};
    crate::ui::widgets::section_header(ui, p, "building-store", aurora_i18n::tr!("inventory-sorter-hints"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-hints-hint"))
            .small()
            .color(p.muted),
    );
    let mut remove = None;
    for (index, hint) in options.store_hints.iter_mut().enumerate() {
        ui.push_id(("inventory_sorter_hint", index), |ui| {
            // a creator hint: everything that creator made, whatever its
            // name says; the store folder is then optional
            let by_creator = hint.creator.is_some();
            if by_creator {
                let name = match hint.creator_name.trim() {
                    "" => aurora_i18n::tr!("inventory-sorter-creator-unknown").to_string(),
                    name => name.to_owned(),
                };
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-sorter-by-creator", name = name))
                        .small()
                        .strong(),
                );
            }
            ui.horizontal(|ui| {
                let hint_text = if by_creator {
                    aurora_i18n::tr!("inventory-sorter-hint-store-optional")
                } else {
                    aurora_i18n::tr!("inventory-sorter-hint-store")
                };
                let store = ui.add(
                    egui::TextEdit::singleline(&mut hint.store)
                        .char_limit(64)
                        .hint_text(hint_text)
                        .desired_width(150.0),
                );
                if by_creator {
                    store.on_hover_text(aurora_i18n::tr!("inventory-sorter-hint-store-optional-tip"));
                }
                ui.label(RichText::new("→").color(p.muted));
                egui::ComboBox::from_id_salt("hint_category")
                    .selected_text(hint.category.folder())
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for category in Category::ALL {
                            ui.selectable_value(&mut hint.category, category, category.folder());
                        }
                    });
                if crate::ui::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-sorter-hint-remove"), true) {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        options.store_hints.remove(index);
    }
    if crate::ui::widgets::action_button(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("inventory-sorter-hint-add"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
    )
    .clicked()
    {
        options.store_hints.push(StoreHint::default());
    }
}

/// The user's rules as cards: when, how the words meet the name, and where
/// to: a new folder under the sort folder or one of the user's own.
fn rules_editor(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, options: &mut sorter::Preferences) {
    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-rules-hint")).color(p.muted));
    ui.add_space(6.0);
    let mut remove = None;
    let mut swap = None;
    let count = options.rules.len();
    let base = options.base_name();
    for (index, rule) in options.rules.iter_mut().enumerate() {
        ui.push_id(("inventory_sorter_rule", index), |ui| {
            crate::ui::widgets::surface(ui, p, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    crate::ui::widgets::switch(ui, p, &mut rule.enabled).on_hover_text(aurora_i18n::tr!("inventory-sorter-rule-enabled"));
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-rule-number", n = index + 1)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ui::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-sorter-rule-remove"), true) {
                            remove = Some(index);
                        }
                        if crate::ui::widgets::icon_button(
                            ui,
                            p,
                            "arrow-down",
                            aurora_i18n::tr!("inventory-sorter-rule-down"),
                            index + 1 < count,
                        ) {
                            swap = Some((index, index + 1));
                        }
                        if crate::ui::widgets::icon_button(ui, p, "arrow-up", aurora_i18n::tr!("inventory-sorter-rule-up"), index > 0) {
                            swap = Some((index - 1, index));
                        }
                    });
                });
                ui.add_enabled_ui(rule.enabled, |ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("inventory-sorter-rule-words"))
                            .small()
                            .color(p.muted),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut rule.words)
                            .char_limit(120)
                            .hint_text(aurora_i18n::tr!("inventory-sorter-rule-words-hint"))
                            .desired_width(f32::INFINITY),
                    );
                    let mut mode = match rule.mode {
                        Match::All => 0,
                        Match::Any => 1,
                        Match::Phrase => 2,
                    };
                    if crate::ui::widgets::segmented(
                        ui,
                        p,
                        &mut mode,
                        &[
                            (aurora_i18n::tr!("inventory-sorter-match-all"), true),
                            (aurora_i18n::tr!("inventory-sorter-match-any"), true),
                            (aurora_i18n::tr!("inventory-sorter-match-phrase"), true),
                        ],
                    ) {
                        rule.mode = [Match::All, Match::Any, Match::Phrase][mode.min(2)];
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-rule-to")).small().color(p.muted));
                    let mut kind = if rule.pre_trash { 2 } else { usize::from(rule.target.is_some()) };
                    if crate::ui::widgets::segmented(
                        ui,
                        p,
                        &mut kind,
                        &[
                            (aurora_i18n::tr!("inventory-sorter-to-new"), true),
                            (aurora_i18n::tr!("inventory-sorter-to-mine"), true),
                            (aurora_i18n::tr!("inventory-sorter-to-pre-trash"), true),
                        ],
                    ) {
                        rule.pre_trash = kind == 2;
                        if rule.pre_trash {
                            rule.target = None;
                            rule.folder.clear();
                        }
                        match kind {
                            1 => state.picker = Some(Picker::new(Pick::Rule(index))),
                            0 => rule.target = None,
                            _ => {}
                        }
                    }
                    match rule.target {
                        _ if rule.pre_trash => {
                            // where it looks: the whole folder sorted, at any
                            // depth, with or without the sort folder's tree
                            ui.add_space(4.0);
                            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-look-in")).small().color(p.muted));
                            let mut look = usize::from(rule.skip_sorted);
                            if crate::ui::widgets::segmented(
                                ui,
                                p,
                                &mut look,
                                &[
                                    (aurora_i18n::tr!("inventory-sorter-look-all"), true),
                                    (aurora_i18n::tr!("inventory-sorter-look-unsorted"), true),
                                ],
                            ) {
                                rule.skip_sorted = look == 1;
                            }
                            let hint = if rule.skip_sorted {
                                aurora_i18n::tr!("inventory-sorter-rule-pre-trash-unsorted-hint", folder = base.as_str())
                            } else {
                                aurora_i18n::tr!("inventory-sorter-rule-pre-trash-hint").into()
                            };
                            ui.horizontal(|ui| {
                                crate::ui::widgets::glyph(ui, "trash-filled", p.warn, 14.0);
                                ui.add(egui::Label::new(RichText::new(hint).small().color(p.muted)).wrap());
                            });
                        }
                        Some(target) => {
                            ui.horizontal(|ui| {
                                let text = if inv.folders.contains_key(&target) {
                                    sorter::folder_path(inv, target)
                                } else {
                                    aurora_i18n::tr!("inventory-sorter-folder-missing").to_owned()
                                };
                                crate::ui::widgets::glyph(ui, "folder-filled", p.violet_light, 14.0);
                                ui.label(RichText::new(text).strong());
                                if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-change")).clicked() {
                                    state.picker = Some(Picker::new(Pick::Rule(index)));
                                }
                            });
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{base} /")).color(p.muted));
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.folder)
                                        .char_limit(200)
                                        .hint_text(aurora_i18n::tr!("inventory-sorter-rule-folder"))
                                        .desired_width(f32::INFINITY),
                                );
                            });
                        }
                    }
                    if rule.enabled && !rule.usable() {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("inventory-sorter-rule-incomplete"))
                                .small()
                                .color(p.warn),
                        );
                    }
                });
            });
        });
        ui.add_space(6.0);
    }
    if let Some(index) = remove {
        options.rules.remove(index);
        state.picker = None;
    }
    if let Some((a, b)) = swap {
        options.rules.swap(a, b);
        state.picker = None;
    }
    if crate::ui::widgets::action_button(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("inventory-sorter-rule-add"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(150.0, 26.0),
    )
    .clicked()
    {
        options.rules.push(Rule::default());
    }
}

/// Destination groups drawn in the preview (an Undo of a whole inventory has
/// one per folder, tens of thousands): the rest are counted.
const PREVIEW_GROUPS: usize = 300;

/// Rows drawn in one preview group: the rest is counted.
const PREVIEW_ROWS: usize = 300;

/// The preview: how many entries move and where, each by name, then what
/// stays and why. Nothing moves before it is shown.
fn preview(ui: &mut egui::Ui, p: &Palette, plan: &Plan) {
    if plan.moves.is_empty() && plan.unknown.is_empty() && plan.kept == 0 {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-nothing")).color(p.muted));
    }
    if !plan.moves.is_empty() {
        let mut summary = aurora_i18n::tr!("inventory-sorter-moves", count = plan.moves.len()).to_owned();
        let unpackers = plan.unpacker_count;
        if unpackers > 0 {
            summary.push_str(", ");
            summary.push_str(&aurora_i18n::tr!("inventory-sorter-unpackers", count = unpackers));
        }
        ui.label(RichText::new(summary).strong());
    }
    if plan.kept > 0 {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-kept", count = plan.kept)).color(p.muted));
    }
    ui.add_space(4.0);
    let groups = plan.grouped();
    let shown = groups.len().min(PREVIEW_GROUPS);
    for (label, rows) in groups.into_iter().take(shown) {
        let title = format!("{label}  ({})", rows.len());
        crate::ui::widgets::collapsing(ui, p, ("inventory_sorter_group", &label), title, false, |ui| {
            for m in rows.iter().take(PREVIEW_ROWS) {
                ui.label(RichText::new(&m.name).color(if m.unpacker { p.violet_light } else { p.ink }));
            }
            more(ui, p, rows.len());
        });
    }
    if plan.group_count() > shown {
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-cleanup-more", count = plan.group_count() - shown))
                .small()
                .color(p.muted),
        );
    }
    if !plan.unknown.is_empty() {
        let title = format!("{}  ({})", aurora_i18n::tr!("inventory-sorter-not-recognised"), plan.unknown.len());
        crate::ui::widgets::collapsing(ui, p, "inventory_sorter_unknown", title, false, |ui| {
            for name in plan.unknown.iter().take(PREVIEW_ROWS) {
                ui.label(RichText::new(name).color(p.muted));
            }
            more(ui, p, plan.unknown.len());
        });
    }
    // what stays, by reason: worn, links, locked, excluded, protected
    for (why, names) in plan.kept_by_reason() {
        let title = format!("{}  ({})", why.label(), names.len());
        crate::ui::widgets::collapsing(ui, p, ("inventory_sorter_kept", why), title, false, |ui| {
            for (name, _) in names.iter().take(PREVIEW_ROWS) {
                ui.label(RichText::new(name).color(p.muted));
            }
            more(ui, p, names.len());
        });
    }
}

/// "…and N more" under a list cut at [`PREVIEW_ROWS`].
fn more(ui: &mut egui::Ui, p: &Palette, total: usize) {
    if total > PREVIEW_ROWS {
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-cleanup-more", count = total - PREVIEW_ROWS))
                .small()
                .color(p.muted),
        );
    }
}
impl Picker {
    fn new(pick: Pick) -> Self {
        Self {
            pick,
            filter: String::new(),
            rows: Vec::new(),
            generation: u64::MAX,
            built: None,
        }
    }
}

/// A searchable list of the user's folders, by path: the folder to sort, or
/// the folder a rule sends entries to.
fn picker(ctx: &egui::Context, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let state = &mut st.sorter;
    let Some(picker) = state.picker.as_mut() else { return };
    if picker.generation != inv.generation && picker.built.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1)) {
        let pick = picker.pick;
        let own_ok = sorter::own_check(inv, &prefs.sorter);
        let target_ok = sorter::target_check(inv);
        let mut rows: Vec<(Uuid, String, String)> = inv
            .folders
            .values()
            .filter(|f| !f.library)
            .map(|f| f.info.id)
            .filter(|id| match pick {
                Pick::Source => sorter::source_allowed(inv, *id),
                Pick::Rule(_) => target_ok(*id) && !prefs.sorter.excludes(inv, *id),
                Pick::Exclude => *id != inv.root && !sorter::untouchable(inv, *id),
                Pick::Own => own_ok(*id),
            })
            .map(|id| {
                let path = sorter::folder_path(inv, id);
                let lower = path.to_lowercase();
                (id, path, lower)
            })
            .collect();
        rows.sort_by_cached_key(|(_, path, _)| crate::natural_order::key(path));
        picker.rows = rows;
        picker.generation = inv.generation;
        picker.built = Some(Instant::now());
    }
    let mut open = true;
    let mut chosen = None;
    // the same steady size as the Auto-sort window: the list's length
    // follows the search, the window does not
    let window_id = format!("inventory_sorter_picker_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-sorter-pick-title"),
        ctx.content_rect().center() - Vec2::new(170.0, 220.0),
        Vec2::new(340.0, 440.0),
    )
    .steady_size();
    window.min_size = Vec2::new(260.0, 240.0);
    window.show(ctx, p, &mut open, |ui| {
        crate::ui::widgets::search_field(
            ui,
            p,
            &mut picker.filter,
            aurora_i18n::tr!("inventory-sorter-pick-search"),
            ui.available_width(),
        );
        ui.add_space(4.0);
        let query = picker.filter.trim().to_lowercase();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (id, path, _) in picker
                .rows
                .iter()
                .filter(|(_, _, lower)| query.is_empty() || lower.contains(&query))
                .take(400)
            {
                // the ink of the Move to picker's rows, not the accent:
                // nothing is chosen until the row is clicked
                if crate::ui::widgets::rail_row(ui, p, "folder-filled", p.ink, path, None, false, 0).clicked() {
                    chosen = Some(*id);
                }
            }
        });
    });
    let pick = picker.pick;
    // (the list may be up to a second old: the choice is checked again)
    let chosen = chosen.filter(|id| match pick {
        Pick::Source => sorter::source_allowed(inv, *id),
        Pick::Rule(_) => sorter::target_allowed(inv, *id) && !prefs.sorter.excludes(inv, *id),
        Pick::Exclude => *id != inv.root && !sorter::untouchable(inv, *id),
        Pick::Own => sorter::own_allowed(inv, &prefs.sorter, *id),
    });
    if let Some(id) = chosen {
        match pick {
            Pick::Source => state.set_source(Source::Folder(id)),
            Pick::Rule(index) => {
                if let Some(rule) = prefs.sorter.rules.get_mut(index) {
                    rule.target = Some(id);
                    rule.pre_trash = false;
                }
            }
            Pick::Exclude => {
                if !prefs.sorter.excluded.contains(&id) {
                    prefs.sorter.excluded.push(id);
                }
            }
            Pick::Own => {
                if !prefs.sorter.own.contains(&id) && sorter::own_allowed(inv, &prefs.sorter, id) {
                    prefs.sorter.own.push(id);
                }
            }
        }
        open = false;
    }
    if !open {
        state.picker = None;
    }
}

#[cfg(test)]
#[path = "sorter_tests.rs"]
mod tests;
