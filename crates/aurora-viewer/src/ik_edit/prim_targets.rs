//! Read-only prim designation, like LLToolObjPicker::pickCallback, alongside
//! the independent LLToolSelect build selection. Selecting a target never
//! changes Edit selection; guided picks bind the locked source immediately.
use super::*;
use crate::{
    build::{BuildSettings, BuildTool, Mods, Tool as BuildMode},
    world::rig_graph::Target,
};

#[derive(Default)]
pub(super) struct State {
    pub edited: Option<(Uuid, String)>,
    pub node: Option<Uuid>,
    manual: bool,
    pub object_handles: bool,
    pub names: std::collections::HashMap<Uuid, String>,
    pub references: std::collections::HashMap<Uuid, Target>,
    requested: std::collections::HashMap<Uuid, Instant>,
    next_request: Option<Instant>,
    commands: Vec<aurora_net::NetCommand>,
    pub edit: Option<Uuid>,
}

impl Editor {
    pub fn sync_build(&mut self, world: &World, build: &BuildTool, settings: &BuildSettings, cursor: (f32, f32), mods: Mods) {
        if !self.open || self.poser.active {
            return;
        }
        if let Some(index) = self.take_project_target_selection().and_then(|id| world.objects.index_of_uuid(&id)) {
            self.picked_project_target(world, build, index);
        }
        self.capture_link_references(world, build);
        for target in self
            .graph
            .definition
            .nodes
            .iter_mut()
            .map(|n| &mut n.parent)
            .chain(self.graph.definition.bindings.iter_mut().map(|b| &mut b.target))
        {
            if let Target::Prim { id, .. } = target
                && let Some(reference) = self.prims.references.get(id)
                && world.ik_targets.watches(*id)
                && let Target::Link { object, link } = reference
                && world
                    .ik_targets
                    .index(world, None, *object, link)
                    .and_then(|i| world.objects.get(i))
                    .is_some_and(|o| o.full_id == *id)
            {
                *target = reference.clone();
            }
        }
        let project_members = self.sync_project_target(world, build);
        let unresolved = self.graph.definition.targets().any(|target| {
            matches!(target,
            Target::Prim { id, name } if !name.is_empty() && world.objects.index_of_uuid(id).is_none())
        });
        let mut ids: Vec<_> = self
            .graph
            .definition
            .nodes
            .iter()
            .map(|n| &n.parent)
            .chain(self.graph.definition.bindings.iter().map(|b| &b.target))
            .filter_map(|t| match t {
                Target::Prim { id, .. } => Some(*id),
                Target::Link { object, .. } => (!object.is_nil()).then_some(*object).or(self.graph.source),
                _ => None,
            })
            .chain(self.bindings.iter().map(|b| b.definition.target))
            .chain(self.target)
            .chain(self.object_import_root())
            .chain(self.graph.source)
            .chain(project_members.into_iter().filter(|_| unresolved))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let now = Instant::now();
        let wanted: std::collections::HashSet<_> = ids.iter().copied().collect();
        self.prims.requested.retain(|id, _| wanted.contains(id));
        self.prims.names.retain(|id, _| wanted.contains(id));
        let before = self.prims.commands.len();
        let ready = self.prims.next_request.is_none_or(|at| now >= at);
        for id in ids {
            if let Some(name) = build
                .props
                .get(&id)
                .map(|p| p.name.as_str())
                .filter(|n| !n.is_empty())
                .or_else(|| world.ik_targets.name(id).filter(|n| !n.is_empty()))
            {
                self.prims.names.insert(id, name.to_owned());
            } else if ready
                && !world.ik_targets.watches(id)
                && self.prims.commands.len() < 4
                && self
                    .prims
                    .requested
                    .get(&id)
                    .is_none_or(|at| now.saturating_duration_since(*at).as_secs() >= 5)
                && let Some(object) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i))
            {
                self.prims.commands.extend(build.prim_properties_query(world, object.key));
                self.prims.requested.insert(id, now);
            }
        }
        if self.prims.commands.len() > before {
            self.prims.next_request = Some(now + std::time::Duration::from_millis(250));
        }
        self.prims.object_handles =
            build.open && (build.manip.dragging() || build.rect.is_some() || build.manipulator_at(world, settings, cursor, mods));
        let edited = edited_prim(world, build);
        if self.prims.edited.as_ref().map(|p| p.0) != edited.as_ref().map(|p| p.0) {
            self.prims.manual = false;
        }
        self.prims.edited = edited;
    }
    pub fn take_prim_requests(&mut self) -> Vec<aurora_net::NetCommand> {
        std::mem::take(&mut self.prims.commands)
    }
    pub fn apply_prim_edit(&mut self, world: &mut World, build: &mut BuildTool, settings: &mut BuildSettings) -> bool {
        let Some(id) = self.prims.edit.take() else { return false };
        let Some(index) = world.objects.index_of_uuid(&id).filter(|i| usable(world, *i)) else {
            return false;
        };
        build.open_build(BuildMode::Edit);
        build.set_edit_linked(world, settings, true);
        build.click_select(world, settings, Some(index), false);
        true
    }
    pub(super) fn prim_label(&self, id: Uuid, saved_name: &str) -> String {
        self.prims.names.get(&id).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| {
            if saved_name.is_empty() {
                aurora_i18n::tr!("ik-edit-prim-name-pending").to_string()
            } else {
                saved_name.to_owned()
            }
        })
    }
    pub(super) fn target_label(&self, target: &Target, world: &World) -> String {
        if let Target::Link { object, link } = target
            && link == crate::world::rig_graph::ROOT_LINK
        {
            return self.prim_label(
                if object.is_nil() {
                    self.graph.source.unwrap_or_default()
                } else {
                    *object
                },
                "",
            );
        }
        if let Target::Prim { id, name } = target {
            self.prim_label(*id, name)
        } else if let Target::Link { object, link } = target
            && !object.is_nil()
            && self.graph.source != Some(*object)
            && let Some(name) = self.prims.names.get(object)
        {
            format!("{name} · {link}")
        } else {
            super::graph::label(target, world, &self.graph.definition.nodes)
        }
    }

    pub(super) fn capture_link_references(&mut self, world: &World, build: &BuildTool) {
        let mut references = std::collections::HashMap::new();
        let concrete: Vec<_> = self
            .bindings
            .iter()
            .map(|b| Target::Prim {
                id: b.definition.target,
                name: b.name.clone(),
            })
            .collect();
        for target in self.graph.definition.targets().chain(concrete.iter()) {
            let Target::Prim { id, name } = target else { continue };
            let Some(source) = crate::world::hand_ik::object_source(world, *id) else {
                continue;
            };
            let link = if source.root == *id {
                crate::world::rig_graph::ROOT_LINK.to_owned()
            } else {
                let name = if name.is_empty() {
                    build
                        .props
                        .get(id)
                        .map(|p| p.name.as_str())
                        .filter(|n| !n.is_empty())
                        .or_else(|| world.ik_targets.name(*id))
                        .unwrap_or(name)
                } else {
                    name
                };
                if name.is_empty() || name == crate::world::rig_graph::ROOT_LINK {
                    continue;
                }
                name.to_owned()
            };
            references.insert(*id, Target::Link { object: source.root, link });
        }
        self.prims.references = references;
    }

    pub(super) fn remember_link_reference(&mut self, world: &World, target: &Target) {
        if let Target::Prim { id, name } = target
            && let Some(source) = crate::world::hand_ik::object_source(world, *id)
            && (source.root == *id || (!name.is_empty() && name != crate::world::rig_graph::ROOT_LINK))
        {
            self.prims.references.insert(
                *id,
                Target::Link {
                    object: source.root,
                    link: if source.root == *id {
                        crate::world::rig_graph::ROOT_LINK.into()
                    } else {
                        name.clone()
                    },
                },
            );
        }
    }

    pub fn link_target_definition(&self) -> Option<(&crate::world::rig_graph::Definition, Option<Uuid>)> {
        (self.open && !self.poser.active).then_some((&self.graph.definition, self.graph.source))
    }

    pub(super) fn stored_definition(&self) -> crate::world::rig_graph::Definition {
        let mut definition = self.graph.definition.clone();
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
        }
        definition
    }

    pub(super) fn target_prim(&self, world: &World, target: &Target) -> Option<Uuid> {
        match target {
            Target::Prim { id, .. } => Some(*id),
            Target::Source => self.graph.source,
            Target::Link { object, link } => world
                .ik_targets
                .index(
                    world,
                    self.graph.source.and_then(|id| crate::world::hand_ik::object_source(world, id)),
                    *object,
                    link,
                )
                .and_then(|i| world.objects.get(i))
                .map(|o| o.full_id),
            _ => None,
        }
    }

    pub(super) fn sync_prim_draft(&mut self) {
        if self.clip.active || self.picking || self.prims.manual {
            return;
        }
        if let Some((id, name)) = &self.prims.edited {
            self.target = Some(*id);
            self.target_name.clone_from(name);
            if let Target::Prim { id: current, .. } = self.graph.target
                && (current != *id || current.is_nil())
            {
                self.graph.target = Target::Prim {
                    id: *id,
                    name: name.clone(),
                };
            }
        }
    }

    pub(super) fn prim_picking(&mut self, on: bool, node: Option<Uuid>) {
        self.cancel_object_import_pick();
        self.cancel_project_target_pick();
        self.picking = on;
        self.links.bone_pick = false;
        if node.is_some() {
            self.links.source = None;
        }
        self.prims.node = node;
        self.drag = None;
        self.gizmo.cancel();
    }

    pub fn picked_prim(&mut self, world: &World, build: &BuildTool, index: usize) -> bool {
        if !self.open || !self.picking || self.links.bone_pick || !usable(world, index) {
            return false;
        }
        if self.project_target_picking() {
            return self.picked_project_target(world, build, index);
        }
        if self.object_import_picking() {
            return self.picked_object_import(world, build, index);
        }
        let Some(object) = world.objects.get(index) else { return false };
        let id = object.full_id;
        let name = build
            .props
            .get(&id)
            .map(|p| p.name.as_str())
            .filter(|n| !n.is_empty())
            .or_else(|| world.ik_targets.name(id))
            .unwrap_or_default()
            .to_owned();
        let target = Target::Prim { id, name: name.clone() };
        if let Some(node) = self.prims.node {
            let Some(node) = self.graph.definition.nodes.iter_mut().find(|n| n.id == node) else {
                return false;
            };
            node.parent = target;
        } else if self.links.source.is_some() {
            let Some(frame) = Frame::new(world) else { return false };
            if !self.finish_link(world, &frame, target) {
                return false;
            }
        } else {
            self.graph.target = target;
        }
        self.target = Some(id);
        self.target_name = name;
        self.prims.edited = edited_prim(world, build);
        self.prims.manual = true;
        self.capture_link_references(world, build);
        self.prim_picking(false, None);
        true
    }
}

fn edited_prim(world: &World, build: &BuildTool) -> Option<(Uuid, String)> {
    (build.open && build.tool == BuildMode::Edit && build.selection.len() == 1)
        .then(|| world.objects.index_of(&build.selection[0]))
        .flatten()
        .filter(|i| usable(world, *i))
        .and_then(|i| world.objects.get(i))
        .map(|o| {
            let name = build
                .props
                .get(&o.full_id)
                .map(|p| p.name.as_str())
                .filter(|n| !n.is_empty())
                .or_else(|| world.ik_targets.name(o.full_id))
                .unwrap_or_default();
            (o.full_id, name.to_owned())
        })
}

fn usable(world: &World, index: usize) -> bool {
    world.objects.get(index).is_some_and(|o| !o.is_avatar())
        && Scene::object_transform(world, index, Instant::now(), 0).is_some_and(|(_, _, hud)| !hud)
}

#[cfg(test)]
mod tests;
