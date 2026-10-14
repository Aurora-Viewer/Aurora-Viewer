//! Guard of the interface rules (AGENTS.md, section 7): the sources of the
//! interface are read by a test, which fails when a window styles itself
//! instead of taking the Aurora theme:
//!
//! - one of egui's stock widgets that Aurora replaced by a shared one of
//!   `ui/widgets.rs` (check box, radio button, separator, window…);
//! - a scroll style of its own (egui's floating bar shows over the
//!   content), when the theme reserves a strip for every bar;
//! - a value of appearance written in a window: a radius or a color given as
//!   a number or a named constant, when the theme has the radii
//!   (`ui.visuals()`) and the `Palette` has the colors.
//!
//! The scan is line-based and simple on purpose: comment lines, test files
//! (`tests.rs`, `*_tests.rs`) and what follows a `#[cfg(test)] mod … {` are
//! left out. Two ways to allow what is legitimate, both with a reason, and
//! both failing the test once they are no longer needed:
//!
//! - a file whose job it is (the shared widgets wrap egui's, the maps paint
//!   with the colors of the map) is listed in `ALLOWED` with the rule;
//! - one line that is content rather than chrome (a legend color, a fixed
//!   badge color) carries a marker comment, `// theme-guard: content — the
//!   reason`, at its end or alone on the line above. Alone on its line, the
//!   marker covers the lines that follow it and need it (the rows of a table
//!   of colors). Only the rules about values of appearance take a marker: a
//!   stock widget is never content.

use std::path::{Path, PathBuf};

/// What a rule looks for in a line.
enum Find {
    /// This text.
    Text(&'static str),
    /// A call opened by this text whose radius, the argument at this index,
    /// is a number other than zero (square by design).
    Radius(&'static str, usize),
    /// A color opened by this text, with a number among its red, green and
    /// blue (a color built from the components of another one is not).
    Components(&'static str),
    /// A named color of egui (`Color32::WHITE`…), other than the two that
    /// say "no color".
    NamedColor,
}

struct Rule {
    /// How `ALLOWED` and the failure message call it.
    name: &'static str,
    find: Find,
    instead: &'static str,
}

const RADIUS: &str = "literal radius";
const COLOR: &str = "literal color";

const fn stock(pattern: &'static str, instead: &'static str) -> Rule {
    Rule {
        name: pattern,
        find: Find::Text(pattern),
        instead,
    }
}

const fn radius(call: &'static str, argument: usize) -> Rule {
    Rule {
        name: RADIUS,
        find: Find::Radius(call, argument),
        instead: "the radius of the theme: ui.visuals().window_corner_radius (cards, pills, highlights, small chrome), \
                  ui.visuals().widgets.inactive.corner_radius (buttons), widgets::inner_radius (a small mark), widgets::TRACK_RADIUS (a thin bar)",
    }
}

const fn color(find: Find) -> Rule {
    Rule {
        name: COLOR,
        find,
        instead: "a role of the Palette (p.ink, p.raised, p.on_accent…) or a derivation of one (gamma_multiply, lerp_to_gamma); \
                  widgets::PICTURE_TINT for a picture drawn with its own colors, Color32::PLACEHOLDER for a text that is only measured",
    }
}

const RULES: &[Rule] = &[
    stock(
        "egui::ProgressBar",
        "widgets::progress (it glides to its value), or widgets::level for a meter",
    ),
    // the method alone: a call chain often puts it on its own line
    stock(".checkbox(", "widgets::checkbox, or widgets::switch_labeled for a lone setting"),
    // `new` and `without_text`
    stock(
        "egui::Checkbox::",
        "widgets::checkbox, or widgets::switch_labeled for a lone setting",
    ),
    stock(".radio(", "widgets::radio"),
    stock(".radio_value(", "widgets::radio_value"),
    stock("egui::RadioButton", "widgets::radio"),
    stock("egui::CollapsingHeader", "widgets::collapsing"),
    stock("color_edit_button", "widgets::color_button"),
    stock(
        "egui::Window::new",
        "widgets::Floater (widgets::Modal for a dialog, widgets::bare_window without a title bar)",
    ),
    stock(
        "Frame::window(",
        "widgets::Floater, or widgets::overlay / widgets::overlay_frame for a panel",
    ),
    stock("Frame::group(", "widgets::surface, or widgets::overlay for a panel over the world"),
    stock("ui.group(", "widgets::surface, or widgets::overlay for a panel over the world"),
    stock(
        ".separator()",
        "widgets::divider between two zones of a window, menu::separator in a menu, nothing between buttons",
    ),
    // egui's floating bar is drawn over the content's right edge (switches,
    // buttons, values): the theme gives every scroll area a reserved one
    stock(
        "ScrollStyle",
        "the theme's scroll bars (Theme::apply sets widgets::reserved_scroll_style): a strip of their own, never over the content",
    ),
    stock(
        "spacing_mut().scroll",
        "the theme's scroll bars (Theme::apply sets widgets::reserved_scroll_style): a strip of their own, never over the content",
    ),
    stock(
        "spacing.scroll",
        "the theme's scroll bars (Theme::apply sets widgets::reserved_scroll_style): a strip of their own, never over the content",
    ),
    radius("rect_filled(", 1),
    radius("rect_stroke(", 1),
    radius("CornerRadius::same(", 0),
    radius(".corner_radius(", 0),
    color(Find::Components("Color32::from_rgb(")),
    color(Find::Components("Color32::from_rgba_premultiplied(")),
    color(Find::Components("Color32::from_rgba_unmultiplied(")),
    color(Find::Components("Color32::from_gray(")),
    color(Find::Components("Color32::from_black_alpha(")),
    color(Find::Components("Color32::from_white_alpha(")),
    color(Find::NamedColor),
];

/// Named colors that are not a value of appearance: nothing, and "not yet".
const NO_COLOR: &[&str] = &["TRANSPARENT", "PLACEHOLDER"];

/// (file under `src/`, rule it may break, why).
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "ui/widgets.rs",
        "egui::Window::new",
        "the shared windows themselves: Floater and bare_window wrap egui's window",
    ),
    (
        "ui/widgets.rs",
        "ScrollStyle",
        "the theme's scroll bars themselves: reserved_scroll_style, which Theme::apply sets",
    ),
    (
        "ui/context/entries.rs",
        ".separator()",
        "`ui` is the builder of the menu entries there: its separators are drawn by menu::separator",
    ),
    (
        "ui/minimap.rs",
        COLOR,
        "content: Firestorm's colors of the mini-map (objects, avatars, tracking) and its marks over the map",
    ),
    (
        "ui/worldmap/mod.rs",
        COLOR,
        "content: the ocean, the marks and the legend of the world map, drawn over its tiles",
    ),
    (
        "ui/debug_overlay.rs",
        COLOR,
        "debug overlay: lines drawn in the world, not a window",
    ),
    (
        "ui/aurora_bg.rs",
        COLOR,
        "brand: the painting of the aurora sky (stars, depths of the navy)",
    ),
    (
        "ui/voice_tag.rs",
        RADIUS,
        "voice ring around a name tag: a visual of the world, kept with the tag",
    ),
];

/// The comment that allows one line of content, followed by its reason.
const MARKER: &str = "// theme-guard:";

/// What is scanned, under `src/`.
const ROOTS: &[&str] = &["ui", "build/ui", "crash/window.rs", "imports/ui", "imports/ui.rs"];

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

/// The lines of `source` up to the test module that ends the file, and for
/// each whether it is code (not a comment line).
fn scanned_lines(source: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        let text = line.trim_start();
        if text.starts_with("#[cfg(test)]") {
            let next = lines.peek().map_or("", |next| next.trim());
            let module = next.strip_prefix("pub(crate) ").or(next.strip_prefix("pub ")).unwrap_or(next);
            if module.starts_with("mod ") && module.ends_with('{') {
                break;
            }
        }
        out.push((line, !text.starts_with("//")));
    }
    out
}

/// A line without its end-of-line comment.
fn code_of(line: &str) -> &str {
    line.find("//").map_or(line, |at| &line[..at])
}

/// Arguments of the call whose opening parenthesis ends at `from` in line
/// `at`, split at its own commas; the call may run over the next lines.
fn arguments(lines: &[(&str, bool)], at: usize, from: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for (offset, (line, _)) in lines[at..].iter().take(40).enumerate() {
        let text = code_of(if offset == 0 { &line[from..] } else { line });
        for c in text.chars() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth == 0 => {
                    out.push(current.trim().to_owned());
                    return out;
                }
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(current.trim().to_owned());
                    current.clear();
                    continue;
                }
                _ => {}
            }
            current.push(c);
        }
        current.push(' ');
    }
    out
}

/// The value of an argument written as a number (`2.0`, `3`, `0xD9`, `4_u8`).
fn number(argument: &str) -> Option<f64> {
    let text = argument.replace('_', "");
    let text = ["f32", "f64", "u8", "i8"]
        .iter()
        .find_map(|suffix| text.strip_suffix(suffix))
        .unwrap_or(&text);
    match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok().map(f64::from),
        None if text.starts_with(|c: char| c.is_ascii_digit()) => text.parse().ok(),
        None => None,
    }
}

/// What `rule` finds in the code line `at`, as written in the source.
fn found(rule: &Rule, lines: &[(&str, bool)], at: usize) -> Option<String> {
    let line = code_of(lines[at].0);
    let calls = |call: &'static str| {
        line.match_indices(call)
            .map(move |(start, _)| arguments(lines, at, start + call.len()))
    };
    match rule.find {
        Find::Text(text) => line.contains(text).then(|| text.to_owned()),
        Find::Radius(call, index) => calls(call)
            .filter_map(|arguments| arguments.get(index).cloned())
            .find(|radius| number(radius).is_some_and(|value| value != 0.0))
            .map(|radius| format!("{call}… {radius}")),
        Find::Components(call) => calls(call)
            .find(|arguments| arguments.iter().take(3).any(|component| number(component).is_some()))
            .map(|arguments| format!("{call}{})", arguments.join(", "))),
        Find::NamedColor => line.match_indices("Color32::").find_map(|(start, prefix)| {
            let name: String = line[start + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let constant = !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            (constant && !NO_COLOR.contains(&name.as_str())).then(|| format!("Color32::{name}"))
        }),
    }
}

/// Whether a marker comment may allow what `rule` finds.
fn takes_marker(rule: &Rule) -> bool {
    rule.name == RADIUS || rule.name == COLOR
}

fn allowed(file: &str, rule: &Rule) -> bool {
    ALLOWED.iter().any(|(path, name, _)| *path == file && *name == rule.name)
}

/// What the scan of one file reports.
#[derive(Default)]
struct Report {
    /// "file:line: what → replacement" for each thing found and not allowed.
    violations: Vec<String>,
    /// Markers without a reason, or that allow nothing.
    markers: Vec<String>,
    /// Names of the rules of `ALLOWED` that this file needed.
    used: Vec<&'static str>,
    /// Lines allowed by a marker.
    marked: usize,
}

fn scan(file: &str, source: &str) -> Report {
    let lines = scanned_lines(source);
    let mut report = Report::default();
    // (line, rule) of what is found, before any exception
    let mut hits: Vec<(usize, &Rule, String)> = Vec::new();
    for (at, (_, code)) in lines.iter().enumerate() {
        if *code {
            hits.extend(RULES.iter().filter_map(|rule| Some((at, rule, found(rule, &lines, at)?))));
        }
    }
    let needs_marker = |at: usize| {
        hits.iter()
            .any(|(line, rule, _)| *line == at && takes_marker(rule) && !allowed(file, rule))
    };
    let mut covered = vec![false; lines.len()];
    for (at, (line, code)) in lines.iter().enumerate() {
        let Some(start) = line.find(MARKER) else { continue };
        let place = format!("  src/{file}:{}", at + 1);
        if line[start + MARKER.len()..].trim().is_empty() {
            report.markers.push(format!("{place}: a marker says why the line is content"));
        }
        let mut used = false;
        if *code {
            used = needs_marker(at);
            covered[at] = used;
        } else {
            // alone on its line: the code lines that follow and need it
            for next in (at + 1..lines.len()).filter(|next| lines[*next].1) {
                if !needs_marker(next) {
                    break;
                }
                covered[next] = true;
                used = true;
            }
        }
        if !used {
            report
                .markers
                .push(format!("{place}: this marker allows nothing any more: remove it"));
        }
    }
    report.marked = covered.iter().filter(|covered| **covered).count();
    for (at, rule, what) in &hits {
        if allowed(file, rule) {
            if !report.used.contains(&rule.name) {
                report.used.push(rule.name);
            }
        } else if !(takes_marker(rule) && covered[*at]) {
            report
                .violations
                .push(format!("  src/{file}:{}: `{what}` → use {}", at + 1, rule.instead));
        }
    }
    report
}

#[test]
fn windows_take_their_appearance_from_the_theme() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    for root in ROOTS {
        let path = src.join(root);
        assert!(path.exists(), "{} is gone: update ROOTS", path.display());
        sources(&path, &mut files);
    }
    assert!(files.len() > 50, "only {} files scanned", files.len());
    let mut violations = Vec::new();
    let mut markers = Vec::new();
    let mut used = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).expect("a source file is UTF-8 text");
        let file = path.strip_prefix(&src).expect("under src").to_string_lossy().replace('\\', "/");
        let report = scan(&file, &source);
        violations.extend(report.violations);
        markers.extend(report.markers);
        used.extend(report.used.into_iter().map(|rule| (file.clone(), rule)));
    }
    assert!(
        violations.is_empty(),
        "the interface styles itself instead of taking the Aurora theme (AGENTS.md, section 7 \"Interface et branding\", \
         and docs/BRANDING.md: shared widgets of ui/widgets.rs, radii of the theme, colors of the Palette):\n{}\n\
         If a shared widget itself must wrap the stock one, or a file paints content by nature (a map), add the file to \
         ALLOWED in ui/theme_guard.rs with the reason. If one radius or color is content rather than chrome (a legend, \
         a fixed badge color), end its line with `{MARKER} content — the reason`, or put that comment alone on the line above.",
        violations.join("\n")
    );
    assert!(markers.is_empty(), "markers of ui/theme_guard.rs to fix:\n{}", markers.join("\n"));
    // every exception still exists and is still needed
    for (file, rule, why) in ALLOWED {
        assert!(
            used.iter().any(|(path, name)| path == file && name == rule),
            "src/{file} no longer needs its exception for `{rule}` ({why}): remove it from ALLOWED"
        );
    }
}

/// The theme's scroll bars take a strip of their own: never egui's default
/// floating bar, which shows over the content's right edge when hovered.
#[test]
fn the_theme_reserves_the_scroll_bars_strip() {
    let ctx = egui::Context::default();
    crate::theme::Theme::default().apply(&ctx, 1.0);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let scroll = ctx.style_of(theme).spacing.scroll;
        assert!(!scroll.floating, "{theme:?}: the scroll bars float over the content");
        assert_eq!(scroll, super::widgets::reserved_scroll_style(), "{theme:?}");
        assert!(scroll.allocated_width() >= scroll.bar_width, "{theme:?}: no strip for the bar");
    }
    // and a window cannot set another style without an exception
    let report = scan("ui/example.rs", "ui.spacing_mut().scroll = egui::style::ScrollStyle::floating();");
    assert_eq!(report.violations.len(), 2, "{:?}", report.violations);
    let report = scan("ui/example.rs", "ui.spacing_mut().scroll.floating = true;");
    assert_eq!(report.violations.len(), 1, "{:?}", report.violations);
    // a marker does not allow it: only an entry of ALLOWED
    let report = scan(
        "ui/example.rs",
        "ui.spacing_mut().scroll.floating = true; // theme-guard: content — no",
    );
    assert_eq!(report.violations.len(), 1, "{:?}", report.violations);
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
    let found = scan("ui/example.rs", source).violations;
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
    // the exception is for its file and its rule only
    let count = |file: &str, source: &str| scan(file, source).violations.len();
    assert_eq!(count("ui/widgets.rs", "egui::Window::new(title)"), 0);
    assert_eq!(count("ui/widgets.rs", "ui.group(|ui| {})"), 1);
    assert_eq!(count("ui/minimap.rs", "egui::Window::new(title)"), 1);
    // the stock radio buttons and separators, not the shared ones
    assert_eq!(
        count(
            "ui/example.rs",
            "ui.radio(on, label);\nui.radio_value(&mut v, 1, label);\nui.separator();"
        ),
        3
    );
    assert_eq!(
        count("ui/example.rs", "widgets::radio(ui, p, on, label);\nmenu::separator(ui, p);"),
        0
    );
    // a stock check box on its own line of a call chain, or without a label
    assert_eq!(
        count(
            "ui/example.rs",
            "let changed = ui\n    .checkbox(&mut on, label)\n    .changed();\nui.add(egui::Checkbox::without_text(&mut on));"
        ),
        2
    );
    assert_eq!(count("ui/example.rs", "widgets::checkbox(ui, p, &mut on, label);"), 0);
}

#[test]
fn the_scan_finds_values_of_appearance_written_in_a_window() {
    let count = |source: &str| scan("ui/example.rs", source).violations.len();
    // a radius written as a number, on the line of the call or below it
    assert_eq!(count("ui.painter().rect_filled(rect, 2.0, p.raised);"), 1);
    assert_eq!(
        count("painter.rect_stroke(\n    rect.shrink(1.0),\n    3.0,\n    stroke,\n    kind,\n);"),
        1
    );
    assert_eq!(count("let r = CornerRadius::same(4);"), 1);
    assert_eq!(count("egui::Frame::new().corner_radius(6.0)"), 1);
    // square by design, the theme's, a named constant, a circle's half size
    assert_eq!(count("ui.painter().rect_filled(rect, 0.0, p.field);"), 0);
    assert_eq!(
        count("ui.painter().rect_filled(rect.shrink(2.0), ui.visuals().window_corner_radius, p.field);"),
        0
    );
    assert_eq!(
        count("painter.rect_filled(bar, TRACK_RADIUS, color);\npainter.rect_filled(badge, H / 2.0, p.rose);"),
        0
    );
    // a color written as numbers or by its name
    assert_eq!(count("let c = Color32::from_rgb(0xD9, 0x2D, 0x2D);"), 1);
    assert_eq!(
        count("let c = egui::Color32::from_rgba_unmultiplied(\n    10,\n    20,\n    30,\n    a,\n);"),
        1
    );
    assert_eq!(count("let shade = Color32::from_black_alpha(160);\nlet text = Color32::WHITE;"), 2);
    // nothing, not yet, a color of the palette and one built from its own components
    assert_eq!(count("let none = Color32::TRANSPARENT;\nlet measured = Color32::PLACEHOLDER;"), 0);
    assert_eq!(
        count("let glass = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), 168);"),
        0
    );
    assert_eq!(count("let swatch = Color32::from_rgb(r, g, b);"), 0);
    // a file allowed for its colors still answers for its radii
    let map = scan(
        "ui/minimap.rs",
        "let sea = Color32::from_rgb(29, 71, 95);\npainter.rect_filled(rect, 4.0, sea);",
    );
    assert_eq!((map.violations.len(), map.used), (1, vec![COLOR]));
}

#[test]
fn a_marker_allows_the_content_it_is_written_on() {
    // at the end of the line, or alone above the lines that need it
    let report = scan(
        "ui/example.rs",
        "let adult = Color32::from_rgb(217, 45, 45); // theme-guard: content — fixed badge color",
    );
    assert!(report.violations.is_empty() && report.markers.is_empty(), "{:?}", report.violations);
    let table = "\
let legend = [
    // theme-guard: content — colors of the overlay
    (Color32::from_rgb(0, 200, 0), yours),
    (Color32::from_rgb(220, 0, 0), others),
];
let text = Color32::WHITE;
";
    let report = scan("ui/example.rs", table);
    assert_eq!((report.violations.len(), report.marked), (1, 2), "{:?}", report.violations);
    assert!(report.violations[0].contains(":6:"), "{}", report.violations[0]);
    assert!(report.markers.is_empty());
    // a marker with nothing left to allow, or without its reason, fails
    let report = scan("ui/example.rs", "// theme-guard: content — was a color\nlet text = p.ink;");
    assert_eq!(report.markers.len(), 1);
    let report = scan("ui/example.rs", "let text = Color32::WHITE; // theme-guard:");
    assert_eq!((report.violations.len(), report.markers.len()), (0, 1));
    // a stock widget is never content
    let report = scan("ui/example.rs", "ui.separator(); // theme-guard: content — no");
    assert_eq!((report.violations.len(), report.markers.len()), (1, 1));
}
