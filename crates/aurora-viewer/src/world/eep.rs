//! Environment Enhancement Project (EEP) day cycles, fetched from the
//! region's (or parcel's) `ExtEnvironment` capability.
//!
//! Field names, defaults and track selection follow the Second Life viewer
//! (llinventory/llsettingssky.cpp, llsettingswater.cpp, llsettingsdaycycle.cpp,
//! newview/llenvironment.cpp), Copyright (C) Linden Research, Inc., originally LGPL 2.1.

use aurora_llsd::Llsd;
use glam::{Quat, Vec2, Vec3};
use uuid::Uuid;

/// Default cloud noise texture (LLSettingsSky DEFAULT_CLOUD_ID).
pub const DEFAULT_CLOUD_ID: Uuid = uuid::uuid!("1dc1368f-e8fe-f02d-a08d-9d9f11c1af6b");
/// Default moon texture.
pub const DEFAULT_MOON_ID: Uuid = uuid::uuid!("d07f6eed-b96a-47cd-b51d-400ad4a1c428");
/// Default water normal map (DEFAULT_WATER_NORMAL).
pub const DEFAULT_WATER_NORMAL: Uuid = uuid::uuid!("822ded49-9a6c-f61c-cb89-6df54f42cdf4");

#[derive(Debug, Clone, Copy)]
pub struct SkyFrame {
    pub sun_rotation: Quat,
    pub moon_rotation: Quat,
    pub sunlight: Vec3,
    pub ambient: Vec3,
    pub blue_horizon: Vec3,
    pub blue_density: Vec3,
    pub haze_density: f32,
    pub haze_horizon: f32,
    pub density_multiplier: f32,
    pub distance_multiplier: f32,
    pub max_y: f32,
    pub glow: Vec3,
    pub cloud_color: Vec3,
    pub cloud_pos_density1: Vec3,
    pub cloud_pos_density2: Vec3,
    pub cloud_scale: f32,
    pub cloud_scroll_rate: Vec2,
    pub cloud_shadow: f32,
    pub cloud_variance: f32,
    pub cloud_id: Uuid,
    pub sun_id: Uuid,
    pub moon_id: Uuid,
    pub sun_scale: f32,
    pub moon_scale: f32,
    pub moon_brightness: f32,
    pub star_brightness: f32,
    pub gamma: f32,
    pub dome_offset: f32,
    pub dome_radius: f32,
    pub probe_ambiance: f32,
    /// No reflection_probe_ambiance key: a "classic" sky (LLSettingsSky::canAutoAdjust).
    pub can_auto_adjust: bool,
}

impl Default for SkyFrame {
    /// LLSettingsSky::defaults() with the legacy haze defaults.
    fn default() -> Self {
        SkyFrame {
            sun_rotation: Quat::from_rotation_y(-std::f32::consts::FRAC_PI_4),
            moon_rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_4 * 3.0),
            sunlight: Vec3::new(0.7342, 0.7815, 0.8999),
            ambient: Vec3::splat(0.25),
            blue_horizon: Vec3::new(0.4954, 0.4954, 0.6399),
            blue_density: Vec3::new(0.2447, 0.4487, 0.7599),
            haze_density: 0.6999,
            haze_horizon: 0.1899,
            density_multiplier: 0.0001799,
            distance_multiplier: 0.8,
            max_y: 1605.0,
            glow: Vec3::new(5.0, 0.001, -0.4799),
            cloud_color: Vec3::splat(0.4099),
            cloud_pos_density1: Vec3::new(1.0, 0.526, 1.0),
            cloud_pos_density2: Vec3::new(1.0, 0.526, 1.0),
            cloud_scale: 0.4199,
            cloud_scroll_rate: Vec2::new(0.2, 0.01),
            cloud_shadow: 0.2699,
            cloud_variance: 0.0,
            cloud_id: DEFAULT_CLOUD_ID,
            sun_id: Uuid::nil(),
            moon_id: DEFAULT_MOON_ID,
            sun_scale: 1.0,
            moon_scale: 1.0,
            moon_brightness: 0.5,
            star_brightness: 250.0,
            gamma: 1.0,
            dome_offset: 0.96,
            dome_radius: 15000.0,
            probe_ambiance: 0.0,
            can_auto_adjust: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WaterFrame {
    pub fog_color: Vec3,
    pub fog_density: f32,
    pub underwater_fog_mod: f32,
    pub fresnel_scale: f32,
    pub fresnel_offset: f32,
    pub blur_multiplier: f32,
    pub normal_scale: Vec3,
    pub wave1: Vec2,
    pub wave2: Vec2,
    pub normal_map: Uuid,
}

impl Default for WaterFrame {
    /// LLSettingsWater::defaults().
    fn default() -> Self {
        WaterFrame {
            fog_color: Vec3::new(0.0156, 0.149, 0.2509),
            fog_density: 2.0,
            underwater_fog_mod: 0.25,
            fresnel_scale: 0.3999,
            fresnel_offset: 0.5,
            blur_multiplier: 0.04,
            normal_scale: Vec3::splat(2.0),
            wave1: Vec2::new(1.05, -0.42),
            wave2: Vec2::new(1.11, -1.16),
            normal_map: DEFAULT_WATER_NORMAL,
        }
    }
}

/// Firestorm's default day cycle asset (LLSettingsDay::DEFAULT_ASSET_ID): what
/// a Second Life region without a valid day cycle shows.
pub const DEFAULT_DAY_ASSET: Uuid = uuid::uuid!("5646d39e-d3d7-6aff-ed71-30fc87d64a91");
/// LLSettingsDay::DEFAULT_DAYLENGTH (4 hours).
pub const DEFAULT_DAY_LENGTH: f64 = 14400.0;
/// LLSettingsDay::DEFAULT_DAYOFFSET (+16 h, i.e. SLT).
pub const DEFAULT_DAY_OFFSET: f64 = 57600.0;
/// Parcel id of a region-wide answer (INVALID_PARCEL_ID).
pub const REGION_PARCEL_ID: i32 = -1;

#[derive(Debug, Clone, Default)]
pub struct DayCycle {
    /// Seconds per day cycle.
    pub length: f64,
    /// Seconds added to Unix time.
    pub offset: f64,
    /// Sky tracks 1..=4 (index 0 = ground level): (position 0..=1, frame), sorted.
    pub sky_tracks: [Vec<(f32, SkyFrame)>; 4],
    /// Water track.
    pub water: Vec<(f32, WaterFrame)>,
}

/// An ExtEnvironment answer (`result["environment"]`), port of
/// LLEnvironment::EnvironmentInfo::extract.
#[derive(Debug, Clone)]
pub struct EnvAnswer {
    /// Parcel the answer is about, [`REGION_PARCEL_ID`] for the whole region.
    pub parcel_id: i32,
    /// The day cycle, when the answer carries one (it may still be unusable,
    /// see [`DayCycle::is_valid`]).
    pub day: Option<DayCycle>,
    /// Altitudes (m) where sky tracks 1..=4 start: 0 and `track_altitudes`
    /// (all 0 when absent, as EnvironmentInfo's constructor leaves them).
    pub altitudes: [f32; 4],
}

/// One settings asset (LLSettingsBase "type": sky, water or daycycle).
#[derive(Debug, Clone)]
pub enum Settings {
    Sky(SkyFrame),
    Water(WaterFrame),
    Day(DayCycle),
}

fn vec3(v: &Llsd) -> Option<Vec3> {
    if v.len() < 3 {
        return None;
    }
    let r = Vec3::new(v.at(0).as_f32(), v.at(1).as_f32(), v.at(2).as_f32());
    r.is_finite().then_some(r)
}

fn vec2(v: &Llsd) -> Option<Vec2> {
    if v.len() < 2 {
        return None;
    }
    let r = Vec2::new(v.at(0).as_f32(), v.at(1).as_f32());
    r.is_finite().then_some(r)
}

fn quat(v: &Llsd) -> Option<Quat> {
    if v.len() < 4 {
        return None;
    }
    let q = Quat::from_xyzw(v.at(0).as_f32(), v.at(1).as_f32(), v.at(2).as_f32(), v.at(3).as_f32()).normalize();
    q.is_finite().then_some(q)
}

fn real(m: &Llsd, k: &str, d: f32) -> f32 {
    if m.has(k) {
        let v = m[k].as_f32();
        if v.is_finite() {
            return v;
        }
    }
    d
}

fn uuid(m: &Llsd, k: &str, d: Uuid) -> Uuid {
    if m.has(k) { m[k].as_uuid() } else { d }
}

/// A frame whose `type` names another kind of settings is refused
/// (LLSettingsDay::initialize); frames without a type are accepted.
fn type_is(m: &Llsd, kind: &str) -> bool {
    !m.has("type") || m["type"].as_str() == kind
}

fn sky_frame(s: &Llsd) -> Option<SkyFrame> {
    if !s.is_map() || !type_is(s, "sky") {
        return None;
    }
    let d = SkyFrame::default();
    // `legacy_haze` holds the classic atmospheric parameters in EEP skies;
    // `ambient` may live in either map.
    let haze = if s.has("legacy_haze") { &s["legacy_haze"] } else { s };
    let ambient = vec3(&haze["ambient"]).or_else(|| vec3(&s["ambient"])).unwrap_or(d.ambient);
    Some(SkyFrame {
        // missing values take LLSettingsSky::defaults() (settingValidation)
        sun_rotation: quat(&s["sun_rotation"]).unwrap_or(d.sun_rotation),
        moon_rotation: quat(&s["moon_rotation"]).unwrap_or(d.moon_rotation),
        sunlight: vec3(&s["sunlight_color"]).unwrap_or(d.sunlight),
        ambient,
        blue_horizon: vec3(&haze["blue_horizon"]).unwrap_or(d.blue_horizon),
        blue_density: vec3(&haze["blue_density"]).unwrap_or(d.blue_density),
        haze_density: real(haze, "haze_density", d.haze_density),
        haze_horizon: real(haze, "haze_horizon", d.haze_horizon),
        density_multiplier: real(haze, "density_multiplier", d.density_multiplier),
        distance_multiplier: real(haze, "distance_multiplier", d.distance_multiplier),
        max_y: real(s, "max_y", d.max_y),
        glow: vec3(&s["glow"]).unwrap_or(d.glow),
        cloud_color: vec3(&s["cloud_color"]).unwrap_or(d.cloud_color),
        cloud_pos_density1: vec3(&s["cloud_pos_density1"]).unwrap_or(d.cloud_pos_density1),
        cloud_pos_density2: vec3(&s["cloud_pos_density2"]).unwrap_or(d.cloud_pos_density2),
        cloud_scale: real(s, "cloud_scale", d.cloud_scale),
        cloud_scroll_rate: vec2(&s["cloud_scroll_rate"]).unwrap_or(d.cloud_scroll_rate),
        cloud_shadow: real(s, "cloud_shadow", d.cloud_shadow),
        cloud_variance: real(s, "cloud_variance", d.cloud_variance),
        cloud_id: uuid(s, "cloud_id", d.cloud_id),
        sun_id: uuid(s, "sun_id", d.sun_id),
        moon_id: uuid(s, "moon_id", d.moon_id),
        sun_scale: real(s, "sun_scale", d.sun_scale),
        moon_scale: real(s, "moon_scale", d.moon_scale),
        moon_brightness: real(s, "moon_brightness", d.moon_brightness),
        star_brightness: real(s, "star_brightness", d.star_brightness),
        gamma: real(s, "gamma", d.gamma),
        dome_offset: real(s, "dome_offset", d.dome_offset).clamp(0.0, 0.999),
        dome_radius: real(s, "dome_radius", d.dome_radius).clamp(1000.0, 2_000_000.0),
        probe_ambiance: real(s, "reflection_probe_ambiance", d.probe_ambiance),
        can_auto_adjust: !s.has("reflection_probe_ambiance"),
    })
}

fn water_frame(w: &Llsd) -> Option<WaterFrame> {
    let d = WaterFrame::default();
    if !w.is_map() || !type_is(w, "water") {
        return None;
    }
    Some(WaterFrame {
        fog_color: vec3(&w["water_fog_color"]).unwrap_or(d.fog_color),
        fog_density: real(w, "water_fog_density", d.fog_density),
        underwater_fog_mod: real(w, "underwater_fog_mod", d.underwater_fog_mod),
        fresnel_scale: real(w, "fresnel_scale", d.fresnel_scale),
        fresnel_offset: real(w, "fresnel_offset", d.fresnel_offset),
        blur_multiplier: real(w, "blur_multiplier", d.blur_multiplier),
        normal_scale: vec3(&w["normal_scale"]).unwrap_or(d.normal_scale),
        wave1: vec2(&w["wave1_direction"]).unwrap_or(d.wave1),
        wave2: vec2(&w["wave2_direction"]).unwrap_or(d.wave2),
        normal_map: uuid(w, "normal_map", d.normal_map),
    })
}

/// Distance from `begin` to `end` going forward around the day
/// (get_wrapping_distance, llenvironment.cpp): a whole day when equal.
fn wrapping_distance(begin: f32, end: f32) -> f32 {
    if begin < end {
        end - begin
    } else if begin > end {
        1.0 - (begin - end)
    } else {
        1.0
    }
}

/// Keyframes surrounding `pos` on a looping, sorted track, and the blend
/// factor (get_bounding_entries + convert_time_to_blend_factor).
fn around<T: Copy>(track: &[(f32, T)], pos: f32) -> Option<(T, T, f32)> {
    let (first, last) = (track.first()?, track.last()?);
    // the last key at or before `pos`, else the last key (wrap)
    let before = track.iter().rev().find(|(p, _)| *p <= pos).unwrap_or(last);
    // the first key after `pos`, else the first key (wrap)
    let after = track.iter().find(|(p, _)| *p > pos).unwrap_or(first);
    let span = wrapping_distance(before.0, after.0).max(1e-6);
    let pos = if pos < before.0 { pos + 1.0 } else { pos };
    let t = ((pos - before.0) / span).clamp(0.0, 1.0);
    Some((before.1, after.1, t))
}

/// Position in a day cycle (0..1) at a Unix time (LLEnvironment's
/// DayInstance: getAdjustedNow + convert_time_to_position).
pub fn cycle_position(unix_secs: f64, length: f64, offset: f64) -> f32 {
    let length = if length > 0.0 { length } else { DEFAULT_DAY_LENGTH };
    ((unix_secs + offset).rem_euclid(length) / length) as f32
}

/// Keyframe position as LLSettingsDay::initialize reads it: clamped to 0..=1
/// (a key at 1.0 stays at the end of the day).
fn key_position(key: &Llsd) -> f32 {
    let p = key["key_keyframe"].as_f32();
    if p.is_finite() { p.clamp(0.0, 1.0) } else { 0.0 }
}

/// Insert a keyframe; a later key at the same position replaces the earlier
/// one (LLSettingsDay's track is a map keyed by position).
fn insert_key<T>(track: &mut Vec<(f32, T)>, pos: f32, value: T) {
    match track.iter_mut().find(|(p, _)| *p == pos) {
        Some(k) => k.1 = value,
        None => track.push((pos, value)),
    }
}

impl DayCycle {
    /// Tracks of a day cycle settings map (`frames` + `tracks`): the
    /// `day_cycle` of an ExtEnvironment answer or a daycycle settings asset.
    /// Port of LLSettingsDay::initialize: track 0 is water, tracks 1..=4 are
    /// skies by altitude; keys naming a missing or mistyped frame are dropped.
    pub fn from_settings(day: &Llsd, length: f64, offset: f64) -> DayCycle {
        let frames = &day["frames"];
        let tracks = &day["tracks"];
        let mut out = DayCycle {
            length,
            offset,
            ..Default::default()
        };
        for key in tracks.at(0).as_array() {
            if let Some(fr) = water_frame(&frames[key["key_name"].as_str()]) {
                insert_key(&mut out.water, key_position(key), fr);
            }
        }
        for t in 0..4 {
            for key in tracks.at(t + 1).as_array() {
                if let Some(fr) = sky_frame(&frames[key["key_name"].as_str()]) {
                    insert_key(&mut out.sky_tracks[t], key_position(key), fr);
                }
            }
            out.sky_tracks[t].sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        out.water.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// A day with one sky and one water key (LLEnvironment's fixed
    /// environments: a sky or water settings item applied on its own).
    pub fn fixed(sky: SkyFrame, water: WaterFrame) -> DayCycle {
        DayCycle {
            length: DEFAULT_DAY_LENGTH,
            offset: DEFAULT_DAY_OFFSET,
            sky_tracks: [vec![(0.0, sky)], Vec::new(), Vec::new(), Vec::new()],
            water: vec![(0.0, water)],
        }
    }

    /// LLEnvironment::recordEnvironment: a day cycle is usable only with a
    /// water track and a ground-level sky track.
    pub fn is_valid(&self) -> bool {
        !self.water.is_empty() && !self.sky_tracks[0].is_empty()
    }

    /// Sky track (0-based) for an altitude, given the region's track
    /// altitudes (LLEnvironment::calculateSkyTrackForAltitude), falling back
    /// to the next lower non-empty track (selectTrackNumber).
    pub fn track_for_altitude(&self, altitudes: &[f32; 4], altitude: f32) -> usize {
        let idx = altitudes.iter().position(|a| altitude <= *a);
        let track = match idx {
            Some(0) => 1,
            None => 4,
            Some(i) => i.min(4),
        };
        let mut t = track - 1;
        while t > 0 && self.sky_tracks[t].is_empty() {
            t -= 1;
        }
        t
    }

    /// Interpolated sky of a track (0-based) at a cycle position.
    pub fn sky_at(&self, pos: f32, track: usize) -> SkyFrame {
        let track = &self.sky_tracks[track.min(3)];
        match around(track, pos) {
            Some((a, b, t)) => a.lerp(&b, t),
            None => SkyFrame::default(),
        }
    }

    pub fn water_at(&self, pos: f32) -> WaterFrame {
        match around(&self.water, pos) {
            Some((a, b, t)) => a.lerp(&b, t),
            None => WaterFrame::default(),
        }
    }
}

impl EnvAnswer {
    /// Parse `result["environment"]` from the ExtEnvironment capability.
    pub fn from_llsd(env: &Llsd) -> EnvAnswer {
        let mut altitudes = [0.0; 4];
        let alts = &env["track_altitudes"];
        for (i, a) in altitudes.iter_mut().skip(1).enumerate() {
            let v = alts.at(i).as_f32();
            *a = if v.is_finite() { v } else { 0.0 };
        }
        let day = env.has("day_cycle").then(|| {
            let length = match env["day_length"].as_i32() {
                n if n > 0 => n as f64,
                _ => DEFAULT_DAY_LENGTH,
            };
            // Firestorm reads a missing offset as INVALID_DAYOFFSET (-1 s)
            let offset = if env.has("day_offset") {
                env["day_offset"].as_i32() as f64
            } else {
                -1.0
            };
            DayCycle::from_settings(&env["day_cycle"], length, offset)
        });
        EnvAnswer {
            parcel_id: if env.has("parcel_id") {
                env["parcel_id"].as_i32()
            } else {
                REGION_PARCEL_ID
            },
            day,
            altitudes,
        }
    }

    /// The day cycle if it is usable (see [`DayCycle::is_valid`]).
    pub fn valid_day(self) -> Option<DayCycle> {
        self.day.filter(DayCycle::is_valid)
    }
}

impl Settings {
    /// Decode a settings asset as LLSettingsVOBase::onAssetDownloadComplete
    /// does (LLSDSerialize::deserialize: binary, XML or notation; assets are
    /// uploaded as notation) and build it by its type (createFromLLSD).
    pub fn from_asset(data: &[u8]) -> Option<Settings> {
        let start = data.iter().position(|c| !c.is_ascii_whitespace())?;
        let data = &data[start..];
        let lower: Vec<u8> = data.iter().take(24).map(u8::to_ascii_lowercase).collect();
        let v = if lower.starts_with(b"<?xml") || lower.starts_with(b"<llsd") {
            aurora_llsd::from_xml(data).ok()?
        } else if lower.starts_with(b"<?") {
            // "<? LLSD/Binary ?>" or "<? LLSD/Notation ?>" header
            let end = data.windows(2).position(|w| w == b"?>")?;
            let body = &data[end + 2..];
            let body = &body[body.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(body.len())..];
            let header: String = lower.iter().map(|&c| c as char).filter(|c| !c.is_whitespace()).collect();
            if header.starts_with("<?llsd/binary") {
                aurora_llsd::from_binary(body).ok()?.0
            } else {
                aurora_llsd::from_notation(body).ok()?
            }
        } else {
            aurora_llsd::from_notation(data).ok()?
        };
        Settings::from_llsd(&v)
    }

    pub fn from_llsd(v: &Llsd) -> Option<Settings> {
        match v["type"].as_str() {
            "sky" => sky_frame(v).map(Settings::Sky),
            "water" => water_frame(v).map(Settings::Water),
            // a settings item's day keeps the default length and offset
            // (LLEnvironment::setEnvironment with a "daycycle")
            "daycycle" => Some(Settings::Day(DayCycle::from_settings(v, DEFAULT_DAY_LENGTH, DEFAULT_DAY_OFFSET))),
            _ => None,
        }
    }

    /// As a day cycle: a sky or water item becomes a fixed day with the
    /// default for the other half.
    pub fn into_day(self) -> DayCycle {
        match self {
            Settings::Day(d) => d,
            Settings::Sky(s) => DayCycle::fixed(s, WaterFrame::default()),
            Settings::Water(w) => DayCycle::fixed(SkyFrame::default(), w),
        }
    }
}

impl SkyFrame {
    pub fn lerp(&self, b: &SkyFrame, t: f32) -> SkyFrame {
        let a = self;
        let l = |x: Vec3, y: Vec3| x.lerp(y, t);
        let lf = |x: f32, y: f32| x + (y - x) * t;
        let pick = |x: Uuid, y: Uuid| if t < 0.5 { x } else { y };
        SkyFrame {
            sun_rotation: a.sun_rotation.slerp(b.sun_rotation, t),
            moon_rotation: a.moon_rotation.slerp(b.moon_rotation, t),
            sunlight: l(a.sunlight, b.sunlight),
            ambient: l(a.ambient, b.ambient),
            blue_horizon: l(a.blue_horizon, b.blue_horizon),
            blue_density: l(a.blue_density, b.blue_density),
            haze_density: lf(a.haze_density, b.haze_density),
            haze_horizon: lf(a.haze_horizon, b.haze_horizon),
            density_multiplier: lf(a.density_multiplier, b.density_multiplier),
            distance_multiplier: lf(a.distance_multiplier, b.distance_multiplier),
            max_y: lf(a.max_y, b.max_y),
            glow: l(a.glow, b.glow),
            cloud_color: l(a.cloud_color, b.cloud_color),
            cloud_pos_density1: l(a.cloud_pos_density1, b.cloud_pos_density1),
            cloud_pos_density2: l(a.cloud_pos_density2, b.cloud_pos_density2),
            cloud_scale: lf(a.cloud_scale, b.cloud_scale),
            cloud_scroll_rate: a.cloud_scroll_rate.lerp(b.cloud_scroll_rate, t),
            cloud_shadow: lf(a.cloud_shadow, b.cloud_shadow),
            cloud_variance: lf(a.cloud_variance, b.cloud_variance),
            cloud_id: pick(a.cloud_id, b.cloud_id),
            sun_id: pick(a.sun_id, b.sun_id),
            moon_id: pick(a.moon_id, b.moon_id),
            sun_scale: lf(a.sun_scale, b.sun_scale),
            moon_scale: lf(a.moon_scale, b.moon_scale),
            moon_brightness: lf(a.moon_brightness, b.moon_brightness),
            star_brightness: lf(a.star_brightness, b.star_brightness),
            gamma: lf(a.gamma, b.gamma),
            dome_offset: lf(a.dome_offset, b.dome_offset),
            dome_radius: lf(a.dome_radius, b.dome_radius),
            probe_ambiance: lf(a.probe_ambiance, b.probe_ambiance),
            can_auto_adjust: a.can_auto_adjust && b.can_auto_adjust,
        }
    }

    /// Sun direction: +X rotated by the sun rotation (LLSettingsSky::getSunDirection).
    pub fn sun_direction(&self) -> Vec3 {
        (self.sun_rotation * Vec3::X).normalize_or(Vec3::Z)
    }

    pub fn moon_direction(&self) -> Vec3 {
        (self.moon_rotation * Vec3::X).normalize_or(Vec3::Z)
    }

    /// The default sky with the sun at a given direction (regions without EEP).
    pub fn with_sun(sun: Vec3) -> SkyFrame {
        let d = sun.normalize_or(Vec3::Z);
        let rot = Quat::from_rotation_arc(Vec3::X, d);
        SkyFrame {
            sun_rotation: rot,
            moon_rotation: Quat::from_rotation_arc(Vec3::X, -d),
            ..Default::default()
        }
    }
}

impl WaterFrame {
    pub fn lerp(&self, b: &WaterFrame, t: f32) -> WaterFrame {
        let a = self;
        let lf = |x: f32, y: f32| x + (y - x) * t;
        WaterFrame {
            fog_color: a.fog_color.lerp(b.fog_color, t),
            fog_density: lf(a.fog_density, b.fog_density),
            underwater_fog_mod: lf(a.underwater_fog_mod, b.underwater_fog_mod),
            fresnel_scale: lf(a.fresnel_scale, b.fresnel_scale),
            fresnel_offset: lf(a.fresnel_offset, b.fresnel_offset),
            blur_multiplier: lf(a.blur_multiplier, b.blur_multiplier),
            normal_scale: a.normal_scale.lerp(b.normal_scale, t),
            wave1: a.wave1.lerp(b.wave1, t),
            wave2: a.wave2.lerp(b.wave2, t),
            normal_map: if t < 0.5 { a.normal_map } else { b.normal_map },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    fn frame(q: [f64; 4]) -> Llsd {
        llsd_map! {
            "sun_rotation" => Llsd::Array(q.iter().map(|v| Llsd::Real(*v)).collect()),
            "sunlight_color" => Llsd::Array(vec![1.0.into(), 0.9.into(), 0.8.into()]),
            "legacy_haze" => llsd_map! {
                "haze_density" => 1.5,
            },
        }
    }

    fn env(extra_track: bool) -> Llsd {
        let mut frames = aurora_llsd::Map::new();
        frames.insert(
            "noon".into(),
            frame([0.0, -std::f64::consts::FRAC_1_SQRT_2, 0.0, std::f64::consts::FRAC_1_SQRT_2]),
        );
        frames.insert("dusk".into(), frame([0.0, 0.0, 0.0, 1.0]));
        frames.insert(
            "sea".into(),
            llsd_map! { "water_fog_density" => 3.0, "wave1_direction" => Llsd::Array(vec![0.5.into(), 0.25.into()]) },
        );
        let high = if extra_track {
            Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "dusk"}])
        } else {
            Llsd::Array(vec![])
        };
        llsd_map! {
            "day_length" => 14400,
            "day_offset" => 0,
            "track_altitudes" => Llsd::Array(vec![1000.0.into(), 2000.0.into(), 3000.0.into()]),
            "day_cycle" => llsd_map! {
                "frames" => Llsd::Map(frames),
                "tracks" => Llsd::Array(vec![
                    Llsd::Array(vec![llsd_map!{"key_keyframe" => 0.0, "key_name" => "sea"}]),
                    Llsd::Array(vec![
                        llsd_map!{"key_keyframe" => 0.0, "key_name" => "noon"},
                        llsd_map!{"key_keyframe" => 0.5, "key_name" => "dusk"},
                    ]),
                    high,
                ]),
            },
        }
    }

    fn day(env: &Llsd) -> DayCycle {
        EnvAnswer::from_llsd(env).valid_day().expect("valid day")
    }

    #[test]
    fn parses_and_interpolates() {
        let a = EnvAnswer::from_llsd(&env(false));
        assert_eq!(a.parcel_id, REGION_PARCEL_ID);
        assert_eq!(a.altitudes, [0.0, 1000.0, 2000.0, 3000.0]);
        let d = a.valid_day().expect("valid day");
        assert_eq!(d.sky_tracks[0].len(), 2);
        let noon = d.sky_at(0.0, 0).sun_direction();
        assert!(noon.z > 0.99, "noon sun overhead: {noon}");
        let mid = d.sky_at(0.25, 0).sun_direction();
        assert!(mid.z > 0.5 && mid.x > 0.5);
        assert!((cycle_position(7200.0, d.length, d.offset) - 0.5).abs() < 1e-5);
        assert!((d.sky_at(0.0, 0).haze_density - 1.5).abs() < 1e-6);
        let w = d.water_at(0.3);
        assert!((w.fog_density - 3.0).abs() < 1e-6);
        assert!((w.wave1.x - 0.5).abs() < 1e-6);
        assert_eq!(w.normal_map, DEFAULT_WATER_NORMAL);
    }

    #[test]
    fn altitude_tracks() {
        let alts = [0.0, 1000.0, 2000.0, 3000.0];
        let d = day(&env(true));
        assert_eq!(d.track_for_altitude(&alts, 20.0), 0);
        assert_eq!(d.track_for_altitude(&alts, 1500.0), 1);
        // track 3 is empty: falls back to track 2
        assert_eq!(d.track_for_altitude(&alts, 2500.0), 1);
        let d = day(&env(false));
        assert_eq!(d.track_for_altitude(&alts, 1500.0), 0);
        // no track_altitudes: all 0 as in Firestorm, the highest track with keys
        let d = day(&env(true));
        assert_eq!(d.track_for_altitude(&[0.0; 4], 20.0), 1);
    }

    #[test]
    fn answer_without_day_or_with_empty_tracks() {
        let a = EnvAnswer::from_llsd(&llsd_map! { "parcel_id" => 12 });
        assert_eq!(a.parcel_id, 12);
        assert!(a.day.is_none());
        // no water key: not a usable day (LLEnvironment::recordEnvironment)
        let mut e = env(false);
        if let Llsd::Map(m) = &mut e
            && let Some(Llsd::Map(dc)) = m.get_mut("day_cycle")
        {
            dc.insert(
                "tracks".into(),
                Llsd::Array(vec![
                    Llsd::Array(vec![]),
                    Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "noon"}]),
                ]),
            );
        }
        let a = EnvAnswer::from_llsd(&e);
        assert!(a.day.as_ref().is_some_and(|d| !d.is_valid()));
        assert!(a.valid_day().is_none());
    }

    #[test]
    fn key_at_one_stays_at_the_end_and_duplicates_keep_the_last() {
        let mut frames = aurora_llsd::Map::new();
        frames.insert("a".into(), frame([0.0, 0.0, 0.0, 1.0]));
        frames.insert(
            "b".into(),
            frame([0.0, -std::f64::consts::FRAC_1_SQRT_2, 0.0, std::f64::consts::FRAC_1_SQRT_2]),
        );
        frames.insert("sea".into(), llsd_map! { "type" => "water" });
        let dc = llsd_map! {
            "frames" => Llsd::Map(frames),
            "tracks" => Llsd::Array(vec![
                Llsd::Array(vec![llsd_map!{"key_keyframe" => 0.0, "key_name" => "sea"}]),
                Llsd::Array(vec![
                    llsd_map!{"key_keyframe" => 0.0, "key_name" => "a"},
                    llsd_map!{"key_keyframe" => 0.5, "key_name" => "a"},
                    llsd_map!{"key_keyframe" => 0.5, "key_name" => "b"},
                    llsd_map!{"key_keyframe" => 1.0, "key_name" => "b"},
                    llsd_map!{"key_keyframe" => 1.7, "key_name" => "b"},
                ]),
            ]),
        };
        let d = DayCycle::from_settings(&dc, 100.0, 0.0);
        let keys: Vec<f32> = d.sky_tracks[0].iter().map(|k| k.0).collect();
        // 1.0 is kept (not wrapped to 0), 1.7 clamps onto it, 0.5 keeps "b"
        assert_eq!(keys, vec![0.0, 0.5, 1.0]);
        assert!(d.sky_at(0.5, 0).sun_direction().z > 0.99);
        // from 0.5 ("b") to 1.0 ("b"): stays "b"; from 0.0 ("a") to 0.5 halfway
        assert!(d.sky_at(0.75, 0).sun_direction().z > 0.99);
        let q = d.sky_at(0.25, 0).sun_direction();
        assert!(q.z > 0.5 && q.x > 0.5, "{q}");
    }

    #[test]
    fn sky_without_sun_rotation_uses_the_default() {
        let s = sky_frame(&llsd_map! { "type" => "sky", "gamma" => 2.0 }).expect("sky");
        assert_eq!(s.sun_rotation, SkyFrame::default().sun_rotation);
        assert!((s.gamma - 2.0).abs() < 1e-6);
        // a water frame on a sky track is refused
        assert!(sky_frame(&llsd_map! { "type" => "water" }).is_none());
    }

    #[test]
    fn settings_assets_in_every_format() {
        let notation = b"{'type':'daycycle','frames':{'s':{'type':'sky','gamma':r1.5},'w':{'type':'water'}},
            'tracks':[[{'key_keyframe':r0,'key_name':'w'}],[{'key_keyframe':r0.25,'key_name':'s'}]]}";
        let Some(Settings::Day(d)) = Settings::from_asset(notation) else {
            panic!("notation day");
        };
        assert!(d.is_valid());
        assert_eq!((d.length, d.offset), (DEFAULT_DAY_LENGTH, DEFAULT_DAY_OFFSET));
        assert!((d.sky_at(0.9, 0).gamma - 1.5).abs() < 1e-6);
        let sky = llsd_map! { "type" => "sky", "gamma" => 0.5 };
        let mut bin = b"<? LLSD/Binary ?>\n".to_vec();
        bin.extend(aurora_llsd::to_binary(&sky));
        assert!(matches!(Settings::from_asset(&bin), Some(Settings::Sky(s)) if (s.gamma - 0.5).abs() < 1e-6));
        let water = aurora_llsd::to_xml(&llsd_map! { "type" => "water", "water_fog_density" => 4.0 });
        let Some(Settings::Water(w)) = Settings::from_asset(&water) else {
            panic!("xml water");
        };
        assert!((w.fog_density - 4.0).abs() < 1e-6);
        let fixed = Settings::Water(w).into_day();
        assert!(fixed.is_valid());
        assert!(Settings::from_asset(b"garbage").is_none());
    }
}
