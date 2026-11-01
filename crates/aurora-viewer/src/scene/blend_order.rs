//! Order of the blended faces of a frame.
//!
//! The Legacy Viewer draws the blended faces of rigged meshes (worn rigged
//! meshes, animesh) in a pass of their own before the other blended faces,
//! writing depth from 1/255 of alpha as they are drawn
//! (LLDrawPoolAlpha::render: forwardRender(true), then forwardRender();
//! write_depth = rigged). It does not sort them by distance: the rigged
//! meshes of one avatar come in its attachment order, the last attachment
//! point first and, on one point, the last object attached first
//! (LLVOAvatar::idleUpdateMisc gives each linkset a `draw_order`, sorted by
//! LLSpatialGroup::CompareRenderOrder), and the faces of a linkset keep
//! their order ("maintaining original draw order",
//! LLVolumeGeometryManager::genDrawInfo). Mesh hair is made for it: its
//! inner layers come before the outer ones, and each strand hides behind it
//! what is drawn after it, eyelashes as well as the layers under it.
//!
//! Here the rigged faces come first, skeleton by skeleton back to front (the
//! Legacy Viewer orders the avatars by the address of their object), each
//! skeleton's in the Legacy Viewer's order (`RigOrder`: the attachment
//! point, then the root's local id for the order of attachment, the
//! simulator giving them in turn, then the prims of the linkset by local
//! id), the faces of a prim in their own order; then the other blended
//! faces back to front. The renderer draws the first group writing depth
//! (`DrawLists::blend_depth`), not quite as the Legacy Viewer does
//! (`RiggedBlendDepth::Layers`, see there): its 1/255 makes
//! - the nearly transparent fringe of fur, feathers or hair hide the glass,
//!   water and particles behind it, a halo around the avatar (#556);
//! - a soft strand edge hide the strands drawn after it behind it. The
//!   Legacy Viewer does not keep the triangle order of a face either: it
//!   reorders them for the vertex cache (LLVolumeFace::cacheOptimize,
//!   meshopt_optimizeVertexCache), so the cards of one face overlap in no
//!   particular order, and drawn that way (#948) they left shards where the
//!   scene behind shows through the hair (#935).
//!
//! With the depth of every layer written before any colour where at least
//! half covered (#556 to #948), a layer drawn first was hidden behind each
//! strand of the layers over it from 50 % on, which then showed what lies
//! behind the hair through them (#935). With only what is nearly opaque
//! written before the colour (`Runs`, #966), a strand between 50 and 95 %
//! hid nothing: an inner layer drawn after the outer one showed over it.
//! In the drawing order and with the triangles of a mesh in the Legacy
//! Viewer's order, what is at least half covered then wrote its depth as it
//! was drawn, as the Legacy Viewer's 1/255 but without its soft edges, and
//! the fainter texels came after, writing nothing (`Split`): a mesh drawn
//! after a sheer dress and under it (a body add-on) showed through the
//! dress wherever its alpha fell under one half, in speckles (#1300). Now
//! each object's fringes also write their depth from 1/255, after all of
//! its colour, into a copy of the scene depth (`Layers`): a mesh drawn after
//! another and behind it is hidden by any of its texels, as in the Legacy
//! Viewer, while one object's soft edges still hide none of its own strands.

use std::cmp::Ordering;

use aurora_render::{DrawCmd, RiggedBlendDepth};

use crate::world::World;

/// Where the blended faces of a rigged mesh come among those of its
/// skeleton, as the Legacy Viewer draws them (see the module).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RigOrder {
    /// Its skeleton: the index of the avatar wearing it, or of the root of
    /// its animesh.
    pub skeleton: u32,
    /// The attachment point of its linkset's root (0 for an animesh).
    pub point: u8,
    /// The local id of its linkset's root: the simulator gives them in
    /// turn, so a later attachment has a greater one.
    pub root: u32,
    /// Its own local id: the prims of a linkset in that order.
    pub local: u32,
}

impl RigOrder {
    pub fn of(world: &World, idx: usize, skeleton: usize) -> Self {
        let objects = &world.objects;
        let mut root = idx;
        for _ in 0..64 {
            let Some(o) = objects.get(root) else {
                break;
            };
            if o.parent_id == 0 {
                break;
            }
            match objects.parent_of(o) {
                Some(p) if !objects.get(p).is_some_and(|p| p.is_avatar()) => root = p,
                _ => break,
            }
        }
        let (point, root_id) = objects.get(root).map_or((0, 0), |r| (r.attachment_point(), r.key.local_id));
        RigOrder {
            skeleton: skeleton as u32,
            point,
            root: root_id,
            local: objects.get(idx).map_or(0, |o| o.key.local_id),
        }
    }

    /// Drawing order of two rigged meshes of one skeleton: the last
    /// attachment point first, on one point the last attached first, the
    /// prims of a linkset in turn.
    fn draw_cmp(&self, other: &RigOrder) -> Ordering {
        other
            .point
            .cmp(&self.point)
            .then(other.root.cmp(&self.root))
            .then(self.local.cmp(&other.local))
    }
}

/// Rigged faces of one skeleton in its order, the skeletons apart.
fn rig_cmp(a: Option<RigOrder>, b: Option<RigOrder>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.skeleton.cmp(&b.skeleton).then(a.draw_cmp(&b)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// One blended face of the frame.
#[derive(Clone, Copy, Debug)]
pub struct BlendEntry {
    /// Distance from the eye to the centre of its object, or of its
    /// skeleton for a rigged mesh (`rig`).
    pub distance: f32,
    pub cmd: DrawCmd,
    /// The face has glow (drawn into the glow right after it).
    pub glow: bool,
    /// The face belongs to a rigged mesh.
    pub rigged: bool,
    /// Its object (index in the scene's slab), whose faces share one
    /// distance and are drawn in their own order.
    pub object: u32,
    /// Its face (texture entry) in the object.
    pub face: u8,
    /// A rigged mesh: its place among those of its skeleton.
    pub rig: Option<RigOrder>,
}

/// AURORA_RIGGED_BLEND: by default the rigged faces apart, object by object
/// in their order what is at least half covered writing its depth as it is
/// drawn, then the fainter texels, then their depth from 1/255, into a copy
/// of the scene depth (`RiggedBlendDepth::Layers`, #1300); `split`, as
/// #1024: the fainter texels writing no depth (`RiggedBlendDepth::Split`);
/// `runs`, as #966: their depth
/// where nearly opaque before their colour, then run by run (`rig_runs`)
/// their colour and their depth where half covered;
/// `copy`, as #948: from 1/255 as they are drawn into a copy of the scene
/// depth, into the scene depth where half covered; `depth05`, as before
/// #948: their depth where half covered before their colour (#556);
/// `write`, from 1/255 into the scene depth as the Legacy Viewer (#499);
/// `sorted`, sorted with the others and drawn without depth, as before
/// #499; `alt`, `split` one second in three (`xmode=old` on the profile
/// lines), the default the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Layers,
    Split,
    Runs,
    Copy,
    Depth05,
    Write,
    Sorted,
    Alternate,
}

pub fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_RIGGED_BLEND").as_deref().map(str::trim) {
        Ok("split") => Mode::Split,
        Ok("runs") => Mode::Runs,
        Ok("copy") => Mode::Copy,
        Ok("depth05") => Mode::Depth05,
        Ok("write") => Mode::Write,
        Ok("sorted") => Mode::Sorted,
        Ok("alt") => Mode::Alternate,
        _ => Mode::Layers,
    })
}

/// With `alt`: the current second draws as `split`.
static OLD_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_old_now(old: bool) {
    OLD_NOW.store(old, std::sync::atomic::Ordering::Relaxed);
}

/// Whether this frame draws the rigged faces apart, writing depth.
pub fn rigged_apart() -> bool {
    mode() != Mode::Sorted
}

/// How the rigged faces write their depth this frame (see the module).
pub fn depth_mode() -> RiggedBlendDepth {
    depth_mode_in(mode(), OLD_NOW.load(std::sync::atomic::Ordering::Relaxed))
}

fn depth_mode_in(mode: Mode, old_now: bool) -> RiggedBlendDepth {
    match mode {
        Mode::Layers => RiggedBlendDepth::Layers,
        Mode::Split => RiggedBlendDepth::Split,
        Mode::Runs => RiggedBlendDepth::Runs,
        Mode::Copy => RiggedBlendDepth::Layered,
        Mode::Depth05 => RiggedBlendDepth::HalfCovered,
        Mode::Write | Mode::Sorted => RiggedBlendDepth::Write,
        Mode::Alternate if old_now => RiggedBlendDepth::Split,
        Mode::Alternate => RiggedBlendDepth::Layers,
    }
}

/// Sort `entries` into drawing order and return how many of them, at the
/// start, are drawn writing depth: the rigged faces, skeleton by skeleton
/// back to front, each in its `RigOrder`, then the others back to front
/// (all of them back to front, none apart, when `rigged_apart` is false).
/// The faces of one object keep their own order, by face then first index;
/// objects at the same distance keep one order from frame to frame (by
/// object): the lists are gathered in parallel, in no fixed order, and an
/// unstable tie would flicker.
pub fn sort(entries: &mut [BlendEntry], rigged_apart: bool) -> usize {
    entries.sort_unstable_by(|a, b| {
        let group = |e: &BlendEntry| !(rigged_apart && e.rigged);
        group(a)
            .cmp(&group(b))
            .then(b.distance.total_cmp(&a.distance))
            .then(rig_cmp(a.rig, b.rig))
            .then(a.object.cmp(&b.object))
            .then(a.face.cmp(&b.face))
            .then(a.cmd.first_index.cmp(&b.cmd.first_index))
            .then(a.cmd.record.cmp(&b.cmd.record))
    });
    if rigged_apart {
        entries.iter().take_while(|e| e.rigged).count()
    } else {
        0
    }
}

/// The runs of the first `n` sorted entries worn on one attachment point
/// of one skeleton (an object without `rig`, alone), as their end indices
/// (`DrawLists::blend_depth_runs`). Drawn run by run, each sees the depth
/// of those drawn before it, never its own: the layers of a hair worn on
/// one point compose as the faces of one object, without hiding each other
/// by a faint edge, while a body and the hair over it, on two points, still
/// hide each other where half covered. One run per object costs a pair of
/// pipeline changes per worn mesh: about 0.3 ms more for 40 avatars of
/// 8 meshes, against 0.05 ms with one run per skeleton.
pub fn rig_runs(entries: &[BlendEntry], n: usize, out: &mut Vec<u32>) {
    out.clear();
    let entries = &entries[..n.min(entries.len())];
    let key = |e: &BlendEntry| match e.rig {
        Some(r) => (r.skeleton, r.point, u32::MAX),
        None => (u32::MAX, 0, e.object),
    };
    for (i, pair) in entries.windows(2).enumerate() {
        if key(&pair[0]) != key(&pair[1]) {
            out.push(i as u32 + 1);
        }
    }
    if !entries.is_empty() {
        out.push(entries.len() as u32);
    }
}

/// `RiggedBlendDepth::Split`: the runs of the first `n` sorted entries, as
/// their end indices (`DrawLists::blend_depth_runs`), cut between two
/// objects worn on one attachment point of one skeleton. Each run draws what
/// is half covered then its fringes before the next one, so the fringes of
/// a layer drawn first still show under the strands of the layers worn over
/// it on that point, as with the Legacy Viewer's 1/255 (#948). The objects
/// of different points share a run: a pair of pipeline changes per worn
/// mesh costs about 0.3 ms for 40 avatars of 8 layers each, and the fringes
/// of the hair then also blend over a body drawn after it. Within one run
/// the fringes come after all of its strands.
pub fn rig_objects(entries: &[BlendEntry], n: usize, out: &mut Vec<u32>) {
    out.clear();
    let entries = &entries[..n.min(entries.len())];
    let point = |e: &BlendEntry| e.rig.map(|r| (r.skeleton, r.point));
    let mut seen: Vec<(u32, u8)> = Vec::new();
    for (i, pair) in entries.windows(2).enumerate() {
        if pair[0].object == pair[1].object {
            continue;
        }
        // a new object: a cut when its point already has one in this run
        if let Some(p) = point(&pair[0]) {
            seen.push(p);
        }
        if point(&pair[1]).is_none_or(|p| seen.contains(&p)) {
            out.push(i as u32 + 1);
            seen.clear();
        }
    }
    if !entries.is_empty() {
        out.push(entries.len() as u32);
    }
}

/// `RiggedBlendDepth::Layers`: the runs of the first `n` sorted entries, one
/// per object, as their end indices (`DrawLists::blend_depth_runs`). Each
/// object's fringes write their depth after all of its colour, so that they
/// hide the objects drawn after them, whatever their point, as the Legacy
/// Viewer's 1/255 (#1300), but none of its own strands (#935).
pub fn each_object(entries: &[BlendEntry], n: usize, out: &mut Vec<u32>) {
    out.clear();
    let entries = &entries[..n.min(entries.len())];
    for (i, pair) in entries.windows(2).enumerate() {
        if pair[0].object != pair[1].object {
            out.push(i as u32 + 1);
        }
    }
    if !entries.is_empty() {
        out.push(entries.len() as u32);
    }
}

/// Which side of the water a blended object is drawn on, by its bounding
/// sphere (centre height, radius) against `plane`, the agent region's water
/// height, given only while the camera is above it: `(after, before)` the
/// water. A group reaching above the plane goes after it
/// (POOL_ALPHA_POST_WATER), one reaching below it before
/// (POOL_ALPHA_PRE_WATER), one crossing it in both, each draw clipped to its
/// side (LLDrawPoolAlpha::renderPostDeferred, lldrawpoolalpha.cpp, and
/// waterClip in deferredUtil.glsl). Without a plane (camera under water, no
/// region) everything goes after, as before.
pub fn water_sides(center_z: f32, radius: f32, plane: Option<f32>) -> (bool, bool) {
    match plane {
        Some(h) => (center_z + radius >= h, center_z - radius < h),
        None => (true, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(record: u32, distance: f32, rigged: bool) -> BlendEntry {
        BlendEntry {
            distance,
            cmd: DrawCmd {
                index_count: 3,
                first_index: 0,
                base_vertex: 0,
                record,
                bounds: [0.0; 4],
            },
            glow: false,
            rigged,
            object: record,
            face: 0,
            rig: None,
        }
    }

    /// A rigged mesh worn by `skeleton`: the root of its linkset, on
    /// `point`, has the local id `root`.
    fn worn(object: u32, distance: f32, skeleton: u32, point: u8, root: u32, local: u32) -> BlendEntry {
        BlendEntry {
            rig: Some(RigOrder {
                skeleton,
                point,
                root,
                local,
            }),
            ..entry(object, distance, true)
        }
    }

    /// A face of `object`, its record given by the allocator in any order.
    fn face(object: u32, face: u8, record: u32, first_index: u32) -> BlendEntry {
        BlendEntry {
            object,
            face,
            cmd: DrawCmd {
                first_index,
                ..entry(record, 3.0, true).cmd
            },
            ..entry(record, 3.0, true)
        }
    }

    fn records(e: &[BlendEntry]) -> Vec<u32> {
        e.iter().map(|e| e.cmd.record).collect()
    }

    /// Mesh hair centred on the head (farther) and eyelashes in front of the
    /// face (nearer), both rigged, with a window and a particle-like card of
    /// the scene: the rigged faces first, back to front, then the others.
    #[test]
    fn rigged_faces_come_first_back_to_front_then_the_others() {
        let mut e = [
            entry(1, 5.0, false), // window
            entry(2, 3.10, true), // eyelashes
            entry(3, 9.0, false), // glass far away
            entry(4, 3.20, true), // hair
        ];
        assert_eq!(sort(&mut e, true), 2);
        assert_eq!(records(&e), [4, 2, 3, 1]);
    }

    /// By default the rigged faces draw what is half covered, then their
    /// fringes, then the fringes' depth (#1300); the old seconds of `alt`
    /// draw as `split` (#935, fringes without depth), `depth05`
    /// as #556 did, `write` and `sorted` write from 1/255.
    #[test]
    fn rigged_depth_mode_follows_the_variable() {
        assert_eq!(depth_mode_in(Mode::Layers, false), RiggedBlendDepth::Layers);
        assert_eq!(depth_mode_in(Mode::Split, false), RiggedBlendDepth::Split);
        assert_eq!(depth_mode_in(Mode::Runs, false), RiggedBlendDepth::Runs);
        assert_eq!(depth_mode_in(Mode::Copy, false), RiggedBlendDepth::Layered);
        assert_eq!(depth_mode_in(Mode::Depth05, false), RiggedBlendDepth::HalfCovered);
        assert_eq!(depth_mode_in(Mode::Write, false), RiggedBlendDepth::Write);
        assert_eq!(depth_mode_in(Mode::Sorted, false), RiggedBlendDepth::Write);
        assert_eq!(depth_mode_in(Mode::Alternate, false), RiggedBlendDepth::Layers);
        assert_eq!(depth_mode_in(Mode::Alternate, true), RiggedBlendDepth::Split);
    }

    #[test]
    fn sorted_mode_keeps_one_back_to_front_list_without_depth() {
        let mut e = [
            entry(1, 5.0, false),
            entry(2, 3.10, true),
            entry(3, 9.0, false),
            entry(4, 3.20, true),
        ];
        assert_eq!(sort(&mut e, false), 0);
        assert_eq!(records(&e), [3, 1, 4, 2]);
    }

    /// Objects at the same distance: their order must not depend on the
    /// order they were gathered in.
    #[test]
    fn faces_at_the_same_distance_keep_a_fixed_order() {
        let mut a = [entry(9, 3.0, true), entry(4, 3.0, true), entry(7, 3.0, true), entry(5, 6.0, false)];
        let mut b = [entry(7, 3.0, true), entry(5, 6.0, false), entry(9, 3.0, true), entry(4, 3.0, true)];
        assert_eq!(sort(&mut a, true), 3);
        assert_eq!(sort(&mut b, true), 3);
        assert_eq!(records(&a), [4, 7, 9, 5]);
        assert_eq!(records(&a), records(&b));
    }

    /// One mesh hair, its inner layer on face 0 and its outer layers on
    /// faces 1 and 2, the outer one split in two draws: drawn in that order
    /// whatever records the allocator gave the faces (a freed slot is
    /// reused), as the Legacy Viewer keeps the faces' order (#935).
    #[test]
    fn the_faces_of_one_object_keep_their_creators_order() {
        let mut e = [
            face(12, 2, 3, 600), // outer layer, second draw
            face(12, 1, 40, 0),  // middle layer
            face(12, 2, 3, 0),   // outer layer, first draw
            face(12, 0, 77, 0),  // inner layer
        ];
        assert_eq!(sort(&mut e, true), 4);
        let order: Vec<(u8, u32)> = e.iter().map(|e| (e.face, e.cmd.first_index)).collect();
        assert_eq!(order, [(0, 0), (1, 0), (2, 0), (2, 600)]);
    }

    /// Two avatars and a window. The nearer avatar wears a mesh body on its
    /// pelvis, a two-prim mesh hair on its skull and eyelashes attached to
    /// the skull after the hair: its rigged meshes keep the Legacy Viewer's
    /// order whatever their own distances (the last point first, on one
    /// point the last attached first, the prims of a linkset in turn),
    /// after the farther avatar's, and before the window.
    #[test]
    fn a_skeletons_rigged_meshes_come_in_the_legacy_viewers_attachment_order() {
        const PELVIS: u8 = 1;
        const SKULL: u8 = 2;
        let mut e = [
            entry(1, 4.0, false),               // window
            worn(10, 3.0, 7, PELVIS, 400, 400), // body
            worn(11, 3.0, 7, SKULL, 500, 501),  // hair, second prim
            worn(12, 3.0, 7, SKULL, 500, 500),  // hair, root prim
            worn(13, 3.0, 7, SKULL, 520, 520),  // eyelashes, attached later
            worn(20, 6.0, 9, SKULL, 300, 300),  // the farther avatar's hair
        ];
        assert_eq!(sort(&mut e, true), 5);
        assert_eq!(records(&e), [20, 13, 12, 11, 10, 1]);
    }

    /// Layers of hair worn as separate meshes on one point: the one
    /// attached last is drawn first, as the Legacy Viewer does, so a hair
    /// made for it (its inner layer attached last) is drawn inside out.
    #[test]
    fn layers_worn_on_one_point_come_last_attached_first() {
        let mut e: Vec<BlendEntry> = (0..4).map(|layer| worn(30 + layer, 2.0, 5, 16, 900 - layer, 900 - layer)).collect();
        e.reverse();
        assert_eq!(sort(&mut e, true), 4);
        assert_eq!(records(&e), [30, 31, 32, 33]);
    }

    /// Without a skeleton order, one run per object (three faces of one,
    /// then two objects), the faces after the rigged ones left out.
    #[test]
    fn rig_runs_split_objects_without_a_skeleton_one_by_one() {
        let mut e = [
            face(12, 0, 1, 0),
            face(12, 1, 2, 0),
            face(12, 2, 3, 0),
            face(13, 0, 4, 0),
            face(14, 0, 5, 0),
            entry(9, 1.0, false),
        ];
        let n = sort(&mut e, true);
        let mut runs = Vec::new();
        rig_runs(&e, n, &mut runs);
        assert_eq!(runs, [3, 4, 5]);
        rig_runs(&e, 0, &mut runs);
        assert!(runs.is_empty());
    }

    /// One avatar: two hair layers and eyelashes on the skull, a body on
    /// the pelvis; a farther avatar's hair. One run per point of each
    /// skeleton: the farther hair, then the skull's three meshes together,
    /// then the body.
    #[test]
    fn rig_runs_group_the_meshes_worn_on_one_point() {
        const PELVIS: u8 = 1;
        const SKULL: u8 = 2;
        let mut e = [
            worn(10, 3.0, 7, PELVIS, 400, 400), // body
            worn(11, 3.0, 7, SKULL, 500, 500),  // hair, outer layer
            worn(12, 3.0, 7, SKULL, 510, 510),  // hair, inner layer
            worn(13, 3.0, 7, SKULL, 520, 520),  // eyelashes
            worn(20, 6.0, 9, SKULL, 300, 300),  // the farther avatar's hair
        ];
        let n = sort(&mut e, true);
        assert_eq!(records(&e), [20, 13, 12, 11, 10]);
        let mut runs = Vec::new();
        rig_runs(&e, n, &mut runs);
        assert_eq!(runs, [1, 4, 5]);
    }

    /// `Split`: a cut before each mesh worn on a point that already has one
    /// in the run (the skull's eyelashes, inner and outer hair layers one by
    /// one), none between points: the farther avatar's hair and the
    /// eyelashes share a run, the outer layer and the body another.
    #[test]
    fn rig_objects_cut_only_between_the_meshes_of_one_point() {
        const PELVIS: u8 = 1;
        const SKULL: u8 = 2;
        let mut e = [
            worn(10, 3.0, 7, PELVIS, 400, 400), // body
            worn(11, 3.0, 7, SKULL, 500, 500),  // hair, outer layer
            worn(12, 3.0, 7, SKULL, 510, 510),  // hair, inner layer
            worn(13, 3.0, 7, SKULL, 520, 520),  // eyelashes
            worn(20, 6.0, 9, SKULL, 300, 300),  // the farther avatar's hair
            entry(9, 1.0, false),
        ];
        let n = sort(&mut e, true);
        assert_eq!(records(&e), [20, 13, 12, 11, 10, 9]);
        let mut runs = Vec::new();
        rig_objects(&e, n, &mut runs);
        assert_eq!(runs, [2, 3, 5]);
        // the faces of one object stay together
        let mut faces = [face(12, 0, 1, 0), face(12, 1, 2, 0), face(13, 0, 3, 0)];
        let n = sort(&mut faces, true);
        rig_objects(&faces, n, &mut runs);
        assert_eq!(runs, [2, 3]);
        rig_objects(&e, 0, &mut runs);
        assert!(runs.is_empty());
    }

    /// `Layers` (#1300): every object its own run, whatever its point, so
    /// that a sheer dress hides a body add-on drawn after it with its
    /// fringes too; the faces of one object stay together.
    #[test]
    fn each_object_cuts_between_every_object() {
        const CHEST: u8 = 1;
        const SKULL: u8 = 2;
        let mut e = [
            worn(10, 3.0, 7, CHEST, 400, 400), // body add-on
            worn(11, 3.0, 7, SKULL, 500, 500), // sheer dress
            worn(20, 6.0, 9, SKULL, 300, 300), // the farther avatar's hair
            entry(9, 1.0, false),
        ];
        let n = sort(&mut e, true);
        assert_eq!(records(&e), [20, 11, 10, 9]);
        let mut runs = Vec::new();
        each_object(&e, n, &mut runs);
        assert_eq!(runs, [1, 2, 3]);
        let mut faces = [face(12, 0, 1, 0), face(12, 1, 2, 0), face(13, 0, 3, 0)];
        let n = sort(&mut faces, true);
        each_object(&faces, n, &mut runs);
        assert_eq!(runs, [2, 3]);
        each_object(&e, 0, &mut runs);
        assert!(runs.is_empty());
    }

    #[test]
    fn no_rigged_face_means_nothing_drawn_with_depth() {
        let mut e = [entry(1, 2.0, false), entry(2, 4.0, false)];
        assert_eq!(sort(&mut e, true), 0);
        assert_eq!(records(&e), [2, 1]);
    }

    #[test]
    fn water_sides_follow_the_legacy_pools() {
        let h = Some(20.0);
        assert_eq!(water_sides(30.0, 2.0, h), (true, false), "above: after the water only");
        assert_eq!(water_sides(10.0, 2.0, h), (false, true), "below: before the water only");
        assert_eq!(water_sides(20.5, 2.0, h), (true, true), "crossing: both, clipped");
        assert_eq!(water_sides(10.0, 2.0, None), (true, false), "camera under water: as before");
    }
}
