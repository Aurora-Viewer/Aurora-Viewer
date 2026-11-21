//! Teleport history: the navigation bar's back / forward list (behaviour of
//! LLTeleportHistory: arrivals after a teleport are recorded, going back or
//! forward moves within the list without truncating it, goToItem jumps to
//! any of its places) and the dated,
//! saved history of the Places window (LLTeleportHistoryStorage,
//! `teleport_history.txt` of the account, one LLSD notation line per place;
//! indra/newview/llteleporthistory.cpp, llteleporthistorystorage.cpp,
//! llpanelteleporthistory.cpp, originally LGPL 2.1).

use aurora_llsd::Llsd;
use aurora_net::RegionHandle;
use crossbeam_channel::{Receiver, TryRecvError};
use glam::{DVec3, Vec3};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

const MAX_ITEMS: usize = 100;

#[derive(Debug, Clone)]
pub struct TpEntry {
    pub region: String,
    pub handle: RegionHandle,
    /// Region-local position.
    pub position: Vec3,
    /// The parcel, once it is known (empty until then, or hidden by RLVa).
    pub parcel: String,
    /// Arrival, in seconds since the epoch (UTC).
    pub at: i64,
    /// On an OpenSim grid, the grid of the place as a Hypergrid address
    /// (`grids::here`), once its region's SimulatorFeatures told it; None
    /// on Second Life or until then. Back / Forward to a place of another
    /// grid go through that grid's gatekeeper (LLTeleportHistory::goToItem).
    pub grid: Option<String>,
}

impl TpEntry {
    /// LLTeleportHistoryItem::mTitle: "Parcel, Region".
    pub fn title(&self) -> String {
        location_title(&self.parcel, &self.region)
    }

    /// The place of an arrival under @showloc: a name that tells nothing and
    /// no destination, as `LLTeleportHistoryItem(RlvStrings::getString(
    /// RlvStringKeys::Hidden::Parcel), LLVector3d::zero)` of
    /// LLTeleportHistory::updateCurrentLocation.
    pub fn hidden(label: &str, at: i64) -> TpEntry {
        TpEntry {
            region: label.to_owned(),
            handle: 0,
            position: Vec3::ZERO,
            parcel: String::new(),
            at,
            grid: None,
        }
    }

    /// `mGlobalPos.isExactlyZero()`: the place leads nowhere. No region has
    /// the handle 0 (the grid's origin is empty).
    pub fn is_hidden(&self) -> bool {
        self.handle == 0 && self.position == Vec3::ZERO
    }
}

/// The places of the session, oldest first, and the one we are in: what
/// the arrows' list shows (LLTeleportHistory::getItems,
/// getCurrentItemIndex).
#[derive(Debug, Clone, Copy, Default)]
pub struct Session<'a> {
    pub items: &'a [TpEntry],
    pub current: usize,
}

#[derive(Debug, Default)]
pub struct TeleportHistory {
    items: Vec<TpEntry>,
    current: usize,
    /// Index being travelled to with back / forward.
    pending: Option<usize>,
}

impl TeleportHistory {
    /// Arrival at a new location (login or teleport); under @showloc the
    /// caller gives `TpEntry::hidden`. The RLVa rules of
    /// LLTeleportHistory::updateCurrentLocation:
    /// - back / forward under @showloc moves in the list and leaves the place
    ///   reached as it was;
    /// - "only append a new item if the list is currently empty or if not
    ///   @showloc=n restricted and the last entry wasn't zero'ed out": a
    ///   hidden arrival takes the place of the one we left, and the next
    ///   arrival takes the place of a hidden one. The session never holds
    ///   two hidden places in a row, nor a count of the teleports made.
    pub fn arrived(&mut self, entry: TpEntry) {
        if let Some(i) = self.pending.take()
            && i < self.items.len()
        {
            self.current = i;
            if !entry.is_hidden() {
                self.items[i] = entry;
            }
            return;
        }
        if !self.items.is_empty() {
            self.items.truncate(self.current + 1);
        }
        match self.items.last_mut() {
            Some(last) if entry.is_hidden() || last.is_hidden() => *last = entry,
            _ => {
                self.items.push(entry);
                if self.items.len() > MAX_ITEMS {
                    self.items.remove(0);
                }
            }
        }
        self.current = self.items.len() - 1;
    }

    /// The teleport did not happen.
    pub fn failed(&mut self) {
        self.pending = None;
    }

    /// The demo's earlier places, oldest first: the next arrival comes after
    /// them.
    pub fn seeded(items: Vec<TpEntry>) -> TeleportHistory {
        let current = items.len().saturating_sub(1);
        TeleportHistory {
            items,
            current,
            pending: None,
        }
    }

    pub fn session(&self) -> Session<'_> {
        Session {
            items: &self.items,
            current: self.current,
        }
    }

    /// The parcel of the current place, once ParcelProperties brought it
    /// (LLTeleportHistory::updateCurrentLocation after the teleport
    /// finished signal of LLViewerParcelMgr).
    pub fn name_parcel(&mut self, parcel: &str) {
        if let Some(e) = self.items.get_mut(self.current) {
            e.parcel = parcel.trim().to_owned();
        }
    }

    /// The current place's grid, once its region's SimulatorFeatures came
    /// (they follow the arrival).
    pub fn set_grid(&mut self, handle: RegionHandle, grid: Option<String>) {
        if let Some(e) = self.items.get_mut(self.current)
            && e.handle == handle
            && e.grid.is_none()
        {
            e.grid = grid;
        }
    }

    /// Update the current entry's region name once it is known.
    pub fn name_current(&mut self, handle: RegionHandle, name: &str) {
        if let Some(e) = self.items.get_mut(self.current)
            && e.handle == handle
            && e.region.is_empty()
        {
            e.region = name.to_owned();
        }
    }

    pub fn previous(&self) -> Option<&TpEntry> {
        self.current.checked_sub(1).and_then(|i| self.items.get(i))
    }

    /// @showloc set or released (RlvUIEnabler::onToggleShowLoc →
    /// LLTeleportHistory::updateCurrentLocation): the places ahead go, and
    /// the current one is named `region` with no destination left, as the
    /// zeroed item of the Legacy Viewer. It stays so while the restriction
    /// holds; once released, `locate_current` says where we are. Returns
    /// the place as it was.
    pub fn replace_current_region(&mut self, region: &str) -> Option<TpEntry> {
        let current = self.items.get_mut(self.current)?;
        let hidden = TpEntry::hidden(region, current.at);
        let previous = std::mem::replace(current, hidden);
        self.items.truncate(self.current + 1);
        Some(previous)
    }

    /// The current place, when it has no destination (a place @showloc hid,
    /// now released), is where the agent stands.
    pub fn locate_current(&mut self, place: TpEntry) {
        if let Some(e) = self.items.get_mut(self.current)
            && e.is_hidden()
        {
            *e = place;
        }
    }

    pub fn next(&self) -> Option<&TpEntry> {
        self.items.get(self.current + 1)
    }

    /// The place the Back arrow leads to, None when there is none before
    /// this one. LLNavigationBar::onTeleportHistoryChanged
    /// (indra/newview/llnavigationbar.cpp) enables the arrows from the
    /// current index alone: a place @showloc hid keeps its arrow, under the
    /// name it was given (`TpEntry::hidden`).
    pub fn back_name(&self) -> Option<String> {
        self.previous().map(|e| e.region.clone())
    }

    pub fn forward_name(&self) -> Option<String> {
        self.next().map(|e| e.region.clone())
    }

    /// Start travelling back; returns the destination.
    pub fn go_back(&mut self) -> Option<TpEntry> {
        self.go_to(self.current.checked_sub(1)?)
    }

    pub fn go_forward(&mut self) -> Option<TpEntry> {
        self.go_to(self.current + 1)
    }

    /// LLTeleportHistory::goToItem (indra/newview/llteleporthistory.cpp):
    /// start travelling to the place `i` of the list; none for the place we
    /// are in or past the end. A place @showloc hid is asked for like
    /// another: goToItem hands its zeroed global position to
    /// LLAgent::teleportViaLocation (indra/newview/llagent.cpp), which sends
    /// a TeleportLocationRequest for the region handle 0 at the position
    /// zero. The grid refuses it, and the failure leaves the list where it
    /// was (`failed`).
    pub fn go_to(&mut self, i: usize) -> Option<TpEntry> {
        if i == self.current {
            return None;
        }
        let e = self.items.get(i).cloned()?;
        self.pending = Some(i);
        Some(e)
    }

    /// LLTeleportHistory::purgeItems: only the current place stays.
    pub fn purge(&mut self) {
        if let Some(e) = self.items.get(self.current).cloned() {
            self.items = vec![e];
        }
        self.current = 0;
        self.pending = None;
    }
}

/// File of the saved history in the account folder.
pub const TELEPORT_HISTORY_FILE: &str = "teleport_history.txt";
const MAX_LEGACY_HISTORY_BYTES: u64 = 16 * 1024 * 1024;
const LEGACY_IMPORT_FILE: &str = "teleport_history_import.json";
/// Two places closer than this with the same title are one
/// (MAX_GLOBAL_POS_OFFSET).
const MAX_GLOBAL_POS_OFFSET: f64 = 5.0;

/// One place of the saved history (LLTeleportHistoryPersistentItem).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryItem {
    /// "Parcel, Region" (LLAgentUI::LOCATION_FORMAT_NORMAL), or the region.
    pub title: String,
    pub global: DVec3,
    /// Seconds since the epoch (UTC).
    pub date: f64,
    /// Firestorm's OpenSim SLURL, kept as read.
    pub slurl: String,
}

impl HistoryItem {
    fn from_llsd(v: &Llsd) -> HistoryItem {
        let p = v["global_pos"].as_array();
        let c = |i: usize| p.get(i).map_or(0.0, Llsd::as_f64);
        HistoryItem {
            title: v["title"].as_str().to_owned(),
            global: DVec3::new(c(0), c(1), c(2)),
            date: v["date"].as_f64(),
            slurl: v["slurl"].as_str().to_owned(),
        }
    }

    fn to_llsd(&self) -> Llsd {
        let mut m = Llsd::new_map();
        m.insert("title", self.title.clone());
        m.insert(
            "global_pos",
            Llsd::Array(vec![
                Llsd::Real(self.global.x),
                Llsd::Real(self.global.y),
                Llsd::Real(self.global.z),
            ]),
        );
        m.insert("date", Llsd::Date(self.date));
        m.insert("slurl", self.slurl.clone());
        m
    }

    /// All four stored fields identify a visit. Treat signed zero alike,
    /// as numeric equality does, while hashing the remaining values exactly.
    fn key(&self) -> (&str, [u64; 4], &str) {
        let numbers = [self.global.x, self.global.y, self.global.z, self.date];
        let bits = numbers.map(|x| if x == 0.0 { 0 } else { x.to_bits() });
        (&self.title, bits, &self.slurl)
    }

    /// Keep a stable fingerprint without storing the deleted destination.
    fn fingerprint(&self) -> [u8; 32] {
        let (title, numbers, slurl) = self.key();
        let mut hash = Sha256::new();
        hash.update((title.len() as u64).to_le_bytes());
        hash.update(title);
        for number in numbers {
            hash.update(number.to_le_bytes());
        }
        hash.update(slurl);
        hash.finalize().into()
    }
}

#[derive(Debug, Default, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
struct ImportPolicy {
    #[serde(default)]
    initial_import_checked: bool,
    cleared_through: Option<f64>,
    removed: BTreeSet<[u8; 32]>,
}

impl ImportPolicy {
    fn load(history: &Path) -> Option<Arc<Self>> {
        match std::fs::read(history.with_file_name(LEGACY_IMPORT_FILE)) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(policy) => Some(Arc::new(policy)),
                Err(e) => {
                    log::warn!("teleport history: can't read import preferences ({e}); import disabled");
                    None
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Arc::default()),
            Err(e) => {
                log::warn!("teleport history: can't open import preferences ({e}); import disabled");
                None
            }
        }
    }

    fn permits(&self, visit: &HistoryItem) -> bool {
        self.cleared_through.is_none_or(|date| visit.date > date)
            && (self.removed.is_empty() || !self.removed.contains(&visit.fingerprint()))
    }
}

#[derive(Debug, Default)]
struct SavedHistory {
    bytes: Vec<u8>,
    policy: Option<Arc<ImportPolicy>>,
}

#[derive(Debug, Default)]
struct LegacyRead {
    items: Vec<HistoryItem>,
    found: bool,
    failed: bool,
}

pub struct LegacyImportReport {
    pub manual: bool,
    pub changed: bool,
    pub added: usize,
    pub found: bool,
    pub failed: bool,
}

/// The saved history, oldest first (LLTeleportHistoryStorage::mItems).
///
/// The file is never lost to a save (#2626): it is replaced atomically, on
/// the disk before the old one goes (a crash or a killed viewer during a
/// save leaves the old file, where a plain write left it empty); a file
/// that exists but cannot be read is never saved over in that session; and
/// the lines this viewer cannot parse are written back as they were. The
/// Legacy Viewer stops at the first bad line and saves over the rest.
#[derive(Debug, Default)]
pub struct HistoryStorage {
    pub items: Vec<HistoryItem>,
    /// Lines of the file that did not parse, kept for the next save.
    kept: Vec<Vec<u8>>,
    /// None: not saved (demo, before login, or a file that could not be read).
    path: Option<PathBuf>,
    import_policy: Option<Arc<ImportPolicy>>,
    legacy_history: Option<(Receiver<LegacyRead>, bool)>,
}

impl HistoryStorage {
    /// LLTeleportHistoryStorage::load, then sorted by date. A line that does
    /// not parse is kept as it is for the next save (the Legacy Viewer stops
    /// reading there); a file that exists but cannot be read is left alone
    /// for the session (not saved over).
    pub fn load(path: PathBuf) -> HistoryStorage {
        // A previous session may still be publishing this account's latest
        // history. Reading its snapshot also avoids waiting for a slow disk.
        let saved = match writers().snapshot(&path) {
            Some(saved) => saved,
            None => match std::fs::read(&path) {
                Ok(bytes) => Arc::new(SavedHistory {
                    bytes,
                    policy: ImportPolicy::load(&path),
                }),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Arc::new(SavedHistory {
                    bytes: Vec::new(),
                    policy: ImportPolicy::load(&path),
                }),
                Err(e) => {
                    log::warn!("teleport history: can't read the saved history ({e}); it is left as it is and not saved this session");
                    return HistoryStorage::default();
                }
            },
        };
        let (mut items, kept) = parse(&saved.bytes);
        if !kept.is_empty() {
            log::info!(
                "teleport history: {} line(s) could not be read; they are kept in the file",
                kept.len()
            );
        }
        items.sort_by(|a, b| a.date.total_cmp(&b.date));
        HistoryStorage {
            items,
            kept,
            path: Some(path),
            import_policy: saved.policy.clone(),
            legacy_history: None,
        }
    }

    /// Discover and parse Legacy Viewer files off the frame. The result belongs
    /// to this session's storage and is dropped with it on logout.
    pub fn start_legacy_import(
        &mut self,
        account: crate::legacy_profile::Account,
        apps: Vec<PathBuf>,
        chosen: Option<PathBuf>,
        manual: bool,
    ) {
        // The destination is keyed by account UUID, not grid. Aditi can
        // share a UUID with Agni, and custom grid folder labels are ambiguous.
        if account.grid != "agni" || !self.import_available() || self.import_busy() || (!manual && !self.check_initial_import()) {
            return;
        }
        let (sender, receiver) = crossbeam_channel::bounded(1);
        match std::thread::Builder::new().name("aurora-legacy-teleports".into()).spawn(move || {
            let _ = sender.send(read_legacy_history(&account, &apps, chosen));
        }) {
            Ok(_) => self.legacy_history = Some((receiver, manual)),
            Err(e) => log::warn!("teleport history: can't start the Legacy Viewer import ({e})"),
        }
    }

    pub fn import_available(&self) -> bool {
        self.path.is_some() && self.import_policy.is_some()
    }

    pub fn import_busy(&self) -> bool {
        self.legacy_history.is_some()
    }

    fn check_initial_import(&mut self) -> bool {
        let Some(policy) = &mut self.import_policy else {
            return false;
        };
        if policy.initial_import_checked {
            return false;
        }
        let empty = self.items.is_empty() && self.kept.is_empty() && policy.removed.is_empty() && policy.cleared_through.is_none();
        // Record the choice even when the history already exists or no
        // source is found. Later logins never re-seed a history.
        Arc::make_mut(policy).initial_import_checked = true;
        self.save();
        empty
    }

    pub fn poll_legacy_import(&mut self) -> Option<LegacyImportReport> {
        let (receiver, manual) = self.legacy_history.as_ref()?;
        let manual = *manual;
        let read = match receiver.try_recv() {
            Ok(read) => read,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => LegacyRead {
                failed: true,
                ..Default::default()
            },
        };
        self.legacy_history = None;
        let added = self.merge_legacy(read.items);
        if let Some(added) = added.filter(|&added| added > 0) {
            log::info!("teleport history: imported {added} visit(s) from Legacy Viewers");
        }
        Some(LegacyImportReport {
            manual,
            changed: added.is_some(),
            added: added.unwrap_or(0),
            found: read.found,
            failed: read.failed,
        })
    }

    fn merge_legacy(&mut self, incoming: Vec<HistoryItem>) -> Option<usize> {
        let policy = self.import_policy.as_ref()?;
        if incoming.is_empty() || self.path.is_none() {
            return None;
        }
        // Merge into the live history so a visit recorded while the worker
        // was reading is included in the same queued snapshot.
        let mut seen = HashSet::with_capacity(self.items.len() + incoming.len());
        let duplicate_indices: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| (!seen.insert(item.key())).then_some(i))
            .collect();
        let additions: Vec<_> = incoming
            .iter()
            .filter(|&item| policy.permits(item) && seen.insert(item.key()))
            .cloned()
            .collect();
        let added = additions.len();
        if added == 0 && duplicate_indices.is_empty() {
            return None;
        }
        let mut duplicates = duplicate_indices.into_iter().peekable();
        let mut index = 0;
        self.items.retain(|_| {
            let keep = duplicates.next_if_eq(&index).is_none();
            index += 1;
            keep
        });
        self.items.extend(additions);
        self.items.sort_by(|a, b| a.date.total_cmp(&b.date));
        self.save();
        Some(added)
    }

    /// A history kept in memory only (the demo's).
    pub fn in_memory(items: Vec<HistoryItem>) -> HistoryStorage {
        HistoryStorage {
            items,
            ..Default::default()
        }
    }

    /// LLTeleportHistoryStorage::addItem: the same title within 5 m replaces
    /// the older entry and keeps its position (teleports through the history
    /// would otherwise creep up a meter each time).
    /// `slurl`: on an OpenSim grid the place's `hop://` SLURL, which names
    /// its grid (empty on Second Life, or until the grid is known).
    pub fn add(&mut self, title: &str, global: DVec3, date: f64, slurl: String) {
        let mut item = HistoryItem {
            title: title.to_owned(),
            global,
            date,
            slurl,
        };
        if let Some(i) = self
            .items
            .iter()
            .position(|x| x.title == item.title && (x.global - item.global).length() < MAX_GLOBAL_POS_OFFSET)
        {
            item.global = self.items[i].global;
            self.items.remove(i);
        }
        self.items.push(item);
        let n = self.items.len();
        if n > 1 && self.items[n - 2].date > self.items[n - 1].date {
            self.items.sort_by(|a, b| a.date.total_cmp(&b.date));
        }
        self.save();
    }

    /// The newest place at `global` gets the SLURL that names its grid,
    /// when that came after it was saved.
    pub fn set_slurl(&mut self, global: DVec3, slurl: String) {
        if let Some(item) = self
            .items
            .iter_mut()
            .rev()
            .find(|x| x.slurl.is_empty() && (x.global - global).length() < MAX_GLOBAL_POS_OFFSET)
        {
            item.slurl = slurl;
            self.save();
        }
    }

    /// "Remove from history" (removeItem + save).
    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            let visit = self.items.remove(index);
            if let Some(policy) = &mut self.import_policy {
                Arc::make_mut(policy).removed.insert(visit.fingerprint());
            }
            self.save();
        }
    }

    /// onClearTeleportHistoryDialog: purgeItems + save. The lines kept
    /// unread go too: the user asked for an empty history.
    pub fn clear(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64());
        self.clear_at(now);
    }

    fn clear_at(&mut self, now: f64) {
        self.legacy_history = None;
        if let Some(policy) = &mut self.import_policy {
            let policy = Arc::make_mut(policy);
            let cutoff = policy.cleared_through.map_or(now, |earlier| earlier.max(now));
            policy.cleared_through = Some(cutoff);
            policy
                .removed
                .extend(self.items.iter().filter(|visit| visit.date > cutoff).map(HistoryItem::fingerprint));
        }
        self.items.clear();
        self.kept.clear();
        self.save();
    }

    /// The file's bytes: the lines kept from the file, then the places.
    fn file(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for line in &self.kept {
            out.extend_from_slice(line);
            out.push(b'\n');
        }
        for it in &self.items {
            out.extend_from_slice(aurora_llsd::to_notation(&it.to_llsd()).as_bytes());
            out.push(b'\n');
        }
        out
    }

    /// Hand the latest complete file to this account's writer.
    fn save(&self) {
        if let Some(path) = &self.path {
            writers().queue(
                path.clone(),
                SavedHistory {
                    bytes: self.file(),
                    policy: self.import_policy.clone(),
                },
                write_file,
            );
        }
    }

    #[cfg(test)]
    fn flush(&self) {
        if let Some(path) = &self.path {
            writers().wait(Some(path));
        }
    }
}

fn read_legacy_history(account: &crate::legacy_profile::Account, apps: &[PathBuf], chosen: Option<PathBuf>) -> LegacyRead {
    let folders = match chosen {
        Some(dir)
            if dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| account.owns(name)) =>
        {
            vec![dir]
        }
        Some(dir) => crate::legacy_profile::account_folders(&[dir], account),
        None => crate::legacy_profile::account_folders(apps, account),
    };
    let mut read = LegacyRead::default();
    for folder in folders {
        let path = folder.join(TELEPORT_HISTORY_FILE);
        let mut bytes = Vec::new();
        match std::fs::File::open(path).and_then(|file| file.take(MAX_LEGACY_HISTORY_BYTES + 1).read_to_end(&mut bytes)) {
            Ok(_) if bytes.len() as u64 <= MAX_LEGACY_HISTORY_BYTES => {
                read.found = true;
                read.items.extend(parse_legacy(&bytes));
            }
            Ok(_) => {
                read.failed = true;
                log::warn!("teleport history: a Legacy Viewer file exceeded the import limit");
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                read.failed = true;
                log::warn!("teleport history: can't read a Legacy Viewer file ({e})");
            }
        }
    }
    read
}

fn parse_legacy(bytes: &[u8]) -> Vec<HistoryItem> {
    notation_rows(bytes)
        .filter_map(|line| {
            let data = aurora_llsd::from_notation(line).ok()?;
            let position = data["global_pos"].as_array();
            if !matches!(position, [x, y, z] if [x, y, z].into_iter().all(|n| matches!(n, Llsd::Real(_) | Llsd::Integer(_))))
                || !matches!(data["date"], Llsd::Date(_))
                || !matches!(data["slurl"], Llsd::String(_) | Llsd::Uri(_) | Llsd::Undef)
            {
                return None;
            }
            let item = HistoryItem::from_llsd(&data);
            (!item.title.trim().is_empty() && item.global.is_finite() && item.date.is_finite()).then_some(item)
        })
        .collect()
}

fn notation_rows(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
}

/// The places of a file, and its lines that are not one (kept as they are).
fn parse(bytes: &[u8]) -> (Vec<HistoryItem>, Vec<Vec<u8>>) {
    let mut items = Vec::new();
    let mut kept = Vec::new();
    for line in notation_rows(bytes) {
        match aurora_llsd::from_notation(line) {
            Ok(v) if v.is_map() => items.push(HistoryItem::from_llsd(&v)),
            _ => kept.push(line.to_vec()),
        }
    }
    (items, kept)
}

fn write_file(path: &Path, saved: &SavedHistory) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Publish suppressions before deleting the corresponding history rows.
    // If the policy cannot be saved, retry both with the latest snapshot.
    if let Some(policy) = &saved.policy {
        let policy_path = path.with_file_name(LEGACY_IMPORT_FILE);
        let encoded = serde_json::to_vec(policy.as_ref())?;
        let changed = match std::fs::read(&policy_path) {
            Ok(on_disk) => on_disk != encoded,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => **policy != ImportPolicy::default(),
            Err(e) => return Err(e),
        };
        if changed {
            crate::atomic_file::write(&policy_path, &encoded)?;
        }
    }
    crate::atomic_file::write(path, &saved.bytes)
}

/// One publisher per account file, shared across session and World resets.
/// A refused write keeps its snapshot pending (a new session of the account
/// reads it from here) and is tried again, waiting longer each time
/// (`save_retry`), or at once with the newer snapshot when the wait ends;
/// the exit's [`finish_writes`] makes the last try. The Legacy Viewer's
/// LLTeleportHistoryStorage::save writes once and warns.
#[derive(Default)]
struct Writers {
    pending: Mutex<HashMap<PathBuf, Arc<SavedHistory>>>,
    finished: Condvar,
    /// Wakes a publisher waiting to try a refused write again.
    wake: Condvar,
    /// The exit waits for the writes: a refused one is tried once more,
    /// then given up.
    closing: std::sync::atomic::AtomicBool,
}

impl Writers {
    fn snapshot(&self, path: &Path) -> Option<Arc<SavedHistory>> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).get(path).cloned()
    }

    fn queue(
        self: &Arc<Self>,
        path: PathBuf,
        bytes: SavedHistory,
        publish: impl FnMut(&Path, &SavedHistory) -> std::io::Result<()> + Send + 'static,
    ) {
        let bytes = Arc::new(bytes);
        // Inserting into a vacant slot reserves the only worker for this
        // path. Later requests replace its snapshot while the write runs.
        if self
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(path.clone(), bytes.clone())
            .is_some()
        {
            return;
        }
        let writers = self.clone();
        let target = path.clone();
        let first = bytes.clone();
        if let Err(e) = std::thread::Builder::new().name("aurora-tp-history".into()).spawn(move || {
            writers.publish(&target, first, publish);
        }) {
            log::warn!("teleport history: no writer thread ({e}); saved on the caller's thread");
            self.publish(&path, bytes, write_file);
        }
    }

    fn publish(&self, path: &Path, mut bytes: Arc<SavedHistory>, mut write: impl FnMut(&Path, &SavedHistory) -> std::io::Result<()>) {
        use std::sync::atomic::Ordering;
        let mut backoff = crate::save_retry::Backoff::default();
        loop {
            let written = write(path, &bytes);
            let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            let newer =
                |pending: &HashMap<PathBuf, Arc<SavedHistory>>| pending.get(path).is_some_and(|latest| !Arc::ptr_eq(latest, &bytes));
            if let Err(e) = written {
                if self.closing.load(Ordering::Acquire) && !newer(&pending) {
                    log::warn!("can't save the teleport history at exit: {e}");
                    pending.remove(path);
                    self.finished.notify_all();
                    return;
                }
                let wait = backoff.failed(std::time::Instant::now());
                log::warn!("can't save the teleport history ({e}); tried again in {} s", wait.as_secs());
                // the exit ends the wait early; a newer snapshot waits for it
                pending = self
                    .wake
                    .wait_timeout_while(pending, wait, |_| !self.closing.load(Ordering::Acquire))
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            } else {
                backoff.succeeded();
                if !newer(&pending) {
                    pending.remove(path);
                    self.finished.notify_all();
                    return;
                }
            }
            bytes = pending
                .get(path)
                .expect("the publishing thread owns this path until its last write")
                .clone();
        }
    }

    /// The exit: every write still pending ends, a refused one after one
    /// more try.
    fn close(&self) {
        use std::sync::atomic::Ordering;
        self.closing.store(true, Ordering::Release);
        {
            // under the lock, so that a publisher about to wait sees it
            let _pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            self.wake.notify_all();
        }
        self.wait(None);
        self.closing.store(false, Ordering::Release);
    }

    fn wait(&self, path: Option<&Path>) {
        let pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        drop(
            self.finished
                .wait_while(pending, |files| match path {
                    Some(path) => files.contains_key(path),
                    None => !files.is_empty(),
                })
                .unwrap_or_else(|p| p.into_inner()),
        );
    }
}

fn writers() -> &'static Arc<Writers> {
    static WRITERS: OnceLock<Arc<Writers>> = OnceLock::new();
    WRITERS.get_or_init(Default::default)
}

/// Called after the viewer window has closed. An orderly exit, including a
/// restart into VR, waits for the final atomic publication for every account.
pub fn finish_writes() {
    writers().close();
}

/// Accordion sections of the history (panel_teleport_history.xml).
pub fn history_sections() -> [&'static str; 9] {
    [
        aurora_i18n::tr!("world-tphistory-today"),
        aurora_i18n::tr!("world-tphistory-yesterday"),
        aurora_i18n::tr!("extra-2-days-ago"),
        aurora_i18n::tr!("extra-3-days-ago"),
        aurora_i18n::tr!("extra-4-days-ago"),
        aurora_i18n::tr!("extra-5-days-ago"),
        aurora_i18n::tr!("world-tphistory-6-days-ago-or-earlier"),
        aurora_i18n::tr!("world-tphistory-1-month-ago-or-earlier"),
        aurora_i18n::tr!("world-tphistory-6-months-ago-or-earlier"),
    ]
}

/// LLTeleportHistoryPanel::getNextTab with FSTPHistoryTZ: the first six
/// sections are days starting at midnight in the history's zone (`offset`:
/// seconds to add to UTC there, 0 for UTC), "6 days ago or earlier" goes
/// back to the same day of the previous month and "1 month ago or earlier"
/// to six months ago, both from midnight UTC as the source counts them. A
/// date in the future counts as today. The returned lookup gives the index
/// in the translated section titles for each date in the same frame.
pub fn history_section_lookup(now: f64, offset: f64) -> impl Fn(f64) -> usize {
    use aurora_net::profile::{civil_from_days, days_from_civil};
    const DAY: f64 = 86_400.0;
    let (y, m, d) = civil_from_days((now / DAY).floor() as i64);
    // the zone's day, back in UTC seconds below
    let today = ((now + offset) / DAY).floor();
    // LLDate::fromYMDHMS normalizes a day past the month's end, as does
    // days_from_civil
    let months_back = |n: i64| {
        let total = y * 12 + (m as i64 - 1) - n;
        days_from_civil(total.div_euclid(12), (total.rem_euclid(12) + 1) as u32, d) as f64 * DAY
    };
    // All visits in a frame share their day and month boundaries. Reading
    // the translated section titles for every date is especially costly.
    let lower: [f64; 8] = std::array::from_fn(|i| match i {
        0..=5 => (today - i as f64) * DAY - offset,
        6 => months_back(1),
        _ => months_back(6),
    });
    move |date| lower.iter().position(|&start| date >= start).unwrap_or(lower.len())
}

/// LLAgentUI::buildLocationString(LOCATION_FORMAT_NORMAL): "Parcel, Region",
/// the region alone when the parcel has no name.
pub fn location_title(parcel: &str, region: &str) -> String {
    match (parcel.trim(), region.trim()) {
        ("", r) => r.to_owned(),
        (p, "") => p.to_owned(),
        (p, r) => format!("{p}, {r}"),
    }
}

/// LLTeleportHistoryPanel::refresh filter: the title contains the text,
/// ignoring case.
pub fn history_matches(title: &str, filter: &str) -> bool {
    filter.is_empty() || title.to_uppercase().contains(&filter.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_section(date: f64, now: f64, offset: f64) -> usize {
        history_section_lookup(now, offset)(date)
    }

    fn e(n: &str) -> TpEntry {
        TpEntry {
            region: n.into(),
            handle: aurora_net::origin_to_handle(256_000, 256_000),
            position: Vec3::new(128.0, 128.0, 25.0),
            parcel: String::new(),
            at: 0,
            grid: None,
        }
    }

    #[test]
    fn a_back_that_never_left_keeps_the_history_as_it_was() {
        let mut h = TeleportHistory::default();
        for place in ["Home", "Plaza", "Store"] {
            h.arrived(e(place));
        }
        let mut away = e("Wright Plaza");
        away.grid = Some("hg.osgrid.org:80".into());
        h.arrived(away);
        assert_eq!(h.go_back().map(|t| t.region), Some("Store".into()));
        // refused (RLVa, a region not found): the app says so
        h.failed();
        // the next arrival, a lure for instance, comes after where we are:
        // Store is not overwritten
        h.arrived(e("Beach"));
        let regions: Vec<_> = h.items.iter().map(|t| t.region.as_str()).collect();
        assert_eq!(regions, ["Home", "Plaza", "Store", "Wright Plaza", "Beach"]);
    }

    #[test]
    fn a_place_learns_its_grid_once_and_for_its_own_region() {
        let mut h = TeleportHistory::default();
        h.arrived(e("Wright Plaza"));
        // another region's features: not ours
        h.set_grid(aurora_net::origin_to_handle(512, 512), Some("other.example".into()));
        assert_eq!(h.session().items[0].grid, None);
        h.set_grid(e("x").handle, Some("hg.osgrid.org".into()));
        assert_eq!(h.session().items[0].grid.as_deref(), Some("hg.osgrid.org"));
        // known once: a later answer does not move it
        h.set_grid(e("x").handle, Some("elsewhere.example".into()));
        assert_eq!(h.session().items[0].grid.as_deref(), Some("hg.osgrid.org"));
    }

    #[test]
    fn a_saved_place_gets_its_slurl_when_the_grid_comes_later() {
        let mut s = HistoryStorage::in_memory(Vec::new());
        let here = DVec3::new(256_140.0, 256_120.0, 25.0);
        s.add(
            "Plaza, Wright Plaza",
            DVec3::new(1.0, 1.0, 1.0),
            1.0,
            "hop://a.example/A/1/1/1".into(),
        );
        s.add("Plaza, Wright Plaza", here, 2.0, String::new());
        s.set_slurl(
            here + DVec3::new(0.0, 0.0, 1.0),
            "hop://hg.osgrid.org/Wright%20Plaza/140/120/25".into(),
        );
        assert_eq!(s.items[1].slurl, "hop://hg.osgrid.org/Wright%20Plaza/140/120/25");
        // a place that already names its grid keeps it
        assert_eq!(s.items[0].slurl, "hop://a.example/A/1/1/1");
    }

    #[test]
    fn back_forward_and_new_branch() {
        let mut h = TeleportHistory::default();
        h.arrived(e("A"));
        h.arrived(e("B"));
        h.arrived(e("C"));
        assert_eq!(h.go_back().map(|x| x.region), Some("B".into()));
        h.arrived(e("B"));
        assert_eq!(h.previous().map(|x| x.region.clone()), Some("A".into()));
        assert_eq!(h.next().map(|x| x.region.clone()), Some("C".into()));
        // a normal teleport from B drops the forward part
        h.arrived(e("D"));
        assert!(h.next().is_none());
        assert_eq!(h.previous().map(|x| x.region.clone()), Some("B".into()));
        // a failed trip keeps the position
        h.go_back();
        h.failed();
        h.arrived(e("E"));
        assert_eq!(h.previous().map(|x| x.region.clone()), Some("D".into()));
        h.purge();
        assert!(h.previous().is_none() && h.next().is_none());
    }

    /// The arrows' list: a jump to any place keeps the list whole, the
    /// place we are in is not a destination, and the parcel names the
    /// current place once it is known.
    #[test]
    fn jump_to_any_place_of_the_session() {
        let mut h = TeleportHistory::default();
        for n in ["A", "B", "C", "D"] {
            h.arrived(e(n));
        }
        assert!(h.go_to(3).is_none(), "the place we are in");
        assert!(h.go_to(9).is_none());
        assert_eq!(h.go_to(0).map(|x| x.region), Some("A".into()));
        h.arrived(e("A"));
        let s = h.session();
        assert_eq!((s.items.len(), s.current), (4, 0));
        assert_eq!(h.next().map(|x| x.region.clone()), Some("B".into()));
        h.name_parcel(" Plaza ");
        assert_eq!(h.session().items[0].title(), "Plaza, A");
        // RLVa hides the place: the parcel goes with the region's name
        h.replace_current_region("(Hidden)");
        assert_eq!(h.session().items[0].title(), "(Hidden)");
        // the demo's earlier places: the next arrival comes after them
        let mut d = TeleportHistory::seeded(vec![e("X"), e("Y")]);
        d.arrived(e("Z"));
        assert_eq!(d.session().current, 2);
        assert_eq!(d.previous().map(|x| x.region.clone()), Some("Y".into()));
    }

    /// LLTeleportHistory::updateCurrentLocation under @showloc: however many
    /// teleports, the session holds one hidden place; its arrow asks for the
    /// zeroed place it keeps (LLTeleportHistory::goToItem).
    #[test]
    fn rlva_hidden_arrivals_share_one_place_that_leads_nowhere() {
        let mut h = TeleportHistory::default();
        h.arrived(e("A"));
        h.arrived(e("B"));
        h.arrived(e("C"));
        assert!(h.go_back().is_some());
        h.arrived(e("B"));
        // the restriction comes at B: B loses its name and its destination,
        // and the places ahead go
        let was = h.replace_current_region("(Hidden)").expect("a current place");
        assert_eq!((was.region.as_str(), was.is_hidden()), ("B", false));
        let s = h.session();
        assert_eq!((s.items.len(), s.current), (2, 1));
        assert!(s.items[1].is_hidden());
        // three teleports under it: still one hidden place, the last one's
        for at in 1..=3 {
            h.arrived(TpEntry::hidden("(Hidden)", at));
        }
        let s = h.session();
        assert_eq!((s.items.len(), s.current), (2, 1));
        assert_eq!((s.items[1].title().as_str(), s.items[1].at), ("(Hidden)", 3));
        assert_eq!(s.items[1].handle, 0);
        assert_eq!(s.items[1].position, Vec3::ZERO);
        // Back to A is a teleport like another; A is left as it was
        assert_eq!(h.back_name().as_deref(), Some("A"));
        assert_eq!(h.go_back().map(|x| x.region), Some("A".into()));
        h.arrived(TpEntry::hidden("(Hidden)", 4));
        let s = h.session();
        assert_eq!((s.items.len(), s.current, s.items[0].region.as_str()), (2, 0, "A"));
        assert!(!s.items[0].is_hidden());
        // the hidden place ahead keeps its arrow (LLNavigationBar::
        // onTeleportHistoryChanged looks at the index alone), and the click
        // asks for what it holds: the region handle 0 at the position zero
        assert!(h.next().is_some_and(TpEntry::is_hidden));
        assert_eq!(h.forward_name().as_deref(), Some("(Hidden)"));
        for asked in [h.go_forward(), h.go_to(1)] {
            let asked = asked.expect("a hidden place is asked for like another");
            assert_eq!((asked.handle, asked.position, asked.grid), (0, Vec3::ZERO, None));
            // the grid refuses it: we are still at A, the list as it was
            h.failed();
            let s = h.session();
            assert_eq!((s.items.len(), s.current, s.items[0].region.as_str()), (2, 0, "A"));
        }
        h.arrived(TpEntry::hidden("(Hidden)", 5));
        // that arrival was no trip in the list: it takes the place of A
        let s = h.session();
        assert_eq!((s.items.len(), s.current), (1, 0));
        assert!(s.items[0].is_hidden());
        // released: the place is the one we stand in, and the list grows again
        h.replace_current_region("D");
        assert!(h.session().items[0].is_hidden(), "no destination until the agent is located");
        h.locate_current(e("D"));
        h.locate_current(e("not hidden any more"));
        assert_eq!(h.session().items[0].region, "D");
        h.arrived(e("E"));
        assert_eq!(h.back_name().as_deref(), Some("D"));
        // a hidden place never located is replaced by the next arrival
        h.replace_current_region("(Hidden)");
        h.arrived(e("F"));
        let regions: Vec<_> = h.session().items.iter().map(|t| t.region.as_str()).collect();
        assert_eq!(regions, ["D", "F"]);
    }

    const DAY: f64 = 86_400.0;

    #[test]
    fn sections_by_day_then_month() {
        use aurora_net::profile::days_from_civil;
        // 2026-10-10 15:00 UTC
        let now = days_from_civil(2026, 10, 10) as f64 * DAY + 15.0 * 3600.0;
        let midnight = days_from_civil(2026, 10, 10) as f64 * DAY;
        assert_eq!(history_section(now, now, 0.0), 0);
        assert_eq!(history_section(midnight, now, 0.0), 0);
        assert_eq!(history_section(midnight - 1.0, now, 0.0), 1);
        assert_eq!(history_section(now + DAY, now, 0.0), 0, "future dates count as today");
        for days in 2..=5 {
            assert_eq!(history_section(midnight - (days as f64 - 0.5) * DAY, now, 0.0), days);
        }
        assert_eq!(history_section(midnight - 5.5 * DAY, now, 0.0), 6);
        // one month back: 2026-09-10 00:00
        let month = days_from_civil(2026, 9, 10) as f64 * DAY;
        assert_eq!(history_section(month, now, 0.0), 6);
        assert_eq!(history_section(month - 1.0, now, 0.0), 7);
        let six = days_from_civil(2026, 4, 10) as f64 * DAY;
        assert_eq!(history_section(six, now, 0.0), 7);
        assert_eq!(history_section(six - 1.0, now, 0.0), 8);
        // January: the months before go back into the previous year
        let jan = days_from_civil(2027, 1, 31) as f64 * DAY;
        assert_eq!(history_section(days_from_civil(2026, 12, 31) as f64 * DAY, jan, 0.0), 6);
        assert_eq!(history_section(days_from_civil(2026, 7, 31) as f64 * DAY, jan, 0.0), 7);
        assert_eq!(history_section(days_from_civil(2026, 7, 30) as f64 * DAY, jan, 0.0), 8);
    }

    /// SLT and local time: the days start at the zone's midnight, the
    /// months stay counted from midnight UTC.
    #[test]
    fn the_days_start_at_the_zones_midnight() {
        use aurora_net::profile::days_from_civil;
        const HOUR: f64 = 3600.0;
        // 2026-11-14 03:00 UTC, 2026-11-13 19:00 PST
        let utc_midnight = days_from_civil(2026, 11, 14) as f64 * DAY;
        let now = utc_midnight + 3.0 * HOUR;
        let pst = -8.0 * HOUR;
        // 2026-11-13 09:00 UTC: today in SLT, yesterday in UTC
        let morning = utc_midnight - 15.0 * HOUR;
        assert_eq!(history_section(morning, now, pst), 0);
        assert_eq!(history_section(morning, now, 0.0), 1);
        // SLT's midnight is 08:00 UTC
        let slt_midnight = utc_midnight - 16.0 * HOUR;
        assert_eq!(history_section(slt_midnight, now, pst), 0);
        assert_eq!(history_section(slt_midnight - 1.0, now, pst), 1);
        // east of UTC (UTC+9): 2026-11-14 12:00 there, its day from 15:00 UTC the day before
        let tokyo = 9.0 * HOUR;
        assert_eq!(history_section(utc_midnight - 9.0 * HOUR, now, tokyo), 0);
        assert_eq!(history_section(utc_midnight - 9.0 * HOUR - 1.0, now, tokyo), 1);
        // a month back stays 2026-10-14 00:00 UTC in every zone
        let month = days_from_civil(2026, 10, 14) as f64 * DAY;
        for offset in [0.0, pst, tokyo] {
            assert_eq!(history_section(month, now, offset), 6);
            assert_eq!(history_section(month - 1.0, now, offset), 7);
        }
    }

    #[test]
    fn storage_merges_close_places_and_saves() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-{}", std::process::id()));
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let _ = std::fs::remove_file(&path);
        let mut s = HistoryStorage::load(path.clone());
        assert!(s.items.is_empty());
        s.add(
            "Place d'Aurora, Aurora Démo",
            DVec3::new(256_140.0, 256_120.0, 25.0),
            1000.0,
            String::new(),
        );
        s.add(
            "Lagune, Lagune Boréale",
            DVec3::new(256_316.0, 256_200.0, 22.0),
            2000.0,
            String::new(),
        );
        // back to the first place, a meter higher: same entry, original position
        s.add(
            "Place d'Aurora, Aurora Démo",
            DVec3::new(256_140.0, 256_120.0, 26.0),
            3000.0,
            String::new(),
        );
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items[1].title, "Place d'Aurora, Aurora Démo");
        assert_eq!(s.items[1].global.z, 25.0);
        assert_eq!(s.items[1].date, 3000.0);
        // an older date is sorted in
        s.add("Nordheim", DVec3::new(256_000.0, 256_300.0, 40.0), 1500.0, String::new());
        assert_eq!(s.items[0].title, "Nordheim");
        s.flush();
        let again = HistoryStorage::load(path.clone());
        assert_eq!(again.items, s.items);
        s.remove(0);
        s.clear();
        s.flush();
        assert!(HistoryStorage::load(path.clone()).items.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_firestorm_lines() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-fs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let text = "{'date':d\"2024-04-02T14:42:28Z\",'global_pos':[r256148,r256195,r24],'slurl':'','title':'Tordangle, Tordangle'}\n\
                    not llsd\n\
                    {'date':d\"2024-03-01T10:00:00.00Z\",'global_pos':[r1,r2,r3],'title':'Ahern'}\n";
        std::fs::write(&path, text).unwrap();
        let s = HistoryStorage::load(path);
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items[0].title, "Ahern");
        assert_eq!(s.items[1].global, DVec3::new(256_148.0, 256_195.0, 24.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_import_keeps_every_distinct_visit_across_restarts() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-import-{}", uuid::Uuid::new_v4()));
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let source = b"{'title':'Plaza','global_pos':[r1,r2,r3],'date':d\"2024-03-01T10:00:00.123456Z\"}\n";
        let base = parse_legacy(source).pop().unwrap();
        let mut at_another_time = base.clone();
        at_another_time.date += 0.000_002;
        let mut at_another_position = base.clone();
        at_another_position.global.z += 1.0;
        let mut with_another_slurl = base.clone();
        with_another_slurl.slurl = "hop://example.invalid/Plaza/1/2/3".into();
        let mut with_another_title = base.clone();
        with_another_title.title = "Plaza annex".into();

        let mut history = HistoryStorage::load(path.clone());
        history.items = vec![at_another_time.clone(), base.clone(), base.clone()];
        history.kept = vec![b"an unreadable Aurora row".to_vec()];
        let imported = vec![base.clone(), at_another_position, with_another_slurl, with_another_title, base];
        assert_eq!(history.merge_legacy(imported.clone()), Some(3));
        assert_eq!(history.items.len(), 5);
        assert_eq!(history.items.last(), Some(&at_another_time));
        history.flush();

        let mut reloaded = HistoryStorage::load(path);
        assert_eq!(reloaded.items, history.items);
        assert_eq!(reloaded.kept, history.kept);
        assert_eq!(reloaded.merge_legacy(imported), None);
        assert_eq!(reloaded.items, history.items);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_import_reads_matching_profiles_only_and_seeds_an_empty_history() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-import-folders-{}", uuid::Uuid::new_v4()));
        let apps = [dir.join("Firestorm_x64"), dir.join("SecondLife")];
        let a = "{'title':'Plaza','global_pos':[r1,r2,r3],'date':d\"2024-03-01T10:00:00Z\"}";
        let b = "{'title':'Sandbox','global_pos':[r1,r2,r3],'date':d\"2024-04-01T10:00:00Z\"}";
        for (app, folder, rows) in [
            (&apps[0], "jane_doe", format!("{a}\r\ninvalid row\r\n{a}\r\n")),
            (&apps[1], "jane_doe", format!("{{}}\n{a}\n{b}\n")),
            (&apps[0], "jane_doe.second_life_beta", b.replace("Sandbox", "Beta")),
            (&apps[1], "jane_other", b.replace("Sandbox", "Other account")),
        ] {
            let profile = app.join(folder);
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(profile.join(TELEPORT_HISTORY_FILE), rows).unwrap();
        }
        let source_path = apps[0].join("jane_doe").join(TELEPORT_HISTORY_FILE);
        let source_bytes = std::fs::read(&source_path).unwrap();
        let path = dir.join("aurora").join(TELEPORT_HISTORY_FILE);
        let mut history = HistoryStorage::load(path.clone());
        let account = crate::legacy_profile::Account {
            first: "Jane".into(),
            last: "Doe".into(),
            grid: "agni".into(),
        };
        for grid in ["aditi", "custom_example", "demo"] {
            let mut other = account.clone();
            other.grid = grid.into();
            history.start_legacy_import(other, apps.to_vec(), None, true);
            assert!(history.legacy_history.is_none());
        }

        history.start_legacy_import(account.clone(), apps.to_vec(), None, false);
        // Wait for the worker while retaining its result for the normal poll.
        let visits = history
            .legacy_history
            .take()
            .unwrap()
            .0
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        sender.send(visits).unwrap();
        history.legacy_history = Some((receiver, false));
        let report = history.poll_legacy_import().unwrap();
        assert!(report.changed && !report.manual);
        history.flush();
        let mut reloaded = HistoryStorage::load(path);
        let titles: Vec<_> = reloaded.items.iter().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, ["Plaza", "Sandbox"]);
        reloaded.start_legacy_import(account.clone(), apps.to_vec(), None, false);
        assert!(!reloaded.import_busy(), "later logins do not scan");
        assert_eq!(std::fs::read(source_path).unwrap(), source_bytes);
        let chosen = read_legacy_history(&account, &apps, Some(apps[1].clone()));
        assert_eq!(chosen.items.len(), 2);
        let chosen = read_legacy_history(&account, &apps, Some(apps[0].join("jane_doe")));
        assert_eq!(chosen.items.len(), 2);
        assert!(chosen.items.iter().all(|visit| visit.title == "Plaza"));
        let wrong = read_legacy_history(&account, &apps, Some(apps[1].join("jane_other")));
        assert!(!wrong.found && wrong.items.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_initial_import_check_is_remembered_without_an_import() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-first-login-{}", uuid::Uuid::new_v4()));
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let mut history = HistoryStorage::load(path.clone());
        assert!(history.check_initial_import(), "no history on first login");
        history.flush();
        assert!(
            !HistoryStorage::load(path).check_initial_import(),
            "a later login requires manual import"
        );

        let path = dir.join("existing").join(TELEPORT_HISTORY_FILE);
        let mut history = HistoryStorage::load(path.clone());
        history.add("Plaza", DVec3::ONE, 1.0, String::new());
        assert!(!history.check_initial_import(), "an existing history is skipped");
        history.clear_at(2.0);
        history.flush();
        assert!(
            !HistoryStorage::load(path).check_initial_import(),
            "clearing never resets eligibility"
        );

        let path = dir.join("cleared").join(TELEPORT_HISTORY_FILE);
        let mut history = HistoryStorage::load(path.clone());
        history.clear_at(2.0);
        history.flush();
        let mut reloaded = HistoryStorage::load(path);
        assert!(!reloaded.check_initial_import(), "a clear before upgrading prevents auto-import");
        reloaded.flush();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn removing_and_clearing_history_persist_across_logins_and_imports() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-suppress-{}", uuid::Uuid::new_v4()));
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let visit = |date| HistoryItem {
            title: "Plaza".into(),
            global: DVec3::ONE,
            date,
            slurl: String::new(),
        };
        let mut history = HistoryStorage::load(path.clone());
        assert_eq!(history.merge_legacy(vec![visit(1.0), visit(2.0)]), Some(2));
        history.remove(0);
        history.flush();
        let mut history = HistoryStorage::load(path.clone());
        assert_eq!(history.items, vec![visit(2.0)]);
        assert_eq!(history.merge_legacy(vec![visit(1.0), visit(3.0), visit(20.0)]), Some(2));
        history.clear_at(5.0);
        history.flush();
        let mut history = HistoryStorage::load(path);
        assert!(history.items.is_empty());
        assert_eq!(
            history.merge_legacy(vec![visit(1.0), visit(2.0), visit(4.0), visit(5.0), visit(6.0), visit(20.0)]),
            Some(1)
        );
        assert_eq!(history.items, vec![visit(6.0)]);
        history.flush();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_policy_save_leaves_the_history_file_in_place() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-policy-fail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let policy_path = dir.join(LEGACY_IMPORT_FILE);
        std::fs::write(&path, b"old history").unwrap();
        std::fs::create_dir(&policy_path).unwrap();
        let saved = SavedHistory {
            bytes: Vec::new(),
            policy: Some(Arc::new(ImportPolicy {
                cleared_through: Some(1.0),
                ..Default::default()
            })),
        };
        assert!(write_file(&path, &saved).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"old history");
        std::fs::remove_dir(policy_path).unwrap();
        write_file(&path, &saved).unwrap();
        assert_eq!(ImportPolicy::load(&path).unwrap().cleared_through, Some(1.0));
        assert!(std::fs::read(&path).unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// #2626: the lines this viewer can't read (not LLSD, not UTF-8) are
    /// written back at the next save instead of being lost.
    #[test]
    fn unreadable_lines_survive_a_save() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-kept-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let mut text = b"{'date':d\"2024-03-01T10:00:00.00Z\",'global_pos':[r1,r2,r3],'title':'Ahern'}\r\n".to_vec();
        text.extend_from_slice(b"a line from a newer viewer\r\n");
        // not UTF-8: a Latin-1 e acute
        text.extend_from_slice(b"{'title':'caf\xe9'\n");
        std::fs::write(&path, &text).unwrap();
        let mut s = HistoryStorage::load(path.clone());
        assert_eq!(s.items.len(), 1);
        s.add("Nordheim", DVec3::new(256_000.0, 256_300.0, 40.0), 2000.0, String::new());
        s.flush();
        let saved = std::fs::read(&path).unwrap();
        let lines: Vec<&[u8]> = saved.split(|&b| b == b'\n').collect();
        assert!(lines.contains(&&b"a line from a newer viewer"[..]));
        assert!(lines.contains(&&b"{'title':'caf\xe9'"[..]));
        let again = HistoryStorage::load(path.clone());
        assert_eq!(again.items, s.items);
        assert_eq!(again.kept.len(), 2);
        // saved again: kept once, not twice
        let mut again = again;
        again.remove(0);
        again.flush();
        assert_eq!(HistoryStorage::load(path.clone()).kept.len(), 2);
        // Clear history empties the file, kept lines included
        again.clear();
        again.flush();
        let after_clear = HistoryStorage::load(path.clone());
        assert!(after_clear.items.is_empty() && after_clear.kept.is_empty());
        assert!(std::fs::read(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #2626: a history that exists but can't be read is never saved over:
    /// the session's places stay in memory only.
    #[test]
    fn unreadable_file_is_never_saved_over() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-locked-{}", std::process::id()));
        // a folder where the file should be: reading it fails, as a locked
        // or damaged file does
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let _ = std::fs::create_dir_all(path.join("inside"));
        let mut s = HistoryStorage::load(path.clone());
        assert!(s.path.is_none());
        s.add("Nordheim", DVec3::new(256_000.0, 256_300.0, 40.0), 2000.0, String::new());
        assert_eq!(s.items.len(), 1);
        assert!(writers().snapshot(&path).is_none());
        drop(s);
        assert!(path.join("inside").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing file is an empty history that is saved at the first place.
    #[test]
    fn missing_file_starts_empty_and_saves() {
        let dir = std::env::temp_dir().join(format!("aurora-tphist-new-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let mut s = HistoryStorage::load(path.clone());
        assert!(s.items.is_empty() && s.path.is_some());
        s.add("Nordheim", DVec3::new(256_000.0, 256_300.0, 40.0), 2000.0, String::new());
        drop(s);
        assert_eq!(HistoryStorage::load(path.clone()).items.len(), 1);
        // A normal shutdown drains the writer before another process opens the file.
        writers().wait(Some(&path));
        assert_eq!(parse(&std::fs::read(&path).unwrap()).0.len(), 1);
        // the atomic write leaves no temporary file behind
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(TELEPORT_HISTORY_FILE)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Keep the first publication pending past the former 500 ms logout wait.
    /// A new session sees that visit; its own write must publish last.
    #[test]
    fn a_slow_save_survives_logout_relogin_and_orderly_shutdown() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("aurora-tphist-relogin-{}", uuid::Uuid::new_v4()));
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let pos = DVec3::new(10.0, 20.0, 30.0);
        let (started, writing) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut first_write = true;
        let empty = SavedHistory {
            bytes: Vec::new(),
            policy: Some(Arc::default()),
        };
        writers().queue(path.clone(), empty, move |path, bytes| {
            if std::mem::take(&mut first_write) {
                started.send(()).unwrap();
                gate.recv().unwrap();
            }
            write_file(path, bytes)
        });
        writing.recv().unwrap();
        let mut first = HistoryStorage::load(path.clone());
        first.add("First", pos, 1.0, String::new());

        let mut next = HistoryStorage::load(path.clone());
        assert_eq!(next.items[0].title, "First");
        next.add("Second", pos, 2.0, String::new());
        drop(next);
        drop(first); // an older session going away cannot replay its first visit

        let other_path = dir.join("other").join(TELEPORT_HISTORY_FILE);
        let mut other = HistoryStorage::load(other_path.clone());
        assert!(other.items.is_empty());
        other.add("Other account", pos, 3.0, String::new());
        other.flush();
        assert_eq!(parse(&std::fs::read(other_path).unwrap()).0[0].title, "Other account");

        let (done, drained) = mpsc::channel();
        let close = std::thread::spawn(move || {
            finish_writes();
            done.send(()).unwrap();
        });
        assert_eq!(
            drained.recv_timeout(Duration::from_millis(600)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        assert!(!path.exists());
        release.send(()).unwrap();
        drained.recv_timeout(Duration::from_secs(10)).unwrap();
        close.join().unwrap();
        assert!(writers().snapshot(&path).is_none());
        let items = HistoryStorage::load(path.clone()).items;
        assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A refused write keeps its snapshot (a new session reads it) and the
    /// exit's wait tries it once more, which goes through.
    #[test]
    fn a_refused_save_stays_pending_and_is_tried_again_at_exit() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = std::env::temp_dir().join(format!("aurora-tphist-refused-{}", uuid::Uuid::new_v4()));
        let path = dir.join(TELEPORT_HISTORY_FILE);
        let writers = Arc::new(Writers::default());
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = tries.clone();
        let saved = SavedHistory {
            bytes: b"visits".to_vec(),
            ..Default::default()
        };
        writers.queue(path.clone(), saved, move |path, bytes| {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
            write_file(path, bytes)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while tries.load(Ordering::SeqCst) < 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(
            writers.snapshot(&path).as_deref().map(|s| s.bytes.as_slice()),
            Some(&b"visits"[..]),
            "kept"
        );
        writers.close();
        assert_eq!(tries.load(Ordering::SeqCst), 2);
        assert!(writers.snapshot(&path).is_none());
        assert_eq!(std::fs::read(&path).unwrap(), b"visits");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn filter_ignores_case() {
        assert!(history_matches("Place d'Aurora, Aurora Démo", ""));
        assert!(history_matches("Place d'Aurora, Aurora Démo", "aurora dé"));
        assert!(!history_matches("Nordheim", "lagune"));
    }
}
