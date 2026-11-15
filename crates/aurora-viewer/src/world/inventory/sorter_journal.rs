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

use super::{Dest, Inventory, Move, actions, folder_path, restorable, target_allowed};
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
    /// A new run, the oldest dropped past [`RUNS`].
    pub fn begin(&mut self, kind: RunKind, at: i64) {
        self.runs.push_back(Run {
            at,
            kind,
            entries: Vec::new(),
        });
        while self.runs.len() > RUNS {
            self.runs.pop_front();
        }
    }

    /// Confirmed moves of the run in progress.
    pub fn record(&mut self, entries: impl IntoIterator<Item = Entry>) {
        if let Some(run) = self.runs.back_mut() {
            run.entries.extend(entries);
        }
    }

    /// The last run that moved something.
    pub fn last(&self) -> Option<&Run> {
        self.runs.iter().rev().find(|run| !run.entries.is_empty())
    }

    /// Forget the last run once it is undone.
    pub fn drop_last(&mut self) {
        if let Some(index) = self.runs.iter().rposition(|run| !run.entries.is_empty()) {
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
        if !where_run_left || !target_allowed(inv, entry.from) || !restorable(inv, entry.id, protected, &holds_worn) {
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
    pub folders: Vec<(Uuid, Uuid, String, i32)>,
    pub items: Vec<(Uuid, Uuid, String)>,
}

impl Snapshot {
    /// My Inventory as it is now, the Library left out.
    pub fn take(inv: &Inventory, at: i64) -> Self {
        let folders = inv
            .folders
            .values()
            .filter(|f| !f.library)
            .map(|f| (f.info.id, f.info.parent, f.info.name.clone(), f.info.type_default))
            .collect();
        let items = inv
            .items
            .values()
            .filter(|it| !actions::library(inv, it.id))
            .map(|it| (it.id, it.parent, it.name.clone()))
            .collect();
        Self { at, folders, items }
    }
}

/// The moves that put entries back where a snapshot saw them: the user's
/// own folders and the items whose folder then still exists and is open
/// now. Entries now in a hands-off place stay (a worn item, an outfit, the
/// Current Outfit…); folders the snapshot had but that are gone are not
/// made again: their entries are counted as not restored.
pub fn restore_moves(inv: &Inventory, snapshot: &Snapshot, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> (Vec<Move>, usize) {
    let holds_worn = actions::worn_ancestors(inv, worn);
    let mut moves = Vec::new();
    let mut missing = 0;
    let mut consider = |id: Uuid, folder: bool, name: &str, was_in: Uuid| {
        let Some(now) = actions::parent(inv, id) else { return };
        if now == was_in {
            return;
        }
        if !inv.folders.contains_key(&was_in) || !target_allowed(inv, was_in) {
            missing += 1;
            return;
        }
        if restorable(inv, id, protected, &holds_worn) && !actions::under(inv, was_in, id) {
            moves.push(restore_move(inv, id, folder, name, now, was_in));
        }
    };
    for (id, parent, name, kind) in &snapshot.folders {
        if *kind == -1 {
            consider(*id, true, name, *parent);
        }
    }
    for (id, parent, name) in &snapshot.items {
        consider(*id, false, name, *parent);
    }
    moves.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.name.cmp(&b.name)));
    (moves, missing)
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
