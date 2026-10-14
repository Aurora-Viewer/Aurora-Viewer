//! `AURORA_DEMO_BARS=<words>`: the states of the bars for the side-by-side
//! comparisons with the mockup, without touching the saved settings. Words,
//! separated by commas: `legacy` (Firestorm's bars), `compact` (the place
//! island as wide as its content), `rights` (the rights named), and one
//! menu opened at frame 300: `menu` (the place), `options` (the place
//! island's right click), `favorites`, `star`, `status`. `normal` or
//! nothing else: the islands as set.

use std::sync::OnceLock;

/// A menu of the islands the demo opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Open {
    Place,
    PlaceOptions,
    Favorites,
    Star,
    Status,
}

/// What `AURORA_DEMO_BARS` asks for (nothing without it).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DemoBars {
    pub on: bool,
    pub classic: Option<bool>,
    pub stretch: Option<bool>,
    pub rights_text: Option<bool>,
    pub open: Option<Open>,
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
            "menu" => d.open = Some(Open::Place),
            "options" => d.open = Some(Open::PlaceOptions),
            "favorites" => d.open = Some(Open::Favorites),
            "star" => d.open = Some(Open::Star),
            "status" => d.open = Some(Open::Status),
            _ => {}
        }
    }
    // a menu or a look of the islands: the islands, whatever is saved
    if d.classic.is_none() && (d.open.is_some() || d.stretch.is_some() || d.rights_text.is_some()) {
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
