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
    /// The graphics card and its driver, as the report gives them.
    pub gpu: Option<String>,
}

pub fn build(record: &super::Record, dir: &Path) -> Summary {
    build_at(record, dir, super::native::boot_unix())
}

/// A session the watcher did not see end (it stopped with the viewer)
/// followed this soon by a start of Windows went down with the system.
const RESTART_WINDOW: u64 = 180;

fn build_at(record: &super::Record, dir: &Path, boot_unix: Option<u64>) -> Summary {
    let text = |name: &str| super::read_limited(&dir.join(name), super::MAX_TRACE).unwrap_or_default();
    let (native_text, panic, freeze_text) = (text("native.txt"), text("panic.txt"), text("freeze.txt"));
    let native = (!native_text.trim().is_empty()).then(|| Stack::parse(&native_text));
    let freeze = threads(&freeze_text);
    let stall = threads(&text("gpu-stall.txt"));
    let location = panic.lines().next().and_then(panic_location);
    let alive = super::watchdog::read_alive(dir);
    let value = |key: &str| alive.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let number = |key: &str| value(key).and_then(|v| v.parse::<u64>().ok());
    let stopped = super::read_limited(&dir.join("stopped-unix.txt"), 32)
        .ok()
        .and_then(|stopped| stopped.trim().parse::<u64>().ok());
    let exit_code = super::read_limited(&dir.join("exit-code.txt"), 32)
        .ok()
        .and_then(|code| code.trim().strip_prefix("0x").and_then(hex32));
    // A failed allocation counts only when the process then failed fast (or
    // nobody saw how it ended): one that was handled leaves a note too.
    let oom = out_of_memory(&text("oom.txt")).filter(|_| matches!(exit_code, None | Some(FAST_FAIL)));
    let (reason, title_reason) = if !panic.trim().is_empty() {
        ("Rust panic".to_owned(), "Rust panic".to_owned())
    } else if let Some((size, _)) = &oom {
        (format!("Out of memory ({size} bytes)"), "out of memory".to_owned())
    } else if let Some(code) = native_text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("Native exception 0x"))
        .and_then(hex32)
    {
        (format!("Native exception 0x{code:08X}"), format!("native exception 0x{code:08X}"))
    } else if let Some((number, name)) = native_text.lines().next().and_then(native_signal) {
        (format!("Native signal {name} ({number})"), format!("native signal {name}"))
    } else if dir.join("failure.txt").exists() {
        // Its text is an error chain with paths: the kind alone is public.
        (
            "Startup or event loop failure".to_owned(),
            "startup or event loop failure".to_owned(),
        )
    } else if text("gpu-lost.txt").trim() == super::GPU_LOST_AT_START {
        // Before the first frame (app.rs): never a driver reset of a session
        // under load, whatever the rest of the report looks like.
        ("GPU device lost at startup".to_owned(), "GPU device lost at startup".to_owned())
    } else if dir.join("gpu-lost.txt").exists() {
        // The viewer closed itself after the device loss (app/gpu_stall.rs).
        ("GPU device lost".to_owned(), "GPU device lost".to_owned())
    } else if !stall.is_empty() {
        // The GPU still had a frame when the session ended (the window
        // answered, the user closed it from the notice or by other means).
        ("GPU not responding".to_owned(), "GPU not responding".to_owned())
    } else if !freeze.is_empty() {
        // The window was still not answering when the session ended.
        ("Freeze".to_owned(), "freeze".to_owned())
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
    if let Some(thread) = super::read_limited(&dir.join("thread.txt"), 64)
        .ok()
        .and_then(|name| super::threads::published_name(&name))
    {
        rows.push(("Thread", format!("{thread} (the session was ended)")));
    }
    if let Some(code) = exit_code {
        let known = exit_code_name(code).map(|name| format!(" ({name})")).unwrap_or_default();
        rows.push(("Exit code", format!("0x{code:08X}{known}")));
    }
    // Without the watcher's word, the last sign of life stands for the end.
    let tick = number("tick");
    let end = stopped.or(tick);
    if let Some(duration) = end.and_then(|end| end.checked_sub(record.started_unix)) {
        let approximate = if stopped.is_none() { " (last sign of life)" } else { "" };
        rows.push(("Session duration", format!("{duration} s{approximate}")));
    }
    if stopped.is_none() {
        rows.push(("End of session", unseen_end(tick, boot_unix).to_owned()));
    }
    // `freeze.txt` is the watchdog's word that the window was frozen at the
    // end; it answers for 5 s more than its last `responsive`, itself up to
    // 5 s older than the end of the session.
    if let (Some(end), Some(_)) = (end, tick) {
        let window = match number("responsive") {
            Some(at) if !freeze.is_empty() => format!(
                "not answering for the last {} s",
                end.saturating_sub(at) + super::watchdog::HUNG_AFTER
            ),
            Some(_) => "answering until the end".to_owned(),
            None => "never shown".to_owned(),
        };
        rows.push(("Window", window));
    }
    if !record.os.is_empty() {
        rows.push(("System", label(&record.os, 120)));
    }
    let (head, tail) = log(&dir.join("session.log"));
    let gpu = gpu_in(&head).map(|gpu| label(gpu, 160));
    if let Some(gpu) = &gpu {
        rows.push(("GPU", gpu.clone()));
    }
    if let Some(notice) = gpu_notice_in(&head) {
        rows.push(("GPU notice", label(notice, 160)));
    }
    if let Some(descriptors) = logged(&head, "] GPU descriptors: ") {
        rows.push(("GPU descriptors", label(descriptors, 160)));
    }
    // aurora-render's device lost callback (main_thread.rs).
    if device_lost_in(&tail) {
        rows.push(("GPU device lost", "yes (driver reset or GPU removed)".into()));
        // the work the GPU had in hand: a hang or a fault (aurora-render
        // device_lost.rs), rebuilt from the renderer's own newest line
        if let Some(work) = gpu_work_in(&tail) {
            rows.push(("GPU work at the loss", work));
        }
    }
    if let Some(errors) = gpu_errors_in(&head, &tail) {
        rows.push(("GPU errors (first of each, in order)", errors));
    }
    if let Some(seconds) = text("gpu-stall.txt").lines().next().and_then(|line| {
        line.strip_prefix("GPU stall: no frame for ")?
            .strip_suffix(" s")?
            .parse::<u64>()
            .ok()
    }) {
        rows.push((
            "GPU not responding",
            format!("no frame for {seconds} s, the window still answering"),
        ));
    }
    if let Some(recoveries) = gpu_recoveries(&text("gpu-recoveries.txt"), record.started_unix) {
        rows.push(("GPU recoveries", recoveries));
    }
    if let Some(hooks) = overlay_hooks(&text("overlay-hooks.txt")) {
        rows.push(("Overlay hooks", hooks));
    }
    if let Some(layers) = vulkan_layers(&text("vulkan-layers.txt")) {
        rows.push(("Vulkan layers on offer", layers));
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
    // The panic's own stack when there is one, else the exception's, else
    // the render thread's held in the GPU, else the frozen threads' (the
    // main thread's first: the one the triage reads).
    let panic = (!panic.trim().is_empty()).then(|| Stack::parse(&panic));
    let oom = oom.map(|(_, stack)| stack);
    let stacks: Vec<(Option<&str>, Stack)> = match panic.or(oom).or(native).filter(|stack| !stack.frames.is_empty()) {
        Some(stack) => vec![(None, stack)],
        None => if stall.is_empty() { freeze } else { stall }
            .into_iter()
            .enumerate()
            .map(|(i, (name, stack))| ((i > 0).then_some(name), stack))
            .collect(),
    };
    let mut modules: Vec<&String> = Vec::new();
    for (name, stack) in &stacks {
        match name {
            Some(name) => body.push_str(&format!("\n### Stack ({name} thread)\n\n```text\n")),
            None => body.push_str("\n### Stack\n\n```text\n"),
        }
        for (index, frame) in stack.trimmed().iter().enumerate() {
            body.push_str(&format!("{index:02} {frame}\n"));
        }
        body.push_str("```\n");
        for module in &stack.modules {
            if !modules.contains(&module) {
                modules.push(module);
            }
        }
    }
    if !modules.is_empty() {
        body.push_str("\n### Modules\n\n```text\n");
        for module in modules {
            body.push_str(module);
            body.push('\n');
        }
        body.push_str("```\n");
    }
    let state: Vec<_> = STATE_ROWS
        .iter()
        .filter_map(|&(key, label)| value(key).and_then(state_value).map(|v| (label, v)))
        .collect();
    if !state.is_empty() {
        body.push_str("\n### Last state\n\nNoted by the viewer every few seconds.\n\n| | |\n|---|---|\n");
        for (label, value) in state {
            body.push_str(&format!("| {label} | {value} |\n"));
        }
    }
    if let Some(flight) = super::flight_recorder::read_report(dir) {
        body.push_str("\n### Flight recorder\n\n```text\n");
        body.push_str(&flight);
        body.push_str("```\n");
    }
    let excerpt = super::privacy::excerpt(&tail);
    if !excerpt.is_empty() {
        body.push_str(&format!(
            "\n### Recent warnings and errors\n\nTime before the end of the log, level, Rust module and kind; the messages stay on the tester's PC.\n\n```text\n{excerpt}\n```\n"
        ));
    }
    Summary { title, body, gpu }
}

/// The GPU device losses the session recovered from (`gpu-recoveries.txt`:
/// `<unix> <ms> same|safe` a line), as numbers and fixed words: how many,
/// and for the first ones when in the session, how long the recovery took
/// and whether it fell back on the safe graphics. None without any.
fn gpu_recoveries(text: &str, started_unix: u64) -> Option<String> {
    let recoveries: Vec<(u64, u64, bool)> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split(' ');
            let unix: u64 = parts.next()?.parse().ok()?;
            let ms: u64 = parts.next()?.parse().ok()?;
            let safe = match parts.next()? {
                "safe" => true,
                "same" => false,
                _ => return None,
            };
            Some((unix.saturating_sub(started_unix), ms, safe))
        })
        .collect();
    if recoveries.is_empty() {
        return None;
    }
    let shown: Vec<String> = recoveries
        .iter()
        .take(5)
        .map(|(at, ms, safe)| format!("at {at} s ({ms} ms{})", if *safe { ", safe graphics" } else { "" }))
        .collect();
    let more = if recoveries.len() > shown.len() { ", …" } else { "" };
    Some(format!("{}: {}{more}", recoveries.len(), shown.join(", ")))
}

/// The keys of `alive.txt` published, with their labels.
const STATE_ROWS: &[(&str, &str)] = &[
    ("screen", "Screen"),
    ("fps", "Frame rate"),
    ("frame_ms", "Frame time (ms)"),
    ("window", "Window size"),
    ("preset", "Graphics preset"),
    ("ram_mb", "Memory (MB)"),
    ("vram_mb", "Video memory used / budget (MB)"),
    ("texture_pressure", "Texture memory pressure"),
    ("objects", "Objects"),
    ("avatars", "Avatars"),
    ("regions", "Regions"),
    ("swapchain_10s", "Swapchain recreations in 10 s (up to a GPU loss)"),
    ("window_state", "Window state"),
    ("effects", "Effects on (graphics switches)"),
    ("gpu_ms", "Last GPU frame time (ms)"),
    ("gpu_peak_ms", "Longest GPU frame in 10 s (ms, up to a GPU loss)"),
    ("gpu_peak_pass", "Heaviest pass of that frame"),
    ("gpu_peak_ao_contact", "Ambient occlusion / contact shadows of that frame (ms)"),
    ("gpu_last_pass", "Heaviest pass of the last frame"),
    ("uploads", "Texture uploads since the previous note (count / MB)"),
    ("upload_peak_mb", "Largest upload of one frame (MB)"),
    ("session_s", "Seconds since the login began"),
    ("world_s", "Seconds in the world (absent while loading)"),
    ("region_s", "Seconds since the region changed"),
    ("loss_reason", "GPU loss: reason given by wgpu"),
    ("loss_cause", "GPU loss: what the message is about"),
    ("loss_seen_by", "GPU loss: first seen by"),
    ("stall_ms", "GPU held the render thread past 2 s, longest (ms)"),
    ("other_viewers", "Other Aurora viewers on this card (count / in front)"),
    ("card_used_mb", "Card memory in use by all programs (MB)"),
    ("card_others_mb", "Card memory in use by other programs (MB)"),
    ("loading", "Still loading (textures in flight / awaiting upload / geometry)"),
];

/// The layers of `vulkan-layers.txt` (crash/mod.rs), all of a layer name's
/// shape or none: the file is local data, and the result is published.
fn vulkan_layers(text: &str) -> Option<String> {
    let text = text.trim();
    let layer =
        |token: &str| token.len() <= 48 && token.starts_with("VK_LAYER_") && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    (text.len() <= 900 && text.split(',').all(layer)).then(|| text.to_owned())
}

/// The GPU errors wgpu handed to the viewer (`wgpu error: …`, logged by
/// aurora-render main_thread.rs), first of each in the order they came:
/// `out-of-memory` or the wgpu operation that failed, never the rest of the
/// message, whose labels are free text. A session stuck on these errors
/// (#2311, x155) otherwise says nothing of what failed. A line of any other
/// module or level is left out, as in [`gpu_work_in`].
fn gpu_errors_in(head: &str, tail: &str) -> Option<String> {
    const SHOWN: usize = 6;
    let mut found: Vec<&str> = Vec::new();
    let mut note = |what| {
        if !found.contains(&what) {
            found.push(what);
        }
    };
    for text in [head, tail] {
        // An error whose operation is still to come: the message goes on
        // over the lines below its record.
        let mut open = false;
        for line in text.lines() {
            if line.starts_with('[') {
                if std::mem::take(&mut open) {
                    note("other");
                }
                let Some((record, message)) = line.split_once("] wgpu error: ") else {
                    continue;
                };
                if !record.ends_with(" ERROR aurora_render::main_thread") {
                    continue;
                }
                if message.starts_with("Out of Memory") {
                    note("out-of-memory");
                } else {
                    open = true;
                }
            } else if open && let Some(rest) = line.trim_start().strip_prefix("In ") {
                open = false;
                note(gpu_operation(rest).unwrap_or("other"));
            }
        }
        if open {
            note("other");
        }
    }
    if found.is_empty() {
        return None;
    }
    let more = if found.len() > SHOWN { ", …" } else { "" };
    found.truncate(SHOWN);
    Some(format!("{}{more}", found.join(", ")))
}

/// `Queue::write_buffer` from wgpu's `Queue::write_buffer, label = '…'`
/// (wgpu-core error.rs, `ContextError`): an object wgpu names and a method's
/// name, nothing else.
fn gpu_operation(text: &str) -> Option<&str> {
    const OBJECTS: [&str; 13] = [
        "Blas",
        "Buffer",
        "CommandEncoder",
        "ComputePass",
        "ComputePipeline",
        "Device",
        "Instance",
        "Queue",
        "RenderBundleEncoder",
        "RenderPass",
        "RenderPipeline",
        "Surface",
        "Texture",
    ];
    let operation = text.split(',').next()?.trim_end();
    let (object, method) = operation.split_once("::")?;
    (operation.len() <= 64
        && OBJECTS.contains(&object)
        && !method.is_empty()
        && method.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
    .then_some(operation)
}

/// A state value as the watchdog writes it: a number, numbers joined by
/// `/` or `x`, a list of the effect words, or one of the fixed names;
/// anything else is not published.
pub(crate) fn state_value(value: &str) -> Option<String> {
    use super::watchdog::{EFFECT_WORDS, WINDOW_STATES};
    let names = [
        "yes",
        "no",
        "unknown",
        "other",
        "none",
        "device-lost",
        "out-of-memory",
        "timeout",
        "callback",
        "wait",
        "start",
        "demo",
    ];
    let screens = ["login", "loading", "world", "potato", "low", "medium", "high", "deluxe", "custom"];
    // the passes of `GpuElement::key`
    let passes = ["shadows_depth", "effects", "terrain_sky", "objects", "water", "transparent", "post"];
    let numeric = |v: &str| !v.is_empty() && v.len() <= 12 && v.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    let effect = |token: &str| {
        let word = token.trim_end_matches(|c: char| c.is_ascii_digit());
        EFFECT_WORDS.contains(&word) && token.len() <= 24
    };
    let ok = names.contains(&value)
        || screens.contains(&value)
        || passes.contains(&value)
        || WINDOW_STATES.contains(&value)
        || numeric(value)
        || (value.contains(['/', 'x']) && value.split(['/', 'x']).count() <= 3 && value.split(['/', 'x']).all(numeric))
        || (value.len() <= 400 && value.split(',').all(effect));
    ok.then(|| value.to_owned())
}

/// What a missing watcher's word means.
fn unseen_end(tick: Option<u64>, boot_unix: Option<u64>) -> &'static str {
    match (tick, boot_unix) {
        (Some(tick), Some(boot)) if boot > tick && boot - tick <= RESTART_WINDOW => {
            "not seen: Windows started again right after (system crash, power loss or forced restart)"
        }
        _ => "not seen: the viewer and its watcher stopped together (process tree ended, or Windows stopped)",
    }
}

/// The exit code of an abort: the standard library's, after a failed
/// allocation among others.
const FAST_FAIL: u32 = 0xC000_0409;

/// The last failed allocation of `oom.txt` (crash/native.rs): its size and
/// its stack, re-validated like the others.
fn out_of_memory(text: &str) -> Option<(u64, Stack)> {
    let last = text.rsplit("Out of memory: ").next().filter(|_| text.contains("Out of memory: "))?;
    let (first, rest) = last.split_once('\n')?;
    let size = first.strip_suffix(" bytes")?.parse().ok()?;
    Some((size, Stack::parse(rest)))
}

/// `Native signal 11 (SIGSEGV)`, the first line of a Linux `native.txt`
/// (crash/linux.rs): the number and a name of the fixed list only.
fn native_signal(line: &str) -> Option<(u8, &'static str)> {
    let (number, name) = line.strip_prefix("Native signal ")?.split_once(' ')?;
    let name = ["SIGSEGV", "SIGBUS", "SIGILL", "SIGFPE", "SIGABRT"]
        .into_iter()
        .find(|known| name.strip_prefix('(').and_then(|n| n.strip_suffix(')')) == Some(*known))?;
    Some((number.parse().ok()?, name))
}

/// The usual meaning of an exit code, for the reader of the report.
fn exit_code_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0x0000_0001 => "ended from outside, as by Task Manager, or exit(1)",
        0x4001_0004 => "ended by a debugger or the end of the Windows session",
        0xC000_0005 => "access violation",
        0xC000_001D => "illegal instruction",
        0xC000_00FD => "stack overflow",
        0xC000_0374 => "heap corruption",
        0xC000_0409 => "fast fail: abort, out of memory or a security check",
        0xC000_013A => "ended by Ctrl+C",
        _ => return None,
    })
}

/// The stacks of `freeze.txt`, one per `Thread: <name>` block.
fn threads(text: &str) -> Vec<(&'static str, Stack)> {
    let mut out = Vec::new();
    for block in text.split("Thread: ").skip(1) {
        let (name, rest) = block.split_once('\n').unwrap_or((block, ""));
        // Only the names the watchdog asks for are published.
        let name = match name.trim() {
            "main" => "main",
            "aurora-render" => "aurora-render",
            _ => continue,
        };
        let stack = Stack::parse(rest);
        if !stack.frames.is_empty() {
            out.push((name, stack));
        }
    }
    out
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
        // A GNU build ID can be 32 bytes (crash/linux.rs).
        result.push_str(&format!(" {key}={}", hex(value, 64)?));
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

/// The overlay hooks that came into the process (`overlay-hooks.txt`, one
/// file name per line), each checked again against the fixed list of
/// overlay_hooks.rs: only its names are published.
fn overlay_hooks(text: &str) -> Option<String> {
    let mut hooks: Vec<&crate::overlay_hooks::Hook> = Vec::new();
    for hook in text.lines().filter_map(|line| crate::overlay_hooks::known(line.trim())) {
        if !hooks.contains(&hook) {
            hooks.push(hook);
        }
    }
    (!hooks.is_empty()).then(|| {
        hooks
            .iter()
            .map(|hook| format!("{}, {}", hook.program, hook.file))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// The start of the session log (the GPU line) and its end (the last
/// warnings); a long session's log is never read whole.
/// What follows `marker` on the first line of the log that has it.
fn logged<'a>(log: &'a str, marker: &str) -> Option<&'a str> {
    log.lines().find_map(|line| line.split_once(marker)).map(|(_, value)| value)
}

/// The card of the session and its driver: the line that names the card
/// opened. The warnings about the cards share its start, and come first, so
/// they are left out: "GPU: no discrete GPU visible…" stood for the card in
/// every report from an integrated GPU. A crash while the device opens comes
/// before that line: then the card that would have been chosen, in the order
/// of adapter_choice.rs, from the list of adapters.
fn gpu_in(log: &str) -> Option<&str> {
    let opened = log
        .lines()
        .filter(|line| !line.contains(" WARN "))
        .find_map(|line| line.split_once("] GPU: "));
    if let Some((_, gpu)) = opened {
        return Some(gpu);
    }
    let adapters: Vec<&str> = log
        .lines()
        .filter_map(|line| line.split_once("] GPU adapter "))
        .filter_map(|(_, adapter)| adapter.split_once(": "))
        .map(|(_, adapter)| adapter)
        .collect();
    let can_present = |adapter: &&&str| adapter.contains(", can present");
    adapters
        .iter()
        .filter(can_present)
        .find(|adapter| adapter.contains("(discrete, "))
        .or_else(|| adapters.iter().find(can_present))
        .or(adapters.first())
        .copied()
}

/// The warning about the cards, when there is one (adapter_choice.rs).
fn gpu_notice_in(log: &str) -> Option<&str> {
    log.lines()
        .filter(|line| line.contains(" WARN "))
        .find_map(|line| line.split_once("] GPU: "))
        .map(|(_, notice)| notice)
}

fn device_lost_in(tail: &str) -> bool {
    tail.lines().any(|line| line.contains("] GPU device lost ("))
}

/// The "GPU work at the loss" row: the newest line the device-lost
/// callback logged (`[… ERROR aurora_render::main_thread] GPU work at the
/// loss: …`; the last one when a recovered session was lost again),
/// rebuilt by [`gpu_work_row`]. A line of any other module or level is
/// left out: a name logged elsewhere could carry the marker.
fn gpu_work_in(tail: &str) -> Option<String> {
    tail.lines().rev().find_map(|line| {
        let (head, text) = line.split_once("] GPU work at the loss: ")?;
        if !head.ends_with(" ERROR aurora_render::main_thread") {
            return None;
        }
        gpu_work_row(text)
    })
}

/// The row rebuilt from the line's frame numbers, ages (`d.ddd`) and one
/// of the fixed endings of aurora-render's `hang_or_fault`; a line that
/// does not parse whole gives nothing, so that no text of the log is
/// published as it is.
fn gpu_work_row(text: &str) -> Option<String> {
    let text = text.trim();
    if text == aurora_render::GPU_WORK_NONE {
        return Some(text.to_owned());
    }
    let rest = text.strip_prefix("last submission frame ")?;
    let (frame, rest) = rest.split_once(", ")?;
    let frame: u64 = frame.parse().ok()?;
    let (age, rest) = rest.split_once(" s before, ")?;
    let age = seconds(age)?;
    let (state, rest) = ["not finished", "finished"]
        .iter()
        .find_map(|state| rest.strip_prefix(&format!("{state}, ")).map(|rest| (*state, rest)))?;
    let (last, verdict) = match rest.strip_prefix("none finished yet, ") {
        Some(verdict) => ("none finished yet".to_owned(), verdict),
        None => {
            let rest = rest.strip_prefix("frame ")?;
            let (finished, rest) = rest.split_once(" finished ")?;
            let finished: u64 = finished.parse().ok()?;
            let (finished_age, verdict) = rest.split_once(" s before, ")?;
            (format!("frame {finished} finished {} s before", seconds(finished_age)?), verdict)
        }
    };
    let verdict = aurora_render::GPU_WORK_VERDICTS.iter().find(|v| **v == verdict)?;
    Some(format!("last submission frame {frame}, {age} s before, {state}, {last}, {verdict}"))
}

/// An age as the line writes it: digits, a point, three digits.
fn seconds(text: &str) -> Option<&str> {
    let (whole, frac) = text.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(whole) && frac.len() == 3 && digits(frac)).then_some(text)
}

/// The session's log says its GPU device was lost.
pub(super) fn device_lost(dir: &Path) -> bool {
    device_lost_in(&log(&dir.join("session.log")).1)
}

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

    const NOTICE: &str = "[t WARN  aurora_render::main_thread] GPU: no discrete GPU visible to Vulkan: on a hybrid PC";

    #[test]
    fn the_notice_about_the_cards_does_not_stand_for_the_card() {
        let log = format!(
            "{NOTICE}\n[t INFO  aurora_render::main_thread] GPU: Intel(R) Iris(R) Xe Graphics (Vulkan, driver Intel Corporation 101.7092)\n"
        );
        assert_eq!(
            gpu_in(&log),
            Some("Intel(R) Iris(R) Xe Graphics (Vulkan, driver Intel Corporation 101.7092)")
        );
        assert_eq!(gpu_notice_in(&log), Some("no discrete GPU visible to Vulkan: on a hybrid PC"));
    }

    #[test]
    fn a_crash_while_the_device_opens_names_the_card_from_the_adapters() {
        let adapter = |i: u32, what: &str| format!("[t INFO  aurora_render::main_thread] GPU adapter {i}: {what}\n");
        // only an integrated GPU, with the notice before the card is opened
        let log = format!(
            "{}{NOTICE}\n",
            adapter(0, "Intel(R) UHD Graphics (integrated, 32678 MB, can present)")
        );
        assert_eq!(gpu_in(&log), Some("Intel(R) UHD Graphics (integrated, 32678 MB, can present)"));
        // the discrete card that can present, as adapter_choice.rs picks it
        let log = format!(
            "{}{}{}",
            adapter(0, "Intel(R) UHD Graphics (integrated, 512 MB, can present)"),
            adapter(1, "NVIDIA One (discrete, 4096 MB, cannot present to the window)"),
            adapter(2, "NVIDIA Two (discrete, 8192 MB, can present)"),
        );
        assert_eq!(gpu_in(&log), Some("NVIDIA Two (discrete, 8192 MB, can present)"));
        // none can present: still a name
        let log = adapter(0, "Intel(R) UHD Graphics (integrated, 512 MB, cannot present to the window)");
        assert_eq!(
            gpu_in(&log),
            Some("Intel(R) UHD Graphics (integrated, 512 MB, cannot present to the window)")
        );
        assert_eq!(gpu_in("[t INFO  viewer] nothing about the card\n"), None);
    }

    #[test]
    fn a_device_lost_at_startup_has_its_own_title_and_says_why() {
        let (dir, record) = session();
        std::fs::write(dir.join("gpu-lost.txt"), super::super::GPU_LOST_AT_START).unwrap();
        std::fs::write(
            dir.join("session.log"),
            format!(
                "{NOTICE}\n[t INFO  aurora_render::main_thread] GPU descriptors: 1048576 in an array, 200 sampled per stage, 1048576 in all pools, 16384 texture pages\n[t INFO  aurora_render::main_thread] GPU: Intel(R) Iris(R) Xe Graphics (Vulkan, driver Intel Corporation 101.7092)\n[t ERROR aurora_render::main_thread] GPU device lost (Unknown): Out of memory\n"
            ),
        )
        .unwrap();
        let summary = build(&record, &dir);
        assert_eq!(summary.title, "Crash: GPU device lost at startup (0.7.1+3.gabcdef)");
        assert_eq!(
            summary.gpu.as_deref(),
            Some("Intel(R) Iris(R) Xe Graphics (Vulkan, driver Intel Corporation 101.7092)")
        );
        for expected in [
            "| Crash | GPU device lost at startup |",
            "| GPU notice | no discrete GPU visible to Vulkan",
            "| GPU descriptors | 1048576 in an array, 200 sampled per stage, 1048576 in all pools, 16384 texture pages |",
        ] {
            assert!(summary.body.contains(expected), "{expected} missing from {}", summary.body);
        }
        // a loss in a running session stays the plain kind
        std::fs::write(dir.join("gpu-lost.txt"), "").unwrap();
        assert_eq!(build(&record, &dir).title, "Crash: GPU device lost (0.7.1+3.gabcdef)");
        // the work the GPU had in hand (aurora-render device_lost.rs): a row
        std::fs::write(
            dir.join("session.log"),
            format!(
                "{NOTICE}\n[t ERROR aurora_render::main_thread] GPU device lost (Unknown): reset\n[t ERROR aurora_render::main_thread] GPU work at the loss: last submission frame 601, 1.101 s before, not finished, frame 599 finished 1.103 s before, a hang (the driver reset the card while that frame ran)\n"
            ),
        )
        .unwrap();
        let body = build(&record, &dir).body;
        assert!(
            body.contains("| GPU work at the loss | last submission frame 601, 1.101 s before, not finished, frame 599 finished 1.103 s before, a hang (the driver reset the card while that frame ran) |"),
            "{body}"
        );
    }

    /// The row comes from the renderer's own line only, rebuilt from its
    /// numbers and fixed words: a name logged by another module that
    /// carries the marker publishes nothing, a line that does not parse
    /// whole neither; of a session recovered then lost again, the last
    /// loss's line is the row.
    #[test]
    fn the_gpu_work_row_is_rebuilt_from_the_renderer_s_newest_line() {
        const HANG: &str = "last submission frame 601, 1.101 s before, not finished, frame 599 finished 1.103 s before, a hang (the driver reset the card while that frame ran)";
        const FAULT: &str = "last submission frame 12, 0.008 s before, not finished, frame 11 finished 0.009 s before, a fault in the work of that frame (not a hang)";
        let own = |text: &str| format!("[t ERROR aurora_render::main_thread] GPU work at the loss: {text}\n");
        // another module's line carrying the marker in a name: nothing
        let injected = format!(
            "[t WARN  aurora_viewer::world::object_im_filtered] object named \"x] GPU work at the loss: {HANG}\" in a private region\n"
        );
        assert_eq!(gpu_work_in(&injected), None);
        assert_eq!(
            gpu_work_in(&format!("[t INFO  aurora_render::main_thread] GPU work at the loss: {HANG}\n")),
            None
        );
        // the renderer's line, rebuilt as it is
        assert_eq!(gpu_work_in(&own(HANG)).as_deref(), Some(HANG));
        assert_eq!(gpu_work_in(&own(FAULT)).as_deref(), Some(FAULT));
        assert_eq!(
            gpu_work_in(&own("last submission frame 0, 3.000 s before, not finished, none finished yet, a hang (the driver reset the card while that frame ran)")).as_deref(),
            Some("last submission frame 0, 3.000 s before, not finished, none finished yet, a hang (the driver reset the card while that frame ran)")
        );
        assert_eq!(
            gpu_work_in(&own("nothing submitted yet (lost while starting)")).as_deref(),
            Some("nothing submitted yet (lost while starting)")
        );
        // anything else in the line: nothing published
        for bad in [
            "last submission frame 601, 1.101 s before, not finished, frame 599 finished 1.103 s before, a hang in a private region",
            "last submission frame 601, 1.101 s before, not finished, frame 599 finished 1.103 s before, a hang (the driver reset the card while that frame ran) at a private region",
            "last submission frame six, 1.101 s before, not finished, none finished yet, a hang (the driver reset the card while that frame ran)",
            "last submission frame 601, 1.1 s before, not finished, none finished yet, a hang (the driver reset the card while that frame ran)",
            "a private region",
        ] {
            assert_eq!(gpu_work_in(&own(bad)), None, "{bad}");
        }
        // a recovered loss, then the fatal one: the last line
        let twice = format!("{}{}", own(FAULT), own(HANG));
        assert_eq!(gpu_work_in(&twice).as_deref(), Some(HANG));
    }

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
            "| Session duration | 42 s |",
            "| System | Windows 11 Pro (26200, x86_64) |",
            "| GPU | NVIDIA GeForce RTX 4070 (Vulkan, driver NVIDIA 581.57) |",
            "00 aurora_viewer+0x1b2c3d4\n01 ?\n02 nvoglv64+0x9abc\n",
            "aurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000",
            "nvoglv64 code=65000000200000",
            "WARN | aurora_net | timeout",
            "| Exit code | 0xC0000005 (access violation) |",
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

    /// A Linux `native.txt` (crash/linux.rs): the handler's frames and the
    /// signal trampoline above the fault, build IDs as module identities.
    const SIGNAL: &str = "Native signal 11 (SIGSEGV)\nSignal code: 1\nFault address: 0x7f0012345000\nNative stack (addresses):\n00: 0x5612aa001000\n01: 0x7f00aa0451d0\n02: 0x5612aa1b2c3d\nFault: aurora-viewer+0x1b2c3d\nNative stack (modules):\n00: aurora-viewer+0x1000\n01: libc.so.6+0x451d0\n02: aurora-viewer+0x1b2c3d\n03: libvulkan_radeon.so+0x9abc\nModules:\naurora-viewer id=0403020106050807090A0B0C0D0E0F100 code=0102030405060708090a0b0c0d0e0f1011121314\nlibc.so.6 id=AABBCCDD11223344556677889900AABB0 code=ddccbbaa443322116677889900aabbccddeeff00\nlibvulkan_radeon.so id=0403020106050807090A0B0C0D0E0F100 code=0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20\n";

    #[test]
    fn a_linux_signal_publishes_its_name_and_module_offsets_from_the_fault() {
        let (dir, record) = session();
        std::fs::write(dir.join("native.txt"), SIGNAL).unwrap();
        let summary = build(&record, &dir);
        assert_eq!(
            summary.title,
            "Crash: native signal SIGSEGV at aurora-viewer+0x1b2c3d (0.7.1+3.gabcdef)"
        );
        let body = &summary.body;
        for expected in [
            "| Crash | Native signal SIGSEGV (11) |",
            "00 aurora-viewer+0x1b2c3d\n01 libvulkan_radeon.so+0x9abc\n",
            "aurora-viewer id=0403020106050807090A0B0C0D0E0F100 code=0102030405060708090a0b0c0d0e0f1011121314",
            // A 32-byte build ID.
            "libvulkan_radeon.so id=0403020106050807090A0B0C0D0E0F100 code=0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20\n",
        ] {
            assert!(body.contains(expected), "missing {expected:?} in\n{body}");
        }
        // The handler's frames above the fault, and every raw address, stay out.
        for private in ["libc.so.6+0x451d0", "0x7f00", "0x5612"] {
            assert!(!body.contains(private), "leaked {private}:\n{body}");
        }
        // Only the fixed signal names are published.
        assert_eq!(native_signal("Native signal 11 (SIGSEGV)"), Some((11, "SIGSEGV")));
        assert_eq!(native_signal("Native signal 6 (SIGABRT)"), Some((6, "SIGABRT")));
        assert_eq!(native_signal("Native signal 11 (Alice)"), None);
        assert_eq!(native_signal("Native signal x (SIGSEGV)"), None);
        assert_eq!(native_signal("Native signal 11 (?)"), None);
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

    const FREEZE: &str = "Freeze: no answer for 20 s\nThread: main\nNative stack (modules):\n00: ntdll+0x9d0e4\n01: aurora_viewer+0x5000\nModules:\nntdll id=0123456789ABCDEF0123456789ABCDEF1 code=1234ABCD1f8000\naurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000\nThread: aurora-render\nNative stack (modules):\n00: nvoglv64+0x9abc\n01: aurora_viewer+0x7000\nModules:\nnvoglv64 code=65000000200000\naurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000\nThread: Private Name\nNative stack (modules):\n00: secret+0x1\n";

    #[test]
    fn recoveries_are_counted_with_when_and_how_and_nothing_else() {
        assert_eq!(gpu_recoveries("", 1000), None);
        assert_eq!(
            gpu_recoveries("1120 812 same\n1300 790 safe\n", 1000).as_deref(),
            Some("2: at 120 s (812 ms), at 300 s (790 ms, safe graphics)")
        );
        // a line that is not the viewer's is left out, never published
        assert_eq!(
            gpu_recoveries("1120 812 same\nC:\\Users\\someone 1 safe\n1130 800 other\n", 1000).as_deref(),
            Some("1: at 120 s (812 ms)")
        );
        let many = "1100 800 same\n".repeat(7);
        assert!(gpu_recoveries(&many, 1000).is_some_and(|s| s.starts_with("7: ") && s.ends_with(", …")));
    }

    #[test]
    fn state_values_outside_the_fixed_words_are_not_published() {
        for private in [
            "Private Place",
            "/home/someone",
            "shadows3,secret",
            "VK_LAYER_OBS_HOOK",
            "",
            "ao-gtao2,",
        ] {
            assert_eq!(state_value(private), None, "{private:?}");
        }
    }

    #[test]
    fn a_session_ended_frozen_publishes_both_threads_and_its_last_state() {
        let (dir, record) = session();
        std::fs::write(dir.join("freeze.txt"), FREEZE).unwrap();
        std::fs::write(dir.join("exit-code.txt"), "0x00000001").unwrap();
        std::fs::write(dir.join("stopped-unix.txt"), "1100").unwrap();
        std::fs::write(dir.join("vulkan-layers.txt"), "VK_LAYER_OBS_HOOK,VK_LAYER_NV_optimus\n").unwrap();
        std::fs::write(
            dir.join("alive.txt"),
            "tick=1098\nresponsive=1075\nram_mb=5120\nscreen=world\nfps=0.4\nwindow=1920x1080\npreset=high\nvram_mb=5800/6000\ntexture_pressure=7/7\nobjects=4200\nswapchain_10s=37\nregion_name=Private Place\neffects=shadows3,contact-shadows,ao-gtao2,aa-smaa,scale100,draw192\nwindow_state=background\ngpu_ms=12.5\ngpu_peak_ms=2400.0\ngpu_peak_pass=effects\nuploads=12/34.5\nsession_s=45\nregion_s=20\nloading=12/3/40\ngpu_peak_ao_contact=20.5/2100.0\nlayers=VK_LAYER_OLD_NOTE\nloss_reason=unknown\nloss_cause=device-lost\nloss_seen_by=callback\nstall_ms=2300\nother_viewers=2/1\ncard_used_mb=6100\ncard_others_mb=3200\nlayers_note=/home/someone\n",
        )
        .unwrap();
        let summary = build_at(&record, &dir, Some(10));
        assert_eq!(summary.title, "Crash: freeze (0.7.1+3.gabcdef)");
        let body = &summary.body;
        for expected in [
            "| Crash | Freeze |",
            "| Exit code | 0x00000001 (ended from outside, as by Task Manager, or exit(1)) |",
            "| Session duration | 100 s |",
            "| Window | not answering for the last 30 s |",
            "### Stack\n\n```text\n00 ntdll+0x9d0e4\n01 aurora_viewer+0x5000\n```",
            "### Stack (aurora-render thread)\n\n```text\n00 nvoglv64+0x9abc\n01 aurora_viewer+0x7000\n```",
            "| Video memory used / budget (MB) | 5800/6000 |",
            "| Texture memory pressure | 7/7 |",
            "| Window size | 1920x1080 |",
            "| Screen | world |",
            "| Swapchain recreations in 10 s (up to a GPU loss) | 37 |",
            "| Effects on (graphics switches) | shadows3,contact-shadows,ao-gtao2,aa-smaa,scale100,draw192 |",
            "| Window state | background |",
            "| Longest GPU frame in 10 s (ms, up to a GPU loss) | 2400.0 |",
            "| Heaviest pass of that frame | effects |",
            "| Seconds since the login began | 45 |",
            "| Still loading (textures in flight / awaiting upload / geometry) | 12/3/40 |",
            "| Vulkan layers on offer | VK_LAYER_OBS_HOOK,VK_LAYER_NV_optimus |",
            "| GPU loss: first seen by | callback |",
            "| GPU held the render thread past 2 s, longest (ms) | 2300 |",
            "| Ambient occlusion / contact shadows of that frame (ms) | 20.5/2100.0 |",
            "| Other Aurora viewers on this card (count / in front) | 2/1 |",
            "| Card memory in use by all programs (MB) | 6100 |",
            "| Card memory in use by other programs (MB) | 3200 |",
        ] {
            assert!(body.contains(expected), "missing {expected:?} in\n{body}");
        }
        // One module list, each module once.
        assert_eq!(body.matches("aurora_viewer id=").count(), 1, "{body}");
        assert!(!body.contains("End of session"), "{body}");
        for private in ["Private", "secret", "region_name", "someone", "layers_note", "OLD_NOTE"] {
            assert!(!body.contains(private), "leaked {private}:\n{body}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_session_its_watcher_did_not_see_end_uses_the_last_sign_of_life() {
        let (dir, record) = session();
        std::fs::write(dir.join("alive.txt"), "tick=1500\nresponsive=1499\n").unwrap();
        let body = build_at(&record, &dir, Some(1600)).body;
        assert!(body.contains("| Session duration | 500 s (last sign of life) |"), "{body}");
        assert!(
            body.contains("| End of session | not seen: Windows started again right after"),
            "{body}"
        );
        assert!(body.contains("| Window | answering until the end |"), "{body}");
        // Windows started long after: nothing says it went down with it.
        let body = build_at(&record, &dir, Some(90_000)).body;
        assert!(
            body.contains("| End of session | not seen: the viewer and its watcher stopped together"),
            "{body}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    const OOM: &str = "Out of memory: 64 bytes\nNative stack (modules):\n00: aurora_viewer+0x10\nModules:\naurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000\nOut of memory: 70368744177664 bytes\nNative stack (modules):\n00: aurora_viewer+0x1234\n01: aurora_viewer+0x5678\nModules:\naurora_viewer id=FEDCBA9876543210FEDCBA98765432101 code=6700000Ac5e0000\n";

    #[test]
    fn a_failed_allocation_names_the_crash_only_when_the_process_failed_fast() {
        let (dir, record) = session();
        std::fs::write(dir.join("oom.txt"), OOM).unwrap();
        std::fs::write(dir.join("exit-code.txt"), "0xC0000409").unwrap();
        let summary = build_at(&record, &dir, None);
        assert_eq!(summary.title, "Crash: out of memory (0.7.1+3.gabcdef)");
        // the last allocation that failed, with its own stack
        assert!(
            summary.body.contains("| Crash | Out of memory (70368744177664 bytes) |"),
            "{}",
            summary.body
        );
        assert!(
            summary.body.contains("00 aurora_viewer+0x1234\n01 aurora_viewer+0x5678\n"),
            "{}",
            summary.body
        );
        // handled (the process went on and ended another way): not the cause
        std::fs::write(dir.join("exit-code.txt"), "0xC0000005").unwrap();
        assert!(!build_at(&record, &dir, None).body.contains("Out of memory ("));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_session_closed_after_the_device_loss_says_so() {
        let (dir, record) = session();
        std::fs::write(dir.join("gpu-lost.txt"), []).unwrap();
        let summary = build_at(&record, &dir, None);
        assert_eq!(summary.title, "Crash: GPU device lost (0.7.1+3.gabcdef)");
        assert!(summary.body.contains("| Crash | GPU device lost |"), "{}", summary.body);
        // a render thread that panicked on the loss keeps its panic
        std::fs::write(
            dir.join("panic.txt"),
            "Rust panic: panicked at wgpu-30.0.1/src/backend/wgpu_core.rs:349:9:\nError in Device::poll\n",
        )
        .unwrap();
        assert!(build_at(&record, &dir, None).body.contains("| Crash | Rust panic |"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_overlay_hooks_of_the_session_are_named_from_the_fixed_list_only() {
        let (dir, record) = session();
        assert!(!build_at(&record, &dir, None).body.contains("| Overlay hooks |"));
        std::fs::write(
            dir.join("overlay-hooks.txt"),
            "DiscordHook64.dll\nC:\\Users\\Private Name\\evil.dll\nRTSSHooks64.dll\nDiscordHook64.dll\n",
        )
        .unwrap();
        let body = build_at(&record, &dir, None).body;
        assert!(
            body.contains(
                "| Overlay hooks | Discord (in-game overlay or Go Live capture), DiscordHook64.dll; \
                 RivaTuner Statistics Server (MSI Afterburner's on-screen display), RTSSHooks64.dll |"
            ),
            "{body}"
        );
        assert!(!body.contains("Private") && !body.contains("evil"), "{body}");
        // a cell of the table: no hook may break it
        assert!(
            crate::overlay_hooks::KNOWN
                .iter()
                .all(|h| !h.program.contains('|') && !h.file.contains('|'))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_critical_thread_death_names_the_thread_with_the_panic() {
        let (dir, record) = session();
        std::fs::write(
            dir.join("panic.txt"),
            "Rust panic: panicked at crates/aurora-net/src/session.rs:10:5:\nboom\nNative stack (modules):\n00: aurora_viewer+0x1000\n",
        )
        .unwrap();
        std::fs::write(dir.join("thread.txt"), "aurora-net").unwrap();
        let body = build_at(&record, &dir, None).body;
        assert!(body.contains("| Crash | Rust panic |"), "{body}");
        assert!(body.contains("| Thread | aurora-net (the session was ended) |"), "{body}");
        std::fs::write(dir.join("thread.txt"), "Private Name").unwrap();
        assert!(!build_at(&record, &dir, None).body.contains("| Thread |"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_crash_before_any_note_of_the_state_still_names_the_vulkan_layers() {
        let (dir, record) = session();
        std::fs::write(dir.join("vulkan-layers.txt"), "VK_LAYER_OBS_HOOK,VK_LAYER_RTSS\n").unwrap();
        let body = build_at(&record, &dir, None).body;
        assert!(
            body.contains("| Vulkan layers on offer | VK_LAYER_OBS_HOOK,VK_LAYER_RTSS |"),
            "{body}"
        );
        assert!(!body.contains("### Last state"), "{body}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_layers_file_that_is_not_all_layer_names_is_not_published() {
        assert_eq!(vulkan_layers(""), None);
        assert_eq!(vulkan_layers("VK_LAYER_OBS_HOOK,C:\\Users\\someone\\layer"), None);
        assert_eq!(vulkan_layers("VK_LAYER_OBS_HOOK,Alice Resident"), None);
        assert_eq!(vulkan_layers(&["VK_LAYER_OBS_HOOK"; 60].join(",")), None);
        assert_eq!(vulkan_layers(" VK_LAYER_NV_present\n").as_deref(), Some("VK_LAYER_NV_present"));
    }

    /// How a wgpu error begins in the log (aurora-render main_thread.rs).
    const WGPU_ERROR: &str = "[2026-11-04T10:00:00.000Z ERROR aurora_render::main_thread] wgpu error: ";

    #[test]
    fn gpu_errors_name_the_kind_and_operation_and_never_a_label() {
        let e = WGPU_ERROR;
        let log = format!(
            "[2026-11-04T09:59:00.000Z INFO  aurora_viewer] start\n\
             {e}Out of Memory\n\
             {e}Validation Error\n\nCaused by:\n  In Queue::write_buffer, label = 'Private Place'\n    In Device::nested\n      Buffer with 'Private Place' label is invalid\n\n\
             {e}Validation Error\n\nCaused by:\n  In Queue::write_buffer, label = 'Private Place'\n    Buffer is invalid\n (repeated 154 times in the last 10 s)\n\
             {e}Validation Error\n\nCaused by:\n  In CommandEncoder::finish\n    Encoder is invalid\n\n\
             [2026-11-04T10:00:01.000Z INFO  aurora_viewer] still here\n"
        );
        let expected = Some("out-of-memory, Queue::write_buffer, CommandEncoder::finish");
        assert_eq!(gpu_errors_in(&log, "").as_deref(), expected);
        // the head and the tail of a short log are the same text
        assert_eq!(gpu_errors_in(&log, &log).as_deref(), expected);
    }

    #[test]
    fn a_gpu_error_of_another_shape_is_other_and_other_modules_are_left_out() {
        let e = WGPU_ERROR;
        for (log, expected) in [
            // no operation before the next record, or before the end
            (
                format!("{e}Validation Error\n\nCaused by:\n  Something else\n[t INFO  a] b\n"),
                Some("other"),
            ),
            (format!("{e}Validation Error\n"), Some("other")),
            // not an operation of wgpu's
            (format!("{e}Validation Error\n\nCaused by:\n  In Alice::resident\n"), Some("other")),
            (format!("{e}Validation Error\n\nCaused by:\n  In a render pass\n"), Some("other")),
            (
                format!("{e}Validation Error\n\nCaused by:\n  In Queue::Write Place\n"),
                Some("other"),
            ),
            // anything logged elsewhere could carry the marker
            (
                "[t INFO  aurora_viewer::chat] wgpu error: Out of Memory\n  In Queue::submit\n".to_owned(),
                None,
            ),
            ("  In Queue::submit\n".to_owned(), None),
        ] {
            assert_eq!(gpu_errors_in(&log, "").as_deref(), expected, "{log:?}");
        }
    }

    #[test]
    fn only_a_few_gpu_errors_are_listed() {
        let log: String = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .iter()
            .map(|m| format!("{WGPU_ERROR}Validation Error\n\nCaused by:\n  In Device::create_{m}\n"))
            .collect();
        let row = gpu_errors_in(&log, "").unwrap();
        assert_eq!(row.matches("Device::").count(), 6, "{row}");
        assert!(row.ends_with("Device::create_f, …"), "{row}");
    }

    #[test]
    fn a_session_with_no_gpu_error_has_no_row_for_them() {
        let (dir, record) = session();
        std::fs::write(dir.join("alive.txt"), "tick=1098\nscreen=world\n").unwrap();
        let body = build_at(&record, &dir, Some(10)).body;
        assert!(!body.contains("GPU errors") && !body.contains("GPU held"), "{body}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_numbers_and_fixed_names_of_the_state_are_published() {
        for (value, published) in [
            ("world", true),
            ("58.3", true),
            ("1920x1080", true),
            ("3120/5800", true),
            ("custom", true),
            ("Alice Resident", false),
            ("12/x", false),
            ("", false),
            ("1234567890123", false),
        ] {
            assert_eq!(state_value(value).is_some(), published, "{value:?}");
        }
    }
}
