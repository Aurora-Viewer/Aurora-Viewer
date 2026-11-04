//! The reverb of the surroundings (AURORA_DEMO_SOUND_REVERB=1|room|hall,
//! docs/TESTING.md): our avatar moved to the open air south-east of the
//! plaza (`room`, `hall`: inside those), with a small closed room ahead on
//! the left and a large hall further ahead, both with a doorway towards the
//! start. A clapper floats above our avatar's head and claps twice every
//! two seconds, wherever it goes: with Preferences › Sound, "Reverb from
//! the surroundings" on, the claps are almost dry in the open, ring briefly
//! in the room and long in the hall.

use super::{DEMO_AGENT, HANDLE, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::objects::{AttachedSound, ObjectUpdate, TerseUpdate};
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec2, Vec3};
use std::sync::OnceLock;
use uuid::Uuid;

/// Where our avatar starts: in the open air (`1`), in the small room
/// (`room`) or in the hall (`hall`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Open,
    Room,
    Hall,
}

pub fn place() -> Option<Place> {
    static PLACE: OnceLock<Option<Place>> = OnceLock::new();
    *PLACE.get_or_init(|| match std::env::var("AURORA_DEMO_SOUND_REVERB").ok()?.as_str() {
        "" | "0" => None,
        "room" => Some(Place::Room),
        "hall" => Some(Place::Hall),
        _ => Some(Place::Open),
    })
}

/// The scenario is on for this run.
pub fn active() -> bool {
    place().is_some()
}

const SOUND: u128 = 0xA0E0_50CC_0000_0000_0000_0000_0000_0021;
const CLAPPER: u32 = 12_430;
/// Local ids of the buildings' prims.
const ROOM_IDS: u32 = 12_440;
const HALL_IDS: u32 = 12_460;
const SIGN: u32 = 12_480;
/// The clapper above our avatar's centre (m), over its name.
const ABOVE: f32 = 1.6;

/// Where our avatar stands: the open air.
const START: Vec2 = Vec2::new(190.0, 40.0);
/// The small room: centre (metres ahead, metres to the left of the start),
/// inside (deep, wide, high) and doorway (wide, high).
/// (Large enough for the camera behind our avatar to stay inside.)
const ROOM: (Vec2, Vec3, Vec2) = (Vec2::new(8.0, 9.5), Vec3::new(7.0, 7.0, 3.0), Vec2::new(1.3, 2.3));
/// The large hall.
const HALL: (Vec2, Vec3, Vec2) = (Vec2::new(33.0, 0.0), Vec3::new(22.0, 30.0, 12.0), Vec2::new(3.5, 4.5));
/// Thickness of the walls (m).
const WALL: f32 = 0.3;

/// The look direction of the demo's start (as demo_tags.rs), and the left.
fn axes() -> (Vec2, Vec2) {
    let ahead = Vec2::new(0.8, 0.4).normalize();
    (ahead, Vec2::new(-ahead.y, ahead.x))
}

/// Our avatar's place for the scenario.
pub fn start() -> Vec3 {
    let (ahead, left) = axes();
    let p = match place() {
        // (a step towards its front wall: the camera behind stays inside)
        Some(Place::Room) => START + ahead * (ROOM.0.x + 1.0) + left * ROOM.0.y,
        // (a third of the way in, from the doorway)
        Some(Place::Hall) => START + ahead * (HALL.0.x - HALL.1.x / 6.0) + left * HALL.0.y,
        _ => START,
    };
    Vec3::new(p.x, p.y, floor_at(p.x, p.y) + 0.9)
}

/// A two-second loop: a double clap, then silence for the tail to ring.
pub fn clip() -> aurora_audio::SoundClip {
    let rate = 48_000u32;
    let mut noise = 0x6C8E_9CF5u32;
    // a band-pass around 1.5 kHz (RBJ, Q 0.9): the body of a clap
    let w = std::f32::consts::TAU * 1_500.0 / rate as f32;
    let alpha = w.sin() / (2.0 * 0.9);
    let a0 = 1.0 + alpha;
    let (b0, b2, a1, a2) = (alpha / a0, -alpha / a0, -2.0 * w.cos() / a0, (1.0 - alpha) / a0);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let pcm: Vec<f32> = (0..2 * rate as usize)
        .map(|i| {
            let t = i as f32 / rate as f32;
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = (noise >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            // each clap: three quick bursts of the palms, then a short decay
            let env = [0.0f32, 0.22]
                .iter()
                .map(|&at| {
                    let d = t - at;
                    if d < 0.0 {
                        0.0
                    } else {
                        let bursts = [0.0f32, 0.004, 0.009]
                            .iter()
                            .map(|b| if d >= *b { (-(d - b) / 0.003).exp() } else { 0.0 })
                            .sum::<f32>();
                        bursts * 0.5 + (-d / 0.02).exp() * 0.4
                    }
                })
                .sum::<f32>();
            let x = white * env;
            let y = b0 * x + b2 * x2 - a1 * y1 - a2 * y2;
            (x2, x1, y2, y1) = (x1, x, y1, y);
            (y * 1.6 + x * 0.15).clamp(-1.0, 1.0)
        })
        .collect();
    aurora_audio::SoundClip::from_pcm(&pcm, 1, rate)
}

pub fn sounds() -> [(Uuid, aurora_audio::SoundClip); 1] {
    [(Uuid::from_u128(SOUND), clip())]
}

/// The walls and the ceiling of a building around `centre` (ahead, left of
/// the start), with a doorway in the wall facing the start (`door_back`:
/// the back wall, else the right one).
fn building(first_id: u32, (centre, inside, door): (Vec2, Vec3, Vec2), door_back: bool, label: &str) -> Vec<ObjectUpdate> {
    let (ahead, left) = axes();
    let c = START + ahead * centre.x + left * centre.y;
    let facing = Quat::from_rotation_z(ahead.y.atan2(ahead.x));
    // the terrain under it: the walls reach below its lowest point, the
    // ceiling sits above its highest
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for i in 0..5 {
        for j in 0..5 {
            let p = c + ahead * inside.x * (i as f32 / 4.0 - 0.5) + left * inside.y * (j as f32 / 4.0 - 0.5);
            let z = floor_at(p.x, p.y);
            low = low.min(z);
            high = high.max(z);
        }
    }
    let (bottom, top) = (low - 0.5, high + inside.z);
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let colour = te([0.78, 0.76, 0.72, 1.0], 0, false, 0.0);
    let mut id = first_id;
    let mut parts = Vec::new();
    // a slab from `from` to `to` (local: ahead, left) and heights
    let mut slab = |from: Vec2, to: Vec2, z0: f32, z1: f32, text: &str| {
        let mid = (from + to) * 0.5;
        let size = (to - from).abs().max(Vec2::splat(WALL));
        let p = c + ahead * mid.x + left * mid.y;
        parts.push(prim(
            id,
            Vec3::new(p.x, p.y, (z0 + z1) * 0.5),
            facing,
            Vec3::new(size.x, size.y, z1 - z0),
            boxp,
            colour.clone(),
            ExtraParams::default(),
            text,
        ));
        id += 1;
    };
    let (hx, hy) = (inside.x * 0.5 + WALL * 0.5, inside.y * 0.5 + WALL * 0.5);
    let (dw, dh) = (door.x * 0.5, low + door.y);
    // front and left walls, whole
    slab(Vec2::new(hx, -hy), Vec2::new(hx, hy), bottom, top, "");
    slab(Vec2::new(-hx, hy), Vec2::new(hx, hy), bottom, top, "");
    if door_back {
        slab(Vec2::new(-hx, -hy), Vec2::new(hx, -hy), bottom, top, "");
        // the back wall, around its doorway
        slab(Vec2::new(-hx, -hy), Vec2::new(-hx, -dw), bottom, top, "");
        slab(Vec2::new(-hx, dw), Vec2::new(-hx, hy), bottom, top, "");
        slab(Vec2::new(-hx, -dw), Vec2::new(-hx, dw), dh, top, label);
    } else {
        slab(Vec2::new(-hx, -hy), Vec2::new(-hx, hy), bottom, top, "");
        // the right wall, around its doorway
        slab(Vec2::new(-hx, -hy), Vec2::new(-dw, -hy), bottom, top, "");
        slab(Vec2::new(dw, -hy), Vec2::new(hx, -hy), bottom, top, "");
        slab(Vec2::new(-dw, -hy), Vec2::new(dw, -hy), dh, top, label);
    }
    // the ceiling
    slab(Vec2::new(-hx, -hy), Vec2::new(hx, hy), top, top + WALL, "");
    parts
}

/// The clapper, the small room, the hall and a sign at the start.
pub fn events() -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut clapper = prim(
        CLAPPER,
        start() + Vec3::Z * ABOVE,
        Quat::IDENTITY,
        Vec3::splat(0.15),
        boxp,
        // (lit, to be found at night)
        te([0.55, 0.36, 0.96, 1.0], 0, true, 0.0),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-sound-reverb-clapper"),
    );
    clapper.owner_id = DEMO_AGENT;
    clapper.sound = AttachedSound {
        id: Uuid::from_u128(SOUND),
        gain: 1.0,
        flags: 1,
        radius: 0.0,
    };
    let (ahead, _) = axes();
    let sign_at = START - ahead * 1.5;
    let mut objects = vec![
        clapper,
        prim(
            SIGN,
            Vec3::new(sign_at.x, sign_at.y, floor_at(sign_at.x, sign_at.y) + 0.5),
            Quat::IDENTITY,
            Vec3::new(0.1, 0.1, 1.0),
            boxp,
            te([0.2, 0.75, 0.7, 1.0], 0, true, 0.0),
            ExtraParams::default(),
            aurora_i18n::tr!("demo-sound-reverb-open"),
        ),
    ];
    objects.extend(building(ROOM_IDS, ROOM, false, aurora_i18n::tr!("demo-sound-reverb-room")));
    objects.extend(building(HALL_IDS, HALL, true, aurora_i18n::tr!("demo-sound-reverb-hall")));
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// The clapper kept above our avatar (every frame).
pub fn tick(agent: Vec3) -> Option<NetEvent> {
    if !active() {
        return None;
    }
    Some(NetEvent::TerseUpdates {
        handle: HANDLE,
        updates: vec![TerseUpdate {
            local_id: CLAPPER,
            state: 0,
            is_avatar: false,
            foot_plane: None,
            position: agent + Vec3::Z * ABOVE,
            velocity: Vec3::ZERO,
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
    fn the_buildings_stand_apart_with_their_doorways() {
        let events = events();
        let NetEvent::ObjectUpdates { objects, .. } = &events[0] else {
            panic!("object updates");
        };
        // the clapper, the sign, then 7 prims a building (one wall in two
        // pieces around the doorway, its lintel, the ceiling)
        assert_eq!(objects.len(), 2 + 2 * 7);
        assert!(
            objects[0].sound.id == Uuid::from_u128(SOUND) && objects[0].sound.flags & 1 != 0,
            "a looping clapper"
        );
        let ids: std::collections::HashSet<u32> = objects.iter().map(|o| o.local_id).collect();
        assert_eq!(ids.len(), objects.len(), "distinct ids");
        // the start stays in the open: every wall at least 5 m away
        let s = START;
        for o in &objects[2..] {
            let half = o.scale.truncate().max_element() * 0.5;
            let d = (o.position.truncate() - s).length();
            assert!(d - half > 5.0, "{} at {d} m", o.local_id);
        }
        // the clip loops two seconds, loud enough and in range
        let c = clip();
        assert!((c.duration().as_secs_f32() - 2.0).abs() < 0.01);
    }
}
