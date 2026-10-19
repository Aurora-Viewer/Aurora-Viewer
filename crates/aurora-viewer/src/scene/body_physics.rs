//! Avatar body physics: the bounce of the breasts, belly and butt of an
//! avatar that wears a physics layer. Port of LLPhysicsMotion /
//! LLPhysicsMotionController (indra/newview/llphysicsmotion.cpp, originally
//! LGPL 2.1).
//!
//! Each of the six motions is a one-dimensional spring in a normalised
//! parameter space [0, 1], pushed by the acceleration and the drag of its
//! reference joint (mChest or mPelvis) along one of that joint's axes, and by
//! gravity. Its position is the weight of hidden `*_Driven` parameters whose
//! volume morphs move the LEFT_PEC / RIGHT_PEC / BELLY / BUTT collision
//! volumes, which mesh bodies are rigged to. Nothing is sent to the
//! simulator: the physics parameters arrive with each avatar's appearance and
//! every viewer animates them locally.
//!
//! Deliberate differences from the Legacy Viewer, so that the bounce is the
//! same at any frame rate and never jumps:
//! - a fixed step of [`STEP`], the joints sampled between two frames and the
//!   result interpolated to the frame time, where the Legacy Viewer steps once
//!   per frame (in sub-steps of at most 0.05 s): its bounce changes with the
//!   frame rate. Its acceleration smoothing, a third of the new value per
//!   frame, becomes the same decay per 1/30 s;
//! - a motion starts where its spring holds it against gravity, not at the
//!   bottom of its range (no jolt when an avatar appears or puts on a
//!   physics layer);
//! - every frame shows the current position (no `min_delta` threshold: moving
//!   four collision volumes costs nothing);
//! - turned off, bodies return to their rest shape instead of freezing.

use super::anim::Rig;
use super::shape::{ShapeParams, VolumeMorph};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::time::Instant;

/// Fixed simulation step (s).
pub const STEP: f32 = 1.0 / 120.0;
/// Time unit of the joint velocities and accelerations (joint_local_factor).
const JOINT_LOCAL_FACTOR: f32 = 30.0;
/// Metres to centimetres (world_to_model_scale).
const WORLD_TO_MODEL: f32 = 100.0;
/// Part of the previous acceleration kept per 1/30 s (smoothing 3).
const ACCEL_KEEP: f32 = 2.0 / 3.0;
const MAX_VELOCITY: f32 = 100.0;
/// A longer gap between two updates restarts the joint tracking, as the
/// Legacy Viewer skips frames below 1 fps.
const MAX_GAP: f64 = 1.0;
/// A joint moving further between two frames was teleported (region
/// change, sit, teleport): tracking restarts instead of a huge acceleration.
const MAX_JUMP: f32 = 10.0;
/// Screen size below which the lowest detail stops an avatar's physics
/// (area_for_min_settings, in square-root pixels).
const AREA_FOR_MIN_SETTINGS: f32 = 1400.0;
/// Half-diagonal of a standing avatar's bounding box (m), for its screen size.
const AVATAR_HALF_SIZE: f32 = 1.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefJoint {
    Chest = 0,
    Pelvis = 1,
}

/// One motion of LLPhysicsMotionController::onInitialize: its reference
/// joint, direction in that joint's frame, controller parameter (its weight
/// is the rest position; it drives the hidden parameters) and behaviour
/// parameters (Mass, Gravity, Drag, MaxEffect, Spring, Gain, Damping).
struct MotionDef {
    joint: RefJoint,
    dir: Vec3,
    controller: i32,
    driven: &'static [i32],
    behavior: [i32; 7],
}

const MOTIONS: [MotionDef; 6] = [
    // Breast cleavage
    MotionDef {
        joint: RefJoint::Chest,
        dir: Vec3::new(-1.0, 0.0, 0.0),
        controller: 1101,
        driven: &[1201],
        behavior: [10000, 10001, 10002, 10007, 10008, 10009, 10010],
    },
    // Breast bounce
    MotionDef {
        joint: RefJoint::Chest,
        dir: Vec3::new(0.0, 0.0, 1.0),
        controller: 1100,
        driven: &[1200],
        behavior: [10000, 10001, 10002, 10003, 10004, 10005, 10006],
    },
    // Breast sway
    MotionDef {
        joint: RefJoint::Chest,
        dir: Vec3::new(0.0, -1.0, 0.0),
        controller: 1105,
        driven: &[1207],
        behavior: [10000, 10001, 10002, 10029, 10030, 10031, 10032],
    },
    // Butt bounce
    MotionDef {
        joint: RefJoint::Pelvis,
        dir: Vec3::new(0.0, 0.0, -1.0),
        controller: 1103,
        driven: &[1205],
        behavior: [10018, 10019, 10020, 10021, 10022, 10023, 10024],
    },
    // Butt left / right
    MotionDef {
        joint: RefJoint::Pelvis,
        dir: Vec3::new(0.0, -1.0, 0.0),
        controller: 1104,
        driven: &[1206],
        behavior: [10018, 10019, 10020, 10025, 10026, 10027, 10028],
    },
    // Belly bounce (legs and skirt morphs have no volume)
    MotionDef {
        joint: RefJoint::Pelvis,
        dir: Vec3::new(0.0, 0.0, -1.0),
        controller: 1102,
        driven: &[1202, 1203, 1204],
        behavior: [10011, 10012, 10013, 10014, 10015, 10016, 10017],
    },
];

/// Defaults of LLPhysicsMotion::sDefaultController, for a parameter the
/// appearance does not give.
const DEFAULT_BEHAVIOR: [f32; 7] = [0.2, 0.0, 0.15, 0.1, 0.1, 10.0, 0.05];

/// The avatar definition the motions need, resolved once.
pub struct PhysicsRig {
    joints: [usize; 2],
    motions: Vec<RigMotion>,
    /// Behaviour parameters' defaults, by id.
    defaults: HashMap<i32, f32>,
}

struct RigMotion {
    /// Rest position in [0, 1]: the controller's weight (never transmitted,
    /// so its default).
    user: f32,
    driven: Vec<VolumeMorph>,
    /// Every driven parameter, with or without a volume (the system body's
    /// vertex morphs): id, range, sexes it applies to.
    weights: Vec<(i32, f32, f32, bool, bool)>,
}

/// The hidden parameters the motions drive.
pub fn driven_ids() -> impl Iterator<Item = i32> {
    MOTIONS.iter().flat_map(|m| m.driven.iter().copied())
}

impl PhysicsRig {
    pub fn new(shape: &ShapeParams, rig: &Rig) -> Option<PhysicsRig> {
        let joint = |n: &str| rig.names.get(n).copied();
        let joints = [joint("mChest")?, joint("mPelvis")?];
        let mut defaults = HashMap::new();
        let motions = MOTIONS
            .iter()
            .map(|m| {
                for (k, id) in m.behavior.iter().enumerate() {
                    defaults.insert(*id, shape.range(*id).map_or(DEFAULT_BEHAVIOR[k], |r| r.2));
                }
                let user = shape
                    .range(m.controller)
                    .filter(|(min, max, _)| max > min)
                    .map_or(0.5, |(min, max, default)| (default - min) / (max - min));
                RigMotion {
                    user,
                    driven: m.driven.iter().filter_map(|id| shape.volume_morph(*id)).collect(),
                    weights: m
                        .driven
                        .iter()
                        .filter_map(|&id| {
                            let (min, max, _) = shape.range(id)?;
                            let (female, male) = shape.sexes(id)?;
                            Some((id, min, max, female, male))
                        })
                        .collect(),
                }
            })
            .collect();
        Some(PhysicsRig { joints, motions, defaults })
    }

    /// Reference joints (mChest, mPelvis).
    pub fn joints(&self) -> [usize; 2] {
        self.joints
    }
}

/// Behaviour of one motion (LLPhysicsMotion::getParamValue).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Behavior {
    pub mass: f32,
    pub gravity: f32,
    pub drag: f32,
    pub max_effect: f32,
    pub spring: f32,
    pub gain: f32,
    pub damping: f32,
}

/// The physics an avatar's appearance asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    motions: [Option<Behavior>; 6],
    male: bool,
}

impl Params {
    /// From an avatar's effective parameter weights. None when no motion
    /// moves anything for its sex: no physics layer worn, or every Max
    /// Effect at 0 (the avatar stays exactly as without physics).
    pub fn from_weights(rig: &PhysicsRig, weights: &HashMap<i32, f32>, male: bool) -> Option<Params> {
        let mut motions = [None; 6];
        let mut any = false;
        for ((def, motion), out) in MOTIONS.iter().zip(&rig.motions).zip(&mut motions) {
            let v: [f32; 7] = std::array::from_fn(|k| {
                let id = def.behavior[k];
                let w = weights
                    .get(&id)
                    .or_else(|| rig.defaults.get(&id))
                    .copied()
                    .unwrap_or(DEFAULT_BEHAVIOR[k]);
                if w.is_finite() { w } else { DEFAULT_BEHAVIOR[k] }
            });
            let b = Behavior {
                // (the parameter's minimum is 0.1; a division by it follows)
                mass: v[0].max(0.01),
                gravity: v[1],
                drag: v[2],
                max_effect: v[3].max(0.0),
                spring: v[4],
                gain: v[5],
                damping: v[6],
            };
            let moves = motion
                .driven
                .iter()
                .any(|d| (if male { d.male } else { d.female }) && !d.offsets.is_empty());
            if b.max_effect > 0.0 && moves {
                *out = Some(b);
                any = true;
            }
        }
        any.then_some(Params { motions, male })
    }

    #[cfg(test)]
    pub fn with(male: bool, motions: [Option<Behavior>; 6]) -> Params {
        Params { motions, male }
    }
}

/// Which avatars move (AvatarPhysics, RenderAvatarPhysicsLODFactor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    pub enabled: bool,
    /// 0 to 1: at 1 every avatar moves; below, avatars small on screen keep
    /// their last shape; 0 stops the physics.
    pub detail: f32,
    /// Screen pixels per radian (height / vertical field of view).
    pub pixel_angle: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            detail: 1.0,
            pixel_angle: 900.0,
        }
    }
}

impl Config {
    pub fn on(&self) -> bool {
        self.enabled && self.detail > 0.0
    }

    /// Whether an avatar `distance` m from the camera is updated: always our
    /// own; another one when its screen size is above the detail's limit
    /// (LLPhysicsMotion::onUpdate, with LLPipeline::calcPixelArea's size).
    pub fn updates(&self, own: bool, distance: f32) -> bool {
        if !self.on() {
            return false;
        }
        if own {
            return true;
        }
        // near objects count as further (calcPixelArea's ramp below 16 m)
        let d = distance.max(0.0);
        let d = if d < 16.0 { (d / 16.0).powi(2) * 16.0 } else { d };
        let size = (AVATAR_HALF_SIZE / d.max(1e-3)).atan() * self.pixel_angle * std::f32::consts::PI.sqrt();
        size > AREA_FOR_MIN_SETTINGS * (1.0 - self.detail.min(1.0))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Motion {
    /// Normalised position, unclamped as the Legacy Viewer keeps it.
    position: f32,
    /// Clamped position one step earlier (interpolation).
    previous: f32,
    velocity: f32,
    velocity_joint: f32,
    accel_joint: f32,
    active: bool,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    t: f64,
    joints: [(Vec3, Quat); 2],
}

/// The motions of one avatar.
pub struct BodyPhysics {
    epoch: Instant,
    motions: [Motion; 6],
    last: Option<Sample>,
    /// Joint positions at the last step.
    step_pos: [Vec3; 2],
    /// Time of the last step (s since `epoch`).
    sim_t: f64,
    /// Interpolation between the last two steps at the frame time.
    alpha: f32,
}

impl BodyPhysics {
    pub fn new(now: Instant) -> Self {
        BodyPhysics {
            epoch: now,
            motions: [Motion::default(); 6],
            last: None,
            step_pos: [Vec3::ZERO; 2],
            sim_t: 0.0,
            alpha: 1.0,
        }
    }

    /// Advance to `now`, the reference joints (mChest, mPelvis) at their
    /// world position and rotation at that time.
    pub fn update(&mut self, rig: &PhysicsRig, params: &Params, now: Instant, joints: [(Vec3, Quat); 2]) {
        for (((def, m), b), r) in MOTIONS.iter().zip(&mut self.motions).zip(&params.motions).zip(&rig.motions) {
            match b {
                Some(b) if !m.active => {
                    // where the spring holds it against gravity
                    let dir = (joints[def.joint as usize].1 * def.dir).normalize_or_zero();
                    let rest = if b.spring > 0.0 {
                        (r.user + dir.z * b.gravity * b.mass / b.spring).clamp(0.0, 1.0)
                    } else {
                        r.user
                    };
                    *m = Motion {
                        position: rest,
                        previous: rest,
                        active: true,
                        ..Motion::default()
                    }
                }
                None => m.active = false,
                _ => {}
            }
        }
        let t = now.saturating_duration_since(self.epoch).as_secs_f64();
        let sample = Sample { t, joints };
        let restart = match &self.last {
            None => true,
            Some(last) => {
                t - last.t > MAX_GAP
                    || last
                        .joints
                        .iter()
                        .zip(&joints)
                        .any(|(a, b)| a.0.distance(b.0) > MAX_JUMP || !b.0.is_finite())
            }
        };
        if restart {
            self.restart(sample);
            return;
        }
        let Some(last) = self.last else {
            return;
        };
        if t <= last.t {
            return;
        }
        let h = f64::from(STEP);
        while self.sim_t + h <= t {
            self.sim_t += h;
            let f = ((self.sim_t - last.t) / (t - last.t)) as f32;
            for (j, def_joint) in [RefJoint::Chest, RefJoint::Pelvis].into_iter().enumerate() {
                let (p0, r0) = last.joints[j];
                let (p1, r1) = joints[j];
                let pos = p0.lerp(p1, f);
                let rot = r0.slerp(r1, f);
                let disp = pos - self.step_pos[j];
                self.step_pos[j] = pos;
                for (((def, m), b), r) in MOTIONS.iter().zip(&mut self.motions).zip(&params.motions).zip(&rig.motions) {
                    if def.joint != def_joint {
                        continue;
                    }
                    if let Some(b) = b {
                        step(m, b, r.user, (rot * def.dir).normalize_or_zero(), disp, STEP);
                    }
                }
            }
        }
        self.last = Some(sample);
        self.alpha = ((t - self.sim_t) / h).clamp(0.0, 1.0) as f32;
    }

    /// Joint tracking starts again from this sample: no velocity from the
    /// jump, the bounce in progress continues.
    fn restart(&mut self, sample: Sample) {
        self.last = Some(sample);
        self.sim_t = sample.t;
        self.step_pos = [sample.joints[0].0, sample.joints[1].0];
        self.alpha = 1.0;
        for m in &mut self.motions {
            m.velocity_joint = 0.0;
            m.accel_joint = 0.0;
        }
    }

    /// Forget the joints' last place: the next update starts tracking again
    /// (the avatar was not updated for a while).
    pub fn hold(&mut self) {
        self.last = None;
    }

    /// Normalised position of each motion at the frame time.
    fn positions(&self) -> [f32; 6] {
        std::array::from_fn(|k| {
            let m = &self.motions[k];
            let cur = m.position.clamp(0.0, 1.0);
            m.previous + (cur - m.previous) * self.alpha
        })
    }

    /// Collision volume offsets (joint, local offset before the parent's
    /// scale), summed per volume, for the motions' current positions.
    pub fn offsets(&self, rig: &PhysicsRig, params: &Params, out: &mut Vec<(usize, Vec3)>) {
        out.clear();
        for ((pos, b), r) in self.positions().into_iter().zip(&params.motions).zip(&rig.motions) {
            let Some(b) = b else {
                continue;
            };
            for d in &r.driven {
                if !(if params.male { d.male } else { d.female }) {
                    continue;
                }
                let w = driven_weight(pos, b.max_effect, d);
                for &(joint, offset) in &d.offsets {
                    match out.iter_mut().find(|o| o.0 == joint) {
                        Some(o) => o.1 += offset * w,
                        None => out.push((joint, offset * w)),
                    }
                }
            }
        }
    }
}

impl BodyPhysics {
    /// The weight of each driven parameter at the frame time (the
    /// system body's morphs of them), for the avatar's sex.
    pub fn driven_weights(&self, rig: &PhysicsRig, params: &Params, out: &mut Vec<(i32, f32)>) {
        out.clear();
        for ((pos, b), r) in self.positions().into_iter().zip(&params.motions).zip(&rig.motions) {
            let Some(b) = b else {
                continue;
            };
            for &(id, min, max, female, male) in &r.weights {
                if if params.male { male } else { female } {
                    out.push((id, rescale(pos, b.max_effect, min, max)));
                }
            }
        }
    }
}

/// One step of LLPhysicsMotion::onUpdate's loop.
fn step(m: &mut Motion, b: &Behavior, user: f32, dir: Vec3, disp: Vec3, h: f32) {
    let unit = h * JOINT_LOCAL_FACTOR;
    // calculateVelocity_local / calculateAcceleration_local
    let velocity_joint = dir.dot(disp) * WORLD_TO_MODEL / unit;
    let accel_raw = (velocity_joint - m.velocity_joint) / unit;
    let keep = ACCEL_KEEP.powf(unit);
    let accel_joint = accel_raw * (1.0 - keep) + m.accel_joint * keep;
    let position = m.position.clamp(0.0, 1.0);
    // spring toward the rest position, joint acceleration, gravity (world
    // up in the motion's direction), damping and drag
    let force = -(position - user) * b.spring + b.gain * accel_joint * b.mass + dir.z * b.gravity * b.mass - b.damping * m.velocity
        + 0.5 * b.drag * velocity_joint * velocity_joint.abs();
    let mut velocity = (m.velocity + force / b.mass * h).clamp(-MAX_VELOCITY, MAX_VELOCITY);
    let new_position = position + velocity * h;
    // stopped at the ends of the range
    if (new_position < 0.0 && velocity < 0.0) || (new_position > 1.0 && velocity > 0.0) {
        velocity = 0.0;
    }
    if !(new_position.is_finite() && velocity.is_finite() && velocity_joint.is_finite() && accel_joint.is_finite()) {
        *m = Motion {
            position: user,
            previous: user,
            active: true,
            ..Motion::default()
        };
        return;
    }
    *m = Motion {
        position: new_position,
        previous: position,
        velocity,
        velocity_joint,
        accel_joint,
        active: true,
    };
}

/// LLPhysicsMotion::setParamValue: the normalised position scaled into
/// [0.5 - effect/2, 0.5 + effect/2] of the driven parameter's range, then
/// clamped to it (LLVisualParam::setWeight).
fn driven_weight(position: f32, max_effect: f32, d: &VolumeMorph) -> f32 {
    rescale(position, max_effect, d.min, d.max)
}

fn rescale(position: f32, max_effect: f32, min: f32, max: f32) -> f32 {
    let lo = 0.5 - max_effect / 2.0;
    let rescaled = lo + max_effect * position;
    (min + (max - min) * rescaled).clamp(min, max)
}

/// Move the collision volumes of a posed skeleton (joint matrices in
/// skeleton space, as `anim::Controller::evaluate` writes them). A volume's
/// local position is scaled by its parent's scale, which the parent's
/// matrix carries: its offset in skeleton space is that matrix's linear
/// part times the local offset.
pub fn apply(rig: &Rig, offsets: &[(usize, Vec3)], palette: &mut [[[f32; 4]; 4]]) {
    for &(joint, offset) in offsets {
        let Some(parent) = rig.parent.get(joint).copied().flatten() else {
            continue;
        };
        let (Some(p), true) = (palette.get(parent), joint < palette.len()) else {
            continue;
        };
        let delta = Mat3::from_mat4(Mat4::from_cols_array_2d(p)) * offset;
        let t = &mut palette[joint][3];
        t[0] += delta.x;
        t[1] += delta.y;
        t[2] += delta.z;
    }
}

/// World position and rotation of a palette joint, the avatar at `root`.
pub fn joint_world(palette: &[[[f32; 4]; 4]], joint: usize, root: (Vec3, Quat)) -> Option<(Vec3, Quat)> {
    let (_, r, t) = Mat4::from_cols_array_2d(palette.get(joint)?).to_scale_rotation_translation();
    Some((root.0 + root.1 * t, (root.1 * r).normalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::time::Duration;

    fn library() -> AvatarLibrary {
        AvatarLibrary::load()
    }

    fn behavior(max_effect: f32) -> Behavior {
        Behavior {
            mass: 0.2,
            gravity: 0.0,
            drag: 0.0,
            max_effect,
            spring: 10.0,
            gain: 10.0,
            damping: 0.5,
        }
    }

    fn only_breast_bounce(b: Behavior) -> Params {
        Params::with(false, [None, Some(b), None, None, None, None])
    }

    /// Chest of an avatar standing upright at `z` (pelvis 0.5 m lower).
    fn standing(z: f32) -> [(Vec3, Quat); 2] {
        [
            (Vec3::new(0.0, 0.0, z), Quat::IDENTITY),
            (Vec3::new(0.0, 0.0, z - 0.5), Quat::IDENTITY),
        ]
    }

    /// Runs `seconds` at `fps`, the chest at `z(t)`; returns the breast
    /// bounce's normalised position at each frame.
    fn run(rig: &PhysicsRig, params: &Params, fps: f32, seconds: f32, z: impl Fn(f32) -> f32) -> Vec<(f32, f32)> {
        let start = Instant::now();
        let mut physics = BodyPhysics::new(start);
        let frames = (seconds * fps).round() as u32;
        (0..=frames)
            .map(|k| {
                let t = k as f32 / fps;
                physics.update(rig, params, start + Duration::from_secs_f32(t), standing(1.5 + z(t)));
                (t, physics.positions()[1])
            })
            .collect()
    }

    /// One 6 cm jump of the chest at 0.5 s, back down at 0.8 s.
    fn hop(t: f32) -> f32 {
        if (0.5..0.8).contains(&t) {
            0.06 * ((t - 0.5) / 0.3 * std::f32::consts::PI).sin()
        } else {
            0.0
        }
    }

    #[test]
    fn the_parameter_ids_are_those_of_avatar_lad() {
        let lad = include_bytes!("../../assets/character/avatar_lad.xml");
        let root = aurora_llsd::xml::parse(lad).expect("avatar_lad.xml");
        let mut names = HashMap::new();
        let mut stack = vec![&root];
        while let Some(e) = stack.pop() {
            if e.name == "param"
                && let (Some(id), Some(name)) = (e.attr("id").and_then(|v| v.trim().parse::<i32>().ok()), e.attr("name"))
            {
                names.insert(id, name.to_owned());
            }
            stack.extend(e.elements());
        }
        let expected = [
            ("Breast", "InOut"),
            ("Breast", "UpDown"),
            ("Breast", "LeftRight"),
            ("Butt", "UpDown"),
            ("Butt", "LeftRight"),
            ("Belly", "UpDown"),
        ];
        for (def, (part, axis)) in MOTIONS.iter().zip(expected) {
            assert_eq!(names[&def.controller], format!("{part}_Physics_{axis}_Controller"));
            let suffixes = [
                "Mass".to_owned(),
                "Gravity".to_owned(),
                "Drag".to_owned(),
                format!("{axis}_Max_Effect"),
                format!("{axis}_Spring"),
                format!("{axis}_Gain"),
                format!("{axis}_Damping"),
            ];
            for (id, suffix) in def.behavior.iter().zip(suffixes) {
                assert_eq!(names[id], format!("{part}_Physics_{suffix}"));
            }
            for id in def.driven {
                assert!(names[id].starts_with(&format!("{part}_Physics_")) && names[id].ends_with("_Driven"));
            }
        }
    }

    #[test]
    fn an_avatar_without_a_physics_layer_gets_no_physics() {
        let lib = library();
        let rig = PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        for male in [false, true] {
            let weights = lib.shape_params.effective_weights_for_sex(male);
            assert_eq!(Params::from_weights(&rig, &weights, male), None);
        }
        // breasts only: nothing moves on a male avatar
        let mut weights = lib.shape_params.effective_weights_for_sex(true);
        weights.insert(10003, 1.0);
        assert_eq!(Params::from_weights(&rig, &weights, true), None);
        weights.insert(10014, 1.0);
        let params = Params::from_weights(&rig, &weights, true).expect("belly physics");
        assert!(params.motions[5].is_some() && params.motions[1].is_none());
    }

    #[test]
    fn rest_leaves_the_shape_unchanged() {
        let lib = library();
        let rig = PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        let all = Params::with(false, [Some(behavior(1.5)); 6]);
        let start = Instant::now();
        let mut physics = BodyPhysics::new(start);
        let mut offsets = Vec::new();
        for k in 0..240 {
            physics.update(&rig, &all, start + Duration::from_millis(k * 16), standing(1.5));
            physics.offsets(&rig, &all, &mut offsets);
            for (_, o) in &offsets {
                assert!(o.length() < 1e-6, "at rest the volumes stay in place: {offsets:?}");
            }
        }
        assert_eq!(offsets.len(), 4, "LEFT_PEC, RIGHT_PEC, BUTT and BELLY");
    }

    #[test]
    fn a_hop_bounces_then_settles_at_rest() {
        let lib = library();
        let rig = PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        let params = only_breast_bounce(behavior(1.0));
        let trace = run(&rig, &params, 60.0, 6.0, hop);
        let user = rig.motions[1].user;
        let peak = trace.iter().map(|(_, p)| (p - user).abs()).fold(0.0, f32::max);
        assert!(peak > 0.02, "the jump moves the breasts: {peak}");
        // springs back past rest at least once
        let above = trace.iter().any(|(t, p)| *t > 0.5 && *p > user + 0.01);
        let below = trace.iter().any(|(t, p)| *t > 0.5 && *p < user - 0.01);
        assert!(above && below, "it oscillates");
        let late = trace
            .iter()
            .filter(|(t, _)| *t > 5.0)
            .map(|(_, p)| (p - user).abs())
            .fold(0.0, f32::max);
        assert!(late < 2e-3, "damped back to rest: {late}");
        // before the jump nothing moved
        assert!(trace.iter().filter(|(t, _)| *t < 0.5).all(|(_, p)| (p - user).abs() < 1e-6));
    }

    #[test]
    fn the_bounce_is_the_same_at_30_60_and_144_fps() {
        let lib = library();
        let rig = PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        let params = only_breast_bounce(behavior(1.0));
        let reference = run(&rig, &params, 240.0, 3.0, hop);
        let at = |trace: &[(f32, f32)], t: f32| {
            let k = trace.partition_point(|s| s.0 < t).min(trace.len() - 1);
            trace[k].1
        };
        let peak = reference.iter().map(|(_, p)| (p - 0.5).abs()).fold(0.0, f32::max);
        for fps in [30.0, 60.0, 144.0] {
            let trace = run(&rig, &params, fps, 3.0, hop);
            let worst = trace.iter().map(|(t, p)| (p - at(&reference, *t)).abs()).fold(0.0, f32::max);
            assert!(worst < 0.15 * peak, "{fps} fps differs by {worst} (peak {peak})");
        }
    }

    #[test]
    fn gravity_and_max_effect_scale_the_offset() {
        let lib = library();
        let rig = PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        let mut b = behavior(1.0);
        b.gravity = 10.0;
        let sag = |b: Behavior| {
            let params = only_breast_bounce(b);
            let trace = run(&rig, &params, 60.0, 8.0, |_| 0.0);
            trace.last().expect("trace").1
        };
        // spring and gravity balance: x = g m / k above rest (the driven
        // parameter's positive weight moves the volumes down)
        let rest = sag(b);
        assert!((rest - (0.5 + 10.0 * 0.2 / 10.0)).abs() < 0.01, "{rest}");
        // already there on the first frame: no settling when it appears
        let first = run(&rig, &only_breast_bounce(b), 60.0, 0.1, |_| 0.0);
        assert!(first.iter().all(|(_, p)| (p - rest).abs() < 1e-3), "{first:?}");
        let start = Instant::now();
        let mut physics = BodyPhysics::new(start);
        let mut offsets = Vec::new();
        let mut down = |effect: f32| {
            b.max_effect = effect;
            let params = only_breast_bounce(b);
            for k in 0..600 {
                physics.update(&rig, &params, start + Duration::from_millis(k * 16), standing(1.5));
            }
            physics.offsets(&rig, &params, &mut offsets);
            offsets.iter().map(|o| o.1.z).fold(0.0, f32::min)
        };
        let small = down(0.5);
        let large = down(2.0);
        assert!(
            small < 0.0 && large < 2.0 * small,
            "gravity lowers the breasts more with a larger effect: {small} {large}"
        );
    }

    #[test]
    fn a_teleport_or_a_long_pause_does_not_kick() {
        let lib = library();
        let rig = PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        let params = only_breast_bounce(behavior(1.0));
        let start = Instant::now();
        let mut physics = BodyPhysics::new(start);
        for k in 0..60 {
            physics.update(&rig, &params, start + Duration::from_millis(k * 16), standing(1.5));
        }
        physics.update(&rig, &params, start + Duration::from_millis(976), standing(140.0));
        physics.update(&rig, &params, start + Duration::from_secs(5), standing(30.0));
        physics.hold();
        physics.update(&rig, &params, start + Duration::from_millis(5100), standing(31.0));
        for k in 1..60 {
            physics.update(&rig, &params, start + Duration::from_millis(5100 + k * 16), standing(31.0));
        }
        assert!((physics.positions()[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn offsets_move_the_volumes_in_the_parents_frame() {
        let lib = library();
        let rig = &lib.rig;
        let pec = rig.names["LEFT_PEC"];
        let chest = rig.names["mChest"];
        let mut scale = rig.scale.clone();
        scale[chest] = Vec3::new(1.0, 1.0, 2.0);
        let mut rot = rig.local_rot.clone();
        rot[chest] = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let world = rig.world_with(&rot, Vec3::ZERO, &rig.local_pos, &scale);
        let mut palette: Vec<[[f32; 4]; 4]> = world.iter().map(|m| m.to_cols_array_2d()).collect();
        apply(rig, &[(pec, Vec3::new(0.0, 0.0, -0.01))], &mut palette);
        // as if the shape had moved the volume's rest position
        let mut moved = rig.local_pos.clone();
        moved[pec] += Vec3::new(0.0, 0.0, -0.01);
        let expected = rig.world_with(&rot, Vec3::ZERO, &moved, &scale)[pec];
        assert!(Mat4::from_cols_array_2d(&palette[pec]).abs_diff_eq(expected, 1e-5));
    }

    #[test]
    fn the_system_body_morphs_follow_the_motions() {
        let lib = library();
        let rig = PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        let all = Params::with(false, [Some(behavior(1.0)); 6]);
        let start = Instant::now();
        let mut physics = BodyPhysics::new(start);
        physics.update(&rig, &all, start, standing(1.5));
        let mut weights = Vec::new();
        physics.driven_weights(&rig, &all, &mut weights);
        let mut ids: Vec<i32> = weights.iter().map(|w| w.0).collect();
        ids.sort_unstable();
        // the belly's legs and skirt morphs have no volume but are driven
        assert_eq!(ids, (1200..=1207).collect::<Vec<_>>());
        assert!(weights.iter().all(|w| w.1.abs() < 1e-6), "rest: {weights:?}");
        for k in 1..60 {
            physics.update(
                &rig,
                &all,
                start + Duration::from_millis(k * 16),
                standing(1.5 + hop(0.5 + k as f32 / 60.0)),
            );
        }
        physics.driven_weights(&rig, &all, &mut weights);
        assert!(weights.iter().any(|w| w.0 == 1200 && w.1.abs() > 0.01), "{weights:?}");
        // breast bounce and cleavage are a female avatar's only (the sway
        // has no sex in avatar_lad.xml)
        let male = Params::with(true, [Some(behavior(1.0)); 6]);
        physics.driven_weights(&rig, &male, &mut weights);
        assert!(weights.iter().all(|w| ![1200, 1201].contains(&w.0)));
        assert!(weights.iter().any(|w| w.0 == 1207));
    }

    #[test]
    fn detail_limits_the_avatars_small_on_screen() {
        let config = Config::default();
        assert!(config.updates(false, 500.0), "detail 1: every avatar");
        let low = Config { detail: 0.5, ..config };
        assert!(low.updates(true, 500.0) && low.updates(false, 5.0) && !low.updates(false, 60.0));
        assert!(!Config { detail: 0.0, ..config }.updates(true, 1.0));
        assert!(!Config { enabled: false, ..config }.updates(true, 1.0));
    }
}
