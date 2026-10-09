//! EEP environment answers in the offline demo
//! (`AURORA_DEMO_EEP_PARCEL=1|parcel|default|stale`): replays what the
//! ExtEnvironment capability answers on the grid, 0.4 s apart like the
//! Yikes log (region day, then the parcel request).
//!
//! - `1`: a dusk region day, then the agent's parcel without a day of its
//!   own: the dusk must stay (it used to fall back to the default sky).
//! - `parcel`: the parcel has its own noon day: crossfade to it over 5 s.
//! - `default`: the region has no day: Firestorm's default day asset (the
//!   built-in default sky offline, the asset cannot be fetched).
//! - `stale`: dusk region day, then answers for a parcel the agent is not on
//!   and for another region: both ignored, the dusk stays.

use super::HANDLE;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetEvent;
use glam::{Quat, Vec3};
use std::sync::{Arc, OnceLock};

/// Agent parcel of the demo (`AgentParcel` local id).
const PARCEL: i32 = 1;
/// Frames of the region answer and of the parcel answer (0.4 s later at 60 Hz).
const REGION_FRAME: u64 = 40;
const PARCEL_FRAME: u64 = 64;

fn scenario() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| std::env::var("AURORA_DEMO_EEP_PARCEL").unwrap_or_default().trim().to_owned())
}

fn arr(v: &[f32]) -> Llsd {
    Llsd::Array(v.iter().map(|x| Llsd::Real(*x as f64)).collect())
}

/// Sky frame with the sun towards `sun` (LLSettingsSky field names).
fn sky(sun: Vec3, sunlight: [f32; 3], horizon: [f32; 3], density: [f32; 3]) -> Llsd {
    let q = Quat::from_rotation_arc(Vec3::X, sun.normalize());
    let m = Quat::from_rotation_arc(Vec3::X, -sun.normalize());
    llsd_map! {
        "type" => "sky",
        "sun_rotation" => arr(&[q.x, q.y, q.z, q.w]),
        "moon_rotation" => arr(&[m.x, m.y, m.z, m.w]),
        "sunlight_color" => arr(&sunlight),
        "star_brightness" => 250.0,
        "legacy_haze" => llsd_map! {
            "blue_horizon" => arr(&horizon),
            "blue_density" => arr(&density),
            "ambient" => arr(&[0.35, 0.3, 0.33]),
            "haze_density" => 0.7,
            "haze_horizon" => 0.19,
            "density_multiplier" => 0.00018,
            "distance_multiplier" => 0.8,
        },
    }
}

/// An ExtEnvironment `environment` map with a one-key day (`None`: no day).
fn environment(parcel_id: i32, frame: Option<Llsd>) -> Llsd {
    let mut env = llsd_map! {
        "parcel_id" => parcel_id,
        "region_id" => uuid::Uuid::nil(),
        "day_length" => 14400,
        "day_offset" => 57600,
        "track_altitudes" => arr(&[1000.0, 2000.0, 3000.0]),
        "is_default" => false,
    };
    if let Some(sky) = frame {
        let mut frames = aurora_llsd::Map::new();
        frames.insert("sky".into(), sky);
        frames.insert("water".into(), llsd_map! { "type" => "water" });
        env.insert(
            "day_cycle",
            llsd_map! {
                "type" => "daycycle",
                "frames" => Llsd::Map(frames),
                "tracks" => Llsd::Array(vec![
                    Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "water"}]),
                    Llsd::Array(vec![llsd_map! {"key_keyframe" => 0.0, "key_name" => "sky"}]),
                ]),
            },
        );
    }
    env
}

fn dusk() -> Llsd {
    sky(Vec3::new(-0.97, 0.1, 0.12), [1.6, 0.75, 0.35], [0.85, 0.45, 0.35], [0.2, 0.25, 0.5])
}

fn noon() -> Llsd {
    sky(Vec3::new(0.3, 0.2, 0.93), [0.9, 0.9, 0.85], [0.25, 0.45, 0.8], [0.25, 0.45, 0.76])
}

fn answer(handle: u64, requested: i32, env: Llsd) -> NetEvent {
    NetEvent::Environment {
        handle,
        parcel_id: requested,
        environment: env,
    }
}

/// Events of this frame for the selected scenario.
pub fn events(frame: u64) -> Vec<NetEvent> {
    let s = scenario();
    if s.is_empty() || s == "0" {
        return Vec::new();
    }
    match frame {
        // the region gets the ExtEnvironment capability: an EEP region
        1 => vec![NetEvent::Capabilities {
            handle: HANDLE,
            caps: Arc::new(
                [
                    ("GetDisplayNames".to_owned(), "demo://names".to_owned()),
                    ("ExtEnvironment".to_owned(), "demo://environment".to_owned()),
                ]
                .into_iter()
                .collect(),
            ),
        }],
        REGION_FRAME => {
            log::info!("demo EEP ({s}): region answer");
            let day = (s != "default").then(dusk);
            vec![answer(HANDLE, -1, environment(-1, day))]
        }
        PARCEL_FRAME => {
            log::info!("demo EEP ({s}): parcel answer");
            match s {
                "parcel" => vec![answer(HANDLE, PARCEL, environment(PARCEL, Some(noon())))],
                "stale" => vec![
                    answer(HANDLE, PARCEL + 8, environment(PARCEL + 8, Some(noon()))),
                    answer(HANDLE + 1, -1, environment(-1, None)),
                ],
                _ => vec![answer(HANDLE, PARCEL, environment(PARCEL, None))],
            }
        }
        _ => Vec::new(),
    }
}
