//! LSL token colors, following LLKeywords::findSegments / processTokensGroup
//! (Firestorm indra/llui/llkeywords.cpp, originally LGPL 2.1). Built-in names
//! come from Firestorm's keywords_lsl_default.xml. Custom functions are also
//! recognized at declarations and calls. This lexer never changes the source.

use crate::theme::Palette;
use egui::{
    Color32, FontId, TextFormat,
    text::{LayoutJob, LayoutSection},
};
use serde::Deserialize;
use std::{collections::HashMap, ops::Range, sync::OnceLock};

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
}

#[derive(Deserialize)]
struct Catalog {
    controls: Vec<String>,
    types: Vec<String>,
    events: Vec<String>,
    functions: Vec<String>,
    constants: Vec<String>,
}

fn keywords() -> &'static HashMap<String, Kind> {
    static WORDS: OnceLock<HashMap<String, Kind>> = OnceLock::new();
    WORDS.get_or_init(|| {
        // Invariant: this JSON is a bundled, tested build-time asset, not input.
        let catalog: Catalog =
            serde_json::from_str(include_str!("../../../assets/lsl/keywords.json")).expect("valid bundled LSL keyword catalog");
        let mut words = HashMap::new();
        for (names, kind) in [
            (catalog.controls, Kind::Keyword),
            (catalog.types, Kind::Type),
            (catalog.events, Kind::Event),
            (catalog.functions, Kind::Function),
            (catalog.constants, Kind::Constant),
        ] {
            words.extend(names.into_iter().map(|name| (name, kind)));
        }
        words.insert("default".into(), Kind::Keyword);
        words
    })
}

struct Token {
    bytes: Range<usize>,
    kind: Kind,
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
            Kind::Number
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
    let words = keywords();
    for token in result.iter_mut().rev() {
        let text = &source[token.bytes.clone()];
        if token.kind == Kind::Variable {
            token.kind = words
                .get(text)
                .copied()
                .unwrap_or(if next_code == "(" { Kind::Function } else { Kind::Variable });
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
            token.kind = Kind::Keyword;
        }
        if token.kind != Kind::Comment && !text.chars().all(char::is_whitespace) {
            previous_code = text;
        }
    }
    result
}

#[derive(PartialEq)]
struct Style {
    font: FontId,
    colors: [Color32; 10],
}

impl Style {
    fn new(p: &Palette, font: FontId) -> Self {
        Self {
            font,
            colors: [
                p.ink,
                p.indigo_light,
                p.violet_light,
                p.rose,
                p.violet_pale,
                p.teal,
                p.amber,
                p.warn,
                p.success,
                p.muted,
            ],
        }
    }
}

/// Tokenization and formats are cached per document, including skin/font changes.
/// egui separately caches the shaped galley for the resulting layout job.
#[derive(Default)]
pub(super) struct Highlighter {
    style: Option<Style>,
    job: LayoutJob,
}

impl Highlighter {
    pub(super) fn layout(&mut self, source: &str, p: &Palette, font: FontId) -> LayoutJob {
        let style = Style::new(p, font);
        if self.style.as_ref() != Some(&style) || self.job.text != source {
            let mut job = LayoutJob {
                text: source.into(),
                ..Default::default()
            };
            for token in tokens(source) {
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
            ("default", Kind::Keyword),
            ("PUBLIC_CHANNEL", Kind::Constant),
            ("NULL_KEY", Kind::Constant),
        ] {
            assert!(parsed.contains(&(text, kind)), "{text}: {parsed:?}");
            assert!(parsed.iter().filter(|(t, _)| *t == text).all(|(_, k)| *k == kind));
        }
        assert_eq!(keywords().get("llsay"), None);
        assert_eq!(keywords().get("rotation"), Some(&Kind::Type));
        assert_eq!(keywords().get("ALL_SIDES"), Some(&Kind::Constant));
        assert_eq!(keywords().get("timer"), Some(&Kind::Event));
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
        let numbers: Vec<_> = parsed.iter().filter(|(_, k)| *k == Kind::Number).map(|(t, _)| *t).collect();
        assert_eq!(numbers, ["0xFF", "1", ".5", "1.25e-3", "2E+4", "3", "4.0", "2"]);
        assert!(parsed.contains(&("+", Kind::Plain)));
        assert!(parsed.contains(&("-", Kind::Plain)));
        assert!(parsed.contains(&("e", Kind::Variable)));
    }

    #[test]
    fn state_names_and_jump_labels_have_keyword_colors() {
        for (text, kind) in lex("state /* next */ awake; jump done; @done; state awake { timer() {} }") {
            if matches!(text, "awake" | "done") {
                assert_eq!(kind, Kind::Keyword);
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
