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
//! (`DrawLists::blend_depth`), by default into a copy of the scene depth
//! (`RiggedBlendDepth::Layered`): the scene depth takes them only where
//! they are at least half covered, as the Legacy Viewer's 1/255 made the
//! nearly transparent fringe of fur, feathers or hair hide the glass, water
//! and particles behind it, a halo around the avatar (#556). With the depth
//! of every layer written before any colour, only where at least half
//! covered (#556 to #935), a layer drawn first was hidden behind each
//! strand of the layers over it from 50 % on, which then showed what lies
//! behind the hair through them (#935).

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

/// AURORA_RIGGED_BLEND: by default (`layers`) the rigged faces apart,
/// writing depth from 1/255 as they are drawn into a copy of the scene
/// depth, and into the scene depth where at least half covered;
/// `depth05`, as before #935: their depth where at least half covered
/// before their colour (#556); `write`, from 1/255 into the scene depth as
/// the Legacy Viewer (#499); `sorted`, sorted with the others and drawn
/// without depth, as before #499; `alt`, `depth05` one second in three
/// (`xmode=old` on the profile lines), `layers` the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Layers,
    Depth05,
    Write,
    Sorted,
    Alternate,
}

pub fn mode() -> Mode {
    static MODE: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_RIGGED_BLEND").as_deref().map(str::trim) {
        Ok("depth05") => Mode::Depth05,
        Ok("write") => Mode::Write,
        Ok("sorted") => Mode::Sorted,
        Ok("alt") => Mode::Alternate,
        _ => Mode::Layers,
    })
}

/// With `alt`: the current second draws as before #935.
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
        Mode::Layers => RiggedBlendDepth::Layered,
        Mode::Depth05 => RiggedBlendDepth::HalfCovered,
        Mode::Write | Mode::Sorted => RiggedBlendDepth::Write,
        Mode::Alternate if old_now => RiggedBlendDepth::HalfCovered,
        Mode::Alternate => RiggedBlendDepth::Layered,
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

    /// By default the rigged faces compose as the Legacy Viewer draws them
    /// on a copy of the depth (#935); `depth05` and the old seconds of `alt`
    /// draw as #556 did, `write` and `sorted` write from 1/255.
    #[test]
    fn rigged_depth_mode_follows_the_variable() {
        assert_eq!(depth_mode_in(Mode::Layers, false), RiggedBlendDepth::Layered);
        assert_eq!(depth_mode_in(Mode::Depth05, false), RiggedBlendDepth::HalfCovered);
        assert_eq!(depth_mode_in(Mode::Write, false), RiggedBlendDepth::Write);
        assert_eq!(depth_mode_in(Mode::Sorted, false), RiggedBlendDepth::Write);
        assert_eq!(depth_mode_in(Mode::Alternate, false), RiggedBlendDepth::Layered);
        assert_eq!(depth_mode_in(Mode::Alternate, true), RiggedBlendDepth::HalfCovered);
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

    #[test]
    fn no_rigged_face_means_nothing_drawn_with_depth() {
        let mut e = [entry(1, 2.0, false), entry(2, 4.0, false)];
        assert_eq!(sort(&mut e, true), 0);
        assert_eq!(records(&e), [2, 1]);
    }
}
