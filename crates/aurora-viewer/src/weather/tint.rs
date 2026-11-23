//! Aurora Weather's tint: the colour and opacity of the rain and the snow,
//! carried into what they leave (the puddles and the wet ground, the snow
//! lying and the snow on avatars), how fast and how loosely they fall, and
//! the presets that set it all together (Blood rain, Ash, Acid rain, Dust
//! storm, Autumn). Drawn for this viewer only; the defaults draw what was
//! drawn before the tint.

use super::{Precipitation, WeatherSettings, fog::srgb_to_linear};

/// White: clear rain, white snow (the default, no tint).
pub const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
/// The fall speed scale's range (ash 0.6, 1 as before).
pub const FALL_SPEED: std::ops::RangeInclusive<f32> = 0.3..=1.5;
/// The flutter scale's range (ash 1.8, 1 as before).
pub const FLUTTER: std::ops::RangeInclusive<f32> = 0.0..=3.0;

/// A colour this near white counts as none: the fog keeps its own.
const NEAR_WHITE: f32 = 0.02;

fn is_white(c: [f32; 3]) -> bool {
    c.iter().all(|v| (1.0 - v).abs() <= NEAR_WHITE)
}

fn linear(c: [f32; 3], opacity: f32) -> [f32; 4] {
    [
        srgb_to_linear(c[0]),
        srgb_to_linear(c[1]),
        srgb_to_linear(c[2]),
        opacity.clamp(0.0, 1.0),
    ]
}

impl WeatherSettings {
    /// The renderer's tint (linear): the defaults give
    /// [`aurora_render::WeatherTint::NONE`], what was drawn before.
    pub fn tint(&self) -> aurora_render::WeatherTint {
        aurora_render::WeatherTint {
            rain: linear(self.rain_tint, self.rain_opacity),
            puddle_opacity: self.puddle_opacity.clamp(0.0, 1.0),
            snow: linear(self.snow_tint, self.snow_opacity),
        }
    }

    /// The fog's colour from what falls (linear), with "Fog follows the
    /// precipitation's colour" on, while rain or snow falls and its colour
    /// is not white; None: the fog's own choice.
    pub fn fog_fall_colour(&self) -> Option<[f32; 3]> {
        if !(self.fog_follows_fall && self.enabled && self.precipitation_on) {
            return None;
        }
        let c = if self.snowing() { self.snow_tint } else { self.rain_tint };
        (!is_white(c)).then(|| c.map(srgb_to_linear))
    }

    /// Set what `preset` sets: colour, opacity, fall, fog and wind; what it
    /// leaves (quality, distances, the ground's switches) stays as it was.
    /// The raw sliders under the presets show its values and can change them.
    pub fn apply_preset(&mut self, preset: WeatherPreset) {
        let clear = WeatherSettings::default();
        // each starts from the plain fall, then sets its own
        self.rain_tint = clear.rain_tint;
        self.rain_opacity = clear.rain_opacity;
        self.puddle_opacity = clear.puddle_opacity;
        self.snow_tint = clear.snow_tint;
        self.snow_opacity = clear.snow_opacity;
        self.fall_speed = clear.fall_speed;
        self.flutter = clear.flutter;
        self.fall_melts = clear.fall_melts;
        self.enabled = true;
        match preset {
            WeatherPreset::Clear => {
                self.fog_follows_fall = false;
                self.debris_on = false;
                self.tumbleweeds = false;
            }
            WeatherPreset::BloodRain => {
                self.precipitation_on = true;
                self.precipitation = Precipitation::Rain;
                self.rain_tint = [0.42, 0.02, 0.03];
                self.rain_opacity = 0.9;
                self.puddle_opacity = 0.85;
                self.fog_on = true;
                self.fog_follows_fall = true;
                self.fog_density = 0.25;
            }
            WeatherPreset::Ash => {
                self.precipitation_on = true;
                self.precipitation = Precipitation::Snow;
                self.snow_tint = [0.36, 0.35, 0.34];
                self.snow_size = 0.35;
                self.fall_speed = 0.6;
                self.flutter = 1.8;
                self.fall_melts = false;
                self.fog_on = true;
                self.fog_follows_fall = true;
                self.fog_density = 0.35;
            }
            WeatherPreset::AcidRain => {
                self.precipitation_on = true;
                self.precipitation = Precipitation::Rain;
                self.rain_tint = [0.62, 0.9, 0.18];
                self.rain_opacity = 0.8;
                self.puddle_opacity = 0.4;
                self.fog_on = true;
                self.fog_follows_fall = true;
                self.fog_density = 0.3;
            }
            WeatherPreset::DustStorm => {
                // nothing falls: sand in the air, low and thick, in a gale
                self.precipitation_on = false;
                self.fog_on = true;
                self.fog_follows_fall = false;
                self.fog_follow_sky = false;
                self.fog_colour = [0.78, 0.62, 0.42];
                self.fog_density = 0.75;
                self.fog_height = 20.0;
                self.fog_drift = 1.0;
                self.wind_on = true;
                self.wind_min = 0.45;
                self.wind_max = 0.85;
                self.debris_on = true;
                self.debris_kind = super::DebrisChoice::Sand;
                self.debris_density = 0.8;
                self.tumbleweeds = true;
            }
            WeatherPreset::Autumn => {
                self.precipitation_on = false;
                self.fog_follows_fall = false;
                self.wind_on = true;
                self.wind_min = 0.12;
                self.wind_max = 0.45;
                self.debris_on = true;
                self.debris_kind = super::DebrisChoice::Leaves;
                self.debris_density = 0.6;
                self.tumbleweeds = false;
            }
        }
    }
}

/// The Weather window's presets: tint, fall, fog and wind set together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherPreset {
    /// Back to clear rain and white snow (the colour and fall only).
    Clear,
    BloodRain,
    Ash,
    AcidRain,
    DustStorm,
    Autumn,
}

impl WeatherPreset {
    pub const ALL: [WeatherPreset; 6] = [
        WeatherPreset::Clear,
        WeatherPreset::BloodRain,
        WeatherPreset::Ash,
        WeatherPreset::AcidRain,
        WeatherPreset::DustStorm,
        WeatherPreset::Autumn,
    ];

    /// Its name in the window.
    pub fn label(self) -> &'static str {
        match self {
            WeatherPreset::Clear => aurora_i18n::tr!("weather-preset-clear"),
            WeatherPreset::BloodRain => aurora_i18n::tr!("weather-preset-blood-rain"),
            WeatherPreset::Ash => aurora_i18n::tr!("weather-preset-ash"),
            WeatherPreset::AcidRain => aurora_i18n::tr!("weather-preset-acid-rain"),
            WeatherPreset::DustStorm => aurora_i18n::tr!("weather-preset-dust-storm"),
            WeatherPreset::Autumn => aurora_i18n::tr!("weather-preset-autumn"),
        }
    }
}

/// `AURORA_WEATHER_PRESET=<plain|blood-rain|ash|acid-rain|dust-storm|autumn>`:
/// that preset until the Weather window is used (demo captures).
pub(crate) fn env_preset() -> Option<WeatherPreset> {
    static PRESET: std::sync::OnceLock<Option<WeatherPreset>> = std::sync::OnceLock::new();
    *PRESET.get_or_init(|| std::env::var("AURORA_WEATHER_PRESET").ok().and_then(|v| WeatherPreset::parse(&v)))
}

impl WeatherPreset {
    /// Its name in `AURORA_WEATHER_PRESET`.
    fn parse(name: &str) -> Option<WeatherPreset> {
        match name.trim().to_ascii_lowercase().as_str() {
            "plain" | "clear" => Some(WeatherPreset::Clear),
            "blood-rain" => Some(WeatherPreset::BloodRain),
            "ash" => Some(WeatherPreset::Ash),
            "acid-rain" => Some(WeatherPreset::AcidRain),
            "dust-storm" => Some(WeatherPreset::DustStorm),
            "autumn" => Some(WeatherPreset::Autumn),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_tint_draws_what_was_drawn_before() {
        let w = WeatherSettings::default();
        assert_eq!(w.tint(), aurora_render::WeatherTint::NONE);
        let wind = super::super::Wind::default();
        let look = w.rain_look(&wind);
        let before = aurora_render::RainLook::default();
        assert_eq!(look.fall_speed, before.fall_speed);
        assert_eq!(look.flutter, before.flutter);
        assert_eq!(look.fall_melts, before.fall_melts);
        // and the fog keeps its own colour
        assert_eq!(w.fog_fall_colour(), None);
    }

    #[test]
    fn the_fog_follows_only_a_coloured_fall_that_is_on() {
        let mut w = WeatherSettings {
            fog_follows_fall: true,
            enabled: true,
            precipitation_on: true,
            ..Default::default()
        };
        // white rain: the fog's own colour
        assert_eq!(w.fog_fall_colour(), None);
        w.rain_tint = [0.5, 0.0, 0.0];
        let c = w.fog_fall_colour().expect("red rain colours the fog");
        assert!(c[0] > 0.2 && c[1] == 0.0);
        // snow falling: the snow's colour (white here)
        w.precipitation = Precipitation::Snow;
        w.touched = true;
        assert_eq!(w.fog_fall_colour(), None);
        // nothing falling: none
        w.precipitation = Precipitation::Rain;
        w.precipitation_on = false;
        assert_eq!(w.fog_fall_colour(), None);
    }

    #[test]
    fn preset_names_parse() {
        assert_eq!(WeatherPreset::parse(" Blood-Rain "), Some(WeatherPreset::BloodRain));
        assert_eq!(WeatherPreset::parse("dust-storm"), Some(WeatherPreset::DustStorm));
        assert_eq!(WeatherPreset::parse("hail"), None);
    }

    #[test]
    fn presets_set_their_look_and_clear_goes_back_to_white() {
        let mut w = WeatherSettings::default();
        w.apply_preset(WeatherPreset::Ash);
        assert_eq!(w.precipitation, Precipitation::Snow);
        assert!(!w.fall_melts && w.fall_speed < 1.0 && w.flutter > 1.0);
        assert!(w.enabled && w.precipitation_on);
        w.apply_preset(WeatherPreset::BloodRain);
        // Ash's fall is not kept
        assert!(w.fall_melts && w.fall_speed == 1.0);
        assert!(w.puddle_opacity > 0.5 && w.rain_tint[0] > w.rain_tint[1]);
        w.apply_preset(WeatherPreset::Clear);
        assert_eq!(w.tint(), aurora_render::WeatherTint::NONE);
        w.apply_preset(WeatherPreset::DustStorm);
        assert!(w.fog_on && !w.precipitation_on && w.wind_max > 0.8);
    }
}
