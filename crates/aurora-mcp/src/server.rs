//! Streamable HTTP & SSE JSON-RPC 2.0 Model Context Protocol (MCP) Server.
//!
//! Binds to `127.0.0.1:3847` (by default) on the background Tokio runtime and
//! routes MCP JSON-RPC 2.0 messages to [`McpRouter`].

use crate::bridge::{DEFAULT_MCP_PORT, McpRequestEnvelope};
use crate::router::{McpProfile, McpRouter};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};

pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

pub struct McpServerHandle {
    pub bind_addr: SocketAddr,
    shutdown_tx: watch::Sender<bool>,
}

impl McpServerHandle {
    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

impl Drop for McpServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Handle a single JSON-RPC 2.0 message (`Value`) against `McpRouter`.
/// Returns `None` for JSON-RPC notifications (which have no `id` and expect no reply).
pub async fn handle_jsonrpc_message(router: &McpRouter, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

    // Notifications (no id) do not produce a JSON-RPC response.
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let req_id = id.unwrap_or(Value::Null);

    let result = match method {
        "initialize" => {
            let client_version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(MCP_PROTOCOL_VERSION);
            Ok(json!({
                "protocolVersion": client_version,
                "capabilities": {
                    "tools": { "listChanged": false },
                    "resources": { "subscribe": false, "listChanged": false },
                    "prompts": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "aurora-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": "Control Aurora Viewer through its native Rust runtime MCP bridge. Start with connection_status and control_acquire (or viewer_control_acquire). Release your lease when finished so temporary camera and setting overrides roll back cleanly."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let tools = router.list_tools();
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(jsonrpc_error(
                    req_id,
                    -32602,
                    "missing tool 'name' in tools/call params",
                ));
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Ok(router.call_tool(name, args).await)
        }
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        other => Err((-32601, format!("method not found: {other}"))),
    };

    match result {
        Ok(res) => Some(json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": res,
        })),
        Err((code, message)) => Some(jsonrpc_error(req_id, code, &message)),
    }
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

/// Spawn the HTTP/SSE MCP server on the provided Tokio runtime handle.
pub fn spawn_server(
    rt: &tokio::runtime::Handle,
    bridge_tx: mpsc::Sender<McpRequestEnvelope>,
    port: Option<u16>,
) -> Result<McpServerHandle, String> {
    let port = port.unwrap_or_else(|| {
        std::env::var("AURORA_MCP_PORT")
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .unwrap_or(DEFAULT_MCP_PORT)
    });
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let std_listener = std::net::TcpListener::bind(addr)
        .map_err(|e| format!("failed to bind MCP HTTP server on {addr}: {e}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| format!("failed to set nonblocking listener: {e}"))?;
    let local_addr = std_listener.local_addr().unwrap_or(addr);

    let router = Arc::new(McpRouter::new(bridge_tx, McpProfile::from_env()));
    let (sse_tx, _) = broadcast::channel::<String>(64);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    rt.spawn(async move {
        let Ok(listener) = TcpListener::from_std(std_listener) else {
            log::error!("aurora-mcp: failed to convert std TcpListener to Tokio TcpListener");
            return;
        };
        log::info!("aurora-mcp: listening on http://{local_addr}/mcp");

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                accept_res = listener.accept() => {
                    let Ok((stream, _peer)) = accept_res else {
                        continue;
                    };
                    let router = router.clone();
                    let sse_tx = sse_tx.clone();
                    let mut conn_shutdown = shutdown_rx.clone();
                    tokio::spawn(async move {
                        let _ = tokio::select! {
                            _ = conn_shutdown.changed() => Ok(()),
                            res = handle_http_connection(stream, router, sse_tx) => res,
                        };
                    });
                }
            }
        }
    });

    Ok(McpServerHandle {
        bind_addr: local_addr,
        shutdown_tx,
    })
}

async fn handle_http_connection(
    stream: TcpStream,
    router: Arc<McpRouter>,
    sse_tx: broadcast::Sender<String>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }
    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0].to_ascii_uppercase();
    let path = parts[1];

    let mut content_length = 0usize;
    let mut wants_sse = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k == "content-length" {
                content_length = v.parse::<usize>().unwrap_or(0);
            } else if k == "accept" && v.contains("text/event-stream") {
                wants_sse = true;
            }
        }
    }

    if method == "OPTIONS" {
        let mut stream = reader.into_inner();
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Accept, Mcp-Session-Id\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    if method == "GET" && path.starts_with("/health") {
        let body = serde_json::to_vec(&json!({
            "ok": true,
            "server": "aurora-mcp",
            "version": env!("CARGO_PKG_VERSION"),
        }))
        .unwrap_or_default();
        let mut stream = reader.into_inner();
        write_http_response(&mut stream, "200 OK", "application/json", &body).await?;
        return Ok(());
    }

    if method == "GET" && (path.starts_with("/sse") || (path.starts_with("/mcp") && wants_sse)) {
        let mut stream = reader.into_inner();
        let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
        stream.write_all(headers.as_bytes()).await?;
        stream
            .write_all(b"event: endpoint\ndata: /message\n\n")
            .await?;
        stream.flush().await?;

        let mut rx = sse_tx.subscribe();
        while let Ok(msg) = rx.recv().await {
            let frame = format!("event: message\ndata: {msg}\n\n");
            if stream.write_all(frame.as_bytes()).await.is_err() {
                break;
            }
            let _ = stream.flush().await;
        }
        return Ok(());
    }

    if method == "POST" {
        let mut body = vec![0u8; content_length.min(16 * 1024 * 1024)];
        reader.read_exact(&mut body).await?;
        let mut stream = reader.into_inner();

        let parsed: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                let err_body = serde_json::to_vec(&jsonrpc_error(
                    Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                ))
                .unwrap_or_default();
                write_http_response(&mut stream, "400 Bad Request", "application/json", &err_body)
                    .await?;
                return Ok(());
            }
        };

        let reply_opt = if let Value::Array(batch) = parsed {
            let mut replies = Vec::new();
            for item in batch {
                if let Some(r) = handle_jsonrpc_message(&router, item).await {
                    if let Ok(s) = serde_json::to_string(&r) {
                        let _ = sse_tx.send(s);
                    }
                    replies.push(r);
                }
            }
            (!replies.is_empty()).then_some(Value::Array(replies))
        } else {
            let r = handle_jsonrpc_message(&router, parsed).await;
            if let Some(ref val) = r
                && let Ok(s) = serde_json::to_string(val)
            {
                let _ = sse_tx.send(s);
            }
            r
        };

        match reply_opt {
            Some(reply) => {
                let out = serde_json::to_vec(&reply).unwrap_or_default();
                write_http_response(&mut stream, "200 OK", "application/json", &out).await?;
            }
            None => {
                write_http_response(&mut stream, "202 Accepted", "application/json", b"").await?;
            }
        }
        return Ok(());
    }

    let mut stream = reader.into_inner();
    write_http_response(
        &mut stream,
        "404 Not Found",
        "application/json",
        b"{\"error\":\"not found\"}",
    )
    .await
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    if !body.is_empty() {
        stream.write_all(body).await?;
    }
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{ViewerRequest, ViewerResponse, create_bridge_channels};

    #[tokio::test]
    async fn mcp_jsonrpc_initialize_lease_and_status_over_http() {
        let (tx, mut rx) = create_bridge_channels();

        // Mock main-thread loop responder.
        tokio::spawn(async move {
            while let Some(env) = rx.recv().await {
                let resp = match env.request {
                    ViewerRequest::ConnectionStatus => Ok(ViewerResponse::Json(json!({
                        "connected": true,
                        "demo_mode": true,
                        "region": "Aurora Plaza",
                    }))),
                    ViewerRequest::ApplyRollbacks { actions } => Ok(ViewerResponse::Json(json!({
                        "restored": actions.len(),
                    }))),
                    _ => Ok(ViewerResponse::Json(json!({"ok": true}))),
                };
                let _ = env.reply_tx.send(resp);
            }
        });

        let handle =
            spawn_server(&tokio::runtime::Handle::current(), tx, Some(0)).expect("spawn server");
        let addr = handle.bind_addr;

        // 1. Test `initialize`
        let init_reply = post_jsonrpc(
            addr,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05" }
            }),
        )
        .await;
        assert_eq!(init_reply["result"]["serverInfo"]["name"], "aurora-mcp");

        // 2. Test `control_acquire`
        let acquire_reply = post_jsonrpc(
            addr,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "control_acquire",
                    "arguments": { "label": "test-suite", "seconds": 120 }
                }
            }),
        )
        .await;
        assert_eq!(acquire_reply["result"]["isError"], false);

        // 3. Test `connection_status`
        let status_reply = post_jsonrpc(
            addr,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "connection_status",
                    "arguments": {}
                }
            }),
        )
        .await;
        let text = status_reply["result"]["content"][0]["text"]
            .as_str()
            .expect("status text");
        assert!(text.contains("Aurora Plaza"));
        assert!(text.contains("test-suite"));
    }

    async fn post_jsonrpc(addr: SocketAddr, payload: Value) -> Value {
        let body = serde_json::to_vec(&payload).expect("serialize");
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(req.as_bytes()).await.expect("write head");
        stream.write_all(&body).await.expect("write body");
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).await.expect("read resp");
        let text = String::from_utf8_lossy(&resp);
        let (_, json_part) = text.split_once("\r\n\r\n").expect("http body separator");
        serde_json::from_str(json_part).expect("parse json reply")
    }
}
