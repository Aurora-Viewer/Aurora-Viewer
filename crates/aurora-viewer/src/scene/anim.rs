//! Skeletal animation: rig built from `avatar_skeleton.xml` (base + Bento
//! bones), animation asset streaming and per-avatar pose evaluation into
//! joint palettes (joint world matrices; meshes apply their inverse binds).

pub(super) mod grounding;
mod legacy_grounding;
mod pointing;

use super::default_motions;
use super::fetch_priority::urgent;
use super::jobs::Jobs;
use crate::world::PlayingAnimation;
use aurora_assets::{Animation, Skeleton};
use aurora_net::{FetchRequest, FetchResult, Fetcher, Lane};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FETCH_KIND_ANIM: u64 = 4 << 60;

/// Palette entries reserved per skeleton instance: its joints, the anchors
/// of the objects it wears (anchors.rs), then the weights of its live
/// morphs (live_morphs.rs).
pub const PALETTE_JOINTS: usize = super::live_morphs::WEIGHTS + super::live_morphs::WEIGHT_MATRICES;

/// First anchor matrix of a palette; the joints are below it.
pub const ANCHOR_BASE: usize = aurora_render::SKELETON_ANCHOR as usize;

/// Anchor matrices per palette: the wearer's root, then one per attachment
/// point of the body (47 in avatar_lad.xml).
pub const ANCHORS: usize = super::live_morphs::WEIGHTS - ANCHOR_BASE;

pub struct Rig {
    pub parent: Vec<Option<usize>>,
    pub local_pos: Vec<Vec3>,
    pub local_rot: Vec<Quat>,
    pub parent_scale: Vec<Vec3>,
    /// Joint's own scale (collision volumes): part of its world matrix,
    /// never inherited (LLXformMatrix).
    pub scale: Vec<Vec3>,
    pub default_world_inv: Vec<Mat4>,
    /// Depth-first order (parents before children).
    pub order: Vec<usize>,
    pub names: HashMap<String, usize>,
    /// One original skeleton name per index; `names` also accepts aliases.
    pub canonical_names: Vec<String>,
    pub pelvis: usize,
    /// Upright root-to-skull chain, including the Bento spine joints.
    pub(super) tag_chain: Vec<(usize, f32)>,
    /// Body attachment points by name ("Nose", "Left Ear"…): the joint
    /// they hang from and their fixed transform below it (avatar_lad.xml).
    /// Filled by the avatar library, which reads the points.
    pub(crate) skin_points: HashMap<String, (usize, Mat4)>,
}

/// Joints in depth-first order, parents before children.
fn depth_first(parent: &[Option<usize>]) -> Vec<usize> {
    let n = parent.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, p) in parent.iter().enumerate() {
        match *p {
            Some(p) if p < n => children[p].push(i),
            _ => roots.push(i),
        }
    }
    let mut order = Vec::with_capacity(n);
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    while let Some(j) = stack.pop() {
        order.push(j);
        for c in children[j].iter().rev() {
            stack.push(*c);
        }
    }
    order
}

impl Rig {
    pub fn new(sk: &Skeleton) -> Rig {
        let n = sk.joints.len().min(ANCHOR_BASE);
        let mut names = HashMap::new();
        let mut parent = Vec::with_capacity(n);
        let mut local_pos = Vec::with_capacity(n);
        let mut local_rot = Vec::with_capacity(n);
        let mut parent_scale = Vec::with_capacity(n);
        let mut scale = Vec::with_capacity(n);
        for (i, j) in sk.joints.iter().take(n).enumerate() {
            names.insert(j.name.clone(), i);
            for a in &j.aliases {
                names.entry(a.clone()).or_insert(i);
            }
            parent.push(j.parent.filter(|p| *p < n));
            local_pos.push(j.pos);
            local_rot.push(aurora_assets::skeleton::maya_q_xyz(j.rot_euler_deg));
            parent_scale.push(j.parent.and_then(|p| sk.joints.get(p)).map(|p| p.scale).unwrap_or(Vec3::ONE));
            scale.push(j.scale);
        }
        let order = depth_first(&parent);
        let pelvis = names.get("mPelvis").copied().unwrap_or(0);
        let skull = names.get("mSkull").copied();
        let mut joint = skull.or_else(|| names.get("mHead").copied());
        let mut tag_chain = Vec::new();
        while let Some(j) = joint {
            // computeBodySize's skull extent, with the complete Bento chain.
            tag_chain.push((j, if Some(j) == skull { std::f32::consts::SQRT_2 } else { 1.0 }));
            joint = parent[j];
        }
        let mut rig = Rig {
            parent,
            local_pos,
            local_rot,
            parent_scale,
            scale,
            default_world_inv: vec![Mat4::IDENTITY; n],
            order,
            names,
            canonical_names: sk.joints.iter().take(n).map(|joint| joint.name.clone()).collect(),
            pelvis,
            tag_chain,
            skin_points: HashMap::new(),
        };
        let rot = rig.local_rot.clone();
        let world = rig.world(&rot, Vec3::ZERO);
        rig.default_world_inv = world.iter().map(|m| m.inverse()).collect();
        rig
    }

    pub fn len(&self) -> usize {
        self.parent.len()
    }

    /// LLVOAvatar::initAttachmentPoints (indra/newview/llvoavatar.cpp,
    /// originally LGPL 2.1): every body attachment point is an
    /// LLViewerJointAttachment, a joint below its bone that
    /// LLVOAvatar::getJoint finds by name, so meshes are skinned to it and
    /// animations key it ("attachment can potentially be animated, needs a
    /// number"); worn animesh rely on it (#1492). `points`: name, bone, the
    /// point's position and rotation under it (avatar_lad.xml); unit scale.
    /// A point on mRoot ("Avatar Center"), which this skeleton has not, is a
    /// root of its own where mRoot rests (the pelvis's rest position: the
    /// pelvis is zero under mRoot, LLAvatarAppearance::buildCharacter), so
    /// it stays put while motions move the pelvis. Added while the palette
    /// has room below its anchors for them and the identity slot; a name
    /// the skeleton already has keeps its joint.
    pub(crate) fn add_attachment_points(&mut self, points: &[(String, Option<usize>, Vec3, Quat)]) {
        for (name, parent, pos, rot) in points {
            if self.len() + 1 >= ANCHOR_BASE {
                break;
            }
            if self.names.contains_key(name) || parent.is_some_and(|p| p >= self.len()) {
                continue;
            }
            let j = self.len();
            self.parent.push(*parent);
            match parent {
                Some(_) => self.local_pos.push(*pos),
                None => self.local_pos.push(self.local_pos[self.pelvis] + *pos),
            }
            self.local_rot.push(*rot);
            self.parent_scale.push(parent.map_or(Vec3::ONE, |p| self.scale[p]));
            self.scale.push(Vec3::ONE);
            self.names.insert(name.clone(), j);
            self.canonical_names.push(name.clone());
        }
        self.order = depth_first(&self.parent);
        let rot = self.local_rot.clone();
        let world = self.world(&rot, Vec3::ZERO);
        self.default_world_inv = world.iter().map(|m| m.inverse()).collect();
    }

    /// The rig joint a mesh skin joint follows, and the inverse bind to
    /// pose it with (LLSkinningUtil::initJointNums,
    /// indra/newview/llskinningutil.cpp). LLVOAvatar::getJoint finds any
    /// joint of the avatar's tree, attachment points included: some meshes
    /// are rigged to "Nose" or "Left Ear". Such a point is not in the
    /// palette, so it is its joint, with the point's fixed transform folded
    /// into the inverse bind (the joint's own scale then also applies to the
    /// point, a shape-dependent few millimetres where the Legacy Viewer has
    /// unit scale). Any other unknown name resolves to the pelvis, keeping
    /// its inverse bind, rather than an unposed identity matrix.
    pub(crate) fn skin_bind(&self, name: &str, inverse_bind: Mat4) -> (usize, Mat4) {
        if let Some(&joint) = self.names.get(name) {
            return (joint, inverse_bind);
        }
        match self.skin_points.get(name) {
            Some(&(joint, local)) => (joint, local * inverse_bind),
            None => (self.pelvis, inverse_bind),
        }
    }

    fn name_tag_height(&self, pos: &[Vec3], scale: &[Vec3], delta: &[f32], pelvis_offset: f32) -> Option<f32> {
        if self.tag_chain.is_empty() {
            return None;
        }
        let mut height = pelvis_offset - self.local_pos[self.pelvis].z;
        for &(j, extent) in &self.tag_chain {
            let parent_scale = self.parent[j].and_then(|p| scale.get(p)).map_or(self.parent_scale[j].z, |s| s.z);
            height += (pos.get(j).unwrap_or(&self.local_pos[j]).z + delta[j]) * parent_scale * extent;
        }
        (height.is_finite() && height > 0.0).then_some(height)
    }

    /// World transforms (rotation + translation) for given local rotations.
    fn world(&self, rot: &[Quat], pelvis_offset: Vec3) -> Vec<Mat4> {
        self.world_with(rot, pelvis_offset, &self.local_pos, &self.scale)
    }

    /// Same with per-avatar joint positions and scales (shape, attachment
    /// overrides). A joint's position is scaled by its parent's scale; its
    /// own scale is part of its matrix only (LLXformMatrix).
    pub(crate) fn world_with(&self, rot: &[Quat], pelvis_offset: Vec3, local_pos: &[Vec3], scale: &[Vec3]) -> Vec<Mat4> {
        let n = self.len();
        let mut wpos = vec![Vec3::ZERO; n];
        let mut wrot = vec![Quat::IDENTITY; n];
        for &j in &self.order {
            let parent_scale = match self.parent[j] {
                Some(p) => scale.get(p).copied().unwrap_or(self.parent_scale[j]),
                None => self.parent_scale[j],
            };
            let mut lp = local_pos.get(j).copied().unwrap_or(self.local_pos[j]) * parent_scale;
            if j == self.pelvis {
                lp += pelvis_offset;
            }
            match self.parent[j] {
                Some(p) => {
                    wpos[j] = wpos[p] + wrot[p] * lp;
                    wrot[j] = (wrot[p] * rot[j]).normalize();
                }
                None => {
                    wpos[j] = lp;
                    wrot[j] = rot[j];
                }
            }
        }
        (0..n)
            .map(|j| Mat4::from_scale_rotation_translation(scale.get(j).copied().unwrap_or(self.scale[j]), wrot[j], wpos[j]))
            .collect()
    }
}

/// Rest skeleton of one avatar: joint positions and scales after its shape
/// and the joint offsets of the meshes it wears.
#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonBase {
    pub local_pos: Vec<Vec3>,
    pub scale: Vec<Vec3>,
}

/// An animation with joint names resolved against the rig.
pub struct BoundAnim {
    pub anim: Animation,
    pub joints: Vec<Option<usize>>,
    /// Upper translation over the entire track, computed once when loading.
    /// Name tags use this envelope, never the current animation sample.
    position_top: Vec<Option<f32>>,
    /// The highest priority of its joints and itself (mMaxPriority): the
    /// strongest animation gives the hand pose.
    max_priority: i32,
    /// LLMotion::addJointState's mJointSignature: per joint driven, its
    /// position and rotation bits, `0xff >> (7 - priority)` when keyed.
    signature: Vec<(usize, [u8; 2])>,
}

impl BoundAnim {
    pub fn bind(anim: Animation, rig: &Rig) -> BoundAnim {
        let joints: Vec<Option<usize>> = anim.joints.iter().map(|j| rig.names.get(&j.joint_name).copied()).collect();
        let max_priority = anim.joints.iter().map(|j| j.priority).fold(anim.base_priority, i32::max);
        let signature = anim
            .joints
            .iter()
            .zip(&joints)
            .filter_map(|(jm, j)| {
                let priority = if jm.priority >= 0 { jm.priority } else { anim.base_priority };
                let bits = if priority < 0 { 0 } else { 0xffu8 >> (7 - priority.min(7)) };
                let keyed = |keys: bool| if keys { bits } else { 0 };
                j.map(|j| (j, [keyed(!jm.pos_keys.is_empty()), keyed(!jm.rot_keys.is_empty())]))
            })
            .collect();
        let position_top = anim
            .joints
            .iter()
            .map(|j| {
                j.pos_keys
                    .iter()
                    .filter(|k| k.position.is_finite())
                    .map(|k| k.position.z)
                    .reduce(f32::max)
            })
            .collect();
        BoundAnim {
            anim,
            joints,
            position_top,
            max_priority,
            signature,
        }
    }
}

// ------------------------------------------------------------- motion control
//
// Port of LLMotionController / LLPoseBlender (indra/llcharacter, Linden
// Research, originally LGPL 2.1): motions ease in and out (cubic), a motion still
// loading starts on arrival (or catches up to its first known signal when opted
// in), a stopped motion keeps playing while it eases out, and a joint no motion
// drives keeps its last pose (never back to
// the T-pose: LLJointStateBlender::blendJointStates).

/// Most joint states blended per joint (JSB_NUM_JOINT_STATES).
const JOINT_STATES: usize = 6;

fn cubic_step(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// LL nlerp: normalized linear blend on the shortest arc.
fn nlerp(t: f32, a: Quat, b: Quat) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    (a * (1.0 - t) + b * t).normalize()
}

struct Motion {
    id: Uuid,
    anim: Arc<BoundAnim>,
    activation: Instant,
    /// Loop phase and ease-in use this clock, independent of new sequences.
    continuous_activation: Instant,
    signal: PlayingAnimation,
    stop: Option<Instant>,
    /// Weight when the ease out began (mResidualWeight).
    residual: f32,
    weight: f32,
    /// Started by another motion (the expression an animation names), not
    /// by the simulator: it ends on its own.
    internal: bool,
    recovery: Option<legacy_grounding::Recovery>,
}

impl Motion {
    fn ease_in_weight(&self, now: Instant) -> f32 {
        let a = &self.anim.anim;
        let start = if a.looping { self.continuous_activation } else { self.activation };
        let elapsed = now.saturating_duration_since(start).as_secs_f64();
        let ease_in = legacy_grounding::ease_in(self.recovery, a.ease_in);
        if ease_in > 0.0 {
            aurora_assets::anim::cubic_step(elapsed / f64::from(ease_in))
        } else {
            1.0
        }
    }

    fn stop_at(&mut self, now: Instant) {
        if self.stop.is_none() {
            self.stop = Some(now);
            self.residual = self.ease_in_weight(self.fade_start(now));
        }
    }

    /// LLKeyframeMotion::setStopTime: preserve an authored post-loop tail,
    /// including a single held pose (loop_in == loop_out == 0).
    fn fade_start(&self, stop: Instant) -> Instant {
        let a = &self.anim.anim;
        if !a.looping || a.loop_out == a.duration {
            return stop;
        }
        let elapsed = stop.saturating_duration_since(self.continuous_activation).as_secs_f64();
        let span = f64::from(a.loop_out - a.loop_in);
        let fraction = if span > 0.0 { (elapsed - f64::from(a.loop_in)) % span } else { 0.0 };
        let delay = (f64::from(a.duration - a.loop_in - a.ease_out) - fraction).max(0.0);
        stop.checked_add(Duration::from_secs_f64(delay)).unwrap_or(stop)
    }

    /// LLKeyframeMotion::onUpdate: a stopped loop advances from its current
    /// phase into the outro, without wrapping again.
    fn local_time(&self, now: Instant) -> f64 {
        let a = &self.anim.anim;
        let start = if a.looping { self.continuous_activation } else { self.activation };
        if let Some(stop) = self.stop.filter(|stop| a.looping && now >= *stop) {
            let phase = a.local_time(stop.saturating_duration_since(start).as_secs_f64());
            (phase + now.duration_since(stop).as_secs_f64()).min(f64::from(a.duration))
        } else {
            a.local_time(now.saturating_duration_since(start).as_secs_f64())
        }
    }
}

/// Motions and held pose of one avatar (or animesh).
#[derive(Default)]
pub struct Controller {
    pub(super) poser: Option<(Arc<crate::ik_edit::poser::SubjectPose>, Mat4)>,
    pub(super) ik: super::ik::Pose,
    /// LLControlAvatar disables default head / eye motions.
    pub control: bool,
    pub(super) ground: Option<grounding::Frame>,
    pub(super) legacy_ground: Option<grounding::Frame>,
    /// Ownership is the saved option, never the availability of a support hit.
    pub(super) aurora_feet: bool,
    placement: grounding::Placement,
    legacy_rot: Vec<Quat>,
    legacy_held: bool,
    ground_handoff: legacy_grounding::Handoff,
    ground_origin: Option<Vec3>,
    /// Opt-in receipt clock; the default keeps Legacy Viewer activation / pauses.
    synchronized: bool,
    /// Newest first (mActiveMotions, push_front).
    motions: Vec<Motion>,
    /// Signals already activated, including completed non-looping motions.
    /// A still-signaled one-shot must not restart automatically every frame.
    signals: HashMap<Uuid, PlayingAnimation>,
    preview_start: Option<(Option<Uuid>, Instant)>,
    /// Joint rotations of the last frame (None before the first one).
    rot: Vec<Quat>,
    /// Animated joint positions as offsets from the rest skeleton, held
    /// like the rotations (a shape change still moves them).
    pos_delta: Vec<Vec3>,
    pelvis_offset: Option<Vec3>,
    /// The same held translations, using whole-track upper bounds for a
    /// stable upright height. Rotations and sampled bobbing never enter it.
    tag_pos_delta: Vec<f32>,
    tag_pelvis_offset: f32,
    pub name_tag_height: Option<f32>,
    /// Look-at point (LookAtPoint): from the head to the target, in the
    /// avatar's root frame; None = look forward.
    pub look: Option<Vec3>,
    /// PointAtPoint in skeleton space, including the feet-origin offset.
    pub point: Option<Vec3>,
    pointing: pointing::Pointing,
    /// The look-at target was cleared: HEAD_ROT stops (eyes keep `look`).
    pub look_cleared: bool,
    procedural: Procedural,
    /// Joints used by the procedural motions, as posed last frame
    /// (skeleton space).
    posed: LookJoints<(Quat, Vec3)>,
    last_eval: Option<Instant>,
    /// Pose frozen since then (LLCharacter::requestPause).
    paused_at: Option<Instant>,
    /// Default motion joint states of the last frame, held during a pause
    /// (LLMotionController::updateIdleActiveMotions does not run them).
    held_procedural: Vec<ProceduralState>,
    /// Sitting on an object: no body noise (LLVOAvatar::sitOnObject).
    pub sitting: bool,
    /// An aiming animation is signalled: no body noise either.
    aiming: bool,
    hands: default_motions::HandMotion,
    /// The live morph weights of the last frame (live_morphs.rs):
    /// expressions and hand poses.
    live_morphs: Vec<(i32, f32)>,
    /// The joint signature of the motions already updated this frame
    /// (LLMotionController::mJointSignature), position and rotation bits.
    joint_signature: Vec<[u8; 2]>,
}

/// A joint state of a default motion: (joint, priority, weight, local
/// rotation, additive).
type ProceduralState = (usize, i32, f32, Quat, bool);

/// Joints of LLHeadRotMotion and LLEyeMotion.
#[derive(Default, Clone, Copy)]
struct LookJoints<T> {
    pelvis: Option<T>,
    torso: Option<T>,
    neck: Option<T>,
    chest: Option<T>,
    head: Option<T>,
    eye_l: Option<T>,
    eye_r: Option<T>,
    alt_l: Option<T>,
    alt_r: Option<T>,
}

impl LookJoints<usize> {
    fn of(rig: &Rig) -> Self {
        let j = |n: &str| rig.names.get(n).copied();
        let neck = j("mNeck");
        LookJoints {
            pelvis: Some(rig.pelvis),
            torso: j("mTorso"),
            neck,
            chest: neck.and_then(|n| rig.parent[n]),
            head: j("mHead"),
            eye_l: j("mEyeLeft"),
            eye_r: j("mEyeRight"),
            alt_l: j("mFaceEyeAltLeft"),
            alt_r: j("mFaceEyeAltRight"),
        }
    }
}

/// Ease in / out of a procedural motion (LLMotionController weights).
#[derive(Default)]
struct Fade {
    on: bool,
    change: Option<Instant>,
    residual: f32,
    weight: f32,
}

impl Fade {
    fn update(&mut self, on: bool, now: Instant, ease: f32) -> f32 {
        if on != self.on || self.change.is_none() {
            self.on = on;
            self.change = Some(now);
            self.residual = self.weight;
        }
        let t = if ease > 0.0 {
            self.change.map(|c| now.duration_since(c).as_secs_f32()).unwrap_or(0.0) / ease
        } else {
            1.0
        };
        self.weight = if on {
            self.residual + (1.0 - self.residual) * cubic_step(t)
        } else {
            self.residual * cubic_step(1.0 - t)
        };
        self.weight
    }
}

/// State of the head and eye motions (LLHeadRotMotion, LLEyeMotion).
struct Procedural {
    head: Fade,
    eyes: Fade,
    torso: Quat,
    last_head: Quat,
    jitter_at: Option<Instant>,
    jitter_time: f32,
    jitter_yaw: f32,
    jitter_pitch: f32,
    look_away_time: f32,
    look_away_yaw: f32,
    look_away_pitch: f32,
    rng: u32,
    /// Start of LLBreatheMotionRot (the avatar's first frame).
    breathe_start: Option<Instant>,
    /// Start of LLBodyNoiseMotion, None while it is stopped.
    noise_start: Option<Instant>,
}

impl Default for Procedural {
    fn default() -> Self {
        let b = Uuid::new_v4();
        let seed = u32::from_le_bytes([b.as_bytes()[0], b.as_bytes()[1], b.as_bytes()[2], b.as_bytes()[3]]);
        Procedural {
            head: Fade::default(),
            eyes: Fade::default(),
            torso: Quat::IDENTITY,
            last_head: Quat::IDENTITY,
            jitter_at: None,
            jitter_time: 0.0,
            jitter_yaw: 0.0,
            jitter_pitch: 0.0,
            look_away_time: 0.0,
            look_away_yaw: 0.0,
            look_away_pitch: 0.0,
            rng: seed | 1,
            breathe_start: None,
            noise_start: None,
        }
    }
}

impl Procedural {
    /// ll_frand(x): uniform in [0, x).
    fn frand(&mut self, x: f32) -> f32 {
        // xorshift32
        let mut r = self.rng;
        r ^= r << 13;
        r ^= r >> 17;
        r ^= r << 5;
        self.rng = r;
        (r >> 8) as f32 / (1u32 << 24) as f32 * x
    }

    /// Eye jitter and looking away (LLEyeMotion::onUpdate).
    fn jitter(&mut self, now: Instant) {
        let elapsed = self.jitter_at.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(f32::MAX);
        if elapsed > self.jitter_time {
            self.jitter_time = EYE_JITTER_MIN_TIME + self.frand(EYE_JITTER_MAX_TIME - EYE_JITTER_MIN_TIME);
            self.jitter_yaw = (self.frand(2.0) - 1.0) * EYE_JITTER_MAX_YAW;
            self.jitter_pitch = (self.frand(2.0) - 1.0) * EYE_JITTER_MAX_PITCH;
            if elapsed < f32::MAX {
                self.look_away_time -= elapsed.max(0.0);
            }
            self.jitter_at = Some(now);
        } else if elapsed > self.look_away_time {
            if self.look_away_yaw == 0.0 && self.look_away_pitch == 0.0 {
                self.look_away_yaw = (self.frand(2.0) - 1.0) * EYE_LOOK_AWAY_MAX_YAW;
                self.look_away_pitch = (self.frand(2.0) - 1.0) * EYE_LOOK_AWAY_MAX_PITCH;
                self.look_away_time = EYE_LOOK_BACK_MIN_TIME + self.frand(EYE_LOOK_BACK_MAX_TIME - EYE_LOOK_BACK_MIN_TIME);
            } else {
                self.look_away_yaw = 0.0;
                self.look_away_pitch = 0.0;
                self.look_away_time = EYE_LOOK_AWAY_MIN_TIME + self.frand(EYE_LOOK_AWAY_MAX_TIME - EYE_LOOK_AWAY_MIN_TIME);
            }
            self.jitter_at = Some(now);
        }
    }
}

// LLHeadRotMotion / LLEyeMotion constants (llheadrotmotion.cpp)
const TORSO_LAG: f32 = 0.35;
const NECK_LAG: f32 = 0.5;
const HEAD_LOOKAT_LAG_HALF_LIFE: f32 = 0.15;
const TORSO_LOOKAT_LAG_HALF_LIFE: f32 = 0.27;
const HEAD_ROTATION_CONSTRAINT: f32 = 0.8 * std::f32::consts::FRAC_PI_2;
const MIN_HEAD_LOOKAT_DISTANCE: f32 = 0.3;
const EYE_JITTER_MIN_TIME: f32 = 0.3;
const EYE_JITTER_MAX_TIME: f32 = 2.5;
const EYE_JITTER_MAX_YAW: f32 = 0.08;
const EYE_JITTER_MAX_PITCH: f32 = 0.015;
const EYE_LOOK_AWAY_MIN_TIME: f32 = 5.0;
const EYE_LOOK_AWAY_MAX_TIME: f32 = 15.0;
const EYE_LOOK_BACK_MIN_TIME: f32 = 1.0;
const EYE_LOOK_BACK_MAX_TIME: f32 = 5.0;
const EYE_LOOK_AWAY_MAX_YAW: f32 = 0.15;
const EYE_LOOK_AWAY_MAX_PITCH: f32 = 0.12;
const EYE_ROT_LIMIT_ANGLE: f32 = 0.3 * std::f32::consts::FRAC_PI_2;
/// HEAD_ROT and EYE ease in / out (s), both MEDIUM_PRIORITY.
const HEAD_ROT_EASE: f32 = 1.0;
const EYE_EASE: f32 = 0.5;
const MEDIUM_PRIORITY: i32 = 1;
/// LLBodyNoiseMotion's priority.
const HIGH_PRIORITY: i32 = 2;

/// LLSmoothInterpolation::getInterpolant.
fn interpolant(dt: f32, half_life: f32) -> f32 {
    (1.0 - 2f32.powf(-dt / half_life)).clamp(0.0, 1.0)
}

/// LLQuaternion::constrain: limit a rotation to `angle`.
fn constrain(q: Quat, angle: f32) -> Quat {
    let q = if q.w < 0.0 { -q } else { q };
    let c = (angle * 0.5).cos();
    if q.w >= c {
        return q;
    }
    let v = Vec3::new(q.x, q.y, q.z).normalize_or_zero() * (angle * 0.5).sin();
    Quat::from_xyzw(v.x, v.y, v.z, c)
}

/// Between the eyes of mHead in the default skeleton (mEyeLeft / mEyeRight
/// at x 0.098, z 0.079, avatar_skeleton.xml).
const EYES_FROM_HEAD: Vec3 = Vec3::new(0.098, 0.0, 0.079);

/// The point between the eyes: the system eyes (mEyeLeft / mEyeRight), or
/// the Bento ones (mFaceEyeAlt*) when a mesh head has moved them away from
/// the system eyes (both pairs stand at the same place in the default
/// skeleton, and LLHeadRotMotion turns both); without eyes, mHead with
/// the default offset.
fn eyes_between(eyes: Option<(Vec3, Vec3)>, alt: Option<(Vec3, Vec3)>, head: Option<(Quat, Vec3)>) -> Option<Vec3> {
    let mid = |(l, r): (Vec3, Vec3)| (l + r) * 0.5;
    match (eyes.map(mid), alt.map(mid)) {
        (Some(e), Some(a)) if a.distance(e) > 0.005 => Some(a),
        (Some(e), _) => Some(e),
        (None, Some(a)) => Some(a),
        (None, None) => head.map(|(r, p)| p + r * EYES_FROM_HEAD),
    }
}

/// Rotation facing `dir` with Z up (LLQuaternion(at, left, up)).
fn facing(dir: Vec3) -> Option<Quat> {
    let left = Vec3::Z.cross(dir).normalize_or_zero();
    if left == Vec3::ZERO {
        return None;
    }
    let up = dir.cross(left);
    Some(Quat::from_mat3(&Mat3::from_cols(dir, left, up)).normalize())
}

impl Controller {
    /// Dev Toolbox J9: the motions playing, newest first, with their local
    /// time (s) and blend weight.
    pub(crate) fn playing(&self, now: Instant) -> Vec<(Uuid, Arc<BoundAnim>, f64, f32)> {
        self.motions
            .iter()
            .map(|m| (m.id, m.anim.clone(), m.local_time(now), m.weight))
            .collect()
    }

    /// Dev Toolbox J9: the last frame's local rotations, animated position
    /// offsets from the rest skeleton (empty before the first), and the
    /// pelvis's animated position under the root (kept apart).
    pub(crate) fn held_pose(&self) -> (&[Quat], &[Vec3], Option<Vec3>) {
        (&self.rot, &self.pos_delta, self.pelvis_offset)
    }

    /// Apply Preferences › Aurora › Animations without restarting live loops.
    /// Enabling catches up to the known phase; disabling keeps the current phase
    /// and uses the Legacy Viewer clock for subsequent starts and pauses.
    pub fn set_synchronized(&mut self, synchronized: bool) {
        if synchronized && !self.synchronized {
            for m in &mut self.motions {
                if m.stop.is_none() && !m.internal && !self.preview_start.is_some_and(|(target, _)| target.is_none_or(|id| id == m.id)) {
                    m.activation = m.signal.sequence_start;
                    m.continuous_activation = m.signal.continuous_start;
                }
            }
        }
        self.synchronized = synchronized;
    }

    /// Motions playing (`avatar summary`, AURORA_PROFILE).
    pub fn motion_count(&self) -> usize {
        self.motions.len()
    }

    /// Freeze the last local pose for appearance hints; shape deltas are still
    /// applied independently when each hint is built.
    pub fn preview_pose(&self) -> (Vec<Quat>, Vec<Vec3>) {
        (self.rot.clone(), self.pos_delta.clone())
    }
    pub fn for_control_avatar() -> Self {
        Self {
            control: true,
            ..Default::default()
        }
    }
    /// FSToolsResyncAnimations (indra/newview/llviewermenu.cpp): stop and
    /// reactivate playing motions locally at one shared instant. Keep the
    /// receipt signals so the next sync does not undo the explicit restart.
    /// Stopped motions and completed one-shots must not be resurrected.
    pub fn resync_animations(&mut self, now: Instant) {
        let now = if self.synchronized { now } else { self.paused_at.unwrap_or(now) };
        let preview = self.preview_start;
        for motion in self
            .motions
            .iter_mut()
            .filter(|m| !m.internal && m.stop.is_none() && !preview.is_some_and(|(target, _)| target.is_none_or(|id| id == m.id)))
        {
            motion.activation = now;
            motion.continuous_activation = now;
            motion.weight = 0.0;
            motion.residual = 0.0;
            motion.recovery = None;
        }
        self.last_eval = None;
    }

    /// Forget the held pose (LLVOAvatar::resetSkeleton): a joint no motion
    /// drives keeps the rotation and position the last motion gave it, which
    /// is how an avatar stays deformed after a pose or a mesh animation
    /// stopped. The next frame starts again from the rest skeleton.
    /// `animations`: also LLMotionController::flushAllMotions, which restarts
    /// the active motions at the time they had reached; motions easing out
    /// are dropped instead (restarted, they would never be stopped again).
    pub fn reset_pose(&mut self, animations: bool) {
        self.rot.clear();
        self.pos_delta.clear();
        self.tag_pos_delta.clear();
        self.pelvis_offset = None;
        self.tag_pelvis_offset = 0.0;
        self.last_eval = None;
        self.placement = grounding::Placement::default();
        self.legacy_rot.clear();
        self.legacy_held = false;
        self.ground_handoff = legacy_grounding::Handoff::default();
        self.ground_origin = None;
        if animations {
            self.motions.retain(|m| m.stop.is_none());
        }
    }

    /// Freeze the displayed pose for editing (LLCharacter::requestPause).
    /// The synchronized mode catches up on release; Legacy suspends playback
    /// like LLMotionController::pauseAllMotions.
    /// An import preview drives its own clock and is never paused.
    pub fn set_paused(&mut self, paused: bool, now: Instant) {
        let paused = paused && self.preview_start.is_none();
        match (self.paused_at, paused) {
            (None, true) => self.paused_at = Some(now),
            (Some(at), false) => {
                self.paused_at = None;
                let duration = now.saturating_duration_since(at);
                if !self.synchronized {
                    for m in &mut self.motions {
                        m.activation += duration;
                        m.continuous_activation += duration;
                        m.stop = m.stop.map(|stop| stop + duration);
                    }
                }
                self.resume_procedural(duration);
            }
            _ => {}
        }
    }

    /// Head/eye interpolation resumes smoothly in either playback mode.
    fn resume_procedural(&mut self, d: Duration) {
        let pr = &mut self.procedural;
        for fade in [&mut pr.head, &mut pr.eyes] {
            fade.change = fade.change.map(|c| c + d);
        }
        pr.jitter_at = pr.jitter_at.map(|t| t + d);
        pr.breathe_start = pr.breathe_start.map(|t| t + d);
        pr.noise_start = pr.noise_start.map(|t| t + d);
        self.last_eval = self.last_eval.map(|t| t + d);
        self.placement.resume(d);
        self.ground_handoff.resume(d);
        self.pointing.resume(d);
    }

    /// Skeleton-space position of the head as posed last frame.
    pub fn head_pos(&self) -> Option<Vec3> {
        self.posed.head.map(|h| h.1)
    }

    /// Skeleton-space point between the eyes as posed last frame
    /// (`eyes_between`), for the look-at line.
    pub fn eyes_pos(&self) -> Option<Vec3> {
        let p = &self.posed;
        let pair = |l: Option<(Quat, Vec3)>, r: Option<(Quat, Vec3)>| Some((l?.1, r?.1));
        eyes_between(pair(p.eye_l, p.eye_r), pair(p.alt_l, p.alt_r), p.head)
    }

    /// The live morph weights of the last frame: the expressions and hand
    /// poses, as differences from the weights baked into the shape.
    pub fn live_morphs(&self) -> &[(i32, f32)] {
        &self.live_morphs
    }

    /// Joint states of the default motions for this frame: head and eyes,
    /// breathing and body noise.
    fn procedural(&mut self, rig: &Rig, base: Option<&SkeletonBase>, j: &LookJoints<usize>, now: Instant) -> Vec<ProceduralState> {
        let dt = self.last_eval.map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.0).min(1.0);
        self.last_eval = Some(now);
        let mut out = Vec::new();
        let p = &self.posed;
        let pr = &mut self.procedural;
        let head_w = pr.head.update(!self.look_cleared, now, HEAD_ROT_EASE);
        let eye_w = pr.eyes.update(true, now, EYE_EASE);
        // LLHeadRotMotion::onUpdate (in the root frame)
        if head_w > 0.0 {
            let target = match self.look {
                Some(look) => {
                    let dist = look.length();
                    if dist < MIN_HEAD_LOOKAT_DISTANCE {
                        p.pelvis.map(|x| x.0).unwrap_or(Quat::IDENTITY)
                    } else {
                        let mut dir = look / dist;
                        if Vec3::Z.cross(dir).length_squared() < 0.15 {
                            // near vertical: lean toward the front
                            dir = dir.lerp(Vec3::X, 0.4).normalize_or(Vec3::X);
                        }
                        facing(dir).unwrap_or(Quat::IDENTITY)
                    }
                }
                None => Quat::IDENTITY,
            };
            let head_local = constrain(target, HEAD_ROTATION_CONSTRAINT);
            let torso_target = nlerp(TORSO_LAG, Quat::IDENTITY, head_local);
            pr.torso = nlerp(interpolant(dt, TORSO_LOOKAT_LAG_HALF_LIFE), pr.torso, torso_target);
            pr.last_head = nlerp(interpolant(dt, HEAD_LOOKAT_LAG_HALF_LIFE), pr.last_head, head_local);
            if let Some(t) = j.torso {
                out.push((t, MEDIUM_PRIORITY, head_w, pr.torso, false));
            }
            if let (Some(neck), Some(head), Some(chest)) = (j.neck, j.head, p.chest) {
                // what is left after the chest, half to the neck, half to the head
                let h = chest.0.inverse() * pr.last_head;
                let half = nlerp(NECK_LAG, Quat::IDENTITY, h);
                out.push((neck, MEDIUM_PRIORITY, head_w, half, false));
                out.push((head, MEDIUM_PRIORITY, head_w, half, false));
            }
        }
        // LLEyeMotion::onUpdate
        if eye_w > 0.0 {
            pr.jitter(now);
            let head_rot = p.head.map(|h| h.0).unwrap_or(Quat::IDENTITY);
            for (l, r, pl, pp) in [(j.eye_l, j.eye_r, p.eye_l, p.eye_r), (j.alt_l, j.alt_r, p.alt_l, p.alt_r)] {
                let (Some(l), Some(r)) = (l, r) else {
                    continue;
                };
                let mut rot = Quat::IDENTITY;
                let mut vergence = 0.0;
                let target = self.look.and_then(|look| {
                    let dist = look.length();
                    Some((facing(look / dist.max(1e-4))?, dist))
                });
                if let Some((tq, dist)) = target {
                    // eye rotation in the head frame, without roll, limited
                    let local = head_rot.inverse() * tq;
                    let f = local * Vec3::X;
                    let yaw = f.y.atan2(f.x);
                    let pitch = (-f.z).atan2((f.x * f.x + f.y * f.y).sqrt());
                    rot = constrain(Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch), EYE_ROT_LIMIT_ANGLE);
                    let iod = match (pl, pp) {
                        (Some(a), Some(b)) => a.1.distance(b.1),
                        _ => 0.064,
                    };
                    vergence = (-(iod * 0.5).atan2(dist)).clamp(-std::f32::consts::FRAC_PI_2, 0.0);
                }
                // foveal offset
                vergence += 4f32.to_radians();
                let jitter = if vergence > -0.05 {
                    Quat::from_rotation_z(pr.jitter_yaw + pr.look_away_yaw) * Quat::from_rotation_y(pr.jitter_pitch + pr.look_away_pitch)
                } else {
                    Quat::IDENTITY
                };
                let vq = if target.is_some() {
                    Quat::from_rotation_z(vergence)
                } else {
                    Quat::IDENTITY
                };
                out.push((l, MEDIUM_PRIORITY, eye_w, rot * jitter * vq, false));
                out.push((r, MEDIUM_PRIORITY, eye_w, rot * jitter * vq.inverse(), false));
            }
        }
        // LLBreatheMotionRot (MEDIUM_PRIORITY, normal blend), from the
        // avatar's first frame. Deliberate difference: it never stops for a
        // small avatar on screen (MIN_REQUIRED_PIXEL_AREA_BREATHE), a cost
        // the GPU skinning does not have.
        let breathe = now.saturating_duration_since(*pr.breathe_start.get_or_insert(now)).as_secs_f32();
        if let Some(chest) = j.chest {
            out.push((chest, MEDIUM_PRIORITY, 1.0, default_motions::breathe(breathe), false));
        }
        // LLBodyNoiseMotion (HIGH_PRIORITY, additive), stopped while
        // sitting on an object or aiming (LLVOAvatar::sitOnObject,
        // processAnimationStateChanges); it starts again from its beginning
        if self.sitting || self.aiming {
            pr.noise_start = None;
        } else if let Some(torso) = j.torso {
            let t = now.saturating_duration_since(*pr.noise_start.get_or_insert(now)).as_secs_f32();
            out.push((torso, HIGH_PRIORITY, 1.0, default_motions::body_noise(t), true));
        }
        out.extend(self.pointing.states(
            rig,
            pointing::Frame {
                rotations: &self.rot,
                position_delta: &self.pos_delta,
                pelvis: self.pelvis_offset.unwrap_or(Vec3::ZERO),
                base,
                point: self.point,
                look: self.look,
                aiming: self.aiming,
            },
            now,
            dt,
        ));
        out
    }

    /// Import preview clocks override the first known signal. Seeking restarts
    /// only the preview, without touching server state.
    pub fn set_preview_clock(&mut self, clock: Option<(Option<Uuid>, Instant)>) {
        if self.preview_start == clock {
            return;
        }
        if let Some((target, _)) = clock {
            self.motions.retain(|m| target.is_some_and(|id| m.id != id));
            self.signals.retain(|id, _| target.is_some_and(|target| *id != target));
        }
        self.preview_start = clock;
    }

    /// Follow the server's animation list (LLVOAvatar::processAnimationStateChanges).
    /// `get` gives an animation once it has loaded.
    /// Return timed stops once, like LLMotionController::updateMotionsByType
    /// -> requestStopMotion: at ease-out, or the authored end when the
    /// scheduled ease-out overlaps ease-in.
    pub fn sync(&mut self, playing: &[PlayingAnimation], now: Instant, mut get: impl FnMut(&Uuid) -> Option<Arc<BoundAnim>>) -> Vec<Uuid> {
        let mut completed = Vec::new();
        let frozen = self.paused_at;
        let now = frozen.unwrap_or(now);
        let preview = self.preview_start;
        let activation = |id, start: Instant| {
            preview
                .filter(|(target, _)| target.is_none_or(|target| target == id))
                .map_or(if self.synchronized { start } else { now }, |(_, start)| start)
        };
        self.aiming = playing.iter().any(|p| default_motions::GUN_AIM_ANIMS.contains(&p.id));
        // stop the motions the server no longer plays
        for m in self.motions.iter_mut().filter(|m| !m.internal) {
            if !playing
                .iter()
                .any(|p| p.id == m.id && p.continuous_start == m.signal.continuous_start)
            {
                m.stop_at(now);
            }
        }
        if frozen.is_some() {
            // a motion stopped during a pause goes at once
            // (LLMotionController::stopMotionLocally: stop_immediate || mPaused)
            self.motions.retain(|m| m.stop != Some(now));
        }
        self.signals.retain(|id, _| playing.iter().any(|p| p.id == *id));
        // Synchronized playback joins the receipt phase. Legacy activates on
        // loading, as LLMotionController::updateLoadingMotions does.
        // A motion easing out is left to finish while a new instance starts.
        for signal in playing {
            if self.signals.get(&signal.id) == Some(signal) {
                continue;
            }
            if let Some(m) = self.motions.iter_mut().find(|m| m.id == signal.id && m.stop.is_none()) {
                if m.anim.anim.looping && m.signal.continuous_start == signal.continuous_start {
                    // LLVOAvatar signals sequence changes, but startMotion
                    // keeps an already active loop running continuously.
                    m.activation = activation(signal.id, signal.sequence_start);
                    m.signal = *signal;
                    self.signals.insert(signal.id, *signal);
                    continue;
                }
                m.stop_at(now);
            }
            if let Some(anim) = get(&signal.id) {
                let emote = default_motions::emote_named(&anim.anim.emote_name);
                let start = activation(signal.id, signal.sequence_start);
                self.motions.insert(
                    0,
                    Motion {
                        id: signal.id,
                        anim,
                        activation: start,
                        continuous_activation: activation(signal.id, signal.continuous_start),
                        signal: *signal,
                        stop: None,
                        residual: 0.0,
                        weight: 0.0,
                        internal: false,
                        recovery: None,
                    },
                );
                self.signals.insert(signal.id, *signal);
                // LLKeyframeMotion::onActivate: the expression the animation
                // names starts with it, unless it is already playing
                if let Some(emote) = emote
                    && !self.motions.iter().any(|m| m.id == emote)
                    && let Some(anim) = get(&emote)
                {
                    self.motions.insert(
                        0,
                        Motion {
                            id: emote,
                            anim,
                            activation: start,
                            continuous_activation: start,
                            signal: *signal,
                            stop: None,
                            residual: 0.0,
                            weight: 0.0,
                            internal: true,
                            recovery: None,
                        },
                    );
                }
            }
        }
        // weights (LLMotionController::updateMotionsByType)
        self.motions.retain_mut(|m| {
            let a = &m.anim.anim;
            // LLMotionController::updateMotionsByType does not request a
            // timed stop during ease-in. If mSendStopTimestamp falls there,
            // LLKeyframeMotion::onUpdate runs to the authored end instead.
            // In particular, the stock pre-jump must last 0.333333 s, not
            // duration - ease_out = 0.133333 s (#1637).
            if m.stop.is_none() && !a.looping && a.duration > 0.0 {
                let duration = f64::from(a.duration);
                let ease_out_start = (duration - f64::from(a.ease_out)).max(0.0);
                let finish = if ease_out_start < f64::from(a.ease_in) {
                    duration
                } else {
                    ease_out_start
                };
                let end = m.activation + Duration::from_secs_f64(finish);
                if now >= end {
                    m.stop_at(end);
                    if !m.internal && !preview.is_some_and(|(target, _)| target.is_none_or(|id| id == m.id)) {
                        completed.push(m.id);
                    }
                }
            }
            match m.stop {
                Some(stop) if now >= stop => {
                    let a = &m.anim.anim;
                    let fade = m.fade_start(stop);
                    if now < fade {
                        m.weight = m.ease_in_weight(now);
                        return true;
                    }
                    let out = now.duration_since(fade).as_secs_f64();
                    if out >= f64::from(a.ease_out) {
                        return false;
                    }
                    m.weight = m.residual * aurora_assets::anim::cubic_step(1.0 - out / f64::from(a.ease_out));
                }
                _ => {
                    m.weight = m.ease_in_weight(now);
                    m.residual = m.weight;
                }
            }
            true
        });
        completed
    }

    /// Evaluate the pose and write the joint world matrices (skeleton
    /// space, feet at z = 0); meshes multiply them by their inverse binds.
    /// `base`: the avatar's rest skeleton (None = default).
    pub fn evaluate(&mut self, rig: &Rig, now: Instant, base: Option<&SkeletonBase>, out: &mut [[[f32; 4]; 4]]) {
        self.evaluate_on_ground(rig, now, base, out, |_| None);
    }

    pub(super) fn evaluate_on_ground(
        &mut self,
        rig: &Rig,
        now: Instant,
        base: Option<&SkeletonBase>,
        out: &mut [[[f32; 4]; 4]],
        ground: impl Fn(Vec3) -> Option<grounding::Surface>,
    ) {
        self.evaluate_with_support(rig, now, base, out, ground, |_| None);
    }

    pub(super) fn evaluate_with_support(
        &mut self,
        rig: &Rig,
        now: Instant,
        base: Option<&SkeletonBase>,
        out: &mut [[[f32; 4]; 4]],
        ground: impl Fn(Vec3) -> Option<grounding::Surface>,
        legacy_ground: impl Fn(Vec3) -> Option<grounding::Surface>,
    ) {
        let now = self.paused_at.unwrap_or(now);
        let n = rig.len().min(out.len());
        // Keep the displayed pose even if signals stop/change or a file arrives
        // during editing. The first evaluation still initializes an unseen rig.
        let hold_pose = self.paused_at.is_some() && self.rot.len() == rig.len();
        if self.rot.len() != rig.len() {
            self.rot = rig.local_rot.clone();
            self.pos_delta = vec![Vec3::ZERO; rig.len()];
            self.tag_pos_delta = vec![0.0; rig.len()];
        }
        let rest_pos: &[Vec3] = match base {
            Some(b) => &b.local_pos,
            None => &rig.local_pos,
        };
        // joint states per joint: (priority, weight, rotation, position),
        // highest priority first, newer motions first among equals
        // (LLJointStateBlender::addJointState)
        // Last flag: this source motion predates the editing arm, which
        // starts on selection rather than with the default motions.
        type State = (i32, f32, Option<Quat>, Option<(Vec3, f32)>, bool, bool, Option<Quat>);
        let mut states: Vec<Vec<State>> = vec![Vec::new(); n];
        let editing = self.control || self.preview_start.is_some() || self.ik.edit.active || self.poser.is_some();
        if let Some(frame) = self.legacy_ground.or(self.ground) {
            let origin = frame.to_world.w_axis.truncate();
            if self
                .ground_origin
                .replace(origin)
                .is_some_and(|previous| previous.distance_squared(origin) > 4.0)
            {
                self.ground_handoff = legacy_grounding::Handoff::default();
            }
        }
        let native_frame = self.legacy_ground.filter(|frame| {
            !self.aurora_feet
                && !editing
                && !self
                    .motions
                    .iter()
                    .any(|motion| motion.stop.is_none() && grounding::unplanted(motion.id, frame.standing_anchor))
        });
        let blend_native = native_frame.is_some()
            && if hold_pose {
                self.legacy_held
            } else {
                self.motions.iter().any(|motion| legacy_grounding::supported(motion.id))
            };
        self.legacy_held = blend_native;
        if blend_native && self.legacy_rot.len() != rig.len() {
            self.legacy_rot.clone_from(&self.rot);
        }
        for m in self.motions.iter_mut().filter(|_| !hold_pose) {
            if m.weight <= 0.0 {
                continue;
            }
            let native = native_frame.map_or_else(Vec::new, |frame| legacy_grounding::sample(m, rig, base, frame, &legacy_ground, now));
            // Recovery's ease-in becomes known at its first evaluated support.
            // Subsequent syncs use the same activation snapshot.
            if m.recovery.is_some() && m.stop.is_none() {
                m.weight = m.ease_in_weight(now);
                m.residual = m.weight;
            }
            let a = &m.anim.anim;
            let t = m.local_time(now) as f32;
            for ((jm, j), top) in a.joints.iter().zip(&m.anim.joints).zip(&m.anim.position_top) {
                let Some(j) = *j else {
                    continue;
                };
                if j >= n {
                    continue;
                }
                let native = native.iter().find_map(|&(joint, rotation)| (joint == j).then_some(rotation));
                let rot = jm.rotation_at_loop(t, a.loop_range());
                let pos = jm
                    .position_at_loop(t, a.loop_range())
                    .filter(|p| p.is_finite())
                    .map(|p| (p, top.unwrap_or(p.z)));
                if rot.is_none() && pos.is_none() {
                    continue;
                }
                let prio = if jm.priority >= 0 { jm.priority } else { a.base_priority };
                let s = &mut states[j];
                let at = s.iter().position(|o| prio > o.0).unwrap_or(s.len());
                if at < JOINT_STATES {
                    let activation = if a.looping { m.continuous_activation } else { m.activation };
                    s.insert(
                        at,
                        (
                            prio,
                            m.weight,
                            rot,
                            pos,
                            false,
                            self.pointing.animation_is_older(activation),
                            native,
                        ),
                    );
                    s.truncate(JOINT_STATES);
                }
            }
        }
        // head, eye, breathing and body noise motions, started with the
        // avatar (startDefaultMotions)
        let lj = LookJoints::of(rig);
        let procedural = if self.control || hold_pose {
            Vec::new()
        } else if self.paused_at.is_some() {
            self.held_procedural.clone()
        } else {
            let p = self.procedural(rig, base, &lj, now);
            self.held_procedural.clone_from(&p);
            p
        };
        for (j, prio, w, r, additive) in procedural {
            if let Some(s) = states.get_mut(j) {
                let editing_arm = !additive && prio == HIGH_PRIORITY && self.pointing.arm_joint(j);
                // the additive motions are blended first
                // (LLMotionController::updateMotions), so they come before
                // the regular states of their priority
                let at = s
                    .iter()
                    .position(|o| {
                        if additive {
                            prio >= o.0
                        } else {
                            prio > o.0 || editing_arm && prio == o.0 && o.5
                        }
                    })
                    .unwrap_or(s.len());
                if at < JOINT_STATES {
                    s.insert(at, (prio, w, Some(r), None, additive, false, None));
                    s.truncate(JOINT_STATES);
                }
            }
        }
        let mut local_pos: Vec<Vec3> = rest_pos.to_vec();
        // joints turned by an additive motion: the held pose stays without it
        let mut added_rot: Vec<(usize, Quat)> = Vec::new();
        let mut native_added_rot: Vec<(usize, Quat)> = Vec::new();
        for (j, s) in states.iter().enumerate() {
            let mut rot_sum = 0.0f32;
            let mut rot = self.rot[j];
            let mut native_rot = if blend_native && hold_pose {
                self.legacy_rot[j]
            } else {
                self.rot[j]
            };
            let mut pos_sum = 0.0f32;
            let mut pos = Vec3::ZERO;
            let mut top = 0.0f32;
            let mut added: Option<Quat> = None;
            for &(_, w, r, p, additive, _, native) in s {
                if additive {
                    // LLJointStateBlender::blendJointStates: what the
                    // stronger regular states leave of its weight
                    if let Some(r) = r.filter(|_| w > 0.0) {
                        let a = added.unwrap_or(Quat::IDENTITY);
                        let sum = (rot_sum + w).min(1.0);
                        added = Some((nlerp(sum - rot_sum, a, r) * a).normalize());
                    }
                    continue;
                }
                if let Some(r) = r {
                    if rot_sum > 0.0 {
                        let sum = (rot_sum + w).min(1.0);
                        rot = nlerp(rot_sum / sum, r, rot);
                        if blend_native {
                            native_rot = nlerp(rot_sum / sum, native.unwrap_or(r), native_rot);
                        }
                        rot_sum = sum;
                    } else {
                        rot = r;
                        native_rot = native.unwrap_or(r);
                        rot_sum = w;
                    }
                }
                if let Some((p, z)) = p {
                    if pos_sum > 0.0 {
                        let sum = (pos_sum + w).min(1.0);
                        pos = p.lerp(pos, pos_sum / sum);
                        top = z + (top - z) * (pos_sum / sum);
                        pos_sum = sum;
                    } else {
                        pos = p;
                        top = z;
                        pos_sum = w;
                    }
                }
            }
            self.rot[j] = rot;
            if blend_native {
                self.legacy_rot[j] = native_rot;
            }
            if let Some(a) = added {
                added_rot.push((j, (a * rot).normalize()));
                if blend_native {
                    native_added_rot.push((j, (a * native_rot).normalize()));
                }
            }
            if pos_sum > 0.0 {
                if j == rig.pelvis {
                    // the pelvis is animated relative to the avatar root
                    self.pelvis_offset = Some(pos);
                    self.tag_pelvis_offset = top;
                } else if let Some(rest) = rest_pos.get(j) {
                    self.pos_delta[j] = pos - *rest;
                    self.tag_pos_delta[j] = top - rest.z;
                }
            }
            if j != rig.pelvis
                && let Some(lp) = local_pos.get_mut(j)
            {
                *lp += self.pos_delta[j];
            }
        }
        let scale = match base {
            Some(b) => b.scale.as_slice(),
            None => rig.scale.as_slice(),
        };
        // LLJointStateBlender::blendJointStates sets an absolute local
        // position, even on a mesh-overridden pelvis. The feet-origin
        // conversion must not add the rest pelvis translation a second time.
        let pelvis_rest =
            rest_pos.get(rig.pelvis).copied().unwrap_or(Vec3::ZERO) - rig.local_pos.get(rig.pelvis).copied().unwrap_or(Vec3::ZERO);
        let pelvis_offset = self.pelvis_offset.map_or(Vec3::ZERO, |position| position - pelvis_rest);
        let source_rotations = if added_rot.is_empty() {
            None
        } else {
            let mut rot = self.rot.clone();
            for (j, r) in added_rot {
                rot[j] = r;
            }
            Some(rot)
        };
        let source_rotations = source_rotations.as_deref().unwrap_or(&self.rot);
        // Include additive source motions in placement, while keeping both
        // additive and placement deltas out of self.rot's held blend state.
        let protected = |names: [&str; 3]| {
            names.iter().any(|name| {
                rig.names.get(*name).and_then(|&j| states.get(j)).is_some_and(|states| {
                    states
                        .iter()
                        .any(|&(priority, weight, _, _, _, _, _)| priority >= 5 && weight > 0.01)
                })
            })
        };
        let allowed = [
            !protected(["mPelvis"; 3]) && !protected(["mHipLeft", "mKneeLeft", "mAnkleLeft"]),
            !protected(["mPelvis"; 3]) && !protected(["mHipRight", "mKneeRight", "mAnkleRight"]),
        ];
        if editing || !self.aurora_feet {
            self.placement = grounding::Placement::default();
        }
        let frame = self.ground.filter(|frame| {
            self.aurora_feet
                && !self.control
                && !self.ik.edit.active
                && self.poser.is_none()
                && self.preview_start.is_none()
                && (frame.standing_anchor || frame.velocity_z.abs() < 0.5)
                && !self
                    .motions
                    .iter()
                    .any(|motion| motion.stop.is_none() && grounding::unplanted(motion.id, frame.standing_anchor))
        });
        let fallback;
        let skeleton = match base {
            Some(base) => base,
            None => {
                fallback = SkeletonBase {
                    local_pos: rig.local_pos.clone(),
                    scale: rig.scale.clone(),
                };
                &fallback
            }
        };
        let corrected = self
            .aurora_feet
            .then(|| {
                self.placement.apply(
                    rig,
                    skeleton,
                    grounding::Pose {
                        rotations: source_rotations,
                        positions: &local_pos,
                        pelvis: pelvis_offset,
                        reference_offset: self.pelvis_offset.map_or(Vec3::ZERO, |_| -pelvis_rest),
                        allowed,
                    },
                    frame,
                    now,
                    &ground,
                )
            })
            .flatten();
        let mut native_rotations;
        let selected = if self.aurora_feet || editing || !blend_native {
            corrected.as_ref().map_or(source_rotations, |pose| pose.rotations.as_slice())
        } else {
            native_rotations = self.legacy_rot.clone();
            for (joint, rotation) in native_added_rot {
                native_rotations[joint] = rotation;
            }
            &native_rotations
        };
        let mut transitioned = None;
        if editing {
            self.ground_handoff = legacy_grounding::Handoff::default();
        } else if self.legacy_ground.is_some() || self.ground.is_some() {
            transitioned = self.ground_handoff.apply(
                rig,
                source_rotations,
                selected,
                corrected.as_ref().map_or(Vec3::ZERO, |pose| pose.pelvis),
                self.aurora_feet,
                allowed,
                now,
            );
        } else {
            self.ground_handoff = legacy_grounding::Handoff::default();
        }
        let selected_pelvis = pelvis_offset + corrected.as_ref().map_or(Vec3::ZERO, |pose| pose.pelvis);
        let (rotations, pelvis) = transitioned.as_ref().map_or((selected, selected_pelvis), |pose| {
            (pose.rotations.as_slice(), pelvis_offset + pose.pelvis)
        });
        // Explicit contacts and authored poses take precedence over automatic
        // foot placement; keep all corrections out of the animation blend state.
        if self.ik.edit.active {
            self.ik.set_rest_positions(rest_pos);
        }
        let edited = self.ik.apply(rig, rotations, &local_pos, pelvis, scale, now, hold_pose);
        let (rotations, positions) = edited.as_ref().map_or((rotations, local_pos.as_slice()), |p| {
            (p.rotations.as_slice(), p.positions.as_slice())
        });
        let world = self.poser.as_ref().map_or_else(
            || rig.world_with(rotations, pelvis, positions, scale),
            |(pose, model)| pose.matrices(rig, *model),
        );
        if !hold_pose {
            self.update_live_morphs(now);
        }
        // Deliberate difference from LLVOAvatar::idleCalcNameTagPosition:
        // the human requested a stable tag, not one following the posed head.
        // Keep shape / mesh offsets and translation deformers, but measure
        // the upright skeleton with the whole animation's translation bounds.
        // Only the cached root-to-skull chain is summed; no extra matrices.
        let tag_pelvis_offset = if self.pelvis_offset.is_some() {
            self.tag_pelvis_offset - pelvis_rest.z
        } else {
            0.0
        };
        self.name_tag_height = rig.name_tag_height(rest_pos, scale, &self.tag_pos_delta, tag_pelvis_offset);
        for j in 0..n {
            out[j] = world[j].to_cols_array_2d();
        }
        let pose = |j: Option<usize>| {
            let m = world.get(j?)?;
            let (_, r, t) = m.to_scale_rotation_translation();
            Some((r.normalize(), t))
        };
        self.posed = LookJoints {
            pelvis: pose(lj.pelvis),
            torso: pose(lj.torso),
            neck: pose(lj.neck),
            chest: pose(lj.chest),
            head: pose(lj.head),
            eye_l: pose(lj.eye_l),
            eye_r: pose(lj.eye_r),
            alt_l: pose(lj.alt_l),
            alt_r: pose(lj.alt_r),
        };
    }

    /// The expression and hand pose morph weights of this frame
    /// (default_motions.rs).
    fn update_live_morphs(&mut self, now: Instant) {
        self.live_morphs.clear();
        // LLEmote::onUpdate: the expression's morph follows the motion's
        // weight, Express_Closed_Mouth (1 by default) the opposite way.
        // With several at once the strongest closes the mouth least (the
        // Legacy Viewer keeps whichever updated last).
        let mut strongest = 0.0f32;
        for m in &self.motions {
            if let Some(param) = default_motions::emote_param(&m.id) {
                let w = m.weight.clamp(0.0, 1.0);
                strongest = strongest.max(w);
                match self.live_morphs.iter_mut().find(|l| l.0 == param) {
                    Some(l) => l.1 = l.1.max(w),
                    None => self.live_morphs.push((param, w)),
                }
            }
        }
        if strongest > 0.0 {
            self.live_morphs.push((default_motions::CLOSED_MOUTH, -strongest));
        }
        // LLKeyframeMotion::applyKeyframes's "Hand Pose": that of the
        // animation of highest priority; among equals the last one updated,
        // the oldest
        if self.control {
            return;
        }
        let mut request: Option<(i32, u32)> = None;
        self.joint_signature.clear();
        for m in self.motions.iter().filter(|m| default_motions::emote_param(&m.id).is_none()) {
            // LLMotionController::updateMotionsByType: a motion at full
            // weight that adds no joint (or no higher priority on one) to
            // those of the newer motions is not updated, so it asks for no
            // pose. An older stand under a newer animation of its priority
            // leaves the hands to it; one without joints only asks while it
            // eases in or out.
            if m.weight >= 1.0 {
                let mut adds = false;
                for &(j, bits) in &m.anim.signature {
                    if self.joint_signature.len() <= j {
                        self.joint_signature.resize(j + 1, [0; 2]);
                    }
                    let held = &mut self.joint_signature[j];
                    for (held, bits) in held.iter_mut().zip(bits) {
                        adds |= bits & !*held != 0;
                        *held |= bits;
                    }
                }
                if !adds {
                    continue;
                }
            }
            if request.is_none_or(|(p, _)| m.anim.max_priority >= p) {
                request = Some((m.anim.max_priority, m.anim.anim.hand_pose));
            }
        }
        // LLEditingMotion requests HAND_POSE_RELAXED_R at priority 3.
        if self.pointing.editing() && request.is_none_or(|(priority, _)| priority < 3) {
            request = Some((3, 7));
        }
        self.hands.update(request.map(|r| r.1), now);
        self.live_morphs.extend(self.hands.weights());
    }
}

// ------------------------------------------------------------------ streaming

struct Entry {
    anim: Option<Arc<BoundAnim>>,
    state: u8, // 0 new, 1 cache, 2 fetching, 3 ready, 4 failed
    retry_at: Option<Instant>,
    failures: u32,
}

pub struct AnimStreamer {
    entries: HashMap<Uuid, Entry>,
    by_key: HashMap<u64, Uuid>,
    next_key: u64,
    cache_dir: PathBuf,
    rx: crossbeam_channel::Receiver<(Uuid, Option<Arc<BoundAnim>>, bool)>,
    tx: crossbeam_channel::Sender<(Uuid, Option<Arc<BoundAnim>>, bool)>,
}

impl AnimStreamer {
    pub fn new(cache_dir: PathBuf) -> AnimStreamer {
        let _ = std::fs::create_dir_all(cache_dir.join("anim"));
        let (tx, rx) = crossbeam_channel::unbounded();
        AnimStreamer {
            entries: HashMap::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir,
            rx,
            tx,
        }
    }

    pub fn get(&mut self, id: &Uuid) -> Option<Arc<BoundAnim>> {
        let e = self.entries.entry(*id).or_insert(Entry {
            anim: None,
            state: 0,
            retry_at: None,
            failures: 0,
        });
        e.anim.clone()
    }

    /// An animation already loaded, without asking for it (the Animation
    /// explorer's priority column).
    pub fn peek(&self, id: &Uuid) -> Option<Arc<BoundAnim>> {
        self.entries.get(id).and_then(|e| e.anim.clone())
    }

    /// Gestures wait for preloads to succeed or fail before running steps.
    pub fn asset_settled(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(|entry| entry.state >= 3)
    }

    /// Register an already-built animation (demo / built-in motions).
    pub fn insert(&mut self, id: Uuid, anim: Arc<BoundAnim>) {
        self.entries.insert(
            id,
            Entry {
                anim: Some(anim),
                state: 3,
                retry_at: None,
                failures: 0,
            },
        );
    }

    pub fn remove_local(&mut self, id: &Uuid) -> Option<Arc<BoundAnim>> {
        self.entries.remove(id).and_then(|entry| entry.anim)
    }

    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>, rig: &Arc<Rig>) {
        while let Ok((id, anim, from_cache)) = self.rx.try_recv() {
            if let Some(e) = self.entries.get_mut(&id) {
                match anim {
                    Some(a) => {
                        e.anim = Some(a);
                        e.state = 3;
                    }
                    None if from_cache => e.state = 4,
                    None => {
                        e.state = 4;
                        e.failures += 1;
                        e.retry_at = Some(Instant::now() + Duration::from_secs(30));
                    }
                }
            }
        }
        let now = Instant::now();
        for (id, e) in self.entries.iter_mut() {
            if e.retry_at.is_some_and(|t| now < t) {
                continue;
            }
            if e.state == 0 {
                e.state = 1;
                let path = self.cache_dir.join("anim").join(format!("{id}.anim"));
                let tx = self.tx.clone();
                let rig = rig.clone();
                let id = *id;
                jobs.spawn(move || {
                    let a = crate::cache::read_touch(&path)
                        .ok()
                        .and_then(|d| aurora_assets::parse_animation(&d).ok())
                        .map(|a| Arc::new(BoundAnim::bind(a, &rig)));
                    let _ = tx.send((id, a, true));
                    super::jobs::JobResult::Done
                });
            } else if e.state == 4 && e.failures <= 3 {
                let Some(base) = viewer_asset else {
                    continue;
                };
                let key = FETCH_KIND_ANIM | self.next_key;
                self.next_key += 1;
                self.by_key.insert(key, *id);
                e.state = 2;
                fetcher.request(FetchRequest {
                    key,
                    url: super::textures::asset_url(base, "animatn_id", id),
                    range: None,
                    // (an avatar stands still until it arrives)
                    priority: urgent::ANIMATION,
                    lane: Lane::Urgent,
                    accept: "*/*",
                });
            }
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult, jobs: &Jobs, rig: &Arc<Rig>) {
        let Some(id) = self.by_key.remove(&r.key) else {
            return;
        };
        match r.data {
            Ok(d) => {
                let path = self.cache_dir.join("anim").join(format!("{id}.anim"));
                let tx = self.tx.clone();
                let rig = rig.clone();
                jobs.spawn(move || {
                    let a = aurora_assets::parse_animation(&d).ok();
                    if a.is_some() {
                        let _ = crate::cache::write(path, &d);
                    } else {
                        log::debug!("animation {id} failed to parse");
                    }
                    let _ = tx.send((id, a.map(|a| Arc::new(BoundAnim::bind(a, &rig))), false));
                    super::jobs::JobResult::Done
                });
            }
            Err(_) => {
                if let Some(e) = self.entries.get_mut(&id) {
                    e.state = 4;
                    e.failures += 1;
                    e.retry_at = Some(Instant::now() + Duration::from_secs(if r.status == 404 { 600 } else { 10 }));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_parity_animation_translations_replace_mesh_joint_positions_including_pelvis() {
        let rig = rig();
        let hip = rig.names["mHipLeft"];
        let mut base = SkeletonBase {
            local_pos: rig.local_pos.clone(),
            scale: rig.scale.clone(),
        };
        base.local_pos[rig.pelvis] += Vec3::new(0.3, -0.2, 0.25);
        base.local_pos[hip] += Vec3::new(-0.2, 0.1, -0.07);
        let pelvis_position = Vec3::new(0.08, -0.03, -0.12);
        let hip_position = rig.local_pos[hip] + Vec3::new(0.03, 0.04, 0.05);
        let mut animation = swing(&rig).anim.clone();
        animation.joints = [("mPelvis", pelvis_position), ("mHipLeft", hip_position)]
            .into_iter()
            .map(|(name, position)| aurora_assets::JointMotion {
                joint_name: name.into(),
                priority: 4,
                rot_keys: Vec::new(),
                pos_keys: vec![aurora_assets::anim::PosKey { time: 0.0, position }],
            })
            .collect();
        let animation = Arc::new(BoundAnim::bind(animation, &rig));
        let now = Instant::now();
        let mut controller = Controller::for_control_avatar();
        let id = Uuid::from_u128(42);
        controller.sync(&[signal(id, 1, now)], now, |_| Some(animation.clone()));
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        controller.evaluate(&rig, now, Some(&base), &mut out);
        let pelvis = Mat4::from_cols_array_2d(&out[rig.pelvis]).w_axis.truncate();
        assert!(pelvis.abs_diff_eq(rig.local_pos[rig.pelvis] + pelvis_position, 1e-6));
        let hip = Mat4::from_cols_array_2d(&out[hip]).w_axis.truncate();
        assert!(hip.abs_diff_eq(pelvis + hip_position, 1e-6));
        controller.reset_pose(false);
        controller.sync(&[], now + Duration::from_secs(3), |_| None);
        controller.evaluate(&rig, now + Duration::from_secs(4), Some(&base), &mut out);
        let pelvis = Mat4::from_cols_array_2d(&out[rig.pelvis]).w_axis.truncate();
        assert!(pelvis.abs_diff_eq(base.local_pos[rig.pelvis], 1e-6));
    }

    fn synchronized_controller() -> Controller {
        Controller {
            synchronized: true,
            ..Default::default()
        }
    }

    #[test]
    fn resync_aligns_playing_animations_and_survives_unchanged_signals_and_sequences() {
        let rig = rig();
        let animation = swing(&rig);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        for (synchronized, control) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut controllers = Vec::new();
            for start in [0.0, 0.65] {
                let mut c = Controller {
                    synchronized,
                    control,
                    ..Default::default()
                };
                let playing = [signal(Uuid::from_u128(77), 1, at(start))];
                c.sync(&playing, at(start), |_| Some(animation.clone()));
                assert!((c.motions[0].local_time(at(1.0)) - (1.0 - start)).abs() < 1e-6);
                c.resync_animations(at(1.25));
                assert!(c.sync(&playing, at(1.5), |_| Some(animation.clone())).is_empty());
                assert_eq!(c.motions.len(), 1);
                assert!((c.motions[0].local_time(at(1.5)) - 0.25).abs() < 1e-6);
                assert_eq!(c.signals[&playing[0].id], playing[0], "receipt clocks are untouched");
                let next = PlayingAnimation {
                    sequence: 2,
                    sequence_start: at(1.6),
                    ..playing[0]
                };
                c.sync(&[next], at(1.75), |_| Some(animation.clone()));
                assert!((c.motions[0].local_time(at(1.75)) - 0.5).abs() < 1e-6);
                c.resync_animations(at(1.9));
                c.sync(&[next], at(2.0), |_| Some(animation.clone()));
                assert!((c.motions[0].local_time(at(2.0)) - 0.1).abs() < 1e-6);
                controllers.push(c);
            }
            let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
            let mut elbows = Vec::new();
            for c in &mut controllers {
                c.evaluate(&rig, at(2.0), None, &mut pose);
                elbows.push(c.rot[rig.names["mElbowLeft"]]);
            }
            assert!(elbows[0].abs_diff_eq(elbows[1], 1e-6), "avatars now draw the same dance phase");
        }
    }

    #[test]
    fn resync_restarts_a_one_shot_but_leaves_stopped_and_finished_motions_alone() {
        let rig = rig();
        let mut bound = BoundAnim::bind(swing(&rig).anim.clone(), &rig);
        bound.anim.looping = false;
        bound.anim.duration = 2.0;
        bound.anim.ease_in = 0.0;
        bound.anim.ease_out = 0.5;
        let animation = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let playing = [signal(Uuid::from_u128(78), 1, t0)];
        for synchronized in [false, true] {
            let mut c = Controller {
                synchronized,
                ..Default::default()
            };
            c.sync(&playing, t0, |_| Some(animation.clone()));
            c.resync_animations(at(1.0));
            assert!(c.sync(&playing, at(2.0), |_| Some(animation.clone())).is_empty());
            assert_eq!(c.sync(&playing, at(2.5), |_| Some(animation.clone())), vec![playing[0].id]);
            c.resync_animations(at(2.6));
            assert!(c.sync(&playing, at(3.0), |_| Some(animation.clone())).is_empty());
            assert!(c.motions.is_empty());
            c.resync_animations(at(3.25));
            c.sync(&playing, at(3.5), |_| Some(animation.clone()));
            assert!(c.motions.is_empty(), "completed signaled motions stay completed");
        }
    }

    #[test]
    fn resync_during_a_pause_resumes_from_the_expected_clock() {
        let rig = rig();
        let animation = swing(&rig);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let playing = [signal(Uuid::from_u128(79), 1, t0)];
        for synchronized in [false, true] {
            let mut c = Controller {
                synchronized,
                ..Default::default()
            };
            c.sync(&playing, t0, |_| Some(animation.clone()));
            c.set_paused(true, at(0.25));
            c.resync_animations(at(1.0));
            c.sync(&playing, at(1.5), |_| Some(animation.clone()));
            c.set_paused(false, at(1.75));
            c.sync(&playing, at(2.0), |_| Some(animation.clone()));
            let expected = if synchronized { 1.0 } else { 0.25 };
            assert!((c.motions[0].local_time(at(2.0)) - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn resync_keeps_an_explicit_preview_clock() {
        let rig = rig();
        let animation = swing(&rig);
        let t0 = Instant::now();
        let id = Uuid::from_u128(80);
        let playing = [signal(id, 1, t0)];
        let mut c = Controller::default();
        c.set_preview_clock(Some((Some(id), t0)));
        c.sync(&playing, t0, |_| Some(animation.clone()));
        c.resync_animations(t0 + Duration::from_secs(1));
        c.sync(&playing, t0 + Duration::from_millis(1250), |_| Some(animation.clone()));
        assert!((c.motions[0].local_time(t0 + Duration::from_millis(1250)) - 1.25).abs() < 1e-6);
    }

    #[test]
    fn resync_scene_restarts_all_residents_and_control_avatars_at_one_instant() {
        let library = Arc::new(super::super::avatar::AvatarLibrary::load());
        let animation = swing(&library.rig);
        let mut scene = super::super::Scene::new(std::path::PathBuf::new(), library);
        let t0 = Instant::now();
        for i in 0..3 {
            let start = t0 + Duration::from_millis(i * 200);
            let mut controller = Controller {
                control: i == 2,
                ..Default::default()
            };
            controller.sync(&[signal(Uuid::from_u128(81), 1, start)], start, |_| Some(animation.clone()));
            scene.motions.insert(Uuid::from_u128(u128::from(i) + 1), controller);
        }
        let restart = t0 + Duration::from_secs(1);
        scene.resync_animations(restart);
        assert_eq!(scene.motions.len(), 3);
        for controller in scene.motions.values() {
            assert!((controller.motions[0].local_time(restart + Duration::from_millis(250)) - 0.25).abs() < 1e-6);
        }
    }

    /// Both skeleton kinds use Legacy timing until the preference opts in.
    /// A live switch joins receipt time; switching back preserves the current
    /// phase and suspends again at the next edit, without restarting the loop.
    #[test]
    fn animation_clock_mode_switches_live_for_avatars_and_animesh() {
        let rig = rig();
        let anim = swing(&rig);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let playing = [signal(Uuid::from_u128(77), 1, t0)];
        let elbow = rig.names["mElbowLeft"];
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let mut sample = |c: &mut Controller, now| {
            c.sync(&playing, now, |_| Some(anim.clone()));
            c.evaluate(&rig, now, None, &mut out);
            c.rot[elbow]
        };
        for control in [false, true] {
            let mut c = if control {
                Controller::for_control_avatar()
            } else {
                Controller::default()
            };
            let mut reference = Controller {
                control,
                ..synchronized_controller()
            };
            assert!(!c.synchronized);
            c.sync(&playing, at(0.0), |_| None);
            assert!(
                sample(&mut c, at(0.75)).abs_diff_eq(sample(&mut reference, at(0.0)), 1e-6),
                "Legacy starts at loading"
            );
            let held = sample(&mut c, at(1.0));
            c.set_paused(true, at(1.0));
            assert!(sample(&mut c, at(4.0)).abs_diff_eq(held, 1e-6));
            c.set_paused(false, at(4.5));
            assert!(sample(&mut c, at(4.5)).abs_diff_eq(held, 1e-6), "Legacy resumes the held pose");
            c.set_synchronized(true);
            assert!(
                sample(&mut c, at(4.5)).abs_diff_eq(sample(&mut reference, at(4.5)), 1e-6),
                "opt-in catches up immediately"
            );
            sample(&mut c, at(5.0));
            c.set_paused(true, at(5.0));
            c.set_paused(false, at(6.25));
            let phase = sample(&mut c, at(6.25));
            assert!(
                phase.abs_diff_eq(sample(&mut reference, at(6.25)), 1e-6),
                "synchronized edits keep receipt time"
            );
            c.set_synchronized(false);
            assert!(
                sample(&mut c, at(6.25)).abs_diff_eq(phase, 1e-6),
                "opting out keeps the current phase"
            );
            let held = sample(&mut c, at(7.0));
            c.set_paused(true, at(7.0));
            c.set_paused(false, at(9.0));
            assert!(sample(&mut c, at(9.0)).abs_diff_eq(held, 1e-6), "Legacy suspends again");
            assert_eq!(c.motions.len(), 1, "switching does not restart a loop");
            c.set_paused(true, at(10.0));
            c.set_synchronized(true);
            c.set_paused(false, at(12.25));
            assert!(
                sample(&mut c, at(12.25)).abs_diff_eq(sample(&mut reference, at(12.25)), 1e-6),
                "enabling during an edit rejoins on release"
            );
        }
    }

    #[test]
    fn the_look_at_line_leaves_from_between_the_eyes() {
        let close = |a: Option<Vec3>, b: Vec3| a.is_some_and(|a| a.distance(b) < 1e-5);
        let eyes = (Vec3::new(0.1, 0.036, 1.7), Vec3::new(0.1, -0.036, 1.7));
        let between = Vec3::new(0.1, 0.0, 1.7);
        // the Bento eyes where the system ones are: the system eyes
        assert!(close(eyes_between(Some(eyes), Some(eyes), None), between));
        // a mesh head moved its Bento eyes: those
        let moved = (Vec3::new(0.12, 0.03, 1.72), Vec3::new(0.12, -0.03, 1.72));
        assert!(close(eyes_between(Some(eyes), Some(moved), None), Vec3::new(0.12, 0.0, 1.72)));
        // no eyes: in front of the head joint, turned with it
        let head = (Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::new(0.0, 0.0, 1.6));
        assert!(close(eyes_between(None, None, Some(head)), Vec3::new(0.0, 0.098, 1.679)));
        assert_eq!(eyes_between(None, None, None), None);
    }

    fn signal(id: Uuid, sequence: i32, start: Instant) -> PlayingAnimation {
        PlayingAnimation {
            id,
            sequence,
            sequence_start: start,
            continuous_start: start,
        }
    }

    /// The rig at rest must give the SL joint world matrices (own scale
    /// included): rigged meshes' inverse bind matrices are built for them.
    #[test]
    fn rest_pose_matches_sl_joint_matrices() {
        let xml = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/character/avatar_skeleton.xml")).expect("skeleton");
        let sk = Skeleton::parse(&xml).expect("parse");
        let rig = Rig::new(&sk);
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        synchronized_controller().evaluate(&rig, Instant::now(), None, &mut out);
        for (j, joint) in sk.joints.iter().enumerate().take(rig.len()) {
            let m = Mat4::from_cols_array_2d(&out[j]);
            let d = (m - joint.world).to_cols_array().iter().fold(0.0f32, |a, v| a.max(v.abs()));
            assert!(d < 1e-4, "{}: {d}", joint.name);
        }
        // a collision volume keeps its own scale
        let cv = rig.names["CHEST"];
        let (s, _, _) = Mat4::from_cols_array_2d(&out[cv]).to_scale_rotation_translation();
        assert!((s - sk.joints[cv].scale).length() < 1e-4);
    }

    fn bend(rig: &Rig, angle: f32, ease: f32) -> Arc<BoundAnim> {
        let anim = Animation {
            base_priority: 2,
            duration: 1.0,
            emote_name: String::new(),
            loop_in: 0.0,
            loop_out: 1.0,
            looping: true,
            ease_in: ease,
            ease_out: ease,
            hand_pose: 0,
            joints: vec![aurora_assets::JointMotion {
                joint_name: "mElbowLeft".into(),
                priority: -1,
                rot_keys: vec![aurora_assets::anim::RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_z(angle),
                }],
                pos_keys: Vec::new(),
            }],
        };
        Arc::new(BoundAnim::bind(anim, rig))
    }

    /// Stand stops while walk is still loading: the elbow eases out, then
    /// keeps its last pose instead of going back to the T-pose; walk blends in
    /// once it arrives (LLMotionController, LLJointStateBlender).
    #[test]
    fn motions_ease_and_hold_the_pose() {
        let xml = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/character/avatar_skeleton.xml")).expect("skeleton");
        let rig = Rig::new(&Skeleton::parse(&xml).expect("parse"));
        let elbow = rig.names["mElbowLeft"];
        let (stand, walk) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let (a_stand, a_walk) = (bend(&rig, 0.5, 0.5), bend(&rig, -0.5, 0.5));
        let mut c = synchronized_controller();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        let mut angle = |c: &mut Controller, t: Instant| {
            c.evaluate(&rig, t, None, &mut out);
            c.rot[elbow].to_axis_angle().1 * c.rot[elbow].to_axis_angle().0.z.signum()
        };
        c.sync(&[signal(stand, 1, t0)], at(0.0), |_| Some(a_stand.clone()));
        c.sync(&[signal(stand, 1, t0)], at(1.0), |_| Some(a_stand.clone()));
        assert!((angle(&mut c, at(1.0)) - 0.5).abs() < 1e-3);
        // stand stopped, walk not loaded yet: still easing out at full pose
        c.sync(&[signal(walk, 2, at(2.0))], at(2.0), |id| (*id == stand).then(|| a_stand.clone()));
        c.sync(&[signal(walk, 2, at(2.0))], at(2.25), |id| (*id == stand).then(|| a_stand.clone()));
        assert!((angle(&mut c, at(2.25)) - 0.5).abs() < 1e-3);
        // ease out over: no motion left, the last pose is held
        c.sync(&[signal(walk, 2, at(2.0))], at(3.0), |_| None);
        assert!(c.motions.is_empty());
        assert!((angle(&mut c, at(3.0)) - 0.5).abs() < 1e-3);
        // walk arrives and is applied (alone on the joint: full rotation)
        c.sync(&[signal(walk, 2, at(2.0))], at(3.1), |_| Some(a_walk.clone()));
        c.sync(&[signal(walk, 2, at(2.0))], at(3.2), |_| Some(a_walk.clone()));
        assert!((angle(&mut c, at(3.2)) + 0.5).abs() < 1e-3);
        // a new stand eases in over walk (same priority, newer first)
        c.sync(&[signal(walk, 2, at(2.0)), signal(stand, 3, at(4.0))], at(4.0), |id| {
            Some(if *id == stand { a_stand.clone() } else { a_walk.clone() })
        });
        c.sync(&[signal(walk, 2, at(2.0)), signal(stand, 3, at(4.0))], at(4.25), |id| {
            Some(if *id == stand { a_stand.clone() } else { a_walk.clone() })
        });
        let mid = angle(&mut c, at(4.25));
        assert!(mid > -0.4 && mid < 0.4, "{mid}");
    }

    fn rig() -> Rig {
        let xml = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/character/avatar_skeleton.xml")).expect("skeleton");
        Rig::new(&Skeleton::parse(&xml).expect("parse"))
    }

    /// The elbow swings from 0 to 1 rad and back over 2 s, in a loop.
    fn swing(rig: &Rig) -> Arc<BoundAnim> {
        let key = |time: f32, angle: f32| aurora_assets::anim::RotKey {
            time,
            rotation: Quat::from_rotation_z(angle),
        };
        let anim = Animation {
            base_priority: 2,
            duration: 2.0,
            emote_name: String::new(),
            loop_in: 0.0,
            loop_out: 2.0,
            looping: true,
            ease_in: 0.0,
            ease_out: 0.0,
            hand_pose: 0,
            joints: vec![aurora_assets::JointMotion {
                joint_name: "mElbowLeft".into(),
                priority: -1,
                rot_keys: vec![key(0.0, 0.0), key(1.0, 1.0), key(2.0, 0.0)],
                pos_keys: Vec::new(),
            }],
        };
        Arc::new(BoundAnim::bind(anim, rig))
    }

    /// Editing freezes the pose, then both kinds of skeleton rejoin the
    /// uninterrupted reference, including after a long second edit.
    #[test]
    fn edited_avatars_and_animesh_rejoin_the_reference_phase_on_release() {
        let rig = rig();
        let elbow = rig.names["mElbowLeft"];
        let a = swing(&rig);
        let id = Uuid::from_u128(7);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        for control in [false, true] {
            let mut c = Controller {
                control,
                synchronized: true,
                ..Default::default()
            };
            let mut reference = Controller {
                control,
                synchronized: true,
                ..Default::default()
            };
            let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
            let mut angle = |c: &mut Controller, t: Instant| {
                c.sync(&[signal(id, 1, t0)], t, |_| Some(a.clone()));
                c.evaluate(&rig, t, None, &mut out);
                let (axis, angle) = c.rot[elbow].to_axis_angle();
                angle * axis.z.signum()
            };
            assert!(angle(&mut c, at(0.0)).abs() < 1e-3);
            assert!((angle(&mut c, at(0.5)) - 0.5).abs() < 1e-3);
            c.set_paused(true, at(0.5));
            for s in [0.75, 1.0, 2.5] {
                let held = angle(&mut c, at(s));
                assert!((held - 0.5).abs() < 1e-3, "{control}, {s}: {held}");
            }
            c.set_paused(false, at(3.0));
            for s in [3.0, 3.25] {
                assert!((angle(&mut c, at(s)) - angle(&mut reference, at(s))).abs() < 1e-6);
            }
            c.set_paused(true, at(3.25));
            assert!((angle(&mut c, at(607.0)) - 0.75).abs() < 1e-3);
            c.set_paused(false, at(607.9));
            assert!((angle(&mut c, at(607.9)) - angle(&mut reference, at(607.9))).abs() < 1e-6);
            assert_eq!(c.motions[0].continuous_activation, t0);
        }
    }

    /// Changed signals and a newly loaded asset cannot move the edited pose.
    /// Their playback origins still belong to when they were first known.
    #[test]
    fn signals_changed_during_edit_keep_the_pose_frozen_and_rejoin_on_release() {
        let rig = rig();
        let a = swing(&rig);
        let (old, new) = (Uuid::from_u128(8), Uuid::from_u128(9));
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        let mut c = synchronized_controller();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        c.sync(&[signal(old, 1, t0)], at(0.0), |_| Some(a.clone()));
        c.sync(&[signal(old, 1, t0)], at(0.5), |_| Some(a.clone()));
        c.evaluate(&rig, at(0.5), None, &mut out);
        let held = out.clone();
        c.set_paused(true, at(0.5));
        c.sync(&[signal(new, 2, at(1.0))], at(1.0), |_| None);
        c.evaluate(&rig, at(1.0), None, &mut out);
        assert_eq!(out, held);
        c.sync(&[signal(new, 2, at(1.0))], at(2.0), |_| Some(a.clone()));
        c.evaluate(&rig, at(2.0), None, &mut out);
        assert_eq!(out, held);
        assert_eq!(c.motions.iter().map(|m| m.id).collect::<Vec<_>>(), [new]);
        c.set_paused(false, at(4.0));
        let m = &c.motions[0];
        assert_eq!(m.activation, at(1.0));
        assert!((m.local_time(at(4.5)) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn name_tag_height_clears_a_deformer_without_following_animated_head_or_bob() {
        let rig = rig();
        let animation = Arc::new(BoundAnim::bind(crate::demo::tags::height_animation(&rig), &rig));
        let epoch = Instant::now();
        let playing = [signal(Uuid::from_u128(261), 1, epoch)];
        let mut controller = synchronized_controller();
        let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        controller.evaluate(&rig, epoch, None, &mut pose);
        let normal = controller.name_tag_height.expect("rest height");
        controller.sync(&playing, epoch, |_| Some(animation.clone()));
        let mut heads = Vec::new();
        let mut heights = Vec::new();
        for millis in [500, 1500, 2500, 3500, 4500] {
            let now = epoch + Duration::from_millis(millis);
            controller.sync(&playing, now, |_| Some(animation.clone()));
            controller.evaluate(&rig, now, None, &mut pose);
            heads.push(controller.head_pos().expect("head"));
            heights.push(controller.name_tag_height.expect("height"));
        }
        assert!(heights[0] > normal + 0.7, "deformer raises the tag: {normal} -> {heights:?}");
        assert!(heads.windows(2).any(|h| h[0].distance(h[1]) > 0.05), "the head really moves");
        assert!(
            heights.iter().all(|h| (h - heights[0]).abs() < 1e-5),
            "tag height must stay still: {heights:?}"
        );
        for head in heads {
            assert!(
                head.z - rig.local_pos[rig.pelvis].z < heights[0],
                "head stays below the upright envelope"
            );
        }
    }

    /// The eyes' yaw in the head's frame (radians, left positive).
    fn eye_yaw(rig: &Rig, pose: &[[[f32; 4]; 4]]) -> f32 {
        let rot = |name: &str| Mat4::from_cols_array_2d(&pose[rig.names[name]]).to_scale_rotation_translation().1;
        let f = (rot("mHead").inverse() * rot("mEyeLeft")) * Vec3::X;
        f.y.atan2(f.x)
    }

    /// A cleared look-at ("none") stops the head and leaves the eyes on
    /// their last point, as the Legacy Viewer does for any avatar; looking
    /// ahead (no target even for us) brings the eyes straight again at
    /// once, at the same instant, so with the same random eye movement.
    #[test]
    fn eyes_keep_a_cleared_target_and_come_back_ahead() {
        let rig = rig();
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        let mut controller = synchronized_controller();
        let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        // looking back over the left shoulder, beyond what the head turns:
        // the eyes at their limit (EYE_ROT_LIMIT_ANGLE), eased in
        controller.look = Some(Vec3::new(-1.0, 1.5, 0.0));
        controller.evaluate(&rig, t0, None, &mut pose);
        controller.evaluate(&rig, at(1.0), None, &mut pose);
        // the random look-away may switch on the frame after a jitter: let
        // it, so the next frames keep the same random offset
        controller.evaluate(&rig, at(1.005), None, &mut pose);
        let left = eye_yaw(&rig, &pose);
        // cleared: the eyes stay where they were
        controller.look_cleared = true;
        controller.evaluate(&rig, at(1.01), None, &mut pose);
        assert!((eye_yaw(&rig, &pose) - left).abs() < 0.02, "the eyes keep their point");
        // ahead: the eyes turn back by the whole look (EYE_ROT_LIMIT_ANGLE)
        controller.look_cleared = false;
        controller.look = None;
        controller.evaluate(&rig, at(1.02), None, &mut pose);
        let ahead = eye_yaw(&rig, &pose);
        assert!(left - ahead > 0.4, "eyes back ahead: {left} -> {ahead}");
    }

    #[test]
    fn name_tag_height_updates_with_shape_and_mesh_offsets_but_not_look_at() {
        let rig = rig();
        let epoch = Instant::now();
        let mut controller = synchronized_controller();
        let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        controller.evaluate(&rig, epoch, None, &mut pose);
        let normal = controller.name_tag_height.expect("height");
        let mut base = SkeletonBase {
            local_pos: rig.local_pos.clone(),
            scale: rig.scale.clone(),
        };
        base.local_pos[rig.names["mTorso"]].z += 0.8;
        base.scale[rig.names["mHead"]] *= 1.5;
        controller.evaluate(&rig, epoch, Some(&base), &mut pose);
        let tall = controller.name_tag_height.expect("height");
        assert!(tall > normal + 0.8);
        for look in [Vec3::X, Vec3::Y, -Vec3::Z] {
            controller.look = Some(look);
            controller.evaluate(&rig, epoch + Duration::from_secs(2), Some(&base), &mut pose);
            assert_eq!(controller.name_tag_height, Some(tall));
        }
        base.local_pos[rig.names["mSpine3"]].z += 0.6;
        controller.evaluate(&rig, epoch + Duration::from_secs(3), Some(&base), &mut pose);
        assert!((controller.name_tag_height.expect("Bento height") - tall - 0.6).abs() < 1e-5);
        controller.evaluate(&rig, epoch + Duration::from_secs(3), None, &mut pose);
        assert_eq!(
            controller.name_tag_height,
            Some(normal),
            "removing the structural deformation restores height"
        );
    }

    /// A stopped animation leaves its joint positions held (the deformed
    /// avatar of #272); "Reset skeleton" brings the rest skeleton back,
    /// "and animations" also drops the motions still easing out.
    #[test]
    fn reset_skeleton_drops_the_pose_a_stopped_animation_left() {
        let rig = rig();
        let mut deformer = crate::demo::tags::height_animation(&rig);
        deformer.ease_in = 0.0;
        deformer.ease_out = 0.5;
        let deformer = Arc::new(BoundAnim::bind(deformer, &rig));
        let torso = rig.names["mTorso"];
        let epoch = Instant::now();
        let at = |s: f32| epoch + Duration::from_secs_f32(s);
        let id = Uuid::from_u128(272);
        let mut c = synchronized_controller();
        let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let torso_z = |c: &mut Controller, t: Instant, pose: &mut Vec<[[f32; 4]; 4]>| {
            c.evaluate(&rig, t, None, pose);
            Mat4::from_cols_array_2d(&pose[torso]).w_axis.z
        };
        let rest = torso_z(&mut c, at(0.0), &mut pose);
        c.sync(&[signal(id, 1, epoch)], at(0.0), |_| Some(deformer.clone()));
        c.sync(&[signal(id, 1, epoch)], at(1.0), |_| Some(deformer.clone()));
        assert!(torso_z(&mut c, at(1.0), &mut pose) > rest + 0.7);
        // stopped, eased out: the stretched torso stays
        c.sync(&[], at(1.0), |_| None);
        c.sync(&[], at(2.0), |_| None);
        assert!(c.motions.is_empty());
        assert!(torso_z(&mut c, at(2.0), &mut pose) > rest + 0.7, "the pose is held");
        c.reset_pose(false);
        assert!((torso_z(&mut c, at(2.1), &mut pose) - rest).abs() < 1e-4);
        // with the animations: a motion easing out goes, the playing one stays
        let other = Uuid::from_u128(273);
        c.sync(&[signal(id, 2, at(3.0)), signal(other, 1, at(3.0))], at(3.0), |_| {
            Some(deformer.clone())
        });
        c.sync(&[signal(other, 1, at(3.0))], at(3.1), |_| Some(deformer.clone()));
        assert_eq!(c.motions.len(), 2);
        c.reset_pose(true);
        assert_eq!(c.motions.iter().map(|m| m.id).collect::<Vec<_>>(), vec![other]);
    }

    #[test]
    fn name_tag_height_respects_position_priority_and_a_reset_animation() {
        let rig = rig();
        let mut deformer = crate::demo::tags::height_animation(&rig);
        deformer.ease_in = 0.0;
        deformer.joints.retain(|j| j.joint_name == "mTorso");
        let low = Arc::new(BoundAnim::bind(deformer.clone(), &rig));
        deformer.joints[0].priority = 6;
        deformer.joints[0]
            .pos_keys
            .iter_mut()
            .for_each(|k| k.position = rig.local_pos[rig.names["mTorso"]]);
        let high = Arc::new(BoundAnim::bind(deformer, &rig));
        let epoch = Instant::now();
        let (lo, hi) = (Uuid::from_u128(261), Uuid::from_u128(262));
        let mut controller = synchronized_controller();
        let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        controller.evaluate(&rig, epoch, None, &mut pose);
        let normal = controller.name_tag_height;
        controller.sync(&[signal(lo, 1, epoch)], epoch, |_| Some(low.clone()));
        controller.evaluate(&rig, epoch, None, &mut pose);
        assert!(controller.name_tag_height > normal);
        controller.sync(&[signal(lo, 1, epoch), signal(hi, 1, epoch)], epoch, |id| {
            Some(if *id == hi { high.clone() } else { low.clone() })
        });
        controller.evaluate(&rig, epoch, None, &mut pose);
        assert_eq!(
            controller.name_tag_height, normal,
            "a winning reset must suppress the lower deformer envelope"
        );
    }

    #[test]
    fn preview_seek_samples_immediately_and_never_sends_a_simulator_stop() {
        let rig = rig();
        let id = Uuid::from_u128(501);
        let mut animation = bend(&rig, 0.0, 0.0).anim.clone();
        animation.looping = false;
        animation.joints[0].rot_keys = vec![
            aurora_assets::anim::RotKey {
                time: 0.0,
                rotation: Quat::IDENTITY,
            },
            aurora_assets::anim::RotKey {
                time: 1.0,
                rotation: Quat::from_rotation_z(1.6),
            },
        ];
        let bound = Arc::new(BoundAnim::bind(animation, &rig));
        let epoch = Instant::now();
        let playing = [signal(id, 0, epoch)];
        let mut controller = Controller::for_control_avatar();
        let mut pose = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let check = |pose: &[[[f32; 4]; 4]], angle: f32| {
            let mut rotations = rig.local_rot.clone();
            let joint = rig.names["mElbowLeft"];
            rotations[joint] = Quat::from_rotation_z(angle);
            let expected = rig.world(&rotations, Vec3::ZERO)[joint];
            let got = Mat4::from_cols_array_2d(&pose[joint]);
            assert!((got - expected).to_cols_array().iter().all(|v| v.abs() < 0.001));
        };
        controller.set_preview_clock(Some((Some(id), epoch)));
        let sample = epoch + Duration::from_millis(500);
        assert!(controller.sync(&playing, sample, |_| Some(bound.clone())).is_empty());
        controller.evaluate(&rig, sample, None, &mut pose);
        check(&pose, 0.8);
        let held = pose.clone();
        controller.sync(&playing, sample, |_| Some(bound.clone()));
        controller.evaluate(&rig, sample, None, &mut pose);
        assert_eq!(held, pose);
        let new_epoch = epoch + Duration::from_secs(3);
        controller.set_preview_clock(Some((Some(id), new_epoch)));
        let sample = new_epoch + Duration::from_millis(250);
        controller.sync(&playing, sample, |_| Some(bound.clone()));
        controller.evaluate(&rig, sample, None, &mut pose);
        check(&pose, 0.4);
        assert!(
            controller
                .sync(&playing, new_epoch + Duration::from_secs(2), |_| Some(bound.clone()))
                .is_empty()
        );
    }

    #[test]
    fn new_sequence_preserves_loop_pose_clock_and_ease_in() {
        let rig = rig();
        let id = Uuid::from_u128(17);
        let anim = bend(&rig, 0.7, 2.0);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let mut c = synchronized_controller();
        let first = signal(id, 1, t0);
        c.sync(&[first], t0, |_| Some(anim.clone()));
        let second = PlayingAnimation {
            sequence: 2,
            sequence_start: at(1.25),
            ..first
        };
        c.sync(&[second], at(1.25), |_| Some(anim.clone()));
        assert_eq!(c.motions.len(), 1);
        assert_eq!(c.motions[0].activation, at(1.25));
        assert_eq!(c.motions[0].continuous_activation, t0);
        assert!((c.motions[0].local_time(at(1.25)) - 0.25).abs() < 1e-12);
        assert!((c.motions[0].weight - 0.68359375).abs() < 1e-6);
        for s in [2.0, 3.01, 20.0] {
            c.sync(&[second], at(s), |_| Some(anim.clone()));
            assert_eq!(c.motions[0].weight, 1.0);
        }
    }

    #[test]
    fn observed_stop_restart_resets_loop_even_between_rendered_frames() {
        let rig = rig();
        let id = Uuid::from_u128(18);
        let anim = bend(&rig, 0.7, 0.5);
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(3);
        let mut c = synchronized_controller();
        c.sync(&[signal(id, 1, t0)], t0, |_| Some(anim.clone()));
        // The world observed an empty list then the same UUID and sequence.
        c.sync(&[signal(id, 1, later)], later, |_| None);
        let loaded = later + Duration::from_millis(250);
        c.sync(&[signal(id, 1, later)], loaded, |_| Some(anim.clone()));
        let active = c.motions.iter().find(|m| m.stop.is_none()).expect("restart");
        assert_eq!(active.continuous_activation, later);
        assert_eq!(active.local_time(loaded), 0.25);
        assert_eq!(active.weight, 0.5);
        assert!(c.motions.iter().any(|m| m.stop == Some(later)));
    }

    #[test]
    fn one_shot_completes_once_and_new_sequence_explicitly_restarts_it() {
        let rig = rig();
        let id = Uuid::from_u128(19);
        let mut bound = BoundAnim::bind(bend(&rig, 0.7, 0.2).anim.clone(), &rig);
        bound.anim.looping = false;
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let first = signal(id, 1, t0);
        let mut c = synchronized_controller();
        assert!(c.sync(&[first], t0, |_| Some(anim.clone())).is_empty());
        assert_eq!(c.sync(&[first], at(2.0), |_| Some(anim.clone())), vec![id]);
        assert!(c.motions.is_empty());
        assert!(c.sync(&[first], at(3.0), |_| Some(anim.clone())).is_empty());
        assert!(c.motions.is_empty());
        let second = PlayingAnimation {
            sequence: 2,
            sequence_start: at(3.0),
            ..first
        };
        c.sync(&[second], at(3.0), |_| Some(anim.clone()));
        assert_eq!(c.motions.len(), 1);
        assert_eq!(c.motions[0].activation, at(3.0));
        c.sync(
            &[PlayingAnimation {
                sequence: 3,
                sequence_start: at(3.1),
                ..first
            }],
            at(3.1),
            |_| Some(anim.clone()),
        );
        assert_eq!(c.motions.len(), 2);
        assert_eq!(c.motions[0].activation, at(3.1));
        assert_eq!(c.motions[1].stop, Some(at(3.1)));
        assert_eq!(c.sync(&[c.signals[&id]], at(4.0), |_| Some(anim.clone())), vec![id]);
    }

    #[test]
    fn pre_jump_plays_through_overlapping_eases_before_notifying_the_simulator() {
        let rig = rig();
        let id = uuid::uuid!("7a4e87fe-de39-6fcb-6223-024b00893244");
        let mut animation = bend(&rig, 0.7, 0.0).anim.clone();
        // The stock pre-jump asset: its scheduled ease-out (0.133333 s)
        // falls inside ease-in. LLKeyframeMotion::onUpdate finishes it at
        // its authored duration instead (indra/llcharacter).
        animation.looping = false;
        animation.duration = 0.333333;
        animation.ease_in = 0.3;
        animation.ease_out = 0.2;
        let anim = Arc::new(BoundAnim::bind(animation, &rig));
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        for synchronized in [false, true] {
            let mut controller = Controller::default();
            controller.set_synchronized(synchronized);
            let playing = [signal(id, 1, t0)];
            let mut agent = crate::agent::AgentState::default();
            agent.record_jump_input(t0);
            for time in [0.0, 0.14, 0.25, 0.31, 0.333] {
                assert!(
                    controller.sync(&playing, at(time), |_| Some(anim.clone())).is_empty(),
                    "early stop at {time}"
                );
                assert!(controller.motions[0].stop.is_none());
                if time > 0.0 {
                    assert!(controller.motions[0].weight > 0.0, "the crouch must remain visible");
                }
            }
            assert_eq!(controller.motions[0].weight, 1.0);
            assert_eq!(controller.sync(&playing, at(0.334), |_| Some(anim.clone())), vec![id]);
            // Both a released tap and a held jump stop the stock animation
            // once; only the released tap additionally needs FINISH_ANIM.
            assert_eq!(agent.animation_stop_flags(id, false, at(0.334)), aurora_net::control::FINISH_ANIM);
            assert_eq!(agent.animation_stop_flags(id, true, at(0.334)), 0);
            assert!(controller.motions[0].weight > 0.9, "ease-out starts from the completed crouch");
            assert!(controller.sync(&playing, at(0.4), |_| Some(anim.clone())).is_empty());
            assert!(controller.motions[0].weight > 0.0);
            assert!(controller.sync(&playing, at(0.54), |_| Some(anim.clone())).is_empty());
            assert!(controller.motions.is_empty());
            assert!(controller.sync(&playing, at(1.0), |_| Some(anim.clone())).is_empty());
        }
    }

    #[test]
    fn one_shot_without_overlapping_eases_notifies_at_the_start_of_ease_out() {
        let rig = rig();
        let id = Uuid::from_u128(502);
        let mut animation = bend(&rig, 0.7, 0.2).anim.clone();
        animation.looping = false;
        let anim = Arc::new(BoundAnim::bind(animation, &rig));
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        for synchronized in [false, true] {
            let mut controller = Controller::default();
            controller.set_synchronized(synchronized);
            let playing = [signal(id, 1, t0)];
            assert!(controller.sync(&playing, t0, |_| Some(anim.clone())).is_empty());
            assert!(controller.sync(&playing, at(0.79), |_| Some(anim.clone())).is_empty());
            assert_eq!(controller.sync(&playing, at(0.81), |_| Some(anim.clone())), vec![id]);
            assert!(controller.motions[0].weight > 0.9);
            assert!(controller.sync(&playing, at(0.9), |_| Some(anim.clone())).is_empty());
            assert!(controller.sync(&playing, at(1.01), |_| Some(anim.clone())).is_empty());
            assert!(controller.motions.is_empty());
        }
    }

    #[test]
    fn pre_jump_catches_up_after_asset_load_and_notifies_simulator_once() {
        let rig = rig();
        let id = uuid::uuid!("7a4e87fe-de39-6fcb-6223-024b00893244");
        let mut bound = BoundAnim::bind(bend(&rig, 0.7, 0.25).anim.clone(), &rig);
        bound.anim.looping = false;
        bound.anim.duration = 0.333333;
        bound.anim.ease_in = 0.3;
        bound.anim.ease_out = 0.2;
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let playing = [signal(id, 1, t0)];
        let mut c = synchronized_controller();
        let mut agent = crate::agent::AgentState::default();
        agent.record_jump_input(t0);
        // A late asset catches up, but its completion still reaches the
        // simulator exactly once so a delayed pre-jump cannot strand us.
        assert!(c.sync(&playing, t0, |_| None).is_empty());
        let stopped = c.sync(&playing, at(2.0), |_| Some(anim.clone()));
        assert_eq!(stopped, vec![id]);
        assert_eq!(
            agent.animation_stop_flags(stopped[0], false, at(2.0)),
            aurora_net::control::FINISH_ANIM
        );
        assert!(c.motions.is_empty(), "the expired pose must not flash on screen");
        assert!(c.sync(&playing, at(2.1), |_| Some(anim.clone())).is_empty());
        assert!(c.sync(&playing, at(3.0), |_| Some(anim.clone())).is_empty());
        assert!(c.motions.is_empty());
    }

    #[test]
    fn server_stop_and_loop_do_not_request_timed_stops() {
        let rig = rig();
        let (once, looped) = (Uuid::from_u128(20), Uuid::from_u128(21));
        let mut bound = BoundAnim::bind(bend(&rig, 0.7, 0.25).anim.clone(), &rig);
        bound.anim.looping = false;
        let once_anim = Arc::new(bound);
        let loop_anim = bend(&rig, -0.7, 0.25);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let loop_signal = signal(looped, 1, t0);
        let mut c = synchronized_controller();
        let get = |id: &Uuid| Some(if *id == once { once_anim.clone() } else { loop_anim.clone() });
        assert!(c.sync(&[signal(once, 1, t0), loop_signal], t0, get).is_empty());
        assert!(c.sync(&[loop_signal], at(0.5), get).is_empty());
        assert!(c.sync(&[loop_signal], at(20.0), get).is_empty());
        assert_eq!(c.motions.len(), 1);
        assert_eq!(c.motions[0].id, looped);
    }

    #[test]
    fn stop_leaves_current_loop_phase_and_preserves_partial_ease_in_weight() {
        let rig = rig();
        let id = Uuid::from_u128(20);
        let mut bound = BoundAnim::bind(bend(&rig, 0.7, 4.0).anim.clone(), &rig);
        bound.anim.duration = 3.0;
        bound.anim.loop_in = 0.5;
        bound.anim.loop_out = 1.5;
        bound.anim.ease_out = 2.0;
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let mut c = synchronized_controller();
        c.sync(&[signal(id, 1, t0)], t0, |_| Some(anim.clone()));
        c.sync(&[], at(2.25), |_| Some(anim.clone()));
        let residual = aurora_assets::anim::cubic_step(2.25 / 4.0);
        assert!((c.motions[0].weight - residual).abs() < 1e-6);
        assert_eq!(c.motions[0].local_time(at(2.25)), 1.25);
        c.sync(&[], at(2.75), |_| Some(anim.clone()));
        assert_eq!(c.motions[0].local_time(at(2.75)), 1.75);
        assert!((c.motions[0].weight - residual * 0.84375).abs() < 1e-6);
        assert_eq!(c.motions[0].local_time(at(4.0)), 3.0);
    }

    #[test]
    fn a_stopped_single_pose_loop_plays_its_authored_recovery_before_easing_out() {
        let rig = rig();
        let id = Uuid::from_u128(22);
        let mut bound = BoundAnim::bind(bend(&rig, 0.7, 0.1).anim.clone(), &rig);
        bound.anim.duration = 2.4;
        bound.anim.loop_in = 0.0;
        bound.anim.loop_out = 0.0;
        bound.anim.ease_out = 0.25;
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let mut c = Controller::default();
        c.sync(&[signal(id, 1, t0)], t0, |_| Some(anim.clone()));
        c.sync(&[signal(id, 1, t0)], at(3.0), |_| Some(anim.clone()));
        assert_eq!(c.motions[0].local_time(at(3.0)), 0.0);
        c.sync(&[], at(3.0), |_| Some(anim.clone()));
        c.sync(&[], at(4.5), |_| Some(anim.clone()));
        assert_eq!(c.motions[0].local_time(at(4.5)), 1.5);
        assert_eq!(
            c.motions[0].weight, 1.0,
            "the two-second recovery must not disappear after a quarter second"
        );
        c.sync(&[], at(5.3), |_| Some(anim.clone()));
        assert!(c.motions[0].weight > 0.0 && c.motions[0].weight < 1.0);
        c.sync(&[], at(5.401), |_| Some(anim.clone()));
        assert!(c.motions.is_empty());
    }

    #[test]
    fn evaluated_joint_palettes_are_continuous_across_repeated_loop_seams() {
        let rig = rig();
        let id = Uuid::from_u128(21);
        let elbow = rig.names["mElbowLeft"];
        let mut bound = BoundAnim::bind(bend(&rig, 0.0, 0.0).anim.clone(), &rig);
        let j = &mut bound.anim.joints[0];
        j.rot_keys = vec![
            aurora_assets::anim::RotKey {
                time: 1.0 / 15.0,
                rotation: Quat::from_rotation_z(0.7),
            },
            aurora_assets::anim::RotKey {
                time: 1.0,
                rotation: Quat::from_rotation_z(-0.7),
            },
        ];
        j.pos_keys = vec![
            aurora_assets::anim::PosKey {
                time: 1.0 / 15.0,
                position: rig.local_pos[elbow] + Vec3::X * 0.1,
            },
            aurora_assets::anim::PosKey {
                time: 1.0,
                position: rig.local_pos[elbow] - Vec3::X * 0.1,
            },
        ];
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let mut c = synchronized_controller();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let first = signal(id, 1, t0);
        c.sync(&[first], t0, |_| Some(anim.clone()));
        for cycle in 1..=100 {
            let before = at(f64::from(cycle) - 1e-6);
            c.sync(&[first], before, |_| Some(anim.clone()));
            c.evaluate(&rig, before, None, &mut out);
            let pose = out[elbow];
            let after = at(f64::from(cycle) + 1e-6);
            let next = PlayingAnimation {
                sequence: cycle + 1,
                sequence_start: after,
                ..first
            };
            c.sync(&[next], after, |_| Some(anim.clone()));
            c.evaluate(&rig, after, None, &mut out);
            let error = Mat4::from_cols_array_2d(&pose) - Mat4::from_cols_array_2d(&out[elbow]);
            assert!(error.to_cols_array().iter().all(|v| v.abs() < 5e-5), "cycle {cycle}: {error:?}");
        }
    }

    #[test]
    fn delayed_loading_and_first_visibility_join_the_same_loop_after_ten_minutes() {
        let rig = rig();
        let id = Uuid::from_u128(22);
        let mut bound = BoundAnim::bind(swing(&rig).anim.clone(), &rig);
        bound.anim.loop_in = 0.5;
        bound.anim.loop_out = 1.5;
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let loaded = t0 + Duration::from_millis(2250);
        let frame = t0 + Duration::from_millis(607250);
        let first = signal(id, 1, t0);
        let changed = PlayingAnimation {
            sequence: 2,
            sequence_start: t0 + Duration::from_secs(1),
            ..first
        };
        let mut a = synchronized_controller();
        let mut b = synchronized_controller();
        let mut unseen = synchronized_controller();
        let mut out_a = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let mut out_b = out_a.clone();
        let mut out_unseen = out_a.clone();
        a.sync(&[first], t0, |_| Some(anim.clone()));
        b.sync(&[first], t0, |_| None);
        b.sync(&[changed], loaded, |_| Some(anim.clone()));
        for c in [&mut a, &mut b, &mut unseen] {
            c.sync(&[changed], frame, |_| Some(anim.clone()));
            assert_eq!(c.motions[0].continuous_activation, t0);
            assert!((c.motions[0].local_time(frame) - 1.25).abs() < 1e-12);
        }
        a.evaluate(&rig, frame, None, &mut out_a);
        b.evaluate(&rig, frame, None, &mut out_b);
        unseen.evaluate(&rig, frame, None, &mut out_unseen);
        assert_eq!(out_a, out_b);
        assert_eq!(out_a, out_unseen);
    }

    #[test]
    fn loading_during_an_edit_preserves_receipt_origins_on_release() {
        let rig = rig();
        let anim = swing(&rig);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let old = signal(Uuid::from_u128(23), 1, t0);
        let new = signal(Uuid::from_u128(24), 1, at(1.0));
        let mut c = synchronized_controller();
        c.sync(&[old], t0, |_| None);
        c.set_paused(true, at(0.5));
        c.sync(&[old, new], at(1.0), |_| None);
        c.set_paused(false, at(4.0));
        c.sync(&[old, new], at(4.25), |_| Some(anim.clone()));
        let old = c.motions.iter().find(|m| m.id == old.id).expect("old motion");
        let new = c.motions.iter().find(|m| m.id == new.id).expect("new motion");
        assert_eq!(old.continuous_activation, t0);
        assert_eq!(new.continuous_activation, at(1.0));
        assert!((old.local_time(at(4.25)) - 0.25).abs() < 1e-12);
        assert!((new.local_time(at(4.25)) - 1.25).abs() < 1e-12);
    }

    #[test]
    fn an_expired_one_shot_completes_once_when_editing_is_released() {
        let rig = rig();
        let id = Uuid::from_u128(25);
        let mut bound = BoundAnim::bind(swing(&rig).anim.clone(), &rig);
        bound.anim.looping = false;
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let playing = [signal(id, 1, t0)];
        let mut c = synchronized_controller();
        c.sync(&playing, t0, |_| Some(anim.clone()));
        c.set_paused(true, at(0.5));
        assert!(c.sync(&playing, at(5.0), |_| Some(anim.clone())).is_empty());
        c.set_paused(false, at(5.25));
        assert_eq!(c.sync(&playing, at(5.25), |_| Some(anim.clone())), [id]);
        assert!(c.motions.is_empty());
        assert!(c.sync(&playing, at(6.0), |_| Some(anim.clone())).is_empty());
    }

    #[test]
    fn joint_priorities_use_separate_rotation_and_position_weight_budgets() {
        let rig = rig();
        let elbow = rig.names["mElbowLeft"];
        let mut high = BoundAnim::bind(bend(&rig, 0.7, 0.5).anim.clone(), &rig);
        high.anim.base_priority = 0;
        high.anim.joints[0].priority = 6;
        let high = Arc::new(high);
        let mut low = BoundAnim::bind(bend(&rig, -0.7, 0.0).anim.clone(), &rig);
        low.anim.base_priority = 6;
        low.anim.joints[0].priority = 0;
        let target = rig.local_pos[elbow] + Vec3::X;
        low.anim.joints[0].pos_keys.push(aurora_assets::anim::PosKey {
            time: 0.0,
            position: target,
        });
        let low = Arc::new(low);
        let t0 = Instant::now();
        let frame = t0 + Duration::from_millis(250);
        let (lo, hi) = (Uuid::from_u128(23), Uuid::from_u128(24));
        let mut c = synchronized_controller();
        let playing = [signal(hi, 1, t0), signal(lo, 1, t0)];
        c.sync(&playing, t0, |id| Some(if *id == hi { high.clone() } else { low.clone() }));
        c.sync(&playing, frame, |_| None);
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        c.evaluate(&rig, frame, None, &mut out);
        // High rotation consumes 0.5, lower rotation fills the other half.
        assert!((c.rot[elbow] * Vec3::X - Vec3::X).length() < 1e-5);
        // Rotation consumed none of the independent position budget.
        assert!((c.pos_delta[elbow] - Vec3::X).length() < 1e-5);
    }

    #[test]
    fn only_six_newest_joint_contributions_are_kept_at_equal_priority() {
        let rig = rig();
        let elbow = rig.names["mElbowLeft"];
        let mut oldest = BoundAnim::bind(bend(&rig, 0.7, 0.0).anim.clone(), &rig);
        oldest.anim.joints[0].pos_keys.push(aurora_assets::anim::PosKey {
            time: 0.0,
            position: rig.local_pos[elbow] + Vec3::X,
        });
        let oldest = Arc::new(oldest);
        let newer = bend(&rig, -0.7, 0.0);
        let t0 = Instant::now();
        let playing: Vec<_> = (1..=7).map(|id| signal(Uuid::from_u128(id), 1, t0)).collect();
        let mut c = synchronized_controller();
        c.sync(&playing, t0, |id| {
            Some(if id.as_u128() == 1 { oldest.clone() } else { newer.clone() })
        });
        c.sync(&playing, t0 + Duration::from_secs(1), |_| None);
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        c.evaluate(&rig, t0 + Duration::from_secs(1), None, &mut out);
        assert_eq!(c.pos_delta[elbow], Vec3::ZERO);
        assert!((c.rot[elbow] * Vec3::X - Quat::from_rotation_z(-0.7) * Vec3::X).length() < 1e-5);
    }

    /// A looping animation that names an expression and a hand pose, and
    /// turns `joint` (none: an animation without joints).
    fn carrier(rig: &Rig, emote: &str, hand_pose: u32, priority: i32, joint: Option<&str>) -> Arc<BoundAnim> {
        let joints = joint
            .map(|name| aurora_assets::JointMotion {
                joint_name: name.into(),
                priority: -1,
                rot_keys: vec![aurora_assets::anim::RotKey {
                    time: 0.0,
                    rotation: Quat::IDENTITY,
                }],
                pos_keys: Vec::new(),
            })
            .into_iter()
            .collect();
        let anim = Animation {
            base_priority: priority,
            duration: 1.0,
            emote_name: emote.into(),
            loop_in: 0.0,
            loop_out: 1.0,
            looping: true,
            ease_in: 0.0,
            ease_out: 0.0,
            hand_pose,
            joints,
        };
        Arc::new(BoundAnim::bind(anim, rig))
    }

    fn morph(c: &Controller, param: i32) -> f32 {
        c.live_morphs().iter().find(|l| l.0 == param).map_or(0.0, |l| l.1)
    }

    /// LLKeyframeMotion::onActivate starts the expression an animation
    /// names: its morph eases in for 0.3 s, holds, then eases out by 2.8 s,
    /// the closed mouth going the other way; the animation keeps looping
    /// without starting it again. Its hand pose blends in.
    #[test]
    fn an_animation_header_plays_its_expression_once_and_its_hand_pose() {
        let rig = rig();
        let mut streamer = AnimStreamer::new(PathBuf::new());
        default_motions::register(&mut streamer, &rig);
        let id = Uuid::from_u128(9);
        let anim = carrier(&rig, "Express_Smile", 3, 4, Some("mWristLeft"));
        let mut get = |m: &Uuid| if *m == id { Some(anim.clone()) } else { streamer.get(m) };
        let mut c = Controller::default();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let t0 = Instant::now();
        let mut step = |c: &mut Controller, s: f32| {
            let now = t0 + Duration::from_secs_f32(s);
            let stopped = c.sync(&[signal(id, 1, t0)], now, &mut get);
            assert!(stopped.is_empty(), "the expression never reaches the simulator");
            c.evaluate(&rig, now, None, &mut out);
        };
        step(&mut c, 0.0);
        step(&mut c, 0.15);
        let w = morph(&c, 318);
        assert!(w > 0.2 && w < 0.8, "{w}");
        assert_eq!(morph(&c, default_motions::CLOSED_MOUTH), -w);
        step(&mut c, 1.0);
        assert_eq!(morph(&c, 318), 1.0);
        // the fist (Hands_Fist, 103) is there; relaxed (101, baked) has gone
        assert_eq!(morph(&c, 103), 1.0);
        assert_eq!(morph(&c, 101), -1.0);
        step(&mut c, 2.1);
        let w = morph(&c, 318);
        assert!(w > 0.2 && w < 0.8, "{w}");
        step(&mut c, 3.0);
        step(&mut c, 4.5);
        assert_eq!(morph(&c, 318), 0.0);
        assert_eq!(morph(&c, default_motions::CLOSED_MOUTH), 0.0);
        assert_eq!(morph(&c, 103), 1.0);
    }

    /// A gesture's expression: the simulator signals its built-in UUID,
    /// played without any asset; a stop eases it out.
    #[test]
    fn a_signalled_expression_plays_without_an_asset() {
        let rig = rig();
        let mut streamer = AnimStreamer::new(PathBuf::new());
        default_motions::register(&mut streamer, &rig);
        let wink = default_motions::emote_named("express_wink_emote").expect("wink");
        assert!(streamer.asset_settled(&wink));
        let mut c = Controller::default();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        c.sync(&[signal(wink, 1, t0)], at(0.0), |m| streamer.get(m));
        c.sync(&[signal(wink, 1, t0)], at(0.5), |m| streamer.get(m));
        c.evaluate(&rig, at(0.5), None, &mut out);
        assert_eq!(morph(&c, 303), 1.0);
        // without animations the hands stay relaxed, as baked
        assert_eq!(morph(&c, 101), 0.0);
        assert_eq!(c.live_morphs().len(), 2);
        c.sync(&[], at(0.6), |m| streamer.get(m));
        c.sync(&[], at(1.3), |m| streamer.get(m));
        c.evaluate(&rig, at(1.3), None, &mut out);
        let w = morph(&c, 303);
        assert!(w > 0.2 && w < 0.8, "{w}");
    }

    /// The hand pose of the strongest animation wins; among equals the
    /// oldest one (the last updated in mActiveMotions), unless a newer one
    /// drives all its joints: then the older one is not updated (#661, an
    /// animation played over an AO stand of the same priority).
    #[test]
    fn the_strongest_animation_gives_the_hand_pose() {
        let rig = rig();
        let wrist = Some("mWristLeft");
        let clips = [
            carrier(&rig, "", 3, 2, wrist),
            carrier(&rig, "", 2, 4, wrist),
            carrier(&rig, "", 12, 4, wrist),
            carrier(&rig, "", 5, 4, Some("mHead")),
            carrier(&rig, "", 8, 6, None),
        ];
        let get = |m: &Uuid| (m.as_u128() as usize).checked_sub(1).and_then(|i| clips.get(i)).cloned();
        let id = |n: u128| Uuid::from_u128(n);
        let mut c = Controller::default();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        let mut play = |c: &mut Controller, playing: &[PlayingAnimation], from: f32| {
            for s in [from, from + 0.5] {
                c.sync(playing, at(s), get);
                c.evaluate(&rig, at(s), None, &mut out);
            }
        };
        let (fist, point) = (signal(id(1), 1, t0), signal(id(2), 2, t0));
        play(&mut c, &[fist, point], 0.0);
        assert_eq!((morph(&c, 102), morph(&c, 103)), (1.0, 0.0));
        // a newer animation of the same priority on the same joints takes it
        let peace = signal(id(3), 3, at(1.0));
        play(&mut c, &[fist, point, peace], 1.0);
        assert_eq!((morph(&c, 102), morph(&c, 791)), (0.0, 1.0));
        // one on another joint does not: both are updated, the older wins
        let other = signal(id(4), 4, at(2.0));
        play(&mut c, &[fist, point, peace, other], 2.0);
        assert_eq!((morph(&c, 791), morph(&c, 667)), (1.0, 0.0));
        // nor does one without joints once eased in, however strong
        let jointless = signal(id(5), 5, at(3.0));
        play(&mut c, &[fist, point, peace, other, jointless], 3.0);
        assert_eq!((morph(&c, 791), morph(&c, 670)), (1.0, 0.0));
    }

    /// LLBreatheMotionRot turns the chest when no stronger animation does;
    /// LLBodyNoiseMotion adds its sway to the torso, except when sitting.
    #[test]
    fn the_chest_breathes_and_the_torso_sways_unless_sitting() {
        let rig = rig();
        let chest = LookJoints::of(&rig).chest.expect("chest");
        let torso = rig.names["mTorso"];
        let mut c = Controller::default();
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let t0 = Instant::now();
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        c.evaluate(&rig, at(0.0), None, &mut out);
        let t = std::f32::consts::FRAC_PI_2;
        c.evaluate(&rig, at(t), None, &mut out);
        assert!((c.rot[chest].angle_between(Quat::IDENTITY) - 0.05).abs() < 1e-3);
        let sway = |c: &Controller| c.held_procedural.iter().find(|s| s.0 == torso && s.4).map(|s| (s.1, s.3));
        let (prio, q) = sway(&c).expect("body noise");
        assert_eq!(prio, HIGH_PRIORITY);
        assert!(q.angle_between(default_motions::body_noise(t)) < 1e-4);
        c.sitting = true;
        c.evaluate(&rig, at(t + 0.1), None, &mut out);
        assert!(sway(&c).is_none());
        // a chest animation of higher priority hides the breath
        let lean = Animation {
            base_priority: 3,
            duration: 1.0,
            emote_name: String::new(),
            loop_in: 0.0,
            loop_out: 1.0,
            looping: true,
            ease_in: 0.0,
            ease_out: 0.0,
            hand_pose: 1,
            joints: vec![aurora_assets::JointMotion {
                joint_name: "mChest".into(),
                priority: -1,
                rot_keys: vec![aurora_assets::anim::RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_x(0.3),
                }],
                pos_keys: Vec::new(),
            }],
        };
        let lean = Arc::new(BoundAnim::bind(lean, &rig));
        c.sync(&[signal(Uuid::from_u128(5), 1, t0)], at(3.0), |_| Some(lean.clone()));
        c.evaluate(&rig, at(3.0), None, &mut out);
        assert!(c.rot[chest].angle_between(Quat::from_rotation_x(0.3)) < 1e-4);
    }
}
