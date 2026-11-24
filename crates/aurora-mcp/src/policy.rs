//! Unified autonomy policy, configuration (`~/.config/aurora/mcp-policy.toml`),
//! and opt-in supervised approval queue (`Part D`, `A11`, `C6`).
//!
//! By default (`AURORA_MCP_POLICY=autonomous`), **no tool asks for human
//! approval**: safety is enforced via budgets, allowlists, rate limits, and
//! `dry_run`. On-screen action approvals activate only when `supervised` is
//! explicitly enabled.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

const APPROVAL_TTL: Duration = Duration::from_secs(120);

/// Operating policy mode for MCP tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    /// Default: agents act autonomously within configured budgets, allowlists, and rate limits.
    #[default]
    Autonomous,
    /// High-risk irreversible/costly actions pause for one-click approval in the viewer control bar.
    Supervised,
    /// Read-only mode: all lease-requiring / mutating actions are refused.
    Locked,
}

impl PolicyMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "autonomous" | "auto" => Some(Self::Autonomous),
            "supervised" | "ask" => Some(Self::Supervised),
            "locked" | "readonly" | "read_only" | "hold" => Some(Self::Locked),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Autonomous => "autonomous",
            Self::Supervised => "supervised",
            Self::Locked => "locked",
        }
    }
}

/// Resolved policy loaded from optional config file + environment overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPolicy {
    pub mode: PolicyMode,
    pub max_pay_lindens: i32,
    pub session_pay_limit: i32,
    pub upload_budget: i32,
    pub debit_allow: String,
    pub settings_allow: Vec<String>,
    pub ask_deny_on_timeout: bool,
    pub destructive_hints: bool,
    pub supervised_pay_threshold: i32,
    pub supervised_upload_threshold: i32,
    pub policy_path: Option<PathBuf>,
}

impl Default for McpPolicy {
    fn default() -> Self {
        Self {
            mode: PolicyMode::Autonomous,
            max_pay_lindens: 100,
            session_pay_limit: 500,
            upload_budget: 1000,
            debit_allow: "self".to_owned(),
            settings_allow: Vec::new(),
            ask_deny_on_timeout: false,
            destructive_hints: true,
            supervised_pay_threshold: 10,
            supervised_upload_threshold: 10,
            policy_path: None,
        }
    }
}

impl McpPolicy {
    /// Loads policy from `$AURORA_MCP_POLICY_FILE` (or `~/.config/aurora/mcp-policy.{toml,json}`)
    /// and applies `AURORA_MCP_*` environment variable overrides on top.
    pub fn from_env_and_file() -> Self {
        let mut policy = Self::default();
        if let Some(path) = discover_policy_file()
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            let map = if path.extension().and_then(|e| e.to_str()) == Some("json") {
                parse_json_flat(&text)
            } else {
                parse_simple_toml(&text)
            };
            policy.apply_map(&map);
            policy.policy_path = Some(path);
        }
        policy.apply_env();
        policy
    }

    fn apply_map(&mut self, map: &HashMap<String, Value>) {
        if let Some(m) = get_str(map, "mode").or_else(|| get_str(map, "policy"))
            && let Some(parsed) = PolicyMode::parse(&m)
        {
            self.mode = parsed;
        }
        if let Some(v) = get_i32(map, "max_pay_lindens") {
            self.max_pay_lindens = v.max(0);
        }
        if let Some(v) = get_i32(map, "session_pay_limit") {
            self.session_pay_limit = v.max(0);
        }
        if let Some(v) = get_i32(map, "upload_budget") {
            self.upload_budget = v.max(0);
        }
        if let Some(v) = get_str_or_list_csv(map, "debit_allow") {
            self.debit_allow = v;
        }
        if let Some(list) = get_str_list(map, "settings_allow") {
            for item in list {
                if !self.settings_allow.iter().any(|x| x.eq_ignore_ascii_case(&item)) {
                    self.settings_allow.push(item);
                }
            }
        }
        if let Some(v) = get_bool(map, "ask_deny_on_timeout") {
            self.ask_deny_on_timeout = v;
        }
        if let Some(v) = get_bool(map, "destructive_hints") {
            self.destructive_hints = v;
        }
        if let Some(v) = get_i32(map, "supervised_pay_threshold") {
            self.supervised_pay_threshold = v.max(0);
        }
        if let Some(v) = get_i32(map, "supervised_upload_threshold") {
            self.supervised_upload_threshold = v.max(0);
        }
    }

    fn apply_env(&mut self) {
        if let Ok(raw) = std::env::var("AURORA_MCP_POLICY")
            && let Some(parsed) = PolicyMode::parse(&raw)
        {
            self.mode = parsed;
        }
        if let Some(v) = env_i32("AURORA_MCP_MAX_PAY_LINDENS") {
            self.max_pay_lindens = v.max(0);
        }
        if let Some(v) = env_i32("AURORA_MCP_SESSION_PAY_LIMIT") {
            self.session_pay_limit = v.max(0);
        }
        if let Some(v) = env_i32("AURORA_MCP_UPLOAD_BUDGET") {
            self.upload_budget = v.max(0);
        }
        if let Ok(raw) = std::env::var("AURORA_MCP_DEBIT_ALLOW")
            && !raw.trim().is_empty()
        {
            self.debit_allow = raw.trim().to_owned();
        }
        if let Ok(raw) = std::env::var("AURORA_MCP_SETTINGS_ALLOW") {
            for item in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if !self.settings_allow.iter().any(|x| x.eq_ignore_ascii_case(item)) {
                    self.settings_allow.push(item.to_owned());
                }
            }
        }
        if let Some(v) = env_bool("AURORA_MCP_ASK_DENY_ON_TIMEOUT") {
            self.ask_deny_on_timeout = v;
        }
        if let Some(v) = env_bool("AURORA_MCP_DESTRUCTIVE_HINTS") {
            self.destructive_hints = v;
        }
        if let Some(v) = env_i32("AURORA_MCP_SUPERVISED_PAY_THRESHOLD") {
            self.supervised_pay_threshold = v.max(0);
        }
        if let Some(v) = env_i32("AURORA_MCP_SUPERVISED_UPLOAD_THRESHOLD") {
            self.supervised_upload_threshold = v.max(0);
        }
    }

    /// Comma-separated `settings_allow` list compatible with `mcp_setting_protected`.
    pub fn settings_allow_csv(&self) -> String {
        self.settings_allow.join(",")
    }

    /// Returns a human-readable approval summary if `mode == Supervised` and
    /// `(tool, args)` crosses the supervised threshold; `None` in `Autonomous` or `Locked`.
    pub fn requires_supervised_approval(&self, tool: &str, args: &Value) -> Option<String> {
        if self.mode != PolicyMode::Supervised {
            return None;
        }
        match tool {
            "economy_pay_avatar" => {
                let amount = args
                    .get("amount")
                    .or_else(|| args.get("amount_l$"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                (amount > i64::from(self.supervised_pay_threshold)).then(|| format!("Pay avatar L${amount}"))
            }
            "economy_pay_object" => {
                let amount = args
                    .get("amount")
                    .or_else(|| args.get("amount_l$"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let lid = args.get("target_local_id").and_then(Value::as_u64).unwrap_or(0);
                (amount > i64::from(self.supervised_pay_threshold)).then(|| format!("Pay object {lid} L${amount}"))
            }
            "asset_upload" => {
                let action = args.get("action").and_then(Value::as_str).unwrap_or("");
                matches!(action.to_ascii_lowercase().as_str(), "upload" | "start")
                    .then(|| "Upload staged assets to grid inventory".to_owned())
            }
            "script_prompt_respond" => {
                (args.get("grant").and_then(Value::as_bool) == Some(true)).then(|| "Grant script permission request".to_owned())
            }
            "parcel_eject" => Some("Eject / ban avatar from parcel".to_owned()),
            "give_inventory" => Some("Give inventory item to another avatar".to_owned()),
            "inventory_delete" => Some("Move inventory items to Trash".to_owned()),
            "prim_delete" => Some("Delete in-world prims".to_owned()),
            "viewer_shutdown" => Some("Shut down Aurora Viewer".to_owned()),
            "object_take_or_derez" => {
                let action = args.get("action").and_then(Value::as_str).unwrap_or("");
                matches!(action.to_ascii_lowercase().as_str(), "delete" | "derez").then(|| "Delete / derez in-world object".to_owned())
            }
            _ => None,
        }
    }
}

/// Checks whether MCP `destructiveHint` annotations should be emitted (`true` by default).
pub fn destructive_hints_enabled() -> bool {
    McpPolicy::from_env_and_file().destructive_hints
}

/// Public snapshot of an action awaiting human approval in `Supervised` mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApprovalView {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub summary: String,
    pub waiting_secs: u64,
}

#[derive(Debug, Clone)]
pub enum ApprovalCheck {
    Approved,
    Pending(PendingApprovalView),
    Denied,
}

#[derive(Debug, Clone)]
struct PendingItem {
    id: String,
    key: (String, String, String),
    summary: String,
    created: Instant,
}

#[derive(Default)]
struct ApprovalState {
    pending: Vec<PendingItem>,
    approved: HashMap<(String, String, String), Instant>,
    denied: HashMap<(String, String, String), Instant>,
}

/// Queue of high-risk action requests used only under `PolicyMode::Supervised`.
#[derive(Default)]
pub struct ApprovalQueue {
    state: Mutex<ApprovalState>,
    has_pending: AtomicBool,
}

impl ApprovalQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_pending(&self) -> bool {
        self.has_pending.load(Ordering::Acquire)
    }

    fn sync_has_pending(&self, st: &ApprovalState) {
        self.has_pending.store(!st.pending.is_empty(), Ordering::Release);
    }

    fn prune(st: &mut ApprovalState, now: Instant) {
        st.pending.retain(|p| now.duration_since(p.created) < APPROVAL_TTL);
        st.approved.retain(|_, at| now.duration_since(*at) < APPROVAL_TTL);
        st.denied.retain(|_, at| now.duration_since(*at) < APPROVAL_TTL);
    }

    /// Consumes a pre-approved matching request, reports a prior denial, or
    /// queues/returns a pending approval handle for the viewer UI.
    pub fn check_or_request(&self, session_id: &str, tool: &str, args_hash: &str, summary: &str) -> ApprovalCheck {
        let now = Instant::now();
        let key = (session_id.to_owned(), tool.to_owned(), args_hash.to_owned());
        let mut st = self.state.lock();
        Self::prune(&mut st, now);

        if st.approved.remove(&key).is_some() {
            self.sync_has_pending(&st);
            return ApprovalCheck::Approved;
        }
        if st.denied.contains_key(&key) {
            self.sync_has_pending(&st);
            return ApprovalCheck::Denied;
        }
        if let Some(existing) = st.pending.iter().find(|p| p.key == key) {
            self.sync_has_pending(&st);
            return ApprovalCheck::Pending(PendingApprovalView {
                id: existing.id.clone(),
                session_id: existing.key.0.clone(),
                tool: existing.key.1.clone(),
                summary: existing.summary.clone(),
                waiting_secs: now.duration_since(existing.created).as_secs(),
            });
        }
        let item = PendingItem {
            id: Uuid::new_v4().to_string(),
            key,
            summary: summary.to_owned(),
            created: now,
        };
        let view = PendingApprovalView {
            id: item.id.clone(),
            session_id: item.key.0.clone(),
            tool: item.key.1.clone(),
            summary: item.summary.clone(),
            waiting_secs: 0,
        };
        st.pending.push(item);
        self.sync_has_pending(&st);
        ApprovalCheck::Pending(view)
    }

    /// Oldest pending approval request for the viewer's control bar overlay.
    pub fn next_pending(&self) -> Option<PendingApprovalView> {
        let now = Instant::now();
        let mut st = self.state.lock();
        Self::prune(&mut st, now);
        self.sync_has_pending(&st);
        st.pending.first().map(|p| PendingApprovalView {
            id: p.id.clone(),
            session_id: p.key.0.clone(),
            tool: p.key.1.clone(),
            summary: p.summary.clone(),
            waiting_secs: now.duration_since(p.created).as_secs(),
        })
    }

    pub fn approve(&self, id: &str) -> bool {
        let now = Instant::now();
        let mut st = self.state.lock();
        let Some(pos) = st.pending.iter().position(|p| p.id == id) else {
            return false;
        };
        let item = st.pending.remove(pos);
        st.denied.remove(&item.key);
        st.approved.insert(item.key, now);
        self.sync_has_pending(&st);
        true
    }

    pub fn deny(&self, id: &str) -> bool {
        let now = Instant::now();
        let mut st = self.state.lock();
        let Some(pos) = st.pending.iter().position(|p| p.id == id) else {
            return false;
        };
        let item = st.pending.remove(pos);
        st.approved.remove(&item.key);
        st.denied.insert(item.key, now);
        self.sync_has_pending(&st);
        true
    }
}

fn discover_policy_file() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("AURORA_MCP_POLICY_FILE").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(explicit);
        return p.exists().then_some(p);
    }
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    let aurora_dir = config_dir.join("aurora");
    let toml_path = aurora_dir.join("mcp-policy.toml");
    if toml_path.exists() {
        return Some(toml_path);
    }
    let json_path = aurora_dir.join("mcp-policy.json");
    json_path.exists().then_some(json_path)
}

fn parse_json_flat(text: &str) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else {
        return out;
    };
    for (k, v) in root {
        if let Value::Object(sub) = v {
            for (sk, sv) in sub {
                out.entry(sk).or_insert(sv);
            }
        } else {
            out.insert(k, v);
        }
    }
    out
}

/// Lightweight zero-dependency TOML subset parser supporting `#` comments,
/// `[section]` headers, strings, integers, booleans, and single-line string arrays.
pub fn parse_simple_toml(text: &str) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let mut section = String::new();
    for raw_line in text.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = inner.trim().to_ascii_lowercase();
            continue;
        }
        let Some((raw_k, raw_v)) = line.split_once('=') else {
            continue;
        };
        let key = raw_k.trim().to_ascii_lowercase();
        let Some(val) = parse_toml_value(raw_v.trim()) else {
            continue;
        };
        if !section.is_empty() {
            out.insert(format!("{section}.{key}"), val.clone());
        }
        out.entry(key).or_insert(val);
    }
    out
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_dq = false;
    let mut in_sq = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_dq => escaped = true,
            '"' if !in_sq => in_dq = !in_dq,
            '\'' if !in_dq => in_sq = !in_sq,
            '#' if !in_dq && !in_sq => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn parse_toml_value(raw: &str) -> Option<Value> {
    if raw == "true" {
        return Some(Value::Bool(true));
    }
    if raw == "false" {
        return Some(Value::Bool(false));
    }
    if let Ok(n) = raw.parse::<i64>() {
        return Some(Value::Number(n.into()));
    }
    if let Some(inner) = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Some(Value::String(inner.replace("\\\"", "\"").replace("\\\\", "\\")));
    }
    if let Some(inner) = raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Some(Value::String(inner.to_owned()));
    }
    if let Some(inner) = raw.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let mut arr = Vec::new();
        for part in inner.split(',') {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            if let Some(v) = parse_toml_value(p) {
                arr.push(v);
            }
        }
        return Some(Value::Array(arr));
    }
    None
}

fn get_str(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(|s| s.trim().to_owned())
}

fn get_i32(map: &HashMap<String, Value>, key: &str) -> Option<i32> {
    map.get(key).and_then(Value::as_i64).and_then(|n| i32::try_from(n).ok())
}

fn get_bool(map: &HashMap<String, Value>, key: &str) -> Option<bool> {
    map.get(key).and_then(Value::as_bool)
}

fn get_str_list(map: &HashMap<String, Value>, key: &str) -> Option<Vec<String>> {
    match map.get(key)? {
        Value::Array(arr) => Some(
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect(),
        ),
        Value::String(s) => Some(s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()),
        _ => None,
    }
}

fn get_str_or_list_csv(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    get_str_list(map, key).map(|list| list.join(","))
}

fn env_i32(key: &str) -> Option<i32> {
    std::env::var(key).ok().and_then(|s| s.trim().parse::<i32>().ok())
}

fn env_bool(key: &str) -> Option<bool> {
    let val = std::env::var(key).ok()?;
    match val.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_policy_is_autonomous_with_expected_caps() {
        let p = McpPolicy::default();
        assert_eq!(p.mode, PolicyMode::Autonomous);
        assert_eq!(p.max_pay_lindens, 100);
        assert_eq!(p.session_pay_limit, 500);
        assert_eq!(p.upload_budget, 1000);
        assert_eq!(p.debit_allow, "self");
        assert!(!p.ask_deny_on_timeout);
        assert!(p.destructive_hints);
        assert!(
            p.requires_supervised_approval("economy_pay_avatar", &json!({ "amount": 99 }))
                .is_none()
        );
    }

    #[test]
    fn simple_toml_parses_sections_comments_and_arrays() {
        let toml = r#"
# Aurora policy config
mode = "supervised"
ask_deny_on_timeout = true # strict timeout
destructive_hints = false

[limits]
max_pay_lindens = 250
upload_budget = 2000
debit_allow = ["self", "Trusted Creator"]
settings_allow = ["camera.fov", "draw_distance"]
"#;
        let map = parse_simple_toml(toml);
        let mut p = McpPolicy::default();
        p.apply_map(&map);

        assert_eq!(p.mode, PolicyMode::Supervised);
        assert!(p.ask_deny_on_timeout);
        assert!(!p.destructive_hints);
        assert_eq!(p.max_pay_lindens, 250);
        assert_eq!(p.upload_budget, 2000);
        assert_eq!(p.debit_allow, "self,Trusted Creator");
        assert_eq!(p.settings_allow, vec!["camera.fov", "draw_distance"]);
    }

    #[test]
    fn supervised_thresholds_and_approval_queue_consume_once() {
        let p = McpPolicy {
            mode: PolicyMode::Supervised,
            supervised_pay_threshold: 10,
            ..Default::default()
        };
        assert!(
            p.requires_supervised_approval("economy_pay_avatar", &json!({ "amount": 5 }))
                .is_none()
        );
        let summary = p
            .requires_supervised_approval("economy_pay_avatar", &json!({ "amount": 25 }))
            .expect("above threshold requires approval");
        assert!(summary.contains("25"));

        let q = ApprovalQueue::new();
        let req_id = match q.check_or_request("s1", "economy_pay_avatar", "hash1", &summary) {
            ApprovalCheck::Pending(v) => v.id,
            other => panic!("expected Pending, got {other:?}"),
        };
        assert_eq!(q.next_pending().expect("pending view").id, req_id);
        assert!(q.approve(&req_id));
        assert!(matches!(
            q.check_or_request("s1", "economy_pay_avatar", "hash1", &summary),
            ApprovalCheck::Approved
        ));
        // Consumed once; second call re-requests approval.
        assert!(matches!(
            q.check_or_request("s1", "economy_pay_avatar", "hash1", &summary),
            ApprovalCheck::Pending(_)
        ));

        let req_id2 = q.next_pending().expect("second pending").id;
        assert!(q.deny(&req_id2));
        assert!(matches!(
            q.check_or_request("s1", "economy_pay_avatar", "hash1", &summary),
            ApprovalCheck::Denied
        ));
    }
}
