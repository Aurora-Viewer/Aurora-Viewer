//! `/tr` in a private conversation: its partner's language.

use aurora_translate::{Lang, parse_lang};

#[derive(Debug, PartialEq, Eq)]
pub enum TrCommand {
    /// `/tr es`, `/tr Spanish`, `/tr español`.
    Set(&'static Lang),
    /// `/tr off`.
    Off,
    /// `/tr` alone: what the conversation is translated to.
    Status,
    /// A language we do not know.
    Unknown(String),
}

/// The command in a line typed in a private conversation, if it is one.
pub fn parse_tr(text: &str) -> Option<TrCommand> {
    let text = text.trim();
    let head = text.get(..3)?;
    if !head.eq_ignore_ascii_case("/tr") {
        return None;
    }
    let rest = &text[3..];
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    let arg = rest.trim();
    Some(if arg.is_empty() {
        TrCommand::Status
    } else if arg.eq_ignore_ascii_case("off") {
        TrCommand::Off
    } else {
        parse_lang(arg).map_or_else(|| TrCommand::Unknown(arg.to_owned()), TrCommand::Set)
    })
}
