//! Public diagnostics use a closed vocabulary. Network errors, panic text
//! and GPU labels can contain arbitrary resident-controlled text, paths or
//! credentials; replacing UUIDs/URLs alone cannot make those lines public.
//! Keep severity, the Rust module that wrote the line (a code identifier,
//! checked against that shape), its time before the end of the log and
//! recognized failure categories; never copy a free-form fragment. The
//! original remains in the private local log.

/// The most recent lines kept, after repeats are folded together.
const MAX_LINES: usize = 20;

pub fn excerpt(log: &str) -> String {
    let end = log.lines().rev().find_map(seconds_of_day);
    // (time of the latest occurrence, the line without its time, repeats)
    let mut entries: std::collections::VecDeque<(Option<u32>, String, usize)> = std::collections::VecDeque::new();
    for line in log.lines() {
        let level = if line.contains(" ERROR ") {
            "ERROR"
        } else if line.contains(" WARN ") {
            "WARN"
        } else {
            continue;
        };
        let lower = line.to_ascii_lowercase();
        // Even a diagnostic attached to a chat/IM line must not be exported.
        if [
            "chat",
            "instantmessage",
            "instant message",
            "password",
            "token",
            "session_id",
            "secure_session",
            "authorization",
            "cookie",
        ]
        .iter()
        .any(|word| lower.contains(word))
        {
            continue;
        }
        let subsystem = if lower.contains("wgpu") || lower.contains("aurora_render") {
            "graphics"
        } else if lower.contains("aurora_net") {
            "network"
        } else if lower.contains("aurora_media") {
            "media"
        } else if lower.contains("aurora_voice") {
            "voice"
        } else if lower.contains("aurora_audio") {
            "audio"
        } else {
            "viewer"
        };
        let mut categories = Vec::new();
        for (needles, category) in [
            (&["panic:", "panicked"][..], "Rust panic"),
            (&["native exception"][..], "native exception"),
            (&["validation error", "validation failed"][..], "GPU validation error"),
            (&["out of memory", "outofmemory"][..], "out of memory"),
            (&["device lost", "devicelost"][..], "GPU device lost"),
            (&["did not answer for"][..], "window frozen"),
            (&["texture memory: pressure"][..], "texture memory pressure"),
            (&["overlay hook loaded"][..], "overlay hook loaded"),
            (&["timeout", "timed out"][..], "timeout"),
            (&["connection refused", "connection reset"][..], "connection failure"),
            (&["decode", "decoding"][..], "asset decoding"),
            (&["not found", "missing"][..], "resource unavailable"),
            (&["permission denied", "access denied"][..], "access denied"),
        ] {
            if needles.iter().any(|needle| lower.contains(needle)) {
                categories.push(category);
            }
        }
        if categories.is_empty() {
            categories.push("additional diagnostic");
        }
        let text = format!("{level} | {} | {}", target(line).unwrap_or(subsystem), categories.join("; "));
        let at = seconds_of_day(line);
        match entries.back_mut() {
            Some(last) if last.1 == text => {
                last.0 = at;
                last.2 += 1;
            }
            _ => entries.push_back((at, text, 1)),
        }
        if entries.len() > MAX_LINES {
            entries.pop_front();
        }
    }
    entries
        .into_iter()
        .map(|(at, text, count)| {
            let mut line = match (at, end) {
                // (across midnight, the log's clock wraps)
                (Some(at), Some(end)) => format!("-{} s | {text}", (i64::from(end) - i64::from(at)).rem_euclid(86_400)),
                _ => text,
            };
            if count > 1 {
                line.push_str(&format!(" (x{count})"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The record's module, as env_logger writes it (`[time LEVEL target] …`),
/// only when it has the shape of a Rust path.
fn target(line: &str) -> Option<&str> {
    let header = line.strip_prefix('[')?.split_once("] ")?.0;
    let target = header.split_whitespace().nth(2)?;
    (target.len() <= 80
        && target
            .split("::")
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')))
    .then_some(target)
}

/// The record's time of day in seconds (`2026-10-25T10:28:12.123Z`).
fn seconds_of_day(line: &str) -> Option<u32> {
    let stamp = line.strip_prefix('[')?.split_whitespace().next()?;
    let time = stamp.split_once('T')?.1.get(..8)?;
    let mut parts = time.split(':').map(|p| p.parse::<u32>().ok());
    let (h, m, s) = (parts.next()??, parts.next()??, parts.next()??);
    (h < 24 && m < 60 && s < 61).then_some(h * 3600 + m * 60 + s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_external_text_can_leak_through_diagnostic_messages() {
        let log = concat!(
            "[time WARN aurora_net] timeout https://sim.example/cap/secret?q=credential Alice Resident C:\\Users\\Private Name\\log\n",
            "[time ERROR wgpu_core] Validation Error: object `Élodie` /home/secret/file mail@private.example 123.45.67.89\n",
            "[time ERROR aurora_viewer] panic: secret multiword name 01234567-89ab-cdef-0123-456789abcdef\n",
            "[time WARN aurora_net] chat Alice Resident: confidential conversation\n",
            "[time ERROR aurora_net] Authorization: Bearer private_token\n",
            "[time INFO aurora_viewer] nothing\n",
            "[time WARN Alice_Resident] a target that is no module path\n"
        );
        let output = excerpt(log);
        assert_eq!(output.lines().count(), 4);
        assert!(output.contains("WARN | aurora_net | timeout"));
        assert!(output.contains("ERROR | wgpu_core | GPU validation error"));
        assert!(output.contains("Rust panic"));
        assert!(output.contains("WARN | viewer | additional diagnostic"));
        for secret in [
            "Alice",
            "Resident",
            "secret",
            "credential",
            "Users",
            "Élodie",
            "mail@",
            "123.45",
            "01234567",
            "confidential",
            "Bearer",
        ] {
            assert!(!output.contains(secret), "leaked {secret}");
        }
    }

    #[test]
    fn lines_say_how_long_before_the_end_and_repeats_fold() {
        let log = concat!(
            "[2026-10-25T23:59:50.100Z WARN  aurora_viewer::scene::textures] texture memory: pressure 1 -> 2\n",
            "[2026-10-25T23:59:58.000Z WARN  aurora_render::main_thread] wgpu error: Validation Error: x\n",
            "[2026-10-25T23:59:59.000Z WARN  aurora_render::main_thread] wgpu error: Validation Error: y\n",
            "[2026-10-26T00:00:04.000Z ERROR aurora_render::main_thread] GPU device lost (Unknown): reset\n",
            "[2026-10-26T00:00:05.000Z INFO  aurora_viewer] last line\n",
        );
        assert_eq!(
            excerpt(log),
            "-15 s | WARN | aurora_viewer::scene::textures | texture memory pressure\n\
             -6 s | WARN | aurora_render::main_thread | GPU validation error (x2)\n\
             -1 s | ERROR | aurora_render::main_thread | GPU device lost"
        );
    }

    #[test]
    fn bounded_recent_warnings_only() {
        let log: String = (0..100)
            .map(|i| format!("[now WARN aurora_net] {}\n", if i % 2 == 0 { "timeout" } else { "missing" }))
            .collect();
        assert_eq!(excerpt(&log).lines().count(), MAX_LINES);
        assert!(excerpt("private unstructured line\nbacktrace with names").is_empty());
    }
}
