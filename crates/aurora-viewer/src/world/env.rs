//! Frame lighting and sky parameters derived from the region's EEP sky
//! (or Second Life's default sky with the simulator's sun direction).
//!
//! Light settings follow LLSettingsSky::calculateLightSettings (Second Life
//! viewer, Copyright (C) Linden Research, Inc., originally LGPL 2.1).

use super::eep::{SkyFrame, WaterFrame};
use aurora_render::{SkyParams, WaterParams};
use glam::Vec3;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Environment {
    /// Direction towards the dominant light (sun, or moon at night).
    pub sun_dir: Vec3,
    /// Linear radiance of that light.
    pub sun_color: Vec3,
    /// 0 at night, 1 in daylight.
    pub sun_visible: f32,
    /// Hemisphere ambient (linear).
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
    pub fog_color: Vec3,
    pub fog_density: f32,
    pub sky: SkyParams,
    pub water: WaterParams,
    /// Texture ids the sky / water need (cloud, sun, moon, water normal map).
    pub textures: [uuid::Uuid; 4],
    /// Sky reflection_probe_ambiance: minimum ambiance of every probe.
    pub probe_ambiance: f32,
}

fn srgb_to_linear(c: Vec3) -> Vec3 {
    let f = |x: f32| {
        let x = x.max(0.0);
        if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
    };
    Vec3::new(f(c.x), f(c.y), f(c.z))
}

/// LLPipeline::setupHWLights (pipeline.cpp) mSunDiffuse / mMoonDiffuse:
/// the sky's light color scaled down so its largest component is at most 1,
/// then clamped to [0, 1]. LLPipeline::bindDeferredShader hands these to the
/// deferred lighting (sunlight_color / moonlight_color), so objects never see
/// a light brighter than white, whatever the sky's sunlight_color.
pub fn normalized_light(c: Vec3) -> Vec3 {
    let max = c.max_element();
    let c = if max > 1.0 { c / max } else { c };
    c.clamp(Vec3::ZERO, Vec3::ONE)
}

fn exp3(v: Vec3) -> Vec3 {
    Vec3::new(v.x.exp(), v.y.exp(), v.z.exp())
}

/// Local time of day presets (Firestorm Monde > Environnement):
/// 1 sunrise, 2 noon, 3 sunset, 4 midnight (sun below the horizon, moon up).
pub fn time_of_day_sun(preset: u8) -> Vec3 {
    match preset {
        1 => Vec3::new(1.0, 0.15, 0.08).normalize(),
        3 => Vec3::new(-1.0, -0.15, 0.08).normalize(),
        4 => Vec3::new(-0.2, 0.3, -0.93).normalize(),
        _ => midday_sun(),
    }
}

/// A pleasant midday direction (SL default "Midday" has the sun high, slightly east).
pub fn midday_sun() -> Vec3 {
    Vec3::new(0.35, 0.25, 0.9).normalize()
}

impl Environment {
    /// Environment for a region without (or before) its EEP settings: the
    /// default sky with the simulator's sun.
    #[cfg(test)]
    pub fn from_sun(sun_dir: Vec3, draw_distance: f32) -> Environment {
        Self::from_eep(
            &SkyFrame::with_sun(sun_dir),
            &WaterFrame::default(),
            glam::Vec2::ZERO,
            draw_distance,
        )
    }

    /// Lighting from an EEP sky frame. `cloud_scroll` is the accumulated
    /// cloud offset (LLEnvironment::mCloudScrollDelta).
    pub fn from_eep(sky: &SkyFrame, water: &WaterFrame, cloud_scroll: glam::Vec2, draw_distance: f32) -> Environment {
        Self::build(sky, water, cloud_scroll, draw_distance)
    }

    fn build(sky: &SkyFrame, water: &WaterFrame, scroll: glam::Vec2, draw_distance: f32) -> Environment {
        let sun = sky.sun_direction();
        let moon = sky.moon_direction();
        let sun_up = sun.z >= 0.0;
        let moon_up = moon.z >= 0.0;
        let light = if sun_up {
            sun
        } else if moon_up {
            moon
        } else {
            Vec3::NEG_Z
        };
        // clamped light norm (LLEnvironment::getClampedLightNorm)
        let mut light_norm = light;
        if light_norm.z < -0.1 {
            light_norm.z = -0.1;
        }

        // calculateLightSettings
        let max_y = sky.max_y;
        let light_atten = (sky.blue_density + Vec3::splat(sky.haze_density * 0.25)) * sky.density_multiplier * max_y;
        let total_density = sky.blue_density + Vec3::splat(sky.haze_density);
        let transmittance = exp3(-total_density * (sky.density_multiplier * max_y));
        let limit = f32::EPSILON * 8.0;
        let mut lighty = light.z.abs();
        if lighty >= limit {
            lighty = 1.0 / lighty;
        }
        let lighty = lighty.max(limit);
        let sun_diffuse = sky.sunlight * exp3(-light_atten * lighty) * transmittance;
        let tmp_ambient = sky.ambient + (Vec3::ONE - sky.ambient) * sky.cloud_shadow * 0.5;
        let moon_brightness = if moon_up { sky.moon_brightness } else { 0.001 };
        let moon_diffuse = sky.sunlight * exp3(-light_atten * lighty) * transmittance * moon_brightness;

        let day = ((sun.z + 0.08) / 0.3).clamp(0.0, 1.0);
        // Absolute scale of the classic (sRGB-like) light colors: SL compensates
        // with auto-exposure; we use a fixed exposure, so normalise the default
        // noon sun to a radiance of ~3.
        let sun_scale = 15.0;
        let diffuse = if sun_up { sun_diffuse } else { moon_diffuse * 0.6 };
        let sun_color = srgb_to_linear(diffuse.min(Vec3::splat(3.0))) * sun_scale * (1.0 - sky.cloud_shadow * 0.35);
        let amb = srgb_to_linear(tmp_ambient.min(Vec3::splat(3.0))) * 3.2;
        // tint the upper hemisphere with the sky's blue
        let tint = srgb_to_linear((sky.blue_horizon * sky.blue_density * 1.6).min(Vec3::splat(2.0)));
        // desaturated: the ambient comes from the whole sky, not just its blue
        let grey = (tint.x + tint.y + tint.z) / 3.0;
        let sky_tint = Vec3::splat(grey).lerp(tint, 0.45);
        let night_amb = Vec3::new(0.66, 0.66, 1.2) * 0.0125 * (1.0 - day);
        let zenith = amb * 1.1 + sky_tint * 0.6 * day + night_amb;
        let horizon = amb + sky_tint * 0.3 * day + night_amb;
        let ground = amb * 0.45 + night_amb * 0.5;

        // haze color and density (calcAtmosphericVars)
        let mut haze = sky.blue_horizon * sky.blue_density * (diffuse * (1.0 - sky.cloud_shadow) + tmp_ambient)
            + Vec3::splat(sky.haze_horizon * sky.haze_density) * (diffuse * (1.0 - sky.cloud_shadow) + tmp_ambient);
        haze = haze.min(Vec3::splat(2.0));
        let fog_color = srgb_to_linear(haze * 2.0).min(Vec3::splat(4.0));
        let total = (total_density.x + total_density.y + total_density.z) / 3.0;
        let sl_density = total * sky.density_multiplier * sky.distance_multiplier;
        let fog_density = sl_density.max(0.35 / draw_distance.max(32.0));

        let hdr_scale = if sky.probe_ambiance != 0.0 {
            sky.gamma.max(0.0).sqrt() * 2.0
        } else {
            1.0
        };
        let sky_params = SkyParams {
            light_norm,
            // skyV.glsl: "magic 0.7 to match legacy color" for the moon
            sunlight: if sun_up { sky.sunlight } else { sky.sunlight * 0.7 },
            // objects: the moon shares the sun's color (getMoonlightColor),
            // with no 0.7 (atmosphericsFuncs.glsl); black with neither up
            object_sunlight: if sun_up || moon_up {
                normalized_light(sky.sunlight)
            } else {
                Vec3::ZERO
            },
            gamma: sky.gamma,
            ambient: sky.ambient,
            blue_horizon: sky.blue_horizon,
            blue_density: sky.blue_density,
            haze_horizon: sky.haze_horizon,
            haze_density: sky.haze_density,
            density_multiplier: sky.density_multiplier,
            distance_multiplier: sky.distance_multiplier,
            max_y,
            glow: sky.glow,
            sun_moon_glow_factor: if sun_up {
                1.0
            } else if moon_up {
                sky.moon_brightness * 0.25
            } else {
                0.0
            },
            cloud_shadow: sky.cloud_shadow,
            cloud_color: sky.cloud_color,
            cloud_scale: sky.cloud_scale,
            cloud_variance: sky.cloud_variance,
            // SL-13084: the x scroll is negated (custom cloud textures are flipped)
            cloud_pos_density1: sky.cloud_pos_density1 + Vec3::new(-scroll.x, scroll.y, 0.0),
            cloud_pos_density2: sky.cloud_pos_density2,
            dome_offset: sky.dome_offset,
            dome_radius: sky.dome_radius,
            hdr_scale,
            classic: sky.can_auto_adjust,
            no_post: sky.probe_ambiance == 0.0,
            sun_dir: sun,
            moon_dir: moon,
            sun_scale: sky.sun_scale,
            moon_scale: sky.moon_scale,
            moon_brightness: sky.moon_brightness,
            star_brightness: sky.star_brightness,
            // slots are filled by the caller once the textures are resident
            cloud_texture: 0,
            sun_texture: 0,
            moon_texture: 0,
        };
        let water_params = WaterParams {
            fog_color: water.fog_color,
            fog_density: water.fog_density,
            underwater_fog_mod: water.underwater_fog_mod,
            fresnel_scale: water.fresnel_scale,
            fresnel_offset: water.fresnel_offset,
            blur_multiplier: water.blur_multiplier,
            normal_scale: water.normal_scale,
            wave1: water.wave1,
            wave2: water.wave2,
            normal_texture: 0,
        };
        Environment {
            sun_dir: light,
            sun_color,
            sun_visible: day,
            zenith,
            horizon,
            ground,
            fog_color,
            fog_density,
            sky: sky_params,
            water: water_params,
            textures: [sky.cloud_id, sky.sun_id, sky.moon_id, water.normal_map],
            probe_ambiance: sky.probe_ambiance,
        }
    }
}

/// Throttles the log line describing the interpolated sky: at most every
/// 30 s, or at once when the sky source changes (region, day cycle, altitude
/// track, time of day preset), identified by `key`.
#[derive(Debug, Default)]
pub struct SkyLog {
    last: Option<(Instant, u64)>,
}

impl SkyLog {
    pub const INTERVAL: Duration = Duration::from_secs(30);

    pub fn due(&mut self, now: Instant, key: u64) -> bool {
        let due = match self.last {
            Some((at, k)) => k != key || now.duration_since(at) >= Self::INTERVAL,
            None => true,
        };
        if due {
            self.last = Some((now, key));
        }
        due
    }
}

impl Environment {
    /// One line with what drives the lighting of this frame (for comparing
    /// with Firestorm on the grid).
    pub fn describe(&self, sky: &SkyFrame) -> String {
        let elevation = sky.sun_direction().z.clamp(-1.0, 1.0).asin().to_degrees();
        format!(
            concat!(
                "sky frame: classic {} probe ambiance {} gamma {} sun elevation {:.1} deg, ",
                "sunlight {:.3?} (objects {:.3?}), ambient {:.3?}, cloud shadow {:.3}, ",
                "haze {:.3} / {:.3}, density x {:.6}, distance x {:.3}"
            ),
            sky.can_auto_adjust,
            sky.probe_ambiance,
            sky.gamma,
            elevation,
            sky.sunlight.to_array(),
            self.sky.object_sunlight.to_array(),
            sky.ambient.to_array(),
            sky.cloud_shadow,
            sky.haze_horizon,
            sky.haze_density,
            sky.density_multiplier,
            sky.distance_multiplier,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_and_night() {
        let noon = Environment::from_sun(Vec3::new(0.0, 0.3, 0.95), 96.0);
        assert!(noon.sun_visible > 0.99);
        assert!(noon.sun_color.max_element() > 0.5, "{:?}", noon.sun_color);
        assert!(noon.zenith.is_finite() && noon.fog_color.is_finite());
        let night = Environment::from_sun(Vec3::new(0.0, 0.3, -0.95), 96.0);
        assert!(night.sun_visible < 0.01);
        assert!(night.sun_color.max_element() < noon.sun_color.max_element());
        // the moon (opposite the sun) lights the night
        assert!(night.sun_dir.z > 0.0);
        // objects are lit by the moon with the sun's normalized color
        assert_eq!(night.sky.object_sunlight, noon.sky.object_sunlight);
    }

    #[test]
    fn sky_log_throttle() {
        let mut log = SkyLog::default();
        let t0 = Instant::now();
        assert!(log.due(t0, 1));
        assert!(!log.due(t0 + Duration::from_secs(10), 1));
        // a new sky source logs at once
        assert!(log.due(t0 + Duration::from_secs(11), 2));
        assert!(!log.due(t0 + Duration::from_secs(40), 2));
        assert!(log.due(t0 + Duration::from_secs(41), 2));
    }

    #[test]
    fn object_light_is_normalized() {
        // EEP skies often carry sunlight_color above 1 (legacy x 3 scale)
        let n = normalized_light(Vec3::new(2.25, 2.4, 3.0));
        assert!((n - Vec3::new(0.75, 0.8, 1.0)).length() < 1e-6, "{n:?}");
        let dim = Vec3::new(0.7342, 0.7815, 0.8999);
        assert_eq!(normalized_light(dim), dim);
        assert_eq!(normalized_light(Vec3::new(-0.5, 0.2, 0.3)), Vec3::new(0.0, 0.2, 0.3));
    }
}
