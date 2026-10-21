//! LSL token colors, following LLKeywords::findSegments / processTokensGroup
//! (Firestorm indra/llui/llkeywords.cpp, originally LGPL 2.1). Built-in names
//! come from the LSL keywords in use (`crate::lsl_keywords`: the Legacy Viewer's
//! keywords_lsl_default.xml, or the region's). Custom functions are also
//! recognized at declarations and calls. This lexer never changes the source.

use super::{
    reference,
    settings::{ColorRole, ScriptSettings},
};
use crate::lsl_keywords::{self, Group};
use crate::theme::Palette;
use egui::{
    Color32, FontId, TextFormat,
    text::{LayoutJob, LayoutSection},
};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Kind {
    Plain,
    Variable,
    Keyword,
    Type,
    Event,
    Function,
    Constant,
    Number,
    String,
    Comment,
    Float,
    Section,
    StringConstant,
    Compound,
    Preprocessor,
}

impl Kind {
    fn role(self) -> ColorRole {
        match self {
            Self::Plain => ColorRole::Text,
            Self::Variable => ColorRole::Variable,
            Self::Keyword => ColorRole::Control,
            Self::Type => ColorRole::Type,
            Self::Event => ColorRole::Event,
            Self::Function => ColorRole::Function,
            Self::Constant => ColorRole::Constant,
            Self::Number => ColorRole::Integer,
            Self::String => ColorRole::String,
            Self::Comment => ColorRole::Comment,
            Self::Float => ColorRole::Float,
            Self::Section => ColorRole::Section,
            Self::StringConstant => ColorRole::StringConstant,
            Self::Compound => ColorRole::Compound,
            Self::Preprocessor => ColorRole::Preprocessor,
        }
    }
}

/// A built-in name's kind in the LSL keywords in use.
fn keyword(name: &str) -> Option<Kind> {
    lsl_keywords::current().word(name).map(|group| match group {
        Group::Control => Kind::Keyword,
        Group::Type => Kind::Type,
        Group::Event => Kind::Event,
        Group::Function => Kind::Function,
        Group::Constant => Kind::Constant,
    })
}

struct Token {
    bytes: Range<usize>,
    kind: Kind,
}

pub(super) fn matching_open_brace(source: &str, close: usize) -> Option<usize> {
    let mut stack = Vec::new();
    for token in tokens(source) {
        if token.bytes.start > close {
            break;
        }
        if token.kind != Kind::Plain {
            continue;
        }
        match &source[token.bytes.clone()] {
            "{" => stack.push(token.bytes.start),
            "}" if token.bytes.start == close => return stack.pop(),
            "}" => {
                stack.pop();
            }
            _ => {}
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ColorLiteral {
    pub bytes: Range<usize>,
    pub components: [Range<usize>; 3],
    pub rgb: crate::ui::color_picker::Rgb,
}

fn color_literals(source: &str, tokens: &[Token]) -> Vec<ColorLiteral> {
    let code: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind != Kind::Comment && !source[t.bytes.clone()].trim().is_empty())
        .collect();
    let mut colors = Vec::new();
    for (i, token) in code.iter().enumerate() {
        if token.kind != Kind::Plain || &source[token.bytes.clone()] != "<" {
            continue;
        }
        let mut at = i + 1;
        let mut rgb = [0.0; 3];
        let mut components = [0..0, 0..0, 0..0];
        let valid = (0..3).all(|n| {
            let Some(first) = code.get(at) else { return false };
            let start = first.bytes.start;
            let sign = match &source[first.bytes.clone()] {
                "-" => {
                    at += 1;
                    -1.0
                }
                "+" => {
                    at += 1;
                    1.0
                }
                _ => 1.0,
            };
            let Some(number) = code.get(at).filter(|t| matches!(t.kind, Kind::Number | Kind::Float)) else {
                return false;
            };
            let text = &source[number.bytes.clone()];
            let value = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                u32::from_str_radix(hex, 16).ok().map(f64::from)
            } else {
                text.parse::<f64>().ok()
            };
            let Some(value) = value.map(|v| v * sign).filter(|v| (0.0..=1.0).contains(v)) else {
                return false;
            };
            rgb[n] = value as f32;
            components[n] = start..number.bytes.end;
            at += 1;
            let separator = if n == 2 { ">" } else { "," };
            if !code
                .get(at)
                .is_some_and(|t| t.kind == Kind::Plain && &source[t.bytes.clone()] == separator)
            {
                return false;
            }
            at += 1;
            true
        });
        if valid {
            colors.push(ColorLiteral {
                bytes: token.bytes.start..code[at - 1].bytes.end,
                components,
                rgb,
            });
        }
    }
    colors
}

pub(super) fn colors(source: &str) -> Vec<ColorLiteral> {
    color_literals(source, &tokens(source))
}

pub(super) fn block_comment_at(source: &str, at: usize) -> Option<Range<usize>> {
    tokens(source)
        .into_iter()
        .find(|token| {
            token.kind == Kind::Comment
                && token.bytes.contains(&at)
                && source[token.bytes.clone()].starts_with("/*")
                && source[token.bytes.clone()].ends_with("*/")
        })
        .map(|token| token.bytes)
}

pub(super) fn brace_pairs(source: &str) -> Vec<(usize, usize)> {
    let mut stack = Vec::new();
    let mut pairs = Vec::new();
    for token in tokens(source) {
        if token.kind != Kind::Plain {
            continue;
        }
        match &source[token.bytes.clone()] {
            "{" => stack.push(token.bytes.start),
            "}" => {
                if let Some(open) = stack.pop() {
                    pairs.push((open, token.bytes.start));
                }
            }
            _ => {}
        }
    }
    pairs.sort_unstable();
    pairs
}

pub(super) const COLOR_SPACE: f32 = 20.0;

fn color_spacing(job: &mut LayoutJob, colors: &[ColorLiteral]) {
    let mut starts = colors.iter().map(|c| c.bytes.start).peekable();
    let mut sections = Vec::new();
    for section in job.sections.drain(..) {
        let mut start = section.byte_range.start.0;
        let end = section.byte_range.end.0;
        let mut spacing = 0.0;
        while start < end {
            if starts.peek() == Some(&start) {
                spacing = COLOR_SPACE;
                starts.next();
            }
            let limit = starts.peek().copied().unwrap_or(end).min(end);
            sections.push(LayoutSection {
                leading_space: spacing,
                byte_range: start.into()..limit.into(),
                format: section.format.clone(),
            });
            start = limit;
            spacing = 0.0;
        }
    }
    job.sections = sections;
}

fn identifier_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn number_end(bytes: &[u8], mut end: usize) -> usize {
    if bytes[end] == b'0' && matches!(bytes.get(end + 1), Some(b'x' | b'X')) {
        end += 2;
        while bytes.get(end).is_some_and(u8::is_ascii_hexdigit) {
            end += 1;
        }
        return end;
    }
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        let digits = exponent;
        while bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            exponent += 1;
        }
        if exponent != digits {
            end = exponent;
        }
    }
    end
}

fn tokens(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let start = cursor;
        // Every cursor movement below ends on a UTF-8 boundary.
        let c = source[cursor..].chars().next().expect("cursor inside source");
        let kind = if bytes[cursor..].starts_with(b"//") {
            cursor = source[cursor..].find('\n').map_or(bytes.len(), |n| cursor + n);
            Kind::Comment
        } else if bytes[cursor..].starts_with(b"/*") {
            cursor = source[cursor + 2..].find("*/").map_or(bytes.len(), |n| cursor + 2 + n + 2);
            Kind::Comment
        } else if c == '#' && source[..cursor].rsplit('\n').next().is_some_and(|line| line.trim().is_empty()) {
            cursor = source[cursor..].find('\n').map_or(bytes.len(), |n| cursor + n);
            Kind::Preprocessor
        } else if c == '"' {
            cursor += 1;
            while cursor < bytes.len() {
                let c = source[cursor..].chars().next().expect("cursor inside string");
                cursor += c.len_utf8();
                if c == '"' {
                    break;
                }
                if c == '\\'
                    && let Some(escaped) = source[cursor..].chars().next()
                {
                    cursor += escaped.len_utf8();
                }
            }
            Kind::String
        } else if c.is_ascii_digit() || (c == '.' && bytes.get(cursor + 1).is_some_and(u8::is_ascii_digit)) {
            cursor = number_end(bytes, cursor);
            if source[start..cursor].contains(['.', 'e', 'E'])
                && !source[start..cursor].starts_with("0x")
                && !source[start..cursor].starts_with("0X")
            {
                Kind::Float
            } else {
                Kind::Number
            }
        } else if identifier_start(c) {
            cursor += c.len_utf8();
            while let Some(c) = source[cursor..].chars().next() {
                if c != '_' && !c.is_alphanumeric() {
                    break;
                }
                cursor += c.len_utf8();
            }
            Kind::Variable
        } else {
            cursor += c.len_utf8();
            if c.is_whitespace() {
                while let Some(c) = source[cursor..].chars().next() {
                    if !c.is_whitespace() {
                        break;
                    }
                    cursor += c.len_utf8();
                }
            }
            Kind::Plain
        };
        result.push(Token {
            bytes: start..cursor,
            kind,
        });
    }
    // Skip comments and whitespace when recognizing custom function calls.
    // A reverse pass avoids repeatedly scanning long comment blocks.
    let mut next_code = "";
    for token in result.iter_mut().rev() {
        let text = &source[token.bytes.clone()];
        if token.kind == Kind::Variable {
            token.kind = keyword(text).unwrap_or(if next_code == "(" { Kind::Function } else { Kind::Variable });
            if text == "default" {
                token.kind = Kind::Section;
            }
            if token.kind == Kind::Constant
                && let Some(entry) = reference::get(text)
            {
                token.kind = match entry.data_type.as_str() {
                    "string" | "key" => Kind::StringConstant,
                    "vector" | "rotation" | "list" => Kind::Compound,
                    _ => Kind::Constant,
                };
            }
        }
        if token.kind != Kind::Comment && !text.chars().all(char::is_whitespace) {
            next_code = text;
        }
    }
    // Labels and state names are not variables (jump label; @label; state name).
    let mut previous_code = "";
    for token in &mut result {
        let text = &source[token.bytes.clone()];
        if token.kind == Kind::Variable && matches!(previous_code, "state" | "jump" | "@") {
            token.kind = Kind::Section;
        }
        if token.kind != Kind::Comment && !text.chars().all(char::is_whitespace) {
            previous_code = text;
        }
    }
    result
}

/// Semantic colors shared with fenced LSL in notecards. Tokenize on the
/// document worker; resolve the current skin and custom colors when drawing.
pub(crate) fn highlight(source: &str) -> Vec<(Range<usize>, ColorRole)> {
    tokens(source).into_iter().map(|token| (token.bytes, token.kind.role())).collect()
}

#[derive(PartialEq)]
struct Style {
    font: FontId,
    colors: [Color32; 15],
    query: String,
    insensitive: bool,
    highlight: Color32,
}

impl Style {
    fn new(p: &Palette, font: FontId, settings: &ScriptSettings, query: &str, insensitive: bool) -> Self {
        Self {
            font,
            colors: [
                Kind::Plain,
                Kind::Variable,
                Kind::Keyword,
                Kind::Type,
                Kind::Event,
                Kind::Function,
                Kind::Constant,
                Kind::Number,
                Kind::String,
                Kind::Comment,
                Kind::Float,
                Kind::Section,
                Kind::StringConstant,
                Kind::Compound,
                Kind::Preprocessor,
            ]
            .map(|kind| settings.color(kind.role(), p)),
            query: query.into(),
            insensitive,
            highlight: p.amber.gamma_multiply(0.3),
        }
    }
}

/// Tokenization and formats are cached per document, including skin/font changes.
/// egui separately caches the shaped galley for the resulting layout job.
#[derive(Default)]
pub(super) struct Highlighter {
    style: Option<Style>,
    /// The keywords' generation the job was colored with
    /// (`lsl_keywords::generation`): a region's own keywords recolor it.
    keywords: u64,
    job: LayoutJob,
    pub colors: Vec<ColorLiteral>,
}

impl Highlighter {
    #[cfg(test)]
    pub(super) fn layout(&mut self, source: &str, p: &Palette, font: FontId) -> LayoutJob {
        self.custom(source, p, font, &ScriptSettings::default(), "", false)
    }

    pub(super) fn custom(
        &mut self,
        source: &str,
        p: &Palette,
        font: FontId,
        settings: &ScriptSettings,
        query: &str,
        insensitive: bool,
    ) -> LayoutJob {
        let style = Style::new(p, font, settings, query, insensitive);
        let keywords = lsl_keywords::generation();
        if self.style.as_ref() != Some(&style) || self.job.text != source || self.keywords != keywords {
            self.keywords = keywords;
            let mut job = LayoutJob {
                text: source.into(),
                ..Default::default()
            };
            let tokens = tokens(source);
            self.colors = color_literals(source, &tokens);
            for token in tokens {
                let format = TextFormat {
                    font_id: style.font.clone(),
                    color: style.colors[token.kind as usize],
                    ..Default::default()
                };
                if let Some(last) = job.sections.last_mut()
                    && last.format == format
                {
                    last.byte_range.end = token.bytes.end.into();
                } else {
                    job.sections.push(LayoutSection {
                        leading_space: 0.0,
                        byte_range: token.bytes.start.into()..token.bytes.end.into(),
                        format,
                    });
                }
            }
            if !query.is_empty() {
                let ranges = super::editing::matches(source, query, insensitive);
                let boundaries: Vec<_> = source.char_indices().map(|(i, _)| i).chain([source.len()]).collect();
                let mut highlights = ranges.into_iter().map(|r| boundaries[r.start]..boundaries[r.end]).peekable();
                let mut sections = Vec::new();
                for section in job.sections.drain(..) {
                    let mut start = section.byte_range.start.0;
                    let end = section.byte_range.end.0;
                    while start < end {
                        while highlights.peek().is_some_and(|r| r.end <= start) {
                            highlights.next();
                        }
                        let mut format = section.format.clone();
                        let limit = if let Some(range) = highlights.peek() {
                            if range.start <= start {
                                format.background = style.highlight;
                                end.min(range.end)
                            } else {
                                end.min(range.start)
                            }
                        } else {
                            end
                        };
                        sections.push(LayoutSection {
                            leading_space: 0.0,
                            byte_range: start.into()..limit.into(),
                            format,
                        });
                        start = limit;
                    }
                }
                job.sections = sections;
            }
            color_spacing(&mut job, &self.colors);
            self.job = job;
            self.style = Some(style);
        }
        self.job.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn detects_only_literal_rgb_vectors_including_endpoints_and_numeric_notation() {
        let source = "// éあ <0,1,0>\nstring s = \"<1,0,0>\"; /* <0,0,1> */\nvector a = <0, 1, .5>; vector b = <+0.1, 1e-1, -0.0>; vector c = <0x0, 0X1, 1.>;\nvector d = <0.25, /*green*/ .75,\n 1>;";
        let colors = color_literals(source, &tokens(source));
        assert_eq!(
            colors.iter().map(|c| c.rgb).collect::<Vec<_>>(),
            [[0.0, 1.0, 0.5], [0.1, 0.1, -0.0], [0.0, 1.0, 1.0], [0.25, 0.75, 1.0]]
        );
        for color in colors {
            assert!(source[color.bytes].starts_with('<'));
        }
        for literal in [
            "<-0.1,0,0>",
            "<1.000000001,0,0>",
            "<2,0,0>",
            "<1e999,0,0>",
            "<a,0,0>",
            "<0+1,0,0>",
            "<0,0,0,1>",
            "<0,0>",
            "<0,0,>",
            "<0,0,0.5foo>",
            "#define COLOR <0,1,0>",
        ] {
            assert!(color_literals(literal, &tokens(literal)).is_empty(), "{literal}");
        }
    }

    #[test]
    fn swatch_spacing_keeps_source_and_character_ranges_with_search_highlights() {
        let source = "vector c = <0,1,0>; vector d = <1,0,0>;";
        let mut highlighter = Highlighter::default();
        let job = highlighter.custom(
            source,
            &Theme::default().palette(),
            FontId::monospace(14.0),
            &ScriptSettings::default(),
            "0",
            false,
        );
        assert_eq!(job.text, source);
        assert_eq!(highlighter.colors.len(), 2);
        let spaces: Vec<_> = job.sections.iter().filter(|s| s.leading_space != 0.0).collect();
        assert_eq!(spaces.len(), 2);
        for section in spaces {
            assert_eq!(&source[section.byte_range.start.0..section.byte_range.start.0 + 1], "<");
        }
    }

    fn lex(source: &str) -> Vec<(&str, Kind)> {
        let tokens = tokens(source);
        let mut cursor = 0;
        for token in &tokens {
            assert_eq!(token.bytes.start, cursor);
            cursor = token.bytes.end;
        }
        assert_eq!(cursor, source.len());
        tokens.into_iter().map(|t| (&source[t.bytes], t.kind)).collect()
    }

    #[test]
    fn lsl_roles_include_custom_functions_and_do_not_guess_uppercase_constants() {
        let source = "integer COUNT = 1; string greet(key who) { return llKey2Name(who); } \
            default { touch_start(integer n) { if (n) llSay(PUBLIC_CHANNEL, greet /* call */ (NULL_KEY)); } }";
        let parsed = lex(source);
        for (text, kind) in [
            ("integer", Kind::Type),
            ("COUNT", Kind::Variable),
            ("who", Kind::Variable),
            ("greet", Kind::Function),
            ("llKey2Name", Kind::Function),
            ("llSay", Kind::Function),
            ("touch_start", Kind::Event),
            ("if", Kind::Keyword),
            ("return", Kind::Keyword),
            ("default", Kind::Section),
            ("PUBLIC_CHANNEL", Kind::Constant),
            ("NULL_KEY", Kind::StringConstant),
        ] {
            assert!(parsed.contains(&(text, kind)), "{text}: {parsed:?}");
            assert!(parsed.iter().filter(|(t, _)| *t == text).all(|(_, k)| *k == kind));
        }
        assert_eq!(keyword("llsay"), None);
        assert_eq!(keyword("rotation"), Some(Kind::Type));
        assert_eq!(keyword("ALL_SIDES"), Some(Kind::Constant));
        assert_eq!(keyword("timer"), Some(Kind::Event));
    }

    #[test]
    fn strings_escapes_and_comments_protect_keyword_lookalikes() {
        let source = "\"é\\\" llSay // not a comment\\\\\" /* if \" */ // integer\r\nname";
        assert_eq!(
            lex(source),
            vec![
                ("\"é\\\" llSay // not a comment\\\\\"", Kind::String),
                (" ", Kind::Plain),
                ("/* if \" */", Kind::Comment),
                (" ", Kind::Plain),
                ("// integer\r", Kind::Comment),
                ("\n", Kind::Plain),
                ("name", Kind::Variable),
            ]
        );
    }

    #[test]
    fn incomplete_input_and_utf8_keep_every_source_byte() {
        for source in [
            "",
            "\r\n\t",
            "/* 未完 é",
            "\"unfinished\\",
            "// ©\n\"世界\"",
            "élève += 1; 🐺",
            "foo /*unfinished",
        ] {
            lex(source);
        }
        assert_eq!(lex("/* 未完 é"), vec![("/* 未完 é", Kind::Comment)]);
        assert_eq!(lex("\"unfinished\\"), vec![("\"unfinished\\", Kind::String)]);
    }

    #[test]
    fn numeric_literals_do_not_consume_arithmetic_operators() {
        let parsed = lex("0xFF+1 .5 -1.25e-3 2E+4 3e 4.0-2");
        let numbers: Vec<_> = parsed
            .iter()
            .filter(|(_, k)| matches!(k, Kind::Number | Kind::Float))
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(numbers, ["0xFF", "1", ".5", "1.25e-3", "2E+4", "3", "4.0", "2"]);
        assert!(parsed.contains(&("+", Kind::Plain)));
        assert!(parsed.contains(&("-", Kind::Plain)));
        assert!(parsed.contains(&("e", Kind::Variable)));
        assert!(parsed.contains(&("1.25e-3", Kind::Float)));
        assert!(parsed.contains(&("0xFF", Kind::Number)));
    }

    #[test]
    fn state_names_and_jump_labels_have_section_colors() {
        for (text, kind) in lex("state /* next */ awake; jump done; @done; state awake { timer() {} }") {
            if matches!(text, "awake" | "done") {
                assert_eq!(kind, Kind::Section);
            }
        }
    }

    #[test]
    fn layout_preserves_text_and_refreshes_for_source_skin_and_font_changes() {
        let mut highlighter = Highlighter::default();
        let mut palette = Theme::default().palette();
        let source = "integer count = 0; // é\r\nllSay(0, \"Bonjour\");";
        let font = FontId::monospace(14.0);
        let job = highlighter.layout(source, &palette, font.clone());
        assert_eq!(job.text, source);
        assert_eq!(job.sections.first().expect("type").format.color, palette.rose);
        assert_eq!(job, highlighter.layout(source, &palette, font.clone()));
        palette.rose = palette.teal;
        let recolored = highlighter.layout(source, &palette, font);
        assert_eq!(recolored.sections[0].format.color, palette.teal);
        assert_eq!(recolored.text, source);
        let resized = highlighter.layout("float delay;", &palette, FontId::monospace(18.0));
        assert_eq!(resized.text, "float delay;");
        assert!(resized.sections.iter().all(|s| s.format.font_id.size == 18.0));
        assert!(resized.sections.iter().any(|s| s.format.color == palette.indigo_light));
    }
}
