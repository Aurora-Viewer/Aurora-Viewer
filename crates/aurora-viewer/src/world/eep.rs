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

#[derive(Debug, Clone, Default)]
pub struct DayCycle {
    /// Seconds per day cycle.
    pub length: f64,
    /// Seconds added to Unix time.
    pub offset: f64,
    /// Sky tracks 1..=4 (index 0 = ground level): (position 0..1, frame), sorted.
    pub sky_tracks: [Vec<(f32, SkyFrame)>; 4],
    /// Water track.
    pub water: Vec<(f32, WaterFrame)>,
    /// Altitudes (m) where sky tracks 2, 3 and 4 start; index 0 is 0.
    pub altitudes: [f32; 4],
    /// Parsed for completeness (LLSettingsBase "is_default"), not used yet.
    #[allow(dead_code)]
    pub is_default: bool,
    /// Parcel the environment belongs to (-1 = region).
    pub parcel_id: i32,
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

fn sky_frame(s: &Llsd) -> Option<SkyFrame> {
    let d = SkyFrame::default();
    // `legacy_haze` holds the classic atmospheric parameters in EEP skies;
    // `ambient` may live in either map.
    let haze = if s.has("legacy_haze") { &s["legacy_haze"] } else { s };
    let ambient = vec3(&haze["ambient"]).or_else(|| vec3(&s["ambient"])).unwrap_or(d.ambient);
    Some(SkyFrame {
        sun_rotation: quat(&s["sun_rotation"])?,
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
    if !w.is_map() {
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

/// Keyframes surrounding `pos` on a looping track, and the blend factor.
fn around<T: Copy>(track: &[(f32, T)], pos: f32) -> Option<(T, T, f32)> {
    let n = track.len();
    if n == 0 {
        return None;
    }
    if n == 1 {
        return Some((track[0].1, track[0].1, 0.0));
    }
    let i1 = track.iter().position(|(p, _)| *p > pos).unwrap_or(0);
    let i0 = if i1 == 0 { n - 1 } else { i1 - 1 };
    let (p0, a) = track[i0];
    let (p1, b) = track[i1];
    let span = (p1 - p0).rem_euclid(1.0).max(1e-4);
    let t = ((pos - p0).rem_euclid(1.0) / span).clamp(0.0, 1.0);
    Some((a, b, t))
}

impl DayCycle {
    /// Parse `result["environment"]` from the ExtEnvironment capability.
    pub fn from_llsd(env: &Llsd) -> Option<DayCycle> {
        let day = &env["day_cycle"];
        let frames = &day["frames"];
        let tracks = &day["tracks"];
        let mut out = DayCycle {
            length: match env["day_length"].as_i32() {
                n if n > 0 => n as f64,
                _ => 14400.0,
            },
            offset: env["day_offset"].as_i32() as f64,
            is_default: env["is_default"].as_bool(),
            parcel_id: if env.has("parcel_id") { env["parcel_id"].as_i32() } else { -1 },
            ..Default::default()
        };
        // tracks[0] is water; tracks[1..=4] are skies by altitude
        for key in tracks.at(0).as_array() {
            let pos = key["key_keyframe"].as_f32().rem_euclid(1.0);
            if let Some(fr) = water_frame(&frames[key["key_name"].as_str()]) {
                out.water.push((pos, fr));
            }
        }
        for t in 0..4 {
            for key in tracks.at(t + 1).as_array() {
                let pos = key["key_keyframe"].as_f32().rem_euclid(1.0);
                if let Some(fr) = sky_frame(&frames[key["key_name"].as_str()]) {
                    out.sky_tracks[t].push((pos, fr));
                }
            }
            out.sky_tracks[t].sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        out.water.sort_by(|a, b| a.0.total_cmp(&b.0));
        if out.sky_tracks[0].is_empty() {
            return None;
        }
        // track_altitudes: start of tracks 2..4
        let alts = &env["track_altitudes"];
        for i in 0..3 {
            out.altitudes[i + 1] = if alts.len() > i { alts.at(i).as_f32() } else { 10001.0 + i as f32 };
        }
        Some(out)
    }

    /// Position in the cycle (0..1) now.
    pub fn position(&self, unix_secs: f64) -> f32 {
        ((unix_secs + self.offset).rem_euclid(self.length) / self.length) as f32
    }

    /// Sky track (0-based) for an altitude (LLEnvironment::calculateSkyTrackForAltitude),
    /// falling back to the next lower non-empty track.
    pub fn track_for_altitude(&self, altitude: f32) -> usize {
        let idx = self.altitudes.iter().position(|a| altitude <= *a);
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

    /// Interpolated sky at a cycle position and altitude.
    pub fn sky_at(&self, pos: f32, altitude: f32) -> SkyFrame {
        let track = &self.sky_tracks[self.track_for_altitude(altitude)];
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

    #[test]
    fn parses_and_interpolates() {
        let d = DayCycle::from_llsd(&env(false)).unwrap();
        assert_eq!(d.sky_tracks[0].len(), 2);
        let noon = d.sky_at(0.0, 0.0).sun_direction();
        assert!(noon.z > 0.99, "noon sun overhead: {noon}");
        let mid = d.sky_at(0.25, 0.0).sun_direction();
        assert!(mid.z > 0.5 && mid.x > 0.5);
        assert!((d.position(7200.0) - 0.5).abs() < 1e-5);
        assert!((d.sky_at(0.0, 0.0).haze_density - 1.5).abs() < 1e-6);
        let w = d.water_at(0.3);
        assert!((w.fog_density - 3.0).abs() < 1e-6);
        assert!((w.wave1.x - 0.5).abs() < 1e-6);
        assert_eq!(w.normal_map, DEFAULT_WATER_NORMAL);
    }

    #[test]
    fn altitude_tracks() {
        let d = DayCycle::from_llsd(&env(true)).unwrap();
        assert_eq!(d.track_for_altitude(20.0), 0);
        assert_eq!(d.track_for_altitude(1500.0), 1);
        // track 3 is empty: falls back to track 2
        assert_eq!(d.track_for_altitude(2500.0), 1);
        let d = DayCycle::from_llsd(&env(false)).unwrap();
        assert_eq!(d.track_for_altitude(1500.0), 0);
    }
}
