//! Public diagnostics use a closed vocabulary. Network errors, panic text
//! and GPU labels can contain arbitrary resident-controlled text, paths or
//! credentials; replacing UUIDs/URLs alone cannot make those lines public.
//! Keep severity/subsystem and recognized failure categories, never copy a
//! free-form fragment. The original remains in the private local log.

pub fn excerpt(log: &str) -> String {
    let mut entries = std::collections::VecDeque::new();
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
        entries.push_back(format!(
            "{level} | {subsystem} | {} [free-form details removed]",
            categories.join("; ")
        ));
        if entries.len() > 20 {
            entries.pop_front();
        }
    }
    entries.into_iter().collect::<Vec<_>>().join("\n")
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
            "[time INFO aurora_viewer] nothing\n"
        );
        let output = excerpt(log);
        assert_eq!(output.lines().count(), 3);
        assert!(output.contains("timeout"));
        assert!(output.contains("GPU validation error"));
        assert!(output.contains("Rust panic"));
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
    fn bounded_recent_warnings_only() {
        let log = "[now WARN aurora_net] timeout\n".repeat(100);
        assert_eq!(excerpt(&log).lines().count(), 20);
        assert!(excerpt("private unstructured line\nbacktrace with names").is_empty());
    }
}
