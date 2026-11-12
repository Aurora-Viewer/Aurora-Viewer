//! Cooperative Control Lease Manager (`LeaseManager`).
//!
//! Coordinates control between the human user and one or more MCP clients, and
//! tracks temporary camera/setting overrides in a rollback stack so they are
//! automatically restored when the lease is released or expires.

use crate::bridge::{McpError, RollbackAction};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
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
    pub ttl_seconds: u64,
    pub remaining_seconds: u64,
    pub expires_at_unix_ms: u64,
    pub rollback_count: usize,
}

struct ActiveLease {
    lease_id: String,
    owner: String,
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
            ttl_seconds: self.ttl.as_secs(),
            remaining_seconds: remaining,
            expires_at_unix_ms: self.expires_at_unix_ms,
            rollback_count: self.rollbacks.len(),
        }
    }
}

pub struct LeaseManager {
    inner: Mutex<Option<ActiveLease>>,
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

    /// Acquire or renew a control lease.
    ///
    /// If the same `owner` (or matching `lease_id`) already holds the active
    /// lease, its TTL is extended while preserving the existing rollback stack.
    pub fn acquire(
        &self,
        owner: impl Into<String>,
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
            let same_holder = existing_lease_id.is_some_and(|id| id == active.lease_id)
                || active.owner == owner;
            if !same_holder {
                let remaining = active.expires_at.saturating_duration_since(now).as_secs();
                return Err(McpError::LeaseConflict {
                    holder: active.owner.clone(),
                    remaining_secs: remaining,
                });
            }
            active.ttl = ttl;
            active.expires_at = now + ttl;
            active.expires_at_unix_ms = expires_unix_ms;
            return Ok((active.snapshot(now), expired_rollbacks));
        }

        let lease = ActiveLease {
            lease_id: Uuid::new_v4().to_string(),
            owner,
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
    pub fn release(
        &self,
        lease_id: Option<&str>,
    ) -> Result<(Option<LeaseSnapshot>, Vec<RollbackAction>), McpError> {
        let mut guard = self.inner.lock();
        let now = Instant::now();
        let Some(active) = guard.as_ref() else {
            return Ok((None, Vec::new()));
        };
        if !active.is_expired(now)
            && let Some(requested_id) = lease_id
            && !requested_id.is_empty()
            && requested_id != active.lease_id
        {
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
        guard
            .as_ref()
            .filter(|l| !l.is_expired(now))
            .map(|l| l.snapshot(now))
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
            if let Some(id) = lease_id
                && !id.is_empty()
                && id != active.lease_id
            {
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
            (
                RollbackAction::RestoreSetting { key: k1, .. },
                RollbackAction::RestoreSetting { key: k2, .. },
            ) => k1 == k2,
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

        let (snap, expired) = mgr
            .acquire("agent-alpha", 120, None)
            .expect("acquire lease");
        assert!(expired.is_empty());
        assert_eq!(snap.owner, "agent-alpha");

        // Another owner cannot steal an active lease.
        let conflict = mgr.acquire("agent-beta", 60, None);
        assert!(matches!(conflict, Err(McpError::LeaseConflict { .. })));

        // Same owner can renew.
        let (renewed, _) = mgr
            .acquire("agent-alpha", 300, Some(&snap.lease_id))
            .expect("renew lease");
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

        let (released, rollbacks) = mgr
            .release(Some(&snap.lease_id))
            .expect("release lease");
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
}
