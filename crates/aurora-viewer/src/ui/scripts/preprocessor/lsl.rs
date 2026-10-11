//! FSLSLPreprocessor::lslopt, reformat_lazy_lists and
//! reformat_switch_statements (Firestorm, BSD-3-Clause). Parse tokens/scopes
//! rather than matching comments and strings with recursive regular expressions.

use super::lex::{self, Kind};
use std::collections::{HashMap, HashSet};

fn error() -> String {
    aurora_i18n::tr!("script-preproc-lsl-extension").into()
}

fn replace(source: &str, start: usize, end: usize, replacement: &str) -> Result<String, String> {
    if source.len() - (end - start) + replacement.len() > aurora_net::scripts::MAX_SOURCE {
        return Err(aurora_i18n::tr!("script-source-too-large").into());
    }
    Ok(format!("{}{replacement}{}", &source[..start], &source[end..]))
}

const LAZY_SET: &str = "list lazy_list_set(list L, integer i, list v)\n{\n    while (llGetListLength(L) < i) L = L + 0;\n    return llListReplaceList(L, v, i, i);\n}\n";

pub(super) fn lazy_lists(source: &str) -> Result<String, String> {
    let mut source = source.to_owned();
    let mut passes = 0;
    loop {
        passes += 1;
        if passes > 1000 {
            return Err(aurora_i18n::tr!("script-preproc-expansion-limit").into());
        }
        let tokens = lex::code(&source);
        let mut changed = false;
        for (at, token) in tokens.iter().enumerate() {
            if token.kind == Kind::Word
                && !matches!(token.text.as_str(), "return" | "do" | "else")
                && tokens.get(at + 1).is_some_and(|t| t.text == "[")
            {
                let end = lex::close(&tokens, at + 1, "[", "]").ok_or_else(error)?;
                if tokens.get(end + 1).is_some_and(|t| t.text == "=") {
                    let mut next = end + 2;
                    let mut depth = 0;
                    while next < tokens.len() {
                        match tokens[next].text.as_str() {
                            ";" if depth == 0 => break,
                            ")" if depth == 0 => break,
                            "(" | "[" => depth += 1,
                            ")" | "]" => depth -= 1,
                            _ => {}
                        }
                        next += 1;
                    }
                    if next == tokens.len() || next == end + 2 {
                        return Err(error());
                    }
                    let index = &source[tokens[at + 1].end..tokens[end].start];
                    let value = &source[tokens[end + 1].end..tokens[next].start];
                    let rewritten = format!("{}=lazy_list_set({},({index}),[{value}])", token.text, token.text);
                    source = replace(&source, token.start, tokens[next].start, &rewritten)?;
                    changed = true;
                    break;
                }
            }
            if token.text == "("
                && let (Some(kind), Some(close), Some(name), Some(open)) =
                    (tokens.get(at + 1), tokens.get(at + 2), tokens.get(at + 3), tokens.get(at + 4))
            {
                let function = match kind.text.as_str() {
                    "integer" => "llList2Integer",
                    "float" => "llList2Float",
                    "string" => "llList2String",
                    "key" => "llList2Key",
                    "vector" => "llList2Vector",
                    "rotation" | "quaternion" => "llList2Rot",
                    "list" => "llList2List",
                    _ => continue,
                };
                let (name, opening, wrapped) = if close.text == ")" && name.text == "(" && tokens.get(at + 5).is_some_and(|t| t.text == "[")
                {
                    (open, at + 5, true)
                } else {
                    (name, at + 4, false)
                };
                if close.text != ")" || name.kind != Kind::Word || tokens.get(opening).is_none_or(|t| t.text != "[") {
                    continue;
                }
                let end = lex::close(&tokens, opening, "[", "]").ok_or_else(error)?;
                if wrapped && tokens.get(end + 1).is_none_or(|t| t.text != ")") {
                    continue;
                }
                let index = &source[tokens[opening].end..tokens[end].start];
                let rewritten = if function == "llList2List" {
                    // Firestorm passes one index too, but llList2List needs both
                    // bounds. Evaluate it once through a helper for side effects.
                    format!("lazy_list_get_list({},({index}))", name.text)
                } else {
                    format!("{function}({},({index}))", name.text)
                };
                source = replace(&source, token.start, tokens[end + usize::from(wrapped)].end, &rewritten)?;
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    let list_helper = if lex::code(&source).iter().any(|t| t.text == "lazy_list_get_list") {
        "list lazy_list_get_list(list L, integer i) { return llList2List(L, i, i); }\n"
    } else {
        ""
    };
    Ok(format!("{LAZY_SET}{list_helper}{source}"))
}

pub(super) fn switches(source: &str) -> Result<String, String> {
    let mut serial = 0;
    let mut prefix = "__aurora_switch_".to_owned();
    while source.contains(&prefix) {
        prefix.push('_');
    }
    switches_inner(source, &prefix, &mut serial, 0)
}

fn switches_inner(source: &str, prefix: &str, serial: &mut usize, depth: usize) -> Result<String, String> {
    if depth > 64 {
        return Err(aurora_i18n::tr!("script-preproc-expansion-limit").into());
    }
    let mut source = source.to_owned();
    loop {
        let tokens = lex::code(&source);
        let Some(start) = tokens.iter().position(|t| t.kind == Kind::Word && t.text == "switch") else {
            return Ok(source);
        };
        let arg_end = lex::close(&tokens, start + 1, "(", ")").ok_or_else(error)?;
        let block_end = lex::close(&tokens, arg_end + 1, "{", "}").ok_or_else(error)?;
        let argument = &source[tokens[start + 1].end..tokens[arg_end].start];
        let body = switches_inner(&source[tokens[arg_end + 1].end..tokens[block_end].start], prefix, serial, depth + 1)?;
        let body_tokens = lex::code(&body);
        let number = *serial;
        *serial += 1;
        let ending = format!("{prefix}{number}_end");
        let mut default = ending.clone();
        let mut cases = Vec::new();
        let mut replacements = Vec::new();
        let mut at = 0;
        while at < body_tokens.len() {
            let token = &body_tokens[at];
            if token.kind == Kind::Word && matches!(token.text.as_str(), "case" | "default") {
                let terminator = (at + 1..body_tokens.len())
                    .find(|i| matches!(body_tokens[*i].text.as_str(), ":" | "{"))
                    .ok_or_else(error)?;
                let label = if token.text == "default" {
                    format!("{prefix}{number}_default")
                } else {
                    format!("{prefix}{number}_case{}", cases.len())
                };
                if token.text == "default" {
                    if default != ending {
                        return Err(error());
                    }
                    default = label.clone();
                } else {
                    cases.push((body[token.end..body_tokens[terminator].start].trim().to_owned(), label.clone()));
                }
                let retained = if body_tokens[terminator].text == "{" { "{" } else { "" };
                replacements.push((token.start, body_tokens[terminator].end, format!("@{label};{retained}")));
                at = terminator + 1;
            } else if token.kind == Kind::Word && token.text == "break" && body_tokens.get(at + 1).is_some_and(|t| t.text == ";") {
                replacements.push((token.start, body_tokens[at + 1].end, format!("jump {ending};")));
                at += 2;
            } else {
                at += 1;
            }
        }
        let mut rewritten_body = body;
        for (start, end, text) in replacements.into_iter().rev() {
            rewritten_body.replace_range(start..end, &text);
        }
        let mut rewritten = "{\n".to_owned();
        // As in Firestorm, preserve case fall-through using labels and jumps.
        for (value, label) in cases {
            rewritten.push_str(&format!("if (({argument}) == ({value})) jump {label};\n"));
        }
        rewritten.push_str(&format!("jump {default};\n{rewritten_body}\n@{ending};\n}}"));
        source = replace(&source, tokens[start].start, tokens[block_end].end, &rewritten)?;
    }
}

struct Declaration {
    start: usize,
    end: usize,
    name: String,
    function: bool,
}

pub(super) fn optimize(source: &str) -> String {
    let tokens = lex::code(source);
    let mut declarations = Vec::new();
    let mut at = 0;
    while at < tokens.len() && tokens[at].text != "default" {
        let start = at;
        let typed = matches!(
            tokens[at].text.as_str(),
            "integer" | "float" | "string" | "key" | "vector" | "rotation" | "quaternion" | "list"
        );
        let name = at + usize::from(typed);
        if tokens.get(name).is_none_or(|t| t.kind != Kind::Word) {
            return source.into();
        }
        let Some(next) = tokens.get(name + 1) else { return source.into() };
        let function = next.text == "(";
        let end = if function {
            let Some(args) = lex::close(&tokens, name + 1, "(", ")") else {
                return source.into();
            };
            let Some(end) = lex::close(&tokens, args + 1, "{", "}") else {
                return source.into();
            };
            end
        } else {
            if !typed {
                return source.into();
            }
            let Some(end) = (name + 1..tokens.len()).find(|i| tokens[*i].text == ";") else {
                return source.into();
            };
            end
        };
        declarations.push(Declaration {
            start: tokens[start].start,
            end: tokens[end].end,
            name: tokens[name].text.clone(),
            function,
        });
        at = end + 1;
    }
    if at == tokens.len() {
        return source.into();
    }
    let state_start = tokens[at].start;
    let by_name: HashMap<_, _> = declarations.iter().enumerate().map(|(i, d)| (d.name.as_str(), i)).collect();
    let mut used = HashSet::new();
    let mut pending = vec![&source[state_start..]];
    while let Some(text) = pending.pop() {
        let tokens = lex::code(text);
        for (i, token) in tokens.iter().enumerate() {
            if token.kind != Kind::Word || i > 0 && tokens[i - 1].text == "." {
                continue;
            }
            if let Some(index) = by_name.get(token.text.as_str()) {
                let declaration = &declarations[*index];
                if declaration.function && tokens.get(i + 1).is_none_or(|t| t.text != "(") {
                    continue;
                }
                if used.insert(*index) {
                    pending.push(&source[declaration.start..declaration.end]);
                }
            }
        }
    }
    let mut output = String::new();
    for (index, declaration) in declarations.into_iter().enumerate() {
        if used.contains(&index) {
            output.push_str(&source[declaration.start..declaration.end]);
            output.push('\n');
        }
    }
    output.push_str(&source[state_start..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lazy_lists_rewrite_nested_typed_reads_and_assignments_without_touching_strings() {
        let source =
            "default { state_entry() { integer x = (integer)L[(integer)N[0]]; L[3] = x; llSay(0, \"L[2]=1;\"); list one = (list)L[0]; } }";
        let result = lazy_lists(source).unwrap();
        assert!(result.contains("llList2Integer(L,(llList2Integer(N,(0))))"));
        assert!(result.contains("L=lazy_list_set(L,(3),[ x])"));
        assert!(result.contains("\"L[2]=1;\""));
        assert!(result.contains("lazy_list_get_list(L,(0))"));
    }
    #[test]
    fn switch_preserves_fallthrough_nested_switches_and_absent_default_exit() {
        let source = "default { touch_start(integer n) { switch(n) { case 0: llSay(0,\"break;\"); case 1: switch(n+1) { case 2: break; } break; } } }";
        let result = switches(source).unwrap();
        assert!(
            !lex::code(&result)
                .iter()
                .any(|t| matches!(t.text.as_str(), "switch" | "case" | "break"))
        );
        assert!(result.contains("\"break;\""));
        assert!(result.contains("jump __aurora_switch_0_end;"));
        assert!(result.contains("jump __aurora_switch_1_end;"));
        assert!(switches("switch(x) { default: ; default: ; }").is_err());
    }
    #[test]
    fn optimizer_keeps_transitive_calls_and_global_dependencies_and_drops_unused_code() {
        let source = "integer used=1; integer unused=2; integer base=3; integer dependent=base; unused_function() { llSay(0,\"used\"); } leaf() { llSay(0,(string)used); } caller() { leaf(); } default { state_entry() { caller(); llSay(0,(string)dependent); } }";
        let result = optimize(source);
        assert!(!result.contains("unused"));
        for text in ["integer used", "integer base", "integer dependent", "leaf()", "caller()", "default"] {
            assert!(result.contains(text), "{text}");
        }
        assert_eq!(optimize("syntax error"), "syntax error");
    }
}
