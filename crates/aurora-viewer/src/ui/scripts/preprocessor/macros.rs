//! Object/function/variadic macros, stringification and token pasting, matching
//! the Boost.Wave features used by Firestorm's FSLSLPreprocessor.

use super::lex::{self, Kind, Token};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone)]
pub(super) struct Macro {
    pub parameters: Option<Vec<String>>,
    pub variadic: bool,
    pub body: Vec<Token>,
}
pub(super) type Definitions = HashMap<String, Macro>;

fn error() -> String {
    aurora_i18n::tr!("script-preproc-macro").into()
}
fn significant(token: &Token) -> bool {
    !matches!(token.kind, Kind::Space | Kind::Comment)
}

pub(super) fn define(text: &str) -> Result<(String, Macro), String> {
    let text = text.trim_start();
    let tokens = lex::code(text);
    let Some(name) = tokens.first().filter(|t| t.kind == Kind::Word) else {
        return Err(error());
    };
    let mut body = &text[name.end..];
    let mut parameters = None;
    let mut variadic = false;
    if body.starts_with('(') {
        let end = lex::close(&tokens, 1, "(", ")").ok_or_else(error)?;
        let mut params = Vec::new();
        let mut at = 2;
        while at < end {
            let token = &tokens[at];
            if token.text == "..." {
                params.push("__VA_ARGS__".into());
                variadic = true;
                at += 1;
                if at != end {
                    return Err(error());
                }
            } else {
                if token.kind != Kind::Word || params.contains(&token.text) {
                    return Err(error());
                }
                params.push(token.text.clone());
                at += 1;
            }
            if at < end {
                if tokens[at].text != "," {
                    return Err(error());
                }
                at += 1;
                if at == end {
                    return Err(error());
                }
            }
        }
        parameters = Some(params);
        body = &text[tokens[end].end..];
    }
    Ok((
        name.text.clone(),
        Macro {
            parameters,
            variadic,
            body: lex::lex(body.trim_start()),
        },
    ))
}

pub(super) fn object(text: &str) -> Macro {
    Macro {
        parameters: None,
        variadic: false,
        body: lex::lex(text),
    }
}

pub(super) fn expand(tokens: &[Token], defs: &Definitions) -> Result<Vec<Token>, String> {
    let tokens = tokens.iter().cloned().map(Rescan::new).collect();
    Ok(rescan(tokens, defs, 0, &mut 0)?.into_iter().map(|t| t.token).collect())
}

#[derive(Clone)]
struct Rescan {
    token: Token,
    hidden: HashSet<String>,
}
impl Rescan {
    fn new(token: Token) -> Self {
        Self {
            token,
            hidden: HashSet::new(),
        }
    }
}

fn limit() -> String {
    aurora_i18n::tr!("script-preproc-expansion-limit").into()
}

// Rescan replacements together with the remaining input: an object macro can
// become a function macro whose arguments follow it in the original stream.
// Per-token hide sets terminate direct and mutual recursion without suppressing
// unrelated invocations later in the file (the C preprocessor rule).
fn rescan(tokens: Vec<Rescan>, defs: &Definitions, depth: usize, operations: &mut usize) -> Result<Vec<Rescan>, String> {
    if depth > 64 {
        return Err(limit());
    }
    let mut output = Vec::new();
    let mut size = 0;
    let mut input: VecDeque<_> = tokens.into();
    while let Some(item) = input.pop_front() {
        *operations += 1;
        if *operations > 500_000 || input.len() + output.len() > 250_000 {
            return Err(limit());
        }
        let token = &item.token;
        if token.kind == Kind::Word
            && !item.hidden.contains(&token.text)
            && let Some(def) = defs.get(&token.text)
        {
            let mut hidden = item.hidden.clone();
            hidden.insert(token.text.clone());
            let replacement = if let Some(params) = &def.parameters {
                let opening = input.iter().position(|t| significant(&t.token));
                if let Some(opening) = opening.filter(|i| input[*i].token.text == "(") {
                    let mut nesting = 0;
                    let end = input
                        .iter()
                        .enumerate()
                        .skip(opening)
                        .find_map(|(i, t)| {
                            if t.token.kind == Kind::Symbol {
                                if t.token.text == "(" {
                                    nesting += 1;
                                }
                                if t.token.text == ")" {
                                    nesting -= 1;
                                }
                            }
                            (nesting == 0).then_some(i)
                        })
                        .ok_or_else(error)?;
                    hidden.retain(|name| name == &token.text || input[end].hidden.contains(name));
                    let invocation: Vec<_> = input.drain(..=end).collect();
                    let mut args = arguments(&invocation[opening + 1..end]);
                    if params.is_empty() && args.len() == 1 && args[0].iter().all(|t| !significant(&t.token)) {
                        args.clear();
                    }
                    if def.variadic {
                        let fixed = params.len() - 1;
                        if args.len() < fixed {
                            return Err(error());
                        }
                        let mut rest = Vec::new();
                        for (n, arg) in args.drain(fixed..).enumerate() {
                            if n > 0 {
                                rest.extend(lex::lex(",").into_iter().map(Rescan::new));
                            }
                            rest.extend(arg);
                        }
                        args.push(rest);
                    }
                    if args.len() != params.len() {
                        return Err(error());
                    }
                    let arguments: HashMap<_, _> = params.iter().cloned().zip(args).collect();
                    Some(substitute(&def.body, &arguments, defs, depth + 1, operations)?)
                } else {
                    None
                }
            } else {
                Some(substitute(&def.body, &HashMap::new(), defs, depth + 1, operations)?)
            };
            if let Some(mut replacement) = replacement {
                if replacement.iter().map(|t| t.token.text.len()).sum::<usize>() > aurora_net::scripts::MAX_SOURCE {
                    return Err(limit());
                }
                for t in &mut replacement {
                    t.hidden.extend(hidden.iter().cloned());
                    t.token.line = token.line;
                }
                // __LINE__ in a macro body uses the invocation's logical line.
                if token.text == "__LINE__" {
                    replacement = lex::lex(&token.line.to_string()).into_iter().map(Rescan::new).collect();
                }
                for replacement in replacement.into_iter().rev() {
                    input.push_front(replacement);
                }
                continue;
            }
        }
        size += item.token.text.len();
        if size > aurora_net::scripts::MAX_SOURCE {
            return Err(limit());
        }
        output.push(item);
    }
    Ok(output)
}

fn arguments(tokens: &[Rescan]) -> Vec<Vec<Rescan>> {
    let mut args = vec![Vec::new()];
    let mut depth = 0;
    for token in tokens {
        if token.token.kind == Kind::Symbol {
            if token.token.text == "(" {
                depth += 1;
            }
            if token.token.text == ")" {
                depth -= 1;
            }
            if token.token.text == "," && depth == 0 {
                args.push(Vec::new());
                continue;
            }
        }
        args.last_mut().expect("one argument").push(token.clone());
    }
    args
}

fn substitute(
    body: &[Token],
    args: &HashMap<String, Vec<Rescan>>,
    defs: &Definitions,
    depth: usize,
    operations: &mut usize,
) -> Result<Vec<Rescan>, String> {
    let mut output = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let token = &body[at];
        if token.text == "#" {
            let next = (at + 1..body.len()).find(|i| significant(&body[*i])).ok_or_else(error)?;
            let arg = args.get(&body[next].text).ok_or_else(error)?;
            let mut text = String::new();
            let mut space = false;
            for token in arg {
                if !significant(&token.token) {
                    space = !text.is_empty();
                } else {
                    if space {
                        text.push(' ');
                    }
                    text.push_str(&token.token.text);
                    space = false;
                }
            }
            output.extend(lex::lex(&lex::quoted(&text)).into_iter().map(Rescan::new));
            at = next + 1;
            continue;
        }
        if token.kind == Kind::Word
            && let Some(arg) = args.get(&token.text)
        {
            let before = body[..at].iter().rev().find(|t| significant(t));
            let after = body[at + 1..].iter().find(|t| significant(t));
            if before.is_some_and(|t| t.text == "##") || after.is_some_and(|t| t.text == "##") {
                if arg.iter().all(|t| !significant(&t.token)) {
                    output.push(Rescan::new(Token {
                        text: String::new(),
                        kind: Kind::Empty,
                        start: 0,
                        end: 0,
                        line: token.line,
                    }));
                } else {
                    output.extend(arg.iter().cloned());
                }
            } else {
                output.extend(rescan(arg.clone(), defs, depth + 1, operations)?);
            }
        } else {
            output.push(Rescan::new(token.clone()));
        }
        at += 1;
    }
    let mut pasted: Vec<Rescan> = Vec::new();
    let mut at = 0;
    while at < output.len() {
        if output[at].token.text == "##" {
            while pasted.last().is_some_and(|t| !significant(&t.token)) {
                pasted.pop();
            }
            let left = pasted.pop();
            let right = (at + 1..output.len()).find(|i| significant(&output[*i].token));
            match (left, right) {
                (Some(left), Some(right)) => {
                    if left.token.kind == Kind::Empty && output[right].token.kind == Kind::Empty {
                        pasted.push(left);
                        at = right + 1;
                        continue;
                    }
                    let merged = lex::code(&format!("{}{}", left.token.text, output[right].token.text));
                    if merged.len() != 1 {
                        return Err(error());
                    }
                    pasted.extend(merged.into_iter().map(|token| Rescan {
                        token,
                        hidden: left.hidden.intersection(&output[right].hidden).cloned().collect(),
                    }));
                    at = right + 1;
                    continue;
                }
                (Some(left), None) => pasted.push(left),
                (None, Some(right)) => {
                    pasted.push(output[right].clone());
                    at = right + 1;
                    continue;
                }
                _ => {}
            }
        } else {
            pasted.push(output[at].clone());
        }
        at += 1;
    }
    Ok(pasted.into_iter().filter(|t| t.token.kind != Kind::Empty).collect())
}

pub(super) fn condition(text: &str, defs: &Definitions, line: usize) -> Result<bool, String> {
    let mut tokens = lex::code(text);
    for token in &mut tokens {
        token.line = line;
    }
    let mut output = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        if tokens[at].text == "defined" {
            at += 1;
            let parens = tokens.get(at).is_some_and(|t| t.text == "(");
            if parens {
                at += 1;
            }
            let name = tokens.get(at).filter(|t| t.kind == Kind::Word).ok_or_else(error)?;
            output.extend(lex::code(if defs.contains_key(&name.text) { "1" } else { "0" }));
            at += 1;
            if parens {
                if !tokens.get(at).is_some_and(|t| t.text == ")") {
                    return Err(error());
                }
                at += 1;
            }
        } else {
            output.push(tokens[at].clone());
            at += 1;
        }
    }
    let expanded = expand(&output, defs)?;
    let tokens: Vec<_> = expanded.into_iter().filter(significant).collect();
    super::expression::evaluate(&tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn macro_expansion_preserves_strings_and_supports_arguments_pasting_and_variadics() {
        let mut defs = Definitions::new();
        for definition in [
            "VALUE 3",
            "TWICE(x) ((x)+(x))",
            "NAME(a,b) a ## b",
            "TEXT(x) #x",
            "CALL(f, ...) f(__VA_ARGS__)",
            "RECURSIVE RECURSIVE",
        ] {
            let (name, value) = define(definition).unwrap();
            defs.insert(name, value);
        }
        let expand_text = |text| lex::join(&expand(&lex::lex(text), &defs).unwrap());
        assert_eq!(expand_text("TWICE(VALUE)"), "((3)+(3))");
        assert_eq!(expand_text("NAME(ll,Say)(0, TEXT( hello   world ))"), "llSay(0, \"hello world\")");
        assert_eq!(expand_text("CALL(llSay, 0, VALUE)"), "llSay( 0, 3)");
        assert_eq!(expand_text("\"VALUE\" RECURSIVE"), "\"VALUE\" RECURSIVE");
        assert!(condition("defined(VALUE) && VALUE == 3 && !defined NO", &defs, 1).unwrap());
        assert!(define("BAD(x,x) x").is_err());
        assert!(expand(&lex::lex("TWICE(1,2)"), &defs).is_err());
    }

    #[test]
    fn replacements_rescan_with_following_input_and_recursive_macros_terminate() {
        let mut defs = Definitions::new();
        for definition in [
            "ALIAS F",
            "F(x) (x)",
            "ID(x) x",
            "LEFT RIGHT",
            "RIGHT LEFT",
            "JOIN(a,b) a ## b",
            "PLUS +",
            "EMPTY(a,b) a ## b next",
            "BOTH(a,b) a ## b",
            "OBJECT alpha ## beta",
        ] {
            let (name, definition) = define(definition).unwrap();
            defs.insert(name, definition);
        }
        let expanded = |s| lex::join(&expand(&lex::lex(s), &defs).unwrap());
        assert_eq!(expanded("ALIAS(7) ID(F)(8) JOIN(AL,IAS)(9)"), "(7) (8) (9)");
        assert_eq!(expanded("LEFT RIGHT F(F(1))"), "LEFT RIGHT ((1))");
        assert_eq!(expanded("ALIAS\n(\n42\n)"), "(\n42\n)");
        assert_eq!(expanded("PLUS+1"), "+ +1");
        assert_eq!(expanded("EMPTY(value,)"), "value next");
        assert_eq!(expanded("BOTH(,)"), "");
        assert_eq!(expanded("OBJECT"), "alphabeta");
    }
}
