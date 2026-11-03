//! Sounds behind walls (AURORA_DEMO_SOUND_OCCLUSION=1, docs/TESTING.md):
//! our avatar moved to an open spot west of the plaza, three looping
//! sources ahead of it, each with its own pitch. Left, one in the open; straight ahead, one behind a wall; right,
//! one shut in a closed box. Walk around them to hear the wall and the box
//! muffle the sound, and Preferences › Sound, "Muffle sounds behind walls",
//! turn it off.

use super::{DEMO_AGENT, HANDLE, floor_at, prim, shape, te};
use aurora_net::{NetEvent, objects::AttachedSound};
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec2, Vec3};
use uuid::Uuid;

/// The scenario is on for this run.
pub fn active() -> bool {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ACTIVE.get_or_init(|| std::env::var("AURORA_DEMO_SOUND_OCCLUSION").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// The three sounds: open, behind the wall, in the box.
const SOUNDS: [u128; 3] = [
    0xA0E0_50CC_0000_0000_0000_0000_0000_0001,
    0xA0E0_50CC_0000_0000_0000_0000_0000_0002,
    0xA0E0_50CC_0000_0000_0000_0000_0000_0003,
];

/// Base pitch of each sound (Hz).
const PITCH: [f32; 3] = [220.0, 165.0, 275.0];

/// A one-second loop rich in highs, so that the low-pass is heard: two
/// sawtooth notes with a burst of noise at each start.
pub fn clip(which: usize) -> aurora_audio::SoundClip {
    let rate = 48_000u32;
    let f = PITCH[which.min(2)];
    let mut noise = 0x9E37_79B9u32 ^ which as u32;
    let pcm: Vec<f32> = (0..rate as usize)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let note = t % 0.5;
            let pitch = if t < 0.5 { f } else { f * 1.5 };
            let saw = 2.0 * (note * pitch).fract() - 1.0;
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = (noise >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            let tick = if note < 0.03 { white * (1.0 - note / 0.03) } else { 0.0 };
            (saw * (-note * 6.0).exp() * 0.22 + tick * 0.2).clamp(-1.0, 1.0)
        })
        .collect();
    aurora_audio::SoundClip::from_pcm(&pcm, 1, rate)
}

pub fn sounds() -> [(Uuid, aurora_audio::SoundClip); 3] {
    std::array::from_fn(|k| (Uuid::from_u128(SOUNDS[k]), clip(k)))
}

/// Where our avatar stands, clear of the plaza's fixtures.
const START: Vec2 = Vec2::new(108.0, 150.0);

/// Our avatar's place for the scenario.
pub fn start() -> Vec3 {
    Vec3::new(START.x, START.y, floor_at(START.x, START.y) + 0.9)
}

/// The emitters, the wall and the box.
pub fn events() -> Vec<NetEvent> {
    let start = START;
    // the look direction of the demo's start (as demo_tags.rs)
    let ahead = Vec2::new(0.8, 0.4).normalize();
    let left = Vec2::new(-ahead.y, ahead.x);
    let facing = Quat::from_rotation_z(ahead.y.atan2(ahead.x));
    let at = |a: f32, l: f32, up: f32| {
        let p = start + ahead * a + left * l;
        Vec3::new(p.x, p.y, floor_at(p.x, p.y) + up)
    };
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let labels = [
        aurora_i18n::tr!("demo-sound-occlusion-open"),
        aurora_i18n::tr!("demo-sound-occlusion-wall"),
        aurora_i18n::tr!("demo-sound-occlusion-box"),
    ];
    let colours = [[0.55, 0.36, 0.96, 1.0], [0.2, 0.75, 0.7, 1.0], [0.96, 0.62, 0.25, 1.0]];
    let places = [at(7.0, 4.5, 1.0), at(10.0, 0.0, 1.0), at(7.0, -4.5, 1.0)];
    let mut objects = Vec::new();
    for k in 0..3 {
        // (the one in the box has its label on the box)
        let text = if k == 2 { "" } else { labels[k] };
        let mut emitter = prim(
            12_400 + k as u32,
            places[k],
            facing,
            Vec3::splat(0.4),
            boxp,
            // (lit, to be found at night)
            te(colours[k], 0, true, 0.0),
            ExtraParams::default(),
            text,
        );
        emitter.owner_id = DEMO_AGENT;
        emitter.sound = AttachedSound {
            id: Uuid::from_u128(SOUNDS[k]),
            gain: 1.0,
            flags: 1,
            radius: 0.0,
        };
        objects.push(emitter);
    }
    // a wall halfway to the middle one, wide enough to hide it from the start
    objects.push(prim(
        12_410,
        at(6.5, 0.0, 1.8),
        facing,
        Vec3::new(0.3, 5.0, 3.6),
        boxp,
        te([0.85, 0.85, 0.88, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    // a closed box around the right one
    objects.push(prim(
        12_411,
        places[2],
        facing,
        Vec3::splat(1.6),
        boxp,
        te([0.6, 0.6, 0.64, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        labels[2],
    ));
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}
