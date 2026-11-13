//! Region weather: the weather a region (or a parcel) sets through its
//! environment, for the viewers that let it (Preferences, "Let regions set
//! the weather"; off by default). Aurora's own: the Legacy Viewer has none.
//!
//! **Laid out as EEP is.** A sky is how one moment looks, with no time in
//! it; a day cycle is the timing: which skies at which times, on which
//! altitude tracks, the viewer blending between neighbouring keyframes. The
//! weather follows the same split. A sky carries the weather of its moment
//! ([`SkyWeather`]: what falls, how likely, how hard, the wind, thunder); the
//! day carries its schedule ([`DayWeather`]: how long each period lasts and
//! the seed). Between two keyframes the weather blends as their colours do,
//! and a sky with no weather is clear and calm at its keyframe, so a storm
//! sky at 18:00 and a plain one at 21:00 make a storm that dies away.
//! Altitude is EEP's own: each track's skies set the weather up there.
//!
//! **How it reaches every viewer.** An EEP setting's description is its
//! inventory item's: only who holds the item sees it, and it is never in
//! what ExtEnvironment sends. The names are (the day's, each frame's), so
//! the weather rides at the end of one: `~` and nine letters on a sky
//! (`Storm ~K7Q2M9XAP`; twelve with an aurora), `~` and five on the day
//! (`Island day ~3F0KZ`), written by Aurora's editors. Other viewers show
//! them as part of the name; Aurora shows the name without it, tagged
//! Aurora Weather.
//!
//! **How every viewer sees the same weather.** Nothing is sent while it
//! runs. The day's position is the clock's, shared by all; time is also
//! cut into periods (5 minutes to a day), and in each a hash of the seed
//! and the period's number decides whether it falls (against the chance at
//! that time of day) and how hard (between the lightest and the heaviest
//! then). The seed is the region's own (its id) unless the day gives one,
//! so that parcels and regions can share a sky or differ. The change
//! between two periods is eased over a few minutes, so clocks a few seconds
//! apart do not show.

use super::{AuroraColours, Precipitation, WeatherSettings};

/// Letters of the codes: Crockford's base 32 (no I, L, O, U: nothing to
/// misread), read in either case.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// A sky's code after its `~`: 32 bits of settings and a 13-bit check.
pub const SKY_LEN: usize = 9;
const SKY_DATA: u32 = 32;
const SKY_VERSION: u64 = 2;
/// A sky's code with an aurora: the same 30 bits of weather after its own
/// version, 13 bits of aurora, a 15-bit check. A sky without one keeps the
/// shorter code (and the skies saved before the aurora, theirs).
pub const SKY_AURORA_LEN: usize = 12;
const SKY_AURORA_DATA: u32 = 45;
const SKY_AURORA_VERSION: u64 = 3;
/// A day's code after its `~`: 16 bits and a 9-bit check.
pub const DAY_LEN: usize = 5;
const DAY_DATA: u32 = 16;
const DAY_VERSION: u64 = 3;
/// The periods a day can choose (minutes): its 4 bits index this.
pub const WINDOWS: [u16; 16] = [5, 10, 15, 20, 30, 45, 60, 90, 120, 180, 240, 360, 480, 720, 1080, 1440];
/// The period of a day that gives none: 30 minutes.
const DEFAULT_WINDOW: u8 = 4;
/// The most characters a name holds (inventory names).
pub const NAME_MAX: usize = 63;

/// The weather of a sky: what it is like at its keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyWeather {
    pub precipitation: Precipitation,
    /// The chance that it falls in a period (0 never, 1 always), in 31sts.
    pub chance: f32,
    /// How hard it falls when it does: between these (0..1, in 15ths).
    pub amount: (f32, f32),
    /// The wind's lulls and gusts (0..1, in 15ths).
    pub wind: (f32, f32),
    /// Where the wind comes from (degrees, one of sixteen points), or the
    /// region's own wind.
    pub wind_from: Option<f32>,
    /// Thunder and lightning when it rains hard enough.
    pub thunder: bool,
    /// The Strikes setting (0..1, in thirds).
    pub strikes: f32,
    /// The aurora in its night sky, if any.
    pub aurora: Option<AuroraSky>,
}

/// The aurora of a sky.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuroraSky {
    /// How bright (0..1, in 15ths).
    pub intensity: f32,
    /// Where it stands (degrees clockwise from north, one of sixteen points).
    pub heading: f32,
    /// How high its foot is seen (0..1, in 7ths).
    pub height: f32,
    pub colours: AuroraColours,
}

impl Default for AuroraSky {
    fn default() -> Self {
        Self {
            intensity: 9.0 / 15.0,
            heading: 0.0,
            height: 2.0 / 7.0,
            colours: AuroraColours::Green,
        }
    }
}

impl Default for SkyWeather {
    /// A new weather in the editor: showers half the time.
    fn default() -> Self {
        Self {
            precipitation: Precipitation::Rain,
            chance: 15.0 / 31.0,
            amount: (4.0 / 15.0, 10.0 / 15.0),
            wind: (2.0 / 15.0, 6.0 / 15.0),
            wind_from: None,
            thunder: false,
            strikes: 1.0 / 3.0,
            aurora: None,
        }
    }
}

impl SkyWeather {
    /// A sky with no weather: clear and calm.
    pub const CLEAR: SkyWeather = SkyWeather {
        precipitation: Precipitation::Rain,
        chance: 0.0,
        amount: (0.0, 0.0),
        wind: (0.0, 0.0),
        wind_from: None,
        thunder: false,
        strikes: 1.0 / 3.0,
        aurora: None,
    };

    /// The letters of the code (without its `~`): nine, or twelve with an
    /// aurora.
    pub fn code(&self) -> String {
        let mut b = Bits::default();
        b.put(if self.aurora.is_some() { SKY_AURORA_VERSION } else { SKY_VERSION }, 2);
        b.put(u64::from(self.precipitation == Precipitation::Snow), 1);
        b.put(level(self.chance, 31), 5);
        let (a0, a1) = ordered(self.amount);
        b.put(level(a0, 15), 4);
        b.put(level(a1, 15), 4);
        let (w0, w1) = ordered(self.wind);
        b.put(level(w0, 15), 4);
        b.put(level(w1, 15), 4);
        let from = self.wind_from.map_or(0, |d| 1 + ((d.rem_euclid(360.0) / 22.5).round() as u64 % 16));
        b.put(from, 5);
        b.put(u64::from(self.thunder), 1);
        b.put(level(self.strikes, 3), 2);
        debug_assert_eq!(b.at, SKY_DATA);
        let Some(a) = self.aurora else {
            return encode(b.v, SKY_DATA, SKY_LEN);
        };
        // (never 0: the aurora is there, if faint)
        b.put(level(a.intensity, 15).max(1), 4);
        b.put((a.heading.rem_euclid(360.0) / 22.5).round() as u64 % 16, 4);
        b.put(level(a.height, 7), 3);
        b.put(u64::from(a.colours.index()), 2);
        debug_assert_eq!(b.at, SKY_AURORA_DATA);
        encode(b.v, SKY_AURORA_DATA, SKY_AURORA_LEN)
    }

    /// The length of this sky's code.
    pub fn code_len(&self) -> usize {
        if self.aurora.is_some() { SKY_AURORA_LEN } else { SKY_LEN }
    }

    /// The weather of nine letters, if they are a sky's code this viewer
    /// reads and their check holds (a typo, or a name that only looks like
    /// a code, is refused).
    pub fn from_code(letters: &str) -> Option<SkyWeather> {
        let (data, version) = if letters.len() == SKY_AURORA_LEN {
            (SKY_AURORA_DATA, SKY_AURORA_VERSION)
        } else {
            (SKY_DATA, SKY_VERSION)
        };
        let len = if version == SKY_AURORA_VERSION { SKY_AURORA_LEN } else { SKY_LEN };
        let mut b = Bits {
            v: decode(letters, data, len)?,
            at: 0,
        };
        if b.take(2) != version {
            return None;
        }
        let snow = b.take(1) == 1;
        let chance = b.take(5) as f32 / 31.0;
        let amount = (b.take(4) as f32 / 15.0, b.take(4) as f32 / 15.0);
        let wind = (b.take(4) as f32 / 15.0, b.take(4) as f32 / 15.0);
        let from = b.take(5);
        let thunder = b.take(1) == 1;
        let strikes = b.take(2) as f32 / 3.0;
        let aurora = (version == SKY_AURORA_VERSION).then(|| AuroraSky {
            intensity: b.take(4) as f32 / 15.0,
            heading: b.take(4) as f32 * 22.5,
            height: b.take(3) as f32 / 7.0,
            colours: AuroraColours::from_index(b.take(2) as u8),
        });
        Some(SkyWeather {
            precipitation: if snow { Precipitation::Snow } else { Precipitation::Rain },
            chance,
            amount,
            wind,
            wind_from: (1..=16).contains(&from).then(|| (from - 1) as f32 * 22.5),
            thunder,
            strikes,
            aurora,
        })
    }

    /// The weather at the end of a sky's name, if it has one.
    pub fn from_name(name: &str) -> Option<SkyWeather> {
        Self::from_code(code_part(name)?)
    }

    /// `name` with this weather's code at its end, in place of any it had,
    /// shortened if it would not fit.
    pub fn name_with(&self, name: &str) -> String {
        with_code(name, &self.code())
    }

    /// The weather `t` of the way from `self` to `b` (0..1), as EEP blends
    /// two keyframes: the numbers in between; what falls, thunder and
    /// whether the wind has a direction from the nearer. An aurora fades in
    /// from a sky without one (and out to one), where it stands and its
    /// colours those of the sky that has it.
    pub fn blend(&self, b: &SkyWeather, t: f32) -> SkyWeather {
        let t = t.clamp(0.0, 1.0);
        let near = if t < 0.5 { self } else { b };
        let mix = |x: f32, y: f32| x + (y - x) * t;
        let wind_from = match (self.wind_from, b.wind_from) {
            (Some(x), Some(y)) => {
                // the short way round
                let d = (y - x + 540.0).rem_euclid(360.0) - 180.0;
                Some((x + d * t).rem_euclid(360.0))
            }
            _ => near.wind_from,
        };
        SkyWeather {
            precipitation: near.precipitation,
            chance: mix(self.chance, b.chance),
            amount: (mix(self.amount.0, b.amount.0), mix(self.amount.1, b.amount.1)),
            wind: (mix(self.wind.0, b.wind.0), mix(self.wind.1, b.wind.1)),
            wind_from,
            thunder: near.thunder,
            strikes: mix(self.strikes, b.strikes),
            aurora: blend_aurora(self.aurora, b.aurora, t),
        }
    }
}

/// The aurora `t` of the way between two keyframes' (see [`SkyWeather::blend`]).
fn blend_aurora(a: Option<AuroraSky>, b: Option<AuroraSky>, t: f32) -> Option<AuroraSky> {
    match (a, b) {
        (Some(x), Some(y)) => {
            let d = (y.heading - x.heading + 540.0).rem_euclid(360.0) - 180.0;
            Some(AuroraSky {
                intensity: lerp(x.intensity, y.intensity, t),
                heading: (x.heading + d * t).rem_euclid(360.0),
                height: lerp(x.height, y.height, t),
                colours: if t < 0.5 { x.colours } else { y.colours },
            })
        }
        (Some(x), None) => Some(AuroraSky {
            intensity: x.intensity * (1.0 - t),
            ..x
        }),
        (None, Some(y)) => Some(AuroraSky {
            intensity: y.intensity * t,
            ..y
        }),
        (None, None) => None,
    }
    .filter(|a| a.intensity > 0.002)
}

/// The weather schedule of a day: how long each period lasts and the seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayWeather {
    /// Index into [`WINDOWS`].
    pub window: u8,
    /// 0: the region's own seed; 1..1023: this one, shared by every parcel
    /// or region that gives it.
    pub seed: u16,
}

impl Default for DayWeather {
    fn default() -> Self {
        Self {
            window: DEFAULT_WINDOW,
            seed: 0,
        }
    }
}

impl DayWeather {
    /// A period's length (s).
    pub fn window_secs(&self) -> f64 {
        f64::from(WINDOWS[usize::from(self.window.min(15))]) * 60.0
    }

    pub fn code(&self) -> String {
        let mut b = Bits::default();
        b.put(DAY_VERSION, 2);
        b.put(u64::from(self.window.min(15)), 4);
        b.put(u64::from(self.seed.min(1023)), 10);
        encode(b.v, DAY_DATA, DAY_LEN)
    }

    pub fn from_code(letters: &str) -> Option<DayWeather> {
        let mut b = Bits {
            v: decode(letters, DAY_DATA, DAY_LEN)?,
            at: 0,
        };
        if b.take(2) != DAY_VERSION {
            return None;
        }
        let window = b.take(4) as u8;
        let seed = b.take(10) as u16;
        Some(DayWeather { window, seed })
    }

    /// The schedule at the end of a day's name, if it has one.
    pub fn from_name(name: &str) -> Option<DayWeather> {
        Self::from_code(code_part(name)?)
    }

    /// `name` with this schedule at its end; the default one leaves the
    /// name without a code.
    pub fn name_with(&self, name: &str) -> String {
        if *self == DayWeather::default() {
            return strip_code(name).to_owned();
        }
        with_code(name, &self.code())
    }

    /// The seed in a region of id `region`.
    pub fn seed_for(&self, region: uuid::Uuid) -> u64 {
        if self.seed == 0 {
            let (a, b) = region.as_u64_pair();
            mix(a ^ b.rotate_left(29))
        } else {
            mix(u64::from(self.seed).wrapping_mul(0xD1B5_4A32_D192_ED03))
        }
    }
}

/// The weather a day cycle sets on one sky track: its keyframes' weathers
/// (clear for a sky with none) and its schedule, read once from the day's
/// LLSD as ExtEnvironment sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionDay {
    /// (position 0..1, weather), by position.
    keys: Vec<(f32, SkyWeather)>,
    pub schedule: DayWeather,
    /// The day's length and offset (s), as the environment gives them.
    length: f64,
    offset: f64,
}

impl RegionDay {
    /// The weather of sky track `track` (0-based, the ground first; an empty
    /// track takes the next lower one's, as the sky drawn does:
    /// `DayCycle::track_for_altitude`, selectTrackNumber) of `day`, or None when
    /// none of its skies carries a weather (the region sets none there).
    pub fn read(day: &aurora_llsd::Llsd, track: usize, length: f64, offset: f64) -> Option<RegionDay> {
        let frames = &day["frames"];
        let keys_of = |t: usize| -> Vec<(f32, Option<SkyWeather>)> {
            // tracks: 0 the water, 1 the ground, 2.. the skies above
            day["tracks"]
                .at(t + 1)
                .as_array()
                .iter()
                .map(|key| {
                    let name = key["key_name"].as_str();
                    let pos = key["key_keyframe"].as_f32();
                    let pos = if pos.is_finite() { pos.clamp(0.0, 1.0) } else { 0.0 };
                    (
                        pos,
                        SkyWeather::from_name(frames[name]["name"].as_str()).or_else(|| SkyWeather::from_name(name)),
                    )
                })
                .collect()
        };
        let mut track = track;
        let mut keys = keys_of(track);
        while keys.is_empty() && track > 0 {
            track -= 1;
            keys = keys_of(track);
        }
        if keys.iter().all(|(_, w)| w.is_none()) {
            return None;
        }
        let mut keys: Vec<(f32, SkyWeather)> = keys.into_iter().map(|(p, w)| (p, w.unwrap_or(SkyWeather::CLEAR))).collect();
        keys.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(RegionDay {
            keys,
            schedule: DayWeather::from_name(day["name"].as_str()).unwrap_or_default(),
            length,
            offset,
        })
    }

    /// One sky's weather all day (the editor's preview).
    pub fn fixed(weather: SkyWeather) -> RegionDay {
        RegionDay {
            keys: vec![(0.0, weather)],
            schedule: DayWeather::default(),
            length: 14_400.0,
            offset: 0.0,
        }
    }

    /// The weather at day position `pos` (0..1): between the keyframes
    /// around it, looping over the day's end.
    pub fn state_at(&self, pos: f32) -> SkyWeather {
        let n = self.keys.len();
        if n == 1 {
            return self.keys[0].1;
        }
        let after = self.keys.iter().position(|(p, _)| *p > pos).unwrap_or(0);
        let before = (after + n - 1) % n;
        let (pa, a) = self.keys[before];
        let (pb, b) = self.keys[after];
        let span = (pb - pa).rem_euclid(1.0);
        let into = (pos - pa).rem_euclid(1.0);
        let t = if span > 1e-6 { into / span } else { 0.0 };
        a.blend(&b, t)
    }

    /// The weather at `secs` (Unix time): the day's position, the
    /// environment's way (eep::cycle_position).
    pub fn state(&self, secs: f64) -> SkyWeather {
        self.state_at(crate::world::eep::cycle_position(secs, self.length, self.offset))
    }

    /// How hard it falls in period `w` at `secs`: whether it falls is
    /// decided once for the period (with the chance at its start), how hard
    /// within the amounts of the moment.
    fn period_fall(&self, seed: u64, w: i64, secs: f64) -> f32 {
        let start = w as f64 * self.schedule.window_secs();
        let h = mix(seed ^ (w as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        if unit(h) >= self.state(start).chance {
            return 0.0;
        }
        let (lo, hi) = ordered(self.state(secs).amount);
        // a little for the weakest, never nothing once it falls
        (lo + (hi - lo) * unit(mix(h ^ 0xA5A5))).max(0.05)
    }

    /// How hard it falls at `secs` on `seed`: the period's, eased into the
    /// next over a fifth of a period (at most five minutes) around their
    /// boundary.
    pub fn fall_at(&self, seed: u64, secs: f64) -> f32 {
        let window = self.schedule.window_secs();
        let ramp = (window * 0.2).min(300.0);
        let w = (secs / window).floor();
        let into = secs - w * window;
        let w = w as i64;
        if into < ramp * 0.5 {
            let k = smoothstep((into + ramp * 0.5) / ramp);
            lerp(self.period_fall(seed, w - 1, secs), self.period_fall(seed, w, secs), k)
        } else if window - into < ramp * 0.5 {
            let k = smoothstep((into - (window - ramp * 0.5)) / ramp);
            lerp(self.period_fall(seed, w, secs), self.period_fall(seed, w + 1, secs), k)
        } else {
            self.period_fall(seed, w, secs)
        }
    }

    /// How deep snow lies in the open at `secs` (0..1) in a region of id
    /// `region`: the last six hours of this weather played again, a minute
    /// at a time, as the renderer builds and melts it (about ten minutes to
    /// a full cover at full snow, fifteen to melt, two in rain). The same
    /// for every viewer: who comes in after it has snowed a while sees
    /// what has built up.
    pub fn snow_lying(&self, region: uuid::Uuid, secs: f64) -> f32 {
        const STEP: f64 = 60.0;
        const SPAN: f64 = 6.0 * 3600.0;
        let seed = self.schedule.seed_for(region);
        let mut depth = 0.0f32;
        let mut t = (secs - SPAN).floor();
        while t < secs {
            let fall = self.fall_at(seed, t);
            let dt = STEP.min(secs - t) as f32;
            if fall > 0.0 && self.state(t).precipitation == Precipitation::Snow {
                depth += (1.0 - depth) * (1.0 - (-dt * fall / 600.0).exp());
            } else {
                let melt = if fall > 0.0 { 120.0 } else { 900.0 };
                depth *= (-dt / melt).exp();
            }
            t += STEP;
        }
        depth
    }

    /// The viewer's weather at `secs` in a region of id `region`: what the
    /// day sets, over the viewer's own settings (the quality, the puddles
    /// and how far they are drawn stay the viewer's). `falling` forces it to
    /// fall as hard as it can then (the editor's preview).
    pub fn apply(&self, own: &WeatherSettings, region: uuid::Uuid, secs: f64, falling: bool) -> WeatherSettings {
        let now = self.state(secs);
        let amount = if falling {
            ordered(now.amount).1.max(0.05)
        } else {
            self.fall_at(self.schedule.seed_for(region), secs)
        };
        let (w0, w1) = ordered(now.wind);
        WeatherSettings {
            precipitation: now.precipitation,
            amount,
            wind_min: w0,
            wind_max: w1,
            wind_follow_region: now.wind_from.is_none(),
            wind_from: now.wind_from.unwrap_or(own.wind_from),
            thunder: now.thunder,
            thunder_frequency: now.strikes,
            // the region's night sky, with or without an aurora
            aurora: now.aurora.is_some(),
            aurora_intensity: now.aurora.map_or(own.aurora_intensity, |a| a.intensity),
            aurora_heading: now.aurora.map_or(own.aurora_heading, |a| a.heading),
            aurora_height: now.aurora.map_or(own.aurora_height, |a| a.height),
            aurora_colours: now.aurora.map_or(own.aurora_colours, |a| a.colours),
            // what falls is the region's, not AURORA_RAIN's
            touched: true,
            ..own.clone()
        }
    }
}

/// The Aurora Weather of a settings asset's map: a sky's (all day), or a
/// day's ground track (with the default day length: applied to myself).
pub fn of_settings(map: &aurora_llsd::Llsd) -> Option<RegionDay> {
    match map["type"].as_str() {
        "daycycle" => RegionDay::read(map, 0, crate::world::eep::DEFAULT_DAY_LENGTH, crate::world::eep::DEFAULT_DAY_OFFSET),
        "sky" => SkyWeather::from_name(map["name"].as_str()).map(RegionDay::fixed),
        _ => None,
    }
}

/// The letters after the last `~` of a name, trimmed.
fn code_part(name: &str) -> Option<&str> {
    let trimmed = name.trim_end();
    let at = trimmed.rfind('~')?;
    Some(trimmed[at + 1..].trim())
}

/// `name` without a weather code (a sky's or a day's) at its end.
pub fn strip_code(name: &str) -> &str {
    let trimmed = name.trim_end();
    match trimmed.rfind('~') {
        Some(at) => {
            let letters = trimmed[at + 1..].trim();
            if SkyWeather::from_code(letters).is_some() || DayWeather::from_code(letters).is_some() {
                trimmed[..at].trim_end()
            } else {
                trimmed
            }
        }
        None => trimmed,
    }
}

/// The name without its Aurora Weather code (a sky's or a day's), when it
/// ends in one: how Aurora's inventory shows it, tagged Aurora Weather.
pub fn coded_name(name: &str) -> Option<&str> {
    let base = strip_code(name);
    (base.len() < name.trim_end().len()).then_some(base)
}

/// `name` with ` ~code` at its end, in place of any code; the name is cut
/// to keep the whole within [`NAME_MAX`].
fn with_code(name: &str, code: &str) -> String {
    let room = NAME_MAX - 2 - code.len();
    let base: String = strip_code(name).chars().take(room).collect();
    format!("{} ~{code}", base.trim_end())
}

/// Whether `name` (without its code) is too long to take a code of `len`.
pub fn too_long_for(name: &str, len: usize) -> bool {
    strip_code(name).chars().count() > NAME_MAX - 2 - len
}

fn ordered(v: (f32, f32)) -> (f32, f32) {
    (v.0.min(v.1), v.0.max(v.1))
}

/// `v` (0..1) as a level of `steps` (0..=steps).
fn level(v: f32, steps: u64) -> u64 {
    (v.clamp(0.0, 1.0) * steps as f32).round() as u64
}

/// `data` (`bits` wide) and its check as `len` letters, the high ones first.
fn encode(data: u64, bits: u32, len: usize) -> String {
    let all = data | (check(data, len as u32 * 5 - bits) << bits);
    (0..len).rev().map(|i| ALPHABET[((all >> (i * 5)) & 31) as usize] as char).collect()
}

/// The data of `len` letters (`bits` wide), if their check holds.
fn decode(letters: &str, bits: u32, len: usize) -> Option<u64> {
    if letters.len() != len {
        return None;
    }
    let mut all = 0u64;
    for c in letters.bytes() {
        let c = c.to_ascii_uppercase();
        all = (all << 5) | ALPHABET.iter().position(|&a| a == c)? as u64;
    }
    let data = all & ((1 << bits) - 1);
    (all >> bits == check(data, len as u32 * 5 - bits)).then_some(data)
}

fn check(data: u64, width: u32) -> u64 {
    data.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - width)
}

/// A bit field writer and reader, low bits first.
#[derive(Default)]
struct Bits {
    v: u64,
    at: u32,
}

impl Bits {
    fn put(&mut self, value: u64, width: u32) {
        self.v |= (value & ((1 << width) - 1)) << self.at;
        self.at += width;
    }
    fn take(&mut self, width: u32) -> u64 {
        let out = (self.v >> self.at) & ((1 << width) - 1);
        self.at += width;
        out
    }
}

/// SplitMix64's finaliser: a good spread of every input bit.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 0..1 from a hash.
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn smoothstep(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::{Llsd, llsd_map};

    fn storm() -> SkyWeather {
        SkyWeather {
            precipitation: Precipitation::Snow,
            chance: 1.0,
            amount: (9.0 / 15.0, 14.0 / 15.0),
            wind: (5.0 / 15.0, 12.0 / 15.0),
            wind_from: Some(292.5),
            thunder: true,
            strikes: 2.0 / 3.0,
            aurora: None,
        }
    }

    fn lights() -> SkyWeather {
        SkyWeather {
            aurora: Some(AuroraSky {
                intensity: 12.0 / 15.0,
                heading: 337.5,
                height: 3.0 / 7.0,
                colours: AuroraColours::Violet,
            }),
            ..SkyWeather::CLEAR
        }
    }

    #[test]
    fn a_coded_name_shows_without_its_code_and_a_plain_tilde_stays() {
        assert_eq!(coded_name(&storm().name_with("Storm")), Some("Storm"));
        assert_eq!(
            coded_name(&DayWeather { window: 7, seed: 1 }.name_with("Island day")),
            Some("Island day")
        );
        assert_eq!(coded_name("Beach ~ sunset"), None);
        assert_eq!(coded_name("Plain sky"), None);
    }

    #[test]
    fn an_aurora_lengthens_the_code_and_a_sky_without_keeps_the_old_one() {
        let w = lights();
        let code = w.code();
        assert_eq!(code.len(), SKY_AURORA_LEN);
        assert_eq!(w.code_len(), SKY_AURORA_LEN);
        assert_eq!(SkyWeather::from_code(&code), Some(w), "{code}");
        assert_eq!(SkyWeather::from_name(&w.name_with("Polar night")), Some(w));
        assert_eq!(strip_code(&w.name_with("Polar night")), "Polar night");
        // the skies coded before the aurora read as they did
        assert_eq!(storm().code().len(), SKY_LEN);
        assert_eq!(SkyWeather::from_code(&storm().code()), Some(storm()));
        // a faint aurora is still one
        let faint = SkyWeather {
            aurora: Some(AuroraSky {
                intensity: 0.0,
                ..AuroraSky::default()
            }),
            ..SkyWeather::CLEAR
        };
        assert!(SkyWeather::from_code(&faint.code()).is_some_and(|f| f.aurora.is_some()));
        // a typo in the aurora's letters is caught
        let mut bad = code.into_bytes();
        bad[1] = if bad[1] == b'0' { b'1' } else { b'0' };
        assert!(SkyWeather::from_code(std::str::from_utf8(&bad).expect("letters")).is_none());
    }

    #[test]
    fn an_aurora_fades_in_and_out_between_keyframes_and_turns_the_short_way() {
        let a = lights();
        let half = SkyWeather::CLEAR.blend(&a, 0.5).aurora.expect("half way in");
        assert!((half.intensity - 6.0 / 15.0).abs() < 1e-5);
        assert_eq!(half.heading, 337.5);
        assert!(SkyWeather::CLEAR.blend(&a, 0.0).aurora.is_none());
        assert!(a.blend(&SkyWeather::CLEAR, 1.0).aurora.is_none());
        let east = SkyWeather {
            aurora: Some(AuroraSky {
                heading: 22.5,
                ..lights().aurora.expect("an aurora")
            }),
            ..SkyWeather::CLEAR
        };
        let mid = a.blend(&east, 0.5).aurora.expect("both");
        assert!(
            (mid.heading - 0.0).abs() < 1e-3 || (mid.heading - 360.0).abs() < 1e-3,
            "{}",
            mid.heading
        );
        // the region's aurora is the viewer's while it sets the weather
        let applied = RegionDay::fixed(a).apply(&WeatherSettings::default(), uuid::Uuid::nil(), 0.0, false);
        assert!(applied.aurora && applied.aurora_colours == AuroraColours::Violet);
        let none = RegionDay::fixed(storm()).apply(
            &WeatherSettings {
                aurora: true,
                ..Default::default()
            },
            uuid::Uuid::nil(),
            0.0,
            false,
        );
        assert!(!none.aurora);
    }

    fn day(name: &str, keys: &[(f32, &str, &str)]) -> Llsd {
        let mut frames = Llsd::new_map();
        let mut track = Vec::new();
        for (pos, key, sky) in keys {
            frames.insert(*key, llsd_map! {"type" => "sky", "name" => *sky});
            track.push(llsd_map! {"key_keyframe" => f64::from(*pos), "key_name" => *key});
        }
        llsd_map! {
            "name" => name,
            "frames" => frames,
            "tracks" => Llsd::Array(vec![Llsd::Array(vec![]), Llsd::Array(track)]),
        }
    }

    #[test]
    fn codes_are_plain_letters_and_read_back_the_same() {
        for w in [SkyWeather::default(), storm(), SkyWeather::CLEAR] {
            let code = w.code();
            assert_eq!(code.len(), SKY_LEN);
            assert!(code.bytes().all(|c| ALPHABET.contains(&c)), "{code}");
            assert_eq!(SkyWeather::from_code(&code), Some(w), "{code}");
            assert_eq!(SkyWeather::from_code(&code.to_lowercase()), Some(w));
        }
        let d = DayWeather { window: 7, seed: 777 };
        assert_eq!(d.code().len(), DAY_LEN);
        assert_eq!(DayWeather::from_code(&d.code()), Some(d));
        // a sky's code is not a day's, nor the other way round
        assert!(DayWeather::from_code(&storm().code()).is_none());
        assert!(SkyWeather::from_code(&d.code()).is_none());
    }

    #[test]
    fn a_typo_or_a_name_that_only_looks_like_a_code_is_refused() {
        let code = storm().code();
        let mut refused = 0;
        for i in 0..SKY_LEN {
            for &c in ALPHABET {
                if code.as_bytes()[i] == c {
                    continue;
                }
                let mut typo = code.clone().into_bytes();
                typo[i] = c;
                if SkyWeather::from_code(std::str::from_utf8(&typo).expect("ascii")).is_none() {
                    refused += 1;
                }
            }
        }
        // a 13-bit check: about one typo in 8000 slips through
        assert!(refused >= SKY_LEN * 31 - 1, "{refused} of {} typos caught", SKY_LEN * 31);
        assert!(SkyWeather::from_name("HELLOWORL").is_none());
        assert!(SkyWeather::from_name("Sunset ~ by the sea").is_none());
    }

    #[test]
    fn the_code_rides_at_the_end_of_a_name_and_a_long_name_is_cut_to_fit() {
        let named = storm().name_with("Storm");
        assert_eq!(named, format!("Storm ~{}", storm().code()));
        assert_eq!(SkyWeather::from_name(&named), Some(storm()));
        assert_eq!(strip_code(&SkyWeather::default().name_with(&named)), "Storm");
        assert_eq!(strip_code("Sky ~ blue"), "Sky ~ blue");
        let long = "x".repeat(70);
        assert!(too_long_for(&long, SKY_LEN));
        let fitted = storm().name_with(&long);
        assert_eq!(fitted.chars().count(), NAME_MAX);
        assert_eq!(SkyWeather::from_name(&fitted), Some(storm()));
        // the default schedule writes no code, and takes one away
        let d = DayWeather { window: 2, seed: 5 };
        assert_eq!(DayWeather::from_name(&d.name_with("Day")), Some(d));
        assert_eq!(DayWeather::default().name_with(&d.name_with("Day")), "Day");
    }

    #[test]
    fn the_weather_blends_between_keyframes_and_a_sky_without_one_is_clear() {
        let s = storm();
        let d = day("Day", &[(0.25, "dawn", "Dawn"), (0.75, "storm", &s.name_with("Storm"))]);
        let r = RegionDay::read(&d, 0, 14_400.0, 0.0).expect("weather");
        assert_eq!(r.state_at(0.75), s);
        assert_eq!(r.state_at(0.25).chance, 0.0);
        // halfway between dawn and the storm: half the chance
        let mid = r.state_at(0.5);
        assert!((mid.chance - 0.5).abs() < 1e-4, "{mid:?}");
        assert!((mid.amount.1 - s.amount.1 * 0.5).abs() < 1e-4);
        // over midnight, from the storm back to dawn
        let late = r.state_at(0.0);
        assert!((late.chance - 0.5).abs() < 1e-4, "{late:?}");
        // a day with no weather at all: the region sets none
        assert!(RegionDay::read(&day("Day", &[(0.0, "a", "Plain")]), 0, 14_400.0, 0.0).is_none());
        // an empty upper track takes the ground's
        assert!(RegionDay::read(&d, 2, 14_400.0, 0.0).is_some());
        // ... or the next lower one's, as the sky drawn does: track 3 empty,
        // track 2 calm, the ground stormy (review of #2410)
        let key = |name: &str| Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => name}]);
        let tiered = llsd_map! {
            "name" => "Day",
            "frames" => llsd_map! {
                "storm" => llsd_map! {"type" => "sky", "name" => s.name_with("Storm")},
                "calm" => llsd_map! {"type" => "sky", "name" => SkyWeather::CLEAR.name_with("Calm")},
            },
            "tracks" => Llsd::Array(vec![Llsd::Array(vec![]), key("storm"), key("calm"), Llsd::Array(vec![])]),
        };
        assert_eq!(
            RegionDay::read(&tiered, 2, 14_400.0, 0.0).expect("calm").state_at(0.0),
            SkyWeather::CLEAR
        );
        assert_eq!(RegionDay::read(&tiered, 0, 14_400.0, 0.0).expect("storm").state_at(0.0), s);
    }

    #[test]
    fn every_viewer_sees_the_same_weather_and_the_seed_shares_or_parts_it() {
        let w = SkyWeather { chance: 0.5, ..storm() };
        let named = day(
            &DayWeather { window: 0, seed: 0 }.name_with("Day"),
            &[(0.0, "s", &w.name_with("S"))],
        );
        let r = RegionDay::read(&named, 0, 14_400.0, 0.0).expect("weather");
        assert_eq!(r.schedule.window, 0);
        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        let t0 = 1_800_000_000.0;
        let falls = |region, t| r.fall_at(r.schedule.seed_for(region), t);
        let differ = (0..300)
            .filter(|i| falls(a, t0 + f64::from(*i) * 300.0) != falls(b, t0 + f64::from(*i) * 300.0))
            .count();
        assert!(differ > 50, "each region its own weather on seed 0: {differ}");
        let shared = DayWeather { window: 0, seed: 9 };
        assert_eq!(shared.seed_for(a), shared.seed_for(b));
    }

    #[test]
    fn it_falls_about_as_often_as_the_chance_and_never_jumps() {
        let w = SkyWeather {
            chance: 0.4,
            ..SkyWeather::default()
        };
        let r = RegionDay {
            schedule: DayWeather { window: 0, seed: 3 },
            ..RegionDay::fixed(w)
        };
        let seed = r.schedule.seed_for(uuid::Uuid::nil());
        let period = r.schedule.window_secs();
        let t0 = 1_800_000_000.0;
        let wet = (0..4000)
            .filter(|i| r.fall_at(seed, t0 + (f64::from(*i) + 0.5) * period) > 0.0)
            .count();
        assert!((1400..=1800).contains(&wet), "{wet} wet periods of 4000 at 40%");
        let mut last = r.fall_at(seed, t0);
        for s in 1..20_000 {
            let now = r.fall_at(seed, t0 + f64::from(s));
            assert!((now - last).abs() < 0.02, "{last} -> {now} at {s} s");
            last = now;
        }
        let clear = RegionDay::fixed(SkyWeather::CLEAR);
        assert!((0..100).all(|i| clear.fall_at(seed, t0 + f64::from(i) * 100.0) == 0.0));
    }

    #[test]
    fn snow_that_has_fallen_a_while_lies_for_who_comes_in_late_and_melts_after() {
        let region = uuid::Uuid::from_u128(5);
        let always = RegionDay::fixed(SkyWeather {
            precipitation: Precipitation::Snow,
            chance: 1.0,
            amount: (1.0, 1.0),
            ..SkyWeather::default()
        });
        let t = 1_800_000_000.0;
        // six hours of heavy snow: a full cover
        assert!(always.snow_lying(region, t) > 0.95);
        // the same for everyone at the same moment
        assert_eq!(always.snow_lying(region, t), always.snow_lying(region, t));
        // never any: none
        assert_eq!(RegionDay::fixed(SkyWeather::CLEAR).snow_lying(region, t), 0.0);
        // rain never builds it
        let rain = RegionDay::fixed(SkyWeather {
            precipitation: Precipitation::Rain,
            ..*always.keys.first().map(|k| &k.1).expect("a key")
        });
        assert_eq!(rain.snow_lying(region, t), 0.0);
    }

    #[test]
    fn a_settings_asset_gives_its_weather_from_its_name() {
        let sky = llsd_map! {"type" => "sky", "name" => storm().name_with("Storm")};
        assert_eq!(of_settings(&sky), Some(RegionDay::fixed(storm())));
        assert_eq!(of_settings(&llsd_map! {"type" => "sky", "name" => "Plain"}), None);
        let mut d = day("Day", &[(0.0, "s", &storm().name_with("S"))]);
        d.insert("type", "daycycle");
        assert!(of_settings(&d).is_some_and(|r| r.state_at(0.3) == storm()));
        assert_eq!(of_settings(&llsd_map! {"type" => "water", "name" => storm().name_with("W")}), None);
    }

    #[test]
    fn the_region_sets_what_falls_and_the_wind_and_the_viewer_keeps_its_quality() {
        let own = WeatherSettings {
            quality: super::super::WeatherQuality::Low,
            puddle_distance: 20.0,
            ..WeatherSettings::default()
        };
        let r = RegionDay::fixed(storm());
        let now = r.apply(&own, uuid::Uuid::nil(), 1_800_000_000.0, false);
        assert_eq!(now.precipitation, Precipitation::Snow);
        assert!(now.amount >= storm().amount.0 - 1e-4, "it always falls at 100%");
        assert_eq!((now.wind_min, now.wind_max), storm().wind);
        assert!(!now.wind_follow_region && now.wind_from == 292.5);
        assert!(now.thunder);
        assert_eq!(now.quality, super::super::WeatherQuality::Low);
        assert_eq!(now.puddle_distance, 20.0);
        // the preview's "falling now": as hard as it can
        let never = RegionDay::fixed(SkyWeather { chance: 0.0, ..storm() });
        assert_eq!(never.apply(&own, uuid::Uuid::nil(), 0.0, true).amount, storm().amount.1);
    }
}
