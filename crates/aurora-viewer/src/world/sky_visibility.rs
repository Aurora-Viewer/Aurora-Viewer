//! Hide skyboxes seen from other heights (Preferences › Graphics, "Hide
//! skyboxes at other heights", on by default). An Aurora addition: the
//! Legacy Viewer has none. With a long draw distance, the builds people put
//! high above the ground on purpose, out of sight, show from the ground,
//! and the people inside them too.
//!
//! A linkset **floats** when its lowest point is at least `FLOAT_CLEARANCE`
//! above the ground or the water under its root, whichever is higher. A
//! floating linkset is hidden while the camera is more than `HIDE_GAP`
//! above or below it (measured to its vertical extent, bottom to top), and
//! comes back once the camera is within `SHOW_GAP` of it: the margin
//! between the two keeps it from flickering at the edge. The rule is the
//! camera's, not the avatar's: camming up to a skybox shows it, and in a
//! skybox the skyboxes far above or below are hidden too. What stands on
//! the ground (a tower, a mountain) never floats, however tall.
//!
//! - An avatar is its own linkset: one flying or standing high up follows
//!   the rule; a seated one follows its seat's linkset, and what an avatar
//!   wears follows the avatar.
//! - A moving or physical linkset (a vehicle, with its riders) never
//!   floats. Avatars do not get this exemption: the simulator reports
//!   them as physical, and a flying one moves.
//! - Our own avatar, what it wears and the linkset it sits on are always
//!   shown; the build tools show everything while they are open.
//! - While the camera is focused on a point (Alt and a click, a focus on an
//!   object), the gap is measured from that point too, and the linkset of
//!   the focused object is always shown: orbiting a skybox from far away
//!   keeps it.
//!
//! What is hidden goes through the same path as the parcel isolation
//! (`World::object_shown`): released from the GPU, no texture or mesh
//! fetched for it, no particles, lights, name tag or click. Accepted limit:
//! someone standing on the roof of a tall building, higher than
//! `FLOAT_CLEARANCE`, floats as far as the rule knows.

use super::World;
use glam::{Quat, Vec3};
use std::time::Instant;
use uuid::Uuid;

/// Height above the ground or the water from which a linkset floats.
pub const FLOAT_CLEARANCE: f32 = 100.0;
/// Vertical gap between the camera and a floating linkset beyond which it is
/// hidden.
pub const HIDE_GAP: f32 = 60.0;
/// Vertical gap under which a hidden linkset shows again.
pub const SHOW_GAP: f32 = 50.0;
/// Object slots whose linkset is measured again per frame (an edit or a
/// move of something still): a whole pass every few frames at most.
const REFRESH_SLOTS: usize = 2048;
/// Water height when the region has not told it yet.
const DEFAULT_WATER: f32 = 20.0;

/// Vertical extent of a linkset (render space).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub bottom: f32,
    pub top: f32,
}

impl Extent {
    /// How far `z` is above or below it (0 within it).
    pub fn gap(self, z: f32) -> f32 {
        (self.bottom - z).max(z - self.top).max(0.0)
    }

    /// It floats above `floor` (the ground or the water under it).
    pub fn floats(self, floor: f32) -> bool {
        self.bottom - floor >= FLOAT_CLEARANCE
    }

    fn add(self, other: Extent) -> Extent {
        Extent {
            bottom: self.bottom.min(other.bottom),
            top: self.top.max(other.top),
        }
    }
}

/// Whether a floating linkset stays hidden, was `hidden` and is `gap` away
/// from the camera vertically.
pub fn stays_hidden(hidden: bool, gap: f32) -> bool {
    if hidden { gap >= SHOW_GAP } else { gap > HIDE_GAP }
}

/// Vertical extent of a box of size `scale` turned by `rotation`, around
/// `center`.
pub fn box_extent(center: Vec3, rotation: Quat, scale: Vec3) -> Extent {
    let half = 0.5
        * ((rotation * Vec3::X).z.abs() * scale.x.abs()
            + (rotation * Vec3::Y).z.abs() * scale.y.abs()
            + (rotation * Vec3::Z).z.abs() * scale.z.abs());
    Extent {
        bottom: center.z - half,
        top: center.z + half,
    }
}

#[derive(Debug, Clone, Copy)]
struct Root {
    id: Uuid,
    /// Its extent when it floats.
    floating: Option<Extent>,
    hidden: bool,
}

/// The state of the rule, per root object slot.
#[derive(Debug, Default)]
pub struct SkyVisibility {
    roots: Vec<Option<Root>>,
    cursor: usize,
    hidden: usize,
    /// Roots whose state changed since the scene last read them: the scene
    /// syncs their linkset again (`Scene::sync`).
    pub changed: Vec<usize>,
}

impl SkyVisibility {
    /// Something is hidden now.
    pub fn active(&self) -> bool {
        self.hidden > 0
    }

    fn hides(&self, idx: usize, id: Uuid) -> bool {
        self.roots.get(idx).copied().flatten().is_some_and(|r| r.id == id && r.hidden)
    }

    fn set_hidden(&mut self, idx: usize, hidden: bool) {
        if let Some(Some(r)) = self.roots.get_mut(idx)
            && r.hidden != hidden
        {
            r.hidden = hidden;
            if hidden {
                self.hidden += 1;
            } else {
                self.hidden -= 1;
            }
            self.changed.push(idx);
        }
    }

    fn forget(&mut self, idx: usize) {
        self.set_hidden(idx, false);
        if let Some(slot) = self.roots.get_mut(idx) {
            *slot = None;
        }
    }
}

impl World {
    /// Measures the linksets that arrived or changed, and hides or shows the
    /// floating ones for a camera at `eye` (render space), focused on
    /// `focus` (a point, or an object) when it is not on our avatar.
    /// `enabled`: the setting is on and the build tools are closed.
    pub fn update_sky_visibility(&mut self, enabled: bool, eye: Vec3, focus: Option<Vec3>, focus_object: Option<Uuid>) {
        let mut sky = std::mem::take(&mut self.sky_visibility);
        if !enabled {
            for idx in 0..sky.roots.len() {
                sky.forget(idx);
            }
            sky.roots.clear();
            self.sky_visibility = sky;
            return;
        }
        let now = Instant::now();
        let n = self.objects.slots.len();
        if sky.roots.len() > n {
            for idx in n..sky.roots.len() {
                sky.forget(idx);
            }
        }
        sky.roots.resize(n, None);
        let seat = self.own_seat_root();
        let focused = focus_object
            .and_then(|id| self.objects.index_of_uuid(&id))
            .map(|idx| self.objects.linkset_root(idx));
        let window = sky.cursor..sky.cursor + REFRESH_SLOTS;
        for idx in 0..n {
            let Some(o) = self.objects.get(idx) else {
                sky.forget(idx);
                continue;
            };
            // a root: nothing above it (a seated avatar follows its seat)
            if o.parent_id != 0 || o.full_id == self.agent_id || Some(idx) == seat || Some(idx) == focused {
                sky.forget(idx);
                continue;
            }
            let known = sky.roots[idx].filter(|r| r.id == o.full_id);
            let vehicle = !o.is_avatar() && (o.update_flags & crate::build::edits::FLAGS_USE_PHYSICS != 0 || o.has_motion());
            let floating = if vehicle {
                // a vehicle and what moves never float (an avatar does)
                None
            } else if known.is_none() || o.is_avatar() || window.contains(&idx) {
                self.floating_extent(idx, now)
            } else {
                known.and_then(|r| r.floating)
            };
            let was_hidden = known.is_some_and(|r| r.hidden);
            if known.is_none() {
                sky.forget(idx);
            }
            sky.roots[idx] = Some(Root {
                id: o.full_id,
                floating,
                hidden: was_hidden,
            });
            let hidden = floating.is_some_and(|e| {
                let gap = focus.map_or(e.gap(eye.z), |f| e.gap(eye.z).min(e.gap(f.z)));
                stays_hidden(was_hidden, gap)
            });
            sky.set_hidden(idx, hidden);
        }
        sky.cursor = if window.end >= n { 0 } else { window.end };
        self.sky_visibility = sky;
    }

    /// The linkset `root` (or the avatar) floats: its vertical extent.
    fn floating_extent(&self, root: usize, now: Instant) -> Option<Extent> {
        let o = self.objects.get(root)?;
        let (pos, rot, hud) = crate::scene::Scene::object_transform_unshifted(self, root, now)?;
        if hud {
            return None;
        }
        let extent = if o.is_avatar() {
            Extent {
                bottom: pos.z - 0.5 * o.scale.z,
                top: pos.z + 0.5 * o.scale.z,
            }
        } else {
            let mut extent = box_extent(pos, rot, o.scale);
            let mut stack: Vec<(usize, u32)> = self.objects.children_of(&o.key).iter().map(|&c| (c, 1)).collect();
            while let Some((idx, depth)) = stack.pop() {
                let Some(child) = self.objects.get(idx) else {
                    continue;
                };
                // its riders are not part of its shape
                if child.is_avatar() || depth > 4 {
                    continue;
                }
                if let Some((p, r, _)) = crate::scene::Scene::object_transform_unshifted(self, idx, now) {
                    extent = extent.add(box_extent(p, r, child.scale));
                }
                stack.extend(self.objects.children_of(&child.key).iter().map(|&c| (c, depth + 1)));
            }
            extent
        };
        let water = self
            .regions
            .get(&o.key.region)
            .and_then(|r| r.info.as_ref())
            .map_or(DEFAULT_WATER, |i| i.water_height);
        let floor = self.ground_height(pos).map_or(water, |g| g.max(water));
        extent.floats(floor).then_some(extent)
    }

    /// The root of what our avatar sits on.
    fn own_seat_root(&self) -> Option<usize> {
        let me = self.objects.get(self.objects.index_of_uuid(&self.agent_id)?)?;
        let seat = self.objects.parent_of(me)?;
        Some(self.objects.linkset_root(seat))
    }

    /// Whether the object is left out by the rule: its linkset, the avatar
    /// it is worn by, or the seat of that avatar, floats out of the
    /// camera's height. Ours never is.
    pub fn sky_object_visible(&self, index: usize) -> bool {
        if !self.sky_visibility.active() {
            return true;
        }
        let mut index = index;
        for _ in 0..64 {
            let Some(o) = self.objects.get(index) else {
                return true;
            };
            if o.full_id == self.agent_id {
                return true;
            }
            match self.objects.parent_of(o) {
                Some(parent) => index = parent,
                // a root, or a parent not arrived yet
                None => return o.parent_id != 0 || !self.sky_visibility.hides(index, o.full_id),
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::skyboxes as scene;
    use std::sync::Arc;

    /// The demo region with the skyboxes scene, our avatar on the ground.
    fn world() -> World {
        static LIB: std::sync::OnceLock<Arc<crate::scene::avatar::AvatarLibrary>> = std::sync::OnceLock::new();
        let lib = LIB.get_or_init(|| Arc::new(crate::scene::avatar::AvatarLibrary::load())).clone();
        let mut world = World::new(lib);
        for event in crate::demo::events().into_iter().chain(scene::scene()) {
            world.apply(event);
        }
        world
    }

    fn prim(world: &World, local_id: u32) -> usize {
        world.objects.index_of_uuid(&crate::demo::action_id(local_id)).expect("scene prim")
    }

    fn avatar(world: &World, id: Uuid) -> usize {
        world.objects.index_of_uuid(&id).expect("scene resident")
    }

    const TOWER: u32 = 97_000;
    const SKYBOX: u32 = 97_010;
    const SKYBOX_WALL: u32 = 97_011;
    const VEHICLE: u32 = 97_020;
    const ISLAND: u32 = 97_030;
    const HUT: u32 = 97_031;
    const BELOW: u32 = 97_040;

    #[test]
    fn from_the_ground_skyboxes_and_the_people_in_them_are_hidden() {
        let mut world = world();
        world.update_sky_visibility(true, Vec3::new(140.0, 130.0, 30.0), None, None);
        for local in [SKYBOX, SKYBOX_WALL, ISLAND, HUT, BELOW] {
            assert!(!world.object_shown(prim(&world, local)), "{local} floats far above");
        }
        for id in [
            scene::SKY_RESIDENT,
            scene::FLYING_RESIDENT,
            scene::ISLAND_RESIDENT,
            scene::BELOW_RESIDENT,
        ] {
            assert!(!world.object_shown(avatar(&world, id)));
        }
        assert!(world.object_shown(prim(&world, TOWER)), "a tall building on the ground");
        assert!(world.object_shown(prim(&world, VEHICLE)), "a physical vehicle never floats");
        assert!(world.object_shown(avatar(&world, scene::RIDER)), "its rider follows it");
        assert!(world.object_shown(avatar(&world, world.agent_id)), "ours");
    }

    #[test]
    fn in_a_skybox_it_shows_and_what_is_far_below_does_not() {
        let mut world = world();
        world.update_sky_visibility(true, scene::SKYBOX + Vec3::new(0.0, 0.0, 2.0), None, None);
        assert!(world.object_shown(prim(&world, SKYBOX)));
        assert!(world.object_shown(prim(&world, SKYBOX_WALL)));
        assert!(world.object_shown(avatar(&world, scene::SKY_RESIDENT)));
        assert!(!world.object_shown(avatar(&world, scene::FLYING_RESIDENT)), "80 m below");
        assert!(world.object_shown(prim(&world, TOWER)), "the ground's buildings are never hidden");
    }

    #[test]
    fn skyboxes_under_a_floating_island_are_hidden_from_it_and_it_from_them() {
        let mut world = world();
        world.update_sky_visibility(true, scene::ISLAND + Vec3::new(0.0, 0.0, 8.0), None, None);
        assert!(world.object_shown(prim(&world, ISLAND)));
        assert!(world.object_shown(prim(&world, HUT)));
        assert!(world.object_shown(avatar(&world, scene::ISLAND_RESIDENT)));
        assert!(!world.object_shown(prim(&world, BELOW)), "200 m under the island");
        assert!(!world.object_shown(avatar(&world, scene::BELOW_RESIDENT)));
        world.update_sky_visibility(true, scene::BELOW + Vec3::new(0.0, 0.0, 2.0), None, None);
        assert!(world.object_shown(prim(&world, BELOW)));
        assert!(!world.object_shown(prim(&world, ISLAND)), "200 m above the skybox");
    }

    #[test]
    fn camming_up_shows_a_skybox_and_the_margin_holds_it() {
        let mut world = world();
        let bottom = scene::SKYBOX.z - 0.25;
        let at = |z: f32| Vec3::new(150.0, 140.0, z);
        world.update_sky_visibility(true, at(30.0), None, None);
        let skybox = prim(&world, SKYBOX);
        assert!(!world.object_shown(skybox));
        world.update_sky_visibility(true, at(bottom - 55.0), None, None);
        assert!(!world.object_shown(skybox), "still hidden within the margin");
        world.update_sky_visibility(true, at(bottom - 45.0), None, None);
        assert!(world.object_shown(skybox), "shown within 50 m");
        assert!(world.sky_visibility.changed.contains(&skybox), "the scene syncs it again");
        world.update_sky_visibility(true, at(bottom - 55.0), None, None);
        assert!(world.object_shown(skybox), "still shown within the margin");
        world.update_sky_visibility(true, at(bottom - 65.0), None, None);
        assert!(!world.object_shown(skybox));
    }

    #[test]
    fn our_seat_stays_shown_and_switching_off_shows_everything() {
        let mut world = world();
        let skybox = prim(&world, SKYBOX);
        let seat = world.objects.get(skybox).expect("skybox").key.local_id;
        let me = avatar(&world, world.agent_id);
        world.objects.get_mut(me).expect("our avatar").parent_id = seat;
        world.update_sky_visibility(true, Vec3::new(140.0, 130.0, 30.0), None, None);
        assert!(world.object_shown(skybox), "what we sit on");
        assert!(world.object_shown(prim(&world, SKYBOX_WALL)));
        assert!(!world.object_shown(prim(&world, ISLAND)));
        world.sky_visibility.changed.clear();
        world.update_sky_visibility(false, Vec3::new(140.0, 130.0, 30.0), None, None);
        assert!(world.object_shown(prim(&world, ISLAND)));
        assert!(world.object_shown(avatar(&world, scene::BELOW_RESIDENT)));
        assert!(world.sky_visibility.changed.contains(&prim(&world, ISLAND)), "synced again");
        assert!(!world.sky_visibility.active());
    }

    #[test]
    fn a_physical_or_moving_avatar_high_up_is_still_hidden() {
        let mut world = world();
        let resident = avatar(&world, scene::SKY_RESIDENT);
        let flyer = avatar(&world, scene::FLYING_RESIDENT);
        // the simulator reports avatars as physical; a flying one moves
        world.objects.get_mut(resident).expect("resident").update_flags |= crate::build::edits::FLAGS_USE_PHYSICS;
        world.objects.get_mut(flyer).expect("flyer").velocity = Vec3::new(3.0, 0.0, 0.0);
        world.update_sky_visibility(true, Vec3::new(140.0, 130.0, 30.0), None, None);
        assert!(!world.object_shown(resident));
        assert!(!world.object_shown(flyer));
        assert!(world.object_shown(prim(&world, VEHICLE)), "a physical object still never floats");
    }

    #[test]
    fn the_camera_focus_keeps_a_skybox_orbited_from_afar() {
        let mut world = world();
        let ground = Vec3::new(140.0, 130.0, 30.0);
        let skybox = prim(&world, SKYBOX);
        // focused on a point of the skybox, the camera swung far below it
        world.update_sky_visibility(true, ground, Some(scene::SKYBOX), None);
        assert!(world.object_shown(skybox));
        assert!(world.object_shown(avatar(&world, scene::SKY_RESIDENT)));
        assert!(!world.object_shown(prim(&world, ISLAND)), "only near the focus");
        // focused on the island's hut: its whole linkset stays
        let hut = world.objects.get(prim(&world, HUT)).expect("hut").full_id;
        world.update_sky_visibility(true, ground, None, Some(hut));
        assert!(world.object_shown(prim(&world, ISLAND)));
        assert!(world.object_shown(prim(&world, HUT)));
        assert!(!world.object_shown(skybox), "the focus is back on the ground");
    }

    #[test]
    fn the_gap_is_measured_to_the_extent_above_and_below() {
        let e = Extent { bottom: 300.0, top: 320.0 };
        assert_eq!(e.gap(310.0), 0.0);
        assert_eq!(e.gap(250.0), 50.0);
        assert_eq!(e.gap(400.0), 80.0);
    }

    #[test]
    fn a_linkset_floats_from_the_clearance_above_its_floor() {
        let e = Extent { bottom: 300.0, top: 320.0 };
        assert!(e.floats(20.0));
        assert!(e.floats(200.0));
        assert!(!e.floats(200.5), "less than 100 m above its floor");
        let tower = Extent { bottom: 22.0, top: 400.0 };
        assert!(!tower.floats(20.0), "a tall building on the ground");
    }

    #[test]
    fn the_margin_keeps_a_skybox_from_flickering() {
        // shown: hidden only beyond HIDE_GAP
        assert!(!stays_hidden(false, 55.0));
        assert!(stays_hidden(false, 61.0));
        // hidden: shown again only within SHOW_GAP
        assert!(stays_hidden(true, 55.0));
        assert!(!stays_hidden(true, 49.0));
    }

    #[test]
    fn a_turned_box_reaches_as_high_as_its_corners() {
        let flat = box_extent(Vec3::new(0.0, 0.0, 300.0), Quat::IDENTITY, Vec3::new(30.0, 30.0, 1.0));
        assert_eq!(flat, Extent { bottom: 299.5, top: 300.5 });
        let standing = box_extent(
            Vec3::new(0.0, 0.0, 300.0),
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            Vec3::new(30.0, 30.0, 1.0),
        );
        assert!((standing.top - 315.0).abs() < 1e-3 && (standing.bottom - 285.0).abs() < 1e-3);
    }
}
