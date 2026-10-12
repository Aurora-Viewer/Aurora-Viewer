//! Guard of the interface rules (AGENTS.md, section 7): the sources of the
//! interface are read by a test, which fails when a window uses one of
//! egui's stock widgets that Aurora replaced by a shared one. A new window
//! then takes the theme from `ui/widgets.rs` instead of styling itself.
//!
//! The scan is line-based and simple on purpose: comment lines, test files
//! (`tests.rs`, `*_tests.rs`) and what follows a `#[cfg(test)] mod … {` are
//! left out; a file that must use a stock widget (the shared widgets wrap
//! egui's) is listed in `ALLOWED` with its reason.

use std::path::{Path, PathBuf};

/// A stock widget and what to use instead.
struct Rule {
    pattern: &'static str,
    instead: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        pattern: "egui::ProgressBar",
        instead: "widgets::progress (it glides to its value), or widgets::level for a meter",
    },
    Rule {
        pattern: "ui.checkbox(",
        instead: "widgets::checkbox, or widgets::switch_labeled for a lone setting",
    },
    Rule {
        pattern: "egui::Checkbox::new",
        instead: "widgets::checkbox, or widgets::switch_labeled for a lone setting",
    },
    Rule {
        pattern: "egui::CollapsingHeader",
        instead: "widgets::collapsing",
    },
    Rule {
        pattern: "color_edit_button",
        instead: "widgets::color_button",
    },
    Rule {
        pattern: "egui::Window::new",
        instead: "widgets::Floater (widgets::Modal for a dialog, widgets::bare_window without a title bar)",
    },
    Rule {
        pattern: "Frame::window(",
        instead: "widgets::Floater, or widgets::overlay / widgets::overlay_frame for a panel",
    },
    Rule {
        pattern: "Frame::group(",
        instead: "widgets::surface, or widgets::overlay for a panel over the world",
    },
    Rule {
        pattern: "ui.group(",
        instead: "widgets::surface, or widgets::overlay for a panel over the world",
    },
];

/// (file under `src/`, pattern it may use, why).
const ALLOWED: &[(&str, &str, &str)] = &[(
    "ui/widgets.rs",
    "egui::Window::new",
    "the shared windows themselves: Floater and bare_window wrap egui's window",
)];

/// What is scanned, under `src/`.
const ROOTS: &[&str] = &["ui", "build/ui", "crash/window.rs"];

fn sources(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
            .expect("a source directory can be read")
            .map(|entry| entry.expect("a directory entry").path())
            .collect();
        entries.sort();
        entries.iter().for_each(|entry| sources(entry, out));
    } else if path.extension().is_some_and(|e| e == "rs") {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        // test files, and this one (it names every pattern)
        if name != "tests.rs" && !name.ends_with("_tests.rs") && name != "theme_guard.rs" {
            out.push(path.to_owned());
        }
    }
}

/// The lines of `source` a window is made of: not the comment lines, and
/// nothing from the test module that ends the file.
fn code_lines(source: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut lines = source.lines().enumerate().peekable();
    while let Some((index, line)) = lines.next() {
        let text = line.trim_start();
        if text.starts_with("#[cfg(test)]") {
            let next = lines.peek().map_or("", |(_, next)| next.trim());
            let module = next.strip_prefix("pub(crate) ").or(next.strip_prefix("pub ")).unwrap_or(next);
            if module.starts_with("mod ") && module.ends_with('{') {
                break;
            }
        }
        if !text.starts_with("//") {
            out.push((index + 1, line));
        }
    }
    out
}

/// "file:line: pattern → replacement" for each stock widget found in `source`.
fn violations(file: &str, source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (number, line) in code_lines(source) {
        for rule in RULES {
            let allowed = ALLOWED.iter().any(|(path, pattern, _)| *path == file && *pattern == rule.pattern);
            if line.contains(rule.pattern) && !allowed {
                found.push(format!("  src/{file}:{number}: `{}` → use {}", rule.pattern, rule.instead));
            }
        }
    }
    found
}

#[test]
fn windows_use_the_shared_widgets() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for root in ROOTS {
        let path = src.join(root);
        assert!(path.exists(), "{} is gone: update ROOTS", path.display());
        sources(&path, &mut files);
    }
    assert!(files.len() > 50, "only {} files scanned", files.len());
    let mut found = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).expect("a source file is UTF-8 text");
        let file = path.strip_prefix(&src).expect("under src").to_string_lossy().replace('\\', "/");
        found.extend(violations(&file, &source));
    }
    assert!(
        found.is_empty(),
        "egui's stock widgets used directly in the interface (AGENTS.md, section 7 \"Interface et branding\": \
         the shared widgets of ui/widgets.rs carry the Aurora theme):\n{}\n\
         If a shared widget itself must wrap the stock one, add its file to ALLOWED in ui/theme_guard.rs with the reason.",
        found.join("\n")
    );
    // every exception still exists and is still needed
    for (file, pattern, why) in ALLOWED {
        let source = std::fs::read_to_string(src.join(file)).unwrap_or_default();
        assert!(
            code_lines(&source).iter().any(|(_, line)| line.contains(pattern)),
            "src/{file} no longer uses `{pattern}` ({why}): remove it from ALLOWED"
        );
    }
}

#[test]
fn the_scan_reads_code_and_skips_comments_and_tests() {
    let source = "\
fn window(ui: &mut egui::Ui) {
    // ui.checkbox(&mut on, \"never\") was here
    ui.checkbox(&mut on, \"label\");
    /// egui::ProgressBar in a doc comment
    ui.add(egui::ProgressBar::new(0.5));
}

#[cfg(test)]
fn helper() {
    let _ = egui::CollapsingHeader::new(\"still code\");
}

#[cfg(test)]
mod check_tests;

fn after() {
    egui::Frame::group(ui.style());
}

#[cfg(test)]
mod tests {
    fn stock() {
        ui.group(|ui| {});
        egui::Window::new(\"t\");
    }
}
";
    let found = violations("ui/example.rs", source);
    let lines: Vec<&str> = found.iter().map(|f| f.split(": ").next().unwrap_or_default().trim()).collect();
    assert_eq!(
        lines,
        [
            "src/ui/example.rs:3",
            "src/ui/example.rs:5",
            "src/ui/example.rs:10",
            "src/ui/example.rs:17"
        ],
        "{found:#?}"
    );
    assert!(found[0].contains("widgets::checkbox"), "{}", found[0]);
    assert!(found[1].contains("widgets::progress"), "{}", found[1]);
    // the exception is for its file and its pattern only
    assert!(violations("ui/widgets.rs", "egui::Window::new(title)").is_empty());
    assert_eq!(violations("ui/widgets.rs", "ui.group(|ui| {})").len(), 1);
    assert_eq!(violations("ui/minimap.rs", "egui::Window::new(title)").len(), 1);
}
