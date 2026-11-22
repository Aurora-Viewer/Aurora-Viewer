//! Per-category token-bucket rate limiter for outgoing MCP mutations (`A7`).
//!
//! Protects regions and accounts from runaway loops without blocking normal
//! autonomous operation: defaults allow generous bursts, and refusals include
//! `retry_after_ms` so agents can pause and retry on their own.

use crate::bridge::{McpError, ViewerRequest};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateBucket {
    Chat,
    Im,
    Teleport,
    Rez,
    GiveInventory,
    ParcelAdmin,
    Payment,
    ScriptCompile,
    Touch,
}

impl RateBucket {
    pub const ALL: &[RateBucket] = &[
        Self::Chat,
        Self::Im,
        Self::Teleport,
        Self::Rez,
        Self::GiveInventory,
        Self::ParcelAdmin,
        Self::Payment,
        Self::ScriptCompile,
        Self::Touch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Im => "im",
            Self::Teleport => "teleport",
            Self::Rez => "rez",
            Self::GiveInventory => "give_inventory",
            Self::ParcelAdmin => "parcel_admin",
            Self::Payment => "payment",
            Self::ScriptCompile => "script_compile",
            Self::Touch => "touch",
        }
    }

    fn base_params(self) -> (f64, f64) {
        match self {
            Self::Chat => (10.0, 2.0),
            Self::Im => (6.0, 1.0),
            Self::Teleport => (3.0, 0.25),
            Self::Rez => (20.0, 5.0),
            Self::GiveInventory => (5.0, 0.5),
            Self::ParcelAdmin => (10.0, 2.0),
            Self::Payment => (5.0, 0.5),
            Self::ScriptCompile => (10.0, 2.0),
            Self::Touch => (15.0, 5.0),
        }
    }
}

#[derive(Debug, Clone)]
struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

/// Token-bucket rate limiter keyed by [`RateBucket`].
pub struct RateLimiter {
    buckets: Mutex<HashMap<RateBucket, TokenBucket>>,
    enabled: bool,
    multiplier: f64,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::from_env()
    }
}

impl RateLimiter {
    pub fn new(enabled: bool, multiplier: f64) -> Self {
        let clamped = if multiplier.is_finite() && multiplier > 0.0 {
            multiplier.clamp(0.1, 100.0)
        } else {
            1.0
        };
        Self {
            buckets: Mutex::new(HashMap::new()),
            enabled,
            multiplier: clamped,
        }
    }

    pub fn disabled() -> Self {
        Self::new(false, 1.0)
    }

    /// Reads `AURORA_MCP_RATE_LIMIT` (`0|false|off|disabled` disables) and
    /// `AURORA_MCP_RATE_MULTIPLIER` (positive float scaling capacities/refills).
    pub fn from_env() -> Self {
        let enabled = !matches!(
            std::env::var("AURORA_MCP_RATE_LIMIT")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "0" | "false" | "off" | "disabled"
        );
        let multiplier = std::env::var("AURORA_MCP_RATE_MULTIPLIER")
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(1.0);
        Self::new(enabled, multiplier)
    }

    pub fn check_and_take(&self, bucket: RateBucket) -> Result<(), McpError> {
        self.check_at(bucket, Instant::now())
    }

    pub(crate) fn check_at(&self, bucket: RateBucket, now: Instant) -> Result<(), McpError> {
        if !self.enabled {
            return Ok(());
        }
        let (base_cap, base_refill) = bucket.base_params();
        let cap = (base_cap * self.multiplier).max(1.0);
        let refill = (base_refill * self.multiplier).max(0.01);

        let mut map = self.buckets.lock();
        let state = map.entry(bucket).or_insert_with(|| TokenBucket {
            tokens: cap,
            last_refill: now,
        });
        let elapsed = now.saturating_duration_since(state.last_refill).as_secs_f64();
        if elapsed > 0.0 {
            state.tokens = (state.tokens + elapsed * refill).min(cap);
            state.last_refill = now;
        }
        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            return Ok(());
        }
        let deficit = 1.0 - state.tokens;
        let retry_after_ms = ((deficit / refill) * 1000.0).ceil().max(50.0) as u64;
        Err(McpError::RateLimited {
            category: bucket.as_str().to_owned(),
            retry_after_ms,
            detail: format!("burst {}, sustained {:.1}/min", cap.round() as u32, refill * 60.0),
        })
    }

    /// Inspects whether `bucket` currently has >= 1 token without consuming any.
    pub fn peek_ok(&self, bucket: RateBucket) -> (bool, Option<u64>) {
        if !self.enabled {
            return (true, None);
        }
        let now = Instant::now();
        let (base_cap, base_refill) = bucket.base_params();
        let cap = (base_cap * self.multiplier).max(1.0);
        let refill = (base_refill * self.multiplier).max(0.01);
        let map = self.buckets.lock();
        let tokens = match map.get(&bucket) {
            Some(st) => {
                let elapsed = now.saturating_duration_since(st.last_refill).as_secs_f64();
                (st.tokens + elapsed * refill).min(cap)
            }
            None => cap,
        };
        if tokens >= 1.0 {
            (true, None)
        } else {
            let wait_ms = (((1.0 - tokens) / refill) * 1000.0).ceil().max(50.0) as u64;
            (false, Some(wait_ms))
        }
    }

    /// Returns remaining capacities for all rate-limit buckets without mutating them.
    pub fn snapshot(&self) -> Value {
        let now = Instant::now();
        let map = self.buckets.lock();
        let mut buckets_json = serde_json::Map::new();
        for &bucket in RateBucket::ALL {
            let (base_cap, base_refill) = bucket.base_params();
            let cap = (base_cap * self.multiplier).max(1.0);
            let refill = (base_refill * self.multiplier).max(0.01);
            let remaining = match map.get(&bucket) {
                Some(st) => {
                    let elapsed = now.saturating_duration_since(st.last_refill).as_secs_f64();
                    (st.tokens + elapsed * refill).min(cap)
                }
                None => cap,
            };
            buckets_json.insert(
                bucket.as_str().to_owned(),
                json!({
                    "remaining": (remaining * 100.0).round() / 100.0,
                    "capacity": cap,
                    "refill_per_sec": refill,
                }),
            );
        }
        json!({
            "enabled": self.enabled,
            "multiplier": self.multiplier,
            "buckets": buckets_json,
        })
    }
}

/// Maps outgoing viewer requests to their rate-limit category, if any.
pub fn rate_bucket_for(req: &ViewerRequest) -> Option<RateBucket> {
    match req {
        ViewerRequest::CommsSend { target_uuid: None, .. } => Some(RateBucket::Chat),
        ViewerRequest::CommsSend { target_uuid: Some(_), .. } => Some(RateBucket::Im),
        ViewerRequest::ImSessionSend { action, .. } if !action.eq_ignore_ascii_case("leave") => Some(RateBucket::Im),
        ViewerRequest::NavTeleport { .. }
        | ViewerRequest::TeleportOffer { .. }
        | ViewerRequest::TeleportLandmark { .. }
        | ViewerRequest::TeleportHome => Some(RateBucket::Teleport),
        ViewerRequest::UiInvokeMenu { name } if name.eq_ignore_ascii_case("world.teleport_home") => Some(RateBucket::Teleport),
        ViewerRequest::PrimRezNew { .. } | ViewerRequest::InventoryRezObject { .. } | ViewerRequest::PrimDuplicate { .. } => {
            Some(RateBucket::Rez)
        }
        ViewerRequest::GiveInventory { .. } => Some(RateBucket::GiveInventory),
        ViewerRequest::ParcelEject { .. }
        | ViewerRequest::ParcelAccessListModify { .. }
        | ViewerRequest::ParcelSetMusic { .. }
        | ViewerRequest::ParcelSetProperties { .. } => Some(RateBucket::ParcelAdmin),
        ViewerRequest::EconomyPayAvatar { .. } | ViewerRequest::EconomyPayObject { .. } => Some(RateBucket::Payment),
        ViewerRequest::LslWriteAndCompile { .. } => Some(RateBucket::ScriptCompile),
        ViewerRequest::ObjectTouch { .. } => Some(RateBucket::Touch),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn burst_is_allowed_then_rejected_with_retry_after_and_refills() {
        let limiter = RateLimiter::new(true, 1.0);
        let t0 = Instant::now();
        // Teleport burst is 3
        for _ in 0..3 {
            limiter.check_at(RateBucket::Teleport, t0).expect("within burst");
        }
        let err = limiter.check_at(RateBucket::Teleport, t0).expect_err("4th must trip");
        match err {
            McpError::RateLimited {
                category, retry_after_ms, ..
            } => {
                assert_eq!(category, "teleport");
                assert!(retry_after_ms >= 3500, "expected ~4000ms wait, got {retry_after_ms}");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }

        // Other buckets remain independent
        limiter.check_at(RateBucket::Chat, t0).expect("chat bucket unaffected");

        // After 4.5s at 0.25/s, 1 token has refilled
        let t1 = t0 + Duration::from_millis(4500);
        limiter.check_at(RateBucket::Teleport, t1).expect("refilled after wait");
    }

    #[test]
    fn disabled_and_multiplier_modes_work() {
        let disabled = RateLimiter::disabled();
        let t0 = Instant::now();
        for _ in 0..50 {
            disabled.check_at(RateBucket::Teleport, t0).expect("disabled allows all");
        }

        let doubled = RateLimiter::new(true, 2.0);
        for _ in 0..6 {
            doubled.check_at(RateBucket::Teleport, t0).expect("2x burst allows 6");
        }
        assert!(matches!(
            doubled.check_at(RateBucket::Teleport, t0),
            Err(McpError::RateLimited { .. })
        ));
    }

    #[test]
    fn snapshot_and_peek_ok_do_not_consume_tokens() {
        let limiter = RateLimiter::new(true, 1.0);
        let snap1 = limiter.snapshot();
        assert_eq!(snap1["enabled"], json!(true));
        assert_eq!(snap1["buckets"]["teleport"]["remaining"], json!(3.0));
        assert_eq!(limiter.peek_ok(RateBucket::Teleport), (true, None));
        let snap2 = limiter.snapshot();
        assert_eq!(snap2["buckets"]["teleport"]["remaining"], json!(3.0));
    }
}
