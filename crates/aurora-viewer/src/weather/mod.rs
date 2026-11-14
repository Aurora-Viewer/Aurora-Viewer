//! The weather of this viewer: what the Environment window's "Weather"
//! section sets, kept in the settings. It is drawn for your view only.

pub mod region;
pub mod sky_mark;

use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// What falls from the sky.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Precipitation {
    #[default]
    Rain,
    Snow,
}

/// The colours of the aurora: green at its foot, and toward its top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AuroraColours {
    /// Green throughout, paler at the top: the most common.
    #[default]
    Green,
    /// Green under violet.
    Violet,
    /// Green under red: a strong storm's.
    Red,
}

impl AuroraColours {
    pub const ALL: [AuroraColours; 3] = [AuroraColours::Green, AuroraColours::Violet, AuroraColours::Red];

    /// Its number in the code and for the shader (env.wgsl `aurora_sky`).
    pub fn index(self) -> u8 {
        match self {
            AuroraColours::Green => 0,
            AuroraColours::Violet => 1,
            AuroraColours::Red => 2,
        }
    }

    pub fn from_index(i: u8) -> AuroraColours {
        match i {
            1 => AuroraColours::Violet,
            2 => AuroraColours::Red,
            _ => AuroraColours::Green,
        }
    }
}

/// How much of the weather's effects are drawn (their cost).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WeatherQuality {
    Low,
    Medium,
    #[default]
    High,
}

/// Puddle view distance, metres: the range of the slider and its default.
pub const PUDDLE_DISTANCE: std::ops::RangeInclusive<f32> = 10.0..=60.0;
pub const PUDDLE_DISTANCE_DEFAULT: f32 = 38.0;
/// How far snow lying on the ground is drawn, metres: the slider's range
/// and its default.
pub const SNOW_DISTANCE: std::ops::RangeInclusive<f32> = 20.0..=120.0;
pub const SNOW_DISTANCE_DEFAULT: f32 = 60.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherSettings {
    pub precipitation: Precipitation,
    /// How much falls, 0 (none) to 1 (a downpour).
    pub amount: f32,
    /// How hard the wind blows in its lulls, 0 (calm) to 1 (a gale).
    pub wind_min: f32,
    /// How hard it blows in its gusts (the files of the single Wind slider
    /// had only this one, as `wind`).
    #[serde(alias = "wind")]
    pub wind_max: f32,
    /// The wind blows the way the region's does; otherwise from
    /// `wind_from`.
    pub wind_follow_region: bool,
    /// Where the wind comes from when it does not follow the region's:
    /// degrees clockwise from north, as on a compass (270: a west wind).
    pub wind_from: f32,
    /// Puddles gather on level ground (rain only).
    pub puddles: bool,
    /// How far the wet ground and its puddles are drawn (m).
    pub puddle_distance: f32,
    /// Thunder and lightning in heavy rain.
    pub thunder: bool,
    /// How often lightning strikes, 0 (seldom) to 1 (often); 0.5 is the
    /// storm's own pace.
    pub thunder_frequency: f32,
    /// Let regions set the weather through their environment
    /// (weather/region.rs).
    pub region_weather: bool,
    /// How big the snowflakes are, 0 (fine powder) to 1 (big wet flakes);
    /// 0.5 is the middle.
    pub snow_size: f32,
    /// Snow settles on the ground (snow only), building up as it falls.
    pub snow_settles: bool,
    /// How far the snow lying on the ground is drawn (m).
    pub snow_distance: f32,
    /// Footprints in the lying snow, where avatars walk.
    pub snow_footprints: bool,
    /// How far around my avatar footprints are kept (m, the width of their
    /// map: 32 to 64, the wider the coarser).
    pub snow_footprints_reach: f32,
    /// Snow gathers on avatars out in the open while it snows, and melts
    /// away under cover, in rain or once it stops (`scene/avatar_snow.rs`).
    pub snow_on_avatars: bool,
    /// Puddles freeze while it snows (or snow lies): ice sheets.
    pub ice: bool,
    /// Deep snow (experimental): parallax through the lying snow.
    pub snow_deep: bool,
    /// The aurora (northern lights) in the night sky.
    pub aurora: bool,
    /// How bright it is, 0 (faint) to 1 (a strong display).
    pub aurora_intensity: f32,
    /// Where it stands: degrees clockwise from north, as on a compass.
    pub aurora_heading: f32,
    /// How high its foot is seen, 0 (on the horizon) to 1 (overhead).
    pub aurora_height: f32,
    pub aurora_colours: AuroraColours,
    pub quality: WeatherQuality,
    /// The menu was used this session: `AURORA_RAIN` no longer applies.
    #[serde(skip)]
    pub touched: bool,
}

impl Default for WeatherSettings {
    fn default() -> Self {
        Self {
            precipitation: Precipitation::Rain,
            amount: 0.0,
            wind_min: 0.0,
            wind_max: 0.0,
            wind_follow_region: true,
            wind_from: 270.0,
            puddles: true,
            puddle_distance: PUDDLE_DISTANCE_DEFAULT,
            thunder: false,
            thunder_frequency: 0.5,
            region_weather: false,
            snow_size: 0.5,
            snow_settles: true,
            snow_distance: SNOW_DISTANCE_DEFAULT,
            snow_footprints: true,
            snow_footprints_reach: 32.0,
            snow_on_avatars: true,
            ice: true,
            snow_deep: false,
            aurora: false,
            aurora_intensity: 0.6,
            aurora_heading: 0.0,
            aurora_height: 0.25,
            aurora_colours: AuroraColours::Green,
            quality: WeatherQuality::High,
            touched: false,
        }
    }
}

/// Who sets the weather drawn now (`App::update_weather_now`): the Weather
/// window shows it, since its own settings then wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeatherSource {
    /// The Weather window's settings.
    #[default]
    Mine,
    /// An environment editor's preview of a sky's Aurora Weather.
    Preview,
    /// A sky or day with Aurora Weather applied to myself.
    MySky,
    /// The region or parcel ("Let regions set the weather").
    Region,
    /// The same, where the region sets none: clear and calm.
    RegionClear,
}

impl WeatherSettings {
    /// How hard it rains, 0 to 1: the menu's, or `AURORA_RAIN` until the
    /// menu is used (demo captures).
    pub fn rain(&self) -> f32 {
        if !self.touched
            && let Some(v) = env_rain()
        {
            return v;
        }
        match self.precipitation {
            Precipitation::Rain => self.amount.clamp(0.0, 1.0),
            // snow wets nothing, makes no rain sound and no thunder
            Precipitation::Snow => 0.0,
        }
    }

    /// Snow is chosen (the menu's, or `AURORA_SNOW` until the menu is used).
    pub fn snowing(&self) -> bool {
        if !self.touched && env_snow().is_some() {
            return true;
        }
        self.precipitation == Precipitation::Snow
    }

    /// How hard whatever falls falls, rain or snow (0..1): what the drops
    /// pass draws.
    pub fn fall(&self) -> f32 {
        if !self.touched
            && let Some(v) = env_snow()
        {
            return v;
        }
        if self.snowing() {
            return self.rain_or_amount().clamp(0.0, 1.0);
        }
        self.rain()
    }

    /// The amount the slider shows: `AURORA_SNOW` or `AURORA_RAIN` until
    /// the menu is used.
    pub fn rain_or_amount(&self) -> f32 {
        if !self.touched
            && let Some(v) = env_snow().or_else(env_rain)
        {
            return v;
        }
        self.amount
    }

    /// The menu is used for the first time: what `AURORA_SNOW` or
    /// `AURORA_RAIN` showed becomes the setting, so that nothing changes
    /// under the pointer (snow shown as snow, its amount kept).
    pub fn take_over(&mut self, before: &WeatherSettings) {
        if before.touched {
            return;
        }
        if self.amount == before.amount {
            self.amount = before.rain_or_amount();
        }
        if self.precipitation == before.precipitation && before.snowing() {
            self.precipitation = Precipitation::Snow;
        }
    }

    /// The flakes' size scale for the renderer (0.5 to 2, 1 in the middle).
    pub fn flake_scale(&self) -> f32 {
        0.5 * 4.0f32.powf(self.snow_size.clamp(0.0, 1.0))
    }

    /// The wind's range, lulls to gusts (0..1, the lower first).
    pub fn wind_range(&self) -> (f32, f32) {
        let (a, b) = (self.wind_min.clamp(0.0, 1.0), self.wind_max.clamp(0.0, 1.0));
        (a.min(b), a.max(b))
    }

    /// Where the wind blows to (unit, world: +x east, +y north) when it does
    /// not follow the region's.
    pub fn wind_heading(&self) -> Option<Vec2> {
        if self.wind_follow_region {
            return None;
        }
        let a = self.wind_from.to_radians();
        // from the north (0°) it blows south
        Some(-Vec2::new(a.sin(), a.cos()))
    }

    /// What the renderer needs besides the rain's strength.
    /// The aurora for the renderer (`RainLook::aurora`): none while off.
    pub fn aurora_look(&self) -> [f32; 4] {
        if !self.touched
            && let Some((brightness, heading, colours)) = env_northern_lights()
        {
            let lit = WeatherSettings {
                aurora: true,
                aurora_intensity: brightness,
                aurora_heading: heading,
                aurora_colours: AuroraColours::from_index(colours),
                touched: true,
                ..self.clone()
            };
            return lit.aurora_look();
        }
        if !self.aurora {
            return [0.0; 4];
        }
        [
            // never quite nothing while on: the faintest is still seen
            0.08 + 0.92 * self.aurora_intensity.clamp(0.0, 1.0),
            self.aurora_heading.rem_euclid(360.0).to_radians(),
            self.aurora_height.clamp(0.0, 1.0),
            f32::from(self.aurora_colours.index()),
        ]
    }

    pub fn rain_look(&self, wind: &Wind) -> aurora_render::RainLook {
        // in snow, the puddles' switch and distance are the lying snow's
        let snowing = self.snowing();
        aurora_render::RainLook {
            puddles: if snowing { self.snow_settles } else { self.puddles },
            distance: if snowing {
                self.snow_distance.clamp(*SNOW_DISTANCE.start(), *SNOW_DISTANCE.end())
            } else {
                self.puddle_distance.clamp(*PUDDLE_DISTANCE.start(), *PUDDLE_DISTANCE.end())
            },
            wind: wind.now.to_array(),
            drift: wind.drift.to_array(),
            snow: if self.snowing() { self.flake_scale() } else { 0.0 },
            // the app gives the detail texture's slot (scene/snow_detail.rs)
            snow_detail: 0,
            // and a region's snow so far (weather/region.rs)
            snow_lying: 0.0,
            footprints: self.snow_footprints && self.snow_settles,
            trail_extent: self.snow_footprints_reach.clamp(32.0, 64.0),
            // the app centres it on my avatar
            trail_centre: [0.0; 2],
            ice: self.ice && self.puddles,
            deep_snow: env_snow_deep().filter(|_| !self.touched).unwrap_or(self.snow_deep) && self.snow_settles,
            // the app lights the sky where the lightning struck (thunder_light)
            lightning: [0.0; 4],
            aurora: self.aurora_look(),
            // the app gives the feet set down (scene/snow_trail.rs)
            snow_stamps: [[0.0; 4]; aurora_render::SNOW_STAMPS],
            snow_stamp_count: 0,
            snow_stamp_z: [0.0; aurora_render::SNOW_STAMPS],
            // the app sets them the frame the clean-up or the snow-fill
            // button is pressed
            clean: false,
            fill: false,
            quality: match env_quality().filter(|_| !self.touched).unwrap_or(self.quality) {
                WeatherQuality::Low => 0,
                WeatherQuality::Medium => 1,
                WeatherQuality::High => 2,
            },
        }
    }
}

/// The wind of the Wind sliders at their strongest (m/s, a gale).
const WIND_MAX: f32 = 18.0;

/// The weather's wind: wandering between the lulls and the gusts of the
/// Wind sliders, blowing the region's way or the one chosen. `drift` is how
/// far it has carried the drops so far (the sum of the wind over time: a
/// gust bends the rain, the drops never jump).
#[derive(Debug, Clone)]
pub struct Wind {
    last: Option<Instant>,
    clock: f32,
    /// The direction it blows (unit), turning slowly to the one it follows.
    heading: Vec2,
    /// The wind now (m/s).
    pub now: Vec2,
    /// How far it has carried a drop so far (m).
    pub drift: Vec2,
}

impl Default for Wind {
    fn default() -> Self {
        Self {
            last: None,
            clock: 0.0,
            heading: Vec2::new(0.94, 0.34),
            now: Vec2::ZERO,
            drift: Vec2::ZERO,
        }
    }
}

/// Where the wind stands between its lulls (0) and its gusts (1) at `t`
/// seconds: slow swells of a minute or so with quicker gusts on them, of no
/// common period. It stays nearer the lulls and rises in gusts, as wind
/// does.
fn gustiness(t: f32) -> f32 {
    let swell = 0.5 * (t * 0.071 + 0.4).sin() + 0.3 * (t * 0.163 + 2.1).sin();
    let gust = 0.6 * (t * 0.53 + 1.3).sin() + 0.4 * (t * 1.37 + 4.0).sin();
    let n = 0.5 + 0.5 * (0.6 * swell + 0.4 * gust) / 0.8;
    n.clamp(0.0, 1.0).powf(1.4)
}

impl Wind {
    /// Move on to `at`: `range` the sliders' lulls and gusts (0..1),
    /// `heading` the chosen direction (None: the region's), `region` the
    /// region's wind at the camera (m/s; its direction only).
    pub fn advance(&mut self, at: Instant, range: (f32, f32), heading: Option<Vec2>, region: Vec2) {
        let dt = self.last.map_or(0.0, |t| at.saturating_duration_since(t).as_secs_f32().min(0.25));
        self.last = Some(at);
        self.clock += dt;
        let to = heading.or_else(|| (region.length_squared() > 0.09).then(|| region.normalize()));
        if let Some(to) = to {
            // a chosen direction turns in a few seconds, the region's slowly
            let tau = if heading.is_some() { 3.0 } else { 20.0 };
            let k = 1.0 - (-dt / tau).exp();
            // turned by its angle, so that it goes round even from right
            // against it
            let a = self.heading.angle_to(to);
            self.heading = Vec2::from_angle(a * k).rotate(self.heading).normalize_or(to);
        }
        let (lo, hi) = (range.0.clamp(0.0, 1.0), range.1.clamp(0.0, 1.0));
        let (lo, hi) = (lo.min(hi), lo.max(hi));
        let t = self.clock;
        let level = lo + (hi - lo) * gustiness(t);
        let speed = WIND_MAX * level.powf(1.5);
        // the direction swings a little, more in the gusts
        let swing = (0.12 * (t * 0.23 + 0.7).sin() + 0.06 * (t * 0.71).sin()) * (0.6 + 0.4 * gustiness(t));
        let dir = Vec2::from_angle(swing).rotate(self.heading);
        self.now = dir * speed;
        self.drift += self.now * dt;
    }

    /// The wind's speed now (m/s).
    #[cfg(test)]
    pub fn speed(&self) -> f32 {
        self.now.length()
    }
}

/// `AURORA_SNOW=<0..1>`: snow of this strength until the menu is used
/// (demo captures).
fn env_snow() -> Option<f32> {
    static SNOW: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *SNOW.get_or_init(|| {
        std::env::var("AURORA_SNOW")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| v.clamp(0.0, 1.0))
    })
}

/// `AURORA_WEATHER_QUALITY=low|medium|high`: the weather's quality until
/// the menu is used (measurements, `AURORA_PROFILE`).
fn env_quality() -> Option<WeatherQuality> {
    static QUALITY: std::sync::OnceLock<Option<WeatherQuality>> = std::sync::OnceLock::new();
    *QUALITY.get_or_init(|| {
        std::env::var("AURORA_WEATHER_QUALITY")
            .ok()
            .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
                "low" => Some(WeatherQuality::Low),
                "medium" => Some(WeatherQuality::Medium),
                "high" => Some(WeatherQuality::High),
                _ => None,
            })
    })
}

/// `AURORA_SNOW_DEEP=0|1`: deep snow off or on until the menu is used.
fn env_snow_deep() -> Option<bool> {
    static DEEP: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *DEEP.get_or_init(|| std::env::var("AURORA_SNOW_DEEP").ok().map(|v| v.trim() == "1"))
}

/// `AURORA_NORTHERN_LIGHTS=<0..1>[,<deg>[,<colours>]]`: the northern lights
/// until the menu is used (demo captures).
fn env_northern_lights() -> Option<(f32, f32, u8)> {
    static LIGHTS: std::sync::OnceLock<Option<(f32, f32, u8)>> = std::sync::OnceLock::new();
    *LIGHTS.get_or_init(|| std::env::var("AURORA_NORTHERN_LIGHTS").ok().and_then(|v| parse_northern_lights(&v)))
}

fn parse_northern_lights(spec: &str) -> Option<(f32, f32, u8)> {
    let mut parts = spec.split(',').map(str::trim);
    let brightness = parts.next()?.parse::<f32>().ok().filter(|v| v.is_finite())?.clamp(0.0, 1.0);
    let heading = parts
        .next()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    let colours = parts.next().and_then(|v| v.parse::<u8>().ok()).unwrap_or(0).min(2);
    Some((brightness, heading, colours))
}

fn env_rain() -> Option<f32> {
    static RAIN: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *RAIN.get_or_init(|| {
        std::env::var("AURORA_RAIN")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| v.clamp(0.0, 1.0))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_northern_lights_reach_the_renderer_only_while_on() {
        let off = WeatherSettings {
            touched: true,
            ..Default::default()
        };
        assert_eq!(off.aurora_look(), [0.0; 4]);
        let on = WeatherSettings {
            aurora: true,
            aurora_intensity: 0.0,
            aurora_heading: 450.0,
            aurora_colours: AuroraColours::Red,
            ..off
        };
        let look = on.aurora_look();
        assert!(look[0] > 0.0, "the faintest is still seen");
        assert!((look[1] - 90f32.to_radians()).abs() < 1e-5);
        assert_eq!(look[3], 2.0);
        assert_eq!(parse_northern_lights("0.8, 270, 1"), Some((0.8, 270.0, 1)));
        assert_eq!(parse_northern_lights("2"), Some((1.0, 0.0, 0)));
        assert_eq!(parse_northern_lights("x"), None);
    }

    #[test]
    fn snow_draws_no_rain_and_old_settings_files_get_the_defaults() {
        let mut w = WeatherSettings {
            amount: 0.7,
            touched: true,
            ..Default::default()
        };
        assert_eq!(w.rain(), 0.7);
        w.precipitation = Precipitation::Snow;
        assert_eq!(w.rain(), 0.0);
        // but it falls: the drops pass draws it as snow
        assert_eq!(w.fall(), 0.7);
        assert!(w.snowing() && w.rain_look(&Wind::default()).snow > 0.0);
        let read: WeatherSettings = serde_json::from_str("{\"amount\": 0.3}").expect("parse");
        assert!(read.puddles);
        assert_eq!(read.puddle_distance, PUDDLE_DISTANCE_DEFAULT);
    }

    #[test]
    fn the_wind_wanders_between_its_lulls_and_gusts_and_carries_the_drops_smoothly() {
        let start = Instant::now();
        let mut w = Wind::default();
        let (lo, hi) = (0.3f32, 0.8f32);
        let (slow_cap, fast_cap) = (WIND_MAX * lo.powf(1.5), WIND_MAX * hi.powf(1.5));
        let mut slowest = f32::MAX;
        let mut fastest = 0.0f32;
        let mut last_drift = Vec2::ZERO;
        for i in 0..20_000 {
            let at = start + std::time::Duration::from_millis(16 * i);
            w.advance(at, (lo, hi), None, Vec2::new(0.0, 5.0));
            if i > 0 {
                slowest = slowest.min(w.speed());
                fastest = fastest.max(w.speed());
                // no jump: at most the strongest gust over one frame
                assert!((w.drift - last_drift).length() < fast_cap * 0.017, "{i}");
            }
            last_drift = w.drift;
        }
        // never below the lulls nor above the gusts, and it uses the range
        assert!(slowest >= slow_cap - 1e-3 && fastest <= fast_cap + 1e-3, "{slowest} {fastest}");
        assert!(slowest < slow_cap + 0.25 * (fast_cap - slow_cap), "lulls reached: {slowest}");
        assert!(fastest > fast_cap - 0.25 * (fast_cap - slow_cap), "gusts reached: {fastest}");
        // turned to the region wind (north) in the 5 minutes
        assert!(w.heading.y > 0.95, "{:?}", w.heading);
        // the same both ways round, and calm is calm
        let mut swapped = Wind::default();
        swapped.advance(start, (hi, lo), None, Vec2::ZERO);
        swapped.advance(start + std::time::Duration::from_secs(1), (hi, lo), None, Vec2::ZERO);
        assert!(swapped.speed() >= slow_cap - 1e-3);
        let mut calm = Wind::default();
        calm.advance(start, (0.0, 0.0), None, Vec2::ZERO);
        calm.advance(start + std::time::Duration::from_secs(1), (0.0, 0.0), None, Vec2::ZERO);
        assert_eq!(calm.now, Vec2::ZERO);
    }

    #[test]
    fn a_chosen_wind_direction_wins_over_the_regions_and_is_a_compass_one() {
        let w = WeatherSettings {
            wind_follow_region: false,
            wind_from: 270.0,
            ..Default::default()
        };
        // a west wind blows east
        let east = w.wind_heading().expect("chosen");
        assert!(east.x > 0.99, "{east}");
        let north = WeatherSettings {
            wind_from: 180.0,
            ..w.clone()
        }
        .wind_heading()
        .expect("chosen");
        assert!(north.y > 0.99, "{north}");
        assert!(WeatherSettings::default().wind_heading().is_none());
        // straight against the start, and the region's wind ignored
        let start = Instant::now();
        let mut wind = Wind {
            heading: -east,
            ..Default::default()
        };
        for i in 0..1000 {
            wind.advance(
                start + std::time::Duration::from_millis(16 * i),
                (0.5, 0.5),
                Some(east),
                Vec2::new(0.0, 5.0),
            );
        }
        assert!(wind.heading.x > 0.95, "{:?}", wind.heading);
    }

    #[test]
    fn old_settings_files_keep_their_wind_as_the_gusts() {
        let read: WeatherSettings = serde_json::from_str("{\"wind\": 0.6}").expect("parse");
        assert_eq!(read.wind_range(), (0.0, 0.6));
        assert!(read.wind_follow_region);
        assert_eq!(read.thunder_frequency, 0.5);
    }
}
