//! The server against a local test client, on 127.0.0.1 and an ephemeral
//! port: the handshake, the refusals and every limit. Nothing leaves the
//! computer.

use aurora_editor_sync::{Config, Event, Hello, Limits, RpcError, Server};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error, Message};
use uuid::Uuid;

type Client = WebSocketStream<TcpStream>;

const METHODS: &[&str] = &["script.list", "script.subscribe"];

fn hello() -> Hello {
    Hello {
        viewer_name: "Aurora Viewer".into(),
        viewer_version: "0.0.0".into(),
        agent_id: Uuid::from_u128(42),
        agent_name: "tester".into(),
        syntax_id: Uuid::from_u128(7),
        commands: false,
    }
}

fn start(limits: Limits) -> Server {
    Server::start(
        &tokio::runtime::Handle::current(),
        Config {
            port: 0,
            limits,
            methods: METHODS,
        },
        hello(),
    )
    .expect("server")
}

async fn raw_connect(port: u16, origin: Option<&str>, host: Option<&str>) -> Result<Client, Error> {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.expect("tcp");
    let mut request = format!("ws://127.0.0.1:{port}/").into_client_request().expect("request");
    if let Some(origin) = origin {
        request.headers_mut().insert("origin", origin.parse().expect("header"));
    }
    if let Some(host) = host {
        request.headers_mut().insert("host", host.parse().expect("header"));
    }
    tokio_tungstenite::client_async(request, stream).await.map(|(ws, _)| ws)
}

async fn connect(port: u16) -> Client {
    raw_connect(port, None, None).await.expect("upgrade")
}

/// The next text message, as JSON.
async fn next_json(ws: &mut Client) -> Option<Value> {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("in time") {
            Some(Ok(Message::Text(text))) => return Some(serde_json::from_str(&text).expect("json")),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

async fn send(ws: &mut Client, value: Value) {
    ws.send(Message::text(value.to_string())).await.expect("send");
}

/// Reads the challenge and answers it with `answer` (the file's text when `None`).
async fn answer(ws: &mut Client, answer: Option<&str>) -> Value {
    let call = next_json(ws).await.expect("handshake");
    assert_eq!(call["method"], json!("session.handshake"));
    let path = call["params"]["challenge"].as_str().expect("challenge").to_owned();
    let secret = std::fs::read_to_string(&path).expect("challenge file");
    let reply = json!({
        "jsonrpc": "2.0",
        "id": call["id"],
        "result": {
            "challenge_response": answer.unwrap_or(&secret),
            "client_name": "test client",
            "client_version": "1.2.3",
            "protocol_version": "1.0",
            "languages": ["lsl", "luau"],
            "features": { "live_sync": true },
        },
    });
    send(ws, reply).await;
    call
}

async fn proven(port: u16) -> Client {
    let mut ws = connect(port).await;
    answer(&mut ws, None).await;
    assert_eq!(next_json(&mut ws).await.expect("ok")["method"], json!("session.ok"));
    ws
}

async fn event(server: &mut Server) -> Event {
    for _ in 0..500 {
        if let Some(event) = server.try_event() {
            return event;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no event");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_editor_proves_itself_then_its_requests_reach_the_viewer() {
    let mut server = start(Limits::default());
    let port = server.port();
    let mut ws = connect(port).await;
    let call = answer(&mut ws, None).await;
    let params = &call["params"];
    assert_eq!(params["protocol_version"], json!("1.0"));
    assert_eq!(params["server_version"], json!("1.0.0"));
    assert_eq!(params["agent_id"], json!(Uuid::from_u128(42).to_string()));
    assert_eq!(params["agent_name"], json!("tester"));
    assert_eq!(params["syntax_id"], json!(Uuid::from_u128(7).to_string()));
    assert_eq!(params["languages"], json!(["lsl", "luau"]));
    assert_eq!(params["features"]["commands"], json!(false));
    assert_eq!(params["features"]["compilation"], json!(true));
    // answered: the file is gone
    let path = params["challenge"].as_str().expect("path").to_owned();
    assert_eq!(
        next_json(&mut ws).await.expect("ok"),
        json!({"jsonrpc": "2.0", "method": "session.ok"})
    );
    assert!(!std::path::Path::new(&path).exists());
    let Event::Opened { conn, client } = event(&mut server).await else {
        panic!("opened first")
    };
    assert_eq!(client.name, "test client");
    assert!(client.has_feature("live_sync"));
    assert_eq!(server.clients(), 1);

    send(&mut ws, json!({"jsonrpc": "2.0", "id": 5, "method": "script.list"})).await;
    let Event::Request {
        conn: c,
        id,
        method,
        params,
    } = event(&mut server).await
    else {
        panic!("request")
    };
    assert_eq!((c, &id, method.as_str(), &params), (conn, &json!(5), "script.list", &Value::Null));
    server.respond(conn, &id, Ok(json!({"script_ids": []})));
    assert_eq!(
        next_json(&mut ws).await.expect("reply"),
        json!({"jsonrpc": "2.0", "id": 5, "result": {"script_ids": []}})
    );

    // system.* answered by the server itself
    send(
        &mut ws,
        json!({"jsonrpc": "2.0", "id": "p", "method": "system.ping", "params": {"timestamp": 9}}),
    )
    .await;
    let pong = next_json(&mut ws).await.expect("pong");
    assert_eq!(pong["result"]["pong"], json!("pong"));
    assert_eq!(pong["result"]["timestamp"], json!(9));
    send(&mut ws, json!({"jsonrpc": "2.0", "id": 1, "method": "system.getVersion"})).await;
    assert_eq!(
        next_json(&mut ws).await.expect("version")["result"],
        json!({"client_name": "Aurora Viewer", "client_version": "0.0.0"})
    );
    // unknown methods, viewer methods as notifications, bad JSON
    send(&mut ws, json!({"jsonrpc": "2.0", "id": 2, "method": "object.content.get"})).await;
    assert_eq!(next_json(&mut ws).await.expect("not found")["error"]["code"], json!(-32601));
    send(&mut ws, json!({"jsonrpc": "2.0", "method": "script.list"})).await;
    let refused = next_json(&mut ws).await.expect("refused");
    assert_eq!(
        (refused["error"]["code"].clone(), refused["id"].clone()),
        (json!(-32600), Value::Null)
    );
    ws.send(Message::text("{not json")).await.expect("send");
    assert_eq!(next_json(&mut ws).await.expect("parse")["error"]["code"], json!(-32700));

    server.notify_all("language.syntax.change", json!({"id": "x"}));
    assert_eq!(
        next_json(&mut ws).await.expect("broadcast")["method"],
        json!("language.syntax.change")
    );

    ws.close(None).await.expect("close");
    assert_eq!(event(&mut server).await, Event::Closed { conn });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_is_answered_before_the_proof_and_a_wrong_answer_closes() {
    let server = start(Limits::default());
    let mut ws = connect(server.port()).await;
    let call = next_json(&mut ws).await.expect("handshake");
    let path = call["params"]["challenge"].as_str().expect("path").to_owned();
    send(&mut ws, json!({"jsonrpc": "2.0", "id": 1, "method": "script.list"})).await;
    let refused = next_json(&mut ws).await.expect("refused");
    assert_eq!(refused["error"]["code"], json!(RpcError::unauthorized().code));
    send(&mut ws, json!({"jsonrpc": "2.0", "id": 2, "method": "system.ping"})).await;
    assert_eq!(next_json(&mut ws).await.expect("refused")["error"]["code"], json!(-32002));
    send(
        &mut ws,
        json!({"jsonrpc": "2.0", "id": call["id"], "result": {"challenge_response": Uuid::new_v4().to_string()}}),
    )
    .await;
    let bye = next_json(&mut ws).await.expect("disconnect");
    assert_eq!(bye["method"], json!("session.disconnect"));
    assert_eq!(bye["params"], json!({"reason": 2, "message": "Invalid challenge response"}));
    assert!(next_json(&mut ws).await.is_none());
    assert!(!std::path::Path::new(&path).exists());
    assert_eq!(server.clients(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browsers_and_other_hosts_are_refused_before_the_upgrade() {
    let server = start(Limits::default());
    let port = server.port();
    for (origin, host) in [
        (Some("https://evil.example"), None),
        (Some("null"), None),
        (None, Some("evil.example:".to_owned() + &port.to_string())),
        (None, Some(format!("127.0.0.1:{}", port.wrapping_add(1)))),
    ] {
        match raw_connect(port, origin, host.as_deref()).await {
            Err(Error::Http(response)) => assert_eq!(response.status(), 403),
            other => panic!("not refused: {:?}", other.map(|_| ())),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_message_closes_the_connection() {
    let server = start(Limits {
        max_message: 1024,
        ..Limits::default()
    });
    let mut ws = proven(server.port()).await;
    let big = json!({"jsonrpc": "2.0", "id": 1, "method": "script.list", "params": {"pad": "x".repeat(4096)}});
    let _ = ws.send(Message::text(big.to_string())).await;
    assert!(next_json(&mut ws).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_silent_client_times_out() {
    let server = start(Limits {
        handshake_timeout: Duration::from_millis(300),
        ..Limits::default()
    });
    let mut ws = connect(server.port()).await;
    let call = next_json(&mut ws).await.expect("handshake");
    let path = call["params"]["challenge"].as_str().expect("path").to_owned();
    let bye = next_json(&mut ws).await.expect("disconnect");
    assert_eq!(bye["params"], json!({"reason": 3, "message": "Handshake failed"}));
    assert!(!std::path::Path::new(&path).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idle_client_is_closed() {
    let limits = Limits {
        ping_interval: Duration::from_millis(100),
        idle_timeout: Duration::from_millis(250),
        ..Limits::default()
    };
    let server = start(limits);
    let mut ws = proven(server.port()).await;
    // not reading: no pong goes back
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut saw_timeout = false;
    while let Some(message) = next_json(&mut ws).await {
        saw_timeout |= message["params"]["reason"] == json!(3);
    }
    assert!(saw_timeout);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn too_many_editors_are_refused() {
    let mut server = start(Limits {
        max_clients: 1,
        ..Limits::default()
    });
    let port = server.port();
    let _first = proven(port).await;
    let mut second = connect(port).await;
    answer(&mut second, None).await;
    let bye = next_json(&mut second).await.expect("refused");
    assert_eq!(bye["params"], json!({"reason": 4, "message": "Too many editor connections"}));
    assert!(matches!(event(&mut server).await, Event::Opened { .. }));
    assert_eq!(server.clients(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handshakes_at_once_are_bounded() {
    let server = start(Limits {
        max_pending: 1,
        ..Limits::default()
    });
    let port = server.port();
    let mut first = connect(port).await;
    assert!(next_json(&mut first).await.is_some());
    // the second one is dropped before its upgrade
    assert!(raw_connect(port, None, None).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flood_is_rate_limited_then_closed() {
    let mut server = start(Limits {
        rate: 1.0,
        burst: 3.0,
        strikes: 5,
        ..Limits::default()
    });
    let mut ws = proven(server.port()).await;
    for i in 0..20 {
        let _ = ws
            .send(Message::text(
                json!({"jsonrpc": "2.0", "id": i, "method": "system.status"}).to_string(),
            ))
            .await;
    }
    let mut ok = 0;
    let mut limited = 0;
    let mut closed_with_reason = false;
    while let Some(message) = next_json(&mut ws).await {
        if message["result"]["status"] == json!("OK") {
            ok += 1;
        } else if message["error"]["code"] == json!(-32004) {
            limited += 1;
        } else if message["method"] == json!("session.disconnect") {
            closed_with_reason = message["params"]["message"] == json!("Too many requests");
        }
    }
    assert!(ok <= 4, "{ok}");
    assert!(limited >= 1);
    assert!(closed_with_reason);
    assert!(matches!(event(&mut server).await, Event::Opened { .. }));
    assert!(matches!(event(&mut server).await, Event::Closed { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_closes_the_clients_and_frees_the_port() {
    let server = start(Limits::default());
    let port = server.port();
    let mut ws = proven(port).await;
    drop(server);
    assert!(next_json(&mut ws).await.is_none());
    // the port can be taken again once the accept loop has gone
    let mut again = None;
    for _ in 0..100 {
        match Server::start(
            &tokio::runtime::Handle::current(),
            Config {
                port,
                limits: Limits::default(),
                methods: METHODS,
            },
            hello(),
        ) {
            Ok(server) => {
                again = Some(server);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
    assert!(again.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_taken_port_fails_at_once() {
    let server = start(Limits::default());
    let taken = Server::start(
        &tokio::runtime::Handle::current(),
        Config {
            port: server.port(),
            limits: Limits::default(),
            methods: METHODS,
        },
        hello(),
    );
    assert!(taken.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_reader_is_dropped_not_buffered() {
    let mut server = start(Limits {
        outgoing_queue: 4,
        ..Limits::default()
    });
    let mut ws = proven(server.port()).await;
    let Event::Opened { conn, .. } = event(&mut server).await else {
        panic!("opened")
    };
    // far more than the queue and the socket's buffers hold, never read
    let blob = "y".repeat(60_000);
    for _ in 0..2000 {
        server.notify(conn, "runtime.debug", json!({"message": blob}));
    }
    let mut gone = false;
    for _ in 0..300 {
        if server.clients() == 0 {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone);
    drop(ws.close(None).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_reader_is_dropped_on_the_bytes_it_has_queued() {
    let mut server = start(Limits {
        outgoing_queue: 100_000,
        outgoing_bytes: 256 * 1024,
        ..Limits::default()
    });
    let mut ws = proven(server.port()).await;
    let Event::Opened { conn, .. } = event(&mut server).await else {
        panic!("opened")
    };
    // few messages, far more bytes than allowed, never read
    let blob = "y".repeat(60_000);
    for _ in 0..2000 {
        server.notify(conn, "runtime.debug", json!({"message": blob}));
    }
    let mut gone = false;
    for _ in 0..300 {
        if !server.is_open(conn) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone);
    drop(ws.close(None).await);
}
