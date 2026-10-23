//! Authoring state for real-bone targets and virtual controls. The definition
//! and evaluator are shared with the scripted runtime, including cycle rules.
use super::*;
use crate::world::rig_graph::{self as data, Binding, Definition, Node, Target, Transform};
use std::sync::Arc;
mod motion;
#[cfg(test)]
mod tests;

pub(super) struct State {
    pub definition: Definition,
    pub(super) source: Option<Uuid>,
    pub selected: Option<Uuid>,
    /// Association identity is its slot, independently of its source bone.
    pub binding: Option<usize>,
    pub profile: Option<usize>,
    pub(super) target: Target,
    context: Option<Arc<ik::graph::Context>>,
    matrices: Vec<Mat4>,
    rig: Option<Arc<Rig>>,
    pub time: f64,
    playing: bool,
    at: Option<Instant>,
    pub(super) imported: bool,
    auto_key: bool,
    key: usize,
    node_tab: u8,
    pub(super) reveal_curve: bool,
    pub error: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            definition: Definition::default(),
            source: None,
            selected: None,
            binding: None,
            profile: None,
            target: Target::Prim {
                id: Uuid::nil(),
                name: String::new(),
            },
            context: None,
            matrices: Vec::new(),
            rig: None,
            time: 0.0,
            playing: false,
            at: None,
            imported: true,
            auto_key: false,
            key: 0,
            node_tab: 0,
            reveal_curve: false,
            error: false,
        }
    }
}
fn degrees(q: Quat) -> Vec3 {
    let (z, y, x) = q.to_euler(EulerRot::ZYX);
    Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI)
}
fn rotation(v: Vec3) -> Quat {
    Quat::from_euler(EulerRot::ZYX, v.z.to_radians(), v.y.to_radians(), v.x.to_radians())
}

impl State {
    pub(super) fn visible_binding(&self, index: usize) -> bool {
        self.definition.bindings.get(index).is_some_and(|binding| {
            self.profile
                .and_then(|i| self.definition.profiles.get(i))
                .is_none_or(|p| p.contains(index, &binding.bone))
        })
    }
    pub(super) fn binding_index(&self, bone: usize, names: &[String]) -> Option<usize> {
        let name = names.get(bone)?;
        self.binding
            .filter(|&index| self.visible_binding(index) && self.definition.bindings.get(index).is_some_and(|b| &b.bone == name))
            .or_else(|| {
                self.definition
                    .bindings
                    .iter()
                    .enumerate()
                    .position(|(i, b)| self.visible_binding(i) && &b.bone == name)
            })
    }
    pub(super) fn binding_goal(&self, index: usize, rig: &Rig) -> Option<Goal> {
        let b = self.definition.bindings.get(index)?;
        let &bone = rig.names.get(&b.bone)?;
        let target = self.context()?.target(&b.target, &self.matrices)?;
        let transform = target.compose(b.offset);
        Some(Goal {
            follow: None,
            bone,
            root: if b.mode == Mode::Direct {
                bone
            } else {
                b.root
                    .as_ref()
                    .and_then(|n| rig.names.get(n))
                    .copied()
                    .unwrap_or_else(|| ik::default_root(rig, bone))
            },
            position: transform.p(),
            rotation: b.orient.then_some(transform.q()),
            contact: Vec3::from_array(b.contact),
            pole: b.pole.map(|p| target.p() + target.q() * Vec3::from_array(p)),
            mode: b.mode,
            weight: b.weight,
        })
    }
    pub(super) fn binding_target(&self, index: usize) -> Option<Transform> {
        self.context()?.target(&self.definition.bindings.get(index)?.target, &self.matrices)
    }
    pub(super) fn replace_runtime(&mut self, runtime: &crate::world::hand_ik::GraphRuntime, now: Instant) {
        let mut definition = (*runtime.definition).clone();
        let target = |t: &mut Target| {
            if let Target::Link { object, .. } = t
                && object.is_nil()
            {
                *object = runtime.source.root;
            }
            if *t == Target::Source {
                *t = Target::Prim {
                    id: runtime.source.root,
                    name: String::new(),
                };
            }
        };
        for node in &mut definition.nodes {
            target(&mut node.parent);
        }
        for (index, binding) in definition.bindings.iter_mut().enumerate() {
            target(&mut binding.target);
            // A selected preset is transient; keep every declared preset editable.
            if definition.profiles.is_empty() {
                binding.enabled = runtime.bindings.contains(&index);
            }
        }
        *self = Self {
            definition,
            source: Some(runtime.source.root),
            time: runtime.time(now),
            ..Self::default()
        };
    }
    pub(super) fn pause(&mut self) {
        self.playing = false;
    }
    fn refresh(&mut self, world: &World, frame: &Frame) {
        self.rig = Some(world.avatar_lib.rig.clone());
        if !self.imported {
            self.imported = true;
            for runtime in world.hand_contacts.graphs() {
                let mut definition = (*runtime.definition).clone();
                let remap: std::collections::HashMap<_, _> = definition.nodes.iter().map(|n| (n.id, Uuid::new_v4())).collect();
                let target = |t: &mut Target| match t {
                    Target::Link { object, .. } if object.is_nil() => *object = runtime.source.root,
                    Target::Node { id } => {
                        if let Some(new) = remap.get(id) {
                            *id = *new;
                        }
                    }
                    Target::Source => {
                        *t = Target::Prim {
                            id: runtime.source.root,
                            name: String::new(),
                        }
                    }
                    _ => {}
                };
                for node in &mut definition.nodes {
                    if let Some(id) = remap.get(&node.id) {
                        node.id = *id;
                    }
                    target(&mut node.parent);
                }
                for (index, binding) in definition.bindings.iter_mut().enumerate() {
                    target(&mut binding.target);
                    if definition.profiles.is_empty() {
                        binding.enabled = runtime.bindings.contains(&index);
                    }
                }
                self.definition.nodes.extend(definition.nodes);
                let mut imported = std::collections::HashSet::new();
                let existing: std::collections::HashSet<_> = self.definition.bindings.iter().map(|b| b.bone.clone()).collect();
                let mut slots = std::collections::HashMap::new();
                for (index, binding) in definition.bindings.into_iter().enumerate() {
                    if !existing.contains(&binding.bone) {
                        slots.insert(index, self.definition.bindings.len());
                        imported.insert(binding.bone.clone());
                        self.definition.bindings.push(binding);
                    }
                }
                for mut profile in definition.profiles {
                    profile.bones.retain(|b| imported.contains(b));
                    if let Some(members) = &mut profile.bindings {
                        *members = members.iter().filter_map(|i| slots.get(i).copied()).collect();
                    }
                    if profile.bindings.is_none() && profile.bones.is_empty() {
                        continue;
                    }
                    let mut suffix = self.definition.profiles.len();
                    while self.definition.profiles.iter().any(|p| p.name == profile.name) {
                        suffix += 1;
                        profile.name = format!("Imported_profile_{suffix}");
                    }
                    self.definition.profiles.push(profile);
                }
                if let Some(selected) = self.profile.filter(|i| *i < self.definition.profiles.len()) {
                    let mut members: Vec<_> = self
                        .definition
                        .bindings
                        .iter()
                        .enumerate()
                        .filter_map(|(i, b)| self.definition.profiles[selected].contains(i, &b.bone).then_some(i))
                        .collect();
                    members.extend(runtime.bindings.iter().filter_map(|i| slots.get(i).copied()));
                    members.sort_unstable();
                    members.dedup();
                    self.definition.profiles[selected].bones.clear();
                    self.definition.profiles[selected].bindings = Some(members);
                }
                self.time = runtime.time(frame.at);
            }
        }
        let dt = self
            .at
            .map_or(0.0, |at| frame.at.saturating_duration_since(at).as_secs_f64().min(0.1));
        self.at = Some(frame.at);
        if self.playing {
            self.time += dt;
        }
        self.context = Some(Arc::new(ik::graph::Context::new(
            world,
            frame.model,
            Arc::new(self.definition.clone()),
            self.source.map(|root| crate::world::hand_ik::Seat {
                root,
                region: world
                    .objects
                    .index_of_uuid(&root)
                    .and_then(|i| world.objects.get(i))
                    .map_or(world.main_region.unwrap_or(0), |o| o.key.region),
            }),
            self.time,
            frame.at,
        )));
        let inverse = frame.model.inverse();
        self.matrices = frame.joints.iter().map(|j| inverse * *j).collect();
    }
    fn context(&self) -> Option<Arc<ik::graph::Context>> {
        let mut context = (**self.context.as_ref()?).clone();
        context.definition = Arc::new(self.definition.clone());
        context.time = self.time;
        Some(Arc::new(context))
    }
    fn node_transform(&self, id: Uuid) -> Option<Transform> {
        self.context()?.target(&Target::Node { id }, &self.matrices)
    }
    pub(super) fn binding_rotation(&self, bone: &str) -> Option<Quat> {
        let binding = self
            .binding
            .filter(|i| self.visible_binding(*i))
            .and_then(|i| self.definition.bindings.get(i))
            .filter(|b| b.bone == bone)
            .or_else(|| {
                self.definition
                    .bindings
                    .iter()
                    .enumerate()
                    .find(|(i, b)| self.visible_binding(*i) && b.bone == bone)
                    .map(|(_, b)| b)
            })?;
        let target = self.context()?.target(&binding.target, &self.matrices)?;
        Some((target.q() * binding.offset.q()).normalize())
    }
    pub(super) fn capture_node(&mut self, desired: Transform) {
        let Some(id) = self.selected else { return };
        let Some(index) = self.definition.nodes.iter().position(|n| n.id == id) else {
            return;
        };
        let Some(parent) = self
            .context()
            .and_then(|c| c.target(&self.definition.nodes[index].parent, &self.matrices))
        else {
            return;
        };
        self.playing = false;
        let local = parent.inverse().compose(desired);
        let node = &mut self.definition.nodes[index];
        if self.auto_key {
            let delta = node.offset.inverse().compose(local);
            motion::key(node, self.time, delta, &mut self.key);
        } else {
            let delta = node.motion.as_ref().map_or_else(Transform::default, |m| m.sample(self.time));
            node.offset = local.compose(delta.inverse());
        }
    }
    pub(super) fn capture(&mut self, goal: &Goal, names: &[String]) {
        let Some(index) = self.binding_index(goal.bone, names) else {
            return;
        };
        let Some(target) = self
            .context()
            .and_then(|c| c.target(&self.definition.bindings[index].target, &self.matrices))
        else {
            return;
        };
        let binding = &mut self.definition.bindings[index];
        binding.offset = target.inverse().compose(Transform::new(
            goal.position,
            goal.rotation.unwrap_or_else(|| target.q() * binding.offset.q()),
        ));
        binding.orient = goal.rotation.is_some();
        binding.contact = goal.contact.to_array();
        binding.weight = goal.weight;
        binding.mode = goal.mode;
        binding.root = names.get(goal.root).cloned();
        binding.pole = goal.pole.map(|p| (target.q().inverse() * (p - target.p())).to_array());
    }
    pub fn request_goals(&self) -> Vec<Goal> {
        self.rig.as_ref().map_or_else(Vec::new, |rig| self.goals(rig))
    }
    pub fn goals(&self, rig: &Rig) -> Vec<Goal> {
        self.context().map_or_else(Vec::new, |c| {
            c.goals(rig)
                .into_iter()
                .filter(|g| g.follow.as_ref().is_some_and(|f| self.visible_binding(f.index)))
                .collect()
        })
    }
}

pub(super) fn label(target: &Target, world: &World, nodes: &[Node]) -> String {
    match target {
        Target::Link { link, .. } => {
            if link == data::ROOT_LINK {
                aurora_i18n::tr!("ik-graph-object-root").to_string()
            } else {
                link.clone()
            }
        }
        Target::Prim { id, name } => {
            if name.is_empty() {
                id.to_string()
            } else {
                name.clone()
            }
        }
        Target::Bone { avatar, bone } => {
            if avatar.is_nil() || *avatar == world.agent_id {
                bone.clone()
            } else {
                format!("{} · {bone}", world.avatar_name(avatar).unwrap_or_else(|| avatar.to_string()))
            }
        }
        Target::Node { id } => nodes
            .iter()
            .find(|n| n.id == *id)
            .map_or_else(|| id.to_string(), |n| n.name.clone()),
        Target::Source => aurora_i18n::tr!("ik-graph-source").to_string(),
    }
}

/// Advanced virtual parents use the read-only prim picker or own rig names.
/// Remote references remain a runtime compatibility feature, not a UI choice.
fn target_ui(
    ui: &mut egui::Ui,
    palette: &Palette,
    target: &mut Target,
    world: &World,
    candidates: (&[Node], Option<Uuid>),
    prim: Option<(Uuid, String)>,
    picking: &mut bool,
) -> bool {
    let (nodes, exclude) = candidates;
    let before = target.clone();
    let mut kind = match target {
        Target::Link { .. } | Target::Prim { .. } | Target::Source => 0,
        Target::Bone { .. } => 1,
        Target::Node { .. } => 2,
    };
    ui.horizontal(|ui| {
        if widgets::radio_value(ui, palette, &mut kind, 0, aurora_i18n::tr!("ik-graph-prim")).changed() {
            let (id, name) = prim.clone().unwrap_or_default();
            *target = Target::Prim { id, name };
        }
        if widgets::radio_value(ui, palette, &mut kind, 1, aurora_i18n::tr!("ik-graph-bone")).changed() {
            *target = Target::default();
        }
        ui.add_enabled_ui(nodes.iter().any(|n| Some(n.id) != exclude), |ui| {
            if widgets::radio_value(ui, palette, &mut kind, 2, aurora_i18n::tr!("ik-graph-node")).changed()
                && let Some(node) = nodes.iter().find(|n| Some(n.id) != exclude)
            {
                *target = Target::Node { id: node.id };
            }
        });
    });
    match target {
        Target::Link { link, .. } => {
            ui.label(link.as_str());
            ui.horizontal_wrapped(|ui| {
                if ui.button(aurora_i18n::tr!("ik-edit-pick")).clicked() {
                    *picking = !*picking;
                }
                if let Some((id, name)) = prim
                    && ui.button(aurora_i18n::tr!("ik-graph-use-picked")).clicked()
                {
                    *target = Target::Prim { id, name };
                }
            });
        }
        Target::Prim { id, name } => {
            ui.label(label(
                &Target::Prim {
                    id: *id,
                    name: name.clone(),
                },
                world,
                nodes,
            ));
            ui.horizontal_wrapped(|ui| {
                if ui.button(aurora_i18n::tr!("ik-edit-pick")).clicked() {
                    *picking = !*picking;
                }
                if let Some((picked, picked_name)) = prim
                    && ui.button(aurora_i18n::tr!("ik-graph-use-picked")).clicked()
                {
                    *id = picked;
                    *name = picked_name;
                }
            });
        }
        Target::Bone { avatar, bone } => {
            egui::ComboBox::from_id_salt("target-bone")
                .selected_text(bone.as_str())
                .height(240.0)
                .show_ui(ui, |ui| {
                    for name in &world.avatar_lib.rig.canonical_names {
                        if ui.selectable_value(bone, name.clone(), name).changed() {
                            *avatar = Uuid::nil();
                        }
                    }
                });
        }
        Target::Node { id } => {
            egui::ComboBox::from_id_salt("target-node")
                .selected_text(nodes.iter().find(|n| n.id == *id).map_or("", |n| n.name.as_str()))
                .show_ui(ui, |ui| {
                    for node in nodes.iter().filter(|n| Some(n.id) != exclude) {
                        ui.selectable_value(id, node.id, &node.name);
                    }
                });
        }
        Target::Source => {
            ui.label(aurora_i18n::tr!("ik-graph-source"));
        }
    }
    *target != before
}

impl Editor {
    pub(super) fn graph_demo(&mut self, world: &World, frame: &Frame, mode: &str) {
        use data::{Curve, Function, Key, Motion, Playback};
        let rig = &world.avatar_lib.rig;
        let bone = rig.names["mWristLeft"];
        let id = Uuid::from_u128(0x994001);
        let child = Uuid::from_u128(0x994002);
        let parent = if mode == "graph-remote" {
            Target::Bone {
                avatar: crate::demo::DEMO_LOUP,
                bone: "mHead".into(),
            }
        } else {
            Target::Prim {
                id: crate::demo::action_id(crate::demo::hand_ik::LEFT),
                name: "Left".into(),
            }
        };
        self.graph.imported = true;
        self.graph.definition.nodes = vec![
            Node {
                id,
                name: aurora_i18n::tr!("ik-graph-demo-anchor").to_string(),
                parent,
                offset: Transform::default(),
                motion: None,
            },
            Node {
                id: child,
                name: aurora_i18n::tr!("ik-graph-demo-control").to_string(),
                parent: Target::Node { id },
                offset: Transform::default(),
                motion: Some(if mode == "graph-keys" {
                    Motion::Keys {
                        duration: 2.0,
                        playback: Playback::PingPong,
                        keys: vec![
                            Key {
                                time: 0.0,
                                transform: Transform::default(),
                                curve: Curve::Bezier {
                                    controls: [0.25, 0.0, 0.75, 1.0],
                                },
                            },
                            Key {
                                time: 2.0,
                                transform: Transform::new(Vec3::Z * 0.04, Quat::from_rotation_z(0.2)),
                                curve: Curve::Smooth,
                            },
                        ],
                    }
                } else {
                    Motion::Function {
                        duration: 2.0,
                        playback: Playback::Loop,
                        function: Function::Sine,
                        position: [0.0, 0.0, 0.035],
                        degrees: [0.0, 0.0, 12.0],
                        phase: 0.0,
                        curve: Curve::Linear,
                    }
                }),
            },
        ];
        self.graph.time = 0.5;
        self.graph.refresh(world, frame);
        if let Some(context) = self.graph.context()
            && let Some(parent) = context.target(&self.graph.definition.nodes[0].parent, &self.graph.matrices)
        {
            self.graph.definition.nodes[0].offset = parent.inverse().compose(Transform::new(self.position, self.rotation()));
        }
        self.graph.definition.bindings = vec![Binding {
            activation: Default::default(),
            bone: "mWristLeft".into(),
            target: Target::Node { id: child },
            offset: Transform::default(),
            orient: true,
            contact: self.contact.to_array(),
            root: Some("mShoulderLeft".into()),
            mode: Mode::Ik,
            pole: None,
            weight: 1.0,
            enabled: true,
        }];
        self.graph.selected = Some(child);
        self.graph.key = 0;
        self.graph.node_tab = if mode == "graph-keys" { 1 } else { 0 };
        self.graph.reveal_curve = mode == "graph-keys";
        self.tab = 4;
        self.visibility.body();
        self.update_graph(world, frame);
        if mode == "graph-cycle" {
            self.graph.definition.nodes[0].parent = Target::Bone {
                avatar: Uuid::nil(),
                bone: "mWristLeft".into(),
            };
        }
        if mode == "graph-remote" {
            self.graph.selected = None;
            self.graph.binding = None;
            self.select(bone, rig, frame);
            self.tab = 1;
        }
        self.capture_link_references(world, &crate::build::BuildTool::default());
    }
    pub(super) fn update_graph(&mut self, world: &World, frame: &Frame) {
        self.graph.refresh(world, frame);
        let rig = &world.avatar_lib.rig;
        if self.canonical_names.is_empty() {
            self.canonical_names.clone_from(&rig.canonical_names);
        }
        let goals = self.graph.goals(rig);
        let order = ik::graph::order(rig, &goals.iter().collect::<Vec<_>>());
        self.graph.error = self.graph.definition.validate().is_err() || order.len() != goals.len();
        if let Some(context) = self.graph.context() {
            for goal in &goals {
                if let Some(actual) = context.preview(goal, &self.graph.matrices) {
                    if let Some(old) = self.goals.iter_mut().find(|g| g.bone == actual.bone) {
                        *old = actual;
                    } else {
                        self.goals.push(actual);
                    }
                }
            }
        }
        if let Some(id) = self.graph.selected {
            if let Some(transform) = self.graph.node_transform(id) {
                self.position = transform.p();
                self.set_rotation(transform.q());
            }
        } else if let Some(bone) = self.selected
            && !self.disabled.contains(&bone)
            && self.graph.definition.bindings.iter().any(|b| rig.names.get(&b.bone) == Some(&bone))
            && let Some(goal) = self
                .graph
                .binding_index(bone, &rig.canonical_names)
                .and_then(|i| self.graph.binding_goal(i, rig))
        {
            self.contact = goal.contact;
            self.weight = goal.weight;
            self.orient = goal.rotation.is_some();
            self.position = goal.position;
            self.pole = goal.pole;
            if let Some(q) = goal.rotation {
                self.set_rotation(q);
            }
            if self
                .graph
                .binding_index(bone, &rig.canonical_names)
                .and_then(|i| self.graph.definition.bindings.get(i))
                .is_some_and(|b| !b.enabled)
            {
                self.released_pose(frame);
            }
        }
    }

    pub(super) fn bind_target(&mut self, world: &World, frame: &Frame, source: usize, target: Target) -> bool {
        let rig = &world.avatar_lib.rig;
        let Some(name) = rig.canonical_names.get(source) else {
            return false;
        };
        if !target.valid()
            || matches!(&target, Target::Bone { avatar,bone } if (!avatar.is_nil() && *avatar != world.agent_id) || rig.names.get(bone) == Some(&source))
        {
            return false;
        }
        let picked = target.clone();
        // A concrete scene pick is already known. Capture its metric frame
        // while metadata loads; its persistent identity is still root/link.
        let mut target = target;
        if let Target::Prim { id, name } = &mut target
            && self
                .graph
                .source
                .is_some_and(|source| crate::world::hand_ik::object_source(world, *id).is_some_and(|s| s.root != source))
        {
            name.clear();
        }
        if self.selected != Some(source) || self.graph.selected.is_some() {
            self.select(source, rig, frame);
        }
        let desired = Goal {
            follow: None,
            bone: source,
            root: self.root,
            position: self.position,
            rotation: Some(self.rotation()),
            contact: self.contact,
            pole: self.pole,
            mode: self.mode,
            weight: self.weight,
        };
        self.graph.refresh(world, frame);
        let old = self.graph.definition.clone();
        let replace = (!self.links.adding)
            .then(|| self.graph.binding_index(source, &rig.canonical_names))
            .flatten();
        let activation = replace
            .and_then(|i| self.graph.definition.bindings.get(i).map(|b| b.activation))
            .unwrap_or_else(|| self.new_binding_activation());
        let binding = Binding {
            activation,
            bone: name.clone(),
            target: target.clone(),
            offset: Transform::default(),
            orient: true,
            contact: desired.contact.to_array(),
            root: rig.canonical_names.get(desired.root).cloned(),
            mode: desired.mode,
            pole: None,
            weight: desired.weight,
            enabled: true,
        };
        let previous_selection = self.graph.binding;
        let index = if let Some(index) = replace {
            self.graph.definition.bindings[index] = binding;
            index
        } else {
            self.graph.definition.bindings.push(binding);
            self.graph.definition.bindings.len() - 1
        };
        if replace.is_none() {
            self.assign_profile_binding(index);
        }
        self.graph.binding = Some(index);
        self.graph.refresh(world, frame);
        let goals = self.graph.goals(rig);
        let available = self.graph.context().and_then(|c| c.target(&target, &self.graph.matrices)).is_some();
        if !available
            || self.graph.definition.validate().is_err()
            || ik::graph::order(rig, &goals.iter().collect::<Vec<_>>()).len() != goals.len()
        {
            self.graph.definition = old;
            self.graph.binding = previous_selection;
            self.graph.refresh(world, frame);
            return false;
        }
        self.graph.capture(&desired, &rig.canonical_names);
        if self.graph.definition.validate().is_err() {
            self.graph.definition = old;
            self.graph.binding = previous_selection;
            self.graph.refresh(world, frame);
            return false;
        }
        self.remember_link_reference(world, &picked);
        self.graph.target = target;
        self.bindings.retain(|b| b.bone != source);
        self.authoring.remove(source);
        self.disabled.retain(|b| *b != source);
        if let Some(goal) = self.goals.iter_mut().find(|g| g.bone == source) {
            *goal = desired;
        } else {
            self.goals.push(desired);
        }
        self.refresh = true;
        true
    }

    pub(super) fn select_binding(&mut self, index: usize, world: &World, frame: &Frame) {
        let Some(bone) = self
            .graph
            .definition
            .bindings
            .get(index)
            .and_then(|b| world.avatar_lib.rig.names.get(&b.bone))
            .copied()
        else {
            return;
        };
        self.graph.binding = Some(index);
        self.visibility.reveal(&world.avatar_lib.rig.canonical_names[bone]);
        self.select(bone, &world.avatar_lib.rig, frame);
    }
    pub(super) fn remove_graph_binding(&mut self, index: usize, world: &World, frame: &Frame) {
        let Some(binding) = self.graph.definition.bindings.get(index) else {
            return;
        };
        let bone = world.avatar_lib.rig.names.get(&binding.bone).copied();
        self.graph.definition.remove_binding(index);
        self.links.copied = None;
        self.graph.binding = self
            .graph
            .binding
            .and_then(|i| if i == index { None } else { Some(i - usize::from(i > index)) });
        if let Some(bone) = bone {
            self.goals.retain(|g| g.bone != bone);
            self.sync_graph_activation(bone, &world.avatar_lib.rig);
            if self.selected == Some(bone) {
                self.select(bone, &world.avatar_lib.rig, frame);
            }
        }
        self.refresh = true;
    }
    pub(super) fn sync_graph_activation(&mut self, bone: usize, rig: &Rig) {
        self.disabled.retain(|&b| b != bone);
        if !self
            .graph
            .definition
            .bindings
            .iter()
            .enumerate()
            .any(|(i, b)| self.graph.visible_binding(i) && rig.names.get(&b.bone) == Some(&bone) && b.enabled)
        {
            self.disabled.push(bone);
        }
    }

    pub(super) fn graph_bindings_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let mut selected = None;
        let mut selected_node = None;
        let mut remove = None;
        let mut edit = None;
        let mut weight = None;
        let mut orientation = None;
        for index in 0..self.graph.definition.bindings.len() {
            if !self.graph.visible_binding(index) {
                continue;
            }
            let mut binding = self.graph.definition.bindings[index].clone();
            let source = world.avatar_lib.rig.names.get(&binding.bone).copied();
            let target = binding.target.clone();
            let target_name = self.target_label(&target, world);
            let source_name = &binding.bone;
            if !self.binding_matches(source_name, &target_name) {
                continue;
            }
            let is_selected = self.graph.selected.is_none() && self.graph.binding == Some(index);
            ui.push_id((index, "graph-binding"), |ui| {
                ui.horizontal(|ui| {
                    let changed = widgets::checkbox(ui, palette, &mut binding.enabled, "").changed();
                    if changed && let Some(&bone) = world.avatar_lib.rig.names.get(&binding.bone) {
                        self.graph.definition.bindings[index].enabled = binding.enabled;
                        self.sync_graph_activation(bone, &world.avatar_lib.rig);
                    }
                    let (source_clicked, target_clicked) = super::bindings::binding_labels(ui, is_selected, source_name, &target_name);
                    if source_clicked {
                        self.select_binding(index, world, frame);
                    }
                    if target_clicked {
                        match &target {
                            Target::Link { .. } | Target::Prim { .. } | Target::Source => {
                                self.prims.edit = self.target_prim(world, &target)
                            }
                            Target::Bone { avatar, bone } if avatar.is_nil() || *avatar == world.agent_id => {
                                selected = world.avatar_lib.rig.names.get(bone).copied()
                            }
                            Target::Node { id } => selected_node = Some(*id),
                            _ => {}
                        }
                    }
                    let reveal = self.links.reveal_menu && is_selected;
                    if reveal {
                        self.links.reveal_menu = false;
                    }
                    if let Some(prim) = super::bindings::target_menu(ui, palette, reveal) {
                        self.graph.binding = Some(index);
                        edit = source.map(|bone| (bone, prim));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("ik-edit-weight"));
                    let mut percent = binding.weight * 100.0;
                    if super::bindings::weight_slider(ui, &mut percent) {
                        weight = Some((index, percent / 100.0));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, palette, "trash-filled", aurora_i18n::tr!("ik-edit-unbind"), true) {
                            remove = Some(index);
                        }
                        let copied = self.copied(ui.ctx(), links::Copy::Binding(index));
                        if widgets::icon_button(
                            ui,
                            palette,
                            if copied { "circle-check-filled" } else { "copy-filled" },
                            aurora_i18n::tr!("ik-profile-copy-binding"),
                            true,
                        ) {
                            self.copy_profile_binding(index, ui.ctx());
                        }
                        if super::bindings::rotation_toggle(ui, palette, binding.orient) {
                            orientation = Some((index, !binding.orient));
                        }
                    });
                });
                if is_selected {
                    self.binding_details_ui(ui, palette, world, frame, index);
                }
                ui.add_space(ui.spacing().item_spacing.y);
            });
        }
        if let Some((index, weight)) = weight {
            self.graph.definition.bindings[index].weight = weight;
            if self.graph.binding == Some(index) {
                self.weight = weight;
            }
        }
        if let Some((index, orient)) = orientation {
            self.graph.definition.bindings[index].orient = orient;
            if self.graph.binding == Some(index) {
                self.orient = orient;
            }
        }
        if let Some(index) = remove {
            self.remove_graph_binding(index, world, frame);
        }
        if let Some(bone) = selected {
            self.visibility.reveal(&world.avatar_lib.rig.canonical_names[bone]);
            self.select(bone, &world.avatar_lib.rig, frame);
            self.tab = 1;
        }
        if let Some(node) = selected_node {
            self.graph.selected = Some(node);
            self.graph.pause();
            self.drag = None;
            self.gizmo.cancel();
            self.control = Control::Bone;
            self.tab = 4;
            self.graph.refresh(world, frame);
        }
        if let Some((bone, prim)) = edit {
            self.begin_link(world, frame, bone, prim);
        }
        if self.graph.error {
            ui.colored_label(palette.muted, aurora_i18n::tr!("ik-graph-cycle"));
        }
    }

    pub(super) fn nodes_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let mut chosen = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.graph.definition.nodes.len() < data::MAX_NODES,
                    egui::Button::new(aurora_i18n::tr!("ik-graph-add")),
                )
                .clicked()
            {
                let id = Uuid::new_v4();
                let bone = self.selected.unwrap_or(world.avatar_lib.rig.pelvis);
                let parent = Target::Bone {
                    avatar: Uuid::nil(),
                    bone: world.avatar_lib.rig.canonical_names[bone].clone(),
                };
                self.graph.definition.nodes.push(Node {
                    id,
                    name: aurora_i18n::tr!("ik-graph-control-name", number = self.graph.definition.nodes.len() + 1).to_string(),
                    parent,
                    offset: Transform::default(),
                    motion: None,
                });
                chosen = Some(id);
            }
            if ui
                .add_enabled(
                    self.graph.selected.is_some(),
                    egui::Button::new(aurora_i18n::tr!("ik-graph-delete")),
                )
                .clicked()
                && let Some(id) = self.graph.selected
            {
                // Remove dependent references too; no dangling or silently
                // reparented animated helpers survive deletion.
                let mut removed = vec![id];
                let mut count = 0;
                while count != removed.len() {
                    count = removed.len();
                    for node in &self.graph.definition.nodes {
                        if let Target::Node { id } = node.parent
                            && removed.contains(&id)
                            && !removed.contains(&node.id)
                        {
                            removed.push(node.id);
                        }
                    }
                }
                self.graph.definition.nodes.retain(|n| !removed.contains(&n.id));
                let mut bones = Vec::new();
                let deletions: Vec<_> = self
                    .graph
                    .definition
                    .bindings
                    .iter()
                    .enumerate()
                    .filter_map(|(index, b)| {
                        let delete = matches!(b.target,Target::Node{id} if removed.contains(&id));
                        if delete && let Some(bone) = world.avatar_lib.rig.names.get(&b.bone) {
                            bones.push(*bone);
                        }
                        delete.then_some(index)
                    })
                    .collect();
                for index in deletions.into_iter().rev() {
                    self.graph.definition.remove_binding(index);
                }
                self.graph.binding = None;
                for bone in bones {
                    self.sync_graph_activation(bone, &world.avatar_lib.rig);
                    if self.disabled.contains(&bone) {
                        self.goals.retain(|g| g.bone != bone);
                    }
                }
                self.graph.selected = None;
                self.refresh = true;
            }
        });
        egui::ScrollArea::vertical()
            .id_salt("virtual-list")
            .max_height(120.0)
            .show(ui, |ui| {
                let width = ((ui.available_width() - ui.spacing().item_spacing.x) * 0.5).max(80.0);
                egui::Grid::new("virtual-parents").num_columns(2).show(ui, |ui| {
                    ui.strong(aurora_i18n::tr!("ik-graph-node"));
                    ui.strong(aurora_i18n::tr!("ik-graph-parent"));
                    ui.end_row();
                    for node in &self.graph.definition.nodes {
                        if ui
                            .add_sized(
                                [width, ui.spacing().interact_size.y],
                                egui::Button::selectable(self.graph.selected == Some(node.id), &node.name).truncate(),
                            )
                            .on_hover_text(&node.name)
                            .clicked()
                        {
                            chosen = Some(node.id);
                        }
                        let parent = label(&node.parent, world, &self.graph.definition.nodes);
                        ui.add_sized([width, ui.spacing().interact_size.y], egui::Label::new(&parent).truncate())
                            .on_hover_text(parent);
                        ui.end_row();
                    }
                });
            });
        if let Some(id) = chosen {
            self.graph.selected = Some(id);
            self.graph.playing = false;
            self.tool = Tool::Move;
            self.drag = None;
            self.gizmo.cancel();
            self.control = Control::Bone;
            self.graph.refresh(world, frame);
        }
        let Some(id) = self.graph.selected else { return };
        let Some(index) = self.graph.definition.nodes.iter().position(|n| n.id == id) else {
            return;
        };
        ui.add(egui::TextEdit::singleline(&mut self.graph.definition.nodes[index].name).char_limit(64));
        let mut tab = self.graph.node_tab as usize;
        widgets::tabs(
            ui,
            palette,
            &mut tab,
            &[
                (aurora_i18n::tr!("ik-graph-placement"), true),
                (aurora_i18n::tr!("ik-graph-motion"), true),
            ],
        );
        self.graph.node_tab = tab as u8;
        if self.graph.node_tab == 1 {
            self.graph.motion_ui(ui, palette, index);
            return;
        }
        ui.label(aurora_i18n::tr!("ik-graph-parent"));
        let mut parent = self.graph.definition.nodes[index].parent.clone();
        let mut picking = self.picking && self.prims.node == Some(id);
        let was_picking = picking;
        let changed = target_ui(
            ui,
            palette,
            &mut parent,
            world,
            (&self.graph.definition.nodes, Some(id)),
            self.prims.edited.clone(),
            &mut picking,
        );
        if picking != was_picking {
            self.prim_picking(picking, Some(id));
        }
        if changed {
            let old = self.graph.definition.nodes[index].parent.clone();
            self.graph.definition.nodes[index].parent = parent;
            if self.graph.definition.validate().is_err() {
                self.graph.definition.nodes[index].parent = old;
                self.graph.error = true;
            }
        }
        let node = &mut self.graph.definition.nodes[index];
        ui.label(aurora_i18n::tr!("ik-graph-offset"));
        let mut p = node.offset.p();
        let mut d = degrees(node.offset.q());
        let mut changed = vector_fields(ui, &mut p, 0.005);
        changed |= vector_fields(ui, &mut d, 0.5);
        if changed {
            self.graph.playing = false;
            node.offset = Transform::new(p, rotation(d));
        }
    }

    pub(super) fn graph_overlay(&mut self, ctx: &egui::Context, palette: &Palette, world: &World, proj: &Projector, frame: &Frame) -> bool {
        let mut chosen = None;
        let mut edit = None;
        let mut owns = false;
        let mut gizmo_owns = false;
        let nodes = self.graph.definition.nodes.clone();
        // Overlapping controls reserve the selected point first. Virtual
        // points also reserve their pixels before real-bone widgets are drawn.
        let point_target = ctx.input(|i| i.pointer.interact_pos()).and_then(|pointer| {
            nodes
                .iter()
                .filter_map(|node| {
                    let transform = self.graph.node_transform(node.id)?;
                    let (at, _) = proj.project(frame.model.transform_point3(transform.p()))?;
                    egui::Rect::from_center_size(at, egui::Vec2::splat(16.0))
                        .contains(pointer)
                        .then_some((node.id, at.distance_sq(pointer)))
                })
                .min_by(|a, b| {
                    (self.graph.selected != Some(a.0))
                        .cmp(&(self.graph.selected != Some(b.0)))
                        .then(a.1.total_cmp(&b.1))
                })
                .map(|(id, _)| id)
        });
        for node in &nodes {
            let Some(transform) = self.graph.node_transform(node.id) else {
                continue;
            };
            let p = frame.model.transform_point3(transform.p());
            let Some((point, _)) = proj.project(p) else { continue };
            let selected = self.graph.selected == Some(node.id);
            if selected
                && self.gizmo_visible(ctx.input(|i| i.modifiers))
                && self.drag.is_none()
                && !self.picking
                && !self.prims.object_handles
            {
                let (_, model_q, _) = frame.model.to_scale_rotation_translation();
                let pose = Pose {
                    position: p,
                    rotation: model_q * transform.q(),
                };
                let tool = self.active_tool(ctx.input(|i| i.modifiers));
                let basis = if self.space == Space::World {
                    Quat::IDENTITY
                } else {
                    pose.rotation
                };
                if let Some(geometry) = Geometry::new(proj, p, basis, tool) {
                    let point_hit = point_target.is_some()
                        || ctx
                            .input(|i| i.pointer.interact_pos())
                            .is_some_and(|p| self.point_owns_pointer(ctx, &world.avatar_lib.rig, p));
                    let (o, change) = self.gizmo.show(ctx, palette, geometry, pose, !point_hit);
                    gizmo_owns |= o;
                    owns |= o;
                    if let Some((pose, _)) = change {
                        edit = Some(Transform::new(
                            frame.model.inverse().transform_point3(pose.position),
                            model_q.inverse() * pose.rotation,
                        ));
                    }
                }
            }
            if (self.picking && !self.links.bone_pick)
                || (self.prims.object_handles && !self.links.bone_pick)
                || ctx.input(|i| i.modifiers.alt)
            {
                continue;
            }
            let response = egui::Area::new(egui::Id::unique(("ik-virtual", node.id)))
                .fixed_pos(point - egui::vec2(8.0, 8.0))
                .order(egui::Order::Background)
                .movable(false)
                .interactable(!gizmo_owns && point_target.is_none_or(|id| id == node.id))
                .show(ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::Vec2::splat(16.0),
                        if self.links.bone_pick {
                            Sense::click()
                        } else if gizmo_owns || point_target.is_some_and(|id| id != node.id) {
                            Sense::hover()
                        } else {
                            Sense::click_and_drag()
                        },
                    );
                    ui.painter()
                        .circle_stroke(rect.center(), if selected { 7.0 } else { 5.0 }, Stroke::new(2.0, palette.teal));
                    if selected {
                        ui.painter().text(
                            rect.center() + egui::vec2(12.0, 16.0),
                            egui::Align2::LEFT_TOP,
                            &node.name,
                            egui::FontId::proportional(12.0),
                            palette.teal,
                        );
                    }
                    response
                })
                .inner;
            if self.links.bone_pick && response.clicked() {
                self.picked_bone(world, frame, Target::Node { id: node.id });
                owns = true;
                continue;
            }
            if response.clicked() || response.drag_started() {
                chosen = Some((node.id, transform));
                owns = true;
            }
            if self.point_drag_enabled(ctx.input(|i| i.modifiers))
                && response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                chosen = Some((node.id, transform));
                let rotate = self.active_tool(ctx.input(|i| i.modifiers)) == Tool::Rotate;
                if self.drag.is_none() || self.drag.is_some_and(|d| matches!(d, Drag::Rotate { .. }) != rotate) {
                    self.position = transform.p();
                    self.set_rotation(transform.q());
                    self.start_drag(proj, frame, ctx.input(|i| i.pointer.press_origin()).unwrap_or(pointer), rotate);
                }
                owns = true;
                // Delay mutable editor access until the node iteration ends.
                if let Some(Drag::Move { point, normal, offset }) = self.drag
                    && let Some((o, d)) = ray(proj, pointer)
                    && let Some(hit) = crate::build::geom::ray_plane(o, d, point, normal)
                {
                    edit = Some(Transform::new(frame.model.inverse().transform_point3(hit + offset), transform.q()));
                }
                if let Some(Drag::Rotate {
                    origin,
                    rotation,
                    right,
                    up,
                }) = self.drag
                {
                    let delta = pointer - origin;
                    edit = Some(Transform::new(
                        transform.p(),
                        Quat::from_axis_angle(right, delta.y * 0.01) * Quat::from_axis_angle(up, delta.x * 0.01) * rotation,
                    ));
                }
            }
            if response.drag_stopped() {
                self.drag = None;
            }
            owns |= point_target == Some(node.id);
        }
        if let Some((id, transform)) = chosen {
            self.select_virtual_frame(id, transform);
        }
        if let Some(transform) = edit {
            self.graph.capture_node(transform);
        }
        owns
    }
    pub(super) fn select_virtual_node(&mut self, id: Uuid) {
        if let Some(transform) = self.graph.node_transform(id) {
            self.select_virtual_frame(id, transform);
        }
    }
    fn select_virtual_frame(&mut self, id: Uuid, transform: Transform) {
        // An explicit virtual selection supersedes the real-bone refresh
        // queued by a profile change or import.
        self.refresh = false;
        self.graph.selected = Some(id);
        self.graph.playing = false;
        self.control = Control::Bone;
        self.tool = Tool::Move;
        self.position = transform.p();
        self.set_rotation(transform.q());
        self.tab = 4;
    }
    pub(super) fn virtual_segments(&self, proj: &Projector, frame: &Frame) -> Vec<segments::Segment> {
        let Some(context) = self.graph.context() else { return Vec::new() };
        self.graph
            .definition
            .nodes
            .iter()
            .filter_map(|node| {
                let parent = context.target(&node.parent, &self.graph.matrices)?;
                let transform = self.graph.node_transform(node.id)?;
                Some(segments::Segment {
                    source: segments::Source::Node(node.id),
                    a: proj.project(frame.model.transform_point3(parent.p()))?,
                    b: proj.project(frame.model.transform_point3(transform.p()))?,
                })
            })
            .collect()
    }

    pub(super) fn graph_definition(&self) -> Definition {
        let mut definition = self.stored_definition();
        for binding in &self.bindings {
            let d = &binding.definition;
            if definition.bindings.iter().any(|b| b.bone == d.bone) {
                continue;
            }
            let mut target = self.prims.references.get(&d.target).cloned().unwrap_or_else(|| Target::Prim {
                id: d.target,
                name: binding.name.clone(),
            });
            if let Target::Link { link, .. } = &mut target
                && link != data::ROOT_LINK
                && !binding.name.is_empty()
            {
                link.clone_from(&binding.name);
            }
            definition.bindings.push(Binding {
                activation: d.activation,
                bone: d.bone.clone(),
                target,
                offset: Transform {
                    position: d.position,
                    rotation: d.rotation.unwrap_or(binding.orientation.to_array()),
                },
                orient: d.rotation.is_some(),
                contact: d.contact,
                root: d.root.clone(),
                mode: d.mode,
                pole: d.pole,
                weight: d.weight,
                enabled: self.enabled(binding.bone),
            });
        }
        definition.prune_profiles();
        // The editor uses a concrete root for live prim manipulation. Export
        // the original source frame again so copies can resolve their own root.
        for target in definition
            .nodes
            .iter_mut()
            .map(|n| &mut n.parent)
            .chain(definition.bindings.iter_mut().map(|b| &mut b.target))
        {
            if let Target::Prim { id, .. } = target
                && let Some(reference) = self.prims.references.get(id)
            {
                *target = reference.clone();
            }
            if matches!(target, Target::Prim { id, .. } if Some(*id) == self.graph.source) {
                *target = Target::Source;
            }
            if matches!(target, Target::Link { object, .. } if Some(*object) == self.graph.source)
                && let Target::Link { object, .. } = target
            {
                *object = Uuid::nil();
            }
            if *target == Target::Source {
                *target = Target::Link {
                    object: Uuid::nil(),
                    link: data::ROOT_LINK.into(),
                };
            }
        }
        definition
    }
    /// Rig definition only: no session nonce, wire envelope or LSL logic.
    pub(super) fn graph_payload(&self) -> Option<String> {
        let definition = self.graph_definition();
        if (definition.bindings.is_empty() && definition.nodes.is_empty())
            || definition.validate().is_err()
            || definition.targets().any(|t| matches!(t, Target::Prim { .. }))
        {
            return None;
        }
        let json = serde_json::to_string(&definition).ok()?;
        let ascii: String = json
            .chars()
            .map(|c| {
                if c.is_ascii() {
                    c.to_string()
                } else {
                    let mut units = [0; 2];
                    c.encode_utf16(&mut units).iter().map(|u| format!("\\u{u:04x}")).collect::<String>()
                }
            })
            .collect();
        if ascii.len() > data::MAX_BYTES {
            return None;
        }
        Some(ascii)
    }
    /// Ready-to-paste LSL declaration, shared by both clipboard exports.
    pub(super) fn graph_rig_declaration(&self) -> Option<String> {
        let ascii = self.graph_payload()?;
        Some(format!("string RIG = {};", serde_json::to_string(&ascii).ok()?))
    }
    pub(super) fn graph_script(&self) -> Option<String> {
        Some(
            include_str!("../../../../scripts/lsl/rig-contacts.lsl")
                .replace("\r\n", "\n")
                .replace("string RIG = \"\";", &self.graph_rig_declaration()?),
        )
    }
}
