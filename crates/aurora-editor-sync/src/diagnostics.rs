//! `script.compiled`: a save's compile result, as the editor's diagnostics.
//!
//! Port of LLScriptEditorWSServer::sendCompileResults and its
//! LSL_LOCATION_PATTERN / LUAU_LOCATION_PATTERN
//! (indra/newview/llscripteditorws.cpp, originally LGPL 2.1). The compiler's
//! messages name a line, a column and the error: never the script's text.

use serde_json::{Value, json};

/// `(line, column) : SEVERITY : message`, 0-based (the LSL compiler's).
/// The whole text must match, as boost::regex_match does.
pub(crate) fn lsl_location(text: &str) -> Option<(u32, u32, &str, &str)> {
    let rest = text.strip_prefix('(')?;
    let (line, rest) = digits(rest)?;
    let rest = rest.strip_prefix(", ")?;
    let (column, rest) = digits(rest)?;
    let rest = rest.strip_prefix(") : ")?;
    // `([^:]+) : (.+)`: the severity runs to the first colon
    let colon = rest.find(':')?;
    let severity = rest[..colon].strip_suffix(' ')?;
    let message = rest[colon..].strip_prefix(": ")?;
    if severity.is_empty() || message.is_empty() {
        return None;
    }
    Some((line, column, severity, message))
}

/// `source:line: message` (Luau's), line 1-based.
pub(crate) fn luau_location(text: &str) -> Option<(&str, u32, &str)> {
    let (source, rest) = text.split_once(':')?;
    let (line, rest) = digits(rest)?;
    let message = rest.strip_prefix(':')?.trim_start();
    Some((source, line, message))
}

/// Leading ASCII digits that fit an S32 below S32_MAX (the Legacy
/// Viewer's convertToS32 and its `< S32_MAX` check).
fn digits(text: &str) -> Option<(u32, &str)> {
    let end = text.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    let value: u32 = text[..end].parse().ok()?;
    if value >= i32::MAX as u32 {
        return None;
    }
    Some((value, &text[end..]))
}

/// The params of `script.compiled`.
pub fn compiled(script_id: &str, compiled: bool, running: bool, errors: &[String], lua: bool) -> Value {
    let mut params = json!({ "script_id": script_id, "success": compiled, "running": running });
    if errors.is_empty() {
        return params;
    }
    let diagnostics: Vec<Value> = errors
        .iter()
        .map(|err| {
            if lua {
                // Lua errors: ":line: message"; no column from the compiler
                match luau_location(err) {
                    Some((_, line, message)) => json!({ "column": 0, "level": "ERROR", "row": line, "message": message }),
                    None => json!({ "column": 0, "level": "ERROR", "row": 0, "message": err }),
                }
            } else {
                match lsl_location(err) {
                    Some((line, column, severity, message)) => {
                        json!({ "row": line + 1, "column": column + 1, "level": severity, "message": message, "format": "lsl" })
                    }
                    None => json!({ "row": 0, "column": 0, "level": "ERROR", "message": err, "format": "lsl" }),
                }
            }
        })
        .collect();
    params["diagnostics"] = Value::Array(diagnostics);
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsl_errors_become_one_based_diagnostics() {
        let p = compiled(
            "abc",
            false,
            false,
            &["(12, 4) : ERROR : Syntax error".into(), "Compile failed".into()],
            false,
        );
        assert_eq!(p["script_id"], json!("abc"));
        assert_eq!(p["success"], json!(false));
        assert_eq!(
            p["diagnostics"][0],
            json!({"row": 13, "column": 5, "level": "ERROR", "message": "Syntax error", "format": "lsl"})
        );
        assert_eq!(
            p["diagnostics"][1],
            json!({"row": 0, "column": 0, "level": "ERROR", "message": "Compile failed", "format": "lsl"})
        );
    }

    #[test]
    fn lua_errors_keep_their_line() {
        let p = compiled("abc", false, false, &[":7: Expected 'end'".into(), "oops".into()], true);
        assert_eq!(
            p["diagnostics"][0],
            json!({"row": 7, "column": 0, "level": "ERROR", "message": "Expected 'end'"})
        );
        assert_eq!(
            p["diagnostics"][1],
            json!({"row": 0, "column": 0, "level": "ERROR", "message": "oops"})
        );
    }

    #[test]
    fn a_clean_compile_has_no_diagnostics() {
        assert_eq!(
            compiled("abc", true, true, &[], false),
            json!({"script_id": "abc", "success": true, "running": true})
        );
    }

    #[test]
    fn odd_locations_fall_back_to_the_whole_message() {
        for text in [
            "(99999999999, 1) : ERROR : x",
            "(2147483647, 1) : ERROR : x",
            "(1, 2) : : x",
            "(1, 2) : ERROR : ",
            "(1,2) : ERROR : x",
            "(a, 2) : ERROR : x",
            "(1, 2) ERROR : x",
            "",
            "(",
            "(1, 2) : ERROR",
        ] {
            assert_eq!(lsl_location(text), None, "{text}");
        }
        assert_eq!(lsl_location("(0, 0) : WARNING : a : b"), Some((0, 0, "WARNING", "a : b")));
        assert_eq!(luau_location("src:3:  spaced"), Some(("src", 3, "spaced")));
        assert_eq!(luau_location("no line here"), None);
        assert_eq!(luau_location(":x: y"), None);
    }
}
