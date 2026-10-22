//! Display of the public diagnostics. The excerpt itself, with its closed
//! vocabulary, is `crash::privacy::excerpt`, shared with crash reports.

/// Translate only our closed vocabulary for display; the public diagnostic
/// data and the canonical exported report are not rewritten.
pub fn display_excerpt(excerpt: &str) -> String {
    let mut display = excerpt.to_owned();
    for (word, key) in [
        ("[free-form details removed]", "bug-log-redacted"),
        ("additional diagnostic", "bug-log-additional"),
        ("GPU validation error", "bug-log-validation"),
        ("GPU device lost", "bug-log-device-lost"),
        ("Rust panic", "bug-log-panic"),
        ("native exception", "bug-log-native"),
        ("out of memory", "bug-log-memory"),
        ("connection failure", "bug-log-connection"),
        ("asset decoding", "bug-log-decoding"),
        ("resource unavailable", "bug-log-unavailable"),
        ("access denied", "bug-log-denied"),
        ("timeout", "bug-log-timeout"),
        ("graphics", "bug-log-graphics"),
        ("network", "bug-log-network"),
        ("media", "bug-log-media"),
        ("voice", "bug-log-voice"),
        ("viewer", "bug-log-viewer"),
        ("WARN", "bug-log-warning"),
        ("ERROR", "bug-log-error"),
    ] {
        display = display.replace(word, aurora_i18n::text(key));
    }
    display
}
