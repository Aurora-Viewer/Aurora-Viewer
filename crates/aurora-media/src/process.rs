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
    /// The system id of the plugin process while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

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

/// `LD_LIBRARY_PATH` of the plugin process on Linux: SLPlugin's folder
/// (Aurora's pack keeps libcef.so there, in `llplugin/`), then the `lib`
/// folder beside it when there is one (a Legacy Viewer install chosen as the
/// plugins folder: `bin/SLPlugin`, `lib/libcef.so`; its wrapper.sh puts
/// `lib` first in LD_LIBRARY_PATH, indra/newview/linux_tools/wrapper.sh),
/// then the inherited value. Empty entries are dropped: the loader reads one
/// as the current folder, which wrapper.sh adds when LD_LIBRARY_PATH was
/// unset or empty.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn plugin_library_path(launcher_dir: &Path, inherited: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut paths = vec![launcher_dir.to_path_buf()];
    if let Some(lib) = launcher_dir.parent().map(|p| p.join("lib")).filter(|lib| lib.is_dir()) {
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
    pub fn discover(explicit: Option<&Path>, pack: Option<&Path>) -> Option<PluginPaths> {
        let own = std::env::current_exe().ok();
        let own = own.as_deref().and_then(Path::parent);
        [explicit, own, pack].into_iter().flatten().find_map(PluginPaths::in_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let sep = if cfg!(windows) { ";" } else { ":" };

        assert_eq!(plugin_library_path(&bin, None), vec![bin.clone(), lib.clone()]);
        assert_eq!(plugin_library_path(&pack, None), vec![pack.clone()]);
        let inherited = std::ffi::OsString::from(format!("/usr/local/lib{sep}{sep}/opt/x"));
        assert_eq!(
            plugin_library_path(&pack, Some(&inherited)),
            vec![pack.clone(), PathBuf::from("/usr/local/lib"), PathBuf::from("/opt/x")]
        );
        // set but empty: no trailing empty entry (the current folder)
        assert_eq!(plugin_library_path(&bin, Some(std::ffi::OsStr::new(""))), vec![bin.clone(), lib]);
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
}
