//! Logging to the console and to `aurora.log` in the logs directory (the
//! previous run is kept as `aurora.previous.log`), so a session can be
//! shared after a test even when the viewer was started without a console.
//! Capability URLs are secrets for the session: their id is masked.

use std::io::Write;
use std::path::{Path, PathBuf};

pub fn logs_dir() -> PathBuf {
    if let Some(test) = aurora_updater::test_environment() {
        return test.root.join("logs");
    }
    directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

pub fn log_path() -> PathBuf {
    logs_dir().join(format!("{}.log", log_stem()))
}

/// The offline demo logs apart, so it never rotates a live session's log;
/// `--title` adds the window name (one log per test window) and
/// AURORA_LOG_NAME picks any other name.
fn log_stem() -> String {
    if let Some(name) = std::env::var_os("AURORA_LOG_NAME").filter(|n| !n.is_empty()) {
        return name.to_string_lossy().into_owned();
    }
    let base = if std::env::var_os("AURORA_DEMO").is_some() {
        "aurora-demo"
    } else {
        "aurora"
    };
    match crate::cli::title_slug() {
        Some(slug) => format!("{base}-{slug}"),
        None => format!("{base}-{}", std::process::id()),
    }
}

/// Show the logs directory in the system file manager.
pub fn open_logs_dir() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Err(e) = open_directory(&dir) {
        log::warn!("cannot open {}: {e}", dir.display());
    }
}

/// Show a directory in the system file manager (also the transcripts folder).
pub fn open_directory(path: &Path) -> std::io::Result<()> {
    crate::open_path::open(path)
}

/// Mask the id of capability URLs (".../cap/<uuid>").
fn redact(line: &str) -> std::borrow::Cow<'_, str> {
    if !line.contains("/cap/") {
        return line.into();
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("/cap/") {
        out.push_str(&rest[..i + 5]);
        rest = &rest[i + 5..];
        let end = rest.find(|c: char| !(c.is_ascii_hexdigit() || c == '-')).unwrap_or(rest.len());
        if end > 0 {
            out.push('…');
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into()
}

thread_local! {
    /// What this thread logs while a capture runs (`capture`).
    static CAPTURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` and return, besides its result, the lines it logged on this
/// thread without their timestamp and target (the diagnostic copied to the
/// clipboard, scene/describe.rs). env_logger writes in the calling thread.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, String) {
    CAPTURE.with(|c| *c.borrow_mut() = Some(String::new()));
    let result = f();
    let text = CAPTURE.with(|c| c.borrow_mut().take()).unwrap_or_default();
    (result, strip_headers(&text))
}

/// Log lines without env_logger's header
/// ("[2026-10-20T12:00:00.000Z INFO  aurora_viewer::scene] text").
fn strip_headers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = match line.strip_prefix('[').and_then(|l| l.split_once("] ")) {
            Some((_, rest)) => rest,
            None => line,
        };
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A line the Dev Toolbox keeps in memory: J3 and L3 show the last warnings,
/// J5 the recent output of a diagnostic, without reading the log file.
#[derive(Debug, Clone)]
pub struct Recent {
    pub at: std::time::Instant,
    /// "HH:MM:SS" of the record's header (UTC, as in the log).
    pub clock: String,
    pub level: log::Level,
    pub text: String,
}

const WARNINGS_KEPT: usize = 200;
const TOOL_LINES_KEPT: usize = 600;

/// The diagnostics whose output J5 copies, by the start of their lines.
pub const TOOL_PREFIXES: &[(&str, &str)] = &[
    ("P3", "slow sync:"),
    ("P4", "still check:"),
    ("P5", "sync check:"),
    ("P6", "perf summary:"),
    ("P6", "net summary:"),
    ("O9", "sculpt watch:"),
    ("M1", "media objects:"),
];

static WARNINGS: std::sync::Mutex<std::collections::VecDeque<Recent>> = std::sync::Mutex::new(std::collections::VecDeque::new());
static TOOL_LINES: std::sync::Mutex<std::collections::VecDeque<(&'static str, Recent)>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// The header of a record: (clock, level, the message after the header).
fn parse_record(text: &str) -> Option<(&str, log::Level, &str)> {
    let rest = text.strip_prefix('[')?;
    let (header, message) = rest.split_once("] ")?;
    let mut parts = header.split_whitespace();
    let stamp = parts.next()?;
    let level = parts.next()?.parse().ok()?;
    let clock = stamp.split_once('T').map_or(stamp, |(_, t)| t.get(..8).unwrap_or(t));
    Some((clock, level, message.trim_end()))
}

/// Keep what the Dev Toolbox shows from one record.
fn remember(text: &str) {
    let Some((clock, level, message)) = parse_record(text) else {
        return;
    };
    let recent = || Recent {
        at: std::time::Instant::now(),
        clock: clock.to_owned(),
        level,
        text: message.to_owned(),
    };
    if level <= log::Level::Warn
        && let Ok(mut ring) = WARNINGS.lock()
    {
        if ring.len() == WARNINGS_KEPT {
            ring.pop_front();
        }
        ring.push_back(recent());
    }
    if let Some((tool, _)) = TOOL_PREFIXES.iter().find(|(_, prefix)| message.starts_with(prefix))
        && let Ok(mut ring) = TOOL_LINES.lock()
    {
        if ring.len() == TOOL_LINES_KEPT {
            ring.pop_front();
        }
        ring.push_back((tool, recent()));
    }
}

/// The last `n` warnings and errors, the oldest first.
pub fn recent_warnings(n: usize) -> Vec<Recent> {
    WARNINGS
        .lock()
        .map(|ring| ring.iter().skip(ring.len().saturating_sub(n)).cloned().collect())
        .unwrap_or_default()
}

/// The lines the diagnostics of [`TOOL_PREFIXES`] wrote in the last `since`.
pub fn tool_lines(since: std::time::Duration) -> Vec<(&'static str, Recent)> {
    let now = std::time::Instant::now();
    TOOL_LINES
        .lock()
        .map(|ring| ring.iter().filter(|(_, r)| now.duration_since(r.at) <= since).cloned().collect())
        .unwrap_or_default()
}

/// Console + file writer.
struct Tee {
    file: Option<std::fs::File>,
    session_file: Option<std::fs::File>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let text = redact(&text);
        CAPTURE.with(|c| {
            if let Some(s) = c.borrow_mut().as_mut() {
                s.push_str(&text);
            }
        });
        remember(&text);
        let _ = std::io::stderr().write_all(text.as_bytes());
        if let Some(f) = &mut self.file {
            let _ = f.write_all(text.as_bytes());
            crate::disk_io::written(crate::disk_io::Origin::Logs, text.len());
        }
        if let Some(f) = &mut self.session_file {
            let _ = f.write_all(text.as_bytes());
            crate::disk_io::written(crate::disk_io::Origin::Logs, text.len());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = &mut self.file {
            let _ = f.flush();
        }
        if let Some(f) = &mut self.session_file {
            let _ = f.flush();
        }
        std::io::stderr().flush()
    }
}

pub fn init() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = log_path();
    let _ = std::fs::rename(&path, dir.join(format!("{}.previous.log", log_stem())));
    let file = std::fs::File::create(&path).ok();
    // `pulseaudio` off: the PulseAudio client logs `ERROR Reactor error:
    // Client disconnected` each time cpal closes a stream (every output or
    // capture device switch). Real stream failures reach aurora-audio through
    // cpal's error callback, which reopens the device.
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,reqwest=warn,symphonia=warn,pulseaudio=off,zbus=warn"),
    )
    .format_timestamp_millis()
    .target(env_logger::Target::Pipe(Box::new(Tee {
        file,
        session_file: crate::crash::session_log(),
    })))
    .init();
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        crate::crash::panic(info, &bt);
        log::error!("panic: {info}\n{bt}");
    }));
    log::info!(
        "Aurora Viewer {} starting ({} {}, {} allocator), log: {}",
        aurora_version::version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::allocator::name(),
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::{parse_record, redact, strip_headers};

    #[test]
    fn record_headers_give_clock_and_level() {
        let (clock, level, message) =
            parse_record("[2026-10-20T02:34:46.538Z WARN  aurora_viewer::scene::textures] texture memory: pressure\n").unwrap();
        assert_eq!(clock, "02:34:46");
        assert_eq!(level, log::Level::Warn);
        assert_eq!(message, "texture memory: pressure");
        assert!(parse_record("  continuation line").is_none());
    }

    #[test]
    fn captured_lines_lose_their_header() {
        let text = "[2026-10-20T12:00:00.000Z INFO  aurora_viewer::scene::describe] attachment '?' point 40\n  prim [0 tex] x\n";
        assert_eq!(strip_headers(text), "attachment '?' point 40\n  prim [0 tex] x\n");
    }

    #[test]
    fn masks_capability_ids() {
        assert_eq!(
            redact(
                "error sending request for url (https://sim1.agni.lindenlab.com:12043/cap/0b6c6a8e-1f0d-4d3e-9a1b-2c3d4e5f6a7b): timeout"
            ),
            "error sending request for url (https://sim1.agni.lindenlab.com:12043/cap/…): timeout"
        );
        assert_eq!(redact("plain line"), "plain line");
    }
}
