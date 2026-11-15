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
use crate::world::inventory::sorter::{self, Age, CleanupPlan, Kind, Match, Move, Plan, Rule, Step};
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
    PreTrash(Vec<(Uuid, Uuid)>),
    /// Clean up's entries, straight to the Trash.
    Trash(Vec<(Uuid, Uuid)>),
    /// Imported settings replacing the user's rules and store hints.
    Replace,
    /// Allow Auto-sort into a default folder (its id and name), or Lost
    /// and Found's switch (None).
    Unlock(Option<Uuid>, String),
}

/// The folder picker: for the folder to sort or for a rule's destination.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pick {
    Source,
    Rule(usize),
    Exclude,
}

struct Picker {
    pick: Pick,
    filter: String,
    /// Every eligible folder and its path, built once per inventory change.
    rows: Vec<(Uuid, String)>,
    generation: u64,
}

#[derive(Default)]
pub(super) struct State {
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
    /// Clean up's rows unchecked: they stay.
    skipped: HashSet<Uuid>,
    /// Clean up's entries on their way to the Trash, a batch at a time.
    trash: Option<TrashJob>,
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
    journal: sorter::journal::Journal,
    /// A run the viewer stopped before its end, to resume.
    unfinished: Option<sorter::journal::Unfinished>,
    snapshots: Vec<(i64, std::path::PathBuf)>,
    /// Batches confirmed since the journal was last written.
    unsaved: usize,
    /// Batches are held: a teleport runs or no region is connected.
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
    /// The preview and Clean up's list being made off the frame, for a
    /// large inventory.
    planning: Option<Planning<Plan>>,
    cleaning: Option<Planning<CleanupPlan>>,
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
        let copy = sorter::snapshot(inv);
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

struct TrashJob {
    /// Each entry and the folder the confirmed list saw it in.
    queue: VecDeque<(Uuid, Uuid)>,
    /// Entries dropped because they changed since the confirmation.
    skipped: usize,
    total: usize,
    /// Entries the delete requests carried, counted once each one is done.
    sent: usize,
    /// The batch on its way: its entries, frames since it was pushed, and
    /// whether the app took it (a delete pending).
    in_flight: Option<(Vec<(Uuid, Uuid)>, u32, bool)>,
    next_at: Option<Instant>,
    /// The journal and the snapshot of its start are taken.
    begun: bool,
}

/// The time between two batches of a job, after the server's reply: about
/// 200 moves a second at most, so a 400,000-item sort neither floods the
/// simulator nor the viewer's own inventory updates.
const PACE: std::time::Duration = std::time::Duration::from_secs(1);
/// A refused batch is tried again this many times, after 2, 4 and 8 s.
const RETRIES: u32 = 3;
/// A destination folder that does not load within this time: its move is
/// skipped and the job goes on.
const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

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
    /// Moves confirmed since the journal last took them.
    confirmed: Vec<sorter::journal::Entry>,
    /// Undo, a snapshot or a resumed run: what it ends.
    restoring: Option<Restoring>,
    /// The journal and the snapshot of its start are taken.
    begun: bool,
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
        }
    }

    /// Put a batch's moves back at the front of the queue, in order.
    fn requeue(&mut self, moves: Vec<Move>) {
        for m in moves.into_iter().rev() {
            self.queue.push_front(m);
        }
    }

    /// The server's answer to the batch on its way.
    fn finish(&mut self, error: Option<&str>, now: Instant) -> Option<String> {
        let batch = self.in_flight.take()?;
        match error {
            None => {
                self.done += batch.moves.len();
                self.confirmed.extend(batch.moves.iter().map(|m| sorter::journal::Entry {
                    id: m.id,
                    folder: m.folder,
                    from: m.from,
                    name: m.name.clone(),
                }));
                self.retries = 0;
                self.next_at = Some(now + PACE);
                None
            }
            Some(error) if self.retries < RETRIES => {
                self.retries += 1;
                self.requeue(batch.moves);
                self.next_at = Some(now + std::time::Duration::from_secs(1 << self.retries));
                Some(aurora_i18n::tr!("inventory-sorter-retrying", error = error, attempt = self.retries))
            }
            Some(error) => {
                // never the whole job for one refused batch: it stays put
                self.failed += batch.moves.len();
                self.retries = 0;
                self.next_at = Some(now + PACE);
                Some(error.to_owned())
            }
        }
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

/// Journal saves: every this many confirmed batches, and at a run's end.
const SAVE_EVERY: usize = 20;

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
        self.journal = match j::load(&j::journal_path(&dir)) {
            Ok(journal) => journal,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(error) => {
                log::warn!("auto-sort: the journal could not be read: {error}");
                Default::default()
            }
        };
        self.unfinished = j::load(&j::unfinished_path(&dir))
            .ok()
            .filter(|u: &j::Unfinished| !u.moves.is_empty());
        self.snapshots = j::snapshots(&dir);
    }

    /// Write the journal (and the run's moves still to send) off the frame.
    fn save_progress(&mut self) {
        self.unsaved = 0;
        let Some(dir) = self.account_dir() else { return };
        let journal = self.journal.clone();
        let unfinished = self.job.as_ref().map(|job| sorter::journal::Unfinished {
            base: job.base.clone(),
            moves: job
                .in_flight
                .iter()
                .flat_map(|batch| batch.moves.iter())
                .chain(job.queue.iter())
                .cloned()
                .collect(),
        });
        std::thread::spawn(move || {
            use sorter::journal as j;
            if let Err(error) = j::save(&j::journal_path(&dir), &journal) {
                log::warn!("auto-sort: the journal could not be saved: {error}");
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

    /// A snapshot of the layout before a large or Trash run, kept with the
    /// account's last ones.
    fn snapshot(&mut self, inv: &Inventory) {
        let Some(dir) = self.account_dir() else { return };
        let at = unix_now();
        let Some(account) = self.account else { return };
        let snapshot = sorter::journal::Snapshot::take(inv, at, account);
        self.snapshots.insert(0, (at, sorter::journal::snapshot_path(&dir, at)));
        self.snapshots.truncate(sorter::journal::SNAPSHOTS);
        std::thread::spawn(move || {
            use sorter::journal as j;
            if let Err(error) = j::save(&j::snapshot_path(&dir, at), &snapshot) {
                log::warn!("auto-sort: the snapshot could not be saved: {error}");
            }
            j::prune_snapshots(&dir);
        });
    }

    pub(super) fn active(&self) -> bool {
        self.open || self.job.is_some() || self.submitted || self.requested.is_some() || self.trash.is_some()
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.submitted || self.trash.is_some()
    }

    fn set_source(&mut self, source: Source) {
        self.open = true;
        if self.busy() {
            return;
        }
        if self.source.as_ref() != Some(&source) {
            self.preview = None;
            self.status.clear();
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
                if !hint.creator_name.is_empty() {
                    mine.creator_name = hint.creator_name;
                }
            }
            None => prefs.store_hints.push(hint),
        }
        self.sorter.open = true;
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
        claim(&mut self.sorter, request, change) || self.windows.iter_mut().any(|w| claim(&mut w.state.sorter, request, change))
    }

    /// The result of `request`: completes the sorter batch that took it.
    pub(crate) fn finish_sorter(&mut self, request: Uuid, error: Option<&str>) {
        fn finish(state: &mut State, request: Uuid, error: Option<&str>) {
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
                && let Some(message) = job.finish(error, Instant::now())
            {
                state.status = message;
            }
        }
        finish(&mut self.sorter, request, error);
        for window in &mut self.windows {
            finish(&mut window.state.sorter, request, error);
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
    // the default folders the user allowed, for the hands-off rule
    let unlocked = prefs.sorter.unlocked_set(inv);
    if inv.sorter_unlocked != unlocked {
        inv.sorter_unlocked = unlocked;
    }
    if st.sorter.open || st.sorter.job.is_some() || st.sorter.trash.is_some() {
        st.sorter.load_account(agent);
    }
    if let Some(shown) = st.sorter.requested.take() {
        let source = default_source(inv, st, shown);
        st.sorter.set_source(Source::Folder(source));
    }
    let free = st.pending.is_none()
        && !actions
            .iter()
            .any(|action| matches!(action, InvAction::Edit(_) | InvAction::Delete(_)));
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
            job.requeue(batch.moves);
            state.submitted = false;
        }
        let silent = job
            .in_flight
            .as_ref()
            .is_some_and(|batch| now.duration_since(batch.sent_at) > ANSWER_LIMIT);
        if silent {
            if let Some(message) = job.finish(Some(aurora_i18n::tr!("inventory-sorter-no-answer")), now) {
                state.status = message;
            }
            state.submitted = false;
        }
    }
    // The journal takes what the server confirmed; saved every few batches
    // with the moves still to send, so a run the viewer stops can resume.
    let mut begin = false;
    if let Some(job) = state.job.as_mut() {
        if !job.confirmed.is_empty() {
            let confirmed = std::mem::take(&mut job.confirmed);
            state.journal.record(confirmed);
            state.unsaved += 1;
        }
        if job.done == 0 && job.in_flight.is_none() && !job.begun {
            job.begun = true;
            begin = true;
        }
    }
    if begin {
        // a run starts: its journal, and a snapshot of the layout first
        state.journal.begin(sorter::journal::RunKind::Move, unix_now());
        state.snapshot(inv);
    }
    if state.unsaved >= SAVE_EVERY {
        state.save_progress();
    }
    // while a teleport runs or no region is connected, nothing is sent and
    // no retry is spent: the job resumes on its own
    if free
        && !hold
        && !state.submitted
        && let Some(job) = state.job.as_mut()
        && !job.paused
        && job.next_at.is_none_or(|at| now >= at)
    {
        match sorter::step(inv, &job.base, &mut job.queue, &prefs.protected, worn, &mut job.skipped) {
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
            Step::Wait => {
                // a destination that never loads: its move is skipped
                let since = *job.waiting_since.get_or_insert(now);
                if now.duration_since(since) > WAIT_LIMIT {
                    if job.queue.pop_front().is_some() {
                        job.skipped += 1;
                    }
                    job.waiting_since = None;
                }
            }
            Step::Done => {
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
                state.job = None;
                state.preview = None;
                state.cleanup = None;
                if restoring == Some(Restoring::Undo) {
                    // the undo run, then the run it undid: neither is undone again
                    state.journal.drop_last();
                    state.journal.drop_last();
                }
                if restoring == Some(Restoring::Resume) {
                    state.unfinished = None;
                }
                // the journal written, and no run left to resume
                state.save_progress();
            }
        }
    }
    send_trash_batch(inv, st, prefs, worn, actions, free && !hold);
    let state = &mut st.sorter;
    if !state.open {
        return;
    }
    let Some(source) = state.source.clone() else {
        state.open = false;
        return;
    };
    let busy = state.busy();
    let loaded = match &source {
        Source::Folder(id) => {
            // Loading the folder is all a preview needs: its direct contents.
            let loaded = inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetched);
            if !loaded && inv.folders.contains_key(id) {
                inv.request(*id);
            }
            loaded
        }
        Source::Entries(_) | Source::Saved(..) => true,
        Source::Whole => {
            let mut loaded = true;
            for id in sorter::whole_sources(inv, &prefs.sorter) {
                if inv.folders.get(&id).is_some_and(|f| f.state != FetchState::Fetched) {
                    inv.request(id);
                    loaded = false;
                }
            }
            loaded
        }
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
        if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Unknown) {
            inv.request(id);
        }
    }
    let settled = state.quiet_since.is_none_or(|q| now.duration_since(q) >= QUIET)
        && state.planned_at.is_none_or(|at| now.duration_since(at) >= REPLAN);
    // A plan made off the frame arrives: kept when it is still for this
    // source (its options and inventory are compared below, like any
    // preview's, and a newer one is made when they moved on).
    if let Some((task, plan)) = Planning::take(&mut state.planning)
        && task.source == source
    {
        state.to_request.extend(plan.unfetched.iter().copied());
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
            move |inv: &Inventory| match &source {
                Source::Folder(id) => sorter::plan(inv, *id, &options, &protected, &worn),
                Source::Entries(ids) => sorter::plan_entries(inv, ids, &options, &protected, &worn),
                Source::Whole => sorter::plan_whole(inv, &options, &protected, &worn),
                Source::Saved(moves, _) => sorter::plan_saved(inv, moves, &protected, &worn),
            }
        };
        if inv.items.len() + inv.folders.len() >= PLAN_IN_BACKGROUND {
            state.planning = Some(Planning::spawn(inv, &prefs.sorter, &source, None, make));
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
    {
        state.to_request.extend(plan.unfetched.iter().copied());
        state.cleanup = Some((task.generation, choices, from, day, plan));
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
        });
        if stale {
            state.cleaned_at = Some(instant);
            let make = {
                let (options, protected, worn) = (prefs.sorter.clone(), prefs.protected.clone(), worn.clone());
                move |inv: &Inventory| sorter::plan_cleanup(inv, &options, &options.cleanup, cleanup_source, now, &protected, &worn)
            };
            if inv.items.len() + inv.folders.len() >= PLAN_IN_BACKGROUND {
                let about = Some((prefs.sorter.cleanup.clone(), cleanup_source, day));
                state.cleaning = Some(Planning::spawn(inv, &prefs.sorter, &source, about, make));
            } else {
                let plan = make(inv);
                state.to_request.extend(plan.unfetched.iter().copied());
                state.cleanup = Some((inv.generation, prefs.sorter.cleanup.clone(), cleanup_source, day, plan));
            }
        }
    }
    if state.autostart && state.preview.is_some() {
        state.autostart = false;
        start(state, prefs);
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
                CLEANUP => cleanup_tab(ui, p, state, &mut prefs.sorter, matches!(source, Source::Folder(_)), busy, free_now),
                _ => {
                    if !waiting.is_empty() {
                        pre_trash_card(ui, p, state, &waiting, waiting_in, busy);
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
            if let Some((.., plan)) = state.cleanup.as_ref() {
                let moves = plan.moves(&state.skipped);
                if !moves.is_empty() && state.job.is_none() {
                    state.status.clear();
                    state.job = Some(Job::new(moves.into(), prefs.sorter.base_name()));
                }
            }
        }
        Some(CleanupAction::Trash) => {
            let state = &mut st.sorter;
            if let Some((.., plan)) = state.cleanup.as_ref() {
                let ids: Vec<(Uuid, Uuid)> = plan
                    .rows
                    .iter()
                    .filter(|r| !state.skipped.contains(&r.id))
                    .map(|r| (r.id, r.from))
                    .collect();
                if !ids.is_empty() {
                    state.confirm = Some(Confirm::Trash(ids));
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
            start(state, prefs);
        }
    }
    if pause && let Some(job) = state.job.as_mut() {
        job.paused = !job.paused;
    }
    if stop && let Some(job) = state.job.as_mut() {
        job.paused = false;
        // the batch on its way finishes; nothing after it is sent
        job.queue.clear();
    }
    if stop && let Some(trash) = state.trash.as_mut() {
        trash.queue.clear();
    }
    state.open = open;
    if !open {
        state.picker = None;
        state.confirm = None;
    }
    picker(ctx, p, inv, st, prefs);
    share_window(ctx, p, st, &prefs.sorter);
    import_window(ctx, p, st, &mut prefs.sorter);
    vocab_window(ctx, p, st);
    confirm(ctx, p, st, prefs);
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
        import.read.clear();
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
    let deleting = st.pending_delete.is_some();
    let free = free && !deleting && !actions.iter().any(|a| matches!(a, InvAction::Delete(_)));
    let now = Instant::now();
    if st.sorter.trash.as_ref().is_some_and(|trash| !trash.begun) {
        // a Trash run starts: its journal, and a snapshot of the layout first
        st.sorter.journal.begin(sorter::journal::RunKind::Trash, unix_now());
        st.sorter.snapshot(inv);
        if let Some(trash) = st.sorter.trash.as_mut() {
            trash.begun = true;
        }
    }
    let Some(trash) = st.sorter.trash.as_mut() else { return };
    // The batch on its way: done once the app took it and its delete is
    // over; never taken (another change went first), back in the queue.
    if let Some((batch, frames, taken)) = trash.in_flight.as_mut() {
        *frames += 1;
        *taken |= deleting;
        if *taken && !deleting {
            trash.sent += batch.len();
            st.sorter.journal.record(batch.iter().map(|(id, from)| sorter::journal::Entry {
                id: *id,
                folder: inv.folders.contains_key(id),
                from: *from,
                name: rules::name(inv, *id),
            }));
            st.sorter.unsaved += 1;
            trash.in_flight = None;
            trash.next_at = Some(now + PACE);
        } else if !*taken && *frames > 2 {
            for entry in batch.drain(..).rev() {
                trash.queue.push_front(entry);
            }
            trash.in_flight = None;
        } else {
            return;
        }
    }
    if !free || trash.next_at.is_some_and(|at| now < at) {
        return;
    }
    if trash.queue.is_empty() {
        st.sorter.status = if trash.skipped > 0 {
            aurora_i18n::tr!("inventory-sorter-done-skipped", count = trash.sent, skipped = trash.skipped)
        } else {
            aurora_i18n::tr!("inventory-cleanup-trash-done", count = trash.sent)
        };
        st.sorter.trash = None;
        st.sorter.cleanup = None;
        st.sorter.skipped.clear();
        return;
    }
    let take = trash.queue.len().min(TRASH_BATCH);
    let picked: Vec<(Uuid, Uuid)> = trash.queue.drain(..take).collect();
    // Re-checked now, like a sort's moves: still where the confirmed list
    // saw it, nowhere hands-off, nothing worn or locked. What changed is
    // skipped and reported, never sent.
    let holds_worn = crate::world::inventory::actions::worn_ancestors(inv, worn);
    let batch: Vec<Uuid> = picked
        .iter()
        .filter(|(id, from)| sorter::trash_ready(inv, *id, *from, &prefs.sorter, &prefs.protected, &holds_worn))
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
        Ok(change) => actions.push(InvAction::Delete(Box::new(DeleteRequest::new(inv, st, &batch, change)))),
        Err(reason) => {
            st.sorter.status = reason;
            st.sorter.trash = None;
        }
    }
}

/// What waits in Pre-Trash, and the step that sends it to the Trash.
fn pre_trash_card(ui: &mut egui::Ui, p: &Palette, state: &mut State, waiting: &[Uuid], waiting_in: Option<Uuid>, busy: bool) {
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
        ui.add_space(4.0);
        let send = ui.add_enabled_ui(!busy, |ui| {
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
            state.confirm = Some(Confirm::PreTrash(waiting.iter().map(|id| (*id, folder)).collect()));
        }
    });
}

/// The confirmations: a whole-inventory sort (with its "I understand" box)
/// and Pre-Trash to Trash. Escape or Cancel closes them, nothing moved.
fn confirm(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
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
        Confirm::Whole => start(&mut st.sorter, prefs),
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
                import.read.clear();
            }
        }
        Confirm::Trash(ids) => {
            st.sorter.status.clear();
            st.sorter.trash = Some(TrashJob {
                total: ids.len(),
                queue: ids.into(),
                sent: 0,
                skipped: 0,
                in_flight: None,
                next_at: None,
                begun: false,
            });
        }
        Confirm::PreTrash(ids) => {
            // the same batches as Clean up's Trash: at most 200 per delete
            // request, each entry re-checked before it goes
            st.sorter.status.clear();
            st.sorter.trash = Some(TrashJob {
                total: ids.len(),
                queue: ids.into(),
                sent: 0,
                skipped: 0,
                in_flight: None,
                next_at: None,
                begun: false,
            });
        }
    }
}

fn start(state: &mut State, prefs: &InventoryPreferences) {
    let Some((_, options, plan)) = state.preview.as_ref() else { return };
    // never a run from a preview of other options, or while a newer one is
    // being made: the user sorts what they saw
    if state.planning.is_some() || *options != prefs.sorter {
        state.status = aurora_i18n::tr!("inventory-sorter-updating").into();
        return;
    }
    state.status.clear();
    if plan.moves.is_empty() {
        state.status = aurora_i18n::tr!("inventory-sorter-nothing").into();
        return;
    }
    let mut job = Job::new(plan.moves.iter().cloned().collect(), prefs.sorter.base_name());
    if let Some(Source::Saved(_, restoring)) = &state.source {
        job.restoring = Some(*restoring);
        if let (Restoring::Resume, Some(unfinished)) = (restoring, &state.unfinished) {
            job.base.clone_from(&unfinished.base);
        }
    }
    state.job = Some(job);
}

/// What is sorted: a folder (with Change…) or the selected entries.
fn source_row(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, source: &Source, busy: bool) {
    crate::ui::widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "folder-filled", p.violet_light, 16.0);
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
                ui.label(RichText::new(text).strong());
            });
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
fn cleanup_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    state: &mut State,
    options: &mut sorter::Preferences,
    has_folder: bool,
    busy: bool,
    free: bool,
) {
    use crate::ui::widgets::{ActionKind, action_button, checkbox, collapsing, section_header, segmented, switch_labeled};
    let base = options.base_name();
    let cleanup = &mut options.cleanup;
    ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-intro")).small().color(p.muted));
    ui.add_space(6.0);
    ui.add_enabled_ui(!busy, |ui| {
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
        ui.push_id("inventory_cleanup_empty_folders", |ui| {
            switch_labeled(
                ui,
                p,
                &mut cleanup.empty_folders,
                aurora_i18n::tr!("inventory-cleanup-empty-folders"),
            )
        });
        ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-never")).small().color(p.muted));
    });
    crate::ui::widgets::divider(ui, p);
    let State {
        cleanup: list,
        skipped,
        cleanup_action,
        ..
    } = state;
    let Some((.., plan)) = list.as_ref() else { return };
    if !plan.unfetched.is_empty() {
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-cleanup-loading", count = plan.unfetched.len()))
                .small()
                .color(p.muted),
        );
    }
    let date = date_text(plan.cutoff);
    if plan.rows.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-nothing", date = date.as_str())).color(p.muted));
        return;
    }
    let going: Vec<_> = plan.rows.iter().filter(|r| !skipped.contains(&r.id)).collect();
    let items: usize = going.iter().map(|r| r.items).sum();
    ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-summary", count = items, date = date.as_str())).strong());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let size = Vec2::new(0.0, 26.0);
        let enabled = !going.is_empty() && !busy && free;
        let label = aurora_i18n::tr!("inventory-cleanup-to-pre-trash", count = going.len());
        if ui
            .add_enabled_ui(enabled, |ui| {
                action_button(ui, p, "trash-filled", &label, ActionKind::Primary, size)
            })
            .inner
            .clicked()
        {
            *cleanup_action = Some(CleanupAction::PreTrash);
        }
        let label = aurora_i18n::tr!("inventory-cleanup-to-trash", count = going.len());
        if ui
            .add_enabled_ui(enabled, |ui| action_button(ui, p, "trash-filled", &label, ActionKind::Danger, size))
            .inner
            .clicked()
        {
            *cleanup_action = Some(CleanupAction::Trash);
        }
    });
    ui.add_space(6.0);
    // folders first (they go whole), then each kind in the filter's order
    let mut groups: Vec<(String, Vec<&sorter::CleanupRow>)> = Vec::new();
    let folders: Vec<_> = plan.rows.iter().filter(|r| r.folder && r.items > 0).collect();
    if !folders.is_empty() {
        groups.push((aurora_i18n::tr!("inventory-cleanup-folders").to_owned(), folders));
    }
    let empty: Vec<_> = plan.rows.iter().filter(|r| r.folder && r.items == 0).collect();
    if !empty.is_empty() {
        groups.push((aurora_i18n::tr!("inventory-cleanup-empty-group").to_owned(), empty));
    }
    for kind in Kind::ALL {
        let rows: Vec<_> = plan.rows.iter().filter(|r| r.kind == Some(kind)).collect();
        if !rows.is_empty() {
            groups.push((kind.label().to_owned(), rows));
        }
    }
    for (label, rows) in groups {
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
    default_folders(ui, p, inv, state, options);
    ui.add_space(10.0);
    undo_and_snapshots(ui, p, inv, state, worn_none());
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

/// No worn set where the Options tab has none: the preview re-checks every
/// entry against what is worn before anything moves.
fn worn_none() -> &'static HashSet<Uuid> {
    static NONE: std::sync::OnceLock<HashSet<Uuid>> = std::sync::OnceLock::new();
    NONE.get_or_init(HashSet::new)
}

/// A snapshot's or a run's date, "2026-11-15 12:30" (UTC).
fn snapshot_date(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M").to_string())
}

/// Undo the last run, put a snapshot's layout back, resume a stopped run,
/// take a snapshot: each opens its preview; nothing moves before Sort.
fn undo_and_snapshots(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, worn: &HashSet<Uuid>) {
    use crate::ui::widgets::{ActionKind, action_button};
    let size = Vec2::new(0.0, 26.0);
    let protected = HashSet::new();
    crate::ui::widgets::section_header(ui, p, "history", aurora_i18n::tr!("inventory-sorter-undo-section"));
    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-undo-hint")).small().color(p.muted));
    if let Some(unfinished) = &state.unfinished {
        let label = aurora_i18n::tr!("inventory-sorter-resume-run", count = unfinished.moves.len());
        if action_button(ui, p, "player-play-filled", &label, ActionKind::Primary, size).clicked() {
            let moves = std::sync::Arc::new(unfinished.moves.clone());
            state.set_source(Source::Saved(moves, Restoring::Resume));
            state.tab = PREVIEW;
        }
    }
    match state.journal.last() {
        Some(run) => {
            let date = snapshot_date(run.at);
            let label = aurora_i18n::tr!("inventory-sorter-undo-run", count = run.entries.len(), date = date.as_str());
            if action_button(ui, p, "arrow-back-up", &label, ActionKind::Flat, size).clicked() {
                let (moves, _) = sorter::journal::undo_moves(inv, run, &protected, worn);
                state.set_source(Source::Saved(std::sync::Arc::new(moves), Restoring::Undo));
                state.tab = PREVIEW;
            }
        }
        None => {
            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-undo-none")).small().color(p.muted));
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
    let mut restore = None;
    for (at, path) in &state.snapshots {
        ui.horizontal(|ui| {
            ui.label(snapshot_date(*at));
            if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-snapshot-restore")).clicked() {
                restore = Some((*at, path.clone()));
            }
        });
    }
    if let Some((at, path)) = restore {
        match sorter::journal::load::<sorter::journal::Snapshot>(&path) {
            Ok(snapshot) => {
                let account = state.account.unwrap_or_default();
                let by_runs = sorter::journal::moved_since(&state.journal, snapshot.at);
                match sorter::journal::restore_moves(inv, &snapshot, account, &by_runs, &protected, worn) {
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
                ui.add(
                    egui::TextEdit::singleline(&mut hint.store)
                        .char_limit(64)
                        .hint_text(hint_text)
                        .desired_width(150.0),
                );
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
        let unpackers = plan.unpackers();
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
    for (label, rows) in plan.grouped() {
        let title = format!("{label}  ({})", rows.len());
        crate::ui::widgets::collapsing(ui, p, ("inventory_sorter_group", &label), title, false, |ui| {
            for m in rows.iter().take(PREVIEW_ROWS) {
                ui.label(RichText::new(&m.name).color(if m.unpacker { p.violet_light } else { p.ink }));
            }
            more(ui, p, rows.len());
        });
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
            for name in names.iter().take(PREVIEW_ROWS) {
                ui.label(RichText::new(*name).color(p.muted));
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
        }
    }
}

/// A searchable list of the user's folders, by path: the folder to sort, or
/// the folder a rule sends entries to.
fn picker(ctx: &egui::Context, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let state = &mut st.sorter;
    let Some(picker) = state.picker.as_mut() else { return };
    if picker.generation != inv.generation {
        let pick = picker.pick;
        let mut rows: Vec<(Uuid, String)> = inv
            .folders
            .values()
            .filter(|f| !f.library)
            .map(|f| f.info.id)
            .filter(|id| match pick {
                Pick::Source => sorter::source_allowed(inv, *id),
                Pick::Rule(_) => sorter::target_allowed(inv, *id),
                Pick::Exclude => *id != inv.root && !sorter::untouchable(inv, *id),
            })
            .map(|id| (id, sorter::folder_path(inv, id)))
            .collect();
        rows.sort_by_cached_key(|(_, path)| crate::natural_order::key(path));
        picker.rows = rows;
        picker.generation = inv.generation;
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
            for (id, path) in picker
                .rows
                .iter()
                .filter(|(_, path)| query.is_empty() || path.to_lowercase().contains(&query))
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
