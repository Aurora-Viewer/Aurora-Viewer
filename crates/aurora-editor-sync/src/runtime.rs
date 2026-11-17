//! `runtime.error` and `runtime.debug`: what a followed script says to its
//! owner (llOwnerSay, the debug channel, its run-time errors), for the
//! editor to show next to the code.
//!
//! Port of LLPublishedObjectMgr::ingestRuntimeChat / buildRuntimeChatEvent
//! and RuntimeEventAggregator (indra/newview/llpublishedobjectmgr.cpp) and
//! LLScriptEditorWSServer::sendRuntimeEvent (indra/newview/llscripteditorws.cpp),
//! originally LGPL 2.1. A run-time error arrives in several chat lines from
//! the same object within a second: they are gathered into one event. Only
//! chat whose script is followed goes out: in the core sync that is a
//! run-time error naming the script (`[script:<name>]`) of the prim that
//! holds it. Stricter: a burst is bounded (64 lines, 16 KiB), where the
//! Legacy Viewer gathers without end.

use crate::diagnostics::{lsl_location, luau_location};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// RuntimeEventAggregator::FRAGMENT_TIMEOUT.
pub const FRAGMENT_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_FRAGMENTS: usize = 64;
const MAX_BURST_TEXT: usize = 16 * 1024;
const RUNTIME_ERROR: &str = "Script run-time error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Debug,
    OwnerSay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vm {
    Lsl,
    Luau,
}

/// One chat line from an object.
#[derive(Debug, Clone)]
pub struct Chat {
    pub from: Uuid,
    pub from_name: String,
    pub text: String,
    pub channel: Channel,
}

/// A burst gathered and ready to be sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Burst {
    pub from: Uuid,
    pub from_name: String,
    pub vm: Vm,
    pub channel: Channel,
    pub fragments: Vec<String>,
}

struct Pending {
    burst: Burst,
    size: usize,
    deadline: Instant,
}

/// RuntimeEventAggregator plus ingestRuntimeChat's choice between sending
/// a line at once and gathering it.
#[derive(Default)]
pub struct Aggregator {
    pending: Option<Pending>,
}

/// `^(.+?)\s+\[script:([^\]]+)\]\s+Script run-time error` on a whole line:
/// the object's name and the script's.
fn error_header(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_suffix(RUNTIME_ERROR)?;
    let rest = rest.trim_end_matches(char::is_whitespace);
    if rest.len() == line.len() - RUNTIME_ERROR.len() {
        return None; // `\s+` before the marker
    }
    let rest = rest.strip_suffix(']')?;
    let open = rest.rfind("[script:")?;
    let script = &rest[open + "[script:".len()..];
    let object = &rest[..open];
    let trimmed = object.trim_end_matches(char::is_whitespace);
    if script.is_empty() || script.contains(']') || trimmed.len() == object.len() || trimmed.is_empty() {
        return None;
    }
    Some((trimmed, script))
}

/// LLStringUtil::getTokens(text, "\n"): the lines, empty ones dropped.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').filter(|l| !l.is_empty())
}

impl Aggregator {
    /// A chat line from an object. `luau` says whether a script of that
    /// object by that name is a Luau one. Returns the bursts now complete.
    pub fn ingest(&mut self, chat: Chat, now: Instant, luau: impl Fn(Uuid, &str) -> bool) -> Vec<Burst> {
        let mut done = Vec::new();
        let header = lines(&chat.text).next().and_then(error_header);
        if header.is_none() && self.pending.is_none() {
            done.push(Burst {
                from: chat.from,
                from_name: chat.from_name,
                vm: Vm::Lsl,
                channel: chat.channel,
                fragments: vec![chat.text],
            });
            return done;
        }
        // a line after the header keeps the burst's language. The Legacy
        // Viewer reads it as LSL2 (only a header line names its script),
        // which cuts a Luau error from its `name:line:` line: kept whole
        // here, so that the editor gets the error's line.
        let vm = match header {
            Some((_, script)) if luau(chat.from, script) => Vm::Luau,
            Some(_) => Vm::Lsl,
            None => self.pending.as_ref().map_or(Vm::Lsl, |p| p.burst.vm),
        };
        if let Some(p) = &self.pending {
            let b = &p.burst;
            let same = b.from == chat.from && b.from_name == chat.from_name && b.vm == vm && b.channel == chat.channel;
            if !same || p.burst.fragments.len() >= MAX_FRAGMENTS || p.size + chat.text.len() > MAX_BURST_TEXT {
                done.extend(self.flush());
            }
        }
        let pending = self.pending.get_or_insert_with(|| Pending {
            burst: Burst {
                from: chat.from,
                from_name: chat.from_name.clone(),
                vm,
                channel: chat.channel,
                fragments: Vec::new(),
            },
            size: 0,
            deadline: now,
        });
        pending.size += chat.text.len();
        pending.burst.fragments.push(chat.text);
        pending.deadline = now + FRAGMENT_TIMEOUT;
        done
    }

    /// Once a frame: a burst quiet for a second is complete.
    pub fn flush_expired(&mut self, now: Instant) -> Option<Burst> {
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            return self.flush();
        }
        None
    }

    pub fn flush(&mut self) -> Option<Burst> {
        self.pending.take().map(|p| p.burst)
    }
}

/// A parsed run-time error (RuntimeEventAggregator::parseError).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ParsedError {
    pub error: String,
    pub line: i64,
    pub column: i64,
}

fn parse_error(vm: Vm, fragments: &[String]) -> ParsedError {
    for line in fragments.iter().flat_map(|f| lines(f)) {
        match vm {
            Vm::Lsl => {
                if let Some((l, c, _, message)) = lsl_location(line) {
                    return ParsedError {
                        error: message.to_owned(),
                        line: l as i64 + 1,
                        column: c as i64 + 1,
                    };
                }
            }
            Vm::Luau => {
                if let Some((_, l, message)) = luau_location(line) {
                    return ParsedError {
                        error: message.to_owned(),
                        line: l as i64,
                        column: 0,
                    };
                }
            }
        }
    }
    // LSL run-time errors often come as plain text: "Math Error"
    if vm == Vm::Lsl
        && let Some(line) = fragments.iter().flat_map(|f| lines(f)).find(|l| !l.contains(RUNTIME_ERROR))
    {
        return ParsedError {
            error: line.to_owned(),
            ..ParsedError::default()
        };
    }
    ParsedError::default()
}

/// What the viewer knows of the script a burst is about.
#[derive(Debug, Clone)]
pub struct Script {
    pub script_id: String,
    pub item: Uuid,
    pub lua: bool,
}

/// The event to send, as buildRuntimeChatEvent and sendRuntimeEvent make
/// it, or `None` when it is not about a followed script. `root` gives the
/// linkset's root of a prim the viewer knows (`None`: unknown prim, as
/// gObjectList.findObject failing); `script` finds a followed script by
/// prim and name.
pub fn event(
    burst: &Burst,
    root: impl Fn(Uuid) -> Option<Uuid>,
    script: impl Fn(Uuid, &str) -> Option<Script>,
) -> Option<(&'static str, Value)> {
    if burst.fragments.is_empty() {
        return None;
    }
    let prim = burst.from;
    let root = root(prim)?;
    let message = burst.fragments.join("\n");
    let mut lines: Vec<&str> = lines(&message).collect();
    let mut object_name = burst.from_name.clone();
    let mut script_name = String::new();
    let mut is_error = false;
    if lines.first().is_some_and(|l| l.ends_with(RUNTIME_ERROR)) {
        is_error = true;
        match lines.first().and_then(|l| error_header(l)) {
            Some((object, name)) => {
                object_name = object.to_owned();
                script_name = name.to_owned();
                lines.remove(0);
            }
            None => lines.clear(),
        }
    }
    let found = if script_name.is_empty() { None } else { script(prim, &script_name) };
    // sendRuntimeEvent: only for a followed script (no published objects in
    // the core sync)
    let found = found?;
    let mut params = json!({
        "script_id": found.script_id,
        "object_id": root.hyphenated().to_string(),
        "prim_id": prim.hyphenated().to_string(),
        "item_id": found.item.hyphenated().to_string(),
        "object_name": object_name,
        "message": message,
        "item": {
            "root_id": root.hyphenated().to_string(),
            "prim_id": prim.hyphenated().to_string(),
            "item_id": found.item.hyphenated().to_string(),
            "name": script_name,
            "language": if burst.vm == Vm::Luau { "luau" } else { "lsl" },
        },
        "channel": match burst.channel {
            Channel::Debug => "debug",
            Channel::OwnerSay => "owner_say",
        },
    });
    if is_error {
        let (error, line, column) = if lines.is_empty() {
            (String::new(), 0, 0)
        } else {
            let vm = if found.lua { Vm::Luau } else { burst.vm };
            let parsed = parse_error(vm, &burst.fragments);
            (parsed.error, parsed.line, parsed.column)
        };
        params["error"] = json!(error);
        params["line"] = json!(line);
        params["column"] = json!(column);
        params["stack"] = json!(lines);
        return Some(("runtime.error", params));
    }
    Some(("runtime.debug", params))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIM: Uuid = Uuid::from_u128(1);
    const ROOT: Uuid = Uuid::from_u128(2);
    const ITEM: Uuid = Uuid::from_u128(3);

    fn chat(text: &str) -> Chat {
        Chat {
            from: PRIM,
            from_name: "Door".into(),
            text: text.into(),
            channel: Channel::Debug,
        }
    }

    fn known(prim: Uuid) -> Option<Uuid> {
        (prim == PRIM).then_some(ROOT)
    }

    fn followed(prim: Uuid, name: &str) -> Option<Script> {
        (prim == PRIM && name == "door script").then(|| Script {
            script_id: "sid".into(),
            item: ITEM,
            lua: false,
        })
    }

    #[test]
    fn error_headers_read_as_the_legacy_pattern() {
        assert_eq!(
            error_header("Door [script:door script] Script run-time error"),
            Some(("Door", "door script"))
        );
        assert_eq!(error_header("My  Door\t[script:a]  Script run-time error"), Some(("My  Door", "a")));
        assert_eq!(error_header("[script:a] Script run-time error"), None);
        assert_eq!(error_header("Door [script:] Script run-time error"), None);
        assert_eq!(error_header("Door [script:a]Script run-time error"), None);
        assert_eq!(error_header("Door[script:a] Script run-time error"), None);
        assert_eq!(error_header("Door [script:a] Script run-time error!"), None);
        assert_eq!(error_header("hello"), None);
        assert_eq!(error_header(""), None);
    }

    #[test]
    fn an_error_burst_is_gathered_then_sent_for_a_followed_script() {
        let start = Instant::now();
        let mut agg = Aggregator::default();
        assert!(
            agg.ingest(chat("Door [script:door script] Script run-time error"), start, |_, _| false)
                .is_empty()
        );
        assert!(
            agg.ingest(chat("Math Error"), start + Duration::from_millis(500), |_, _| false)
                .is_empty()
        );
        assert!(agg.flush_expired(start + Duration::from_millis(1400)).is_none());
        let burst = agg.flush_expired(start + Duration::from_millis(1500)).expect("complete");
        assert_eq!(burst.fragments.len(), 2);
        let (method, params) = event(&burst, known, followed).expect("followed");
        assert_eq!(method, "runtime.error");
        assert_eq!(params["script_id"], json!("sid"));
        assert_eq!(params["object_id"], json!(ROOT.to_string()));
        assert_eq!(params["prim_id"], json!(PRIM.to_string()));
        assert_eq!(params["item_id"], json!(ITEM.to_string()));
        assert_eq!(params["object_name"], json!("Door"));
        assert_eq!(params["error"], json!("Math Error"));
        assert_eq!(params["stack"], json!(["Math Error"]));
        assert_eq!(params["channel"], json!("debug"));
        assert_eq!(params["item"]["language"], json!("lsl"));
    }

    #[test]
    fn chat_of_scripts_not_followed_never_goes_out() {
        let mut agg = Aggregator::default();
        let now = Instant::now();
        // plain owner-say: no script named, nothing in the core sync
        let plain = agg.ingest(chat("hello"), now, |_, _| false);
        assert_eq!(plain.len(), 1);
        assert!(event(&plain[0], known, followed).is_none());
        // another script of the same object
        agg.ingest(chat("Door [script:other] Script run-time error"), now, |_, _| false);
        let burst = agg.flush().expect("burst");
        assert!(event(&burst, known, followed).is_none());
        // an object the viewer does not know
        agg.ingest(
            Chat {
                from: Uuid::from_u128(9),
                ..chat("Door [script:door script] Script run-time error")
            },
            now,
            |_, _| false,
        );
        let burst = agg.flush().expect("burst");
        assert!(event(&burst, known, followed).is_none());
    }

    #[test]
    fn another_sender_closes_the_burst_and_bursts_are_bounded() {
        let now = Instant::now();
        let mut agg = Aggregator::default();
        agg.ingest(chat("Door [script:door script] Script run-time error"), now, |_, _| false);
        let other = Chat {
            from: Uuid::from_u128(5),
            ..chat("x")
        };
        let done = agg.ingest(other, now, |_, _| false);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].from, PRIM);
        agg.flush();
        agg.ingest(chat("Door [script:door script] Script run-time error"), now, |_, _| false);
        let mut flushed = 0;
        for _ in 0..1000 {
            flushed += agg.ingest(chat(&"x".repeat(1000)), now, |_, _| false).len();
        }
        assert!(flushed >= 1000 * 1000 / MAX_BURST_TEXT - 1);
        assert!(
            agg.pending
                .as_ref()
                .is_some_and(|p| p.burst.fragments.len() <= MAX_FRAGMENTS && p.size <= MAX_BURST_TEXT)
        );
    }

    #[test]
    fn luau_errors_read_their_line() {
        let now = Instant::now();
        let mut agg = Aggregator::default();
        agg.ingest(chat("Door [script:door script] Script run-time error"), now, |_, name| {
            name == "door script"
        });
        agg.ingest(chat("door script:12: attempt to index nil"), now, |_, name| name == "door script");
        let burst = agg.flush().expect("burst");
        assert_eq!(burst.vm, Vm::Luau);
        let (_, params) = event(&burst, known, |p, n| followed(p, n).map(|s| Script { lua: true, ..s })).expect("event");
        assert_eq!(params["error"], json!("attempt to index nil"));
        assert_eq!(params["line"], json!(12));
        assert_eq!(params["item"]["language"], json!("luau"));
    }

    #[test]
    fn lsl_locations_are_one_based() {
        let parsed = parse_error(Vm::Lsl, &["(3, 0) : ERROR : Stack-Heap Collision".to_owned()]);
        assert_eq!(
            parsed,
            ParsedError {
                error: "Stack-Heap Collision".into(),
                line: 4,
                column: 1
            }
        );
        assert_eq!(parse_error(Vm::Luau, &["nothing".to_owned()]), ParsedError::default());
    }
}
