//! Avatar Studio shape handles (#5022): dots on the avatar that drive the
//! body and face sliders of the shape draft. Aurora's own: the Legacy Viewer
//! edits a shape with its sliders only (LLPanelEditWearable).
//!
//! Nothing here is a new piece of data: a handle is a point on a collision
//! volume or a face bone and the one or two existing sliders that move it
//! most. The mapping is derived from avatar_lad.xml's parameters by moving
//! each slider across its range on the rest skeleton (`derive`), and a drag
//! solves, in screen space, for the slider values that bring the point
//! under the cursor (`Drag::step`): the sliders stay the source of truth.

use super::catalog;
use crate::scene::{
    anim::Rig,
    avatar::{AvatarLibrary, AvatarPart},
    shape::ShapeParams,
};
use glam::{Mat4, Quat, Vec2, Vec3};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The handles shown at once: the body, or the head, framed by the camera.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Part {
    #[default]
    Body,
    Head,
}

impl Part {
    /// The shape editor's tabs whose sliders it holds: Body, Torso and
    /// Legs; Head, Eyes, Ears, Nose, Mouth and Chin.
    pub fn groups(self) -> &'static [usize] {
        match self {
            Part::Body => &[0, 7, 8],
            Part::Head => &[1, 2, 3, 4, 5, 6],
        }
    }
    /// Below this movement over its whole range (m), a slider gets no
    /// handle: a face slider moves its bone a few millimetres.
    fn min_reach(self) -> f32 {
        match self {
            Part::Body => 0.005,
            Part::Head => 0.0015,
        }
    }
}

/// Body sliders that get no handle: Hover lifts the whole avatar (no shape
/// change; the Studio bar has its own Hover). The face sliders that only
/// reshape the system head's mesh (Chin Cleft, Jowls, Eyelash Length…) move
/// no joint, so they get none either, by `derive`'s own rule.
const LEFT_OUT: [i32; 1] = [11001];
/// "male" driver parameter.
const PARAM_MALE: i32 = 80;
/// How far from a face bone its handle points sit (m): a bone has no
/// surface of its own, the dot sits just off it, facing the way it moves.
const FACE_POINT: f32 = 0.008;
/// Two sliders share a handle only when they move its point in directions
/// at least this far apart (|cos|), so that a drag can tell them apart.
const MAX_SHARED_COS: f32 = 0.8;
/// A slider goes to its best point or one nearly as good (fraction of the
/// best score) when the best is taken.
const SECOND_CHOICE: f32 = 0.6;

/// A point on a collision volume: `local` is a point of its unit sphere,
/// which the volume's own scale turns into its ellipsoid. Or, on the system
/// head, a vertex of its mesh (`vertex`), where the face sliders' morphs
/// show: `joint` is then the bone the vertex follows most and `local` the
/// vertex at rest in that bone's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub joint: usize,
    pub local: Vec3,
    pub vertex: Option<u32>,
}

impl Anchor {
    /// A point of a volume's ellipsoid, or just off a bone.
    pub fn on(joint: usize, local: Vec3) -> Self {
        Self {
            joint,
            local,
            vertex: None,
        }
    }
    /// Where the point is for these joint matrices (avatar or world space),
    /// without the morphs ([`Body::drawn`] has them).
    pub fn at(&self, joints: &[Mat4]) -> Option<Vec3> {
        joints.get(self.joint).map(|m| m.transform_point3(self.local))
    }
    /// The direction the surface faces at the point (a volume's).
    pub fn normal(&self, joints: &[Mat4]) -> Option<Vec3> {
        let m = joints.get(self.joint)?;
        let (scale, rotation, _) = m.to_scale_rotation_translation();
        // the ellipsoid's normal: the unit sphere's, divided by the scale
        (rotation * (self.local / scale.max(Vec3::splat(1e-4)))).try_normalize()
    }
}

/// The system head's mesh, whose vertices hold the face handles while it
/// shows: the face sliders mostly reshape it through morphs, which move no
/// bone where they show.
fn head_part(lib: &AvatarLibrary) -> Option<&AvatarPart> {
    lib.parts.iter().find(|p| p.name == "head")
}

/// A vertex of a system body part with its morphs at `weights` (every
/// visual parameter's effective weight), skinned by `joints` as the avatar
/// draws it (`world × default_world_inv`, avatar.rs).
fn surface(rig: &Rig, part: &AvatarPart, weights: &HashMap<i32, f32>, joints: &[Mat4], vertex: u32) -> Option<Vec3> {
    let mut position = Vec3::from_array(part.vertices.get(vertex as usize)?.pos);
    for morph in &part.morphs {
        let w = weights.get(&morph.param).copied().filter(|w| w.is_finite()).unwrap_or(0.0);
        if w != 0.0
            && let Some(delta) = morph.delta(vertex)
        {
            position += delta * w;
        }
    }
    skinned(rig, part, joints, vertex, position)
}

/// Every vertex of a part with its morphs at `weights`, at rest (before
/// the skin): what [`derive`] compares across a slider's range.
fn morphed_positions(part: &AvatarPart, weights: &HashMap<i32, f32>) -> Vec<Vec3> {
    let mut positions: Vec<Vec3> = part.vertices.iter().map(|v| Vec3::from_array(v.pos)).collect();
    for morph in &part.morphs {
        let w = weights.get(&morph.param).copied().filter(|w| w.is_finite()).unwrap_or(0.0);
        if w != 0.0 {
            morph.add_to(&mut positions, w);
        }
    }
    positions
}

/// A vertex's rest `position` skinned by `joints`.
fn skinned(rig: &Rig, part: &AvatarPart, joints: &[Mat4], vertex: u32, position: Vec3) -> Option<Vec3> {
    let skin = part.skin.get(vertex as usize)?;
    let (mut sum, mut total) = (Vec3::ZERO, 0.0);
    for (joint, weight) in skin.joints.iter().zip(skin.weights) {
        let (Some(m), Some(inverse)) = (joints.get(*joint as usize), rig.default_world_inv.get(*joint as usize)) else {
            continue;
        };
        let w = f32::from(weight) / f32::from(u16::MAX);
        if w > 0.0 {
            sum += (*m * *inverse).transform_point3(position) * w;
            total += w;
        }
    }
    (total > 0.0).then(|| sum / total)
}

/// The rig joint a vertex follows most.
fn main_joint(part: &AvatarPart, vertex: u32) -> Option<usize> {
    let skin = part.skin.get(vertex as usize)?;
    skin.joints
        .iter()
        .zip(skin.weights)
        .max_by_key(|(_, w)| *w)
        .map(|(j, _)| *j as usize)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Handle {
    pub anchor: Anchor,
    /// The same point on the other side (a left volume's right twin, or the
    /// other side of a volume on the midline): one slider moves both.
    pub mirror: Option<Anchor>,
    /// One or two slider ids, the one that moves the point most first.
    pub params: Vec<i32>,
    /// The way each slider moves the point as it grows, in the volume's own
    /// frame (the twin's: Y mirrored), to fade a dot seen along its motion.
    pub directions: Vec<Vec3>,
    /// Whether the handle resizes its part rather than moving it
    /// ([`resizes`]): drawn as a grow mark with the part's smallest and
    /// largest outlines, dragged away from or towards its centre.
    pub resize: bool,
    /// The area of the avatar it belongs to ([`area_of`]): the view shows
    /// the areas first, then the dots of the one picked.
    pub area: usize,
}

impl Handle {
    pub fn anchor(&self, mirrored: bool) -> Anchor {
        if mirrored {
            self.mirror.unwrap_or(self.anchor)
        } else {
            self.anchor
        }
    }
}

/// Whether a slider resizes the volume its handle sits on rather than moving
/// it (Hand Size, Foot Size, Neck Thickness…): over the slider's range the
/// volume's centre moves much less than the handle point, and the volume
/// grows on at least two of its axes. A part pushed out one way (Belly,
/// Butt) is a slide. `lo` / `hi`: the volume's matrix at the slider's ends;
/// `reach`: how far the handle point moves.
pub fn resizes(lo: Mat4, hi: Mat4, reach: f32) -> bool {
    let (scale_lo, _, centre_lo) = lo.to_scale_rotation_translation();
    let (scale_hi, _, centre_hi) = hi.to_scale_rotation_translation();
    let grown = (scale_hi - scale_lo).abs();
    let most = grown.max_element();
    if reach <= 0.0 || most < 1e-4 {
        return false;
    }
    let axes = grown.to_array().iter().filter(|g| **g >= most * 0.5).count();
    centre_lo.distance(centre_hi) < reach * RESIZE_CENTRE && axes >= 2
}

/// A resize moves its part's centre less than this share of the handle
/// point's motion.
const RESIZE_CENTRE: f32 = 0.8;

/// The outline of a collision volume as seen along `view` (world space):
/// the points of its ellipsoid where the surface turns away from the eye,
/// for the smallest and largest sizes a resize handle shows. Exact for a
/// distant eye, close for the Studio's.
pub fn outline(volume: Mat4, view: Vec3, samples: usize) -> Vec<Vec3> {
    let linear = glam::Mat3::from_mat4(volume);
    // the unit sphere's points whose image's normal is across the view
    let Some(normal) = (linear.transpose() * view).try_normalize() else {
        return Vec::new();
    };
    let side = normal.any_orthonormal_vector();
    let up = normal.cross(side);
    let samples = samples.max(3);
    (0..samples)
        .map(|i| {
            let angle = std::f32::consts::TAU * i as f32 / samples as f32;
            volume.transform_point3(side * angle.cos() + up * angle.sin())
        })
        .collect()
}

/// The handles of one sex and part, and the joints each of their sliders
/// moves (for the mesh body rule, [`shown_params`]).
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub male: bool,
    pub part: Part,
    /// The head's handles are on the system head's mesh ([`derive`]).
    pub system_head: bool,
    pub handles: Vec<Handle>,
    pub moves: HashMap<i32, HashSet<usize>>,
}

/// The other side's joint name: `L_` / `R_`, `LEFT_` / `RIGHT_`, or a
/// `Left` / `Right` suffix (the bones), as in avatar_skeleton.xml.
fn mirror_name(name: &str) -> Option<String> {
    for (a, b) in [("L_", "R_"), ("R_", "L_"), ("LEFT_", "RIGHT_"), ("RIGHT_", "LEFT_")] {
        if let Some(rest) = name.strip_prefix(a) {
            return Some(format!("{b}{rest}"));
        }
    }
    for (a, b) in [("Left", "Right"), ("Right", "Left")] {
        if let Some(rest) = name.strip_suffix(a) {
            return Some(format!("{rest}{b}"));
        }
    }
    None
}

fn is_right(name: &str) -> bool {
    name.starts_with("R_") || name.starts_with("RIGHT_") || name.ends_with("Right")
}

fn mirrored(local: Vec3) -> Vec3 {
    Vec3::new(local.x, -local.y, local.z)
}

/// The left-side form of a point and its twin on the other side: the avatar
/// is symmetric about its XZ plane (+Y is its left), and the skeleton's
/// volumes keep the same axes on both sides.
fn canonical(rig: &Rig, anchor: Anchor) -> (Anchor, Option<Anchor>) {
    let name = &rig.canonical_names[anchor.joint];
    let twin = mirror_name(name).and_then(|n| rig.names.get(&n).copied());
    match twin {
        Some(twin) if is_right(name) => (
            Anchor::on(twin, mirrored(anchor.local)),
            Some(Anchor::on(anchor.joint, anchor.local)),
        ),
        Some(twin) => (anchor, Some(Anchor::on(twin, mirrored(anchor.local)))),
        // on the midline: a side point's twin is the other side of the
        // same volume; a front, back, top or bottom point has none
        None if anchor.local.normalize_or_zero().y < -0.5 => (Anchor::on(anchor.joint, mirrored(anchor.local)), Some(anchor)),
        None if anchor.local.normalize_or_zero().y > 0.5 => (anchor, Some(Anchor::on(anchor.joint, mirrored(anchor.local)))),
        None => (anchor, None),
    }
}

/// The joints whose lowest one stands on the floor.
fn feet(rig: &Rig) -> Vec<usize> {
    ["mFootLeft", "mFootRight", "mToeLeft", "mToeRight"]
        .iter()
        .filter_map(|n| rig.names.get(*n).copied())
        .collect()
}

/// The height of the lowest foot joint.
fn ground(joints: &[Mat4], feet: &[usize]) -> f32 {
    feet.iter()
        .filter_map(|f| joints.get(*f))
        .map(|m| m.w_axis.z)
        .reduce(f32::min)
        .unwrap_or(0.0)
}

/// The skeleton of a shape on the rest pose, standing on the floor as the
/// avatar is drawn: a longer leg lifts the body rather than sinking the
/// feet, so a handle goes where the slider shows.
fn rest_joints(rig: &Rig, shape: &crate::scene::shape::Shape) -> Vec<Mat4> {
    let mut joints = rig.world_with(&rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
    let floor = ground(&joints, &feet(rig));
    for m in &mut joints {
        m.w_axis.z -= floor;
    }
    joints
}

/// The skeleton at rest with a sex's base shape: the frame [`derive`] places
/// its anchors in (face_surface.rs reads a mesh head on it).
pub fn base_rest_joints(lib: &AvatarLibrary, male: bool) -> Vec<Mat4> {
    let sex = BTreeMap::from([(PARAM_MALE, f32::from(u8::from(male)))]);
    let base = lib.shape_params.weights_with(&[], &sex);
    rest_joints(&lib.rig, &lib.shape_params.shape_weights(base, &lib.rig))
}

/// Where two of these volumes meet when one hangs below the other (the
/// thigh and the shin: the knee), as `joints` place it; none otherwise.
fn meeting_point(rig: &Rig, joints: &[Mat4], volumes: impl Iterator<Item = usize>) -> Option<Vec3> {
    let bones: HashSet<usize> = volumes.filter_map(|v| rig.parent[v]).collect();
    bones
        .iter()
        .find(|b| rig.parent[**b].is_some_and(|p| bones.contains(&p)))
        .and_then(|b| joints.get(*b))
        .map(|m| m.w_axis.truncate())
}

/// The joints whose matrix differs between two skeletons.
fn moved_joints(a: &[Mat4], b: &[Mat4]) -> HashSet<usize> {
    a.iter()
        .zip(b)
        .enumerate()
        .filter(|(_, (a, b))| {
            let (sa, _, ta) = a.to_scale_rotation_translation();
            let (sb, _, tb) = b.to_scale_rotation_translation();
            ta.distance(tb) > 0.0005 || (sa - sb).abs().max_element() > 0.002
        })
        .map(|(j, _)| j)
        .collect()
}

/// Whether a joint carries face handles: the Bento face bones (`mFace…`)
/// and the eyes, which the face sliders move; not the tongue and teeth,
/// inside the mouth, whose dots would float over the lips.
fn face_bone(name: &str) -> bool {
    let inside = name.contains("Tongue") || name.contains("Teeth");
    (name.starts_with("mFace") && !inside) || name == "mEyeLeft" || name == "mEyeRight"
}

/// The directions a handle point may lie in from its volume's centre or its
/// bone: the six axes, then the twelve ways between two of them (the lower
/// side, the upper front…), so that two sliders pushing the same point the
/// same way can each have one (Saddle Bags below Hip Width).
fn directions() -> Vec<Vec3> {
    let axes = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
    let mut out = axes.to_vec();
    for (i, a) in axes.iter().enumerate() {
        for b in &axes[i + 1..] {
            // two different axes, not opposite ones
            if a.dot(*b).abs() < 0.5 {
                out.push((*a + *b).normalize());
            }
        }
    }
    out
}

/// Where a handle of `part` may sit: points of each collision volume's
/// ellipsoid (the body), or just off each face bone and on the head's
/// volume (the head), in each of [`directions`].
fn candidates(lib: &AvatarLibrary, part: Part) -> Vec<Anchor> {
    let Some(skeleton) = &lib.skeleton else {
        return Vec::new();
    };
    let axes = directions();
    let head = lib.rig.names.get("HEAD").copied();
    skeleton
        .joints
        .iter()
        .enumerate()
        .take(lib.rig.len())
        .flat_map(|(joint, info)| {
            let name = &lib.rig.canonical_names[joint];
            let scale = match part {
                Part::Body if info.is_collision_volume => Some(1.0),
                Part::Head if Some(joint) == head => Some(1.0),
                Part::Head if face_bone(name) => Some(FACE_POINT),
                _ => None,
            };
            let axes = &axes;
            scale
                .into_iter()
                .flat_map(move |scale| axes.iter().map(move |axis| Anchor::on(joint, *axis * scale)))
        })
        .collect()
}

/// A place a slider could put its handle, scored.
#[derive(Clone, Copy, Debug)]
struct Choice {
    score: f32,
    /// The way the slider moves the point, in its joint's frame.
    direction: Vec3,
    anchor: Anchor,
    twin: Option<Anchor>,
    /// How directly the slider reshapes the volume: itself (0), through
    /// its bone (1) or the bone above (2).
    level: Option<u8>,
    /// The point with the slider at its maximum (rest skeleton, feet on
    /// the floor).
    at: Vec3,
    resize: bool,
}

/// Two face dots on the system head's mesh keep at least this far apart
/// (m), so that each can be taken; a slider whose best vertex is that close
/// to a taken one shares its dot or takes its next best.
const MESH_GAP: f32 = 0.012;

/// The system head's vertices that may hold a handle, on the canonical
/// (left, +Y) side and the midline, with their twin on the other side.
fn head_vertices(lib: &AvatarLibrary, head: &AvatarPart) -> Vec<(Anchor, Option<Anchor>)> {
    let rig = &lib.rig;
    let positions: Vec<Vec3> = head.vertices.iter().map(|v| Vec3::from_array(v.pos)).collect();
    let anchor = |vertex: usize| {
        let joint = main_joint(head, vertex as u32)?;
        let local = rig.default_world_inv.get(joint)?.transform_point3(positions[vertex]);
        Some(Anchor {
            joint,
            local,
            vertex: Some(vertex as u32),
        })
    };
    // the file repeats a vertex along its UV seams: one of each place
    let mut seen: Vec<Vec3> = Vec::new();
    let mut out = Vec::new();
    for (vertex, p) in positions.iter().enumerate() {
        if p.y < -MIDLINE || seen.iter().any(|q| q.distance_squared(*p) < 1e-8) {
            continue;
        }
        seen.push(*p);
        let Some(canon) = anchor(vertex) else { continue };
        let twin = (p.y > MIDLINE)
            .then(|| {
                let want = mirrored(*p);
                positions
                    .iter()
                    .enumerate()
                    .map(|(i, q)| (i, q.distance_squared(want)))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .filter(|(_, d)| *d < MIRROR_MATCH * MIRROR_MATCH)
                    .and_then(|(i, _)| anchor(i))
            })
            .flatten();
        out.push((canon, twin));
    }
    out
}

/// A head vertex this close to the midline (m) has no twin.
const MIDLINE: f32 = 0.003;
/// How close the mirror of a vertex must find one on the other side (m).
const MIRROR_MATCH: f32 = 0.004;

/// Where each slider of this sex and part puts its handle, and which joints
/// it moves. A slider goes to the point it moves most over its range (feet
/// on the floor), the point facing the way it moves preferred; among points
/// it moves nearly as much (a longer leg lifts the whole body, wider hips
/// shift the whole leg), one on a volume it reshapes itself or through the
/// bones just above it (the thigh, not the foot; the head for Height).
/// Sliders that land on the same point share its handle, two at most and
/// only when they move it in clearly different directions; a slider that
/// finds no room takes its next best point if nearly as good, or stays a
/// slider only.
///
/// The head's points are the face bones, which a mesh head follows; while
/// the system head shows (`system_head`), the vertices of its mesh instead,
/// where the face sliders' morphs show (Egg Head on the crown, not on the
/// chin bone its skeleton part moves most).
pub fn derive(lib: &AvatarLibrary, male: bool, part: Part, system_head: bool) -> Layout {
    let params = &lib.shape_params;
    let rig = &lib.rig;
    let skeleton = lib.skeleton.as_ref();
    let sex = BTreeMap::from([(PARAM_MALE, f32::from(u8::from(male)))]);
    let base = params.weights_with(&[], &sex);
    let head = head_part(lib).filter(|_| part == Part::Head && system_head);
    // each point once, on its canonical side, in the skeleton's order (which
    // settles ties)
    let mut points: Vec<(Anchor, Option<Anchor>)> = Vec::new();
    for anchor in candidates(lib, part) {
        if head.is_some() && face_bone(&rig.canonical_names[anchor.joint]) {
            continue;
        }
        let pair = canonical(rig, anchor);
        if !points.iter().any(|p| p.0 == pair.0) {
            points.push(pair);
        }
    }
    if let Some(head) = head {
        points.extend(head_vertices(lib, head));
    }
    // where a head vertex is at rest, to keep the face dots apart
    let rest = head.map(|h| {
        let joints = rest_joints(rig, &params.shape_weights(base.clone(), rig));
        (h, joints, morphed_positions(h, &base))
    });
    let at_rest = |anchor: Anchor| -> Option<Vec3> {
        let (h, joints, positions) = rest.as_ref()?;
        let v = anchor.vertex?;
        skinned(rig, h, joints, v, *positions.get(v as usize)?)
    };
    let mut layout = Layout {
        male,
        part,
        system_head: head.is_some(),
        ..Default::default()
    };
    let mut scored: Vec<(i32, f32, Vec<Choice>)> = Vec::new();
    let sex_bit = if male { 2 } else { 1 };
    for param in catalog::PARAMS
        .iter()
        .filter(|p| part.groups().contains(&p.group) && p.sex & sex_bit != 0 && !LEFT_OUT.contains(&p.id))
    {
        let weights = |w: f32| {
            let mut changed = sex.clone();
            changed.insert(param.id, w);
            params.with_changes(base.clone(), &changed)
        };
        let (weights_lo, weights_hi) = (weights(param.min), weights(param.max));
        let (shape_lo, shape_hi) = (
            params.shape_weights(weights_lo.clone(), rig),
            params.shape_weights(weights_hi.clone(), rig),
        );
        let (lo, hi) = (rest_joints(rig, &shape_lo), rest_joints(rig, &shape_hi));
        let moves = moved_joints(&lo, &hi);
        let meshes = head.map(|h| (h, morphed_positions(h, &weights_lo), morphed_positions(h, &weights_hi)));
        if moves.is_empty() && meshes.is_none() {
            continue;
        }
        // the joints whose own offset or size the slider changes
        let reshaped =
            |j: usize| shape_lo.local_pos[j].distance(shape_hi.local_pos[j]) > 1e-4 || shape_lo.scale[j].distance(shape_hi.scale[j]) > 1e-4;
        let level = |j: usize| {
            let parent = rig.parent[j];
            let grandparent = parent.and_then(|p| rig.parent[p]);
            [Some(j), parent, grandparent]
                .iter()
                .position(|j| j.is_some_and(reshaped))
                .map(|level| level as u8)
        };
        // the point at both ends of the range, and the way its surface faces
        let ends = |anchor: &Anchor| -> Option<(Vec3, Vec3, Vec3)> {
            match (anchor.vertex, &meshes) {
                (Some(v), Some((h, at_lo, at_hi))) => {
                    let normal = *h.normals.get(v as usize)?;
                    let rotation = hi.get(anchor.joint)?.to_scale_rotation_translation().1;
                    Some((
                        skinned(rig, h, &lo, v, *at_lo.get(v as usize)?)?,
                        skinned(rig, h, &hi, v, *at_hi.get(v as usize)?)?,
                        (rotation * normal).try_normalize()?,
                    ))
                }
                (Some(_), None) => None,
                (None, _) => Some((anchor.at(&lo)?, anchor.at(&hi)?, anchor.normal(&hi)?)),
            }
        };
        let mut options: Vec<Choice> = Vec::new();
        for (canon, twin) in &points {
            // the canonical side and its twin move alike: the better of the
            // two speaks for the pair
            let mut best: Option<Choice> = None;
            for (anchor, is_twin) in [(Some(*canon), false), (*twin, true)] {
                let Some(anchor) = anchor else { continue };
                let Some((a, b, normal)) = ends(&anchor) else { continue };
                let moved = b - a;
                let reach = moved.length();
                if reach < part.min_reach() {
                    continue;
                }
                let facing = normal.dot(moved / reach).max(0.0);
                let score = reach * (0.75 + 0.25 * facing);
                if best.is_some_and(|b| b.score >= score) {
                    continue;
                }
                // the twin's direction, mirrored, is the canonical side's;
                // kept in the canonical joint's frame, which the pose turns
                let world = if is_twin { mirrored(moved) / reach } else { moved / reach };
                let rotation = hi.get(canon.joint).map_or(Quat::IDENTITY, |m| m.to_scale_rotation_translation().1);
                best = Some(Choice {
                    score,
                    direction: rotation.inverse() * world,
                    anchor: *canon,
                    twin: *twin,
                    level: anchor.vertex.is_none().then(|| level(anchor.joint)).flatten(),
                    at: if is_twin { mirrored(b) } else { b },
                    resize: anchor.vertex.is_none() && resizes(lo[anchor.joint], hi[anchor.joint], reach),
                });
            }
            options.extend(best);
        }
        let Some(best) = options.iter().map(|o| o.score).reduce(f32::max) else {
            continue;
        };
        // within 10 % of the best, a reshaped volume first: where a chain of
        // volumes the slider shifts itself meet (Knee Angle at the knee);
        // otherwise the highest (Height on the head, not the neck), then
        // the most directly reshaped (Arm Length on the forearm, not the
        // hand Hand Size holds); then by score
        let near = |score: f32| score >= best * 0.9;
        let volume = |j: usize| skeleton.is_some_and(|s| s.joints.get(j).is_some_and(|info| info.is_collision_volume));
        let shifts_volumes_only = (0..rig.len()).all(|j| volume(j) || !reshaped(j));
        let meeting = shifts_volumes_only
            .then(|| {
                meeting_point(
                    rig,
                    &hi,
                    options
                        .iter()
                        .filter(|o| near(o.score) && o.level == Some(0))
                        .map(|o| o.anchor.joint),
                )
            })
            .flatten();
        let distance = |o: &Choice| meeting.map_or(0.0, |m| m.distance(o.at));
        let band = |height: f32| (height * 100.0).round() as i32;
        options.sort_by(|a, b| {
            let (na, nb) = (near(a.score) && a.level.is_some(), near(b.score) && b.level.is_some());
            nb.cmp(&na).then(if na && nb {
                distance(a)
                    .total_cmp(&distance(b))
                    .then(band(b.at.z).cmp(&band(a.at.z)))
                    .then(a.level.cmp(&b.level))
                    .then(b.score.total_cmp(&a.score))
            } else {
                b.score.total_cmp(&a.score)
            })
        });
        layout.moves.insert(param.id, moves);
        scored.push((param.id, best, options));
    }
    // strongest sliders choose first, by id among equals (stable)
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut directions: HashMap<i32, Vec3> = HashMap::new();
    // where each head-mesh handle is at rest
    let mut placed: Vec<Option<Vec3>> = Vec::new();
    for (id, best, options) in scored {
        // a resize is dragged radially, so any point of a part it resizes
        // serves (Hand Size when Arm Length holds the side of the hand)
        for choice in options.into_iter().filter(|o| o.score >= best * SECOND_CHOICE || o.resize) {
            // the same point, or on the head's mesh one too close to tell apart
            let rest = at_rest(choice.anchor);
            let taken = layout
                .handles
                .iter()
                .zip(&placed)
                .position(|(h, at)| h.anchor == choice.anchor || at.zip(rest).is_some_and(|(a, b)| a.distance(b) < MESH_GAP));
            match taken {
                // a dot either slides or resizes: one drag cannot do both
                Some(i)
                    if layout.handles[i].params.len() < 2
                        && layout.handles[i].resize == choice.resize
                        && layout.handles[i]
                            .params
                            .iter()
                            .all(|other| directions[other].dot(choice.direction).abs() < MAX_SHARED_COS)
                        // a dot belongs to one area (Eyes, Arms…)
                        && layout.handles[i].area == area_of(rig, part, choice.anchor, id) =>
                {
                    let handle = &mut layout.handles[i];
                    handle.params.push(id);
                    // the shared dot's own frame: a vertex's twin keeps it
                    handle.directions.push(choice.direction);
                }
                Some(_) => continue,
                None => {
                    layout.handles.push(Handle {
                        anchor: choice.anchor,
                        mirror: choice.twin,
                        params: vec![id],
                        directions: vec![choice.direction],
                        resize: choice.resize,
                        area: area_of(rig, part, choice.anchor, id),
                    });
                    placed.push(rest);
                }
            }
            directions.insert(id, choice.direction);
            break;
        }
    }
    layout.moves.retain(|id, _| directions.contains_key(id));
    if part == Part::Head && !system_head {
        lift_to_face(lib, &base, &mut layout);
        spread_face(lib, &base, &mut layout);
    }
    layout
}

/// The least distance between two face dots of one area under a mesh head
/// (m): a dot's width at the head's close up.
const FACE_GAP: f32 = 0.007;

/// Under a mesh head, several sliders' dots land on one spot: the lip bones
/// lift to the same point of the system head's surface, where Lip Thickness,
/// Lip Cleft Depth, Lip Ratio, Lip Fullness and Mouth Position met, and a
/// dot on the midline had its twin on top of it. The twin of a dot on the
/// midline goes; then the dots of each area (those shown together) are
/// pushed apart across the face, never along the way out from the head's
/// centre (face_surface.rs moves each along that way onto the worn head),
/// until [`FACE_GAP`] apart; a dot on the midline only up or down, so that
/// it stays there.
fn spread_face(lib: &AvatarLibrary, base: &HashMap<i32, f32>, layout: &mut Layout) {
    let rig = &lib.rig;
    let joints = rest_joints(rig, &lib.shape_params.shape_weights(base.clone(), rig));
    let Some(centre) = rig.names.get("HEAD").and_then(|&j| joints.get(j)).map(|m| m.w_axis.truncate()) else {
        return;
    };
    for handle in &mut layout.handles {
        let (Some(at), Some(twin)) = (handle.anchor.at(&joints), handle.mirror.and_then(|m| m.at(&joints))) else {
            continue;
        };
        if at.distance(twin) < 0.001 {
            handle.mirror = None;
        }
    }
    let areas: Vec<usize> = layout.handles.iter().map(|h| h.area).collect();
    for area in areas.iter().copied().collect::<std::collections::BTreeSet<_>>() {
        // (handle, mirrored, where at rest)
        let mut dots: Vec<(usize, bool, Vec3)> = Vec::new();
        for (index, handle) in layout.handles.iter().enumerate().filter(|(_, h)| h.area == area) {
            for (mirrored, anchor) in [(false, Some(handle.anchor)), (true, handle.mirror)] {
                if let Some(at) = anchor.and_then(|a| a.at(&joints)) {
                    dots.push((index, mirrored, at));
                }
            }
        }
        // each round's pushes taken together: the two sides stay mirrored
        for _ in 0..80 {
            let mut moved = false;
            let mut pushes = vec![Vec3::ZERO; dots.len()];
            for i in 0..dots.len() {
                for k in i + 1..dots.len() {
                    let (a, b) = (dots[i].2, dots[k].2);
                    let out = ((a + b) * 0.5 - centre).normalize_or_zero();
                    let apart = (a - b).reject_from_normalized(out);
                    let gap = apart.length();
                    if gap >= FACE_GAP {
                        continue;
                    }
                    // one spot: one up, the other down
                    let way = if gap > 1e-5 {
                        apart / gap
                    } else {
                        Vec3::Z.reject_from_normalized(out).normalize_or_zero()
                    };
                    let push = way * (FACE_GAP - gap) * 0.5;
                    let midline = |p: Vec3, push: Vec3| if p.y.abs() < 0.001 { Vec3::new(push.x, 0.0, push.z) } else { push };
                    pushes[i] += midline(a, push);
                    pushes[k] -= midline(b, push);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
            for (dot, push) in dots.iter_mut().zip(pushes) {
                dot.2 += push;
            }
        }
        for (index, mirrored, at) in dots {
            let handle = &mut layout.handles[index];
            let anchor = if mirrored {
                handle.mirror.as_mut()
            } else {
                Some(&mut handle.anchor)
            };
            if let Some(anchor) = anchor
                && let Some(bone) = joints.get(anchor.joint)
            {
                anchor.local = bone.inverse().transform_point3(at);
            }
        }
    }
}

/// How far the system head's surface may be from a face bone for that
/// bone's dots to be moved onto it ([`lift_to_face`]), in metres: the lips'
/// and cheeks' bones lie one to two centimetres under it; the jaw's, deep
/// in the head, keeps its dots.
const SURFACE_REACH: f32 = 0.03;

/// Under a mesh head, the face dots would sit on its bones, inside the head:
/// a Bento head's lips and cheeks follow bones a centimetre or more under
/// its skin. Each bone's dots are moved by the way from the bone to the
/// nearest point of the system head's surface (dots of one bone keep their
/// spacing), and kept in the bone's own frame: they follow the bone where a
/// head's joint positions put it (a furry head's snout takes its dots
/// along) and grow with it (Upper Cheeks).
fn lift_to_face(lib: &AvatarLibrary, base: &HashMap<i32, f32>, layout: &mut Layout) {
    let rig = &lib.rig;
    let Some(head) = head_part(lib) else {
        return;
    };
    let joints = rest_joints(rig, &lib.shape_params.shape_weights(base.clone(), rig));
    let positions = morphed_positions(head, base);
    let surface: Vec<Vec3> = (0..head.vertices.len() as u32)
        .filter_map(|v| skinned(rig, head, &joints, v, *positions.get(v as usize)?))
        .collect();
    let mut way_out: HashMap<usize, Option<Vec3>> = HashMap::new();
    let mut lift = |anchor: Anchor| -> Anchor {
        let Some(bone) = joints.get(anchor.joint) else {
            return anchor;
        };
        let origin = bone.w_axis.truncate();
        let way = *way_out.entry(anchor.joint).or_insert_with(|| {
            surface
                .iter()
                .map(|v| *v - origin)
                .filter(|d| d.length() < SURFACE_REACH)
                .min_by(|a, b| a.length().total_cmp(&b.length()))
        });
        match (way, anchor.at(&joints)) {
            (Some(way), Some(at)) => Anchor {
                local: bone.inverse().transform_point3(at + way),
                ..anchor
            },
            _ => anchor,
        }
    };
    for handle in &mut layout.handles {
        handle.anchor = lift(handle.anchor);
        handle.mirror = handle.mirror.map(&mut lift);
    }
}

/// The areas of the body: where its dots sit on the skeleton. The head's are
/// the shape editor's face tabs (1 to 6, `catalog::Param::group`).
pub const AREA_HEAD_NECK: usize = 100;
pub const AREA_TORSO: usize = 101;
pub const AREA_ARMS: usize = 102;
pub const AREA_LEGS: usize = 103;

/// The area a slider's dot belongs to. On the head, the shape editor's tab of
/// the slider (Head, Eyes, Ears, Nose, Mouth, Chin): its sliders are grouped
/// by the part of the face they shape. On the body, the part of the skeleton
/// the dot sits on, since the editor's Body tab (Height, Body Thickness,
/// Body Fat) spans the whole avatar: below a collarbone the arms, below a
/// hip or on the pelvis and butt the legs, the neck and head, the torso
/// otherwise.
pub fn area_of(rig: &Rig, part: Part, anchor: Anchor, id: i32) -> usize {
    if part == Part::Head {
        return catalog::PARAMS.iter().find(|p| p.id == id).map_or(0, |p| p.group);
    }
    let named = |j: usize, names: &[&str]| names.contains(&rig.canonical_names[j].as_str());
    let mut joint = Some(anchor.joint);
    if named(anchor.joint, &["PELVIS", "BUTT"]) {
        return AREA_LEGS;
    }
    while let Some(j) = joint {
        if named(j, &["mCollarLeft", "mCollarRight"]) {
            return AREA_ARMS;
        }
        // (a feral's hind legs are the Bento hind limbs)
        if named(j, &["mHipLeft", "mHipRight"]) || rig.canonical_names[j].starts_with("mHindLimb") {
            return AREA_LEGS;
        }
        if named(j, &["mNeck", "mHead"]) {
            return AREA_HEAD_NECK;
        }
        joint = rig.parent[j];
    }
    AREA_TORSO
}

/// The sliders of `handle` that move something drawn: all of them while the
/// system body shows; with it hidden (a mesh body), only those that move a
/// joint the worn rigged meshes are skinned to, so that a drag always shows.
pub fn shown_params<'a>(
    layout: &'a Layout,
    handle: &'a Handle,
    system_body: bool,
    rigged: &'a HashSet<usize>,
) -> impl Iterator<Item = i32> + 'a {
    handle
        .params
        .iter()
        .copied()
        .filter(move |id| system_body || layout.moves.get(id).is_some_and(|moves| !moves.is_disjoint(rigged)))
}

/// The pose a drag holds still: each joint's rotation relative to its
/// parent, taken from the drawn skeleton, and the avatar's model matrix.
#[derive(Clone, Debug)]
pub struct Pose {
    pub rot: Vec<Quat>,
    pub model: Mat4,
}

impl Pose {
    /// `joints`: the drawn joint matrices in world space (`ik_edit::Frame`).
    pub fn from_joints(rig: &Rig, model: Mat4, joints: &[Mat4]) -> Option<Self> {
        if joints.len() < rig.len() {
            return None;
        }
        let inverse = model.inverse();
        let world: Vec<Quat> = joints
            .iter()
            .take(rig.len())
            .map(|m| (inverse * *m).to_scale_rotation_translation().1)
            .collect();
        let rot = (0..rig.len())
            .map(|j| match rig.parent[j] {
                Some(p) => (world[p].inverse() * world[j]).normalize(),
                None => world[j],
            })
            .collect();
        Some(Self { rot, model })
    }
}

/// The body for a drag: the appearance values' weights expanded once, the
/// pose, and the feet that stay on the floor.
pub struct Body<'a> {
    pub params: &'a ShapeParams,
    pub rig: &'a Rig,
    pub base: HashMap<i32, f32>,
    pub pose: Pose,
    feet: Vec<usize>,
    head: Option<&'a AvatarPart>,
}

impl<'a> Body<'a> {
    pub fn new(lib: &'a AvatarLibrary, values: &[u8], pose: Pose) -> Self {
        let feet = feet(&lib.rig);
        Self {
            params: &lib.shape_params,
            rig: &lib.rig,
            base: lib.shape_params.weights(values),
            pose,
            feet,
            head: head_part(lib),
        }
    }
    /// Every weight and the joints in world space for these slider values:
    /// the skeleton of the shape in the held pose, lifted or lowered so that
    /// the lowest foot keeps its height, as the avatar stands on the ground
    /// whatever its legs (the Studio grounds its presets the same way).
    fn skeleton(&self, sliders: &BTreeMap<i32, f32>) -> (HashMap<i32, f32>, Vec<Mat4>) {
        let weights = self.params.with_changes(self.base.clone(), sliders);
        let shape = self.params.shape_weights(weights.clone(), self.rig);
        let joints = self.rig.world_with(&self.pose.rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
        let lift = self.pose.model * Mat4::from_translation(-Vec3::Z * ground(&joints, &self.feet));
        (weights, joints.into_iter().map(|m| lift * m).collect())
    }
    /// The point in world space for these slider values ([`Self::skeleton`]),
    /// a head vertex with its morphs.
    pub fn point(&self, sliders: &BTreeMap<i32, f32>, anchor: Anchor) -> Option<Vec3> {
        let (weights, joints) = self.skeleton(sliders);
        match (anchor.vertex, self.head) {
            (Some(vertex), Some(head)) => surface(self.rig, head, &weights, &joints, vertex),
            (Some(_), None) => None,
            (None, _) => anchor.at(&joints),
        }
    }
    /// The joint's matrix in world space for these slider values, on the
    /// same terms as [`Self::point`]: a resize handle's part outline.
    pub fn joint(&self, sliders: &BTreeMap<i32, f32>, joint: usize) -> Option<Mat4> {
        self.skeleton(sliders).1.get(joint).copied()
    }
    /// Where the point is drawn: on the drawn skeleton (`joints`, world
    /// space), with the morphs of these slider values for a head vertex.
    pub fn drawn(&self, sliders: &BTreeMap<i32, f32>, anchor: Anchor, joints: &[Mat4]) -> Option<Vec3> {
        match anchor.vertex {
            Some(_) => self.drawn_with(&self.weights(sliders), anchor, joints),
            None => anchor.at(joints),
        }
    }
    /// Every weight for these slider values: computed once a frame for all
    /// the dots of [`Self::drawn_with`].
    pub fn weights(&self, sliders: &BTreeMap<i32, f32>) -> HashMap<i32, f32> {
        self.params.with_changes(self.base.clone(), sliders)
    }
    /// [`Self::drawn`] with the weights already worked out.
    pub fn drawn_with(&self, weights: &HashMap<i32, f32>, anchor: Anchor, joints: &[Mat4]) -> Option<Vec3> {
        match (anchor.vertex, self.head) {
            (Some(vertex), Some(head)) => surface(self.rig, head, weights, joints, vertex),
            (Some(_), None) => None,
            (None, _) => anchor.at(joints),
        }
    }
    /// The way the drawn surface faces at the point: a vertex's normal
    /// turned by its bone, or the volume's ellipsoid's.
    pub fn drawn_normal(&self, anchor: Anchor, joints: &[Mat4]) -> Option<Vec3> {
        match (anchor.vertex, self.head) {
            (Some(vertex), Some(head)) => {
                let normal = *head.normals.get(vertex as usize)?;
                let rotation = joints.get(anchor.joint)?.to_scale_rotation_translation().1;
                (rotation * normal).try_normalize()
            }
            (Some(_), None) => None,
            (None, _) => anchor.normal(joints),
        }
    }
}

/// Damped least squares: the slider changes that best move the point by
/// `error` (screen points), given how each slider moves it (`columns`,
/// points per unit of weight). `Δw = (JᵀJ + λ² I)⁻¹ Jᵀ e`, solved in slider
/// space (one or two sliders a handle): a slider that barely moves the
/// point on screen (seen along its motion) takes a small change instead of
/// an unbounded one.
pub fn damped_step(columns: &[Vec2], error: Vec2, lambda: f32) -> Vec<f32> {
    let l2 = lambda * lambda;
    match columns {
        [] => Vec::new(),
        [c] => {
            let den = c.length_squared() + l2;
            vec![if den > 1e-12 { c.dot(error) / den } else { 0.0 }]
        }
        [c0, c1] => {
            let (a, b, d) = (c0.length_squared() + l2, c0.dot(*c1), c1.length_squared() + l2);
            let det = a * d - b * b;
            if det.is_finite() && det.abs() > 1e-9 * (a * d).max(1e-12) {
                let (r0, r1) = (c0.dot(error), c1.dot(error));
                vec![(d * r0 - b * r1) / det, (a * r1 - b * r0) / det]
            } else {
                // two sliders moving the point the same way: share the move
                damped_rows(columns, error, lambda)
            }
        }
        _ => damped_rows(columns, error, lambda),
    }
}

/// The same in screen space, `Δw = Jᵀ (J Jᵀ + λ² I)⁻¹ e`: the smallest
/// change when the sliders' motions on screen are alike.
fn damped_rows(columns: &[Vec2], error: Vec2, lambda: f32) -> Vec<f32> {
    let (mut a, mut b, mut d) = (lambda * lambda, 0.0, lambda * lambda);
    for c in columns {
        a += c.x * c.x;
        b += c.x * c.y;
        d += c.y * c.y;
    }
    let det = a * d - b * b;
    if !det.is_finite() || det.abs() < 1e-12 {
        return vec![0.0; columns.len()];
    }
    let y = Vec2::new(d * error.x - b * error.y, a * error.y - b * error.x) / det;
    columns.iter().map(|c| c.dot(y)).collect()
}

/// [`damped_step`] within each slider's range: a slider that reaches its
/// limit stays there and the others take what is left of the move.
/// `ranges`: (current value, min, max) of each slider.
pub fn clamped_step(columns: &[Vec2], error: Vec2, lambda: f32, ranges: &[(f32, f32, f32)]) -> Vec<f32> {
    let mut fixed: Vec<Option<f32>> = vec![None; columns.len()];
    loop {
        let free: Vec<usize> = (0..columns.len()).filter(|i| fixed[*i].is_none()).collect();
        let residual = fixed.iter().zip(columns).fold(error, |e, (delta, c)| e - *c * delta.unwrap_or(0.0));
        let step = damped_step(&free.iter().map(|i| columns[*i]).collect::<Vec<_>>(), residual, lambda);
        let mut clamped = false;
        for (k, &i) in free.iter().enumerate() {
            let (value, min, max) = ranges[i];
            let next = value + step[k];
            if next < min || next > max {
                fixed[i] = Some(next.clamp(min, max) - value);
                clamped = true;
            }
        }
        if !clamped {
            let mut out: Vec<f32> = fixed.iter().map(|d| d.unwrap_or(0.0)).collect();
            for (k, &i) in free.iter().enumerate() {
                out[i] = step[k];
            }
            return out;
        }
    }
}

/// The dot a drag holds: which handle, which of its two dots, and the
/// sliders it moves (those [`shown_params`] keeps).
#[derive(Clone, Debug, PartialEq)]
pub struct Grip {
    pub handle: usize,
    pub mirrored: bool,
    pub anchor: Anchor,
    pub params: Vec<i32>,
    /// A resize handle ([`Handle::resize`]): dragged radially.
    pub resize: bool,
}

impl Grip {
    /// The same dot, wherever it is anchored: the mesh head's surface read
    /// arriving mid-drag moves the anchor, and the drag must keep its dot.
    pub fn same_dot(&self, other: &Grip) -> bool {
        self.handle == other.handle && self.mirrored == other.mirrored && self.params == other.params
    }
}

/// A handle being dragged.
#[derive(Clone, Debug)]
pub struct Drag {
    pub grip: Grip,
    /// Where the dot was on screen when pressed, and where the pointer was.
    dot: Vec2,
    pointer: Vec2,
    /// Drawn point minus the model's at the press: the drawn skeleton also
    /// carries what the model leaves out (animation offsets, mesh joint
    /// positions), which no slider changes.
    offset: Vec3,
    /// A resize: the view it is seen along, the part's centre on screen at
    /// the press, then the way out from it the drag measures and the part's
    /// outline's reach that way at the press. Pressed near the middle of the
    /// part as seen (a hand facing the camera), where no way leads out, the
    /// drag scrubs like a number: up and right grow, down and left shrink.
    view: Vec3,
    centre: Vec2,
    way: Vec2,
    radius: f32,
    /// A dot its sliders hardly move as seen from here (a volume moved
    /// along the view, Knee Angle from the side; a face bone that only
    /// grows under a mesh head): the slider it scrubs like a number, and
    /// its value at the press. Never a drag that does nothing.
    scrub: Option<(i32, f32)>,
    /// The way a scrub grows its slider on screen: the way the slider moves
    /// the dot, however little (the dot on the left of the hips grows to
    /// the left), else up and to the right ([`SCRUB`]).
    scrub_way: Vec2,
    /// The sliders' values at the press, for the log of the drag
    /// ([`Self::describe`]).
    start_values: Vec<(i32, f32)>,
}

/// A resize pressed closer to its part's centre on screen than this share
/// of the part's reach scrubs ([`Drag::way`]).
const PRESSED_INSIDE: f32 = 0.35;

/// The scrubbing way: up and to the right (screen y grows downwards).
const SCRUB: Vec2 = Vec2::new(std::f32::consts::FRAC_1_SQRT_2, -std::f32::consts::FRAC_1_SQRT_2);

/// A dot whose sliders move it less than this across their whole range, on
/// screen (points), scrubs instead ([`Drag::scrub`]).
const SMALL_ON_SCREEN: f32 = 6.0;

/// The least motion on screen (points) across a slider's range whose way a
/// scrub follows: less is noise (a dot seen straight along its motion).
const SCRUB_FOLLOWS: f32 = 0.5;

/// The pointer's travel along a scrub's way that takes a scrubbed slider across
/// its whole range (points), about a slider's width.
const SCRUB_RANGE: f32 = 240.0;

/// Whether a joint is a collision volume (`L_UPPER_ARM`, `BELLY`…, named in
/// capitals), which has a size of its own to outline; a bone (`mFace…`)
/// has none.
pub fn is_volume(rig: &Rig, joint: usize) -> bool {
    rig.canonical_names.get(joint).is_some_and(|name| !name.starts_with('m'))
}

impl Drag {
    /// `drawn`: the dot's point on the drawn avatar (world space); `view`:
    /// the direction the camera looks at it along.
    pub fn start(
        body: &Body,
        sliders: &BTreeMap<i32, f32>,
        grip: Grip,
        drawn: Vec3,
        view: Vec3,
        project: impl Fn(Vec3) -> Option<Vec2>,
        pointer: Vec2,
    ) -> Option<Self> {
        let model = body.point(sliders, grip.anchor)?;
        let dot = project(drawn)?;
        let offset = drawn - model;
        let centre = body
            .joint(sliders, grip.anchor.joint)
            .and_then(|m| project(m.w_axis.truncate() + offset))
            .unwrap_or(dot);
        let mut drag = Self {
            grip,
            dot,
            pointer,
            offset,
            view: view.normalize_or_zero(),
            centre,
            way: SCRUB,
            radius: 0.0,
            scrub: None,
            scrub_way: SCRUB,
            start_values: Vec::new(),
        };
        drag.start_values = drag
            .grip
            .params
            .iter()
            .filter_map(|&id| Some((id, *sliders.get(&id).or_else(|| body.base.get(&id))?)))
            .collect();
        // a resize of a bone has no outline to follow (a face bone under a
        // mesh head): a scrub
        if drag.grip.resize && !is_volume(body.rig, drag.grip.anchor.joint) {
            drag.scrub = drag.scrubbed(body, sliders, drag.grip.params.first().copied());
            return Some(drag);
        }
        if drag.grip.resize {
            // out from the part's centre towards the press, when the press
            // is clearly off the centre
            let out = pointer - centre;
            if let Some(way) = out.try_normalize()
                && out.length() > drag.reach(body, sliders, &project, way)? * PRESSED_INSIDE
            {
                drag.way = way;
            }
            drag.radius = drag.reach(body, sliders, &project, drag.way)?;
        }
        // how far, and which way, each slider moves what the drag follows
        // across its range
        let motion = |id: i32| -> Option<Vec2> {
            let param = catalog::PARAMS.iter().find(|p| p.id == id)?;
            let at = |value: f32| {
                let mut values = sliders.clone();
                values.insert(id, value);
                if drag.grip.resize {
                    drag.reach(body, &values, &project, drag.way).map(|r| Vec2::new(r, 0.0))
                } else {
                    body.point(&values, drag.grip.anchor).and_then(|p| project(p + drag.offset))
                }
            };
            Some(at(param.max)? - at(param.min)?)
        };
        let widest = drag
            .grip
            .params
            .iter()
            .filter_map(|&id| Some((id, motion(id)?)))
            .max_by(|a, b| a.1.length().total_cmp(&b.1.length()));
        if widest.is_none_or(|(_, motion)| motion.length() < SMALL_ON_SCREEN) {
            let id = widest.map(|(id, _)| id).or(drag.grip.params.first().copied());
            drag.scrub = drag.scrubbed(body, sliders, id);
            let moved = widest.map(|(_, motion)| motion).filter(|m| m.length() > SCRUB_FOLLOWS);
            // seen straight along its motion (Love Handles from the front):
            // pulled away from its part's centre on screen, it grows the
            // slider when the slider takes the point away from that centre
            let outward = || -> Option<Vec2> {
                let param = catalog::PARAMS.iter().find(|p| Some(p.id) == id)?;
                let point = |value: Option<f32>| {
                    let mut values = sliders.clone();
                    if let Some(value) = value {
                        values.insert(param.id, value);
                    }
                    body.point(&values, drag.grip.anchor)
                };
                let along = point(Some(param.max))? - point(Some(param.min))?;
                let centre = body.joint(sliders, drag.grip.anchor.joint)?.w_axis.truncate();
                let away = along.dot(point(None)? - centre);
                let out = drag.dot - drag.centre;
                (along.length() > 1e-5 && away.abs() > 1e-9 && out.length() > SMALL_ON_SCREEN * 0.5)
                    .then(|| out.normalize() * away.signum())
            };
            drag.scrub_way = match moved {
                // a resize grows out the way it was pressed (or in)
                Some(m) if drag.grip.resize && drag.way != SCRUB => drag.way * m.x.signum(),
                Some(m) if !drag.grip.resize => m.normalize(),
                None if !drag.grip.resize => outward().unwrap_or(SCRUB),
                _ => SCRUB,
            };
        }
        Some(drag)
    }

    /// What the drag did, for the log once it ends: each slider from its
    /// value at the press to `sliders`', and how far the pointer and the
    /// dot (`dot`, drawn now) went on screen.
    pub fn describe(&self, sliders: &BTreeMap<i32, f32>, pointer: Vec2, dot: Vec2) -> String {
        let values: Vec<String> = self
            .start_values
            .iter()
            .map(|(id, from)| format!("{id} {from:.3} -> {:.3}", sliders.get(id).copied().unwrap_or(*from)))
            .collect();
        format!(
            "shape handle drag: {} ({}{}{}), pointer moved {:.0}, dot moved {:.0}",
            values.join(", "),
            if self.grip.mirrored { "mirrored dot" } else { "dot" },
            if self.grip.resize { ", resize" } else { "" },
            if self.scrub.is_some() {
                format!(", scrub way {:.2}", self.scrub_way)
            } else {
                String::new()
            },
            pointer - self.pointer,
            dot - self.dot,
        )
    }

    /// Whether the drag scrubs a slider ([`Self::scrub`]).
    #[cfg(test)]
    pub fn scrubs(&self) -> bool {
        self.scrub.is_some()
    }

    /// The scrub of slider `id` from its value at the press (the body's own
    /// weight when the shape does not list it).
    fn scrubbed(&self, body: &Body, sliders: &BTreeMap<i32, f32>, id: Option<i32>) -> Option<(i32, f32)> {
        let id = id?;
        let value = sliders.get(&id).or_else(|| body.base.get(&id)).copied()?;
        Some((id, value))
    }

    /// How far the part's outline reaches from its centre at the press,
    /// along `way` (screen points): what a resize drag drives.
    fn reach(&self, body: &Body, values: &BTreeMap<i32, f32>, project: &impl Fn(Vec3) -> Option<Vec2>, way: Vec2) -> Option<f32> {
        let volume = body.joint(values, self.grip.anchor.joint)?;
        outline(volume, self.view, 24)
            .into_iter()
            .filter_map(|p| project(p + self.offset))
            .map(|q| (q - self.centre).dot(way))
            .reduce(f32::max)
    }

    /// Move the sliders so that the point follows the pointer (a few
    /// Gauss-Newton steps from the current values, each linearised by
    /// nudging every slider a little). A resize follows the part's outline
    /// instead, which shows from any side: it reaches as much farther from
    /// the centre as the pointer has gone, so pulled away from the part, on
    /// any side and from any angle, it grows. Returns whether a value changed.
    pub fn step(
        &self,
        body: &Body,
        sliders: &mut BTreeMap<i32, f32>,
        ranges: impl Fn(i32) -> Option<(f32, f32)>,
        project: impl Fn(Vec3) -> Option<Vec2>,
        pointer: Vec2,
    ) -> bool {
        if let Some((id, start)) = self.scrub {
            let Some((min, max)) = ranges(id) else {
                return false;
            };
            let value = (start + (pointer - self.pointer).dot(self.scrub_way) * (max - min) / SCRUB_RANGE).clamp(min, max);
            let changed = sliders.get(&id).is_none_or(|old| (old - value).abs() > 1e-6);
            sliders.insert(id, value);
            return changed;
        }
        let anchor = self.grip.anchor;
        let resize = self.grip.resize.then_some((self.way, self.radius));
        // what the drag drives (screen points), and where it should be
        let measure = |values: &BTreeMap<i32, f32>| match resize {
            Some((way, _)) => self.reach(body, values, &project, way).map(|r| Vec2::new(r, 0.0)),
            None => body.point(values, anchor).and_then(|p| project(p + self.offset)),
        };
        let target = match resize {
            // pressed off the centre: how far the pointer is from the part's
            // centre, in any direction (the camera at any angle), counted
            // below zero past the centre, so that pushed in past the centre
            // of a small part (a hand) it keeps shrinking instead of growing
            Some((way, radius)) if self.way != SCRUB => {
                let out = pointer - self.centre;
                let signed = out.length() * if out.dot(way) < 0.0 { -1.0 } else { 1.0 };
                Vec2::new((radius + signed - (self.pointer - self.centre).length()).max(0.0), 0.0)
            }
            // pressed on the centre: scrubbing
            Some((way, radius)) => Vec2::new((radius + (pointer - self.pointer).dot(way)).max(0.0), 0.0),
            None => self.dot + (pointer - self.pointer),
        };
        let screen = measure;
        let mut changed = false;
        for _ in 0..3 {
            let Some(at) = screen(sliders) else {
                break;
            };
            let error = target - at;
            if error.length() < 0.25 {
                break;
            }
            let mut columns = Vec::new();
            let mut limits = Vec::new();
            let mut ids = Vec::new();
            for &id in &self.grip.params {
                let Some((min, max)) = ranges(id) else { continue };
                // a slider the shape does not list (an older shape asset) is
                // drawn at the body's own weight: start from it, never from
                // the minimum, which would jump the body there
                let value = sliders
                    .get(&id)
                    .or_else(|| body.base.get(&id))
                    .copied()
                    .unwrap_or((min + max) * 0.5)
                    .clamp(min, max);
                // nudged both ways where the range allows: many sliders bend
                // in the middle (Leg Muscles drives one morph below its
                // middle, another above), and a one-sided slope taken at the
                // bend overshoots it back and forth, a jitter
                let eps = (max - min) * 0.01;
                let (below, above) = ((value - eps).max(min), (value + eps).min(max));
                if above - below < 1e-6 {
                    continue;
                }
                let at_value = |v: f32| {
                    let mut nudged = sliders.clone();
                    nudged.insert(id, v);
                    screen(&nudged)
                };
                let (Some(low), Some(high)) = (at_value(below), at_value(above)) else {
                    continue;
                };
                columns.push((high - low) / (above - below));
                limits.push((value, min, max));
                ids.push(id);
            }
            let strongest = columns.iter().map(|c| c.length()).fold(0.0, f32::max);
            if strongest < 1e-3 {
                break;
            }
            let delta = clamped_step(&columns, error, strongest * 0.05, &limits);
            // a step is kept only when it brings the point closer, halved
            // until it does: never an overshoot past the pointer and back
            let mut scale = 1.0;
            let mut kept = None;
            for _ in 0..5 {
                let mut trial = sliders.clone();
                let mut moved = false;
                for ((id, d), (value, min, max)) in ids.iter().zip(&delta).zip(&limits) {
                    // a linearised step stays within a quarter of the range
                    let quarter = (max - min) * 0.25;
                    let next = (value + (d * scale).clamp(-quarter, quarter)).clamp(*min, *max);
                    if (next - value).abs() > 1e-6 {
                        trial.insert(*id, next);
                        moved = true;
                    }
                }
                if !moved {
                    break;
                }
                if screen(&trial).is_some_and(|now| (target - now).length() < error.length()) {
                    kept = Some(trial);
                    break;
                }
                scale *= 0.5;
            }
            let Some(trial) = kept else {
                break;
            };
            *sliders = trial;
            changed = true;
        }
        changed
    }
}

/// The path one slider moves the point along, from its minimum to its
/// maximum (`samples` points, the other sliders held), as the model gives
/// it: the track drawn under a hovered or held dot. A slider that scales or
/// bends shows its true course, not a straight guess.
pub fn track(body: &Body, sliders: &BTreeMap<i32, f32>, anchor: Anchor, id: i32, (min, max): (f32, f32), samples: usize) -> Vec<Vec3> {
    let samples = samples.max(2);
    (0..samples)
        .filter_map(|i| {
            let mut values = sliders.clone();
            values.insert(id, min + (max - min) * i as f32 / (samples - 1) as f32);
            body.point(&values, anchor)
        })
        .collect()
}

#[cfg(test)]
#[path = "handles_tests.rs"]
mod tests;
