//! Skeletal animation: rig built from `avatar_skeleton.xml` (base + Bento
//! bones), animation asset streaming and per-avatar pose evaluation into
//! joint palettes (joint world matrices; meshes apply their inverse binds).

use super::jobs::Jobs;
use crate::world::PlayingAnimation;
use aurora_assets::{Animation, Skeleton};
use aurora_net::{FetchRequest, FetchResult, Fetcher};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FETCH_KIND_ANIM: u64 = 4 << 60;

/// Palette entries reserved per skeleton instance.
pub const PALETTE_JOINTS: usize = 192;

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
    pub pelvis: usize,
}

impl Rig {
    pub fn new(sk: &Skeleton) -> Rig {
        let n = sk.joints.len().min(PALETTE_JOINTS);
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
        // depth-first order
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut roots = Vec::new();
        for (i, parent) in parent.iter().enumerate().take(n) {
            match *parent {
                Some(p) => children[p].push(i),
                None => roots.push(i),
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
        let pelvis = names.get("mPelvis").copied().unwrap_or(0);
        let mut rig = Rig {
            parent,
            local_pos,
            local_rot,
            parent_scale,
            scale,
            default_world_inv: vec![Mat4::IDENTITY; n],
            order,
            names,
            pelvis,
        };
        let rot = rig.local_rot.clone();
        let world = rig.world(&rot, Vec3::ZERO);
        rig.default_world_inv = world.iter().map(|m| m.inverse()).collect();
        rig
    }

    pub fn len(&self) -> usize {
        self.parent.len()
    }

    /// World transforms (rotation + translation) for given local rotations.
    fn world(&self, rot: &[Quat], pelvis_offset: Vec3) -> Vec<Mat4> {
        self.world_with(rot, pelvis_offset, &self.local_pos, &self.scale)
    }

    /// Same with per-avatar joint positions and scales (shape, attachment
    /// overrides). A joint's position is scaled by its parent's scale; its
    /// own scale is part of its matrix only (LLXformMatrix).
    pub(super) fn world_with(&self, rot: &[Quat], pelvis_offset: Vec3, local_pos: &[Vec3], scale: &[Vec3]) -> Vec<Mat4> {
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
}

impl BoundAnim {
    pub fn bind(anim: Animation, rig: &Rig) -> BoundAnim {
        let joints = anim.joints.iter().map(|j| rig.names.get(&j.joint_name).copied()).collect();
        BoundAnim { anim, joints }
    }
}

// ------------------------------------------------------------- motion control
//
// Port of LLMotionController / LLPoseBlender (indra/llcharacter, Linden
// Research, originally LGPL 2.1): motions ease in and out (cubic), a motion still
// loading starts when it arrives, a stopped motion keeps playing while it
// eases out, and a joint no motion drives keeps its last pose (never back to
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
}

impl Motion {
    fn ease_in_weight(&self, now: Instant) -> f32 {
        let a = &self.anim.anim;
        let start = if a.looping { self.continuous_activation } else { self.activation };
        let elapsed = now.saturating_duration_since(start).as_secs_f64();
        if a.ease_in > 0.0 {
            aurora_assets::anim::cubic_step(elapsed / f64::from(a.ease_in))
        } else {
            1.0
        }
    }

    fn stop_at(&mut self, now: Instant) {
        if self.stop.is_none() {
            self.residual = self.ease_in_weight(now);
            self.stop = Some(now);
        }
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
    /// LLControlAvatar disables default head / eye motions.
    pub control: bool,
    /// Newest first (mActiveMotions, push_front).
    motions: Vec<Motion>,
    /// Signals already activated, including completed non-looping motions.
    /// A still-signaled one-shot must not restart automatically every frame.
    signals: HashMap<Uuid, PlayingAnimation>,
    /// Joint rotations of the last frame (None before the first one).
    rot: Vec<Quat>,
    /// Animated joint positions as offsets from the rest skeleton, held
    /// like the rotations (a shape change still moves them).
    pos_delta: Vec<Vec3>,
    pelvis_offset: Vec3,
    /// Look-at point (LookAtPoint): from the head to the target, in the
    /// avatar's root frame; None = look forward.
    pub look: Option<Vec3>,
    /// The look-at target was cleared: HEAD_ROT stops (eyes keep `look`).
    pub look_cleared: bool,
    procedural: Procedural,
    /// Joints used by the procedural motions, as posed last frame
    /// (skeleton space).
    posed: LookJoints<(Quat, Vec3)>,
    last_eval: Option<Instant>,
}

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
        let t = self.change.map(|c| now.duration_since(c).as_secs_f32()).unwrap_or(0.0) / ease;
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
    pub fn for_control_avatar() -> Self {
        Self {
            control: true,
            ..Default::default()
        }
    }
    /// Skeleton-space position of the head as posed last frame.
    pub fn head_pos(&self) -> Option<Vec3> {
        self.posed.head.map(|h| h.1)
    }

    /// Joint states of the head and eye motions for this frame:
    /// (joint, weight, local rotation).
    fn procedural(&mut self, j: &LookJoints<usize>, now: Instant) -> Vec<(usize, f32, Quat)> {
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
                out.push((t, head_w, pr.torso));
            }
            if let (Some(neck), Some(head), Some(chest)) = (j.neck, j.head, p.chest) {
                // what is left after the chest, half to the neck, half to the head
                let h = chest.0.inverse() * pr.last_head;
                let half = nlerp(NECK_LAG, Quat::IDENTITY, h);
                out.push((neck, head_w, half));
                out.push((head, head_w, half));
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
                out.push((l, eye_w, rot * jitter * vq));
                out.push((r, eye_w, rot * jitter * vq.inverse()));
            }
        }
        out
    }

    /// Follow the server's animation list (LLVOAvatar::processAnimationStateChanges).
    /// `get` gives an animation once it has loaded.
    /// Return timed stops once, at the start of ease-out, like
    /// LLMotionController::updateMotionsByType -> requestStopMotion.
    pub fn sync(&mut self, playing: &[PlayingAnimation], now: Instant, mut get: impl FnMut(&Uuid) -> Option<Arc<BoundAnim>>) -> Vec<Uuid> {
        let mut completed = Vec::new();
        // stop the motions the server no longer plays
        for m in self.motions.iter_mut() {
            if !playing
                .iter()
                .any(|p| p.id == m.id && p.continuous_start == m.signal.continuous_start)
            {
                m.stop_at(now);
            }
        }
        self.signals.retain(|id, _| playing.iter().any(|p| p.id == *id));
        // start the new ones, once loaded (updateLoadingMotions: activated
        // when they arrive). A motion easing out is left to finish and a
        // new instance starts (startMotion, deprecateMotionInstance).
        for signal in playing {
            if self.signals.get(&signal.id) == Some(signal) {
                continue;
            }
            if let Some(m) = self.motions.iter_mut().find(|m| m.id == signal.id && m.stop.is_none()) {
                if m.anim.anim.looping && m.signal.continuous_start == signal.continuous_start {
                    // LLVOAvatar signals sequence changes, but startMotion
                    // keeps an already active loop running continuously.
                    m.activation = now;
                    m.signal = *signal;
                    self.signals.insert(signal.id, *signal);
                    continue;
                }
                m.stop_at(now);
            }
            if let Some(anim) = get(&signal.id) {
                self.motions.insert(
                    0,
                    Motion {
                        id: signal.id,
                        anim,
                        activation: now,
                        continuous_activation: now,
                        signal: *signal,
                        stop: None,
                        residual: 0.0,
                        weight: 0.0,
                    },
                );
                self.signals.insert(signal.id, *signal);
            }
        }
        // weights (LLMotionController::updateMotionsByType)
        self.motions.retain_mut(|m| {
            let a = &m.anim.anim;
            // a non-looping motion stops itself before its end to ease out
            // (activateMotionInstance: mSendStopTimestamp)
            if m.stop.is_none() && !a.looping && a.duration > 0.0 {
                let end = m.activation + Duration::from_secs_f64((f64::from(a.duration) - f64::from(a.ease_out)).max(0.0));
                if now >= end {
                    m.stop_at(end);
                    completed.push(m.id);
                }
            }
            match m.stop {
                Some(stop) if now >= stop => {
                    let a = &m.anim.anim;
                    let out = now.duration_since(stop).as_secs_f64();
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
        let n = rig.len().min(out.len());
        if self.rot.len() != rig.len() {
            self.rot = rig.local_rot.clone();
            self.pos_delta = vec![Vec3::ZERO; rig.len()];
        }
        let rest_pos: &[Vec3] = match base {
            Some(b) => &b.local_pos,
            None => &rig.local_pos,
        };
        // joint states per joint: (priority, weight, rotation, position),
        // highest priority first, newer motions first among equals
        // (LLJointStateBlender::addJointState)
        type State = (i32, f32, Option<Quat>, Option<Vec3>);
        let mut states: Vec<Vec<State>> = vec![Vec::new(); n];
        for m in &self.motions {
            if m.weight <= 0.0 {
                continue;
            }
            let a = &m.anim.anim;
            let t = m.local_time(now) as f32;
            for (jm, j) in a.joints.iter().zip(m.anim.joints.iter()) {
                let Some(j) = *j else {
                    continue;
                };
                if j >= n {
                    continue;
                }
                let rot = jm.rotation_at_loop(t, a.loop_range());
                let pos = jm.position_at_loop(t, a.loop_range()).filter(|p| p.is_finite());
                if rot.is_none() && pos.is_none() {
                    continue;
                }
                let prio = if jm.priority >= 0 { jm.priority } else { a.base_priority };
                let s = &mut states[j];
                let at = s.iter().position(|o| prio > o.0).unwrap_or(s.len());
                if at < JOINT_STATES {
                    s.insert(at, (prio, m.weight, rot, pos));
                    s.truncate(JOINT_STATES);
                }
            }
        }
        // head and eye motions, started with the avatar (startDefaultMotions)
        let lj = LookJoints::of(rig);
        let procedural = if self.control { Vec::new() } else { self.procedural(&lj, now) };
        for (j, w, r) in procedural {
            if let Some(s) = states.get_mut(j) {
                let at = s.iter().position(|o| MEDIUM_PRIORITY > o.0).unwrap_or(s.len());
                if at < JOINT_STATES {
                    s.insert(at, (MEDIUM_PRIORITY, w, Some(r), None));
                    s.truncate(JOINT_STATES);
                }
            }
        }
        let mut local_pos: Vec<Vec3> = rest_pos.to_vec();
        for (j, s) in states.iter().enumerate() {
            let mut rot_sum = 0.0f32;
            let mut rot = self.rot[j];
            let mut pos_sum = 0.0f32;
            let mut pos = Vec3::ZERO;
            for &(_, w, r, p) in s {
                if let Some(r) = r {
                    if rot_sum > 0.0 {
                        let sum = (rot_sum + w).min(1.0);
                        rot = nlerp(rot_sum / sum, r, rot);
                        rot_sum = sum;
                    } else {
                        rot = r;
                        rot_sum = w;
                    }
                }
                if let Some(p) = p {
                    if pos_sum > 0.0 {
                        let sum = (pos_sum + w).min(1.0);
                        pos = p.lerp(pos, pos_sum / sum);
                        pos_sum = sum;
                    } else {
                        pos = p;
                        pos_sum = w;
                    }
                }
            }
            self.rot[j] = rot;
            if pos_sum > 0.0 {
                if j == rig.pelvis {
                    // the pelvis is animated relative to the avatar root
                    self.pelvis_offset = pos;
                } else if let Some(rest) = rest_pos.get(j) {
                    self.pos_delta[j] = pos - *rest;
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
        let world = rig.world_with(&self.rot, self.pelvis_offset, &local_pos, scale);
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
                    priority: 8e11,
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
        Controller::default().evaluate(&rig, Instant::now(), None, &mut out);
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
        let mut c = Controller::default();
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

    #[test]
    fn new_sequence_preserves_loop_pose_clock_and_ease_in() {
        let rig = rig();
        let id = Uuid::from_u128(17);
        let anim = bend(&rig, 0.7, 2.0);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let mut c = Controller::default();
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
        let mut c = Controller::default();
        c.sync(&[signal(id, 1, t0)], t0, |_| Some(anim.clone()));
        // The world observed an empty list then the same UUID and sequence.
        c.sync(&[signal(id, 1, later)], later, |_| Some(anim.clone()));
        let active = c.motions.iter().find(|m| m.stop.is_none()).expect("restart");
        assert_eq!(active.continuous_activation, later);
        assert_eq!(active.local_time(later), 0.0);
        assert_eq!(active.weight, 0.0);
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
        let mut c = Controller::default();
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
    fn pre_jump_notifies_simulator_at_ease_out_once_after_asset_loads() {
        let rig = rig();
        let id = uuid::uuid!("7a4e87fe-de39-6fcb-6223-024b00893244");
        let mut bound = BoundAnim::bind(bend(&rig, 0.7, 0.25).anim.clone(), &rig);
        bound.anim.looping = false;
        let anim = Arc::new(bound);
        let t0 = Instant::now();
        let at = |s| t0 + Duration::from_secs_f64(s);
        let playing = [signal(id, 1, t0)];
        let mut c = Controller::default();
        let mut agent = crate::agent::AgentState::default();
        agent.record_jump_input(t0);
        // Loading must not consume the completion; its clock starts on arrival.
        assert!(c.sync(&playing, t0, |_| None).is_empty());
        assert!(c.sync(&playing, at(2.0), |_| Some(anim.clone())).is_empty());
        assert!(c.sync(&playing, at(2.74), |_| Some(anim.clone())).is_empty());
        let stopped = c.sync(&playing, at(2.75), |_| Some(anim.clone()));
        assert_eq!(stopped, vec![id]);
        assert_eq!(
            agent.animation_stop_flags(stopped[0], false, at(2.75)),
            aurora_net::control::FINISH_ANIM
        );
        assert_eq!(c.motions.len(), 1, "notification precedes visual ease-out completion");
        assert!(c.sync(&playing, at(2.9), |_| Some(anim.clone())).is_empty());
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
        let mut c = Controller::default();
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
        let mut c = Controller::default();
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
        let mut c = Controller::default();
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
    fn loaded_motion_starts_on_arrival_and_shared_frame_time_gives_identical_palettes() {
        let rig = rig();
        let id = Uuid::from_u128(22);
        let anim = bend(&rig, 0.7, 0.5);
        let t0 = Instant::now();
        let loaded = t0 + Duration::from_secs(2);
        let frame = loaded + Duration::from_millis(250);
        let first = signal(id, 1, t0);
        let changed = PlayingAnimation {
            sequence: 2,
            sequence_start: frame,
            ..first
        };
        let mut a = Controller::default();
        let mut b = Controller::default();
        let mut out_a = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        let mut out_b = out_a.clone();
        for c in [&mut a, &mut b] {
            c.sync(&[first], t0, |_| None);
            c.sync(&[first], loaded, |_| Some(anim.clone()));
            c.sync(&[changed], frame, |_| Some(anim.clone()));
            assert_eq!(c.motions[0].continuous_activation, loaded);
            assert_eq!(c.motions[0].weight, 0.5);
        }
        a.evaluate(&rig, frame, None, &mut out_a);
        b.evaluate(&rig, frame, None, &mut out_b);
        assert_eq!(out_a, out_b);
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
        let mut c = Controller::default();
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
        let mut c = Controller::default();
        c.sync(&playing, t0, |id| {
            Some(if id.as_u128() == 1 { oldest.clone() } else { newer.clone() })
        });
        c.sync(&playing, t0 + Duration::from_secs(1), |_| None);
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        c.evaluate(&rig, t0 + Duration::from_secs(1), None, &mut out);
        assert_eq!(c.pos_delta[elbow], Vec3::ZERO);
        assert!((c.rot[elbow] * Vec3::X - Quat::from_rotation_z(-0.7) * Vec3::X).length() < 1e-5);
    }
}
