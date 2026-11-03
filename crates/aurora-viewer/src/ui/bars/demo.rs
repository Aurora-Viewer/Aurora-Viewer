//! `AURORA_DEMO_BARS=<words>`: the states of the bars for the side-by-side
//! comparisons with the mockup, without touching the saved settings. Words,
//! separated by commas: `legacy` (Firestorm's bars), `compact` (the place
//! island as wide as its content), `rights` (the rights named), and one
//! menu opened at frame 300: `menu` (the place), `options` (the place
//! island's right click), `favorites`, `star`, `status`, `emoji` (the chat
//! island's emoji picker); `nobug` / `bug`: the bug report button hidden /
//! shown; `voice`: talking on the Speak button, its level at 62 %;
//! `chatwide`: the chat island 560 px wide; `read`: no unread conversation
//! counted. The favourites: `fav12` (the mockup's twelve), `nofav` (none),
//! `added` (the twelve, the last one just added from the star), `favsnap`
//! (the twelve without item thumbnails: their parcels' snapshots), and where
//! they live: `row` (the row under the islands, `folded`: folded to its
//! tab, `rowdrag`: a pill dragged at frame 650), `dock` (the dock's
//! button; `favorites` then opens its grid); `chatname`: a resident's name
//! of the chat pills clicked at frame 700. The balance: `nobalance` (hidden,
//! with either bars), `balancemenu` (the account island's right click).
//! The system island: `systip`
//! (its detailed tooltip shown), `sysmenu` (its right click),
//! `profiles` (two saved quality fixtures), `fpspresets` (the fixtures and
//! the FPS hover card, with either bar layout),
//! `netgraph` (Firestorm's traffic indicator shown), `tplist` (earlier
//! places in the session, the back arrow's list of them open), `sysperf` /
//! `sysnet` (Performance opened from the island at frame 300, by its frame
//! rate / its network, under the island); the clock: `clocklocal` (local
//! time, whatever is saved: a click then does not change it), `noseconds`
//! (without its seconds), `clock12` / `clock24` (12 / 24 hours). The hidden interface: `hidden`
//! (hidden at frame 600, its hint shown), `peek` (hidden, the pointer held at
//! the top edge: the top islands back), `peekleft` (hidden, the pointer held
//! at the left edge: the left toolbar back); with `legacy`, Firestorm's
//! question; `immersive` (Conversations opened at frame 300, immersive mode
//! at frame 600: the chat kept, the name tags hidden, the buttons in the
//! corner), `immersivetags` (the same, the name tags shown, not saved),
//! `immersivehuds` (the same, the HUDs hidden, not saved). The chat bar: `autohide` (shown only while it is used, with
//! either bars), `chatstart` (autohidden, Enter pressed at frame 300 on a
//! draft: the bar back). The location field's suggestions: `locations`
//! (the place menu opened at frame 300, "Aur" typed in its destination
//! field: the landmarks and places it names; with `legacy`, typed in the
//! location field).
//! `normal` or nothing
//! else: the
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
    /// The account island's right click (the balance shown or hidden).
    Account,
    /// The system island's right click.
    System,
    /// The back arrow's list of the session's places, the demo's earlier
    /// places before the arrival.
    TpList,
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
    /// The favourites of the demo, and where they live.
    pub favorites: Option<Favorites>,
    pub access: Option<crate::settings::FavoritesAccess>,
    pub folded: Option<bool>,
    pub row_drag: bool,
    /// A resident's name of the chat pills clicked at frame 700.
    pub click_name: bool,
    /// The system island's detailed tooltip shown.
    pub tip: bool,
    /// Named quality fixtures; optionally show the FPS hover card.
    pub graphics_profiles: bool,
    pub fps_presets: bool,
    /// The traffic indicator shown or hidden, whatever is saved.
    pub net_stats: Option<bool>,
    /// The balance shown or hidden, whatever is saved.
    pub show_balance: Option<bool>,
    /// The clock's time and seconds, whatever is saved.
    pub clock_zone: Option<crate::settings::ClockZone>,
    pub clock_seconds: Option<bool>,
    pub clock_format: Option<crate::settings::ClockFormat>,
    /// Performance opened from the system island.
    pub perf: Option<Perf>,
    /// The interface hidden at frame 600; true: the pointer at the top edge.
    pub hidden: Option<bool>,
    /// Hidden, the pointer held at the left edge: the left toolbar back.
    pub peek_left: bool,
    /// Conversations opened at frame 300, immersive mode at frame 600.
    pub immersive: bool,
    /// Immersive mode with the name tags shown.
    pub immersive_tags: bool,
    /// Immersive mode with the HUDs hidden.
    pub immersive_no_huds: bool,
    /// The chat bar autohidden (#1009), whatever is saved.
    pub autohide: bool,
    /// Autohidden, Enter pressed at frame 300 on a draft: the bar back.
    pub chat_start: bool,
    /// "Aur" typed in the place menu's destination field (the location
    /// field with `legacy`) at frame 300: its suggestions shown.
    pub locations: bool,
}

/// How the demo opens Performance from the system island.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perf {
    /// Its frame rate.
    Fps,
    /// Its network (Performance on its Network section).
    Network,
}

/// The favourites the demo's Favorites folder holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Favorites {
    /// The mockup's twelve.
    Twelve,
    /// None.
    Empty,
    /// The twelve, the last one just added from the star.
    Added,
    /// The twelve without item thumbnails: their parcels' snapshots.
    Snapshots,
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
            "fav12" => d.favorites = Some(Favorites::Twelve),
            "nofav" => d.favorites = Some(Favorites::Empty),
            "added" => d.favorites = Some(Favorites::Added),
            "favsnap" => d.favorites = Some(Favorites::Snapshots),
            "row" => d.access = Some(crate::settings::FavoritesAccess::Row),
            "dock" => d.access = Some(crate::settings::FavoritesAccess::Dock),
            "folded" => {
                d.access = Some(crate::settings::FavoritesAccess::Row);
                d.folded = Some(true);
            }
            "chatname" => d.click_name = true,
            "systip" => d.tip = true,
            "profiles" => d.graphics_profiles = true,
            "fpspresets" => {
                d.graphics_profiles = true;
                d.fps_presets = true;
            }
            "sysmenu" => d.open = Some(Open::System),
            "tplist" => d.open = Some(Open::TpList),
            "locations" => {
                d.open = Some(Open::Place);
                d.locations = true;
            }
            "netgraph" => d.net_stats = Some(true),
            "nobalance" => d.show_balance = Some(false),
            "balancemenu" => d.open = Some(Open::Account),
            "clocklocal" => d.clock_zone = Some(crate::settings::ClockZone::Local),
            "clock12" => d.clock_format = Some(crate::settings::ClockFormat::H12),
            "clock24" => d.clock_format = Some(crate::settings::ClockFormat::H24),
            "noseconds" => d.clock_seconds = Some(false),
            "sysperf" => d.perf = Some(Perf::Fps),
            "sysnet" => d.perf = Some(Perf::Network),
            "hidden" => d.hidden = Some(false),
            "peek" => d.hidden = Some(true),
            "peekleft" => {
                d.hidden = Some(false);
                d.peek_left = true;
            }
            "immersive" => d.immersive = true,
            "immersivetags" => {
                d.immersive = true;
                d.immersive_tags = true;
            }
            "immersivehuds" => {
                d.immersive = true;
                d.immersive_no_huds = true;
            }
            "rowdrag" => {
                d.access = Some(crate::settings::FavoritesAccess::Row);
                d.row_drag = true;
            }
            "autohide" => d.autohide = true,
            "chatstart" => {
                d.autohide = true;
                d.chat_start = true;
            }
            _ => {}
        }
    }
    // a menu or a look of the islands: the islands, whatever is saved
    let islands_only = d.open.is_some()
        || d.stretch.is_some()
        || d.rights_text.is_some()
        || d.bug.is_some()
        || d.voice
        || d.read
        || d.favorites.is_some()
        || d.access.is_some()
        || d.click_name
        || d.tip
        || d.net_stats.is_some()
        || d.perf.is_some()
        || d.hidden.is_some();
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
        let typed = parse("locations");
        assert!(typed.locations && typed.open == Some(Open::Place));
        let clock = parse("clocklocal,noseconds");
        assert_eq!(clock.clock_zone, Some(crate::settings::ClockZone::Local));
        assert_eq!(clock.clock_seconds, Some(false));
        assert_eq!(parse("clock12").clock_format, Some(crate::settings::ClockFormat::H12));
        let peek = parse("peekleft");
        assert!(peek.peek_left && peek.hidden == Some(false) && peek.classic == Some(false));
        assert_eq!(
            parse("normal"),
            DemoBars {
                on: true,
                ..DemoBars::default()
            }
        );
    }
}
