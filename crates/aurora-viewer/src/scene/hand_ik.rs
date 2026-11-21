//! The analytic two-segment solver: a joint pair reaches a target with a
//! shaped end-local contact point. Like LLJointSolverRP3::solve
//! (indra/llcharacter/lljointsolverrp3.cpp), it uses segment lengths and a
//! pole plane. The IK goals of the editors (`scene::ik::body`) and the
//! native foot placement (`scene::anim::legacy_grounding`) solve through it;
//! nothing is sent to the simulator.

use glam::{Quat, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub position: Vec3,
    pub rotation: Option<Quat>,
    pub contact: Vec3,
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
