//! Complete local pose channels, including collision volumes and per-joint scale.
use super::*;
use crate::scene::anim::SkeletonBase;
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) struct Pose {
    pub rotations: Vec<Quat>,
    pub positions: Vec<Vec3>,
    pub scales: Vec<Vec3>,
    pub locks: HashMap<usize, Mat4>,
    /// What each joint's rotation is authored from: the captured rotation
    /// until T-pose, a preset or a file replaces it. Lives in the pose so that
    /// undo restores it with the rotations. Port of FSJointState::mBaseRotation
    /// (indra/newview/fsjointpose.h).
    pub origin: Vec<Quat>,
}
impl Pose {
    pub fn capture(rig: &Rig, frame: &Frame) -> Option<Self> {
        let (pose, scales) = authoring::local_pose(rig, frame)?;
        Some(Self {
            origin: pose.rotations.clone(),
            rotations: pose.rotations,
            positions: pose.positions,
            scales,
            locks: HashMap::new(),
        })
    }
    pub fn valid(&self, rig: &Rig) -> bool {
        self.rotations.len() == rig.len()
            && self.positions.len() == rig.len()
            && self.scales.len() == rig.len()
            && self.origin.len() == rig.len()
            && self
                .rotations
                .iter()
                .chain(&self.origin)
                .all(|q| q.is_finite() && (0.99..=1.01).contains(&q.length_squared()))
            && self.positions.iter().all(|p| p.is_finite() && p.abs().max_element() <= 100.0)
            && self.scales.iter().all(|s| scale_valid(*s))
            && self.locks.iter().all(|(b, m)| *b < rig.len() && m.is_finite())
    }
    /// Bake the displayed world locks into skeleton-local channels before I/O.
    /// Export must never mistake another subject's world coordinates for a pose.
    pub fn snapshot(&self, rig: &Rig, model: Mat4) -> Option<Self> {
        let resolved = self.resolved(rig, model);
        let joints = rig.world_with(&resolved.rotations, Vec3::ZERO, &resolved.positions, &self.scales);
        let mut pose = Self {
            rotations: resolved.rotations,
            positions: resolved.positions,
            scales: self.scales.clone(),
            locks: HashMap::new(),
            origin: self.origin.clone(),
        };
        for bone in self.locks.keys() {
            pose.locks.insert(*bone, joints[*bone]);
        }
        pose.valid(rig).then_some(pose)
    }
    pub fn matrices(&self, rig: &Rig, model: Mat4) -> Vec<Mat4> {
        if self.locks.is_empty() {
            return rig.world_with(&self.rotations, Vec3::ZERO, &self.positions, &self.scales);
        }
        let pose = self.resolved(rig, model);
        rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &self.scales)
    }
}

/// A joint scale the poser keeps. Its sign is the shape's: some sliders
/// mirror a face or pectoral joint on one axis, and refusing those refused
/// the whole pose of such an avatar (#1249). Its floor is the capture's
/// (authoring::local_pose): a slider crossing zero leaves a joint far below
/// what an edit may reach, and that joint must not refuse the pose either.
pub(super) fn scale_valid(scale: Vec3) -> bool {
    scale.is_finite() && scale.abs().min_element() >= 1e-5 && scale.abs().max_element() <= 100.0
}
/// An edited scale held within the poser's bounds, keeping each axis's sign.
pub(super) fn bounded_scale(scale: Vec3) -> Vec3 {
    scale.signum() * scale.abs().clamp(Vec3::splat(0.001), Vec3::splat(10.0))
}

#[derive(Clone)]
pub(super) struct Session {
    /// Shape and worn-mesh rest from which the local pose channels are based.
    pub rest: Arc<SkeletonBase>,
    /// Editor-only creation paths bind to Scene's authoritative rest on the
    /// next animation tick, before rendering the pose.
    pub rest_initialized: bool,
    pub base: Arc<Pose>,
    pub pose: Arc<Pose>,
    pub enabled: bool,
    pub undo: Vec<Arc<Pose>>,
    pub redo: Vec<Arc<Pose>>,
}
impl Session {
    pub fn new(pose: Pose) -> Self {
        let pose = Arc::new(pose);
        let rest = Arc::new(SkeletonBase {
            local_pos: pose.positions.clone(),
            scale: pose.scales.clone(),
        });
        Self {
            rest,
            rest_initialized: false,
            base: pose.clone(),
            pose,
            enabled: true,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
    pub fn rebase(&mut self, rest: Arc<SkeletonBase>) {
        if !self.rest_initialized {
            self.rest = rest;
            self.rest_initialized = true;
            return;
        }
        if Arc::ptr_eq(&self.rest, &rest) {
            return;
        }
        if *self.rest == *rest {
            self.rest = rest;
            return;
        }
        let old = &self.rest;
        for pose in [&mut self.base, &mut self.pose]
            .into_iter()
            .chain(self.undo.iter_mut())
            .chain(self.redo.iter_mut())
        {
            rebase_pose(Arc::make_mut(pose), old, &rest);
        }
        self.rest = rest;
    }
    pub fn checkpoint(&mut self) {
        if self.undo.len() == 64 {
            self.undo.remove(0);
        }
        self.undo.push(self.pose.clone());
        self.redo.clear();
    }
    pub fn history(&mut self, redo: bool) {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(pose) = from.pop() {
            to.push(std::mem::replace(&mut self.pose, pose));
        }
    }
    pub fn edited(&self, bone: usize) -> bool {
        self.pose.rotations[bone].dot(self.base.rotations[bone]).abs() < 0.9999999
            || self.pose.positions[bone].distance_squared(self.base.positions[bone]) > 1e-10
            || self.pose.scales[bone].distance_squared(self.base.scales[bone]) > 1e-10
    }
    /// The pose an export writes: the user's changes on the rest skeleton,
    /// without what the captured pose brought (FSPoserAnimator::
    /// getJointExportRotation, indra/newview/fsposeranimator.cpp, writes the
    /// change and not the base too). A change, not a filter on edited joints,
    /// so that touching a joint does not bring its captured rotation along.
    pub fn authored(&self, rig: &Rig, model: Mat4) -> Option<Pose> {
        let resolved = self.pose.snapshot(rig, model)?;
        let pose = Pose {
            rotations: (0..rig.len())
                .map(|b| (rig.local_rot[b] * resolved.origin[b].inverse() * resolved.rotations[b]).normalize())
                .collect(),
            positions: (0..rig.len())
                .map(|b| rig.local_pos[b] + resolved.positions[b] - self.base.positions[b])
                .collect(),
            scales: rig.scale.clone(),
            locks: HashMap::new(),
            origin: rig.local_rot.clone(),
        };
        pose.valid(rig).then_some(pose)
    }
}

fn rebase_pose(pose: &mut Pose, old: &SkeletonBase, new: &SkeletonBase) {
    let count = pose.positions.len().min(old.local_pos.len()).min(new.local_pos.len());
    for bone in 0..count {
        pose.positions[bone] += new.local_pos[bone] - old.local_pos[bone];
    }
    let count = pose.scales.len().min(old.scale.len()).min(new.scale.len());
    for bone in 0..count {
        let raw = new.scale[bone] * (pose.scales[bone] / old.scale[bone]);
        let sign = pose.scales[bone].signum() * old.scale[bone].signum() * new.scale[bone].signum();
        let rebased = sign * raw.abs().clamp(Vec3::splat(1e-5), Vec3::splat(100.0));
        // A shape slider can flatten a rest axis to zero (#1249): the ratio
        // is then meaningless, so that axis takes the new rest scale, held
        // in the same bounds. A rest still flat after a later shape change
        // would otherwise give the pose a zero axis, which `scale_valid`
        // refuses: the whole pose, and every undo step rebased with it,
        // would be dropped.
        let fallback = new.scale[bone].signum() * new.scale[bone].abs().clamp(Vec3::splat(1e-5), Vec3::splat(100.0));
        let usable = old.scale[bone].abs().cmpge(Vec3::splat(1e-5));
        pose.scales[bone] = Vec3::select(usable, rebased, fallback);
    }
}
/// A joint's rotation mirrored on its counterpart, in that joint's own rest
/// frame. Origins go through it too: a mirrored joint keeps the change only.
pub(super) fn mirrored(rig: &Rig, q: Quat, from: usize, to: usize) -> Quat {
    let relative = rig.local_rot[from].inverse() * q;
    (rig.local_rot[to] * Quat::from_xyzw(-relative.x, relative.y, -relative.z, relative.w)).normalize()
}
