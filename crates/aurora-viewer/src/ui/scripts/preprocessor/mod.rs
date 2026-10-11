//! FSLSLPreprocessor::start_process / encode / decode (Firestorm,
//! indra/newview/fslslpreproc.cpp, originally BSD-3-Clause; see the bundled
//! PREPROCESSOR-LICENSE.txt). The worker never blocks the viewer's frame.

use super::settings::ScriptSettings;
use aurora_net::scripts::MAX_SOURCE;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

mod clock;
mod expression;
mod lex;
mod lsl;
mod macros;

const START: &str = "//start_unprocessed_text\n/*";
const END: &str = "*/\n//end_unprocessed_text";

pub(super) fn decode(source: &str) -> String {
    let Some(body) = source.strip_prefix(START).and_then(|s| s.split_once(END).map(|p| p.0)) else {
        return source.into();
    };
    let mut output = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        output.push(c);
        if matches!(c, '/' | '*') && chars.peek() == Some(&'|') {
            chars.next();
        }
    }
    output
}

fn encode(source: &str) -> String {
    let mut output = START.to_owned();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        output.push(c);
        if matches!(c, '/' | '*') && chars.peek().is_some_and(|c| matches!(c, '/' | '*' | '|')) {
            output.push('|');
        }
    }
    output.push_str(END);
    output.push_str("\n//nfo_preprocessor_version 0\n");
    output
}

#[derive(Clone, Default)]
pub(super) struct Environment {
    pub agent: Uuid,
    pub agent_name: String,
}
pub(super) struct Include {
    pub source: String,
    pub asset: Uuid,
}
pub(super) struct Request {
    pub id: Uuid,
    pub name: String,
    pub reply: mpsc::Sender<Result<Include, String>>,
}
pub(super) struct Output {
    pub uploaded: String,
    #[cfg(test)]
    pub generated: String,
    pub mono: Option<bool>,
    pub warnings: Vec<String>,
}

pub(super) struct Job {
    pub done: mpsc::Receiver<Result<Output, String>>,
    pub includes: mpsc::Receiver<Request>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub(super) fn start(source: String, name: String, settings: ScriptSettings, environment: Environment) -> Job {
    let (done_tx, done) = mpsc::channel();
    let (include_tx, includes) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = cancelled.clone();
    std::thread::spawn(move || {
        let mut loader = |include: &str, from: &str| -> Result<Include, String> {
            let (tx, rx) = mpsc::channel();
            include_tx
                .send(Request {
                    id: Uuid::new_v4(),
                    name: include.into(),
                    reply: tx,
                })
                .map_err(|_| aurora_i18n::tr!("script-preproc-cancelled").to_owned())?;
            let start = Instant::now();
            let result = loop {
                if stop.load(Ordering::Relaxed) {
                    return Err(aurora_i18n::tr!("script-preproc-cancelled").into());
                }
                if start.elapsed() > Duration::from_secs(45) {
                    return Err(aurora_i18n::tr!("script-preproc-include-timeout").into());
                }
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(result) => break result,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return Err(aurora_i18n::tr!("script-preproc-cancelled").into()),
                }
            };
            match result {
                Ok(include) => Ok(include),
                Err(_) if settings.disk_includes && !settings.include_directory.is_empty() => {
                    let root = std::path::Path::new(&settings.include_directory);
                    let from = std::path::Path::new(from);
                    let relative = from
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .map(|p| root.join(p).join(include));
                    let path = relative.filter(|p| p.is_file()).unwrap_or_else(|| root.join(include));
                    super::files::read(&path, false).map(|source| Include {
                        source,
                        asset: Uuid::nil(),
                    })
                }
                Err(reason) => Err(reason),
            }
        };
        let result = process(&source, &name, &settings, &environment, &mut loader);
        let _ = done_tx.send(result);
    });
    Job { done, includes, cancelled }
}

struct Conditional {
    parent: bool,
    active: bool,
    taken: bool,
    otherwise: bool,
}

struct Processor<'a, F> {
    definitions: macros::Definitions,
    once: HashSet<String>,
    loader: &'a mut F,
    warnings: Vec<String>,
    operations: usize,
}

pub(super) fn process<F>(
    source: &str,
    name: &str,
    settings: &ScriptSettings,
    environment: &Environment,
    loader: &mut F,
) -> Result<Output, String>
where
    F: FnMut(&str, &str) -> Result<Include, String>,
{
    let mut definitions = macros::Definitions::new();
    let (date, time) = clock::builtins();
    for (name, value) in [
        ("__AGENTID__", lex::quoted(&environment.agent.to_string())),
        ("__AGENTKEY__", lex::quoted(&environment.agent.to_string())),
        ("__AGENTIDRAW__", environment.agent.to_string()),
        ("__AGENTNAME__", lex::quoted(&environment.agent_name)),
        ("__DATE__", lex::quoted(&date)),
        ("__TIME__", lex::quoted(&time)),
        ("__STDC__", "1".into()),
        // Wave's default C++ mode with variadics, as configured by Firestorm.
        // https://www.boost.org/doc/libs/latest/libs/wave/doc/predefined_macros.html
        ("__cplusplus", "199711L".into()),
        ("__BASE_FILE__", lex::quoted(name)),
        (
            "__UNIXTIME__",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string(),
        ),
    ] {
        definitions.insert(name.into(), macros::object(&value));
    }
    for name in ["list", "float", "integer", "key", "rotation", "quaternion", "string", "vector"] {
        let (key, def) = macros::define(&format!("{name}(...) (({name})(__VA_ARGS__))"))?;
        definitions.insert(key, def);
    }
    let mut processor = Processor {
        definitions,
        once: HashSet::new(),
        loader,
        warnings: Vec::new(),
        operations: 0,
    };
    let mut generated = processor.file(source, name, Uuid::nil(), 0)?;
    if settings.lazy_lists || processor.definitions.contains_key("USE_LAZY_LISTS") {
        generated = lsl::lazy_lists(&generated)?;
    }
    if settings.switches || processor.definitions.contains_key("USE_SWITCHES") {
        generated = lsl::switches(&generated)?;
    }
    if settings.optimize {
        generated = lsl::optimize(&generated);
    }
    let uploaded = format!("{}\n{generated}", encode(source));
    if uploaded.len() > MAX_SOURCE {
        return Err(aurora_i18n::tr!("script-source-too-large").into());
    }
    let mono = if source.lines().any(|line| line == "//mono") {
        Some(true)
    } else if source.lines().any(|line| line == "//lsl2") {
        Some(false)
    } else {
        None
    };
    Ok(Output {
        uploaded,
        #[cfg(test)]
        generated,
        mono,
        warnings: processor.warnings,
    })
}

impl<F> Processor<'_, F>
where
    F: FnMut(&str, &str) -> Result<Include, String>,
{
    fn file(&mut self, source: &str, name: &str, asset: Uuid, depth: usize) -> Result<String, String> {
        if self.once.contains(name) {
            return Ok(String::new());
        }
        if depth > 32 || source.len() > MAX_SOURCE {
            return Err(aurora_i18n::tr!("script-preproc-include-limit").into());
        }
        let saved: Vec<_> = [
            "__FILE__",
            "__SHORTFILE__",
            "__ASSETID__",
            "__ASSETIDRAW__",
            "__LINE__",
            "__INCLUDE_LEVEL__",
        ]
        .map(|name| (name, self.definitions.get(name).cloned()))
        .into();
        self.definitions.insert("__FILE__".into(), macros::object(&lex::quoted(name)));
        self.definitions
            .insert("__INCLUDE_LEVEL__".into(), macros::object(&depth.to_string()));
        self.definitions.insert(
            "__SHORTFILE__".into(),
            macros::object(&lex::quoted(
                std::path::Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or(name),
            )),
        );
        self.definitions
            .insert("__ASSETID__".into(), macros::object(&lex::quoted(&asset.to_string())));
        self.definitions
            .insert("__ASSETIDRAW__".into(), macros::object(&asset.to_string().replace('-', "_")));
        let tokens = lex::lex(&decode(source));
        for token in &tokens {
            let incomplete = match token.kind {
                lex::Kind::Comment => token.text.starts_with("/*") && !token.text.ends_with("*/"),
                lex::Kind::String => {
                    token.text.len() < 2
                        || token.text.as_bytes().first() != token.text.as_bytes().last()
                        || token.text[..token.text.len() - 1].chars().rev().take_while(|c| *c == '\\').count() % 2 != 0
                }
                _ => false,
            };
            if incomplete {
                return Err(diagnostic(name, token.line - 1, aurora_i18n::tr!("script-preproc-directive")));
            }
        }
        let clean: String = tokens
            .into_iter()
            .map(|t| {
                if t.kind == lex::Kind::Comment {
                    t.text.chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect()
                } else {
                    t.text
                }
            })
            .collect();
        let mut lines = clean.lines().enumerate().peekable();
        let mut output = String::new();
        let mut stack: Vec<Conditional> = Vec::new();
        let mut pending = Vec::new();
        let mut logical_offset: i64 = 0;
        while let Some((line, text)) = lines.next() {
            self.operations += 1;
            if self.operations > 100_000 {
                return Err(aurora_i18n::tr!("script-preproc-expansion-limit").into());
            }
            let mut text = text.to_owned();
            while text.ends_with('\\') {
                text.pop();
                if let Some((_, next)) = lines.next() {
                    text.push_str(next);
                } else {
                    return Err(diagnostic(name, line, aurora_i18n::tr!("script-preproc-directive")));
                }
            }
            let logical_line = (line as i64 + 1 + logical_offset).max(1) as usize;
            self.definitions
                .insert("__LINE__".into(), macros::object(&logical_line.to_string()));
            let active = stack.last().is_none_or(|s| s.active);
            let result = (|| -> Result<(), String> {
                if let Some(directive) = text.trim_start().strip_prefix('#') {
                    self.flush(&mut pending, &mut output)?;
                    let directive = directive.trim_start();
                    let end = directive.find(char::is_whitespace).unwrap_or(directive.len());
                    let command = &directive[..end];
                    let argument = directive[end..].trim();
                    match command {
                        "if" | "ifdef" | "ifndef" => {
                            if command != "if" && !identifier(argument) {
                                return Err(aurora_i18n::tr!("script-preproc-directive").into());
                            }
                            let matched = active
                                && if command == "if" {
                                    macros::condition(argument, &self.definitions, logical_line)?
                                } else {
                                    self.definitions.contains_key(argument) == (command == "ifdef")
                                };
                            stack.push(Conditional {
                                parent: active,
                                active: matched,
                                taken: matched,
                                otherwise: false,
                            });
                        }
                        "elif" => {
                            let state = stack
                                .last_mut()
                                .ok_or_else(|| aurora_i18n::tr!("script-preproc-conditional").to_owned())?;
                            if state.otherwise {
                                return Err(aurora_i18n::tr!("script-preproc-conditional").into());
                            }
                            state.active = state.parent && !state.taken && macros::condition(argument, &self.definitions, logical_line)?;
                            state.taken |= state.active;
                        }
                        "else" => {
                            if !argument.is_empty() {
                                return Err(aurora_i18n::tr!("script-preproc-directive").into());
                            }
                            let state = stack
                                .last_mut()
                                .ok_or_else(|| aurora_i18n::tr!("script-preproc-conditional").to_owned())?;
                            if state.otherwise {
                                return Err(aurora_i18n::tr!("script-preproc-conditional").into());
                            }
                            state.active = state.parent && !state.taken;
                            state.taken = true;
                            state.otherwise = true;
                        }
                        "endif" => {
                            if !argument.is_empty() {
                                return Err(aurora_i18n::tr!("script-preproc-directive").into());
                            }
                            stack
                                .pop()
                                .ok_or_else(|| aurora_i18n::tr!("script-preproc-conditional").to_owned())?;
                        }
                        _ if !active => {}
                        "define" => {
                            let (name, definition) = macros::define(argument)?;
                            self.definitions.insert(name, definition);
                        }
                        "undef" => {
                            if !identifier(argument) {
                                return Err(aurora_i18n::tr!("script-preproc-directive").into());
                            }
                            self.definitions.remove(argument);
                        }
                        "include" => {
                            let mut tokens = lex::lex(argument);
                            for token in &mut tokens {
                                token.line = logical_line;
                            }
                            let expanded = lex::join(&macros::expand(&tokens, &self.definitions)?);
                            let expanded = expanded.trim();
                            let include = expanded
                                .strip_prefix('"')
                                .and_then(|s| s.strip_suffix('"'))
                                .or_else(|| expanded.strip_prefix('<').and_then(|s| s.strip_suffix('>')))
                                .filter(|s| !s.is_empty())
                                .ok_or_else(|| aurora_i18n::tr!("script-preproc-directive").to_owned())?;
                            if !self.once.contains(include) {
                                let loaded = (self.loader)(include, name)?;
                                output.push_str(&self.file(&loaded.source, include, loaded.asset, depth + 1)?);
                            }
                        }
                        "pragma" if argument == "once" => {
                            self.once.insert(name.into());
                        }
                        "pragma" => {}
                        "line" => {
                            let tokens = lex::code(&lex::join(&macros::expand(&lex::lex(argument), &self.definitions)?));
                            let next: usize = tokens
                                .first()
                                .and_then(|t| t.text.parse().ok())
                                .filter(|n| *n > 0 && *n <= i32::MAX as usize)
                                .ok_or_else(|| aurora_i18n::tr!("script-preproc-directive").to_owned())?;
                            logical_offset = next as i64 - line as i64 - 2;
                            if let Some(filename) = tokens.get(1) {
                                if filename.kind != lex::Kind::String || tokens.len() != 2 {
                                    return Err(aurora_i18n::tr!("script-preproc-directive").into());
                                }
                                self.definitions.insert("__FILE__".into(), macros::object(&filename.text));
                            }
                        }
                        "error" => return Err(argument.into()),
                        "warning" => self.warnings.push(diagnostic(name, line, argument)),
                        "" => {}
                        _ => return Err(aurora_i18n::tr!("script-preproc-directive").into()),
                    }
                } else if active {
                    let mut tokens = lex::lex(&format!("{text}\n"));
                    for token in &mut tokens {
                        token.line = logical_line;
                    }
                    pending.extend(tokens);
                }
                if output.len() > MAX_SOURCE {
                    return Err(aurora_i18n::tr!("script-preproc-expansion-limit").into());
                }
                Ok(())
            })();
            result.map_err(|e| diagnostic(name, line, &e))?;
        }
        self.flush(&mut pending, &mut output)
            .map_err(|e| diagnostic(name, clean.lines().count(), &e))?;
        if !stack.is_empty() {
            return Err(diagnostic(
                name,
                clean.lines().count(),
                aurora_i18n::tr!("script-preproc-conditional"),
            ));
        }
        for (key, value) in saved {
            if let Some(value) = value {
                self.definitions.insert(key.into(), value);
            } else {
                self.definitions.remove(key);
            }
        }
        Ok(output)
    }

    fn flush(&self, pending: &mut Vec<lex::Token>, output: &mut String) -> Result<(), String> {
        if !pending.is_empty() {
            output.push_str(&lex::join(&macros::expand(pending, &self.definitions)?));
            pending.clear();
            if output.len() > MAX_SOURCE {
                return Err(aurora_i18n::tr!("script-preproc-expansion-limit").into());
            }
        }
        Ok(())
    }
}

fn diagnostic(file: &str, line: usize, message: &str) -> String {
    format!("({line}, 0) {file}: {message}")
}

fn identifier(text: &str) -> bool {
    let tokens = lex::code(text);
    tokens.len() == 1 && tokens[0].kind == lex::Kind::Word
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoded_source_roundtrips_comments_delimiters_and_unicode() {
        for source in ["// hello\n/* */\n/|*|//\\ あé", "", "default { state_entry() {} }"] {
            assert_eq!(decode(&format!("{}\ndefault {{}}", encode(source))), source);
        }
        assert_eq!(decode("//start_unprocessed_text\n/*broken"), "//start_unprocessed_text\n/*broken");
    }
    #[test]
    fn includes_conditions_macros_and_original_source_are_preserved() {
        let settings = ScriptSettings {
            optimize: false,
            lazy_lists: false,
            switches: false,
            ..Default::default()
        };
        let source = "#define CHANNEL 0\n#include \"library.lsl\"\n#include \"library.lsl\"\n#if defined(CHANNEL) && CHANNEL == 0\ndefault { state_entry() { greet(); } }\n#else\n#error wrong branch\n#endif";
        let mut calls = 0;
        let output = process(source, "main", &settings, &Environment::default(), &mut |name, _| {
            assert_eq!(name, "library.lsl");
            calls += 1;
            Ok(Include {
                source: "#pragma once\ngreet() { llSay(CHANNEL, __SHORTFILE__); }".into(),
                asset: Uuid::nil(),
            })
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(output.generated.contains("llSay(0, \"library.lsl\")"));
        assert!(!output.generated.contains("wrong branch"));
        assert_eq!(decode(&output.uploaded), source);
    }
    #[test]
    fn malformed_conditions_and_recursive_includes_fail_without_uploading() {
        let settings = ScriptSettings::default();
        for source in ["#if 1\ndefault {}", "#else\n", "#define F(x) x\nF(1,2)", "#include \"loop\""] {
            let result = process(source, "main", &settings, &Environment::default(), &mut |_, _| {
                Ok(Include {
                    source: source.into(),
                    asset: Uuid::nil(),
                })
            });
            assert!(result.is_err(), "{source}");
        }
    }

    #[test]
    fn multiline_macros_line_directives_and_unterminated_input_are_handled() {
        let settings = ScriptSettings {
            optimize: false,
            lazy_lists: false,
            switches: false,
            ..Default::default()
        };
        let output = process(
            "#define F(x) x\n#define ALIAS F\n#define WHERE __LINE__\nALIAS\n(42)\nWHERE\n#line 100 \"virtual.lsl\"\nWHERE __FILE__",
            "main",
            &settings,
            &Environment::default(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert!(output.generated.contains("42\n6\n100 \"virtual.lsl\""), "{}", output.generated);
        let output = process(
            "\n#if __LINE__ == 2\ncorrect\n#else\n#error incorrect line\n#endif",
            "main",
            &settings,
            &Environment::default(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert_eq!(output.generated, "\ncorrect\n");
        for source in ["/* missing end", "\"missing end", "\"escaped end\\\""] {
            assert!(process(source, "main", &settings, &Environment::default(), &mut |_, _| unreachable!()).is_err());
        }
    }
}
