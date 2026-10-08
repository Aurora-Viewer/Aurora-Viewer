//! Logging to the console and to `aurora.log` in the logs directory (the
//! previous run is kept as `aurora.previous.log`), so a session can be
//! shared after a test even when the viewer was started without a console.
//! Capability URLs are secrets for the session: their id is masked.

use std::io::Write;
use std::path::PathBuf;

pub fn logs_dir() -> PathBuf {
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
        None => base.to_string(),
    }
}

/// Show the logs directory in the system file manager.
pub fn open_logs_dir() {
    let dir = logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(&dir).spawn() {
        log::warn!("cannot open {}: {e}", dir.display());
    }
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

/// Console + file writer.
struct Tee {
    file: Option<std::fs::File>,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let text = redact(&text);
        let _ = std::io::stderr().write_all(text.as_bytes());
        if let Some(f) = &mut self.file {
            let _ = f.write_all(text.as_bytes());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = &mut self.file {
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,reqwest=warn"))
        .format_timestamp_millis()
        .target(env_logger::Target::Pipe(Box::new(Tee { file })))
        .init();
    std::panic::set_hook(Box::new(|info| {
        let bt = std::backtrace::Backtrace::force_capture();
        log::error!("panic: {info}\n{bt}");
    }));
    log::info!(
        "Aurora Viewer {} starting ({} {}), log: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::redact;

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
