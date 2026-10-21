//! History of the notification center: every notification read, answered or
//! closed, with what became of it, so that one acknowledged by mistake can be
//! found again. Kept per account between sessions (`notification_history.json`
//! in the account folder, 30 days and 500 entries at most), read and written
//! on a worker thread with the forward-compatible reading of `tolerant`, never
//! sent anywhere. Aurora's own: Firestorm's notification well forgets what was
//! closed.

use crossbeam_channel::{self as mpsc, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub const FILE: &str = "notification_history.json";
/// Entries kept: the newest 500, none older than 30 days.
pub const MAX_ENTRIES: usize = 500;
pub const MAX_AGE_SECS: u64 = 30 * 24 * 3600;

/// What the card looked like (its icon and accent in the list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Look {
    Alert,
    Teleport,
    Friendship,
    Inventory,
    Group,
    Script,
    Permissions,
    Url,
    Message,
    Call,
    /// Information, and a kind a newer version wrote.
    #[serde(other)]
    Info,
}

/// What became of the notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Accepted, allowed, a dialog's button, a reply sent.
    Answered,
    /// Declined, refused, blocked.
    Declined,
    /// An inventory offer kept.
    Kept,
    /// Closed without an answer (a script menu ignored).
    Ignored,
    /// Information read (or "Mark all read").
    #[serde(other)]
    Read,
}

/// The useful action left once a notification is gone: an offer refused or
/// expired cannot be accepted any more (the server decides), but what it was
/// about can still be opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Follow {
    #[default]
    None,
    /// Open the conversation of this session (a message, a resident's offer).
    Conversation(Uuid),
    /// Show this place (a teleport offer's SLURL).
    Place(String),
    /// Open this item or folder in the inventory (an inventory offer kept).
    Inventory(Uuid),
    /// Information that can be put back in "To do".
    Restore,
}

/// Identity behind a notification's visible name, retained in its history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameTarget {
    Avatar(Uuid),
    Group(Uuid),
    Experience(Uuid),
    Object(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameLink {
    pub name: String,
    pub target: NameTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// When it arrived (seconds since the epoch).
    pub time: u64,
    #[serde(default = "info")]
    pub look: Look,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_links: Vec<NameLink>,
    #[serde(default = "read")]
    pub outcome: Outcome,
    #[serde(default)]
    pub follow: Follow,
}

fn info() -> Look {
    Look::Info
}

fn read() -> Outcome {
    Outcome::Read
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    entries: Vec<Entry>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

struct Storage {
    loaded: Receiver<Vec<Entry>>,
    writer: Option<Sender<Vec<Entry>>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Storage {
    fn drop(&mut self) {
        // the last write is published before the account's session ends
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Newest first.
#[derive(Default)]
pub struct History {
    entries: VecDeque<Entry>,
    storage: Option<Storage>,
    /// Entries added before the file was read, merged into it once read.
    early: Vec<Entry>,
    /// The worker has delivered the file.
    read: bool,
}

impl History {
    /// The account's history, read on a worker; the demo keeps it in memory
    /// (`Default`).
    pub fn load(path: PathBuf) -> Self {
        let (loaded_tx, loaded) = mpsc::bounded(1);
        let (writer, writes) = mpsc::unbounded::<Vec<Entry>>();
        let worker = std::thread::Builder::new()
            .name("aurora-notification-history".into())
            .spawn(move || {
                let entries = match std::fs::read(&path) {
                    Ok(bytes) => crate::tolerant::from_slice::<Saved>(&bytes, FILE)
                        .map(|r| r.value.entries)
                        .unwrap_or_default(),
                    Err(error) => {
                        if error.kind() != std::io::ErrorKind::NotFound {
                            log::warn!("Cannot read the notification history: {error}");
                        }
                        Vec::new()
                    }
                };
                let _ = loaded_tx.send(entries);
                while let Ok(mut entries) = writes.recv() {
                    // only the latest state matters
                    while let Ok(newer) = writes.try_recv() {
                        entries = newer;
                    }
                    let save = || -> std::io::Result<()> {
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        let bytes = serde_json::to_vec(&Saved { entries }).map_err(std::io::Error::other)?;
                        crate::atomic_file::write(&path, &bytes)
                    };
                    if let Err(error) = save() {
                        log::warn!("Cannot save the notification history: {error}");
                    }
                }
            });
        match worker {
            Ok(worker) => History {
                storage: Some(Storage {
                    loaded,
                    writer: Some(writer),
                    worker: Some(worker),
                }),
                ..History::default()
            },
            Err(error) => {
                log::warn!("Cannot start the notification history: {error}");
                History::default()
            }
        }
    }

    /// Take the file's entries once the worker has read them.
    pub fn poll(&mut self) {
        let Some(read) = self.storage.as_ref().and_then(|s| s.loaded.try_recv().ok()) else {
            return;
        };
        self.read = true;
        self.entries = read.into();
        let early = std::mem::take(&mut self.early);
        for e in early.into_iter().rev() {
            self.entries.push_front(e);
        }
        self.prune(now_secs());
        if !self.entries.is_empty() {
            self.save();
        }
    }

    /// The worker has not delivered the file yet.
    fn loading(&self) -> bool {
        self.storage.is_some() && !self.read
    }

    pub fn add(&mut self, entry: Entry) {
        if self.loading() {
            self.early.insert(0, entry);
            return;
        }
        self.entries.push_front(entry);
        self.prune(now_secs());
        self.save();
    }

    /// Put an entry back (information acknowledged by mistake): it leaves the
    /// history and is returned to become a notification again.
    pub fn take(&mut self, index: usize) -> Option<Entry> {
        // the order of `entries`: the early ones first
        if index < self.early.len() {
            return Some(self.early.remove(index));
        }
        let e = self.entries.remove(index - self.early.len())?;
        self.save();
        Some(e)
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.early.iter().chain(self.entries.iter())
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len() + self.early.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 500 entries at most, none older than 30 days.
    fn prune(&mut self, now: u64) {
        self.entries.retain(|e| now.saturating_sub(e.time) <= MAX_AGE_SECS);
        self.entries.truncate(MAX_ENTRIES);
    }

    fn save(&self) {
        if let Some(writer) = self.storage.as_ref().and_then(|s| s.writer.as_ref()) {
            let _ = writer.send(self.entries.iter().cloned().collect());
        }
    }
}

/// Entries matching a search (title or body, case-insensitive), with their
/// index for `History::take`.
pub fn search<'a>(history: &'a History, query: &str) -> Vec<(usize, &'a Entry)> {
    let q = query.trim().to_lowercase();
    history
        .entries()
        .enumerate()
        .filter(|(_, e)| q.is_empty() || e.title.to_lowercase().contains(&q) || e.body.to_lowercase().contains(&q))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(time: u64, title: &str) -> Entry {
        Entry {
            time,
            look: Look::Info,
            title: title.into(),
            body: String::new(),
            name_links: Vec::new(),
            outcome: Outcome::Read,
            follow: Follow::Restore,
        }
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("aurora-notif-history-{}", Uuid::new_v4()))
            .join(name)
    }

    /// Wait for the worker to deliver the file.
    fn loaded(path: PathBuf) -> History {
        let mut h = History::load(path);
        for _ in 0..500 {
            h.poll();
            if h.read {
                return h;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("history not read");
    }

    #[test]
    fn entries_survive_a_new_session_newest_first() {
        let path = temp(FILE);
        {
            let mut h = loaded(path.clone());
            h.add(entry(now_secs() - 10, "older"));
            h.add(entry(now_secs(), "newer"));
        } // dropping the history publishes the last write
        let h = loaded(path.clone());
        assert_eq!(h.entries().map(|e| e.title.as_str()).collect::<Vec<_>>(), vec!["newer", "older"]);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn entries_added_before_the_file_is_read_are_kept_on_top() {
        let path = temp(FILE);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        let saved = Saved {
            entries: vec![entry(now_secs() - 60, "from disk")],
        };
        std::fs::write(&path, serde_json::to_vec(&saved).expect("json")).expect("write");
        let mut h = History::load(path.clone());
        h.add(entry(now_secs(), "early"));
        for _ in 0..500 {
            h.poll();
            if h.read {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            h.entries().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["early", "from disk"]
        );
        drop(h);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn the_history_keeps_500_entries_of_the_last_30_days() {
        let mut h = History::default();
        let now = now_secs();
        h.add(entry(now - MAX_AGE_SECS - 5, "too old"));
        assert!(h.is_empty());
        for i in 0..(MAX_ENTRIES + 20) {
            h.add(entry(now, &format!("n{i}")));
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        assert_eq!(
            h.entries().next().map(|e| e.title.as_str()),
            Some(format!("n{}", MAX_ENTRIES + 19).as_str())
        );
    }

    #[test]
    fn an_entry_can_be_taken_back_and_searched() {
        let mut h = History::default();
        h.add(entry(now_secs(), "Group notice: meeting"));
        h.add(entry(now_secs(), "Tess offers a teleport"));
        let found = search(&h, "MEETING");
        assert_eq!(found.len(), 1);
        let (i, _) = found[0];
        let back = h.take(i).expect("entry");
        assert_eq!(back.title, "Group notice: meeting");
        assert_eq!(h.len(), 1);
        assert_eq!(search(&h, "").len(), 1);
    }

    /// A file of a newer build: an unknown look or outcome falls back, an
    /// unknown field is ignored, the entries stay.
    #[test]
    fn a_newer_file_keeps_its_entries() {
        let json = br#"{"entries":[{"time":5,"look":"hologram","title":"t","outcome":"archived","follow":{"teleport_to":"x"},"extra":1}],"version":9}"#;
        let read = crate::tolerant::from_slice::<Saved>(json, FILE).expect("read");
        let e = &read.value.entries[0];
        assert_eq!((e.look, e.outcome, &e.title), (Look::Info, Outcome::Read, &"t".to_string()));
        assert_eq!(e.follow, Follow::None);
    }
}
