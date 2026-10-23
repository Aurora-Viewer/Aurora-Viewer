//! Extended limb chains retain analytic bend planes and reconcile shared roots.
use super::*;

const BODY_JOINTS: [&str; 5] = ["mPelvis", "mTorso", "mChest", "mCollarLeft", "mCollarRight"];
// Prefer the nearby seed pose in redundant body joints. Without this secondary
// objective, equally good contacts can select different body poses after
// arbitrarily small changes to the animation or target positions.
const BODY_POSE_WEIGHT: f32 = 0.00001;
// A warm start chooses a nearby contact solution, not a permanent posture.
// Recover toward the current animation when it can carry the contacts itself;
// otherwise the contact solve reinstates only the body assistance still needed.
const BODY_RECOVERY_SECONDS: f32 = 0.25;

fn rotation_delta(from: Quat, to: Quat) -> Vec3 {
    let delta = (to * from.inverse()).normalize();
    (if delta.w < 0.0 { -delta } else { delta }).to_scaled_axis()
}

/// Warm starts contain corrections only, never animation inputs or IK contacts.
/// Reusing the preceding solution avoids choosing another redundant body pose
/// from scratch at every frame, while the current animation remains the base.
#[derive(Default)]
pub(super) struct Continuity {
    editing: bool,
    chains: Vec<(usize, usize)>,
    corrections: Vec<(usize, Quat)>,
    retention: f32,
}

impl Continuity {
    pub(super) fn begin_frame(&mut self, editing: bool, dt: f32) {
        if self.editing != editing {
            self.clear();
            self.editing = editing;
        }
        self.retention = (-dt / BODY_RECOVERY_SECONDS).exp();
    }

    pub(super) fn clear(&mut self) {
        self.chains.clear();
        self.corrections.clear();
    }
}

pub(super) fn limb(rig: &Rig, bone: usize) -> bool {
    rig.canonical_names
        .get(bone)
        .is_some_and(|n| ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"].contains(&n.as_str()))
}

/// Preserve the animated ABC plane, as in LLJointSolverRP3::solve
/// (indra/llcharacter/lljointsolverrp3.cpp), without an arm-specific pole.
/// Transport it with the endpoint direction so moving the contact through the
/// original AB direction cannot flip the knuckle to the opposite side.
fn animated_pole([a, b, c]: [Vec3; 3], rotation: Quat, target: Vec3) -> Vec3 {
    let Some(from) = (c - a).try_normalize().or_else(|| (b - a).try_normalize()) else {
        return Vec3::ZERO;
    };
    let to = (target - a).try_normalize().unwrap_or(from);
    let projected = |v: Vec3| {
        let v = v - from * v.dot(from);
        // Relative to segment length: a finger's valid bend plane must not be
        // discarded by a millimetre threshold intended for a whole limb.
        (v.length_squared() > a.distance_squared(b) * 0.000001).then(|| v.normalize())
    };
    let bend = projected(b - a)
        .or_else(|| projected(rotation * Vec3::Z))
        .or_else(|| projected(rotation * Vec3::X))
        .unwrap_or(rotation * Vec3::Y);
    Quat::from_rotation_arc(from, to) * bend
}

/// The last two segments keep their bend plane, even when the torso/pelvis
/// participates. Reachable contacts use the limb alone to preserve the pose.
pub(super) fn analytic(
    rig: &Rig,
    pose: &mut LocalPose,
    path: &[usize],
    joints: &[(Vec3, Quat)],
    parent: Quat,
    goal: &Goal,
    contact: Vec3,
    reachable_only: bool,
) -> bool {
    let start = path.len() - 3;
    let [(a, aq), (b, bq), (c, cq)] = [joints[start], joints[start + 1], joints[start + 2]];
    let target = goal.rotation.map_or(goal.position, |q| goal.position - q * contact);
    let end = if goal.rotation.is_some() { c } else { c + cq * contact };
    let (upper, lower, distance) = (a.distance(b), b.distance(end), a.distance(target));
    // A straight limb is reachable. The analytic solver's tiny singularity
    // clamp must not recruit the pelvis and displace another planted foot.
    if reachable_only && (distance > upper + lower + 0.0001 || distance < (upper - lower).abs() - 0.0001) {
        return false;
    }
    let name = &rig.canonical_names[goal.bone];
    let leg = ["mAnkleLeft", "mAnkleRight", "mFootLeft", "mFootRight"].contains(&name.as_str());
    let mut pole = goal.pole.map_or_else(
        || {
            if leg {
                Vec3::X
            } else if limb(rig, goal.bone) {
                Vec3::new(0.0, if name.ends_with("Right") { -1.0 } else { 1.0 }, -0.5)
            } else {
                // A protected wrist shortens a finger chain to three joints.
                // Its phalanges must retain their own plane, not an elbow's.
                animated_pole([a, b, end], aq, target)
            }
        },
        |p| p - a,
    );
    if goal.pole.is_some() {
        let axis = (target - a).normalize_or_zero();
        if (pole - axis * pole.dot(axis)).length_squared() < 0.000001 {
            pole = b - a;
        }
    }
    let parent = if start == 0 { parent } else { joints[start - 1].1 };
    let Some(solved) = hand_ik::solve(
        [a, b, c],
        [aq, bq, cq],
        parent,
        hand_ik::Target {
            position: goal.position,
            rotation: goal.rotation,
            contact,
        },
        pole,
    ) else {
        return false;
    };
    for (&joint, rotation) in path[start..].iter().zip(solved) {
        if joint != goal.bone || goal.rotation.is_some() {
            pose.rotations[joint] = rotation;
        }
    }
    true
}

/// Project the limb root into its reachable shell using each body ancestor.
/// Endpoint CCD stalls near straight limbs; the coarse two-segment construction
/// brings the hip/shoulder into reach before solving the actual bend plane.
pub(super) fn assist(
    rig: &Rig,
    pose: &mut LocalPose,
    path: &[usize],
    joints: &mut [(Vec3, Quat)],
    parent: Quat,
    goal: &Goal,
    contact: Vec3,
) {
    let start = path.len() - 3;
    let last = path.len() - 1;
    let target = goal.rotation.map_or(goal.position, |q| goal.position - q * contact);
    let end = if goal.rotation.is_some() {
        joints[last].0
    } else {
        joints[last].0 + joints[last].1 * contact
    };
    let upper = joints[start].0.distance(joints[start + 1].0);
    let lower = joints[start + 1].0.distance(end);
    let (near, far) = ((upper - lower).abs() + 0.0001, (upper + lower - 0.0001).max(0.0001));
    for _ in 0..4 {
        for index in (0..start).rev() {
            if rig.canonical_names[path[index]].starts_with("mSpine") {
                continue;
            }
            let (pivot, q) = joints[index];
            let root = joints[start].0;
            let distance = root.distance(target);
            if (near..=far).contains(&distance) {
                return;
            }
            let Some(direction) = (target - root).try_normalize() else {
                continue;
            };
            let reach = if distance < near { near } else { far };
            let parent_q = if index == 0 { parent } else { joints[index - 1].1 };
            let Some(solved) = hand_ik::solve(
                [pivot, root, root + direction * reach],
                [q, joints[start].1, joints[last].1],
                parent_q,
                hand_ik::Target {
                    position: target,
                    rotation: None,
                    contact: Vec3::ZERO,
                },
                root - pivot,
            ) else {
                continue;
            };
            let delta = ((parent_q * solved[0]) * q.inverse()).normalize();
            let angle = delta.to_axis_angle().1.abs();
            let delta = Quat::IDENTITY.slerp(delta, (0.5 / angle.max(0.5)).min(1.0)).normalize();
            pose.rotations[path[index]] = (parent_q.inverse() * delta * q).normalize();
            for joint in &mut joints[index..] {
                joint.0 = pivot + delta * (joint.0 - pivot);
                joint.1 = (delta * joint.1).normalize();
            }
        }
    }
}

/// Shared ancestors can move an earlier contact. Reproject full-strength pins
/// together, retaining the best bounded result. Partial blends run only once.
#[allow(clippy::too_many_arguments)] // Solver inputs plus the source-pose secondary objective.
fn balance(
    rig: &Rig,
    pose: &mut LocalPose,
    pelvis: Vec3,
    scale: &[Vec3],
    reference: &[Quat],
    pins: &[&Goal],
    score: &impl Fn(&LocalPose) -> f32,
    error: &mut f32,
) {
    for name in BODY_JOINTS {
        let Some(&joint) = rig.names.get(name) else {
            continue;
        };
        let matrices = rig.world_with(&pose.rotations, pelvis, &pose.positions, scale);
        let (_, world_q, pivot) = matrices[joint].to_scale_rotation_translation();
        let parent_q = rig.parent[joint].map_or(Quat::IDENTITY, |p| matrices[p].to_scale_rotation_translation().1);
        let mut h = glam::Mat3::from_diagonal(Vec3::splat(BODY_POSE_WEIGHT));
        let mut gradient = -BODY_POSE_WEIGHT * (parent_q * rotation_delta(reference[joint], pose.rotations[joint]));
        for goal in pins {
            let path = chain(rig, goal.bone, goal.root);
            let drives = goal.mode == Mode::Ik && limb(rig, goal.bone) && path.as_ref().is_some_and(|p| p.contains(&joint));
            let mut descendant = Some(goal.bone);
            while let Some(j) = descendant {
                if j == joint {
                    break;
                }
                descendant = rig.parent[j];
            }
            if descendant.is_none() {
                continue;
            }
            let resolved = goal.follow.as_ref().and_then(|f| f.resolve(goal, rig, pose, pelvis, scale));
            if goal.follow.is_some() && resolved.is_none() {
                continue;
            }
            let g = resolved.as_ref().unwrap_or(goal);
            let contact = matrices[g.bone].transform_point3(g.contact);
            let mut v = contact - pivot;
            let mut normal = None;
            if g.mode == Mode::Ik && limb(rig, g.bone) {
                let Some(middle) = rig.parent[g.bone] else {
                    continue;
                };
                let Some(root) = rig.parent[middle] else {
                    continue;
                };
                let p = matrices[root].w_axis.truncate();
                let b = matrices[middle].w_axis.truncate();
                let c = if g.rotation.is_some() {
                    matrices[g.bone].w_axis.truncate()
                } else {
                    contact
                };
                let target = g.rotation.map_or(g.position, |q| {
                    g.position - q * (g.contact * scale.get(g.bone).copied().unwrap_or(rig.scale[g.bone]))
                });
                let (upper, lower, distance) = (p.distance(b), b.distance(c), p.distance(target));
                if distance < upper + lower - 0.005 && distance > (upper - lower).abs() + 0.005 {
                    continue;
                }
                v = p - pivot;
                normal = (target - p).try_normalize();
            }
            let columns = Vec3::AXES.map(|axis| {
                let j = axis.cross(v);
                normal.map_or(j, |n| n * n.dot(j))
            });
            let jacobian = glam::Mat3::from_cols(columns[0], columns[1], columns[2]);
            h += jacobian.transpose() * jacobian;
            if drives {
                gradient += jacobian.transpose() * (g.position - contact);
            }
        }
        let delta = h.inverse() * gradient;
        if !delta.is_finite() || delta.length_squared() < 0.00000001 {
            continue;
        }
        let delta = delta.clamp_length_max(0.25);
        let before = pose.clone();
        let mut accepted = false;
        for fraction in [1.0, 0.5] {
            pose.clone_from(&before);
            pose.rotations[joint] = (parent_q.inverse() * Quat::from_scaled_axis(delta * fraction) * world_q).normalize();
            for g in pins {
                if g.mode == Mode::Ik && limb(rig, g.bone) && chain(rig, g.bone, g.root).is_some_and(|p| p.len() > 3) {
                    apply_goal(
                        rig,
                        pose,
                        pelvis,
                        scale,
                        &Goal {
                            root: default_root(rig, g.bone),
                            ..(*g).clone()
                        },
                    );
                } else {
                    apply_goal(rig, pose, pelvis, scale, g);
                }
            }
            let candidate = score(pose);
            if candidate < *error {
                *error = candidate;
                accepted = true;
                break;
            }
        }
        if !accepted {
            *pose = before;
        }
    }
}

pub(super) fn reconcile(
    rig: &Rig,
    pose: &mut LocalPose,
    pelvis: Vec3,
    scale: &[Vec3],
    tasks: &[(&Goal, f32)],
    order: &[usize],
    continuity: &mut Continuity,
) {
    let pins: Vec<_> = order
        .iter()
        .filter_map(|&i| {
            let (g, fade) = tasks[i];
            (g.weight == 1.0 && fade == 1.0 && g.bone < rig.len() && g.position.is_finite() && g.contact.is_finite()).then_some(g)
        })
        .collect();
    if pins.is_empty()
        || tasks.len() < 2
        || !tasks
            .iter()
            .any(|(g, _)| g.mode == Mode::Ik && limb(rig, g.bone) && chain(rig, g.bone, g.root).is_some_and(|p| p.len() > 3))
    {
        continuity.clear();
        return;
    }
    let source = pose.rotations.clone();
    let chains: Vec<_> = pins.iter().map(|g| (g.bone, g.root)).collect();
    let coherent = pins.len() > 1 && pins.len() == tasks.len();
    if coherent && continuity.chains == chains {
        for &(joint, correction) in &continuity.corrections {
            let correction = Quat::IDENTITY.slerp(correction, continuity.retention);
            pose.rotations[joint] = (correction * source[joint]).normalize();
        }
        // Reproject the contacts after carrying only the body corrections into
        // the new animation frame. Individual limbs still start from that frame.
        for g in &pins {
            if g.mode == Mode::Ik && limb(rig, g.bone) && chain(rig, g.bone, g.root).is_some_and(|path| path.len() > 3) {
                apply_goal(
                    rig,
                    pose,
                    pelvis,
                    scale,
                    &Goal {
                        root: default_root(rig, g.bone),
                        ..(*g).clone()
                    },
                );
            } else {
                apply_goal(rig, pose, pelvis, scale, g);
            }
        }
    }
    // Penalize movement away from the nearby solution, rather than pulling
    // redundant joints back to the raw animation when a contact axis frees up.
    let reference = pose.rotations.clone();
    let score = |pose: &LocalPose| {
        let matrices = rig.world_with(&pose.rotations, pelvis, &pose.positions, scale);
        let contact_error = pins
            .iter()
            .map(|g| {
                let resolved = g.follow.as_ref().and_then(|f| f.resolve(g, rig, pose, pelvis, scale));
                if g.follow.is_some() && resolved.is_none() {
                    return 0.0;
                }
                let g = resolved.as_ref().unwrap_or(g);
                let (_, q, _) = matrices[g.bone].to_scale_rotation_translation();
                matrices[g.bone].transform_point3(g.contact).distance_squared(g.position)
                    + g.rotation.map_or(0.0, |target| 0.02 * (1.0 - q.dot(target).abs().min(1.0)).powi(2))
            })
            .sum::<f32>();
        let body_error = BODY_JOINTS
            .iter()
            .filter_map(|name| rig.names.get(*name))
            .map(|&joint| rotation_delta(reference[joint], pose.rotations[joint]).length_squared())
            .sum::<f32>();
        contact_error + BODY_POSE_WEIGHT * body_error
    };
    let mut error = score(pose);
    let mut best = pose.clone();
    for _ in 0..(64 / pins.len()).clamp(1, 16) {
        // A millimetre-sized dead zone switches body assistance on abruptly
        // near a straight arm: a tiny source motion then snaps its bend angle.
        // Stop at numerical contact precision instead of a visible reach gap.
        if error < 0.0000000001 {
            break;
        }
        let prior = error;
        balance(rig, pose, pelvis, scale, &reference, &pins, &score, &mut error);
        best.clone_from(pose);
        if pins.len() == 1 {
            apply_goal(rig, pose, pelvis, scale, pins[0]);
        }
        let candidate = score(pose);
        if candidate < error {
            error = candidate;
            best.clone_from(pose);
        }
        pose.clone_from(&best);
        if prior - error < 0.000000000001 {
            break;
        }
    }
    *pose = best;
    continuity.clear();
    if coherent {
        continuity.chains = chains;
        for name in BODY_JOINTS {
            let Some(&joint) = rig.names.get(name) else { continue };
            if pins
                .iter()
                .any(|g| g.mode == Mode::Ik && limb(rig, g.bone) && chain(rig, g.bone, g.root).is_some_and(|path| path.contains(&joint)))
            {
                continuity
                    .corrections
                    .push((joint, (pose.rotations[joint] * source[joint].inverse()).normalize()));
            }
        }
    }
}

#[cfg(test)]
mod tests;
