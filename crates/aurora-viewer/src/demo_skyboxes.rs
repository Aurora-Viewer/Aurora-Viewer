//! Skyboxes hidden from other heights (`AURORA_DEMO_SKYBOXES=ground|skybox|
//! island|below[,off]`, docs/TESTING.md; `world::sky_visibility`). Ahead of
//! the start: a tall tower on the ground, a skybox at 300 m with a resident
//! inside, a resident flying at 220 m, a hovering physical vehicle at 250 m
//! with its rider, and far above a floating island at 3500 m with a
//! resident on it and a second skybox 200 m below it. The mode chooses where
//! our avatar starts: on the ground, in the 300 m skybox, on the island or
//! in the skybox under it; `off` turns the setting off to compare; `view`
//! puts the flycam at our avatar at frame 300, aimed at what that start
//! tests (up at the skybox from the ground, down at the vehicle from the
//! skybox, at the skybox under the island, at the island from below it).
//! The draw distance is 512 m for the run.

use super::{HANDLE, IDLE_ANIM, SEAT_ANIM, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Ground,
    Skybox,
    Island,
    Below,
}

/// Where the skybox at 300 m, the island and the skybox below it stand.
pub const SKYBOX: Vec3 = Vec3::new(160.0, 146.0, 300.0);
pub const ISLAND: Vec3 = Vec3::new(150.0, 136.0, 3500.0);
pub const BELOW: Vec3 = Vec3::new(156.0, 140.0, 3300.0);
/// The island's size.
const ISLAND_SIZE: Vec3 = Vec3::new(70.0, 70.0, 10.0);

const fn id(n: u128) -> Uuid {
    Uuid::from_u128(0xA0E0_5B0C_0000_0000_0000_0000_0000_0000 | n)
}

pub const SKY_RESIDENT: Uuid = id(1);
pub const FLYING_RESIDENT: Uuid = id(2);
pub const RIDER: Uuid = id(3);
pub const ISLAND_RESIDENT: Uuid = id(4);
pub const BELOW_RESIDENT: Uuid = id(5);

/// The scenario's mode: where it starts, the setting off, the flycam view.
#[derive(Debug, Clone, Copy)]
pub struct Mode {
    pub start: Start,
    pub off: bool,
    pub view: bool,
}

pub fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_SKYBOXES").ok()?;
        let parts: Vec<&str> = v.split(',').map(str::trim).collect();
        let start = match parts.first().copied() {
            Some("skybox") => Start::Skybox,
            Some("island") => Start::Island,
            Some("below") => Start::Below,
            _ => Start::Ground,
        };
        Some(Mode {
            start,
            off: parts.contains(&"off"),
            view: parts.contains(&"view"),
        })
    })
}

/// With `view`: where the flycam goes (from above our avatar at `me`) and
/// what it looks at.
pub fn view(me: Vec3) -> Option<(Vec3, Vec3)> {
    let mode = mode().filter(|m| m.view)?;
    let target = match mode.start {
        Start::Ground => Vec3::new(156.0, 136.0, 250.0),
        Start::Skybox => Vec3::new(148.0, 134.0, 225.0),
        Start::Island => BELOW,
        // its rim: straight up leaves the flycam no horizon
        Start::Below => ISLAND + Vec3::new(120.0, 80.0, 0.0),
    };
    let eye = me + Vec3::Z * 2.5;
    // a step back from the target, so that our avatar stays out of the view
    let back = (eye - target).with_z(0.0).normalize_or_zero() * 6.0;
    Some((eye + back, target))
}

/// The run's draw distance and setting (the demo saves its preferences:
/// set in every mode).
pub fn apply_preferences(settings: &mut crate::settings::Settings) {
    if let Some(mode) = mode() {
        settings.draw_distance = 512.0;
        settings.hide_skyboxes = !mode.off;
    }
}

/// Our avatar's start for the mode, standing (the centre of a 1.9 m body).
pub fn start() -> Option<Vec3> {
    Some(match mode()?.start {
        Start::Ground => return None,
        Start::Skybox => SKYBOX + Vec3::new(-4.0, -4.0, 1.2),
        Start::Island => ISLAND + Vec3::new(-25.0, -25.0, 0.5 * ISLAND_SIZE.z + 0.95),
        Start::Below => BELOW + Vec3::new(-4.0, -4.0, 1.2),
    })
}

fn boxp() -> aurora_prim::VolumeParams {
    shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0)
}

/// A room of `size` whose floor's centre is `at`: the floor is the root, the
/// four walls and the roof its children (the ids `base..base + 6`).
fn room(base: u32, at: Vec3, size: Vec3, color: [f32; 4]) -> Vec<ObjectUpdate> {
    let floor = prim(
        base,
        at,
        Quat::IDENTITY,
        Vec3::new(size.x, size.y, 0.5),
        boxp(),
        te(color, 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    let wall = |n: u32, offset: Vec3, scale: Vec3| {
        let mut o = prim(
            base + n,
            offset,
            Quat::IDENTITY,
            scale,
            boxp(),
            te(color, 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        o.parent_id = base;
        o
    };
    let (hx, hy, h) = (0.5 * size.x, 0.5 * size.y, size.z);
    vec![
        floor,
        wall(1, Vec3::new(hx, 0.0, 0.5 * h), Vec3::new(0.3, size.y, h)),
        wall(2, Vec3::new(-hx, 0.0, 0.5 * h), Vec3::new(0.3, size.y, h)),
        wall(3, Vec3::new(0.0, hy, 0.5 * h), Vec3::new(size.x, 0.3, h)),
        wall(4, Vec3::new(0.0, -hy, 0.5 * h), Vec3::new(size.x, 0.3, h)),
        wall(5, Vec3::new(0.0, 0.0, h), Vec3::new(size.x, size.y, 0.3)),
    ]
}

fn resident(local_id: u32, full_id: Uuid, first: &str, pos: Vec3) -> ObjectUpdate {
    let mut a = prim(
        local_id,
        pos,
        Quat::from_rotation_z(std::f32::consts::PI + 0.46),
        Vec3::new(0.45, 0.6, 1.9),
        boxp(),
        te([1.0; 4], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    a.pcode = LL_PCODE_LEGACY_AVATAR;
    a.full_id = full_id;
    a.name_values = format!("FirstName STRING RW SV {first}\nLastName STRING RW SV Sky");
    a
}

pub fn events() -> Vec<NetEvent> {
    if mode().is_none() {
        return Vec::new();
    }
    scene()
}

/// The scene's objects and animations, whatever the mode (the unit tests of
/// `world::sky_visibility` read it too).
pub fn scene() -> Vec<NetEvent> {
    let mut objects = Vec::new();
    // on the ground, 160 m tall: never floats
    let (tx, ty) = (168.0, 120.0);
    let ground = height(tx, ty);
    objects.push(prim(
        97_000,
        Vec3::new(tx, ty, ground + 80.0),
        Quat::IDENTITY,
        Vec3::new(6.0, 6.0, 160.0),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
        te([0.62, 0.66, 0.78, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    // the skybox at 300 m, violet, its resident inside
    objects.extend(room(97_010, SKYBOX, Vec3::new(16.0, 16.0, 6.0), [0.55, 0.36, 0.96, 1.0]));
    objects.push(resident(97_100, SKY_RESIDENT, "Skybox", SKYBOX + Vec3::new(2.0, 1.0, 1.2)));
    // flying alone at 220 m
    objects.push(resident(97_101, FLYING_RESIDENT, "Flying", Vec3::new(150.0, 128.0, 220.0)));
    // a physical vehicle hovering at 250 m, its rider seated on it
    let mut vehicle = prim(
        97_020,
        Vec3::new(146.0, 140.0, 250.0),
        Quat::from_rotation_z(0.5),
        Vec3::new(6.0, 3.0, 1.2),
        boxp(),
        te([0.99, 0.75, 0.25, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    vehicle.update_flags |= crate::build::edits::FLAGS_USE_PHYSICS;
    objects.push(vehicle);
    let mut rider = resident(97_102, RIDER, "Rider", Vec3::new(0.0, 0.0, 1.5));
    rider.parent_id = 97_020;
    objects.push(rider);
    // the floating island at 3500 m, green, with a hut and its resident
    objects.push(prim(
        97_030,
        ISLAND,
        Quat::IDENTITY,
        ISLAND_SIZE,
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
        te([0.35, 0.72, 0.42, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    ));
    let mut hut = prim(
        97_031,
        Vec3::new(8.0, 6.0, 0.5 * ISLAND_SIZE.z + 2.0),
        Quat::IDENTITY,
        Vec3::splat(4.0),
        boxp(),
        te([0.93, 0.85, 0.7, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    hut.parent_id = 97_030;
    objects.push(hut);
    objects.push(resident(
        97_103,
        ISLAND_RESIDENT,
        "Island",
        ISLAND + Vec3::new(-18.0, -20.0, 0.5 * ISLAND_SIZE.z + 0.95),
    ));
    // the skybox 200 m under the island, blue, its resident inside
    objects.extend(room(97_040, BELOW, Vec3::new(14.0, 14.0, 5.0), [0.3, 0.55, 0.95, 1.0]));
    objects.push(resident(97_104, BELOW_RESIDENT, "Below", BELOW + Vec3::new(2.0, 1.0, 1.2)));
    let mut ev = vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }];
    for avatar in [SKY_RESIDENT, FLYING_RESIDENT, ISLAND_RESIDENT, BELOW_RESIDENT] {
        ev.push(NetEvent::AvatarAnimations {
            avatar,
            anims: vec![(IDLE_ANIM, 1)],
            sources: Vec::new(),
        });
    }
    ev.push(NetEvent::AvatarAnimations {
        avatar: RIDER,
        anims: vec![(SEAT_ANIM, 1)],
        sources: Vec::new(),
    });
    ev
}
