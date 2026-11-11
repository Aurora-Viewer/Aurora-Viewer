//! Volumetric clouds in the offline demo
//! (`AURORA_DEMO_CLOUDS=fair|broken|stratocumulus|overcast`): the setting
//! Preferences › Aurora › Clouds is turned on, and the region answers its
//! ExtEnvironment with a one-key day whose sky is fixed: the sun high in the
//! south-east, the Legacy Viewer's haze and cloud colours, and a cover that
//! gives the preset's cloud type (aurora-render `clouds.rs`: the type follows
//! the sky's `cloud_shadow`). The camera looks at the horizon from behind
//! the avatar, with half the picture in sky, for captures.
//!
//! - `fair`: separate heaps of cumulus (`cloud_shadow` 0.27, the Legacy
//!   Viewer's default sky).
//! - `broken`: broken cumulus, bigger and closer together.
//! - `stratocumulus`: a lumpy sheet with gaps.
//! - `overcast`: a grey stratus sheet.
//!
//! `AURORA_CLOUDS=legacy` keeps the same sky with the Legacy Viewer's layer
//! (before / after).

use super::HANDLE;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetEvent;
use glam::{Quat, Vec3};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

/// Frame of the region answer.
const REGION_FRAME: u64 = 40;

/// The preset asked for, `None` when the scenario is off or unknown.
pub fn scenario() -> Option<&'static str> {
    static S: OnceLock<Option<&'static str>> = OnceLock::new();
    *S.get_or_init(|| {
        let value = std::env::var("AURORA_DEMO_CLOUDS").ok()?;
        PRESETS.iter().map(|p| p.0).find(|name| *name == value.trim())
    })
}

/// Name, `cloud_shadow` (the cover).
const PRESETS: [(&str, f32); 4] = [("fair", 0.27), ("broken", 0.37), ("stratocumulus", 0.42), ("overcast", 0.62)];

fn arr(v: &[f32]) -> Llsd {
    Llsd::Array(v.iter().map(|x| Llsd::Real(*x as f64)).collect())
}

/// The sky of a preset: a fixed afternoon with the sun 50 degrees up.
fn sky(cloud_shadow: f32) -> Llsd {
    let sun = Vec3::new(0.45, -0.55, 0.77).normalize();
    let q = Quat::from_rotation_arc(Vec3::X, sun);
    let m = Quat::from_rotation_arc(Vec3::X, -sun);
    llsd_map! {
        "type" => "sky",
        "sun_rotation" => arr(&[q.x, q.y, q.z, q.w]),
        "moon_rotation" => arr(&[m.x, m.y, m.z, m.w]),
        "sunlight_color" => arr(&[0.7342, 0.7815, 0.8999]),
        "star_brightness" => 0.0,
        "cloud_color" => arr(&[0.4099, 0.4099, 0.4099]),
        "cloud_pos_density1" => arr(&[1.0, 0.526, 1.0]),
        "cloud_pos_density2" => arr(&[1.0, 0.526, 1.0]),
        "cloud_scale" => 0.42,
        "cloud_scroll_rate" => arr(&[0.0, 0.0]),
        "cloud_shadow" => cloud_shadow,
        "cloud_variance" => 0.0,
        "legacy_haze" => llsd_map! {
            "blue_horizon" => arr(&[0.4954, 0.4954, 0.6399]),
            "blue_density" => arr(&[0.2447, 0.4487, 0.7599]),
            "ambient" => arr(&[0.25, 0.25, 0.25]),
            "haze_density" => 0.7,
            "haze_horizon" => 0.19,
            "density_multiplier" => 0.00018,
            "distance_multiplier" => 0.8,
        },
    }
}

/// Where the demo camera looks: east, a few degrees up, so that the sky
/// takes the upper half of the picture.
pub fn view_direction() -> Option<Vec3> {
    scenario()?;
    // AURORA_DEMO_CLOUDS_PITCH=<degrees>: a lower or higher view (the clouds
    // at the horizon are seen at 1 to 3 degrees)
    let degrees = std::env::var("AURORA_DEMO_CLOUDS_PITCH")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map_or(12.0, |v| v.clamp(-10.0, 60.0));
    let up = degrees.to_radians();
    Some(Vec3::new(up.cos(), 0.0, up.sin()))
}

/// An ExtEnvironment `environment` map with a one-key day.
fn environment(sky: Llsd) -> Llsd {
    let mut frames = aurora_llsd::Map::new();
    frames.insert("sky".into(), sky);
    frames.insert("water".into(), llsd_map! { "type" => "water" });
    llsd_map! {
        "parcel_id" => -1,
        "region_id" => Uuid::nil(),
        "day_length" => 14400,
        "day_offset" => 57600,
        "track_altitudes" => arr(&[1000.0, 2000.0, 3000.0]),
        "is_default" => false,
        "day_cycle" => llsd_map! {
            "type" => "daycycle",
            "frames" => Llsd::Map(frames),
            "tracks" => Llsd::Array(vec![
                Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "water"}]),
                Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "sky"}]),
            ]),
        },
    }
}

/// Events of this frame for the selected preset.
pub fn events(frame: u64) -> Vec<NetEvent> {
    let Some(name) = scenario() else {
        return Vec::new();
    };
    match frame {
        1 => vec![NetEvent::Capabilities {
            handle: HANDLE,
            caps: Arc::new(
                [("ExtEnvironment".to_owned(), "demo://environment".to_owned())]
                    .into_iter()
                    .collect(),
            ),
        }],
        REGION_FRAME => {
            log::info!("demo clouds ({name}): region answer");
            let shadow = PRESETS.iter().find(|p| p.0 == name).map_or(0.27, |p| p.1);
            vec![NetEvent::Environment {
                handle: HANDLE,
                parcel_id: -1,
                environment: environment(sky(shadow)),
            }]
        }
        _ => Vec::new(),
    }
}

/// The Legacy Viewer's default cloud noise texture is an asset of the grid:
/// offline, the demo draws a stand-in in the scenario only (the other demo
/// skies keep no clouds): a tileable noise of soft blobs and finer detail,
/// its value in the red channel as the shaders read it, around 0.5.
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if scenario().is_none() || *id != crate::world::eep::DEFAULT_CLOUD_ID {
        return None;
    }
    const SIZE: u32 = 256;
    let mut px = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let v = cloud_noise(x as f32 / SIZE as f32, y as f32 / SIZE as f32);
            let b = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            px.extend_from_slice(&[b, b, b, 255]);
        }
    }
    Some((px, SIZE, SIZE))
}

fn lattice(x: u32, y: u32, period: u32, octave: u32) -> f32 {
    let (x, y) = (x % period, y % period);
    let mut h = x.wrapping_mul(0x27d4_eb2d) ^ y.wrapping_mul(0x1656_67b1) ^ octave.wrapping_mul(0x9e37_79b9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

/// Periodic value noise at (u, v) in [0, 1): four octaves from 5 cells.
fn cloud_noise(u: f32, v: f32) -> f32 {
    let mut sum = 0.0;
    let mut total = 0.0;
    let mut amp = 1.0;
    for octave in 0..5u32 {
        let period = 5 << octave;
        let (fx, fy) = (u * period as f32, v * period as f32);
        let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
        let at = |dx, dy| lattice(x0 + dx, y0 + dy, period, octave);
        let top = at(0, 0) + (at(1, 0) - at(0, 0)) * sx;
        let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * sx;
        sum += amp * (top + (bottom - top) * sy);
        total += amp;
        amp *= 0.55;
    }
    // more contrast, around 0.5
    (sum / total - 0.5) * 1.8 + 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_cover_the_three_cloud_types() {
        let kinds: Vec<f32> = PRESETS.iter().map(|p| aurora_render::clouds_type_of_shadow(p.1)).collect();
        assert_eq!(kinds[0], 1.0, "fair: cumulus");
        assert!(kinds[1] > 0.3 && kinds[1] < 1.0, "broken: between");
        assert!(kinds[2] > 0.0 && kinds[2] < kinds[1], "stratocumulus: lower");
        assert_eq!(kinds[3], 0.0, "overcast: stratus");
    }

    #[test]
    fn the_stand_in_noise_tiles_and_spreads() {
        // periodic: the edges meet
        for i in 0..8 {
            let t = i as f32 / 8.0;
            assert!((cloud_noise(0.0, t) - cloud_noise(1.0, t)).abs() < 1e-4);
            assert!((cloud_noise(t, 0.0) - cloud_noise(t, 1.0)).abs() < 1e-4);
        }
        let n = 64;
        let values: Vec<f32> = (0..n * n)
            .map(|i| cloud_noise((i % n) as f32 / n as f32, (i / n) as f32 / n as f32))
            .collect();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let spread = values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.1, "{mean}");
        assert!(spread.sqrt() > 0.1, "a flat texture makes no clouds: {}", spread.sqrt());
    }

    #[test]
    fn the_sky_answers_with_clouds() {
        let Llsd::Map(m) = sky(0.45) else { panic!("a map") };
        assert!(m.contains_key("cloud_shadow") && m.contains_key("cloud_pos_density1"));
    }
}
