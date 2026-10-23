//! Simultaneous associations for one source joint share one weighted goal.
//! Resolve after their dependencies, normalize competing targets, and apply
//! total influence once. A later row must never silently replace an earlier one.
use super::*;

pub(super) fn combine(rig: &Rig, pose: &LocalPose, pelvis: Vec3, scale: &[Vec3], peers: &[(&Goal, f32)]) -> Option<Goal> {
    let mut resolved = Vec::with_capacity(peers.len());
    for &(goal, fade) in peers {
        let weight = goal.weight * fade;
        if !weight.is_finite() || weight <= 0.0 {
            continue;
        }
        let Some(goal) = (match &goal.follow {
            Some(follow) => follow.resolve(goal, rig, pose, pelvis, scale),
            None => Some(goal.clone()),
        }) else {
            continue;
        };
        resolved.push((goal, weight));
    }
    let first = &resolved.first()?.0;
    let bone = first.bone;
    let matrices = rig.world_with(&pose.rotations, pelvis, &pose.positions, scale);
    let q = matrices.get(bone)?.to_scale_rotation_translation().1;
    let shaped = scale.get(bone).copied().unwrap_or(rig.scale[bone]);
    let oriented = resolved.iter().any(|(g, _)| g.rotation.is_some());
    let reference = resolved.iter().find_map(|(g, _)| g.rotation).unwrap_or(q);
    let (mut position, mut contact, mut rotation, mut pole) = (Vec3::ZERO, Vec3::ZERO, glam::Vec4::ZERO, Vec3::ZERO);
    let (mut total, mut pole_weight) = (0.0, 0.0);
    let mut result = first.clone();
    result.follow = None;
    for (goal, weight) in &resolved {
        let r = goal.rotation.unwrap_or(q);
        let r = if r.dot(reference) < 0.0 { -r } else { r };
        rotation += glam::Vec4::from_array(r.to_array()) * *weight;
        position += (goal.position - if oriented { r * (shaped * goal.contact) } else { Vec3::ZERO }) * *weight;
        contact += goal.contact * *weight;
        if let Some(p) = goal.pole {
            pole += p * *weight;
            pole_weight += *weight;
        }
        if goal.mode == Mode::Ik && (result.mode == Mode::Direct || depth(rig, goal.root) < depth(rig, result.root)) {
            result.mode = Mode::Ik;
            result.root = goal.root;
        }
        total += *weight;
    }
    result.position = position / total;
    result.contact = if oriented { Vec3::ZERO } else { contact / total };
    result.rotation = oriented.then(|| Quat::from_array(rotation.normalize().to_array()));
    result.pole = (pole_weight > 0.0).then(|| pole / pole_weight);
    result.weight = total.min(1.0);
    Some(result)
}

#[cfg(test)]
mod tests;
