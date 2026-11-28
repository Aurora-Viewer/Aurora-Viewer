//! The parameter names of a call, written in the gap before each argument: an
//! IDE's inlay hints (Aurora's own; the Legacy Viewer writes none). `llSay`
//! reads `llSay(channel 0, msg "hi")` while its text stays `llSay(0, "hi")`.
//!
//! The hints never touch the text. The room one takes is layout space -- the
//! `leading_space` of the section that starts at its argument (`space` below) --
//! and the pill is painted into that room, so the caret, the selection, the
//! undo ranges and a copy stay the script's own.
//!
//! A name comes from the catalog's signature (`integer channel` names
//! `channel`); a signature written with types alone names nothing, and a call
//! the catalog does not know gets none.

use super::Lang;

use super::reference;
use egui::text::{ByteIndex, LayoutJob, LayoutSection};

/// The air between a pill and the argument it stands before.
pub(super) const GAP: f32 = 4.0;

/// The air on either side of a pill's words.
pub(super) const PAD: f32 = 6.0;

/// How much of the palette's raised surface a pill shows: faint, so that the
/// name reads beside the argument and the script's own text stays the ink.
pub(super) const PILL_ALPHA: f32 = 0.42;

/// The font the pills are written in: the editor's, two points smaller, so
/// that a name reads as a note beside the argument rather than as code.
pub(super) fn hint_font(font: &egui::FontId) -> egui::FontId {
    egui::FontId::new((font.size - 2.0).max(8.0), font.family.clone())
}

/// One parameter's name, in the gap before its argument.
pub(super) struct Inlay {
    /// The argument's first character, in the text's characters: where the pill
    /// goes.
    pub at: usize,
    /// The same first character as a byte: where the layout section starts.
    pub byte: usize,
    /// What the pill says, `channel`.
    pub text: String,
    /// The room the layout already keeps before the argument, which the pill
    /// stands left of: a colour literal's preview keeps `COLOR_SPACE` there
    /// (`syntax::color_spacing`), where its swatch is drawn.
    pub before: f32,
    /// The room the hint takes: its words, their air, and the gap after it, in
    /// points.
    pub space: f32,
    /// What the argument means, from the definitions: the pill's tooltip.
    pub help: Option<String>,
}

impl Inlay {
    /// The pill of this hint, before its argument: its words and their air,
    /// short of the gap after it and of the room already kept before the
    /// argument (a colour's preview).
    pub(super) fn pill(&self, at_left: f32, center_y: f32, height: f32) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(at_left - self.before - self.space, center_y - height / 2.0),
            egui::pos2(at_left - self.before - GAP, center_y + height / 2.0),
        )
    }
}

/// The parameter names of the calls in `source`, in the text's order, with the
/// room each takes (`measure` gives a text's width in the hint's font). A call
/// of one of the script's own functions reads the names its declaration gives
/// it, a call of the catalogue's read the catalogue's.
pub(super) fn hints(source: &str, lang: Lang, colors: &[usize], measure: impl Fn(&str) -> f32) -> Vec<Inlay> {
    let lua = lang.is_lua();
    let chars: Vec<char> = source.chars().collect();
    let code = super::syntax::code_chars(source, lang);
    // where each character starts, in bytes, for the layout sections
    let bytes: Vec<usize> = source.char_indices().map(|(byte, _)| byte).chain([source.len()]).collect();
    // the script's own functions: LSL's `type name(...)`, Lua's `function`
    let declared = if lua {
        lua_declarations(&chars, &code)
    } else {
        declarations(&chars, &code)
    };
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        // a call is a known name against its bracket, over code only
        if !code[at] || chars[at] != '(' {
            at += 1;
            continue;
        }
        let call = call_name(&chars, &code, at, lang);
        let (candidates, helps) = call
            .as_deref()
            .map(|name| match reference::get(name, lang) {
                Some(entry) => (
                    crate::lsl_keywords::signature_parameters(&entry.signature),
                    entry.arguments.iter().map(|argument| argument.help.clone()).collect::<Vec<_>>(),
                ),
                // the script's own: the parameters its declaration writes
                None => (declared.get(name).cloned().unwrap_or_default(), Vec::new()),
            })
            .unwrap_or_default();
        let close = closing_bracket(&chars, &code, at);
        if let (false, Some(close)) = (candidates.is_empty(), close) {
            // the arguments: what the call's own commas separate, with where
            // each begins and ends
            let starts = arguments(&chars, &code, at, close);
            // a definition's parameter list is no call: LSL's arguments are
            // `type name`, Lua's the list a `function` opens
            let definition = if lua {
                call.as_deref().is_some_and(|name| after_function(&chars, &code, at, name))
            } else {
                starts.iter().any(|(_, start, end, _)| declares(&chars[*start..*end]))
            };
            if !definition {
                for (argument, _, _, first) in starts {
                    let Some(first) = first else { continue };
                    let Some(name) = candidates.get(argument).and_then(|param| named(param)) else {
                        continue;
                    };
                    let text = name.to_owned();
                    // a colour literal already keeps its preview's room before
                    // it: the pill stands left of the swatch
                    let before = if colors.contains(&bytes[first]) {
                        super::syntax::COLOR_SPACE
                    } else {
                        0.0
                    };
                    found.push(Inlay {
                        at: first,
                        byte: bytes[first],
                        before,
                        space: measure(&text) + PAD * 2.0 + GAP,
                        help: helps.get(argument).filter(|help| !help.is_empty()).cloned(),
                        text,
                    });
                }
            }
        }
        // a call inside an argument is found by its own bracket as the walk goes
        at += 1;
    }
    found
}

/// The arguments of the call whose bracket opens at `open` and closes at
/// `close`: each one's place among them, where it starts and ends, and the
/// first character it is written with (a blank argument has none).
fn arguments(chars: &[char], code: &[bool], open: usize, close: usize) -> Vec<(usize, usize, usize, Option<usize>)> {
    let mut starts = Vec::new();
    let mut start = open + 1;
    let mut depth = 0i32;
    // a vector or a rotation, `<1, 0, 0>`, comma and all: a comparison after a
    // value is no opener
    let mut angles = 0i32;
    let mut previous: Option<char> = None;
    let mut argument = 0;
    for index in open + 1..=close {
        if !code[index] {
            continue;
        }
        let ch = chars[index];
        if index == close {
            starts.push((argument, start, close, first_char(chars, start, close)));
            break;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '<' if !value_before(previous) => angles += 1,
            '>' if angles > 0 => angles -= 1,
            ',' if depth == 0 && angles == 0 => {
                starts.push((argument, start, index, first_char(chars, start, index)));
                argument += 1;
                start = index + 1;
            }
            _ => {}
        }
        previous = Some(ch);
    }
    starts
}

/// The functions the script declares, with the parameters each writes
/// (`integer spin(key id, float rate)` gives `spin` -> [`key id`, `float rate`]),
/// by name: a call to one reads its hints as a call of the catalogue's does.
fn declarations(chars: &[char], code: &[bool]) -> std::collections::HashMap<String, Vec<String>> {
    let mut functions = std::collections::HashMap::new();
    let mut at = 0;
    while at < chars.len() {
        if !code[at] || chars[at] != '(' {
            at += 1;
            continue;
        }
        if let Some(name) = call_name(chars, code, at, Lang::Lsl) {
            // the type the declaration puts before the name: `integer spin(`
            let name_start = at - name.chars().count();
            // the blank between the two is no part of either
            let mut kind_end = name_start;
            while kind_end > 0 && chars[kind_end - 1].is_whitespace() {
                kind_end -= 1;
            }
            let mut kind_start = kind_end;
            while kind_start > 0 && (chars[kind_start - 1] == '_' || chars[kind_start - 1].is_alphanumeric()) {
                kind_start -= 1;
            }
            let kind: String = chars[kind_start..kind_end].iter().collect();
            if let Some(close) = closing_bracket(chars, code, at) {
                let starts = arguments(chars, code, at, close);
                let params: Vec<String> = starts
                    .iter()
                    .filter_map(|(_, start, end, _)| {
                        let text: String = chars[*start..*end].iter().collect();
                        let text = text.trim().to_owned();
                        (!text.is_empty()).then_some(text)
                    })
                    .collect();
                // A declaration writes its type, and so does a function still
                // being written without one: only a definition opens a block
                // after its parameters, and only a definition is written with
                // `type name` pairs.
                let typed = super::syntax::is_type(&kind);
                let definition = opens_a_block(chars, code, close)
                    && !starts.is_empty()
                    && starts.iter().all(|(_, start, end, _)| declares(&chars[*start..*end]));
                if typed || definition {
                    functions.insert(name, params);
                }
            }
        }
        at += 1;
    }
    functions
}

/// The `function` declarations of a Lua script, by the name its calls write
/// (`spin`, `M:run`, `M.run`), with the names their parameters list.
fn lua_declarations(chars: &[char], code: &[bool]) -> std::collections::HashMap<String, Vec<String>> {
    let mut functions = std::collections::HashMap::new();
    let mut at = 0;
    while at < chars.len() {
        if !code[at] || chars[at] != '(' {
            at += 1;
            continue;
        }
        if let Some(name) = call_name(chars, code, at, Lang::Lua)
            && after_function(chars, code, at, &name)
            && let Some(close) = closing_bracket(chars, code, at)
        {
            functions.insert(name, lua_parameters(chars, code, at, close));
        }
        at += 1;
    }
    functions
}

/// Whether the word before a call's name is `function`: its list is a
/// definition's, whose parameters name the arguments of its calls.
fn after_function(chars: &[char], code: &[bool], open: usize, name: &str) -> bool {
    let mut at = open.saturating_sub(name.chars().count());
    while at > 0 && chars[at - 1].is_whitespace() {
        at -= 1;
    }
    let end = at;
    while at > 0 && code[at - 1] && (chars[at - 1] == '_' || chars[at - 1].is_alphanumeric()) {
        at -= 1;
    }
    let word: String = chars[at..end].iter().collect();
    word == "function"
}

/// The parameters of a Lua definition's list: the names it writes, the Luau
/// types after each `:` left out and a `...` naming nothing.
fn lua_parameters(chars: &[char], code: &[bool], open: usize, close: usize) -> Vec<String> {
    let mut params = Vec::new();
    let mut depth = 0i32;
    let mut at_name = true;
    let mut i = open + 1;
    while i < close {
        if !code[i] {
            i += 1;
            continue;
        }
        match chars[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => at_name = true,
            c if depth == 0 && at_name && (c == '_' || c.is_alphanumeric()) => {
                let start = i;
                while i < close && code[i] && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                    i += 1;
                }
                params.push(chars[start..i].iter().collect());
                at_name = false;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    params
}

/// Whether the code after a definition's closing bracket opens a block
/// (`myFunction(...)` then `{`), the shape of a function whose return type is
/// not written yet.
fn opens_a_block(chars: &[char], code: &[bool], close: usize) -> bool {
    (close + 1..chars.len())
        .find(|index| code[*index] && !chars[*index].is_whitespace())
        .is_some_and(|index| chars[index] == '{')
}

/// The name a call's bracket follows (`llSay` in `llSay(`), over code only; in
/// Lua a library's member is one name (`ll.Say`, `table.insert`, `M:run`).
fn call_name(chars: &[char], code: &[bool], open: usize, lang: Lang) -> Option<String> {
    let lua = lang.is_lua();
    let is_name = |c: char| c == '_' || c.is_alphanumeric();
    let mut start = open;
    while start > 0 && code[start - 1] && is_name(chars[start - 1]) {
        start -= 1;
    }
    while lua
        && start >= 2
        && code[start - 1]
        && code[start - 2]
        && (chars[start - 1] == '.' || chars[start - 1] == ':')
        && is_name(chars[start - 2])
    {
        start -= 2;
        while start > 0 && code[start - 1] && is_name(chars[start - 1]) {
            start -= 1;
        }
    }
    (start < open).then(|| chars[start..open].iter().collect())
}

/// The bracket that closes the call opened at `open`.
fn closing_bracket(chars: &[char], code: &[bool], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for index in open..chars.len() {
        if !code[index] {
            continue;
        }
        match chars[index] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether the character before `<` ends a value, which makes it a comparison
/// rather than the opener of a vector or a rotation.
fn value_before(previous: Option<char>) -> bool {
    previous.is_some_and(|ch| ch == '_' || ch.is_alphanumeric() || ch == ')' || ch == ']')
}

/// The first character of an argument: the first that is not a blank. A
/// string's quote is the argument's own, so the code's mask is no guide here.
fn first_char(chars: &[char], start: usize, end: usize) -> Option<usize> {
    (start..end).find(|index| !chars[*index].is_whitespace())
}

/// Whether an argument is written as a declaration (`integer total`): a
/// definition's parameter list, which is no call and takes no pills.
fn declares(text: &[char]) -> bool {
    let text: String = text.iter().collect();
    let mut words = text.split_whitespace();
    let Some(kind) = words.next() else {
        return false;
    };
    super::syntax::is_type(kind) && words.next().is_some()
}

/// A parameter's name, as the catalog writes it (`integer channel`): a type
/// alone, as some catalogues write them, names nothing.
fn named(param: &str) -> Option<&str> {
    let name = param.split_whitespace().next_back()?;
    let identifier = name.chars().all(|c| c == '_' || c.is_alphanumeric());
    (!name.is_empty() && identifier && !super::syntax::is_type(name)).then_some(name)
}

/// Sets the room each hint takes in the job, splitting the sections at the
/// argument it stands before. The text stays the script's; only the layout
/// moves, so a caret, a selection or an undo range is still the text's own.
pub(super) fn space(job: &mut LayoutJob, inlays: &[Inlay]) {
    for inlay in inlays {
        let at = ByteIndex(inlay.byte);
        let Some(index) = job
            .sections
            .iter()
            .position(|section| section.byte_range.start <= at && at < section.byte_range.end)
        else {
            continue;
        };
        if job.sections[index].byte_range.start == at {
            job.sections[index].leading_space += inlay.space;
            continue;
        }
        let end = job.sections[index].byte_range.end;
        job.sections[index].byte_range.end = at;
        let format = job.sections[index].format.clone();
        job.sections.insert(
            index + 1,
            LayoutSection {
                leading_space: inlay.space,
                byte_range: at..end,
                format,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hints of a text, as `(character, text)`.
    fn names(source: &str, lang: Lang) -> Vec<(usize, String)> {
        hints(source, lang, &[], |text| text.len() as f32)
            .into_iter()
            .map(|inlay| (inlay.at, inlay.text))
            .collect()
    }

    /// A colour literal keeps its preview's room before it
    /// (`syntax::COLOR_SPACE`, where its swatch is drawn): the pill stands left
    /// of the swatch, and its own room is added beyond that.
    #[test]
    fn a_colour_literal_keeps_its_preview_clear_of_the_pill() {
        let source = "llSetColor(<0.5, 1.0, 0.25>, ALL_SIDES)";
        let colour = source.find('<').expect("the literal");
        let hints = hints(source, Lang::Lsl, &[colour], |text| text.len() as f32);
        assert_eq!(hints[0].text, "color");
        assert_eq!(hints[0].before, super::super::syntax::COLOR_SPACE);
        // the pill ends a gap before the preview's room, not under the swatch
        let at = 200.0;
        let pill = hints[0].pill(at, 100.0, 16.0);
        assert_eq!(pill.right(), at - super::super::syntax::COLOR_SPACE - GAP);
        assert_eq!(pill.left(), at - super::super::syntax::COLOR_SPACE - hints[0].space);
        // the argument after the literal keeps its own room as before
        assert_eq!(hints[1].before, 0.0);
        assert_eq!(hints[1].pill(at, 100.0, 16.0).right(), at - GAP);
    }

    #[test]
    fn the_arguments_of_a_call_take_their_parameter_names() {
        let source = "default { state_entry() { llSay(0, \"hi\"); } }";
        let hints = names(source, Lang::Lsl);
        assert_eq!(hints.len(), 2);
        // the definitions' spelling of each name (`integer channel`)
        assert_eq!(hints[0].1, "channel");
        assert_eq!(hints[1].1, "msg");
        // each pill stands where its argument starts
        let first = source.find('0').expect("the channel");
        let second = source.find("\"hi\"").expect("the message");
        assert_eq!(hints[0].0, source[..first].chars().count());
        assert_eq!(hints[1].0, source[..second].chars().count());
    }

    #[test]
    fn a_call_within_an_argument_takes_its_own_names() {
        let source = "llSay(0, llKey2Name(llDetectedKey(1)))";
        let hints = names(source, Lang::Lsl);
        assert_eq!(
            hints.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>(),
            ["channel", "msg", "id", "number"]
        );
    }

    #[test]
    fn a_comma_in_a_string_is_no_argument_of_its_own() {
        let source = "llSay(0, \"a, b\")";
        assert_eq!(names(source, Lang::Lsl).len(), 2, "two arguments, two names");
        // a comment before an argument does not hide it either
        let source = "llSay(/* the channel */ 0, \"x\")";
        assert_eq!(names(source, Lang::Lsl).len(), 2);
    }

    #[test]
    fn an_unnamed_signature_and_an_unknown_call_name_nothing() {
        // a function of the script: the catalog knows no signature for it
        assert!(names("my_helper(1, 2)", Lang::Lsl).is_empty());
        // and a name in a comment is no call
        assert!(names("// llSay(0, \"hi\")", Lang::Lsl).is_empty());
    }

    #[test]
    fn a_vector_and_a_rotation_are_one_argument_each() {
        let source = "llSetColor(<0.5, 1.0, 0.25>, ALL_SIDES)";
        assert_eq!(
            names(source, Lang::Lsl).into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
            ["color", "face"]
        );
        let source = "llSetPrimitiveParams([PRIM_ROT_LOCAL, <0, 0, 0, 1>])";
        assert_eq!(names(source, Lang::Lsl).len(), 1, "the list holds one argument");
    }

    /// A pill carries what the definitions say its argument means.
    #[test]
    fn a_pill_carries_the_meaning_of_its_argument() {
        let say = hints("llSay(0, \"hi\")", Lang::Lsl, &[], |text| text.len() as f32);
        assert_eq!(
            say[0].help.as_deref(),
            Some("Chat channel (any 32-bit signed integer value) to chat the message on.")
        );
        assert_eq!(say[1].help.as_deref(), Some("Text message string to broadcast."));
        // a call the definitions do not describe keeps its name and no help
        let own = hints("my_helper(1)", Lang::Lsl, &[], |text| text.len() as f32);
        assert!(own.is_empty(), "an unknown call names nothing");
    }

    /// A call of the script's own function reads the names its declaration
    /// writes, before or after the call.

    #[test]
    fn a_call_of_the_scripts_own_function_takes_its_names() {
        let source = "integer spin(key id, float rate) { return 0; }\n\
default { state_entry() { spin(1, 0.5); } }";
        assert_eq!(
            names(source, Lang::Lsl).into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
            ["id", "rate"],
            "the call's two arguments, the declaration's own names"
        );
        // the declaration may come after the call
        let source = "default { state_entry() { spin(1, 2); } }\n\
integer spin(integer count, float rate) { return count; }";
        assert_eq!(
            names(source, Lang::Lsl).into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
            ["count", "rate"]
        );
        // a script's function is no call of its own, and a type alone names none
        assert!(names("integer spin(integer) { return 0; }\nspin(1)", Lang::Lsl).is_empty());
    }

    /// A function written without its return type yet -- a definition, as only
    /// a definition opens a block after its parameters -- names its call too.
    #[test]
    fn a_function_without_its_return_type_still_names_its_call() {
        let source = "myFunction(integer helo1, string hello2)\n{\n    llOwnerSay(\"hello\");\n}\n\ndefault\n{\n    state_entry()\n    {\n        llSay(0, \"Hello, Avatar!\");\n        myFunction(123, \"test\");\n    }\n\n    touch_start(integer total_number)\n    {\n        llSay(0, \"Touched.\");\n    }\n}\n";
        let hints = names(source, Lang::Lsl);
        assert!(hints.iter().any(|(_, text)| text == "helo1"), "the first argument's name");
        assert!(hints.iter().any(|(_, text)| text == "hello2"), "the second argument's name");
    }

    #[test]
    fn a_definition_takes_no_pills() {
        // the event a script handles, and a function it declares: their
        // arguments are the script's own `type name`
        assert!(names("default { touch_start(integer total) { } }", Lang::Lsl).is_empty());
        assert!(names("string llSay(integer channel) { return \"\"; }", Lang::Lsl).is_empty());
        // while a call of the same name still reads its names
        assert_eq!(names("default { state_entry() { llSay(0, \"hi\"); } }", Lang::Lsl).len(), 2);
    }

    /// A Lua script's own functions name their calls as LSL's do, their Luau
    /// types no part of the names, and the definition's own list takes none.
    #[test]
    fn a_call_of_the_scripts_own_lua_function_takes_its_names() {
        let source = "local function spin(id: number, rate)\n    return id\nend\nspin(1, 0.5)\n";
        assert_eq!(
            names(source, Lang::Lua).into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
            ["id", "rate"],
            "the call's two arguments, the declaration's own names"
        );
        // a table's method keeps the name its call writes, colon and all
        let source = "function M:run(name, count)\n    return name\nend\nM:run(\"a\", 2)\n";
        assert_eq!(
            names(source, Lang::Lua).into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
            ["name", "count"]
        );
        // it is no call of its own, and a `...` names nothing
        let source = "local function spin(...)\nend\nspin(1, 2)\n";
        assert!(names(source, Lang::Lua).is_empty());
    }

    /// A Lua call of the catalogue reads SLua's keywords, its library's name
    /// part of the call.
    #[test]
    fn a_lua_call_of_the_catalogue_takes_its_names() {
        let source = "ll.Say(0, \"hi\")\n";
        assert_eq!(
            names(source, Lang::Lua).into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
            ["channel", "msg"]
        );
        let source = "table.insert(values, 1)\n";
        assert_eq!(
            names(source, Lang::Lua).into_iter().map(|(_, text)| text).collect::<Vec<_>>(),
            ["a", "i"]
        );
    }

    #[test]
    fn a_pill_takes_its_words_their_air_and_the_gap() {
        let source = "llSay(0, \"hi\")";
        let inlay = hints(source, Lang::Lsl, &[], |text| text.len() as f32)
            .into_iter()
            .next()
            .expect("a hint");
        assert_eq!(inlay.space, "channel".len() as f32 + PAD * 2.0 + GAP);
        assert_eq!(inlay.byte, source.find('0').expect("the channel"));
    }
}
