//! The plaza's mirror, and AURORA_DEMO_MIRROR=1: a mirror of its own on
//! the open patch at (100, 100), the avatar 3 m in front of it, facing it,
//! and blended and shiny things between the two (#3479):
//! - on the left, a skin-coloured sphere under a lace shell (a blended
//!   texture of opaque threads, half-covered borders and holes), as sheer
//!   lingerie over a body;
//! - on the right, a pane of red glass at half alpha, over a black shiny
//!   sphere, as latex;
//! - a closed room around them, lit by a warm lamp, with its own probe.
//!
//! The Legacy Viewer's mirror (a hero probe) draws the whole scene, the
//! blended faces included, lit as the main view: the lace and the glass show
//! in the mirror as they do in front of it, the black sphere stays black and
//! the room keeps its dim warm light, the sun stopped at the roof.

use super::{floor_at, height, prim, shape, te};
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Mat3, Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

pub const SKIN: Uuid = Uuid::from_u128(0xde19_3e71_0000_4000_8000_0000_0000_0001);
pub const LACE: Uuid = Uuid::from_u128(0xde19_3e71_0000_4000_8000_0000_0000_0002);
/// Local ids taken by `objects`.
pub const COUNT: u32 = 14;

pub fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::var("AURORA_DEMO").is_ok_and(|v| v.trim() == "1") && std::env::var("AURORA_DEMO_MIRROR").is_ok_and(|v| v.trim() == "1")
    });
    *ENABLED
}

/// The plaza's mirror: the center and rotation of its probe and panel (the
/// probe's plane along its local +Z).
pub fn placement() -> (Vec3, Quat) {
    let (cx, cy) = (140.0, 130.0);
    let rot = Quat::from_rotation_z(-1.2) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    (Vec3::new(cx + 2.0, cy - 9.0, height(cx, cy) + 2.0), rot)
}

/// A mirror as built in SL: an invisible box reflection probe flagged as
/// mirror (its plane through the center, along its +Z), and a shiny 10 cm
/// panel of `color` through that plane.
pub fn mirror(first: u32, center: Vec3, rot: Quat, color: [f32; 4]) -> [ObjectUpdate; 2] {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let probe = aurora_prim::extra::ReflectionProbeParams {
        ambiance: 0.0,
        clip_distance: 0.0,
        flags: 0x1 | 0x4,
    };
    [
        prim(
            first,
            center,
            rot,
            Vec3::new(3.4, 3.0, 1.0),
            boxp,
            te([1.0, 1.0, 1.0, 0.0], 0, false, 0.0),
            ExtraParams {
                reflection_probe: Some(probe),
                ..Default::default()
            },
            "",
        ),
        prim(
            first + 1,
            center,
            rot,
            Vec3::new(3.0, 2.6, 0.1),
            boxp,
            te(color, 3, false, 0.0),
            ExtraParams::default(),
            "",
        ),
    ]
}

/// The scenario's mirror, 4 m east of (100, 100), facing west, upright
/// (its local Y up).
fn axes() -> (Vec3, Vec3, Quat) {
    let (x, y) = (104.0, 100.0);
    // the room's floor half a metre over the ground, clear of its slope
    let center = Vec3::new(x, y, floor_at(x, y) + 0.5 + 1.6);
    let front = -Vec3::X;
    let rot = Quat::from_mat3(&Mat3::from_cols(-Vec3::Y, Vec3::Z, front));
    (center, front, rot)
}

/// Where the avatar starts, and where it looks: 3 m in front of the mirror,
/// facing it.
pub fn start() -> (Vec3, Vec3) {
    let (center, front, _) = axes();
    let at = center + front * 3.0;
    // on the room's floor (its slab's top 5 cm over the floor level)
    (Vec3::new(at.x, at.y, center.z - 1.6 + 0.05 + 0.84), -front)
}

pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if !enabled() {
        return None;
    }
    match *id {
        SKIN => Some(solid([224, 172, 140, 255])),
        LACE => Some(lace()),
        _ => None,
    }
}

fn solid(rgba: [u8; 4]) -> (Vec<u8>, u32, u32) {
    const N: usize = 16;
    (rgba.repeat(N * N), N as u32, N as u32)
}

/// Black lace: threads every 8 texels (opaque), a half-covered border on
/// each side of them, holes elsewhere.
fn lace() -> (Vec<u8>, u32, u32) {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let edge = |v: usize| match v % 8 {
                0 => 255,
                1 | 7 => 128,
                _ => 0,
            };
            let a: u8 = edge(x).max(edge(y)).max(edge(x + y));
            let i = (y * N + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[20, 16, 18, a]);
        }
    }
    (rgba, N as u32, N as u32)
}

fn faces(texture: Uuid, color: [f32; 4]) -> Arc<aurora_prim::te::TextureEntry> {
    let mut entry = (*te(color, 0, false, 0.0)).clone();
    for face in &mut entry.faces {
        face.texture = texture;
    }
    Arc::new(entry)
}

/// The mirror and the things around it, from local id `first`.
pub fn objects(first: u32) -> Vec<ObjectUpdate> {
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let (center, front, rot) = axes();
    // the avatar's left, looking at the mirror
    let left = Vec3::Z.cross(-front);
    // `distance` in front of the mirror, `offset` to the avatar's left, `up`
    // over the room's floor (level: the mirror's)
    let floor = center.z - 1.6;
    let at = |distance: f32, offset: f32, up: f32| {
        let p = center + front * distance + left * offset;
        Vec3::new(p.x, p.y, floor + up)
    };
    let facing = Quat::from_rotation_arc(Vec3::Y, front);
    let lace = at(1.5, 0.9, 1.3);
    // a dark face, as most mirrors are built: the room's lamp would light a
    // pale one over its reflection
    let mut objects = mirror(first, center, rot, [0.05, 0.05, 0.05, 1.0]).to_vec();
    objects.extend([
        prim(
            first + 2,
            lace,
            Quat::IDENTITY,
            Vec3::splat(0.6),
            sphere,
            faces(SKIN, [1.0; 4]),
            ExtraParams::default(),
            "",
        ),
        prim(
            first + 3,
            lace,
            Quat::IDENTITY,
            Vec3::splat(0.64),
            sphere,
            faces(LACE, [1.0; 4]),
            ExtraParams::default(),
            "",
        ),
        // black, shininess high (latex)
        prim(
            first + 4,
            at(1.5, -0.9, 0.4),
            Quat::IDENTITY,
            Vec3::splat(0.5),
            sphere,
            te([0.02, 0.02, 0.02, 1.0], 3, false, 0.0),
            ExtraParams::default(),
            "",
        ),
        // red glass at half alpha
        prim(
            first + 5,
            at(1.5, -0.9, 1.3),
            facing,
            Vec3::new(0.7, 0.04, 0.9),
            boxp,
            te([0.9, 0.1, 0.1, 0.5], 0, false, 0.0),
            ExtraParams::default(),
            "",
        ),
    ]);
    // a closed room around them, lit by a warm lamp and its own probe: the
    // sun stops at its roof, its probe holds its dim warm light, as the
    // rooms of the screenshots in #3479
    let wall = te([0.55, 0.5, 0.45, 1.0], 0, false, 0.0);
    let (length, width, height) = (8.6, 6.0, 3.6);
    let mid = length * 0.5 - 0.6;
    let side = Quat::from_rotation_arc(Vec3::Y, left);
    let room = [
        // behind the camera, behind the mirror, left, right
        (at(8.0, 0.0, height * 0.5), facing, Vec3::new(width, 0.2, height)),
        (at(-0.6, 0.0, height * 0.5), facing, Vec3::new(width, 0.2, height)),
        (at(mid, width * 0.5, height * 0.5), side, Vec3::new(length, 0.2, height)),
        (at(mid, -width * 0.5, height * 0.5), side, Vec3::new(length, 0.2, height)),
        // roof and floor
        (at(mid, 0.0, height), facing, Vec3::new(width, length, 0.2)),
        (at(mid, 0.0, 0.0), facing, Vec3::new(width, length, 0.1)),
    ];
    let mut id = first + 6;
    for (pos, rot, scale) in room {
        objects.push(prim(id, pos, rot, scale, boxp, wall.clone(), ExtraParams::default(), ""));
        id += 1;
    }
    // the lamp under the roof, between the avatar and the mirror: what faces
    // the mirror is lit
    objects.push(prim(
        id,
        at(1.8, 0.0, height - 0.4),
        Quat::IDENTITY,
        Vec3::splat(0.25),
        sphere,
        te([1.0, 0.8, 0.55, 1.0], 0, true, 0.0),
        ExtraParams {
            light: Some(aurora_prim::extra::LightParams {
                color: [1.0, 0.55, 0.25, 1.0],
                radius: 10.0,
                cutoff: 0.0,
                falloff: 0.75,
            }),
            ..Default::default()
        },
        "",
    ));
    // the room's probe, an invisible box filling it
    objects.push(prim(
        id + 1,
        at(mid, 0.0, height * 0.5),
        facing,
        Vec3::new(width - 0.2, length - 0.2, height - 0.2),
        boxp,
        te([1.0, 1.0, 1.0, 0.0], 0, false, 0.0),
        ExtraParams {
            reflection_probe: Some(aurora_prim::extra::ReflectionProbeParams {
                ambiance: 0.0,
                clip_distance: 0.0,
                flags: 0x1,
            }),
            ..Default::default()
        },
        "",
    ));
    objects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_fixture_stands_in_front_of_the_mirror() {
        let (lace, w, h) = lace();
        assert_eq!(lace.len(), (w * h * 4) as usize);
        // threads, half-covered borders and holes
        for a in [0, 128, 255] {
            assert!(lace.chunks(4).any(|p| p[3] == a));
        }
        let (center, front, rot) = axes();
        // upright, its plane facing the avatar
        assert!((rot * Vec3::Z).distance(front) < 1e-5 && (rot * Vec3::Y).distance(Vec3::Z) < 1e-5);
        let (at, look) = start();
        // on the plaza's side, facing the mirror
        assert!(front.dot(at - center) > 2.5);
        assert!(look.dot(center - at) > 0.0);
        assert_eq!(objects(1).len(), COUNT as usize);
    }
}
