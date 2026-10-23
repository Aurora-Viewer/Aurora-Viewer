//! Named authoring groups share the runtime profile selectors. Clipboard
//! payloads carry the referenced virtual hierarchy, never runtime sessions.
use super::*;
use crate::world::rig_graph::{self as data, Binding, Definition, Node, Profile, Target};
use serde::{Deserialize, Serialize};

#[derive(Default)]
pub(super) struct State {
    paste: bool,
    rename: bool,
    create: Option<String>,
    focus: bool,
    pub error: bool,
    pub animations: Vec<animations::SavedStack>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Clipboard {
    aurora_ik_binding: u8,
    binding: Binding,
    nodes: Vec<Node>,
}

impl Editor {
    pub(super) fn ensure_profiles(&mut self, world: &World, frame: &Frame) {
        if !self.bindings.is_empty() {
            let previous = self.graph.definition.bindings.len();
            self.graph.definition = self.graph_definition();
            self.bindings.clear();
            for slot in previous..self.graph.definition.bindings.len() {
                self.assign_profile_binding(slot);
            }
        }
        // Authoring groups keep stable slots, including when their last
        // binding is removed. Bone-based declarations remain valid on input.
        for i in 0..self.graph.definition.profiles.len() {
            if self.graph.definition.profiles[i].bindings.is_none() {
                let members = self
                    .graph
                    .definition
                    .bindings
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, b)| self.graph.definition.profiles[i].contains(slot, &b.bone).then_some(slot))
                    .collect();
                self.graph.definition.profiles[i].bones.clear();
                self.graph.definition.profiles[i].bindings = Some(members);
            }
        }
        if self.graph.definition.profiles.is_empty()
            && (!self.graph.definition.bindings.is_empty() || !self.graph.definition.nodes.is_empty() || !self.animations.rows.is_empty())
        {
            let ([ease_in, ease_out], _) = self.global_smoothing();
            self.graph.definition.profiles.push(Profile {
                name: aurora_i18n::tr!("ik-profile-default").to_string(),
                bones: Vec::new(),
                bindings: Some((0..self.graph.definition.bindings.len()).collect()),
                smoothing: Some([ease_in, ease_out]),
            });
        }
        if self.graph.definition.profiles.is_empty() {
            self.graph.profile = None;
            return;
        }
        if self.graph.profile.is_none_or(|i| i >= self.graph.definition.profiles.len()) {
            self.switch_profile(0, world, frame);
        }
    }

    pub(super) fn assign_profile_binding(&mut self, index: usize) {
        let Some(profile) = self.graph.profile else { return };
        let Some(p) = self.graph.definition.profiles.get(profile) else {
            return;
        };
        let mut members: Vec<_> = self
            .graph
            .definition
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(i, b)| (i != index && p.contains(i, &b.bone)).then_some(i))
            .collect();
        members.push(index);
        let p = &mut self.graph.definition.profiles[profile];
        p.bones.clear();
        p.bindings = Some(members);
    }

    pub(super) fn switch_profile(&mut self, profile: usize, world: &World, frame: &Frame) {
        if profile >= self.graph.definition.profiles.len() {
            return;
        }
        self.profiles
            .animations
            .resize_with(self.graph.definition.profiles.len(), Default::default);
        if let Some(old) = self.graph.profile.filter(|i| *i < self.profiles.animations.len()) {
            self.profiles.animations[old] = self.animations.save();
        } else if self.profiles.animations.iter().all(|s| s.rows.is_empty()) && !self.animations.rows.is_empty() {
            self.profiles.animations[profile] = self.animations.save();
        }
        self.animations.restore(&self.profiles.animations[profile]);
        self.graph.profile = Some(profile);
        self.links.filter.clear();
        self.links.copied = None;
        self.profiles.rename = false;
        self.graph.binding = None;
        self.cancel_link_pick();
        let rig = &world.avatar_lib.rig;
        let bones: Vec<_> = self
            .graph
            .definition
            .bindings
            .iter()
            .filter_map(|b| rig.names.get(&b.bone).copied())
            .collect();
        self.goals.retain(|g| !bones.contains(&g.bone));
        self.disabled.retain(|b| !bones.contains(b));
        self.update_graph(world, frame);
        for bone in bones {
            self.sync_graph_activation(bone, rig);
        }
        if let Some(bone) = self.selected {
            self.select(bone, rig, frame);
        }
        self.drag = None;
        self.gizmo.cancel();
        self.refresh = true;
    }

    fn unique_profile_name(&self, base: &str) -> String {
        if !self.graph.definition.profiles.iter().any(|p| p.name == base) {
            return base.to_owned();
        }
        let mut base = base.to_owned();
        while base.len() > 54 {
            base.pop();
        }
        let mut suffix = 2;
        loop {
            let name = aurora_i18n::tr!("ik-profile-numbered", name = &base, number = suffix).to_string();
            if !self.graph.definition.profiles.iter().any(|p| p.name == name) {
                return name;
            }
            suffix += 1;
        }
    }

    pub(super) fn add_profile(&mut self, duplicate: bool, world: &World, frame: &Frame) -> bool {
        self.ensure_profiles(world, frame);
        let name = self.unique_profile_name(if duplicate {
            let Some(source) = self.graph.profile.and_then(|i| self.graph.definition.profiles.get(i)) else {
                return false;
            };
            &source.name
        } else {
            aurora_i18n::text("ik-profile-default")
        });
        self.add_named_profile(duplicate, name, world, frame)
    }

    fn add_named_profile(&mut self, duplicate: bool, name: String, world: &World, frame: &Frame) -> bool {
        if !self.valid_new_profile_name(&name) {
            return false;
        }
        if self.graph.definition.profiles.len() >= data::MAX_BINDINGS {
            return false;
        }
        let selected = self.graph.profile.unwrap_or(0);
        let source = self.graph.definition.profiles.get(selected).cloned();
        if duplicate && source.is_none() {
            return false;
        }
        let members: Vec<_> = self
            .graph
            .definition
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(i, b)| (duplicate && source.as_ref().is_some_and(|p| p.contains(i, &b.bone))).then_some(i))
            .collect();
        if self.graph.definition.bindings.len() + members.len() > data::MAX_BINDINGS {
            return false;
        }
        let old = self.graph.definition.clone();
        let mut copied = Vec::new();
        for index in members {
            copied.push(self.graph.definition.bindings.len());
            self.graph.definition.bindings.push(self.graph.definition.bindings[index].clone());
        }
        self.graph.definition.profiles.push(Profile {
            name,
            bones: Vec::new(),
            bindings: Some(copied),
            smoothing: if duplicate {
                source.and_then(|p| p.smoothing).or(Some(self.global_smoothing().0))
            } else {
                Some([0.2, 0.2])
            },
        });
        if self.graph.definition.validate().is_err()
            || serde_json::to_vec(&self.graph.definition).map_or(true, |b| b.len() > data::MAX_BYTES)
        {
            self.graph.definition = old;
            return false;
        }
        self.profiles
            .animations
            .resize_with(self.graph.definition.profiles.len(), Default::default);
        if duplicate {
            let last = self.profiles.animations.len() - 1;
            self.profiles.animations[last] = self.animations.save();
        }
        self.switch_profile(self.graph.definition.profiles.len() - 1, world, frame);
        true
    }

    fn delete_profile(&mut self, world: &World, frame: &Frame) {
        let Some(index) = self.graph.profile else { return };
        if self.graph.definition.profiles.len() <= 1 {
            return;
        }
        let source = self.graph.definition.profiles.remove(index);
        let removed: Vec<_> = self
            .graph
            .definition
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                (source.contains(i, &b.bone) && !self.graph.definition.profiles.iter().any(|p| p.contains(i, &b.bone))).then_some(i)
            })
            .collect();
        for i in removed.into_iter().rev() {
            self.graph.definition.remove_binding(i);
        }
        if index < self.profiles.animations.len() {
            self.profiles.animations.remove(index);
        }
        self.animations = animations::Stack::default();
        self.graph.profile = None;
        self.switch_profile(index.min(self.graph.definition.profiles.len() - 1), world, frame);
    }

    pub(super) fn binding_clipboard(&self, index: usize) -> Option<String> {
        let definition = self.stored_definition();
        let binding = definition.bindings.get(index)?.clone();
        let mut nodes = Vec::new();
        let mut target = &binding.target;
        while let Target::Node { id } = target {
            if nodes.len() >= data::MAX_NODES || nodes.iter().any(|n: &Node| n.id == *id) {
                return None;
            }
            let node = definition.nodes.iter().find(|n| n.id == *id)?;
            nodes.push(node.clone());
            target = &node.parent;
        }
        let text = serde_json::to_string(&Clipboard {
            aurora_ik_binding: 1,
            binding,
            nodes,
        })
        .ok()?;
        (text.len() <= data::MAX_BYTES).then_some(text)
    }

    pub(super) fn copy_profile_binding(&mut self, index: usize, ctx: &egui::Context) {
        if let Some(text) = self.binding_clipboard(index) {
            ctx.copy_text(text);
            self.acknowledge_copy(ctx, links::Copy::Binding(index));
            self.profiles.error = false;
        } else {
            self.profiles.error = true;
        }
    }

    pub fn take_binding_paste_request(&mut self) -> bool {
        std::mem::take(&mut self.profiles.paste)
    }

    pub(super) fn binding_paste_ui(&mut self, ui: &mut egui::Ui) {
        if ui.button(aurora_i18n::tr!("ik-profile-paste-binding")).clicked() {
            self.profiles.paste = true;
        }
    }

    pub fn paste_binding(&mut self, world: &World, text: &str) {
        self.profiles.error = true;
        let Some(frame) = Frame::new(world) else { return };
        if self.paste_binding_text(world, &frame, text) {
            self.profiles.error = false;
        }
    }

    pub(super) fn paste_binding_text(&mut self, world: &World, frame: &Frame, text: &str) -> bool {
        if text.len() > data::MAX_BYTES {
            return false;
        }
        let Ok(mut copied) = serde_json::from_str::<Clipboard>(text) else {
            return false;
        };
        if copied.aurora_ik_binding != 1 || !world.avatar_lib.rig.names.contains_key(&copied.binding.bone) {
            return false;
        }
        let check = Definition {
            profiles: Vec::new(),
            nodes: copied.nodes.clone(),
            bindings: vec![copied.binding.clone()],
        };
        if check.validate().is_err() {
            return false;
        }
        self.ensure_profiles(world, frame);
        if self.graph.definition.bindings.len() >= data::MAX_BINDINGS
            || self.graph.definition.nodes.len() + copied.nodes.len() > data::MAX_NODES
        {
            return false;
        }
        let remap: std::collections::HashMap<_, _> = copied.nodes.iter().map(|n| (n.id, Uuid::new_v4())).collect();
        let target = |t: &mut Target| {
            if let Target::Node { id } = t
                && let Some(new) = remap.get(id)
            {
                *id = *new;
            }
        };
        for node in &mut copied.nodes {
            node.id = remap[&node.id];
            target(&mut node.parent);
        }
        target(&mut copied.binding.target);
        let old = self.graph.definition.clone();
        self.graph.definition.nodes.extend(copied.nodes);
        let index = self.graph.definition.bindings.len();
        self.graph.definition.bindings.push(copied.binding);
        self.assign_profile_binding(index);
        if self.graph.definition.validate().is_err()
            || serde_json::to_vec(&self.graph.definition).map_or(true, |b| b.len() > data::MAX_BYTES)
        {
            self.graph.definition = old;
            return false;
        }
        self.update_graph(world, frame);
        if self.graph.error {
            self.graph.definition = old;
            self.update_graph(world, frame);
            return false;
        }
        self.select_binding(index, world, frame);
        self.refresh = true;
        true
    }

    fn valid_new_profile_name(&self, name: &str) -> bool {
        !name.is_empty()
            && name.trim() == name
            && name.len() <= 64
            && !name.chars().any(|c| c.is_control() || c == '|')
            && !self.graph.definition.profiles.iter().any(|p| p.name == name)
    }

    pub(super) fn begin_profile_creation(&mut self) {
        self.profiles.create = Some(self.unique_profile_name(aurora_i18n::text("ik-profile-default")));
        self.profiles.focus = true;
    }

    fn create_profile_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let Some(mut name) = self.profiles.create.take() else { return };
        let mut confirmed = false;
        let mut cancelled = false;
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .id(egui::Id::unique("ik-profile-new-name"))
                    .char_limit(64)
                    .desired_width((ui.available_width() - 110.0).max(80.0)),
            );
            if self.profiles.focus {
                response.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                    state.store(ui.ctx(), response.id);
                }
                self.profiles.focus = false;
            }
            let valid = self.valid_new_profile_name(name.trim());
            confirmed = ui
                .add_enabled(valid, egui::Button::new(aurora_i18n::tr!("ik-library-create")))
                .clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid);
            cancelled = widgets::icon_button(ui, palette, "x", aurora_i18n::tr!("common-cancel"), true)
                || ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
        if confirmed {
            self.profiles.error = !self.add_named_profile(false, name.trim().to_owned(), world, frame);
        } else if !cancelled {
            self.profiles.create = Some(name);
        }
    }

    pub(super) fn profile_selector_ui(&self, ui: &mut egui::Ui, selected: &mut usize, salt: &str, width: f32) {
        let Some(profile) = self.graph.definition.profiles.get(*selected) else {
            return;
        };
        egui::ComboBox::from_id_salt(salt)
            .selected_text(&profile.name)
            .width(width)
            .truncate()
            .show_ui(ui, |ui| {
                for (i, p) in self.graph.definition.profiles.iter().enumerate() {
                    ui.selectable_value(selected, i, &p.name);
                }
            })
            .response
            .on_hover_text(&profile.name);
    }

    pub(super) fn profiles_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        self.ensure_profiles(world, frame);
        if self.graph.definition.profiles.is_empty() {
            if self.profiles.create.is_none() && ui.button(aurora_i18n::tr!("ik-profile-add")).clicked() {
                self.begin_profile_creation();
            }
            self.create_profile_ui(ui, palette, world, frame);
            return;
        }
        widgets::surface(ui, palette, |ui| {
            ui.set_min_width(ui.available_width());
            let mut selected = self.graph.profile.unwrap_or(0);
            ui.horizontal(|ui| {
                ui.strong(aurora_i18n::tr!("ik-profile-title"));
                let width = (ui.available_width() - 140.0).max(80.0);
                self.profile_selector_ui(ui, &mut selected, "ik-profile-selection", width);
                if widgets::icon_button(ui, palette, "pencil-filled", aurora_i18n::tr!("ik-profile-rename"), true) {
                    self.profiles.rename = !self.profiles.rename;
                }
                if widgets::icon_button(
                    ui,
                    palette,
                    "plus",
                    aurora_i18n::tr!("ik-profile-add"),
                    self.graph.definition.profiles.len() < data::MAX_BINDINGS,
                ) {
                    self.begin_profile_creation();
                }
                if widgets::icon_button(ui, palette, "copy-filled", aurora_i18n::tr!("ik-profile-duplicate"), true) {
                    self.profiles.error = !self.add_profile(true, world, frame);
                    selected = self.graph.profile.unwrap_or(0);
                }
                if widgets::icon_button(
                    ui,
                    palette,
                    "trash-filled",
                    aurora_i18n::tr!("ik-profile-delete"),
                    self.graph.definition.profiles.len() > 1,
                ) {
                    self.delete_profile(world, frame);
                    selected = self.graph.profile.unwrap_or(0);
                }
            });
            if self.graph.profile != Some(selected) {
                self.switch_profile(selected, world, frame);
            }
            if self.profiles.rename {
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("ik-profile-name"));
                    let name = &mut self.graph.definition.profiles[selected].name;
                    if ui
                        .add(egui::TextEdit::singleline(name).char_limit(64).desired_width(ui.available_width()))
                        .lost_focus()
                    {
                        *name = name.trim().to_owned();
                    }
                });
            }
            self.create_profile_ui(ui, palette, world, frame);
            self.smoothing_ui(ui, palette);
            self.bindings_ui(ui, palette, world, frame);

            if self.profiles.error || self.graph.definition.validate().is_err() {
                ui.colored_label(palette.muted, aurora_i18n::tr!("ik-profile-invalid"));
            }
        });
    }
}

#[cfg(test)]
pub(super) mod tests;
