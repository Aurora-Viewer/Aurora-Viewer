//! The outline of a script: the states, events, functions and globals it
//! declares, in the source's order, with the line each one starts on. A simple
//! scan of the lines that hold code (comments and strings are blanked out), as
//! an IDE's outline view: `default` and `state name` at the top level, the
//! events inside a state, the functions and the globals of the top level
//! (LSL's declarations start with their type, SLua's with `function` or a
//! name).

use super::Lang;

/// What an outline item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    State,
    Event,
    /// A function of the top level: LSL's, or Lua's `function f()`.
    Function,
    /// Lua's `local function f()`, and `local f = function`: the function of
    /// this file alone.
    LocalFunction,
    /// Lua's `function M:run()` and `function M.run()`: a function of a table.
    Method,
    Global,
}

/// One item of the outline.
#[derive(Clone)]
pub(super) struct Item {
    pub kind: Kind,
    pub name: String,
    /// The line it starts on, counted from 0 as the source is.
    pub line: usize,
    /// The line of the brace that closes it, or the source's last line when
    /// it is not closed.
    pub end: usize,
}

/// The kinds of block the scan follows: a state's body holds events, a
/// function's body holds statements.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    State,
    Function,
    Other,
}

/// The types an LSL global or function starts with.
const TYPES: [&str; 7] = ["integer", "float", "string", "key", "vector", "rotation", "list"];

/// The outline of `source`.
pub(super) fn outline(source: &str, lang: Lang) -> Vec<Item> {
    let lua = lang.is_lua();
    let chars: Vec<char> = source.chars().collect();
    let code = super::syntax::code_chars(source, lang);
    // the same lines with what is not code blanked out, so that a brace or a
    // word in a comment or a string is not one of the code's
    let mut text = String::with_capacity(source.len());
    for (ch, is_code) in chars.iter().zip(&code) {
        text.push(if *is_code || *ch == '\n' { *ch } else { ' ' });
    }
    let mut items = Vec::new();
    let mut stack: Vec<(Block, Option<usize>)> = Vec::new();
    let mut pending: Option<(Block, Option<usize>)> = None;
    for (index, line) in text.lines().enumerate() {
        let block = stack.last().map(|(block, _)| *block);
        let trimmed = line.trim_start();
        let found = match block {
            // SLua's blocks end with `end`, not a brace: its top level is what
            // starts at the left margin, its bodies are indented
            None if lua => (trimmed.len() == line.len()).then(|| lua_top(trimmed)).flatten(),
            None => lsl_top(trimmed),
            Some(Block::State) => call_name(trimmed).map(|name| (Kind::Event, name)),
            Some(Block::Function | Block::Other) => None,
        };
        if let Some((kind, name)) = found {
            let item = items.len();
            items.push(Item {
                kind,
                name: name.to_owned(),
                line: index,
                end: index,
            });
            // a brace opening on a later line belongs to it
            pending = match kind {
                Kind::State => Some((Block::State, Some(item))),
                Kind::Event => Some((Block::Other, Some(item))),
                Kind::Function | Kind::LocalFunction | Kind::Method => Some((Block::Function, Some(item))),
                Kind::Global => None,
            };
        }
        for ch in line.chars() {
            match ch {
                '{' => stack.push(pending.take().unwrap_or((Block::Other, None))),
                '}' => {
                    if let Some((_, Some(item))) = stack.pop() {
                        items[item].end = index;
                    }
                }
                _ => {}
            }
        }
    }
    // a block the source leaves open reaches its last line
    let last = text.lines().count().saturating_sub(1);
    for (_, item) in stack {
        if let Some(item) = item {
            items[item].end = last;
        }
    }
    items
}

/// The state (if any) and the item whose block holds line: the deepest one,
/// which is what a sticky scroll shows above the source (default › event).
pub(super) fn enclosing(items: &[Item], line: usize) -> Option<(Option<&Item>, &Item)> {
    let holds = |item: &Item| item.kind != Kind::Global && item.line <= line && line <= item.end;
    let inner = items.iter().filter(|item| holds(item)).max_by_key(|item| item.line)?;
    let state = items
        .iter()
        .filter(|item| item.kind == Kind::State && holds(item))
        .max_by_key(|item| item.line)
        .filter(|state| !std::ptr::eq(*state, inner));
    Some((state, inner))
}

fn is_ident(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

fn ident(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_ident) && !text.starts_with(|c: char| c.is_ascii_digit())
}

/// `name(` at the start of a line: an event of a state.
fn call_name(line: &str) -> Option<&str> {
    let name = line.split('(').next()?;
    let name = name.trim_end();
    if ident(name) { Some(name) } else { None }
}

/// A declaration of the top level of an LSL script.
fn lsl_top(line: &str) -> Option<(Kind, &str)> {
    let mut words = line.split_whitespace();
    let first = words.next()?;
    if first == "default" {
        return Some((Kind::State, "default"));
    }
    if first == "state" {
        let name = words.next()?;
        return ident(name).then_some((Kind::State, name));
    }
    let second = words.next();
    if TYPES.contains(&first) {
        let second = second?;
        // `integer add(integer a, integer b)`: a function with a return type
        if second.contains('(') {
            return call_name(second).map(|name| (Kind::Function, name));
        }
        // `integer count = 0;`: a global, named before its value
        let name = second.trim_end_matches([';', ',']).split('=').next()?.trim_end();
        return ident(name).then_some((Kind::Global, name));
    }
    // `add(integer a, integer b)`: a function of the top level
    if first.contains('(') {
        return call_name(first).map(|name| (Kind::Function, name));
    }
    None
}

/// A declaration of the top level of an SLua script.
fn lua_top(line: &str) -> Option<(Kind, &str)> {
    let local = line.starts_with("local ");
    let rest = line.strip_prefix("local ").map_or(line, |rest| rest.trim_start());
    if let Some(name) = rest.strip_prefix("function ") {
        // `M:run` and `M.run` hang on a table: a method, not a plain function
        let name = name
            .trim_start()
            .split(|ch: char| !is_ident(ch) && ch != ':' && ch != '.')
            .next()
            .unwrap_or_default();
        let named = !name.is_empty() && name.chars().all(|ch| is_ident(ch) || ch == ':' || ch == '.');
        if !named {
            return None;
        }
        let kind = if name.contains([':', '.']) {
            Kind::Method
        } else if local {
            Kind::LocalFunction
        } else {
            Kind::Function
        };
        return Some((kind, name));
    }
    let (name, value) = rest.split_once('=')?;
    let name = name.trim_end();
    if !ident(name) {
        return None;
    }
    if value.trim_start().starts_with("function") {
        Some((if local { Kind::LocalFunction } else { Kind::Function }, name))
    } else {
        Some((Kind::Global, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(items: &[Item]) -> Vec<(Kind, &str, usize)> {
        items.iter().map(|item| (item.kind, item.name.as_str(), item.line)).collect()
    }

    /// The outline is a tree: a state's events sit under it, the functions and
    /// globals at the top, and the filter keeps the state of what it keeps.

    #[test]
    fn an_items_block_reaches_the_brace_that_closes_it() {
        let source = "\
default
{
    state_entry()
    {
    }
    // between the events
    touch_start(integer n)
    {
    }
}
";
        let items = outline(source, Lang::Lsl);
        assert_eq!((items[0].line, items[0].end), (0, 9), "default closes on line 10");
        assert_eq!((items[1].line, items[1].end), (2, 4), "state_entry closes on line 4");
        assert_eq!((items[2].line, items[2].end), (6, 8), "touch_start closes on line 9");
        // the sticky chain: the state and the event holding a line
        let (state, inner) = enclosing(&items, 3).expect("inside state_entry");
        assert_eq!(
            (state.map(|item| item.name.as_str()), inner.name.as_str()),
            (Some("default"), "state_entry")
        );
        let (state, inner) = enclosing(&items, 5).expect("inside the state, between its events");
        assert_eq!((state.map(|item| item.name.as_str()), inner.name.as_str()), (None, "default"));
        assert!(enclosing(&items, 0).is_some(), "the state holds its own line");
        // and a line outside every block has none
        assert!(enclosing(&items, 11).is_none(), "a line after every block");
    }

    #[test]
    fn an_lsl_script_lists_its_globals_functions_states_and_events() {
        let source = "\
// a comment { with a brace
integer count = 0;
string message = \"a string {\";

integer add(integer a, integer b)
{
    return a + b;
}

default
{
    state_entry()
    {
        llSay(0, message);
    }
    touch_start(integer total)
    {
        count = add(count, 1);
    }
}

state busy
{
    state_entry() { }
}
";
        let items = outline(source, Lang::Lsl);
        assert_eq!(
            named(&items),
            vec![
                (Kind::Global, "count", 1),
                (Kind::Global, "message", 2),
                (Kind::Function, "add", 4),
                (Kind::State, "default", 9),
                (Kind::Event, "state_entry", 11),
                (Kind::Event, "touch_start", 15),
                (Kind::State, "busy", 21),
                (Kind::Event, "state_entry", 23),
            ]
        );
    }

    #[test]
    fn an_slua_script_lists_its_functions_and_globals() {
        let source = "\
-- a comment
local count = 0
local function add(a, b)
    return a + b
end

function main()
    count = add(count, 1)
end

handler = function(event)
end

local helper = function(x)
    return x
end

function Shapes:area(r)
    return r
end

function Shapes.perimeter(r)
    return r
end
";
        let items = outline(source, Lang::Lua);
        // a function of the file alone, the top level's, and a table's
        assert_eq!(
            named(&items),
            vec![
                (Kind::Global, "count", 1),
                (Kind::LocalFunction, "add", 2),
                (Kind::Function, "main", 6),
                (Kind::Function, "handler", 10),
                (Kind::LocalFunction, "helper", 13),
                (Kind::Method, "Shapes:area", 17),
                (Kind::Method, "Shapes.perimeter", 21),
            ]
        );
        // a word in a comment or a long string is no function
        let quiet = "-- function fake()\nlocal s = [[\nfunction other()\n]]\n";
        assert!(
            outline(quiet, Lang::Lua).iter().all(|item| item.kind != Kind::Function),
            "{:?}",
            named(&outline(quiet, Lang::Lua))
        );
    }

    #[test]
    fn a_brace_in_a_comment_or_a_string_does_not_open_a_block() {
        // the string and the comment would otherwise hide the event
        let source = "\
default
{
    // }
    state_entry()
    {
        llSay(0, \"}{ {(\");
    }
    touch_start(integer n) { }
}
";
        let items = outline(source, Lang::Lsl);
        assert_eq!(
            named(&items),
            vec![
                (Kind::State, "default", 0),
                (Kind::Event, "state_entry", 3),
                (Kind::Event, "touch_start", 7)
            ]
        );
    }
}
