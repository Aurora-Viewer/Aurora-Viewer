//! Aurora Weather's debris: what the Weather window's "Debris" card sets,
//! made into the renderer's [`aurora_render::DebrisLook`] (drawn by
//! aurora-render `debris.rs`). The user picks what blows about (leaves,
//! sand, paper, embers; tumbleweeds on their own switch): the viewer cannot
//! know whether the place is a forest, a street or a desert. Drawn for this
//! viewer only; off by default.

use super::WeatherSettings;
use serde::{Deserialize, Serialize};

/// What the wind blows about (the card's choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DebrisChoice {
    #[default]
    Leaves,
    Sand,
    Paper,
    Embers,
    /// Dust motes hanging in the air, seen in sunbeams.
    Motes,
}

impl DebrisChoice {
    pub const ALL: [DebrisChoice; 5] = [
        DebrisChoice::Leaves,
        DebrisChoice::Sand,
        DebrisChoice::Paper,
        DebrisChoice::Embers,
        DebrisChoice::Motes,
    ];

    fn render(self) -> aurora_render::DebrisKind {
        match self {
            DebrisChoice::Leaves => aurora_render::DebrisKind::Leaves,
            DebrisChoice::Sand => aurora_render::DebrisKind::Sand,
            DebrisChoice::Paper => aurora_render::DebrisKind::Paper,
            DebrisChoice::Embers => aurora_render::DebrisKind::Embers,
            DebrisChoice::Motes => aurora_render::DebrisKind::Motes,
        }
    }

    /// Its name in `AURORA_DEBRIS`.
    fn parse(name: &str) -> Option<DebrisChoice> {
        match name.trim().to_ascii_lowercase().as_str() {
            "leaves" => Some(DebrisChoice::Leaves),
            "sand" => Some(DebrisChoice::Sand),
            "paper" => Some(DebrisChoice::Paper),
            "embers" => Some(DebrisChoice::Embers),
            "motes" => Some(DebrisChoice::Motes),
            _ => None,
        }
    }
}

/// The density slider's default (0..1).
pub const DEBRIS_DENSITY_DEFAULT: f32 = 0.5;

/// `AURORA_DEBRIS=<leaves|sand|paper|embers|motes>[,stream]`: that debris on
/// until the Weather window is used (demo captures); `,stream` lets leaves
/// come on the wind where no foliage is near.
pub(crate) fn env_debris() -> Option<(DebrisChoice, bool)> {
    static DEBRIS: std::sync::OnceLock<Option<(DebrisChoice, bool)>> = std::sync::OnceLock::new();
    *DEBRIS.get_or_init(|| {
        let v = std::env::var("AURORA_DEBRIS").ok()?;
        let mut parts = v.split(',');
        let kind = DebrisChoice::parse(parts.next()?)?;
        Some((kind, parts.any(|p| p.trim().eq_ignore_ascii_case("stream"))))
    })
}

impl WeatherSettings {
    /// How much sand blows (0..1): the debris' sand on, its density; it
    /// gathers in drifts and snakes along the ground (none while it rains:
    /// wet sand does not blow).
    pub fn sand_blowing(&self) -> f32 {
        if !(self.enabled && self.debris_on && self.debris_kind == DebrisChoice::Sand) || self.rain() > 0.0 {
            return 0.0;
        }
        self.debris_density.clamp(0.0, 1.0)
    }

    /// The debris drawn (of `in_effect`'s settings), None when its switch
    /// or the main one is off: `quality` the weather's (0..2).
    pub fn debris_look(&self, quality: u32) -> Option<aurora_render::DebrisLook> {
        if !(self.enabled && self.debris_on) {
            return None;
        }
        Some(aurora_render::DebrisLook {
            kind: self.debris_kind.render(),
            density: self.debris_density.clamp(0.0, 1.0),
            quality: quality.min(2),
            stream: self.debris_stream,
            size: 1.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debris_is_off_by_default_and_follows_its_switches() {
        let mut w = WeatherSettings::default();
        assert!(!w.debris_on);
        assert_eq!(w.debris_look(2), None);
        w.debris_on = true;
        // the main switch off: none
        assert_eq!(w.debris_look(2), None);
        w.enabled = true;
        let look = w.debris_look(1).expect("on");
        assert_eq!(look.kind, aurora_render::DebrisKind::Leaves);
        assert_eq!(look.quality, 1);
        assert!(!look.stream);
    }

    #[test]
    fn debris_names_parse() {
        assert_eq!(DebrisChoice::parse(" Sand"), Some(DebrisChoice::Sand));
        assert_eq!(DebrisChoice::parse("snow"), None);
    }
}
