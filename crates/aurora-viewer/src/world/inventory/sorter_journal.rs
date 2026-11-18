//! Auto-sort's memory of what it did, kept per account so nothing is lost
//! for good: a journal of every confirmed move of the last runs (for "Undo
//! last run"), snapshots of the inventory's layout (folders and where each
//! entry was, names only, never an asset id), and the moves a run had left
//! when the viewer stopped (for "Resume last run").
//!
//! Undo and a snapshot's layout give back plain moves (`Move` with
//! `restore` set): they go through the same preview and the same paced,
//! re-checked batches as a sort. Nothing here sends anything.
//!
//! The files are gzip-compressed JSON in the account's folder
//! (`settings::account_dir`), under `auto-sort/`.

use super::{Dest, Inventory, Move, actions, folder_path, holds_closed, restorable, restore_target_allowed};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Runs kept in the journal.
pub const RUNS: usize = 5;
/// Snapshots kept per account (the oldest goes).
pub const SNAPSHOTS: usize = 5;

/// One confirmed move: the entry and the folder it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: Uuid,
    pub folder: bool,
    pub from: Uuid,
    pub name: String,
    /// Where the run put it, when known: Undo leaves an entry the user
    /// moved on since (it is no longer there).
    #[serde(default)]
    pub to: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunKind {
    /// A sort, a move to Pre-Trash, an undo or a restored layout.
    Move,
    /// Entries sent to the Trash (Clean up, Empty Pre-Trash).
    Trash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Its own number in the journal: what is recorded, undone and
    /// forgotten is found by it, never by its time (two runs may start in
    /// the same second).
    #[serde(default)]
    pub id: u64,
    /// Seconds since 1970.
    pub at: i64,
    pub kind: RunKind,
    pub entries: Vec<Entry>,
    /// An Undo run: the run it undoes. Never offered as "Undo last run"
    /// itself (undoing an Undo would put the sort back).
    #[serde(default)]
    pub undoes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    pub runs: VecDeque<Run>,
    /// The last id given: ids are never given twice, even after a run is
    /// forgotten (a stale Undo then never touches a newer run).
    #[serde(default)]
    pub last_id: u64,
    /// The file could not be read this session (busy, not allowed): this
    /// session's runs are kept in memory only, the file never written over.
    #[serde(skip)]
    pub read_only: bool,
    /// Its file is being read off the frame (a large history is hundreds of
    /// megabytes of text): no run starts and nothing is written meanwhile.
    #[serde(skip)]
    pub loading: bool,
}

impl Journal {
    /// A new run, the oldest dropped past [`RUNS`]; its id.
    pub fn begin(&mut self, kind: RunKind, at: i64) -> u64 {
        self.begin_keeping(kind, at, None)
    }

    /// A new run; the run `keep` (one an Undo is putting back) is never the
    /// one dropped to keep the last five.
    fn begin_keeping(&mut self, kind: RunKind, at: i64, keep: Option<u64>) -> u64 {
        let id = self.runs.iter().map(|run| run.id).max().unwrap_or(0).max(self.last_id) + 1;
        self.last_id = id;
        // a run that moved nothing (stopped at once, everything skipped or
        // refused) never pushes a real one out of the last five
        self.runs.retain(|run| !run.entries.is_empty());
        self.runs.push_back(Run {
            id,
            at,
            kind,
            entries: Vec::new(),
            undoes: None,
        });
        while self.runs.len() > RUNS {
            let Some(oldest) = self.runs.iter().position(|run| Some(run.id) != keep && run.id != id) else {
                break;
            };
            self.runs.remove(oldest);
        }
        id
    }

    /// Every Undo run of `undone` (this session's, a stopped session's, one
    /// started afresh instead of resumed) forgotten with what it put back.
    pub fn forget_undos_of(&mut self, undone: u64) {
        let undos: Vec<u64> = self
            .runs
            .iter()
            .filter(|run| run.undoes == Some(undone))
            .map(|run| run.id)
            .collect();
        for undo in undos {
            self.forget_undone(undo, undone);
        }
    }

    /// Runs read from an older file have no id: numbered in order.
    pub fn number_runs(&mut self) {
        let mut next = self.runs.iter().map(|run| run.id).max().unwrap_or(0).max(self.last_id);
        for run in self.runs.iter_mut().filter(|run| run.id == 0) {
            next += 1;
            run.id = next;
        }
        self.last_id = next;
    }

    /// Confirmed moves of the run `id` (nothing when it was dropped).
    pub fn record_in(&mut self, id: u64, entries: impl IntoIterator<Item = Entry>) {
        if let Some(run) = self.runs.iter_mut().find(|run| run.id == id) {
            run.entries.extend(entries);
        }
    }

    /// Confirmed moves of the run in progress.
    #[cfg(test)]
    pub fn record(&mut self, entries: impl IntoIterator<Item = Entry>) {
        if let Some(run) = self.runs.back_mut() {
            run.entries.extend(entries);
        }
    }

    /// An Undo of the run `undone` begins; its id.
    pub fn begin_undo(&mut self, at: i64, undone: u64) -> u64 {
        let id = self.begin_keeping(RunKind::Move, at, Some(undone));
        if let Some(run) = self.runs.iter_mut().find(|run| run.id == id) {
            run.undoes = Some(undone);
        }
        id
    }

    /// The last run that moved something (an Undo's own run never).
    #[cfg(test)]
    pub fn last(&self) -> Option<&Run> {
        self.runs.iter().rev().find(|run| !run.entries.is_empty() && run.undoes.is_none())
    }

    /// The last run with something left to put back: a run whose entries
    /// are all gone (purged from the Trash, deleted) never hides an older
    /// one.
    pub fn last_undoable(&self, exists: impl Fn(&Entry) -> bool) -> Option<&Run> {
        self.runs
            .iter()
            .rev()
            .filter(|run| run.undoes.is_none())
            .find(|run| run.entries.iter().any(&exists))
    }

    /// Changes whenever a run starts, ends or records moves: what was read
    /// from the journal at one value still holds at the same one.
    pub fn version(&self) -> (u64, usize, usize) {
        (self.last_id, self.runs.len(), self.runs.back().map_or(0, |run| run.entries.len()))
    }

    /// After the Undo run `undo` of the run `undone`: the undo's own run
    /// goes, and from the undone run only the entries it put back; that run
    /// goes once nothing is left in it. A stopped or partial undo leaves the
    /// rest to undo later; an undo that moved nothing forgets nothing.
    pub fn forget_undone(&mut self, undo: u64, undone: u64) {
        let Some(undo_index) = self.runs.iter().position(|run| run.id == undo) else {
            return;
        };
        let Some(undo) = self.runs.remove(undo_index) else { return };
        let back: HashSet<Uuid> = undo.entries.iter().map(|e| e.id).collect();
        let Some(index) = self.runs.iter().position(|run| run.id == undone) else {
            return;
        };
        let run = &mut self.runs[index];
        run.entries.retain(|e| !back.contains(&e.id));
        if run.entries.is_empty() {
            self.runs.remove(index);
        }
    }
}

/// The moves that put a run back: each entry to the folder it came from,
/// when that folder is still there and open, and the entry has not been
/// moved again by hand (it is still where the run put it, or in the Trash
/// for a Trash run). What cannot go back is counted, not guessed.
pub fn undo_moves(inv: &Inventory, run: &Run, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> (Vec<Move>, usize) {
    let holds_worn = actions::worn_ancestors(inv, worn);
    let trash = actions::system(inv, 14);
    let mut targets = Targets::new(inv);
    let mut moves = Vec::new();
    let mut skipped = 0;
    // an entry moved twice in a run goes back where it was first
    let mut first_from: HashMap<Uuid, &Entry> = HashMap::new();
    for entry in &run.entries {
        first_from.entry(entry.id).or_insert(entry);
    }
    for entry in first_from.values() {
        let Some(now) = actions::parent(inv, entry.id) else {
            skipped += 1;
            continue;
        };
        let in_trash = trash.is_some_and(|t| actions::under(inv, entry.id, t));
        let where_run_left = match run.kind {
            RunKind::Trash => in_trash,
            RunKind::Move => !in_trash,
        };
        if now == entry.from {
            continue;
        }
        // moved on by hand since the run put it somewhere: it stays there
        if run.kind == RunKind::Move && entry.to.is_some_and(|to| to != now) {
            skipped += 1;
            continue;
        }
        if !where_run_left || !targets.allowed(entry.from) || !restorable(inv, entry.id, protected, &holds_worn) {
            skipped += 1;
            continue;
        }
        // an entry journaled at logout while its batch was on its way has
        // no kind or name: both are read from the inventory now
        let folder = inv.folders.get(&entry.id);
        let name = match (entry.name.is_empty(), folder, inv.items.get(&entry.id)) {
            (true, Some(f), _) => f.info.name.as_str(),
            (true, None, Some(it)) => it.name.as_str(),
            _ => entry.name.as_str(),
        };
        let mut back = restore_move(entry.id, folder.is_some(), name, now, entry.from, targets.label(entry.from));
        // only an Undo of a Trash run takes entries out of the Trash
        back.from_trash = run.kind == RunKind::Trash;
        moves.push(back);
    }
    moves.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.name.cmp(&b.name)));
    (moves, skipped)
}

fn restore_move(id: Uuid, folder: bool, name: &str, from: Uuid, to: Uuid, label: String) -> Move {
    Move {
        id,
        folder,
        name: name.to_owned(),
        from,
        to: Dest::Folder(to),
        label,
        unpacker: false,
        restore: true,
        from_trash: false,
        snapshot: false,
    }
}

/// What a restore asks of each folder an entry goes back to, worked out
/// once per folder: a layout or an Undo of a large run puts hundreds of
/// thousands of entries back into a few thousand folders.
struct Targets<'a> {
    inv: &'a Inventory,
    pre_trash: HashSet<Uuid>,
    allowed: HashMap<Uuid, bool>,
    labels: HashMap<Uuid, String>,
}

impl<'a> Targets<'a> {
    fn new(inv: &'a Inventory) -> Self {
        Self {
            inv,
            pre_trash: super::pre_trash_folders(inv),
            allowed: HashMap::new(),
            labels: HashMap::new(),
        }
    }

    /// `restore_target_allowed`, remembered.
    fn allowed(&mut self, to: Uuid) -> bool {
        let inv = self.inv;
        *self.allowed.entry(to).or_insert_with(|| restore_target_allowed(inv, to))
    }

    fn label(&mut self, to: Uuid) -> String {
        let inv = self.inv;
        self.labels.entry(to).or_insert_with(|| folder_path(inv, to)).clone()
    }
}

/// The inventory's layout at one moment: its folders (id, parent, name,
/// type) and where each item was. Names only, never an asset id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Seconds since 1970.
    pub at: i64,
    /// The account it was taken on: never restored on another one.
    #[serde(default)]
    pub account: Uuid,
    pub folders: Vec<(Uuid, Uuid, String, i32)>,
    pub items: Vec<(Uuid, Uuid, String)>,
    /// A layout to share: the folders whose contents were never read (the
    /// server failed to give them, or the wait ran out): unknown, never
    /// empty.
    #[serde(default)]
    pub unread: Vec<Uuid>,
}

impl Snapshot {
    /// My Inventory as it is now, the Library left out.
    pub fn take(inv: &Inventory, at: i64, account: Uuid) -> Self {
        let folders = inv
            .folders
            .values()
            .filter(|f| !f.library)
            .map(|f| (f.info.id, f.info.parent, f.info.name.clone(), f.info.type_default))
            .collect();
        let items = inv
            .items
            .values()
            // the Library's folders carry their flag: one lookup an item,
            // not a walk up its folders
            .filter(|it| inv.folders.get(&it.parent).is_none_or(|f| !f.library))
            .map(|it| (it.id, it.parent, it.name.clone()))
            .collect();
        Self {
            at,
            account,
            folders,
            items,
            unread: Vec::new(),
        }
    }
}

impl Snapshot {
    /// The layout to share (`Save my inventory layout to share`): no
    /// account, every folder and item numbered in place of its inventory
    /// id, and nothing of the Trash, Lost and Found, Calling Cards (other
    /// residents' names) or Landmarks (places the user goes); calling cards
    /// and landmarks elsewhere are left out too.
    pub fn take_to_share(inv: &Inventory, at: i64) -> Self {
        // (the Trash, Lost and Found, Calling Cards, Landmarks, the Photo
        // Album and Favorites: places and residents)
        const LEFT_OUT: [i32; 6] = [14, 16, 2, 3, 15, 23];
        // (the folders shown, found once from My Inventory down: one walk,
        // never one walk up per entry at 400,000 items)
        let mut shown: HashSet<Uuid> = HashSet::new();
        let mut stack = vec![inv.root];
        while let Some(id) = stack.pop() {
            let Some(f) = inv.folders.get(&id) else { continue };
            if f.library || LEFT_OUT.contains(&f.info.type_default) || !shown.insert(id) {
                continue;
            }
            stack.extend(f.children.iter().copied());
        }
        let hidden = |id: Uuid| !shown.contains(&id);
        let mut numbers: HashMap<Uuid, Uuid> = HashMap::new();
        let mut number = |id: Uuid| {
            let next = numbers.len() as u128 + 1;
            *numbers.entry(id).or_insert_with(|| Uuid::from_u128(next))
        };
        let mut kept: Vec<&super::super::Folder> = inv.folders.values().filter(|f| !hidden(f.info.id)).collect();
        kept.sort_by_key(|f| f.info.id);
        let folders = kept
            .iter()
            .map(|f| (number(f.info.id), number(f.info.parent), f.info.name.clone(), f.info.type_default))
            .collect();
        let mut items: Vec<_> = inv
            .items
            .values()
            .filter(|it| {
                !matches!(it.asset_type, 2 | 3)
                    && !matches!(it.inv_type, 2 | 3)
                    // (a snapshot anywhere, renamed or Aurora's 360° one:
                    // `360 Snapshot : <place>`; its name holds a place)
                    && it.inv_type != 15
                    && !it.name.to_lowercase().contains("snapshot :")
                    && !crate::snapshot::capture_360::is_marked(&it.name)
                    && !hidden(it.parent)
            })
            .collect();
        items.sort_by_key(|it| it.id);
        let items = items.iter().map(|it| (number(it.id), number(it.parent), it.name.clone())).collect();
        let unread = kept
            .iter()
            .filter(|f| f.state != super::super::FetchState::Fetched)
            .map(|f| number(f.info.id))
            .collect();
        Self {
            at,
            account: Uuid::nil(),
            folders,
            items,
            unread,
        }
    }
}

/// What a layout restore leaves alone, for the preview's summary. A
/// restore never stops on any of these: it skips and counts them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RestoreSummary {
    /// Deleted for good since the snapshot.
    pub gone: usize,
    /// In the Trash since: only Undo takes things out of the Trash.
    pub trashed: usize,
    /// Their snapshot folder is gone or now hands-off: not guessed.
    pub folder_gone: usize,
    /// Arrived after the snapshot: left exactly where they are.
    pub new_since: usize,
    /// Moved by hand since (not by an Auto-sort run): listed in their own
    /// group of the preview, moved back only when its Sort is pressed.
    pub by_hand: usize,
    /// Worn, a link, locked, or now in a hands-off place: they stay.
    pub kept: usize,
}

/// A snapshot of another account: never restored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OtherAccount;

/// Resolve an anonymized export using exact, unique names and unchanged
/// destination paths. Its numbered identifiers are only local to the file.
/// Folder moves and duplicate item names cannot be recovered this way.
pub fn match_shared_layout(inv: &Inventory, shared: &Snapshot, account: Uuid) -> (Snapshot, usize) {
    let mut result = Snapshot {
        at: shared.at,
        account,
        ..Default::default()
    };
    let mut used_folders = HashSet::new();
    let mut branches: HashMap<Uuid, Vec<(Uuid, &str, i32)>> = HashMap::new();
    for (id, parent, name, kind) in &shared.folders {
        branches.entry(*parent).or_default().push((*id, name, *kind));
        used_folders.insert(*id);
    }
    let mut roots = shared
        .folders
        .iter()
        .filter(|(_, parent, _, kind)| *kind == 8 && !used_folders.contains(parent));
    let Some((root, ..)) = roots.next().filter(|_| roots.next().is_none()) else {
        return (result, shared.items.len());
    };
    let unread: HashSet<Uuid> = shared.unread.iter().copied().collect();
    let mut matched: HashMap<Uuid, Uuid> = HashMap::new();
    let mut pending = vec![(*root, inv.root)];
    while let Some((old, live)) = pending.pop() {
        let Some(folder) = inv.folders.get(&live) else { continue };
        if matched.contains_key(&old) || unread.contains(&old) || folder.state != super::super::FetchState::Fetched {
            continue;
        }
        matched.insert(old, live);
        let Some(children) = branches.get(&old) else { continue };
        let mut unique = HashMap::new();
        for (id, name, kind) in children {
            unique.entry((*name, *kind)).and_modify(|entry| *entry = None).or_insert(Some(*id));
        }
        let mut current = HashMap::new();
        for child in folder
            .children
            .iter()
            .filter_map(|id| inv.folders.get(id))
            .filter(|f| !f.library && f.info.parent == live)
        {
            current
                .entry((child.info.name.as_str(), child.info.type_default))
                .and_modify(|entry| *entry = None)
                .or_insert(Some(child.info.id));
        }
        for (key, old) in unique {
            if let (Some(old), Some(Some(live))) = (old, current.get(&key)) {
                pending.push((old, *live));
            }
        }
    }
    let mut saved_names = HashMap::new();
    for (_, parent, name) in &shared.items {
        saved_names
            .entry(name.as_str())
            .and_modify(|entry| *entry = None)
            .or_insert(Some(*parent));
    }
    let mut current_names = HashMap::new();
    for item in inv
        .items
        .values()
        .filter(|it| inv.folders.get(&it.parent).is_some_and(|folder| !folder.library))
    {
        current_names
            .entry(item.name.as_str())
            .and_modify(|entry| *entry = None)
            .or_insert(Some(item.id));
    }
    for (name, parent) in saved_names {
        if let (Some(to), Some(Some(id))) = (parent.and_then(|p| matched.get(&p)), current_names.get(name)) {
            result.items.push((*id, *to, name.to_owned()));
        }
    }
    let unmatched = shared.items.len() - result.items.len();
    (result, unmatched)
}

/// The moves that put entries back where a snapshot saw them. Only the
/// user's own folders and items whose snapshot folder still exists and is
/// open; nothing out of the Trash (Undo only), nothing worn, linked, locked
/// or now in a hands-off place. `moved_by_runs` holds what Auto-sort runs
/// moved since the snapshot: anything else that moved was moved by hand and
/// is labelled apart so the preview shows it on its own.
pub fn restore_moves(
    inv: &Inventory,
    snapshot: &Snapshot,
    account: Uuid,
    moved_by_runs: &HashSet<Uuid>,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
    prefs: &super::Preferences,
) -> Result<(Vec<Move>, RestoreSummary), OtherAccount> {
    if !snapshot.account.is_nil() && snapshot.account != account {
        return Err(OtherAccount);
    }
    let holds_worn = actions::worn_ancestors(inv, worn);
    let trash = actions::system(inv, 14);
    let by_hand_label = aurora_i18n::tr!("inventory-sorter-moved-by-hand");
    let mut moves = Vec::new();
    let mut summary = RestoreSummary::default();
    // what the snapshot knew: the rest arrived after it
    let known: HashSet<Uuid> = snapshot
        .items
        .iter()
        .map(|(id, _, _)| *id)
        .chain(snapshot.folders.iter().map(|(id, ..)| *id))
        .collect();
    let mut targets = Targets::new(inv);
    let mut consider = |id: Uuid, folder: bool, name: &str, was_in: Uuid| {
        let Some(now) = actions::parent(inv, id) else {
            summary.gone += 1;
            return;
        };
        if now == was_in {
            return;
        }
        if trash.is_some_and(|t| actions::under(inv, id, t)) {
            summary.trashed += 1;
            return;
        }
        // a layout puts nothing back into Pre-Trash: what the user took
        // out of it stays out (only Undo of a Trash run goes back there)
        let into_pre_trash = targets.pre_trash.contains(&was_in);
        if !inv.folders.contains_key(&was_in)
            || into_pre_trash
            || !targets.allowed(was_in)
            || actions::ancestors(inv, was_in).any(|id| protected.contains(&id))
        {
            summary.folder_gone += 1;
            return;
        }
        // a folder excluded since the snapshot is neither read nor changed:
        // nothing leaves it, nothing goes back into it
        let excluded = prefs.excludes(inv, id) || prefs.excludes(inv, was_in) || folder && holds_closed(inv, id, &prefs.excluded);
        if excluded || !restorable(inv, id, protected, &holds_worn) || actions::under(inv, was_in, id) {
            summary.kept += 1;
            return;
        }
        let mut m = restore_move(id, folder, name, now, was_in, targets.label(was_in));
        m.snapshot = true;
        if !moved_by_runs.contains(&id) {
            summary.by_hand += 1;
            m.label = format!("{by_hand_label} · {}", m.label);
        }
        moves.push(m);
    };
    for (id, parent, name, kind) in &snapshot.folders {
        if *kind == -1 {
            consider(*id, true, name, *parent);
        }
    }
    for (id, parent, name) in &snapshot.items {
        consider(*id, false, name, *parent);
    }
    summary.new_since = inv
        .items
        .keys()
        .filter(|id| !known.contains(id) && !actions::library(inv, **id))
        .count();
    moves.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.name.cmp(&b.name)));
    Ok((moves, summary))
}

/// What Auto-sort runs moved at or after `since` (seconds): a restore tells
/// those apart from what was moved by hand.
pub fn moved_since(journal: &Journal, since: i64) -> HashSet<Uuid> {
    journal
        .runs
        .iter()
        .filter(|run| run.at >= since)
        .flat_map(|run| run.entries.iter().map(|e| e.id))
        .collect()
}
/// A run's moves left when the viewer stopped, to resume at the next login.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Unfinished {
    pub base: String,
    pub moves: Vec<Move>,
    /// The sort folder by id: a resumed run finds it even renamed.
    #[serde(default)]
    pub base_id: Option<Uuid>,
    /// An Undo stopped by the viewer: the run it undoes, and its own runs
    /// so far (forgotten with what they put back once it ends).
    #[serde(default)]
    pub undoes: Option<u64>,
    #[serde(default)]
    pub runs: Vec<u64>,
}

/// Where a layout saved to share goes: the Desktop, else Documents, else
/// the current folder; `when` (a date and time a file name may hold)
/// names it.
pub fn share_path(desktop: Option<PathBuf>, documents: Option<PathBuf>, here: PathBuf, when: &str) -> PathBuf {
    desktop
        .or(documents)
        .unwrap_or(here)
        .join(format!("Aurora inventory snapshot {when}.json"))
}

/// Write a layout to share as plain JSON (anyone can open it), through a
/// temporary file.
pub fn save_shared(path: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    // (the viewer's own safe writer: it waits for a file an antivirus or an
    // indexer holds, never loses the save)
    let bytes = serde_json::to_vec(snapshot).map_err(std::io::Error::other)?;
    crate::atomic_file::write(path, &bytes)
}

/// The folder of Auto-sort's files for one account.
pub fn folder(account: &Path) -> PathBuf {
    account.join("auto-sort")
}

/// Write `value` as gzip-compressed JSON, through a temporary file so a
/// crash never leaves half a file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    serde_json::to_writer(&mut gz, value).map_err(std::io::Error::other)?;
    // (the viewer's own safe writer: it waits for a file an antivirus or an
    // indexer holds, and is on the disk before it replaces the old one)
    crate::atomic_file::write(path, &gz.finish()?)
}

/// Read a file written by [`save`]; a missing or unreadable one is None
/// (logged by the caller).
pub fn load<T: for<'de> Deserialize<'de>>(path: &Path) -> std::io::Result<T> {
    let file = std::fs::File::open(path)?;
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    serde_json::from_reader(gz).map_err(std::io::Error::other)
}

/// Read a user-picked private backup (.json.gz) or older shared layout
/// (.json). Limit the decoded size as well as the compressed input.
pub fn load_layout_file(path: &Path) -> std::io::Result<Snapshot> {
    const LIMIT: u64 = 256 * 1024 * 1024;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > LIMIT {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "layout file is too large"));
    }
    let mut file = std::io::BufReader::new(file);
    let mut decoded: Box<dyn Read> = if file.fill_buf()?.starts_with(&[0x1f, 0x8b]) {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut bytes = Vec::new();
    decoded.by_ref().take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "decoded layout is too large"));
    }
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

pub fn journal_path(account: &Path) -> PathBuf {
    folder(account).join("journal.json.gz")
}

pub fn unfinished_path(account: &Path) -> PathBuf {
    folder(account).join("unfinished.json.gz")
}

pub fn snapshot_path(account: &Path, at: i64) -> PathBuf {
    folder(account).join(format!("snapshot-{at}.json.gz"))
}

/// The snapshots on disk, newest first: (time, path).
pub fn snapshots(account: &Path) -> Vec<(i64, PathBuf)> {
    let Ok(dir) = std::fs::read_dir(folder(account)) else {
        return Vec::new();
    };
    let mut found: Vec<(i64, PathBuf)> = dir
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let at = name.strip_prefix("snapshot-")?.strip_suffix(".json.gz")?.parse().ok()?;
            Some((at, entry.path()))
        })
        .collect();
    found.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    found
}

/// Keep the newest [`SNAPSHOTS`] snapshots.
pub fn prune_snapshots(account: &Path) {
    for (_, path) in snapshots(account).into_iter().skip(SNAPSHOTS) {
        if let Err(error) = std::fs::remove_file(&path) {
            log::warn!("auto-sort: could not remove an old snapshot {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
#[path = "sorter_journal_tests.rs"]
mod tests;
