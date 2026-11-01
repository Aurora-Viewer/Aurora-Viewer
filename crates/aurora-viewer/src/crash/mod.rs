//! Crash recovery, independent of the viewer's event loop and GPU.
//! Firestorm reference: llappviewer.cpp (errorCallback, marker files) and
//! llcrashlogger.cpp (recovery on the next launch). Deliberate difference:
//! a report is sent only as a public GitHub issue in the tester's name
//! (`github.rs`), with the text they can read first (`summary.rs`), and
//! asked for by default; the full diagnostics stay in the session folder.

pub mod github;
mod native;
pub mod privacy;
// Bug report form (disabled, see main.rs):
// mod report;
mod software;
mod summary;
mod threads;
mod watchdog;
mod window;

pub use watchdog::{State, note as note_state, watch_window};

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
#[cfg(windows)]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_RECORD: u64 = 128 * 1024;
const MAX_TRACE: u64 = 256 * 1024;
static SESSION: OnceLock<Session> = OnceLock::new();

#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema: u32,
    pub pid: u32,
    pub process_started: u64,
    pub started_unix: u64,
    pub version: String,
    #[serde(default)]
    pub commit: String,
    pub profile: String,
    pub language: String,
    pub font: String,
    pub font_scale: f32,
    pub theme: crate::theme::Theme,
    pub log_path: PathBuf,
    pub demo: bool,
    #[serde(default)]
    pub os: String,
    /// The Preferences choice of the session that crashed.
    #[serde(default)]
    pub crash_reports: crate::settings::CrashReports,
}

struct Session {
    dir: PathBuf,
    panic_file: std::fs::File,
}

/// The log has an immutable session-specific path, so a second instance or
/// an immediate restart cannot rotate it into another crash's report.
pub fn session_log() -> Option<std::fs::File> {
    std::fs::File::create(SESSION.get()?.dir.join("session.log")).ok()
}

fn root() -> PathBuf {
    root_for(std::env::var_os("AURORA_DEMO").is_some(), std::env::var_os("AURORA_DEMO_CRASH_DIR"))
}

fn root_for(demo: bool, demo_dir: Option<std::ffi::OsString>) -> PathBuf {
    if demo && let Some(path) = demo_dir {
        return PathBuf::from(path);
    }
    crate::logging::logs_dir()
        .join("crashes")
        .join(if demo { "demo" } else { "sessions" })
}

fn read_limited(path: &Path, limit: u64) -> std::io::Result<String> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(std::io::Error::other("crash file exceeds the size limit"));
    }
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text)?;
    if text.len() as u64 > limit {
        return Err(std::io::Error::other("crash file grew beyond the size limit"));
    }
    Ok(text)
}

fn load(dir: &Path) -> anyhow::Result<Record> {
    let record: Record = serde_json::from_str(&read_limited(&dir.join("session.json"), MAX_RECORD)?)?;
    anyhow::ensure!(record.schema == 1, "unsupported crash session format");
    Ok(record)
}

fn save(dir: &Path, record: &Record) -> anyhow::Result<()> {
    // Rename publishes a complete snapshot, including when the viewer dies
    // midway through an update. The watcher never reads a half-written JSON.
    let pending = dir.join("session.pending");
    let mut file = std::fs::File::create(&pending)?;
    file.write_all(&serde_json::to_vec_pretty(record)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(pending, dir.join("session.json"))?;
    Ok(())
}

/// Internal helpers are dispatched before logging, credentials, networking,
/// or renderer initialization. They cannot recursively start another watcher.
pub fn helper() -> anyhow::Result<bool> {
    let args: Vec<_> = std::env::args_os().collect();
    let Some(index) = args.iter().position(|arg| arg == "--crash-watch" || arg == "--crash-show") else {
        return Ok(false);
    };
    let id = args.get(index + 1).ok_or_else(|| anyhow::anyhow!("missing crash session"))?;
    let dir = session_dir(&root(), id)?;
    let record = load(&dir)?;
    if args[index] == "--crash-watch" {
        native::watch(&dir, &record)?;
    } else {
        show_once(&dir)?;
    }
    Ok(true)
}

/// Never accept arbitrary files or paths from a command line / report: a
/// session is named by its id and looked up under the helper's own root.
/// Do not compare canonical paths instead. A viewer started from a packaged
/// (MSIX) application has its new folders stored in the package's cache,
/// while the root they appear in keeps its ordinary location: the canonical
/// parent of a session is then not the canonical root.
fn session_dir(root: &Path, id: &std::ffi::OsStr) -> anyhow::Result<PathBuf> {
    let id = uuid::Uuid::parse_str(&id.to_string_lossy()).map_err(|_| anyhow::anyhow!("invalid session id"))?;
    let dir = root.join(id.to_string());
    // A link under that name would lead out of the root.
    anyhow::ensure!(
        dir.symlink_metadata().is_ok_and(|entry| entry.is_dir()),
        "invalid crash session directory"
    );
    // Where the folder is really stored: the file manager opened from the
    // window is not redirected like this process and needs that location.
    Ok(dir.canonicalize()?)
}

fn helper_command(dir: &Path, mode: &str, demo: bool) -> std::io::Result<std::process::Command> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    let (Some(session_root), Some(id)) = (dir.parent(), dir.file_name()) else {
        return Err(std::io::ErrorKind::InvalidInput.into());
    };
    let session_root = std::path::absolute(session_root)?;
    command.arg(mode).arg(id);
    // An inherited installation working directory would block the updater's
    // directory swap on Windows, even after the viewer itself has exited.
    command.current_dir(&session_root);
    if demo {
        // The helper's working directory differs from the viewer's: a
        // relative AURORA_DEMO_CRASH_DIR would name another folder there.
        command.env("AURORA_DEMO_CRASH_DIR", &session_root);
    }
    Ok(command)
}

fn spawn_helper(dir: &Path, mode: &str) -> std::io::Result<std::process::Child> {
    let mut command = helper_command(dir, mode, std::env::var_os("AURORA_DEMO").is_some())?;
    if let Some(title) = &crate::cli::get().title {
        command.arg("--title").arg(title);
    }
    if let Some(fps) = crate::cli::get().fps_limit {
        command.arg("--fps-limit").arg(fps.to_string());
    }
    command
        .env_remove("AURORA_DEMO_CRASH")
        .env_remove("AURORA_CAPTURE")
        .env_remove("AURORA_CAPTURE_EXIT");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command.spawn()
}

/// Best effort: failure to start crash recovery must not prevent startup.
pub fn start() {
    if let Err(error) = start_inner() {
        eprintln!("cannot start crash recovery: {error}");
    }
}

fn start_inner() -> anyhow::Result<()> {
    std::fs::create_dir_all(root())?;
    recover();
    let settings = crate::settings::Settings::load();
    aurora_i18n::select(&settings.language, crate::cli::get().language.as_deref());
    let dir = root().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir(&dir)?;
    let record = Record {
        schema: 1,
        pid: std::process::id(),
        process_started: native::current_process_started(),
        started_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        version: aurora_version::version().to_owned(),
        commit: aurora_version::commit().to_owned(),
        profile: env!("AURORA_BUILD_PROFILE").to_owned(),
        language: aurora_i18n::language().into(),
        font: settings.font,
        font_scale: settings.font_scale,
        theme: crate::ui::skin::recovery_theme(),
        log_path: crate::logging::log_path(),
        demo: std::env::var_os("AURORA_DEMO").is_some(),
        os: format!(
            "{} ({}, {})",
            sysinfo::System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into()),
            sysinfo::System::kernel_version().unwrap_or_default(),
            std::env::consts::ARCH
        ),
        crash_reports: settings.crash_reports,
    };
    save(&dir, &record)?;
    let panic_file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("panic.txt"))?;
    native::install(&dir)?;
    watchdog::start(&dir);
    let _ = SESSION.set(Session {
        dir: dir.clone(),
        panic_file,
    });
    if record.demo && std::env::var("AURORA_DEMO_CRASH").as_deref() == Ok("recovery") {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let mut child = spawn_helper(&dir, "--crash-watch")?;
        // The child must hold a process handle before a very early crash.
        // A bounded handshake also avoids losing crashes to PID reuse.
        let deadline = Instant::now() + Duration::from_secs(3);
        while !dir.join("watching").exists() && Instant::now() < deadline {
            if child.try_wait()?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    Ok(())
}

pub fn update(theme: &crate::theme::Theme, settings: &crate::settings::Settings) {
    if let Some(session) = SESSION.get()
        && let Ok(mut record) = load(&session.dir)
    {
        record.theme = theme.clone();
        record.language = aurora_i18n::language().into();
        record.font.clone_from(&settings.font);
        record.font_scale = settings.font_scale;
        record.crash_reports = settings.crash_reports;
        if let Err(error) = save(&session.dir, &record) {
            log::warn!("cannot update crash session: {error}");
        }
    }
}

pub fn panic(info: &std::panic::PanicHookInfo<'_>, trace: &std::backtrace::Backtrace) {
    if let Some(session) = SESSION.get() {
        let mut file = &session.panic_file;
        let _ = writeln!(file, "Rust panic: {info}\n{trace}");
        // A release build has no debug info for the trace above to resolve.
        native::write_stack(&mut file);
        let _ = file.sync_all();
        threads::on_panic(&session.dir);
    }
}

/// An allocation the allocator could not give (allocator.rs): recorded
/// with its stack before the standard library aborts. Allocates nothing.
#[cold]
pub fn out_of_memory(size: usize) {
    native::out_of_memory(size);
}

/// The long-lived thread whose panic means the session cannot go on
/// (`threads.rs`): the app ends the session when it sees one.
pub fn thread_died() -> Option<&'static str> {
    threads::died()
}

pub fn finished(success: bool) {
    if let Some(session) = SESSION.get() {
        let marker = if success { "normal" } else { "failed" };
        let _ = std::fs::write(session.dir.join(marker), []);
    }
}

pub fn failure(message: &str) {
    if let Some(session) = SESSION.get() {
        let _ = std::fs::write(session.dir.join("failure.txt"), message);
        let _ = std::fs::write(session.dir.join("failed"), []);
    }
}

/// The GPU has kept a frame for `seconds` while the window still answers:
/// `gpu-stall.txt` (with the render thread's stack on Windows), so a
/// session ended now is reported as such. [`gpu_answered`] takes it back.
pub fn gpu_stalled(seconds: u64) {
    if let Some(session) = SESSION.get() {
        watchdog::save_gpu_stall(&session.dir, seconds);
    }
}

/// The GPU device is lost for good (app/gpu_stall.rs): the session, which
/// ends now, is reported as such, with the safe graphics offered next.
pub fn gpu_lost() {
    if let Some(session) = SESSION.get() {
        let _ = std::fs::write(session.dir.join("gpu-lost.txt"), []);
        let _ = std::fs::write(session.dir.join("failed"), []);
        ending_in_gpu_trouble(&session.dir);
    }
}

/// A known overlay hook arrived in the process (overlay_hooks.rs): its file
/// name goes to `overlay-hooks.txt`, so the report names it however long
/// ago it came (the log's tail may no longer hold its line).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn overlay_hook(file: &str) {
    if let Some(session) = SESSION.get()
        && let Ok(mut out) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(session.dir.join("overlay-hooks.txt"))
    {
        let _ = writeln!(out, "{file}");
    }
}

/// The user closed the viewer from the notice while the GPU did not answer:
/// the session is reported even though it ends through the usual way out.
pub fn gpu_closed() {
    if let Some(session) = SESSION.get() {
        let _ = std::fs::write(session.dir.join("failed"), []);
        ending_in_gpu_trouble(&session.dir);
    }
}

pub fn gpu_answered() {
    if let Some(session) = SESSION.get() {
        let _ = std::fs::remove_file(session.dir.join("gpu-stall.txt"));
    }
}

/// Left in the crash root when a session ended with the GPU in trouble
/// (stalled, lost, or the window frozen): the next launch offers the safe
/// graphics once ([`take_gpu_trouble`]).
const GPU_TROUBLE: &str = "gpu-trouble";

fn gpu_trouble(dir: &Path) -> bool {
    ["gpu-stall.txt", "gpu-lost.txt", "freeze.txt"]
        .iter()
        .any(|name| dir.join(name).exists())
        || summary::device_lost(dir)
}

fn note_gpu_trouble(dir: &Path) {
    if gpu_trouble(dir)
        && let Some(root) = dir.parent()
    {
        let _ = std::fs::write(root.join(GPU_TROUBLE), []);
    }
}

/// The session ends now with the GPU in trouble. On Windows its watcher
/// marks it once the viewer is gone ([`preserve`], which also leaves out a
/// session Windows itself ended); elsewhere no watcher outlives the viewer
/// (crash/native.rs), so the session marks itself.
fn ending_in_gpu_trouble(dir: &Path) {
    #[cfg(not(windows))]
    note_gpu_trouble(dir);
    #[cfg(windows)]
    let _ = dir;
}

/// The last session that ended badly had the GPU in trouble: true once,
/// then the mark is gone. Read after [`start`], which marks a session
/// whose watcher died with it.
pub fn take_gpu_trouble() -> bool {
    std::fs::remove_file(root().join(GPU_TROUBLE)).is_ok()
}

/// A viewer stopped by Windows ending the session (shutdown, restart, sign
/// out) did not crash: nothing to report.
fn should_report(dir: &Path, exit_code: u32) -> bool {
    !dir.join("system-ending").exists() && (exit_code != 0 || dir.join("failed").exists() || !dir.join("normal").exists())
}

fn preserve(dir: &Path, exit_code: u32) -> anyhow::Result<()> {
    std::fs::write(dir.join("exit-code.txt"), format!("0x{exit_code:08X}"))?;
    std::fs::write(dir.join("failed"), [])?;
    std::fs::write(
        dir.join("stopped-unix.txt"),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string(),
    )?;
    note_gpu_trouble(dir);
    Ok(())
}

fn recover() {
    recover_in(&root(), |dir| {
        let _ = spawn_helper(dir, "--crash-show");
    });
}

fn recover_in(root: &Path, mut show: impl FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let dir = entry.path();
        if dir.join("acknowledged").exists() || dir.join("recovery-attempted").exists() {
            continue;
        }
        let Ok(record) = load(&dir) else { continue };
        if native::is_alive(record.pid, record.process_started) {
            continue;
        }
        if !should_report(&dir, 0) {
            // Closed normally, but its watcher could not start and remove
            // it: nothing to report, and nobody else will ever clean it.
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        // At most one recovery attempt per session, even if the dialog itself
        // cannot start. Files remain available locally; no restart loop.
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("recovery-attempted"))
            .is_ok()
            // A session the system cannot tell from a live one (written on
            // Linux before its start time was recorded) is kept, not shown:
            // a window for each of those older sessions at once would report
            // what may only have been a viewer killed or still running.
            && native::identifies(record.process_started)
        {
            // Its watcher died with it: nobody marked it at its end.
            note_gpu_trouble(&dir);
            show(&dir);
        }
    }
}

fn show_once(dir: &Path) -> anyhow::Result<()> {
    // Held as an OS file lock, so a failed helper cannot leave a stale lock.
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("dialog.lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    if dir.join("acknowledged").exists() {
        return Ok(());
    }
    let record = load(dir)?;
    // What a crash report would publish, kept beside the private files.
    let summary = summary::build(&record, dir);
    if let Err(error) = std::fs::write(dir.join("public-report.md"), format!("# {}\n\n{}", summary.title, summary.body)) {
        let _ = std::fs::write(dir.join("summary-error.txt"), error.to_string());
    }
    aurora_i18n::select(&record.language, None);
    if let Err(error) = window::show(dir, record, summary) {
        let _ = std::fs::write(dir.join("dialog-error.txt"), format!("{error:#}"));
        return Err(error);
    }
    std::fs::write(dir.join("acknowledged"), [])?;
    Ok(())
}

fn trace(dir: &Path) -> String {
    let mut result = String::new();
    for name in ["panic.txt", "native.txt", "oom.txt", "failure.txt", "gpu-stall.txt", "freeze.txt"] {
        if let Ok(text) = read_limited(&dir.join(name), MAX_TRACE)
            && !text.trim().is_empty()
        {
            result.push_str(&text);
            result.push('\n');
        }
    }
    result
}

/// Deliberate failures are only available in the offline demo.
pub fn demo() -> bool {
    if std::env::var_os("AURORA_DEMO").is_none() {
        return false;
    }
    match std::env::var("AURORA_DEMO_CRASH").as_deref() {
        Ok("panic") => panic!("Offline crash recovery demonstration"),
        Ok("native") => native::demo_exception(),
        // An allocation no machine can give: the allocator's failure path,
        // then the standard library's abort (fast fail).
        Ok("oom") => {
            let block = vec![0u8; 1 << 46];
            std::hint::black_box(&block);
            false
        }
        Ok("overflow") => {
            demo_overflow(0);
            false
        }
        Ok("missing" | "recovery") => std::process::exit(9),
        Ok("normal") => true,
        _ => false,
    }
}

/// Endless recursion with a frame the optimiser cannot remove.
#[allow(unconditional_recursion)]
fn demo_overflow(depth: u64) -> u64 {
    let frame = std::hint::black_box([depth; 64]);
    demo_overflow(depth + 1) + frame[0]
}

/// `AURORA_DEMO_CRASH=thread`, once the window shows: a background thread
/// under a critical name panics, as the network runtime's session loop
/// could; the app ends the session at its next frame.
pub fn demo_thread_death() {
    if std::env::var_os("AURORA_DEMO").is_none() || std::env::var("AURORA_DEMO_CRASH").as_deref() != Ok("thread") {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("aurora-net".into())
        .spawn(|| panic!("Offline thread death demonstration"));
}

/// `AURORA_DEMO_CRASH=freeze`, once the window shows: the main thread stops
/// answering, the watchdog saves its stacks, then the process is ended
/// from outside like Task Manager's End task (exit code 1).
pub fn demo_freeze() {
    if std::env::var_os("AURORA_DEMO").is_none() || std::env::var("AURORA_DEMO_CRASH").as_deref() != Ok("freeze") {
        return;
    }
    std::thread::sleep(std::time::Duration::from_secs(watchdog::HUNG_AFTER + 8));
    #[cfg(windows)]
    // SAFETY: ends this process, as another program would.
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        TerminateProcess(GetCurrentProcess(), 1);
    }
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_snapshot_survives_language_theme_changes_and_rejects_unknown_formats() {
        let dir = std::env::temp_dir().join(format!("aurora-crash-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let mut record = Record {
            schema: 1,
            pid: 17,
            process_started: 123,
            started_unix: 456,
            version: "0.2.0+7.gabcdef".into(),
            commit: "abcdef".into(),
            profile: "debug".into(),
            language: "fr".into(),
            font: "Inter".into(),
            font_scale: 1.25,
            theme: Default::default(),
            log_path: dir.join("viewer.log"),
            demo: true,
            os: String::new(),
            crash_reports: Default::default(),
        };
        save(&dir, &record).unwrap();
        record.language = "en".into();
        record.theme.panel = "#fafafa".into();
        save(&dir, &record).unwrap();
        let restored = load(&dir).unwrap();
        assert_eq!(restored.language, "en");
        assert_eq!(restored.theme.panel, "#fafafa");
        assert_eq!(restored.version, "0.2.0+7.gabcdef");
        assert_eq!(restored.font_scale, 1.25);
        assert!(!dir.join("session.pending").exists());
        record.schema = 2;
        save(&dir, &record).unwrap();
        assert!(load(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn scratch_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("aurora-crash-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        root
    }

    /// Off Windows no watcher outlives the viewer: a session that ends with
    /// the GPU stalled or lost marks the next start itself, once.
    #[cfg(not(windows))]
    #[test]
    fn a_session_ending_in_gpu_trouble_marks_the_next_start_itself() {
        let root = scratch_root();
        for trouble in ["stall", "lost"] {
            let dir = root.join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir(&dir).unwrap();
            if trouble == "stall" {
                watchdog::save_gpu_stall(&dir, 6);
                let text = std::fs::read_to_string(dir.join("gpu-stall.txt")).unwrap();
                assert_eq!(text, "GPU stall: no frame for 6 s\n");
            } else {
                std::fs::write(dir.join("gpu-lost.txt"), []).unwrap();
            }
            assert!(!root.join(GPU_TROUBLE).exists());
            ending_in_gpu_trouble(&dir);
            assert!(root.join(GPU_TROUBLE).exists(), "{trouble}");
            std::fs::remove_file(root.join(GPU_TROUBLE)).unwrap();
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// What the viewer hands to its helper process (session argument, demo
    /// root, working directory) must lead that process back to the folder
    /// the viewer created.
    #[test]
    fn the_helper_finds_the_session_its_viewer_created() {
        let root = scratch_root();
        let dir = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&dir).unwrap();
        let exported = |command: &std::process::Command| {
            command
                .get_envs()
                .find(|(name, _)| *name == "AURORA_DEMO_CRASH_DIR")
                .and_then(|(_, value)| value.map(std::ffi::OsStr::to_owned))
        };
        for mode in ["--crash-watch", "--crash-show"] {
            let command = helper_command(&dir, mode, true).unwrap();
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args[0], mode);
            let helper_root = root_for(true, exported(&command));
            assert_eq!(command.get_current_dir(), Some(helper_root.as_path()));
            assert_eq!(session_dir(&helper_root, args[1]).unwrap(), dir.canonicalize().unwrap());
        }
        // A relative AURORA_DEMO_CRASH_DIR is resolved by the viewer: the
        // helper does not start in the same working directory.
        let relative = Path::new("crash-reports").join(uuid::Uuid::new_v4().to_string());
        let command = helper_command(&relative, "--crash-watch", true).unwrap();
        assert_eq!(
            root_for(true, exported(&command)),
            std::env::current_dir().unwrap().join("crash-reports")
        );
        // Outside the demo both processes derive the root from the logs.
        let command = helper_command(&dir, "--crash-watch", false).unwrap();
        assert_eq!(exported(&command), None);
        assert!(root_for(false, Some("ignored".into())).ends_with("crashes/sessions"));
        assert!(root_for(true, None).ends_with("crashes/demo"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_helper_only_opens_session_folders_of_its_root() {
        let root = scratch_root();
        let elsewhere = scratch_root();
        let session = uuid::Uuid::new_v4().to_string();
        let outside = uuid::Uuid::new_v4().to_string();
        let file = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir(root.join(&session)).unwrap();
        std::fs::create_dir(elsewhere.join(&outside)).unwrap();
        std::fs::write(root.join(&file), []).unwrap();
        let open = |id: &str| session_dir(&root, std::ffi::OsStr::new(id));
        assert_eq!(open(&session).unwrap(), root.join(&session).canonicalize().unwrap());
        let escape = format!("../{}/{outside}", elsewhere.file_name().unwrap().to_string_lossy());
        let absolute = elsewhere.join(&outside).to_string_lossy().into_owned();
        let missing = uuid::Uuid::new_v4().to_string();
        for refused in ["", ".", "..", "session", &escape, &absolute, &outside, &missing, &file] {
            assert!(open(refused).is_err(), "{refused}");
        }
        #[cfg(windows)]
        {
            // A junction needs no privilege, unlike a symbolic link.
            let link = uuid::Uuid::new_v4().to_string();
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(root.join(&link))
                .arg(elsewhere.join(&outside))
                .output()
                .unwrap();
            assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
            assert!(root.join(&link).is_dir());
            assert!(open(&link).is_err());
            std::fs::remove_dir(root.join(&link)).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(elsewhere).unwrap();
    }

    #[cfg(any(windows, target_os = "linux"))]
    fn test_session(root: &Path, pid: u32, process_started: u64, markers: &[&str]) -> PathBuf {
        let dir = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&dir).unwrap();
        let record = Record {
            schema: 1,
            pid,
            process_started,
            started_unix: 456,
            version: "0.4.0".into(),
            commit: "abcdef".into(),
            profile: "debug".into(),
            language: "en".into(),
            font: "Inter".into(),
            font_scale: 1.0,
            theme: Default::default(),
            log_path: dir.join("viewer.log"),
            demo: true,
            os: String::new(),
            crash_reports: Default::default(),
        };
        save(&dir, &record).unwrap();
        for marker in markers {
            std::fs::write(dir.join(marker), []).unwrap();
        }
        dir
    }

    /// A session whose watcher never started (on Linux there is none) stays
    /// behind: the next launch removes it after a normal close and reports
    /// it once after a crash.
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn the_next_launch_clears_closed_sessions_and_reports_crashed_ones_once() {
        let root = scratch_root();
        let session = |pid: u32, process_started: u64, markers: &[&str]| test_session(&root, pid, process_started, markers);
        // No process started at instant 1: these viewers have stopped.
        let closed = session(std::process::id(), 1, &["normal"]);
        let failed = session(std::process::id(), 1, &["normal", "failed"]);
        let killed = session(std::process::id(), 1, &[]);
        let seen = session(std::process::id(), 1, &["failed", "acknowledged"]);
        let running = session(std::process::id(), native::current_process_started(), &["normal"]);
        std::fs::create_dir(root.join("not-a-session")).unwrap();
        let mut shown = Vec::new();
        recover_in(&root, |dir| shown.push(dir.to_owned()));
        shown.sort();
        let mut crashed = vec![failed.clone(), killed.clone()];
        crashed.sort();
        assert_eq!(shown, crashed);
        assert!(!closed.exists());
        for kept in [&failed, &killed, &seen, &running, &root.join("not-a-session")] {
            assert!(kept.exists(), "{}", kept.display());
        }
        shown.clear();
        recover_in(&root, |dir| shown.push(dir.to_owned()));
        assert!(shown.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Sessions written on Linux before the start time was recorded: the
    /// ones closed normally are removed, the others kept without a window,
    /// and one whose PID still runs is left alone.
    #[cfg(target_os = "linux")]
    #[test]
    fn sessions_without_a_start_time_are_cleared_but_never_reported() {
        let root = scratch_root();
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        let closed = test_session(&root, dead, 0, &["normal"]);
        let killed = test_session(&root, dead, 0, &[]);
        let running = test_session(&root, std::process::id(), 0, &[]);
        let mut shown = Vec::new();
        recover_in(&root, |dir| shown.push(dir.to_owned()));
        assert!(shown.is_empty(), "{shown:?}");
        assert!(!closed.exists());
        assert!(killed.join("recovery-attempted").exists());
        assert!(!running.join("recovery-attempted").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn normal_exit_and_crashes_are_distinct() {
        let dir = std::env::temp_dir().join(format!("aurora-crash-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        assert!(should_report(&dir, 0));
        std::fs::write(dir.join("normal"), []).unwrap();
        assert!(!should_report(&dir, 0));
        assert!(should_report(&dir, 0xC0000005));
        std::fs::write(dir.join("failed"), []).unwrap();
        assert!(should_report(&dir, 0));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn absent_and_oversized_traces_are_not_read_as_valid_reports() {
        let dir = std::env::temp_dir().join(format!("aurora-crash-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        assert!(trace(&dir).is_empty());
        let file = std::fs::File::create(dir.join("panic.txt")).unwrap();
        file.set_len(MAX_TRACE + 1).unwrap();
        assert!(trace(&dir).is_empty());
        drop(file);
        std::fs::write(dir.join("native.txt"), "Exception 0xC0000005\n").unwrap();
        assert!(trace(&dir).contains("0xC0000005"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
