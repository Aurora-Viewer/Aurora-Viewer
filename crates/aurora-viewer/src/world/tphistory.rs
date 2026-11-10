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
use std::path::PathBuf;

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
    /// Arrival at a new location (login or teleport).
    pub fn arrived(&mut self, entry: TpEntry) {
        if let Some(i) = self.pending.take()
            && i < self.items.len()
        {
            self.current = i;
            self.items[i] = entry;
            return;
        }
        if !self.items.is_empty() {
            self.items.truncate(self.current + 1);
        }
        self.items.push(entry);
        if self.items.len() > MAX_ITEMS {
            self.items.remove(0);
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

    /// RlvUIEnabler::onToggleShowLoc keeps the navigation destination while
    /// removing the current location's visible name.
    pub fn replace_current_region(&mut self, region: &str) -> Option<TpEntry> {
        let current = self.items.get_mut(self.current)?;
        let previous = current.clone();
        current.region = region.to_owned();
        current.parcel.clear();
        Some(previous)
    }

    pub fn next(&self) -> Option<&TpEntry> {
        self.items.get(self.current + 1)
    }

    /// Start travelling back; returns the destination.
    pub fn go_back(&mut self) -> Option<TpEntry> {
        self.go_to(self.current.checked_sub(1)?)
    }

    pub fn go_forward(&mut self) -> Option<TpEntry> {
        self.go_to(self.current + 1)
    }

    /// LLTeleportHistory::goToItem: start travelling to the place `i` of
    /// the list; none for the place we are in or past the end.
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

/// How long dropping a storage waits for its last save (logout, quit): a
/// stuck disk cannot hold up the viewer longer than that.
const LAST_SAVE_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

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
    /// The thread that writes the file (atomic_file flushes the disk: never
    /// on the frame thread), started at the first save.
    writer: Option<Writer>,
}

#[derive(Debug)]
struct Writer {
    files: std::sync::mpsc::Sender<Vec<u8>>,
    /// Told when the thread has written everything and stopped.
    /// (in a Mutex: the World is shared with the frame's worker threads)
    done: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Drop for HistoryStorage {
    fn drop(&mut self) {
        if let Some(Writer { files, done }) = self.writer.take() {
            drop(files);
            let _ = done.into_inner().map(|d| d.recv_timeout(LAST_SAVE_WAIT));
        }
    }
}

impl HistoryStorage {
    /// LLTeleportHistoryStorage::load, then sorted by date. A line that does
    /// not parse is kept as it is for the next save (the Legacy Viewer stops
    /// reading there); a file that exists but cannot be read is left alone
    /// for the session (not saved over).
    pub fn load(path: PathBuf) -> HistoryStorage {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                log::warn!("teleport history: can't read the saved history ({e}); it is left as it is and not saved this session");
                return HistoryStorage::default();
            }
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
            writer: None,
        }
    }

    /// A history kept in memory only (the demo's).
    pub fn in_memory(items: Vec<HistoryItem>) -> HistoryStorage {
        let mut storage = HistoryStorage::default();
        storage.items = items;
        storage
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

    /// Hands the file to the writer thread; the newest one waiting wins.
    fn save(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let file = self.file();
        if self.writer.is_none() {
            self.writer = spawn_writer(path.clone());
        }
        let sent = self.writer.as_ref().is_some_and(|w| w.files.send(file.clone()).is_ok());
        if !sent {
            // no thread: written here, as before
            self.writer = None;
            if let Err(e) = write_file(&path, &file) {
                log::warn!("can't save the teleport history: {e}");
            }
        }
    }

    /// Waits for the saves handed to the writer (tests).
    #[cfg(test)]
    fn flush(&mut self) {
        if let Some(Writer { files, done }) = self.writer.take() {
            drop(files);
            let _ = done.into_inner().map(|d| d.recv());
        }
    }
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

fn spawn_writer(path: PathBuf) -> Option<Writer> {
    let (files, queue) = std::sync::mpsc::channel::<Vec<u8>>();
    let (finished, done) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("aurora-tp-history".into())
        .spawn(move || {
            while let Ok(mut file) = queue.recv() {
                while let Ok(newer) = queue.try_recv() {
                    file = newer;
                }
                if let Err(e) = write_file(&path, &file) {
                    log::warn!("can't save the teleport history: {e}");
                }
            }
            let _ = finished.send(());
        })
        .map_err(|e| log::warn!("teleport history: no writer thread ({e}); saved on the caller's thread"))
        .ok()?;
    Some(Writer {
        files,
        done: std::sync::Mutex::new(done),
    })
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

/// LLTeleportHistoryPanel::getNextTab with FSTPHistoryTZ "utc" (the
/// default): the first six sections are days starting at midnight UTC,
/// "6 days ago or earlier" goes back to the same day of the previous month and
/// "1 month ago or earlier" to six months ago. A date in the future counts as
/// today. Returns the index in HISTORY_SECTIONS.
pub fn history_section(date: f64, now: f64) -> usize {
    use aurora_net::profile::{civil_from_days, days_from_civil};
    const DAY: f64 = 86_400.0;
    let today = (now / DAY).floor();
    let (y, m, d) = civil_from_days(today as i64);
    // LLDate::fromYMDHMS normalizes a day past the month's end, as does
    // days_from_civil
    let months_back = |n: i64| {
        let total = y * 12 + (m as i64 - 1) - n;
        days_from_civil(total.div_euclid(12), (total.rem_euclid(12) + 1) as u32, d) as f64 * DAY
    };
    for i in 0..history_sections().len() - 1 {
        let lower = match i {
            0..=5 => (today - i as f64) * DAY,
            6 => months_back(1),
            _ => months_back(6),
        };
        if date >= lower {
            return i;
        }
    }
    history_sections().len() - 1
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

    fn e(n: &str) -> TpEntry {
        TpEntry {
            region: n.into(),
            handle: 0,
            position: Vec3::ZERO,
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

    const DAY: f64 = 86_400.0;

    #[test]
    fn sections_by_day_then_month() {
        use aurora_net::profile::days_from_civil;
        // 2026-10-10 15:00 UTC
        let now = days_from_civil(2026, 10, 10) as f64 * DAY + 15.0 * 3600.0;
        let midnight = days_from_civil(2026, 10, 10) as f64 * DAY;
        assert_eq!(history_section(now, now), 0);
        assert_eq!(history_section(midnight, now), 0);
        assert_eq!(history_section(midnight - 1.0, now), 1);
        assert_eq!(history_section(now + DAY, now), 0, "future dates count as today");
        for days in 2..=5 {
            assert_eq!(history_section(midnight - (days as f64 - 0.5) * DAY, now), days);
        }
        assert_eq!(history_section(midnight - 5.5 * DAY, now), 6);
        // one month back: 2026-09-10 00:00
        let month = days_from_civil(2026, 9, 10) as f64 * DAY;
        assert_eq!(history_section(month, now), 6);
        assert_eq!(history_section(month - 1.0, now), 7);
        let six = days_from_civil(2026, 4, 10) as f64 * DAY;
        assert_eq!(history_section(six, now), 7);
        assert_eq!(history_section(six - 1.0, now), 8);
        // January: the months before go back into the previous year
        let jan = days_from_civil(2027, 1, 31) as f64 * DAY;
        assert_eq!(history_section(days_from_civil(2026, 12, 31) as f64 * DAY, jan), 6);
        assert_eq!(history_section(days_from_civil(2026, 7, 31) as f64 * DAY, jan), 7);
        assert_eq!(history_section(days_from_civil(2026, 7, 30) as f64 * DAY, jan), 8);
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
        assert!(s.writer.is_none());
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
        // dropping waits for the last save (logout, quit)
        drop(s);
        assert_eq!(HistoryStorage::load(path.clone()).items.len(), 1);
        // the atomic write leaves no temporary file behind
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(TELEPORT_HISTORY_FILE)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_ignores_case() {
        assert!(history_matches("Place d'Aurora, Aurora Démo", ""));
        assert!(history_matches("Place d'Aurora, Aurora Démo", "aurora dé"));
        assert!(!history_matches("Nordheim", "lagune"));
    }
}
