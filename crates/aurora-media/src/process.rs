//! Plugin process parent (LLPluginProcessParent): listens on a loopback TCP
//! port, launches SLPlugin with that port, tells it which plugin DLL to load
//! and exchanges `\0`-terminated LLSD XML messages with it. Internal messages
//! (hello, load_plugin_response, heartbeat, shm_*) are handled here; the
//! others are queued for the media class.
//!
//! Port of indra/llplugin/llpluginprocessparent.cpp (Copyright (C) Linden
//! Research, Inc., originally LGPL 2.1). Everything runs from `idle()` with non-blocking
//! sockets, like the parent's idle loop without its optional pump thread.

use crate::message::PluginMessage;
use crate::shm::SharedMemory;
use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub const CLASS_INTERNAL: &str = "internal";

/// mPluginLaunchTimeout / mPluginLockupTimeout.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);
const LOCKUP_TIMEOUT: Duration = Duration::from_secs(15);
/// GOODBYE_SECONDS of the child is 12 s: give it a little more.
const EXIT_TIMEOUT: Duration = Duration::from_secs(14);
/// Bytes read from the plugin per `idle` at most. The CEF and LibVLC
/// plugins send small LLSD messages (the pixels go through shared memory),
/// a few KB per frame; 256 KiB parses in a few milliseconds and still lets
/// about 15 MB/s through at 60 fps.
const INPUT_BUDGET: usize = 256 * 1024;
/// Size of one socket read.
const READ_CHUNK: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcState {
    /// Listening, process launched, waiting for its connection.
    Launched,
    /// Connected, waiting for "hello".
    Connected,
    /// load_plugin sent.
    Loading,
    Running,
    /// shutdown_plugin sent.
    Exiting,
    Done,
    Error(String),
}

pub struct PluginProcess {
    listener: Option<TcpListener>,
    stream: Option<TcpStream>,
    child: Option<Child>,
    state: ProcState,
    input: MessageFramer,
    output: Vec<u8>,
    heartbeat: Instant,
    /// LLPluginProcessParent::mBlocked: no heartbeat is sent while CEF
    /// waits for a native file picker (or another blocking response).
    blocked: bool,
    plugin_file: String,
    plugin_dir: String,
    shm: HashMap<String, SharedMemory>,
    received: VecDeque<PluginMessage>,
    sleep_time: f64,
    exit_deadline: Option<Instant>,
    pub plugin_version: String,
    pub cpu_usage: f64,
}

impl PluginProcess {
    /// The system id of the plugin process while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// Bind the listening socket and launch `launcher` (SLPlugin) with the
    /// port as its only argument, in `plugin_dir`, with `env` added to its
    /// environment.
    pub fn launch(launcher: &Path, plugin_dir: &Path, plugin_file: &Path, env: &[(String, String)]) -> std::io::Result<PluginProcess> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let mut cmd = Command::new(launcher);
        cmd.arg(port.to_string()).current_dir(plugin_dir);
        cmd.envs(env.iter().map(|(name, value)| (name, value)));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // Linux: CEF's host process (dullahan_host, started by CEF with this
        // environment) has no rpath that finds libcef.so; see
        // `plugin_library_path`.
        #[cfg(target_os = "linux")]
        if let Some(dir) = launcher.parent() {
            let inherited = std::env::var_os("LD_LIBRARY_PATH");
            if let Ok(joined) = std::env::join_paths(plugin_library_path(dir, inherited.as_deref())) {
                cmd.env("LD_LIBRARY_PATH", joined);
            }
        }
        let child = cmd.spawn()?;
        log::info!(
            "media: launched {} (port {port}) for {}",
            launcher.display(),
            plugin_file
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        Ok(PluginProcess {
            listener: Some(listener),
            stream: None,
            child: Some(child),
            state: ProcState::Launched,
            input: MessageFramer::default(),
            output: Vec::new(),
            heartbeat: Instant::now() + LAUNCH_TIMEOUT,
            blocked: false,
            plugin_file: plugin_file.to_string_lossy().into_owned(),
            plugin_dir: plugin_dir.to_string_lossy().into_owned(),
            shm: HashMap::new(),
            received: VecDeque::new(),
            sleep_time: 0.01,
            exit_deadline: None,
            plugin_version: String::new(),
            cpu_usage: 0.0,
        })
    }

    pub fn state(&self) -> &ProcState {
        &self.state
    }

    pub fn is_running(&self) -> bool {
        self.state == ProcState::Running
    }

    pub fn is_done(&self) -> bool {
        matches!(self.state, ProcState::Done | ProcState::Error(_))
    }

    pub fn error(&self) -> Option<&str> {
        match &self.state {
            ProcState::Error(e) => Some(e),
            _ => None,
        }
    }

    fn fail(&mut self, why: String) {
        if !matches!(self.state, ProcState::Error(_)) {
            log::warn!("media: plugin {} failed: {why}", self.plugin_file);
        }
        self.state = ProcState::Error(why);
        self.stream = None;
        self.listener = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Queue a message on the socket (sent by the next `idle`).
    pub fn send(&mut self, m: &PluginMessage) {
        if self.stream.is_none() {
            return;
        }
        if m.has("blocking_response") {
            self.blocked = false;
            // LLPluginProcessParent::sendMessage resets the timeout after
            // a blocking request: the resident may have taken any time.
            self.heartbeat = Instant::now() + LOCKUP_TIMEOUT;
        }
        log::trace!("media → plugin: {}:{}", m.class, m.name);
        self.output.extend_from_slice(m.generate().as_bytes());
        self.output.push(0);
    }

    /// setSleepTime: how long the plugin sleeps between idles.
    pub fn set_sleep_time(&mut self, t: f64, force: bool) {
        if force || (t - self.sleep_time).abs() > 1e-6 {
            self.sleep_time = t;
            let m = PluginMessage::new(CLASS_INTERNAL, "sleep_time").with("time", t);
            self.send(&m);
        }
    }

    /// Messages for the media class, in arrival order.
    pub fn take_messages(&mut self) -> VecDeque<PluginMessage> {
        std::mem::take(&mut self.received)
    }

    pub fn add_shared_memory(&mut self, size: usize) -> Option<String> {
        let seg = SharedMemory::create(size)?;
        let name = seg.name().to_string();
        let m = PluginMessage::new(CLASS_INTERNAL, "shm_add")
            .with("name", name.as_str())
            .with("size", size as i32);
        self.shm.insert(name.clone(), seg);
        self.send(&m);
        Some(name)
    }

    /// The plugin unmaps first; ours is released on shm_remove_response.
    pub fn remove_shared_memory(&mut self, name: &str) {
        if self.shm.contains_key(name) {
            let m = PluginMessage::new(CLASS_INTERNAL, "shm_remove").with("name", name);
            self.send(&m);
        }
    }

    pub fn shared_memory(&self, name: &str) -> Option<&SharedMemory> {
        self.shm.get(name)
    }

    /// requestShutdown: ask the plugin to exit; killed if it lingers.
    pub fn shutdown(&mut self) {
        match self.state {
            ProcState::Running | ProcState::Loading | ProcState::Connected => {
                let m = PluginMessage::new(CLASS_INTERNAL, "shutdown_plugin");
                self.send(&m);
                let _ = self.pump_output();
                self.state = ProcState::Exiting;
                self.exit_deadline = Some(Instant::now() + EXIT_TIMEOUT);
            }
            ProcState::Launched => self.fail("shut down before connecting".into()),
            _ => {}
        }
    }

    fn child_exited(&mut self) -> bool {
        match self.child.as_mut().map(|c| c.try_wait()) {
            Some(Ok(Some(status))) => {
                log::info!("media: plugin process exited ({status})");
                self.child = None;
                true
            }
            Some(Ok(None)) => false,
            Some(Err(_)) | None => true,
        }
    }

    pub fn idle(&mut self) {
        match self.state {
            ProcState::Done | ProcState::Error(_) => return,
            ProcState::Exiting => {
                let _ = self.pump_output();
                let _ = self.pump_input();
                if self.child_exited() {
                    self.state = ProcState::Done;
                    self.stream = None;
                } else if self.exit_deadline.is_some_and(|d| Instant::now() > d) {
                    self.fail("did not exit in time".into());
                }
                return;
            }
            _ => {}
        }
        if self.state == ProcState::Launched {
            match self.listener.as_ref().map(|l| l.accept()) {
                Some(Ok((stream, _))) => {
                    let _ = stream.set_nonblocking(true);
                    let _ = stream.set_nodelay(true);
                    self.stream = Some(stream);
                    self.listener = None;
                    self.state = ProcState::Connected;
                }
                Some(Err(e)) if e.kind() == ErrorKind::WouldBlock => {}
                Some(Err(e)) => return self.fail(format!("accept: {e}")),
                None => {}
            }
        }
        if self.stream.is_some() {
            if let Err(e) = self.pump_output() {
                return self.fail(format!("send: {e}"));
            }
            if let Err(e) = self.pump_input() {
                return self.fail(format!("receive: {e}"));
            }
        }
        // pluginLockedUpOrQuit
        if self.child_exited() {
            return self.fail("plugin process quit".into());
        }
        if self.timed_out(Instant::now()) {
            self.fail(
                if self.state == ProcState::Running {
                    "plugin locked up"
                } else {
                    "plugin launch timed out"
                }
                .into(),
            );
        }
    }

    fn timed_out(&self, now: Instant) -> bool {
        !self.blocked && now > self.heartbeat
    }

    fn pump_output(&mut self) -> std::io::Result<()> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(());
        };
        while !self.output.is_empty() {
            match stream.write(&self.output) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Read at most [`INPUT_BUDGET`] bytes from the plugin, then handle every
    /// complete message, in order. The budget keeps one frame short however
    /// much the plugin has queued (issue #2882: the old loop read until the
    /// socket was empty, which a plugin writing as fast as it is read never
    /// allows, then took each message off the front of one buffer, which
    /// grows with the square of the backlog); the rest stays in the socket
    /// for the next `idle`.
    fn pump_input(&mut self) -> std::io::Result<()> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(());
        };
        // The messages read before an error or the end are still handled.
        let end = read_budgeted(stream, &mut self.input, INPUT_BUDGET);
        while let Some(raw) = self.input.next_message() {
            let len = raw.len();
            match PluginMessage::parse(raw) {
                Some(m) => self.receive(m),
                None => log::debug!("media: unparsable plugin message ({} bytes)", len + 1),
            }
        }
        match end? {
            ReadEnd::Drained => {}
            // More is waiting: the plugin is talking, so it is not locked up
            // even while its heartbeat still waits behind the rest.
            ReadEnd::Budget => {
                if self.state == ProcState::Running {
                    self.heartbeat = self.heartbeat.max(Instant::now() + LOCKUP_TIMEOUT);
                }
            }
            ReadEnd::Closed if self.state == ProcState::Exiting => self.stream = None,
            ReadEnd::Closed => return Err(ErrorKind::ConnectionAborted.into()),
        }
        Ok(())
    }

    fn receive(&mut self, m: PluginMessage) {
        if m.has("blocking_request") {
            self.blocked = true;
        }
        if m.class != CLASS_INTERNAL {
            log::trace!("media ← plugin: {}:{}", m.class, m.name);
            self.received.push_back(m);
            return;
        }
        match m.name.as_str() {
            "hello" => {
                if self.state == ProcState::Connected {
                    let lp = PluginMessage::new(CLASS_INTERNAL, "load_plugin")
                        .with("file", self.plugin_file.as_str())
                        .with("dir", self.plugin_dir.as_str());
                    self.send(&lp);
                    self.state = ProcState::Loading;
                } else {
                    self.fail("hello in wrong state".into());
                }
            }
            "load_plugin_response" => {
                if self.state == ProcState::Loading {
                    self.plugin_version = m.string("plugin_version");
                    log::info!("media: plugin version {}", self.plugin_version);
                    let t = self.sleep_time;
                    self.set_sleep_time(t, true);
                    self.state = ProcState::Running;
                    self.heartbeat = Instant::now() + LOCKUP_TIMEOUT;
                } else {
                    self.fail("load_plugin_response in wrong state".into());
                }
            }
            "heartbeat" => {
                self.heartbeat = Instant::now() + LOCKUP_TIMEOUT;
                self.cpu_usage = m.real("cpu_usage");
            }
            "shm_add_response" => {}
            "shm_remove_response" => {
                let name = m.string("name");
                self.shm.remove(&name);
            }
            other => log::debug!("media: unknown internal message {other}"),
        }
    }
}

/// Splits the plugin's byte stream into its `\0`-terminated messages. The
/// bytes are walked forward once: handing out a message only moves the start
/// offset, and the consumed front is dropped once, before the next bytes are
/// appended (only a partial message is left to move by then), so the cost
/// stays linear in the data received.
#[derive(Default)]
struct MessageFramer {
    buf: Vec<u8>,
    /// Start of the first message not handed out yet.
    start: usize,
    /// Where the search for the next `\0` resumes: `start..scanned` holds
    /// none, so a long message arriving over several reads is scanned once.
    scanned: usize,
}

impl MessageFramer {
    fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 {
            self.buf.drain(..self.start);
            self.scanned -= self.start;
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete message, without its `\0`.
    fn next_message(&mut self) -> Option<&[u8]> {
        match self.buf[self.scanned..].iter().position(|&b| b == 0) {
            Some(i) => {
                let begin = self.start;
                let end = self.scanned + i;
                self.start = end + 1;
                self.scanned = self.start;
                Some(&self.buf[begin..end])
            }
            None => {
                self.scanned = self.buf.len();
                None
            }
        }
    }

    /// Bytes received and not handed out yet.
    #[cfg(test)]
    fn pending(&self) -> usize {
        self.buf.len() - self.start
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadEnd {
    /// Nothing more is waiting for now.
    Drained,
    /// The budget is used up; more may be waiting.
    Budget,
    /// The other side closed the connection.
    Closed,
}

/// Read from the non-blocking `reader` into `framer` until it would block,
/// closes, or `budget` bytes were read.
fn read_budgeted(reader: &mut impl Read, framer: &mut MessageFramer, budget: usize) -> std::io::Result<ReadEnd> {
    let mut buf = [0u8; READ_CHUNK];
    let mut left = budget;
    while left > 0 {
        let want = left.min(buf.len());
        match reader.read(&mut buf[..want]) {
            Ok(0) => return Ok(ReadEnd::Closed),
            Ok(n) => {
                framer.push(&buf[..n]);
                left -= n;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(ReadEnd::Drained),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(ReadEnd::Budget)
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        if matches!(self.state, ProcState::Running | ProcState::Loading | ProcState::Connected) {
            self.shutdown();
        }
        // give the plugin time to shut CEF down cleanly, then make sure it is gone
        if let Some(mut child) = self.child.take() {
            let stream = self.stream.take();
            let _ = std::thread::Builder::new().name("media plugin exit".into()).spawn(move || {
                let deadline = Instant::now() + EXIT_TIMEOUT;
                while Instant::now() < deadline {
                    if let Ok(Some(_)) = child.try_wait() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                drop(stream);
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

/// `LD_LIBRARY_PATH` of the plugin process on Linux: SLPlugin's folder
/// (Aurora's pack keeps libcef.so there, in `llplugin/`), then the `lib`
/// folder beside it when there is one (a Legacy Viewer install chosen as the
/// plugins folder: `bin/SLPlugin`, `lib/libcef.so`; its wrapper.sh puts
/// `lib` first in LD_LIBRARY_PATH, indra/newview/linux_tools/wrapper.sh),
/// then the inherited value. The folder is resolved first, as wrapper.sh
/// resolves its own path with `readlink -f`: `lib` sits beside the real
/// `bin`, not beside a link to it. Empty entries are dropped: the loader
/// reads one as the current folder, which wrapper.sh adds when
/// LD_LIBRARY_PATH was unset or empty.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn plugin_library_path(launcher_dir: &Path, inherited: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let dir = launcher_dir.canonicalize().unwrap_or_else(|_| launcher_dir.to_path_buf());
    let mut paths = vec![dir.clone()];
    if let Some(lib) = dir.parent().map(|p| p.join("lib")).filter(|lib| lib.is_dir()) {
        paths.push(lib);
    }
    if let Some(old) = inherited {
        paths.extend(std::env::split_paths(old).filter(|p| !p.as_os_str().is_empty()));
    }
    paths
}

/// File names of the launcher: `SLPlugin.exe` on Windows, `SLPlugin`
/// elsewhere.
const LAUNCHERS: &[&str] = if cfg!(windows) {
    &["SLPlugin.exe", "slplugin.exe"]
} else {
    &["SLPlugin"]
};

/// Where SLPlugin and the media plugin libraries live.
#[derive(Debug, Clone)]
pub struct PluginPaths {
    pub launcher: PathBuf,
    pub plugin_dir: PathBuf,
}

impl PluginPaths {
    /// Path of a plugin library (`media_plugin_cef`, `media_plugin_libvlc`,
    /// `media_plugin_gstreamer`): `<name>.dll` on Windows, `<name>.dylib` on
    /// macOS (LLDir_Mac::getLLPluginFilename), `lib<name>.so` elsewhere.
    pub fn plugin_file(&self, basename: &str) -> PathBuf {
        let file = if cfg!(windows) {
            format!("{basename}.dll")
        } else if cfg!(target_os = "macos") {
            format!("{basename}.dylib")
        } else {
            format!("lib{basename}.so")
        };
        self.plugin_dir.join(file)
    }

    pub fn has_plugin(&self, basename: &str) -> bool {
        self.plugin_file(basename).is_file()
    }

    /// A folder holding `llplugin\` and SLPlugin (either inside `llplugin\`,
    /// as in Aurora's pack, or next to it, as in a Firestorm / SL install);
    /// on macOS, a folder holding SLPlugin.app ([`Self::bundle_in`]).
    pub fn in_dir(dir: &Path) -> Option<PluginPaths> {
        if cfg!(target_os = "macos") {
            return Self::bundle_in(dir);
        }
        let plugin_dir = dir.join("llplugin");
        if !plugin_dir.is_dir() {
            return None;
        }
        for exe in LAUNCHERS {
            for base in [dir, plugin_dir.as_path()] {
                let launcher = base.join(exe);
                if launcher.is_file() {
                    return Some(PluginPaths {
                        launcher,
                        plugin_dir: plugin_dir.clone(),
                    });
                }
            }
        }
        None
    }

    /// macOS: SLPlugin.app laid out as in the Legacy Viewer's application
    /// (indra/llfilesystem/lldir_mac.cpp: the launcher in its Contents/MacOS,
    /// the plugins, CEF's framework and helpers and LibVLC in its
    /// Contents/Frameworks, the folder SLPlugin runs in). Found in
    /// `llplugin/` (Aurora's pack), in the folder itself, or in the
    /// Contents/Resources of an application chosen as the folder (an
    /// installed Legacy Viewer's .app).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn bundle_in(dir: &Path) -> Option<PluginPaths> {
        [dir.join("llplugin"), dir.to_path_buf(), dir.join("Contents").join("Resources")]
            .into_iter()
            .map(|base| base.join("SLPlugin.app"))
            .find_map(|app| {
                let contents = app.join("Contents");
                let launcher = contents.join("MacOS").join("SLPlugin");
                launcher.is_file().then(|| PluginPaths {
                    launcher,
                    plugin_dir: contents.join("Frameworks"),
                })
            })
    }

    /// Search order: the folder chosen by the user, the viewer's own folder,
    /// then Aurora's media plugins pack. The plugins of another program are
    /// only used when the user names its folder: an installed Firestorm or
    /// Second Life viewer is not looked for.
    ///
    /// A relative folder is made absolute against the viewer's current folder
    /// first: SLPlugin is started in `plugin_dir`, so a relative launcher
    /// (Unix), `LD_LIBRARY_PATH` (Linux) or plugin file and folder sent in
    /// `load_plugin` would then be looked for from there and never found.
    /// `std::path::absolute`, not `canonicalize`, so that Windows paths keep
    /// no `\\?\` prefix and links stay as the user chose them. A folder that
    /// cannot be made absolute (current folder deleted) is skipped.
    pub fn discover(explicit: Option<&Path>, pack: Option<&Path>) -> Option<PluginPaths> {
        let own = std::env::current_exe().ok();
        let own = own.as_deref().and_then(Path::parent);
        [explicit, own, pack]
            .into_iter()
            .flatten()
            .filter_map(|dir| std::path::absolute(dir).ok())
            .find_map(|dir| PluginPaths::in_dir(&dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A running process on the viewer's end of a loopback connection, and
    /// the plugin's end.
    fn running(heartbeat: Instant) -> (PluginProcess, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("listener");
        let plugin = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (stream, _) = listener.accept().expect("accept");
        stream.set_nonblocking(true).expect("non-blocking");
        let process = PluginProcess {
            listener: None,
            stream: Some(stream),
            child: None,
            state: ProcState::Running,
            input: MessageFramer::default(),
            output: Vec::new(),
            heartbeat,
            blocked: false,
            plugin_file: String::new(),
            plugin_dir: String::new(),
            shm: HashMap::new(),
            received: VecDeque::new(),
            sleep_time: 0.01,
            exit_deadline: None,
            plugin_version: String::new(),
            cpu_usage: 0.0,
        };
        (process, plugin)
    }

    /// `count` media messages numbered from 0, each ended by its `\0`.
    fn numbered(count: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        for i in 0..count {
            let m = PluginMessage::new("media", "navigate_complete").with("index", i as i32);
            bytes.extend_from_slice(m.generate().as_bytes());
            bytes.push(0);
        }
        bytes
    }

    /// A non-blocking stream holding a backlog: `chunk` bytes per read at
    /// most, then `WouldBlock` once it is empty.
    struct Backlog {
        data: Vec<u8>,
        pos: usize,
        chunk: usize,
    }

    impl Read for Backlog {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.chunk).min(self.data.len() - self.pos);
            if n == 0 {
                return Err(ErrorKind::WouldBlock.into());
            }
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    fn indices(framer: &mut MessageFramer, into: &mut Vec<i32>) {
        while let Some(raw) = framer.next_message() {
            into.push(PluginMessage::parse(raw).expect("message").s32("index"));
        }
    }

    #[test]
    fn many_messages_in_one_read_come_out_in_order() {
        let mut framer = MessageFramer::default();
        framer.push(&numbered(5000));
        let mut seen = Vec::new();
        indices(&mut framer, &mut seen);
        assert_eq!(seen, (0..5000).collect::<Vec<_>>());
        assert_eq!(framer.pending(), 0);
        // the next bytes start a fresh message
        framer.push(&numbered(1));
        indices(&mut framer, &mut seen);
        assert_eq!(seen.len(), 5001);
        assert_eq!(seen[5000], 0);
    }

    #[test]
    fn a_message_split_across_reads_is_handed_out_once_complete() {
        let bytes = numbered(3);
        // every split point, including inside a terminator's neighbours
        for cut in 0..=bytes.len() {
            let mut framer = MessageFramer::default();
            let mut seen = Vec::new();
            framer.push(&bytes[..cut]);
            indices(&mut framer, &mut seen);
            framer.push(&bytes[cut..]);
            indices(&mut framer, &mut seen);
            assert_eq!(seen, vec![0, 1, 2], "cut at {cut}");
            assert_eq!(framer.pending(), 0);
        }
        // one byte per read
        let mut framer = MessageFramer::default();
        let mut seen = Vec::new();
        for b in &bytes {
            framer.push(std::slice::from_ref(b));
            indices(&mut framer, &mut seen);
        }
        assert_eq!(seen, vec![0, 1, 2]);
    }

    #[test]
    fn a_huge_backlog_is_read_within_the_budget_of_each_frame_and_nothing_is_lost() {
        let count = 100_000;
        let data = numbered(count);
        let total = data.len();
        let largest = data.split(|&b| b == 0).map(<[u8]>::len).max().unwrap_or(0);
        let mut stream = Backlog {
            data,
            pos: 0,
            chunk: READ_CHUNK,
        };
        let mut framer = MessageFramer::default();
        let mut seen = Vec::with_capacity(count);
        let started = Instant::now();
        let mut frames = 0;
        loop {
            let before = stream.pos;
            let end = read_budgeted(&mut stream, &mut framer, INPUT_BUDGET).expect("read");
            assert!(stream.pos - before <= INPUT_BUDGET, "frame {frames} read past its budget");
            indices(&mut framer, &mut seen);
            // only a partial message is ever carried to the next frame
            assert!(framer.pending() <= largest);
            frames += 1;
            if end == ReadEnd::Drained {
                break;
            }
            assert_eq!(end, ReadEnd::Budget);
        }
        assert_eq!(seen, (0..count as i32).collect::<Vec<_>>());
        assert_eq!(frames, total / INPUT_BUDGET + 1);
        // Linear work: well under a second even in a debug build (the old
        // reader took minutes on such a backlog).
        assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
    }

    #[test]
    fn a_closed_connection_still_delivers_what_was_read() {
        let mut framer = MessageFramer::default();
        let mut seen = Vec::new();
        let mut closed: &[u8] = &numbered(2);
        assert_eq!(
            read_budgeted(&mut closed, &mut framer, INPUT_BUDGET).expect("read"),
            ReadEnd::Closed
        );
        indices(&mut framer, &mut seen);
        assert_eq!(seen, vec![0, 1]);
    }

    #[test]
    fn a_flooding_plugin_is_read_a_budget_per_idle_in_order_and_stays_alive() {
        let (mut process, mut plugin) = running(Instant::now() + LOCKUP_TIMEOUT);
        let count = 20_000;
        let data = numbered(count);
        let shortest = data.split(|&b| b == 0).map(<[u8]>::len).filter(|&n| n > 0).min().unwrap_or(1) + 1;
        let writer = std::thread::spawn(move || {
            plugin.write_all(&data).expect("write");
            plugin
        });
        let mut seen = Vec::with_capacity(count);
        let deadline = Instant::now() + Duration::from_secs(60);
        while seen.len() < count {
            assert!(Instant::now() < deadline, "only {} of {count} messages arrived", seen.len());
            process.pump_input().expect("pump");
            let got = process.take_messages();
            // one budget of bytes, plus the message completed from before
            assert!(got.len() <= INPUT_BUDGET / shortest + 1, "{} messages in one idle", got.len());
            seen.extend(got.iter().map(|m| m.s32("index")));
            if got.is_empty() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(seen, (0..count as i32).collect::<Vec<_>>());
        assert!(!process.timed_out(Instant::now()));
        drop(writer.join().expect("writer"));
        process.state = ProcState::Done;
    }

    #[test]
    fn media_file_picker_suspends_lockup_timeout_until_the_blocking_response() {
        let now = Instant::now();
        let (mut process, _plugin) = running(now - Duration::from_secs(1));
        assert!(process.timed_out(now));
        process.receive(
            PluginMessage::new("media", "pick_file")
                .with("blocking_request", true)
                .with("multiple_files", true),
        );
        assert!(!process.timed_out(now + Duration::from_secs(600)));
        assert_eq!(process.take_messages().front().expect("file request").name, "pick_file");
        // Ordinary traffic must not release the wait for the picker.
        process.send(&PluginMessage::new("media", "set_volume").with("volume", 0.5));
        assert!(!process.timed_out(now + Duration::from_secs(600)));
        process.send(&crate::file_picker::response(Vec::new()));
        assert!(!process.blocked);
        assert!(!process.timed_out(Instant::now()));
        assert!(process.timed_out(Instant::now() + LOCKUP_TIMEOUT + Duration::from_secs(1)));
        process.state = ProcState::Done;
    }

    fn folder(root: &Path, name: &str, launcher_inside: bool) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("llplugin")).expect("fixture");
        let launcher = if launcher_inside { dir.join("llplugin") } else { dir.clone() }.join(LAUNCHERS[0]);
        std::fs::write(launcher, b"").expect("fixture");
        dir
    }

    // (macOS looks for SLPlugin.app instead: the test below)
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_chosen_folder_comes_before_the_pack_and_nothing_else_is_searched() {
        let root = std::env::temp_dir().join(format!("aurora-media-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let chosen = folder(&root, "chosen", false);
        let pack = folder(&root, "pack", true);
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).expect("fixture");

        let found = PluginPaths::discover(Some(&chosen), Some(&pack)).expect("chosen folder");
        assert_eq!(found.launcher, chosen.join(LAUNCHERS[0]));
        // a folder without plugins falls through to the pack
        let found = PluginPaths::discover(Some(&empty), Some(&pack)).expect("pack");
        assert_eq!(found.launcher, pack.join("llplugin").join(LAUNCHERS[0]));
        assert_eq!(found.plugin_dir, pack.join("llplugin"));
        assert_eq!(
            found.plugin_file("media_plugin_cef"),
            pack.join("llplugin").join(if cfg!(windows) {
                "media_plugin_cef.dll"
            } else {
                "libmedia_plugin_cef.so"
            })
        );
        // no pack: nothing, even on a PC where another viewer is installed
        assert!(PluginPaths::discover(Some(&empty), None).is_none());
        assert!(PluginPaths::discover(None, None).is_none());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn plugin_library_path_adds_the_lib_folder_beside_slplugin_and_drops_empty_entries() {
        let root = std::env::temp_dir().join(format!("aurora-media-ldpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // a Legacy Viewer install: bin/SLPlugin, lib/libcef.so
        let bin = root.join("legacy").join("bin");
        let lib = root.join("legacy").join("lib");
        std::fs::create_dir_all(&bin).expect("fixture");
        std::fs::create_dir_all(&lib).expect("fixture");
        // Aurora's pack: llplugin/SLPlugin, nothing beside llplugin
        let pack = root.join("pack").join("llplugin");
        std::fs::create_dir_all(&pack).expect("fixture");
        // the paths come back resolved (macOS' temp folder is a link,
        // Windows' gets its `\\?\` prefix)
        let (bin, lib, pack) = (
            bin.canonicalize().expect("fixture"),
            lib.canonicalize().expect("fixture"),
            pack.canonicalize().expect("fixture"),
        );
        let sep = if cfg!(windows) { ";" } else { ":" };

        assert_eq!(plugin_library_path(&bin, None), vec![bin.clone(), lib.clone()]);
        assert_eq!(plugin_library_path(&pack, None), vec![pack.clone()]);
        let inherited = std::ffi::OsString::from(format!("/usr/local/lib{sep}{sep}/opt/x"));
        assert_eq!(
            plugin_library_path(&pack, Some(&inherited)),
            vec![pack.clone(), PathBuf::from("/usr/local/lib"), PathBuf::from("/opt/x")]
        );
        // set but empty: no trailing empty entry (the current folder)
        assert_eq!(
            plugin_library_path(&bin, Some(std::ffi::OsStr::new(""))),
            vec![bin.clone(), lib.clone()]
        );
        // a link to the install's bin: lib is found beside the real folder
        #[cfg(unix)]
        {
            let link = root.join("link-to-bin");
            std::os::unix::fs::symlink(&bin, &link).expect("fixture");
            assert_eq!(plugin_library_path(&link, None), vec![bin.clone(), lib]);
        }
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn macos_finds_slplugin_app_in_the_pack_a_folder_or_an_application() {
        let root = std::env::temp_dir().join(format!("aurora-media-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bundle = |base: &Path| {
            let macos = base.join("SLPlugin.app").join("Contents").join("MacOS");
            std::fs::create_dir_all(&macos).expect("fixture");
            std::fs::write(macos.join("SLPlugin"), b"").expect("fixture");
            base.join("SLPlugin.app").join("Contents")
        };
        let pack = root.join("pack");
        let pack_contents = bundle(&pack.join("llplugin"));
        let legacy = root.join("Legacy.app");
        let legacy_contents = bundle(&legacy.join("Contents").join("Resources"));
        let found = PluginPaths::bundle_in(&pack).expect("pack");
        assert_eq!(found.launcher, pack_contents.join("MacOS").join("SLPlugin"));
        assert_eq!(found.plugin_dir, pack_contents.join("Frameworks"));
        let found = PluginPaths::bundle_in(&legacy).expect("an application");
        assert_eq!(found.plugin_dir, legacy_contents.join("Frameworks"));
        assert!(PluginPaths::bundle_in(&root).is_none(), "nothing directly in the root");
        let _ = std::fs::remove_dir_all(&root);
    }

    // (macOS looks for SLPlugin.app instead)
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_relative_folder_gives_an_absolute_launcher_and_plugin_dir() {
        // relative to the test's current folder, as a folder typed in the
        // preferences is relative to the viewer's
        let rel = PathBuf::from(format!("aurora-media-relative-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&rel);
        folder(Path::new("."), rel.to_str().expect("ascii name"), false);
        let found = PluginPaths::discover(Some(&rel), None).expect("relative folder");
        let abs = std::env::current_dir().expect("current folder").join(&rel);
        std::fs::remove_dir_all(&rel).expect("cleanup");
        assert!(found.launcher.is_absolute(), "{}", found.launcher.display());
        assert_eq!(found.launcher, abs.join(LAUNCHERS[0]));
        assert_eq!(found.plugin_dir, abs.join("llplugin"));
    }
}
