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
    input: Vec<u8>,
    output: Vec<u8>,
    heartbeat: Instant,
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
    /// Bind the listening socket and launch `launcher` (SLPlugin) with the
    /// port as its only argument, in `plugin_dir`.
    pub fn launch(launcher: &Path, plugin_dir: &Path, plugin_file: &Path) -> std::io::Result<PluginProcess> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let mut cmd = Command::new(launcher);
        cmd.arg(port.to_string()).current_dir(plugin_dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
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
            input: Vec::new(),
            output: Vec::new(),
            heartbeat: Instant::now() + LAUNCH_TIMEOUT,
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
        if Instant::now() > self.heartbeat {
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

    fn pump_input(&mut self) -> std::io::Result<()> {
        let mut buf = [0u8; 16384];
        loop {
            let Some(stream) = self.stream.as_mut() else {
                return Ok(());
            };
            match stream.read(&mut buf) {
                Ok(0) => {
                    if self.state == ProcState::Exiting {
                        self.stream = None;
                        return Ok(());
                    }
                    return Err(ErrorKind::ConnectionAborted.into());
                }
                Ok(n) => self.input.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        while let Some(end) = self.input.iter().position(|&b| b == 0) {
            let raw: Vec<u8> = self.input.drain(..=end).collect();
            match PluginMessage::parse(&raw[..raw.len() - 1]) {
                Some(m) => self.receive(m),
                None => log::debug!("media: unparsable plugin message ({} bytes)", raw.len()),
            }
        }
        Ok(())
    }

    fn receive(&mut self, m: PluginMessage) {
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

/// Where SLPlugin and the media plugin DLLs live.
#[derive(Debug, Clone)]
pub struct PluginPaths {
    pub launcher: PathBuf,
    pub plugin_dir: PathBuf,
}

impl PluginPaths {
    /// Path of a plugin DLL (`media_plugin_cef`, `media_plugin_libvlc`).
    pub fn plugin_file(&self, basename: &str) -> PathBuf {
        let file = if cfg!(windows) {
            format!("{basename}.dll")
        } else {
            format!("lib{basename}.so")
        };
        self.plugin_dir.join(file)
    }

    pub fn has_plugin(&self, basename: &str) -> bool {
        self.plugin_file(basename).is_file()
    }

    /// A folder holding `llplugin\` and SLPlugin (either next to it, as in a
    /// Firestorm / SL install, or inside `llplugin\`).
    pub fn in_dir(dir: &Path) -> Option<PluginPaths> {
        let plugin_dir = dir.join("llplugin");
        if !plugin_dir.is_dir() {
            return None;
        }
        for exe in ["SLPlugin.exe", "slplugin.exe"] {
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

    /// Search order: an explicit folder, the viewer's own folder, then
    /// installed Firestorm / Second Life viewers.
    pub fn discover(explicit: Option<&Path>) -> Option<PluginPaths> {
        if let Some(p) = explicit.and_then(PluginPaths::in_dir) {
            return Some(p);
        }
        if let Some(p) = std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(Path::parent)
            .and_then(PluginPaths::in_dir)
        {
            return Some(p);
        }
        let mut roots = Vec::new();
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Ok(v) = std::env::var(var) {
                roots.push(PathBuf::from(v));
            }
        }
        let mut candidates = Vec::new();
        for root in roots {
            let Ok(rd) = std::fs::read_dir(&root) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_lowercase();
                // Firestorm first (same plugins as the viewer we mirror)
                let rank = if name.starts_with("firestorm") {
                    0
                } else if name.starts_with("secondlife") || name.starts_with("second life") {
                    1
                } else {
                    continue;
                };
                candidates.push((rank, e.path()));
            }
        }
        candidates.sort();
        candidates.into_iter().find_map(|(_, p)| PluginPaths::in_dir(&p))
    }
}
