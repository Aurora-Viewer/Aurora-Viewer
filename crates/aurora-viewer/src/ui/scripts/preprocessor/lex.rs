//! Tokenization for the Firestorm LSL preprocessor rules. Byte offsets always
//! stay on UTF-8 boundaries; strings/comments are indivisible for transforms.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Word,
    Number,
    String,
    Comment,
    Space,
    Symbol,
    Empty,
}

#[derive(Clone, Debug)]
pub(super) struct Token {
    pub text: String,
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

pub(super) fn lex(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut line = 1;
    while at < source.len() {
        let start = at;
        let c = source[at..].chars().next().expect("UTF-8 token boundary");
        let kind = if source[at..].starts_with("//") {
            at = source[at..].find('\n').map_or(source.len(), |i| at + i);
            Kind::Comment
        } else if source[at..].starts_with("/*") {
            at = source[at + 2..].find("*/").map_or(source.len(), |i| at + i + 4);
            Kind::Comment
        } else if c == '"' || c == '\'' {
            at += 1;
            while at < source.len() {
                let next = source[at..].chars().next().expect("string boundary");
                at += next.len_utf8();
                if next == c {
                    break;
                }
                if next == '\\'
                    && let Some(escaped) = source[at..].chars().next()
                {
                    at += escaped.len_utf8();
                }
            }
            Kind::String
        } else if c.is_whitespace() {
            at += c.len_utf8();
            while let Some(next) = source[at..].chars().next() {
                if !next.is_whitespace() {
                    break;
                }
                at += next.len_utf8();
            }
            Kind::Space
        } else if c == '_' || c.is_alphabetic() {
            at += c.len_utf8();
            while let Some(next) = source[at..].chars().next() {
                if next != '_' && !next.is_alphanumeric() {
                    break;
                }
                at += next.len_utf8();
            }
            Kind::Word
        } else if c.is_ascii_digit() {
            at += 1;
            while let Some(next) = source[at..].chars().next() {
                if !next.is_ascii_alphanumeric() && next != '.' {
                    break;
                }
                at += next.len_utf8();
            }
            Kind::Number
        } else {
            at += c.len_utf8();
            if [
                "##", "&&", "||", "<<", ">>", "==", "!=", "<=", ">=", "++", "--", "+=", "-=", "*=", "/=", "&=", "|=", "^=", "...",
            ]
            .iter()
            .any(|s| source[start..].starts_with(s))
            {
                at = start + if source[start..].starts_with("...") { 3 } else { 2 };
            }
            Kind::Symbol
        };
        tokens.push(Token {
            text: source[start..at].into(),
            kind,
            start,
            end: at,
            line,
        });
        line += source[start..at].bytes().filter(|b| *b == b'\n').count();
    }
    tokens
}

pub(super) fn code(source: &str) -> Vec<Token> {
    lex(source)
        .into_iter()
        .filter(|t| !matches!(t.kind, Kind::Space | Kind::Comment))
        .collect()
}

pub(super) fn join(tokens: &[Token]) -> String {
    let mut output = String::new();
    let mut previous: Option<&Token> = None;
    for token in tokens {
        if let Some(left) = previous {
            let merged_word = matches!(left.kind, Kind::Word | Kind::Number) && matches!(token.kind, Kind::Word | Kind::Number);
            let merged_symbol = left.kind == Kind::Symbol
                && token.kind == Kind::Symbol
                && matches!(
                    format!("{}{}", left.text, token.text).as_str(),
                    "++" | "--"
                        | "&&"
                        | "||"
                        | "<<"
                        | ">>"
                        | "<="
                        | ">="
                        | "=="
                        | "!="
                        | "+="
                        | "-="
                        | "*="
                        | "/="
                        | "&="
                        | "|="
                        | "^="
                        | "##"
                        | "//"
                        | "/*"
                );
            if merged_word || merged_symbol {
                output.push(' ');
            }
        }
        output.push_str(&token.text);
        previous = Some(token);
    }
    output
}

pub(super) fn close(tokens: &[Token], start: usize, left: &str, right: &str) -> Option<usize> {
    if tokens.get(start)?.text != left {
        return None;
    }
    let mut depth = 0;
    for (i, token) in tokens.iter().enumerate().skip(start) {
        if token.kind != Kind::Symbol {
            continue;
        }
        if token.text == left {
            depth += 1;
        }
        if token.text == right {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

pub(super) fn quoted(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
