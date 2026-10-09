//! Teleport history: the navigation bar's back / forward list (behaviour of
//! LLTeleportHistory: arrivals after a teleport are recorded, going back or
//! forward moves within the list without truncating it) and the dated,
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

    pub fn next(&self) -> Option<&TpEntry> {
        self.items.get(self.current + 1)
    }

    /// Start travelling back; returns the destination.
    pub fn go_back(&mut self) -> Option<TpEntry> {
        let i = self.current.checked_sub(1)?;
        self.pending = Some(i);
        self.items.get(i).cloned()
    }

    pub fn go_forward(&mut self) -> Option<TpEntry> {
        let i = self.current + 1;
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
            Llsd::Array(vec![Llsd::Real(self.global.x), Llsd::Real(self.global.y), Llsd::Real(self.global.z)]),
        );
        m.insert("date", Llsd::Date(self.date));
        m.insert("slurl", self.slurl.clone());
        m
    }
}

/// The saved history, oldest first (LLTeleportHistoryStorage::mItems).
#[derive(Debug, Default)]
pub struct HistoryStorage {
    pub items: Vec<HistoryItem>,
    /// None: not saved (demo, before login).
    path: Option<PathBuf>,
}

impl HistoryStorage {
    /// LLTeleportHistoryStorage::load: lines that do not parse are skipped
    /// (Firestorm stops at the first one), then sorted by date.
    pub fn load(path: PathBuf) -> HistoryStorage {
        let mut items = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                match aurora_llsd::from_notation(line.as_bytes()) {
                    Ok(v) if v.is_map() => items.push(HistoryItem::from_llsd(&v)),
                    _ => log::info!("teleport history: a line could not be read"),
                }
            }
        }
        items.sort_by(|a, b| a.date.total_cmp(&b.date));
        HistoryStorage { items, path: Some(path) }
    }

    /// A history kept in memory only (the demo's).
    pub fn in_memory(items: Vec<HistoryItem>) -> HistoryStorage {
        HistoryStorage { items, path: None }
    }

    /// LLTeleportHistoryStorage::addItem: the same title within 5 m replaces
    /// the older entry and keeps its position (teleports through the history
    /// would otherwise creep up a meter each time).
    pub fn add(&mut self, title: &str, global: DVec3, date: f64) {
        let mut item = HistoryItem {
            title: title.to_owned(),
            global,
            date,
            slurl: String::new(),
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

    /// "Supprimer de l'historique" (removeItem + save).
    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.save();
        }
    }

    /// onClearTeleportHistoryDialog: purgeItems + save.
    pub fn clear(&mut self) {
        self.items.clear();
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut text = String::new();
        for it in &self.items {
            text.push_str(&aurora_llsd::to_notation(&it.to_llsd()));
            text.push('\n');
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, text) {
            log::warn!("can't save the teleport history: {e}");
        }
    }
}

/// Accordion sections of the history (panel_teleport_history.xml).
pub const HISTORY_SECTIONS: [&str; 9] = [
    "Aujourd'hui",
    "Hier",
    "Il y a 2 jours",
    "Il y a 3 jours",
    "Il y a 4 jours",
    "Il y a 5 jours",
    "Il y a 6 jours et plus",
    "Il y a 1 mois et plus",
    "Il y a 6 mois et plus",
];

/// LLTeleportHistoryPanel::getNextTab with FSTPHistoryTZ "utc" (the
/// default): the first six sections are days starting at midnight UTC,
/// "6 jours et plus" goes back to the same day of the previous month and
/// "1 mois et plus" to six months ago. A date in the future counts as
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
    for i in 0..HISTORY_SECTIONS.len() - 1 {
        let lower = match i {
            0..=5 => (today - i as f64) * DAY,
            6 => months_back(1),
            _ => months_back(6),
        };
        if date >= lower {
            return i;
        }
    }
    HISTORY_SECTIONS.len() - 1
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
        }
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
        s.add("Place d'Aurora, Aurora Démo", DVec3::new(256_140.0, 256_120.0, 25.0), 1000.0);
        s.add("Lagune, Lagune Boréale", DVec3::new(256_316.0, 256_200.0, 22.0), 2000.0);
        // back to the first place, a meter higher: same entry, original position
        s.add("Place d'Aurora, Aurora Démo", DVec3::new(256_140.0, 256_120.0, 26.0), 3000.0);
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items[1].title, "Place d'Aurora, Aurora Démo");
        assert_eq!(s.items[1].global.z, 25.0);
        assert_eq!(s.items[1].date, 3000.0);
        // an older date is sorted in
        s.add("Nordheim", DVec3::new(256_000.0, 256_300.0, 40.0), 1500.0);
        assert_eq!(s.items[0].title, "Nordheim");
        let again = HistoryStorage::load(path.clone());
        assert_eq!(again.items, s.items);
        s.remove(0);
        s.clear();
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
    fn filter_ignores_case() {
        assert!(history_matches("Place d'Aurora, Aurora Démo", ""));
        assert!(history_matches("Place d'Aurora, Aurora Démo", "aurora dé"));
        assert!(!history_matches("Nordheim", "lagune"));
    }
}
