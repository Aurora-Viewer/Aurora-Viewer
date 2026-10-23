//! General local skeletal constraints and a reversible frozen-pose editor.
//! Two-link chains use the analytic contact solver; longer chains use bounded
//! CCD over just their ancestor path. No skeleton translations are changed
//! by IK. Direct mode explicitly edits the selected joint's local transform.

use super::{anim::Rig, hand_ik};
pub use crate::world::hand_ik::ConstraintMode as Mode;
use glam::{Quat, Vec3};
use std::time::Instant;
mod activation;
pub mod animations;
mod body;
pub mod graph;
mod multiple;

pub fn targets(world: &crate::world::World, avatar: usize, dz: f32, now: Instant) -> Vec<Goal> {
    let Some(subject) = world.objects.get(avatar).map(|o| o.full_id) else {
        return Vec::new();
    };
    let Some((p, r, _)) = super::Scene::object_transform_unshifted(world, avatar, now) else {
        return Vec::new();
    };
    let height = Vec3::Z * (dz + world.appearance_hover(&subject));
    let seated = world.objects.get(avatar).is_some_and(|o| o.parent_id != 0);
    let p = p + if seated { r * height } else { height };
    let model = glam::Mat4::from_rotation_translation(r, p) * glam::Mat4::from_translation(-world.avatar_lib.pelvis);
    graph::runtime(world, subject, model, now)
}

#[derive(Clone, Debug)]
pub struct Goal {
    pub follow: Option<graph::Follow>,
    /// Authored strength, independent of the runtime arrival/release fade.
    pub weight: f32,
    pub bone: usize,
    pub root: usize,
    pub position: Vec3,
    pub rotation: Option<Quat>,
    pub contact: Vec3,
    /// Bend-plane target in avatar axes, independent of the effector contact.
    pub pole: Option<Vec3>,
    pub mode: Mode,
}

impl Goal {
    fn same_binding(&self, other: &Self) -> bool {
        self.bone == other.bone
            && match (&self.follow, &other.follow) {
                (Some(a), Some(b)) => a.same_binding(b),
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Clone, Default)]
pub struct EditRequest {
    pub active: bool,
    pub goals: Vec<Goal>,
    /// Authored local channels are applied before explicit target constraints.
    pub joints: Vec<JointEdit>,
    /// Explicitly released bones suppress the corresponding live contacts.
    pub released: Vec<usize>,
    pub animations: Vec<animations::Sample>,
    pub suppress_live: bool,
    /// Locally hide script contacts while retaining their runtime state.
    pub suppress_script_ik: bool,
}

#[derive(Clone)]
pub struct JointEdit {
    pub bone: usize,
    pub rotation: Option<Quat>,
    /// Local displacement from the sampled/frozen animation, including root offsets.
    pub translation: Option<Vec3>,
    pub weight: f32,
}

#[derive(Clone)]
pub struct LocalPose {
    pub rotations: Vec<Quat>,
    pub positions: Vec<Vec3>,
}

#[derive(Default)]
pub struct Pose {
    pub targets: Vec<Goal>,
    pub edit: EditRequest,
    held: Vec<activation::Held>,
    frozen: Option<LocalPose>,
    rest_positions: Vec<Vec3>,
    at: Option<Instant>,
    was_paused: bool,
    body_continuity: body::Continuity,
}

impl Pose {
    pub fn has_external_targets(&self) -> bool {
        self.targets
            .iter()
            .chain(self.edit.goals.iter())
            .chain(self.held.iter().map(|held| &held.goal))
            .any(|g| g.follow.as_ref().is_some_and(|f| f.context.external()))
    }
    pub fn refresh_external_targets(&mut self, world: &crate::world::World, now: Instant) {
        let mut contexts = std::collections::HashMap::new();
        for goal in self
            .targets
            .iter_mut()
            .chain(self.edit.goals.iter_mut())
            .chain(self.held.iter_mut().map(|held| &mut held.goal))
        {
            if let Some(follow) = &mut goal.follow
                && follow.context.external()
            {
                follow.context = contexts
                    .entry(std::sync::Arc::as_ptr(&follow.context) as usize)
                    .or_insert_with(|| follow.context.refreshed(world, now))
                    .clone();
            }
        }
    }
    pub fn set_rest_positions(&mut self, positions: &[Vec3]) {
        self.rest_positions.clear();
        self.rest_positions.extend_from_slice(positions);
    }
    #[allow(clippy::too_many_arguments)] // Same pose inputs as Controller::evaluate.
    pub fn apply(
        &mut self,
        rig: &Rig,
        rotations: &[Quat],
        positions: &[Vec3],
        pelvis: Vec3,
        scale: &[Vec3],
        now: Instant,
        paused: bool,
    ) -> Option<LocalPose> {
        let elapsed = self.at.map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f32());
        self.at = Some(now);
        let elapsed = if (paused && !self.edit.active) || (!paused && self.was_paused) {
            0.0
        } else {
            elapsed
        };
        self.was_paused = paused;
        self.body_continuity.begin_frame(self.edit.active, elapsed.min(0.1));
        if !self.edit.active {
            self.frozen = None;
            self.rest_positions.clear();
        }
        if self.targets.is_empty() && self.held.is_empty() && !self.edit.active {
            self.body_continuity.clear();
            return None;
        }
        let mut result = LocalPose {
            rotations: rotations.to_vec(),
            positions: positions.to_vec(),
        };
        if self.edit.active {
            // Snapshot animation inputs, never already solved IK. Otherwise
            // disabling a contact bakes its correction into the frozen base.
            result = self.frozen.get_or_insert_with(|| result.clone()).clone();
            if self.edit.suppress_live {
                result.rotations.clone_from(&rig.local_rot);
                result.positions.clone_from(if self.rest_positions.len() == rig.len() {
                    &self.rest_positions
                } else {
                    &rig.local_pos
                });
                result.positions[rig.pelvis] -= pelvis;
            }
            if !self.edit.animations.is_empty() {
                animations::apply(rig, &mut result, pelvis, &self.edit.animations);
            }
            for joint in &self.edit.joints {
                if joint.bone >= rig.len() || !joint.weight.is_finite() || !(0.0..=1.0).contains(&joint.weight) {
                    continue;
                }
                if let Some(q) = joint
                    .rotation
                    .filter(|q| q.is_finite() && (0.9..=1.1).contains(&q.length_squared()))
                {
                    result.rotations[joint.bone] = result.rotations[joint.bone].slerp(q.normalize(), joint.weight).normalize();
                }
                if let Some(p) = joint.translation.filter(|p| p.is_finite()) {
                    result.positions[joint.bone] += p * joint.weight;
                }
            }
        }
        let source = result.clone();
        for goal in &self.targets {
            if !self.held.iter().any(|held| held.goal.same_binding(goal)) {
                self.held.push(activation::Held::new(goal));
            }
        }
        for held in &mut self.held {
            let incoming = self.targets.iter().find(|g| g.same_binding(&held.goal));
            if held.update(incoming, rig, &source, pelvis, scale, elapsed) {
                self.body_continuity.clear();
            }
        }
        self.held.retain(activation::Held::retained);
        if self.held.is_empty() && !self.edit.active {
            self.body_continuity.clear();
            return None;
        }
        let edit = &self.edit;
        let visible = |goal: &Goal| {
            goal.weight > 0.0
                && !(edit.active
                    && (edit.suppress_script_ik || edit.released.contains(&goal.bone) || edit.goals.iter().any(|g| g.bone == goal.bone)))
        };
        let mut tasks: Vec<_> = self
            .held
            .iter()
            .filter(|held| held.weight > 0.0 && visible(&held.goal))
            .map(|held| (&held.goal, held.weight))
            .collect();
        if self.edit.active {
            tasks.extend(self.edit.goals.iter().map(|g| (g, 1.0)));
        }
        let releasing: Vec<_> = self
            .held
            .iter()
            .filter(|held| visible(&held.goal))
            .filter_map(activation::Held::peer_release)
            .filter(|(g, _)| tasks.iter().any(|(active, _)| active.bone == g.bone))
            .collect();
        tasks.extend(releasing);
        activation::release(
            rig,
            &self.held.iter().filter(|held| visible(&held.goal)).collect::<Vec<_>>(),
            &tasks,
            &source,
            &mut result,
        );
        let refs: Vec<_> = tasks.iter().map(|t| t.0).collect();
        let order = graph::order(rig, &refs);
        let collective = order
            .iter()
            .filter(|&&i| tasks[i].0.weight == 1.0 && tasks[i].1 == 1.0)
            .map(|&i| tasks[i].0.bone)
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1
            && order.iter().any(|&i| {
                let g = tasks[i].0;
                g.mode == Mode::Ik && body::limb(rig, g.bone) && chain(rig, g.bone, g.root).is_some_and(|p| p.len() > 3)
            });
        let mut applied = Vec::with_capacity(order.len());
        let mut combined: Vec<(Goal, f32)> = Vec::with_capacity(order.len());
        for &index in &order {
            if combined.iter().any(|(g, _)| g.bone == tasks[index].0.bone) {
                continue;
            }
            let peers: Vec<_> = order
                .iter()
                .filter(|&&i| tasks[i].0.bone == tasks[index].0.bone)
                .map(|&i| tasks[i])
                .collect();
            let mut merged = if peers.len() > 1 {
                let Some(goal) = multiple::combine(rig, &result, pelvis, scale, &peers) else {
                    continue;
                };
                (goal, 1.0)
            } else {
                (tasks[index].0.clone(), tasks[index].1)
            };
            protect_parent_contacts(rig, &mut merged.0, &combined, &applied);
            let index = combined.len();
            combined.push(merged);
            let tasks = &combined;
            let (goal, weight) = (&tasks[index].0, tasks[index].1);
            let before = result.clone();
            // Solve shared body joints together, rather than seeding the body
            // with competing per-hand coarse solutions before reconciliation.
            let confined = (collective
                && goal.weight == 1.0
                && weight == 1.0
                && goal.mode == Mode::Ik
                && body::limb(rig, goal.bone)
                && chain(rig, goal.bone, goal.root).is_some_and(|p| p.len() > 3))
            .then(|| Goal {
                root: default_root(rig, goal.bone),
                ..goal.clone()
            });
            if !apply_goal(rig, &mut result, pelvis, scale, confined.as_ref().unwrap_or(goal)) {
                continue;
            }
            applied.push(goal.bone);
            if weight < 1.0 {
                for (j, q) in result.rotations.iter_mut().enumerate() {
                    *q = before.rotations[j].slerp(*q, weight).normalize();
                    result.positions[j] = before.positions[j].lerp(result.positions[j], weight);
                }
            }
        }
        let tasks: Vec<_> = combined.iter().map(|(g, weight)| (g, *weight)).collect();
        let order: Vec<_> = (0..tasks.len()).collect();
        body::reconcile(rig, &mut result, pelvis, scale, &tasks, &order, &mut self.body_continuity);
        if self.held.iter().any(activation::Held::waiting) {
            // A parent's active pin can bring a child into range. Waiting
            // contacts have not contributed any correction to this pose.
            let matrices = rig.world_with(&result.rotations, pelvis, &result.positions, scale);
            for held in &mut self.held {
                if visible(&held.goal) && held.arrive_after_contacts(&matrices) {
                    self.body_continuity.clear();
                }
            }
        }
        for held in &mut self.held {
            // Temporary authoring channels never enter a script's release cache.
            if !edit.active && visible(&held.goal) && applied.contains(&held.goal.bone) {
                held.capture(rig, &source, &result);
            }
        }
        Some(result)
    }
}

/// A child may use its requested ancestry only while it preserves already
/// solved parents whose authored influence is full. Their runtime fade still
/// owns its frame throughout entry, avoiding a change of chains at fade end.
/// Position pins allow rotation at their origin; oriented/off-origin contacts
/// retain the complete parent frame.
fn protect_parent_contacts(rig: &Rig, goal: &mut Goal, solved: &[(Goal, f32)], applied: &[usize]) {
    if goal.mode != Mode::Ik {
        return;
    }
    let Some(path) = chain(rig, goal.bone, goal.root) else { return };
    let mut root = 0;
    for (parent, fade) in solved {
        if parent.bone == goal.bone || parent.weight != 1.0 || *fade <= 0.0 || !applied.contains(&parent.bone) {
            continue;
        }
        if let Some(index) = path.iter().position(|&bone| bone == parent.bone) {
            let oriented = parent.rotation.is_some()
                || parent
                    .follow
                    .as_ref()
                    .is_some_and(|f| f.context.definition.bindings[f.index].orient);
            root = root.max(index + usize::from(oriented || parent.contact.length_squared() > 0.0));
        }
    }
    goal.root = path[root.min(path.len() - 1)];
}

fn depth(rig: &Rig, mut bone: usize) -> usize {
    let mut depth = 0;
    while let Some(parent) = rig.parent.get(bone).copied().flatten() {
        depth += 1;
        bone = parent;
    }
    depth
}

pub fn default_root(rig: &Rig, bone: usize) -> usize {
    let parent = rig.parent.get(bone).copied().flatten().unwrap_or(bone);
    rig.parent
        .get(parent)
        .copied()
        .flatten()
        .filter(|j| *j != rig.pelvis)
        .unwrap_or(parent)
}

/// The four limb switches opt into body assistance; explicit roots on other
/// bindings remain creator-controlled, including same-avatar bone targets.
pub fn extended_root(rig: &Rig, bone: usize) -> Option<usize> {
    if !body::limb(rig, bone) {
        return None;
    }
    let name = if rig.canonical_names[bone].starts_with("mWrist") {
        "mTorso"
    } else {
        "mPelvis"
    };
    rig.names
        .get(name)
        .copied()
        .filter(|root| chain(rig, bone, *root).is_some_and(|p| p.len() > 3))
}

pub fn supports_pole(rig: &Rig, bone: usize, root: usize) -> bool {
    chain(rig, bone, root).is_some_and(|p| p.len() == 3 || (p.len() > 3 && body::limb(rig, bone)))
}

/// Root to effector, limited to twelve joints; an invalid ancestry is rejected.
pub fn chain(rig: &Rig, bone: usize, root: usize) -> Option<Vec<usize>> {
    if bone >= rig.len() || root >= rig.len() {
        return None;
    }
    let mut path = vec![bone];
    while *path.last()? != root {
        if path.len() >= 12 {
            return None;
        }
        path.push(rig.parent[*path.last()?]?);
    }
    path.reverse();
    Some(path)
}

pub fn apply_goal(rig: &Rig, pose: &mut LocalPose, pelvis: Vec3, scale: &[Vec3], goal: &Goal) -> bool {
    let resolved;
    let goal = if let Some(follow) = &goal.follow {
        let Some(value) = follow.resolve(goal, rig, pose, pelvis, scale) else {
            return false;
        };
        resolved = value;
        &resolved
    } else {
        goal
    };
    if !goal.weight.is_finite() || !(0.0..=1.0).contains(&goal.weight) {
        return false;
    }
    if goal.weight == 0.0 {
        return true;
    }
    if goal.weight == 1.0 {
        return solve_goal(rig, pose, pelvis, scale, goal);
    }
    let before = pose.clone();
    if !solve_goal(rig, pose, pelvis, scale, goal) {
        *pose = before;
        return false;
    }
    // Blend local transforms, like the animation mixer, to retain segment
    // lengths. Interpolating the world target would create a second pin.
    for (j, rotation) in pose.rotations.iter_mut().enumerate() {
        *rotation = before.rotations[j].slerp(*rotation, goal.weight).normalize();
        pose.positions[j] = before.positions[j].lerp(pose.positions[j], goal.weight);
    }
    true
}

fn solve_goal(rig: &Rig, pose: &mut LocalPose, pelvis: Vec3, scale: &[Vec3], goal: &Goal) -> bool {
    if goal.bone >= rig.len()
        || !goal.position.is_finite()
        || !goal.contact.is_finite()
        || goal.pole.is_some_and(|p| !p.is_finite())
        || goal
            .rotation
            .is_some_and(|q| !q.is_finite() || !(0.9..=1.1).contains(&q.length_squared()))
    {
        return false;
    }
    let matrices = rig.world_with(&pose.rotations, pelvis, &pose.positions, scale);
    let (_, current_q, _) = matrices[goal.bone].to_scale_rotation_translation();
    let contact = goal.contact * scale.get(goal.bone).copied().unwrap_or(rig.scale[goal.bone]);
    if !current_q.is_finite() || !contact.is_finite() {
        return false;
    }
    if goal.mode == Mode::Direct {
        let (parent_p, parent_q, parent_scale) = match rig.parent[goal.bone] {
            Some(parent) => {
                let (_, q, p) = matrices[parent].to_scale_rotation_translation();
                (p, q, scale.get(parent).copied().unwrap_or(rig.scale[parent]))
            }
            None => (Vec3::ZERO, Quat::IDENTITY, rig.parent_scale[goal.bone]),
        };
        if parent_scale.abs().min_element() < 0.00001 || !parent_q.is_finite() {
            return false;
        }
        let q = goal.rotation.unwrap_or(current_q).normalize();
        let p = goal.position - q * contact;
        let extra = if goal.bone == rig.pelvis { pelvis } else { Vec3::ZERO };
        let local = (parent_q.inverse() * (p - parent_p) - extra) / parent_scale;
        if !local.is_finite() {
            return false;
        }
        pose.positions[goal.bone] = local;
        pose.rotations[goal.bone] = (parent_q.inverse() * q).normalize();
        return true;
    }
    let Some(path) = chain(rig, goal.bone, goal.root) else {
        return false;
    };
    if goal.pole.is_some() && !supports_pole(rig, goal.bone, goal.root) {
        return false;
    }
    let mut joints: Vec<_> = path
        .iter()
        .map(|&j| {
            let (_, q, p) = matrices[j].to_scale_rotation_translation();
            (p, q)
        })
        .collect();
    if joints.iter().any(|(p, q)| !p.is_finite() || !q.is_finite()) {
        return false;
    }
    let parent_q = rig.parent[goal.root].map_or(Quat::IDENTITY, |p| matrices[p].to_scale_rotation_translation().1);
    let extended = path.len() > 3 && body::limb(rig, goal.bone);
    if (path.len() == 3 || extended) && body::analytic(rig, pose, &path, &joints, parent_q, goal, contact, extended) {
        return true;
    }
    if extended {
        body::assist(rig, pose, &path, &mut joints, parent_q, goal, contact);
        if body::analytic(rig, pose, &path, &joints, parent_q, goal, contact, true) {
            return true;
        }
    }
    let last = joints.len() - 1;
    let target = goal.rotation.map_or(goal.position, |q| goal.position - q.normalize() * contact);
    // Only this short ancestor path is updated inside CCD; the whole skeleton
    // is evaluated once per goal, never once per CCD step.
    for _ in 0..16 {
        let end = if goal.rotation.is_some() {
            joints[last].0
        } else {
            joints[last].0 + joints[last].1 * contact
        };
        if end.distance_squared(target) < 0.000001 {
            break;
        }
        for index in (0..last).rev() {
            // Bento's alternating spine offsets cancel in the rest skeleton.
            // Preserve their animation; body assistance uses torso/chest/collar.
            if extended && rig.canonical_names[path[index]].starts_with("mSpine") {
                continue;
            }
            let pivot = joints[index].0;
            let end = if goal.rotation.is_some() {
                joints[last].0
            } else {
                joints[last].0 + joints[last].1 * contact
            };
            let (Some(from), Some(to)) = ((end - pivot).try_normalize(), (target - pivot).try_normalize()) else {
                continue;
            };
            let delta = Quat::from_rotation_arc(from, to);
            let angle = delta.to_axis_angle().1.abs();
            let delta = Quat::IDENTITY.slerp(delta, (0.5 / angle.max(0.5)).min(1.0)).normalize();
            let parent = if index == 0 { parent_q } else { joints[index - 1].1 };
            pose.rotations[path[index]] = (parent.inverse() * delta * joints[index].1).normalize();
            for joint in &mut joints[index..] {
                joint.0 = pivot + delta * (joint.0 - pivot);
                joint.1 = (delta * joint.1).normalize();
            }
        }
    }
    if let Some(q) = goal.rotation {
        let parent = if last == 0 { parent_q } else { joints[last - 1].1 };
        pose.rotations[goal.bone] = (parent.inverse() * q).normalize();
    }
    if extended {
        return body::analytic(rig, pose, &path, &joints, parent_q, goal, contact, false);
    }
    true
}

#[cfg(test)]
mod tests;
