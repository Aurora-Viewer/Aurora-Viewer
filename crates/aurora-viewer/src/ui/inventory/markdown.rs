//! An optional CommonMark view of the source draft. Never used for uploads.
//! Classic follows LLPreviewNotecard / LLViewerTextEditor (Firestorm, LGPL 2.1);
//! Markdown and the source editor with live preview are Aurora additions.

use std::{ops::Range, sync::Arc};

use egui::text::{LayoutJob, TextFormat};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::theme::Palette;

pub(super) mod code;
mod formatting;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum View {
    #[default]
    Classic,
    Markdown,
    Edit,
}

impl View {
    pub(super) fn choose(&mut self, ui: &mut egui::Ui, p: &Palette) -> bool {
        let mut index = *self as usize;
        let changed = super::super::widgets::tabs(
            ui,
            p,
            &mut index,
            &[
                (aurora_i18n::tr!("notecard-view-classic"), true),
                (aurora_i18n::tr!("notecard-view-markdown"), true),
                (aurora_i18n::tr!("notecard-view-source"), true),
            ],
        );
        *self = [Self::Classic, Self::Markdown, Self::Edit][index];
        changed
    }
}

/// Only parse a changed draft, on the worker pool. Each window owns its cache;
/// replacing a pending receiver also discards stale replies after further edits.
#[derive(Default)]
pub(super) struct Cache {
    source: Option<String>,
    ready: Option<Arc<Rendered>>,
    pending: Option<std::sync::mpsc::Receiver<Rendered>>,
}

impl Cache {
    pub(super) fn get(&mut self, ctx: &egui::Context, text: &str) -> Option<Arc<Rendered>> {
        if self.source.as_deref() != Some(text) {
            self.source = Some(text.to_owned());
            // Keep the last preview visible while typing; only this receiver
            // can publish a result for the latest draft.
            let text = text.to_owned();
            let ctx = ctx.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            self.pending = Some(rx);
            rayon::spawn(move || {
                let _ = tx.send(Rendered::parse(&text));
                ctx.request_repaint();
            });
        }
        if let Some(rx) = &self.pending
            && let Ok(rendered) = rx.try_recv()
        {
            self.ready = Some(Arc::new(rendered));
            self.pending = None;
        }
        self.ready.clone()
    }
}

pub(super) fn editor(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &super::Icons,
    id: egui::Id,
    text: &mut String,
    source: Option<&mut aurora_net::inventory::notecard::Notecard>,
    editable: bool,
    size: egui::Vec2,
    view: View,
    cache: &mut Cache,
    settings: &super::super::scripts::settings::ScriptSettings,
) -> super::embedded::EditorResult {
    use super::embedded::{self, Appearance};

    let history = formatting::history_key(ui, id, editable);
    let syntax = (view == View::Edit).then(|| cache.get(ui.ctx(), text)).flatten();
    let preview_id = id.with("markdown");
    let mut preview = |ui: &mut egui::Ui, text: &mut String, source: Option<&mut aurora_net::inventory::notecard::Notecard>, size| {
        let Some(rendered) = cache.get(ui.ctx(), text) else {
            ui.allocate_ui(size, |ui| {
                ui.spinner();
            });
            return embedded::EditorResult::default();
        };
        let mut source = source;
        if let Some(source) = source.as_deref_mut() {
            embedded::clipboard(ui.ctx(), preview_id, source, &rendered.text, false);
        }
        scroll(ui, preview_id, size, |ui, size| {
            embedded::editor(
                ui,
                p,
                icons,
                preview_id,
                text,
                source.as_deref(),
                false,
                size,
                Appearance::Markdown(&rendered, settings),
            )
        })
    };
    match view {
        View::Classic => scroll(ui, id, size, |ui, size| {
            let mut result = embedded::editor(ui, p, icons, id, text, source.as_deref(), editable, size, Appearance::Classic);
            if let Some(redo) = history {
                result.changed |= formatting::history(ui.ctx(), id, text, redo);
            }
            result
        }),
        View::Markdown => preview(ui, text, source, size),
        View::Edit => {
            let top = ui.cursor().top();
            let can_format = editable && !ui.ctx().memory(|memory| memory.has_focus(preview_id));
            let command = formatting::toolbar(ui, p, id, can_format);
            let toolbar_height = ui.cursor().top() - top;
            ui.columns(2, |columns| {
                for (column, label) in columns.iter_mut().zip([
                    aurora_i18n::tr!("notecard-source-heading"),
                    aurora_i18n::tr!("notecard-preview-heading"),
                ]) {
                    column.add(egui::Label::new(egui::RichText::new(label).color(p.muted)).truncate());
                }
                let height = (size.y - toolbar_height - ui_heading_height(&columns[0])).max(24.0);
                let left_size = egui::vec2(columns[0].available_width(), height);
                let mut result = scroll(&mut columns[0], id, left_size, |ui, size| {
                    embedded::editor(
                        ui,
                        p,
                        icons,
                        id,
                        text,
                        source.as_deref(),
                        editable,
                        size,
                        Appearance::CodeSource(syntax.as_deref(), settings),
                    )
                });
                if let Some(redo) = history {
                    result.changed |= formatting::history(columns[0].ctx(), id, text, redo);
                }
                if let Some(command) = command {
                    result.changed |= formatting::apply(columns[0].ctx(), id, text, command);
                }
                // Parse after the source editor consumed this frame's input.
                let right_size = egui::vec2(columns[1].available_width(), height);
                let right = preview(&mut columns[1], text, source, right_size);
                result.save |= right.save;
                result.open = right.open.or(result.open);
                result.copy = right.copy.or(result.copy);
                result
            })
        }
    }
}

fn ui_heading_height(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y
}

fn scroll(
    ui: &mut egui::Ui,
    id: egui::Id,
    size: egui::Vec2,
    show: impl FnOnce(&mut egui::Ui, egui::Vec2) -> super::embedded::EditorResult,
) -> super::embedded::EditorResult {
    let out = egui::ScrollArea::both()
        .id_salt(id.with("scroll"))
        .max_height(size.y)
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            // Fill the viewport, not the outer area: a reserved scroll bar
            // reduces both axes. Also cap the pixel-rounded child UI at the
            // actual viewport so rounding cannot create another scroll bar.
            let size = ui.available_size().min(viewport.size()).min(size);
            ui.set_max_size(size);
            show(ui, size)
        });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("test_scroll_geometry"), (out.inner_rect, out.content_size)));
    out.inner
}

#[derive(Clone, Default)]
struct Style {
    heading: u8,
    strong: bool,
    italic: bool,
    strike: bool,
    code: bool,
    block_code: bool,
    quote: bool,
    link: Option<Arc<str>>,
    role: Option<super::super::scripts::settings::ColorRole>,
}

struct Span {
    bytes: Range<usize>,
    style: Style,
}

pub(super) struct Rendered {
    pub(super) source: String,
    source_spans: Vec<(Range<usize>, super::super::scripts::settings::ColorRole)>,
    pub text: String,
    spans: Vec<Span>,
    pub links: Vec<(Range<usize>, Arc<str>)>,
    pub code_blocks: Vec<Range<usize>>,
    chars: usize,
}

impl Rendered {
    fn push(&mut self, text: &str, style: &Style) {
        let start = self.text.len();
        self.text.push_str(text);
        self.spans.push(Span {
            bytes: start..self.text.len(),
            style: style.clone(),
        });
        if let Some(url) = &style.link {
            let start = self.chars;
            self.links.push((start..start + text.chars().count(), url.clone()));
        }
        self.chars += text.chars().count();
    }

    fn newline(&mut self, count: usize) {
        if !self.text.is_empty() {
            let present = self.text.chars().rev().take_while(|c| *c == '\n').count();
            for _ in present..count {
                self.push("\n", &Style::default());
            }
        }
    }

    pub(super) fn parse(source: &str) -> Self {
        let mut rendered = Self {
            source: source.into(),
            source_spans: Vec::new(),
            text: String::new(),
            spans: Vec::new(),
            links: Vec::new(),
            code_blocks: Vec::new(),
            chars: 0,
        };
        let mut style = Style::default();
        let mut stack = Vec::new();
        let mut lists: Vec<Option<u64>> = Vec::new();
        let mut cell = 0;
        let mut code = None;
        let mut block_start = None;
        for (event, range) in Parser::new_ext(
            source,
            Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES,
        )
        .into_offset_iter()
        {
            match event {
                Event::Start(tag) => {
                    stack.push(style.clone());
                    match tag {
                        Tag::Heading { level, .. } => {
                            rendered.newline(2);
                            style.heading = level as u8;
                            style.strong = true;
                        }
                        Tag::Paragraph if lists.is_empty() => rendered.newline(2),
                        Tag::Strong => style.strong = true,
                        Tag::Emphasis => style.italic = true,
                        Tag::Strikethrough => style.strike = true,
                        Tag::CodeBlock(kind) => {
                            rendered.newline(2);
                            style.code = true;
                            style.block_code = true;
                            block_start = Some(rendered.chars);
                            if code::is_lsl(&kind) {
                                code = Some(code::Block::new(&rendered));
                            }
                        }
                        Tag::BlockQuote(_) => {
                            rendered.newline(2);
                            style.quote = true;
                        }
                        Tag::List(first) => {
                            rendered.newline(1);
                            lists.push(first);
                        }
                        Tag::Item => {
                            rendered.newline(1);
                            rendered.push(&"  ".repeat(lists.len().saturating_sub(1)), &style);
                            let prefix = if let Some(Some(number)) = lists.last_mut() {
                                let prefix = format!("{number}. ");
                                *number = number.saturating_add(1);
                                prefix
                            } else {
                                "• ".into()
                            };
                            rendered.push(&prefix, &style);
                        }
                        Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                            // Images remain a selectable alt label with an explicit
                            // link; previewing a notecard never downloads its URLs.
                            style.link = Some(Arc::from(dest_url.as_ref()));
                        }
                        Tag::Table(_) => {
                            rendered.newline(2);
                            style.code = true;
                        }
                        Tag::TableHead => {
                            cell = 0;
                            style.strong = true;
                        }
                        Tag::TableRow => cell = 0,
                        Tag::TableCell => {
                            if cell > 0 {
                                rendered.push("  │  ", &style);
                            }
                            cell += 1;
                        }
                        _ => {}
                    }
                }
                Event::End(tag) => {
                    if tag == TagEnd::CodeBlock
                        && let Some(start) = block_start.take()
                    {
                        if rendered.chars == start {
                            rendered.push("\n", &style);
                        }
                        rendered.code_blocks.push(start..rendered.chars);
                    }
                    if tag == TagEnd::CodeBlock
                        && let Some(code) = code.take()
                    {
                        code.finish(source, &mut rendered);
                    }
                    match tag {
                        TagEnd::Paragraph => rendered.newline(if lists.is_empty() { 2 } else { 1 }),
                        TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::BlockQuote(_) | TagEnd::Table => rendered.newline(2),
                        TagEnd::Item | TagEnd::TableHead | TagEnd::TableRow => rendered.newline(1),
                        TagEnd::List(_) => {
                            lists.pop();
                            rendered.newline(if lists.is_empty() { 2 } else { 1 });
                        }
                        _ => {}
                    }
                    style = stack.pop().unwrap_or_default();
                }
                Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                    let start = rendered.text.len();
                    rendered.push(&text, &style);
                    if let Some(code) = &mut code {
                        code.push(range, start..rendered.text.len());
                    }
                }
                Event::Code(text) => rendered.push(
                    &text,
                    &Style {
                        code: true,
                        ..style.clone()
                    },
                ),
                Event::SoftBreak => {
                    // CommonMark folds ordinary soft breaks into spaces, but
                    // notecard inventory rows must retain their source layout.
                    let before = source[..range.start].rsplit('\n').next().unwrap_or_default();
                    let after = source[range.end..].split('\n').next().unwrap_or_default();
                    let attachment_line = before
                        .chars()
                        .chain(after.chars())
                        .any(|c| c as u32 >= aurora_net::inventory::notecard::FIRST_EMBEDDED_CHAR);
                    rendered.push(if attachment_line { "\n" } else { " " }, &style);
                }
                Event::HardBreak => rendered.push("\n", &style),
                Event::TaskListMarker(checked) => rendered.push(if checked { "[x] " } else { "[ ] " }, &style),
                Event::Rule => {
                    rendered.newline(2);
                    rendered.push("────────────", &style);
                    rendered.newline(2);
                }
                _ => {}
            }
        }
        rendered
    }

    pub(super) fn layout(
        &self,
        ui: &egui::Ui,
        p: &Palette,
        settings: &super::super::scripts::settings::ScriptSettings,
        append: &mut dyn FnMut(&mut LayoutJob, &str, TextFormat),
        job: &mut LayoutJob,
    ) {
        let body = egui::TextStyle::Body.resolve(ui.style());
        let bold = super::super::fonts::semibold(ui.ctx());
        for span in &self.spans {
            let s = &span.style;
            let mut font = if s.code {
                egui::TextStyle::Monospace.resolve(ui.style())
            } else {
                body.clone()
            };
            if s.heading > 0 {
                font.size = body.size * [1.8, 1.55, 1.3, 1.15, 1.05, 1.0][usize::from(s.heading - 1).min(5)];
            }
            let color = if let Some(role) = s.role {
                settings.color(role, p)
            } else if s.link.is_some() {
                p.teal
            } else if s.quote {
                p.muted
            } else {
                p.ink
            };
            let strong = super::super::fonts::semibold_format(&bold, font.size, color);
            append(
                job,
                &self.text[span.bytes.clone()],
                TextFormat {
                    font_id: if s.strong && !s.code { strong.font_id } else { font },
                    coords: if s.strong { strong.coords } else { Default::default() },
                    color,
                    background: if s.code && !s.block_code {
                        p.field
                    } else {
                        egui::Color32::TRANSPARENT
                    },
                    italics: s.italic || s.quote,
                    underline: if s.link.is_some() {
                        egui::Stroke::new(0.5, color)
                    } else {
                        egui::Stroke::NONE
                    },
                    strikethrough: if s.strike {
                        egui::Stroke::new(1.0, color)
                    } else {
                        egui::Stroke::NONE
                    },
                    ..Default::default()
                },
            );
        }
    }

    /// One surface per code block, derived from the shaped rows so wrapping,
    /// blank lines and mixed font heights keep the rectangle continuous.
    pub(super) fn code_rects(&self, galley: &egui::Galley, pos: egui::Pos2, clip: egui::Rect, padding: f32) -> Vec<egui::Rect> {
        let mut at = 0;
        let mut rows = galley
            .rows
            .iter()
            .map(|row| {
                let start = at;
                at += row.char_count_including_newline().0;
                (start..at, row.rect().translate(pos.to_vec2()))
            })
            .peekable();
        let mut rects = Vec::new();
        for block in &self.code_blocks {
            while rows.peek().is_some_and(|(range, _)| range.end <= block.start) {
                rows.next();
            }
            let mut rect = egui::Rect::NOTHING;
            while rows.peek().is_some_and(|(range, _)| range.start < block.end) {
                if let Some((_, row)) = rows.next() {
                    rect = rect.union(row);
                }
            }
            if rect.is_finite() {
                let rect = egui::Rect::from_min_max(
                    egui::pos2(clip.left(), rect.top() - padding),
                    egui::pos2(clip.right(), rect.bottom() + padding),
                )
                .intersect(clip);
                if rect.is_positive() {
                    rects.push(rect);
                }
            }
        }
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commonmark_keeps_unicode_attachments_and_nested_styles_without_changing_source() {
        let source = "# Été 🐺\n\n**bold *both*** and `a * b`\n\n- one\n- [x] two\n\n> quote\n\n[link](https://example.com)\n\n\u{100000}";
        let rendered = Rendered::parse(source);
        assert!(rendered.text.starts_with("Été 🐺\n\n"));
        assert!(rendered.text.contains("bold both and a * b"));
        assert!(rendered.text.contains("• one\n• [x] two"));
        assert!(rendered.text.contains('\u{100000}'));
        assert!(
            rendered
                .spans
                .iter()
                .any(|s| s.style.strong && s.style.italic && &rendered.text[s.bytes.clone()] == "both")
        );
        assert!(rendered.links.iter().any(|(range, url)| {
            rendered.text.chars().skip(range.start).take(range.len()).collect::<String>() == "link" && url.as_ref() == "https://example.com"
        }));
        assert!(source.starts_with("# "));
    }

    #[test]
    fn attachment_lines_keep_their_breaks_and_inline_items_stay_inline() {
        for newline in ["\n", "\r\n"] {
            let source = [
                "Été 🐺 **avant**",
                "\u{100000}",
                "*Repère* \u{100001}",
                "\u{100002}",
                "Texte *après*",
            ]
            .join(newline);
            let rendered = Rendered::parse(&source);
            assert_eq!(
                rendered.text,
                "Été 🐺 avant\n\u{100000}\nRepère \u{100001}\n\u{100002}\nTexte après\n\n"
            );
        }
        let rendered = Rendered::parse("Ordinary\nsoft break\n\n\u{100000} \u{100001}\nafter\ncontinued");
        assert_eq!(rendered.text, "Ordinary soft break\n\n\u{100000} \u{100001}\nafter continued\n\n");
    }

    #[test]
    fn code_escapes_tables_and_html_are_text_not_executable_content() {
        let rendered =
            Rendered::parse("\\*literal\\* &amp;\n\n```rust\n**raw**\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n<script>hello</script>");
        assert!(rendered.text.contains("*literal* &"));
        assert!(rendered.text.contains("**raw**"));
        assert!(rendered.text.contains("A  │  B\n1  │  2"));
        assert!(rendered.text.contains("<script>hello</script>"));
    }
}
