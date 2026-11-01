//! 3D sound for headphones (AURORA_DEMO_SOUND_HRTF=1|circle|updown,
//! docs/TESTING.md): one looping sound circling the listener (the camera)
//! slowly on the level of the ears, ahead, to the right, behind, to the
//! left; one swinging ahead from below to high above and back. With
//! headphones and Preferences › Sound, "3D sound for headphones (HRTF)" on,
//! behind is heard behind and up is heard up; off, both are only panned
//! between the ears.

use super::{DEMO_AGENT, HANDLE, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::objects::{AttachedSound, TerseUpdate};
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec2, Vec3};
use std::sync::OnceLock;
use uuid::Uuid;

/// Which sounds play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Both,
    Circle,
    UpDown,
}

pub fn mode() -> Option<Mode> {
    static MODE: OnceLock<Option<Mode>> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_DEMO_SOUND_HRTF").ok()?.as_str() {
        "" | "0" => None,
        "circle" => Some(Mode::Circle),
        "updown" => Some(Mode::UpDown),
        _ => Some(Mode::Both),
    })
}

pub fn active() -> bool {
    mode().is_some()
}

/// The circling sound, then the rising and falling one.
const SOUNDS: [u128; 2] = [0xA0E0_50CC_0000_0000_0000_0000_0000_0011, 0xA0E0_50CC_0000_0000_0000_0000_0000_0012];
const LOCAL_IDS: [u32; 2] = [12_420, 12_421];

/// Distance of both sounds from the listener (m).
const RADIUS: f32 = 4.0;
/// A turn of the circling sound (s).
const CIRCLE_PERIOD: f32 = 16.0;
/// From low to high and back (s).
const SWING_PERIOD: f32 = 10.0;
/// The swing's lowest and highest elevations (degrees; the measurements
/// stop at -40).
const SWING: (f32, f32) = (-40.0, 80.0);

/// Loops rich in highs, where the ears find a direction: brushed noise
/// bursts on a soft tone for the circling one, bright clicks on a bell-like
/// tone for the swinging one.
pub fn clip(which: usize) -> aurora_audio::SoundClip {
    let rate = 48_000u32;
    let mut noise = 0x2545_F491u32 ^ which as u32;
    let pcm: Vec<f32> = (0..rate as usize)
        .map(|i| {
            let t = i as f32 / rate as f32;
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = (noise >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            if which == 0 {
                // four bursts a second, each fading over 120 ms
                let beat = t % 0.25;
                let burst = white * (-beat / 0.04).exp() * 0.35;
                let tone = (t * 330.0 * std::f32::consts::TAU).sin() * 0.08;
                burst + tone
            } else {
                // two clicks a second and a ringing tone
                let beat = t % 0.5;
                let click = white * (-beat / 0.006).exp() * 0.5;
                let bell = (t * 880.0 * std::f32::consts::TAU).sin() * (-beat * 5.0).exp() * 0.2;
                click + bell
            }
        })
        .collect();
    aurora_audio::SoundClip::from_pcm(&pcm, 1, rate)
}

pub fn sounds() -> Vec<(Uuid, aurora_audio::SoundClip)> {
    playing().map(|k| (Uuid::from_u128(SOUNDS[k]), clip(k))).collect()
}

fn playing() -> impl Iterator<Item = usize> {
    let m = mode();
    (0..2).filter(move |k| match m {
        Some(Mode::Both) => true,
        Some(Mode::Circle) => *k == 0,
        Some(Mode::UpDown) => *k == 1,
        None => false,
    })
}

/// Where our avatar stands: the open slope west of the plaza of the
/// occlusion scenario (run one at a time).
const START: Vec2 = Vec2::new(108.0, 150.0);

pub fn start() -> Vec3 {
    Vec3::new(START.x, START.y, floor_at(START.x, START.y) + 0.9)
}

/// The two emitters, lit and labelled, ahead of the start until the first
/// frame moves them.
pub fn events() -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let labels = [
        aurora_i18n::tr!("demo-sound-hrtf-circle"),
        aurora_i18n::tr!("demo-sound-hrtf-updown"),
    ];
    let colours = [[0.55, 0.36, 0.96, 1.0], [0.96, 0.62, 0.25, 1.0]];
    let objects = playing()
        .map(|k| {
            let at = START + Vec2::new(RADIUS, k as f32);
            let mut emitter = prim(
                LOCAL_IDS[k],
                Vec3::new(at.x, at.y, floor_at(at.x, at.y) + 2.0),
                Quat::IDENTITY,
                Vec3::splat(0.35),
                boxp,
                te(colours[k], 0, true, 0.0),
                ExtraParams::default(),
                labels[k],
            );
            emitter.owner_id = DEMO_AGENT;
            emitter.sound = AttachedSound {
                id: Uuid::from_u128(SOUNDS[k]),
                gain: 1.0,
                flags: 1,
                radius: 0.0,
            };
            emitter
        })
        .collect();
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// The direction the camera looks at the start (as demo_tags.rs and the
/// occlusion scenario).
fn start_ahead() -> Vec3 {
    Vec3::new(0.8, 0.4, 0.0).normalize()
}

/// Offset of each sound from the listener `t` seconds in, `ahead` being the
/// horizontal direction the circle starts from and the swing faces.
pub fn offsets(t: f32, ahead: Vec3) -> [Vec3; 2] {
    let ahead = Vec3::new(ahead.x, ahead.y, 0.0).normalize_or(Vec3::X);
    let right = ahead.cross(Vec3::Z);
    // clockwise seen from above: ahead, right, behind, left
    let a = t / CIRCLE_PERIOD * std::f32::consts::TAU;
    let circle = (ahead * a.cos() + right * a.sin()) * RADIUS;
    let (lo, hi) = (SWING.0.to_radians(), SWING.1.to_radians());
    let s = 0.5 - 0.5 * (t / SWING_PERIOD * std::f32::consts::TAU).cos();
    let e = lo + (hi - lo) * s;
    let swing = (ahead * e.cos() + Vec3::Z * e.sin()) * RADIUS;
    [circle, swing]
}

/// The lowest point of the swing stays above the ground: under it, the
/// terrain would muffle the sound (sound occlusion).
fn above_ground(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z.max(floor_at(p.x, p.y) + 0.3))
}

/// Every frame: the emitters placed around the listener (the camera, at
/// `listener` in region coordinates), from the start's view.
pub fn tick(listener: Vec3) -> Option<NetEvent> {
    if !active() {
        return None;
    }
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f32();
    let ahead = start_ahead();
    let now = offsets(t, ahead);
    let soon = offsets(t + 0.05, ahead);
    let updates: Vec<TerseUpdate> = playing()
        .map(|k| TerseUpdate {
            local_id: LOCAL_IDS[k],
            state: 0,
            is_avatar: false,
            foot_plane: None,
            position: above_ground(listener + now[k]),
            velocity: (above_ground(listener + soon[k]) - above_ground(listener + now[k])) / 0.05,
            acceleration: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            texture_entry: None,
        })
        .collect();
    (!updates.is_empty()).then_some(NetEvent::TerseUpdates { handle: HANDLE, updates })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sounds_circle_and_swing_around_the_listener() {
        let ahead = Vec3::new(0.0, 1.0, 0.3);
        let at = |t: f32| offsets(t, ahead);
        // the circle: ahead, right, behind, left, level with the ears
        let q = CIRCLE_PERIOD / 4.0;
        for (k, want) in [Vec3::Y, Vec3::X, -Vec3::Y, -Vec3::X].into_iter().enumerate() {
            assert!(
                (at(q * k as f32)[0] - want * RADIUS).length() < 1e-3,
                "{k}: {}",
                at(q * k as f32)[0]
            );
        }
        // the swing: low ahead, then high, always ahead
        assert!((at(0.0)[1].z / RADIUS - SWING.0.to_radians().sin()).abs() < 1e-4);
        assert!((at(SWING_PERIOD / 2.0)[1].z / RADIUS - SWING.1.to_radians().sin()).abs() < 1e-4);
        for i in 0..100 {
            let o = at(i as f32 * 0.37);
            assert!((o[0].length() - RADIUS).abs() < 1e-3 && (o[1].length() - RADIUS).abs() < 1e-3);
            assert!(o[0].z.abs() < 1e-4 && o[1].y >= -1e-4 && o[1].x.abs() < 1e-4);
        }
    }
}
