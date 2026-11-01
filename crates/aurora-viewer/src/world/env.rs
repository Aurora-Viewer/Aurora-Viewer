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
    /// Texture ids the sky / water need (cloud, sun, moon, water normal map,
    /// rainbow, halo).
    pub textures: [uuid::Uuid; 6],
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

/// Number of local time of day presets (`Panels::time_of_day` 1..=5).
pub const TIME_OF_DAY_PRESETS: u8 = 5;

/// The presets in the order of the Legacy Viewer's World > Environment menu
/// (Sunrise, Midday, Midday (Legacy), Sunset, Midnight); their numbers stay
/// those saved in the settings.
pub const TIME_OF_DAY_ORDER: [u8; 5] = [1, 2, 5, 3, 4];

/// The library sky of a time of day preset (LLEnvironment::KNOWN_SKY_*,
/// llenvironment.cpp), which World > Environment applies with
/// setManualEnvironment(ENV_LOCAL, KNOWN_SKY_*): 1 sunrise, 2 midday,
/// 3 sunset, 4 midnight, 5 the legacy midday.
pub fn time_of_day_sky(preset: u8) -> Option<uuid::Uuid> {
    let id = match preset {
        1 => 0x01e41537_ff51_2f1f_8ef7_17e4df760bfb,
        2 => 0xc46226b4_0e43_5a56_9708_d27ca1df3292,
        3 => 0x084e26cd_a900_28e8_08d0_64a9de5c15e2,
        4 => 0x8a01b97a_cb20_c1ea_ac63_f7ea84ad0090,
        5 => 0x6c83e853_e7f8_cad7_8ee6_5f31c453721c,
        _ => return None,
    };
    Some(uuid::Uuid::from_u128(id))
}

/// Sun of a time of day preset for the built-in sky shown when its library
/// sky cannot be fetched (and in the offline demo): 1 sunrise, 2 and 5
/// noon, 3 sunset, 4 midnight (sun below the horizon, moon up).
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

/// Whether a sky of this reflection probe ambiance is tone mapped: a classic
/// sky (ambiance 0) is not, whatever the tone mapper chosen (no_post in
/// LLPipeline::tonemap). Preferences grey the tone mapper choice then.
pub fn tone_mapped(probe_ambiance: f32) -> bool {
    probe_ambiance != 0.0
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
        // objects: the moon shares the sun's color (getMoonlightColor), with
        // no 0.7 (atmosphericsFuncs.glsl); black with neither up
        let object_sunlight = if sun_up || moon_up {
            normalized_light(sky.sunlight)
        } else {
            Vec3::ZERO
        };
        let (sun_color, zenith, horizon, ground) = if tone_mapped(sky.probe_ambiance) {
            // A sky with a reflection probe ambiance lights surfaces as
            // softenLightF's non-classic branch (calcAtmosphericVarsLinear,
            // RenderHDRSkySunlightScale and RenderSkyAmbientScale 1): the
            // objects' light attenuated along the light, made linear, x 3 of
            // pbrBaseLight (shade_pbr divides the diffuse by PI as pbrPunctual
            // does); the sky ambient is amblit's grey luminance, what the
            // probes mix with by their ambiance. The automatic exposure makes
            // up for the absolute level, as in the Legacy Viewer.
            let sunlit = srgb_to_linear(object_sunlight * exp3(-light_atten / light_norm.z.max(1e-6)));
            let amblit = srgb_to_linear(tmp_ambient.max(Vec3::ZERO).powf(0.9) * 0.57);
            let grey = Vec3::splat(amblit.dot(Vec3::new(0.2126, 0.7152, 0.0722)));
            (sunlit * 3.0, grey, grey, grey)
        } else {
            // Absolute scale of the classic (sRGB-like) light colors without
            // tone mapping: the default noon sun normalised to a radiance of ~3.
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
            (
                sun_color,
                amb * 1.1 + sky_tint * 0.6 * day + night_amb,
                amb + sky_tint * 0.3 * day + night_amb,
                amb * 0.45 + night_amb * 0.5,
            )
        };
        let diffuse = if sun_up { sun_diffuse } else { moon_diffuse * 0.6 };

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
            object_sunlight,
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
            no_post: !tone_mapped(sky.probe_ambiance),
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
            // LLDrawPoolWLSky::renderSkyHazeDeferred: no rainbow nor halo
            // without a light to make them
            moisture_level: if sun_up || moon_up { sky.moisture_level } else { 0.0 },
            droplet_radius: sky.droplet_radius,
            ice_level: if sun_up || moon_up { sky.ice_level } else { 0.0 },
            rainbow_texture: 0,
            halo_texture: 0,
            // set by the caller from the preferences (world/night_sky.rs)
            stars: Default::default(),
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
            textures: [
                sky.cloud_id,
                sky.sun_id,
                sky.moon_id,
                water.normal_map,
                // fetched only while they show
                if sky_params.moisture_level > 0.0 {
                    sky.rainbow_id
                } else {
                    uuid::Uuid::nil()
                },
                if sky_params.ice_level > 0.0 {
                    sky.halo_id
                } else {
                    uuid::Uuid::nil()
                },
            ],
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
                "haze {:.3} / {:.3}, density x {:.6}, distance x {:.3}, ",
                "sun texture {} x {:.2}"
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
            sky.sun_id,
            sky.sun_scale,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_skies_with_probe_ambiance_are_tone_mapped() {
        assert!(!tone_mapped(0.0));
        assert!(tone_mapped(0.5));
        assert!(tone_mapped(1.0));
        // the frame says no_post exactly when the sky is not tone mapped
        let sun = Vec3::new(0.0, 0.3, 0.95);
        let classic = Environment::from_sun(sun, 96.0);
        assert!(classic.sky.no_post);
        let hdr = SkyFrame {
            probe_ambiance: 0.6,
            ..SkyFrame::with_sun(sun)
        };
        let hdr = Environment::from_eep(&hdr, &WaterFrame::default(), glam::Vec2::ZERO, 96.0);
        assert!(!hdr.sky.no_post);
    }

    /// A sky with a probe ambiance lights surfaces as softenLightF's
    /// non-classic branch: the normalized sunlight attenuated along the light
    /// (calcAtmosphericVars), linear, x 3, and a grey ambient; a classic sky
    /// keeps its own scale.
    #[test]
    fn probe_ambiance_skies_light_as_the_legacy_linear_branch() {
        let sun = Vec3::new(0.3, 0.2, 0.88).normalize();
        let sky = SkyFrame {
            probe_ambiance: 1.0,
            can_auto_adjust: false,
            sunlight: Vec3::new(1.741, 1.741, 1.929),
            ambient: Vec3::ZERO,
            cloud_shadow: 0.54,
            ..SkyFrame::with_sun(sun)
        };
        let env = Environment::from_eep(&sky, &WaterFrame::default(), glam::Vec2::ZERO, 96.0);
        let atten = (sky.blue_density + Vec3::splat(sky.haze_density * 0.25)) * sky.density_multiplier * sky.max_y;
        let sunlit = normalized_light(sky.sunlight) * exp3(-atten / env.sky.light_norm.z);
        let expected = srgb_to_linear(sunlit) * 3.0;
        assert!((env.sun_color - expected).length() < 1e-5, "{:?} vs {expected:?}", env.sun_color);
        // amblit: pow(ambient + cloud shadow / 2, 0.9) x 0.57, its luminance
        let amblit = srgb_to_linear(Vec3::splat(0.27f32.powf(0.9) * 0.57)).x;
        for c in [env.zenith, env.horizon, env.ground] {
            assert!((c - Vec3::splat(amblit)).abs().max_element() < 1e-5, "{c:?}");
        }
        // the same sky without probe ambiance
        let classic = SkyFrame {
            probe_ambiance: 0.0,
            can_auto_adjust: true,
            ..sky
        };
        let classic = Environment::from_eep(&classic, &WaterFrame::default(), glam::Vec2::ZERO, 96.0);
        assert!(classic.sun_color.max_element() > 2.0 * env.sun_color.max_element());
        assert!(classic.zenith.z > classic.zenith.x, "{:?}", classic.zenith);
    }

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

    /// LLDrawPoolWLSky: no rainbow nor halo without a light; their textures
    /// are only wanted while they show.
    #[test]
    fn rainbow_and_halo_need_a_light() {
        let wet = |sun: Vec3, moon: Vec3| {
            let sky = SkyFrame {
                moisture_level: 0.8,
                ice_level: 0.6,
                moon_rotation: glam::Quat::from_rotation_arc(Vec3::X, moon),
                ..SkyFrame::with_sun(sun)
            };
            Environment::from_eep(&sky, &WaterFrame::default(), glam::Vec2::ZERO, 96.0)
        };
        let day = wet(Vec3::new(0.0, 0.3, 0.95), Vec3::new(0.0, -0.3, -0.95));
        assert_eq!((day.sky.moisture_level, day.sky.ice_level), (0.8, 0.6));
        assert!(!day.textures[4].is_nil() && !day.textures[5].is_nil());
        let dark = wet(Vec3::new(0.0, 0.3, -0.95), Vec3::new(0.0, -0.3, -0.95));
        assert_eq!((dark.sky.moisture_level, dark.sky.ice_level), (0.0, 0.0));
        assert!(dark.textures[4].is_nil() && dark.textures[5].is_nil());
        let dry = Environment::from_sun(Vec3::new(0.0, 0.3, 0.95), 96.0);
        assert!(dry.textures[4].is_nil() && dry.textures[5].is_nil());
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
