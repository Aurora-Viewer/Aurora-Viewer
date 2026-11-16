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

use super::{Dest, Inventory, Move, actions, folder_path, restorable, restore_target_allowed};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
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
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    pub runs: VecDeque<Run>,
}

impl Journal {
    /// A new run, the oldest dropped past [`RUNS`]; its id.
    pub fn begin(&mut self, kind: RunKind, at: i64) -> u64 {
        let id = self.runs.iter().map(|run| run.id).max().unwrap_or(0) + 1;
        self.runs.push_back(Run {
            id,
            at,
            kind,
            entries: Vec::new(),
        });
        while self.runs.len() > RUNS {
            self.runs.pop_front();
        }
        id
    }

    /// Runs read from an older file have no id: numbered in order.
    pub fn number_runs(&mut self) {
        let mut next = self.runs.iter().map(|run| run.id).max().unwrap_or(0);
        for run in self.runs.iter_mut().filter(|run| run.id == 0) {
            next += 1;
            run.id = next;
        }
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

    /// The last run that moved something.
    pub fn last(&self) -> Option<&Run> {
        self.runs.iter().rev().find(|run| !run.entries.is_empty())
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
    let mut moves = Vec::new();
    let mut skipped = 0;
    // the newest move of an entry wins: undo puts it where it was first
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
        if !where_run_left || !restore_target_allowed(inv, entry.from) || !restorable(inv, entry.id, protected, &holds_worn) {
            skipped += 1;
            continue;
        }
        moves.push(restore_move(inv, entry.id, entry.folder, &entry.name, now, entry.from));
    }
    moves.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.name.cmp(&b.name)));
    (moves, skipped)
}

fn restore_move(inv: &Inventory, id: Uuid, folder: bool, name: &str, from: Uuid, to: Uuid) -> Move {
    Move {
        id,
        folder,
        name: name.to_owned(),
        from,
        to: Dest::Folder(to),
        label: folder_path(inv, to),
        unpacker: false,
        restore: true,
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
        if !inv.folders.contains_key(&was_in) || !restore_target_allowed(inv, was_in) {
            summary.folder_gone += 1;
            return;
        }
        if !restorable(inv, id, protected, &holds_worn) || actions::under(inv, was_in, id) {
            summary.kept += 1;
            return;
        }
        let mut m = restore_move(inv, id, folder, name, now, was_in);
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
}

/// The folder of Auto-sort's files for one account.
pub fn folder(account: &Path) -> PathBuf {
    account.join("auto-sort")
}

/// Write `value` as gzip-compressed JSON, through a temporary file so a
/// crash never leaves half a file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("tmp");
    {
        let file = std::fs::File::create(&temporary)?;
        let mut gz = flate2::write::GzEncoder::new(std::io::BufWriter::new(file), flate2::Compression::fast());
        serde_json::to_writer(&mut gz, value).map_err(std::io::Error::other)?;
        gz.finish()?.flush()?;
    }
    std::fs::rename(&temporary, path)
}

/// Read a file written by [`save`]; a missing or unreadable one is None
/// (logged by the caller).
pub fn load<T: for<'de> Deserialize<'de>>(path: &Path) -> std::io::Result<T> {
    let file = std::fs::File::open(path)?;
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    serde_json::from_reader(gz).map_err(std::io::Error::other)
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
