//! AURORA_DEMO_NEON_ROOM=1: a closed room on the open patch at (100, 100)
//! that shows the traced global illumination (AURORA_RT_GI=1) against the
//! Legacy Viewer's light:
//! - a wide doorway in its south wall lets the sky in: its light bounces
//!   into the room, whose far corners stay in the shade of the roof (the
//!   probes' sight of the sky);
//! - its north wall is red: the floor and the sphere near it take its
//!   colour;
//! - a strip of pink PBR emissive along its east wall and a cyan tube in a
//!   corner light the walls around them (no light of their own: only the
//!   traced light sees an emissive face);
//! - a covered porch outside the doorway: shade under a roof, in the open.
//!
//! The avatar starts inside, at the west end, facing the pink strip. With
//! AURORA_GI=0 the bounces stay but the room takes the sky's whole ambient;
//! with AURORA_RT_GI=0, the Legacy Viewer's light, the neons light nothing.
//!
//! `closed`: the doorway walled up, the neons the room's only light; `dark`:
//! walled up and the neons plain, no light at all: the traced light must
//! leave the room black (no light the bounces make up).

use super::{height, prim, shape, te};
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use uuid::Uuid;

/// The pink strip and the cyan tube.
pub const NEON: [Uuid; 2] = [
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0060),
    Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0061),
];
/// Local ids taken by `objects` (the open room takes one less).
pub const COUNT: u32 = 19;

/// The room's center on the ground, its length along X, width along Y and
/// height (m).
const CENTER: (f32, f32) = (100.0, 100.0);
const LENGTH: f32 = 9.0;
const WIDTH: f32 = 7.0;
const HEIGHT: f32 = 3.4;
/// The doorway in the south wall: from `DOOR.0` to `DOOR.1` along X (from
/// the center), `DOOR_HEIGHT` high.
const DOOR: (f32, f32) = (-0.5, 2.5);
const DOOR_HEIGHT: f32 = 2.4;
/// How far the walls and the porch's floor go under the floor level, so
/// that the ground's slope shows nowhere under them.
const FOUNDATION: f32 = 1.0;

/// What the room is built with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    /// `1`: the doorway open, the neons lit.
    Open,
    /// `closed`: the doorway walled up, the neons lit.
    Closed,
    /// `dark`: the doorway walled up, the neons plain.
    Dark,
}

impl Variant {
    fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "1" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

fn variant() -> Option<Variant> {
    static VARIANT: std::sync::LazyLock<Option<Variant>> = std::sync::LazyLock::new(|| {
        if !std::env::var("AURORA_DEMO").is_ok_and(|v| v.trim() == "1") {
            return None;
        }
        std::env::var("AURORA_DEMO_NEON_ROOM").ok().as_deref().and_then(Variant::parse)
    });
    *VARIANT
}

pub fn enabled() -> bool {
    variant().is_some()
}

/// The room's floor level: a metre over the ground at its center, clear of
/// the ground's slope under its east end (the plaza is far: the ground is
/// the terrain).
fn floor() -> f32 {
    height(CENTER.0, CENTER.1) + FOUNDATION
}

/// The top of the room's or the porch's floor under (x, y), for the demo's
/// offline walking (demo.rs `floor_at`): the avatar stands on it.
pub fn floor_under(x: f32, y: f32) -> Option<f32> {
    if !enabled() {
        return None;
    }
    let (dx, dy) = (x - CENTER.0, y - CENTER.1);
    let (hl, hw) = (LENGTH * 0.5, WIDTH * 0.5);
    let room = dx.abs() <= hl && dy.abs() <= hw;
    let porch = (dx - 1.0).abs() <= 2.0 && (dy + hw + 1.6).abs() <= 1.5;
    (room || porch).then(|| floor() + 0.05)
}

/// A point `x` along the room's length, `y` across, `z` over its floor (from
/// its center).
fn at(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(CENTER.0 + x, CENTER.1 + y, floor() + z)
}

/// Where the avatar starts, and where it looks: at the west end, facing the
/// pink strip.
pub fn start() -> (Vec3, Vec3) {
    // (on the slab, its top 5 cm over the floor level)
    (at(-1.0, 0.0, 0.05 + 0.84), Vec3::X)
}

/// The neon materials: dark faces that send their colour (no light of their
/// own), as a resident's emissive strip.
pub fn materials() -> [(Uuid, aurora_assets::PbrMaterial); 2] {
    let neon = |emissive_factor| aurora_assets::PbrMaterial {
        base_color_factor: [0.05, 0.05, 0.05, 1.0],
        metallic_factor: 0.0,
        roughness_factor: 0.4,
        emissive_factor,
        ..Default::default()
    };
    [(NEON[0], neon([1.0, 0.12, 0.55])), (NEON[1], neon([0.1, 0.85, 1.0]))]
}

/// The room, the porch and what stands in them, from local id `first`.
pub fn objects(first: u32) -> Vec<ObjectUpdate> {
    objects_of(first, variant().unwrap_or(Variant::Open))
}

fn objects_of(first: u32, variant: Variant) -> Vec<ObjectUpdate> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let cylinder = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0);
    let white = [0.78, 0.76, 0.72, 1.0];
    let red = [0.75, 0.12, 0.1, 1.0];
    let (hl, hw) = (LENGTH * 0.5, WIDTH * 0.5);
    let wall_east = hl - (hl - DOOR.1) * 0.5;
    let wall_west = (DOOR.0 - hl) * 0.5;
    // (the walls stand on their foundation)
    let (wall_z, wall_h) = ((HEIGHT - FOUNDATION) * 0.5, HEIGHT + FOUNDATION);
    // (position, scale, colour) of the plain boxes
    let boxes = [
        // floor and roof
        (at(0.0, 0.0, 0.0), Vec3::new(LENGTH, WIDTH, 0.1), white),
        (at(0.0, 0.0, HEIGHT), Vec3::new(LENGTH, WIDTH, 0.2), white),
        // west (behind the camera) and east walls
        (at(-hl, 0.0, wall_z), Vec3::new(0.2, WIDTH, wall_h), white),
        (at(hl, 0.0, wall_z), Vec3::new(0.2, WIDTH, wall_h), white),
        // the red north wall
        (at(0.0, hw, wall_z), Vec3::new(LENGTH, 0.2, wall_h), red),
        // the south wall on both sides of the doorway, and over it
        (at(wall_west, -hw, wall_z), Vec3::new(DOOR.0 + hl, 0.2, wall_h), white),
        (at(wall_east, -hw, wall_z), Vec3::new(hl - DOOR.1, 0.2, wall_h), white),
        (
            at((DOOR.0 + DOOR.1) * 0.5, -hw, (DOOR_HEIGHT + HEIGHT) * 0.5),
            Vec3::new(DOOR.1 - DOOR.0, 0.2, HEIGHT - DOOR_HEIGHT),
            white,
        ),
        // the doorway's sill, then the porch: its floor, its roof and two
        // posts
        (
            at((DOOR.0 + DOOR.1) * 0.5, -hw, -FOUNDATION * 0.5),
            Vec3::new(DOOR.1 - DOOR.0, 0.2, FOUNDATION + 0.1),
            white,
        ),
        (at(1.0, -hw - 1.6, -FOUNDATION * 0.5), Vec3::new(4.0, 3.0, FOUNDATION + 0.1), white),
        (at(1.0, -hw - 1.6, 2.8), Vec3::new(4.0, 3.0, 0.15), white),
        (at(-0.85, -hw - 2.95, 1.4), Vec3::new(0.15, 0.15, 2.8), white),
        (at(2.85, -hw - 2.95, 1.4), Vec3::new(0.15, 0.15, 2.8), white),
        // a pale box near the doorway
        (at(2.2, -1.6, 0.45), Vec3::splat(0.8), white),
    ];
    let mut objects = Vec::with_capacity(COUNT as usize);
    let mut id = first;
    for (pos, scale, color) in boxes {
        objects.push(prim(
            id,
            pos,
            Quat::IDENTITY,
            scale,
            boxp,
            te(color, 0, false, 0.0),
            ExtraParams::default(),
            "",
        ));
        id += 1;
    }
    // a pale sphere by the red wall
    objects.push(prim(
        id,
        at(1.0, hw - 1.3, 0.55),
        Quat::IDENTITY,
        Vec3::splat(1.0),
        sphere,
        te(white, 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    id += 1;
    // the pink strip along the east wall, the cyan tube standing in the
    // north-east corner
    // (`dark`: plain dark faces, sending nothing)
    let neon = |id, pos, scale, shape, mat| {
        let lit = variant != Variant::Dark;
        prim(
            id,
            pos,
            Quat::IDENTITY,
            scale,
            shape,
            te(if lit { [1.0; 4] } else { [0.05, 0.05, 0.05, 1.0] }, 0, false, 0.0),
            ExtraParams {
                render_materials: if lit { (0..6).map(|f| (f, mat)).collect() } else { Vec::new() },
                ..Default::default()
            },
            "",
        )
    };
    objects.push(neon(id, at(hl - 0.15, 0.0, 2.6), Vec3::new(0.06, 5.0, 0.2), boxp, NEON[0]));
    objects.push(neon(
        id + 1,
        at(hl - 0.5, hw - 0.5, 0.85),
        Vec3::new(0.12, 0.12, 1.6),
        cylinder,
        NEON[1],
    ));
    id += 2;
    // the room's probe, an invisible box filling it
    objects.push(prim(
        id,
        at(0.0, 0.0, HEIGHT * 0.5),
        Quat::IDENTITY,
        Vec3::new(LENGTH - 0.2, WIDTH - 0.2, HEIGHT - 0.2),
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
    id += 1;
    // `closed` and `dark`: the doorway walled up, in the wall's plane
    if variant != Variant::Open {
        objects.push(prim(
            id,
            at((DOOR.0 + DOOR.1) * 0.5, -hw, DOOR_HEIGHT * 0.5),
            Quat::IDENTITY,
            Vec3::new(DOOR.1 - DOOR.0, 0.2, DOOR_HEIGHT),
            boxp,
            te(white, 0, false, 0.0),
            ExtraParams::default(),
            "",
        ));
    }
    objects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neon_room_starts_inside_facing_the_strip() {
        for variant in [Variant::Open, Variant::Closed, Variant::Dark] {
            let objects = objects_of(1, variant);
            // the walled-up doorway takes the last id
            assert_eq!(objects.len() as u32, COUNT - u32::from(variant == Variant::Open));
            // the local ids follow one another from `first`
            assert!(objects.iter().enumerate().all(|(k, o)| o.local_id == 1 + k as u32));
            // the neons send light but in the dark room
            let emissive = objects.iter().filter(|o| !o.extra.render_materials.is_empty()).count();
            assert_eq!(emissive, if variant == Variant::Dark { 0 } else { 2 });
        }
        assert_eq!(Variant::parse(" closed "), Some(Variant::Closed));
        assert_eq!(Variant::parse("0"), None);
        let (pos, look) = start();
        let (hl, hw) = (LENGTH * 0.5, WIDTH * 0.5);
        // inside the walls, under the roof, facing east
        assert!((pos.x - CENTER.0).abs() < hl - 1.0 && (pos.y - CENTER.1).abs() < hw - 1.0);
        assert!(pos.z > floor() && pos.z < floor() + HEIGHT);
        assert!(look.dot(Vec3::X) > 0.99);
        // (`floor_under` is off unless the scenario is on)
        assert_eq!(floor_under(pos.x, pos.y), None);
        // the south wall's pieces and the doorway cover the whole wall
        assert!((DOOR.0 + hl) + (DOOR.1 - DOOR.0) + (hl - DOOR.1) - LENGTH < 1e-5);
        // two emissive materials, sending light, with a dark base
        for (_, m) in materials() {
            assert!(m.emissive_factor.iter().any(|&c| c > 0.8));
            assert!(m.base_color_factor[..3].iter().all(|&c| c < 0.1));
        }
    }
}
