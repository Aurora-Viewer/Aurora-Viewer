//! What avatars wear, placed by the GPU.
//!
//! What an avatar wears follows it: a prim follows its animated attachment
//! point (LLViewerJointAttachment under its joint), a rigged mesh the
//! skeleton that deforms it. Every one of them moved at each frame, so the
//! sync placed them all again (most of the objects it visited in a busy
//! place) and sent their draw records again when their matrix changed.
//! Yet a worn prim does not move relative to its attachment point, nor a
//! rigged mesh relative to its skeleton.
//!
//! Each palette therefore ends with **anchors** (`anim::ANCHOR_BASE`),
//! rigid world matrices written at each frame: the first places the
//! skeleton itself (skeleton space to world, `skeleton_model`), the others
//! the attachment points of the body as posed in this frame.
//!
//! - The record of a **worn prim** holds its placement relative to the
//!   anchor of its point (`flags::ANCHORED`), which the vertex shaders
//!   compose; the culling composes its bounding sphere the same way
//!   (`CullObject::anchor`).
//! - The record of a **rigged mesh** (worn mesh, bento, animesh worn or on
//!   the ground) reads the anchor of its skeleton instead of holding the
//!   skeleton's matrix; the culling computes its sphere at each frame from
//!   the joint boxes of its geometry as the palette poses them
//!   (`object_flags::POSED`, [`JointBoxes`]), as the sync did.
//!
//! The sync writes the few anchors in use and no longer visits what hangs
//! from them (`SyncState::anchored`): such an object is placed again only
//! when it changes itself, or an object between it and its point, and when
//! the detail level check comes round.
//!
//! Left to the sync as before: HUD attachments, flexible prims, objects
//! that move by themselves (spin) and what is linked below those.
//!
//! The CPU still wants to know where these objects are (blended faces to
//! sort, hover, lights, impostors): their centre is recomputed from the
//! anchors whenever their anchor changed, a product per object (the
//! anchors of an impostor that keeps its pose, impostors.rs, do not change
//! while it stands); a rigged mesh's follows its
//! skeleton from where its last visit found it, and the lists the CPU still
//! builds test it with the extent of its skeleton, as LL does. Exact
//! transforms are computed on demand, as for any object
//! (`Scene::object_transform`).
//!
//! `AURORA_CPU_ATTACH=1` leaves all of them to the sync (comparison).

use super::anim::{ANCHOR_BASE, ANCHORS, PALETTE_JOINTS};
use super::animesh::JointBounds;
use super::avatar::AvatarLibrary;
use super::meshes::MeshStreamer;
use super::sync_sets::{IndexSet, MAX_DEPTH};
use super::{GeomKey, Scene, hover};
use crate::fast_hash::FastMap;
use crate::world::World;
use crate::world::objects::Object;
use aurora_render::Renderer;
use aurora_render::arena::RangeAllocator;
use aurora_render::gpu_cull::JointBox;
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// The anchor an object is placed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnchorRef {
    /// Owner of the palette (object index): the wearer, or the root of an
    /// animated object.
    pub wearer: u32,
    /// Anchor of the palette: 0 the skeleton's, else an attachment point.
    pub slot: u8,
    /// Index of the anchor matrix in the palettes buffer.
    pub matrix: u32,
}

/// A rigged mesh the GPU poses for culling (`object_flags::POSED`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    /// Joint boxes of its geometry (first, count) and first skin binding
    /// of its mesh; no box: `rest` stands for its bounds.
    pub boxes: u32,
    pub count: u32,
    pub binds: u32,
    /// Sphere of its skeleton, in the space of the skeleton's anchor.
    pub rest: [f32; 4],
}

/// Skeleton space to world for the skeleton of object `owner_idx` placed at
/// (`pos`, `rot`): the matrix rigged meshes are drawn with, and the first
/// anchor of the owner's palette. `scale`: HUD zoom of a rigged HUD.
pub fn skeleton_model(world: &World, meshes: &MeshStreamer, owner_idx: usize, pos: Vec3, mut rot: Quat, scale: f32) -> Mat4 {
    let mut pos = pos;
    // LLControlAvatar::matchVolumeTransform: ground animesh also follow
    // their root mesh's unscaled bind rotation.
    if let Some(root) = world.objects.get(owner_idx)
        && !root.is_avatar()
        && Scene::wearer_avatar(world, owner_idx).is_none()
        && let Some(skin) = root
            .visual_volume()
            .sculpt
            .and_then(|s| meshes.meta(&s.texture))
            .and_then(|m| m.skin)
    {
        let (_, bind_rot, _) = skin.bind_shape.to_scale_rotation_translation();
        if bind_rot.is_finite() {
            rot = (rot * bind_rot).normalize();
        }
        // LLViewerObject::getRenderPosition: a ground control avatar's
        // pelvis fixup is world vertical. Worn animesh follows its point.
        pos.z += world.avatar_shape_dz.get(&root.full_id).copied().unwrap_or(0.0);
    }
    skeleton_at(&world.avatar_lib, pos, rot, scale)
}

/// Skeleton space (feet at z = 0) to world for a skeleton whose root, the
/// pelvis, is at (`pos`, `rot`).
fn skeleton_at(lib: &AvatarLibrary, pos: Vec3, rot: Quat, scale: f32) -> Mat4 {
    Mat4::from_scale_rotation_translation(Vec3::splat(scale), rot, pos) * Mat4::from_translation(-lib.pelvis)
}

/// Radius around its owner that holds a skeleton and what it deforms (LL
/// culls rigged attachments with their wearer's extents).
pub fn skeleton_extent(owner: &Object) -> f32 {
    if owner.is_avatar() {
        2.5
    } else {
        (owner.visual_scale().length() * 0.75).max(1.0)
    }
}

/// An attachment point in its wearer's space, as posed.
pub struct AttachFrame {
    pub pos: Vec3,
    pub rot: Quat,
    pub hud: bool,
}

/// Where attachment point `point` of `wearer` is, relative to the avatar's
/// root: on its animated joint when the avatar is posed, at its rest place
/// otherwise; a HUD point in the HUD's own space (screen aspect applied).
pub fn attach_frame(world: &World, wearer: &Object, point: u8) -> AttachFrame {
    let lib = &world.avatar_lib;
    let ap = lib.attach_points.get(&point).copied();
    let hud = ap.is_some_and(|a| a.hud) || (31..=38).contains(&point);
    let (mut pos, mut rot) = ap.map(|a| (a.position, a.rotation)).unwrap_or((Vec3::ZERO, Quat::IDENTITY));
    if hud {
        pos.y *= world.hud_aspect;
        return AttachFrame { pos, rot, hud };
    }
    if let Some(a) = ap {
        let joint_world = a.joint.and_then(|j| {
            world
                .avatar_poses
                .get(&wearer.full_id)
                .and_then(|p| p.get(j))
                .map(|delta| *delta * a.rest_joint)
        });
        let (position, rotation) = a.frame(joint_world);
        pos = position - lib.pelvis;
        rot = rotation;
    }
    AttachFrame { pos, rot, hud }
}

/// Anchor of a palette that holds attachment point `point`: the skeleton's
/// for a point the library does not know (placed at the wearer's root, as
/// `attach_frame`); None for a HUD point or one past the palette's anchors.
pub fn anchor_slot(lib: &AvatarLibrary, point: u8) -> Option<u8> {
    match lib.attach_points.get(&point) {
        Some(ap) if ap.hud => None,
        Some(ap) => ap.anchor,
        None => (!(31..=38).contains(&point)).then_some(0),
    }
}

/// AURORA_ANCHOR_CENTRES=every: the centre of every anchored object is
/// recomputed at each frame, as before, instead of those whose anchor
/// changed (comparison).
fn every_centre() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_ANCHOR_CENTRES").is_ok_and(|v| v == "every"))
}

/// AURORA_CPU_ATTACH=1: the sync places everything, as before the anchors.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_CPU_ATTACH").is_none_or(|v| v == "0"))
}

/// The anchor object `o` hangs from and its transform relative to it, when
/// that transform is constant: `o` is an attachment or linked below one,
/// on a body point, and nothing from it up to the point moves by itself.
/// `matrix_of` gives the anchor matrix index of a wearer's anchor (None:
/// the wearer has no palette yet).
pub fn anchored(world: &World, o: &Object, matrix_of: impl Fn(&Object, u8) -> Option<u32>) -> Option<(AnchorRef, Vec3, Quat)> {
    if o.is_flexible() {
        return None;
    }
    // transform of `o` in the space of `cur`'s parent, composed as
    // `Scene::object_transform` does it
    let (mut pos, mut rot) = (o.position, o.rotation);
    let mut cur = o;
    for _ in 0..MAX_DEPTH {
        if cur.has_motion() || cur.is_avatar() {
            return None;
        }
        let pidx = world.objects.parent_of(cur)?;
        let parent = world.objects.get(pidx)?;
        if parent.is_avatar() {
            if cur.state == 0 {
                return None;
            }
            let slot = anchor_slot(&world.avatar_lib, cur.attachment_point())?;
            let matrix = matrix_of(parent, slot)?;
            if slot == 0 {
                // the wearer's root, in the skeleton's space
                pos += world.avatar_lib.pelvis;
            }
            return Some((
                AnchorRef {
                    wearer: pidx as u32,
                    slot,
                    matrix,
                },
                pos,
                rot,
            ));
        }
        pos = parent.position + parent.rotation * pos;
        rot = (parent.rotation * rot).normalize();
        cur = parent;
    }
    None
}

/// The owner of a palette (a wearer, the root of an animated object) with
/// anchors in use.
struct Wearer {
    id: Uuid,
    /// Anchors in use (bit per slot): written at each frame.
    used: u64,
    /// Anchors the anchored objects met this frame referred to.
    seen: u64,
    /// Anchors asked for this frame by an object the sync visits anyway
    /// (an avatar's own body).
    kept: u64,
    /// First anchor matrix of its palette this frame; None while the
    /// wearer cannot be placed (gone, no palette, parent unknown): its
    /// anchors and what hangs from them stay where they were.
    base: Option<u32>,
    /// Where it is and the extent of its skeleton (`skeleton_extent`).
    at: Vec3,
    extent: f32,
    /// Anchors whose matrix changed this frame (bit per slot): what hangs
    /// from the others keeps its centre.
    changed: u64,
    /// An object was placed from one of its anchors since the last frame,
    /// or its extent changed: every centre is brought up to date.
    refresh: bool,
    /// Where its anchors were last written from: place, rotation, first
    /// anchor matrix and anchors in use. A wearer that kept its pose and
    /// its place (an impostor standing, impostors.rs) has the same anchors.
    written: Option<(Vec3, Quat, usize, u64)>,
}

/// The wearer of a run of anchored objects, in `update_anchors`.
#[derive(Clone, Copy)]
struct Run {
    wearer: u32,
    base: Option<u32>,
    at: Vec3,
    extent: f32,
    seen: u64,
    changed: u64,
    refresh: bool,
}

/// Joint boxes of the rigged geometries on the GPU (`CullTables::boxes`):
/// one range per geometry, given when a posed object first needs it and
/// taken back with the geometry.
#[derive(Default)]
pub struct JointBoxes {
    ranges: HashMap<GeomKey, (u32, u32)>,
    free: Option<RangeAllocator>,
}

impl JointBoxes {
    /// The range of a geometry's joint boxes in the GPU table (first,
    /// count), written through `set` when it is new; None when the
    /// geometry has no box.
    pub fn range(&mut self, key: GeomKey, bounds: &[JointBounds], mut set: impl FnMut(usize, JointBox)) -> Option<(u32, u32)> {
        if bounds.is_empty() {
            return None;
        }
        if let Some(&range) = self.ranges.get(&key) {
            return Some(range);
        }
        let len = bounds.len() as u32;
        let free = self.free.get_or_insert_with(|| RangeAllocator::new(0));
        let base = match free.alloc(len) {
            Some(base) => base,
            None => {
                free.grow(free.capacity() + len);
                // (the new room, with what was free before it: always fits)
                free.alloc(len).unwrap_or(free.capacity() - len)
            }
        };
        for (i, b) in bounds.iter().enumerate() {
            set(
                base as usize + i,
                JointBox {
                    min: b.min.to_array(),
                    joint: b.joint as u32,
                    max: b.max.to_array(),
                    _pad: 0,
                },
            );
        }
        self.ranges.insert(key, (base, len));
        Some((base, len))
    }

    /// The geometry is freed, or replaced (its objects are given the new
    /// range by their next visit; until then the culling reads boxes that
    /// may be another geometry's, or falls back on the skeleton's sphere).
    pub fn release(&mut self, key: &GeomKey) {
        if let Some((base, len)) = self.ranges.remove(key)
            && let Some(free) = &mut self.free
        {
            free.free(base, len);
        }
    }
}

/// The anchors in use and the objects placed from them.
#[derive(Default)]
pub struct Anchors {
    wearers: HashMap<u32, Wearer>,
    /// Anchored objects (validated at each frame, removed lazily).
    members: IndexSet,
    /// AURORA_SYNC_CHECK: centres left as they were, and those of them not
    /// where their anchor puts them (must stay 0).
    pub kept_centres: (u64, u64),
    /// AURORA_SYNC_CHECK: anchors left as they were, and those of them not
    /// what the wearer's pose and place give (must stay 0).
    pub kept_anchors: (u64, u64),
}

impl Anchors {
    pub fn clear(&mut self) {
        *self = Anchors::default();
    }

    /// Object `idx` was placed: from `anchor`, or by the sync.
    pub fn placed(&mut self, world: &World, idx: usize, anchor: Option<AnchorRef>) {
        let Some(a) = anchor else {
            self.members.remove(idx);
            return;
        };
        let Some(wearer) = world.objects.get(a.wearer as usize) else {
            return;
        };
        self.members.insert(idx);
        let w = self.wearer(a.wearer, wearer.full_id);
        w.used |= 1 << a.slot;
        w.refresh = true;
    }

    /// The entry of palette owner `idx`, new when another object took the
    /// slot of one that left.
    fn wearer(&mut self, idx: u32, id: Uuid) -> &mut Wearer {
        let new = || Wearer {
            id,
            used: 0,
            seen: 0,
            kept: 0,
            base: None,
            at: Vec3::ZERO,
            extent: 0.0,
            changed: 0,
            refresh: true,
            written: None,
        };
        let w = self.wearers.entry(idx).or_insert_with(new);
        if w.id != id {
            *w = new();
        }
        w
    }

    /// Anchor `slot` of palette owner `idx` is used in this frame by an
    /// object the sync visits at each frame: written with the others.
    pub fn keep(&mut self, idx: u32, id: Uuid, slot: u8) {
        let w = self.wearer(idx, id);
        w.used |= 1 << slot;
        w.kept |= 1 << slot;
    }

    /// The object left its slot.
    pub fn forget(&mut self, idx: usize) {
        self.members.remove(idx);
    }

    /// The objects placed from an anchor (as the last frame left them).
    pub fn members(&self) -> &[usize] {
        self.members.as_slice()
    }
}

impl Scene {
    /// Anchor matrix index of anchor `slot` of `wearer`, once its palette
    /// exists on the CPU side (a palette allocated in this frame is sized
    /// by the next `update_poses`).
    pub(super) fn anchor_matrix(palette_slots: &FastMap<Uuid, u32>, palettes: usize, wearer: &Object, slot: u8) -> Option<u32> {
        let base = *palette_slots.get(&wearer.full_id)? as usize * PALETTE_JOINTS;
        (base + PALETTE_JOINTS <= palettes && (slot as usize) < ANCHORS).then_some((base + ANCHOR_BASE + slot as usize) as u32)
    }

    /// After the sync placed this frame's objects (`now`: the time it
    /// placed them at): write the anchors in use, upload them with the
    /// palettes posed in this frame, and bring the centre of every anchored
    /// object up to date for what the CPU does with it (hover index too).
    pub(super) fn update_anchors(&mut self, renderer: &mut Renderer, world: &mut World, now: Instant, verify: bool) {
        let lib = self.avatar_lib.clone();
        let mut posed = std::mem::take(&mut self.posed_slots);
        posed.sort_unstable();
        // anchors of the wearers not posed in this frame
        let mut moved: Vec<std::ops::Range<usize>> = Vec::new();
        let (kept_anchors, wrong_anchors) = {
            let k = &mut self.anchors.kept_anchors;
            (&mut k.0, &mut k.1)
        };
        // (the seconds of AURORA_IMPOSTOR_POSES=every or alt that pose every
        // impostor are the old way here too)
        let every_centre = every_centre() || super::impostors::pose_every_frame();
        for (&widx, w) in &mut self.anchors.wearers {
            w.seen = 0;
            w.base = None;
            w.changed = 0;
            w.refresh |= every_centre;
            let Some(wearer) = world.objects.get(widx as usize).filter(|o| o.full_id == w.id) else {
                continue;
            };
            let Some(&slot) = self.palette_slots.get(&w.id) else {
                continue;
            };
            let base = slot as usize * PALETTE_JOINTS + ANCHOR_BASE;
            let Some(block) = self.palettes.get_mut(base..base + ANCHORS) else {
                continue;
            };
            let xf = self
                .xf_cache
                .get(widx as usize)
                .or_else(|| Self::object_transform_in(world, widx as usize, now, 0, Some(&self.xf_cache)));
            let Some((pp, pr, _)) = xf else {
                continue;
            };
            w.base = Some(base as u32);
            w.at = pp;
            let extent = skeleton_extent(wearer);
            w.refresh |= extent != w.extent;
            w.extent = extent;
            let written = Some((pp, pr, base, w.used));
            // not posed in this frame and not moved: its anchors are those
            // already written (computed anyway to check them)
            let kept = !w.refresh && !every_centre && w.written == written && posed.binary_search(&(slot as usize)).is_err();
            if kept && !verify {
                continue;
            }
            w.written = written;
            let (mut lo, mut hi) = (ANCHORS, 0);
            let mut bits = w.used;
            while bits != 0 {
                let s = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let Some(m) = block.get_mut(s) else {
                    continue;
                };
                let anchor = match s.checked_sub(1).and_then(|i| lib.anchor_points.get(i)) {
                    Some(&point) => {
                        let f = attach_frame(world, wearer, point);
                        Mat4::from_rotation_translation(pr * f.rot, pp + pr * f.pos)
                    }
                    None => skeleton_model(world, &self.meshes, widx as usize, pp, pr, 1.0),
                }
                .to_cols_array_2d();
                if kept {
                    *kept_anchors += 1;
                    if *m != anchor {
                        *wrong_anchors += 1;
                    }
                    continue;
                }
                if *m != anchor {
                    *m = anchor;
                    lo = lo.min(s);
                    hi = hi.max(s + 1);
                    w.changed |= 1 << s;
                }
            }
            if lo < hi && posed.binary_search(&(slot as usize)).is_err() {
                moved.push(base + lo..base + hi);
            }
        }
        if !self.live_morphs.uploaded {
            renderer.set_palette_range(&self.palettes, 0..self.live_morphs.matrices.len());
            self.live_morphs.uploaded = true;
        }
        let mut driven = Vec::new();
        for (id, slot) in &self.palette_slots {
            let index = *slot as usize * PALETTE_JOINTS + super::live_morphs::WEIGHTS;
            self.physics_driven(id, &mut driven);
            if let Some(motions) = self.motions.get(id) {
                driven.extend_from_slice(motions.live_morphs());
            }
            let weights = self.live_morphs.weights(id, now, &driven);
            let range = index..index + weights.len();
            if let Some(previous) = self.palettes.get_mut(range.clone())
                && *previous != weights
            {
                previous.copy_from_slice(&weights);
                moved.push(range);
            }
        }
        renderer.set_palette_slots(&self.palettes, &posed, PALETTE_JOINTS);
        for range in moved {
            renderer.set_palette_range(&self.palettes, range);
        }
        if !self.palette_fresh.is_empty() {
            renderer.seed_palette_slots(&self.palettes, &self.palette_fresh, PALETTE_JOINTS);
            self.palette_fresh.clear();
        }
        posed.clear();
        self.posed_slots = posed;

        // (the prims of what an avatar wears are next to each other in the
        // set: its wearer is looked up once for the run)
        let Anchors {
            wearers,
            members,
            kept_centres,
            ..
        } = &mut self.anchors;
        let mut run: Option<Run> = None;
        let close = |run: Option<Run>, wearers: &mut HashMap<u32, Wearer>| {
            if let Some(run) = run
                && let Some(w) = wearers.get_mut(&run.wearer)
            {
                w.seen |= run.seen;
            }
        };
        let mut i = 0;
        while i < members.len() {
            let idx = members.as_slice()[i];
            let anchor = self.gpu.get(idx).and_then(|g| g.anchor);
            if let Some(a) = anchor
                && run.is_none_or(|r| r.wearer != a.wearer)
            {
                close(run.take(), wearers);
                run = wearers.get(&a.wearer).map(|w| Run {
                    wearer: a.wearer,
                    base: w.base,
                    at: w.at,
                    extent: w.extent,
                    seen: 0,
                    changed: w.changed,
                    refresh: w.refresh,
                });
            }
            let Some((a, run)) = anchor.zip(run.as_mut()) else {
                // released, or placed by the sync again
                members.remove(idx);
                continue;
            };
            i += 1;
            run.seen |= 1 << a.slot;
            let Some(base) = run.base else {
                continue;
            };
            if base + a.slot as u32 != a.matrix {
                // the wearer's palette changed place: the sync builds its
                // records again (a rigged mesh's hold the palette itself)
                if let Some(o) = world.objects.get_mut_untracked(idx) {
                    o.render.needs_records = true;
                    o.material_dirty = true;
                }
                self.sets.dirty.insert(idx);
                continue;
            }
            if !run.refresh && run.changed & (1 << a.slot) == 0 {
                // (its anchor did not move: its centre is still right)
                if verify && let (Some(m), Some(g)) = (self.palettes.get(a.matrix as usize), self.gpu.get(idx)) {
                    let centre = Mat4::from_cols_array_2d(m).transform_point3(g.local_center);
                    let wide = g.posed.map(|_| (run.at, run.extent));
                    kept_centres.0 += 1;
                    if centre.distance(g.center) > 1e-4
                        || wide.is_some_and(|w| g.wide.is_none_or(|g| g.0.distance(w.0) > 1e-4 || g.1 != w.1))
                    {
                        kept_centres.1 += 1;
                    }
                }
                continue;
            }
            if let (Some(m), Some(g)) = (self.palettes.get(a.matrix as usize), self.gpu.get_mut(idx)) {
                g.center = Mat4::from_cols_array_2d(m).transform_point3(g.local_center);
                // a rigged mesh: the CPU's lists test it with its skeleton
                g.wide = g.posed.map(|_| (run.at, run.extent));
                self.pick.set(idx, hover::PickEntry::of(g));
            }
        }
        close(run, wearers);
        // an anchor nothing hangs from any more is no longer written
        wearers.retain(|_, w| {
            w.used = w.seen | std::mem::take(&mut w.kept);
            // (one that could not be placed keeps its refresh for the frame
            // it is)
            w.refresh &= w.base.is_none();
            w.used != 0
        });
        self.stats.anchored = members.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn attachment_rotations_place_spine_and_chest_linksets_in_rest_and_animated_poses() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for ev in crate::demo::events().into_iter().chain(crate::demo::worn::events()) {
            world.apply(ev);
        }
        let wearer = world.objects.index_of_uuid(&crate::demo::DEMO_AGENT).expect("wearer");
        let local = |id| {
            world
                .objects
                .iter()
                .find(|(_, o)| o.key.local_id == id)
                .map(|(i, _)| i)
                .expect("worn prim")
        };
        let root = local(crate::demo::worn::ROOT);
        let child = local(crate::demo::worn::CHILD);
        let now = Instant::now();
        let (wearer_pos, wearer_rot, _) = Scene::object_transform(&world, wearer, now, 0).expect("wearer frame");
        for (point, offset, point_rot) in [
            (1u8, Vec3::new(0.15, 0.0, -0.1), Quat::from_xyzw(0.5, 0.5, 0.5, 0.5)),
            (9u8, Vec3::new(-0.15, 0.0, -0.1), Quat::from_xyzw(-0.5, -0.5, 0.5, 0.5)),
        ] {
            world.objects.get_mut(root).expect("root").state = point.rotate_left(4);
            // Reference attachment frame: a child joint of mChest with
            // the local position and setQuat rotation from avatar_lad.xml.
            let chest = lib.skeleton.as_ref().expect("skeleton").find("mChest").expect("chest");
            let rest_point = lib.skeleton.as_ref().expect("skeleton").joints[chest].world_position() + offset;
            for pose in [
                None,
                Some(Mat4::from_rotation_translation(
                    Quat::from_rotation_y(0.4),
                    Vec3::new(0.1, -0.2, 0.3),
                )),
            ] {
                world.avatar_poses.clear();
                if let Some(pose) = pose {
                    world.avatar_poses.insert(crate::demo::DEMO_AGENT, vec![pose; lib.rig.len()]);
                }
                let pose = pose.unwrap_or(Mat4::IDENTITY);
                let (_, pose_rot, _) = pose.to_scale_rotation_translation();
                let anchor = Mat4::from_rotation_translation(
                    wearer_rot * pose_rot * point_rot,
                    wearer_pos + wearer_rot * (pose.transform_point3(rest_point) - lib.pelvis),
                );
                let root_object = world.objects.get(root).expect("root");
                let root_local = Mat4::from_rotation_translation(root_object.rotation, root_object.position);
                for idx in [root, child] {
                    let object = world.objects.get(idx).expect("prim");
                    let local = Mat4::from_scale_rotation_translation(object.scale, object.rotation, object.position);
                    let expected = anchor * if idx == root { local } else { root_local * local };
                    let (pos, rot, hud) = Scene::object_transform(&world, idx, now, 0).expect("attachment frame");
                    assert!(!hud);
                    let actual = Mat4::from_scale_rotation_translation(object.scale, rot, pos);
                    assert!(
                        actual.abs_diff_eq(expected, 1e-4),
                        "point {point}, prim {idx}: {actual:?} != {expected:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_body_point_has_its_anchor() {
        let lib = AvatarLibrary::load();
        assert!(lib.rig.len() <= ANCHOR_BASE, "the joints fit below the anchors");
        let body = lib.attach_points.values().filter(|ap| !ap.hud).count();
        assert!(body > 0 && body < ANCHORS, "{body} body points for {ANCHORS} anchors");
        assert_eq!(lib.anchor_points.len(), body);
        let mut seen = std::collections::HashSet::new();
        for (id, ap) in &lib.attach_points {
            match anchor_slot(&lib, *id) {
                Some(slot) => {
                    assert!(!ap.hud && slot >= 1 && (slot as usize) < ANCHORS);
                    assert_eq!(lib.anchor_points[slot as usize - 1], *id);
                    assert!(seen.insert(slot), "anchor {slot} given twice");
                }
                None => assert!(ap.hud),
            }
        }
        // a point the library does not know hangs from the root; HUD ids never
        assert_eq!(anchor_slot(&lib, 200), Some(0));
        assert_eq!(anchor_slot(&lib, 35), None);
    }

    /// The transform relative to the anchor, composed with the anchor,
    /// gives the transform the CPU computes through the parent chain.
    #[test]
    fn anchored_transform_matches_the_parent_chain() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::crowd_events(1, false)) {
            world.apply(ev);
        }
        let now = Instant::now();
        // pose the avatars' joints so the points leave their rest place
        let lib = world.avatar_lib.clone();
        let pose = Mat4::from_rotation_translation(Quat::from_rotation_y(0.4), Vec3::new(0.1, -0.2, 0.3));
        let avatars: Vec<Uuid> = world
            .objects
            .slots
            .iter()
            .flatten()
            .filter(|o| o.is_avatar())
            .map(|o| o.full_id)
            .collect();
        for id in avatars {
            world.avatar_poses.insert(id, vec![pose; lib.rig.len()]);
        }
        // one attachment on a point the library does not know: it hangs
        // from the wearer's root, through the skeleton's anchor
        let unknown = (0..world.objects.slots.len())
            .find(|&i| {
                world.objects.get(i).is_some_and(|o| {
                    o.state != 0
                        && world
                            .objects
                            .parent_of(o)
                            .and_then(|p| world.objects.get(p))
                            .is_some_and(|p| p.is_avatar())
                })
            })
            .expect("an attachment");
        world.objects.get_mut(unknown).expect("attachment").state = 200u8.rotate_right(4);
        let (mut checked, mut slot0) = (0, 0);
        for idx in 0..world.objects.slots.len() {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let Some((a, lp, lr)) = anchored(&world, o, |_, slot| Some(slot as u32)) else {
                continue;
            };
            let wearer = world.objects.get(a.wearer as usize).expect("wearer");
            let (pp, pr, _) = Scene::object_transform(&world, a.wearer as usize, now, 0).expect("wearer placed");
            let anchor = match a.slot.checked_sub(1) {
                Some(i) => {
                    let f = attach_frame(&world, wearer, lib.anchor_points[i as usize]);
                    assert!(!f.hud);
                    Mat4::from_rotation_translation(pr * f.rot, pp + pr * f.pos)
                }
                None => {
                    slot0 += 1;
                    skeleton_at(&lib, pp, pr, 1.0)
                }
            };
            let composed = anchor * Mat4::from_scale_rotation_translation(o.scale, lr, lp);
            let (pos, rot, hud) = Scene::object_transform(&world, idx, now, 0).expect("placed");
            assert!(!hud);
            let direct = Mat4::from_scale_rotation_translation(o.scale, rot, pos);
            assert!(composed.abs_diff_eq(direct, 1e-4), "object {idx}: {composed:?} / {direct:?}");
            checked += 1;
        }
        // 8 linksets of 7 prims on the crowd avatar, at least
        assert!(checked >= 56, "{checked} anchored objects");
        assert!(slot0 >= 1, "the unknown point uses the skeleton's anchor");
    }

    /// A geometry's joint boxes get their range once, give it back with
    /// the geometry, and the next geometries take the room again.
    #[test]
    fn joint_boxes_ranges_are_given_once_and_taken_again() {
        let key = |lod| GeomKey::Mesh {
            id: Uuid::from_u128(7),
            lod,
        };
        let bounds = |n: u8| -> Vec<JointBounds> {
            (0..n)
                .map(|j| JointBounds {
                    joint: j,
                    min: Vec3::splat(j as f32),
                    max: Vec3::splat(j as f32 + 1.0),
                })
                .collect()
        };
        let mut table: Vec<JointBox> = Vec::new();
        let mut written = 0;
        let mut boxes = JointBoxes::default();
        let mut range = |boxes: &mut JointBoxes, k: GeomKey, b: &[JointBounds]| {
            boxes.range(k, b, |i, v| {
                if table.len() <= i {
                    table.resize(i + 1, JointBox::NONE);
                }
                table[i] = v;
                written += 1;
            })
        };
        assert_eq!(range(&mut boxes, key(0), &[]), None);
        assert_eq!(range(&mut boxes, key(0), &bounds(3)), Some((0, 3)));
        assert_eq!(range(&mut boxes, key(1), &bounds(5)), Some((3, 5)));
        // asked again: the same range, nothing written
        assert_eq!(range(&mut boxes, key(0), &bounds(3)), Some((0, 3)));
        boxes.release(&key(0));
        boxes.release(&key(0));
        // too large for the freed room, then one that fits it
        assert_eq!(range(&mut boxes, key(2), &bounds(4)), Some((8, 4)));
        assert_eq!(range(&mut boxes, key(3), &bounds(2)), Some((0, 2)));
        drop(range);
        assert_eq!(written, 3 + 5 + 4 + 2);
        assert_eq!((table[3].joint, table[7].joint, table[1].joint), (0, 4, 1));
        assert_eq!(table[8].max, [1.0; 3]);
    }

    /// What moves by itself, and what is linked below it, stays with the
    /// sync; so do HUD attachments and avatars.
    #[test]
    fn moving_and_hud_objects_are_not_anchored() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for ev in crate::demo::events()
            .into_iter()
            .chain(crate::demo::crowd_events(1, false))
            .chain(crate::demo::hud::events())
        {
            world.apply(ev);
        }
        let any = |_: &Object, slot: u8| Some(slot as u32);
        let n = world.objects.slots.len();
        let anchored_now = |world: &World| -> Vec<usize> {
            (0..n)
                .filter(|&i| world.objects.get(i).is_some_and(|o| anchored(world, o, any).is_some()))
                .collect()
        };
        let before = anchored_now(&world);
        assert!(!before.is_empty());
        for &idx in &before {
            let o = world.objects.get(idx).expect("object");
            assert!(!o.is_avatar() && !o.is_flexible());
            // up to the attachment itself: the object whose parent is the wearer
            let mut root = o;
            while let Some(p) = world
                .objects
                .parent_of(root)
                .and_then(|p| world.objects.get(p))
                .filter(|p| !p.is_avatar())
            {
                root = p;
            }
            let point = root.attachment_point();
            assert!(!(31..=38).contains(&point), "HUD point {point}");
        }
        // a wearer without a palette: nothing is anchored
        assert!((0..n).all(|i| world.objects.get(i).is_none_or(|o| anchored(&world, o, |_, _| None).is_none())));
        // an attachment root starts spinning: it and its prims leave
        let root = before
            .iter()
            .copied()
            .find(|&i| world.objects.get(i).is_some_and(|o| !world.objects.children_of(&o.key).is_empty()))
            .expect("a worn linkset");
        let children = world.objects.children_of(&world.objects.get(root).expect("root").key).to_vec();
        world.objects.get_mut(root).expect("root").angular_velocity = Vec3::Z;
        let after = anchored_now(&world);
        assert!(!after.contains(&root));
        assert!(children.iter().all(|c| !after.contains(c)));
        assert_eq!(after.len(), before.len() - 1 - children.len());
    }
}
