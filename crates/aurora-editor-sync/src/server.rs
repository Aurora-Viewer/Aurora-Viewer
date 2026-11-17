//! The local WebSocket server an external editor connects to.
//!
//! Port of LLWebsocketMgr::WSServer (indra/llcorehttp/llwebsocketmgr.cpp),
//! LLJSONRPCConnection / LLJSONRPCServer (indra/llcorehttp/lljsonrpcws.cpp)
//! and LLScriptEditorWSConnection (indra/newview/llscripteditorws.cpp),
//! originally LGPL 2.1.
//!
//! Every connection runs on the network runtime, never on the main thread:
//! the viewer only exchanges bounded channel messages with it once a frame.
//! A connection goes through:
//! 1. the TCP accept, on 127.0.0.1 only, within the connection limits;
//! 2. the HTTP upgrade, refused (403) for a browser or another host
//!    (`upgrade`), within 10 seconds and 64 KiB;
//! 3. the challenge (`session.handshake`): nothing it asks is answered
//!    (`-32002`) until it has answered with what the challenge file says,
//!    within 30 seconds; a wrong answer closes it;
//! 4. `session.ok`, then requests: the `system.*` ones are answered here,
//!    the viewer's are handed to it as [`Event::Request`].
//!
//! Limits the Legacy Viewer does not have: message size, clients, rate,
//! idle time, outgoing queue in messages and bytes (`limits`). No message
//! body, of a client or of the viewer, is ever written to the log: method
//! names, sizes and reasons only, and the name and version a client gives
//! itself (printable characters, 64 at most).

use crate::challenge::Challenge;
use crate::limits::{Bucket, Limits, Strikes};
use crate::rpc::{self, Decoded, Incoming, RpcError};
use crate::upgrade::{self, Headers, Refusal};
use crate::{ConnId, DisconnectReason};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};
use uuid::Uuid;

/// What the viewer says of itself in `session.handshake`
/// (LLScriptEditorWSConnection::sendHandshake).
#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    pub viewer_name: String,
    pub viewer_version: String,
    /// The agent: kept as the Legacy Viewer sends it, before the proof
    /// (the human's decision of 2026-11-17).
    pub agent_id: Uuid,
    pub agent_name: String,
    pub syntax_id: Uuid,
    /// `features.commands`: `command.*` answered.
    pub commands: bool,
}

/// What a client said of itself in its handshake answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
    pub script_name: String,
    pub script_language: String,
    pub languages: BTreeSet<String>,
    pub features: BTreeSet<String>,
}

impl ClientInfo {
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.contains(feature)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A client proved itself.
    Opened {
        conn: ConnId,
        client: ClientInfo,
    },
    /// A call to one of the viewer's methods; answer with [`Server::respond`].
    Request {
        conn: ConnId,
        id: Value,
        method: String,
        params: Value,
    },
    Closed {
        conn: ConnId,
    },
}

pub struct Config {
    pub port: u16,
    pub limits: Limits,
    /// The methods the viewer answers. All are asynchronous in the Legacy
    /// Viewer (registerAsyncMethod): called as a notification, refused.
    pub methods: &'static [&'static str],
}

/// The `system.*` methods every connection has (LLJSONRPCServer::setupConnectionMethods).
const SYSTEM_METHODS: [&str; 5] = [
    "system.getStats",
    "system.listMethods",
    "system.ping",
    "system.getVersion",
    "system.status",
];
/// LLScriptEditorWSServer::DEFAULT_SERVER_NAME, reported by system.getStats.
const SERVER_NAME: &str = "script_editor_server";
const EVENT_QUEUE: usize = 512;
/// A write that does not finish in this time: a client that does not read.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest message the viewer sends (a syntax file as JSON is about
/// 2 MB); a larger reply is refused rather than sent.
pub const MAX_REPLY: usize = 8 * 1024 * 1024;

enum Out {
    Text(String),
    /// `session.disconnect`, then the close frame.
    Disconnect(DisconnectReason, String),
}

struct Link {
    tx: mpsc::Sender<Out>,
    kill: Arc<Notify>,
    /// The bytes of the texts queued for it, not written yet.
    queued: Arc<AtomicUsize>,
}

impl Link {
    /// Queues a message, or drops a client that does not read: past the
    /// queue's messages or bytes, it is closed rather than buffered.
    fn push(&self, out: Out, max_bytes: usize) {
        if let Out::Text(text) = &out {
            let len = text.len();
            if self.queued.fetch_add(len, Ordering::AcqRel) + len > max_bytes {
                self.kill.notify_one();
                return;
            }
        }
        if self.tx.try_send(out).is_err() {
            self.kill.notify_one();
        }
    }
}

struct Shared {
    port: u16,
    limits: Limits,
    methods: &'static [&'static str],
    hello: RwLock<Hello>,
    /// Clients that proved themselves.
    links: Mutex<HashMap<ConnId, Link>>,
    pending: AtomicUsize,
    next_conn: AtomicU32,
    next_call: AtomicU64,
    requests: AtomicU64,
    notifications: AtomicU64,
}

impl Shared {
    fn links(&self) -> std::sync::MutexGuard<'_, HashMap<ConnId, Link>> {
        // a panic elsewhere never leaves the map half-written: take it back
        self.links.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn hello(&self) -> Hello {
        self.hello.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn respond(&self, conn: ConnId, id: &Value, result: Result<Value, RpcError>) {
        let mut text = match &result {
            Ok(value) => rpc::response(id, value.clone()),
            Err(e) => rpc::error(id, e),
        };
        if text.len() > MAX_REPLY {
            log::warn!("editor sync: a reply of {} bytes not sent", text.len());
            text = rpc::error(id, &RpcError::internal("Reply too large"));
        }
        self.send(conn, Out::Text(text));
    }

    fn send(&self, conn: ConnId, out: Out) {
        if let Some(link) = self.links().get(&conn) {
            link.push(out, self.limits.outgoing_bytes);
        }
    }

    fn is_open(&self, conn: ConnId) -> bool {
        self.links().contains_key(&conn)
    }
}

pub struct Server {
    shared: Arc<Shared>,
    events: mpsc::Receiver<Event>,
    stop: watch::Sender<bool>,
}

impl Server {
    /// Listens on 127.0.0.1:`port` (only: `ExternalWebsocketSyncLocal` off,
    /// all interfaces, is not ported). Fails at once when the port is taken.
    pub fn start(runtime: &tokio::runtime::Handle, config: Config, hello: Hello) -> io::Result<Server> {
        let std_listener = std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, config.port)))?;
        std_listener.set_nonblocking(true)?;
        let port = std_listener.local_addr()?.port();
        let listener = {
            let _entered = runtime.enter();
            // proxy-guard: a server on 127.0.0.1 for editors on this PC; a
            // proxy is for the viewer's outgoing connections
            TcpListener::from_std(std_listener)?
        };
        let shared = Arc::new(Shared {
            port,
            limits: config.limits,
            methods: config.methods,
            hello: RwLock::new(hello),
            links: Mutex::new(HashMap::new()),
            pending: AtomicUsize::new(0),
            next_conn: AtomicU32::new(1),
            next_call: AtomicU64::new(1000),
            requests: AtomicU64::new(0),
            notifications: AtomicU64::new(0),
        });
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
        let (stop, stop_rx) = watch::channel(false);
        runtime.spawn(accept_loop(listener, shared.clone(), events_tx, stop_rx));
        log::info!("editor sync: listening on 127.0.0.1:{port} (this computer only)");
        Ok(Server { shared, events, stop })
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }

    /// Changes what the next handshakes say (the agent, the syntax id).
    pub fn set_hello(&self, change: impl FnOnce(&mut Hello)) {
        change(&mut self.shared.hello.write().unwrap_or_else(|e| e.into_inner()));
    }

    /// The next event, without waiting.
    pub fn try_event(&mut self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    pub fn clients(&self) -> usize {
        self.shared.links().len()
    }

    /// Whether a connection is still there (an `Event::Closed` can come
    /// late, or be lost when the viewer was far behind).
    pub fn is_open(&self, conn: ConnId) -> bool {
        self.shared.is_open(conn)
    }

    /// The answer to an [`Event::Request`].
    pub fn respond(&self, conn: ConnId, id: &Value, result: Result<Value, RpcError>) {
        self.shared.respond(conn, id, result);
    }

    /// For an answer made on another thread (a file read off the frame).
    pub fn responder(&self) -> Responder {
        Responder(self.shared.clone())
    }

    pub fn notify(&self, conn: ConnId, method: &str, params: Value) {
        self.shared.notifications.fetch_add(1, Ordering::Relaxed);
        self.shared.send(conn, Out::Text(rpc::notification(method, Some(params))));
    }

    /// To every client that proved itself (notifyAll): serialised once.
    pub fn notify_all(&self, method: &str, params: Value) {
        let text = rpc::notification(method, Some(params));
        let links = self.shared.links();
        for link in links.values() {
            self.shared.notifications.fetch_add(1, Ordering::Relaxed);
            link.push(Out::Text(text.clone()), self.shared.limits.outgoing_bytes);
        }
    }

    /// `session.disconnect` then close (LLScriptEditorWSConnection::sendDisconnect).
    pub fn disconnect(&self, conn: ConnId, reason: DisconnectReason, message: &str) {
        self.shared.send(conn, Out::Disconnect(reason, message.to_owned()));
    }
}

/// Answers [`Event::Request`]s from any thread. Once the server stops,
/// its answers go nowhere.
#[derive(Clone)]
pub struct Responder(Arc<Shared>);

impl Responder {
    pub fn respond(&self, conn: ConnId, id: &Value, result: Result<Value, RpcError>) {
        self.0.respond(conn, id, result);
    }

    pub fn is_open(&self, conn: ConnId) -> bool {
        self.0.is_open(conn)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // every task sees it and closes: the port is free once the accept
        // loop has gone (closeAllConnections, 1001 "Server shutting down")
        let _ = self.stop.send(true);
        log::info!("editor sync: stopped");
    }
}

/// Releases a connection's slot among those still in their handshake.
struct PendingSlot(Arc<Shared>, bool);
impl PendingSlot {
    fn release(&mut self) {
        if std::mem::take(&mut self.1) {
            self.0.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
impl Drop for PendingSlot {
    fn drop(&mut self) {
        self.release();
    }
}

async fn accept_loop(listener: TcpListener, shared: Arc<Shared>, events: mpsc::Sender<Event>, mut stop: watch::Receiver<bool>) {
    loop {
        let accepted = tokio::select! {
            _ = stop.changed() => break,
            accepted = listener.accept() => accepted,
        };
        let (stream, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(e) => {
                // out of sockets: wait instead of spinning
                log::warn!("editor sync: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let limits = &shared.limits;
        let pending = shared.pending.fetch_add(1, Ordering::AcqRel);
        let slot = PendingSlot(shared.clone(), true);
        if pending >= limits.max_pending || shared.links().len() + pending >= limits.max_clients + limits.max_pending {
            log_once(&FULL, "editor sync: a connection was refused, too many at once");
            drop(slot);
            continue;
        }
        tokio::spawn(connection(stream, shared.clone(), events.clone(), stop.clone(), slot));
    }
}

static FULL: AtomicBool = AtomicBool::new(false);
static REFUSED: [AtomicBool; 3] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];

/// LL_WARNS_ONCE: a page can knock as often as it likes.
fn log_once(flag: &AtomicBool, text: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        log::warn!("{text}");
    }
}

fn forbidden() -> ErrorResponse {
    let mut response = ErrorResponse::new(None);
    *response.status_mut() = StatusCode::FORBIDDEN;
    response
}

/// The headers `upgrade::check` reads. A header that is not text, or a
/// Host given twice, reads as the worst case.
fn read_headers(request: &Request) -> Headers<'_> {
    let h = request.headers();
    let mut hosts = h.get_all("host").iter();
    let host = match (hosts.next(), hosts.next()) {
        (Some(one), None) => one.to_str().ok(),
        _ => None,
    };
    Headers {
        origin: h.get("origin").map(|v| v.to_str().unwrap_or("")),
        host,
        sec_fetch: ["sec-fetch-site", "sec-fetch-mode", "sec-fetch-dest"]
            .iter()
            .any(|n| h.contains_key(*n)),
    }
}

type Ws = WebSocketStream<TcpStream>;

async fn connection(
    stream: TcpStream,
    shared: Arc<Shared>,
    events: mpsc::Sender<Event>,
    mut stop: watch::Receiver<bool>,
    mut slot: PendingSlot,
) {
    let _ = stream.set_nodelay(true);
    let limits = shared.limits.clone();
    let port = shared.port;
    let mut refusal: Option<Refusal> = None;
    // the callback's types are tungstenite's (Callback::on_request)
    #[allow(clippy::result_large_err)]
    let callback = |request: &Request, response: Response| match upgrade::check(read_headers(request), port) {
        Ok(()) => Ok(response),
        Err(r) => {
            refusal = Some(r);
            Err(forbidden())
        }
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(limits.max_message))
        .max_frame_size(Some(limits.max_message))
        .max_write_buffer_size(2 * MAX_REPLY);
    let upgraded = tokio::time::timeout(
        limits.upgrade_timeout,
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config)),
    )
    .await;
    let mut ws = match upgraded {
        Ok(Ok(ws)) => ws,
        _ => {
            if let Some(r) = refusal {
                log_once(
                    &REFUSED[r as usize],
                    &format!("editor sync: refused a connection from {}", r.describe()),
                );
            }
            return;
        }
    };

    // a stop ends a handshake too: its challenge file goes with it
    let client = tokio::select! {
        client = handshake(&mut ws, &shared, &limits) => client,
        _ = stop.changed() => None,
    };
    let Some(client) = client else {
        return;
    };
    // a place among the clients, or out (stricter: the Legacy Viewer takes any number)
    let conn = shared.next_conn.fetch_add(1, Ordering::Relaxed).max(1);
    let (tx, rx) = mpsc::channel(limits.outgoing_queue.max(1));
    let kill = Arc::new(Notify::new());
    let queued = Arc::new(AtomicUsize::new(0));
    let admitted = {
        let mut links = shared.links();
        let room = links.len() < limits.max_clients;
        if room {
            links.insert(
                conn,
                Link {
                    tx,
                    kill: kill.clone(),
                    queued: queued.clone(),
                },
            );
        }
        room
    };
    if !admitted {
        log::warn!("editor sync: an editor was refused, {} are connected already", limits.max_clients);
        let _ = goodbye(&mut ws, DisconnectReason::InternalError, "Too many editor connections").await;
        return;
    }
    slot.release();
    // let in before it is told so: what it asks on hearing it is answered
    if send(&mut ws, rpc::notification("session.ok", None)).await.is_err()
        || events
            .try_send(Event::Opened {
                conn,
                client: client.clone(),
            })
            .is_err()
    {
        shared.links().remove(&conn);
        return;
    }
    log::info!(
        "editor sync: {} {} connected (connection {conn})",
        printable(&client.name),
        printable(&client.version)
    );
    session(ws, conn, &shared, &events, rx, kill, queued, &mut stop).await;
    shared.links().remove(&conn);
    // the viewer must hear it to let go of the scripts it followed
    let _ = tokio::time::timeout(Duration::from_secs(5), events.send(Event::Closed { conn })).await;
    log::info!("editor sync: connection {conn} closed");
}

/// A client's own words in the log: printable, short.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(64).collect()
}

async fn send(ws: &mut Ws, text: String) -> Result<(), ()> {
    match tokio::time::timeout(WRITE_TIMEOUT, ws.send(Message::Text(Utf8Bytes::from(text)))).await {
        Ok(Ok(())) => Ok(()),
        _ => Err(()),
    }
}

async fn goodbye(ws: &mut Ws, reason: DisconnectReason, message: &str) -> Result<(), ()> {
    let params = json!({ "reason": reason as i64, "message": message });
    send(ws, rpc::notification("session.disconnect", Some(params))).await?;
    close(ws, CloseCode::Normal, message).await;
    Ok(())
}

async fn close(ws: &mut Ws, code: CloseCode, reason: &str) {
    let frame = CloseFrame {
        code,
        reason: Utf8Bytes::from(rpc::truncate(reason, 120).to_owned()),
    };
    let _ = tokio::time::timeout(WRITE_TIMEOUT, ws.close(Some(frame))).await;
}

/// Sends the challenge and waits for the answer. `None`: closed.
async fn handshake(ws: &mut Ws, shared: &Shared, limits: &Limits) -> Option<ClientInfo> {
    let challenge = match tokio::task::spawn_blocking(Challenge::create).await {
        Ok(Ok(challenge)) => challenge,
        Ok(Err(e)) => {
            log::warn!("editor sync: unable to write the challenge file: {e}");
            let _ = goodbye(ws, DisconnectReason::InternalError, "Unable to issue a challenge").await;
            return None;
        }
        Err(_) => return None,
    };
    let hello = shared.hello();
    let call_id = format!("rpc_{}", shared.next_call.fetch_add(1, Ordering::Relaxed));
    let params = json!({
        "server_version": "1.0.0",
        "protocol_version": "1.0",
        "viewer_name": hello.viewer_name,
        "viewer_version": hello.viewer_version,
        "agent_id": hello.agent_id.hyphenated().to_string(),
        "agent_name": hello.agent_name,
        "challenge": challenge.path().to_string_lossy(),
        "languages": ["lsl", "luau"],
        "syntax_id": hello.syntax_id.hyphenated().to_string(),
        "features": {
            "live_sync": true,
            "compilation": true,
            "syntax_cache": true,
            "commands": hello.commands,
            "unified_diagnostics": true,
        },
    });
    if send(ws, rpc::request(&call_id, "session.handshake", params)).await.is_err() {
        return None;
    }
    let deadline = tokio::time::Instant::now() + limits.handshake_timeout;
    let mut bucket = Bucket::new(limits.rate, limits.burst, Instant::now());
    let mut strikes = Strikes::new(Instant::now());
    loop {
        let message = match tokio::time::timeout_at(deadline, ws.next()).await {
            Err(_) => {
                log::warn!("editor sync: handshake failed: no answer in time");
                let _ = goodbye(ws, DisconnectReason::Timeout, "Handshake failed").await;
                return None;
            }
            Ok(None | Some(Err(_))) => return None,
            Ok(Some(Ok(message))) => message,
        };
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => return None,
            _ => continue,
        };
        if !bucket.take(Instant::now()) {
            if strikes.add(Instant::now()) > limits.strikes {
                let _ = goodbye(ws, DisconnectReason::ProtocolError, "Too many requests").await;
                return None;
            }
            continue;
        }
        match rpc::decode(&text) {
            Decoded::Message(Incoming::Response { id, outcome }) if rpc::same_id(&id, &call_id) => {
                let result = match outcome {
                    Ok(result) => result,
                    Err(_) => {
                        log::warn!("editor sync: handshake failed: the client refused it");
                        let _ = goodbye(ws, DisconnectReason::ProtocolError, "Handshake failed").await;
                        return None;
                    }
                };
                // only something that could read the file knows what it says
                let proven = challenge.answered_by(result.get("challenge_response"));
                drop(challenge);
                if !proven {
                    log::warn!("editor sync: invalid or missing challenge response from a client");
                    let _ = goodbye(ws, DisconnectReason::ProtocolError, "Invalid challenge response").await;
                    return None;
                }
                let client = client_info(&result);
                if client.protocol_version != "1.0" {
                    log::warn!(
                        "editor sync: protocol version mismatch, expected 1.0, got {}",
                        printable(&client.protocol_version)
                    );
                }
                return Some(client);
            }
            // nothing for a peer that has not proven itself
            Decoded::Message(Incoming::Request { id, .. }) => {
                let refused = send(ws, rpc::error(&id, &RpcError::unauthorized())).await;
                if refused.is_err() {
                    return None;
                }
            }
            _ => {}
        }
    }
}

const MAX_CLIENT_FIELD: usize = 256;
const MAX_CLIENT_LIST: usize = 64;

/// handleHandshakeResponse's reading of the answer, bounded.
fn client_info(result: &Value) -> ClientInfo {
    let text = |key: &str| rpc::truncate(result.get(key).and_then(Value::as_str).unwrap_or(""), MAX_CLIENT_FIELD).to_owned();
    let languages = result
        .get("languages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .take(MAX_CLIENT_LIST)
                .map(|s| rpc::truncate(s, MAX_CLIENT_FIELD).to_owned())
                .collect()
        })
        .unwrap_or_default();
    let features = result
        .get("features")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(_, on)| truthy(on))
                .take(MAX_CLIENT_LIST)
                .map(|(k, _)| rpc::truncate(k, MAX_CLIENT_FIELD).to_owned())
                .collect()
        })
        .unwrap_or_default();
    ClientInfo {
        name: text("client_name"),
        version: text("client_version"),
        protocol_version: text("protocol_version"),
        script_name: text("script_name"),
        script_language: text("script_language"),
        languages,
        features,
    }
}

/// LLSD::asBoolean of a JSON value: true, or a number other than zero.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        _ => false,
    }
}

type Sink = futures_util::stream::SplitSink<Ws, Message>;
type Stream = futures_util::stream::SplitStream<Ws>;

/// How long a closing connection waits for the client's close: what it
/// still sends is read meanwhile, so that the socket is not reset with
/// unread data, which on Windows throws away our last message before the
/// client has read it.
const CLOSE_LINGER: Duration = Duration::from_secs(2);

/// Writes one message. False when it failed, did not finish in
/// [`WRITE_TIMEOUT`], or the connection was killed meanwhile (a client that
/// does not read blocks the write: the kill must not wait for it).
async fn write(sink: &mut Sink, message: Message, kill: &Notify) -> bool {
    tokio::select! {
        written = tokio::time::timeout(WRITE_TIMEOUT, sink.send(message)) => matches!(written, Ok(Ok(()))),
        _ = kill.notified() => false,
    }
}

/// Ends a connection cleanly: `session.disconnect` if there is a reason,
/// the close frame, then what the client still sends, until its close.
async fn farewell(sink: &mut Sink, stream: &mut Stream, kill: &Notify, disconnect: Option<(DisconnectReason, &str)>, close: CloseFrame) {
    if let Some((reason, message)) = disconnect {
        let params = json!({ "reason": reason as i64, "message": message });
        let text = rpc::notification("session.disconnect", Some(params));
        if !write(sink, Message::Text(Utf8Bytes::from(text)), kill).await {
            return;
        }
    }
    if !write(sink, Message::Close(Some(close)), kill).await {
        return;
    }
    let deadline = tokio::time::Instant::now() + CLOSE_LINGER;
    while let Ok(Some(Ok(message))) = tokio::time::timeout_at(deadline, stream.next()).await {
        if matches!(message, Message::Close(_)) {
            break;
        }
    }
}

fn close_frame(code: CloseCode, reason: &str) -> CloseFrame {
    CloseFrame {
        code,
        reason: Utf8Bytes::from(rpc::truncate(reason, 120).to_owned()),
    }
}

/// A client that proved itself: requests, the viewer's messages, pings.
#[allow(clippy::too_many_arguments)]
async fn session(
    ws: Ws,
    conn: ConnId,
    shared: &Shared,
    events: &mpsc::Sender<Event>,
    mut rx: mpsc::Receiver<Out>,
    kill: Arc<Notify>,
    queued: Arc<AtomicUsize>,
    stop: &mut watch::Receiver<bool>,
) {
    let limits = &shared.limits;
    let (mut sink, mut stream) = ws.split();
    let mut bucket = Bucket::new(limits.rate, limits.burst, Instant::now());
    let mut strikes = Strikes::new(Instant::now());
    let mut last_frame = Instant::now();
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + limits.ping_interval, limits.ping_interval);
    // replies made here, written in order with the viewer's
    let mut replies: Vec<String> = Vec::new();
    let killed = || log::warn!("editor sync: connection {conn} closed, it does not read what it is sent");
    loop {
        for text in replies.drain(..) {
            // a write that fails or does not finish ends the connection
            if !write(&mut sink, Message::Text(Utf8Bytes::from(text)), &kill).await {
                killed();
                return;
            }
        }
        // what ends the connection, once out of the select
        let disconnect: (Option<(DisconnectReason, String)>, CloseFrame) = tokio::select! {
            _ = stop.changed() => (None, close_frame(CloseCode::Away, "Server shutting down")),
            _ = kill.notified() => {
                killed();
                return;
            }
            out = rx.recv() => match out {
                Some(Out::Text(text)) => {
                    queued.fetch_sub(text.len(), Ordering::AcqRel);
                    if !write(&mut sink, Message::Text(Utf8Bytes::from(text)), &kill).await {
                        killed();
                        return;
                    }
                    continue;
                }
                Some(Out::Disconnect(reason, message)) => {
                    let frame = close_frame(CloseCode::Normal, &message);
                    (Some((reason, message)), frame)
                }
                None => return,
            },
            _ = ping.tick() => {
                if last_frame.elapsed() > limits.idle_timeout {
                    log::info!("editor sync: connection {conn} closed, nothing heard from it");
                    let message = "Connection timed out";
                    (Some((DisconnectReason::Timeout, message.to_owned())), close_frame(CloseCode::Normal, message))
                } else if write(&mut sink, Message::Ping(Bytes::new()), &kill).await {
                    continue;
                } else {
                    return;
                }
            }
            message = stream.next() => {
                let message = match message {
                    None | Some(Err(_)) => return,
                    Some(Ok(message)) => message,
                };
                last_frame = Instant::now();
                match message {
                    Message::Text(text) => {
                        let allowed = bucket.take(Instant::now());
                        if !allowed && strikes.add(Instant::now()) > limits.strikes {
                            log::warn!("editor sync: connection {conn} closed, too many requests");
                            let message = "Too many requests";
                            (Some((DisconnectReason::ProtocolError, message.to_owned())), close_frame(CloseCode::Normal, message))
                        } else {
                            if let Some(reply) = handle(&text, allowed, conn, shared, events) {
                                replies.push(reply);
                            }
                            continue;
                        }
                    }
                    Message::Close(_) => {
                        // tungstenite has queued the answering close
                        let _ = tokio::time::timeout(WRITE_TIMEOUT, sink.close()).await;
                        return;
                    }
                    // only text messages are spoken; a binary one counts for the rate
                    Message::Binary(_) => {
                        if bucket.take(Instant::now()) || strikes.add(Instant::now()) <= limits.strikes {
                            continue;
                        }
                        (None, close_frame(CloseCode::Policy, "Too many requests"))
                    }
                    _ => continue,
                }
            }
        };
        let (reason, frame) = disconnect;
        let reason = reason.as_ref().map(|(r, m)| (*r, m.as_str()));
        farewell(&mut sink, &mut stream, &kill, reason, frame).await;
        return;
    }
}

/// One text message of a proven client: the reply to send now, if any.
fn handle(text: &str, allowed: bool, conn: ConnId, shared: &Shared, events: &mpsc::Sender<Event>) -> Option<String> {
    let message = match rpc::decode(text) {
        Decoded::Message(message) => message,
        Decoded::Reply(e) => return Some(rpc::error(&Value::Null, &e)),
        Decoded::Ignored => return None,
    };
    match message {
        Incoming::Request { id, method, params } => {
            if !allowed {
                return Some(rpc::error(&id, &RpcError::rate_limited()));
            }
            shared.requests.fetch_add(1, Ordering::Relaxed);
            if let Some(result) = system(&method, &params, shared) {
                return Some(rpc::response(&id, result));
            }
            if !shared.methods.contains(&method.as_str()) {
                return Some(rpc::error(&id, &RpcError::method_not_found(&method)));
            }
            match events.try_send(Event::Request {
                conn,
                id: id.clone(),
                method,
                params,
            }) {
                Ok(()) => None,
                // the viewer is behind: say so rather than queue without end
                Err(_) => Some(rpc::error(&id, &RpcError::unavailable())),
            }
        }
        Incoming::Notification { method, .. } => {
            // the viewer's methods are asynchronous: never as a notification
            if allowed && shared.methods.contains(&method.as_str()) {
                let details = format!("Method {} cannot be called as a notification", rpc::truncate(&method, 128));
                return Some(rpc::error(&Value::Null, &RpcError::invalid_request(&details)));
            }
            None
        }
        // no call of the viewer is waiting once the handshake is done
        Incoming::Response { .. } => None,
    }
}

/// The `system.*` methods (LLJSONRPCServer, and LLScriptEditorWSServer's
/// handlePing / handleGetVersion overrides).
fn system(method: &str, params: &Value, shared: &Shared) -> Option<Value> {
    Some(match method {
        "system.listMethods" => {
            let mut all: Vec<&str> = SYSTEM_METHODS.iter().copied().chain(shared.methods.iter().copied()).collect();
            all.sort_unstable();
            json!(all)
        }
        "system.ping" => {
            let mut reply = Map::new();
            reply.insert("pong".into(), "pong".into());
            if let Some(timestamp) = params.get("timestamp") {
                reply.insert("timestamp".into(), timestamp.clone());
            }
            let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
            reply.insert("server_time".into(), json!(now.as_millis() as u64));
            Value::Object(reply)
        }
        "system.getVersion" => {
            let hello = shared.hello();
            json!({ "client_name": hello.viewer_name, "client_version": hello.viewer_version })
        }
        "system.status" => json!({ "status": "OK" }),
        "system.getStats" => json!({
            "server_name": SERVER_NAME,
            "connection_count": shared.links().len(),
            "is_running": true,
            "global_method_count": 1,
            "total_requests_handled": shared.requests.load(Ordering::Relaxed),
            "total_notifications_sent": shared.notifications.load(Ordering::Relaxed),
            "uptime": iso_now(),
        }),
        _ => return None,
    })
}

/// The current time as LLDate::asString writes it: `YYYY-MM-DDTHH:MM:SSZ`.
fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // civil_from_days (H. Hinnant), for days since 1970-01-01
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_answers_are_bounded() {
        let long = "x".repeat(10_000);
        let mut features = Map::new();
        for i in 0..1000 {
            features.insert(format!("f{i}"), json!(true));
        }
        features.insert("off".into(), json!(false));
        let result = json!({
            "client_name": long,
            "client_version": 3,
            "protocol_version": "1.0",
            "languages": ["lsl", 4, "luau"],
            "features": features,
        });
        let info = client_info(&result);
        assert_eq!(info.name.len(), MAX_CLIENT_FIELD);
        assert_eq!(info.version, "");
        assert_eq!(info.languages, BTreeSet::from(["lsl".to_owned(), "luau".to_owned()]));
        assert_eq!(info.features.len(), MAX_CLIENT_LIST);
        assert!(!info.has_feature("off"));
        assert_eq!(client_info(&json!("not an object")), ClientInfo::default());
    }

    #[test]
    fn printable_drops_control_characters() {
        assert_eq!(printable("a\nb\u{1b}[31mc"), "ab[31mc");
        assert_eq!(printable(&"y".repeat(500)).len(), 64);
    }

    #[test]
    fn iso_dates() {
        let text = iso_now();
        assert_eq!(text.len(), 20);
        assert!(text.ends_with('Z') && text.as_bytes()[10] == b'T');
    }
}
