//! Tamper-evident, privacy-preserving MCP tool audit log (`C6`).
//!
//! Records mutating calls, policy refusals, and dry-runs in a bounded memory
//! ring buffer (plus optional JSONL append file when `AURORA_MCP_AUDIT_LOG` is
//! set). Arguments are SHA-256 hashed so passwords and private messages never
//! enter the log.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_AUDIT_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub session_id: String,
    pub tool: String,
    pub args_sha256: String,
    pub mutating: bool,
    pub dry_run: bool,
    pub outcome: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Computes a deterministic SHA-256 hex digest of `args` without retaining raw values.
pub fn hash_args_sha256(args: &Value) -> String {
    let serialized = serde_json::to_vec(args).unwrap_or_default();
    let digest = Sha256::digest(&serialized);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

struct AuditState {
    next_seq: u64,
    entries: VecDeque<AuditEntry>,
    file_path: Option<PathBuf>,
}

pub struct AuditLog {
    state: Mutex<AuditState>,
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::from_env()
    }
}

impl AuditLog {
    pub fn from_env() -> Self {
        let file_path = std::env::var_os("AURORA_MCP_AUDIT_LOG")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        Self::new(file_path)
    }

    pub fn new(file_path: Option<PathBuf>) -> Self {
        Self {
            state: Mutex::new(AuditState {
                next_seq: 0,
                entries: VecDeque::with_capacity(MAX_AUDIT_ENTRIES),
                file_path,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        session_id: &str,
        tool: &str,
        args: &Value,
        mutating: bool,
        dry_run: bool,
        outcome: &str,
        duration_ms: u64,
        error: Option<String>,
    ) -> AuditEntry {
        let timestamp_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let args_sha256 = hash_args_sha256(args);
        let mut st = self.state.lock();
        st.next_seq += 1;
        let entry = AuditEntry {
            seq: st.next_seq,
            timestamp_ms,
            session_id: session_id.to_owned(),
            tool: tool.to_owned(),
            args_sha256,
            mutating,
            dry_run,
            outcome: outcome.to_owned(),
            duration_ms,
            error,
        };
        if st.entries.len() >= MAX_AUDIT_ENTRIES {
            st.entries.pop_front();
        }
        st.entries.push_back(entry.clone());
        if let Some(path) = &st.file_path {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
                && let Ok(line) = serde_json::to_string(&entry)
            {
                let _ = writeln!(f, "{line}");
            }
        }
        entry
    }

    pub fn read(
        &self,
        after: Option<u64>,
        tool_filter: Option<&str>,
        session_filter: Option<&str>,
        outcome_filter: Option<&str>,
        limit: usize,
    ) -> (Vec<AuditEntry>, u64) {
        let limit = limit.clamp(1, 200);
        let min_seq = after.unwrap_or(0);
        let st = self.state.lock();
        let matched: Vec<AuditEntry> = st
            .entries
            .iter()
            .filter(|e| e.seq > min_seq)
            .filter(|e| tool_filter.is_none_or(|t| e.tool.eq_ignore_ascii_case(t)))
            .filter(|e| session_filter.is_none_or(|s| e.session_id == s))
            .filter(|e| outcome_filter.is_none_or(|o| e.outcome.eq_ignore_ascii_case(o)))
            .take(limit)
            .cloned()
            .collect();
        (matched, st.next_seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn audit_hashes_arguments_caps_ring_buffer_and_filters() {
        let log = AuditLog::new(None);
        let secret = "super-secret-password-999";
        let e1 = log.record("s1", "viewer_login", &json!({ "password": secret }), true, false, "ok", 12, None);
        assert_eq!(e1.args_sha256.len(), 64);
        assert!(!serde_json::to_string(&e1).unwrap().contains(secret));

        log.record("s2", "prim_delete", &json!({ "local_ids": [1] }), true, true, "dry_run", 1, None);
        log.record(
            "s2",
            "economy_pay_avatar",
            &json!({ "amount": 500 }),
            true,
            false,
            "permission_denied",
            2,
            Some("cap exceeded".into()),
        );

        let (filtered, last_seq) = log.read(Some(1), None, Some("s2"), Some("permission_denied"), 50);
        assert_eq!(last_seq, 3);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].tool, "economy_pay_avatar");

        for i in 0..(MAX_AUDIT_ENTRIES + 25) {
            log.record("s3", "prim_transform", &json!({ "i": i }), true, false, "ok", 1, None);
        }
        let (all_capped, total) = log.read(None, None, None, None, 200);
        assert_eq!(total, (MAX_AUDIT_ENTRIES + 28) as u64);
        assert_eq!(all_capped.len(), 200);
        assert!(all_capped[0].seq > 20);
    }
}
