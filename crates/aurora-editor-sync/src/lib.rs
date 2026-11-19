//! The external script editor sync: a WebSocket server on this computer
//! that an outside editor (the extensions that speak the Legacy Viewer's
//! protocol, such as the sl-vscode-plugin) connects to, to follow the
//! scripts the user sent to it and get their compile results.
//!
//! Port of the Legacy Viewer's LLWebsocketMgr, LLJSONRPCConnection /
//! LLJSONRPCServer (indra/llcorehttp) and LLScriptEditorWSServer /
//! LLScriptEditorWSConnection (indra/newview/llscripteditorws.cpp),
//! originally LGPL 2.1. The protocol follows it to the letter; the server
//! is stricter (see `server` and `limits`). Nothing here knows the viewer:
//! it hands requests to it as events and sends what it is told.

pub mod challenge;
pub mod demo_client;
pub mod diagnostics;
pub mod limits;
pub mod rpc;
pub mod runtime;
pub mod server;
pub mod subscriptions;
pub mod upgrade;

pub use limits::Limits;
pub use rpc::RpcError;
pub use server::{ClientInfo, Config, Event, Hello, Responder, Server};

/// A connection, as the viewer knows it. Never 0.
pub type ConnId = u32;

/// LLScriptEditorWSConnection::DisconnectReason, sent in `session.disconnect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    Normal = 0,
    EditorClosed = 1,
    ProtocolError = 2,
    Timeout = 3,
    InternalError = 4,
}

/// For the fuzz target: every reader of what a client sends, and of the
/// chat a script says, on any input. The text is a message; its first two
/// lines are also an upgrade's `Origin` and `Host`; split at its NUL bytes,
/// chat lines from one object.
pub fn fuzz_input(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let _ = rpc::decode(text);
    let mut lines = text.splitn(3, '\n');
    let origin = lines.next().filter(|o| !o.is_empty());
    let host = lines.next();
    let _ = upgrade::check(
        upgrade::Headers {
            origin,
            host,
            sec_fetch: false,
        },
        9020,
    );
    let lua = data.first().is_some_and(|b| b & 1 == 1);
    let _ = diagnostics::compiled("id", false, false, &[text.to_owned()], lua);
    let mut aggregator = runtime::Aggregator::default();
    let now = std::time::Instant::now();
    let prim = uuid::Uuid::from_u128(1);
    let mut bursts = Vec::new();
    for (n, line) in text.split('\0').enumerate() {
        let channel = if n % 2 == 0 {
            runtime::Channel::Debug
        } else {
            runtime::Channel::OwnerSay
        };
        let chat = runtime::Chat {
            from: prim,
            from_name: "object".into(),
            text: line.to_owned(),
            channel,
        };
        bursts.extend(aggregator.ingest(chat, now, |_, _| lua));
    }
    bursts.extend(aggregator.flush());
    for burst in &bursts {
        let script = |_, name: &str| {
            Some(runtime::Script {
                script_id: name.to_owned(),
                item: prim,
                lua,
            })
        };
        let _ = runtime::event(burst, Some, script);
    }
}
