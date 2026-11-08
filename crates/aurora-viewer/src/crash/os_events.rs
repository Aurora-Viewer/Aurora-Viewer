//! Best-effort OS GPU-reset records collected for a stopped session.
//! Raw kernel lines remain in the private crash folder; public reports use
//! only a fixed status and a count.

#[cfg(any(target_os = "linux", windows, test))]
use std::io::Read;
use std::path::Path;
#[cfg(any(target_os = "linux", windows, test))]
use std::process::{Child, Command, Output, Stdio};
#[cfg(any(target_os = "linux", windows, test))]
use std::time::{Duration, Instant};

#[cfg(any(target_os = "linux", windows))]
const COLLECTION_BUDGET: Duration = Duration::from_secs(2);
#[cfg(any(target_os = "linux", windows, test))]
const MAX_COMMAND_OUTPUT: u64 = 4 * 1024 * 1024;

/// Keep one look-ahead record so an exactly full result differs from a
/// truncated one. The public status must disclose discarded evidence.
#[cfg(any(target_os = "linux", windows, test))]
fn bounded_records(records: impl IntoIterator<Item = String>) -> (Vec<String>, bool) {
    let mut records = records.into_iter();
    let kept = records.by_ref().take(60).collect();
    (kept, records.next().is_some())
}

#[cfg(any(target_os = "linux", windows, test))]
fn output_before(command: &mut Command, deadline: Instant) -> std::io::Result<Output> {
    if Instant::now() >= deadline {
        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "OS diagnostics budget expired"));
    }
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    if let Err(error) = std::thread::Builder::new().name("aurora-os-read".into()).spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout {
            let mut stdout = stdout.take(MAX_COMMAND_OUTPUT);
            let _ = stdout.read_to_end(&mut bytes);
        }
        let _ = sender.send(bytes);
    }) {
        stop_child(child);
        return Err(error);
    }
    let status = match wait_until(&mut child, deadline) {
        Ok(status) => status,
        Err(error) => {
            stop_child(child);
            return Err(error);
        }
    };
    // Even a successfully exited utility can leave an inherited pipe open.
    // The same budget covers reading; never join that potentially stuck reader.
    let stdout = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "OS diagnostics output timed out")
            }
            std::sync::mpsc::RecvTimeoutError::Disconnected => std::io::Error::other("OS diagnostics reader stopped"),
        })?;
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

/// Cleanup is best effort under resource exhaustion: never wait or panic on
/// the report thread, including when a new reaper thread cannot be created.
#[cfg(any(target_os = "linux", windows, test))]
fn stop_child(mut child: Child) {
    let _ = child.kill();
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = std::thread::Builder::new().name("aurora-os-reap".into()).spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(any(target_os = "linux", windows, test))]
fn wait_until(child: &mut Child, deadline: Instant) -> std::io::Result<std::process::ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "OS diagnostics command timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(target_os = "linux")]
pub fn collect(dir: &Path, unix: Option<u64>) {
    let Some(unix) = unix else {
        let _ = std::fs::write(dir.join("os-gpu-events.status"), "not collected: crash time unavailable");
        return;
    };
    let since = unix.saturating_sub(120);
    let until = unix.saturating_add(300);
    let since_arg = format!("@{since}");
    let until_arg = format!("@{until}");
    let deadline = Instant::now() + COLLECTION_BUDGET;
    let result = output_before(
        std::process::Command::new("journalctl").args([
            // The kernel's messages of every boot in the window: `-k` would
            // keep the current boot only, and a GPU hang often needed a reboot.
            "_TRANSPORT=kernel",
            "--lines=1000",
            "--since",
            &since_arg,
            "--until",
            &until_arg,
            "--output=short-unix",
            "--no-pager",
        ]),
        deadline,
    );
    let Ok(output) = result else {
        let _ = std::fs::write(dir.join("os-gpu-events.status"), public_status(None));
        return;
    };
    let mut capped =
        output.stdout.len() >= MAX_COMMAND_OUTPUT as usize || output.stdout.iter().filter(|&&byte| byte == b'\n').count() >= 1000;
    let (lines, records_capped) = if output.status.success() {
        parse_journal(&String::from_utf8_lossy(&output.stdout))
    } else {
        let fallback = output_before(std::process::Command::new("dmesg").args(["--time-format=raw"]), deadline);
        let Ok(fallback) = fallback else {
            let _ = std::fs::write(dir.join("os-gpu-events.status"), public_status(None));
            return;
        };
        if !fallback.status.success() {
            let _ = std::fs::write(dir.join("os-gpu-events.status"), public_status(None));
            return;
        }
        capped |= fallback.stdout.len() >= MAX_COMMAND_OUTPUT as usize;
        parse_dmesg(
            &String::from_utf8_lossy(&fallback.stdout),
            sysinfo::System::boot_time(),
            since,
            until,
        )
    };
    let status = kernel_status(&lines, capped || records_capped);
    if lines.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-events.status"), status);
    } else {
        let _ = std::fs::write(dir.join("os-gpu-events.txt"), lines.join("\n"));
        let _ = std::fs::write(
            dir.join("os-gpu-events.status"),
            format!("{status} near the last heartbeat (approximate)"),
        );
    }
}

#[cfg(target_os = "linux")]
fn public_status(records: Option<&[String]>) -> String {
    match records {
        None => "not collected: kernel journal access denied or unavailable".into(),
        Some([]) => "collected: no matching kernel GPU records".into(),
        Some(lines) => format!("collected: {} matching kernel records", lines.len()),
    }
}

#[cfg(target_os = "linux")]
fn kernel_status(records: &[String], capped: bool) -> String {
    if capped {
        format!(
            "collected (incomplete): {} matching kernel records; query or retained-record limit reached",
            records.len()
        )
    } else {
        public_status(Some(records))
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn collect(_dir: &Path, _unix: Option<u64>) {}

#[cfg(windows)]
pub fn collect(dir: &Path, unix: Option<u64>) {
    let Some(unix) = unix else {
        let _ = std::fs::write(dir.join("os-gpu-events.status"), "not collected: crash time unavailable");
        return;
    };
    let since = unix.saturating_sub(120);
    let until = unix.saturating_add(300);
    let deadline = Instant::now() + COLLECTION_BUDGET;
    let mut events = Vec::new();
    let mut queried = 0u8;
    let mut capped = false;
    // The window in the query itself: WER files every kind of problem as
    // 1001, and the newest 1000 of them alone can miss a crash some time ago.
    let query = event_query(since, until);
    for channel in ["System", "Application"] {
        let result = output_before(
            std::process::Command::new(system_tool("wevtutil.exe")).args([
                "qe",
                channel,
                &query,
                "/rd:true",
                "/f:xml",
                // One root element around the events: without it, two or
                // more events are not one XML document and none parse.
                "/e:Events",
                // a cap all the same; the parser keeps the window too
                "/c:1000",
            ]),
            deadline,
        );
        if let Ok(output) = result
            && output.status.success()
        {
            queried += 1;
            let xml = String::from_utf8_lossy(&output.stdout);
            capped |= output.stdout.len() >= MAX_COMMAND_OUTPUT as usize || windows_event_count(&xml) >= 1000;
            let (records, records_capped) = parse_windows_events(&xml, since, until);
            capped |= records_capped;
            events.extend(records);
        }
    }
    let (events, records_capped) = bounded_records(events);
    capped |= records_capped;
    if !events.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-events.txt"), events.join("\n"));
    }
    let settings = windows_graphics_settings(deadline);
    if !settings.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-settings.txt"), settings.join("\n"));
    }
    let dumps = watchdog_dumps(since, until);
    if !dumps.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-dumps.txt"), dumps.join("\n"));
    }
    let status = windows_status(queried, events.len(), capped);
    let _ = std::fs::write(dir.join("os-gpu-events.status"), status);
}

/// The XPath of the GPU reset events between `since` and `until` (Unix
/// seconds): Display 4101 (a driver reset, System) and Windows Error
/// Reporting's 1001 (Application), which `parse_windows_events` narrows to
/// its LiveKernelEvent reports.
#[cfg(any(windows, test))]
fn event_query(since: u64, until: u64) -> String {
    let time = |unix: u64| {
        chrono::DateTime::from_timestamp(unix.min(i64::MAX as u64) as i64, 0)
            .unwrap_or_default()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };
    format!(
        "/q:*[System[(EventID=4101 or EventID=1001) and TimeCreated[@SystemTime>='{}' and @SystemTime<='{}']]]",
        time(since),
        time(until)
    )
}

/// A tool of Windows' own, by its full path: a bare name is searched in the
/// viewer's folder before System32.
#[cfg(windows)]
fn system_tool(name: &str) -> std::path::PathBuf {
    windows_root().join("System32").join(name)
}

#[cfg(windows)]
fn windows_root() -> std::path::PathBuf {
    std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
}

#[cfg(any(windows, test))]
fn windows_status(queried: u8, events: usize, capped: bool) -> String {
    if queried == 0 {
        "not collected: Windows event log query unavailable or denied".to_owned()
    } else if capped {
        format!(
            "collected (incomplete): {events} Windows GPU reset events; query or retained-record limit reached{}",
            if queried == 1 {
                " and the other channel was unavailable"
            } else {
                ""
            }
        )
    } else if queried == 1 {
        format!("collected (partial): {events} Windows GPU reset events; one event channel unavailable")
    } else {
        format!("collected: {events} Windows GPU reset events in the time window")
    }
}

#[cfg(any(windows, test))]
fn windows_event_count(xml: &str) -> usize {
    roxmltree::Document::parse(xml).map_or(0, |document| {
        document.descendants().filter(|node| node.tag_name().name() == "Event").count()
    })
}

#[cfg(any(windows, test))]
fn parse_windows_events(xml: &str, since: u64, until: u64) -> (Vec<String>, bool) {
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return (Vec::new(), false);
    };
    bounded_records(
        document
            .descendants()
            .filter(|node| node.tag_name().name() == "Event")
            .filter_map(|event| {
                let system = event.children().find(|node| node.tag_name().name() == "System")?;
                let id: u32 = system
                    .children()
                    .find(|node| node.tag_name().name() == "EventID")?
                    .text()?
                    .parse()
                    .ok()?;
                let provider = system
                    .children()
                    .find(|node| node.tag_name().name() == "Provider")?
                    .attribute("Name")?;
                let created = system
                    .children()
                    .find(|node| node.tag_name().name() == "TimeCreated")?
                    .attribute("SystemTime")?;
                let time = chrono::DateTime::parse_from_rfc3339(created).ok()?;
                let timestamp = time.timestamp();
                if timestamp < since as i64 || timestamp > until as i64 {
                    return None;
                }
                if id == 4101 && provider.eq_ignore_ascii_case("Display") {
                    return Some(format!("Display 4101 at {}", time.to_rfc3339()));
                }
                if id == 1001 && provider.to_ascii_lowercase().contains("error reporting") {
                    let mut data = event
                        .descendants()
                        .filter(|node| node.tag_name().name() == "Data")
                        .filter_map(|node| node.text());
                    // Windows Error Reporting files every kind of problem as
                    // 1001: only its LiveKernelEvent reports are GPU resets.
                    // The code is P1, after the event's name: the fields
                    // before it (the bucket) are numbers of their own.
                    data.find(|text| text.trim().eq_ignore_ascii_case("LiveKernelEvent"))?;
                    let code = data.find_map(live_kernel_code)?;
                    return Some(format!("LiveKernelEvent {code} at {}", time.to_rfc3339()));
                }
                None
            }),
    )
}

#[cfg(any(windows, test))]
fn live_kernel_code(text: &str) -> Option<&'static str> {
    match text.trim().strip_prefix("0x").unwrap_or(text.trim()).to_ascii_lowercase().as_str() {
        "141" => Some("141"),
        "117" => Some("117"),
        "1a8" => Some("1a8"),
        _ => None,
    }
}

#[cfg(windows)]
fn windows_graphics_settings(deadline: Instant) -> Vec<String> {
    let mut values = Vec::new();
    for (key, name) in [
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "TdrDelay"),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "TdrDdiDelay"),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "TdrLevel"),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "HwSchMode"),
        (r"HKLM\SOFTWARE\Microsoft\Windows\Dwm", "OverlayTestMode"),
    ] {
        let output = output_before(
            std::process::Command::new(system_tool("reg.exe")).args(["query", key, "/v", name]),
            deadline,
        );
        if let Ok(output) = output
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let value = stdout.lines().find(|line| line.contains(name)).unwrap_or("").trim();
            if !value.is_empty() {
                values.push(format!("{name}: {value}"));
            }
        }
    }
    values
}

#[cfg(windows)]
fn watchdog_dumps(since: u64, until: u64) -> Vec<String> {
    std::fs::read_dir(windows_root().join("LiveKernelReports").join("WATCHDOG"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            if !entry
                .path()
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("dmp"))
            {
                return None;
            }
            let metadata = entry.metadata().ok()?;
            let modified = metadata.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            if modified < since || modified > until {
                return None;
            }
            Some(format!("{} {modified}", entry.file_name().to_string_lossy()))
        })
        .take(60)
        .collect()
}

#[cfg(target_os = "linux")]
fn parse_journal(text: &str) -> (Vec<String>, bool) {
    bounded_records(
        text.lines()
            .filter(|line| gpu_kernel_line(line))
            .map(|line| line.chars().take(2048).collect()),
    )
}

#[cfg(target_os = "linux")]
fn parse_dmesg(text: &str, boot: u64, since: u64, until: u64) -> (Vec<String>, bool) {
    bounded_records(text.lines().filter_map(|line| {
        let rest = line.strip_prefix('[')?.trim_start();
        let (seconds, message) = rest.split_once(']')?;
        let time = seconds.trim().parse::<f64>().ok()?;
        let unix = boot.saturating_add(time as u64);
        (unix >= since && unix <= until && gpu_kernel_line(message)).then(|| line.chars().take(2048).collect())
    }))
}

/// A kernel line of a GPU driver's: one that names it. "timeout", "reset"
/// or "fault" alone would take USB, disk and network lines too, and
/// "default"; the public row counts these lines as GPU records.
#[cfg(target_os = "linux")]
fn gpu_kernel_line(line: &str) -> bool {
    const TERMS: [&str; 5] = ["amdgpu", "radeon", "nouveau", "nvidia", "i915"];
    // Short terms only as whole words: "xe" is in "exec".
    const WORDS: [&str; 3] = ["xe", "drm", "gpu"];
    let lower = line.to_ascii_lowercase();
    TERMS.iter().any(|term| lower.contains(term)) || lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| WORDS.contains(&word))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn kernel_filter_takes_short_terms_as_whole_words_only() {
        assert!(gpu_kernel_line("xe 0000:03:00.0: [drm] GT0: hang"));
        assert!(gpu_kernel_line("[drm:amdgpu_job_timedout] ring gfx_0.0.0 timeout"));
        assert!(!gpu_kernel_line("audit: exec of /usr/bin/string during boot"));
        // the words of a GPU reset, from another driver: not counted
        assert!(!gpu_kernel_line("usb 1-4: reset high-speed USB device number 3 using xhci_hcd"));
        assert!(!gpu_kernel_line("nvme nvme0: I/O 12 QID 3 timeout, aborting"));
        assert!(!gpu_kernel_line("cfg80211: loading the default regulatory fault rules"));
    }

    #[test]
    fn journal_parser_filters_gpu_lines_and_caps_count_and_length() {
        let text = "noise\nkernel: amdgpu ring timeout\n";
        assert_eq!(parse_journal(text).0, ["kernel: amdgpu ring timeout"]);
        let many = (0..80).map(|n| format!("kernel: drm reset {n}")).collect::<Vec<_>>().join("\n");
        let (records, capped) = parse_journal(&many);
        assert_eq!(records.len(), 60);
        assert!(capped);
        assert!(kernel_status(&records, capped).starts_with("collected (incomplete):"));
        assert!(!parse_journal(&many.lines().take(60).collect::<Vec<_>>().join("\n")).1);
        assert!(parse_journal(&"x".repeat(4096)).0.is_empty());
        assert_eq!(parse_journal(&format!("kernel: nvidia {}", "x".repeat(3000))).0[0].len(), 2048);
    }

    #[test]
    fn unavailable_kernel_access_is_reported_separately_from_no_events() {
        assert!(public_status(None).starts_with("not collected:"));
        assert!(public_status(Some(&[])).starts_with("collected:"));
        assert!(kernel_status(&[], true).starts_with("collected (incomplete):"));
    }

    #[test]
    fn dmesg_fallback_uses_the_boot_time_and_requested_window() {
        let text = "[ 100.0] amdgpu: ring timeout\n[ 200.0] drm: reset\n[ 100.0] unrelated\n";
        assert_eq!(parse_dmesg(text, 1_000, 1_090, 1_150).0, ["[ 100.0] amdgpu: ring timeout"]);
    }
}

#[cfg(test)]
mod windows_parser_tests {
    #[test]
    fn windows_event_fixtures_keep_only_gpu_event_names_and_times_in_range() {
        let xml = r#"<Events xmlns="http://schemas.microsoft.com/win/2004/08/events/event">
          <Event><System><Provider Name="Display"/><EventID>4101</EventID><TimeCreated SystemTime="2026-11-04T10:00:00.000Z"/></System><EventData><Data>private event contents</Data></EventData></Event>
          <Event><System><Provider Name="Windows Error Reporting"/><EventID>1001</EventID><TimeCreated SystemTime="2026-11-04T10:00:01.000Z"/></System><EventData><Data>LiveKernelEvent</Data><Data>141</Data></EventData></Event>
          <Event><System><Provider Name="Display"/><EventID>4101</EventID><TimeCreated SystemTime="2026-11-04T12:00:00.000Z"/></System></Event>
        </Events>"#;
        let start = chrono::DateTime::parse_from_rfc3339("2026-11-04T09:59:00Z").unwrap().timestamp() as u64;
        let end = chrono::DateTime::parse_from_rfc3339("2026-11-04T10:05:00Z").unwrap().timestamp() as u64;
        let (events, capped) = super::parse_windows_events(xml, start, end);
        assert!(!capped);
        assert_eq!(events.len(), 2);
        assert!(events[0].starts_with("Display 4101 at 2026-11-04T10:00:00"));
        assert!(events[1].starts_with("LiveKernelEvent 141 at 2026-11-04T10:00:01"));
        assert!(events.iter().all(|event| !event.contains("private event contents")));
    }

    #[test]
    fn the_event_query_asks_for_the_window_only() {
        let since = chrono::DateTime::parse_from_rfc3339("2026-11-04T09:58:00Z").unwrap().timestamp() as u64;
        assert_eq!(
            super::event_query(since, since + 420),
            "/q:*[System[(EventID=4101 or EventID=1001) and TimeCreated[@SystemTime>='2026-11-04T09:58:00.000Z' and @SystemTime<='2026-11-04T10:05:00.000Z']]]"
        );
    }

    #[test]
    fn a_live_kernel_event_code_is_read_after_the_event_name() {
        // a bucket that happens to read 141 is not the code
        let xml = r#"<Events><Event><System><Provider Name="Windows Error Reporting"/><EventID>1001</EventID><TimeCreated SystemTime="2026-11-04T10:00:01.000Z"/></System><EventData><Data>141</Data><Data>0</Data><Data>LiveKernelEvent</Data><Data>Not available</Data><Data>0</Data><Data>117</Data></EventData></Event></Events>"#;
        let (events, _) = super::parse_windows_events(xml, 0, i64::MAX as u64);
        assert_eq!(events.len(), 1);
        assert!(events[0].starts_with("LiveKernelEvent 117 at "), "{events:?}");
    }

    #[test]
    fn malformed_windows_event_xml_is_not_collected_as_a_record() {
        assert!(super::parse_windows_events("<Event><System>", 0, u64::MAX).0.is_empty());
    }

    #[test]
    fn windows_event_status_does_not_claim_unverified_registry_or_dump_samples() {
        let status = super::windows_status(2, 0, false);
        assert!(!status.contains("registry"));
        assert!(!status.contains("dump"));
    }

    #[test]
    fn matching_windows_records_disclose_truncation_below_the_query_cap() {
        let event = r#"<Event><System><Provider Name="Display"/><EventID>4101</EventID><TimeCreated SystemTime="2026-11-04T10:00:00Z"/></System></Event>"#;
        for count in [60, 61] {
            let xml = format!("<Events>{}</Events>", event.repeat(count));
            let (records, capped) = super::parse_windows_events(&xml, 0, i64::MAX as u64);
            assert_eq!(records.len(), 60);
            assert_eq!(capped, count > 60);
            assert_eq!(
                super::windows_status(2, records.len(), capped).starts_with("collected (incomplete):"),
                count > 60
            );
        }
        let (combined, capped) = super::bounded_records((0..80).map(|n| n.to_string()));
        assert_eq!(combined.len(), 60);
        assert!(capped, "the combined channels have the same report limit");
    }

    #[test]
    fn capped_windows_query_is_inconclusive() {
        let events = (0..1000).map(|_| "<Event/>").collect::<String>();
        let xml = format!("<Events>{events}</Events>");
        assert_eq!(super::windows_event_count(&xml), 1000);
        assert!(super::windows_event_count("<Events><Event/></Events>") < 1000);
        assert!(super::windows_status(2, 0, true).starts_with("collected (incomplete):"));
        assert!(super::windows_status(1, 2, false).starts_with("collected (partial):"));
    }
}

#[cfg(all(test, unix))]
mod timeout_tests {
    use super::*;

    #[test]
    fn inherited_stdout_cannot_extend_the_collection_deadline() {
        for script in ["sleep 2 & exit 0", "sleep 2 & wait"] {
            let started = Instant::now();
            let result = output_before(
                std::process::Command::new("sh").args(["-c", script]),
                started + Duration::from_millis(50),
            );
            assert!(started.elapsed() < Duration::from_secs(1), "{script}");
            assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        }
    }

    #[test]
    fn os_command_is_killed_at_its_deadline() {
        let started = Instant::now();
        let result = output_before(
            std::process::Command::new("sh").args(["-c", "exec sleep 5"]),
            started + Duration::from_millis(50),
        );
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
