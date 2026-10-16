//! Fenced LSL shares the script lexer's semantic colors. Source offsets stay
//! separate from the selectable preview, including CRLF and nested Markdown.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag};

use super::{Rendered, Span, Style};
use crate::ui::scripts;

pub(super) fn is_lsl(kind: &CodeBlockKind<'_>) -> bool {
    matches!(kind, CodeBlockKind::Fenced(info) if info.split_whitespace().next().is_some_and(|name| name.eq_ignore_ascii_case("lsl")))
}

/// Re-evaluate only for script key commands, at their actual insertion point.
/// This includes unfinished fences while typing, but excludes the fence rows.
pub(in crate::ui::inventory) fn scope(source: &str, at: usize) -> Option<Range<usize>> {
    for (event, range) in Parser::new(source).into_offset_iter() {
        if let Event::Start(Tag::CodeBlock(kind)) = event
            && is_lsl(&kind)
        {
            let block = &source[range.clone()];
            let Some(newline) = block.find('\n') else { continue };
            let start = range.start + newline + 1;
            let opening = block[..newline].trim();
            let marker = opening.chars().next()?;
            let count = opening.chars().take_while(|c| *c == marker).count();
            let last = block.trim_end_matches(['\r', '\n']);
            let closing = last.rfind('\n').map_or(0, |n| n + 1);
            let line = last[closing..].trim();
            let run = line.chars().take_while(|c| *c == marker).count();
            let closed = run >= count && line[run..].trim().is_empty();
            let end = if closed { range.start + closing } else { range.end };
            if at >= start && (at < end || (at == end && !closed)) {
                return Some(start..end);
            }
        }
    }
    None
}

pub(super) struct Block {
    start: usize,
    span: usize,
    pieces: Vec<(Range<usize>, Range<usize>)>,
}

impl Block {
    pub(super) fn new(rendered: &Rendered) -> Self {
        Self {
            start: rendered.text.len(),
            span: rendered.spans.len(),
            pieces: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, source: Range<usize>, rendered: Range<usize>) {
        self.pieces.push((source, rendered));
    }

    pub(super) fn finish(self, source: &str, rendered: &mut Rendered) {
        let tokens = scripts::highlight_lsl(&rendered.text[self.start..]);
        rendered.spans.truncate(self.span);
        for (bytes, role) in &tokens {
            rendered.spans.push(Span {
                bytes: self.start + bytes.start..self.start + bytes.end,
                style: Style {
                    code: true,
                    block_code: true,
                    role: Some(*role),
                    ..Default::default()
                },
            });
        }
        for (raw, visible) in self.pieces {
            let Some(map) = source_map(&source[raw.clone()], &rendered.text[visible.clone()]) else {
                continue;
            };
            let start = visible.start - self.start;
            let end = visible.end - self.start;
            let first = tokens.partition_point(|(range, _)| range.end <= start);
            for (range, role) in tokens[first..].iter().take_while(|(range, _)| range.start < end) {
                rendered.source_spans.push((
                    raw.start + map[range.start.max(start) - start]..raw.start + map[range.end.min(end) - start],
                    *role,
                ));
            }
        }
    }
}

/// pulldown-cmark may normalize CRLF. Map UTF-8 boundaries back to the exact
/// raw buffer; never apply guessed offsets if a piece was transformed further.
fn source_map(raw: &str, visible: &str) -> Option<Vec<usize>> {
    let mut map = vec![0; visible.len() + 1];
    let mut at = 0;
    for (byte, c) in raw.char_indices() {
        if c == '\r' && raw[byte..].starts_with("\r\n") {
            continue;
        }
        if visible.get(at..)?.chars().next()? != c {
            return None;
        }
        map[at] = byte;
        at += c.len_utf8();
        map[at] = byte + c.len_utf8();
    }
    (at == visible.len()).then_some(map)
}

pub(in crate::ui::inventory) fn source_layout(
    rendered: &Rendered,
    p: &crate::theme::Palette,
    settings: &scripts::settings::ScriptSettings,
    normal: egui::TextFormat,
    append: &mut dyn FnMut(&mut egui::text::LayoutJob, &str, egui::TextFormat),
    job: &mut egui::text::LayoutJob,
) {
    let mut at = 0;
    for (range, role) in &rendered.source_spans {
        append(job, &rendered.source[at..range.start], normal.clone());
        append(
            job,
            &rendered.source[range.clone()],
            egui::TextFormat {
                color: settings.color(*role, p),
                ..normal.clone()
            },
        );
        at = range.end;
    }
    append(job, &rendered.source[at..], normal);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::scripts::settings::ColorRole;

    #[test]
    fn lsl_colors_share_script_tokens_and_preserve_raw_unicode_crlf_and_nested_fences() {
        for newline in ["\n", "\r\n"] {
            for prefix in ["", "> ", "  "] {
                let source = [
                    "```LSL extra",
                    "// Été 🐺",
                    "integer count = 2;",
                    "/* comment",
                    "llNotAFunction(); */",
                    "llSay(PUBLIC_CHANNEL, \"Été 🐺\");",
                    "```",
                ]
                .map(|line| format!("{prefix}{line}"))
                .join(newline);
                let rendered = Rendered::parse(&source);
                assert_eq!(rendered.source, source);
                for (needle, role) in [
                    ("integer", ColorRole::Type),
                    ("llSay", ColorRole::Function),
                    ("\"Été 🐺\"", ColorRole::String),
                ] {
                    assert!(
                        rendered
                            .spans
                            .iter()
                            .any(|span| span.style.role == Some(role) && &rendered.text[span.bytes.clone()] == needle),
                        "preview: {needle} in {source:?}"
                    );
                    assert!(
                        rendered
                            .source_spans
                            .iter()
                            .any(|(range, found)| *found == role && &source[range.clone()] == needle),
                        "source: {needle} in {source:?}"
                    );
                }
                assert!(rendered.spans.iter().any(
                    |span| span.style.role == Some(ColorRole::Comment) && rendered.text[span.bytes.clone()].contains("llNotAFunction")
                ));
                assert!(rendered.source_spans.windows(2).all(|spans| spans[0].0.end <= spans[1].0.start));
            }
        }
        for source in ["`llSay()`", "```text\nllSay();\n```", "```lslang\nllSay();\n```", "    llSay();"] {
            let rendered = Rendered::parse(source);
            assert!(rendered.source_spans.is_empty());
            assert!(rendered.spans.iter().all(|span| span.style.role.is_none()));
        }
    }

    #[test]
    fn script_scope_excludes_prose_and_fences_and_accepts_unfinished_lsl() {
        let source = "Été \"{\n```lsl\ndefault {\n    \n}\n```\nplain {";
        let body = source.find("default").expect("body")..source.rfind("```").expect("closing");
        for at in [body.start, body.start + 10, body.end - 1] {
            assert_eq!(scope(source, at), Some(body.clone()));
        }
        for at in [0, source.find("lsl").expect("language"), body.end, source.len()] {
            assert_eq!(scope(source, at), None);
        }
        for source in ["```lsl\n", "~~~LSL\r\ndefault {\r\n    "] {
            let range = scope(source, source.len()).expect("unfinished code");
            assert_eq!(range.end, source.len());
        }
        assert!(scope("```text\n{", 9).is_none());
    }

    #[test]
    fn lsl_layout_resolves_custom_script_colors_without_reparsing_or_changing_source() {
        let source = "Before 🐺\r\n\r\n```lsl\r\nllSay(0, \"Été\");\r\n```\r\nAfter";
        let rendered = Rendered::parse(source);
        let p = crate::theme::Theme::default().palette();
        let mut settings = scripts::settings::ScriptSettings::default();
        settings.colors.insert(ColorRole::Function, p.rose.to_srgba_unmultiplied());
        let ctx = egui::Context::default();
        let mut raw = egui::text::LayoutJob::default();
        let mut preview = egui::text::LayoutJob::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut append = |job: &mut egui::text::LayoutJob, text: &str, format| job.append(text, 0.0, format);
            source_layout(&rendered, &p, &settings, egui::TextFormat::default(), &mut append, &mut raw);
            rendered.layout(ui, &p, &settings, &mut append, &mut preview);
        });
        output.textures_delta.clear();
        assert_eq!(raw.text, source);
        for job in [&raw, &preview] {
            let section = job
                .sections
                .iter()
                .find(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0] == "llSay")
                .expect("function");
            assert_eq!(section.format.color, p.rose);
        }
    }
}
