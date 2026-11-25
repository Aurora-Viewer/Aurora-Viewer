//! Recent participants and open tabs, scoped to one account and grid.
//! Unlike LLRecentPeople and FSFloaterIMContainer::saveOpenIMs, publish during
//! use, not just logout. One bounded mailbox coalesces snapshots; the worker
//! loads, serializes and atomically flushes them without blocking frames.

use super::{World, open_ims::Conversation, recent::Met};
use crossbeam_channel::{Receiver, Sender, TrySendError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use uuid::Uuid;

const FILE: &str = "social_history.json";
const TABS_FILE: &str = "open_tabs.json";
const ACTIVITY_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// New residents and tab changes have a short crash-recovery window.
const SAVE_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Saved {
    version: u32,
    recent: Vec<Met>,
    conversations: Vec<Conversation>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            version: 1,
            recent: Vec::new(),
            conversations: Vec::new(),
        }
    }
}

enum Job {
    Save(Saved),
    Sync(Sender<bool>),
}

/// The bounded, newest-snapshot queue follows world/rlva/login.rs::Writer.
/// Load/poll follows notification_history.rs; flush follows chat_log/store.rs.
struct Storage {
    loaded: Receiver<io::Result<Saved>>,
    writer: Option<Sender<Job>>,
    discard: Receiver<Job>,
    done: Receiver<()>,
    cancelled: Arc<AtomicBool>,
}

impl Storage {
    fn save(&self, saved: Saved) {
        let Some(writer) = &self.writer else { return };
        let mut job = Job::Save(saved);
        loop {
            match writer.try_send(job) {
                Ok(()) | Err(TrySendError::Disconnected(_)) => return,
                Err(TrySendError::Full(latest)) => {
                    job = latest;
                    if let Ok(Job::Sync(done)) = self.discard.try_recv() {
                        let _ = done.send(false);
                    }
                }
            }
        }
    }

    fn flush(&self, timeout: Duration) -> bool {
        let Some(writer) = &self.writer else { return true };
        let (tx, rx) = crossbeam_channel::bounded(1);
        writer.send_timeout(Job::Sync(tx), timeout).is_ok() && rx.recv_timeout(timeout).unwrap_or(false)
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.writer.take();
        // Same bounded logout wait as tphistory.rs: a stuck disk must not
        // hold the viewer open indefinitely.
        if self.done.recv_timeout(Duration::from_millis(500)).is_err() {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

#[derive(Default)]
pub struct History {
    storage: Option<Storage>,
    loading: bool,
    closed: HashSet<Uuid>,
    recent_revision: u64,
    membership_revision: u64,
    profile: Option<(PathBuf, Uuid, String)>,
    log_recent: bool,
    restore_tabs: bool,
    conversations: Vec<Conversation>,
    ready: bool,
    save_due: Option<Instant>,
}

/// Exact custom login URIs have separate keys, including URIs that differ only
/// in punctuation. This deliberately differs from chat_log::grid_name and
/// its transcript-folder naming. No URI is written in the path or logged.
pub fn grid_key(grid: &crate::settings::GridChoice, uri: &str) -> String {
    match grid {
        crate::settings::GridChoice::SecondLife => "agni".into(),
        crate::settings::GridChoice::SecondLifeBeta => "aditi".into(),
        crate::settings::GridChoice::Custom => {
            use sha2::{Digest, Sha256};
            let digest: String = Sha256::digest(uri.trim().as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("custom-{digest}")
        }
    }
}

fn path(accounts: &Path, agent: Uuid, grid: &str) -> PathBuf {
    accounts.join(agent.to_string()).join("social").join(grid).join(FILE)
}

fn read_fields(path: &Path, include_recent: bool) -> io::Result<Option<Saved>> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("social history exceeds size limit"));
    }
    crate::disk_io::read(crate::disk_io::Origin::Other, bytes.len());
    #[derive(Deserialize)]
    struct Tabs {
        version: u32,
        conversations: Vec<Conversation>,
    }
    let mut saved: Saved = if include_recent {
        serde_json::from_slice(&bytes)
    } else {
        serde_json::from_slice::<Tabs>(&bytes).map(|tabs| Saved {
            version: tabs.version,
            conversations: tabs.conversations,
            recent: Vec::new(),
        })
    }
    .map_err(|_| io::Error::other("invalid social history"))?;
    if saved.version != 1 {
        return Err(io::Error::other("unsupported social history version"));
    }
    // Normalize older, uncapped files on the reader thread too. The frame
    // thread receives at most MAX_PEOPLE entries to merge with early activity.
    let mut recent = super::recent::RecentPeople::default();
    recent.merge(Uuid::nil(), std::mem::take(&mut saved.recent));
    saved.recent = recent.list().to_vec();
    Ok(Some(saved))
}

fn write_if(path: &Path, saved: &Saved, still: impl FnOnce() -> bool) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(saved).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("social history exceeds size limit"));
    }
    if crate::atomic_file::write_if(path, &bytes, still)? {
        crate::disk_io::written(crate::disk_io::Origin::Other, bytes.len());
        Ok(())
    } else {
        Err(io::Error::other("session ended"))
    }
}

/// The sidecar always owns tab recovery. Before the split, development builds
/// kept tabs in the recent file. Read only its tab fields when logging is off.
fn read_profile(path: &Path, log_recent: bool, legacy: Option<&Path>) -> io::Result<Saved> {
    let tabs = read_fields(&path.with_file_name(TABS_FILE), false)?;
    let mut saved = if log_recent || tabs.is_none() {
        read_fields(path, log_recent)?
    } else {
        None
    }
    .unwrap_or_else(|| Saved {
        conversations: legacy.map(super::open_ims::load).unwrap_or_default(),
        ..Saved::default()
    });
    if let Some(tabs) = tabs {
        saved.conversations = tabs.conversations;
    }
    Ok(saved)
}

#[cfg(test)]
fn read(path: &Path) -> io::Result<Option<Saved>> {
    if !path.exists() && !path.with_file_name(TABS_FILE).exists() {
        return Ok(None);
    }
    read_profile(path, true, None).map(Some)
}

fn run(
    loaded: Sender<io::Result<Saved>>,
    writes: Receiver<Job>,
    path: PathBuf,
    legacy: Option<PathBuf>,
    cancelled: Arc<AtomicBool>,
    log_recent: bool,
) {
    let saved = read_profile(&path, log_recent, legacy.as_deref());
    let mut last_recent = saved.as_ref().ok().map(|s| s.recent.clone()).unwrap_or_default();
    let mut last_tabs = None;
    let writable = saved.is_ok();
    let _ = loaded.send(saved);
    if !writable {
        return;
    }
    let mut retry: Option<Saved> = None;
    let mut next = None;
    loop {
        let job = next.take().or_else(|| {
            if retry.is_some() {
                match writes.recv_timeout(Duration::from_secs(5)) {
                    Ok(job) => Some(job),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => retry.take().map(Job::Save),
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => None,
                }
            } else {
                writes.recv().ok()
            }
        });
        let Some(job) = job else { break };
        match job {
            Job::Save(mut saved) => {
                while let Ok(newer) = writes.try_recv() {
                    match newer {
                        Job::Save(newer) => saved = newer,
                        other => {
                            next = Some(other);
                            break;
                        }
                    }
                }
                let still = || !cancelled.load(Ordering::Acquire);
                let mut publish = || -> io::Result<()> {
                    if last_tabs.as_ref() != Some(&saved.conversations) {
                        write_if(
                            &path.with_file_name(TABS_FILE),
                            &Saved {
                                conversations: saved.conversations.clone(),
                                ..Saved::default()
                            },
                            still,
                        )?;
                        last_tabs = Some(saved.conversations.clone());
                    }
                    if log_recent && last_recent != saved.recent {
                        write_if(
                            &path,
                            &Saved {
                                recent: saved.recent.clone(),
                                ..Saved::default()
                            },
                            still,
                        )?;
                        last_recent.clone_from(&saved.recent);
                    }
                    Ok(())
                };
                match publish() {
                    Ok(()) => retry = None,
                    Err(_) => {
                        if retry.is_none() {
                            log::warn!("Social history could not be saved; retaining the previous file and retrying");
                        }
                        retry = Some(saved);
                    }
                }
            }
            Job::Sync(done) => {
                let _ = done.send(retry.is_none());
            }
        }
    }
}

impl History {
    pub fn load(accounts: &Path, agent: Uuid, grid: &str, log_recent: bool) -> Self {
        if agent.is_nil() || grid.is_empty() {
            return Self::default();
        }
        let (loaded_tx, loaded) = crossbeam_channel::bounded(1);
        let (writer, writes) = crossbeam_channel::bounded(1);
        let discard = writes.clone();
        let (finished, done) = crossbeam_channel::bounded(1);
        let file = path(accounts, agent, grid);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let legacy = (grid == "agni").then(|| accounts.join(agent.to_string()).join(super::open_ims::FILE));
        match std::thread::Builder::new().name("aurora-social-history".into()).spawn(move || {
            run(loaded_tx, writes, file, legacy, worker_cancelled, log_recent);
            let _ = finished.send(());
        }) {
            Ok(_) => Self {
                storage: Some(Storage {
                    loaded,
                    writer: Some(writer),
                    discard,
                    done,
                    cancelled,
                }),
                loading: true,
                log_recent,
                restore_tabs: true,
                profile: Some((accounts.to_owned(), agent, grid.to_owned())),
                ..Self::default()
            },
            Err(_) => {
                log::warn!("Social history worker could not start; keeping this session in memory");
                Self::default()
            }
        }
    }

    pub fn cancel_saved_im(&mut self, id: Uuid) {
        if self.loading {
            self.closed.insert(id);
        }
    }

    fn take_loaded(&mut self, timeout: Duration) -> Option<Saved> {
        if !self.loading {
            return None;
        }
        let result = self.storage.as_ref()?.loaded.recv_timeout(timeout).ok()?;
        self.loading = false;
        match result {
            Ok(mut saved) => {
                self.conversations.clone_from(&saved.conversations);
                saved.conversations.retain(|c| !c.id().is_nil() && !self.closed.contains(&c.id()));
                self.closed.clear();
                self.ready = true;
                self.recent_revision = u64::MAX;
                Some(saved)
            }
            Err(_) => {
                log::warn!("Social history could not be read; the existing file will not be overwritten");
                None
            }
        }
    }

    fn save(&mut self, recent: &super::recent::RecentPeople, conversations: Vec<Conversation>) {
        if !self.ready || (self.recent_revision == recent.revision && self.conversations == conversations) {
            return;
        }
        self.recent_revision = recent.revision;
        self.membership_revision = recent.membership_revision;
        self.conversations = conversations.clone();
        if let Some(storage) = &self.storage {
            storage.save(Saved {
                recent: if self.log_recent { recent.list().to_vec() } else { Vec::new() },
                conversations,
                ..Saved::default()
            });
        }
    }

    pub fn flush(&self, timeout: Duration) -> bool {
        self.storage.as_ref().is_none_or(|s| s.flush(timeout))
    }
}

impl World {
    /// Switch per-profile storage before processing the next frame's interactions.
    /// The last enabled snapshot survives an opt-out. A fresh enable starts
    /// from that history, so encounters during the disabled period stay private.
    pub fn sync_recent_logging(&mut self) {
        let enabled = self.chat_log.cfg.log_conversations;
        if self.social_history.log_recent == enabled {
            return;
        }
        let Some((accounts, agent, grid)) = self.social_history.profile.clone() else {
            return;
        };
        self.flush_social_history();
        drop(std::mem::take(&mut self.social_history));
        if enabled {
            self.recent = Default::default();
        }
        self.social_history = History::load(&accounts, agent, &grid, enabled);
        self.social_history.restore_tabs = false;
    }

    pub fn poll_social_history(&mut self) {
        self.apply_social_history(Duration::ZERO);
    }

    /// A fast logout may precede the initial read. Give that read a bounded
    /// chance to merge before publishing the final session state.
    pub fn flush_social_history(&mut self) {
        self.apply_social_history(Duration::from_millis(500));
        self.save_social_history_at(Instant::now(), true);
        self.social_history.flush(Duration::from_millis(500));
    }

    fn apply_social_history(&mut self, timeout: Duration) {
        if let Some(saved) = self.social_history.take_loaded(timeout) {
            if self.social_history.log_recent {
                self.recent.merge(self.agent_id, saved.recent);
            }
            if !self.social_history.restore_tabs {
                return;
            }
            for conversation in saved.conversations {
                if conversation.id() != self.agent_id && !self.reopen_ims.iter().any(|c| c.id() == conversation.id()) {
                    self.reopen_ims.push(conversation);
                }
            }
        }
    }

    /// Called during use and before logout. The reopen preference controls
    /// restoration only; recording the currently open tabs remains unconditional.
    pub fn save_social_history(&mut self) {
        self.save_social_history_at(Instant::now(), false);
    }

    fn save_social_history_at(&mut self, now: Instant, force: bool) {
        if !self.social_history.ready {
            return;
        }
        let tabs_changed = !self.open_conversations().eq(self.social_history.conversations.iter().copied());
        let recent_changed = self.social_history.log_recent && self.social_history.recent_revision != self.recent.revision;
        if !tabs_changed && !recent_changed {
            self.social_history.save_due = None;
            return;
        }
        let interval = if tabs_changed || self.social_history.membership_revision != self.recent.membership_revision {
            SAVE_INTERVAL
        } else {
            ACTIVITY_INTERVAL
        };
        let deadline = now + interval;
        let due = self.social_history.save_due.map_or(deadline, |old| old.min(deadline));
        self.social_history.save_due = Some(due);
        if !force && now < due {
            return;
        }
        self.social_history.save_due = None;
        let conversations = self.open_conversations().collect();
        self.social_history.save(&self.recent, conversations);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Instant, SystemTime};

    fn write(path: &Path, saved: &Saved) -> io::Result<()> {
        write_if(path, saved, || true)
    }

    fn save_during_use(w: &mut World) {
        let now = Instant::now();
        w.save_social_history_at(now, false);
        w.save_social_history_at(now + SAVE_INTERVAL, false);
    }

    fn world() -> World {
        World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("aurora-social-history-{}", Uuid::new_v4()))
    }

    fn loaded(w: &mut World) {
        let until = Instant::now() + Duration::from_secs(10);
        while w.social_history.loading {
            w.poll_social_history();
            assert!(Instant::now() < until, "storage load timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn start(root: &Path, agent: Uuid, grid: &str) -> World {
        let mut w = world();
        w.agent_id = agent;
        w.social_history = History::load(root, agent, grid, true);
        loaded(&mut w);
        w
    }

    #[test]
    fn social_history_saves_during_use_restores_tabs_and_preserves_reopen_preference() {
        let root = root();
        let me = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let mut w = start(&root, me, "agni");
        w.recent.add(me, other, false, "", when);
        w.social.session_mut(other);
        save_during_use(&mut w);
        assert!(w.social_history.flush(Duration::from_secs(10)));
        // Read while the first session is still alive: no logout/drop saved it.
        let mut reopened = start(&root, me, "agni");
        assert_eq!(reopened.recent.list(), w.recent.list());
        reopened.restore_open_ims(true);
        assert_eq!(reopened.social.ims[0].other, other);
        reopened.social.ims.clear();
        reopened.cancel_saved_im(other);
        save_during_use(&mut reopened);
        assert!(reopened.social_history.flush(Duration::from_secs(10)));
        let mut closed = start(&root, me, "agni");
        closed.restore_open_ims(true);
        assert!(closed.social.ims.is_empty());
        assert_eq!(closed.recent.list()[0].when, when);
        // Disabling automatic reopening never disables recent history.
        w.social.session_mut(Uuid::from_u128(3));
        save_during_use(&mut w);
        assert!(w.social_history.flush(Duration::from_secs(10)));
        let mut disabled = start(&root, me, "agni");
        disabled.restore_open_ims(false);
        save_during_use(&mut disabled);
        assert!(disabled.social_history.flush(Duration::from_secs(10)));
        assert!(disabled.social.ims.is_empty());
        assert!(!disabled.recent.is_empty());
        drop((w, reopened, closed, disabled));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn social_history_accounts_grids_and_legacy_migration_are_isolated() {
        let root = root();
        let (me, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        super::super::open_ims::save(
            &root.join(me.to_string()).join(super::super::open_ims::FILE),
            &[Conversation::Resident(other)],
        );
        let mut agni = start(&root, me, "agni");
        assert_eq!(agni.reopen_ims, [Conversation::Resident(other)]);
        agni.recent.add(me, other, false, "", SystemTime::UNIX_EPOCH);
        save_during_use(&mut agni);
        assert!(agni.social_history.flush(Duration::from_secs(10)));
        for (agent, grid) in [(me, "aditi"), (other, "agni"), (me, "custom-one")] {
            let w = start(&root, agent, grid);
            assert!(w.reopen_ims.is_empty() && w.recent.is_empty());
        }
        use crate::settings::GridChoice;
        assert_ne!(
            grid_key(&GridChoice::Custom, "https://example.invalid/a-b"),
            grid_key(&GridChoice::Custom, "https://example.invalid/a_b")
        );
        drop(agni);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn social_history_merges_early_activity_and_honors_a_close_during_load() {
        let root = root();
        let (me, a, b) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        write(
            &path(&root, me, "agni"),
            &Saved {
                recent: vec![Met {
                    id: a,
                    when: old,
                    region: String::new(),
                }],
                conversations: vec![Conversation::Resident(a)],
                ..Saved::default()
            },
        )
        .unwrap();
        let mut w = world();
        w.agent_id = me;
        w.social_history = History::load(&root, me, "agni", true);
        w.recent.add(me, a, false, "", old + Duration::from_secs(10));
        w.recent.add(me, b, false, "", old + Duration::from_secs(20));
        w.cancel_saved_im(a);
        loaded(&mut w);
        assert!(w.reopen_ims.is_empty());
        assert_eq!(w.recent.list().iter().map(|m| m.id).collect::<Vec<_>>(), [b, a]);
        save_during_use(&mut w);
        assert!(w.social_history.flush(Duration::from_secs(10)));
        assert_eq!(read(&path(&root, me, "agni")).unwrap().unwrap().recent, w.recent.list());
        drop(w);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn social_history_never_overwrites_unreadable_or_newer_formats() {
        for bytes in [b"broken".as_slice(), br#"{"version":2,"recent":[],"conversations":[]}"#] {
            let root = root();
            let me = Uuid::from_u128(1);
            let p = path(&root, me, "agni");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, bytes).unwrap();
            let mut w = start(&root, me, "agni");
            w.recent.add(me, Uuid::from_u128(2), false, "", SystemTime::UNIX_EPOCH);
            save_during_use(&mut w);
            drop(w);
            assert_eq!(std::fs::read(p).unwrap(), bytes);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn social_history_flush_merges_a_fast_logout_and_cancelled_writes_keep_the_last_file() {
        let root = root();
        let (me, a, b) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let file = path(&root, me, "agni");
        let previous = Saved {
            recent: vec![Met {
                id: a,
                when: SystemTime::UNIX_EPOCH,
                region: String::new(),
            }],
            conversations: vec![Conversation::Resident(a)],
            ..Saved::default()
        };
        write(&file, &previous).unwrap();
        let mut w = world();
        w.agent_id = me;
        w.social_history = History::load(&root, me, "agni", true);
        w.recent.add(me, b, false, "", SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        w.social.session_mut(b);
        w.flush_social_history();
        let saved = read(&file).unwrap().unwrap();
        assert_eq!(saved.recent.len(), 2);
        assert_eq!(saved.conversations, [Conversation::Resident(b), Conversation::Resident(a)]);
        assert!(write_if(&file, &Saved::default(), || false).is_err());
        assert_eq!(read(&file).unwrap().unwrap().recent, saved.recent);
        assert_eq!(std::fs::read_dir(file.parent().unwrap()).unwrap().count(), 2);
        drop(w);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn social_history_coalesces_busy_chat_and_flushes_the_last_change() {
        let root = root();
        let (me, peer) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut w = start(&root, me, "agni");
        let base = Instant::now();
        let when = SystemTime::UNIX_EPOCH;
        let file = path(&root, me, "agni");
        let mut publications = 0;
        for frame in 0..600 {
            w.recent.add(me, peer, false, "", when + Duration::from_millis(frame * 16));
            let before = w.social_history.recent_revision;
            w.save_social_history_at(base + Duration::from_millis(frame * 16), false);
            publications += usize::from(w.social_history.recent_revision != before);
            if frame < 125 {
                assert!(!file.exists(), "no per-line snapshot before the coalescing deadline");
            }
        }
        assert_eq!(publications, 1, "only the new resident needs a checkpoint in the first 9.6 seconds");
        let before = w.social_history.recent_revision;
        w.save_social_history_at(base + Duration::from_secs(29), false);
        assert_eq!(w.social_history.recent_revision, before);
        w.save_social_history_at(base + Duration::from_secs(33), false);
        assert_eq!(w.social_history.recent_revision, w.recent.revision);
        w.social.session_mut(peer);
        w.flush_social_history();
        let saved = read(&file).unwrap().unwrap();
        assert_eq!(saved.recent, w.recent.list());
        assert_eq!(saved.conversations, [Conversation::Resident(peer)]);
        drop(w);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn social_history_bounds_an_existing_uncapped_file_before_publication() {
        let root = root();
        let me = Uuid::from_u128(1);
        let file = path(&root, me, "agni");
        let saved = Saved {
            recent: (2..super::super::recent::MAX_PEOPLE + 102)
                .map(|n| Met {
                    id: Uuid::from_u128(n as u128),
                    when: SystemTime::UNIX_EPOCH + Duration::from_secs(n as u64),
                    region: String::new(),
                })
                .collect(),
            ..Saved::default()
        };
        write(&file, &saved).unwrap();
        let restored = read(&file).unwrap().unwrap();
        assert_eq!(restored.recent.len(), super::super::recent::MAX_PEOPLE);
        assert_eq!(restored.recent[0].id, saved.recent.last().unwrap().id);
        assert_eq!(restored.recent.last().unwrap().id, saved.recent[100].id);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_changed_tab_shortens_an_existing_activity_deadline() {
        let root = root();
        let me = Uuid::from_u128(1);
        let peer = Uuid::from_u128(2);
        let mut w = start(&root, me, "agni");
        let base = Instant::now();
        w.recent.add(me, peer, false, "", SystemTime::UNIX_EPOCH);
        w.flush_social_history();
        w.recent.add(me, peer, false, "", SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        w.save_social_history_at(base, false);
        w.social.session_mut(peer);
        w.save_social_history_at(base + Duration::from_secs(5), false);
        w.save_social_history_at(base + Duration::from_secs(7), false);
        assert!(w.social_history.flush(Duration::from_secs(10)));
        assert_eq!(
            read(&path(&root, me, "agni")).unwrap().unwrap().conversations,
            [Conversation::Resident(peer)]
        );
        drop(w);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn logging_off_preserves_old_history_and_only_recovers_tabs() {
        let root = root();
        let me = Uuid::from_u128(1);
        let old = Uuid::from_u128(2);
        let private = Uuid::from_u128(3);
        let tab = Uuid::from_u128(4);
        let file = path(&root, me, "agni");
        let mut w = start(&root, me, "agni");
        w.recent.add(me, old, false, "saved", SystemTime::UNIX_EPOCH);
        w.flush_social_history();
        let original = std::fs::read(&file).unwrap();
        w.chat_log.cfg.log_conversations = false;
        w.sync_recent_logging();
        loaded(&mut w);
        w.recent.add(me, private, false, "private", SystemTime::now());
        w.social.session_mut(tab);
        w.flush_social_history();
        drop(w);
        assert_eq!(std::fs::read(&file).unwrap(), original);
        let mut w = world();
        w.agent_id = me;
        w.chat_log.cfg.log_conversations = false;
        w.social_history = History::load(&root, me, "agni", false);
        loaded(&mut w);
        assert!(w.recent.is_empty());
        w.restore_open_ims(true);
        assert_eq!(w.social.ims[0].other, tab);
        w.recent.add(me, private, false, "private", SystemTime::now());
        // Reopening the private tab legitimately wants its display name.
        // Start measuring after that independent operation.
        let asks = w.social.avatar_names.take_asks();
        assert!(!asks.contains(&old), "opted-out login must not look up a saved contact");
        w.chat_log.cfg.log_conversations = true;
        w.sync_recent_logging();
        loaded(&mut w);
        assert_eq!(w.recent.list().iter().map(|m| m.id).collect::<Vec<_>>(), [old]);
        assert!(
            w.social.avatar_names.take_asks().is_empty(),
            "restoring Recent cannot queue name lookups"
        );
        w.flush_social_history();
        assert_eq!(std::fs::read(&file).unwrap(), original);
        drop(w);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn social_history_crash_child() {
        let Some(root) = std::env::var_os("AURORA_SOCIAL_TEST_CHILD") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut w = start(&root, Uuid::from_u128(1), "agni");
        w.recent.add(w.agent_id, Uuid::from_u128(2), false, "", SystemTime::UNIX_EPOCH);
        w.social.session_mut(Uuid::from_u128(2));
        save_during_use(&mut w);
        assert!(w.social_history.flush(Duration::from_secs(10)));
        std::fs::write(root.join("ready"), b"ready").unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn social_history_survives_a_killed_process_without_logout() {
        let root = root();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "world::social_history::tests::social_history_crash_child", "--nocapture"])
            .env("AURORA_SOCIAL_TEST_CHILD", &root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(30);
        while !root.join("ready").exists() && Instant::now() < until && child.try_wait().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = root.join("ready").exists();
        let _ = child.kill();
        let _ = child.wait();
        assert!(ready, "child did not save during use");
        let mut w = start(&root, Uuid::from_u128(1), "agni");
        assert_eq!(w.recent.list()[0].id, Uuid::from_u128(2));
        w.restore_open_ims(true);
        assert_eq!(w.social.ims[0].other, Uuid::from_u128(2));
        drop(w);
        std::fs::remove_dir_all(root).unwrap();
    }
}
