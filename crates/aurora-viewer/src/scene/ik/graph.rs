//! Same-frame rig targets. Own bones (including attachment anchors) resolve
//! from the working local pose, never from last frame's solved IK palette.
use super::*;
use crate::{
    scene::Scene,
    world::{
        World,
        hand_ik::Seat,
        rig_graph::{Definition, Target, Transform},
    },
};
use glam::Mat4;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Clone, Debug)]
enum Anchor {
    Fixed(Transform),
    Bone { bone: usize, offset: Transform },
}

#[derive(Clone, Debug)]
pub struct Context {
    pub definition: Arc<Definition>,
    anchors: HashMap<Target, Anchor>,
    pub time: f64,
    model: Mat4,
    source: Option<Seat>,
    external: bool,
    subject: Uuid,
    activation: Option<crate::world::hand_ik::ActivationStamp>,
    smoothing: Option<[f32; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ActivationKey {
    source: Option<Seat>,
    session: Uuid,
    epoch: Uuid,
    bundle: Uuid,
    cycle: u32,
}

pub fn rigid(matrix: Mat4) -> Option<Transform> {
    let (_, q, p) = matrix.to_scale_rotation_translation();
    (q.is_finite() && p.is_finite() && q.length_squared() > 0.01).then(|| Transform::new(p, q))
}

fn joint(world: &World, avatar: Uuid, bone: usize) -> Option<Mat4> {
    Some(*world.avatar_poses.get(&avatar)?.get(bone)? * world.avatar_lib.rig.default_world_inv.get(bone)?.inverse())
}

fn attachment_joint(world: &World, subject: Uuid, index: usize) -> Option<usize> {
    let mut o = world.objects.get(index)?;
    for _ in 0..17 {
        let parent = world.objects.get(world.objects.parent_of(o)?)?;
        if parent.is_avatar() {
            return (parent.full_id == subject)
                .then(|| world.avatar_lib.attach_points.get(&o.attachment_point()).and_then(|p| p.joint))
                .flatten();
        }
        o = parent;
    }
    None
}

impl Context {
    pub fn external(&self) -> bool {
        self.external
    }
    pub fn refreshed(&self, world: &World, now: Instant) -> Arc<Self> {
        Arc::new(
            Self::new_for_avatar(
                world,
                self.subject,
                self.model,
                self.definition.clone(),
                self.source,
                self.time,
                now,
            )
            .activated(self.activation.clone())
            .smoothed(self.smoothing),
        )
    }
    pub(super) fn activated(mut self, activation: Option<crate::world::hand_ik::ActivationStamp>) -> Self {
        self.activation = activation;
        self
    }
    pub(super) fn smoothed(mut self, smoothing: Option<[f32; 2]>) -> Self {
        self.smoothing = smoothing;
        self
    }
    pub fn new(world: &World, model: Mat4, definition: Arc<Definition>, source: Option<Seat>, time: f64, now: Instant) -> Self {
        Self::new_for_avatar(world, world.agent_id, model, definition, source, time, now)
    }
    pub fn new_for_avatar(
        world: &World,
        subject: Uuid,
        model: Mat4,
        definition: Arc<Definition>,
        source: Option<Seat>,
        time: f64,
        now: Instant,
    ) -> Self {
        let mut anchors = HashMap::new();
        let rig = &world.avatar_lib.rig;
        let inverse = model.inverse();
        for target in definition
            .nodes
            .iter()
            .map(|n| &n.parent)
            .chain(definition.bindings.iter().map(|b| &b.target))
        {
            if anchors.contains_key(target) {
                continue;
            }
            let anchor = (|| match target {
                Target::Node { .. } => None,
                Target::Bone { avatar, bone } => {
                    let &bone = rig.names.get(bone)?;
                    if avatar.is_nil() || *avatar == subject {
                        return Some(Anchor::Bone {
                            bone,
                            offset: Transform::default(),
                        });
                    }
                    let index = world.objects.index_of_uuid(avatar)?;
                    let object = world.objects.get(index)?;
                    if !object.is_avatar() {
                        return None;
                    }
                    let (p, q, false) = Scene::object_transform(world, index, now, 0)? else {
                        return None;
                    };
                    let other = Mat4::from_rotation_translation(q, p) * Mat4::from_translation(-world.avatar_lib.pelvis);
                    Some(Anchor::Fixed(rigid(inverse * other * joint(world, *avatar, bone)?)?))
                }
                Target::Link { .. } | Target::Prim { .. } | Target::Source => {
                    let index = match target {
                        Target::Link { object, link } => world.ik_targets.index(world, source, *object, link)?,
                        Target::Prim { id, .. } => world.objects.index_of_uuid(id)?,
                        _ => world.objects.index_of_uuid(&source?.root)?,
                    };
                    if let (Target::Prim { name, .. }, Some(source)) = (target, source)
                        && !name.is_empty()
                        && crate::world::hand_ik::target_index(world, source, world.objects.get(index)?.full_id).is_none()
                    {
                        return None;
                    }
                    if world.objects.get(index)?.is_avatar() {
                        return None;
                    }
                    let (p, q, false) = Scene::object_transform(world, index, now, 0)? else {
                        return None;
                    };
                    // Rigid metric frame: a prim's scale must not resize offsets.
                    let (_, mq, mp) = model.to_scale_rotation_translation();
                    let transform = Transform::new(mq.inverse() * (p - mp), mq.inverse() * q);
                    if let Some(bone) = attachment_joint(world, subject, index) {
                        let frame = rigid(joint(world, subject, bone)?)?;
                        Some(Anchor::Bone {
                            bone,
                            offset: frame.inverse().compose(transform),
                        })
                    } else {
                        Some(Anchor::Fixed(transform))
                    }
                }
            })();
            if let Some(anchor) = anchor {
                anchors.insert(target.clone(), anchor);
            }
        }
        let external = definition
            .nodes
            .iter()
            .map(|n| &n.parent)
            .chain(definition.bindings.iter().map(|b| &b.target))
            .any(|t| match t {
                Target::Bone { avatar, .. } => !avatar.is_nil() && *avatar != subject,
                Target::Prim { id, .. } => world
                    .objects
                    .index_of_uuid(id)
                    .and_then(|i| Scene::wearer_avatar(world, i))
                    .is_some_and(|(id, _)| id != subject),
                Target::Link { object, link } => world
                    .ik_targets
                    .index(world, source, *object, link)
                    .and_then(|i| Scene::wearer_avatar(world, i))
                    .is_some_and(|(id, _)| id != subject),
                _ => false,
            });
        Self {
            definition,
            anchors,
            time,
            model,
            source,
            external,
            subject,
            activation: None,
            smoothing: None,
        }
    }

    pub fn target(&self, target: &Target, matrices: &[Mat4]) -> Option<Transform> {
        self.resolve(target, matrices, 0)
    }
    fn resolve(&self, target: &Target, matrices: &[Mat4], depth: usize) -> Option<Transform> {
        if depth > crate::world::rig_graph::MAX_NODES {
            return None;
        }
        if let Target::Node { id } = target {
            let node = self.definition.nodes.iter().find(|n| n.id == *id)?;
            let base = self.resolve(&node.parent, matrices, depth + 1)?.compose(node.offset);
            return Some(base.compose(node.motion.as_ref().map_or_else(Transform::default, |m| m.sample(self.time))));
        }
        match self.anchors.get(target)? {
            Anchor::Fixed(frame) => Some(*frame),
            Anchor::Bone { bone, offset } => Some(rigid(*matrices.get(*bone)?)?.compose(*offset)),
        }
    }
    fn dependencies(&self, target: &Target, depth: usize, out: &mut Vec<usize>) {
        if depth > crate::world::rig_graph::MAX_NODES {
            return;
        }
        if let Target::Node { id } = target {
            if let Some(node) = self.definition.nodes.iter().find(|n| n.id == *id) {
                self.dependencies(&node.parent, depth + 1, out);
            }
        } else if let Some(Anchor::Bone { bone, .. }) = self.anchors.get(target) {
            out.push(*bone);
        }
    }
    pub fn goals(self: &Arc<Self>, rig: &Rig) -> Vec<Goal> {
        self.definition
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(index, b)| {
                if !b.enabled {
                    return None;
                }
                let &bone = rig.names.get(&b.bone)?;
                let root = match (b.mode, &b.root) {
                    (Mode::Direct, _) => bone,
                    (_, Some(name)) => *rig.names.get(name)?,
                    (_, None) => default_root(rig, bone),
                };
                if b.mode == Mode::Ik && chain(rig, bone, root).is_none() {
                    return None;
                }
                Some(Goal {
                    bone,
                    root,
                    position: Vec3::ZERO,
                    rotation: None,
                    contact: Vec3::from_array(b.contact),
                    pole: None,
                    mode: b.mode,
                    weight: b.weight,
                    follow: Some(Follow {
                        context: self.clone(),
                        index,
                    }),
                })
            })
            .collect()
    }
    pub fn preview(&self, goal: &Goal, matrices: &[Mat4]) -> Option<Goal> {
        let binding = self.definition.bindings.get(goal.follow.as_ref()?.index)?;
        let target = self.target(&binding.target, matrices)?;
        let transform = target.compose(binding.offset);
        let mut result = goal.clone();
        result.follow = None;
        result.position = transform.p();
        result.rotation = binding.orient.then_some(transform.q());
        result.pole = binding.pole.map(|p| target.p() + target.q() * Vec3::from_array(p));
        Some(result)
    }
}

#[derive(Clone, Debug)]
pub struct Follow {
    pub context: Arc<Context>,
    pub index: usize,
}
impl Follow {
    pub(super) fn same_binding(&self, other: &Self) -> bool {
        self.index == other.index && self.context.source == other.context.source && self.context.subject == other.context.subject
    }
    pub(super) fn activation_key(&self) -> Option<ActivationKey> {
        let stamp = self.context.activation.as_ref()?;
        Some(ActivationKey {
            source: self.context.source,
            session: stamp.session,
            epoch: stamp.epoch,
            bundle: stamp.bundle,
            cycle: *stamp.cycles.get(self.index)?,
        })
    }
    pub(super) fn activation_settings(&self) -> Option<crate::world::rig_graph::Activation> {
        let mut settings = self.context.definition.bindings.get(self.index)?.activation;
        if let Some([ease_in, ease_out]) = self.context.smoothing {
            settings.ease_in = ease_in;
            settings.ease_out = ease_out;
        }
        Some(settings)
    }
    pub(super) fn resolve(&self, goal: &Goal, rig: &Rig, pose: &LocalPose, pelvis: Vec3, scale: &[Vec3]) -> Option<Goal> {
        let matrices = rig.world_with(&pose.rotations, pelvis, &pose.positions, scale);
        self.context.preview(goal, &matrices)
    }
}

fn ancestor(rig: &Rig, a: usize, mut b: usize) -> bool {
    for _ in 0..rig.len() {
        if a == b {
            return true;
        }
        let Some(parent) = rig.parent.get(b).copied().flatten() else {
            return false;
        };
        b = parent;
    }
    false
}
fn writes(rig: &Rig, goal: &Goal, bone: usize) -> bool {
    let root = if goal.mode == Mode::Direct { goal.bone } else { goal.root };
    ancestor(rig, root, bone)
}

/// Stable dependency ordering. Cycles and everything depending on them are
/// omitted; unrelated constraints continue to work.
pub fn order(rig: &Rig, goals: &[&Goal]) -> Vec<usize> {
    let mut dependencies = vec![HashSet::new(); goals.len()];
    for (i, goal) in goals.iter().enumerate() {
        if let Some(follow) = &goal.follow {
            let mut bones = Vec::new();
            follow
                .context
                .dependencies(&follow.context.definition.bindings[follow.index].target, 0, &mut bones);
            for (j, writer) in goals.iter().enumerate() {
                if writer.weight > 0.0 && bones.iter().any(|b| writes(rig, writer, *b)) {
                    dependencies[i].insert(j);
                }
            }
        }
        for (j, parent) in goals.iter().enumerate() {
            if i != j && parent.bone != goal.bone && parent.weight > 0.0 && ancestor(rig, parent.bone, goal.root) {
                dependencies[i].insert(j);
            }
        }
    }
    // Peers are solved as one weighted joint target. All of their anchors
    // must be ready before any peer, including an otherwise independent prim.
    let original = dependencies.clone();
    for (i, goal) in goals.iter().enumerate() {
        for (j, peer) in goals.iter().enumerate() {
            if peer.bone == goal.bone {
                dependencies[i].extend(&original[j]);
            }
        }
    }
    let mut result = Vec::new();
    while let Some(next) = (0..goals.len())
        .filter(|i| !result.contains(i) && dependencies[*i].iter().all(|j| result.contains(j)))
        .min_by_key(|i| depth(rig, goals[*i].root))
    {
        result.push(next);
    }
    result
}

pub fn runtime(world: &World, subject: Uuid, model: Mat4, now: Instant) -> Vec<Goal> {
    world
        .hand_contacts
        .for_avatar(world.agent_id, subject)
        .into_iter()
        .flat_map(|c| c.graphs())
        .filter(|g| crate::world::hand_ik::source_eligible_for(world, subject, g.source))
        .flat_map(|g| {
            let context = Arc::new(
                Context::new_for_avatar(world, subject, model, g.definition.clone(), Some(g.source), g.time(now), now)
                    .activated(Some(g.activation.clone()))
                    .smoothed(g.smoothing),
            );
            context
                .goals(&world.avatar_lib.rig)
                .into_iter()
                .filter(|goal| goal.follow.as_ref().is_some_and(|f| g.bindings.contains(&f.index)))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests;
