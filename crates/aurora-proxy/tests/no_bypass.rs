//! No request leaves around the proxy rule: production code of every crate
//! builds its HTTP clients with `aurora_proxy::http_client` and opens no
//! outgoing socket of its own, except the reviewed ones below. Test code
//! (`#[cfg(test)]` items, `tests.rs`, `*_tests.rs`, `tests/`) is free.
//!
//! A line that is a reviewed exception carries `proxy-guard:` and the reason,
//! at its end or in the comment lines right above it.

use std::path::{Path, PathBuf};

/// What builds a client or a socket around the rule.
const FORBIDDEN: &[&str] = &[
    "Client::builder()",
    "Client::new()",
    "ClientBuilder::new()",
    "Client::default()",
    "reqwest::get(",
    "reqwest::blocking",
    ".no_proxy()",
    ".proxy(",
    "TcpStream::connect",
    "UdpSocket::bind",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !matches!(name.as_str(), "tests" | "target" | "fixtures" | "benches" | "examples") {
                rust_files(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") && !name.starts_with("test_") {
            out.push(path);
        }
    }
}

/// The source without its comments and string contents (kept as blanks, so
/// lines keep their numbers), for brace counting.
fn code_only(src: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut chars = src.chars().peekable();
    let mut block = 0usize;
    while let Some(c) = chars.next() {
        if c == '\n' {
            lines.push(std::mem::take(&mut line));
            continue;
        }
        if block > 0 {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block -= 1;
            } else if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                block += 1;
            }
            continue;
        }
        match c {
            '/' if chars.peek() == Some(&'/') => {
                // the rest of the line is a comment
                while chars.peek().is_some_and(|&n| n != '\n') {
                    chars.next();
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                block = 1;
            }
            'r' if matches!(chars.peek(), Some('#') | Some('"')) && !line.ends_with(|p: char| p.is_alphanumeric() || p == '_') => {
                let mut hashes = 0;
                while chars.peek() == Some(&'#') {
                    chars.next();
                    hashes += 1;
                }
                if chars.peek() != Some(&'"') {
                    line.push('r');
                    line.push_str(&"#".repeat(hashes));
                    continue;
                }
                chars.next();
                line.push_str("\"\"");
                'raw: while let Some(n) = chars.next() {
                    if n == '\n' {
                        lines.push(std::mem::take(&mut line));
                    } else if n == '"' {
                        let mut seen = 0;
                        while seen < hashes && chars.peek() == Some(&'#') {
                            chars.next();
                            seen += 1;
                        }
                        if seen == hashes {
                            break 'raw;
                        }
                    }
                }
            }
            '"' => {
                line.push_str("\"\"");
                while let Some(n) = chars.next() {
                    match n {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        '\n' => lines.push(std::mem::take(&mut line)),
                        _ => {}
                    }
                }
            }
            '\'' => {
                // a char literal ('{', '\'', '\u{7b}') or a lifetime
                let rest: String = chars.clone().take(12).collect();
                if let Some(end) = rest.find('\'').filter(|&e| e <= 10 && (e == 1 || rest.starts_with('\\'))) {
                    for _ in 0..=end {
                        chars.next();
                    }
                    line.push_str("' '");
                } else {
                    line.push('\'');
                }
            }
            _ => line.push(c),
        }
    }
    lines.push(line);
    lines
}

/// The line numbers (0-based) of the items under `#[cfg(test)]`.
fn test_lines(code: &[String]) -> Vec<bool> {
    let mut skip = vec![false; code.len()];
    let mut i = 0;
    while i < code.len() {
        if code[i].trim() == "#[cfg(test)]" {
            // the item that follows, to its closing brace (or its `;`)
            let mut depth = 0i64;
            let mut opened = false;
            let mut j = i;
            while j < code.len() {
                skip[j] = true;
                for c in code[j].chars() {
                    match c {
                        '{' => {
                            depth += 1;
                            opened = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if (opened && depth <= 0) || (!opened && j > i && code[j].trim_end().ends_with(';')) {
                    break;
                }
                j += 1;
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    skip
}

/// `pattern` in `line`, not as the end of a longer name
/// (`MediaDataClient::default()` is not `Client::default()`).
fn contains_word(line: &str, pattern: &str) -> bool {
    line.match_indices(pattern)
        .any(|(at, _)| pattern.starts_with('.') || !line[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
}

/// Line `n` carries the marker, or the `//` comment lines right above it do.
fn reviewed(raw: &[&str], n: usize) -> bool {
    if raw.get(n).is_some_and(|l| l.contains("proxy-guard:")) {
        return true;
    }
    raw[..n.min(raw.len())]
        .iter()
        .rev()
        .take_while(|l| l.trim_start().starts_with("//"))
        .any(|l| l.contains("proxy-guard:"))
}

#[test]
fn production_code_follows_the_proxy_rule() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("crates folder").flatten() {
        // the build gate is a compiler wrapper, not a part of the viewer;
        // aurora-proxy is the rule itself
        if entry.file_name() != "aurora-build-gate" && entry.file_name() != "aurora-proxy" {
            rust_files(&entry.path().join("src"), &mut files);
        }
    }
    assert!(files.len() > 100, "the crates' sources were not found");
    let mut found = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).expect("source");
        let raw: Vec<&str> = src.lines().collect();
        let code = code_only(&src);
        let skip = test_lines(&code);
        for (n, line) in code.iter().enumerate() {
            if skip.get(n).copied().unwrap_or(false) {
                continue;
            }
            let Some(pattern) = FORBIDDEN.iter().find(|p| contains_word(line, p)) else {
                continue;
            };
            if reviewed(&raw, n) {
                continue;
            }
            found.push(format!("{}:{}: {pattern}", path.display(), n + 1));
        }
    }
    assert!(
        found.is_empty(),
        "build HTTP clients with aurora_proxy::http_client and sockets through the proxy rule, \
         or mark a reviewed exception with `// proxy-guard: <reason>`:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_scanner_sees_through_comments_strings_and_tests() {
    let src = "fn a() {\n    // Client::new()\n    let s = \"Client::new()\";\n    let c = '{';\n}\n#[cfg(test)]\nmod tests {\n    fn b() { Client::new(); }\n}\nfn c() { Client::new(); }\n";
    let code = code_only(src);
    let skip = test_lines(&code);
    let hits: Vec<usize> = code
        .iter()
        .enumerate()
        .filter(|(n, l)| !skip[*n] && l.contains("Client::new()"))
        .map(|(n, _)| n + 1)
        .collect();
    assert_eq!(hits, vec![10]);
    assert!(!contains_word("MediaDataClient::default()", "Client::default()"));
    // the marker in the comment block right above, not further up
    let raw = ["// proxy-guard: why", "// more of why", "bind()", "", "bind()"];
    assert!(reviewed(&raw, 2));
    assert!(!reviewed(&raw, 4));
    assert!(contains_word("reqwest::Client::default()", "Client::default()"));
}
