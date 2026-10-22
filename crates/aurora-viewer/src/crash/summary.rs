//! Public text of a crash report, built from the stopped session's files.
//! It is meant for a public GitHub issue, so only values of a known shape
//! are copied: module + offset frames, module identities, a source location
//! and the closed log vocabulary of `privacy.rs`. Raw addresses, panic
//! messages, paths and the log itself stay in the local session folder.
//! Headings and values stay in English for the repository's triage workflow.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Recognised by the repository's crash triage.
pub const MARKER: &str = "<!-- aurora-crash v1 -->";
const MAX_FRAMES: usize = 40;
const LOG_HEAD: u64 = 64 * 1024;
const LOG_TAIL: u64 = 256 * 1024;

pub struct Summary {
    pub title: String,
    pub body: String,
}

pub fn build(record: &super::Record, dir: &Path) -> Summary {
    let text = |name: &str| super::read_limited(&dir.join(name), super::MAX_TRACE).unwrap_or_default();
    let (native_text, panic) = (text("native.txt"), text("panic.txt"));
    let native = (!native_text.trim().is_empty()).then(|| Stack::parse(&native_text));
    let location = panic.lines().next().and_then(panic_location);
    let (reason, title_reason) = if !panic.trim().is_empty() {
        ("Rust panic".to_owned(), "Rust panic".to_owned())
    } else if let Some(code) = native_text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Native exception 0x"))
        .and_then(hex32)
    {
        (format!("Native exception 0x{code:08X}"), format!("native exception 0x{code:08X}"))
    } else if dir.join("failure.txt").exists() {
        // Its text is an error chain with paths: the kind alone is public.
        (
            "Startup or event loop failure".to_owned(),
            "startup or event loop failure".to_owned(),
        )
    } else {
        ("Unexpected shutdown".to_owned(), "unexpected shutdown".to_owned())
    };
    let at = match (&location, native.as_ref().and_then(|stack| stack.fault.clone())) {
        (Some(location), _) => format!(" at {location}"),
        (None, Some(fault)) => format!(" at {fault}"),
        _ => String::new(),
    };
    let version = token(&record.version, 40);
    let title = format!("Crash: {title_reason}{at} ({version})");

    let mut rows = vec![("Version", version), ("Build", token(&record.profile, 16))];
    if !record.commit.is_empty() {
        rows.insert(1, ("Commit", token(&record.commit, 40)));
    }
    rows.push(("Crash", reason));
    if let Some(code) = super::read_limited(&dir.join("exit-code.txt"), 32)
        .ok()
        .and_then(|code| code.trim().strip_prefix("0x").and_then(hex32))
    {
        rows.push(("Exit code", format!("0x{code:08X}")));
    }
    if let Some(duration) = super::read_limited(&dir.join("stopped-unix.txt"), 32)
        .ok()
        .and_then(|stopped| stopped.trim().parse::<u64>().ok())
        .and_then(|stopped| stopped.checked_sub(record.started_unix))
    {
        rows.push(("Session duration", format!("{duration} s")));
    }
    if !record.os.is_empty() {
        rows.push(("System", label(&record.os, 120)));
    }
    let (head, tail) = log(&dir.join("session.log"));
    if let Some(gpu) = head
        .lines()
        .find_map(|line| line.split_once("] GPU: "))
        .map(|(_, gpu)| label(gpu, 160))
    {
        rows.push(("GPU", gpu));
    }
    if record.demo {
        rows.push(("Demo session", "yes".into()));
    }

    let mut body = format!(
        "{MARKER}\nSent from the crash window of Aurora Viewer. Raw addresses, messages, paths and the log stay on the tester's PC.\n\n| Field | Value |\n|---|---|\n"
    );
    for (field, value) in rows {
        body.push_str(&format!("| {field} | {value} |\n"));
    }
    if let Some(location) = &location {
        body.push_str(&format!("\n### Rust panic\n\nLocation: `{location}`\n"));
    }
    // The panic's own stack when there is one, else the exception's.
    let panic = (!panic.trim().is_empty()).then(|| Stack::parse(&panic));
    if let Some(stack) = panic.or(native).filter(|stack| !stack.frames.is_empty()) {
        body.push_str("\n### Stack\n\n```text\n");
        for (index, frame) in stack.trimmed().iter().enumerate() {
            body.push_str(&format!("{index:02} {frame}\n"));
        }
        body.push_str("```\n");
        if !stack.modules.is_empty() {
            body.push_str("\n### Modules\n\n```text\n");
            for module in &stack.modules {
                body.push_str(module);
                body.push('\n');
            }
            body.push_str("```\n");
        }
    }
    let excerpt = super::privacy::excerpt(&tail);
    if !excerpt.is_empty() {
        body.push_str(&format!("\n### Recent warnings and errors\n\n```text\n{excerpt}\n```\n"));
    }
    Summary { title, body }
}

/// Frames written by `native::write_modules`, re-validated: the session
/// folder is local data, and the result is published.
struct Stack {
    fault: Option<String>,
    frames: Vec<String>,
    modules: Vec<String>,
}

impl Stack {
    fn parse(text: &str) -> Stack {
        let mut stack = Stack {
            fault: None,
            frames: Vec::new(),
            modules: Vec::new(),
        };
        #[derive(PartialEq)]
        enum Section {
            Other,
            Frames,
            Modules,
        }
        let mut section = Section::Other;
        for line in text.lines() {
            match line {
                "Native stack (modules):" => section = Section::Frames,
                "Modules:" => section = Section::Modules,
                _ if line.starts_with("Fault: ") => stack.fault = frame(&line["Fault: ".len()..]),
                _ if section == Section::Frames => match line.split_once(": ") {
                    Some((index, "?")) if index.bytes().all(|b| b.is_ascii_digit()) => stack.frames.push("?".into()),
                    Some((index, location)) if index.bytes().all(|b| b.is_ascii_digit()) => match frame(location) {
                        Some(frame) => stack.frames.push(frame),
                        None => section = Section::Other,
                    },
                    _ => section = Section::Other,
                },
                _ if section == Section::Modules => match module(line) {
                    Some(module) => stack.modules.push(module),
                    None => section = Section::Other,
                },
                _ => {}
            }
        }
        stack
    }

    /// The frames above the fault are the exception filter's own.
    fn trimmed(&self) -> &[String] {
        let start = self
            .fault
            .as_ref()
            .and_then(|fault| self.frames.iter().position(|frame| frame == fault))
            .unwrap_or(0);
        let frames = &self.frames[start..];
        &frames[..frames.len().min(MAX_FRAMES)]
    }
}

fn name(value: &str) -> Option<&str> {
    (!value.is_empty() && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')))
        .then_some(value)
}

fn hex(value: &str, max: usize) -> Option<&str> {
    (!value.is_empty() && value.len() <= max && value.bytes().all(|b| b.is_ascii_hexdigit())).then_some(value)
}

fn hex32(value: &str) -> Option<u32> {
    u32::from_str_radix(hex(value, 8)?, 16).ok()
}

fn frame(value: &str) -> Option<String> {
    let (module, offset) = value.split_once("+0x")?;
    Some(format!("{}+0x{}", name(module)?, hex(offset, 16)?))
}

fn module(line: &str) -> Option<String> {
    let mut parts = line.split(' ');
    let mut result = name(parts.next()?)?.to_owned();
    for part in parts {
        let (key, value) = part.split_once('=')?;
        if !matches!(key, "id" | "code") {
            return None;
        }
        result.push_str(&format!(" {key}={}", hex(value, 48)?));
    }
    Some(result)
}

/// `panicked at <file>:<line>:<column>:` reduced to a path inside the
/// sources: a local build's absolute paths name the developer's folders.
fn panic_location(line: &str) -> Option<String> {
    let location = line.strip_prefix("Rust panic: panicked at ")?.strip_suffix(':')?;
    let mut parts = location.rsplitn(3, ':');
    let (column, row, file) = (parts.next()?, parts.next()?, parts.next()?);
    if !column.bytes().chain(row.bytes()).all(|b| b.is_ascii_digit()) || column.is_empty() || row.is_empty() {
        return None;
    }
    let file = file.replace('\\', "/");
    let file = if let Some((_, rest)) = file.split_once("/registry/src/") {
        // Crate sources: drop the registry index folder.
        rest.split_once('/')?.1
    } else if let Some(index) = file.find("/library/").filter(|_| file.contains("/rustc/")) {
        &file[index + 1..]
    } else if let Some(index) = file.find("crates/") {
        &file[index..]
    } else if file.starts_with('/') || file.as_bytes().get(1) == Some(&b':') {
        file.rsplit('/').next()?
    } else {
        &file
    };
    file.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
        .then(|| format!("{file}:{row}:{column}"))
}

/// Fixed identifiers of our own build (version, commit, profile).
fn token(value: &str, max: usize) -> String {
    let value: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-' | '_'))
        .take(max)
        .collect();
    if value.is_empty() { "unknown".into() } else { value }
}

/// System and driver labels: no markup, no table separator.
fn label(value: &str, max: usize) -> String {
    let value: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_' | '(' | ')' | ',' | '+' | '/'))
        .take(max)
        .collect();
    value.trim().to_owned()
}

/// The start of the session log (the GPU line) and its end (the last
/// warnings); a long session's log is never read whole.
fn log(path: &Path) -> (String, String) {
    let read = |from_end: bool, limit: u64| -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let start = if from_end { len.saturating_sub(limit) } else { 0 };
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(limit).read_to_end(&mut bytes)?;
        // A tail that begins halfway through a line drops that fragment.
        if start > 0
            && let Some(end) = bytes.iter().position(|&b| b == b'\n')
        {
            bytes.drain(..=end);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    (read(false, LOG_HEAD).unwrap_or_default(), read(true, LOG_TAIL).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (std::path::PathBuf, super::super::Record) {
        let dir = std::env::temp_dir().join(format!("aurora-crash-summary-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let record = super::super::Record {
            schema: 1,
            pid: 1,
            process_started: 2,
            started_unix: 1000,
            version: "0.7.1+3.gabcdef".into(),
            commit: "abcdef0123".into(),
            profile: "release".into(),
            language: "fr".into(),
            font: "Inter".into(),
            font_scale: 1.0,
            theme: Default::default(),
            log_path: dir.join("PRIVATE_LOG_PATH.log"),
            demo: false,
            os: "Windows 11 Pro (26200, x86_64)".into(),
            crash_reports: Default::default(),
        };
        (dir, record)
    }

    const NATIVE: &str = "Native exception 0xC0000005\nFault address: 0x00007FF6A1B2C3D4\nNative stack (addresses):\n00: 0x00007ffb12345678\n01: 0x00007FF6A1B2C3D4\nFault: aurora_viewer+0x1b2c3d4\nNative stack (modules):\n00: KERNELBASE+0x5678\n01: aurora_viewer+0x1b2c3d4\n02: ?\n03: nvoglv64+0x9abc\nModules:\nKERNELBASE id=0123456789ABCDEF0123456789ABCDEF1 code=6712ABCD3f0000\naurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000\nnvoglv64 code=65000000200000\n";

    #[test]
    fn a_native_crash_publishes_module_offsets_from_the_fault_and_no_raw_address() {
        let (dir, record) = session();
        std::fs::write(dir.join("native.txt"), NATIVE).unwrap();
        std::fs::write(dir.join("exit-code.txt"), "0xC0000005").unwrap();
        std::fs::write(dir.join("stopped-unix.txt"), "1042").unwrap();
        std::fs::write(
            dir.join("session.log"),
            "[t INFO  aurora_render::main_thread] GPU: NVIDIA GeForce RTX 4070 (Vulkan, driver NVIDIA 581.57)\n[t WARN  aurora_net] timeout https://sim.example/cap/SECRET Alice Resident\n",
        )
        .unwrap();
        let summary = build(&record, &dir);
        assert_eq!(
            summary.title,
            "Crash: native exception 0xC0000005 at aurora_viewer+0x1b2c3d4 (0.7.1+3.gabcdef)"
        );
        let body = &summary.body;
        assert!(body.starts_with(MARKER));
        for expected in [
            "| Commit | abcdef0123 |",
            "| Exit code | 0xC0000005 |",
            "| Session duration | 42 s |",
            "| System | Windows 11 Pro (26200, x86_64) |",
            "| GPU | NVIDIA GeForce RTX 4070 (Vulkan, driver NVIDIA 581.57) |",
            "00 aurora_viewer+0x1b2c3d4\n01 ?\n02 nvoglv64+0x9abc\n",
            "aurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000",
            "nvoglv64 code=65000000200000",
            "WARN | network | timeout [free-form details removed]",
        ] {
            assert!(body.contains(expected), "missing {expected:?} in\n{body}");
        }
        // The exception filter's own frames, above the fault, are dropped.
        assert!(!body.contains("KERNELBASE+0x5678"), "{body}");
        for private in ["0x00007", "PRIVATE_LOG_PATH", "SECRET", "Alice", "Demo session"] {
            assert!(!body.contains(private), "leaked {private}:\n{body}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_panic_publishes_its_location_without_message_or_local_paths() {
        let (dir, mut record) = session();
        record.demo = true;
        std::fs::write(
            dir.join("panic.txt"),
            "Rust panic: panicked at C:\\Users\\Private Name\\Aurora\\crates\\aurora-viewer\\src\\app.rs:120:9:\nresident Alice Resident said PRIVATE_MESSAGE\n   0: <unknown>\nNative stack (modules):\n00: aurora_viewer+0x1000\n01: aurora_viewer+0x2000\nModules:\naurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000\n",
        )
        .unwrap();
        let summary = build(&record, &dir);
        assert_eq!(
            summary.title,
            "Crash: Rust panic at crates/aurora-viewer/src/app.rs:120:9 (0.7.1+3.gabcdef)"
        );
        assert!(summary.body.contains("Location: `crates/aurora-viewer/src/app.rs:120:9`"));
        assert!(summary.body.contains("00 aurora_viewer+0x1000\n01 aurora_viewer+0x2000\n"));
        assert!(summary.body.contains("| Demo session | yes |"));
        for private in ["Users", "Private", "Alice", "PRIVATE_MESSAGE"] {
            assert!(!summary.body.contains(private), "leaked {private}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn panic_locations_keep_only_paths_inside_the_sources() {
        for (line, expected) in [
            (
                "Rust panic: panicked at crates/aurora-net/src/udp.rs:5:1:",
                Some("crates/aurora-net/src/udp.rs:5:1"),
            ),
            (
                "Rust panic: panicked at C:\\Users\\runneradmin\\.cargo\\registry\\src\\index.crates.io-1949cf8c6b5b557f\\wgpu-core-27.0.3\\src\\device\\mod.rs:40:12:",
                Some("wgpu-core-27.0.3/src/device/mod.rs:40:12"),
            ),
            (
                "Rust panic: panicked at /rustc/0123abcd/library/core/src/option.rs:2:3:",
                Some("library/core/src/option.rs:2:3"),
            ),
            ("Rust panic: panicked at D:\\private folder\\main.rs:1:2:", Some("main.rs:1:2")),
            ("Rust panic: panicked at src/a`b|c.rs:1:2:", None),
            ("Rust panic: panicked at src/main.rs:x:2:", None),
            ("Rust panic: something else", None),
        ] {
            assert_eq!(panic_location(line).as_deref(), expected, "{line}");
        }
    }

    #[test]
    fn malformed_stack_lines_end_their_section_instead_of_being_published() {
        let stack = Stack::parse(
            "Native stack (modules):\n00: good+0x10\n01: bad name+0x20\n02: later+0x30\nModules:\ngood id=ABC code=12\nbad|module code=1\nlater code=2\nFault: x+0xZZ\n",
        );
        assert_eq!(stack.frames, ["good+0x10"]);
        assert_eq!(stack.modules, ["good id=ABC code=12"]);
        assert_eq!(stack.fault, None);
    }

    #[test]
    fn an_empty_session_still_gives_a_report() {
        let (dir, mut record) = session();
        record.version = "| <b>".into();
        record.os.clear();
        let summary = build(&record, &dir);
        assert_eq!(summary.title, "Crash: unexpected shutdown (b)");
        assert!(!summary.body.contains("### Stack"));
        assert!(!summary.body.contains("| System |"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
