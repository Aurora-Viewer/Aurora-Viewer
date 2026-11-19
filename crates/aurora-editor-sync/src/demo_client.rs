//! A local editor for the demo mode (`AURORA_DEMO_EDITOR_SYNC`): it does
//! what an outside editor does, over the same loopback socket and the same
//! checks, so that the sync can be seen without one. It proves itself with
//! the challenge file, lists the scripts, follows the first one and reports
//! what it receives. It never connects anywhere but 127.0.0.1.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::mpsc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// What the demo editor saw.
#[derive(Debug, Clone, PartialEq)]
pub enum Seen {
    /// It proved itself (`session.ok`).
    Connected,
    /// `script.subscribe` answered.
    Followed {
        script_id: String,
        success: bool,
    },
    /// `script.compiled`: its diagnostics.
    Compiled {
        success: bool,
        diagnostics: usize,
        first_row: Option<i64>,
    },
    Unfollowed,
    Closed,
}

/// The largest challenge file read (it holds a UUID).
const MAX_CHALLENGE: u64 = 1024;

/// Connects to the server on `port` (tried for ten seconds) and reports.
pub fn spawn(runtime: &tokio::runtime::Handle, port: u16) -> mpsc::Receiver<Seen> {
    let (tx, rx) = mpsc::channel();
    runtime.spawn(async move {
        let mut stream = None;
        for _ in 0..40 {
            // proxy-guard: the demo editor speaks to the viewer's own server
            // on 127.0.0.1, never through a proxy
            if let Ok(s) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                stream = Some(s);
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let Some(stream) = stream else {
            let _ = tx.send(Seen::Closed);
            return;
        };
        let url = format!("ws://127.0.0.1:{port}/");
        let Ok((mut ws, _)) = tokio_tungstenite::client_async(url, stream).await else {
            let _ = tx.send(Seen::Closed);
            return;
        };
        while let Some(Ok(message)) = ws.next().await {
            let Message::Text(text) = message else { continue };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let method = value.get("method").and_then(Value::as_str).unwrap_or("");
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let reply = match method {
                "session.handshake" => {
                    let answer = params
                        .get("challenge")
                        .and_then(Value::as_str)
                        .and_then(read_challenge)
                        .unwrap_or_default();
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": value.get("id").cloned().unwrap_or(Value::Null),
                        "result": {
                            "challenge_response": answer,
                            "client_name": "Aurora demo editor",
                            "client_version": "1.0",
                            "protocol_version": "1.0",
                            "languages": ["lsl", "luau"],
                            "features": { "live_sync": true, "compilation": true },
                        },
                    }))
                }
                "session.ok" => {
                    let _ = tx.send(Seen::Connected);
                    Some(json!({ "jsonrpc": "2.0", "id": 1, "method": "script.list", "params": {} }))
                }
                "script.compiled" => {
                    let diagnostics = params.get("diagnostics").and_then(Value::as_array);
                    let _ = tx.send(Seen::Compiled {
                        success: params.get("success").and_then(Value::as_bool).unwrap_or(false),
                        diagnostics: diagnostics.map_or(0, Vec::len),
                        first_row: diagnostics
                            .and_then(|d| d.first())
                            .and_then(|d| d.get("row"))
                            .and_then(Value::as_i64),
                    });
                    None
                }
                "script.unsubscribe" => {
                    let _ = tx.send(Seen::Unfollowed);
                    None
                }
                "session.disconnect" => break,
                _ => match value.get("id").and_then(Value::as_i64) {
                    // script.list: follow the first script
                    Some(1) => value
                        .pointer("/result/script_ids/0")
                        .and_then(Value::as_str)
                        .map(|id| json!({ "jsonrpc": "2.0", "id": 2, "method": "script.subscribe", "params": { "script_id": id } })),
                    Some(2) => {
                        let result = value.get("result").cloned().unwrap_or(Value::Null);
                        let _ = tx.send(Seen::Followed {
                            script_id: result.get("script_id").and_then(Value::as_str).unwrap_or("").to_owned(),
                            success: result.get("success").and_then(Value::as_bool).unwrap_or(false),
                        });
                        None
                    }
                    _ => None,
                },
            };
            if let Some(reply) = reply
                && ws.send(Message::text(reply.to_string())).await.is_err()
            {
                break;
            }
        }
        let _ = tx.send(Seen::Closed);
    });
    rx
}

/// The challenge file's content, as an editor reads it.
fn read_challenge(path: &str) -> Option<String> {
    let path = std::path::Path::new(path);
    if std::fs::metadata(path).ok()?.len() > MAX_CHALLENGE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}
