//! Aurora Weather: footprints in the lying snow. The feet of the avatars
//! near the camera (their L_FOOT / R_FOOT collision volumes, as the water's
//! contacts read them) are watched; a foot that lifts and is set down again
//! presses its print, a rounded sole along its heading, into the renderer's
//! footprints map (`RainLook::snow_prints`, env.wgsl `vs_snow_print`).
//! The rest of the body presses too where it comes down to the ground (a
//! knee, a seat, hands, a body lying in the snow): each of its collision
//! volumes whose underside is near the ground the feet found makes an oval
//! print the shape of the volume seen from above.

use super::{Scene, anchors, anim};
use crate::world::World;
use glam::{Mat3, Mat4, Vec3};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid::Uuid;

const FEET: [&str; 2] = ["L_FOOT", "R_FOOT"];

/// The body's collision volumes that press the snow when they come down to
/// the ground (the feet's own prints aside).
const BODY: [&str; 17] = [
    "PELVIS",
    "BUTT",
    "BELLY",
    "LOWER_BACK",
    "UPPER_BACK",
    "CHEST",
    "HEAD",
    "L_UPPER_LEG",
    "R_UPPER_LEG",
    "L_LOWER_LEG",
    "R_LOWER_LEG",
    "L_UPPER_ARM",
    "R_UPPER_ARM",
    "L_LOWER_ARM",
    "R_LOWER_ARM",
    "L_HAND",
    "R_HAND",
];

/// A body part presses once its underside is within this of the ground
/// its feet found (m); the renderer judges it again against the surface
/// the snow lies on (`BODY_ABOVE`).
const BODY_DOWN: f32 = 0.2;
/// How far above the surface the snow lies on a body part's underside may
/// be and still press (m): a kneeling knee, a seat, not a standing shin.
const BODY_ABOVE: f32 = 0.15;
/// A body part presses another print once it has moved this far (m).
const BODY_EVERY: f32 = 0.08;
/// What is physical or moving (a ball, a sled, a vehicle's wheels): each of
/// its prims is looked at once it has moved this far (m), pressing a print
/// where its underside is within `OBJECT_DOWN` of the ground below it.
const OBJECT_EVERY: f32 = 0.15;
const OBJECT_DOWN: f32 = 0.3;
/// Prims wider than this (half, m) press none (a moving platform, a ship).
const OBJECT_WIDEST: f32 = 2.0;

#[derive(Clone, Copy, Debug)]
struct Foot {
    time: f32,
    /// The lowest the foot has been lately (its sole on the ground).
    low: f32,
    /// Where it was last frame.
    at: Vec3,
    /// Where it pressed its last print.
    last_print: Option<glam::Vec2>,
}

/// A foot down presses another print once it has moved this far (m): a
/// foot at rest keeps one.
const PRINT_EVERY: f32 = 0.04;
/// The most prints kept in a region, the oldest dropped first.
const MAX_PRINTS: usize = 16384;
/// A print is gone once this much snow has fallen on it (`fallen` units:
/// it is pressed exp(-fallen / 240) deep, 3 % at 840).
const BURIED: f32 = 840.0;

/// One foot this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Step {
    /// Lifted, or in the air.
    None,
    /// Down: it presses its print (again: the same print is no deeper).
    Down,
}

/// A foot is down within this of the lowest it has been lately (m).
const DOWN: f32 = 0.04;
/// A foot that moved more than this in a frame was carried there (m).
const JUMP: f32 = 4.0;

impl Foot {
    /// The foot at `at` at `time` (s): down while its sole is near the
    /// lowest it reached lately. Pressing each frame it is down, not once
    /// per step: a walk cycle slides its planted foot (an SL walk is about
    /// 3 m/s), which a test for a foot at rest misses; a planted foot makes
    /// one print, a sliding one drags it a little, as in real snow. Whether
    /// it stands on the snow at all is the renderer's (the stamps' heights).
    fn advance(&mut self, at: Vec3, time: f32) -> Step {
        let dt = time - self.time;
        self.time = time;
        // a jump of more than a few metres since the last frame (a teleport
        // within the region, a sit far away): the ground is found again
        let moved = self.at.distance(at);
        self.at = at;
        if !(0.0..0.5).contains(&dt) || moved > JUMP {
            self.low = at.z;
            return Step::None;
        }
        // the ground's height follows the lowest the foot reaches, rising
        // slowly (a step up a stair is soon the new ground)
        self.low = (self.low + dt * 0.25).min(at.z);
        if at.z < self.low + DOWN { Step::Down } else { Step::None }
    }
}

#[derive(Default)]
pub struct SnowTrails {
    feet: HashMap<(Uuid, u8), Foot>,
    /// Where each body part (`BODY`'s index) pressed its last print.
    parts: HashMap<(Uuid, u8), glam::Vec2>,
    /// Where each moving prim was last looked at.
    objects: HashMap<Uuid, glam::Vec2>,
    /// The region's footprints (the renderer's `snow_prints`), and their
    /// generation.
    pub prints: std::sync::Arc<Vec<[f32; 8]>>,
    pub generation: u64,
    /// How much snow has fallen so far (seconds times how hard).
    pub fallen: f32,
    last: Option<f32>,
    /// The region they are in (other coordinates elsewhere: started again).
    region: Option<u64>,
}

impl SnowTrails {
    /// Every footprint gone at once (the Weather window's clean-up button);
    /// the next ones are pressed as before.
    pub fn clear(&mut self) {
        self.prints = Default::default();
        self.generation += 1;
        self.feet.clear();
        self.parts.clear();
        self.objects.clear();
    }

    /// The feet set down this frame within `reach` m of `centre` (the
    /// footprints' map), as the renderer's stamps.
    /// One frame: the snow falling (`snowing`, 0..1), the prints pressed
    /// now by the feet within `reach` m of `centre` added to the region's.
    pub fn press(scene: &mut Scene, world: &World, centre: Vec3, reach: f32, time: f32, snowing: f32) {
        let trails = &mut scene.snow_trails;
        let dt = trails.last.map_or(0.0, |t| (time - t).clamp(0.0, 1.0));
        trails.last = Some(time);
        trails.fallen += dt * snowing.clamp(0.0, 1.0);
        let region = world.main_region;
        if trails.region != region {
            // the feet too: the ground they found was the last region's (a
            // foot kept its low there, and a higher ground here took over a
            // minute to rise to: my feet pressed nothing after a teleport)
            trails.region = region;
            trails.clear();
        }
        // the buried ones go (checked now and then: they go slowly)
        let fallen = trails.fallen;
        if trails.prints.first().is_some_and(|p| fallen - p[5] > BURIED) {
            std::sync::Arc::make_mut(&mut trails.prints).retain(|p| fallen - p[5] <= BURIED);
            trails.generation += 1;
        }
        let mut pressed: Vec<[f32; 8]> = Vec::new();
        let now = Instant::now();
        let mut seen = HashSet::new();
        let mut seen_parts = HashSet::new();
        for (idx, o) in world.objects.avatars() {
            let Some((p, r, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if (p - centre).truncate().length() > reach {
                continue;
            }
            let Some(slot) = scene.palette_slots.get(&o.full_id) else {
                continue;
            };
            let base = *slot as usize * anim::PALETTE_JOINTS;
            let model = anchors::skeleton_model(world, &scene.meshes, idx, p, r, 1.0);
            for (k, name) in FEET.iter().enumerate() {
                let Some(&j) = scene.avatar_lib.rig.names.get(*name) else {
                    continue;
                };
                let Some(joint) = scene.palettes.get(base + j) else {
                    continue;
                };
                let m: Mat4 = model * Mat4::from_cols_array_2d(joint);
                if !m.is_finite() {
                    continue;
                }
                // the volume's centre, its sole below it, its heading along
                // its long axis
                let axes = Mat3::from_mat4(m);
                let half_height = (axes.transpose() * Vec3::Z).length();
                let at = m.w_axis.truncate() - Vec3::Z * half_height;
                let key = (o.full_id, k as u8);
                seen.insert(key);
                let foot = scene.snow_trails.feet.entry(key).or_insert(Foot {
                    time,
                    low: at.z,
                    at,
                    last_print: None,
                });
                if foot.advance(at, time) == Step::Down && foot.last_print.is_none_or(|l| l.distance(at.truncate()) > PRINT_EVERY) {
                    foot.last_print = Some(at.truncate());
                    let forward = axes.x_axis.truncate();
                    let half_length = (forward.length() * 1.1).clamp(0.08, 0.2);
                    pressed.push([at.x, at.y, forward.y.atan2(forward.x), half_length, at.z, fallen, 0.0, 0.0]);
                }
            }
            // the rest of the body, against the ground its feet found; only
            // standing on something (the simulator's contact plane) or
            // seated (a pose on the snow): a flier's feet are its lowest
            // too, and its body along them would fill the list with prints
            // the renderer finds in the air
            if o.foot_plane.is_none() && o.parent_id == 0 {
                continue;
            }
            let ground = [0u8, 1u8]
                .iter()
                .filter_map(|k| scene.snow_trails.feet.get(&(o.full_id, *k)))
                .map(|f| f.low)
                .reduce(f32::min);
            let Some(ground) = ground else {
                continue;
            };
            for (k, name) in BODY.iter().enumerate() {
                let Some(&j) = scene.avatar_lib.rig.names.get(*name) else {
                    continue;
                };
                let Some(joint) = scene.palettes.get(base + j) else {
                    continue;
                };
                let m: Mat4 = model * Mat4::from_cols_array_2d(joint);
                let key = (o.full_id, k as u8);
                let Some(print) = body_print(m, ground, fallen) else {
                    continue;
                };
                seen_parts.insert(key);
                let at = glam::Vec2::new(print[0], print[1]);
                if scene.snow_trails.parts.get(&key).is_none_or(|l| l.distance(at) > BODY_EVERY) {
                    scene.snow_trails.parts.insert(key, at);
                    pressed.push(print);
                }
            }
        }
        // what is physical or moving, prim by prim (not worn, not avatars)
        let moving: Vec<usize> = scene
            .gpu
            .iter()
            .enumerate()
            .filter(|(_, g)| g.physical && !g.is_avatar && !g.hud && !g.rigged && g.owner_avatar.is_none())
            .map(|(idx, _)| idx)
            .collect();
        let mut seen_objects = HashSet::new();
        for idx in moving {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let Some((p, r, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if (p - centre).truncate().length() > reach {
                continue;
            }
            seen_objects.insert(o.full_id);
            let at = p.truncate();
            if scene
                .snow_trails
                .objects
                .get(&o.full_id)
                .is_some_and(|l| l.distance(at) <= OBJECT_EVERY)
            {
                continue;
            }
            scene.snow_trails.objects.insert(o.full_id, at);
            let m = Mat4::from_scale_rotation_translation(o.scale * 0.5, r, p);
            let half_height = (Mat3::from_mat4(m).transpose() * Vec3::Z).length();
            let under = p.z - half_height;
            // the ground just below its underside (not itself: it moves)
            let Some(ground) = scene.ground_below(world, Vec3::new(p.x, p.y, under + 0.05), OBJECT_DOWN + 0.05) else {
                continue;
            };
            if let Some(print) = object_print(m, ground, fallen) {
                pressed.push(print);
            }
        }
        let trails = &mut scene.snow_trails;
        trails.feet.retain(|k, _| seen.contains(k));
        trails.parts.retain(|k, _| seen_parts.contains(k));
        trails.objects.retain(|k, _| seen_objects.contains(k));
        if !pressed.is_empty() {
            let prints = std::sync::Arc::make_mut(&mut trails.prints);
            prints.extend(pressed);
            if prints.len() > MAX_PRINTS {
                let extra = prints.len() - MAX_PRINTS;
                prints.drain(..extra);
            }
            trails.generation += 1;
        }
    }
}

/// The print of a body part (its collision volume's world matrix `m`, a
/// unit sphere scaled to the ellipsoid) when its underside is within
/// `BODY_DOWN` of `ground`: an oval the volume's shape seen from above,
/// along its longest level axis (x, y, heading, half length, underside z,
/// `fallen`, `BODY_ABOVE`, width over length).
fn body_print(m: Mat4, ground: f32, fallen: f32) -> Option<[f32; 8]> {
    oval_print(m, ground, BODY_DOWN, BODY_ABOVE, 0.5, fallen)
}

/// The same for a prim that moves (`m`: its half size, rotation and
/// position), its underside within `OBJECT_DOWN` of the ground below it.
fn object_print(m: Mat4, ground: f32, fallen: f32) -> Option<[f32; 8]> {
    oval_print(m, ground, OBJECT_DOWN, BODY_ABOVE, OBJECT_WIDEST, fallen)
}

fn oval_print(m: Mat4, ground: f32, down: f32, above: f32, widest: f32, fallen: f32) -> Option<[f32; 8]> {
    if !m.is_finite() {
        return None;
    }
    let axes = Mat3::from_mat4(m);
    // the ellipsoid's lowest point is its centre less its half height (the
    // length of the axes' z components together)
    let half_height = (axes.transpose() * Vec3::Z).length();
    let centre = m.w_axis.truncate();
    let under = centre.z - half_height;
    if under > ground + down {
        return None;
    }
    // seen from above: along its longest level axis, and across it, as far
    // as the ellipsoid reaches (each the root of the axes' squares along it)
    let level = [axes.x_axis.truncate(), axes.y_axis.truncate(), axes.z_axis.truncate()];
    let long = level
        .iter()
        .copied()
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?;
    let along = long.normalize_or_zero();
    let across = glam::Vec2::new(-along.y, along.x);
    let reach = |d: glam::Vec2| level.iter().map(|v| v.dot(d) * v.dot(d)).sum::<f32>().sqrt();
    let half_length = reach(along);
    if half_length > widest {
        return None;
    }
    let half_length = half_length.max(0.04);
    let width = (reach(across) / half_length).clamp(0.3, 1.0);
    Some([centre.x, centre.y, along.y.atan2(along.x), half_length, under, fallen, above, width])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_part_presses_an_oval_of_its_shape_only_near_the_ground() {
        // a shin lying along x: 25 cm long, 6 cm thick, its centre 7 cm up
        let shin = Mat4::from_scale_rotation_translation(Vec3::new(0.25, 0.06, 0.06), glam::Quat::IDENTITY, Vec3::new(1.0, 2.0, 0.07));
        let p = body_print(shin, 0.0, 5.0).expect("a kneeling shin presses");
        assert!((p[3] - 0.25).abs() < 1e-4 && p[2].abs() < 1e-4, "{p:?}");
        assert!((p[4] - 0.01).abs() < 1e-4 && p[5] == 5.0 && p[6] == BODY_ABOVE);
        assert!((p[7] - 0.3).abs() < 1e-4, "narrow: {}", p[7]);
        // the same shin standing (50 cm up): none
        let standing = Mat4::from_scale_rotation_translation(Vec3::new(0.25, 0.06, 0.06), glam::Quat::IDENTITY, Vec3::new(1.0, 2.0, 0.5));
        assert!(body_print(standing, 0.0, 5.0).is_none());
        // a seat, round from above
        let seat = Mat4::from_scale_rotation_translation(Vec3::splat(0.15), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.17));
        let s = body_print(seat, 0.0, 0.0).expect("a seat presses");
        assert!((s[7] - 1.0).abs() < 1e-4 && (s[3] - 0.15).abs() < 1e-4, "{s:?}");
    }

    #[test]
    fn a_rolling_ball_presses_and_a_ship_does_not() {
        // a 40 cm ball resting on ground at 10 m
        let ball = Mat4::from_scale_rotation_translation(Vec3::splat(0.2), glam::Quat::from_rotation_x(0.7), Vec3::new(0.0, 0.0, 10.2));
        let b = object_print(ball, 10.0, 1.0).expect("a ball on the ground presses");
        assert!((b[4] - 10.0).abs() < 1e-3 && (b[7] - 1.0).abs() < 1e-3, "{b:?}");
        // the same ball a metre up: none
        let up = Mat4::from_scale_rotation_translation(Vec3::splat(0.2), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 11.2));
        assert!(object_print(up, 10.0, 1.0).is_none());
        // a 10 m hull on the water: too wide to press
        let hull = Mat4::from_scale_rotation_translation(Vec3::new(5.0, 1.5, 1.0), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 11.0));
        assert!(object_print(hull, 10.0, 1.0).is_none());
    }

    #[test]
    fn a_foot_presses_while_it_is_down_and_not_while_it_is_lifted() {
        let mut foot = Foot {
            time: 0.0,
            low: 0.0,
            at: Vec3::ZERO,
            last_print: None,
        };
        let mut t = 0.0;
        // standing: down every frame (the same print)
        for _ in 0..10 {
            t += 0.016;
            assert_eq!(foot.advance(Vec3::ZERO, t), Step::Down);
        }
        // a step: lifted 10 cm mid-swing, sliding as it lands
        let mut down = 0;
        let mut up = 0;
        for i in 0..25 {
            t += 0.016;
            let lift = (i as f32 / 25.0 * std::f32::consts::PI).sin() * 0.1;
            match foot.advance(Vec3::new(i as f32 * 0.05, 0.0, lift), t) {
                Step::Down => down += 1,
                Step::None => up += 1,
            }
        }
        // (the ground's height creeps up 25 cm/s while lifted, for stairs)
        assert!(up > 6 && down > 2, "{up} up, {down} down");
        // a long pause (a frame skipped by a hidden window): no print then
        t += 2.0;
        assert_eq!(foot.advance(Vec3::new(5.0, 0.0, 3.0), t), Step::None);
    }

    #[test]
    fn a_foot_carried_far_finds_the_ground_again_at_once() {
        let mut foot = Foot {
            time: 0.0,
            low: 0.0,
            at: Vec3::ZERO,
            last_print: None,
        };
        // teleported 50 m away onto ground 20 m higher, without a pause:
        // down from the next frame (not after a minute of the low rising)
        assert_eq!(foot.advance(Vec3::new(50.0, 0.0, 20.0), 0.016), Step::None);
        assert_eq!(foot.advance(Vec3::new(50.0, 0.0, 20.0), 0.032), Step::Down);
    }
}
