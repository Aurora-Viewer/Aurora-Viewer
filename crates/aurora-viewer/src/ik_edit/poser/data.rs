//! Complete local pose channels, including collision volumes and per-joint scale.
use super::*;
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) struct Pose {
    pub rotations: Vec<Quat>,
    pub positions: Vec<Vec3>,
    pub scales: Vec<Vec3>,
    pub locks: HashMap<usize, Mat4>,
}
impl Pose {
    pub fn capture(rig: &Rig, frame: &Frame) -> Option<Self> {
        let (pose, scales) = authoring::local_pose(rig, frame)?;
        Some(Self {
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
            && self
                .rotations
                .iter()
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
/// the whole pose of such an avatar (#1249).
pub(super) fn scale_valid(scale: Vec3) -> bool {
    scale.is_finite() && scale.abs().min_element() >= 0.001 && scale.abs().max_element() <= 100.0
}
/// An edited scale held within the poser's bounds, keeping each axis's sign.
pub(super) fn bounded_scale(scale: Vec3) -> Vec3 {
    scale.signum() * scale.abs().clamp(Vec3::splat(0.001), Vec3::splat(10.0))
}

#[derive(Clone)]
pub(super) struct Session {
    pub base: Arc<Pose>,
    pub pose: Arc<Pose>,
    pub enabled: bool,
    pub undo: Vec<Arc<Pose>>,
    pub redo: Vec<Arc<Pose>>,
}
impl Session {
    pub fn new(pose: Pose) -> Self {
        let pose = Arc::new(pose);
        Self {
            base: pose.clone(),
            pose,
            enabled: true,
            undo: Vec::new(),
            redo: Vec::new(),
        }
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
}
