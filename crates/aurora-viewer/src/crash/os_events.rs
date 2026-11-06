//! Best-effort OS GPU-reset records collected for a stopped session.
//! Raw kernel lines remain in the private crash folder; public reports use
//! only a fixed status and a count.

use std::path::Path;

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
    let result = std::process::Command::new("journalctl")
        .args([
            "-k",
            "--lines=1000",
            "--since",
            &since_arg,
            "--until",
            &until_arg,
            "--output=short-unix",
            "--no-pager",
        ])
        .output();
    let Ok(output) = result else {
        let _ = std::fs::write(dir.join("os-gpu-events.status"), public_status(None));
        return;
    };
    let lines = if output.status.success() {
        parse_journal(&String::from_utf8_lossy(&output.stdout))
    } else {
        let fallback = std::process::Command::new("dmesg").args(["--time-format=raw"]).output();
        let Ok(fallback) = fallback else {
            let _ = std::fs::write(dir.join("os-gpu-events.status"), public_status(None));
            return;
        };
        if !fallback.status.success() {
            let _ = std::fs::write(dir.join("os-gpu-events.status"), public_status(None));
            return;
        }
        parse_dmesg(
            &String::from_utf8_lossy(&fallback.stdout),
            sysinfo::System::boot_time(),
            since,
            until,
        )
    };
    if lines.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-events.status"), public_status(Some(&lines)));
    } else {
        let _ = std::fs::write(dir.join("os-gpu-events.txt"), lines.join("\n"));
        let _ = std::fs::write(
            dir.join("os-gpu-events.status"),
            format!("{} near the last heartbeat (approximate)", public_status(Some(&lines))),
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
    let mut events = Vec::new();
    let mut queried = 0u8;
    for channel in ["System", "Application"] {
        let result = std::process::Command::new("wevtutil.exe")
            .args([
                "qe",
                channel,
                "/q:*[System[(EventID=4101 or EventID=1001)]]",
                "/rd:true",
                "/f:xml",
                // Keep enough recent records to cover the recovery window
                // even on busy systems; the parser filters to the time span.
                "/c:1000",
            ])
            .output();
        if let Ok(output) = result
            && output.status.success()
        {
            queried += 1;
            events.extend(parse_windows_events(&String::from_utf8_lossy(&output.stdout), since, until));
        }
    }
    if !events.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-events.txt"), events.join("\n"));
    }
    let settings = windows_graphics_settings();
    if !settings.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-settings.txt"), settings.join("\n"));
    }
    let dumps = watchdog_dumps(since, until);
    if !dumps.is_empty() {
        let _ = std::fs::write(dir.join("os-gpu-dumps.txt"), dumps.join("\n"));
    }
    let status = if queried == 0 {
        "not collected: Windows event log query unavailable or denied".to_owned()
    } else if queried == 1 {
        format!(
            "collected (partial): {} Windows GPU reset events; one event channel unavailable",
            events.len()
        )
    } else {
        format!(
            "collected: {} Windows GPU reset events in the time window; registry and dump metadata sampled",
            events.len()
        )
    };
    let _ = std::fs::write(dir.join("os-gpu-events.status"), status);
}

#[cfg(any(windows, test))]
fn parse_windows_events(xml: &str, since: u64, until: u64) -> Vec<String> {
    let Ok(document) = roxmltree::Document::parse(xml) else {
        return Vec::new();
    };
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
                let code = event
                    .descendants()
                    .filter(|node| node.tag_name().name() == "Data")
                    .filter_map(|node| node.text())
                    .find_map(live_kernel_code)?;
                return Some(format!("LiveKernelEvent {code} at {}", time.to_rfc3339()));
            }
            None
        })
        .take(60)
        .collect()
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
        let events = super::parse_windows_events(xml, start, end);
        assert_eq!(events.len(), 2);
        assert!(events[0].starts_with("Display 4101 at 2026-11-04T10:00:00"));
        assert!(events[1].starts_with("LiveKernelEvent 141 at 2026-11-04T10:00:01"));
        assert!(events.iter().all(|event| !event.contains("private event contents")));
    }

    #[test]
    fn malformed_windows_event_xml_is_not_collected_as_a_record() {
        assert!(super::parse_windows_events("<Event><System>", 0, u64::MAX).is_empty());
    }
}

#[cfg(windows)]
fn windows_graphics_settings() -> Vec<String> {
    let mut values = Vec::new();
    for (key, name) in [
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "TdrDelay"),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "TdrDdiDelay"),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "TdrLevel"),
        (r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "HwSchMode"),
        (r"HKLM\SOFTWARE\Microsoft\Windows\Dwm", "OverlayTestMode"),
    ] {
        let output = std::process::Command::new("reg.exe").args(["query", key, "/v", name]).output();
        if let Ok(output) = output
            && output.status.success()
        {
            let value = String::from_utf8_lossy(&output.stdout)
                .lines()
                .find(|line| line.contains(name))
                .unwrap_or("")
                .trim();
            if !value.is_empty() {
                values.push(format!("{name}: {value}"));
            }
        }
    }
    values
}

#[cfg(windows)]
fn watchdog_dumps(since: u64, until: u64) -> Vec<String> {
    let root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    std::fs::read_dir(root.join("LiveKernelReports").join("WATCHDOG"))
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
fn parse_journal(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| gpu_kernel_line(line))
        .take(60)
        .map(|line| line.chars().take(2048).collect())
        .collect()
}

#[cfg(target_os = "linux")]
fn parse_dmesg(text: &str, boot: u64, since: u64, until: u64) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?.trim_start();
            let (seconds, message) = rest.split_once(']')?;
            let time = seconds.trim().parse::<f64>().ok()?;
            let unix = boot.saturating_add(time as u64);
            (unix >= since && unix <= until && gpu_kernel_line(message)).then(|| line.chars().take(2048).collect())
        })
        .take(60)
        .collect()
}

#[cfg(target_os = "linux")]
fn gpu_kernel_line(line: &str) -> bool {
    const TERMS: [&str; 12] = [
        "amdgpu", "radeon", "nouveau", "nvidia", "i915", "xe", "drm", "gpu", "ring", "timeout", "reset", "fault",
    ];
    let lower = line.to_ascii_lowercase();
    TERMS.iter().any(|term| lower.contains(term))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn journal_parser_filters_gpu_lines_and_caps_count_and_length() {
        let text = "noise\nkernel: amdgpu ring timeout\n";
        assert_eq!(parse_journal(text), ["kernel: amdgpu ring timeout"]);
        let many = (0..80).map(|n| format!("kernel: drm reset {n}")).collect::<Vec<_>>().join("\n");
        assert_eq!(parse_journal(&many).len(), 60);
        assert!(parse_journal(&"x".repeat(4096)).is_empty());
        assert_eq!(parse_journal(&format!("kernel: nvidia {}", "x".repeat(3000)))[0].len(), 2048);
    }

    #[test]
    fn unavailable_kernel_access_is_reported_separately_from_no_events() {
        assert!(public_status(None).starts_with("not collected:"));
        assert!(public_status(Some(&[])).starts_with("collected:"));
    }

    #[test]
    fn dmesg_fallback_uses_the_boot_time_and_requested_window() {
        let text = "[ 100.0] amdgpu: ring timeout\n[ 200.0] drm: reset\n[ 100.0] unrelated\n";
        assert_eq!(parse_dmesg(text, 1_000, 1_090, 1_150), ["[ 100.0] amdgpu: ring timeout"]);
    }
}
