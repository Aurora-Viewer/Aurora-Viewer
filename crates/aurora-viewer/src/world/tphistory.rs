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
use glam::{DVec3, Vec3};
use std::collections::HashMap;
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
    /// The file exists but could not be read: nothing can be added to it.
    unreadable: bool,
}

impl HistoryStorage {
    /// LLTeleportHistoryStorage::load, then sorted by date. A line that does
    /// not parse is kept as it is for the next save (the Legacy Viewer stops
    /// reading there); a file that exists but cannot be read is left alone
    /// for the session (not saved over).
    pub fn load(path: PathBuf) -> HistoryStorage {
        // A previous session may still be publishing this account's latest
        // history. Reading its snapshot also avoids waiting for a slow disk.
        let bytes = match writers().snapshot(&path) {
            Some(bytes) => bytes,
            None => match std::fs::read(&path) {
                Ok(bytes) => Arc::new(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Arc::default(),
                Err(e) => {
                    log::warn!("teleport history: can't read the saved history ({e}); it is left as it is and not saved this session");
                    return HistoryStorage {
                        unreadable: true,
                        ..HistoryStorage::default()
                    };
                }
            },
        };
        let (mut items, kept) = parse(&bytes);
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
            unreadable: false,
        }
    }

    /// A Legacy Viewer's places added to this history, adding only. `add`'s
    /// rule decides between two entries of one place: the newest visit
    /// stays, at the older position, so importing again changes nothing.
    /// The number of places added or updated; on a failed save the history
    /// stays as it was, in the file too. Nothing to add is never a failed
    /// save, even over a history that could not be read.
    pub fn merge(&mut self, items: Vec<HistoryItem>) -> std::io::Result<usize> {
        if items.is_empty() {
            return Ok(0);
        }
        if self.unreadable {
            return Err(std::io::Error::other("the saved history could not be read"));
        }
        let before = self.items.clone();
        let (changed, _) = merge_places(&mut self.items, items);
        if changed > 0 {
            self.items.sort_by(|a, b| a.date.total_cmp(&b.date));
            if let Err(e) = self.write_now() {
                self.items = before;
                return Err(e);
            }
            // again by the writer, flushed to the disk, after any save it
            // still holds
            self.save();
        }
        Ok(changed)
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
            self.items.remove(index);
            self.save();
        }
    }

    /// onClearTeleportHistoryDialog: purgeItems + save. The lines kept
    /// unread go too: the user asked for an empty history.
    pub fn clear(&mut self) {
        self.items.clear();
        self.kept.clear();
        self.save();
    }

    /// In one step, for a caller that has to know it worked: a full disk
    /// leaves the old file. No disk flush: on the frame thread.
    fn write_now(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::atomic_file::publish(path, &self.file())
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
            writers().queue(path.clone(), self.file(), write_file);
        }
    }

    #[cfg(test)]
    fn flush(&self) {
        if let Some(path) = &self.path {
            writers().wait(Some(path));
        }
    }
}

/// A place another file can bring: titled, at a finite position and date.
fn importable(item: &HistoryItem) -> bool {
    !item.title.is_empty() && item.global.is_finite() && item.date.is_finite()
}

/// The 5 m cell of a position: two places closer than
/// MAX_GLOBAL_POS_OFFSET are in the same cell or in neighbouring ones.
fn cell(global: DVec3) -> [i64; 3] {
    let c = (global / MAX_GLOBAL_POS_OFFSET).floor();
    // finite here (`importable`); `as` saturates a far one
    [c.x as i64, c.y as i64, c.z as i64]
}

/// `HistoryStorage::merge` on its items, without the save: the places
/// added or updated, and the distances measured (for the tests). Each place
/// is looked for among the entries of its title in its 5 m cell and the 26
/// around it, not among all the entries of its title. Between several
/// entries in range, the latest visit wins, the last one between equals:
/// any other choice would move a visit again at the next import.
fn merge_places(items: &mut Vec<HistoryItem>, new: Vec<HistoryItem>) -> (usize, usize) {
    let mut cells: HashMap<String, HashMap<[i64; 3], Vec<usize>>> = HashMap::new();
    // an entry of ours with no title, or not at a finite position, is in
    // range of no importable place
    for (i, item) in items
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.title.is_empty() && item.global.is_finite())
    {
        cells
            .entry(item.title.clone())
            .or_default()
            .entry(cell(item.global))
            .or_default()
            .push(i);
    }
    let (mut changed, mut measured) = (0, 0);
    for item in new.into_iter().filter(importable) {
        let by_cell = cells.entry(item.title.clone()).or_default();
        let [x, y, z] = cell(item.global);
        let mut found: Option<usize> = None;
        for dx in -1..=1_i64 {
            for dy in -1..=1_i64 {
                for dz in -1..=1_i64 {
                    let key = [x.saturating_add(dx), y.saturating_add(dy), z.saturating_add(dz)];
                    for &i in by_cell.get(&key).into_iter().flatten() {
                        measured += 1;
                        if (items[i].global - item.global).length() >= MAX_GLOBAL_POS_OFFSET {
                            continue;
                        }
                        let later = |f: usize| items[i].date.total_cmp(&items[f].date).then(i.cmp(&f)).is_gt();
                        if found.is_none_or(later) {
                            found = Some(i);
                        }
                    }
                }
            }
        }
        match found {
            Some(i) if items[i].date < item.date => {
                items[i].date = item.date;
                if !item.slurl.is_empty() {
                    items[i].slurl = item.slurl;
                }
                changed += 1;
            }
            Some(_) => {}
            None => {
                by_cell.entry([x, y, z]).or_default().push(items.len());
                items.push(item);
                changed += 1;
            }
        }
    }
    (changed, measured)
}

/// The places of a Legacy Viewer's `teleport_history.txt`, oldest first; a
/// line that is not one, or a place without a title or with a position or
/// date that is not a number, is left out.
pub fn places(bytes: &[u8]) -> Vec<HistoryItem> {
    let (items, kept) = parse(bytes);
    let read = items.len();
    let mut items: Vec<_> = items.into_iter().filter(importable).collect();
    let skipped = kept.len() + read - items.len();
    if skipped > 0 {
        log::info!("teleport history: {skipped} line(s) could not be read");
    }
    items.sort_by(|a, b| a.date.total_cmp(&b.date));
    items
}

/// The places of a file, and its lines that are not one (kept as they are).
fn parse(bytes: &[u8]) -> (Vec<HistoryItem>, Vec<Vec<u8>>) {
    let mut items = Vec::new();
    let mut kept = Vec::new();
    for line in bytes.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match aurora_llsd::from_notation(line) {
            Ok(v) if v.is_map() => items.push(HistoryItem::from_llsd(&v)),
            _ => kept.push(line.to_vec()),
        }
    }
    (items, kept)
}

fn write_file(path: &std::path::Path, file: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic_file::write(path, file)
}

/// One publisher per account file, shared across session and World resets.
/// A refused write keeps its snapshot pending (a new session of the account
/// reads it from here) and is tried again, waiting longer each time
/// (`save_retry`), or at once with the newer snapshot when the wait ends;
/// the exit's [`finish_writes`] makes the last try. The Legacy Viewer's
/// LLTeleportHistoryStorage::save writes once and warns.
#[derive(Default)]
struct Writers {
    pending: Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>,
    finished: Condvar,
    /// Wakes a publisher waiting to try a refused write again.
    wake: Condvar,
    /// The exit waits for the writes: a refused one is tried once more,
    /// then given up.
    closing: std::sync::atomic::AtomicBool,
}

impl Writers {
    fn snapshot(&self, path: &Path) -> Option<Arc<Vec<u8>>> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).get(path).cloned()
    }

    fn queue(self: &Arc<Self>, path: PathBuf, bytes: Vec<u8>, publish: impl FnMut(&Path, &[u8]) -> std::io::Result<()> + Send + 'static) {
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

    fn publish(&self, path: &Path, mut bytes: Arc<Vec<u8>>, mut write: impl FnMut(&Path, &[u8]) -> std::io::Result<()>) {
        use std::sync::atomic::Ordering;
        let mut backoff = crate::save_retry::Backoff::default();
        loop {
            let written = write(path, &bytes);
            let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            let newer = |pending: &HashMap<PathBuf, Arc<Vec<u8>>>| pending.get(path).is_some_and(|latest| !Arc::ptr_eq(latest, &bytes));
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

    fn place(title: &str, x: f64, date: f64) -> HistoryItem {
        HistoryItem {
            title: title.into(),
            global: DVec3::new(x, 0.0, 0.0),
            date,
            slurl: String::new(),
        }
    }

    #[test]
    fn merge_keeps_the_newest_visit_of_a_place_and_adds_nothing_twice() {
        let mut history = HistoryStorage::in_memory(vec![place("Home", 0.0, 100.0), place("Shop", 10.0, 200.0)]);
        let legacy = places(
            b"{'date':d\"1970-01-01T00:00:50Z\",'global_pos':[r0,r0,r0],'slurl':'','title':'Home'}\n\
             not a place\n\
             {'date':d\"1970-01-01T00:05:00Z\",'global_pos':[r13,r0,r0],'slurl':'','title':'Shop'}\n\
             {'date':d\"1970-01-01T00:02:30Z\",'global_pos':[r500,r0,r0],'slurl':'','title':'Shop'}\n\
             {'date':d\"1970-01-01T00:01:00Z\",'global_pos':[r900,r0,r0],'slurl':'','title':'Far'}\n",
        );
        assert_eq!(legacy.len(), 4);
        assert_eq!(history.merge(legacy.clone()).unwrap(), 3);
        let seen: Vec<_> = history.items.iter().map(|i| (i.title.as_str(), i.global.x, i.date)).collect();
        assert_eq!(
            seen,
            [
                ("Far", 900.0, 60.0),
                ("Home", 0.0, 100.0),
                ("Shop", 500.0, 150.0),
                // the visit is the Legacy one, the position stays ours
                ("Shop", 10.0, 300.0),
            ]
        );
        assert_eq!(history.merge(legacy).unwrap(), 0);
    }

    #[test]
    fn merge_settles_at_once_when_a_place_is_in_range_of_two_entries() {
        let legacy = vec![place("Shop", 0.0, 10.0), place("Shop", 6.0, 20.0), place("Shop", 3.0, 30.0)];
        let mut history = HistoryStorage::default();
        assert_eq!(history.merge(legacy.clone()).unwrap(), 3);
        assert_eq!(history.merge(legacy).unwrap(), 0);
    }

    /// The merge as it was, every entry of a title measured.
    fn merge_places_by_title(items: &mut Vec<HistoryItem>, new: Vec<HistoryItem>) -> usize {
        let mut by_title: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, item) in items.iter().enumerate() {
            by_title.entry(item.title.clone()).or_default().push(i);
        }
        let mut changed = 0;
        for item in new.into_iter().filter(|item| !item.title.is_empty()) {
            let same = by_title.entry(item.title.clone()).or_default();
            let found = same
                .iter()
                .copied()
                .filter(|&i| (items[i].global - item.global).length() < MAX_GLOBAL_POS_OFFSET)
                .max_by(|&a, &b| items[a].date.total_cmp(&items[b].date));
            match found {
                Some(i) if items[i].date < item.date => {
                    items[i].date = item.date;
                    if !item.slurl.is_empty() {
                        items[i].slurl = item.slurl;
                    }
                    changed += 1;
                }
                Some(_) => {}
                None => {
                    same.push(items.len());
                    items.push(item);
                    changed += 1;
                }
            }
        }
        changed
    }

    /// Made-up places, the same each run: few titles, close together and
    /// across cell borders, dates that repeat, some with a SLURL.
    fn made_up_places(n: usize, seed: u64) -> Vec<HistoryItem> {
        let mut s = seed;
        let mut next = move |m: u64| {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (s >> 33) % m
        };
        (0..n)
            .map(|_| HistoryItem {
                title: ["Home", "Shop", "Pier", ""][next(4) as usize].into(),
                global: DVec3::new(next(40) as f64 * 0.75 - 7.5, next(12) as f64 * 0.9, next(3) as f64 * 2.5),
                date: next(20) as f64 * 10.0,
                slurl: if next(3) == 0 {
                    format!("hop://grid/{}", next(100))
                } else {
                    String::new()
                },
            })
            .collect()
    }

    #[test]
    fn merge_by_cells_gives_what_merge_by_title_gave() {
        for seed in 0..40 {
            let ours: Vec<_> = made_up_places(60, seed).into_iter().filter(|item| !item.title.is_empty()).collect();
            let legacy = made_up_places(120, seed + 1000);
            let mut by_title = ours.clone();
            let mut by_cells = ours;
            let expected = merge_places_by_title(&mut by_title, legacy.clone());
            assert_eq!(merge_places(&mut by_cells, legacy).0, expected, "seed {seed}");
            assert_eq!(by_cells, by_title, "seed {seed}");
        }
    }

    #[test]
    fn a_long_history_of_one_title_is_merged_without_measuring_every_pair() {
        // one title, a place every 10 m: measured pair by pair, 50 million
        // distances
        let n = 10_000;
        let legacy: Vec<_> = (0..n).map(|i| place("Sandbox", i as f64 * 10.0, i as f64)).collect();
        let mut items = Vec::new();
        let (changed, measured) = merge_places(&mut items, legacy.clone());
        assert_eq!(changed, n);
        assert!(measured <= 2 * n, "{measured} distances measured");
        let (changed, measured) = merge_places(&mut items, legacy);
        assert_eq!(changed, 0);
        assert!(measured <= 3 * n, "{measured} distances measured");
        assert_eq!(items.len(), n);
    }

    #[test]
    fn places_leave_out_the_untitled_and_what_is_not_a_number() {
        let read = places(
            b"{'date':d\"1970-01-01T00:00:50Z\",'global_pos':[r0,r0,r0],'slurl':'','title':''}\n\
              {'date':d\"1970-01-01T00:00:50Z\",'global_pos':[rnan,r0,r0],'slurl':'','title':'Nan'}\n\
              {'date':d\"1970-01-01T00:00:50Z\",'global_pos':[rinf,r0,r0],'slurl':'','title':'Far'}\n\
              {'date':d\"1970-01-01T00:00:50Z\",'global_pos':[r1,r2,r3],'slurl':'','title':'Home'}\n",
        );
        let titles: Vec<_> = read.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Home"]);
        // what the window counts is what the merge can add
        let mut history = HistoryStorage::default();
        assert_eq!(history.merge(read.clone()).unwrap(), read.len());
        let mut bad = place("Home", f64::NAN, 10.0);
        assert_eq!(merge_places(&mut history.items, vec![bad.clone()]).0, 0);
        bad.global.x = 0.0;
        bad.date = f64::INFINITY;
        assert_eq!(merge_places(&mut history.items, vec![bad]).0, 0);
    }

    #[test]
    fn a_merge_that_cannot_be_saved_leaves_the_history_as_it_was() {
        let dir = std::env::temp_dir().join(format!("aurora-tphistory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // the file's folder is a file
        std::fs::write(dir.join("taken"), "").unwrap();
        let mut history = HistoryStorage::in_memory(vec![place("Home", 0.0, 100.0)]);
        history.path = Some(dir.join("taken").join(TELEPORT_HISTORY_FILE));
        assert!(history.merge(vec![place("Far", 900.0, 60.0)]).is_err());
        assert_eq!(history.items.len(), 1);
        history.path = Some(dir.join(TELEPORT_HISTORY_FILE));
        assert_eq!(history.merge(vec![place("Far", 900.0, 60.0)]).unwrap(), 1);
        assert_eq!(HistoryStorage::load(dir.join(TELEPORT_HISTORY_FILE)).items.len(), 2);
        history.flush();
        assert_eq!(HistoryStorage::load(dir.join(TELEPORT_HISTORY_FILE)).items.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_merge_keeps_the_lines_that_could_not_be_read() {
        let dir = std::env::temp_dir().join(format!("aurora-tphistory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(TELEPORT_HISTORY_FILE);
        std::fs::write(&path, "not a place\n").unwrap();
        let mut history = HistoryStorage::load(path.clone());
        assert_eq!(history.merge(vec![place("Far", 900.0, 60.0)]).unwrap(), 1);
        history.flush();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("not a place\n"), "{text}");
        assert_eq!(HistoryStorage::load(path).items.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_merge_does_not_replace_a_file_that_could_not_be_read() {
        // a folder where the file should be: it exists and cannot be read
        let dir = std::env::temp_dir().join(format!("aurora-tphistory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut history = HistoryStorage::load(dir.clone());
        assert!(history.merge(vec![place("Far", 900.0, 60.0)]).is_err());
        assert!(history.items.is_empty());
        assert!(dir.is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn places_of_a_legacy_file_come_oldest_first_without_the_bad_lines() {
        let line = |p: &HistoryItem| aurora_llsd::to_notation(&p.to_llsd());
        let text = format!(
            "{}\nnot a place\n{}\n",
            line(&place("Late", 0.0, 200.0)),
            line(&place("Early", 10.0, 100.0))
        );
        let read = places(text.as_bytes());
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].title, "Early");
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
        // nothing to import is not a failed save
        assert_eq!(s.merge(Vec::new()).ok(), Some(0));
        let item = s.items[0].clone();
        assert!(s.merge(vec![item]).is_err(), "a place to add still fails");
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
        writers().queue(path.clone(), Vec::new(), move |path, bytes| {
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
        writers.queue(path.clone(), b"visits".to_vec(), move |path, bytes| {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
            write_file(path, bytes)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while tries.load(Ordering::SeqCst) < 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(writers.snapshot(&path).as_deref().map(Vec::as_slice), Some(&b"visits"[..]), "kept");
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
