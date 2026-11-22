//! The external script editor sync, on the viewer's side: the local
//! WebSocket server of `aurora_editor_sync` started for the scripts the
//! user sends to the external editor, the scripts an editor may follow,
//! their compile results, their run-time errors and the syntax files.
//!
//! Port of LLScriptEditorWSServer (indra/newview/llscripteditorws.cpp),
//! LLScriptEdContainer::startWebsocketServer / unsubscribeScript /
//! sendCompileResults (indra/newview/llpreviewscript.cpp), the chat
//! forwarding of LLFloaterIMNearbyChatHandler::processChat
//! (indra/newview/llfloaterimnearbychathandler.cpp) and the cache files of
//! LLSyntaxDefCache (indra/newview/llsyntaxid.cpp), originally LGPL 2.1.
//!
//! What goes out, and when:
//! - the server starts with the first script sent to the external editor
//!   while `websocket_sync` is on (ensureServerRunning), and stops at
//!   logout, at quit and when the setting goes off. Off by default here,
//!   where the Legacy Viewer has it on: the human's decision of 2026-11-17;
//! - an editor may follow only a script the user sent to the external
//!   editor, and only one that may be edited then (`can_edit`, the rights
//!   of `app::scripts::script_info`); the script's text never goes through
//!   the socket in the core sync: it stays in the exported file;
//! - a save's result (the compiler's lines) goes to the editor that
//!   follows the script; a followed script's run-time errors go to every
//!   editor, as the Legacy Viewer sends them, when
//!   `websocket_forward_debug` is on;
//! - nothing an editor sends saves or compiles a script: the file's watch
//!   does that, as before (the Legacy Viewer's LLLiveLSLFile).
//!
//! Nothing a client sends and no script text is written to the log.

use crate::ui::scripts::{Compiled, Synced};
use aurora_editor_sync::runtime::{self, Aggregator};
use aurora_editor_sync::subscriptions::{Subscriptions, subscribe_reply};
use aurora_editor_sync::{Config, ConnId, Event, Hello, Limits, Responder, RpcError, Server, diagnostics};
use aurora_llsd::Llsd;
use aurora_net::scripts::ScriptTarget;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The viewer's methods (LLScriptEditorWSServer's registerAsyncMethod of the
/// core sync); the tight integration's `object.*` and `command.*` are not
/// answered.
const METHODS: &[&str] = &[
    "script.subscribe",
    "script.unsubscribe",
    "script.list",
    "language.syntax.id",
    "language.syntax",
    "language.syntax.cache",
    "language.syntax.get",
];

/// LLSyntaxDefCache's names of the keyword files (FILENAME_INTERNAL_LSL,
/// FILENAME_INTERNAL_LUA).
const LSL_FILE: &str = "lsl_keywords.xml";
const LUA_FILE: &str = "lua_keywords.xml";

/// Chat lines kept between two frames for the run-time events.
const MAX_CHAT: usize = 256;

/// Syntax files being read and converted at once, for every editor: one
/// more is answered "busy" rather than queued (each is a few MB).
const MAX_SYNTAX_JOBS: usize = 2;

/// A new port is used once it has not changed for this long: the setting's
/// field passes through many values while it is dragged.
const PORT_SETTLE: Duration = Duration::from_millis(750);

/// After a stop, the port is free once the accept loop has gone, on the
/// network runtime: starting on it again is retried that long, silently.
const RELEASE_GRACE: Duration = Duration::from_secs(2);

/// The port the server listens on: the setting, or the default for a port
/// below 1024 (a hand-edited settings file; 0 would take any free port).
fn listen_port(setting: u16) -> u16 {
    if setting < 1024 {
        crate::ui::scripts::settings::WEBSOCKET_PORT
    } else {
        setting
    }
}

/// The tips the app shows (ExternalEditorServerStarted / Failed / Stopped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tip {
    Started(u16),
    Failed(u16),
    Stopped,
}

/// The chat of objects, for the run-time events: filled by the world's
/// chat while [`ChatTap::on`], taken once a frame.
#[derive(Default)]
pub struct ChatTap {
    pub on: bool,
    lines: VecDeque<runtime::Chat>,
}

impl ChatTap {
    /// An object's Debug or OwnerSay line (processChat's forwardChatToIDE),
    /// after the chat's filters.
    pub fn offer(&mut self, message: &aurora_net::ChatMessage) {
        use aurora_net::{ChatSourceType, ChatType};
        if !self.on || message.source_type != ChatSourceType::Object {
            return;
        }
        let channel = match message.chat_type {
            ChatType::Debug => runtime::Channel::Debug,
            ChatType::OwnerSay => runtime::Channel::OwnerSay,
            _ => return,
        };
        if self.lines.len() == MAX_CHAT {
            self.lines.pop_front();
        }
        self.lines.push_back(runtime::Chat {
            from: message.source_id,
            from_name: message.from_name.clone(),
            text: message.message.clone(),
            channel,
        });
    }

    fn take(&mut self) -> VecDeque<runtime::Chat> {
        std::mem::take(&mut self.lines)
    }
}

/// What the frame knows.
pub struct Frame<'a> {
    pub enabled: bool,
    pub port: u16,
    pub forward_debug: bool,
    pub logged_in: bool,
    pub scripts: Vec<Synced<'a>>,
    pub compiled: Vec<Compiled>,
    pub hello: Hello,
    pub now: Instant,
}

#[derive(Default)]
pub struct EditorSync {
    server: Option<Server>,
    /// The port the server was started on (`listen_port` of the setting).
    listening: u16,
    /// A port the setting moved to, and since when: the server moves there
    /// once it has settled (`PORT_SETTLE`).
    port_change: Option<(u16, Instant)>,
    /// Syntax files being read (`MAX_SYNTAX_JOBS`).
    syntax_jobs: Arc<AtomicUsize>,
    /// The port a server of ours stopped on, and when (`RELEASE_GRACE`).
    released: Option<(u16, Instant)>,
    /// The port a start failed on: tried again with the next script sent
    /// to the external editor, as each openInExternalEditor tries.
    failed_port: Option<u16>,
    /// A script was sent to the external editor this session.
    wanted: bool,
    /// The scripts sent to the external editor at the last frame.
    seen: Vec<ScriptTarget>,
    subs: Subscriptions<ScriptTarget>,
    /// The scripts offered, by editor, with their id.
    offered: HashMap<ScriptTarget, String>,
    aggregator: Aggregator,
    hello: Option<Hello>,
    /// AURORA_DEMO_EDITOR_SYNC's editor (`aurora_editor_sync::demo_client`).
    pub demo: Option<std::sync::mpsc::Receiver<aurora_editor_sync::demo_client::Seen>>,
}

impl EditorSync {
    /// The port the server listens on.
    pub fn port(&self) -> Option<u16> {
        self.server.as_ref().map(Server::port)
    }

    /// Whether the chat tap is wanted.
    pub fn wants_chat(&self, forward_debug: bool) -> bool {
        forward_debug && self.server.is_some() && !self.subs.is_empty()
    }

    /// Logout or quit: the server stops, nothing is kept.
    pub fn stop(&mut self) {
        *self = EditorSync::default();
    }

    /// Once a frame. `root` gives the root prim of a prim the viewer knows.
    pub fn update(
        &mut self,
        runtime: &tokio::runtime::Handle,
        frame: Frame<'_>,
        chat: &mut ChatTap,
        root: impl Fn(Uuid) -> Option<Uuid>,
    ) -> Vec<Tip> {
        let mut tips = Vec::new();
        if !frame.logged_in {
            if self.server.is_some() || self.wanted {
                self.stop();
            }
            chat.take();
            return tips;
        }
        if !frame.enabled {
            if self.server.is_some() {
                self.close();
                tips.push(Tip::Stopped);
            }
            self.failed_port = None;
            chat.take();
            return tips;
        }
        let new_script = frame.scripts.iter().any(|s| s.can_edit && !self.seen.contains(&s.target));
        self.seen = frame.scripts.iter().map(|s| s.target).collect();
        if new_script {
            self.wanted = true;
            self.failed_port = None;
        }
        let port = listen_port(frame.port);
        if self.server.is_some() && port != self.listening {
            // a new port: the server moves there once the setting settles
            match self.port_change {
                Some((p, since)) if p == port => {
                    if frame.now.saturating_duration_since(since) >= PORT_SETTLE {
                        self.port_change = None;
                        self.close();
                    }
                }
                _ => self.port_change = Some((port, frame.now)),
            }
        } else {
            self.port_change = None;
        }
        if self.server.is_none() && self.wanted && self.failed_port != Some(port) {
            let config = Config {
                port,
                limits: Limits::default(),
                methods: METHODS,
            };
            match Server::start(runtime, config, frame.hello.clone()) {
                Ok(server) => {
                    self.hello = Some(frame.hello.clone());
                    self.server = Some(server);
                    self.listening = port;
                    self.released = None;
                    tips.push(Tip::Started(port));
                }
                // our own server is still letting go of it: next frame
                Err(_) if self.released.is_some_and(|(p, at)| p == port && at.elapsed() < RELEASE_GRACE) => {}
                Err(e) => {
                    log::warn!("editor sync: cannot listen on port {port}: {e}");
                    self.failed_port = Some(port);
                    tips.push(Tip::Failed(port));
                }
            }
        }
        let Some(server) = self.server.as_mut() else {
            chat.take();
            return tips;
        };
        // the agent and the syntax of the next handshakes;
        // broadcastLanguageChange when the syntax changes
        if self.hello.as_ref() != Some(&frame.hello) {
            let changed = self.hello.as_ref().is_some_and(|h| h.syntax_id != frame.hello.syntax_id);
            let hello = frame.hello.clone();
            server.set_hello(|h| *h = hello);
            if changed {
                server.notify_all(
                    "language.syntax.change",
                    json!({ "id": frame.hello.syntax_id.hyphenated().to_string() }),
                );
            }
            self.hello = Some(frame.hello.clone());
        }
        // the scripts sent to the external editor (subscribeScriptEditor),
        // and those whose editor closed or stopped (unsubscribeScript)
        let gone: Vec<ScriptTarget> = self
            .offered
            .keys()
            .filter(|t| !frame.scripts.iter().any(|s| s.target == **t))
            .copied()
            .collect();
        for target in gone {
            if let Some(id) = self.offered.remove(&target)
                && let Some(conn) = self.subs.remove(&id)
            {
                server.notify(conn, "script.unsubscribe", json!({ "script_id": id }));
            }
        }
        for s in &frame.scripts {
            let same = match self.offered.get(&s.target).and_then(|id| self.subs.get(id)) {
                Some(e) => e.name == s.name && e.lua == s.lua,
                // offered while it may be edited, then kept while the
                // editor has it open (as the Legacy Viewer); one an editor
                // let go of (`script.unsubscribe`) may be followed again,
                // only while it may still be edited
                None => !s.can_edit,
            };
            if !same {
                let id = self.subs.offer(s.target.object, s.target.item, s.name, s.lua, s.target);
                self.offered.insert(s.target, id);
            }
        }
        while let Some(event) = server.try_event() {
            match event {
                // the server logged it; the core sync asks nothing of its features
                Event::Opened { .. } => {}
                Event::Closed { conn } => self.subs.drop_connection(conn),
                Event::Request { conn, id, method, params } => {
                    let call = Call {
                        conn,
                        id: &id,
                        responder: server.responder(),
                        runtime,
                        syntax_id: frame.hello.syntax_id,
                        jobs: &self.syntax_jobs,
                    };
                    let open = |t: &ScriptTarget| frame.scripts.iter().any(|s| s.target == *t);
                    if let Some(reply) = answer(&mut self.subs, &call, &method, &params, &open, &root) {
                        server.respond(conn, &id, reply);
                    }
                }
            }
        }
        // sendCompileResults
        for c in frame.compiled {
            if let Some(id) = self.offered.get(&c.target)
                && let Some(conn) = self.subs.connection(id)
            {
                server.notify(
                    conn,
                    "script.compiled",
                    diagnostics::compiled(id, c.compiled, c.running, &c.errors, c.lua),
                );
            }
        }
        // forwardChatToIDE
        let lines = chat.take();
        if frame.forward_debug {
            let mut bursts = Vec::new();
            for line in lines {
                let luau = |prim: Uuid, name: &str| self.subs.find_by_name(prim, name).is_some_and(|(_, e)| e.lua);
                bursts.extend(self.aggregator.ingest(line, frame.now, luau));
            }
            bursts.extend(self.aggregator.flush_expired(frame.now));
            for burst in bursts {
                let script = |prim: Uuid, name: &str| {
                    self.subs.find_by_name(prim, name).map(|(id, e)| runtime::Script {
                        script_id: id.clone(),
                        item: e.item,
                        lua: e.lua,
                    })
                };
                if let Some((method, params)) = runtime::event(&burst, &root, script) {
                    server.notify_all(method, params);
                }
            }
        } else {
            self.aggregator.flush();
        }
        tips
    }

    /// The setting went off or the port changed: the server stops, the
    /// scripts sent to the editor stay known.
    fn close(&mut self) {
        if self.server.take().is_some() {
            self.released = Some((self.listening, Instant::now()));
        }
        self.subs = Subscriptions::default();
        self.offered.clear();
        self.aggregator = Aggregator::default();
        self.hello = None;
    }
}

/// A request being answered.
struct Call<'a> {
    conn: ConnId,
    id: &'a Value,
    responder: Responder,
    runtime: &'a tokio::runtime::Handle,
    syntax_id: Uuid,
    jobs: &'a Arc<AtomicUsize>,
}

/// Gives a syntax job's place back, whatever becomes of the job.
struct JobSlot(Arc<AtomicUsize>);
impl Drop for JobSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Call<'_> {
    /// Reads and answers on the network runtime's blocking threads: a
    /// keyword file is up to a few MB. `None`: answered there; past
    /// `MAX_SYNTAX_JOBS`, the busy answer now.
    fn later(&self, work: impl FnOnce() -> Result<Value, RpcError> + Send + 'static) -> Option<Result<Value, RpcError>> {
        if self.jobs.fetch_add(1, Ordering::AcqRel) >= MAX_SYNTAX_JOBS {
            self.jobs.fetch_sub(1, Ordering::AcqRel);
            return Some(Err(RpcError::unavailable()));
        }
        let slot = JobSlot(self.jobs.clone());
        let (responder, conn, id) = (self.responder.clone(), self.conn, self.id.clone());
        self.runtime.spawn_blocking(move || {
            let _slot = slot;
            responder.respond(conn, &id, work());
        });
        None
    }
}

/// A request's answer, or `None` when it is answered later on another
/// thread (a syntax file read off the frame).
fn answer(
    subs: &mut Subscriptions<ScriptTarget>,
    call: &Call<'_>,
    method: &str,
    params: &Value,
    open: &impl Fn(&ScriptTarget) -> bool,
    root: &impl Fn(Uuid) -> Option<Uuid>,
) -> Option<Result<Value, RpcError>> {
    let text = |name: &str| params.get(name).and_then(Value::as_str).unwrap_or("");
    let conn = call.conn;
    let syntax_id = call.syntax_id.hyphenated().to_string();
    match method {
        // handleScriptSubscribe
        "script.subscribe" => {
            let script_id = text("script_id");
            subs.release_closed(|c| call.responder.is_open(c));
            let result = subs.subscribe(conn, script_id, open);
            if let Err(e) = &result {
                log::info!("editor sync: a subscription refused ({e:?})");
            }
            Some(Ok(subscribe_reply(script_id, result, |o| root(o).unwrap_or(o))))
        }
        // handleScriptUnsubscribe
        "script.unsubscribe" => {
            subs.unsubscribe(conn, text("script_id"));
            Some(Ok(Value::Null))
        }
        // handleFileWatcherFileListRequest; LLFile::tmpdir ends with a
        // separator, which editors may count on
        "script.list" => {
            let folder = crate::ui::scripts::live_folder().unwrap_or_else(std::env::temp_dir);
            let mut temp_dir = folder.display().to_string();
            if !temp_dir.ends_with(std::path::MAIN_SEPARATOR) {
                temp_dir.push(std::path::MAIN_SEPARATOR);
            }
            Some(Ok(json!({
                "temp_dir": temp_dir,
                "script_ids": subs.ids(),
                "success": true,
            })))
        }
        // handleLanguageIdRequest
        "language.syntax.id" => Some(Ok(json!({ "id": syntax_id }))),
        // handleSyntaxRequest
        "language.syntax" => {
            let file = match text("kind") {
                "" => return Some(Err(RpcError::invalid_params("No syntax category specified"))),
                "defs.lua" => SyntaxFile::Lua,
                "defs.lsl" => SyntaxFile::Lsl,
                _ => return Some(Err(RpcError::invalid_params("Unknown syntax category requested"))),
            };
            let source = file.source();
            call.later(move || {
                let defs = source
                    .and_then(|s| s.llsd())
                    .ok_or_else(|| RpcError::internal("Syntax definitions are unavailable"))?;
                Ok(json!({ "id": syntax_id, "defs": llsd_json(&defs), "success": true }))
            })
        }
        // handleSyntaxCacheRequest
        "language.syntax.cache" => {
            let files: Vec<&str> = [SyntaxFile::Lsl, SyntaxFile::Lua]
                .into_iter()
                .filter(|f| f.source().is_some())
                .map(SyntaxFile::name)
                .collect();
            Some(Ok(json!({ "files": files, "success": true })))
        }
        // handleSyntaxCacheFileRequest: only the files of the list
        "language.syntax.get" => {
            let filename = text("filename");
            if filename.is_empty() {
                return Some(Err(RpcError::invalid_params("No filename specified")));
            }
            let Some(source) = SyntaxFile::named(filename).and_then(SyntaxFile::source) else {
                return Some(Err(RpcError::invalid_params("Requested syntax cache file not found")));
            };
            let as_json = params.get("as_json").and_then(Value::as_bool).unwrap_or(false);
            call.later(move || {
                if as_json {
                    let doc = source
                        .llsd()
                        .ok_or_else(|| RpcError::internal("Failed to load and format syntax cache file."))?;
                    Ok(json!({ "content": llsd_json(&doc), "success": true }))
                } else {
                    let text = source
                        .bytes()
                        .and_then(|b| String::from_utf8(b).ok())
                        .filter(|t| !t.is_empty())
                        .ok_or_else(|| RpcError::internal("Failed to load syntax cache file"))?;
                    Ok(json!({ "content": text, "success": true }))
                }
            })
        }
        _ => Some(Err(RpcError::method_not_found(method))),
    }
}

/// A keyword file the editor may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyntaxFile {
    Lsl,
    Lua,
}

/// Where its text is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    File(PathBuf),
    BundledLua,
}

/// The largest keyword file read (the bundled SLua one is about 700 KB).
const MAX_SYNTAX_FILE: u64 = 4 * 1024 * 1024;

impl SyntaxFile {
    fn name(self) -> &'static str {
        match self {
            SyntaxFile::Lsl => LSL_FILE,
            SyntaxFile::Lua => LUA_FILE,
        }
    }

    fn named(name: &str) -> Option<SyntaxFile> {
        match name {
            LSL_FILE => Some(SyntaxFile::Lsl),
            LUA_FILE => Some(SyntaxFile::Lua),
            _ => None,
        }
    }

    /// The file of the keywords in use: the region's, from the cache;
    /// SLua's bundled one when the region gave none. The bundled LSL
    /// keywords have no file (Aurora keeps them as JSON).
    fn source(self) -> Option<Source> {
        let dir = crate::settings::cache_dir().join("lsl");
        match self {
            SyntaxFile::Lsl => {
                let id = crate::lsl_keywords::current_id()?;
                let path = aurora_net::lsl_syntax::cache_file(&dir, id);
                path.is_file().then_some(Source::File(path))
            }
            SyntaxFile::Lua => match crate::lsl_keywords::lua().id {
                Some(id) => {
                    let path = aurora_net::lsl_syntax::lua_cache_file(&dir, id);
                    Some(if path.is_file() { Source::File(path) } else { Source::BundledLua })
                }
                None => Some(Source::BundledLua),
            },
        }
    }
}

impl Source {
    fn bytes(&self) -> Option<Vec<u8>> {
        match self {
            Source::BundledLua => Some(crate::lsl_keywords::BUNDLED_LUA_XML.to_vec()),
            Source::File(path) => {
                let size = std::fs::metadata(path).ok()?.len();
                if size > MAX_SYNTAX_FILE {
                    log::warn!("editor sync: a syntax file of {size} bytes not sent");
                    return None;
                }
                std::fs::read(path).ok()
            }
        }
    }

    fn llsd(&self) -> Option<Llsd> {
        aurora_llsd::from_xml(&self.bytes()?).ok()
    }
}

/// LlsdToJson (indra/llcommon/llsdjson.cpp): a URI, a date and a UUID as
/// their strings. A binary, which the Legacy Viewer refuses, is null; so is
/// a real JSON cannot hold (NaN, infinity).
fn llsd_json(value: &Llsd) -> Value {
    match value {
        Llsd::Undef | Llsd::Binary(_) => Value::Null,
        Llsd::Boolean(b) => Value::Bool(*b),
        Llsd::Integer(i) => json!(i),
        Llsd::Real(r) => serde_json::Number::from_f64(*r).map_or(Value::Null, Value::Number),
        Llsd::String(s) | Llsd::Uri(s) => Value::String(s.clone()),
        Llsd::Uuid(u) => Value::String(u.hyphenated().to_string()),
        Llsd::Date(seconds) => Value::String(iso_date(*seconds)),
        Llsd::Array(items) => Value::Array(items.iter().map(llsd_json).collect()),
        Llsd::Map(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), llsd_json(v))).collect::<Map<String, Value>>()),
    }
}

/// LLDate::asString: `YYYY-MM-DDTHH:MM:SSZ`, UTC.
fn iso_date(seconds: f64) -> String {
    let total = if seconds.is_finite() { seconds.floor() as i64 } else { 0 };
    let (days, secs) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    // civil_from_days (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

/// gAgentUsername (LLStartUp): the first name, and the last unless it is
/// "Resident".
pub fn agent_name(first: &str, last: &str) -> String {
    if last.is_empty() || last.eq_ignore_ascii_case("resident") {
        first.to_owned()
    } else {
        format!("{first} {last}")
    }
}

#[cfg(test)]
mod tests;
