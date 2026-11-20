//! The Chat Service's files: one CSV archive per resident, the index and
//! the deletion state, in the account's own folder. Port of the storage
//! half of LLChatServiceHistory (indra/newview/llchatservicehistory.cpp and
//! llchatservicehistorycore.cpp, LL viewer 26.5, LGPL 2.1): the same file
//! names, the same checks and the same order of writes, so that a folder
//! left by one viewer reads the same in the other.
//!
//! Everything here runs on the transcript thread (`aurora-chatlog`), in
//! turn with the transcript writes, as the LL viewer's General queue runs
//! them under its storage mutex. [`Storage`] sits behind a mutex too: the
//! stitched reads of the IM windows scan an archive under the same lock.
//!
//! The Rust [`ArchiveScan`] carries no file stamp, so the size and the
//! modification time of each archive, which publishRows and the repairs
//! compare before they touch a file, are kept here, per resident, from the
//! last scan or write.

use super::{IndexEntry, PreparedArchive, StateLoad};
use crate::world::chat_log::service::csv::{self, ArchiveScan, ArchiveState};
use aurora_llsd::{Llsd, Map};
use aurora_net::chat_service::{HistoryRow, UUID_TICK_LIMIT, parse_canonical_uuid};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;

/// The non-authoritative list of the archives (INDEX_NAME).
pub const INDEX_NAME: &str = "chat_service_index.csv";
/// The deletion cutoff and its pending flag (STATE_NAME).
pub const STATE_NAME: &str = "chat_service_state.xml";
/// The name of a resident not resolved when the index is written
/// (PENDING_NAME).
const PENDING_NAME: &str = "(name pending)";
const PREFIX: &str = "chat_service_";

/// The account's Chat Service folder for a grid:
/// `<account folder>/chat_service/<grid>`. The LL viewer writes these files
/// in the per-account folder, which the chat-logs folder preference never
/// moves; Aurora's account folder is the settings' one, so the archives
/// stay out of a chosen transcript folder and out of the Legacy layout,
/// which the Legacy Viewer reads too. The grid is part of the path like
/// the transcripts' (`chat_logs/<grid>`): an account id is only unique on
/// its grid.
pub fn folder(account_dir: &Path, grid: &str) -> PathBuf {
    let grid = crate::world::chat_log::names::clean_file_name(grid.trim());
    let grid = if grid.is_empty() { "grid".to_string() } else { grid };
    account_dir.join("chat_service").join(grid)
}

/// `chat_service_<uuid>.csv` (archiveName).
pub fn archive_name(resident: Uuid) -> String {
    format!("{PREFIX}{}.csv", resident.hyphenated())
}

/// The physical identity of an archive (archiveStamp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    mtime: SystemTime,
}

/// Absent (`Some(false)`), a regular file (`Some(true)`), or anything else:
/// a folder, a link, an error (`None`). Port of inspectRegular
/// (llchatservicehistorycore.cpp): owned names are never followed.
fn inspect_regular(path: &Path) -> Option<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => (m.file_type().is_file() && !m.file_type().is_symlink()).then_some(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
        Err(_) => None,
    }
}

fn stamp(path: &Path) -> Option<Stamp> {
    if inspect_regular(path) != Some(true) {
        return None;
    }
    let m = std::fs::metadata(path).ok()?;
    Some(Stamp {
        size: m.len(),
        mtime: m.modified().ok()?,
    })
}

/// LLFile::remove: a missing path is removed already.
fn remove(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// syncDirectory: the folder's entries on the disk. Windows commits a
/// rename with MOVEFILE_WRITE_THROUGH instead, as the LL viewer does.
#[cfg(unix)]
fn sync_directory(file: &Path) -> bool {
    file.parent()
        .and_then(|dir| std::fs::File::open(dir).ok())
        .is_some_and(|dir| dir.sync_all().is_ok())
}

#[cfg(not(unix))]
fn sync_directory(_: &Path) -> bool {
    true
}

/// writeFile: the whole file, flushed to the disk when `durable`.
fn write_file(path: &Path, contents: &[u8], durable: bool) -> bool {
    let written = (|| {
        let mut file = std::fs::File::create(path)?;
        file.write_all(contents)?;
        if durable {
            file.sync_all()?;
        }
        Ok::<_, std::io::Error>(())
    })();
    written.is_ok()
}

/// replaceFile: an atomic publication of `from` as `to`.
fn replace_file(from: &Path, to: &Path, durable: bool) -> bool {
    if !rename(from, to, durable) {
        return false;
    }
    !durable || sync_directory(to)
}

#[cfg(windows)]
fn rename(from: &Path, to: &Path, durable: bool) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let wide = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (from, to) = (wide(from), wide(to));
    let flags = MOVEFILE_REPLACE_EXISTING | if durable { MOVEFILE_WRITE_THROUGH } else { 0 };
    // SAFETY: both paths are NUL-terminated UTF-16 buffers that outlive the call.
    unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) != 0 }
}

#[cfg(not(windows))]
fn rename(from: &Path, to: &Path, _durable: bool) -> bool {
    std::fs::rename(from, to).is_ok()
}

/// writeReplace: a complete `.tmp` sibling, then the replacement; neither
/// path may be anything but a regular file or absent.
fn write_replace(destination: &Path, contents: &[u8], durable: bool) -> bool {
    let temporary = with_suffix(destination, ".tmp");
    inspect_regular(destination).is_some()
        && inspect_regular(&temporary).is_some()
        && write_file(&temporary, contents, durable)
        && replace_file(&temporary, destination, durable)
}

/// parseDecimalTicks: one canonical unsigned decimal below 2^60.
pub fn parse_decimal_ticks(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.is_empty() || (b.len() > 1 && b[0] == b'0') || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut parsed: u64 = 0;
    for &ch in b {
        let digit = u64::from(ch - b'0');
        if parsed > (UUID_TICK_LIMIT - 1 - digit) / 10 {
            return None;
        }
        parsed = parsed * 10 + digit;
    }
    Some(parsed)
}

/// The LLSD document of a state file, `None` when it does not parse. With
/// `whole`, nothing but white space may follow the document
/// (fromXMLDocument then `input >> std::ws; peek() == EOF`): the closing
/// tag ends the bytes and comes once.
fn state_document(bytes: &[u8], whole: bool) -> Option<Map> {
    if whole {
        let end = bytes.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(0, |i| i + 1);
        let closing = b"</llsd>";
        let count = bytes.windows(closing.len()).filter(|w| w == closing).count();
        if !bytes[..end].ends_with(closing) || count != 1 {
            return None;
        }
    }
    match aurora_llsd::from_xml(bytes).ok()? {
        Llsd::Map(m) => Some(m),
        _ => None,
    }
}

/// readStateFile: exactly the two canonical fields.
fn read_state_file(path: &Path) -> Option<(u64, bool)> {
    if inspect_regular(path) != Some(true) {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let map = state_document(&bytes, true)?;
    if map.len() != 2 {
        return None;
    }
    let (Some(Llsd::String(ticks)), Some(Llsd::Boolean(pending))) = (map.get("deleted_before_uuid_ticks"), map.get("cleanup_pending"))
    else {
        return None;
    };
    Some((parse_decimal_ticks(ticks)?, *pending))
}

/// recoverBoundaryCandidate: a canonical cutoff salvaged from a damaged
/// state; 0 when there is none.
fn recover_boundary_candidate(path: &Path) -> u64 {
    if inspect_regular(path) != Some(true) {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    match state_document(&bytes, false)
        .as_ref()
        .and_then(|m| m.get("deleted_before_uuid_ticks"))
    {
        Some(Llsd::String(ticks)) => parse_decimal_ticks(ticks).unwrap_or(0),
        _ => 0,
    }
}

/// stateContents: LLSD pretty XML, as LLSDSerialize::toPrettyXML writes it.
pub fn state_contents(boundary: u64, pending: bool) -> String {
    let mut map = Map::new();
    map.insert("deleted_before_uuid_ticks".into(), Llsd::String(boundary.to_string()));
    map.insert("cleanup_pending".into(), Llsd::Boolean(pending));
    aurora_llsd::to_pretty_xml_string(&Llsd::Map(map))
}

/// The CSV row of the index for one archive (regenerateIndex).
fn index_row(out: &mut String, resident: Uuid, name: Option<&crate::world::names::AvatarName>) {
    let account = name.map_or(PENDING_NAME, |n| n.username.as_str());
    let display = name.map_or(PENDING_NAME, |n| n.display_name.as_str());
    out.push_str(&csv::quote_csv(&archive_name(resident)));
    out.push(',');
    out.push_str(&resident.hyphenated().to_string());
    out.push(',');
    out.push_str(&csv::quote_csv(account));
    out.push(',');
    out.push_str(&csv::quote_csv(display));
    out.push('\n');
}

/// canonicalArchiveName: `chat_service_<canonical uuid>.csv`.
pub fn canonical_archive_name(name: &str) -> Option<Uuid> {
    let id = name.strip_prefix(PREFIX)?.strip_suffix(".csv")?;
    if id.len() != 36 {
        return None;
    }
    parse_canonical_uuid(id)
}

/// ownedArtifactName: the names this feature writes, and nothing else.
pub fn owned_artifact_name(name: &str) -> bool {
    if is_index_or_state(name) {
        return true;
    }
    let Some(rest) = name.strip_prefix(PREFIX) else {
        return false;
    };
    if rest.len() <= 36 || !rest.is_char_boundary(36) || parse_canonical_uuid(&rest[..36]).is_none() {
        return false;
    }
    matches!(&rest[36..], ".csv" | ".csv.tmp" | ".csv.corrupt")
}

fn is_index_or_state(name: &str) -> bool {
    name == INDEX_NAME
        || name.strip_suffix(".tmp") == Some(INDEX_NAME)
        || name == STATE_NAME
        || name.strip_suffix(".tmp") == Some(STATE_NAME)
}

/// The names of the folder that begin with `chat_service_`
/// (LLDirIterator with that pattern). A missing folder has none.
fn listed(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(PREFIX))
        .collect()
}

/// One account's Chat Service folder, with the stamps of its archives.
#[derive(Debug)]
pub struct Storage {
    dir: PathBuf,
    agent: Uuid,
    stamps: HashMap<Uuid, Stamp>,
}

impl Storage {
    pub fn new(dir: PathBuf, agent: Uuid) -> Storage {
        Storage {
            dir,
            agent,
            stamps: HashMap::new(),
        }
    }

    fn archive_path(&self, resident: Uuid) -> PathBuf {
        self.dir.join(archive_name(resident))
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join(STATE_NAME)
    }

    /// The folder for a write; the LL viewer's per-account folder always
    /// exists, this one is made on the first write.
    fn ensure_dir(&self) -> bool {
        std::fs::create_dir_all(&self.dir).is_ok()
    }

    /// scanArchive on the file: the stamp first, then the bytes. Absent is
    /// a valid empty archive, anything not a regular file fails.
    fn scan(&self, resident: Uuid, deleted_before_ticks: u64, display_cap: usize) -> (ArchiveScan, Option<Stamp>) {
        let path = self.archive_path(resident);
        let failed = || {
            (
                ArchiveScan {
                    state: ArchiveState::Failed,
                    ..ArchiveScan::default()
                },
                None,
            )
        };
        match inspect_regular(&path) {
            None => return failed(),
            Some(false) => {
                return (
                    ArchiveScan {
                        state: ArchiveState::Absent,
                        ..ArchiveScan::default()
                    },
                    None,
                );
            }
            Some(true) => {}
        }
        let Some(stamp) = stamp(&path) else {
            return failed();
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return failed();
        };
        let scan = csv::scan_archive(&bytes, self.agent, resident, deleted_before_ticks, display_cap);
        (scan, Some(stamp))
    }

    /// A view's read of an archive: the newest `display_cap` rows above the
    /// cutoff. Changes nothing (readStitched's scan).
    pub fn read_archive(&self, resident: Uuid, deleted_before_ticks: u64, display_cap: usize) -> ArchiveScan {
        self.scan(resident, deleted_before_ticks, display_cap).0
    }

    /// loadState: a valid state file is authoritative (a leftover `.tmp`
    /// is removed); neither file is a fresh account; anything else hides
    /// all history until a delete.
    pub fn load_state(&self) -> StateLoad {
        let path = self.state_path();
        let temporary = with_suffix(&path, ".tmp");
        let canonical = inspect_regular(&path);
        let temp = inspect_regular(&temporary);
        if canonical == Some(true)
            && let Some((deleted_before_ticks, cleanup_pending)) = read_state_file(&path)
        {
            if temp != Some(false) && !(remove(&temporary) && sync_directory(&path)) {
                return StateLoad::Unsafe;
            }
            return StateLoad::Safe {
                deleted_before_ticks,
                cleanup_pending,
            };
        }
        if canonical == Some(false) && temp == Some(false) {
            return StateLoad::Safe {
                deleted_before_ticks: 0,
                cleanup_pending: false,
            };
        }
        StateLoad::Unsafe
    }

    /// enumerateArchives: the residents with a canonical archive, and
    /// whether any owned file other than the index and the state exists.
    pub fn enumerate(&self) -> (Vec<Uuid>, bool) {
        let mut residents = Vec::new();
        let mut artifacts = false;
        for name in listed(&self.dir) {
            if let Some(id) = canonical_archive_name(&name) {
                residents.push(id);
            }
            if owned_artifact_name(&name) && !is_index_or_state(&name) {
                artifacts = true;
            }
        }
        residents.sort();
        (residents, artifacts)
    }

    /// prepareArchive: a valid or absent archive gives its summary; a torn
    /// tail is cut at the last complete row and a corrupt file is set aside
    /// as `.corrupt`, each only while the file is still the one scanned.
    pub fn prepare_archive(&mut self, resident: Uuid, deleted_before_ticks: u64) -> Option<PreparedArchive> {
        let path = self.archive_path(resident);
        inspect_regular(&path)?;
        let (scan, scanned) = self.scan(resident, deleted_before_ticks, 0);
        match scan.state {
            ArchiveState::Valid | ArchiveState::Absent => {
                self.set_stamp(resident, scanned);
                Some(PreparedArchive {
                    summary: scan,
                    bytes_changed: false,
                })
            }
            ArchiveState::Torn => {
                let current = stamp(&path);
                if scanned.is_none() || current != scanned || !truncate(&path, scan.valid_prefix_bytes) {
                    return None;
                }
                let (rescan, stamp) = self.scan(resident, deleted_before_ticks, 0);
                if rescan.state != ArchiveState::Valid {
                    return None;
                }
                self.set_stamp(resident, stamp);
                Some(PreparedArchive {
                    summary: rescan,
                    bytes_changed: true,
                })
            }
            ArchiveState::Corrupt => {
                let corrupt = with_suffix(&path, ".corrupt");
                if !remove(&corrupt) {
                    return None;
                }
                let current = stamp(&path);
                if scanned.is_none() || current != scanned || std::fs::rename(&path, &corrupt).is_err() {
                    return None;
                }
                self.stamps.remove(&resident);
                Some(PreparedArchive {
                    summary: ArchiveScan::default(),
                    bytes_changed: true,
                })
            }
            ArchiveState::Failed => None,
        }
    }

    fn set_stamp(&mut self, resident: Uuid, stamp: Option<Stamp>) {
        match stamp {
            Some(s) => self.stamps.insert(resident, s),
            None => self.stamps.remove(&resident),
        };
    }

    /// publishRows: the rows of a complete pass, oldest first, appended to
    /// the valid archive or written as a new one through its `.tmp`. The
    /// file must still be the one `expected` describes (present or absent,
    /// same size and time); the answer is the summary advanced by the rows.
    pub fn publish_rows(&mut self, resident: Uuid, rows: &[HistoryRow], append: bool, expected: ArchiveScan) -> Option<ArchiveScan> {
        let mut resulting = expected;
        if rows.is_empty() {
            return Some(resulting);
        }
        let path = self.archive_path(resident);
        let exists = inspect_regular(&path)?;
        if exists != (resulting.state != ArchiveState::Absent) {
            return None;
        }
        if exists && (stamp(&path).is_none() || stamp(&path) != self.stamps.get(&resident).copied()) {
            return None;
        }
        if !self.ensure_dir() {
            return None;
        }
        let mut out = String::new();
        if append {
            let Ok(meta) = std::fs::metadata(&path) else {
                return None;
            };
            if !meta.is_file() {
                return None;
            }
            if meta.len() > 0 && last_byte(&path)? != b'\n' {
                out.push('\n');
            }
            for row in rows {
                csv::write_csv_row(&mut out, row);
            }
            let appended = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .and_then(|mut f| f.write_all(out.as_bytes()).and_then(|()| f.flush()));
            if appended.is_err() {
                return None;
            }
        } else {
            out.push_str(csv::CSV_HEADER);
            for row in rows {
                csv::write_csv_row(&mut out, row);
            }
            if !write_replace(&path, out.as_bytes(), false) {
                return None;
            }
        }
        resulting.state = ArchiveState::Valid;
        if resulting.oldest.is_none() {
            resulting.oldest = Some(rows[0].key);
        }
        resulting.row_count = resulting.row_count.saturating_add(u32::try_from(rows.len()).unwrap_or(u32::MAX));
        resulting.newest = rows.last().map(|r| r.key);
        let written = stamp(&path)?;
        self.stamps.insert(resident, written);
        resulting.valid_prefix_bytes = written.size;
        Some(resulting)
    }

    /// regenerateIndex: one row per archive still present, names spelt out
    /// or pending; no archive left removes the index.
    pub fn regenerate_index(&self, entries: &[IndexEntry]) -> bool {
        let index = self.dir.join(INDEX_NAME);
        let temporary = with_suffix(&index, ".tmp");
        for (path, is_temp) in [(&index, false), (&temporary, true)] {
            let regular = inspect_regular(path);
            if (regular.is_none() || (is_temp && regular == Some(true))) && !remove(path) {
                return false;
            }
        }
        let mut sorted: Vec<&IndexEntry> = entries.iter().collect();
        sorted.sort_by_key(|e| e.resident);
        sorted.dedup_by_key(|e| e.resident);
        let mut out = String::from(csv::INDEX_HEADER);
        let mut any = false;
        for e in sorted {
            if inspect_regular(&self.archive_path(e.resident)) != Some(true) {
                continue;
            }
            any = true;
            index_row(&mut out, e.resident, e.name.as_ref());
        }
        if !any {
            return remove(&index);
        }
        self.ensure_dir() && write_replace(&index, out.as_bytes(), false)
    }

    /// Delete step 1 (runDelete): the cutoff is the click's, or a later one
    /// salvaged from the state file or its `.tmp`, written durably with
    /// cleanup pending. An unreadable state is replaced through the index's
    /// scratch name (recoverStateForDelete). The answer is the cutoff
    /// written, `None` on failure.
    pub fn begin_delete_state(&mut self, click_ticks: u64) -> Option<u64> {
        if !self.ensure_dir() {
            return None;
        }
        let path = self.state_path();
        let temporary = with_suffix(&path, ".tmp");
        let boundary = click_ticks.max(recover_boundary_candidate(&path).max(recover_boundary_candidate(&temporary)));
        let canonical_valid = read_state_file(&path).is_some();
        let written = if canonical_valid {
            clear_unsafe_state_temporary(&path) && write_replace(&path, state_contents(boundary, true).as_bytes(), true)
        } else {
            self.recover_state_for_delete(&path, boundary)
        };
        written.then_some(boundary)
    }

    /// recoverStateForDelete: an unsafe state stays in place until a
    /// durable pending cutoff has reached the state's `.tmp`.
    fn recover_state_for_delete(&self, path: &Path, boundary: u64) -> bool {
        let temporary = with_suffix(path, ".tmp");
        let recovery = with_suffix(&self.dir.join(INDEX_NAME), ".tmp");
        remove(&recovery)
            && write_file(&recovery, state_contents(boundary, true).as_bytes(), true)
            && replace_file(&recovery, &temporary, true)
            && remove(path)
            && replace_file(&temporary, path, true)
    }

    /// sweepServiceArtifacts: every owned file but the state, then the
    /// state's `.tmp`. Every path is tried; a missing one is removed
    /// already. The stamps go with the archives.
    pub fn sweep_service_artifacts(&mut self) -> bool {
        let state = self.state_path();
        let mut targets: Vec<PathBuf> = listed(&self.dir)
            .into_iter()
            .filter(|n| owned_artifact_name(n) && n != STATE_NAME)
            .map(|n| self.dir.join(n))
            .collect();
        targets.push(with_suffix(&state, ".tmp"));
        let mut ok = true;
        for path in &targets {
            ok &= remove(path);
        }
        self.stamps.clear();
        ok && sync_directory(&state)
    }

    /// writeState: the final durable state of a delete.
    pub fn write_state(&self, boundary: u64, pending: bool) -> bool {
        self.ensure_dir() && write_replace(&self.state_path(), state_contents(boundary, pending).as_bytes(), true)
    }
}

/// clearUnsafeStateTemporary: a state `.tmp` that is not a regular file
/// (or absent) is removed before the durable replacement reuses the name.
fn clear_unsafe_state_temporary(path: &Path) -> bool {
    let temporary = with_suffix(path, ".tmp");
    if inspect_regular(&temporary).is_some() {
        return true;
    }
    remove(&temporary) && sync_directory(path)
}

/// truncateArchive: cut the torn tail, keeping the valid prefix.
fn truncate(path: &Path, bytes: u64) -> bool {
    inspect_regular(path) == Some(true)
        && std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|f| f.set_len(bytes))
            .is_ok()
}

fn last_byte(path: &Path) -> Option<u8> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::End(-1)).ok()?;
    let mut b = [0u8; 1];
    f.read_exact(&mut b).ok()?;
    Some(b[0])
}

#[cfg(test)]
mod tests;
