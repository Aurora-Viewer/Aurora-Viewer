//! Opt-in audio inheritance from OBJECT_REZZER_KEY. This is independent of
//! link parents and owners: only a verified rez chain extends an object mute.
//! The bridge queries only audible/pending sources and necessary ancestors.
use super::mutes::MuteList;
use aurora_net::{
    RegionHandle,
    bridge::query::{MAX_REZZERS, Query, Reply, Rezzer},
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

const GRACE: Duration = Duration::from_secs(3);
const TIMEOUT: Duration = Duration::from_secs(5);
const INTERVAL: Duration = Duration::from_millis(250);
const LIMIT: usize = 4096;

/// In-memory scope only; its bridge session key is never logged or saved.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Scope {
    pub agent: Uuid,
    pub region: Option<RegionHandle>,
    pub session: Option<Uuid>,
}

#[derive(Clone, Copy)]
struct Target {
    first: Instant,
    signature: Option<(Uuid, Uuid)>,
    attempted: Option<Instant>,
}

struct Pending {
    id: Uuid,
    at: Instant,
    targets: Vec<(Uuid, Option<(Uuid, Uuid)>)>,
    scope: Scope,
}

#[derive(Default)]
pub struct Tracker {
    scope: Scope,
    enabled: bool,
    blockers: HashSet<Uuid>,
    mute_version: Option<u64>,
    targets: HashMap<Uuid, Target>,
    links: HashMap<Uuid, Rezzer>,
    pending: Option<Pending>,
    last: Option<Instant>,
    scan: Option<Instant>,
    needed: HashSet<Uuid>,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Allowed,
    Blocked,
    Unknown,
}

impl Tracker {
    pub fn configure(&mut self, enabled: bool, scope: Scope, mutes: &MuteList) {
        if self.scope != scope {
            self.targets.clear();
            self.links.clear();
            self.pending = None;
            self.last = None;
            self.scan = None;
            self.needed.clear();
            self.mute_version = None;
            self.generation = self.generation.wrapping_add(1);
        }
        if self.enabled != enabled {
            self.generation = self.generation.wrapping_add(1);
            self.scan = None;
        }
        self.scope = scope;
        self.enabled = enabled;
        if self.mute_version != Some(mutes.version) {
            self.blockers = mutes.sound_blocked_objects();
            self.mute_version = Some(mutes.version);
            self.scan = None;
            self.generation = self.generation.wrapping_add(1);
        }
        if !enabled || self.blockers.is_empty() {
            self.targets.clear();
            self.links.clear();
            self.pending = None;
            self.needed.clear();
        }
    }

    pub fn available(&self) -> bool {
        self.scope.session.is_some()
    }
    fn active(&self) -> bool {
        self.enabled && self.available() && !self.blockers.is_empty()
    }

    pub fn scan_due(&self, now: Instant) -> bool {
        self.active() && self.scan.is_none_or(|at| now.saturating_duration_since(at) >= INTERVAL)
    }

    pub fn set_live(&mut self, ids: &[Uuid], now: Instant) {
        self.needed.clear();
        for source in ids {
            let mut id = *source;
            for _ in 0..64 {
                if id.is_nil() || !self.needed.insert(id) {
                    break;
                }
                if self.blockers.contains(&id) {
                    break;
                }
                let Some(link) = self.links.get(&id) else {
                    break;
                };
                id = link.rezzer;
            }
        }
        if self.targets.len() >= LIMIT / 2 {
            self.targets.retain(|id, _| self.needed.contains(id));
            self.links.retain(|id, _| self.needed.contains(id));
        }
        let ids: Vec<_> = self.needed.iter().copied().collect();
        if let Some(region) = self.scope.region {
            for id in ids {
                self.watch(id, region, None, now);
            }
        }
        self.scan = Some(now);
    }

    pub fn watch(&mut self, id: Uuid, region: RegionHandle, signature: Option<(Uuid, Uuid)>, now: Instant) {
        if !self.active() || id.is_nil() || self.scope.region != Some(region) {
            return;
        }
        if let Some(t) = self.targets.get_mut(&id) {
            if signature.is_some() && t.signature != signature {
                t.signature = signature;
                t.first = now;
                t.attempted = None;
                self.links.remove(&id);
                self.generation = self.generation.wrapping_add(1);
            }
            return;
        }
        // Do not evict a live source or its ancestry midway through a query.
        if self.targets.len() >= LIMIT {
            return;
        }
        self.targets.insert(
            id,
            Target {
                first: now,
                signature,
                attempted: None,
            },
        );
    }

    fn state(&self, object: Uuid) -> State {
        if !self.active() {
            return State::Allowed;
        }
        let mut id = object;
        let mut seen = HashSet::new();
        for _ in 0..64 {
            if id.is_nil() || !seen.insert(id) {
                return State::Allowed;
            }
            if self.blockers.contains(&id) {
                return State::Blocked;
            }
            let Some(link) = self.links.get(&id) else {
                return State::Unknown;
            };
            if self.blockers.contains(&link.root) {
                return State::Blocked;
            }
            id = link.rezzer;
        }
        State::Allowed
    }

    pub fn blocked(&self, object: Uuid) -> bool {
        self.state(object) == State::Blocked
    }

    pub fn waiting(&self, object: Uuid, region: RegionHandle, first_seen: Instant, now: Instant) -> bool {
        !object.is_nil()
            && self.active()
            && self.scope.region == Some(region)
            && self.targets.get(&object).map_or_else(
                || now.saturating_duration_since(first_seen) < GRACE,
                |t| now.saturating_duration_since(t.first) < GRACE,
            )
            && self.state(object) == State::Unknown
    }

    pub fn request(&mut self, now: Instant) -> Option<(Uuid, Query)> {
        if !self.active() {
            return None;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| now.saturating_duration_since(p.at) >= TIMEOUT)
        {
            self.pending = None;
        }
        if self.pending.is_some() || self.last.is_some_and(|at| now.saturating_duration_since(at) < INTERVAL) {
            return None;
        }
        let mut wanted: Vec<_> = self
            .targets
            .iter()
            .filter(|(id, t)| {
                self.needed.contains(id)
                    && !self.links.contains_key(id)
                    && t.attempted.is_none_or(|at| now.saturating_duration_since(at) >= TIMEOUT)
            })
            .map(|(id, t)| (*id, t.first))
            .collect();
        wanted.sort_unstable_by_key(|(id, first)| (*first, *id));
        wanted.truncate(MAX_REZZERS);
        if wanted.is_empty() {
            return None;
        }
        let id = Uuid::new_v4();
        let ids: Vec<_> = wanted.into_iter().map(|(id, _)| id).collect();
        let targets = ids
            .iter()
            .map(|id| {
                let t = self.targets.get_mut(id).expect("selected target");
                t.attempted = Some(now);
                (*id, t.signature)
            })
            .collect();
        self.pending = Some(Pending {
            id,
            at: now,
            targets,
            scope: self.scope,
        });
        self.last = Some(now);
        Some((id, Query::Rezzers(ids)))
    }

    pub fn owns(&self, id: Uuid) -> bool {
        self.pending.as_ref().is_some_and(|p| p.id == id)
    }

    pub fn reply(&mut self, id: Uuid, result: Result<Reply, String>, enabled: bool, scope: Scope, now: Instant) {
        if !self.owns(id) {
            return;
        }
        let Some(p) = self.pending.take() else {
            return;
        };
        if !enabled || scope != p.scope || scope != self.scope || now.saturating_duration_since(p.at) >= TIMEOUT {
            return;
        }
        let Ok(Reply::Rezzers(links)) = result else {
            return;
        };
        for link in links {
            if let Some((_, signature)) = p.targets.iter().find(|(id, _)| *id == link.object)
                && self.targets.get(&link.object).is_some_and(|t| t.signature == *signature)
            {
                self.links.insert(link.object, link);
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }
}

#[cfg(test)]
mod tests;
