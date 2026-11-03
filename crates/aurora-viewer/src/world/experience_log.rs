//! The experience event log: what objects did with the permissions the agent
//! gave to experiences. Port of LLExperienceLog (indra/newview/
//! llexperiencelog.cpp, originally LGPL 2.1): an event equal to the previous
//! one only counts it again, days older than `max_days` are dropped (0 days:
//! today only, nothing read back at login), and the log and its three
//! settings live in one file per account.
//!
//! The Legacy Viewer writes the file once, at exit. Here it is written on a
//! worker thread, a moment after the last change (an event, a setting), so a
//! crash loses little and the frame never waits for the disk. The log keeps
//! [`MAX_ENTRIES`] at most: a script alternating two events would otherwise
//! add an entry each time for the whole kept period.

use aurora_net::ExperienceLogEvent;
use crossbeam_channel::{self as mpsc, Sender};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FILE: &str = "experience_events.json";

/// LLExperienceLog::LLExperienceLog.
const DEFAULT_MAX_DAYS: u32 = 7;
const DEFAULT_PAGE_SIZE: usize = 25;
/// The days spinner of panel_experience_log.xml (0..14).
pub const MAX_DAYS: u32 = 14;
/// Entries kept, the newest.
pub const MAX_ENTRIES: usize = 500;
/// A write waits for this much quiet after the last change…
const WRITE_QUIET: Duration = Duration::from_secs(1);
/// …but never longer than this after the first one.
const WRITE_MAX_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// "YYYY-MM-DD", local time.
    pub day: String,
    /// "HH:MM:SS", local time, of the latest occurrence.
    pub time: String,
    pub experience: Uuid,
    pub permission: i32,
    pub owner: Uuid,
    pub object_name: String,
    pub parcel_name: String,
    pub attachment: bool,
    pub count: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    events: Vec<LogEntry>,
    max_days: u32,
    notify: bool,
    page_size: usize,
}

impl Default for Stored {
    fn default() -> Self {
        Stored {
            events: Vec::new(),
            max_days: DEFAULT_MAX_DAYS,
            notify: false,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

/// The worker that writes the file.
struct Writer {
    sender: Option<Sender<Stored>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Writer {
    fn drop(&mut self) {
        // the last state is written before the account's session ends
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Writer")
    }
}

impl Writer {
    fn start(path: PathBuf) -> Option<Writer> {
        let (sender, changes) = mpsc::unbounded::<Stored>();
        let worker = std::thread::Builder::new().name("aurora-experience-log".into()).spawn(move || {
            while let Ok(mut stored) = changes.recv() {
                // only the latest state matters: wait for a quiet moment
                let first = Instant::now();
                while first.elapsed() < WRITE_MAX_DELAY {
                    match changes.recv_timeout(WRITE_QUIET) {
                        Ok(newer) => stored = newer,
                        Err(_) => break,
                    }
                }
                write(&path, &stored);
            }
        });
        match worker {
            Ok(worker) => Some(Writer {
                sender: Some(sender),
                worker: Some(worker),
            }),
            Err(error) => {
                log::warn!("Cannot start the experience log writer: {error}");
                None
            }
        }
    }
}

#[derive(Debug)]
pub struct ExperienceLog {
    /// Oldest first, as they arrived.
    pub events: Vec<LogEntry>,
    /// 0..=[`MAX_DAYS`]; change it with [`ExperienceLog::set_max_days`].
    pub max_days: u32,
    /// "Notify all events": say each new event in the chat.
    pub notify: bool,
    pub page_size: usize,
    writer: Option<Writer>,
}

impl Default for ExperienceLog {
    fn default() -> Self {
        let Stored {
            events,
            max_days,
            notify,
            page_size,
        } = Stored::default();
        ExperienceLog {
            events,
            max_days,
            notify,
            page_size,
            writer: None,
        }
    }
}

impl ExperienceLog {
    /// The account's log (LLExperienceLog::loadEvents); `today` drops the
    /// days that expired. A small file read once at login; every write is
    /// the worker's.
    pub fn load(path: PathBuf, today: &str) -> Self {
        let stored = read(&path);
        let mut log = ExperienceLog {
            events: stored.events,
            max_days: stored.max_days.min(MAX_DAYS),
            notify: stored.notify,
            page_size: stored.page_size.max(1),
            writer: Writer::start(path),
        };
        // loadEvents: with 0 days, nothing is read back
        if log.max_days == 0 {
            log.events.clear();
        }
        log.erase_expired(today);
        log.cap();
        log
    }

    /// LLExperienceLog::handleExperienceMessage: a repeat of the previous
    /// event counts it again, anything else is a new entry. The entry that
    /// was added or counted.
    pub fn record(&mut self, event: &ExperienceLogEvent, day: &str, time: &str) -> &LogEntry {
        let same = |last: &LogEntry| {
            last.day == day
                && last.experience == event.experience
                && last.object_name == event.object_name
                && last.owner == event.owner
                && last.parcel_name == event.parcel_name
                && last.permission == event.permission
        };
        if let Some(last) = self.events.last_mut().filter(|last| same(last)) {
            last.count = last.count.saturating_add(1);
            last.time = time.to_owned();
        } else {
            // a new day may have started since the older entries
            self.erase_expired(day);
            self.events.push(LogEntry {
                day: day.to_owned(),
                time: time.to_owned(),
                experience: event.experience,
                permission: event.permission,
                owner: event.owner,
                object_name: event.object_name.clone(),
                parcel_name: event.parcel_name.clone(),
                attachment: event.attachment,
                count: 1,
            });
            self.cap();
        }
        self.save();
        self.events.last().expect("an entry was just recorded or counted")
    }

    /// LLExperienceLog::eraseExpired / isNotExpired: the days before
    /// `today - max_days` go (0 days keeps today only). True when some went.
    pub fn erase_expired(&mut self, today: &str) -> bool {
        let Ok(today) = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d") else {
            return false;
        };
        let boundary = today - chrono::Duration::days(i64::from(self.max_days));
        let before = self.events.len();
        self.events
            .retain(|e| chrono::NaiveDate::parse_from_str(&e.day, "%Y-%m-%d").is_ok_and(|day| day >= boundary));
        self.events.len() != before
    }

    /// The Legacy panel's refresh: the days that expired since (a new day,
    /// fewer days kept) leave the list and the file.
    pub fn refresh(&mut self, today: &str) {
        if self.erase_expired(today) {
            self.save();
        }
    }

    /// LLPanelExperienceLog::logSizeChanged: the days kept, then the list
    /// shown again without the days that no longer fit.
    pub fn set_max_days(&mut self, days: u32, today: &str) {
        self.max_days = days.min(MAX_DAYS);
        self.erase_expired(today);
        self.save();
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.save();
    }

    pub fn pages(&self) -> usize {
        self.events.len().div_ceil(self.page_size.max(1)).max(1)
    }

    /// Hand the state to the worker; nothing is written on this thread.
    pub fn save(&self) {
        let Some(sender) = self.writer.as_ref().and_then(|w| w.sender.as_ref()) else {
            return;
        };
        let _ = sender.send(Stored {
            events: self.events.clone(),
            max_days: self.max_days,
            notify: self.notify,
            page_size: self.page_size,
        });
    }

    /// The newest [`MAX_ENTRIES`].
    fn cap(&mut self) {
        let over = self.events.len().saturating_sub(MAX_ENTRIES);
        self.events.drain(..over);
    }
}

/// The file, with what a newer build added left out (`tolerant`). A file that
/// cannot be read at all is kept aside before the next write replaces it.
fn read(path: &Path) -> Stored {
    match std::fs::read(path) {
        Ok(bytes) => match crate::tolerant::from_slice::<Stored>(&bytes, FILE) {
            Some(read) => {
                if read.repaired() {
                    crate::tolerant::backup(path);
                }
                read.value
            }
            None => {
                crate::tolerant::backup(path);
                Stored::default()
            }
        },
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("Cannot read the experience log: {error}");
            }
            Stored::default()
        }
    }
}

fn write(path: &Path, stored: &Stored) {
    let save = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(stored).map_err(std::io::Error::other)?;
        crate::atomic_file::write(path, &bytes)
    };
    if let Err(error) = save() {
        log::warn!("Cannot save the experience log: {error}");
    }
}

/// "YYYY-MM-DD" and "HH:MM:SS" of now, local time.
pub fn now() -> (String, String) {
    let now = chrono::Local::now();
    (now.format("%Y-%m-%d").to_string(), now.format("%H:%M:%S").to_string())
}

/// LLTrans ExperiencePermissionShort<n>: the event column.
pub fn permission_short(permission: i32) -> String {
    match permission {
        1 => aurora_i18n::tr!("ui-experiences-log-take-controls").to_string(),
        3 => aurora_i18n::tr!("ui-experiences-log-trigger-animations").to_string(),
        4 => aurora_i18n::tr!("ui-experiences-log-attach").to_string(),
        9 => aurora_i18n::tr!("ui-experiences-log-track-camera").to_string(),
        10 => aurora_i18n::tr!("ui-experiences-log-control-camera").to_string(),
        11 => aurora_i18n::tr!("ui-experiences-log-teleport").to_string(),
        12 => aurora_i18n::tr!("ui-experiences-log-permission").to_string(),
        16 => aurora_i18n::tr!("ui-experiences-log-sit").to_string(),
        17 => aurora_i18n::tr!("ui-experiences-log-environment").to_string(),
        other => aurora_i18n::tr!("ui-experiences-log-unknown", permission = other.to_string()).to_string(),
    }
}

/// LLTrans ExperiencePermission<n>: what the object was allowed to do, for
/// the notification.
pub fn permission_long(permission: i32) -> String {
    match permission {
        1 => aurora_i18n::tr!("ui-experiences-log-long-take-controls").to_string(),
        3 => aurora_i18n::tr!("ui-experiences-log-long-trigger-animations").to_string(),
        4 => aurora_i18n::tr!("ui-experiences-log-long-attach").to_string(),
        9 => aurora_i18n::tr!("ui-experiences-log-long-track-camera").to_string(),
        10 => aurora_i18n::tr!("ui-experiences-log-long-control-camera").to_string(),
        11 => aurora_i18n::tr!("ui-experiences-log-long-teleport").to_string(),
        12 => aurora_i18n::tr!("ui-experiences-log-long-permission").to_string(),
        16 => aurora_i18n::tr!("ui-experiences-log-long-sit").to_string(),
        17 => aurora_i18n::tr!("ui-experiences-log-long-environment").to_string(),
        other => aurora_i18n::tr!("ui-experiences-log-unknown", permission = other.to_string()).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(object: &str, permission: i32) -> ExperienceLogEvent {
        ExperienceLogEvent {
            experience: Uuid::from_u128(1),
            permission,
            owner: Uuid::from_u128(2),
            object_name: object.into(),
            parcel_name: "Plaza".into(),
            attachment: false,
        }
    }

    fn temp() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("aurora-xp-log-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        (dir, path)
    }

    #[test]
    fn a_repeated_event_is_counted_not_listed_again() {
        let mut log = ExperienceLog::default();
        log.record(&event("Door", 11), "2026-05-01", "10:00:00");
        let counted = log.record(&event("Door", 11), "2026-05-01", "10:00:05").clone();
        log.record(&event("Chair", 16), "2026-05-01", "10:01:00");
        log.record(&event("Door", 11), "2026-05-01", "10:02:00");
        assert_eq!((counted.count, counted.time.as_str()), (2, "10:00:05"));
        assert_eq!(log.events.len(), 3);
        assert_eq!(log.events.last().map(|e| e.object_name.as_str()), Some("Door"));
    }

    #[test]
    fn days_older_than_the_limit_are_dropped() {
        let mut log = ExperienceLog::default();
        for day in ["2026-04-20", "2026-04-24", "2026-04-30"] {
            log.record(&event(day, 4), day, "10:00:00");
        }
        // a new day drops what expired with it
        log.record(&event("Today", 4), "2026-05-01", "10:00:00");
        let days: Vec<_> = log.events.iter().map(|e| e.day.as_str()).collect();
        assert_eq!(days, ["2026-04-24", "2026-04-30", "2026-05-01"]);
        // fewer days: the list loses them at once (logSizeChanged)
        log.set_max_days(1, "2026-05-01");
        let days: Vec<_> = log.events.iter().map(|e| e.day.as_str()).collect();
        assert_eq!(days, ["2026-04-30", "2026-05-01"]);
        // 0 days keeps today only (isNotExpired)
        log.set_max_days(0, "2026-05-01");
        assert_eq!(log.events.len(), 1);
        // past the spinner's range: 14
        log.set_max_days(365, "2026-05-01");
        assert_eq!(log.max_days, MAX_DAYS);
    }

    #[test]
    fn the_log_keeps_the_newest_entries_only() {
        let mut log = ExperienceLog::default();
        for n in 0..MAX_ENTRIES + 20 {
            // two objects in turn: never a repeat
            log.record(&event(&format!("Object {}", n % 2), 4), "2026-05-01", &n.to_string());
        }
        assert_eq!(log.events.len(), MAX_ENTRIES);
        assert_eq!(log.events.last().map(|e| e.time.clone()), Some((MAX_ENTRIES + 19).to_string()));
        assert_eq!(log.events.first().map(|e| e.time.clone()), Some("20".to_owned()));
    }

    #[test]
    fn pages_follow_the_page_size() {
        let mut log = ExperienceLog::default();
        assert_eq!(log.pages(), 1);
        for n in 0..51 {
            log.record(&event(&n.to_string(), 4), "2026-05-01", "10:00:00");
        }
        assert_eq!(log.pages(), 3);
    }

    #[test]
    fn the_log_and_its_settings_survive_a_restart() {
        let (dir, path) = temp();
        let mut log = ExperienceLog::load(path.clone(), "2026-05-01");
        log.notify = true;
        log.record(&event("Door", 11), "2026-05-01", "10:00:00");
        // the writer finishes its last write when the log goes
        drop(log);
        let loaded = ExperienceLog::load(path, "2026-05-02");
        assert_eq!((loaded.events.len(), loaded.notify), (1, true));
        drop(loaded);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn with_no_days_kept_nothing_is_read_back() {
        let (dir, path) = temp();
        let mut log = ExperienceLog::load(path.clone(), "2026-05-01");
        log.record(&event("Door", 11), "2026-05-01", "10:00:00");
        log.set_max_days(0, "2026-05-01");
        assert_eq!(log.events.len(), 1);
        drop(log);
        let loaded = ExperienceLog::load(path, "2026-05-01");
        assert_eq!((loaded.events.len(), loaded.max_days), (0, 0));
        drop(loaded);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_file_is_kept_aside_not_lost() {
        let (dir, path) = temp();
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(&path, b"not json").expect("write");
        let mut log = ExperienceLog::load(path.clone(), "2026-05-01");
        assert!(log.events.is_empty());
        log.record(&event("Door", 11), "2026-05-01", "10:00:00");
        drop(log);
        let kept = std::fs::read(crate::tolerant::backup_path(&path)).expect("the unreadable file is kept");
        assert_eq!(kept, b"not json");
        let _ = std::fs::remove_dir_all(dir);
    }
}
