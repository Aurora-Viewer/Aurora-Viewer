//! Per-(account, grid) snapshot of the agent's friends list so Aurora can
//! announce when a resident ends a friendship, both live (`TerminateFriendship`)
//! and since the previous login (`buddy-list` diff). Aurora-only addition: the
//! Legacy Viewer does not announce ended friendships.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long an ended-friendship notice waits for its resident's name before
/// falling back to the last saved name (matching `Social::take_online_notices`).
const NAME_WAIT: Duration = Duration::from_secs(10);

/// Writes that failed on disk are kept in memory until retried or exit.
static OWED: crate::save_retry::OwedFiles = crate::save_retry::OwedFiles::new("aurora-known-friends", "known friends snapshot");

/// File name inside `settings::account_dir(&agent_id)`, isolated per grid so
/// Agni and Aditi friend lists never collide for the same avatar UUID.
pub fn file_name(grid: &str) -> String {
    format!("known_friends-{grid}.json")
}

/// Flush any pending write retry when the viewer closes.
pub fn save_before_exit() {
    OWED.finish();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedFriend {
    pub id: Uuid,
    pub name: String,
}

impl Default for SavedFriend {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    friends: Vec<SavedFriend>,
}

#[derive(Debug, Clone)]
struct PendingEnded {
    id: Uuid,
    fallback: String,
    queued_at: Option<Instant>,
}

#[derive(Default)]
pub struct FriendSnapshot {
    path: Option<PathBuf>,
    friends: HashMap<Uuid, String>,
    pending: Vec<PendingEnded>,
}

/// Load the saved friend list from `path`, returning `None` if no valid
/// snapshot exists yet (first run or unreadable file).
pub fn load(path: &Path) -> Option<Vec<SavedFriend>> {
    crate::saves::wait_for(path, crate::saves::BEFORE_READ);
    let bytes = match OWED.owed(path).map_or_else(|| std::fs::read(path), Ok) {
        Ok(bytes) => bytes,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("cannot read known friends snapshot ({e}): {}", path.display());
            }
            return None;
        }
    };
    let parsed: Saved = match serde_json::from_slice(&bytes) {
        Ok(saved) => saved,
        Err(e) => {
            log::warn!("known friends snapshot {} unreadable ({e})", path.display());
            return None;
        }
    };
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(parsed.friends.len());
    for entry in parsed.friends {
        if !entry.id.is_nil() && seen.insert(entry.id) {
            out.push(entry);
        }
    }
    Some(out)
}

impl FriendSnapshot {
    /// Last saved fallback name for `id`, if known.
    pub fn saved_name(&self, id: &Uuid) -> Option<&str> {
        self.friends.get(id).map(String::as_str).filter(|s| !s.is_empty())
    }

    /// Reconcile the login `buddy-list` against `previous`. When `previous` is
    /// `Some` and `notify` is enabled, any friend missing from `current` is
    /// queued for notification and its UUID is returned for name lookup.
    pub fn reconcile(
        &mut self,
        path: Option<PathBuf>,
        previous: Option<Vec<SavedFriend>>,
        current: &[Uuid],
        name_of: impl Fn(&Uuid) -> Option<String>,
        notify: bool,
    ) -> Vec<Uuid> {
        self.path = path;
        self.pending.clear();

        let prev_map: Option<HashMap<Uuid, String>> = previous.map(|list| list.into_iter().map(|f| (f.id, f.name)).collect());

        let mut next_friends = HashMap::with_capacity(current.len());
        for &id in current {
            if id.is_nil() {
                continue;
            }
            let name = name_of(&id)
                .filter(|n| !n.is_empty())
                .or_else(|| prev_map.as_ref().and_then(|m| m.get(&id)).cloned())
                .unwrap_or_default();
            next_friends.insert(id, name);
        }

        let mut missing = Vec::new();
        if let Some(prev) = &prev_map
            && notify
        {
            for (id, fallback) in prev {
                if !next_friends.contains_key(id) {
                    self.push_pending(*id, fallback.clone());
                    missing.push(*id);
                }
            }
        }

        let changed = prev_map.as_ref().is_none_or(|prev| prev != &next_friends);
        self.friends = next_friends;
        if changed {
            self.save();
        }
        missing
    }

    /// Record a newly formed friendship and persist the snapshot.
    pub fn on_added(&mut self, id: Uuid, name: Option<String>) {
        if id.is_nil() {
            return;
        }
        self.pending.retain(|p| p.id != id);
        let name = name.filter(|n| !n.is_empty()).unwrap_or_default();
        let changed = match self.friends.get(&id) {
            None => true,
            Some(existing) => !name.is_empty() && existing != &name,
        };
        if changed {
            self.friends
                .entry(id)
                .and_modify(|e| {
                    if !name.is_empty() {
                        *e = name.clone();
                    }
                })
                .or_insert(name);
            self.save();
        }
    }

    /// Remove `id` from the snapshot (e.g. when we terminate a friendship).
    pub fn on_removed(&mut self, id: &Uuid) {
        self.pending.retain(|p| p.id != *id);
        if self.friends.remove(id).is_some() {
            self.save();
        }
    }

    /// Queue a live ended-friendship notification while awaiting name lookup.
    pub fn queue_ended(&mut self, id: Uuid, fallback: String) {
        if id.is_nil() {
            return;
        }
        self.on_removed(&id);
        self.push_pending(id, fallback);
    }

    /// Refresh stored fallback names from the name cache and save if changed.
    pub fn refresh_and_save(&mut self, name_of: impl Fn(&Uuid) -> Option<String>) {
        let mut changed = false;
        for (id, stored) in self.friends.iter_mut() {
            if let Some(fresh) = name_of(id).filter(|n| !n.is_empty())
                && *stored != fresh
            {
                *stored = fresh;
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    /// Drain pending ended-friendship notices whose name is resolved or whose
    /// wait timed out, once the world is shown (`started`).
    pub fn take_ready(&mut self, started: bool, name_known: impl Fn(&Uuid) -> bool) -> Vec<(Uuid, String)> {
        self.take_ready_at(started, Instant::now(), name_known)
    }

    fn take_ready_at(&mut self, started: bool, now: Instant, name_known: impl Fn(&Uuid) -> bool) -> Vec<(Uuid, String)> {
        if !started {
            return Vec::new();
        }
        let mut ready = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            let queued_at = *self.pending[i].queued_at.get_or_insert(now);
            let id = self.pending[i].id;
            if name_known(&id) || now.saturating_duration_since(queued_at) >= NAME_WAIT {
                let item = self.pending.remove(i);
                ready.push((item.id, item.fallback));
            } else {
                i += 1;
            }
        }
        ready
    }

    fn push_pending(&mut self, id: Uuid, fallback: String) {
        if !self.pending.iter().any(|p| p.id == id) {
            self.pending.push(PendingEnded {
                id,
                fallback,
                queued_at: None,
            });
        }
    }

    fn save(&self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let mut friends: Vec<SavedFriend> = self
            .friends
            .iter()
            .map(|(id, name)| SavedFriend {
                id: *id,
                name: name.clone(),
            })
            .collect();
        friends.sort_by_key(|f| f.id);
        let Ok(bytes) = serde_json::to_vec_pretty(&Saved { friends }) else {
            return;
        };
        crate::saves::replace(path.clone(), move || {
            if let Err(e) = OWED.write(&path, bytes) {
                log::warn!("cannot save known friends snapshot ({e}); kept, tried again later");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn snapshot_persists_per_account_and_grid() {
        let root = std::env::temp_dir().join(format!("aurora-friends-{}", Uuid::new_v4()));
        let agni = root.join("acct").join(file_name("agni"));
        let aditi = root.join("acct").join(file_name("aditi"));
        assert_eq!(load(&agni), None);

        let mut snap = FriendSnapshot::default();
        let missing = snap.reconcile(
            Some(agni.clone()),
            None,
            &[id(1), id(2)],
            |u| match u.as_u128() {
                1 => Some("Jane Doe".into()),
                _ => None,
            },
            true,
        );
        assert!(missing.is_empty(), "first login must not emit removals");

        let loaded = load(&agni).expect("saved snapshot");
        assert_eq!(
            loaded,
            vec![
                SavedFriend {
                    id: id(1),
                    name: "Jane Doe".into(),
                },
                SavedFriend {
                    id: id(2),
                    name: String::new(),
                },
            ]
        );
        assert_eq!(load(&aditi), None, "other grid stays separate");

        std::fs::write(&agni, b"{not valid json").unwrap();
        assert_eq!(load(&agni), None, "corrupt file ignored without panic");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refused_write_is_kept_and_retried() {
        let root = std::env::temp_dir().join(format!("aurora-friends-refused-{}", Uuid::new_v4()));
        let blocker = root.join("acct");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&blocker, b"blocker").unwrap();
        let path = blocker.join(file_name("agni"));

        let mut snap = FriendSnapshot::default();
        snap.reconcile(Some(path.clone()), None, &[id(10)], |_| Some("Orion".into()), true);
        let loaded = load(&path).expect("kept in memory while write refused");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Orion");

        std::fs::remove_file(&blocker).unwrap();
        OWED.retry_now();
        assert_eq!(OWED.owed(&path), None);
        assert!(path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offline_diff_respects_notify_toggle_and_local_changes() {
        let root = std::env::temp_dir().join(format!("aurora-friends-diff-{}", Uuid::new_v4()));
        let path = root.join(file_name("agni"));

        // Initial login with Jane and Tom.
        let mut snap = FriendSnapshot::default();
        snap.reconcile(
            Some(path.clone()),
            None,
            &[id(1), id(2)],
            |u| Some(if u.as_u128() == 1 { "Jane Doe" } else { "Tom Smith" }.into()),
            true,
        );
        // Locally remove Tom and add Alice before logging out.
        snap.on_removed(&id(2));
        snap.on_added(id(3), Some("Alice".into()));

        // Next login: Alice is missing (unfriended offline), Tom is absent too (removed locally).
        let previous = load(&path);
        let mut snap2 = FriendSnapshot::default();
        let missing = snap2.reconcile(Some(path.clone()), previous, &[id(1)], |_| None, true);
        assert_eq!(missing, vec![id(3)]);
        let ready = snap2.take_ready(true, |_| true);
        assert_eq!(ready, vec![(id(3), "Alice".into())]);

        // With notify = false, missing friend is silently dropped from snapshot without queueing.
        let mut snap3 = FriendSnapshot::default();
        let missing_off = snap3.reconcile(
            Some(path.clone()),
            Some(vec![SavedFriend {
                id: id(1),
                name: "Jane Doe".into(),
            }]),
            &[],
            |_| None,
            false,
        );
        assert!(missing_off.is_empty());
        assert!(snap3.take_ready(true, |_| true).is_empty());
        assert_eq!(load(&path), Some(vec![]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pending_notice_waits_for_world_start_and_name_timeout() {
        let mut snap = FriendSnapshot::default();
        snap.queue_ended(id(5), "Saved Name".into());
        snap.queue_ended(id(5), "Saved Name".into());
        assert!(snap.take_ready_at(false, Instant::now(), |_| true).is_empty());

        let t0 = Instant::now();
        assert!(snap.take_ready_at(true, t0, |_| false).is_empty());
        assert!(snap.take_ready_at(true, t0 + Duration::from_secs(9), |_| false).is_empty());
        let ready = snap.take_ready_at(true, t0 + NAME_WAIT, |_| false);
        assert_eq!(ready, vec![(id(5), "Saved Name".into())]);
    }
}
