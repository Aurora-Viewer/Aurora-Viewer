//! Second Life / OpenSim Linkset Data (LSD) Store & Stress-Testing Engine.
//!
//! Enforces exact LSL Linkset Data specification limits:
//! - Total store capacity: `131,072` bytes (128 KiB) per linkset root prim.
//! - UTF-8 byte accounting for keys, values, and protected passphrase hashes.
//! - Regular expression key search (`llLinksetDataFindKeys`) and Mono 64 KiB
//!   string/list serialization checks.

use crate::bridge::{LsdOperation, McpError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Maximum Linkset Data storage per linkset in Second Life (128 KiB).
pub const LSD_MAX_BYTES: usize = 131_072;
/// Maximum LSL string payload safely handled by a single Mono script event/call.
pub const LSL_MONO_SAFE_STRING_BYTES: usize = 32_768;

/// Return codes matching LSL `LINKSETDATA_*` constants.
pub const LINKSETDATA_OK: i32 = 0;
pub const LINKSETDATA_EMEMORY: i32 = 1;
pub const LINKSETDATA_ENOKEY: i32 = 2;
pub const LINKSETDATA_EPROTECTED: i32 = 3;
pub const LINKSETDATA_MULTIDELETE: i32 = 4;
pub const LINKSETDATA_NOTFOUND: i32 = 5;
pub const LINKSETDATA_NOUPDATE: i32 = 6;

/// Human-readable LSL constant name for a `LINKSETDATA_*` status code.
pub fn lsd_status_name(code: i32) -> &'static str {
    match code {
        LINKSETDATA_OK => "LINKSETDATA_OK",
        LINKSETDATA_EMEMORY => "LINKSETDATA_EMEMORY",
        LINKSETDATA_ENOKEY => "LINKSETDATA_ENOKEY",
        LINKSETDATA_EPROTECTED => "LINKSETDATA_EPROTECTED",
        LINKSETDATA_MULTIDELETE => "LINKSETDATA_MULTIDELETE",
        LINKSETDATA_NOTFOUND => "LINKSETDATA_NOTFOUND",
        LINKSETDATA_NOUPDATE => "LINKSETDATA_NOUPDATE",
        _ => "unknown",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LsdEntry {
    pub value: String,
    /// SHA-256 hex digest of the passphrase if written via `llLinksetDataWriteProtected`.
    pub pass_hash: Option<String>,
}

impl LsdEntry {
    /// Byte cost in the simulator's Linkset Data store: UTF-8 key bytes +
    /// UTF-8 value bytes + 32 bytes overhead when protected by a passphrase.
    pub fn byte_cost(&self, key: &str) -> usize {
        key.len() + self.value.len() + if self.pass_hash.is_some() { 32 } else { 0 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinksetDataStore {
    entries: BTreeMap<String, LsdEntry>,
}

impl LinksetDataStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn used_bytes(&self) -> usize {
        self.entries.iter().map(|(k, v)| v.byte_cost(k)).sum()
    }

    pub fn available_bytes(&self) -> usize {
        LSD_MAX_BYTES.saturating_sub(self.used_bytes())
    }

    pub fn count_keys(&self) -> usize {
        self.entries.len()
    }

    pub fn reset(&mut self) {
        self.entries.clear();
    }

    pub fn write(&mut self, key: &str, value: &str, passphrase: Option<&str>) -> i32 {
        if key.is_empty() {
            return LINKSETDATA_ENOKEY;
        }
        let pass = passphrase.filter(|p| !p.is_empty());
        let pass_hash = pass.map(hash_passphrase);

        if let Some(existing) = self.entries.get(key) {
            if existing.pass_hash != pass_hash {
                return LINKSETDATA_EPROTECTED;
            }
            if value.is_empty() {
                self.entries.remove(key);
                return LINKSETDATA_OK;
            }
            if existing.value == value {
                return LINKSETDATA_NOUPDATE;
            }
        } else if value.is_empty() {
            return LINKSETDATA_NOTFOUND;
        }

        let candidate = LsdEntry {
            value: value.to_owned(),
            pass_hash,
        };
        let old_cost = self.entries.get(key).map_or(0, |e| e.byte_cost(key));
        let new_cost = candidate.byte_cost(key);
        let next_used = self.used_bytes().saturating_sub(old_cost) + new_cost;
        if next_used > LSD_MAX_BYTES {
            return LINKSETDATA_EMEMORY;
        }
        self.entries.insert(key.to_owned(), candidate);
        LINKSETDATA_OK
    }

    pub fn delete(&mut self, key: &str, passphrase: Option<&str>) -> i32 {
        let Some(existing) = self.entries.get(key) else {
            return LINKSETDATA_NOTFOUND;
        };
        let pass = passphrase.filter(|p| !p.is_empty());
        let pass_hash = pass.map(hash_passphrase);
        if existing.pass_hash != pass_hash {
            return LINKSETDATA_EPROTECTED;
        }
        self.entries.remove(key);
        LINKSETDATA_OK
    }

    pub fn read(&self, key: &str, passphrase: Option<&str>) -> Result<Option<String>, i32> {
        let Some(existing) = self.entries.get(key) else {
            return Ok(None);
        };
        let pass = passphrase.filter(|p| !p.is_empty());
        let pass_hash = pass.map(hash_passphrase);
        if existing.pass_hash != pass_hash {
            return Err(LINKSETDATA_EPROTECTED);
        }
        Ok(Some(existing.value.clone()))
    }

    pub fn inspect(
        &self,
        filter_regex: Option<&str>,
        include_protected: bool,
        passphrase: Option<&str>,
        limit: usize,
    ) -> Result<Value, McpError> {
        let re = match filter_regex.filter(|s| !s.is_empty()) {
            Some(pattern) => Some(Regex::new(pattern).map_err(|e| McpError::InvalidParams(e.to_string()))?),
            None => None,
        };
        let pass_hash = passphrase.filter(|p| !p.is_empty()).map(hash_passphrase);
        let cap = limit.clamp(1, 100);
        let mut matched_total = 0usize;
        let mut items = Vec::new();

        for (key, entry) in &self.entries {
            if let Some(re) = &re
                && !re.is_match(key)
            {
                continue;
            }
            let is_protected = entry.pass_hash.is_some();
            if is_protected && !include_protected && pass_hash.is_none() {
                continue;
            }
            matched_total += 1;
            if items.len() < cap {
                let unlocked = !is_protected || entry.pass_hash == pass_hash;
                items.push(json!({
                    "key": key,
                    "protected": is_protected,
                    "unlocked": unlocked,
                    "value_bytes": entry.value.len(),
                    "total_cost_bytes": entry.byte_cost(key),
                    "value": if unlocked {
                        Value::String(entry.value.clone())
                    } else {
                        Value::String("<protected: passphrase required>".into())
                    }
                }));
            }
        }

        let used = self.used_bytes();
        Ok(json!({
            "capacity_bytes": LSD_MAX_BYTES,
            "used_bytes": used,
            "available_bytes": LSD_MAX_BYTES.saturating_sub(used),
            "utilization_pct": ((used as f64 / LSD_MAX_BYTES as f64) * 10000.0).round() / 100.0,
            "total_keys": self.entries.len(),
            "matched_keys": matched_total,
            "returned_keys": items.len(),
            "entries": items,
        }))
    }

    pub fn apply_operations(&mut self, ops: &[LsdOperation]) -> Value {
        let mut results = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                LsdOperation::Write { key, value, passphrase } => {
                    let rc = self.write(key, value, passphrase.as_deref());
                    results.push(json!({
                        "op": "write",
                        "key": key,
                        "status_code": rc,
                        "ok": rc == LINKSETDATA_OK || rc == LINKSETDATA_NOUPDATE,
                    }));
                }
                LsdOperation::Delete { key, passphrase } => {
                    let rc = self.delete(key, passphrase.as_deref());
                    results.push(json!({
                        "op": "delete",
                        "key": key,
                        "status_code": rc,
                        "ok": rc == LINKSETDATA_OK,
                    }));
                }
                LsdOperation::Reset => {
                    self.reset();
                    results.push(json!({
                        "op": "reset",
                        "status_code": LINKSETDATA_OK,
                        "ok": true,
                    }));
                }
                LsdOperation::ImportJson { entries, passphrase } => {
                    let mut written = 0usize;
                    let mut failed = 0usize;
                    for (k, v) in entries {
                        let rc = self.write(k, v, passphrase.as_deref());
                        if rc == LINKSETDATA_OK || rc == LINKSETDATA_NOUPDATE {
                            written += 1;
                        } else {
                            failed += 1;
                        }
                    }
                    results.push(json!({
                        "op": "import_json",
                        "written": written,
                        "failed": failed,
                        "ok": failed == 0,
                    }));
                }
            }
        }
        let used = self.used_bytes();
        json!({
            "used_bytes": used,
            "available_bytes": LSD_MAX_BYTES.saturating_sub(used),
            "total_keys": self.entries.len(),
            "results": results,
        })
    }
}

fn hash_passphrase(pass: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pass.as_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Offline Linkset Data & JSON payload stress tester (`lsd_stress_test`).
///
/// Validates whether a proposed JSON object or KV table fits within Second Life's
/// 128 KiB LSD ceiling and checks individual keys/values against LSL Mono's
/// string heap constraints.
pub fn stress_test_payload(payload: &Value, protected_keys: bool, filter_regex: Option<&str>) -> Result<Value, McpError> {
    let mut store = LinksetDataStore::new();
    let mut oversized_values = Vec::new();
    let mut rejected_keys = Vec::new();
    let pass = protected_keys.then_some("stress-test-passphrase");

    match payload {
        Value::Object(map) => {
            for (k, v) in map {
                let val_str = match v {
                    Value::String(s) => s.clone(),
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                if val_str.len() > LSL_MONO_SAFE_STRING_BYTES {
                    oversized_values.push(json!({
                        "key": k,
                        "bytes": val_str.len(),
                        "warning": "exceeds safe 32 KiB LSL Mono single-string threshold"
                    }));
                }
                let rc = store.write(k, &val_str, pass);
                if rc != LINKSETDATA_OK && rc != LINKSETDATA_NOUPDATE {
                    rejected_keys.push(json!({
                        "key": k,
                        "bytes": k.len() + val_str.len(),
                        "status_code": rc,
                    }));
                }
            }
        }
        other => {
            let serialized = serde_json::to_string(other).unwrap_or_default();
            let rc = store.write("root", &serialized, pass);
            if rc != LINKSETDATA_OK {
                rejected_keys.push(json!({
                    "key": "root",
                    "bytes": serialized.len(),
                    "status_code": rc,
                }));
            }
        }
    }

    let regex_matches = if let Some(pattern) = filter_regex.filter(|p| !p.is_empty()) {
        let re = Regex::new(pattern).map_err(|e| McpError::InvalidParams(e.to_string()))?;
        store.entries.keys().filter(|k| re.is_match(k)).count()
    } else {
        store.count_keys()
    };

    let used = store.used_bytes();
    Ok(json!({
        "fits_in_128kib": rejected_keys.is_empty() && used <= LSD_MAX_BYTES,
        "capacity_bytes": LSD_MAX_BYTES,
        "used_bytes": used,
        "available_bytes": LSD_MAX_BYTES.saturating_sub(used),
        "utilization_pct": ((used as f64 / LSD_MAX_BYTES as f64) * 10000.0).round() / 100.0,
        "stored_keys": store.count_keys(),
        "regex_matched_keys": regex_matches,
        "rejected_keys": rejected_keys,
        "mono_heap_warnings": oversized_values,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_linkset_data_store_enforces_128kib_quota_and_protection() {
        let mut store = LinksetDataStore::new();
        assert_eq!(store.write("hp", "100/100", None), LINKSETDATA_OK);
        assert_eq!(store.write("secret", "token-xyz", Some("pin123")), LINKSETDATA_OK);

        // Protected key cannot be read or overwritten without the passphrase.
        assert_eq!(store.read("secret", None), Err(LINKSETDATA_EPROTECTED));
        assert_eq!(store.read("secret", Some("pin123")), Ok(Some("token-xyz".to_owned())));
        assert_eq!(store.write("secret", "hacked", None), LINKSETDATA_EPROTECTED);

        // Overflowing 131,072 bytes returns LINKSETDATA_EMEMORY.
        let huge = "x".repeat(LSD_MAX_BYTES);
        assert_eq!(store.write("overflow", &huge, None), LINKSETDATA_EMEMORY);
    }

    #[test]
    fn mcp_stress_tester_reports_utilization_and_regex() {
        let payload = json!({
            "char:stats:str": 18,
            "char:stats:dex": 14,
            "hud:theme": "aurora-dark"
        });
        let report = stress_test_payload(&payload, true, Some("^char:stats:")).expect("stress test");
        assert_eq!(report["fits_in_128kib"], true);
        assert_eq!(report["stored_keys"], 3);
        assert_eq!(report["regex_matched_keys"], 2);
    }
}
