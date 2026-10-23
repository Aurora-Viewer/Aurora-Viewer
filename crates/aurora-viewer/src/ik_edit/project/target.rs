//! Project-wide object designation. Named anchors are resolved only inside
//! that linkset; replacing an object never recaptures bone calibration.
use super::*;
use crate::{build::BuildTool, world::rig_graph::Target};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct State {
    pub root: Option<Uuid>,
    pub name: String,
    pub picking: bool,
    pub(super) selected: Option<Uuid>,
    pub(super) unresolved: usize,
}

fn root(world: &World, mut index: usize) -> Option<usize> {
    let mut seen = HashSet::new();
    for _ in 0..64 {
        let object = world.objects.get(index)?;
        if object.is_avatar() || !seen.insert(index) {
            return None;
        }
        if object.parent_id == 0 {
            return Some(index);
        }
        let parent = world.objects.parent_of(object)?;
        if world.objects.get(parent)?.is_avatar() {
            return Some(index);
        }
        index = parent;
    }
    None
}

fn members(world: &World, id: Uuid) -> Vec<Uuid> {
    let Some(index) = world.objects.index_of_uuid(&id) else {
        return Vec::new();
    };
    let mut pending = vec![index];
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(index) = pending.pop() {
        if !seen.insert(index) || seen.len() > 512 {
            break;
        }
        let Some(object) = world.objects.get(index).filter(|o| !o.is_avatar()) else {
            continue;
        };
        result.push(object.full_id);
        // Seated avatars and their worn descendants are not linkset members.
        pending.extend(world.objects.children_of(&object.key).iter().copied().take(512 - result.len()));
    }
    result
}

impl Editor {
    pub(in crate::ik_edit) fn project_target_demo(&mut self, mode: &str) {
        let root = if mode == "library-target-missing" {
            Uuid::from_u128(0xdead8811)
        } else {
            crate::demo::action_id(crate::demo::hand_ik::ROOT)
        };
        self.graph.source = Some(root);
        self.project.target = State {
            root: Some(root),
            name: "Contact rig".into(),
            ..Default::default()
        };
        if mode == "library-target-pick" {
            self.begin_project_target_pick();
        }
    }
    pub(in crate::ik_edit) fn take_project_target_selection(&mut self) -> Option<Uuid> {
        self.project.target.selected.take()
    }
    pub(in crate::ik_edit) fn project_target_picking(&self) -> bool {
        self.project.target.picking
    }
    pub(in crate::ik_edit) fn cancel_project_target_pick(&mut self) {
        self.project.target.picking = false;
    }
    pub(in crate::ik_edit) fn begin_project_target_pick(&mut self) {
        self.cancel_link_pick();
        self.prim_picking(true, None);
        self.project.target.picking = true;
    }
    pub(in crate::ik_edit) fn picked_project_target(&mut self, world: &World, build: &BuildTool, index: usize) -> bool {
        let Some(index) = root(world, index) else { return false };
        let Some(object) = world.objects.get(index) else { return false };
        if !Scene::object_transform(world, index, Instant::now(), 0).is_some_and(|(_, _, hud)| !hud) {
            return false;
        }
        if let Some(name) = build
            .props
            .get(&object.full_id)
            .map(|p| p.name.as_str())
            .filter(|n| !n.is_empty())
            .or_else(|| world.ik_targets.name(object.full_id))
        {
            self.prims.names.insert(object.full_id, name.to_owned());
        }
        let old = self.graph.source;
        self.capture_link_references(world, build);
        for reference in self.prims.references.values_mut() {
            if let Target::Link { object: root, .. } = reference
                && Some(*root) == old
            {
                *root = object.full_id;
            }
        }
        self.graph.source = Some(object.full_id);
        for target in self
            .graph
            .definition
            .nodes
            .iter_mut()
            .map(|n| &mut n.parent)
            .chain(self.graph.definition.bindings.iter_mut().map(|b| &mut b.target))
        {
            if let Target::Link { object: target_root, .. } = target
                && Some(*target_root) == old
            {
                *target_root = object.full_id;
            }
            if matches!(target, Target::Prim { id, .. } if Some(*id) == old) {
                *target = Target::Prim {
                    id: object.full_id,
                    name: String::new(),
                };
            }
        }
        self.project.target.root = self.graph.source;
        self.project.target.name = self.prims.names.get(&object.full_id).cloned().unwrap_or_default();
        self.object_data = object_data::State::default();
        self.cancel_link_pick();
        self.sync_project_target(world, build);
        self.capture_link_references(world, build);
        if let Some(frame) = Frame::new(world) {
            self.update_graph(world, &frame);
        }
        self.refresh = self.graph.selected.is_none();
        true
    }
    pub(in crate::ik_edit) fn sync_project_target(&mut self, world: &World, build: &BuildTool) -> Vec<Uuid> {
        let Some(source) = self.graph.source else { return Vec::new() };
        if self.project.target.root != Some(source) {
            self.project.target = State {
                root: Some(source),
                ..Default::default()
            };
        }
        if let Some(name) = build
            .props
            .get(&source)
            .map(|p| p.name.as_str())
            .or_else(|| world.ik_targets.name(source))
        {
            self.project.target.name = name.to_owned();
            while self.project.target.name.len() > 256 {
                self.project.target.name.pop();
            }
        }
        let ids = members(world, source);
        let complete = !ids.is_empty() && ids.iter().all(|id| build.props.contains_key(id));
        let mut names: HashMap<&str, Option<Uuid>> = HashMap::new();
        for id in &ids {
            if let Some(props) = build.props.get(id).filter(|p| !p.name.is_empty()) {
                names.entry(&props.name).and_modify(|i| *i = None).or_insert(Some(*id));
            }
        }
        self.project.target.unresolved = 0;
        for target in self
            .graph
            .definition
            .nodes
            .iter_mut()
            .map(|n| &mut n.parent)
            .chain(self.graph.definition.bindings.iter_mut().map(|b| &mut b.target))
        {
            if let Target::Link { object, link } = target
                && (object.is_nil() || *object == source)
                && link != crate::world::rig_graph::ROOT_LINK
                && (names.get(link.as_str()) == Some(&None) || (complete && !names.contains_key(link.as_str())))
            {
                self.project.target.unresolved += 1;
            }
            if let Target::Prim { id, name } = target {
                if let Some(Target::Link { object, .. }) = self.prims.references.get(id)
                    && *object != source
                {
                    continue;
                }
                // A scene pick may precede its property reply. Once every prim
                // is known, retain a unique name for this exact local child so
                // copied products can resolve the same calibrated frame. Never
                // relabel external UUIDs, the source root or authored names.
                if name.is_empty()
                    && *id != source
                    && complete
                    && ids.contains(id)
                    && let Some(props) = build.props.get(id).filter(|p| !p.name.is_empty() && p.name.len() <= 256)
                    && names.get(props.name.as_str()) == Some(&Some(*id))
                {
                    name.clone_from(&props.name);
                }
                if name.is_empty() {
                    continue;
                }
                let resolved = if names.get(name.as_str()) == Some(&None) {
                    None
                } else if complete {
                    names.get(name.as_str()).copied().flatten()
                } else {
                    // A known in-linkset UUID remains usable while its property
                    // replies arrive. Replacements wait for unambiguous names.
                    (ids.contains(id) && build.props.get(id).is_none_or(|p| p.name == *name)).then_some(*id)
                };
                *id = resolved.unwrap_or_default();
                self.project.target.unresolved += usize::from(resolved.is_none());
            }
        }
        ids
    }
    pub(in crate::ik_edit) fn project_target_ui(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("ik-project-target"));
            let label = if self.graph.source.is_none() {
                aurora_i18n::tr!("ik-project-target-none").to_string()
            } else if self.project.target.name.is_empty() {
                self.graph.source.map(|id| self.prim_label(id, "")).unwrap_or_default()
            } else {
                self.project.target.name.clone()
            };
            ui.add(widgets::elided_label(ui, &label, (ui.available_width() - 155.0).max(40.0)))
                .on_hover_text(&label);
            if ui
                .add_enabled(!self.project.busy, egui::Button::new(aurora_i18n::tr!("ik-project-target-pick")))
                .clicked()
            {
                self.begin_project_target_pick();
            }
        });
        if self.project.target.picking {
            ui.label(aurora_i18n::tr!("ik-project-target-hint"));
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.cancel_link_pick();
            }
            if !ui.ctx().is_pointer_over_egui() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        } else if let Some((id, _)) = self.prims.edited
            && ui
                .add_enabled(!self.project.busy, egui::Button::new(aurora_i18n::tr!("ik-project-target-edited")))
                .clicked()
        {
            self.project.target.selected = Some(id);
        }
        if self.graph.source.is_some_and(|id| world.objects.index_of_uuid(&id).is_none()) {
            ui.colored_label(p.muted, aurora_i18n::tr!("ik-project-target-missing"));
        } else if self.project.target.unresolved > 0 {
            ui.colored_label(
                p.muted,
                aurora_i18n::tr!("ik-project-target-unresolved", count = self.project.target.unresolved),
            );
        }
    }
}

#[cfg(test)]
mod tests;
