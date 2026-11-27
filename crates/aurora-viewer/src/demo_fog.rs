//! Aurora Weather's fog in the offline demo
//! (`AURORA_DEMO_FOG=sunset|neon|indoor|underwater`): the fog turns on (at
//! `AURORA_FOG`'s density, 0.45 without it), the region answers its
//! ExtEnvironment with a fixed sky, a few props stand on the plaza and the
//! camera is placed for captures (the interface hidden).
//!
//! - `sunset`: the sun low in the west behind a row of posts and a wall with
//!   gaps: god rays through the fog, the posts' shadows in it.
//! - `neon`: a street at night between two rows of blocks, neon signs on
//!   their fronts (fullbright, glowing, each with its light) and a projector
//!   lamp: the lights glowing through the fog.
//! - `indoor`: inside a closed room with a doorway, looking out at the fog:
//!   the room stays clear, the fog outside.
//! - `underwater`: under the lagoon's water looking up the beach: the
//!   water's fog alone, no fog on top of it.
//! - `shelter`: from outside, the room's doorway (a tongue of fog going in a
//!   few metres; the red lamp inside lights the fog out of the doorway only,
//!   not through the walls), an open porch on posts (fog under it) and a
//!   closed glass box (clear inside): the fog flowing (aurora-render fog.rs
//!   `Flow`).
//! - `aerial`: 60 m up, looking across the region to its hill and the sea
//!   past it: the fog lies on the hill and the sea to the horizon, with no
//!   line where the volume ends.
//! - `ground`: 4 m over the ground, looking across it at a low angle: the
//!   fog even over it, without bands at the slices' distances.

use super::{HANDLE, height, prim, shape, te};
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::extra::LightImageParams;
use aurora_prim::params::{LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE};
use aurora_prim::te::BLANK_TEXTURE;
use aurora_prim::{ExtraParams, LightParams};
use glam::{Quat, Vec3};
use std::sync::{Arc, OnceLock};

/// Frame of the region answer (after the clouds' scenario's, never both).
const REGION_FRAME: u64 = 40;
/// The props' first local id.
const FIRST_ID: u32 = 9900;
/// The fog's density when `AURORA_FOG` is not given.
pub const DENSITY: f32 = 0.45;

/// The scenario asked for, `None` when it is off or unknown, or outside demo
/// mode (the variable left set never turns the fog on in a grid session).
pub fn scenario() -> Option<&'static str> {
    static S: OnceLock<Option<&'static str>> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var_os("AURORA_DEMO")?;
        parse(&std::env::var("AURORA_DEMO_FOG").ok()?)
    })
}

fn parse(value: &str) -> Option<&'static str> {
    ["sunset", "neon", "indoor", "underwater", "shelter", "aerial", "ground"]
        .into_iter()
        .find(|name| *name == value.trim())
}

fn arr(v: &[f32]) -> Llsd {
    Llsd::Array(v.iter().map(|x| Llsd::Real(*x as f64)).collect())
}

/// Toward the sun of a scenario (azimuth 0 east, 90 north).
fn sun_of(name: &str) -> Vec3 {
    let (azimuth, elevation) = match name {
        // low in the west, in front of the camera
        "sunset" => (180.0f32, 5.0f32),
        // well below the horizon
        "neon" => (200.0, -25.0),
        _ => (150.0, 40.0),
    };
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    Vec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin())
}

/// The direction to the sun for the lighting and shadows (demo.rs
/// `sun_override`), the sky's own.
pub fn sun() -> Option<Vec3> {
    scenario().map(sun_of)
}

/// The scenario's sky: a warm, hazy sunset; a dark night; a hazy afternoon.
fn sky(name: &str) -> Llsd {
    let sun = sun_of(name);
    let q = Quat::from_rotation_arc(Vec3::X, sun);
    let m = Quat::from_rotation_arc(Vec3::X, -sun);
    let (sunlight, horizon, ambient, haze, stars) = match name {
        "sunset" => ([1.0, 0.62, 0.36], [0.8, 0.55, 0.45], [0.32, 0.26, 0.28], 1.4, 0.0),
        "neon" => ([0.18, 0.2, 0.3], [0.08, 0.09, 0.16], [0.05, 0.05, 0.09], 0.9, 1.0),
        _ => ([0.73, 0.78, 0.9], [0.4954, 0.4954, 0.6399], [0.25, 0.25, 0.25], 1.0, 0.0),
    };
    llsd_map! {
        "type" => "sky",
        "sun_rotation" => arr(&[q.x, q.y, q.z, q.w]),
        "moon_rotation" => arr(&[m.x, m.y, m.z, m.w]),
        "sunlight_color" => arr(&sunlight),
        "star_brightness" => stars,
        "cloud_shadow" => 0.0,
        "legacy_haze" => llsd_map! {
            "blue_horizon" => arr(&horizon),
            "blue_density" => arr(&[0.2447, 0.4487, 0.7599]),
            "ambient" => arr(&ambient),
            "haze_density" => haze,
            "haze_horizon" => 0.25,
            "density_multiplier" => 0.00018,
            "distance_multiplier" => 0.8,
        },
    }
}

/// Events of this frame for the scenario: the ExtEnvironment capability,
/// then the region's fixed day.
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
            log::info!("demo fog ({name}): region answer");
            vec![NetEvent::Environment {
                handle: HANDLE,
                parcel_id: -1,
                environment: super::clouds::environment(sky(name)),
            }]
        }
        _ => Vec::new(),
    }
}

/// `AURORA_DEMO_FOG_SWING=<degrees>`: the camera swings side to side by
/// this much, every 200 frames, through its view at frames 100, 200, 300…
/// at its fastest (a capture there shows the fog while the camera turns,
/// against a still one).
fn swing() -> f32 {
    static SWING: OnceLock<f32> = OnceLock::new();
    *SWING.get_or_init(|| {
        std::env::var("AURORA_DEMO_FOG_SWING")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(0.0, |v| v.clamp(0.0, 90.0))
    })
}

/// The yaw of the swing at `frame` (rad).
fn swing_yaw(degrees: f32, frame: u64) -> f32 {
    degrees.to_radians() * (frame as f32 * std::f32::consts::PI / 100.0).sin()
}

/// Where the camera stands and what it looks at at `frame`.
pub fn camera_at(frame: u64) -> Option<(Vec3, Vec3)> {
    let (eye, target) = camera()?;
    let yaw = swing_yaw(swing(), frame);
    let turned = Quat::from_rotation_z(yaw) * (target - eye);
    Some((eye, eye + turned))
}

/// Where the camera stands and what it looks at.
pub fn camera() -> Option<(Vec3, Vec3)> {
    let at = |x: f32, y: f32, up: f32| Vec3::new(x, y, height(x, y) + up);
    Some(match scenario()? {
        // east of the posts, looking west into the low sun
        "sunset" => (at(152.0, 128.0, 2.2), at(110.0, 130.0, 6.0)),
        // down the street between the blocks
        "neon" => (at(150.0, 106.0, 1.8), at(120.0, 106.0, 3.0)),
        // inside the room, looking out of its doorway
        "indoor" => (at(160.5, 140.0, 1.7), at(150.0, 140.0, 1.5)),
        // high over the south-west, looking toward the hill in the north-east
        "aerial" => (at(40.0, 40.0, 60.0), at(190.0, 170.0, 0.0)),
        // low over the ground, looking across it (the slices' bands)
        "ground" => (at(110.0, 100.0, 4.0), at(170.0, 150.0, 0.0)),
        // west of the room, the porch on the right, the glass box on the left
        "shelter" => (at(134.0, 140.0, 2.0), at(156.0, 140.0, 1.4)),
        // under the lagoon, looking up the beach
        _ => (Vec3::new(40.0, 130.0, 17.5), Vec3::new(80.0, 130.0, 19.0)),
    })
}

/// The scenario's props, from local id `FIRST_ID` on.
pub fn objects() -> Vec<ObjectUpdate> {
    let Some(name) = scenario() else {
        return Vec::new();
    };
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    // the props' local ids, one after another
    let next = std::cell::Cell::new(FIRST_ID);
    let take = || {
        let id = next.get();
        next.set(id + 1);
        id
    };
    let mut out = Vec::new();
    let solid = |pos: Vec3, scale: Vec3, color: [f32; 4]| {
        prim(
            take(),
            pos,
            Quat::IDENTITY,
            scale,
            boxp,
            te(color, 0, false, 0.0),
            ExtraParams::default(),
            "",
        )
    };
    let ground = |x: f32, y: f32| height(x, y);
    match name {
        "sunset" => {
            // a row of posts north to south, and a wall with gaps behind
            for k in 0..9 {
                let y = 116.0 + k as f32 * 3.5;
                out.push(solid(
                    Vec3::new(128.0, y, ground(128.0, y) + 3.0),
                    Vec3::new(0.5, 0.5, 6.0),
                    [0.3, 0.28, 0.26, 1.0],
                ));
            }
            for k in 0..6 {
                let y = 112.0 + k as f32 * 6.0;
                out.push(solid(
                    Vec3::new(120.0, y, ground(120.0, y) + 4.0),
                    Vec3::new(0.6, 4.0, 8.0),
                    [0.4, 0.36, 0.32, 1.0],
                ));
            }
        }
        "neon" => {
            let colors = [
                [1.0, 0.1, 0.6, 1.0],
                [0.1, 0.9, 1.0, 1.0],
                [0.7, 0.2, 1.0, 1.0],
                [1.0, 0.55, 0.1, 1.0],
            ];
            for side in [-1.0f32, 1.0] {
                for k in 0..5 {
                    let x = 145.0 - k as f32 * 7.0;
                    let y = 106.0 + side * 6.0;
                    let g = ground(x, y);
                    out.push(solid(
                        Vec3::new(x, y + side * 2.0, g + 6.0),
                        Vec3::new(6.0, 4.0, 12.0),
                        [0.12, 0.12, 0.14, 1.0],
                    ));
                    // the sign on the block's front, toward the street
                    let c = colors[(k + if side > 0.0 { 2 } else { 0 }) % colors.len()];
                    let light = LightParams {
                        color: c,
                        radius: 9.0,
                        cutoff: 0.0,
                        falloff: 0.6,
                    };
                    out.push(prim(
                        take(),
                        Vec3::new(x, y - side * 0.05, g + 3.2 + (k % 2) as f32 * 1.5),
                        Quat::IDENTITY,
                        Vec3::new(3.0, 0.12, 0.35),
                        boxp,
                        te(c, 0, true, 0.6),
                        ExtraParams {
                            light: Some(light),
                            ..Default::default()
                        },
                        "",
                    ));
                }
            }
            // a street lamp turned down: a cone of light in the fog
            let g = ground(132.0, 106.0);
            out.push(prim(
                take(),
                Vec3::new(132.0, 106.0, g + 6.0),
                Quat::IDENTITY,
                Vec3::new(0.4, 0.4, 0.1),
                boxp,
                te([1.0, 0.95, 0.8, 1.0], 0, true, 0.3),
                ExtraParams {
                    light: Some(LightParams {
                        color: [1.0, 0.92, 0.75, 1.0],
                        radius: 12.0,
                        cutoff: 0.0,
                        falloff: 0.3,
                    }),
                    light_image: Some(LightImageParams {
                        texture: BLANK_TEXTURE,
                        params: Vec3::new(1.0, 0.0, 0.0),
                    }),
                    ..Default::default()
                },
                "",
            ));
        }
        "indoor" | "shelter" => {
            // a room 8 x 8 x 4 m around (158, 140), its west wall with a doorway
            let (x0, y0) = (158.0, 140.0);
            let g = ground(x0, y0);
            let grey = [0.7, 0.68, 0.65, 1.0];
            out.push(solid(Vec3::new(x0, y0, g + 4.1), Vec3::new(8.4, 8.4, 0.2), grey));
            solid(Vec3::new(x0 + 4.1, y0, g + 2.0), Vec3::new(0.2, 8.4, 4.0), grey);
            out.push(solid(Vec3::new(x0, y0 + 4.1, g + 2.0), Vec3::new(8.4, 0.2, 4.0), grey));
            solid(Vec3::new(x0, y0 - 4.1, g + 2.0), Vec3::new(8.4, 0.2, 4.0), grey);
            // the west wall in two halves around a 2 m doorway, a lintel
            out.push(solid(Vec3::new(x0 - 4.1, y0 + 2.55, g + 2.0), Vec3::new(0.2, 3.1, 4.0), grey));
            solid(Vec3::new(x0 - 4.1, y0 - 2.55, g + 2.0), Vec3::new(0.2, 3.1, 4.0), grey);
            out.push(solid(Vec3::new(x0 - 4.1, y0, g + 3.25), Vec3::new(0.2, 2.0, 1.5), grey));
            // a red lamp in the room: its light in the fog leaves by the
            // doorway only, never through the walls
            out.push(prim(
                take(),
                Vec3::new(x0 + 1.0, y0, g + 2.0),
                Quat::IDENTITY,
                Vec3::new(0.2, 0.2, 0.2),
                boxp,
                te([1.0, 0.2, 0.15, 1.0], 0, true, 0.4),
                ExtraParams {
                    light: Some(LightParams {
                        color: [1.0, 0.15, 0.1, 1.0],
                        radius: 12.0,
                        cutoff: 0.0,
                        falloff: 0.3,
                    }),
                    ..Default::default()
                },
                "",
            ));
            // an open porch: a roof 5 x 5 m on four posts, south-west of the room
            let (px, py) = (150.0, 132.0);
            let pg = ground(px, py);
            out.push(solid(Vec3::new(px, py, pg + 3.1), Vec3::new(5.0, 5.0, 0.2), grey));
            for (dx, dy) in [(-2.3, -2.3), (2.3, -2.3), (-2.3, 2.3), (2.3, 2.3)] {
                out.push(solid(Vec3::new(px + dx, py + dy, pg + 1.5), Vec3::new(0.25, 0.25, 3.0), grey));
            }
            // a closed glass box 3 m a side, north-west of the room: clear inside
            let (bx, by) = (150.0, 148.0);
            let bg = ground(bx, by);
            let glass = [0.75, 0.85, 0.9, 0.25];
            for (pos, scale) in [
                (Vec3::new(bx, by, bg + 3.0), Vec3::new(3.0, 3.0, 0.1)),
                (Vec3::new(bx, by, bg + 0.05), Vec3::new(3.0, 3.0, 0.1)),
                (Vec3::new(bx - 1.5, by, bg + 1.5), Vec3::new(0.1, 3.0, 3.0)),
                (Vec3::new(bx + 1.5, by, bg + 1.5), Vec3::new(0.1, 3.0, 3.0)),
                (Vec3::new(bx, by - 1.5, bg + 1.5), Vec3::new(3.0, 0.1, 3.0)),
                (Vec3::new(bx, by + 1.5, bg + 1.5), Vec3::new(3.0, 0.1, 3.0)),
            ] {
                out.push(solid(pos, scale, glass));
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scenario_is_found_by_its_name_only() {
        assert_eq!(parse(" neon "), Some("neon"));
        assert_eq!(parse("sunset"), Some("sunset"));
        assert_eq!(parse("fog"), None);
    }

    #[test]
    fn the_sunset_is_low_in_the_west_and_the_night_below_the_horizon() {
        let s = sun_of("sunset");
        assert!(s.x < -0.9 && s.z > 0.0 && s.z < 0.15, "{s}");
        assert!(sun_of("neon").z < -0.3);
        assert!((s.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_swing_passes_the_view_at_full_speed_every_hundred_frames() {
        for frame in [100, 200, 900] {
            assert!(swing_yaw(20.0, frame).abs() < 1e-4, "{frame}");
            let step = (swing_yaw(20.0, frame + 1) - swing_yaw(20.0, frame)).abs().to_degrees();
            assert!(step > 0.6, "{frame}: {step}");
        }
        assert_eq!(swing_yaw(0.0, 50), 0.0);
    }
}
