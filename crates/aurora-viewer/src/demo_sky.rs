//! Rainbow and 22° halo in the offline demo
//! (`AURORA_DEMO_SKY_OPTICS=rainbow|halo|clear`):
//! the region answers its ExtEnvironment with a one-key day whose sky has a
//! moisture level (rainbow) or an ice level (halo), the light placed for the
//! demo camera, which looks east.
//!
//! - `rainbow`: the sun low in the west, behind the camera: the bow stands
//!   over the eastern horizon, around the point opposite the sun.
//! - `halo`: the sun low in the east, in front of the camera, inside its
//!   22° ring.
//! - `clear`: the sky and view of `rainbow` without the bow (before / after).
//!
//! The rainbow and halo textures of Second Life are assets of the grid:
//! offline, [`texture`] draws stand-ins with the same layout (the shader's
//! coordinates), from the optics of the bow and of the halo.

use super::HANDLE;
use crate::world::eep::{DEFAULT_HALO_ID, DEFAULT_RAINBOW_ID};
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetEvent;
use glam::{Quat, Vec3};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

/// Frame of the region answer.
const REGION_FRAME: u64 = 40;

fn scenario() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| std::env::var("AURORA_DEMO_SKY_OPTICS").unwrap_or_default().trim().to_owned())
}

fn arr(v: &[f32]) -> Llsd {
    Llsd::Array(v.iter().map(|x| Llsd::Real(*x as f64)).collect())
}

/// A clear afternoon sky with the sun towards `sun` and the given rainbow /
/// halo strengths.
fn sky(sun: Vec3, moisture: f32, ice: f32) -> Llsd {
    let q = Quat::from_rotation_arc(Vec3::X, sun.normalize());
    let m = Quat::from_rotation_arc(Vec3::X, -sun.normalize());
    llsd_map! {
        "type" => "sky",
        "sun_rotation" => arr(&[q.x, q.y, q.z, q.w]),
        "moon_rotation" => arr(&[m.x, m.y, m.z, m.w]),
        "sunlight_color" => arr(&[0.95, 0.9, 0.82]),
        "star_brightness" => 250.0,
        "cloud_scale" => 0.0,
        "moisture_level" => moisture,
        "droplet_radius" => 800.0,
        "ice_level" => ice,
        "legacy_haze" => llsd_map! {
            "blue_horizon" => arr(&[0.25, 0.4, 0.75]),
            "blue_density" => arr(&[0.25, 0.45, 0.8]),
            "ambient" => arr(&[0.3, 0.3, 0.33]),
            "haze_density" => 0.6,
            "haze_horizon" => 0.19,
            "density_multiplier" => 0.00018,
            "distance_multiplier" => 0.8,
        },
    }
}

/// The scenario's sky, `None` when it is off.
fn scenario_sky() -> Option<Llsd> {
    let elevation = |deg: f32| deg.to_radians();
    match scenario() {
        // west, 12° up: the bow's top 30° over the eastern horizon
        "rainbow" | "clear" => {
            let e = elevation(12.0);
            let moisture = if scenario() == "rainbow" { 1.0 } else { 0.0 };
            Some(sky(Vec3::new(-e.cos(), 0.0, e.sin()), moisture, 0.0))
        }
        // east, 14° up
        "halo" => {
            let e = elevation(14.0);
            Some(sky(Vec3::new(e.cos(), 0.0, e.sin()), 0.0, 1.0))
        }
        _ => None,
    }
}

/// Where the demo camera looks: east, raised to the bow's top or to the halo.
pub fn view_direction() -> Option<Vec3> {
    let up = match scenario() {
        "rainbow" | "clear" => 22.0_f32,
        "halo" => 16.0,
        _ => return None,
    }
    .to_radians();
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

/// Events of this frame for the selected scenario.
pub fn events(frame: u64) -> Vec<NetEvent> {
    let Some(sky) = scenario_sky() else {
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
            log::info!("demo sky ({}): region answer", scenario());
            vec![NetEvent::Environment {
                handle: HANDLE,
                parcel_id: -1,
                environment: environment(sky),
            }]
        }
        _ => Vec::new(),
    }
}

/// sRGB-like bytes of a color in 0..1.
fn bytes(c: Vec3) -> [u8; 4] {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    [b(c.x), b(c.y), b(c.z), 255]
}

/// Hue of a wavelength band position `p` (0 red .. 1 violet).
fn spectrum(p: f32) -> Vec3 {
    const STOPS: [(f32, [f32; 3]); 6] = [
        (0.0, [0.36, 0.11, 0.08]),
        (0.2, [0.36, 0.2, 0.0]),
        (0.4, [0.27, 0.28, 0.0]),
        (0.6, [0.16, 0.28, 0.16]),
        (0.8, [0.0, 0.23, 0.38]),
        (1.0, [0.22, 0.22, 0.32]),
    ];
    let p = p.clamp(0.0, 1.0);
    let i = STOPS.iter().rposition(|(at, _)| *at <= p).unwrap_or(0).min(STOPS.len() - 2);
    let (a, ca) = STOPS[i];
    let (b, cb) = STOPS[i + 1];
    Vec3::from(ca).lerp(Vec3::from(cb), (p - a) / (b - a))
}

/// The rainbow texture: 8 droplet radii across, 512 rows of skyF's v
/// (rows top to bottom, v up the image). Outside the bow a faint veil; the
/// colored band from red at v 0.18 (42° from the antisolar point) to violet
/// at 0.23; inside it the brighter interior of the bow fading towards v
/// 0.67, where the shader's stretched interior reaches.
fn rainbow() -> (Vec<u8>, u32, u32) {
    const W: u32 = 8;
    const H: u32 = 512;
    let veil = 36.0 / 255.0;
    let mut px = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H {
        let v = 1.0 - (row as f32 + 0.5) / H as f32;
        for x in 0..W {
            // bigger drops: a slightly brighter, purer bow
            let drop = 0.88 + 0.12 * x as f32 / (W - 1) as f32;
            let c = if (0.178..0.232).contains(&v) {
                let p = (v - 0.178) / 0.054;
                // the band over a background going from the veil (outside)
                // to the interior's grey (inside), with soft edges
                let ground = Vec3::splat(veil + (0.3 * drop - veil) * ((p - 0.7) / 0.3).clamp(0.0, 1.0));
                let edge = (p * 6.0).min((1.0 - p) * 6.0).clamp(0.0, 1.0);
                ground.lerp(spectrum(p) * drop + Vec3::splat(0.04), edge)
            } else if v >= 0.232 {
                // the interior, brightest along the bow
                let k = ((0.67 - v) / (0.67 - 0.232)).clamp(0.0, 1.0);
                Vec3::splat(veil + (0.3 - veil) * k.powf(1.3) * drop)
            } else {
                Vec3::splat(veil)
            };
            px.extend_from_slice(&bytes(c));
        }
    }
    (px, W, H)
}

/// The halo texture: 128 × 128, every column the profile skyF reads at the
/// left edge, rows of v = sin(angle to the light) (top to bottom, v up the
/// image). Dark inside 21.5°, a red inner rim, then white light fading out
/// to about 35° (the halo's tail).
fn halo() -> (Vec<u8>, u32, u32) {
    const N: u32 = 128;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for row in 0..N {
        let v = 1.0 - (row as f32 + 0.5) / N as f32;
        let deg = v.clamp(0.0, 1.0).asin().to_degrees();
        let c = if deg < 21.5 {
            Vec3::ZERO
        } else {
            let t = deg - 21.5;
            // rim: red, then orange and yellow in the first degree and a half
            let rim = Vec3::new(0.42, 0.16, 0.02).lerp(Vec3::new(0.44, 0.38, 0.2), (t / 1.5).clamp(0.0, 1.0));
            let fade = (-(t - 1.0).max(0.0) / 4.5).exp();
            rim.lerp(Vec3::splat(0.42), (t / 3.0).clamp(0.0, 1.0)) * fade * (t / 0.4).clamp(0.0, 1.0)
        };
        let p = bytes(c);
        for _ in 0..N {
            px.extend_from_slice(&p);
        }
    }
    (px, N, N)
}

/// Pixels of the stand-in textures (RGBA8, rows top to bottom), in the
/// offline demo's scenario only: these are the ids of Second Life's own
/// textures, which a grid session fetches.
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if std::env::var_os("AURORA_DEMO").is_none() || view_direction().is_none() {
        None
    } else if *id == DEFAULT_RAINBOW_ID {
        Some(rainbow())
    } else if *id == DEFAULT_HALO_ID {
        Some(halo())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Row of skyF's v in a texture of `h` rows (v runs up the image).
    fn row(v: f32, h: u32) -> usize {
        ((1.0 - v) * h as f32) as usize
    }

    #[test]
    fn rainbow_band_runs_from_red_outside_to_blue_inside() {
        let (px, w, h) = rainbow();
        assert_eq!((w, h, px.len()), (8, 512, 8 * 512 * 4));
        let at = |v: f32| {
            let i = row(v, h) * w as usize * 4 + 4 * 4;
            [px[i], px[i + 1], px[i + 2]]
        };
        let red = at(0.186);
        let blue = at(0.222);
        assert!(red[0] > red[2] + 40, "{red:?}");
        assert!(blue[2] > blue[0] + 40, "{blue:?}");
        // the veil outside, a brighter interior inside
        assert_eq!(at(0.1), [36, 36, 36]);
        assert!(at(0.3)[0] > 60);
    }

    #[test]
    fn halo_is_dark_inside_22_degrees_and_lit_on_its_ring() {
        let (px, n, _) = halo();
        let at = |deg: f32| {
            let v = deg.to_radians().sin();
            let i = row(v, n) * n as usize * 4;
            [px[i], px[i + 1], px[i + 2]]
        };
        assert_eq!(at(15.0), [0, 0, 0]);
        let rim = at(22.2);
        assert!(rim[0] > 60 && rim[0] > rim[2], "{rim:?}");
        assert!(at(26.0)[2] > 40);
        assert!(at(50.0)[0] < 10);
    }

    #[test]
    fn scenarios_put_the_light_where_the_camera_sees_the_effect() {
        let rainbow = sky(Vec3::new(-1.0, 0.0, 0.2), 1.0, 0.0);
        let Some(crate::world::eep::Settings::Sky(s)) = crate::world::eep::Settings::from_llsd(&rainbow) else {
            panic!("a sky");
        };
        assert_eq!((s.moisture_level, s.ice_level, s.droplet_radius), (1.0, 0.0, 800.0));
        assert!(s.sun_direction().x < -0.9);
    }
}
