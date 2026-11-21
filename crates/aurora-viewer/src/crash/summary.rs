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
    // An `alive.txt` emptied by a system that stopped at once still has its
    // date: without it, a Windows restart would never be told (#3056).
    let seen = tick.or_else(|| super::watchdog::alive_written(dir));
    let end = stopped.or(seen);
    if let Some(duration) = end.and_then(|end| end.checked_sub(record.started_unix)) {
        let approximate = if stopped.is_none() { " (last sign of life)" } else { "" };
        rows.push(("Session duration", format!("{duration} s{approximate}")));
    }
    if stopped.is_none() {
        rows.push(("End of session", unseen_end(seen, boot_unix).to_owned()));
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
    let gpu_error_ring = gpu_error_ring(&tail, stopped.or(tick));
    if !gpu_error_ring.is_empty() {
        let _ = std::fs::write(dir.join("gpu-errors-private.txt"), gpu_error_ring.join("\n\n"));
    }
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
        // device_lost.rs), rebuilt from the renderer's own lines: the loss
        // that began the end of the session, then the recoveries' losses
        if let Some((first, later)) = gpu_work_in(&tail) {
            rows.push(("GPU work at the loss", first));
            if let Some(later) = later {
                rows.push(("GPU work at the later losses", later));
            }
        }
    }
    let has_gpu_trouble = dir.join("gpu-lost.txt").exists()
        || dir.join("gpu-stall.txt").exists()
        || !stall.is_empty()
        || value("loss_reason").is_some()
        || value("loss_cause").is_some();
    // Head and tail both: in a long session the first errors, usually the
    // cause, are only in the head. On a GPU loss "none" says something too:
    // the device was lost without an error of wgpu's own before it (#2497).
    if let Some(errors) = gpu_errors_in(&head, &tail) {
        rows.push(("GPU errors", errors));
    } else if has_gpu_trouble {
        rows.push(("GPU errors", "none in the retained session log".into()));
    }
    if has_gpu_trouble {
        let risk = risk_factors(
            value("effects"),
            number("session_s"),
            value("vram_mb").and_then(vram_headroom),
            value("region_s").and_then(|v| v.parse().ok()),
            // the same filter as the Last state rows: a value the watchdog
            // wrote is published only in a known shape
            value("preset").and_then(state_value).as_deref(),
            gpu.as_deref().and_then(driver_version),
            !text("overlay-hooks.txt").trim().is_empty(),
        );
        // right under the crash's kind, whichever rows come before it
        let at = rows.iter().position(|(field, _)| *field == "Crash").map_or(rows.len(), |i| i + 1);
        rows.insert(at, ("Risk factors", risk));
    }
    if let Some(seconds) = text("gpu-stall.txt").lines().next().and_then(|line| {
        line.strip_prefix("GPU stall: no frame for ")?
            .strip_suffix(" s")?
            .parse::<u64>()
            .ok()
    }) {
        rows.push((
            "GPU not responding",
            format!("noticed after {seconds} s (how long the GPU hung is unknown), the window still answering"),
        ));
    }
    if let Some(recoveries) = gpu_recoveries(&text("gpu-recoveries.txt"), record.started_unix) {
        rows.push(("GPU recoveries", recoveries));
    }
    // A row only when it has something to say: the hooks seen, the layers
    // listed, a runtime loaded. The snapshot at the loss is written on a
    // loss only (Windows), so its row says "none" when no hook was loaded.
    if let Some(hooks) = overlay_hooks(&text("overlay-hooks.txt")) {
        rows.push(("Overlay hooks", hooks));
    }
    if dir.join("overlay-hooks-loss.txt").exists() {
        let at_loss = overlay_hooks(&text("overlay-hooks-loss.txt")).unwrap_or_else(|| "none loaded".into());
        rows.push(("Overlay hooks at GPU loss", at_loss));
    }
    if let Some(layers) = vulkan_layers(&text("vulkan-layers.txt")) {
        rows.push(("Vulkan layers installed", layers));
        // eligible: not turned off by their variable or the loader's filter,
        // which does not prove the instance enabled them
        if let Some(eligible) = super::read_limited(&dir.join("vulkan-layers-eligible.txt"), 900)
            .ok()
            .filter(|names| !names.trim().is_empty())
            .and_then(|names| vulkan_layers(&names))
        {
            rows.push(("Vulkan layers eligible (known ones)", eligible));
        }
    }
    // Collected for every crash; shown on a GPU loss or stall, or when the
    // OS recorded something in the window.
    let os_events = text("os-gpu-events.txt");
    if has_gpu_trouble || !os_events.trim().is_empty() {
        let os_status = super::read_limited(&dir.join("os-gpu-events.status"), 256)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| os_gpu_status_known(s));
        rows.push((
            "OS GPU reset records",
            os_status
                .map(|status| os_gpu_event_summary(&status, &os_events))
                .unwrap_or_else(|| "not collected: operating system collection unavailable".into()),
        ));
    }
    if let Some(xr) = xr_runtime_in(&head, &tail) {
        rows.push(("XR runtime", xr));
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
        .filter_map(|&(key, label)| {
            value(key).and_then(state_value).map(|v| {
                let v = if key == "loss_cause" && v == "other" {
                    "unexpected Vulkan result (code not kept by wgpu-hal)".to_owned()
                } else {
                    v
                };
                (label, v)
            })
        })
        .collect();
    if !state.is_empty() {
        body.push_str("\n### Last state\n\nNoted by the viewer every few seconds.\n\n| | |\n|---|---|\n");
        for (label, value) in state {
            body.push_str(&format!("| {label} | {value} |\n"));
        }
    }
    let excerpt = super::privacy::excerpt(&tail);
    if !excerpt.is_empty() {
        body.push_str(&format!(
            "\n### Recent warnings and errors\n\nTime before the end of the log, level, Rust module and kind; the messages stay on the tester's PC.\n\n```text\n{excerpt}\n```\n"
        ));
    }
    Summary { title, body, gpu }
}

/// Public OS evidence uses fixed labels. Raw Linux kernel messages and local
/// Windows details remain in the crash folder.
fn os_gpu_event_summary(status: &str, events: &str) -> String {
    let mut labels = Vec::new();
    for line in events.lines().take(60) {
        let lower = line.to_ascii_lowercase();
        if let Some(timestamp) = line.strip_prefix("Display 4101 at ") {
            if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(timestamp) {
                labels.push(format!("Display 4101 at {}", timestamp.to_rfc3339()));
            }
        } else if let Some(rest) = line.strip_prefix("LiveKernelEvent ") {
            if let Some((code, timestamp)) = rest.split_once(" at ")
                && matches!(code, "141" | "117" | "1a8")
                && let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(timestamp)
            {
                labels.push(format!("LiveKernelEvent {code} at {}", timestamp.to_rfc3339()));
            }
        } else {
            // whole words for the short ones and the events: "xe" is in
            // "exec", "fault" in "default", "ring" in "string"
            let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).collect();
            let word = |w: &str| words.contains(&w);
            let driver = if lower.contains("amdgpu") {
                Some("amdgpu")
            } else if lower.contains("radeon") {
                Some("Radeon")
            } else if lower.contains("nouveau") || lower.contains("nvidia") {
                Some("NVIDIA")
            } else if lower.contains("i915") || word("xe") {
                Some("Intel")
            } else if word("drm") {
                Some("DRM")
            } else if word("gpu") {
                Some("GPU")
            } else {
                None
            };
            let event = if words.iter().any(|w| w.starts_with("timeout") || w.starts_with("timedout")) {
                Some("timeout")
            } else if words.iter().any(|w| w.starts_with("reset")) {
                Some("reset")
            } else if word("fault") || word("faults") {
                Some("fault")
            } else if word("ring") {
                Some("ring")
            } else {
                None
            };
            if let Some(driver) = driver {
                labels.push(match event {
                    Some(event) => format!("{driver} {event}"),
                    None => format!("{driver} GPU event"),
                });
            }
        }
    }
    labels.sort();
    labels.dedup();
    if labels.is_empty() {
        status.to_owned()
    } else {
        format!("{status}; observed: {}", labels.join(", "))
    }
}

fn os_gpu_status_known(status: &str) -> bool {
    status.starts_with("collected:")
        || status.starts_with("collected (partial):")
        || status.starts_with("collected (incomplete):")
        || status.starts_with("not collected:")
}

/// What was stacked up at a GPU loss or stall, from what the report already
/// has: the heavy effects first, then memory headroom, an overlay hook, a
/// region change, the preset, driver and session length. The order is a
/// guess at weight, not a measure, so no High / Medium / Low rating comes
/// with it: a rating would read as a finding, and the high preset alone
/// (contact shadows, GTAO) would rate most reports the same. A rating can
/// come back once crash statistics give the weights.
fn risk_factors(
    effects: Option<&str>,
    uptime: Option<u64>,
    headroom: Option<u8>,
    region_age: Option<u64>,
    preset: Option<&str>,
    driver: Option<String>,
    has_hook: bool,
) -> String {
    let mut factors = Vec::new();
    if let Some(effects) = effects {
        for (needle, label, weight) in [
            ("contact", "contact shadows", 5),
            ("gtao", "GTAO", 5),
            ("ssr", "SSR", 4),
            ("msaa", "MSAA", 3),
            ("projector", "projector shadows", 3),
            ("indirect", "indirect light", 3),
            ("probe", "probes", 2),
        ] {
            if effects.to_ascii_lowercase().contains(needle) {
                factors.push((weight, label.to_owned()));
            }
        }
    }
    if has_hook {
        factors.push((4, "overlay hook recorded".to_owned()));
    }
    if let Some(headroom) = headroom
        && headroom <= 10
    {
        factors.push((5, format!("video-memory headroom {headroom}%")));
    }
    if let Some(seconds) = region_age.filter(|seconds| *seconds <= 60) {
        factors.push((2, format!("region changed {seconds}s before the last state")));
    }
    if let Some(preset) = preset {
        factors.push((1, format!("{preset} preset")));
    }
    if let Some(driver) = driver {
        factors.push((1, driver));
    }
    if let Some(seconds) = uptime {
        factors.push((1, format!("session {seconds}s")));
    }
    if factors.is_empty() {
        return "none known: the graphics state was not noted".into();
    }
    // stable: factors of the same weight keep the order above
    factors.sort_by_key(|factor| std::cmp::Reverse(factor.0));
    format!(
        "{} (not included: 60-second memory minimum, driver age, scene events)",
        factors.into_iter().map(|(_, s)| s).collect::<Vec<_>>().join(" · ")
    )
}

fn vram_headroom(value: &str) -> Option<u8> {
    let (used, budget) = value.split_once('/')?;
    let used = used.parse::<u64>().ok()?;
    let budget = budget.parse::<u64>().ok()?;
    (budget > 0 && used <= budget).then(|| (((budget - used) * 100) / budget).min(100) as u8)
}

fn driver_version(gpu: &str) -> Option<String> {
    let lower = gpu.to_ascii_lowercase();
    let vendor = if lower.contains("nvidia") {
        "NVIDIA"
    } else if lower.contains("amd") || lower.contains("radeon") {
        "AMD"
    } else if lower.contains("intel") {
        "Intel"
    } else {
        return None;
    };
    let tail = gpu.split("driver ").nth(1)?;
    // the leading digits and points of the first token that starts with a
    // digit: `26.2.5` from Mesa's `26.2.5-arch3.1`, a distribution's suffix
    let version = tail.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches(|c: char| !c.is_ascii_digit());
        let end = token.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(token.len());
        let token = token[..end].trim_end_matches('.');
        (!token.is_empty() && token.len() <= 24).then_some(token)
    })?;
    Some(format!("{vendor} driver {version}"))
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
    ("vram_headroom_60s_mb", "Video memory headroom, tightest in 60 s (MB)"),
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

/// Loader enumeration reports installed layers, not which layer was active
/// in the created instance. Publish only names of a known shape.
fn vulkan_layers(text: &str) -> Option<String> {
    let text = text.trim();
    let layer =
        |token: &str| token.len() <= 48 && token.starts_with("VK_LAYER_") && token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    (text.len() <= 900 && text.split(',').all(layer)).then(|| text.to_owned())
}

/// The VR runtime of the session, from aurora-render xr_device.rs's lines,
/// which give a fixed name only (the runtime's own name stays in the log).
fn xr_runtime_in(head: &str, tail: &str) -> Option<String> {
    const KNOWN: [&str; 5] = ["SteamVR", "Meta", "Virtual Desktop", "Monado", "Windows Mixed Reality"];
    let runtime = [head, tail]
        .into_iter()
        .flat_map(str::lines)
        .filter_map(|line| line.split_once(" OpenXR runtime loaded: ").map(|(_, name)| name))
        .find(|name| KNOWN.contains(name) || *name == "other OpenXR runtime")?;
    let created = [head, tail]
        .into_iter()
        .flat_map(str::lines)
        .any(|line| line.ends_with(" OpenXR runtime created Vulkan device"));
    Some(format!(
        "{runtime}, {}",
        if created {
            "which created the Vulkan device"
        } else {
            "no Vulkan device of its own noted"
        }
    ))
}

/// The GPU errors wgpu handed to the viewer, first of each in the order they
/// came, with how many there were: `out-of-memory`, with the call when the
/// log names it (`out-of-memory in Device::create_bind_group`, #2497), or
/// the wgpu operation that failed, never the rest of the message, whose
/// labels are free text. A session stuck on these errors (#2311, x155)
/// otherwise says nothing of what failed. The first ones come first: they
/// are usually the cause. A line of any other module or level is left out,
/// as in [`gpu_work_in`].
fn gpu_errors_in(head: &str, tail: &str) -> Option<String> {
    const SHOWN: usize = 10;
    let mut found: Vec<(String, usize)> = Vec::new();
    for text in [head_only(head, tail), tail] {
        for error in wgpu_errors(text) {
            let name = wgpu_error_name(&error);
            match found.iter_mut().find(|(seen, _)| *seen == name) {
                Some((_, count)) => *count = count.saturating_add(error.count),
                None => found.push((name, error.count)),
            }
        }
    }
    if found.is_empty() {
        return None;
    }
    let more = if found.len() > SHOWN { ", …" } else { "" };
    found.truncate(SHOWN);
    let names: Vec<_> = found.iter().map(|(name, count)| format!("{name} (x{count})")).collect();
    Some(format!("{}{more}", names.join(", ")))
}

/// One wgpu error of the log (`wgpu error: …`, aurora-render
/// main_thread.rs and gpu_alloc.rs): its time, its message (the rest of its header line and
/// the lines below it, the repeat count taken off) and how many errors it
/// stands for.
struct WgpuError<'a> {
    time: &'a str,
    lines: Vec<&'a str>,
    count: usize,
}

fn wgpu_errors(text: &str) -> Vec<WgpuError<'_>> {
    let mut errors: Vec<WgpuError<'_>> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        if log_header(line) {
            open = false;
            let Some((record, message)) = line.split_once("] wgpu error: ") else {
                continue;
            };
            // the device's handler, or an error scope of gpu_alloc.rs (the
            // render targets refused while video memory is full, #2887)
            if !record.ends_with(" ERROR aurora_render::main_thread") && !record.ends_with(" ERROR aurora_render::gpu_alloc") {
                continue;
            }
            let time = record.trim_start_matches('[').split_whitespace().next().unwrap_or("unknown");
            errors.push(WgpuError {
                time,
                lines: vec![message],
                count: 1,
            });
            open = true;
        } else if open && let Some(error) = errors.last_mut() {
            error.lines.push(line);
        }
    }
    let trim = |lines: &mut Vec<&str>| {
        while lines.len() > 1 && lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.pop();
        }
    };
    for error in &mut errors {
        trim(&mut error.lines);
        if let Some(last) = error.lines.last_mut()
            && let Some((count, rest)) = repeated_count(last)
        {
            error.count = count.max(1);
            *last = rest;
        }
        trim(&mut error.lines);
    }
    errors
}

/// The count `LogThrottle::note` (aurora-render gpu_alloc.rs) puts at the
/// end of a record that stands for several errors, ` (repeated N times in
/// the last S s)`, and the line without it. N counts every call since the
/// last line written, this one included. The count ends the record's last
/// line: on a line of its own after a message that ends with a newline
/// (wgpu's validation errors), after the text otherwise (out-of-memory,
/// #2497).
fn repeated_count(line: &str) -> Option<(usize, &str)> {
    let (before, rest) = line.rsplit_once("(repeated ")?;
    let (count, after) = rest.split_once(" times in the last ")?;
    let seconds = after.strip_suffix(" s)")?;
    if seconds.is_empty() || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((count.parse().ok()?, before.trim_end()))
}

/// What the report calls an error: `out-of-memory`, with the call when the
/// message names one, else the operation that failed, else `other`.
fn wgpu_error_name(error: &WgpuError<'_>) -> String {
    let out_of_memory = error.lines.first().is_some_and(|line| line.starts_with("Out of Memory"));
    let operation = error
        .lines
        .iter()
        .skip(1)
        .find_map(|line| line.trim_start().strip_prefix("In "))
        .and_then(gpu_operation);
    match (out_of_memory, operation) {
        (true, Some(operation)) => format!("out-of-memory in {operation}"),
        (true, None) => "out-of-memory".to_owned(),
        (false, Some(operation)) => operation.to_owned(),
        (false, None) => "other".to_owned(),
    }
}

/// The part of the log's head its tail does not hold too. A log shorter
/// than the head and the tail together is in both where they overlap, and
/// an error read twice would be counted twice: the head stops where the
/// tail's first whole record begins.
fn head_only<'a>(head: &'a str, tail: &str) -> &'a str {
    let Some(first) = tail.lines().find(|line| log_header(line)) else {
        return head;
    };
    let mut at = 0;
    for line in head.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == first {
            return &head[..at];
        }
        at += line.len();
    }
    head
}

/// The most recent distinct wgpu error blocks from the retained private log.
/// The raw text contains resource labels, so this artifact never enters the
/// public report. Its fixed cap keeps it below 1 MiB.
fn gpu_error_ring(log: &str, loss_unix: Option<u64>) -> Vec<String> {
    const MAX_ERRORS: usize = 50;
    const MAX_ENTRY_BYTES: usize = 16 * 1024;
    // (time of the newest, message, count), the newest last
    let mut blocks: Vec<(&str, String, usize)> = Vec::new();
    for error in wgpu_errors(log) {
        let mut body = error.lines.join("\n");
        body.truncate(body.floor_char_boundary(MAX_ENTRY_BYTES));
        if let Some(index) = blocks.iter().position(|(_, prior, _)| *prior == body) {
            let (_, prior, count) = blocks.remove(index);
            blocks.push((error.time, prior, count.saturating_add(error.count)));
            continue;
        }
        if blocks.len() == MAX_ERRORS {
            blocks.remove(0);
        }
        blocks.push((error.time, body, error.count));
    }
    let end = loss_unix.and_then(|s| chrono::DateTime::from_timestamp(s as i64, 0));
    blocks
        .into_iter()
        .map(|(at, body, count)| {
            let age = match (chrono::DateTime::parse_from_rfc3339(at), end) {
                (Ok(at), Some(end)) => format!("{}s before session end", end.signed_duration_since(at).num_seconds().max(0)),
                _ => "age unavailable".into(),
            };
            let operation = body
                .lines()
                .find_map(|line| line.trim_start().strip_prefix("In ").and_then(gpu_operation))
                .unwrap_or("other");
            format!("time={at} operation={operation} count={count} age={age}\n{body}")
        })
        .collect()
}

fn log_header(line: &str) -> bool {
    let Some(header) = line
        .strip_prefix('[')
        .and_then(|line| line.split_once("] ").map(|(header, _)| header))
    else {
        return false;
    };
    let mut fields = header.split_whitespace();
    let Some(timestamp) = fields.next() else { return false };
    if chrono::DateTime::parse_from_rfc3339(timestamp).is_err() {
        return false;
    }
    let Some(level) = fields.next() else { return false };
    if !["TRACE", "DEBUG", "INFO", "WARN", "ERROR"].contains(&level) {
        return false;
    }
    let Some(target) = fields.next() else { return false };
    target.len() <= 96
        && target
            .split("::")
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
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

/// A state value as the watchdog writes it: a number (a `-` before it for
/// a headroom gone negative), numbers joined by `/` or `x`, a list of the
/// effect words, or one of the fixed names; anything else is not published.
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
    let passes = [
        "shadows_depth",
        "effects",
        "terrain_sky",
        "objects",
        "water",
        "transparent",
        "weather",
        "post",
    ];
    let numeric = |v: &str| {
        let digits = v.strip_prefix('-').unwrap_or(v);
        !digits.is_empty() && v.len() <= 12 && digits.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    };
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

/// Losses this close to the next one belong to the same end of the
/// session: the recovery counts them together over the same span
/// (`LOSS_SPAN`, app/gpu_recovery.rs).
const LOSS_CHAIN_S: i64 = 5 * 60;
/// Later losses published at most (the recovery closes at the third).
const MAX_LATER_LOSSES: usize = 4;

/// The "GPU work at the loss" row and the "GPU work at the later losses"
/// one, from the lines the device-lost callback logged
/// (`[… ERROR aurora_render::main_thread] GPU work at the loss: …`),
/// rebuilt by [`gpu_work_row`]. A line of any other module or level is
/// left out: a name logged elsewhere could carry the marker.
///
/// The first row is the loss that began the end of the session: the
/// oldest of the last losses less than [`LOSS_CHAIN_S`] apart. Each
/// recovery makes a new device, whose own line follows; the newest line
/// alone described the last of them, often a recovery's device lost before
/// its first frame ("nothing submitted yet"), and hid the loss that
/// mattered (RX 9070 XT reports of 0.9.3, #2238). A line without a
/// readable time stays with the line after it.
fn gpu_work_in(tail: &str) -> Option<(String, Option<String>)> {
    let lines: Vec<(Option<i64>, String)> = tail
        .lines()
        .filter_map(|line| {
            let (head, text) = line.split_once("] GPU work at the loss: ")?;
            if !head.ends_with(" ERROR aurora_render::main_thread") {
                return None;
            }
            let at = head
                .strip_prefix('[')
                .and_then(|head| head.split_whitespace().next())
                .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(stamp).ok())
                .map(|at| at.timestamp());
            Some((at, gpu_work_row(text)?))
        })
        .collect();
    let mut start = lines.len().checked_sub(1)?;
    while start > 0 {
        let apart = match (lines[start - 1].0, lines[start].0) {
            (Some(before), Some(after)) => after - before,
            _ => 0,
        };
        if apart > LOSS_CHAIN_S {
            break;
        }
        start -= 1;
    }
    let chain = &lines[start..];
    let later: Vec<String> = chain
        .iter()
        .enumerate()
        .skip(1)
        .take(MAX_LATER_LOSSES)
        .map(|(i, (_, row))| format!("loss {}: {row}", i + 1))
        .collect();
    Some((chain[0].1.clone(), (!later.is_empty()).then(|| later.join(" / "))))
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
    let detection_note = if verdict.starts_with("a hang") || verdict.starts_with("undecided") {
        "; noticed after about 1 s (detection latency; actual hang duration unknown)"
    } else {
        ""
    };
    Some(format!(
        "last submission frame {frame}, {age} s before, {state}, {last}, {verdict}{detection_note}"
    ))
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

    #[test]
    fn operating_system_gpu_evidence_is_labeled_and_redacted() {
        let linux = os_gpu_event_summary(
            "collected: 2 matching kernel records",
            "[private timestamp] amdgpu ring timeout at PRIVATE_PATH\nDRM reset for resident PRIVATE_NAME",
        );
        assert!(linux.contains("amdgpu timeout"));
        assert!(linux.contains("DRM reset"));
        assert!(!linux.contains("PRIVATE_PATH"));
        assert!(!linux.contains("PRIVATE_NAME"));

        let windows = os_gpu_event_summary(
            "collected: 2 Windows GPU reset events in the time window",
            "Display 4101 at 2026-11-04T10:00:00.000Z\nLiveKernelEvent 141 at 2026-11-04T10:00:01.000Z\nRaw PRIVATE MESSAGE",
        );
        assert!(windows.contains("Display 4101 at 2026-11-04T10:00:00+00:00"));
        assert!(windows.contains("LiveKernelEvent 141 at 2026-11-04T10:00:01+00:00"));
        assert!(!windows.contains("PRIVATE"));
    }

    #[test]
    fn os_status_accepts_only_known_collected_and_unavailable_forms() {
        for status in [
            "collected: no matching records",
            "collected (partial): one channel failed",
            "collected (incomplete): query reached cap",
            "not collected: query unavailable",
        ] {
            assert!(os_gpu_status_known(status), "{status}");
        }
        assert!(!os_gpu_status_known("collected arbitrary private text"));
    }

    #[test]
    fn partial_and_incomplete_os_event_statuses_keep_collected_records() {
        for status in [
            "collected (partial): 1 Windows GPU reset event; one event channel unavailable",
            "collected (incomplete): 60 Windows GPU reset events; query reached the record cap",
        ] {
            let (dir, record) = session();
            std::fs::write(dir.join("os-gpu-events.status"), status).unwrap();
            std::fs::write(dir.join("os-gpu-events.txt"), "Display 4101 at 2026-11-04T10:00:00.000Z").unwrap();
            let body = build_at(&record, &dir, None).body;
            assert!(body.contains(status), "{body}");
            assert!(body.contains("Display 4101 at 2026-11-04T10:00:00+00:00"), "{body}");
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

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
    fn the_crash_gpu_row_shows_apu_dedicated_and_shared_memory() {
        let (dir, record) = session();
        std::fs::write(
            dir.join("session.log"),
            "[t INFO  aurora_render::main_thread] GPU: AMD Radeon Graphics (Vulkan, driver AMD 1.2, memory 512 MB dedicated + 7800 MB shared)\n",
        )
        .unwrap();
        let body = build(&record, &dir).body;
        assert!(body.contains("| GPU | AMD Radeon Graphics (Vulkan, driver AMD 1.2, memory 512 MB dedicated + 7800 MB shared) |"));
        std::fs::remove_dir_all(dir).unwrap();
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
            body.contains("| GPU work at the loss | last submission frame 601, 1.101 s before, not finished, frame 599 finished 1.103 s before, a hang (the driver reset the card while that frame ran); noticed after about 1 s (detection latency; actual hang duration unknown) |"),
            "{body}"
        );
    }

    /// The row comes from the renderer's own line only, rebuilt from its
    /// numbers and fixed words: a name logged by another module that
    /// carries the marker publishes nothing, a line that does not parse
    /// whole neither; of a session recovered then lost again, the first
    /// loss's line is the row and the later one follows it.
    #[test]
    fn the_gpu_work_row_is_rebuilt_from_the_renderer_s_own_lines() {
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
        let alone = |log: &str| gpu_work_in(log).map(|(first, later)| (first, later.is_none()));
        let expected_hang = format!("{HANG}; noticed after about 1 s (detection latency; actual hang duration unknown)");
        assert_eq!(alone(&own(HANG)), Some((expected_hang.clone(), true)));
        assert_eq!(alone(&own(FAULT)), Some((FAULT.to_owned(), true)));
        let no_completion = "last submission frame 0, 3.000 s before, not finished, none finished yet, a hang (the driver reset the card while that frame ran)";
        let expected_no_completion = format!("{no_completion}; noticed after about 1 s (detection latency; actual hang duration unknown)");
        assert_eq!(alone(&own(no_completion)), Some((expected_no_completion, true)));
        assert_eq!(
            alone(&own("nothing submitted yet (lost while starting)")),
            Some(("nothing submitted yet (lost while starting)".to_owned(), true))
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
        // a recovered loss, then the fatal one: the first, then the later
        let twice = format!("{}{}", own(FAULT), own(HANG));
        assert_eq!(
            gpu_work_in(&twice),
            Some((FAULT.to_owned(), Some(format!("loss 2: {expected_hang}"))))
        );
    }

    /// The RX 9070 XT reports of 0.9.3 (#2238): a hang, then each
    /// recovery's new device lost before its first frame. The row is the
    /// hang; the recoveries' losses follow it. A loss more than five minutes
    /// before the next one (recovered then, long ago) is not part of the end.
    #[test]
    fn the_gpu_work_row_is_the_loss_that_began_the_end_not_the_recovery_s() {
        const HANG: &str = "last submission frame 34120, 1.204 s before, not finished, frame 34119 finished 1.211 s before, a hang (the driver reset the card while that frame ran)";
        const FAULT: &str = "last submission frame 12, 0.008 s before, not finished, frame 11 finished 0.009 s before, a fault in the work of that frame (not a hang)";
        const NONE: &str = "nothing submitted yet (lost while starting)";
        let at = |stamp: &str, text: &str| format!("[{stamp} ERROR aurora_render::main_thread] GPU work at the loss: {text}\n");
        let noted = "; noticed after about 1 s (detection latency; actual hang duration unknown)";
        let log = format!(
            "{}[2026-11-08T08:21:30.100Z WARN  aurora_render::renderer] GPU device lost\n{}{}",
            at("2026-11-08T07:40:00.000Z", FAULT),
            at("2026-11-08T08:21:31.000Z", HANG),
            at("2026-11-08T08:21:49.500Z", NONE),
        );
        assert_eq!(gpu_work_in(&log), Some((format!("{HANG}{noted}"), Some(format!("loss 2: {NONE}")))));
        // recovered once, lost again, the safe level's device lost at start
        let three = format!(
            "{}{}{}",
            at("2026-11-08T12:00:00.000Z", HANG),
            at("2026-11-08T12:00:07.000Z", FAULT),
            at("2026-11-08T12:00:13.000Z", NONE),
        );
        assert_eq!(
            gpu_work_in(&three),
            Some((format!("{HANG}{noted}"), Some(format!("loss 2: {FAULT} / loss 3: {NONE}"))))
        );
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
        assert!(!summary.body.contains("Risk factors"));
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
            "tick=1098\nresponsive=1075\nram_mb=5120\nscreen=world\nfps=0.4\nwindow=1920x1080\npreset=high\nvram_mb=5800/6000\nvram_headroom_60s_mb=-512\ntexture_pressure=7/7\nobjects=4200\nswapchain_10s=37\nregion_name=Private Place\neffects=shadows3,contact-shadows,ao-gtao2,aa-smaa,scale100,draw192\nwindow_state=background\ngpu_ms=12.5\ngpu_peak_ms=2400.0\ngpu_peak_pass=effects\nuploads=12/34.5\nsession_s=45\nregion_s=20\nloading=12/3/40\ngpu_peak_ao_contact=20.5/2100.0\nlayers=VK_LAYER_OLD_NOTE\nloss_reason=unknown\nloss_cause=device-lost\nloss_seen_by=callback\nstall_ms=2300\nother_viewers=2/1\ncard_used_mb=6100\ncard_others_mb=3200\nlayers_note=/home/someone\n",
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
            "| Video memory headroom, tightest in 60 s (MB) | -512 |",
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
            "| Vulkan layers installed | VK_LAYER_OBS_HOOK,VK_LAYER_NV_optimus |",
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

    #[test]
    fn a_sign_of_life_emptied_by_a_stopped_system_still_dates_the_end() {
        let (dir, record) = session();
        // What a power cut leaves of a file renamed without a flush.
        let alive = dir.join("alive.txt");
        std::fs::write(&alive, [0u8; 300]).unwrap();
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1500);
        std::fs::File::options()
            .append(true)
            .open(&alive)
            .unwrap()
            .set_modified(at)
            .unwrap();
        let body = build_at(&record, &dir, Some(1600)).body;
        assert!(body.contains("| Session duration | 500 s (last sign of life) |"), "{body}");
        assert!(
            body.contains("| End of session | not seen: Windows started again right after"),
            "{body}"
        );
        // The content is lost: nothing to say about the window.
        assert!(!body.contains("| Window |"), "{body}");
        // A session that ended before its first sign of life stays unknown.
        std::fs::remove_file(&alive).unwrap();
        let body = build_at(&record, &dir, Some(1600)).body;
        assert!(!body.contains("Session duration"), "{body}");
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
        // no hook seen: no row
        assert!(!build_at(&record, &dir, None).body.contains("| Overlay hooks"));
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
    fn the_gpu_loss_hook_snapshot_uses_only_known_module_names() {
        let (dir, record) = session();
        std::fs::write(dir.join("gpu-lost.txt"), "").unwrap();
        std::fs::write(
            dir.join("overlay-hooks-loss.txt"),
            "DiscordHook64.dll\nC:\\Users\\Private\\evil.dll\n",
        )
        .unwrap();
        let body = build_at(&record, &dir, None).body;
        assert!(
            body.contains("| Overlay hooks at GPU loss | Discord (in-game overlay or Go Live capture), DiscordHook64.dll |"),
            "{body}"
        );
        assert!(!body.contains("Private") && !body.contains("evil"), "{body}");
        // a snapshot with no known hook in it says so
        std::fs::write(dir.join("overlay-hooks-loss.txt"), "").unwrap();
        assert!(
            build_at(&record, &dir, None)
                .body
                .contains("| Overlay hooks at GPU loss | none loaded |")
        );
        // no snapshot (no loss, or not Windows): no row
        std::fs::remove_file(dir.join("overlay-hooks-loss.txt")).unwrap();
        assert!(!build_at(&record, &dir, None).body.contains("Overlay hooks at GPU loss"));
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
            body.contains("| Vulkan layers installed | VK_LAYER_OBS_HOOK,VK_LAYER_RTSS |"),
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

    #[test]
    fn xr_runtime_rows_use_only_the_closed_runtime_vocabulary() {
        let log = "[t INFO aurora_render::xr_device] OpenXR runtime loaded: SteamVR\n[t INFO aurora_render::xr_device] OpenXR runtime created Vulkan device\n";
        assert_eq!(xr_runtime_in(log, "").as_deref(), Some("SteamVR, which created the Vulkan device"));
        assert_eq!(
            xr_runtime_in("[t INFO aurora_render::xr_device] OpenXR runtime loaded: /home/alice\n", ""),
            None
        );
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
        // a record that says "repeated 154 times" stands for 154 (#2311, x155)
        let expected = Some("out-of-memory (x1), Queue::write_buffer (x155), CommandEncoder::finish (x1)");
        assert_eq!(gpu_errors_in(&log, "").as_deref(), expected);
        // the head and the tail of a short log are the same text
        assert_eq!(gpu_errors_in(&log, &log).as_deref(), expected);
    }

    #[test]
    fn a_refusal_caught_by_an_error_scope_counts_like_the_others() {
        // the render targets refused at login (gpu_alloc.rs's scopes, #2887)
        let log = "[2026-11-04T10:00:00.000Z ERROR aurora_render::gpu_alloc] wgpu error: Out of Memory\n\nCaused by:\n  In Device::create_texture, label = 'hdr'\n    Not enough memory left.\n\
             [2026-11-04T10:00:00.000Z WARN  aurora_render::renderer] wgpu error: Out of Memory\n\
             [2026-11-04T10:00:01.000Z ERROR aurora_render::main_thread] wgpu error: Out of Memory\n\nCaused by:\n  In Device::create_texture, label = 'hdr'\n";
        assert_eq!(
            gpu_errors_in(log, "").as_deref(),
            Some("out-of-memory in Device::create_texture (x2)")
        );
    }

    /// The counts as aurora-render writes them: each error through its
    /// `LogThrottle`, in the text main_thread.rs's `error_text` gives (#2497).
    #[test]
    fn gpu_error_counts_follow_the_throttle_of_the_log() {
        use std::time::{Duration, Instant};
        // an out-of-memory ends with no newline, a validation error with one:
        // the repeat count ends a line of text, or is a line of its own
        let oom =
            "wgpu error: Out of Memory\n\nCaused by:\n  In Device::create_bind_group, label = 'Private Place'\n    Not enough memory left.";
        let validation =
            "wgpu error: Validation Error\n\nCaused by:\n  In Queue::write_buffer, label = 'Private Place'\n    Buffer is invalid\n";
        let mut throttle = aurora_render::gpu_alloc::LogThrottle::new(Duration::from_secs(10));
        let start = Instant::now();
        let mut log = String::new();
        // seven of each: six in the period, the last after it
        for second in [0u64, 1, 2, 3, 4, 5, 11] {
            for message in [oom, validation] {
                if let Some(line) = throttle.note(message, start + Duration::from_secs(second)) {
                    log.push_str(&format!(
                        "[2026-11-04T10:00:{second:02}.000Z ERROR aurora_render::main_thread] {line}\n"
                    ));
                }
            }
        }
        assert_eq!(log.matches("(repeated 6 times").count(), 2, "{log}");
        let expected = Some("out-of-memory in Device::create_bind_group (x7), Queue::write_buffer (x7)");
        assert_eq!(gpu_errors_in(&log, &log).as_deref(), expected);
        // the private file counts the same
        let ring = gpu_error_ring(&log, None);
        assert!(ring.iter().all(|entry| entry.contains(" count=7 ")), "{ring:?}");
    }

    #[test]
    fn a_log_read_in_overlapping_head_and_tail_counts_each_error_once() {
        let records: Vec<String> = (0..4)
            .map(|i| {
                format!(
                    "{}Validation Error\n\nCaused by:\n  In Queue::submit\n    bad\n\n",
                    WGPU_ERROR.replace(":00.000Z", &format!(":0{i}.000Z"))
                )
            })
            .collect();
        // overlapping: records 1 and 2 in both
        assert_eq!(
            gpu_errors_in(&records[..3].concat(), &records[1..].concat()).as_deref(),
            Some("Queue::submit (x4)")
        );
        // apart, as in a log longer than both
        assert_eq!(
            gpu_errors_in(&records[..2].concat(), &records[2..].concat()).as_deref(),
            Some("Queue::submit (x4)")
        );
        // an error only in the head (the first ones of a long session) is kept
        assert_eq!(
            gpu_errors_in(&format!("{WGPU_ERROR}Out of Memory\n"), &records[3]).as_deref(),
            Some("out-of-memory (x1), Queue::submit (x1)")
        );
    }

    #[test]
    fn an_out_of_memory_error_says_which_call_when_the_log_does() {
        let e = WGPU_ERROR;
        let named = format!(
            "{e}Out of Memory\n\nCaused by:\n  In Device::create_bind_group, label = 'Private Place'\n    Not enough memory left.\n\
             [2026-11-04T10:00:01.000Z INFO  aurora_viewer] still here\n"
        );
        assert_eq!(
            gpu_errors_in(&named, &named).as_deref(),
            Some("out-of-memory in Device::create_bind_group (x1)")
        );
        // a name that is not an operation of wgpu's is not published
        let odd = format!("{e}Out of Memory\n\nCaused by:\n  In Alice::resident\n");
        assert_eq!(gpu_errors_in(&odd, "").as_deref(), Some("out-of-memory (x1)"));
        // a repeat record, as the throttle writes it: five more
        let repeated = format!(
            "{named}{e}Out of Memory\n\nCaused by:\n  In Device::create_bind_group, label = 'Private Place'\n    Not enough memory left. (repeated 5 times in the last 10 s)\n"
        );
        assert_eq!(
            gpu_errors_in(&repeated, "").as_deref(),
            Some("out-of-memory in Device::create_bind_group (x6)")
        );
        // a label that looks like a count is not one
        let odd = format!("{e}Validation Error\n\nCaused by:\n  In Queue::submit, label = '(repeated 9 times'\n    bad\n");
        assert_eq!(gpu_errors_in(&odd, "").as_deref(), Some("Queue::submit (x1)"));
        // the log of a build that wrote no sources, at its very end
        assert_eq!(
            gpu_errors_in(&format!("{e}Out of Memory\n"), "").as_deref(),
            Some("out-of-memory (x1)")
        );
    }

    #[test]
    fn a_gpu_error_of_another_shape_is_other_and_other_modules_are_left_out() {
        let e = WGPU_ERROR;
        for (log, expected) in [
            // no operation before the next record, or before the end
            (
                format!("{e}Validation Error\n\nCaused by:\n  Something else\n[t INFO  a] b\n"),
                Some("other (x1)"),
            ),
            (format!("{e}Validation Error\n"), Some("other (x1)")),
            // not an operation of wgpu's
            (
                format!("{e}Validation Error\n\nCaused by:\n  In Alice::resident\n"),
                Some("other (x1)"),
            ),
            (
                format!("{e}Validation Error\n\nCaused by:\n  In a render pass\n"),
                Some("other (x1)"),
            ),
            (
                format!("{e}Validation Error\n\nCaused by:\n  In Queue::Write Place\n"),
                Some("other (x1)"),
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
    fn repeated_gpu_errors_are_counted_without_publishing_labels() {
        let log: String = ["a", "b", "c", "d", "e", "f", "g", "h"]
            .iter()
            .map(|m| format!("{WGPU_ERROR}Validation Error\n\nCaused by:\n  In Device::create_{m}\n"))
            .collect();
        let row = gpu_errors_in(&log, "").unwrap();
        assert_eq!(row.matches("Device::").count(), 8, "{row}");
        assert!(row.contains("Device::create_a (x1)"), "{row}");
        assert!(!row.contains("label"), "{row}");
        // ten at most, the first ones: the rest stays in the private file
        let log: String = (0..12)
            .map(|i| format!("{WGPU_ERROR}Validation Error\n\nCaused by:\n  In Device::create_{i}\n"))
            .collect();
        let row = gpu_errors_in(&log, "").unwrap();
        assert_eq!(row.matches("Device::").count(), 10, "{row}");
        assert!(row.starts_with("Device::create_0 (x1)") && row.ends_with(", …"), "{row}");
    }

    #[test]
    fn private_gpu_error_ring_deduplicates_caps_and_records_age_and_raw_label() {
        let stamp = "2026-11-04T10:00:00.000Z";
        let one = format!(
            "[{stamp} ERROR aurora_render::main_thread] wgpu error: Validation Error\n  In Queue::write_buffer, label = 'private label'\n"
        );
        let log = format!(
            "{one}[2026-11-04T10:00:05.000Z ERROR aurora_render::main_thread] wgpu error: Validation Error\n  In Queue::write_buffer, label = 'private label'\n(repeated 5 times in the last 10 s)\n"
        );
        let end = chrono::DateTime::parse_from_rfc3339(stamp).unwrap().timestamp() as u64 + 10;
        let ring = gpu_error_ring(&log, Some(end));
        assert_eq!(ring.len(), 1);
        // the first, then a record standing for 5
        assert!(ring[0].contains("operation=Queue::write_buffer count=6 age=5s"), "{}", ring[0]);
        assert!(ring[0].contains("private label"), "{}", ring[0]);

        let many = (0..55)
            .map(|i| format!("[{stamp} ERROR aurora_render::main_thread] wgpu error: Validation Error\n  In Device::create_{i}\n"))
            .collect::<String>();
        let capped = gpu_error_ring(&many, Some(end));
        assert_eq!(capped.len(), 50);
        assert!(!capped.iter().any(|entry| entry.contains("Device::create_0")));
    }

    #[test]
    fn risk_factors_are_ordered_by_weight_and_effect_labels_are_fixed() {
        let risk = risk_factors(
            Some("contact-shadows,ssr"),
            Some(123),
            Some(6),
            Some(4),
            Some("high"),
            Some("AMD driver 25.10".into()),
            true,
        );
        assert!(
            risk.starts_with("contact shadows · video-memory headroom 6% · SSR · overlay hook recorded"),
            "{risk}"
        );
        assert!(risk.contains("session 123s (not included:"), "{risk}");
        // a list, no rating: the weights are a guess, not a measure
        for rating in ["High", "Medium", "Low"] {
            assert!(!risk.contains(rating), "{risk}");
        }
        assert!(risk_factors(None, None, None, None, None, None, false).starts_with("none known"));
    }

    #[test]
    fn video_memory_headroom_is_computed_from_the_existing_state_sample() {
        assert_eq!(vram_headroom("940/1000"), Some(6));
        assert_eq!(vram_headroom("100/0"), None);
        assert_eq!(vram_headroom("100/x"), None);
    }

    #[test]
    fn driver_risk_label_keeps_only_known_vendor_and_numeric_version() {
        assert_eq!(
            driver_version("NVIDIA GeForce RTX 4070 (Vulkan, driver NVIDIA 581.57)"),
            Some("NVIDIA driver 581.57".into())
        );
        assert_eq!(
            driver_version("AMD Radeon (Vulkan, driver Mesa 25.2.0 (private path))"),
            Some("AMD driver 25.2.0".into())
        );
        // a distribution's suffix (#2482's report)
        assert_eq!(
            driver_version("AMD Radeon RX 9070 XT (RADV GFX1201) (Vulkan, driver radv Mesa 26.2.5-arch3.1)"),
            Some("AMD driver 26.2.5".into())
        );
        assert_eq!(driver_version("Unknown hardware"), None);
    }

    #[test]
    fn a_gpu_loss_with_no_gpu_error_says_the_log_had_none() {
        let (dir, record) = session();
        std::fs::write(dir.join("alive.txt"), "tick=1098\nscreen=world\n").unwrap();
        // another crash: nothing to say, no row
        assert!(!build_at(&record, &dir, Some(10)).body.contains("| GPU errors |"));
        std::fs::write(dir.join("gpu-lost.txt"), "").unwrap();
        let body = build_at(&record, &dir, Some(10)).body;
        assert!(body.contains("| GPU errors | none in the retained session log |"), "{body}");
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

    #[test]
    fn unexpected_device_loss_has_human_wording_but_keeps_the_saved_key() {
        let (dir, record) = session();
        std::fs::write(dir.join("alive.txt"), "tick=1098\nloss_cause=other\n").unwrap();
        let body = build_at(&record, &dir, Some(10)).body;
        assert!(body.contains("unexpected Vulkan result (code not kept by wgpu-hal)"), "{body}");
        assert!(!body.contains("| GPU loss: what the message is about | other |"), "{body}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
