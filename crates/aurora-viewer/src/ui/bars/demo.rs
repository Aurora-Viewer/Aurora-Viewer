//! `AURORA_DEMO_BARS=<words>`: the states of the bars for the side-by-side
//! comparisons with the mockup, without touching the saved settings. Words,
//! separated by commas: `legacy` (Firestorm's bars), `compact` (the place
//! island as wide as its content), `rights` (the rights named), and one
//! menu opened at frame 300: `menu` (the place), `options` (the place
//! island's right click), `favorites`, `star`, `status`, `emoji` (the chat
//! island's emoji picker); `nobug` / `bug`: the bug report button hidden /
//! shown; `voice`: talking on the Speak button, its level at 62 %;
//! `chatwide`: the chat island 560 px wide; `read`: no unread conversation
//! counted. `normal` or nothing else: the
//! islands as set. With any word the voice light is the mockup's
//! (connected).

use std::sync::OnceLock;

/// A menu of the islands the demo opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Open {
    Place,
    PlaceOptions,
    Favorites,
    Star,
    Status,
    Emoji,
}

/// What `AURORA_DEMO_BARS` asks for (nothing without it).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DemoBars {
    pub on: bool,
    pub classic: Option<bool>,
    pub stretch: Option<bool>,
    pub rights_text: Option<bool>,
    pub bug: Option<bool>,
    pub open: Option<Open>,
    /// Talking, the level at 62 % (the mockup's voice state).
    pub voice: bool,
    /// The chat island's width for this run.
    pub chat_width: Option<f32>,
    /// No unread conversation shown (the demo's IMs count otherwise).
    pub read: bool,
}

pub fn parse(value: &str) -> DemoBars {
    let mut d = DemoBars {
        on: true,
        ..DemoBars::default()
    };
    for word in value.split(',').map(str::trim) {
        match word {
            "legacy" => d.classic = Some(true),
            "islands" => d.classic = Some(false),
            "compact" => d.stretch = Some(false),
            "stretch" => d.stretch = Some(true),
            "rights" => d.rights_text = Some(true),
            "icons" => d.rights_text = Some(false),
            "bug" => d.bug = Some(true),
            "nobug" => d.bug = Some(false),
            "menu" => d.open = Some(Open::Place),
            "options" => d.open = Some(Open::PlaceOptions),
            "favorites" => d.open = Some(Open::Favorites),
            "star" => d.open = Some(Open::Star),
            "status" => d.open = Some(Open::Status),
            "emoji" => d.open = Some(Open::Emoji),
            "voice" => d.voice = true,
            "chatwide" => d.chat_width = Some(560.0),
            "read" => d.read = true,
            _ => {}
        }
    }
    // a menu or a look of the islands: the islands, whatever is saved
    let islands_only = d.open.is_some() || d.stretch.is_some() || d.rights_text.is_some() || d.bug.is_some() || d.voice || d.read;
    if d.classic.is_none() && (islands_only || d.chat_width.is_some()) {
        d.classic = Some(false);
    }
    d
}

/// The demo's request, read once.
pub fn get() -> DemoBars {
    static DEMO: OnceLock<DemoBars> = OnceLock::new();
    *DEMO.get_or_init(|| std::env::var("AURORA_DEMO_BARS").map(|v| parse(&v)).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_combine() {
        let d = parse("compact,rights,menu");
        assert_eq!(d.classic, Some(false));
        assert_eq!(d.stretch, Some(false));
        assert_eq!(d.rights_text, Some(true));
        assert_eq!(d.open, Some(Open::Place));
        assert_eq!(parse("legacy").classic, Some(true));
        assert_eq!(
            parse("normal"),
            DemoBars {
                on: true,
                ..DemoBars::default()
            }
        );
    }
}
