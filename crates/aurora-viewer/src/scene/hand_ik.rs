//! Two-segment hand contacts layered over the unmodified animation pose.
//! Like LLJointSolverRP3::solve (indra/llcharacter/lljointsolverrp3.cpp),
//! use segment lengths and a pole plane. Each compatible viewer solves
//! received definitions locally; no modified poses are sent to the simulator.

#[cfg(test)]
use super::{Scene, anim::Rig};
#[cfg(test)]
use crate::world::{
    World,
    hand_ik::{self, Hand},
};
use glam::{Quat, Vec3};
#[cfg(test)]
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub position: Vec3,
    pub rotation: Option<Quat>,
    pub contact: Vec3,
}

#[cfg(test)]
pub type Targets = [Option<Target>; 2];

#[cfg(test)]
#[derive(Default)]
pub struct Pose {
    pub targets: Targets,
    held: Targets,
    weights: [f32; 2],
    at: Option<Instant>,
}

#[cfg(test)]
impl Pose {
    /// The base rotations must never be overwritten: otherwise stopping an
    /// IK constraint would leave its corrected pose in unanimated joints.
    #[allow(clippy::too_many_arguments)] // Rig pose inputs plus the contact clock/edit freeze.
    pub fn rotations(
        &mut self,
        rig: &Rig,
        base_rot: &[Quat],
        pelvis: Vec3,
        local_pos: &[Vec3],
        scale: &[Vec3],
        now: Instant,
        paused: bool,
    ) -> Option<Vec<Quat>> {
        let dt = self
            .at
            .map_or(1.0 / 60.0, |at| now.saturating_duration_since(at).as_secs_f32().min(0.1));
        self.at = Some(now);
        if !paused {
            for i in 0..2 {
                if let Some(target) = self.targets[i] {
                    self.held[i] = Some(target);
                    self.weights[i] = (self.weights[i] + dt / 0.2).min(1.0);
                } else {
                    self.weights[i] = (self.weights[i] - dt / 0.2).max(0.0);
                    if self.weights[i] == 0.0 {
                        self.held[i] = None;
                    }
                }
            }
        }
        if self.held.iter().all(Option::is_none) {
            return None;
        }
        let world = rig.world_with(base_rot, pelvis, local_pos, scale);
        let mut rotations = base_rot.to_vec();
        for hand in [Hand::Left, Hand::Right] {
            let i = hand.index();
            let Some(mut target) = self.held[i] else { continue };
            let [a, b, c] = hand.joints().map(|name| rig.names.get(name).copied());
            let (Some(a), Some(b), Some(c)) = (a, b, c) else { continue };
            if rig.parent[b] != Some(a) || rig.parent[c] != Some(b) {
                continue;
            }
            let transform = |j: usize| {
                let (_, rotation, position) = world[j].to_scale_rotation_translation();
                (rotation, position)
            };
            let [(ar, ap), (br, bp), (cr, cp)] = [a, b, c].map(transform);
            // LLXformMatrix joint scales are not inherited. Scale the authored
            // palm point by this avatar's actual wrist scale, like its skin.
            target.contact *= scale.get(c).copied().unwrap_or(rig.scale[c]);
            let parent_rot = rig.parent[a].map_or(Quat::IDENTITY, |p| transform(p).0);
            // Avatar axes: +X forward, +Y left, +Z up. Elbows prefer an
            // outward/downward plane, including near a straight-arm singularity.
            let pole = Vec3::new(0.0, if hand == Hand::Left { 1.0 } else { -1.0 }, -0.5);
            if let Some(solved) = solve([ap, bp, cp], [ar, br, cr], parent_rot, target, pole) {
                for (joint, rotation) in [a, b, c].into_iter().zip(solved) {
                    // Matrix decomposition through nonuniform shape scales
                    // must not perturb an unconstrained local wrist rotation.
                    if joint == c && target.rotation.is_none() {
                        continue;
                    }
                    rotations[joint] = base_rot[joint].slerp(rotation, self.weights[i]).normalize();
                }
            }
        }
        Some(rotations)
    }
}

/// Analytic contact with a shaped wrist-local point (zero for wrist pivot).
/// Without an orientation target, elbow-to-palm is one rigid virtual segment:
/// solving it reaches the contact exactly without changing wrist articulation
/// or iterating on an estimated wrist position. With an authored orientation,
/// subtract the rotated palm offset to obtain the wrist goal first.
pub(super) fn solve(points: [Vec3; 3], rotations: [Quat; 3], parent: Quat, target: Target, pole: Vec3) -> Option<[Quat; 3]> {
    let [a, b, wrist] = points;
    let (c, goal) = match target.rotation {
        Some(q) => (wrist, target.position - q * target.contact),
        None => (wrist + rotations[2] * target.contact, target.position),
    };
    let ab = b - a;
    let bc = c - b;
    let upper = ab.length();
    let lower = bc.length();
    if !points.iter().all(|p| p.is_finite())
        || !c.is_finite()
        || !goal.is_finite()
        || !upper.is_finite()
        || !lower.is_finite()
        || upper < 0.0001
        || lower < 0.0001
        || !target.position.is_finite()
        || !target.contact.is_finite()
        || target.rotation.is_some_and(|q| !q.is_finite())
    {
        return None;
    }
    let delta = goal - a;
    let direction = delta.try_normalize().or_else(|| (c - a).try_normalize()).unwrap_or(Vec3::X);
    let eps = upper.min(lower) * 0.0001;
    let distance = delta.length().clamp((upper - lower).abs() + eps, upper + lower - eps);
    let x = (upper * upper - lower * lower + distance * distance) / (2.0 * distance);
    let y = (upper * upper - x * x).max(0.0).sqrt();
    let projection = |v: Vec3| (v - direction * v.dot(direction)).try_normalize();
    let bend = projection(pole).or_else(|| projection(Vec3::Z)).or_else(|| projection(Vec3::X))?;
    let elbow = a + direction * x + bend * y;
    let end = a + direction * distance;
    let shoulder_delta = Quat::from_rotation_arc(ab / upper, (elbow - a).normalize());
    let shoulder = (shoulder_delta * rotations[0]).normalize();
    let elbow_delta = Quat::from_rotation_arc((shoulder_delta * bc).normalize(), (end - elbow).normalize());
    let elbow_rotation = (elbow_delta * shoulder_delta * rotations[1]).normalize();
    let result = [
        (parent.inverse() * shoulder).normalize(),
        (shoulder.inverse() * elbow_rotation).normalize(),
        target
            .rotation
            .map_or_else(|| rotations[1].inverse() * rotations[2], |q| elbow_rotation.inverse() * q)
            .normalize(),
    ];
    result.iter().all(|q| q.is_finite()).then_some(result)
}

#[cfg(test)]
mod tests;
