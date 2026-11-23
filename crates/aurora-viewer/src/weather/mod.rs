//! The weather of this viewer: what the Environment window's "Weather"
//! section sets, kept in the settings. It is drawn for your view only.

use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub mod debris;
pub mod fog;
pub mod tint;
pub use debris::DebrisChoice;
pub use fog::{FogDrift, FogTier};
pub use tint::WeatherPreset;

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
pub const PUDDLE_DISTANCE: std::ops::RangeInclusive<f32> = 10.0..=256.0;
pub const PUDDLE_DISTANCE_DEFAULT: f32 = 38.0;
/// How far snow lying on the ground is drawn, metres: the slider's range
/// and its default.
pub const SNOW_DISTANCE: std::ops::RangeInclusive<f32> = 20.0..=256.0;
pub const SNOW_DISTANCE_DEFAULT: f32 = 60.0;
/// Deep snow's reach around my avatar (m, at High weather quality): as far
/// as the rain's shelter map, which its surface is read from. Its cells
/// grow coarser toward its edge, one more ring of them past each doubling
/// (4 at 15 m, 5 at 40 m): 40 m costs about 0.12 ms of GPU more than 15 m
/// (RTX 3090, 1080p, 2026-11-15).
pub const DEEP_SNOW_REACH: std::ops::RangeInclusive<f32> = 10.0..=40.0;
pub const DEEP_SNOW_REACH_DEFAULT: f32 = 15.0;
/// The wind's defaults, its lulls and its gusts (0..1): a breeze, gusting
/// to a steady wind (`ui/weather_window.rs` `wind_word`).
pub const WIND_MIN_DEFAULT: f32 = 0.1;
pub const WIND_MAX_DEFAULT: f32 = 0.35;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WeatherSettings {
    /// The window's main switch: off, none of its weather is drawn (its
    /// settings are kept, greyed). Off by default.
    pub enabled: bool,
    /// The "Falling" card's switch: rain or snow falls (and thunder comes
    /// with the rain). Off by default.
    pub precipitation_on: bool,
    pub precipitation: Precipitation,
    /// How much falls, 0 (none) to 1 (a downpour).
    pub amount: f32,
    /// The "Wind" card's switch: the weather's wind blows. Off by default.
    pub wind_on: bool,
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
    /// How far around my avatar deep snow is drawn (m, `DEEP_SNOW_REACH`).
    pub snow_deep_reach: f32,
    /// The aurora (northern lights) in the night sky.
    pub aurora: bool,
    /// How bright it is, 0 (faint) to 1 (a strong display).
    pub aurora_intensity: f32,
    /// Where it stands: degrees clockwise from north, as on a compass.
    pub aurora_heading: f32,
    /// How high its foot is seen, 0 (on the horizon) to 1 (overhead).
    pub aurora_height: f32,
    pub aurora_colours: AuroraColours,
    /// The "Fog" card's switch (weather/fog.rs): fog that pools low,
    /// drifts with the wind and scatters the light. Off by default.
    pub fog_on: bool,
    /// How thick it is, 0 (a light mist) to 1 (a pea-souper).
    pub fog_density: f32,
    /// How high it reaches before it thins to a third (m).
    pub fog_height: f32,
    /// How much of the wind's speed carries it (0..1).
    pub fog_drift: f32,
    /// It takes the sky's light alone, as the haze does; off, `fog_colour`.
    pub fog_follow_sky: bool,
    /// Its own colour (sRGB) when it does not follow the sky.
    pub fog_colour: [f32; 3],
    /// The region's sky haze scales it: a hazy sky, a thicker fog.
    pub fog_from_sky: bool,
    /// How it is drawn: height fog, a volume, a volume lit by the lights.
    pub fog_tier: FogTier,
    /// The glow around the lights and the sun (0 even, 0.9 tight).
    pub fog_anisotropy: f32,
    /// The rain's colour (sRGB, weather/tint.rs): white is clear water.
    pub rain_tint: [f32; 3],
    /// How much of the drops is seen (0..1; 1 as before).
    pub rain_opacity: f32,
    /// How opaque the puddles' liquid is (0 clear water, as before; 1 as
    /// thick as paint: its own colour, rough, hardly a mirror).
    pub puddle_opacity: f32,
    /// The snow's colour (sRGB): falling, lying and on avatars.
    pub snow_tint: [f32; 3],
    /// How much of the snow is seen (0..1; 1 as before).
    pub snow_opacity: f32,
    /// How fast what falls falls (a scale, 1 as before; ash falls slower).
    pub fall_speed: f32,
    /// How far flakes swing as they fall (a scale, 1 as before).
    pub flutter: f32,
    /// Snow lying melts once it stops falling (off: ash stays).
    pub fall_melts: bool,
    /// The fog takes the colour of what falls (when it is not white).
    pub fog_follows_fall: bool,
    /// The "Debris" card's switch (weather/debris.rs): the wind blows
    /// leaves, sand, paper or embers about. Off by default.
    pub debris_on: bool,
    pub debris_kind: DebrisChoice,
    /// How much of it (0..1).
    pub debris_density: f32,
    /// Leaves with no foliage near come on the wind from upwind.
    pub debris_stream: bool,
    /// Tumbleweeds roll across open ground (their own switch, with any
    /// debris or none).
    pub tumbleweeds: bool,
    /// Textures never taken as foliage (false positives of the classifier,
    /// `scene::jobs::classify_foliage`): kept on this computer only.
    pub debris_not_foliage: Vec<uuid::Uuid>,
    pub quality: WeatherQuality,
    /// The menu was used this session: `AURORA_RAIN` no longer applies.
    #[serde(skip)]
    pub touched: bool,
    /// The former switch "Let regions set the weather" (removed with the
    /// region weather, #4280), read once by `upgrade_to_switches`: on, none
    /// of these settings was drawn.
    #[serde(rename = "region_weather", skip_serializing)]
    pub(crate) former_region_weather: Option<bool>,
}

impl Default for WeatherSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            precipitation_on: false,
            precipitation: Precipitation::Rain,
            // the switches turn the weather on: once on, it falls at full
            // strength
            amount: 1.0,
            wind_on: false,
            // and once the wind is on, it blows: a breeze
            wind_min: WIND_MIN_DEFAULT,
            wind_max: WIND_MAX_DEFAULT,
            wind_follow_region: true,
            wind_from: 270.0,
            puddles: true,
            puddle_distance: PUDDLE_DISTANCE_DEFAULT,
            thunder: false,
            thunder_frequency: 0.5,
            snow_size: 0.5,
            snow_settles: true,
            snow_distance: SNOW_DISTANCE_DEFAULT,
            snow_footprints: true,
            snow_footprints_reach: 32.0,
            snow_on_avatars: true,
            ice: true,
            snow_deep: false,
            snow_deep_reach: DEEP_SNOW_REACH_DEFAULT,
            aurora: false,
            aurora_intensity: 0.6,
            aurora_heading: 0.0,
            aurora_height: 0.25,
            aurora_colours: AuroraColours::Green,
            fog_on: false,
            fog_density: fog::FOG_DENSITY_DEFAULT,
            fog_height: fog::FOG_HEIGHT_DEFAULT,
            fog_drift: fog::FOG_DRIFT_DEFAULT,
            fog_follow_sky: true,
            fog_colour: fog::FOG_COLOUR_DEFAULT,
            fog_from_sky: true,
            fog_tier: FogTier::VolumetricLights,
            fog_anisotropy: fog::FOG_ANISOTROPY_DEFAULT,
            rain_tint: tint::WHITE,
            rain_opacity: 1.0,
            puddle_opacity: 0.0,
            snow_tint: tint::WHITE,
            snow_opacity: 1.0,
            fall_speed: 1.0,
            flutter: 1.0,
            fall_melts: true,
            fog_follows_fall: false,
            debris_on: false,
            debris_kind: DebrisChoice::Leaves,
            debris_density: debris::DEBRIS_DENSITY_DEFAULT,
            debris_stream: false,
            tumbleweeds: false,
            debris_not_foliage: Vec::new(),
            quality: WeatherQuality::High,
            touched: false,
            former_region_weather: None,
        }
    }
}

/// A teleport to another region (#3664): whether what the weather left on
/// the ground (lying snow, wet ground and puddles, footprints, snow on the
/// avatars) goes, as with the Weather window's clean-up button. It belongs
/// to the place left, and kept, it lay on the new region for many minutes
/// while it melted. Only rain or snow falling now keeps it: it falls the
/// same everywhere, and cleaned up, the ground went bare or dry under it and
/// filled again.
pub fn clean_on_arrival(now: &WeatherSettings) -> bool {
    now.fall() <= 0.0
}

impl WeatherSettings {
    /// The weather drawn: these settings with what their switches turn off
    /// taken out (`App::update_weather_now`, the window's summary). Off is
    /// what it was before the switches: nothing falling, no wind, no aurora.
    /// `AURORA_RAIN`, `AURORA_SNOW` and `AURORA_NORTHERN_LIGHTS` still win
    /// until the window is used (they are read before these values).
    pub fn in_effect(&self) -> WeatherSettings {
        let mut w = self.clone();
        // what the variables set is on, for the summary's words too
        if !w.touched && (env_rain().is_some() || env_snow().is_some()) {
            w.enabled = true;
            w.precipitation_on = true;
        }
        if !w.touched && env_northern_lights().is_some() {
            w.enabled = true;
        }
        if !w.touched && fog::env_fog().is_some() {
            w.enabled = true;
            w.fog_on = true;
        }
        if !w.touched && fog::env_fog_off() {
            w.fog_on = false;
        }
        if !w.touched
            && let Some(preset) = tint::env_preset()
        {
            w.apply_preset(preset);
        }
        if !w.touched
            && let Some((kind, stream)) = debris::env_debris()
        {
            w.enabled = true;
            w.debris_on = true;
            w.debris_kind = kind;
            w.debris_stream = stream;
        }
        if !(w.enabled && w.precipitation_on) {
            w.amount = 0.0;
            w.thunder = false;
        }
        if !(w.enabled && w.wind_on) {
            w.wind_min = 0.0;
            w.wind_max = 0.0;
        }
        if !w.enabled {
            w.aurora = false;
            w.fog_on = false;
            w.debris_on = false;
            w.tumbleweeds = false;
        }
        w
    }

    /// A file from before the switches (settings version 43): what it drew
    /// stays on. Nothing falling, no wind and no aurora was how it was off;
    /// a fall at 0 then starts at full strength, as a new one does. With
    /// "Let regions set the weather" on, none of it was drawn (the region's
    /// weather, or a clear and calm sky where it set none): the main switch
    /// stays off, the cards keep these settings for when it is turned on.
    pub fn upgrade_to_switches(&mut self) {
        let regions_set_it = self.former_region_weather.take() == Some(true);
        self.precipitation_on = self.amount > 0.0;
        self.wind_on = self.wind_max > 0.0;
        self.enabled = !regions_set_it && (self.precipitation_on || self.wind_on || self.aurora);
        if self.amount <= 0.0 {
            self.amount = 1.0;
        }
    }

    /// A file from before the wind's defaults (settings version 44): a wind
    /// switched off and left still takes them, so that switching it on
    /// blows, as a new one does.
    pub fn upgrade_wind_defaults(&mut self) {
        if !self.wind_on && self.wind_max <= 0.0 {
            self.wind_min = WIND_MIN_DEFAULT;
            self.wind_max = WIND_MAX_DEFAULT;
        }
    }

    /// How hard it rains, 0 to 1: the menu's, or `AURORA_RAIN` until the
    /// menu is used (demo captures).
    pub fn rain(&self) -> f32 {
        if !self.touched
            && let Some(v) = env_rain()
        {
            // AURORA_SNOW_AFTER: the rain stops when the snow starts
            return if env_snow_now().is_some() { 0.0 } else { v };
        }
        match self.precipitation {
            Precipitation::Rain => self.amount.clamp(0.0, 1.0),
            // snow wets nothing, makes no rain sound and no thunder
            Precipitation::Snow => 0.0,
        }
    }

    /// Snow is chosen (the menu's, or `AURORA_SNOW` until the menu is used).
    pub fn snowing(&self) -> bool {
        if !self.touched && env_snow_now().is_some() {
            return true;
        }
        self.precipitation == Precipitation::Snow
    }

    /// How hard whatever falls falls, rain or snow (0..1): what the drops
    /// pass draws.
    pub fn fall(&self) -> f32 {
        if !self.touched
            && let Some(v) = env_snow_now()
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
            && let Some(v) = env_snow_now().or_else(env_rain)
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
        // and what they made fall stays on, unless a switch was just used
        if (env_rain().is_some() || env_snow().is_some())
            && self.enabled == before.enabled
            && self.precipitation_on == before.precipitation_on
        {
            self.enabled = true;
            self.precipitation_on = true;
        }
    }

    /// The flakes' size scale for the renderer (0.5 to 2, 1 in the middle).
    pub fn flake_scale(&self) -> f32 {
        0.5 * 4.0f32.powf(self.snow_size.clamp(0.0, 1.0))
    }

    /// Where the weather's wind stands between its lulls and gusts, for what
    /// the gusts lift and churn (the debris, the drops): none with the wind
    /// off or steady (its lulls as strong as its gusts), all of the curve
    /// once the gusts are well above the lulls.
    pub fn gust(&self, wind: &Wind) -> f32 {
        let (lo, hi) = self.wind_range();
        wind.gust * ((hi - lo) * 4.0).clamp(0.0, 1.0)
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

    /// How far the weather's memory of the ground must reach (m,
    /// `RainLook::memory_reach`): the wet ground's distance (drawn in any
    /// rain, puddles or not), and the snow's while it settles, falling or
    /// melting (`RainLook::snow_distance`; with "Snow settles" off none
    /// lies, and its hidden slider changes nothing).
    /// Both whichever falls: the memory's square keeps its size from rain
    /// to snow (a new size starts its texels again).
    fn memory_reach(&self) -> f32 {
        let wet = self.puddle_distance.clamp(*PUDDLE_DISTANCE.start(), *PUDDLE_DISTANCE.end());
        let snow = self.snow_distance.clamp(*SNOW_DISTANCE.start(), *SNOW_DISTANCE.end());
        if self.snow_settles { wet.max(snow) } else { wet }
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
            // the app gives AURORA_SNOW_LYING's floor (demo captures)
            snow_lying: 0.0,
            memory_reach: self.memory_reach(),
            // the same while it melts as while it fell
            snow_distance: self.snow_distance.clamp(*SNOW_DISTANCE.start(), *SNOW_DISTANCE.end()),
            deep_reach: env_snow_deep_reach()
                .filter(|_| !self.touched)
                .unwrap_or(self.snow_deep_reach)
                .clamp(*DEEP_SNOW_REACH.start(), *DEEP_SNOW_REACH.end()),
            // whatever falls: in rain or clear weather `puddles` is the
            // puddles' own switch, and the snow melting away still showed
            snow_settles: self.snow_settles,
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
            quality: self.quality_index(),
            // (the app draws what lies in the sand's colour while it is
            // the sand's, `App::lying_sand`)
            tint: self.tint(),
            sand: self.sand_blowing(),
            gust: self.gust(wind),
            fall_speed: self.fall_speed.clamp(*tint::FALL_SPEED.start(), *tint::FALL_SPEED.end()),
            flutter: self.flutter.clamp(*tint::FLUTTER.start(), *tint::FLUTTER.end()),
            fall_melts: self.fall_melts,
        }
    }

    /// The weather's quality as the renderer takes it: 0 low, 1 medium, 2
    /// high (`AURORA_WEATHER_QUALITY` until the window is used).
    pub fn quality_index(&self) -> u32 {
        match env_quality().filter(|_| !self.touched).unwrap_or(self.quality) {
            WeatherQuality::Low => 0,
            WeatherQuality::Medium => 1,
            WeatherQuality::High => 2,
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
    /// Where it stands between its lulls (0) and its gusts (1): the one
    /// gust curve the drops, the fog and the debris share.
    pub gust: f32,
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
            gust: 0.0,
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
        self.gust = gustiness(t);
        let level = lo + (hi - lo) * self.gust;
        let speed = WIND_MAX * level.powf(1.5);
        // the direction swings a little, more in the gusts
        let swing = (0.12 * (t * 0.23 + 0.7).sin() + 0.06 * (t * 0.71).sin()) * (0.6 + 0.4 * self.gust);
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

/// `AURORA_SNOW` once `AURORA_SNOW_AFTER=<s>` seconds have passed since
/// launch (at once without it): before, `AURORA_RAIN` falls, so that a
/// capture shows the puddles it left frozen.
fn env_snow_now() -> Option<f32> {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static AFTER: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    let start = *START.get_or_init(std::time::Instant::now);
    let after = *AFTER.get_or_init(|| {
        std::env::var("AURORA_SNOW_AFTER")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    });
    env_snow().filter(|_| start.elapsed().as_secs_f32() >= after)
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

/// `AURORA_SNOW_DEEP_REACH=<m>`: deep snow's reach until the menu is used
/// (demo captures: the terrain past the plaza's floor).
fn env_snow_deep_reach() -> Option<f32> {
    static REACH: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *REACH.get_or_init(|| std::env::var("AURORA_SNOW_DEEP_REACH").ok().and_then(|v| v.trim().parse().ok()))
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
    fn a_teleport_cleans_up_unless_rain_or_snow_is_falling() {
        let weather = |precipitation, amount| WeatherSettings {
            precipitation,
            amount,
            touched: true,
            ..WeatherSettings::default()
        };
        assert!(!clean_on_arrival(&weather(Precipitation::Snow, 0.6)));
        assert!(!clean_on_arrival(&weather(Precipitation::Rain, 0.6)));
        assert!(clean_on_arrival(&weather(Precipitation::Snow, 0.0)), "snow at amount 0");
        assert!(clean_on_arrival(&weather(Precipitation::Rain, 0.0)), "rain at amount 0");
    }

    #[test]
    fn the_renderer_gets_both_distances_and_deep_snows_reach() {
        let w = WeatherSettings {
            puddle_distance: 200.0,
            snow_distance: 60.0,
            snow_deep_reach: 30.0,
            touched: true,
            ..WeatherSettings::default()
        };
        let look = w.rain_look(&Wind::default());
        // the farther of the two, whatever falls now
        assert_eq!(look.memory_reach, 200.0);
        assert_eq!(look.deep_reach, 30.0);
        // the snow's only while it settles: its hidden slider sizes nothing
        let snow_far = WeatherSettings {
            snow_distance: 250.0,
            ..w.clone()
        };
        assert_eq!(snow_far.rain_look(&Wind::default()).memory_reach, 250.0);
        let unsettled = WeatherSettings {
            snow_settles: false,
            ..snow_far
        };
        assert_eq!(unsettled.rain_look(&Wind::default()).memory_reach, 200.0);
        // the defaults keep today's sizes
        let d = WeatherSettings {
            touched: true,
            ..WeatherSettings::default()
        }
        .rain_look(&Wind::default());
        assert_eq!((d.memory_reach, d.deep_reach), (SNOW_DISTANCE_DEFAULT, DEEP_SNOW_REACH_DEFAULT));
        // out of range: clamped
        let far = WeatherSettings {
            snow_deep_reach: 500.0,
            puddle_distance: 9000.0,
            ..w
        };
        let look = far.rain_look(&Wind::default());
        assert_eq!(
            (look.memory_reach, look.deep_reach),
            (*PUDDLE_DISTANCE.end(), *DEEP_SNOW_REACH.end())
        );
    }

    #[test]
    fn melting_snow_is_drawn_as_far_as_it_fell_and_the_memory_covers_it() {
        // the snow lying is drawn at its own distance in rain and in clear
        // weather too (it was twice the puddles': at the defaults 76 m, past
        // the memory's 60, a line where it stopped), never past the memory
        for (wet, snow) in [(38.0, 60.0), (10.0, 256.0), (200.0, 40.0), (256.0, 20.0), (128.0, 128.0)] {
            // snowing, raining, clear
            for (precipitation, amount) in [(Precipitation::Snow, 0.5), (Precipitation::Rain, 0.5), (Precipitation::Rain, 0.0)] {
                let w = WeatherSettings {
                    precipitation,
                    amount,
                    puddle_distance: wet,
                    snow_distance: snow,
                    touched: true,
                    ..WeatherSettings::default()
                };
                let look = w.rain_look(&Wind::default());
                assert_eq!(look.snow_distance, snow, "{precipitation:?}, {wet} / {snow} m");
                assert!(look.snow_distance <= look.memory_reach, "{precipitation:?}, {wet} / {snow} m");
            }
        }
    }

    #[test]
    fn snow_settles_off_hides_the_lying_snow_whatever_falls() {
        for precipitation in [Precipitation::Rain, Precipitation::Snow] {
            let off = WeatherSettings {
                precipitation,
                amount: 0.5,
                snow_settles: false,
                touched: true,
                ..WeatherSettings::default()
            };
            let look = off.rain_look(&Wind::default());
            assert!(!look.snow_settles, "{precipitation:?}");
            assert!(!look.footprints && !look.deep_snow);
            let on = WeatherSettings { snow_settles: true, ..off };
            assert!(on.rain_look(&Wind::default()).snow_settles, "{precipitation:?}");
        }
    }

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
    fn the_switches_turn_off_what_they_hold_and_keep_its_settings() {
        // off by default, at full strength once on
        let d = WeatherSettings::default();
        assert!(!d.enabled && !d.precipitation_on && !d.wind_on && !d.aurora);
        assert_eq!(d.amount, 1.0);
        let on = WeatherSettings {
            enabled: true,
            precipitation_on: true,
            wind_on: true,
            amount: 0.8,
            wind_min: 0.2,
            wind_max: 0.6,
            thunder: true,
            aurora: true,
            touched: true,
            ..Default::default()
        };
        assert_eq!(on.in_effect(), on);
        // the main switch: nothing at all, its settings kept
        let all_off = WeatherSettings {
            enabled: false,
            ..on.clone()
        };
        let drawn = all_off.in_effect();
        assert_eq!((drawn.fall(), drawn.wind_range(), drawn.thunder), (0.0, (0.0, 0.0), false));
        assert_eq!(drawn.aurora_look(), [0.0; 4]);
        assert_eq!(all_off.amount, 0.8);
        // a card's switch: its own part only
        let dry = WeatherSettings {
            precipitation_on: false,
            ..on.clone()
        }
        .in_effect();
        assert_eq!((dry.fall(), dry.thunder, dry.wind_range()), (0.0, false, (0.2, 0.6)));
        assert!(dry.aurora);
        let calm = WeatherSettings {
            wind_on: false,
            ..on.clone()
        }
        .in_effect();
        assert_eq!((calm.wind_range(), calm.fall()), ((0.0, 0.0), 0.8));
    }

    #[test]
    fn a_file_from_before_the_switches_keeps_what_it_drew() {
        let read = |json: &str| {
            let mut w: WeatherSettings = serde_json::from_str(json).expect("parse");
            w.upgrade_to_switches();
            w
        };
        let rain = read("{\"amount\": 0.4, \"wind_max\": 0.0, \"aurora\": false}");
        assert!(rain.enabled && rain.precipitation_on && !rain.wind_on);
        assert_eq!(rain.amount, 0.4);
        let night = read("{\"amount\": 0.0, \"wind_max\": 0.3, \"aurora\": true}");
        assert!(night.enabled && !night.precipitation_on && night.wind_on);
        // turned on later, it falls at full strength
        assert_eq!(night.amount, 1.0);
        let clear = read("{\"amount\": 0.0, \"wind_max\": 0.0, \"aurora\": false}");
        assert!(!clear.enabled && !clear.precipitation_on && !clear.wind_on);
        // "Let regions set the weather" off: its own weather was drawn
        let own = read("{\"amount\": 0.4, \"wind_max\": 0.3, \"aurora\": true, \"region_weather\": false}");
        assert!(own.enabled && own.precipitation_on && own.wind_on);
    }

    #[test]
    fn a_file_where_regions_set_the_weather_stays_off() {
        // the region decided, and where it set none the sky was clear: none
        // of these settings was drawn, so the main switch stays off and the
        // cards keep them for when it is turned on
        let mut w: WeatherSettings =
            serde_json::from_str("{\"amount\": 0.6, \"wind_max\": 0.3, \"aurora\": true, \"region_weather\": true}").expect("parse");
        w.upgrade_to_switches();
        assert!(!w.enabled && w.precipitation_on && w.wind_on && w.aurora);
        assert_eq!((w.amount, w.former_region_weather), (0.6, None));
        assert_eq!(w.in_effect().fall(), 0.0);
        // read once, never written again
        let saved = serde_json::to_value(&w).expect("serialize");
        assert!(saved.get("region_weather").is_none());
    }

    #[test]
    fn a_still_wind_switched_off_takes_the_breeze_and_a_set_one_is_kept() {
        let d = WeatherSettings::default();
        assert_eq!(d.wind_range(), (WIND_MIN_DEFAULT, WIND_MAX_DEFAULT));
        let read = |json: &str| {
            let mut w: WeatherSettings = serde_json::from_str(json).expect("parse");
            w.upgrade_wind_defaults();
            w
        };
        // still and off: switched on, it blows
        let still = read("{\"wind_on\": false, \"wind_min\": 0.0, \"wind_max\": 0.0}");
        assert_eq!(still.wind_range(), (WIND_MIN_DEFAULT, WIND_MAX_DEFAULT));
        // set by hand, or on: kept as it is
        let set = read("{\"wind_on\": false, \"wind_min\": 0.0, \"wind_max\": 0.6}");
        assert_eq!(set.wind_range(), (0.0, 0.6));
        let on = read("{\"wind_on\": true, \"wind_min\": 0.0, \"wind_max\": 0.0}");
        assert_eq!(on.wind_range(), (0.0, 0.0));
    }

    #[test]
    fn old_settings_files_keep_their_wind_as_the_gusts() {
        let read: WeatherSettings = serde_json::from_str("{\"wind\": 0.6}").expect("parse");
        // the lulls they had not are the default's
        assert_eq!(read.wind_range(), (WIND_MIN_DEFAULT, 0.6));
        assert!(read.wind_follow_region);
        assert_eq!(read.thunder_frequency, 0.5);
    }
}
