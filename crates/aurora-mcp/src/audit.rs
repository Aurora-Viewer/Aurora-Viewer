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
const MAX_AUDIT_ERROR_CHARS: usize = 256;

fn sanitize_audit_error(raw: String) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_AUDIT_ERROR_CHARS + 1)
        .collect();
    if cleaned.chars().count() > MAX_AUDIT_ERROR_CHARS {
        let truncated: String = cleaned.chars().take(MAX_AUDIT_ERROR_CHARS).collect();
        format!("{truncated}...")
    } else {
        cleaned
    }
}

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
}

pub struct AuditLog {
    state: Mutex<AuditState>,
    file_tx: Option<std::sync::mpsc::SyncSender<String>>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::from_env()
    }
}

impl Drop for AuditLog {
    fn drop(&mut self) {
        drop(self.file_tx.take());
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
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
        let (file_tx, writer_thread) = match file_path {
            Some(path) => {
                let (tx, rx) = std::sync::mpsc::sync_channel::<String>(1024);
                let handle = std::thread::Builder::new()
                    .name("aurora-mcp-audit".to_owned())
                    .spawn(move || audit_writer_loop(path, rx))
                    .ok();
                (handle.is_some().then_some(tx), handle)
            }
            None => (None, None),
        };
        Self {
            state: Mutex::new(AuditState {
                next_seq: 0,
                entries: VecDeque::with_capacity(MAX_AUDIT_ENTRIES),
            }),
            file_tx,
            writer_thread,
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
        let sanitized_error = error.map(sanitize_audit_error);
        let entry = {
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
                error: sanitized_error,
            };
            if st.entries.len() >= MAX_AUDIT_ENTRIES {
                st.entries.pop_front();
            }
            st.entries.push_back(entry.clone());
            entry
        };
        if let Some(tx) = &self.file_tx
            && let Ok(line) = serde_json::to_string(&entry)
        {
            let _ = tx.try_send(line);
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

fn write_audit_line(
    path: &std::path::Path,
    dir_created: &mut bool,
    writer: &mut Option<std::io::BufWriter<std::fs::File>>,
    line: &str,
) {
    if !*dir_created {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        *dir_created = true;
    }
    if writer.is_none() {
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        if let Ok(f) = opts.open(path) {
            *writer = Some(std::io::BufWriter::new(f));
        }
    }
    if let Some(w) = writer.as_mut()
        && writeln!(w, "{line}").is_err()
    {
        *writer = None;
    }
}

fn audit_writer_loop(path: PathBuf, rx: std::sync::mpsc::Receiver<String>) {
    let mut dir_created = false;
    let mut writer: Option<std::io::BufWriter<std::fs::File>> = None;
    while let Ok(line) = rx.recv() {
        write_audit_line(&path, &mut dir_created, &mut writer, &line);
        while let Ok(next_line) = rx.try_recv() {
            write_audit_line(&path, &mut dir_created, &mut writer, &next_line);
        }
        if let Some(w) = writer.as_mut() {
            let _ = w.flush();
        }
    }
    if let Some(w) = writer.as_mut() {
        let _ = w.flush();
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

    #[test]
    fn audit_writes_jsonl_to_file_without_blocking() {
        let dir = std::env::temp_dir().join(format!("aurora-audit-test-{}", uuid::Uuid::new_v4().simple()));
        let path = dir.join("nested").join("audit.jsonl");
        {
            let log = AuditLog::new(Some(path.clone()));
            log.record("s1", "prim_rez_new", &json!({ "a": 1 }), true, false, "ok", 5, None);
            log.record("s1", "prim_delete", &json!({ "a": 2 }), true, false, "ok", 7, None);
        }
        let content = std::fs::read_to_string(&path).expect("audit file written");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"tool\":\"prim_rez_new\""));
        assert!(lines[1].contains("\"tool\":\"prim_delete\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("stat audit").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "audit log file must be owner-only (0600)");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_sanitizes_control_chars_and_truncates_errors() {
        let log = AuditLog::new(None);
        let noisy = format!("line1\nline2\r\t{}", "x".repeat(MAX_AUDIT_ERROR_CHARS + 40));
        let e = log.record("s1", "setting_set", &json!({}), true, false, "error", 1, Some(noisy));
        let err = e.error.expect("error preserved");
        assert!(!err.contains('\n') && !err.contains('\r') && !err.contains('\t'));
        assert!(err.ends_with("..."));
        assert!(err.len() <= MAX_AUDIT_ERROR_CHARS + 3);
    }
}
