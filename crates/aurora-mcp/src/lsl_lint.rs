//! Local LSL and SLua (Luau) syntax and sanity checker (`C5`).
//!
//! Runs completely locally in milliseconds without uploading scripts to the
//! simulator, reporting exact 1-based `line` and `column` diagnostics.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LintDiagnostic {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LintReport {
    pub ok: bool,
    pub target: String,
    pub byte_len: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub diagnostics: Vec<LintDiagnostic>,
}

#[derive(Debug, Clone)]
enum TokKind {
    Ident(String),
    Literal,
    Punct(char),
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokKind,
    line: usize,
    col: usize,
}

fn diag(severity: &'static str, code: &'static str, message: impl Into<String>, line: usize, column: usize) -> LintDiagnostic {
    LintDiagnostic {
        severity,
        code,
        message: message.into(),
        line,
        column,
    }
}

fn is_luau_target(target: &str) -> bool {
    matches!(target.trim().to_ascii_lowercase().as_str(), "luau" | "slua")
}

/// Curated built-in LSL function signatures `(name, expected_arity)`.
const LSL_BUILTINS: &[(&str, usize)] = &[
    ("llAbs", 1),
    ("llAllowInventoryDrop", 1),
    ("llAtan2", 2),
    ("llAvatarOnSitTarget", 0),
    ("llAxes2Rot", 3),
    ("llAxisAngle2Rot", 2),
    ("llBase64ToString", 1),
    ("llBreakAllLinks", 0),
    ("llBreakLink", 1),
    ("llCastRay", 3),
    ("llCeil", 1),
    ("llClearLinkMedia", 2),
    ("llClearPrimMedia", 1),
    ("llCos", 1),
    ("llCreateLink", 2),
    ("llDeleteSubList", 3),
    ("llDeleteSubString", 3),
    ("llDetectedGroup", 1),
    ("llDetectedKey", 1),
    ("llDetectedLinkNumber", 1),
    ("llDetectedName", 1),
    ("llDetectedOwner", 1),
    ("llDetectedPos", 1),
    ("llDetectedRot", 1),
    ("llDetectedTouchBinormal", 1),
    ("llDetectedTouchFace", 1),
    ("llDetectedTouchNormal", 1),
    ("llDetectedTouchPos", 1),
    ("llDetectedTouchST", 1),
    ("llDetectedTouchUV", 1),
    ("llDetectedType", 1),
    ("llDialog", 4),
    ("llDie", 0),
    ("llDumpList2String", 2),
    ("llEuler2Rot", 1),
    ("llFabs", 1),
    ("llFloor", 1),
    ("llFrand", 1),
    ("llGetAlpha", 1),
    ("llGetAndResetTime", 0),
    ("llGetColor", 1),
    ("llGetCreator", 0),
    ("llGetDisplayName", 1),
    ("llGetFreeMemory", 0),
    ("llGetInventoryCreator", 1),
    ("llGetInventoryKey", 1),
    ("llGetInventoryName", 2),
    ("llGetInventoryNumber", 1),
    ("llGetInventoryPermMask", 2),
    ("llGetInventoryType", 1),
    ("llGetKey", 0),
    ("llGetLinkKey", 1),
    ("llGetLinkMedia", 3),
    ("llGetLinkName", 1),
    ("llGetLinkNumber", 0),
    ("llGetLinkPrimitiveParams", 2),
    ("llGetListEntryType", 2),
    ("llGetListLength", 1),
    ("llGetNotecardLine", 2),
    ("llGetNotecardLineSync", 2),
    ("llGetNumberOfNotecardLines", 1),
    ("llGetNumberOfPrims", 0),
    ("llGetObjectDesc", 0),
    ("llGetObjectDetails", 2),
    ("llGetObjectName", 0),
    ("llGetOwner", 0),
    ("llGetOwnerKey", 1),
    ("llGetParcelDetails", 2),
    ("llGetParcelFlags", 1),
    ("llGetPermissions", 0),
    ("llGetPermissionsKey", 0),
    ("llGetPos", 0),
    ("llGetPrimMediaParams", 2),
    ("llGetPrimitiveParams", 1),
    ("llGetRegionFPS", 0),
    ("llGetRegionName", 0),
    ("llGetRegionTimeDilation", 0),
    ("llGetRot", 0),
    ("llGetScale", 0),
    ("llGetScriptState", 1),
    ("llGetSubString", 3),
    ("llGetTexture", 1),
    ("llGetTime", 0),
    ("llGetTimestamp", 0),
    ("llGetUnixTime", 0),
    ("llGetUsedMemory", 0),
    ("llGetUsername", 1),
    ("llGiveInventory", 2),
    ("llGiveInventoryList", 3),
    ("llGiveMoney", 2),
    ("llHTTPRequest", 3),
    ("llHTTPResponse", 3),
    ("llInsertString", 3),
    ("llInstantMessage", 2),
    ("llJson2List", 1),
    ("llJsonGetValue", 2),
    ("llJsonSetValue", 3),
    ("llJsonValueType", 2),
    ("llKey2Name", 1),
    ("llLinkParticleSystem", 2),
    ("llLinksetDataAvailable", 0),
    ("llLinksetDataCountKeys", 0),
    ("llLinksetDataDelete", 1),
    ("llLinksetDataFindKeys", 3),
    ("llLinksetDataListKeys", 2),
    ("llLinksetDataRead", 1),
    ("llLinksetDataReset", 0),
    ("llLinksetDataWrite", 2),
    ("llList2Float", 2),
    ("llList2Integer", 2),
    ("llList2Json", 2),
    ("llList2Key", 2),
    ("llList2List", 3),
    ("llList2Rot", 2),
    ("llList2String", 2),
    ("llList2Vector", 2),
    ("llListFindList", 2),
    ("llListInsertList", 3),
    ("llListReplaceList", 4),
    ("llListSort", 3),
    ("llListen", 4),
    ("llListenControl", 2),
    ("llListenRemove", 1),
    ("llLoadURL", 3),
    ("llLog", 1),
    ("llLog10", 1),
    ("llLoopSound", 2),
    ("llMD5String", 2),
    ("llMessageLinked", 4),
    ("llMinEventDelay", 1),
    ("llOwnerSay", 1),
    ("llParseString2List", 3),
    ("llParseStringKeepNulls", 3),
    ("llParticleSystem", 1),
    ("llPlaySound", 2),
    ("llPow", 2),
    ("llPreloadSound", 1),
    ("llRegionSay", 2),
    ("llRegionSayTo", 3),
    ("llReleaseURL", 1),
    ("llRemoveInventory", 1),
    ("llRequestAgentData", 2),
    ("llRequestDisplayName", 1),
    ("llRequestPermissions", 2),
    ("llRequestSecureURL", 0),
    ("llRequestURL", 0),
    ("llRequestUsername", 1),
    ("llResetOtherScript", 1),
    ("llResetScript", 0),
    ("llResetTime", 0),
    ("llRezAtRoot", 5),
    ("llRezObject", 5),
    ("llRezObjectWithParams", 2),
    ("llRot2Euler", 1),
    ("llRotBetween", 2),
    ("llRound", 1),
    ("llSHA1String", 1),
    ("llSHA256String", 2),
    ("llSay", 2),
    ("llScaleByFactor", 1),
    ("llSensor", 4),
    ("llSensorRemove", 0),
    ("llSensorRepeat", 5),
    ("llSetAlpha", 2),
    ("llSetClickAction", 1),
    ("llSetColor", 2),
    ("llSetHoverHeight", 3),
    ("llSetLinkAlpha", 3),
    ("llSetLinkColor", 3),
    ("llSetLinkMedia", 3),
    ("llSetLinkPrimitiveParams", 2),
    ("llSetLinkPrimitiveParamsFast", 2),
    ("llSetLinkTexture", 3),
    ("llSetMemoryLimit", 1),
    ("llSetObjectDesc", 1),
    ("llSetObjectName", 1),
    ("llSetPayPrice", 2),
    ("llSetPos", 1),
    ("llSetPrimMediaParams", 2),
    ("llSetPrimitiveParams", 1),
    ("llSetRegionPos", 1),
    ("llSetRot", 1),
    ("llSetScale", 1),
    ("llSetScriptState", 2),
    ("llSetSoundQueueing", 1),
    ("llSetStatus", 2),
    ("llSetText", 3),
    ("llSetTexture", 2),
    ("llSetTimerEvent", 1),
    ("llShout", 2),
    ("llSin", 1),
    ("llSitTarget", 2),
    ("llSleep", 1),
    ("llSqrt", 1),
    ("llStartAnimation", 1),
    ("llStopAnimation", 1),
    ("llStopSound", 0),
    ("llStringLength", 1),
    ("llStringToBase64", 1),
    ("llStringTrim", 2),
    ("llSubStringIndex", 2),
    ("llTan", 1),
    ("llTargetOmega", 3),
    ("llTextBox", 3),
    ("llToLower", 1),
    ("llToUpper", 1),
    ("llTriggerSound", 2),
    ("llUnSit", 1),
    ("llVecDist", 2),
    ("llVecMag", 1),
    ("llVecNorm", 1),
    ("llWhisper", 2),
];

fn tokenize_and_check_delimiters(source: &str, luau: bool, diags: &mut Vec<LintDiagnostic>) -> Vec<Token> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut tokens = Vec::new();
    let mut stack: Vec<(char, usize, usize)> = Vec::new();

    let advance_nl = |idx: &mut usize, ln: &mut usize, cl: &mut usize, b: &[u8]| {
        if b[*idx] == b'\r' && b.get(*idx + 1) == Some(&b'\n') {
            *idx += 2;
        } else {
            *idx += 1;
        }
        *ln += 1;
        *cl = 1;
    };

    while i < len {
        let ch = bytes[i];
        if ch == b'\n' || ch == b'\r' {
            advance_nl(&mut i, &mut line, &mut col, bytes);
            continue;
        }
        if ch == b' ' || ch == b'\t' {
            i += 1;
            col += 1;
            continue;
        }

        // Line & block comments.
        if !luau && ch == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < len && bytes[i] != b'\n' && bytes[i] != b'\r' {
                i += 1;
                col += 1;
            }
            continue;
        }
        if !luau && ch == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let (sl, sc) = (line, col);
            i += 2;
            col += 2;
            let mut closed = false;
            while i < len {
                if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    col += 2;
                    closed = true;
                    break;
                }
                if bytes[i] == b'\n' || bytes[i] == b'\r' {
                    advance_nl(&mut i, &mut line, &mut col, bytes);
                } else {
                    i += 1;
                    col += 1;
                }
            }
            if !closed {
                diags.push(diag(
                    "error",
                    "E_UNTERMINATED_COMMENT",
                    "unterminated block comment /* ... */",
                    sl,
                    sc,
                ));
            }
            continue;
        }
        if luau && ch == b'-' && bytes.get(i + 1) == Some(&b'-') {
            if bytes.get(i + 2) == Some(&b'[') && bytes.get(i + 3) == Some(&b'[') {
                let (sl, sc) = (line, col);
                i += 4;
                col += 4;
                let mut closed = false;
                while i < len {
                    if bytes[i] == b']' && bytes.get(i + 1) == Some(&b']') {
                        i += 2;
                        col += 2;
                        closed = true;
                        break;
                    }
                    if bytes[i] == b'\n' || bytes[i] == b'\r' {
                        advance_nl(&mut i, &mut line, &mut col, bytes);
                    } else {
                        i += 1;
                        col += 1;
                    }
                }
                if !closed {
                    diags.push(diag(
                        "error",
                        "E_UNTERMINATED_COMMENT",
                        "unterminated Luau block comment --[[ ... ]]",
                        sl,
                        sc,
                    ));
                }
            } else {
                while i < len && bytes[i] != b'\n' && bytes[i] != b'\r' {
                    i += 1;
                    col += 1;
                }
            }
            continue;
        }

        // Quoted strings ("..." and in Luau also '...').
        if ch == b'"' || (luau && ch == b'\'') {
            let quote = ch;
            let (sl, sc) = (line, col);
            i += 1;
            col += 1;
            let mut closed = false;
            while i < len {
                let c = bytes[i];
                if c == b'\\' {
                    i += 1;
                    col += 1;
                    if i < len {
                        if bytes[i] == b'\n' || bytes[i] == b'\r' {
                            advance_nl(&mut i, &mut line, &mut col, bytes);
                        } else {
                            i += 1;
                            col += 1;
                        }
                    }
                    continue;
                }
                if c == quote {
                    i += 1;
                    col += 1;
                    closed = true;
                    break;
                }
                if !luau && (c == b'\n' || c == b'\r') {
                    // LSL strings may span lines, or commonly reflect a missing closing quote: keep scanning cleanly.
                    advance_nl(&mut i, &mut line, &mut col, bytes);
                } else if c == b'\n' || c == b'\r' {
                    break;
                } else {
                    i += 1;
                    col += 1;
                }
            }
            if !closed {
                diags.push(diag("error", "E_UNTERMINATED_STRING", "unterminated string literal", sl, sc));
            }
            tokens.push(Token {
                kind: TokKind::Literal,
                line: sl,
                col: sc,
            });
            continue;
        }

        // Identifiers / numbers.
        if ch.is_ascii_alphabetic() || ch == b'_' {
            let start = i;
            let sc = col;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
                col += 1;
            }
            let text = String::from_utf8_lossy(&bytes[start..i]).into_owned();
            tokens.push(Token {
                kind: TokKind::Ident(text),
                line,
                col: sc,
            });
            continue;
        }
        if ch.is_ascii_digit() {
            let sc = col;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_') {
                i += 1;
                col += 1;
            }
            tokens.push(Token {
                kind: TokKind::Literal,
                line,
                col: sc,
            });
            continue;
        }

        let c = ch as char;
        if matches!(c, '(' | '[' | '{') {
            stack.push((c, line, col));
        } else if matches!(c, ')' | ']' | '}') {
            let expected = match c {
                ')' => '(',
                ']' => '[',
                '}' => '{',
                _ => unreachable!(),
            };
            match stack.pop() {
                Some((open, _, _)) if open == expected => {}
                Some((open, ol, oc)) => {
                    diags.push(diag(
                        "error",
                        "E_MISMATCHED_DELIMITER",
                        format!("unexpected '{c}'; expected closing delimiter for '{open}' opened at {ol}:{oc}"),
                        line,
                        col,
                    ));
                }
                None => {
                    diags.push(diag(
                        "error",
                        "E_UNEXPECTED_DELIMITER",
                        format!("unexpected closing delimiter '{c}' with no matching opener"),
                        line,
                        col,
                    ));
                }
            }
        }
        if matches!(c, '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '<' | '>' | '=' | '+' | '-') {
            tokens.push(Token {
                kind: TokKind::Punct(c),
                line,
                col,
            });
        }
        // Advance UTF-8 safe single codepoint / byte.
        let step = source[i..].chars().next().map_or(1, char::len_utf8);
        i += step;
        col += 1;
    }

    while let Some((open, ol, oc)) = stack.pop() {
        diags.push(diag(
            "error",
            "E_UNCLOSED_DELIMITER",
            format!("unclosed '{open}' opened at line {ol}, column {oc}"),
            ol,
            oc,
        ));
    }

    tokens
}

fn check_lsl_tokens(tokens: &[Token], byte_len: usize, target: &str, diags: &mut Vec<LintDiagnostic>) {
    static LSL_BUILTIN_MAP: std::sync::OnceLock<HashMap<&'static str, usize>> = std::sync::OnceLock::new();
    let builtins = LSL_BUILTIN_MAP.get_or_init(|| LSL_BUILTINS.iter().copied().collect());
    let mut depth = 0isize;
    let mut paren_depth = 0isize;
    let mut has_default = false;
    let mut seen_states: HashSet<String> = HashSet::new();
    let mut current_state: Option<String> = None;
    let mut seen_events_in_state: HashSet<String> = HashSet::new();
    let mut expect_state_brace: Option<(String, usize, usize)> = None;

    let mut i = 0usize;
    while i < tokens.len() {
        let tok = &tokens[i];
        match &tok.kind {
            TokKind::Punct('{') => {
                if depth == 0
                    && let Some((ref state_name, sl, sc)) = expect_state_brace.take()
                {
                    if !seen_states.insert(state_name.clone()) {
                        diags.push(diag(
                            "error",
                            "E_DUPLICATE_STATE",
                            format!("duplicate state '{state_name}'"),
                            sl,
                            sc,
                        ));
                    }
                    if state_name == "default" {
                        has_default = true;
                    }
                    current_state = Some(state_name.clone());
                    seen_events_in_state.clear();
                }
                depth += 1;
            }
            TokKind::Punct('}') => {
                depth = (depth - 1).max(0);
                if depth == 0 {
                    current_state = None;
                    seen_events_in_state.clear();
                }
            }
            TokKind::Punct('(') => {
                paren_depth += 1;
            }
            TokKind::Punct(')') => {
                paren_depth = (paren_depth - 1).max(0);
            }
            TokKind::Ident(ident) => {
                // Top-level state declaration: `default { ... }` or `state name { ... }`
                if depth == 0 && paren_depth == 0 {
                    if ident == "default" && matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokKind::Punct('{'))) {
                        expect_state_brace = Some(("default".to_owned(), tok.line, tok.col));
                    } else if ident == "state"
                        && let Some(Token {
                            kind: TokKind::Ident(name),
                            line,
                            col,
                        }) = tokens.get(i + 1)
                        && matches!(tokens.get(i + 2).map(|t| &t.kind), Some(TokKind::Punct('{')))
                    {
                        expect_state_brace = Some((name.clone(), *line, *col));
                    }
                }
                // Inside state body at depth == 1: event handler `<event_name>(...) {`
                if depth == 1
                    && paren_depth == 0
                    && current_state.is_some()
                    && matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokKind::Punct('(')))
                    && !seen_events_in_state.insert(ident.clone())
                {
                    diags.push(diag(
                        "error",
                        "E_DUPLICATE_EVENT",
                        format!(
                            "duplicate event '{ident}' in state '{}'",
                            current_state.as_deref().unwrap_or("default")
                        ),
                        tok.line,
                        tok.col,
                    ));
                }

                // LSL library call arity check (`ll*(`) at expression depth >= 2
                if depth >= 2
                    && ident.starts_with("ll")
                    && ident.len() > 2
                    && ident.as_bytes()[2].is_ascii_uppercase()
                    && matches!(tokens.get(i + 1).map(|t| &t.kind), Some(TokKind::Punct('(')))
                {
                    if let Some(&expected_arity) = builtins.get(ident.as_str()) {
                        // Count top-level arguments inside `(...)` when closing paren matches on token stream.
                        let mut p = 1isize;
                        let mut br = 0isize;
                        let mut ang = 0isize;
                        let mut j = i + 2;
                        let mut commas = 0usize;
                        let mut has_any = false;
                        while j < tokens.len() && p > 0 {
                            match &tokens[j].kind {
                                TokKind::Punct('(') => {
                                    if p == 1 {
                                        has_any = true;
                                    }
                                    p += 1;
                                }
                                TokKind::Punct(')') => p -= 1,
                                TokKind::Punct('[') => {
                                    if p == 1 {
                                        has_any = true;
                                    }
                                    br += 1;
                                }
                                TokKind::Punct(']') => br = (br - 1).max(0),
                                TokKind::Punct('<') => {
                                    let starts_vec =
                                        j == i + 2 || matches!(tokens[j - 1].kind, TokKind::Punct('(' | ',' | '[' | '=' | '+' | '-' | '<'));
                                    if starts_vec {
                                        if p == 1 {
                                            has_any = true;
                                        }
                                        ang += 1;
                                    }
                                }
                                TokKind::Punct('>') if ang > 0 => ang -= 1,
                                TokKind::Punct(',') if p == 1 && br == 0 && ang == 0 => commas += 1,
                                TokKind::Punct('{') | TokKind::Ident(_) | TokKind::Literal if p == 1 => has_any = true,
                                _ => {}
                            }
                            j += 1;
                        }
                        if p == 0 {
                            let actual_arity = if !has_any && commas == 0 { 0 } else { commas + 1 };
                            if actual_arity != expected_arity {
                                diags.push(diag(
                                    "warning",
                                    "W_LL_ARITY",
                                    format!("{ident} expects {expected_arity} argument(s), found {actual_arity}"),
                                    tok.line,
                                    tok.col,
                                ));
                            }
                        }
                    } else {
                        diags.push(diag(
                            "warning",
                            "W_UNKNOWN_LL_FUNC",
                            format!("unrecognized ll* built-in '{ident}' (verify spelling if not custom/new)"),
                            tok.line,
                            tok.col,
                        ));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    if !has_default {
        diags.push(diag(
            "error",
            "E_NO_DEFAULT_STATE",
            "LSL script is missing a 'default { ... }' state",
            1,
            1,
        ));
    }

    let limit = if target.eq_ignore_ascii_case("lsl2") {
        16 * 1024
    } else {
        64 * 1024
    };
    if byte_len > limit {
        diags.push(diag(
            "warning",
            "W_SCRIPT_SIZE",
            format!("script is {byte_len} bytes, exceeding the typical {limit}-byte {target} source budget"),
            1,
            1,
        ));
    }
}

fn check_luau_tokens(tokens: &[Token], diags: &mut Vec<LintDiagnostic>) {
    let mut blocks: Vec<(&str, usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < tokens.len() {
        let tok = &tokens[i];
        if let TokKind::Ident(ident) = &tok.kind {
            // Skip method/field names after `.` or `:`? Wait: `tokens` omits `.`, keep `:` check.
            let after_colon = i > 0 && matches!(tokens[i - 1].kind, TokKind::Punct(':'));
            if !after_colon {
                match ident.as_str() {
                    "function" | "if" | "do" => blocks.push((ident.as_str(), tok.line, tok.col)),
                    "repeat" => blocks.push(("repeat", tok.line, tok.col)),
                    "end" => match blocks.pop() {
                        Some(("repeat", ol, oc)) => diags.push(diag(
                            "error",
                            "E_LUAU_BLOCK",
                            format!("unexpected 'end'; 'repeat' at {ol}:{oc} must close with 'until'"),
                            tok.line,
                            tok.col,
                        )),
                        Some(_) => {}
                        None => diags.push(diag(
                            "error",
                            "E_LUAU_END",
                            "unexpected 'end' with no matching block",
                            tok.line,
                            tok.col,
                        )),
                    },
                    "until" => match blocks.pop() {
                        Some(("repeat", _, _)) => {}
                        Some((open, ol, oc)) => diags.push(diag(
                            "error",
                            "E_LUAU_BLOCK",
                            format!("unexpected 'until'; '{open}' at {ol}:{oc} must close with 'end'"),
                            tok.line,
                            tok.col,
                        )),
                        None => diags.push(diag(
                            "error",
                            "E_LUAU_UNTIL",
                            "unexpected 'until' with no matching 'repeat'",
                            tok.line,
                            tok.col,
                        )),
                    },
                    _ => {}
                }
            }
        }
        i += 1;
    }
    while let Some((open, ol, oc)) = blocks.pop() {
        let closer = if open == "repeat" { "until" } else { "end" };
        diags.push(diag(
            "error",
            "E_LUAU_UNCLOSED",
            format!("unclosed '{open}' opened at line {ol}, column {oc} (expected '{closer}')"),
            ol,
            oc,
        ));
    }
}

pub fn lint_script(source: &str, target: &str) -> LintReport {
    let norm_target = if target.trim().is_empty() { "mono" } else { target.trim() };
    let luau = is_luau_target(norm_target);
    let mut diags = Vec::new();
    let byte_len = source.len();

    if source.trim().is_empty() {
        diags.push(diag("error", "E_EMPTY_SCRIPT", "script source is empty", 1, 1));
    } else {
        let tokens = tokenize_and_check_delimiters(source, luau, &mut diags);
        if luau {
            check_luau_tokens(&tokens, &mut diags);
        } else {
            check_lsl_tokens(&tokens, byte_len, norm_target, &mut diags);
        }
    }

    let error_count = diags.iter().filter(|d| d.severity == "error").count();
    let warning_count = diags.iter().filter(|d| d.severity == "warning").count();
    LintReport {
        ok: error_count == 0,
        target: norm_target.to_owned(),
        byte_len,
        error_count,
        warning_count,
        diagnostics: diags,
    }
}

pub fn check_script(source: &str, target: &str) -> Value {
    serde_json::to_value(lint_script(source, target)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_lsl_script_passes_cleanly() {
        let src = r#"
default
{
    state_entry()
    {
        llSay(0, "Hello, Second Life!");
        llSetColor(<1.0, 0.5, 0.0>, 0);
        llCastRay(<0.0, 0.0, 0.0>, <1.0, 0.0, 0.0>, [RC_MAX_HITS, 1]);
    }
    touch_start(integer total_number)
    {
        llOwnerSay("Touched by " + llDetectedName(0));
    }
}
"#;
        let report = lint_script(src, "mono");
        assert!(report.ok, "{:?}", report.diagnostics);
        assert_eq!(report.error_count, 0);
        assert_eq!(report.warning_count, 0);
    }

    #[test]
    fn detects_missing_default_unclosed_delimiters_duplicate_state_and_arity() {
        let missing = lint_script("state idle { state_entry() { llSay(0); } }", "mono");
        assert!(!missing.ok);
        assert!(missing.diagnostics.iter().any(|d| d.code == "E_NO_DEFAULT_STATE"));
        assert!(missing.diagnostics.iter().any(|d| d.code == "W_LL_ARITY"));

        let unclosed = lint_script("default {\n  state_entry() {\n    llOwnerSay(\"unterminated);\n", "mono");
        assert!(!unclosed.ok);
        assert!(
            unclosed
                .diagnostics
                .iter()
                .any(|d| d.code == "E_UNTERMINATED_STRING" && d.line == 3)
        );
        assert!(unclosed.diagnostics.iter().any(|d| d.code == "E_UNCLOSED_DELIMITER"));

        let dup = lint_script(
            "default { state_entry() {} state_entry() {} }\nstate s { state_entry() {} }\nstate s { state_entry() {} }",
            "mono",
        );
        assert!(!dup.ok);
        assert!(dup.diagnostics.iter().any(|d| d.code == "E_DUPLICATE_EVENT"));
        assert!(dup.diagnostics.iter().any(|d| d.code == "E_DUPLICATE_STATE"));
    }

    #[test]
    fn luau_block_balance_works() {
        let ok = lint_script(
            "-- comment\nlocal function greet(name)\n  if name ~= '' then\n    ll.OwnerSay(name)\n  end\nend\n",
            "luau",
        );
        assert!(ok.ok, "{:?}", ok.diagnostics);

        let bad = lint_script("local function f()\n  if true then\n    ll.Say(0, 'hi')\nend\n", "luau");
        assert!(!bad.ok);
        assert!(bad.diagnostics.iter().any(|d| d.code == "E_LUAU_UNCLOSED"));
    }
}
