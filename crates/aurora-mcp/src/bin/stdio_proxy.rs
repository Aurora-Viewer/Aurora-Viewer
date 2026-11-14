//! Thin stdio-to-HTTP MCP proxy binary (`aurora-mcp-stdio`).
//!
//! Bridges standard `stdio` MCP hosts (Antigravity, Claude Desktop, Cursor) to
//! the running Aurora Viewer HTTP MCP endpoint (`http://127.0.0.1:3847/mcp`).

use aurora_mcp::{DEFAULT_MCP_PORT, MCP_PROTOCOL_VERSION, McpProfile, list_tools_for_profile};
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn main() {
    let port = std::env::var("AURORA_MCP_PORT")
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_MCP_PORT);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let err = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("parse error: {e}") }
                });
                let _ = writeln!(stdout, "{err}");
                let _ = stdout.flush();
                continue;
            }
        };

        let id = parsed.get("id").cloned();
        let method = parsed.get("method").and_then(Value::as_str).unwrap_or("");
        let params = parsed.get("params").cloned().unwrap_or_else(|| json!({}));

        match forward_http(addr, trimmed.as_bytes()) {
            Ok(Some(reply_body)) if !reply_body.trim().is_empty() => {
                let _ = writeln!(stdout, "{}", reply_body.trim());
                let _ = stdout.flush();
            }
            Ok(_) => {}
            Err(e) => {
                if id.is_none() || method.starts_with("notifications/") {
                    continue;
                }
                let req_id = id.unwrap_or(Value::Null);
                let reply = handle_offline_request(req_id, method, &params, port, &e);
                let _ = writeln!(stdout, "{reply}");
                let _ = stdout.flush();
            }
        }
    }
}

fn handle_offline_request(req_id: Value, method: &str, params: &Value, port: u16, err: &std::io::Error) -> Value {
    let result = match method {
        "initialize" => {
            let client_version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(MCP_PROTOCOL_VERSION);
            Some(json!({
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
                "instructions": "Control Aurora Viewer through its native Rust runtime MCP bridge. Start with connection_status and control_acquire (or viewer_control_acquire), then pass the returned lease_id to all mutating tools (including viewer_login, viewer_shutdown, prim_*, economy_*, and nav_*). Release your lease with control_release when finished so temporary camera and setting overrides roll back cleanly."
            }))
        }
        "ping" => Some(json!({})),
        "tools/list" => Some(json!({ "tools": list_tools_for_profile(McpProfile::from_env()) })),
        "resources/list" => Some(json!({ "resources": [] })),
        "resources/templates/list" => Some(json!({ "resourceTemplates": [] })),
        "prompts/list" => Some(json!({ "prompts": [] })),
        "tools/call" if params.get("name").and_then(Value::as_str) == Some("connection_status") => {
            let payload = json!({
                "connected": false,
                "logged_in": false,
                "status": "viewer_offline",
                "port": port,
                "message": "Aurora Viewer is not running or MCP port is closed."
            });
            Some(json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
                }],
                "isError": false
            }))
        }
        _ => None,
    };

    match result {
        Some(res) => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": res
        }),
        None => json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {
                "code": -32000,
                "message": format!("Aurora Viewer MCP endpoint (127.0.0.1:{port}) unreachable: {err}")
            }
        }),
    }
}

fn forward_http(addr: SocketAddr, body: &[u8]) -> Result<Option<String>, std::io::Error> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let auth_line = std::env::var("AURORA_MCP_TOKEN")
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();

    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\n{auth_line}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let Some((head, payload)) = text.split_once("\r\n\r\n") else {
        return Ok(None);
    };
    if head.starts_with("HTTP/1.1 202") || payload.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(payload.to_owned()))
}
