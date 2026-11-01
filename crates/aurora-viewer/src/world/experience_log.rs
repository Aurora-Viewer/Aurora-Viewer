//! The experience event log: what objects did with the permissions the agent
//! gave to experiences. Port of LLExperienceLog (indra/newview/
//! llexperiencelog.cpp, originally LGPL 2.1): an event equal to the previous
//! one only counts it again, days older than `max_days` are dropped, and the
//! log and its three settings live in one file per account.

use aurora_net::ExperienceLogEvent;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const FILE: &str = "experience_events.json";

/// LLExperienceLog::LLExperienceLog.
const DEFAULT_MAX_DAYS: u32 = 7;
const DEFAULT_PAGE_SIZE: usize = 25;

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

#[derive(Debug)]
pub struct ExperienceLog {
    /// Oldest first, as they arrived.
    pub events: Vec<LogEntry>,
    pub max_days: u32,
    /// "Notify all events": say each new event in the chat.
    pub notify: bool,
    pub page_size: usize,
    path: Option<PathBuf>,
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
            path: None,
        }
    }
}

impl ExperienceLog {
    /// The account's log (LLExperienceLog::loadEvents); `today` drops the
    /// days that expired.
    pub fn load(path: PathBuf, today: &str) -> Self {
        let stored = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Stored>(&bytes).unwrap_or_else(|error| {
                log::warn!("Cannot read the experience log: {error}");
                Stored::default()
            }),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("Cannot read the experience log: {error}");
                }
                Stored::default()
            }
        };
        let mut log = ExperienceLog {
            events: stored.events,
            max_days: stored.max_days,
            notify: stored.notify,
            page_size: stored.page_size.max(1),
            path: Some(path),
        };
        log.erase_expired(today);
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
            last.count += 1;
            last.time = time.to_owned();
        } else {
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
        }
        self.save();
        self.events.last().expect("an entry was just recorded or counted")
    }

    /// LLExperienceLog::eraseExpired / isNotExpired: the days before
    /// `today - max_days` go (nothing goes when `max_days` is 0).
    pub fn erase_expired(&mut self, today: &str) {
        if self.max_days == 0 {
            return;
        }
        let Ok(today) = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d") else {
            return;
        };
        let boundary = today - chrono::Duration::days(i64::from(self.max_days));
        self.events
            .retain(|e| chrono::NaiveDate::parse_from_str(&e.day, "%Y-%m-%d").is_ok_and(|day| day >= boundary));
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.save();
    }

    pub fn pages(&self) -> usize {
        self.events.len().div_ceil(self.page_size.max(1)).max(1)
    }

    pub fn save(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        write(
            path,
            &Stored {
                events: self.events.clone(),
                max_days: self.max_days,
                notify: self.notify,
                page_size: self.page_size,
            },
        );
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
        for day in ["2026-04-20", "2026-04-24", "2026-04-30", "2026-05-01"] {
            log.record(&event(day, 4), day, "10:00:00");
        }
        log.erase_expired("2026-05-01");
        let days: Vec<_> = log.events.iter().map(|e| e.day.as_str()).collect();
        assert_eq!(days, ["2026-04-24", "2026-04-30", "2026-05-01"]);
        log.max_days = 0;
        log.record(&event("Old", 4), "2000-01-01", "10:00:00");
        log.erase_expired("2026-05-01");
        assert_eq!(log.events.len(), 4);
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
        let dir = std::env::temp_dir().join(format!("aurora-xp-log-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        let mut log = ExperienceLog::load(path.clone(), "2026-05-01");
        log.notify = true;
        log.record(&event("Door", 11), "2026-05-01", "10:00:00");
        let loaded = ExperienceLog::load(path, "2026-05-02");
        assert_eq!((loaded.events.len(), loaded.notify), (1, true));
        let _ = std::fs::remove_dir_all(dir);
    }
}
