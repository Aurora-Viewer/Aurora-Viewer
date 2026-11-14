//! Cooperative Control Lease Manager (`LeaseManager`).
//!
//! Coordinates control between the human user and one or more MCP clients, and
//! tracks temporary camera/setting overrides in a rollback stack so they are
//! automatically restored when the lease is released or expires.

use crate::bridge::{McpError, RollbackAction};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Default lease TTL (300 seconds, matching Legacy Viewer MCP `control_acquire`).
pub const DEFAULT_LEASE_SECONDS: u64 = 300;
/// Maximum lease TTL (3600 seconds / 1 hour).
pub const MAX_LEASE_SECONDS: u64 = 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseSnapshot {
    pub lease_id: String,
    pub owner: String,
    /// What the holder said it is doing (control_acquire `purpose`).
    #[serde(default)]
    pub purpose: String,
    pub ttl_seconds: u64,
    pub remaining_seconds: u64,
    pub expires_at_unix_ms: u64,
    pub rollback_count: usize,
}

struct ActiveLease {
    lease_id: String,
    owner: String,
    purpose: String,
    ttl: Duration,
    expires_at: Instant,
    expires_at_unix_ms: u64,
    rollbacks: Vec<RollbackAction>,
}

impl ActiveLease {
    fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }

    fn snapshot(&self, now: Instant) -> LeaseSnapshot {
        let remaining = self.expires_at.saturating_duration_since(now).as_secs();
        LeaseSnapshot {
            lease_id: self.lease_id.clone(),
            owner: self.owner.clone(),
            purpose: self.purpose.clone(),
            ttl_seconds: self.ttl.as_secs(),
            remaining_seconds: remaining,
            expires_at_unix_ms: self.expires_at_unix_ms,
            rollback_count: self.rollbacks.len(),
        }
    }
}

/// How the person at the viewer shares it with agents (the control bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanMode {
    /// An agent asks first: Allow / Not now; unanswered for ASK_TIMEOUT, it goes ahead.
    Ask,
    /// Agents take turns without asking (the person is away).
    Auto,
    /// No agent may take control: the person is using the viewer.
    Hold,
}

impl HumanMode {
    /// AURORA_MCP_HUMAN_MODE=ask|auto|hold; Auto when unset, the behaviour
    /// before the queue existed.
    pub fn from_env() -> Self {
        match std::env::var("AURORA_MCP_HUMAN_MODE").as_deref().map(str::trim) {
            Ok("ask") => Self::Ask,
            Ok("hold") => Self::Hold,
            _ => Self::Auto,
        }
    }
}

/// Unanswered this long, an Ask lets the agent in (the person may be away).
pub const ASK_TIMEOUT: Duration = Duration::from_secs(60);
/// An Allow is good for this long, for the agent to come back for its lease.
const APPROVAL_VALID: Duration = Duration::from_secs(120);
/// "Not now" keeps an agent waiting this long before it may ask again.
pub const NOT_NOW_FOR: Duration = Duration::from_secs(300);
/// A waiter that has not asked again for this long has given up.
const WAITER_STALE: Duration = Duration::from_secs(120);

struct Waiter {
    owner: String,
    purpose: String,
    ttl_secs: u64,
    since: Instant,
    last_seen: Instant,
}

struct Shared {
    mode: HumanMode,
    queue: Vec<Waiter>,
    /// The agent being asked about, and since when.
    asking: Option<(String, Instant)>,
    approved: Option<(String, Instant)>,
    not_now: HashMap<String, Instant>,
}

/// Why an agent did not get the lease yet (control_acquire answers it with
/// `ok: false, waiting: true` instead of an error).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waiting {
    /// `busy` (another agent holds it), `next_in_line` (another agent is
    /// ahead), `awaiting_approval` (the person is being asked),
    /// `not_now` (the person said not now), `held_by_human` (Hold).
    pub reason: String,
    /// 1 = next.
    pub position: usize,
    pub holder: Option<String>,
    pub holder_purpose: Option<String>,
    pub holder_remaining_secs: Option<u64>,
    /// Ask again after about this long.
    pub retry_after_secs: u64,
}

pub enum AcquireOutcome {
    Granted(LeaseSnapshot, Vec<RollbackAction>),
    Waiting(Waiting),
}

/// What the viewer's control bar and `control_queue` show.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlView {
    pub mode: HumanMode,
    pub active: Option<LeaseSnapshot>,
    /// The agent the person is being asked about, if any.
    pub asking: Option<QueuedAgent>,
    /// Seconds until an unanswered Ask lets it in.
    pub asking_auto_in_secs: Option<u64>,
    pub queue: Vec<QueuedAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedAgent {
    pub owner: String,
    pub purpose: String,
    pub ttl_seconds: u64,
    pub waiting_secs: u64,
}

pub struct LeaseManager {
    inner: Mutex<Option<ActiveLease>>,
    shared: Mutex<Shared>,
    auto_acquire: bool,
}

impl Default for LeaseManager {
    fn default() -> Self {
        Self::new(std::env::var("AURORA_MCP_AUTO_LEASE").as_deref() == Ok("1"))
    }
}

impl LeaseManager {
    pub fn new(auto_acquire: bool) -> Self {
        Self {
            inner: Mutex::new(None),
            shared: Mutex::new(Shared {
                mode: HumanMode::from_env(),
                queue: Vec::new(),
                asking: None,
                approved: None,
                not_now: HashMap::new(),
            }),
            auto_acquire,
        }
    }

    /// Returns any rollbacks from an expired lease so the caller can dispatch
    /// them to the viewer before starting a new operation.
    pub fn take_expired_rollbacks(&self) -> Option<(String, Vec<RollbackAction>)> {
        let mut guard = self.inner.lock();
        let now = Instant::now();
        if guard.as_ref().is_some_and(|l| l.is_expired(now)) {
            let expired = guard.take()?;
            let mut actions = expired.rollbacks;
            actions.reverse();
            return Some((expired.lease_id, actions));
        }
        None
    }

    /// Acquire or renew a control lease, taking turns with other agents and
    /// with the person at the viewer (`HumanMode`).
    ///
    /// The holder renewing (same `owner` or `lease_id`) is granted at once.
    /// Otherwise the agent joins the queue (when `wait`) and is answered
    /// `Waiting` until it is first in line, the viewer is free and, in Ask
    /// mode, the person allowed it or did not answer for ASK_TIMEOUT. A
    /// waiter keeps its place by asking again.
    pub fn acquire_turn(
        &self,
        owner: &str,
        purpose: &str,
        ttl_seconds: u64,
        existing_lease_id: Option<&str>,
        wait: bool,
    ) -> Result<AcquireOutcome, McpError> {
        let owner = if owner.trim().is_empty() { "mcp-agent" } else { owner.trim() };
        let now = Instant::now();
        let ttl_secs = ttl_seconds.clamp(1, MAX_LEASE_SECONDS);
        // the holder renewing, or an expired lease to clear
        {
            let guard = self.inner.lock();
            if let Some(active) = guard.as_ref().filter(|l| !l.is_expired(now)) {
                let same = existing_lease_id.is_some_and(|id| id == active.lease_id) || active.owner == owner;
                if same {
                    drop(guard);
                    let (snap, rb) = self.acquire(owner, purpose, ttl_secs, existing_lease_id)?;
                    return Ok(AcquireOutcome::Granted(snap, rb));
                }
            }
        }
        let mut sh = self.shared.lock();
        sh.queue.retain(|w| now.duration_since(w.last_seen) < WAITER_STALE);
        sh.not_now.retain(|_, at| now.duration_since(*at) < NOT_NOW_FOR);
        if sh
            .approved
            .as_ref()
            .is_some_and(|(_, at)| now.duration_since(*at) >= APPROVAL_VALID)
        {
            sh.approved = None;
        }
        // keep (or take) a place in the queue
        if let Some(w) = sh.queue.iter_mut().find(|w| w.owner == owner) {
            w.last_seen = now;
            w.purpose = purpose.to_owned();
            w.ttl_secs = ttl_secs;
        } else if wait || sh.mode == HumanMode::Ask {
            sh.queue.push(Waiter {
                owner: owner.to_owned(),
                purpose: purpose.to_owned(),
                ttl_secs,
                since: now,
                last_seen: now,
            });
        }
        let position = sh.queue.iter().position(|w| w.owner == owner).map_or(0, |i| i + 1);
        let holder = {
            let guard = self.inner.lock();
            guard.as_ref().filter(|l| !l.is_expired(now)).map(|l| {
                (
                    l.owner.clone(),
                    l.purpose.clone(),
                    l.expires_at.saturating_duration_since(now).as_secs(),
                )
            })
        };
        let waiting = |reason: &str, retry: u64, holder: &Option<(String, String, u64)>| Waiting {
            reason: reason.to_owned(),
            position,
            holder: holder.as_ref().map(|h| h.0.clone()),
            holder_purpose: holder.as_ref().map(|h| h.1.clone()),
            holder_remaining_secs: holder.as_ref().map(|h| h.2),
            retry_after_secs: retry,
        };
        let busy = |holder: &Option<(String, String, u64)>| -> Result<AcquireOutcome, McpError> {
            let (who, _, left) = holder.clone().unwrap_or_default();
            Err(McpError::LeaseConflict {
                holder: who,
                remaining_secs: left,
            })
        };
        if sh.mode == HumanMode::Hold {
            let h = Some(("the person at the viewer (Hold)".to_owned(), String::new(), 0));
            return if wait {
                Ok(AcquireOutcome::Waiting(waiting("held_by_human", 30, &h)))
            } else {
                busy(&h)
            };
        }
        if holder.is_some() {
            let retry = holder.as_ref().map_or(10, |h| h.2.clamp(5, 30));
            return if wait {
                Ok(AcquireOutcome::Waiting(waiting("busy", retry, &holder)))
            } else {
                busy(&holder)
            };
        }
        if let Some(head) = sh.queue.first().filter(|w| w.owner != owner) {
            let ahead = Some((head.owner.clone(), head.purpose.clone(), 0));
            return if wait {
                Ok(AcquireOutcome::Waiting(waiting("next_in_line", 10, &ahead)))
            } else {
                busy(&ahead)
            };
        }
        if let Some(at) = sh.not_now.get(owner) {
            let left = NOT_NOW_FOR.saturating_sub(now.duration_since(*at)).as_secs();
            return Ok(AcquireOutcome::Waiting(waiting("not_now", left.max(5), &None)));
        }
        if sh.mode == HumanMode::Ask {
            let approved = sh.approved.as_ref().is_some_and(|(o, _)| o == owner);
            let since = match &sh.asking {
                Some((o, at)) if o == owner => *at,
                _ => {
                    sh.asking = Some((owner.to_owned(), now));
                    now
                }
            };
            let unanswered = now.duration_since(since) >= ASK_TIMEOUT;
            if !approved && !unanswered {
                let left = ASK_TIMEOUT.saturating_sub(now.duration_since(since)).as_secs();
                return Ok(AcquireOutcome::Waiting(waiting("awaiting_approval", left.clamp(3, 10), &None)));
            }
        }
        // granted: out of the queue, nothing pending
        sh.queue.retain(|w| w.owner != owner);
        sh.asking = None;
        sh.approved = None;
        drop(sh);
        let (snap, rb) = self.acquire(owner, purpose, ttl_secs, existing_lease_id)?;
        Ok(AcquireOutcome::Granted(snap, rb))
    }

    /// The control bar's state, also served by `control_queue`.
    pub fn view(&self) -> ControlView {
        let now = Instant::now();
        let active = self.status();
        let sh = self.shared.lock();
        let queued = |w: &Waiter| QueuedAgent {
            owner: w.owner.clone(),
            purpose: w.purpose.clone(),
            ttl_seconds: w.ttl_secs,
            waiting_secs: now.duration_since(w.since).as_secs(),
        };
        let live: Vec<&Waiter> = sh.queue.iter().filter(|w| now.duration_since(w.last_seen) < WAITER_STALE).collect();
        let asking = sh
            .asking
            .as_ref()
            .filter(|_| sh.mode == HumanMode::Ask && active.is_none())
            .and_then(|(o, at)| live.iter().find(|w| &w.owner == o).map(|w| (queued(w), *at)));
        ControlView {
            mode: sh.mode,
            asking_auto_in_secs: asking
                .as_ref()
                .map(|(_, at)| ASK_TIMEOUT.saturating_sub(now.duration_since(*at)).as_secs()),
            asking: asking.map(|(q, _)| q),
            queue: live.iter().map(|w| queued(w)).collect(),
            active,
        }
    }

    /// The person allowed `owner` (Allow on the card).
    pub fn human_allow(&self, owner: &str) {
        let mut sh = self.shared.lock();
        sh.not_now.remove(owner);
        sh.approved = Some((owner.to_owned(), Instant::now()));
    }

    /// The person said Not now: `owner` waits NOT_NOW_FOR, behind the others.
    pub fn human_not_now(&self, owner: &str) {
        let mut sh = self.shared.lock();
        sh.not_now.insert(owner.to_owned(), Instant::now());
        if sh.asking.as_ref().is_some_and(|(o, _)| o == owner) {
            sh.asking = None;
        }
        if let Some(i) = sh.queue.iter().position(|w| w.owner == owner) {
            let w = sh.queue.remove(i);
            sh.queue.push(w);
        }
    }

    /// The person took the viewer back: the lease ends now (its rollbacks are
    /// returned to apply) and the mode becomes Hold until they change it.
    pub fn human_take_back(&self) -> Vec<RollbackAction> {
        let taken = self.inner.lock().take();
        self.shared.lock().mode = HumanMode::Hold;
        taken.map_or_else(Vec::new, |l| {
            let mut rb = l.rollbacks;
            rb.reverse();
            rb
        })
    }

    pub fn set_mode(&self, mode: HumanMode) {
        let mut sh = self.shared.lock();
        sh.mode = mode;
        if mode != HumanMode::Ask {
            sh.asking = None;
        }
    }

    /// Acquire or renew a control lease at once (no queue, no asking).
    ///
    /// If the same `owner` (or matching `lease_id`) already holds the active
    /// lease, its TTL is extended while preserving the existing rollback stack.
    pub fn acquire(
        &self,
        owner: impl Into<String>,
        purpose: &str,
        ttl_seconds: u64,
        existing_lease_id: Option<&str>,
    ) -> Result<(LeaseSnapshot, Vec<RollbackAction>), McpError> {
        let owner = {
            let s = owner.into();
            if s.trim().is_empty() {
                "mcp-agent".to_owned()
            } else {
                s.trim().to_owned()
            }
        };
        let ttl_secs = ttl_seconds.clamp(1, MAX_LEASE_SECONDS);
        let ttl = Duration::from_secs(ttl_secs);
        let now = Instant::now();
        let expires_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .saturating_add(ttl)
            .as_millis() as u64;

        let mut guard = self.inner.lock();
        let mut expired_rollbacks = Vec::new();
        if guard.as_ref().is_some_and(|l| l.is_expired(now))
            && let Some(mut expired) = guard.take()
        {
            expired.rollbacks.reverse();
            expired_rollbacks = expired.rollbacks;
        }

        if let Some(active) = guard.as_mut() {
            let same_holder = existing_lease_id.is_some_and(|id| id == active.lease_id) || active.owner == owner;
            if !same_holder {
                let remaining = active.expires_at.saturating_duration_since(now).as_secs();
                return Err(McpError::LeaseConflict {
                    holder: active.owner.clone(),
                    remaining_secs: remaining,
                });
            }
            active.ttl = ttl;
            if !purpose.is_empty() {
                active.purpose = purpose.to_owned();
            }
            active.expires_at = now + ttl;
            active.expires_at_unix_ms = expires_unix_ms;
            return Ok((active.snapshot(now), expired_rollbacks));
        }

        let lease = ActiveLease {
            lease_id: Uuid::new_v4().to_string(),
            owner,
            purpose: purpose.to_owned(),
            ttl,
            expires_at: now + ttl,
            expires_at_unix_ms: expires_unix_ms,
            rollbacks: Vec::new(),
        };
        let snap = lease.snapshot(now);
        *guard = Some(lease);
        Ok((snap, expired_rollbacks))
    }

    /// Release the active control lease and return its rollback stack in LIFO
    /// restoration order.
    pub fn release(&self, lease_id: Option<&str>) -> Result<(Option<LeaseSnapshot>, Vec<RollbackAction>), McpError> {
        let mut guard = self.inner.lock();
        let now = Instant::now();
        let Some(active) = guard.as_ref() else {
            return Ok((None, Vec::new()));
        };
        if !active.is_expired(now) && !lease_id.is_some_and(|id| !id.is_empty() && id == active.lease_id) {
            return Err(McpError::LeaseConflict {
                holder: active.owner.clone(),
                remaining_secs: active.expires_at.saturating_duration_since(now).as_secs(),
            });
        }
        let released = guard.take().expect("checked active lease");
        let snap = released.snapshot(now);
        let mut rollbacks = released.rollbacks;
        rollbacks.reverse();
        Ok((Some(snap), rollbacks))
    }

    /// Inspect current lease state without mutating it.
    pub fn status(&self) -> Option<LeaseSnapshot> {
        let guard = self.inner.lock();
        let now = Instant::now();
        guard.as_ref().filter(|l| !l.is_expired(now)).map(|l| l.snapshot(now))
    }

    /// Verify that a mutating operation is permitted under the active lease (or
    /// auto-acquire if single-agent auto-lease mode is enabled).
    pub fn verify_for_mutation(&self, lease_id: Option<&str>) -> Result<String, McpError> {
        let now = Instant::now();
        let mut guard = self.inner.lock();
        if guard.as_ref().is_some_and(|l| l.is_expired(now)) {
            *guard = None;
        }
        if let Some(active) = guard.as_ref() {
            if !lease_id.is_some_and(|id| !id.is_empty() && id == active.lease_id) {
                return Err(McpError::LeaseConflict {
                    holder: active.owner.clone(),
                    remaining_secs: active.expires_at.saturating_duration_since(now).as_secs(),
                });
            }
            return Ok(active.lease_id.clone());
        }
        if self.auto_acquire {
            let ttl = Duration::from_secs(DEFAULT_LEASE_SECONDS);
            let expires_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .saturating_add(ttl)
                .as_millis() as u64;
            let lease_id = Uuid::new_v4().to_string();
            *guard = Some(ActiveLease {
                lease_id: lease_id.clone(),
                owner: "auto-lease".to_owned(),
                purpose: String::new(),
                ttl,
                expires_at: now + ttl,
                expires_at_unix_ms: expires_unix_ms,
                rollbacks: Vec::new(),
            });
            return Ok(lease_id);
        }
        Err(McpError::NoActiveLease)
    }

    /// Record a rollback action on the active lease, deduplicating so only the
    /// initial pre-lease snapshot for a given setting key or camera state is kept.
    pub fn record_rollback(&self, action: RollbackAction) {
        let mut guard = self.inner.lock();
        let Some(active) = guard.as_mut() else {
            return;
        };
        let already_recorded = active.rollbacks.iter().any(|existing| match (existing, &action) {
            (RollbackAction::RestoreSetting { key: k1, .. }, RollbackAction::RestoreSetting { key: k2, .. }) => k1 == k2,
            (RollbackAction::RestoreCamera { .. }, RollbackAction::RestoreCamera { .. }) => true,
            (RollbackAction::ResetPoser, RollbackAction::ResetPoser) => true,
            _ => false,
        });
        if !already_recorded {
            active.rollbacks.push(action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_lease_lifecycle_and_rollback_deduplication() {
        let mgr = LeaseManager::new(false);
        assert!(mgr.verify_for_mutation(None).is_err());

        let (snap, expired) = mgr.acquire("agent-alpha", "", 120, None).expect("acquire lease");
        assert!(expired.is_empty());
        assert_eq!(snap.owner, "agent-alpha");

        // Another owner cannot steal an active lease.
        let conflict = mgr.acquire("agent-beta", "", 60, None);
        assert!(matches!(conflict, Err(McpError::LeaseConflict { .. })));

        // Omitting lease_id cannot hijack or release an active lease.
        assert!(matches!(mgr.verify_for_mutation(None), Err(McpError::LeaseConflict { .. })));
        assert!(matches!(mgr.verify_for_mutation(Some("")), Err(McpError::LeaseConflict { .. })));
        assert!(matches!(mgr.release(None), Err(McpError::LeaseConflict { .. })));

        // Same owner can renew.
        let (renewed, _) = mgr.acquire("agent-alpha", "", 300, Some(&snap.lease_id)).expect("renew lease");
        assert_eq!(renewed.lease_id, snap.lease_id);
        assert_eq!(renewed.ttl_seconds, 300);

        // Record two mutations to the same setting key; only the original value is kept.
        mgr.record_rollback(RollbackAction::RestoreSetting {
            key: "camera.fov".into(),
            value: json!(1.047),
        });
        mgr.record_rollback(RollbackAction::RestoreSetting {
            key: "camera.fov".into(),
            value: json!(1.500),
        });
        mgr.record_rollback(RollbackAction::RestoreCamera {
            mode: "third_person".into(),
            position: [128.0, 128.0, 25.0],
            focus: [128.0, 130.0, 25.0],
            fov: 1.047,
        });

        let (released, rollbacks) = mgr.release(Some(&snap.lease_id)).expect("release lease");
        assert!(released.is_some());
        assert_eq!(rollbacks.len(), 2);
        // LIFO order: camera restored first, then setting.
        assert!(matches!(rollbacks[0], RollbackAction::RestoreCamera { .. }));
        assert_eq!(
            rollbacks[1],
            RollbackAction::RestoreSetting {
                key: "camera.fov".into(),
                value: json!(1.047),
            }
        );
    }

    fn granted(o: Result<AcquireOutcome, McpError>) -> LeaseSnapshot {
        match o {
            Ok(AcquireOutcome::Granted(snap, _)) => snap,
            Ok(AcquireOutcome::Waiting(w)) => panic!("expected the lease, got waiting: {}", w.reason),
            Err(e) => panic!("expected the lease, got {e}"),
        }
    }

    fn waiting(o: Result<AcquireOutcome, McpError>) -> Waiting {
        match o {
            Ok(AcquireOutcome::Waiting(w)) => w,
            Ok(AcquireOutcome::Granted(snap, _)) => panic!("expected to wait, got the lease of {}", snap.owner),
            Err(e) => panic!("expected to wait, got {e}"),
        }
    }

    fn manager(mode: HumanMode) -> LeaseManager {
        let mgr = LeaseManager::new(false);
        mgr.set_mode(mode);
        mgr
    }

    #[test]
    fn agents_queue_and_take_turns_in_order() {
        let mgr = manager(HumanMode::Auto);
        let alpha = granted(mgr.acquire_turn("alpha", "rebuild coop 4", 600, None, true));
        assert_eq!(alpha.purpose, "rebuild coop 4");
        let w = waiting(mgr.acquire_turn("beta", "upload table", 300, None, true));
        assert_eq!((w.reason.as_str(), w.position, w.holder.as_deref()), ("busy", 1, Some("alpha")));
        assert_eq!(w.holder_purpose.as_deref(), Some("rebuild coop 4"));
        let w = waiting(mgr.acquire_turn("gamma", "", 300, None, true));
        assert_eq!(w.position, 2);
        mgr.release(Some(&alpha.lease_id)).expect("release");
        // gamma may not jump the queue, even without waiting
        assert!(matches!(
            mgr.acquire_turn("gamma", "", 300, None, false),
            Err(McpError::LeaseConflict { ref holder, .. }) if holder == "beta"
        ));
        let beta = granted(mgr.acquire_turn("beta", "upload table", 300, None, true));
        assert_eq!(beta.owner, "beta");
        let view = mgr.view();
        assert_eq!(view.active.map(|a| a.owner).as_deref(), Some("beta"));
        assert_eq!(view.queue.iter().map(|q| q.owner.as_str()).collect::<Vec<_>>(), ["gamma"]);
    }

    #[test]
    fn ask_mode_waits_for_allow() {
        let mgr = manager(HumanMode::Ask);
        let w = waiting(mgr.acquire_turn("alpha", "film the coop", 300, None, false));
        assert_eq!(w.reason, "awaiting_approval");
        let view = mgr.view();
        assert_eq!(view.asking.as_ref().map(|a| a.purpose.as_str()), Some("film the coop"));
        assert!(view.asking_auto_in_secs.is_some_and(|s| s <= ASK_TIMEOUT.as_secs()));
        mgr.human_allow("alpha");
        assert_eq!(granted(mgr.acquire_turn("alpha", "film the coop", 300, None, false)).owner, "alpha");
        assert!(mgr.view().asking.is_none());
    }

    #[test]
    fn not_now_sends_the_agent_back() {
        let mgr = manager(HumanMode::Ask);
        waiting(mgr.acquire_turn("alpha", "", 300, None, true));
        mgr.human_not_now("alpha");
        assert_eq!(waiting(mgr.acquire_turn("alpha", "", 300, None, true)).reason, "not_now");
        // Allow lifts it
        mgr.human_allow("alpha");
        granted(mgr.acquire_turn("alpha", "", 300, None, true));
    }

    #[test]
    fn hold_and_take_back_keep_agents_out() {
        let mgr = manager(HumanMode::Auto);
        let alpha = granted(mgr.acquire_turn("alpha", "", 300, None, true));
        mgr.record_rollback(RollbackAction::RestoreSetting {
            key: "camera.fov".into(),
            value: json!(60.0),
        });
        let rollbacks = mgr.human_take_back();
        assert_eq!(rollbacks.len(), 1, "the camera is restored when the person takes the viewer back");
        assert_eq!(mgr.view().mode, HumanMode::Hold);
        assert!(mgr.verify_for_mutation(Some(&alpha.lease_id)).is_err(), "the old lease is gone");
        assert_eq!(waiting(mgr.acquire_turn("alpha", "", 300, None, true)).reason, "held_by_human");
        mgr.set_mode(HumanMode::Auto);
        granted(mgr.acquire_turn("alpha", "", 300, None, true));
    }
}
