//! Doppler shift (AURORA_DEMO_SOUND_DOPPLER=1, docs/TESTING.md): one
//! looping buzz passes in front of the listener, back and forth along a
//! straight line 8 m ahead, at 20 m/s, like a passing car. With
//! Preferences › Sound, "Doppler effect" at 1, it is heard higher while it
//! comes closer and drops in pitch as it goes by; at 0 its pitch stays the
//! same all the way.

use super::{DEMO_AGENT, HANDLE, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::objects::{AttachedSound, TerseUpdate};
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec2, Vec3};
use std::sync::OnceLock;
use uuid::Uuid;

pub fn active() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_DEMO_SOUND_DOPPLER").is_ok_and(|v| !v.is_empty() && v != "0"))
}

const SOUND: u128 = 0xA0E0_50CC_0000_0000_0000_0000_0000_0041;
const LOCAL_ID: u32 = 12_490;

/// The listener's spot: the open slope west of the plaza (the 3D sound
/// scenario's: run one at a time).
const START: Vec2 = Vec2::new(108.0, 150.0);
/// Distance of the line from the listener (m), half its length (m) and the
/// speed along it (m/s).
const AHEAD: f32 = 8.0;
const HALF: f32 = 50.0;
const SPEED: f32 = 20.0;
/// Height of the source over the ground (m).
const ABOVE: f32 = 1.5;

pub fn start() -> Vec3 {
    Vec3::new(START.x, START.y, floor_at(START.x, START.y) + 0.9)
}

/// The direction the camera looks at the start: north-west, away from the
/// plaza.
fn ahead() -> Vec2 {
    Vec2::new(-0.63, 0.77).normalize()
}

/// A steady buzz rich in harmonics, so the pitch is easy to follow: 110 Hz
/// and its first harmonics, whole cycles in its one second (no click at
/// the loop).
pub fn clip() -> aurora_audio::SoundClip {
    let rate = 48_000u32;
    let pcm: Vec<f32> = (0..rate as usize)
        .map(|i| {
            let t = i as f32 / rate as f32;
            (1..=8)
                .map(|n| (t * 110.0 * n as f32 * std::f32::consts::TAU).sin() / n as f32)
                .sum::<f32>()
                * 0.22
        })
        .collect();
    aurora_audio::SoundClip::from_pcm(&pcm, 1, rate)
}

pub fn sounds() -> Vec<(Uuid, aurora_audio::SoundClip)> {
    if active() {
        vec![(Uuid::from_u128(SOUND), clip())]
    } else {
        Vec::new()
    }
}

/// Where the source is `t` seconds in, on the ground plane: from the left
/// end of the line to the right end and back (a triangle wave).
pub fn along(t: f32) -> Vec2 {
    let ahead = ahead();
    let right = Vec2::new(ahead.y, -ahead.x);
    let lap = 4.0 * HALF / SPEED;
    let u = (t / lap).rem_euclid(1.0);
    // -1 at the left end, +1 at the right one
    let s = if u < 0.5 { 4.0 * u - 1.0 } else { 3.0 - 4.0 * u };
    START + ahead * AHEAD + right * (s * HALF)
}

fn at(t: f32) -> Vec3 {
    let p = along(t);
    Vec3::new(p.x, p.y, floor_at(p.x, p.y) + ABOVE)
}

/// The source, lit and labelled, at its start.
pub fn events() -> Vec<NetEvent> {
    if !active() {
        return Vec::new();
    }
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut emitter = prim(
        LOCAL_ID,
        at(0.0),
        Quat::IDENTITY,
        Vec3::splat(0.5),
        boxp,
        te([0.96, 0.62, 0.25, 1.0], 0, true, 0.0),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-sound-doppler-source"),
    );
    emitter.owner_id = DEMO_AGENT;
    emitter.sound = AttachedSound {
        id: Uuid::from_u128(SOUND),
        gain: 1.0,
        flags: 1,
        radius: 0.0,
    };
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![emitter],
    }]
}

/// Every frame: the source moved along its line, with the velocity the
/// Doppler shift reads (as a simulator's terse update carries it): along
/// the line only, so that the steps of the ground it follows (the plaza's
/// edge) do not jolt the pitch.
pub fn tick() -> Option<NetEvent> {
    if !active() {
        return None;
    }
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f32();
    let now = at(t);
    let v = (along(t + 0.02) - along(t)) / 0.02;
    Some(NetEvent::TerseUpdates {
        handle: HANDLE,
        updates: vec![TerseUpdate {
            local_id: LOCAL_ID,
            state: 0,
            is_avatar: false,
            foot_plane: None,
            position: now,
            velocity: Vec3::new(v.x, v.y, 0.0),
            acceleration: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            texture_entry: None,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_passes_in_front_back_and_forth_at_its_speed() {
        let lap = 4.0 * HALF / SPEED;
        let centre = START + ahead() * AHEAD;
        // the left end, the middle in front of the listener, the right end
        assert!((along(0.0).distance(centre) - HALF).abs() < 1e-3);
        assert!(along(lap / 4.0).distance(centre) < 1e-3);
        assert!((along(lap / 2.0).distance(centre) - HALF).abs() < 1e-3);
        assert!(along(lap / 2.0).distance(along(0.0)) > 2.0 * HALF - 1e-2);
        assert!(along(lap).distance(along(0.0)) < 1e-3);
        // on the line, AHEAD in front of the listener, at SPEED
        for i in 0..200 {
            let t = i as f32 * 0.093;
            let p = along(t);
            assert!(((p - START).dot(ahead()) - AHEAD).abs() < 1e-3);
            let u = (t / lap).rem_euclid(1.0);
            if (u - 0.5).abs() > 0.01 && u > 0.01 && u < 0.99 {
                let speed = along(t + 0.01).distance(p) / 0.01;
                assert!((speed - SPEED).abs() < 0.05, "{t}: {speed}");
            }
        }
        // the whole line inside the region
        for i in 0..=100 {
            let p = along(lap / 200.0 * i as f32);
            assert!(p.x > 0.0 && p.x < 256.0 && p.y > 0.0 && p.y < 256.0);
        }
    }
}
