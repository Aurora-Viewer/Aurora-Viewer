//! The region console of estate managers: a command line POSTed to the
//! SimConsoleAsync capability, its output coming back on the event queue as
//! `SimConsoleResponse`.
//!
//! Port of LLFloaterRegionDebugConsole (indra/newview/
//! llfloaterregiondebugconsole.cpp, originally LGPL 2.1). Its fallback to
//! the older synchronous SimConsole capability is left out: the viewer never
//! requests that capability (buildCapabilityNames), so it is never there.

use aurora_llsd::Llsd;

/// The POST body: the command line as a bare LLSD string
/// (`LLSD(input->getText())`).
pub fn command_body(command: &str) -> Llsd {
    Llsd::String(command.to_owned())
}

/// The text of a `SimConsoleResponse` event (ConsoleResponseNode::post:
/// `input["body"].asString()`), however the simulator wraps it.
pub fn response_text(body: &Llsd) -> String {
    match body {
        Llsd::Map(m) => m.get("body").map(Llsd::to_string_value).unwrap_or_default(),
        other => other.to_string_value(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    #[test]
    fn the_command_is_a_bare_string() {
        let decoded = aurora_llsd::from_xml(&aurora_llsd::to_xml(&command_body("help"))).unwrap();
        assert_eq!(decoded, Llsd::String("help".into()));
    }

    #[test]
    fn the_response_text_however_it_is_wrapped() {
        assert_eq!(response_text(&Llsd::String("ok\nline 2".into())), "ok\nline 2");
        assert_eq!(response_text(&llsd_map! { "body" => "wrapped" }), "wrapped");
        assert_eq!(response_text(&Llsd::Undef), "");
    }
}
