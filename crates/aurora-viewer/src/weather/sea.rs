//! The Sea panel's settings (Aurora Weather window, `ui/environment.rs`):
//! which wind raises the waves of enhanced water, or a sea state set by
//! hand, and how the sea is shaped (its swell, its chop, the fetch, the
//! sharpness of its crests, its foam, its style), with built-in and saved
//! presets. Aurora's own: the Legacy Viewer's water has no waves to set.
//! The renderer's side is `aurora_render::water_sea::SeaTune`.

use aurora_render::water_fft::Spectrum;
use aurora_render::water_sea::SeaTune;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Where the wind that raises the waves comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SeaSource {
    /// The weather's wind while Aurora Weather blows one, the region's
    /// otherwise (how the sea always followed the wind).
    #[default]
    Auto,
    /// The region's wind, whatever the weather.
    Region,
    /// Aurora Weather's wind (calm while it blows none).
    Weather,
    /// Set here: `SeaSettings::state` from `SeaSettings::from`.
    Mine,
}

/// How the wind chop is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SeaStyle {
    /// A sea still growing over its fetch (JONSWAP).
    #[default]
    Natural,
    /// A fully developed open ocean (Phillips).
    Ocean,
    /// A few clean crossing wave trains, as before the spectrum.
    Stylized,
}

/// The shape of a sea: what a preset holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SeaShape {
    /// The sea state set here, 0 (glassy) to 1 (a storm): the wind speed
    /// it stands for is `state × MAX_WIND`.
    pub state: f32,
    /// The long swell's height and the wind chop's (1: as the wind makes
    /// them).
    pub swell: f32,
    pub chop: f32,
    /// How far the wind blows over open water (km).
    pub fetch: f32,
    /// How sharp the crests are (1: the wind's own).
    pub choppiness: f32,
    /// How much foam the whitecaps make (1: the wind's own).
    pub foam: f32,
    /// How much water the waves throw into the air (1: as a real sea
    /// does; more, and from calmer seas, the higher).
    pub spray: f32,
    pub style: SeaStyle,
}

impl Default for SeaShape {
    fn default() -> Self {
        Self {
            state: 0.3,
            swell: 1.0,
            chop: 1.0,
            fetch: 15.0,
            choppiness: 1.0,
            foam: 1.0,
            spray: 1.0,
            style: SeaStyle::Natural,
        }
    }
}

/// A preset saved under a name.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SeaPreset {
    pub name: String,
    pub shape: SeaShape,
}

/// The wind speed of a full sea state (m/s): a storm, Beaufort 9.
pub const MAX_WIND: f32 = 24.0;
/// The sliders' ranges; with "Extreme waves" on, the swell, the chop and
/// the foam go to `EXTREME_SCALE` times, the sea state to
/// `EXTREME_STATE` (about 40 m/s, a hurricane's) and the height limit to
/// `EXTREME_LIMIT` m.
pub const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=2.0;
pub const EXTREME_SCALE: f32 = 6.0;
pub const EXTREME_STATE: f32 = 1.7;
pub const EXTREME_LIMIT: f32 = 40.0;
pub const FETCH_RANGE: std::ops::RangeInclusive<f32> = 0.5..=50.0;
pub const SHARP_RANGE: std::ops::RangeInclusive<f32> = 0.2..=1.6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SeaSettings {
    pub source: SeaSource,
    /// Where the wind set here comes from: degrees clockwise from north,
    /// as on a compass (270: a west wind).
    pub from: f32,
    pub shape: SeaShape,
    /// The presets saved by the user.
    pub saved: Vec<SeaPreset>,
    /// The highest the open sea's waves stand, trough to crest (m; the top
    /// of its slider, `limit_top`: no limit).
    pub max_height: f32,
    /// "Extreme waves": the sliders go far past nature's (a safety catch
    /// for the seas no one would sail).
    pub extreme: bool,
}

impl Default for SeaSettings {
    fn default() -> Self {
        Self {
            source: SeaSource::Auto,
            from: 270.0,
            shape: SeaShape::default(),
            saved: Vec::new(),
            max_height: NO_LIMIT,
            extreme: false,
        }
    }
}

/// The built-in presets (their names are the i18n keys'): calm lake,
/// breeze, open swell, storm and stylised.
pub const PRESETS: [&str; 5] = ["calm", "breeze", "swell", "storm", "stylized"];

/// The shape of built-in preset `key` (`PRESETS`).
pub fn preset(key: &str) -> Option<SeaShape> {
    let shape = |state, swell, chop, fetch, choppiness, foam, spray, style| SeaShape {
        state,
        swell,
        chop,
        fetch,
        choppiness,
        foam,
        spray,
        style,
    };
    Some(match key {
        // a mirror with a ripple: no swell, a short fetch
        "calm" => shape(0.06, 0.0, 0.7, 1.0, 0.6, 0.4, 1.0, SeaStyle::Natural),
        "breeze" => shape(0.25, 0.7, 1.0, 15.0, 1.0, 1.0, 1.0, SeaStyle::Natural),
        // long rollers from far away under a light wind
        "swell" => shape(0.3, 1.8, 0.4, 50.0, 0.8, 0.6, 1.0, SeaStyle::Ocean),
        "storm" => shape(0.9, 1.3, 1.3, 50.0, 1.3, 1.5, 1.3, SeaStyle::Natural),
        // (a lively sea, throwing water at a moderate wind)
        "stylized" => shape(0.35, 1.0, 1.0, 15.0, 1.0, 0.8, 1.8, SeaStyle::Stylized),
        _ => return None,
    })
}

impl SeaSettings {
    /// The top of the sea state's slider, the scales' and the height
    /// limit's (further with "Extreme waves").
    pub fn state_top(&self) -> f32 {
        if self.extreme { EXTREME_STATE } else { 1.0 }
    }
    pub fn scale_top(&self) -> f32 {
        if self.extreme { EXTREME_SCALE } else { *SCALE_RANGE.end() }
    }
    pub fn limit_top(&self) -> f32 {
        if self.extreme { EXTREME_LIMIT } else { NO_LIMIT }
    }

    /// The wind blowing the sea set here (m/s, toward), for `Mine`.
    pub fn own_wind(&self) -> Vec2 {
        let from = self.from.to_radians();
        -Vec2::new(from.sin(), from.cos()) * self.shape.state.clamp(0.0, self.state_top()) * MAX_WIND
    }

    /// The wind over the water, from the region's (`region`) and the
    /// weather's (`weather`, none while it blows none).
    pub fn wind(&self, region: Vec2, weather: Option<Vec2>) -> Vec2 {
        match self.source {
            SeaSource::Auto => weather.unwrap_or(region),
            SeaSource::Region => region,
            SeaSource::Weather => weather.unwrap_or(Vec2::ZERO),
            SeaSource::Mine => self.own_wind(),
        }
    }

    /// The renderer's shaping, under the height limit (none at the top of
    /// its slider).
    pub fn tune(&self) -> SeaTune {
        let s = &self.shape;
        let top = self.scale_top();
        let max_height = self.max_height;
        SeaTune {
            swell: s.swell.clamp(0.0, top),
            chop: s.chop.clamp(0.0, top),
            fetch: s.fetch.clamp(*FETCH_RANGE.start(), *FETCH_RANGE.end()) * 1000.0,
            choppiness: s.choppiness.clamp(*SHARP_RANGE.start(), *SHARP_RANGE.end()),
            foam: s.foam.clamp(0.0, top),
            spray: s.spray.clamp(0.0, top),
            spectrum: match s.style {
                SeaStyle::Natural => Some(Spectrum::Jonswap),
                SeaStyle::Ocean => Some(Spectrum::Phillips),
                SeaStyle::Stylized => None,
            },
            max_height: if max_height.is_finite() && max_height < self.limit_top() {
                max_height.max(0.1)
            } else {
                f32::INFINITY
            },
        }
    }

    /// The built-in preset or saved one the shape now is, if any (its key
    /// or its name, and whether it is built in).
    pub fn current_preset(&self) -> Option<(String, bool)> {
        PRESETS
            .iter()
            .find(|k| preset(k).is_some_and(|p| same(&p, &self.shape)))
            .map(|k| (k.to_string(), true))
            .or_else(|| {
                self.saved
                    .iter()
                    .find(|p| same(&p.shape, &self.shape))
                    .map(|p| (p.name.clone(), false))
            })
    }

    /// Save the shape now under `name` (replacing one of the same name).
    pub fn save_preset(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let shape = self.shape.clone();
        match self.saved.iter_mut().find(|p| p.name == name) {
            Some(p) => p.shape = shape,
            None => self.saved.push(SeaPreset {
                name: name.to_string(),
                shape,
            }),
        }
        true
    }
}

/// The height limit's slider top (m): there, no limit.
pub const NO_LIMIT: f32 = 10.0;

/// Two shapes the sliders cannot tell apart.
fn same(a: &SeaShape, b: &SeaShape) -> bool {
    let close = |x: f32, y: f32| (x - y).abs() < 0.005;
    a.style == b.style
        && close(a.state, b.state)
        && close(a.swell, b.swell)
        && close(a.chop, b.chop)
        && (a.fetch - b.fetch).abs() < 0.05
        && close(a.choppiness, b.choppiness)
        && close(a.foam, b.foam)
        && close(a.spray, b.spray)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_waves_follow_the_wind_chosen() {
        let mut s = SeaSettings::default();
        let region = Vec2::new(3.0, 0.0);
        let gust = Vec2::new(0.0, 9.0);
        let weather = Some(gust);
        assert_eq!(s.wind(region, weather), gust);
        assert_eq!(s.wind(region, None), region);
        s.source = SeaSource::Region;
        assert_eq!(s.wind(region, weather), region);
        s.source = SeaSource::Weather;
        assert_eq!(s.wind(region, None), Vec2::ZERO);
        // set here: a west wind blows east (+x), at the state's speed
        s.source = SeaSource::Mine;
        s.from = 270.0;
        s.shape.state = 0.5;
        let w = s.wind(region, weather);
        assert!((w - Vec2::new(MAX_WIND * 0.5, 0.0)).length() < 1e-3, "{w}");
    }

    #[test]
    fn presets_are_recognised_and_saved_ones_replace_their_namesake() {
        let mut s = SeaSettings::default();
        for key in PRESETS {
            s.shape = preset(key).expect("built in");
            assert_eq!(s.current_preset(), Some((key.to_string(), true)));
        }
        s.shape.foam = 1.9;
        assert_eq!(s.current_preset(), None);
        assert!(s.save_preset(" Mine "));
        assert!(!s.save_preset("  "));
        assert_eq!(s.current_preset(), Some(("Mine".to_string(), false)));
        s.shape.chop = 0.1;
        assert!(s.save_preset("Mine"));
        assert_eq!(s.saved.len(), 1);
        assert_eq!(s.saved[0].shape.chop, 0.1);
    }

    #[test]
    fn the_tune_follows_the_shape_and_the_limit() {
        let mut s = SeaSettings::default();
        assert_eq!(s.tune().max_height, f32::INFINITY);
        s.max_height = 3.0;
        assert_eq!(s.tune().max_height, 3.0);
        s.shape = preset("stylized").expect("built in");
        assert_eq!(s.tune().spectrum, None);
        s.shape.fetch = 1000.0;
        assert_eq!(s.tune().fetch, 50_000.0);
        // the safety catch: past nature's only once it is on
        s.shape.swell = 5.0;
        s.max_height = 25.0;
        assert_eq!(s.tune().swell, 2.0);
        assert_eq!(s.tune().max_height, f32::INFINITY, "the top of the normal slider: no limit");
        s.extreme = true;
        assert_eq!(s.tune().swell, 5.0);
        assert_eq!(s.tune().max_height, 25.0);
    }
}
