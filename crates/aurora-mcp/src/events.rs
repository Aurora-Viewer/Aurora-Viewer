//! Per-session MCP event bus with bounded ring buffer, async broadcast, and
//! race-free `wait_for` filtering (`A8`, `C1`, `C2`).

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

/// Maximum number of viewer/bridge events retained in the shared ring buffer.
pub const MAX_EVENTS_BUFFER: usize = 1000;
const BROADCAST_CAPACITY: usize = 256;
const MAX_SESSIONS: usize = 256;
const SESSION_IDLE_TTL: Duration = Duration::from_secs(7200);

/// Known event source categories published on [`EventBus`].
pub const KNOWN_EVENT_SOURCES: &[&str] = &[
    "chat",
    "im",
    "teleport",
    "nav",
    "voice",
    "prompt",
    "offer",
    "region",
    "radar",
    "script",
    "collision",
    "lease",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEvent {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub source: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: Value,
}

impl McpEvent {
    /// Serializes the event payload used in `events_read` and `wait_for_event`.
    pub fn to_json(&self) -> Value {
        json!({
            "seq": self.seq,
            "timestamp_ms": self.timestamp_ms,
            "source": self.source,
            "type": self.event_type,
            "data": self.data,
        })
    }

    /// Formats the event as an MCP JSON-RPC 2.0 `notifications/message` frame.
    pub fn to_jsonrpc_notification(&self) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/message",
            "params": self.to_json(),
        })
    }
}

/// Filter predicate used by [`EventBus::wait_for`].
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Empty or contains `"*"` / `"all"` matches any source (case-insensitive).
    pub sources: Vec<String>,
    /// Empty matches any event type (case-insensitive exact match otherwise).
    pub event_types: Vec<String>,
    /// Optional case-insensitive substring check against event type or JSON data.
    pub contains_text: Option<String>,
    /// Exact field equality requirements on `ev.data` top-level keys.
    pub data_equals: HashMap<String, Value>,
}

impl EventFilter {
    /// Matches events from a single `source` and any of `types` (all types if empty).
    pub fn source_and_types(source: &str, types: &[&str]) -> Self {
        Self {
            sources: vec![source.to_ascii_lowercase()],
            event_types: types.iter().map(|t| (*t).to_owned()).collect(),
            contains_text: None,
            data_equals: HashMap::new(),
        }
    }

    /// Returns true if `ev` satisfies all filter predicates.
    pub fn matches(&self, ev: &McpEvent) -> bool {
        if !self.sources.is_empty() && !sources_match_list(&self.sources, &ev.source) {
            return false;
        }
        if !self.event_types.is_empty() && !self.event_types.iter().any(|t| t.eq_ignore_ascii_case(&ev.event_type)) {
            return false;
        }
        if !self.data_equals.is_empty() {
            let Some(obj) = ev.data.as_object() else {
                return false;
            };
            for (k, expected) in &self.data_equals {
                if obj.get(k) != Some(expected) {
                    return false;
                }
            }
        }
        if let Some(needle) = &self.contains_text
            && !needle.is_empty()
        {
            let needle_lc = needle.to_ascii_lowercase();
            let hay_type = ev.event_type.to_ascii_lowercase();
            let hay_data = ev.data.to_string().to_ascii_lowercase();
            if !hay_type.contains(&needle_lc) && !hay_data.contains(&needle_lc) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
struct SessionSub {
    sources: HashSet<String>,
    resource_subs: HashSet<String>,
    cursor: u64,
    last_seen: Instant,
}

struct BusState {
    next_seq: u64,
    buffer: VecDeque<McpEvent>,
    sessions: HashMap<String, SessionSub>,
}

/// Shared viewer event bus supporting independent per-session subscriptions/cursors,
/// live SSE broadcast, and blocking `wait_for` queries.
pub struct EventBus {
    state: Mutex<BusState>,
    tx: broadcast::Sender<McpEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            state: Mutex::new(BusState {
                next_seq: 0,
                buffer: VecDeque::with_capacity(MAX_EVENTS_BUFFER),
                sessions: HashMap::new(),
            }),
            tx,
        }
    }

    /// Always buffers the event and broadcasts it on the async channel so
    /// `wait_for` callers and any subscribed sessions receive it without loss.
    pub fn publish(&self, source: &str, event_type: &str, data: Value) -> u64 {
        let norm_source = normalize_source(source);
        let ts_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let ev = {
            let mut st = self.state.lock();
            st.next_seq += 1;
            let seq = st.next_seq;
            let ev = McpEvent {
                seq,
                timestamp_ms: ts_ms,
                source: norm_source,
                event_type: event_type.to_owned(),
                data,
            };
            if st.buffer.len() >= MAX_EVENTS_BUFFER {
                st.buffer.pop_front();
            }
            st.buffer.push_back(ev.clone());
            ev
        };
        let seq = ev.seq;
        let _ = self.tx.send(ev);
        seq
    }

    pub fn last_seq(&self) -> u64 {
        self.state.lock().next_seq
    }

    /// Adds `source` (or `"*"` / `"all"`) to `session_id`'s subscription set.
    pub fn subscribe_session(&self, session_id: &str, source: &str) -> Vec<String> {
        let norm = normalize_source(source);
        let now = Instant::now();
        let mut st = self.state.lock();
        if st.sessions.len() >= MAX_SESSIONS && !st.sessions.contains_key(session_id) {
            st.sessions.retain(|_, s| now.duration_since(s.last_seen) < SESSION_IDLE_TTL);
        }
        let next_seq = st.next_seq;
        let sub = st.sessions.entry(session_id.to_owned()).or_insert_with(|| SessionSub {
            sources: HashSet::new(),
            resource_subs: HashSet::new(),
            cursor: next_seq,
            last_seen: now,
        });
        if !norm.is_empty() {
            sub.sources.insert(norm);
        }
        sub.last_seen = now;
        sorted_sources(&sub.sources)
    }

    /// Removes `source` (or clears all when `"*"` / `"all"`) for `session_id` only.
    pub fn unsubscribe_session(&self, session_id: &str, source: &str) -> Vec<String> {
        let norm = normalize_source(source);
        let now = Instant::now();
        let mut st = self.state.lock();
        let Some(sub) = st.sessions.get_mut(session_id) else {
            return Vec::new();
        };
        if norm == "*" {
            sub.sources.clear();
        } else {
            sub.sources.remove(&norm);
        }
        sub.last_seen = now;
        sorted_sources(&sub.sources)
    }

    pub fn session_sources(&self, session_id: &str) -> Vec<String> {
        let st = self.state.lock();
        st.sessions.get(session_id).map(|s| sorted_sources(&s.sources)).unwrap_or_default()
    }

    pub fn session_cursor(&self, session_id: &str) -> Option<u64> {
        self.state.lock().sessions.get(session_id).map(|s| s.cursor)
    }

    /// Advances `session_id`'s read cursor forward to at least `seq`.
    pub fn set_session_cursor(&self, session_id: &str, seq: u64) {
        let now = Instant::now();
        let mut st = self.state.lock();
        let sub = st.sessions.entry(session_id.to_owned()).or_insert_with(|| SessionSub {
            sources: HashSet::new(),
            resource_subs: HashSet::new(),
            cursor: 0,
            last_seen: now,
        });
        sub.cursor = sub.cursor.max(seq);
        sub.last_seen = now;
    }

    /// True if `session_id` has `"*"` or `source` subscribed.
    pub fn session_matches(&self, session_id: &str, source: &str) -> bool {
        let norm = normalize_source(source);
        let st = self.state.lock();
        let Some(sub) = st.sessions.get(session_id) else {
            return false;
        };
        sub.sources.contains("*") || sub.sources.contains(&norm)
    }

    /// Reads buffered events after `after` (or the session's cursor if `None`),
    /// filtered by `sources_override` or the session's active subscriptions,
    /// returning `(events, used_after, last_seq, active_sources)`.
    pub fn read_for_session(
        &self,
        session_id: &str,
        after: Option<u64>,
        sources_override: Option<&[String]>,
        limit: usize,
    ) -> (Vec<McpEvent>, u64, u64, Vec<String>) {
        let limit = limit.clamp(1, 200);
        let now = Instant::now();
        let mut st = self.state.lock();
        let last_seq = st.next_seq;
        let used_after = after.unwrap_or_else(|| st.sessions.get(session_id).map_or(0, |s| s.cursor));
        let session_sources: HashSet<String> = st.sessions.get(session_id).map(|s| s.sources.clone()).unwrap_or_default();
        let override_norm: Option<Vec<String>> = sources_override
            .filter(|s| !s.is_empty())
            .map(|list| list.iter().map(|x| normalize_source(x)).collect());

        let events: Vec<McpEvent> = st
            .buffer
            .iter()
            .filter(|ev| ev.seq > used_after)
            .filter(|ev| match &override_norm {
                Some(list) => sources_match_list(list, &ev.source),
                None => session_sources.contains("*") || session_sources.contains(&ev.source),
            })
            .take(limit)
            .cloned()
            .collect();

        if let Some(sub) = st.sessions.get_mut(session_id) {
            if let Some(last) = events.last() {
                sub.cursor = sub.cursor.max(last.seq);
            } else if after.is_none() {
                sub.cursor = sub.cursor.max(last_seq);
            }
            sub.last_seen = now;
        }
        let active_sources = sorted_sources(&session_sources);
        (events, used_after, last_seq, active_sources)
    }

    pub fn subscribe_broadcast(&self) -> broadcast::Receiver<McpEvent> {
        self.tx.subscribe()
    }

    pub fn subscribe_resource(&self, session_id: &str, uri: &str) -> Vec<String> {
        let now = Instant::now();
        let mut st = self.state.lock();
        if st.sessions.len() >= MAX_SESSIONS && !st.sessions.contains_key(session_id) {
            st.sessions.retain(|_, s| now.duration_since(s.last_seen) < SESSION_IDLE_TTL);
        }
        let next_seq = st.next_seq;
        let sub = st.sessions.entry(session_id.to_owned()).or_insert_with(|| SessionSub {
            sources: HashSet::new(),
            resource_subs: HashSet::new(),
            cursor: next_seq,
            last_seen: now,
        });
        sub.resource_subs.insert(uri.to_owned());
        sub.last_seen = now;
        sorted_sources(&sub.resource_subs)
    }

    pub fn unsubscribe_resource(&self, session_id: &str, uri: &str) -> Vec<String> {
        let mut st = self.state.lock();
        let Some(sub) = st.sessions.get_mut(session_id) else {
            return Vec::new();
        };
        sub.resource_subs.remove(uri);
        sub.last_seen = Instant::now();
        sorted_sources(&sub.resource_subs)
    }

    /// Returns any subscribed resource URIs for `session_id` affected by `source`.
    pub fn matching_subscribed_resources(&self, session_id: &str, source: &str) -> Vec<String> {
        let candidates = crate::resources::uris_for_event_source(source);
        let st = self.state.lock();
        let Some(sub) = st.sessions.get(session_id) else {
            return Vec::new();
        };
        candidates
            .into_iter()
            .filter(|uri| sub.resource_subs.contains(*uri))
            .map(str::to_owned)
            .collect()
    }

    pub fn remove_session(&self, session_id: &str) -> bool {
        self.state.lock().sessions.remove(session_id).is_some()
    }

    /// Replays buffered events with `seq > after_seq` (for Streamable HTTP `Last-Event-ID`).
    pub fn replay_after(&self, after_seq: u64, limit: usize) -> Vec<McpEvent> {
        let st = self.state.lock();
        st.buffer
            .iter()
            .filter(|ev| ev.seq > after_seq)
            .take(limit.clamp(1, MAX_EVENTS_BUFFER))
            .cloned()
            .collect()
    }

    /// Waits up to `timeout` for the first event with `seq > after` matching `filter`.
    /// Subscribes to the broadcast channel before scanning the buffer so no events
    /// published concurrently can be missed.
    pub async fn wait_for(&self, filter: &EventFilter, after: u64, timeout: Duration) -> Option<McpEvent> {
        let mut rx = self.tx.subscribe();
        if let Some(hit) = self.find_buffered(filter, after) {
            return Some(hit);
        }

        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return None,
                res = rx.recv() => match res {
                    Ok(ev) => {
                        if ev.seq > after && filter.matches(&ev) {
                            return Some(ev);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some(hit) = self.find_buffered(filter, after) {
                            return Some(hit);
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    }

    fn find_buffered(&self, filter: &EventFilter, after: u64) -> Option<McpEvent> {
        let st = self.state.lock();
        st.buffer.iter().find(|ev| ev.seq > after && filter.matches(ev)).cloned()
    }
}

fn normalize_source(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    if s == "all" { "*".to_owned() } else { s }
}

fn sources_match_list(list: &[String], source: &str) -> bool {
    let source_lc = source.to_ascii_lowercase();
    list.iter().any(|item| {
        let item_lc = item.trim().to_ascii_lowercase();
        item_lc == "*" || item_lc == "all" || item_lc == source_lc
    })
}

fn sorted_sources(set: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = set.iter().cloned().collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn per_session_subscriptions_and_cursors_are_isolated() {
        let bus = EventBus::new();
        assert_eq!(bus.subscribe_session("sess-a", "chat"), vec!["chat"]);
        assert_eq!(bus.subscribe_session("sess-b", "ALL"), vec!["*"]);

        let s1 = bus.publish("chat", "ChatFromSimulator", json!({ "message": "hello" }));
        let s2 = bus.publish("im", "InstantMessage", json!({ "message": "secret" }));
        assert_eq!((s1, s2), (1, 2));

        // sess-a subscribed only to chat; sess-b receives both.
        let (ev_a, _, _, _) = bus.read_for_session("sess-a", None, None, 50);
        assert_eq!(ev_a.len(), 1);
        assert_eq!(ev_a[0].source, "chat");
        assert_eq!(bus.session_cursor("sess-a"), Some(1));

        let (ev_b, _, _, _) = bus.read_for_session("sess-b", None, None, 50);
        assert_eq!(ev_b.len(), 2);
        assert_eq!(bus.session_cursor("sess-b"), Some(2));

        // Unsubscribing * on sess-a leaves sess-b intact.
        assert!(bus.unsubscribe_session("sess-a", "*").is_empty());
        assert_eq!(bus.session_sources("sess-b"), vec!["*"]);

        let _s3 = bus.publish("nav", "WalkFinished", json!({ "outcome": "arrived" }));
        let (ev_a2, _, _, _) = bus.read_for_session("sess-a", None, None, 50);
        assert!(ev_a2.is_empty());
        let (ev_b2, _, _, _) = bus.read_for_session("sess-b", None, None, 50);
        assert_eq!(ev_b2.len(), 1);
        assert_eq!(ev_b2[0].event_type, "WalkFinished");
    }

    #[test]
    fn event_filter_checks_sources_types_text_and_data() {
        let ev = McpEvent {
            seq: 5,
            timestamp_ms: 1000,
            source: "chat".into(),
            event_type: "ChatFromSimulator".into(),
            data: json!({ "message": "Door unlocked", "channel": 0 }),
        };
        let mut data_eq = HashMap::new();
        data_eq.insert("channel".into(), json!(0));
        let filter = EventFilter {
            sources: vec!["chat".into()],
            event_types: vec!["chatfromsimulator".into()],
            contains_text: Some("UNLOCKED".into()),
            data_equals: data_eq,
        };
        assert!(filter.matches(&ev));

        let mut wrong = filter.clone();
        wrong.contains_text = Some("locked_missing".into());
        assert!(!wrong.matches(&ev));

        let mut wrong_data = filter.clone();
        wrong_data.data_equals.insert("channel".into(), json!(99));
        assert!(!wrong_data.matches(&ev));
    }

    #[tokio::test]
    async fn wait_for_finds_buffered_and_live_events_and_times_out() {
        let bus = Arc::new(EventBus::new());
        let seq1 = bus.publish("nav", "WalkFinished", json!({ "outcome": "arrived" }));

        // Immediately matches already-buffered event with seq > 0.
        let hit = bus
            .wait_for(
                &EventFilter::source_and_types("nav", &["WalkFinished"]),
                0,
                Duration::from_millis(200),
            )
            .await
            .expect("buffered match");
        assert_eq!(hit.seq, seq1);

        // Wakes up on future matching event while ignoring non-matching ones.
        let bus_bg = Arc::clone(&bus);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            bus_bg.publish("chat", "ChatFromSimulator", json!({ "message": "noise" }));
            tokio::time::sleep(Duration::from_millis(20)).await;
            bus_bg.publish("teleport", "AgentMovementComplete", json!({ "position": [10, 20, 30] }));
        });

        let tp = bus
            .wait_for(
                &EventFilter::source_and_types("teleport", &["AgentMovementComplete", "TeleportFailed"]),
                seq1,
                Duration::from_millis(500),
            )
            .await
            .expect("live teleport event");
        assert_eq!(tp.event_type, "AgentMovementComplete");

        // Times out cleanly when nothing matches.
        let none = bus
            .wait_for(
                &EventFilter::source_and_types("region", &["RegionRestart"]),
                tp.seq,
                Duration::from_millis(40),
            )
            .await;
        assert!(none.is_none());
    }

    #[test]
    fn resource_subscriptions_replay_and_session_removal_work() {
        let bus = EventBus::new();
        assert_eq!(bus.subscribe_resource("s1", "aurora://chat/recent"), vec!["aurora://chat/recent"]);
        assert_eq!(bus.matching_subscribed_resources("s1", "chat"), vec!["aurora://chat/recent"]);
        assert!(bus.matching_subscribed_resources("s1", "radar").is_empty());

        let s1 = bus.publish("chat", "ChatFromSimulator", json!({ "message": "a" }));
        let s2 = bus.publish("nav", "WalkFinished", json!({ "outcome": "arrived" }));
        let replayed = bus.replay_after(s1, 10);
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].seq, s2);

        assert_eq!(bus.unsubscribe_resource("s1", "aurora://chat/recent"), Vec::<String>::new());
        assert!(bus.remove_session("s1"));
        assert!(!bus.remove_session("s1"));
    }
}
