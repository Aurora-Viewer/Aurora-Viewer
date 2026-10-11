//! Crash recovery, independent of the viewer's event loop and GPU.
//! Firestorm reference: llappviewer.cpp (errorCallback, marker files) and
//! llcrashlogger.cpp (recovery on the next launch). Deliberate difference:
//! no automatic submission; diagnostics stay in the local session folder.

mod native;
mod report;
mod software;
mod window;

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
    if std::env::var_os("AURORA_DEMO").is_some()
        && let Some(path) = std::env::var_os("AURORA_DEMO_CRASH_DIR")
    {
        return PathBuf::from(path);
    }
    crate::logging::logs_dir()
        .join("crashes")
        .join(if std::env::var_os("AURORA_DEMO").is_some() {
            "demo"
        } else {
            "sessions"
        })
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
    let dir = PathBuf::from(args.get(index + 1).ok_or_else(|| anyhow::anyhow!("missing crash session"))?);
    // Never accept arbitrary files or paths from a command line / report.
    let canonical = dir.canonicalize()?;
    anyhow::ensure!(
        canonical.parent() == Some(root().canonicalize()?.as_path()),
        "invalid crash session directory"
    );
    anyhow::ensure!(
        uuid::Uuid::parse_str(&dir.file_name().unwrap_or_default().to_string_lossy()).is_ok(),
        "invalid session id"
    );
    let record = load(&dir)?;
    if args[index] == "--crash-watch" {
        native::watch(&dir, &record)?;
    } else {
        show_once(&dir)?;
    }
    Ok(true)
}

fn spawn_helper(dir: &Path, mode: &str) -> std::io::Result<std::process::Child> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    let session_root = root().canonicalize()?;
    command.arg(mode).arg(dir.canonicalize()?);
    // An inherited installation working directory would block the updater's
    // directory swap on Windows, even after the viewer itself has exited.
    command.current_dir(&session_root);
    if std::env::var_os("AURORA_DEMO").is_some() {
        command.env("AURORA_DEMO_CRASH_DIR", &session_root);
    }
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
    };
    save(&dir, &record)?;
    let panic_file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("panic.txt"))?;
    native::install(&dir)?;
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
        if let Err(error) = save(&session.dir, &record) {
            log::warn!("cannot update crash session: {error}");
        }
    }
}

pub fn panic(info: &std::panic::PanicHookInfo<'_>, trace: &std::backtrace::Backtrace) {
    if let Some(session) = SESSION.get() {
        let mut file = &session.panic_file;
        let _ = writeln!(file, "Rust panic: {info}\n{trace}");
        let _ = file.sync_all();
    }
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

fn should_report(dir: &Path, exit_code: u32) -> bool {
    exit_code != 0 || dir.join("failed").exists() || !dir.join("normal").exists()
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
    Ok(())
}

fn recover() {
    let Ok(entries) = std::fs::read_dir(root()) else { return };
    for entry in entries.flatten() {
        let dir = entry.path();
        if (dir.join("normal").exists() && !dir.join("failed").exists())
            || dir.join("acknowledged").exists()
            || dir.join("recovery-attempted").exists()
        {
            continue;
        }
        let Ok(record) = load(&dir) else { continue };
        if native::is_alive(record.pid, record.process_started) {
            continue;
        }
        // At most one recovery attempt per session, even if the dialog itself
        // cannot start. Files remain available locally; no restart loop.
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("recovery-attempted"))
            .is_ok()
        {
            let _ = spawn_helper(&dir, "--crash-show");
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
    aurora_i18n::select(&record.language, None);
    if let Err(error) = window::show(dir, record) {
        let _ = std::fs::write(dir.join("dialog-error.txt"), format!("{error:#}"));
        return Err(error);
    }
    std::fs::write(dir.join("acknowledged"), [])?;
    Ok(())
}

fn trace(dir: &Path) -> String {
    let mut result = String::new();
    for name in ["panic.txt", "native.txt", "failure.txt"] {
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
        Ok("missing" | "recovery") => std::process::exit(9),
        Ok("normal") => true,
        _ => false,
    }
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
