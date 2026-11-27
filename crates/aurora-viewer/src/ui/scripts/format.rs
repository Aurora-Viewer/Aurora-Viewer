//! The canonical form of a script: four spaces a level, one brace a line where
//! it opens or closes an LSL block, one space between the tokens that want one
//! and none where they do not. What the Code menu's Format writes (Aurora's
//! own: neither the Legacy Viewer nor its preprocessor formats a script).
//!
//! The formatter moves white space only: it never reorders, adds or drops a
//! token, and a string, a comment or a preprocessor directive is copied as it
//! is, so the script it gives does what the script it had did. The lines of the
//! source are kept -- a statement is never joined to the one before it -- and a
//! blank line between two statements stays one.

use super::Lang;

use super::syntax::{self, Kind, Token};

/// The words a space comes before their bracket: `if (`, `while (`.
const SPACED: [&str; 5] = ["if", "while", "for", "switch", "return"];

/// The binary operators: a space on either side.
const BINARY: [&str; 31] = [
    "=", "+", "-", "*", "/", "%", "==", "!=", "~=", "<", ">", "<=", ">=", "&&", "||", "!", "&", "|", "^", "<<", ">>", "+=", "-=", "*=",
    "/=", "%=", "&=", "|=", "^=", "<<=", ">>=",
];

/// The operators and punctuation of two or three characters, longest first.
const SYMBOLS: [&str; 22] = [
    "<<=", ">>=", "==", "!=", "~=", "<=", ">=", "&&", "||", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "..", "::", "->",
];

/// The text `source` formatted, read as Lua when `lua`.
pub(super) fn text(source: &str, lang: Lang) -> String {
    // the one lexer the editor and the highlighter read: a string, a comment or
    // a directive is known here as it is there
    let tokens = syntax::tokens(source, lang);
    let mut out = Output {
        text: String::with_capacity(source.len() + 16),
        line: true,
        depth: 0,
        prev: None,
        before: None,
        cast: false,
        question: 0,
        parens: 0,
        angles: 0,
        lang,
    };
    // the byte where what is written so far ends
    let mut at = 0;
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let raw = &source[token.bytes.clone()];
        // the lexer's own white space: the gaps between the tokens carry it
        if raw.chars().all(char::is_whitespace) {
            index += 1;
            continue;
        }
        out.whitespace(&source[at..token.bytes.start]);
        let end = match token.kind {
            // a string or a comment: copied as it is, its own lines with it
            Kind::String | Kind::Comment => {
                out.verbatim(raw);
                token.bytes.end
            }
            // a preprocessor directive of LSL keeps its lines, continuations
            // too: a macro is not the formatter's to rewrite
            Kind::Preprocessor => {
                let end = directive_end(source, token);
                out.verbatim(&source[token.bytes.start..end]);
                // the directive swallowed its last line end: the code after it
                // starts a line
                if !out.text.ends_with('\n') {
                    out.text.push('\n');
                }
                out.line = true;
                end
            }
            // the code: a word, a number, or the longest operator here
            _ => {
                let end = if token.kind == Kind::Plain {
                    operator_end(source, &tokens, index)
                } else {
                    token.bytes.end
                };
                out.token(&source[token.bytes.start..end]);
                end
            }
        };
        at = end;
        index += 1;
        // the tokens of what was just written (a directive's continuations)
        while index < tokens.len() && tokens[index].bytes.start < end {
            index += 1;
        }
    }
    out.whitespace(&source[at..]);
    out.finish()
}

/// The end of the directive that starts at `token`: its `\` continuations are
/// its own lines.
fn directive_end(source: &str, token: &Token) -> usize {
    let mut start = token.bytes.start;
    let mut end = token.bytes.end;
    while end < source.len() && source[start..end].trim_end().ends_with('\\') {
        start = end + 1;
        end = source[start..].find('\n').map_or(source.len(), |n| start + n);
    }
    end
}

/// The end of the operator at the punctuation token `index`: the longest of
/// the two or three characters its neighbours spell (`==`, `<<=`), or the
/// character alone.
fn operator_end(source: &str, tokens: &[Token], index: usize) -> usize {
    let start = tokens[index].bytes.start;
    let mut limit = tokens[index].bytes.end;
    let mut at = index + 1;
    while let Some(next) = tokens.get(at) {
        if next.kind != Kind::Plain || next.bytes.start != limit {
            break;
        }
        if source[next.bytes.clone()].chars().all(char::is_whitespace) {
            break;
        }
        limit = next.bytes.end;
        at += 1;
    }
    for width in (2..=3).rev() {
        if start + width <= limit && source.is_char_boundary(start + width) {
            let text = &source[start..start + width];
            if SYMBOLS.contains(&text) || BINARY.contains(&text) {
                return start + width;
            }
        }
    }
    start + source[start..].chars().next().map_or(1, char::len_utf8)
}

/// A cast's type: `(integer)x` takes no space after its bracket.
/// Where the caret goes once `source` was rewritten as `formatted`: the same
/// character of the code -- the characters before `at`, their white space
/// aside -- in the text that replaced it.
pub(super) fn caret(source: &str, formatted: &str, at: usize) -> usize {
    let total = formatted.chars().count();
    if at >= source.chars().count() {
        return total;
    }
    let code = source.chars().take(at).filter(|c| !c.is_whitespace()).count();
    let mut seen = 0;
    for (index, c) in formatted.chars().enumerate() {
        if seen == code {
            return index;
        }
        if !c.is_whitespace() {
            seen += 1;
        }
    }
    total
}

/// What the code wrote: a word or a number, a symbol, a string.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Prev {
    Word(String),
    Symbol(String),
    Value,
}

impl Prev {
    fn text(&self) -> &str {
        match self {
            Prev::Word(text) | Prev::Symbol(text) => text,
            Prev::Value => "",
        }
    }

    fn opens(&self) -> bool {
        matches!(self, Prev::Symbol(text) if matches!(text.as_str(), "(" | "[" | "{"))
    }

    fn operator(&self) -> bool {
        matches!(self, Prev::Symbol(text) if is_operator(text))
    }

    fn value(&self) -> bool {
        match self {
            Prev::Word(_) | Prev::Value => true,
            Prev::Symbol(text) => matches!(text.as_str(), ")" | "]" | "}"),
        }
    }
}

struct Output {
    text: String,
    /// The next token starts a line: its indentation comes first.
    line: bool,
    depth: usize,
    prev: Option<Prev>,
    /// The token before `prev`: a `)` after a type in brackets closes a cast.
    before: Option<Prev>,
    /// The `)` just written closed a cast: no space after it.
    cast: bool,
    /// The `?`s still waiting for their `:` on this line.
    question: u32,
    /// The brackets still open: a `;` inside them keeps its statement's line.
    parens: u32,
    /// A `<` that opened a vector or a rotation, against a comparison.
    angles: u32,
    lang: Lang,
}

impl Output {
    fn indent(&mut self) {
        if self.line {
            for _ in 0..self.depth {
                self.text.push_str("    ");
            }
            self.line = false;
        }
    }

    fn push(&mut self, token: &str) {
        let space = !self.line && self.space_before(token);
        self.indent();
        if space {
            self.text.push(' ');
        }
        self.text.push_str(token);
    }

    /// A string, a comment or a directive: as it is, its lines with it, and no
    /// token of the code after it.
    fn verbatim(&mut self, text: &str) {
        let space = !self.line && self.space_before("value");
        self.indent();
        if space {
            self.text.push(' ');
        }
        self.text.push_str(text);
        self.line = text.ends_with('\n');
        self.prev = Some(Prev::Value);
        self.before = None;
        self.cast = false;
        self.question = 0;
    }

    /// The white space of the source between two tokens: its line breaks, its
    /// spaces spent by the tokens that follow.
    fn whitespace(&mut self, text: &str) {
        let newlines = text.chars().filter(|c| *c == '\n').count();
        if newlines > 0 {
            self.newlines(newlines.min(2));
        }
    }

    fn newlines(&mut self, count: usize) {
        while self.text.ends_with(' ') {
            self.text.pop();
        }
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
        if count > 1 && !self.text.is_empty() && !self.text.ends_with("\n\n") {
            self.text.push('\n');
        }
        self.line = true;
        self.question = 0;
    }

    fn token(&mut self, token: &str) {
        if !self.lang.is_lua() && matches!(token, "{" | "}") {
            self.brace(token);
            return;
        }
        if self.lang.is_lua() {
            // `}` and `end` close a block before their own line is indented
            if matches!(token, "}" | "end" | "until" | "else" | "elseif") {
                self.depth = self.depth.saturating_sub(1);
            }
        }
        match token {
            "(" | "[" => self.parens += 1,
            ")" | "]" => self.parens = self.parens.saturating_sub(1),
            _ => {}
        }
        // `(integer)`: a type in brackets is a cast, and no space follows it
        let cast = token == ")"
            && matches!(&self.prev, Some(Prev::Word(word)) if syntax::is_type(word))
            && matches!(&self.before, Some(Prev::Symbol(open)) if open == "(");
        self.push(token);
        self.cast = cast;
        match token {
            "?" => self.question += 1,
            ":" if self.question > 0 => self.question -= 1,
            _ => {}
        }
        if token == "<" && !self.prev.as_ref().is_some_and(Prev::value) {
            self.angles += 1;
        } else if token == ">" && self.angles > 0 {
            self.angles -= 1;
        }
        if self.lang.is_lua() {
            if matches!(token, "{" | "function" | "do" | "if" | "repeat" | "else" | "elseif") {
                self.depth += 1;
            }
        } else if token == ";" && self.parens == 0 {
            self.newlines(1);
        }
        self.before = self.prev.take();
        self.prev = Some(match token {
            "(" | ")" | "[" | "]" | "{" | "}" | "." | "," | ";" => Prev::Symbol(token.to_owned()),
            _ if is_operator(token) => Prev::Symbol(token.to_owned()),
            _ => Prev::Word(token.to_owned()),
        });
    }

    /// A brace of an LSL block: one a line, its body a level in, the brace that
    /// closes it level with the line that opened it.
    fn brace(&mut self, token: &str) {
        if token == "{" {
            if !self.line {
                self.newlines(1);
            }
            self.indent();
            self.text.push('{');
            self.depth += 1;
            self.newlines(1);
        } else {
            self.depth = self.depth.saturating_sub(1);
            if !self.line {
                self.newlines(1);
            }
            self.indent();
            self.text.push('}');
            self.newlines(1);
        }
        self.before = self.prev.take();
        self.prev = Some(Prev::Symbol(token.to_owned()));
        self.cast = false;
        self.question = 0;
        self.parens = 0;
        self.angles = 0;
    }

    fn space_before(&self, token: &str) -> bool {
        let Some(prev) = &self.prev else {
            return false;
        };
        if matches!(token, ")" | "]" | "," | ";") {
            return false;
        }
        // a Lua table's braces take a space inside, `{ a, b }`
        if token == "}" {
            return self.lang.is_lua() && !prev.opens();
        }
        if token == "." || (prev.opens() && !(self.lang.is_lua() && prev.text() == "{")) {
            return false;
        }
        // a label: `@again;` keeps its name against the sign
        if prev.text() == "@" {
            return false;
        }
        if token == ":" {
            return self.question > 0;
        }
        if token == "(" {
            return SPACED.contains(&prev.text()) || prev.operator() || matches!(prev.text(), "," | ";");
        }
        if token == "[" {
            // an index and a subscript stay on their value, a list opens after
            // the operator or the comma that announced it
            return !prev.value();
        }
        if self.cast {
            return is_operator(token);
        }
        if token == "<" && !prev.value() {
            // a vector or a rotation's opener: the space before it is its
            // neighbour's
            return true;
        }
        if self.angles > 0 && prev.text() == "<" {
            return false;
        }
        if token == ">" && self.angles > 0 {
            return false;
        }
        if Self::unary(token, self.prev.as_ref()) {
            return !prev.opens();
        }
        if prev.operator() {
            return !Self::unary(prev.text(), self.before.as_ref());
        }
        true
    }

    /// A `-`, `+`, `!`, `~` or `#` with no value before it is unary: no space
    /// after it.
    fn unary(token: &str, before: Option<&Prev>) -> bool {
        if !matches!(token, "-" | "+" | "!" | "~" | "#") {
            return false;
        }
        let Some(prev) = before else {
            return true;
        };
        prev.opens() || prev.operator() || matches!(prev.text(), "," | ";" | "?" | ":") || SPACED.contains(&prev.text())
    }

    fn finish(mut self) -> String {
        while self.text.ends_with(' ') || self.text.ends_with('\n') {
            self.text.pop();
        }
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text
    }
}

fn is_operator(token: &str) -> bool {
    BINARY.contains(&token)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines of `source`, formatted as LSL.
    fn lsl(source: &str) -> String {
        text(source, Lang::Lsl)
    }

    #[test]
    fn a_messy_script_gets_the_canonical_lsl_form() {
        let source = "default{state_entry(){llSay(0,\"hi\");integer i=1+2*3;if(i>4){i=i-1;}else{i=i+1;}list l=[1,2];vector v=<1,2,3>;float f=(float)i;}}\n";
        assert_eq!(
            lsl(source),
            "default\n{\n    state_entry()\n    {\n        llSay(0, \"hi\");\n        integer i = 1 + 2 * 3;\n        if (i > 4)\n        {\n            i = i - 1;\n        }\n        else\n        {\n            i = i + 1;\n        }\n        list l = [1, 2];\n        vector v = <1, 2, 3>;\n        float f = (float)i;\n    }\n}\n"
        );
    }

    /// A label, a `do` and its `while`, and a `switch`: the lines the source
    /// kept stay, the braces that open or close a block get theirs.
    #[test]
    fn labels_do_while_and_switch_get_their_lines() {
        let source = "integer f(){@again;do{llSleep(0.1);}while(1>2);switch(1){case 1:jump again;default:return 0;}}\n";
        assert_eq!(
            lsl(source),
            "integer f()\n{\n    @again;\n    do\n    {\n        llSleep(0.1);\n    }\n    while (1 > 2);\n    switch (1)\n    {\n        case 1: jump again;\n        default: return 0;\n    }\n}\n"
        );
    }

    #[test]
    fn a_lua_script_gets_the_canonical_lua_form() {
        let source = "local t={a=1,b=2}\nfunction f(x)\nif x>1 then\nreturn x\nelseif x<0 then\nreturn -x\nelse\nfor i=1,3 do\nt[i]=i\nend\nend\nend\n";
        assert_eq!(
            text(source, Lang::Lua),
            "local t = { a = 1, b = 2 }\nfunction f(x)\n    if x > 1 then\n        return x\n    elseif x < 0 then\n        return -x\n    else\n        for i = 1, 3 do\n            t[i] = i\n        end\n    end\nend\n"
        );
    }

    /// A string, a comment and a directive are copied as they are: the
    /// formatter moves the white space of the code alone.
    #[test]
    fn strings_comments_and_directives_are_left_as_they_are() {
        let source = "#define FOO(a) (a\\\n +1)\n// keep   this\ndefault{ state_entry(){ string s = \"a  b // not a comment\"; /* multi\nline */ llSay(0,s); } }\n";
        let formatted = lsl(source);
        assert!(formatted.starts_with("#define FOO(a) (a\\\n +1)\n// keep   this\n"), "{formatted}");
        assert!(formatted.contains("\"a  b // not a comment\""));
        assert!(formatted.contains("/* multi\nline */"));
        assert!(formatted.ends_with("llSay(0, s);\n    }\n}\n"), "{formatted}");
    }

    /// A long string and a long comment of Lua keep their bytes, as the
    /// lexer the editor reads tells them.
    #[test]
    fn a_lua_long_string_and_comment_keep_their_bytes() {
        let source = "local s=[[a  b]]\n--[[ keep   this ]]\nlocal t={1,2}\n";
        assert_eq!(
            text(source, Lang::Lua),
            "local s = [[a  b]]\n--[[ keep   this ]]\nlocal t = { 1, 2 }\n"
        );
    }

    /// Formatting a script the formatter wrote changes nothing: the same script
    /// gives the same text, whichever form it came in.
    #[test]
    fn formatting_twice_changes_nothing() {
        for (source, lang) in [
            ("default{state_entry(){if(1){llSay(0,\"x\");}else{llSay(0,\"y\");}}}\n", Lang::Lsl),
            (
                "#define X 1\ninteger f(integer a){return a*2;}\ndefault{state_entry(){}}\n",
                Lang::Lsl,
            ),
            ("local t={1,2}\nif t[1] then\nreturn {x=1}\nend\n", Lang::Lua),
        ] {
            let once = text(source, lang);
            assert_eq!(text(&once, lang), once, "{source}");
        }
    }

    /// The caret stays on the same character of the code once the white space
    /// around it moved.
    #[test]
    fn the_caret_stays_on_the_same_character_of_the_code() {
        let source = "a  =  b\n";
        let formatted = lsl(source);
        assert_eq!(formatted, "a = b\n");
        assert_eq!(caret(source, &formatted, 0), 0);
        // the caret after `=`: the code's second character, then what follows it
        assert_eq!(caret(source, &formatted, 4), 3);
        // the end of the text stays its end
        assert_eq!(caret(source, &formatted, source.chars().count()), formatted.chars().count());
    }
}
