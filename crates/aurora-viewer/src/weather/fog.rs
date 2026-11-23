//! Aurora Weather's fog: what the Weather window's "Fog" card sets, made
//! into the renderer's [`aurora_render::FogLook`] (drawn by aurora-render
//! `fog.rs`). Drawn for your view only, from data every viewer has: the
//! region's sky haze, its wind and the grid position. Nothing is sent and
//! nothing Aurora-only is read from the world (AGENTS.md, "No data of our
//! own in shared places").

use super::WeatherSettings;
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// How the fog is drawn (the card's choice), cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FogTier {
    /// Height fog on each surface: no volume, nearly free.
    Height,
    /// A volume lit by the sun and the sky, with the shadows: god rays.
    Volumetric,
    /// The volume lit by the local lights too: neon through the fog.
    #[default]
    VolumetricLights,
}

impl FogTier {
    pub const ALL: [FogTier; 3] = [FogTier::Height, FogTier::Volumetric, FogTier::VolumetricLights];

    fn render(self) -> aurora_render::FogTier {
        match self {
            FogTier::Height => aurora_render::FogTier::Height,
            FogTier::Volumetric => aurora_render::FogTier::Volumetric,
            FogTier::VolumetricLights => aurora_render::FogTier::VolumetricLights,
        }
    }
}

/// The density slider's default (0..1).
pub const FOG_DENSITY_DEFAULT: f32 = 0.4;
/// The height slider (m): how high the fog reaches before it thins to a third.
pub const FOG_HEIGHT: std::ops::RangeInclusive<f32> = 1.0..=60.0;
pub const FOG_HEIGHT_DEFAULT: f32 = 6.0;
/// How much of the wind's speed carries the fog (0..1).
pub const FOG_DRIFT_DEFAULT: f32 = 0.35;
/// The glow around the lights (Henyey-Greenstein g, 0..0.9).
pub const FOG_ANISOTROPY: std::ops::RangeInclusive<f32> = 0.0..=0.9;
pub const FOG_ANISOTROPY_DEFAULT: f32 = 0.55;
/// Its own colour when it does not follow the sky (sRGB): a cool grey.
pub const FOG_COLOUR_DEFAULT: [f32; 3] = [0.82, 0.85, 0.9];

/// How far the wind has carried the fog: the sum of the wind over time
/// (a gust never makes the wisps jump).
#[derive(Debug, Clone, Default)]
pub struct FogDrift {
    last: Option<Instant>,
    /// Metres, world x and y.
    pub at: Vec2,
}

impl FogDrift {
    /// Move on to `at`, carried by `wind` (m/s) times the card's drift.
    pub fn advance(&mut self, at: Instant, wind: Vec2, drift: f32) {
        let dt = self.last.map_or(0.0, |t| at.saturating_duration_since(t).as_secs_f32().min(0.25));
        self.last = Some(at);
        let v = if wind.is_finite() { wind } else { Vec2::ZERO };
        self.at += v * drift.clamp(0.0, 1.0) * dt;
        // the noise repeats every few hundred metres: kept small for its
        // precision over a long session (fog.wgsl `FOG_NOISE_PERIOD` is a
        // divisor of it)
        self.at = Vec2::new(self.at.x.rem_euclid(DRIFT_WRAP), self.at.y.rem_euclid(DRIFT_WRAP));
    }
}

/// The drift wraps here (m): a multiple of the noise's period
/// (aurora-render fog.rs `NOISE_PERIOD`).
const DRIFT_WRAP: f32 = 3072.0;

/// Seconds of the UTC day now: the noise's slow change, shared by every
/// viewer whose clock is right.
pub fn utc_clock() -> f32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| (d.as_secs() % 86_400) as f32 + d.subsec_millis() as f32 * 1e-3)
}

impl WeatherSettings {
    /// The fog drawn (of `in_effect`'s settings: `AURORA_FOG` turns it on
    /// there), None when its switch or the main one is off:
    /// `drift` how far the wind carried it, `clock` the UTC day's seconds,
    /// `region_origin` the camera's region corner on the grid, `quality`
    /// the weather's (0..2).
    pub fn fog_look(&self, drift: Vec2, clock: f32, region_origin: [f64; 2], quality: u32) -> Option<aurora_render::FogLook> {
        if !(self.enabled && self.fog_on) {
            return None;
        }
        let env = (!self.touched).then(env_fog).flatten();
        let tier = env_fog_tier().filter(|_| !self.touched).unwrap_or(self.fog_tier);
        let density = env.unwrap_or(self.fog_density).clamp(0.0, 1.0);
        Some(aurora_render::FogLook {
            tier: tier.render(),
            quality: quality.min(2),
            density,
            from_sky: self.fog_from_sky,
            height: self.fog_height.clamp(*FOG_HEIGHT.start(), *FOG_HEIGHT.end()),
            color: match env_fog_colour().filter(|_| !self.touched) {
                Some(c) => c.map(|c| c.map(srgb_to_linear)),
                None => (!self.fog_follow_sky).then(|| self.fog_colour.map(srgb_to_linear)),
            },
            anisotropy: self.fog_anisotropy.clamp(*FOG_ANISOTROPY.start(), *FOG_ANISOTROPY.end()),
            drift: drift.to_array(),
            clock,
            region_origin,
        })
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// `AURORA_FOG=<0..1>`: fog of this density until the Weather window is
/// used (demo captures, measurements); a demo fog scenario's own without it,
/// none with `AURORA_FOG=off` (the scenario's scene without fog, to compare).
pub(super) fn env_fog() -> Option<f32> {
    static FOG: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *FOG.get_or_init(|| match std::env::var("AURORA_FOG") {
        Ok(v) => v.trim().parse::<f32>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0)),
        Err(_) => crate::demo::fog::scenario().map(|_| crate::demo::fog::DENSITY),
    })
}

/// `AURORA_FOG=off`: no fog until the Weather window is used, whatever the
/// saved settings say (a fog demo scene without its fog, to compare).
pub(super) fn env_fog_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var("AURORA_FOG").is_ok_and(|v| v.trim().eq_ignore_ascii_case("off")))
}

/// `AURORA_FOG_COLOUR=sky|<r>,<g>,<b>` (sRGB, 0..1): the fog's colour until
/// the Weather window is used: `Some(None)` the sky's light, `Some(Some(c))`
/// its own.
fn env_fog_colour() -> Option<Option<[f32; 3]>> {
    static COLOUR: std::sync::OnceLock<Option<Option<[f32; 3]>>> = std::sync::OnceLock::new();
    *COLOUR.get_or_init(|| parse_colour(&std::env::var("AURORA_FOG_COLOUR").ok()?))
}

fn parse_colour(v: &str) -> Option<Option<[f32; 3]>> {
    if v.trim().eq_ignore_ascii_case("sky") {
        return Some(None);
    }
    let c: Vec<f32> = v
        .split(',')
        .filter_map(|s| s.trim().parse::<f32>().ok())
        .filter(|x| x.is_finite())
        .collect();
    (c.len() == 3).then(|| Some([c[0], c[1], c[2]].map(|x| x.clamp(0.0, 1.0))))
}

/// `AURORA_FOG_TIER=height|volumetric|lights`: the fog's tier until the
/// Weather window is used.
fn env_fog_tier() -> Option<FogTier> {
    static TIER: std::sync::OnceLock<Option<FogTier>> = std::sync::OnceLock::new();
    *TIER.get_or_init(|| std::env::var("AURORA_FOG_TIER").ok().and_then(|v| parse_tier(&v)))
}

fn parse_tier(v: &str) -> Option<FogTier> {
    match v.trim().to_ascii_lowercase().as_str() {
        "height" => Some(FogTier::Height),
        "volumetric" => Some(FogTier::Volumetric),
        "lights" => Some(FogTier::VolumetricLights),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> WeatherSettings {
        WeatherSettings {
            enabled: true,
            fog_on: true,
            touched: true,
            ..WeatherSettings::default()
        }
    }

    #[test]
    fn the_fog_is_off_by_default_and_needs_both_switches() {
        let d = WeatherSettings {
            touched: true,
            ..WeatherSettings::default()
        };
        assert!(!d.fog_on);
        assert!(d.fog_look(Vec2::ZERO, 0.0, [0.0; 2], 2).is_none());
        let main_off = WeatherSettings { enabled: false, ..on() };
        assert!(main_off.fog_look(Vec2::ZERO, 0.0, [0.0; 2], 2).is_none());
        assert!(on().fog_look(Vec2::ZERO, 0.0, [0.0; 2], 2).is_some());
        // the main switch off: none drawn, the card's settings kept
        assert!(!main_off.in_effect().fog_on && main_off.fog_on);
    }

    #[test]
    fn the_fog_follows_the_sky_unless_given_its_own_colour() {
        let follow = on().fog_look(Vec2::ZERO, 0.0, [0.0; 2], 2).expect("on");
        assert_eq!(follow.color, None);
        assert!(follow.from_sky);
        let own = WeatherSettings {
            fog_follow_sky: false,
            fog_colour: [1.0, 0.0, 0.5],
            ..on()
        }
        .fog_look(Vec2::ZERO, 0.0, [0.0; 2], 2)
        .expect("on");
        let c = own.color.expect("own colour");
        assert_eq!((c[0], c[1]), (1.0, 0.0));
        assert!((c[2] - 0.214).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn its_settings_are_clamped_and_the_tier_is_kept() {
        let look = WeatherSettings {
            fog_density: 3.0,
            fog_height: 900.0,
            fog_anisotropy: -1.0,
            fog_tier: FogTier::Height,
            ..on()
        }
        .fog_look(Vec2::new(1.0, 2.0), 5.0, [256.0, 512.0], 7)
        .expect("on");
        assert_eq!((look.density, look.height, look.anisotropy), (1.0, *FOG_HEIGHT.end(), 0.0));
        assert_eq!((look.tier, look.quality), (aurora_render::FogTier::Height, 2));
        assert_eq!((look.drift, look.clock, look.region_origin), ([1.0, 2.0], 5.0, [256.0, 512.0]));
    }

    #[test]
    fn the_drift_is_the_wind_over_time_without_jumps() {
        let start = Instant::now();
        let mut d = FogDrift::default();
        d.advance(start, Vec2::new(10.0, 0.0), 0.5);
        assert_eq!(d.at, Vec2::ZERO);
        d.advance(start + std::time::Duration::from_millis(100), Vec2::new(10.0, 0.0), 0.5);
        assert!((d.at.x - 0.5).abs() < 1e-4, "{}", d.at);
        // a hitch of seconds moves it a quarter second's worth at most
        d.advance(start + std::time::Duration::from_secs(10), Vec2::new(10.0, 0.0), 0.5);
        assert!((d.at.x - 1.75).abs() < 1e-3, "{}", d.at);
        // a bad wind leaves it where it is
        let before = d.at;
        d.advance(start + std::time::Duration::from_millis(10_100), Vec2::new(f32::NAN, 0.0), 0.5);
        assert_eq!(d.at, before);
        // and it wraps on the noise's period
        let mut far = FogDrift {
            last: Some(start),
            at: Vec2::new(DRIFT_WRAP - 0.1, 0.0),
        };
        far.advance(start + std::time::Duration::from_millis(100), Vec2::new(10.0, 0.0), 1.0);
        assert!(far.at.x < 1.0, "{}", far.at);
    }

    #[test]
    fn the_tier_variable_reads_its_three_words() {
        assert_eq!(parse_tier("height"), Some(FogTier::Height));
        assert_eq!(parse_tier(" Volumetric "), Some(FogTier::Volumetric));
        assert_eq!(parse_tier("lights"), Some(FogTier::VolumetricLights));
        assert_eq!(parse_tier("off"), None);
    }

    #[test]
    fn the_colour_variable_reads_sky_or_three_numbers() {
        assert_eq!(parse_colour(" Sky "), Some(None));
        assert_eq!(parse_colour("1, 0.5, 2"), Some(Some([1.0, 0.5, 1.0])));
        assert_eq!(parse_colour("red"), None);
        assert_eq!(parse_colour("0.1,0.2"), None);
    }

    #[test]
    fn the_utc_clock_stays_within_a_day() {
        let c = utc_clock();
        assert!((0.0..86_400.0).contains(&c), "{c}");
    }
}
