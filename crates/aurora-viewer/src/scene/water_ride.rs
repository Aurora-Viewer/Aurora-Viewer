//! Which objects ride the wind's sea, and the sea under their hull
//! (movement/water_ride.rs keeps and eases their rides). A physical linkset
//! root floating at the region water rides: its linkset's box is its hull, the
//! sea is sampled at its centre, bow, stern and both sides through the same
//! `surface_level` as our avatar's float and the visual contacts, so it
//! meets the drawn waves, the sheltered water included. What stands still
//! (a house, a pier) never rides: it shelters the water instead
//! (water_shelter.rs).

use super::Scene;
use crate::world::World;
use aurora_render::water_sea::Sea;
use aurora_render::{WaterEffects, WaterGround};
use glam::{Vec2, Vec3};
use std::time::Instant;

/// How far from the camera objects ride (m), and how many at most.
const RANGE: f32 = 96.0;
const MAX_RIDERS: usize = 32;
/// How often the candidates are looked for again (s).
const SCAN: f32 = 0.5;

/// Advance the rides to `time` on `sea`, around `eye`. Objects that start
/// or stop riding are touched, so the scene places them every frame while
/// they ride (`Object::render.rides_water`, sync_sets::moves_by_itself)
/// and once more at rest. `held`: the linksets selected in Edit (their
/// roots' indices), which ease back to flat while they are, so the edit
/// tools stay on what they move.
pub fn update(world: &mut World, settings: WaterEffects, sea: &Sea, ground: &WaterGround, time: f32, eye: Vec3, held: &[usize]) {
    if !settings.enabled || !settings.waves {
        for id in world.water_ride.clear() {
            set_riding(world, &id, false);
        }
        return;
    }
    if world.water_ride.candidates.is_empty() || !(0.0..SCAN).contains(&(time - world.water_ride.scan_at)) {
        world.water_ride.scan_at = time;
        let mut near: Vec<(f32, usize)> = world
            .objects
            .iter()
            .filter(|(_, o)| {
                o.parent_id == 0 && !o.is_avatar() && !o.is_attachment() && o.update_flags & crate::build::edits::FLAGS_USE_PHYSICS != 0
            })
            .filter_map(|(i, _)| {
                let (p, _, _) = Scene::object_transform_raw(world, i, Instant::now(), 0, None)?;
                let d = p.truncate().distance_squared(eye.truncate());
                (d < RANGE * RANGE).then_some((d, i))
            })
            .collect();
        near.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        world.water_ride.candidates = near
            .into_iter()
            .take(MAX_RIDERS)
            .filter_map(|(_, i)| {
                let (centre, half) = linkset_hull(world, i)?;
                Some((i, centre, half))
            })
            .collect();
    }
    let now = Instant::now();
    let mut targets = Vec::new();
    let mut hulls = Vec::new();
    for &(i, centre, half) in &world.water_ride.candidates {
        if held.contains(&i) {
            continue;
        }
        let Some(o) = world.objects.get(i) else {
            continue;
        };
        let Some((p, r, _)) = Scene::object_transform_raw(world, i, now, 0, None) else {
            continue;
        };
        let Some([_, level]) = ground.sample(p.truncate()) else {
            continue;
        };
        // the hull: the whole linkset's box (a boat whose root is a small
        // prim still rides on its length and beam)
        let hull = p + r * centre;
        let reach_down = (r * Vec3::X).z.abs() * half.x + (r * Vec3::Y).z.abs() * half.y + (r * Vec3::Z).z.abs() * half.z;
        // floating: its bottom at the water or a little under it, and its
        // centre not deep down (a wreck, a submarine)
        if hull.z - reach_down > level + 0.5 || hull.z < level - 4.0 {
            continue;
        }
        let forward = (r * Vec3::X).truncate().normalize_or(Vec2::X);
        let side = forward.perp();
        let sea_at = |q: Vec2| {
            let ground_z = ground.sample(q).map_or(level - 20.0, |g| g[0]);
            let at = ground.sea_point(q, sea, level - ground_z);
            aurora_render::water_effects::surface_level(q, time, settings, sea, level, &at) - level
        };
        let c = hull.truncate();
        let (bow, stern) = (sea_at(c + forward * half.x), sea_at(c - forward * half.x));
        let (port, starboard) = (sea_at(c + side * half.y), sea_at(c - side * half.y));
        let heave = (sea_at(c) * 2.0 + bow + stern + port + starboard) / 6.0;
        let tilt = crate::movement::water_ride::hull_tilt(
            forward.extend(0.0),
            side.extend(0.0),
            2.0 * half.x,
            2.0 * half.y,
            bow - stern,
            port - starboard,
        );
        targets.push((o.full_id, heave, tilt));
        hulls.push((o.full_id, p, r, centre, half));
    }
    let keep: Vec<_> = targets.iter().map(|t| t.0).collect();
    // what leaves the water (or is selected) eases back to flat first
    for id in world.water_ride.release(&keep, time) {
        set_riding(world, &id, false);
    }
    for (id, heave, tilt) in targets {
        if world.water_ride.follow(id, time, heave, tilt) {
            set_riding(world, &id, true);
        }
    }
    let on = standing_on_decks(world, &hulls, now);
    world.water_ride.stand(&on, time);
}

/// The avatars standing on the deck of a riding hull (`hulls`: id, root
/// position and rotation, the hull's centre and half size in the root's
/// frame), and how far the deck lifts their feet now. The simulator keeps
/// them on its flat deck; drawn, they ride with it (a seated one already
/// does, as a child of the linkset).
fn standing_on_decks(world: &World, hulls: &[(uuid::Uuid, Vec3, glam::Quat, Vec3, Vec3)], now: Instant) -> Vec<(uuid::Uuid, f32)> {
    if hulls.is_empty() {
        return Vec::new();
    }
    let mut on = Vec::new();
    for (i, o) in world.objects.iter() {
        if !o.is_avatar() || o.parent_id != 0 {
            continue;
        }
        let Some((p, _, _)) = Scene::object_transform_raw(world, i, now, 0, None) else {
            continue;
        };
        let feet = p - Vec3::Z * (o.scale.z.abs() * 0.5);
        for &(id, root, r, centre, half) in hulls {
            let Some((heave, tilt)) = world.water_ride.pose(&id) else {
                continue;
            };
            // aboard: over it (with a 20 cm margin), its feet from its
            // bottom to just above its top (a deck, a cabin, its roof)
            let local = r.inverse() * (feet - (root + r * centre));
            if local.x.abs() > half.x + 0.2 || local.y.abs() > half.y + 0.2 || local.z < -half.z || local.z > half.z + 0.6 {
                continue;
            }
            // where the drawn deck takes the feet: lifted, and turned with it
            // about the root
            let drawn = root + Vec3::Z * heave + tilt * (feet - root);
            on.push((o.full_id, drawn.z - feet.z));
            break;
        }
    }
    on
}

/// Mark the object riding or not and have the scene place it again.
fn set_riding(world: &mut World, id: &uuid::Uuid, riding: bool) {
    let Some(i) = world.objects.index_of_uuid(id) else {
        return;
    };
    if let Some(o) = world.objects.get_mut_untracked(i) {
        o.render.rides_water = riding;
    }
    world.objects.touch(i);
}

/// The linkset of root `idx` as a box in the root's frame: its centre and
/// half size, round the root and every prim linked to it (their positions
/// and rotations are the root's frame's).
fn linkset_hull(world: &World, idx: usize) -> Option<(Vec3, Vec3)> {
    let root = world.objects.get(idx)?;
    let corners = |centre: Vec3, rotation: glam::Quat, half: Vec3| {
        (0..8).map(move |i| {
            let s = Vec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            );
            centre + rotation * (s * half)
        })
    };
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut add = |p: Vec3| {
        lo = lo.min(p);
        hi = hi.max(p);
    };
    corners(Vec3::ZERO, glam::Quat::IDENTITY, root.scale.abs() * 0.5).for_each(&mut add);
    for &c in world.objects.children_of(&root.key) {
        if let Some(child) = world.objects.get(c).filter(|c| !c.is_avatar()) {
            corners(child.position, child.rotation, child.scale.abs() * 0.5).for_each(&mut add);
        }
    }
    (lo.is_finite() && hi.is_finite()).then(|| ((lo + hi) * 0.5, (hi - lo) * 0.5))
}
