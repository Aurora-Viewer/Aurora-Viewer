//! Completion of LSL and SLua names: the words
//! of the keywords in use (the region's own syntax included) and the words the
//! script itself writes, offered under the caret as it is typed, with the
//! signature and the help of the one chosen.
//!
//! The list belongs to its field, as the emoji codes' list does: it lives in
//! the frame's memory under the field's id, takes the arrows, Enter, Tab and
//! Escape before the field sees them, and writes its choice in the source as
//! an edit the undoer knows.

use super::Lang;

use super::editing;
use crate::theme::Palette;
use crate::ui::{menu, widgets};
use egui::text_edit::TextEditOutput;
use egui::{Id, RichText};
use std::ops::Range;

/// The least letters typed before the list opens: one letter would offer half
/// the language.
const MIN_TYPED: usize = 2;

/// The most names the list holds.
const MOST: usize = 64;

/// The room the list gives its rows before it scrolls.
const ROWS_HEIGHT: f32 = 180.0;

/// The room under the rows for the signature and the help of the one chosen:
/// the same for every name, so the list does not resize as the choice moves.
const DETAIL_HEIGHT: f32 = 58.0;

/// What a suggestion is: a word of the script, or a kind of the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    /// A name the script declares or uses: nothing the catalog knows.
    Name,
    /// A body the list writes in place of the word, its placeholders to fill.
    Snippet,
    Function,
    Event,
    Type,
    Constant,
    Control,
}

/// One name the list offers.
#[derive(Clone)]
pub(super) struct Item {
    pub text: String,
    pub kind: Kind,
    /// Its signature, as the catalog writes it (empty for a name of the script).
    pub detail: String,
    /// What it does (empty for a word of the script).
    pub docs: String,
    /// A name the script declares (a parameter, a local, a global, a function):
    /// it stands before the catalog's, and its help is its declaration.
    own: bool,
    /// The body a snippet writes, with its `${1:…}` placeholders where the
    /// name goes.
    snippet: Option<&'static str>,
    /// Its letters in one case, so that `llSay` comes before `llSHA1String`.
    lower: String,
    /// The language deprecated this name: the row strikes it through.
    deprecated: bool,
}

impl Kind {
    fn of(group: &str) -> Kind {
        match group {
            "functions" => Kind::Function,
            "events" => Kind::Event,
            "types" => Kind::Type,
            "constants" => Kind::Constant,
            _ => Kind::Control,
        }
    }

    /// The Tabler icon of the kind.
    fn icon(self) -> &'static str {
        match self {
            Kind::Name | Kind::Constant => "variable",
            Kind::Snippet => "braces",
            Kind::Function => "function-filled",
            Kind::Event => "bolt-filled",
            Kind::Type => "braces",
            Kind::Control => "code",
        }
    }
}

/// The name the caret is in: the letters typed so far, and the whole word an
/// accepted name replaces.
pub(super) struct Typed {
    /// The letters before the caret, in char indices.
    pub range: Range<usize>,
    /// The letters typed so far.
    pub text: String,
    /// The whole word around the caret.
    pub word: Range<usize>,
}

fn is_ident(ch: char, lang: Lang) -> bool {
    let lua = lang.is_lua();
    ch == '_' || ch.is_alphanumeric() || (lua && ch == '.')
}

/// What is being typed at `at` (a caret position in char indices), or None
/// when the caret is not in a name, is in a comment or a string, or the name
/// starts with a digit.
pub(super) fn typed(source: &str, at: usize, lang: Lang) -> Option<Typed> {
    let chars: Vec<char> = source.chars().collect();
    let at = at.min(chars.len());
    let start = chars[..at].iter().rposition(|ch| !is_ident(*ch, lang)).map_or(0, |index| index + 1);
    if start == at || chars[start].is_ascii_digit() {
        return None;
    }
    let mut end = at;
    while end < chars.len() && is_ident(chars[end], lang) {
        end += 1;
    }
    // the letters of a name, never the text of a comment or a string
    let code = super::syntax::code_chars(source, lang);
    if !(start..end).all(|index| code.get(index).copied().unwrap_or(false)) {
        return None;
    }
    Some(Typed {
        range: start..at,
        text: chars[start..at].iter().collect(),
        word: start..end,
    })
}

/// The names that start with what is typed: the catalog's (the region's own
/// syntax included) and the script's own words, the exact case first, then the
/// script's names before the catalog's kinds, alphabetically within each.
pub(super) fn candidates(lang: Lang, source: &str, at: usize, typed: &str, limit: usize) -> Vec<Item> {
    let catalog = super::syntax::catalog(lang);
    let lowered = typed.to_lowercase();
    let chars: Vec<char> = source.chars().collect();
    let code = super::syntax::code_chars(source, lang);
    let mut items: Vec<Item> = Vec::new();
    for entry in catalog.entries() {
        if !entry.name.starts_with(typed) && !entry.name.to_lowercase().starts_with(&lowered) {
            continue;
        }
        items.push(Item {
            text: entry.name.clone(),
            kind: Kind::of(&entry.group),
            detail: entry.signature.clone(),
            docs: entry.tooltip.clone(),
            own: false,
            snippet: None,
            lower: entry.name.to_lowercase(),
            deprecated: entry.deprecated,
        });
    }
    for item in declared(source, &chars, &code, at, lang) {
        if item.text.starts_with(typed) || item.lower.starts_with(&lowered) {
            items.push(item);
        }
    }
    items.extend(snippets(typed, lang));
    let declared = every_declared(source, &chars, &code, lang);
    for name in words(&chars, &code, lang) {
        // the letters being typed are not one of the choices: they are there
        if name == typed || catalog.entry(&name).is_some() {
            continue;
        }
        // a name the script declares is offered where it holds, never as a
        // plain word of the file
        if declared.contains(&name) {
            continue;
        }
        if !name.starts_with(typed) && !name.to_lowercase().starts_with(&lowered) {
            continue;
        }
        items.push(Item {
            lower: name.to_lowercase(),
            text: name,
            kind: Kind::Name,
            detail: String::new(),
            docs: String::new(),
            own: false,
            snippet: None,
            deprecated: false,
        });
    }
    items.sort_by(|a, b| order(typed, a).cmp(&order(typed, b)));
    items.dedup_by(|a, b| a.text == b.text);
    items.truncate(limit);
    items
}

/// Where a name stands in the list: the exact word first, then the letters in
/// the same case, the script's own declarations before the catalog's, then the
/// kind, then the letters, so that `llSay` comes before `llSHA1String`.
fn order<'a>(typed: &str, item: &'a Item) -> (u8, bool, Kind, &'a str, &'a str) {
    let case = if item.text == typed {
        0
    } else if item.text.starts_with(typed) {
        1
    } else {
        2
    };
    (case, !item.own, item.kind, item.lower.as_str(), item.text.as_str())
}

/// The snippets the list writes with their name: a body
/// with `${1:…}` placeholders and the source line's indentation put in front of
/// its lines.
const SNIPPETS: &[(&str, &str)] = &[
    ("if", "if (${1:condition})\n{\n    ${2}\n}"),
    ("for", "for (${1:integer i = 0}; ${2:i < count}; ${3:++i})\n{\n    ${4}\n}"),
    ("while", "while (${1:condition})\n{\n    ${2}\n}"),
    ("do", "do\n{\n    ${1}\n} while (${2:condition});"),
    ("state", "state ${1:name}\n{\n    ${2}\n}"),
];

/// The same for Lua, whose blocks end with `end` (or an `until`) instead of a
/// brace, and whose functions take their parameters without a type.
const LUA_SNIPPETS: &[(&str, &str)] = &[
    ("if", "if ${1:condition} then\n    ${2}\nend"),
    ("for", "for ${1:i} = 1, ${2:count} do\n    ${3}\nend"),
    ("pairs", "for ${1:key}, ${2:value} in pairs(${3:table}) do\n    ${4}\nend"),
    ("while", "while ${1:condition} do\n    ${2}\nend"),
    ("repeat", "repeat\n    ${1}\nuntil ${2:condition}"),
    ("do", "do\n    ${1}\nend"),
    ("function", "function ${1:name}(${2})\n    ${3}\nend"),
    ("local", "local ${1:name} = ${2:value}"),
    ("localfunction", "local function ${1:name}(${2})\n    ${3}\nend"),
];

/// The snippet names that start with what is typed.
fn snippets(typed: &str, lang: Lang) -> Vec<Item> {
    let lua = lang.is_lua();
    let lowered = typed.to_lowercase();
    let names = if lua { LUA_SNIPPETS } else { SNIPPETS };
    names
        .iter()
        .filter(|(name, _)| name.starts_with(typed) || name.to_lowercase().starts_with(&lowered))
        .map(|(name, body)| Item {
            lower: name.to_lowercase(),
            text: (*name).to_owned(),
            kind: Kind::Snippet,
            detail: String::new(),
            docs: String::new(),
            own: false,
            snippet: Some(body),
            deprecated: false,
        })
        .collect()
}

/// The LSL types a declaration starts with.
const TYPES: [&str; 7] = ["integer", "float", "string", "key", "vector", "rotation", "list"];

/// The names the script declares and that hold at `at`: the parameters and the
/// locals of the blocks the caret is in, and the globals and functions of the
/// file, which hold everywhere.
fn declared(source: &str, chars: &[char], code: &[bool], at: usize, lang: Lang) -> Vec<Item> {
    let mut items = Vec::new();
    for item in super::outline::outline(source, lang) {
        let kind = match item.kind {
            super::outline::Kind::Function | super::outline::Kind::LocalFunction | super::outline::Kind::Method => Kind::Function,
            super::outline::Kind::Global => Kind::Name,
            // a state is reached by `state` and an event by its call
            super::outline::Kind::State | super::outline::Kind::Event => continue,
        };
        items.push(own(item.name, kind, String::new()));
    }
    // the blocks the caret is in, outermost first; each one's own names, the
    // deeper blocks' being theirs (a local of an event does not reach another)
    let blocks = super::crumbs::blocks(chars, code);
    let holding: Vec<&super::crumbs::Block> = blocks.iter().filter(|block| block.open < at && at <= block.close).collect();
    for block in &holding {
        items.extend(parameters(chars, block, lang));
        items.extend(locals(chars, code, block, at, lang, &blocks));
    }
    items
}

/// Every name the script declares, wherever it holds: a word of the file that
/// is one of them is not offered as a plain word.
fn every_declared(source: &str, chars: &[char], code: &[bool], lang: Lang) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for item in super::outline::outline(source, lang) {
        names.insert(item.name);
    }
    let blocks = super::crumbs::blocks(chars, code);
    for block in &blocks {
        for item in parameters(chars, block, lang) {
            names.insert(item.text);
        }
        for item in locals(chars, code, block, block.close, lang, &blocks) {
            names.insert(item.text);
        }
    }
    names
}

/// A name the script declares.
fn own(name: String, kind: Kind, detail: String) -> Item {
    Item {
        lower: name.to_lowercase(),
        text: name,
        kind,
        detail,
        docs: String::new(),
        own: true,
        snippet: None,
        deprecated: false,
    }
}

/// The parameters of a block whose header declares one: `touch_start(integer
/// total)`, `integer add(integer a, integer b)`. A condition (`if`, `for`,
/// `while`, `else`, a state) declares none.
fn parameters(chars: &[char], block: &super::crumbs::Block, lang: Lang) -> Vec<Item> {
    let lua = lang.is_lua();
    let header = &chars[block.header.clone()];
    let mut items = Vec::new();
    let Some(open) = header.iter().position(|ch| *ch == '(') else {
        return items;
    };
    let before: String = header[..open].iter().collect();
    let words: Vec<&str> = before.split_whitespace().collect();
    if words.is_empty() || words.len() > 2 {
        return items;
    }
    if matches!(words[0], "if" | "for" | "while" | "switch" | "do" | "else" | "state" | "default")
        || words.iter().any(|word| !word.chars().all(|ch| ch == '_' || ch.is_alphanumeric()))
    {
        return items;
    }
    // the declaration's brackets, to their match
    let mut depth = 0;
    let mut end = header.len();
    for (index, ch) in header.iter().enumerate().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = index;
                    break;
                }
            }
            _ => {}
        }
    }
    let inside: String = header[open + 1..end].iter().collect();
    for part in inside.split(',') {
        let part = part.trim();
        let Some(name) = part.split_whitespace().last() else {
            continue;
        };
        if part == "void" || !name.chars().all(|ch| ch == '_' || ch.is_alphanumeric()) {
            continue;
        }
        // `integer total` keeps its type; a Lua parameter is only its name
        let detail = if lua { String::new() } else { part.to_owned() };
        items.push(own(name.to_owned(), Kind::Name, detail));
    }
    items
}

/// The locals a block declares before `at`: `integer count = 0;`,
/// `for (integer i = 0; …)`, and SLua's `local count = 0`.
fn locals(chars: &[char], code: &[bool], block: &super::crumbs::Block, at: usize, lang: Lang, all: &[super::crumbs::Block]) -> Vec<Item> {
    let lua = lang.is_lua();
    let mut items = Vec::new();
    let word = |from: usize| -> (String, usize) {
        let mut end = from;
        while end < chars.len() && (chars[end] == '_' || chars[end].is_alphanumeric()) {
            end += 1;
        }
        (chars[from..end].iter().collect(), end)
    };
    let mut index = block.open + 1;
    while index < at.min(block.close) {
        // a block inside this one holds its own names
        if let Some(child) = all.iter().find(|child| child.open == index) {
            index = child.close + 1;
            continue;
        }
        if !code[index] || !(chars[index] == '_' || chars[index].is_alphanumeric()) {
            index += 1;
            continue;
        }
        let (first, after_first) = word(index);
        // a declaration starts where a statement does
        let boundary = chars[..index]
            .iter()
            .zip(&code[..index])
            .rev()
            .find(|(ch, code)| **code && !ch.is_whitespace())
            .is_none_or(|(ch, _)| matches!(ch, '{' | '}' | ';' | '(' | ','));
        if lua && first == "local" && boundary {
            let mut next = after_first;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            let (second, _) = word(next);
            let name = if second == "function" {
                let mut after = next + second.len();
                while after < chars.len() && chars[after].is_whitespace() {
                    after += 1;
                }
                word(after).0
            } else {
                second
            };
            if !name.is_empty() {
                items.push(own(name, Kind::Name, String::new()));
            }
        } else if !lua && TYPES.contains(&first.as_str()) && boundary {
            let mut next = after_first;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            let (name, after_name) = word(next);
            let mut after = after_name;
            while after < chars.len() && chars[after].is_whitespace() {
                after += 1;
            }
            // `integer count = 0;` rather than `integer add(...)`, a function
            let declares = !name.is_empty() && matches!(chars.get(after), Some('=') | Some(';') | Some(','));
            if declares {
                items.push(own(name.clone(), Kind::Name, format!("{first} {name}")));
            }
        }
        index = after_first.max(index + 1);
    }
    items
}

/// The names the script writes, in the code (not in its comments or strings),
/// each once, by the letters they start with.
fn words(chars: &[char], code: &[bool], lang: Lang) -> Vec<String> {
    let mut found = std::collections::BTreeSet::new();
    let mut at = 0;
    while at < chars.len() {
        if !code[at] || !is_ident(chars[at], lang) || chars[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && is_ident(chars[at], lang) && code[at] {
            at += 1;
        }
        if at - start >= 2 {
            found.insert(chars[start..at].iter().collect::<String>());
        }
    }
    found.into_iter().collect()
}

/// A snippet the reader is filling: where its placeholders are in the source,
/// the one the caret is on, and the source's length when the body went in, so
/// that an edit moves what follows it.
#[derive(Clone, Default)]
pub(super) struct Snippet {
    stops: Vec<Range<usize>>,
    current: usize,
    /// Where the caret goes once the last placeholder is filled.
    end: usize,
    /// The source's length when the body went in, in letters.
    len: usize,
}

impl Snippet {
    /// Moves the placeholders after an edit at `caret` of `delta` letters, and
    /// stretches the one the edit is in.
    fn edited(&mut self, caret: usize, delta: isize) {
        for stop in &mut self.stops {
            if stop.end <= caret {
                continue;
            }
            if stop.start >= caret {
                stop.start = (stop.start as isize + delta).max(0) as usize;
                stop.end = (stop.end as isize + delta).max(stop.start as isize) as usize;
            } else {
                stop.end = (stop.end as isize + delta).max(stop.start as isize) as usize;
            }
        }
        if self.end > caret {
            self.end = (self.end as isize + delta).max(0) as usize;
        }
    }
}

/// A key of a snippet being filled, taken before the field sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
    Next,
    Previous,
    End,
}

/// Before the field is drawn: Tab and Shift+Tab walk the placeholders of the
/// snippet being filled, and Escape ends it.
pub(super) fn snippet_keys(ui: &egui::Ui) -> Option<Step> {
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
            Some(Step::Previous)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
            Some(Step::Next)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            Some(Step::End)
        } else {
            None
        }
    })
}

/// After the field is drawn: the placeholder the key asked for, selected.
pub(super) fn snippet_step(ui: &egui::Ui, field: Id, e: &mut super::Editor, step: Step) {
    let len = e.source.chars().count();
    let (range, done) = {
        let Some(session) = e.tools.snippet.as_mut() else {
            return;
        };
        match step {
            Step::End => (None, true),
            Step::Next if session.current + 1 >= session.stops.len() => (Some(session.end.min(len)..session.end.min(len)), true),
            Step::Next => {
                session.current += 1;
                let stop = &session.stops[session.current];
                (Some(stop.start.min(len)..stop.end.min(len)), false)
            }
            Step::Previous if session.current > 0 => {
                session.current -= 1;
                let stop = &session.stops[session.current];
                (Some(stop.start.min(len)..stop.end.min(len)), false)
            }
            Step::Previous => {
                let stop = &session.stops[0];
                (Some(stop.start.min(len)..stop.end.min(len)), false)
            }
        }
    };
    if done {
        e.tools.snippet = None;
    }
    if let Some(range) = range {
        editing::select(ui.ctx(), field, range, false);
    }
    ui.ctx().request_repaint();
}

/// A snippet's body as it goes in: the placeholders taken out, each line after
/// the first given the indentation of the line it is written on, and where each
/// placeholder is in the text, in the order of their numbers.
fn expand(body: &str, indent: &str) -> (String, Vec<Range<usize>>) {
    let chars: Vec<char> = body.chars().collect();
    let mut text = String::new();
    let mut stops: Vec<(u8, Range<usize>)> = Vec::new();
    let mut index = 0;
    let mut line_start = true;
    while index < chars.len() {
        if line_start && !text.is_empty() {
            // a line under the first takes the source line's indentation
            let at = text.chars().count();
            text.push_str(indent);
            let added = indent.chars().count();
            for (_, stop) in &mut stops {
                if stop.start >= at {
                    stop.start += added;
                    stop.end += added;
                }
            }
            line_start = false;
        }
        if chars[index] == '$' && chars.get(index + 1) == Some(&'{') {
            let mut close = index + 2;
            while close < chars.len() && chars[close] != '}' {
                close += 1;
            }
            let inner: String = chars[index + 2..close.min(chars.len())].iter().collect();
            let (number, placeholder) = match inner.split_once(':') {
                Some((number, placeholder)) => (number, placeholder),
                None => (inner.as_str(), ""),
            };
            let start = text.chars().count();
            text.push_str(placeholder);
            let number: u8 = number.trim().parse().unwrap_or(0);
            stops.push((number, start..start + placeholder.chars().count()));
            index = close + 1;
            continue;
        }
        text.push(chars[index]);
        line_start = chars[index] == '\n';
        index += 1;
    }
    stops.sort_by_key(|(number, _)| *number);
    (text, stops.into_iter().map(|(_, range)| range).collect())
}

/// The indentation of the line `at` is on.
fn line_indent(source: &str, at: usize) -> String {
    let chars: Vec<char> = source.chars().collect();
    let at = at.min(chars.len());
    let start = chars[..at].iter().rposition(|ch| *ch == '\n').map_or(0, |index| index + 1);
    chars[start..at].iter().take_while(|ch| **ch == ' ' || **ch == '\t').collect()
}

/// The list of one field, kept between frames.
#[derive(Clone, Default)]
struct List {
    /// The word it was made for: where it starts and what is typed.
    token: Option<(usize, String)>,
    items: Vec<Item>,
    selected: usize,
    /// A letter of this word was typed: a caret that a click or a jump left in
    /// a word does not open the list by itself (it would take the window's
    /// Enter and Escape).
    opened: bool,
    /// Escape closed the list for that word: it opens again when the word
    /// changes.
    dismissed: Option<(usize, String)>,
    /// The field had the keyboard in the last frame (egui takes it away on
    /// Escape before the frame's widgets see the key).
    had_focus: bool,
}

impl List {
    fn shown(&self) -> bool {
        self.opened && !self.items.is_empty() && self.dismissed != self.token
    }
}

/// A key of the open list, taken before the field sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Nav {
    Up,
    Down,
    Pick,
    Dismiss,
}

fn list_id(field: Id) -> Id {
    field.with("completion")
}

/// Whether the reader typed or erased a letter this frame: the list belongs to
/// the typing, as a menu of suggestions does.
fn typing(ui: &egui::Ui) -> bool {
    ui.input(|input| {
        input.events.iter().any(|event| match event {
            egui::Event::Text(text) => !text.is_empty(),
            egui::Event::Key {
                key: egui::Key::Backspace | egui::Key::Delete,
                pressed: true,
                ..
            } => true,
            _ => false,
        })
    })
}

/// Before the field is drawn: while its list is shown, the arrows, Tab, Enter
/// and Escape are the list's (Enter does not break the line, Tab does not
/// indent). Nothing is taken while the list is not shown.
pub(super) fn keys(ui: &egui::Ui, field: Id) -> Option<Nav> {
    let list: List = ui.ctx().data(|data| data.get_temp(list_id(field)))?;
    if !list.shown() || !list.had_focus {
        return None;
    }
    let none = egui::Modifiers::NONE;
    let nav = ui.input_mut(|input| {
        if input.consume_key(none, egui::Key::ArrowUp) {
            Some(Nav::Up)
        } else if input.consume_key(none, egui::Key::ArrowDown) {
            Some(Nav::Down)
        } else if input.consume_key(none, egui::Key::Tab) || input.consume_key(none, egui::Key::Enter) {
            Some(Nav::Pick)
        } else if input.consume_key(none, egui::Key::Escape) {
            Some(Nav::Dismiss)
        } else {
            None
        }
    });
    if nav.is_some() {
        // the key was the list's: the field keeps the keyboard
        ui.ctx().memory_mut(|memory| memory.request_focus(field));
    }
    nav
}

/// After the field is drawn: the caret's word is completed, the key taken by
/// [`keys`] applied, its choice written in the source, and the list drawn
/// under the caret.
pub(super) fn after(
    ui: &egui::Ui,
    p: &Palette,
    lang: Lang,
    field: Id,
    e: &mut super::Editor,
    output: &mut TextEditOutput,
    nav: Option<Nav>,
) {
    // a snippet being filled moves its placeholders with the text
    let length = e.source.chars().count();
    if let Some(session) = e.tools.snippet.as_mut()
        && length != session.len
    {
        let caret = output.cursor_range.map(|range| range.primary.index.0).unwrap_or(0);
        session.edited(caret, length as isize - session.len as isize);
        session.len = length;
    }
    let source = &mut e.source;
    let mut list: List = ui.ctx().data(|data| data.get_temp(list_id(field))).unwrap_or_default();
    let caret = output
        .cursor_range
        .filter(|range| range.is_empty())
        .map(|range| range.primary.index.0);
    let word = caret
        .filter(|_| output.response.has_focus() && !ui.is_sizing_pass() && ui.is_enabled())
        .and_then(|at| typed(source, at, lang));
    let token = word
        .as_ref()
        .filter(|word| word.text.chars().count() >= MIN_TYPED)
        .map(|word| (word.range.start, word.text.clone()));
    if token != list.token {
        list.items = token
            .as_ref()
            .map(|(start, text)| candidates(lang, source, *start + text.chars().count(), text, MOST))
            .unwrap_or_default();
        list.selected = 0;
        // a letter typed, or a letter erased: the list is the reader's
        list.opened = typing(ui);
        list.token = token;
    }
    if list.token.is_none() {
        list.dismissed = None;
    }
    let mut pick = None;
    match nav {
        Some(Nav::Up) if !list.items.is_empty() => list.selected = (list.selected + list.items.len() - 1) % list.items.len(),
        Some(Nav::Down) if !list.items.is_empty() => list.selected = (list.selected + 1) % list.items.len(),
        Some(Nav::Pick) => pick = Some(list.selected),
        Some(Nav::Dismiss) => list.dismissed = list.token.clone(),
        _ => {}
    }
    let drawn = list.shown() && pick.is_none();
    let anchor = caret.map(|at| {
        output
            .galley
            .pos_from_cursor(egui::text::CCursor::new(at))
            .translate(output.galley_pos.to_vec2())
    });
    if drawn && let Some(anchor) = anchor {
        pick = draw(ui, p, anchor, &list);
    }
    if let (Some(index), Some(word)) = (pick, word) {
        if let Some(item) = list.items.get(index)
            && let Some(session) = accept(ui, field, source, &word, item)
        {
            e.tools.snippet = Some(session);
        }
        list = List::default();
    }
    list.had_focus = output.response.has_focus() || nav.is_some();
    ui.ctx().data_mut(|data| data.insert_temp(list_id(field), list));
}

/// Writes the chosen name over the word: a function brings its bracket, the
/// caret after it, so that its arguments can be typed at once.
fn accept(ui: &egui::Ui, field: Id, source: &mut String, word: &Typed, item: &Item) -> Option<Snippet> {
    let chars: Vec<char> = source.chars().collect();
    let mut text = item.text.clone();
    // a snippet writes its body, its placeholders to fill
    let snippets = item.snippet.map(|body| expand(body, &line_indent(source, word.word.start)));
    if let Some((body, _)) = &snippets {
        text = body.clone();
    } else if item.kind == Kind::Function && chars.get(word.word.end).copied() != Some('(') {
        text.push('(');
    }
    let base = word.word.start;
    let at = base + text.chars().count();
    if !editing::replace(ui.ctx(), field, source, word.word.clone(), &text) {
        return None;
    }
    if let Some((_, stops)) = snippets {
        let stops: Vec<Range<usize>> = stops.into_iter().map(|stop| base + stop.start..base + stop.end).collect();
        let session = Snippet {
            current: 0,
            end: at,
            len: source.chars().count(),
            stops,
        };
        // the first placeholder takes the caret, its words selected
        let first = session.stops.first().cloned();
        if let Some(first) = first {
            editing::select(ui.ctx(), field, first, false);
        } else {
            editing::select(ui.ctx(), field, at..at, false);
        }
        ui.ctx().request_repaint();
        return Some(session);
    }
    editing::select(ui.ctx(), field, at..at, false);
    ui.ctx().request_repaint();
    None
}

/// The first lines of a help, on one paragraph: a tooltip of the catalog runs
/// to many lines, and the list keeps the room for its rows.
fn brief(docs: &str) -> String {
    const LINES: usize = 2;
    let mut lines = docs.lines().filter(|line| !line.trim().is_empty());
    let mut text = lines.by_ref().take(LINES).collect::<Vec<_>>().join(" ");
    if lines.next().is_some() {
        text.push('…');
    }
    text
}

/// The list under the caret: a row per name, a lit one for the arrows' choice,
/// and under them the signature and the help of that one; the row clicked, if
/// any.
fn draw(ui: &egui::Ui, p: &Palette, anchor: egui::Rect, list: &List) -> Option<usize> {
    let mut picked = None;
    egui::Area::new(egui::Id::unique("script_completion_list"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(anchor.left_bottom() + egui::vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_min_width(300.0);
                ui.set_max_width(360.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("script_completion_rows")
                    .max_height(ROWS_HEIGHT)
                    .show(ui, |ui| {
                        for (index, item) in list.items.iter().enumerate() {
                            let row = menu::suggestion_row_struck(
                                ui,
                                p,
                                item.kind.icon(),
                                &item.text,
                                "",
                                index == list.selected,
                                item.deprecated,
                            );
                            // the arrows scroll only a list with more rows than
                            // it shows: a short list stays put, its last row
                            // clear of the panel's edge
                            if index == list.selected && list.items.len() > 8 {
                                row.scroll_to_me(Some(egui::Align::Center));
                            }
                            if row.clicked() {
                                picked = Some(index);
                            }
                        }
                        // the glass's edge takes a few points: the last row
                        // stands clear of it, with or without a scroll bar
                        ui.add_space(6.0);
                    });
                // what the one chosen asks for and does, in a band of its
                // own room: the list keeps its height from one name to the next
                ui.add_space(4.0);
                ui.allocate_ui_with_layout(
                    egui::Vec2::new(ui.available_width(), DETAIL_HEIGHT),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if let Some(item) = list.items.get(list.selected) {
                            if !item.detail.is_empty() {
                                ui.add(
                                    egui::Label::new(RichText::new(&item.detail).font(egui::FontId::monospace(11.5)).color(p.ink))
                                        .truncate(),
                                );
                            }
                            if !item.docs.is_empty() {
                                ui.add(egui::Label::new(RichText::new(brief(&item.docs)).size(11.0).color(p.muted)).wrap());
                            }
                        }
                    },
                );
            });
        });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_is_the_name_before_the_caret_and_never_in_a_comment_or_a_string() {
        let source = "default { state_entry() { llSay(0, \"llSet\"); } } // llSet";
        // on the first letter of the call, then after its first letters
        let typed_at = |needle: &str, offset: usize| {
            let byte = source.find(needle).expect("the needle") + offset;
            typed(source, source[..byte].chars().count(), Lang::Lsl)
        };
        let start = source.find("llSay(0").expect("the call");
        let call = typed_at("llSay(0", 3).expect("llS of the call");
        assert_eq!(
            (call.range, call.text.as_str(), call.word),
            (start..start + 3, "llS", start..start + 5)
        );
        // the caret at the end of a word that ends the source
        let end = typed_of("llSay", 5, Lang::Lsl).expect("the end of a word");
        assert_eq!((end.range, end.text.as_str(), end.word), (0..5, "llSay", 0..5));
        // a name in a string, in a comment, a number and a lone letter
        let in_string = source.find("\"llSet").expect("the string") + 2;
        assert!(typed(source, source[..in_string].chars().count(), Lang::Lsl).is_none());
        let in_comment = source.find("// llSet").expect("the comment") + 4;
        assert!(typed(source, source[..in_comment].chars().count(), Lang::Lsl).is_none());
        assert!(typed_of("12", 2, Lang::Lsl).is_none());
        assert!(typed_of("(", 1, Lang::Lsl).is_none());
        // SLua keeps its library's dot in the name
        let lua = typed_of("ll.Sa", 5, Lang::Lua).expect("an SLua name");
        assert_eq!((lua.text.as_str(), lua.word), ("ll.Sa", 0..5));
    }

    fn typed_of(source: &str, at: usize, lang: Lang) -> Option<Typed> {
        typed(source, at, lang)
    }

    #[test]
    fn the_catalog_offers_its_names_by_case_then_kind_then_letter() {
        let names = candidates(Lang::Lsl, "", 0, "llSay", MOST);
        assert_eq!(names.first().map(|item| item.text.as_str()), Some("llSay"));
        assert_eq!(names[0].kind, Kind::Function);
        assert!(names[0].detail.starts_with("void llSay("), "{}", names[0].detail);
        assert!(!names[0].docs.is_empty());
        assert!(names.iter().all(|item| item.text.to_lowercase().starts_with("llsay")));
        // a constant and a type, and the exact case before the rest
        let active = candidates(Lang::Lsl, "", 0, "ACT", MOST);
        assert!(active.iter().any(|item| item.text == "ACTIVE" && item.kind == Kind::Constant));
        let rotation = candidates(Lang::Lsl, "", 0, "rotatio", MOST);
        assert!(rotation.iter().any(|item| item.text == "rotation" && item.kind == Kind::Type));
    }

    #[test]
    fn the_scripts_own_names_come_first_and_only_once() {
        let source = "integer counter = 0;\ndefault { state_entry() { counter = counter + 1; llSay(0, \"\"); } }";
        let names = candidates(Lang::Lsl, source, source.chars().count(), "cou", MOST);
        assert_eq!(names.first().map(|item| item.text.as_str()), Some("counter"));
        assert_eq!(names[0].kind, Kind::Name);
        assert!(names[0].detail.is_empty());
        // a catalog word used in the script is not offered twice
        let say = candidates(Lang::Lsl, source, source.chars().count(), "llSa", MOST);
        assert_eq!(say.iter().filter(|item| item.text == "llSay").count(), 1);
        // a name inside a comment is not one of the script's words
        let commented = candidates(Lang::Lsl, "// hiddenname\ndefault { }", 0, "hidden", MOST);
        assert!(commented.iter().all(|item| item.text != "hiddenname"));
    }

    #[test]
    fn a_word_no_catalog_knows_is_offered_as_a_name() {
        let names = candidates(Lang::Lsl, "default { state_entry() { myHelper; } }", 0, "myH", MOST);
        assert_eq!(names.first().map(|item| item.text.as_str()), Some("myHelper"));
        assert_eq!(names[0].kind, Kind::Name);
        assert!(!names[0].own, "a word used but not declared");
    }

    const SCOPED: &str = "\
integer shared = 0;
integer give(integer amount)
{
    return amount;
}
default
{
    touch_start(integer total)
    {
        integer counter = 0;
        counter = counter + 1;
    }
    state_entry()
    {
    }
}
";

    /// The caret just after `needle` in `source`.
    fn after(source: &str, needle: &str) -> usize {
        let byte = source.find(needle).expect("the needle") + needle.len();
        source[..byte].chars().count()
    }

    #[test]
    fn a_parameter_holds_only_inside_its_event() {
        let inside = candidates(Lang::Lsl, SCOPED, after(SCOPED, "counter = counter"), "tot", MOST);
        assert_eq!(inside.first().map(|item| item.text.as_str()), Some("total"));
        assert_eq!(inside[0].kind, Kind::Name);
        assert!(inside[0].own);
        assert_eq!(inside[0].detail, "integer total", "its type reads beside it");
        // the same letters in another event find no parameter
        let elsewhere = candidates(Lang::Lsl, SCOPED, after(SCOPED, "state_entry()\n    {"), "tot", MOST);
        assert!(elsewhere.iter().all(|item| item.text != "total"));
    }

    #[test]
    fn a_local_holds_from_its_declaration_in_its_block_only() {
        let later = candidates(Lang::Lsl, SCOPED, after(SCOPED, "counter = counter"), "cou", MOST);
        assert_eq!(later.first().map(|item| item.text.as_str()), Some("counter"));
        assert_eq!(later[0].detail, "integer counter");
        // before its declaration, and in another block, it does not hold
        let before = candidates(Lang::Lsl, SCOPED, after(SCOPED, "touch_start(integer total)"), "cou", MOST);
        assert!(before.iter().all(|item| item.text != "counter"));
        let in_function = candidates(Lang::Lsl, SCOPED, after(SCOPED, "return amount"), "amo", MOST);
        let other_event = candidates(Lang::Lsl, SCOPED, after(SCOPED, "state_entry()\n    {"), "cou", MOST);
        assert!(
            in_function.iter().any(|item| item.text == "amount"),
            "the function's parameter holds in the function"
        );
        assert!(other_event.iter().all(|item| item.text != "counter"));
    }

    #[test]
    fn a_global_and_a_function_hold_everywhere_and_stand_first() {
        let names = candidates(Lang::Lsl, SCOPED, after(SCOPED, "state_entry()\n    {"), "giv", MOST);
        assert_eq!(names.first().map(|item| item.text.as_str()), Some("give"));
        assert_eq!(names[0].kind, Kind::Function);
        assert!(names[0].own);
        let shared = candidates(Lang::Lsl, SCOPED, after(SCOPED, "state_entry()\n    {"), "sha", MOST);
        assert_eq!(shared.first().map(|item| item.text.as_str()), Some("shared"));
    }

    #[test]
    fn a_deprecated_name_is_offered_struck_through() {
        let items = candidates(Lang::Lsl, "", 0, "llClo", MOST);
        let cloud = items.iter().find(|item| item.text == "llCloud").expect("the deprecated call");
        assert!(cloud.deprecated);
        let say = candidates(Lang::Lsl, "", 0, "llSay", MOST);
        assert!(say.iter().any(|item| item.text == "llSay" && !item.deprecated));
    }

    #[test]
    fn the_scripts_own_names_stand_before_the_catalogs_in_the_same_case() {
        let items = [(false, Kind::Function, "llSays"), (true, Kind::Function, "llSays")];
        let mut built: Vec<Item> = items
            .into_iter()
            .map(|(own, kind, text)| Item {
                lower: text.to_lowercase(),
                text: text.to_owned(),
                kind,
                detail: String::new(),
                docs: String::new(),
                own,
                snippet: None,
                deprecated: false,
            })
            .collect();
        built.sort_by(|a, b| order("llSa", a).cmp(&order("llSa", b)));
        assert!(built[0].own, "the script's own name first");
    }

    /// Lua's snippets are Lua's: `if` closes with `end`, `repeat` waits on
    /// its `until`, and a `function` writes its parameters without a type.
    #[test]
    fn lua_snippets_write_lua_blocks() {
        let names = |typed: &str| -> Vec<String> { snippets(typed, Lang::Lua).into_iter().map(|item| item.text).collect() };
        assert_eq!(names("i"), ["if"]);
        assert_eq!(names("re"), ["repeat"]);
        assert_eq!(names("fo"), ["for"]);
        assert_eq!(names("pa"), ["pairs"]);
        assert_eq!(names("lo"), ["local", "localfunction"]);
        let body = |name: &str| {
            snippets(name, Lang::Lua)
                .into_iter()
                .find(|item| item.text == name)
                .and_then(|item| item.snippet)
                .expect("the snippet")
        };
        assert_eq!(body("if"), "if ${1:condition} then\n    ${2}\nend");
        assert!(body("repeat").ends_with("until ${2:condition}"));
        assert!(body("function").ends_with("\nend"));
        // and LSL's own are not offered to a Lua script
        assert!(snippets("re", Lang::Lsl).is_empty());
        assert!(snippets("if", Lang::Lsl)[0].snippet.expect("LSL's if").starts_with("if ("));
    }

    #[test]
    fn a_snippet_takes_the_lines_indentation_and_keeps_its_placeholders() {
        let (text, stops) = expand("if (${1:condition})\n{\n    ${2}\n}", "    ");
        assert_eq!(text, "if (condition)\n    {\n        \n    }");
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0], 4..13, "the placeholder's words");
        assert_eq!(stops[1].start, stops[1].end, "an empty one");
        assert_eq!(&text[stops[1].start..stops[1].end], "");
        // no indentation on the first line: the caret is already on it
        let (text, _) = expand("do\n{\n    ${1}\n} while (${2:condition});", "");
        assert_eq!(text, "do\n{\n    \n} while (condition);");
    }

    #[test]
    fn a_snippets_placeholders_move_with_what_is_typed_in_them() {
        let mut snippet = Snippet {
            stops: vec![4..13, 30..30],
            current: 0,
            end: 40,
            len: 40,
        };
        // `condition` replaced by `on`: the caret at 6, seven letters gone
        snippet.edited(6, -7);
        assert_eq!(snippet.stops[0], 4..6, "the placeholder shrinks with its words");
        assert_eq!(snippet.stops[1], 23..23, "and what follows moves");
        assert_eq!(snippet.end, 33);
    }

    /// The help under the list keeps two lines, in the band whose height is
    /// the same for every name.
    #[test]
    fn the_help_under_the_list_keeps_two_lines() {
        assert_eq!(brief("One.\nTwo."), "One. Two.");
        assert_eq!(brief("One.\n\nTwo.\nThree.\nFour."), "One. Two.…");
        assert_eq!(brief(""), "");
    }
}
