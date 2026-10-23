//! Persistent editor associations. Offsets are captured from authored goals,
//! never from the partially weighted rendered pose, so moving a prim cannot
//! progressively bake the blend into its own target.
use super::*;
use crate::world::hand_ik::Constraint;

pub(super) struct Binding {
    pub bone: usize,
    pub name: String,
    pub definition: Constraint,
    pub orientation: Quat,
    pub available: bool,
    to_target: Mat4,
}

impl Binding {
    fn frame(&mut self, world: &World, frame: &Frame) -> bool {
        self.available = false;
        let Some(index) = world.objects.index_of_uuid(&self.definition.target) else {
            return false;
        };
        let Some((p, q, false)) = Scene::object_transform(world, index, frame.at, 0) else {
            return false;
        };
        let (_, model_q, model_p) = frame.model.to_scale_rotation_translation();
        // Subtract region positions before rotating. Multiplying two world
        // matrices first loses contact precision through large translations.
        self.to_target = Mat4::from_rotation_translation((q.inverse() * model_q).normalize(), q.inverse() * (model_p - p));
        self.available = self.to_target.is_finite();
        self.available
    }

    pub fn capture(&mut self, goal: &Goal, root_name: Option<&str>) {
        if !self.available {
            return;
        }
        let q = self.to_target.to_scale_rotation_translation().1;
        if let Some(rotation) = goal.rotation {
            self.orientation = (q * rotation).normalize();
        }
        self.definition.position = self.to_target.transform_point3(goal.position).to_array();
        self.definition.rotation = goal.rotation.map(|r| (q * r).normalize().to_array());
        self.definition.pole = goal.pole.map(|p| self.to_target.transform_point3(p).to_array());
        self.definition.contact = goal.contact.to_array();
        self.definition.root = root_name.map(str::to_owned);
        self.definition.mode = goal.mode;
        self.definition.weight = goal.weight;
    }

    fn goal(&self, rig: &Rig) -> Option<Goal> {
        let d = &self.definition;
        if !self.available || !d.valid() {
            return None;
        }
        let transform = self.to_target.inverse();
        let q = transform.to_scale_rotation_translation().1;
        Some(Goal {
            follow: None,
            bone: self.bone,
            root: match (d.mode, &d.root) {
                (Mode::Direct, _) => self.bone,
                (_, Some(name)) => *rig.names.get(name)?,
                (_, None) => ik::default_root(rig, self.bone),
            },
            position: transform.transform_point3(Vec3::from_array(d.position)),
            rotation: d.rotation.map(|r| (q * Quat::from_array(r)).normalize()),
            contact: Vec3::from_array(d.contact),
            pole: d.pole.map(|p| transform.transform_point3(Vec3::from_array(p))),
            mode: d.mode,
            weight: d.weight,
        })
    }
}

impl Editor {
    pub(super) fn import_script_bindings(&mut self, world: &World, frame: &Frame) {
        self.initialized = false;
        self.graph.imported = false;
        self.update_bindings(world, frame);
        self.update_graph(world, frame);
        self.ensure_profiles(world, frame);
        self.refresh = true;
    }

    /// Strength edits must not recapture target offsets, enable a disabled
    /// association, or select a different bone (including during virtual edits).
    pub(super) fn set_binding_weight(&mut self, bone: usize, rig: &Rig, weight: f32) {
        if !weight.is_finite() || !(0.0..=1.0).contains(&weight) {
            return;
        }
        for binding in self.bindings.iter_mut().filter(|b| b.bone == bone) {
            binding.definition.weight = weight;
        }
        if let Some(index) = self.graph.binding_index(bone, &rig.canonical_names)
            && let Some(binding) = self.graph.definition.bindings.get_mut(index)
        {
            binding.weight = weight;
        }
        for goal in self.goals.iter_mut().filter(|g| g.bone == bone) {
            goal.weight = weight;
        }
        if self.selected == Some(bone) && self.graph.selected.is_none() {
            self.weight = weight;
        }
    }

    /// Toggle orientation without recapturing offsets or changing activation,
    /// strength, selection or another association's calibration.
    pub(super) fn set_binding_orientation(&mut self, bone: usize, rig: &Rig, orient: bool) {
        let mut rotation = None;
        for binding in self.bindings.iter_mut().filter(|b| b.bone == bone) {
            binding.definition.rotation = orient.then(|| binding.orientation.to_array());
            rotation = binding.goal(rig).and_then(|g| g.rotation);
        }
        if let Some(name) = rig.canonical_names.get(bone) {
            if let Some(index) = self.graph.binding_index(bone, &rig.canonical_names)
                && let Some(binding) = self.graph.definition.bindings.get_mut(index)
            {
                binding.orient = orient;
            }
            if orient && self.graph.definition.bindings.iter().any(|b| b.bone == *name) {
                rotation = self.graph.binding_rotation(name);
            }
        }
        for goal in self.goals.iter_mut().filter(|g| g.bone == bone) {
            goal.rotation = rotation;
        }
        if self.selected == Some(bone) && self.graph.selected.is_none() {
            self.orient = orient;
            if let Some(rotation) = rotation {
                self.set_rotation(rotation);
            }
        }
    }

    pub(super) fn enabled(&self, bone: usize) -> bool {
        self.goals.iter().any(|g| g.bone == bone) && !self.disabled.contains(&bone)
    }
    pub(super) fn limb_enabled(&self, bone: usize) -> bool {
        self.enabled(bone) && !self.authoring.owns(bone) && self.goals.iter().any(|g| g.bone == bone && g.mode == Mode::Ik)
    }

    pub(super) fn enable_limb(&mut self, bone: usize, enabled: bool, rig: &Rig, frame: &Frame) {
        self.enable(bone, enabled, rig, frame);
        if !enabled {
            return;
        }
        let Some(limb) = limbs(rig).into_iter().find(|l| l.end == bone) else {
            return;
        };
        let Some(goal) = self.goals.iter_mut().find(|g| g.bone == bone && g.mode == Mode::Direct) else {
            return;
        };
        goal.mode = Mode::Ik;
        goal.root = ik::extended_root(rig, bone).unwrap_or(limb.root);
        goal.pole = Some(limb.pole(frame));
        let goal = goal.clone();
        if !self.clip.active {
            self.graph.capture(&goal, &rig.canonical_names);
            if let Some(binding) = self.bindings.iter_mut().find(|b| b.bone == bone) {
                binding.capture(&goal, rig.canonical_names.get(goal.root).map(String::as_str));
            }
        }
        if self.selected == Some(bone) {
            self.select(bone, rig, frame);
        }
    }

    pub(super) fn enable(&mut self, bone: usize, enabled: bool, rig: &Rig, frame: &Frame) {
        if enabled && self.authoring.owns(bone) {
            self.authoring.remove(bone);
            self.goals.retain(|g| g.bone != bone);
        }
        if enabled && limbs(rig).iter().any(|l| l.end == bone) {
            self.limb_controls = true;
            self.visibility.reveal(&rig.canonical_names[bone]);
        }
        self.disabled.retain(|b| *b != bone);
        if !self.clip.active
            && let Some(name) = rig.canonical_names.get(bone)
        {
            let indices: Vec<_> = self
                .graph
                .definition
                .bindings
                .iter()
                .enumerate()
                .filter_map(|(i, b)| (&b.bone == name && self.graph.visible_binding(i)).then_some(i))
                .collect();
            for index in indices {
                let binding = &mut self.graph.definition.bindings[index];
                binding.enabled = enabled;
            }
        }
        if enabled {
            if !self.goals.iter().any(|g| g.bone == bone) {
                self.select(bone, rig, frame);
                if let Some(limb) = limbs(rig).into_iter().find(|l| l.end == bone) {
                    self.root = ik::extended_root(rig, bone).unwrap_or(limb.root);
                    self.pole = Some(limb.pole(frame));
                }
                // Activation is deliberately per limb even with symmetry on.
                let symmetry = self.symmetry;
                self.symmetry = false;
                self.commit();
                self.symmetry = symmetry;
            }
            // Existing pins retain their creator-selected root. Extended roots
            // are defaults for new pins, never an activation-time migration.
            if self.selected == Some(bone) {
                // Selection of an inactive pin rebases its displayed transform.
                // Restoring the pin must also restore the inspector's target.
                self.select(bone, rig, frame);
            }
        } else {
            self.disabled.push(bone);
            self.drag = None;
            self.gizmo.cancel();
            if self.selected == Some(bone) {
                self.released_pose(frame);
                self.refresh = true;
            }
        }
    }

    pub(super) fn attach(&mut self, world: &World, frame: &Frame) {
        let (Some(bone), Some(target)) = (self.selected, self.target) else {
            return;
        };
        self.orient = true;
        self.commit();
        let rig = &world.avatar_lib.rig;
        let Some(goal) = self.goals.iter().find(|g| g.bone == bone) else {
            return;
        };
        let mut binding = Binding {
            bone,
            name: self.target_name.clone(),
            definition: Constraint {
                activation: self.binding_activation(bone, rig).unwrap_or_else(|| self.new_binding_activation()),
                bone: rig.canonical_names[bone].clone(),
                target,
                position: [0.0; 3],
                rotation: None,
                contact: [0.0; 3],
                root: None,
                mode: self.mode,
                pole: None,
                weight: goal.weight,
            },
            orientation: Quat::IDENTITY,
            available: false,
            to_target: Mat4::IDENTITY,
        };
        if !binding.frame(world, frame) {
            return;
        }
        binding.capture(goal, rig.canonical_names.get(goal.root).map(String::as_str));
        if !binding.definition.valid() {
            return;
        }
        self.bindings.retain(|b| b.bone != bone);
        self.authoring.remove(bone);
        self.remember_link_reference(
            world,
            &crate::world::rig_graph::Target::Prim {
                id: target,
                name: binding.name.clone(),
            },
        );
        self.bindings.push(binding);
    }

    pub(super) fn update_bindings(&mut self, world: &World, frame: &Frame) {
        let rig = &world.avatar_lib.rig;
        self.initialized = true;
        for binding in &mut self.bindings {
            if !binding.frame(world, frame) {
                continue;
            }
            if let Some(goal) = binding.goal(rig) {
                if let Some(old) = self.goals.iter_mut().find(|g| g.bone == goal.bone) {
                    *old = goal
                } else {
                    self.goals.push(goal)
                }
            } else {
                binding.available = false;
            }
        }
        if let Some(bone) = self.selected
            && !self.disabled.contains(&bone)
            && self.bindings.iter().any(|b| b.bone == bone && b.available)
            && let Some(goal) = self.goals.iter().find(|g| g.bone == bone).cloned()
        {
            self.position = goal.position;
            self.pole = goal.pole;
            if let Some(q) = goal.rotation {
                self.set_rotation(q)
            }
        }
    }

    pub(super) fn bindings_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        ui.horizontal(|ui| {
            ui.strong(aurora_i18n::tr!("ik-edit-bindings"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.binding_paste_ui(ui));
        });
        let count = self
            .graph
            .definition
            .bindings
            .iter()
            .enumerate()
            .filter(|(i, _)| self.graph.visible_binding(*i))
            .count()
            + self.bindings.len();
        if count > 0 || !self.links.filter.is_empty() {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - ui.spacing().interact_size.y - ui.spacing().item_spacing.x).max(60.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.links.filter)
                        .id_salt("ik-binding-filter")
                        .hint_text(aurora_i18n::tr!("ik-binding-filter"))
                        .desired_width(width)
                        .char_limit(128),
                );
                if widgets::icon_button(
                    ui,
                    palette,
                    "x",
                    aurora_i18n::tr!("ik-binding-clear-filter"),
                    !self.links.filter.is_empty(),
                ) {
                    self.links.filter.clear();
                }
            });
        }
        self.graph_bindings_ui(ui, palette, world, frame);
        let mut chosen = None;
        let mut removed = None;
        let mut activation = None;
        let mut edit = None;
        let mut weight = None;
        let mut orientation = None;
        for binding in &self.bindings {
            let mut enabled = self.enabled(binding.bone);
            let target = self.prim_label(binding.definition.target, &binding.name);
            if !self.binding_matches(&binding.definition.bone, &target) {
                continue;
            }
            ui.push_id(("ik-binding", binding.bone), |ui| {
                ui.horizontal(|ui| {
                    if widgets::checkbox(ui, palette, &mut enabled, "").changed() {
                        activation = Some((binding.bone, enabled));
                    }
                    let (source_clicked, target_clicked) = binding_labels(
                        ui,
                        self.selected == Some(binding.bone) && self.graph.selected.is_none(),
                        &binding.definition.bone,
                        &target,
                    );
                    if source_clicked {
                        chosen = Some(binding.bone);
                    }
                    if target_clicked {
                        self.prims.edit = Some(binding.definition.target);
                    }
                    let reveal = self.links.reveal_menu && self.selected == Some(binding.bone);
                    if reveal {
                        self.links.reveal_menu = false;
                    }
                    if let Some(prim) = target_menu(ui, palette, reveal) {
                        edit = Some((binding.bone, prim));
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("ik-edit-weight"));
                    let mut percent = binding.definition.weight * 100.0;
                    if weight_slider(ui, &mut percent) {
                        weight = Some((binding.bone, percent / 100.0));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, palette, "trash-filled", aurora_i18n::tr!("ik-edit-unbind"), true) {
                            removed = Some(binding.bone);
                        }
                        if rotation_toggle(ui, palette, binding.definition.rotation.is_some()) {
                            orientation = Some((binding.bone, binding.definition.rotation.is_none()));
                        }
                    });
                });
                if !binding.available {
                    ui.colored_label(palette.muted, aurora_i18n::tr!("ik-edit-target-lost"));
                }
            });
        }
        let rig = &world.avatar_lib.rig;
        if let Some((bone, weight)) = weight {
            self.set_binding_weight(bone, rig, weight);
        }
        if let Some((bone, orient)) = orientation {
            self.set_binding_orientation(bone, rig, orient);
        }
        if let Some((bone, enabled)) = activation {
            self.enable(bone, enabled, rig, frame);
        }
        if let Some(bone) = removed {
            self.bindings.retain(|b| b.bone != bone);
            self.goals.retain(|g| g.bone != bone);
            self.disabled.push(bone);
            self.refresh = true;
        }
        if let Some(bone) = chosen {
            self.visibility.reveal(&rig.canonical_names[bone]);
            self.select(bone, rig, frame);
            self.tab = 1;
        }
        if let Some((bone, prim)) = edit {
            self.begin_link(world, frame, bone, prim);
        }
        if !self.links.filter.trim().is_empty()
            && !self
                .graph
                .definition
                .bindings
                .iter()
                .enumerate()
                .any(|(i, b)| self.graph.visible_binding(i) && self.binding_matches(&b.bone, &self.target_label(&b.target, world)))
            && !self
                .bindings
                .iter()
                .any(|b| self.binding_matches(&b.definition.bone, &self.prim_label(b.definition.target, &b.name)))
        {
            ui.colored_label(palette.muted, aurora_i18n::tr!("ik-binding-no-match"));
        }
    }

    pub(super) fn export_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let advanced = !self.graph.definition.nodes.is_empty() || !self.graph.definition.bindings.is_empty();
        let valid = if advanced {
            self.graph_payload().is_some() && !self.graph.error
        } else {
            (1..=4).contains(&self.bindings.len()) && self.bindings.iter().all(|b| b.definition.valid())
        };
        if !valid && (!self.bindings.is_empty() || advanced) {
            ui.colored_label(
                palette.muted,
                if self.graph.error {
                    aurora_i18n::tr!("ik-graph-cycle")
                } else {
                    aurora_i18n::tr!("ik-graph-export-invalid")
                },
            );
        }
        ui.horizontal(|ui| {
            let width = ((ui.available_width() - ui.spacing().item_spacing.x) * 0.5).floor();
            for (kind, label, icon) in [
                (links::Copy::Script, "ik-edit-copy-config", "code"),
                (links::Copy::Data, "ik-edit-copy-data", "clipboard-text-filled"),
            ] {
                let copied = self.copied(ui.ctx(), kind);
                let response = widgets::action_button_if(
                    ui,
                    palette,
                    if copied { "circle-check-filled" } else { icon },
                    aurora_i18n::text(if copied { "ik-copy-done" } else { label }),
                    widgets::ActionKind::Flat,
                    egui::vec2(width, ui.spacing().interact_size.y),
                    valid,
                );
                let response = if kind == links::Copy::Data {
                    response.on_hover_text(aurora_i18n::tr!("ik-edit-copy-data-hint"))
                } else {
                    response
                };
                if response.clicked()
                    && let Some(text) = if kind == links::Copy::Script {
                        self.script()
                    } else {
                        self.graph_rig_declaration()
                    }
                {
                    ui.ctx().copy_text(text);
                    self.acknowledge_copy(ui.ctx(), kind);
                }
            }
        });
    }

    pub(super) fn script(&self) -> Option<String> {
        self.graph_script()
    }
}

/// Keep names legible; influence and secondary actions use the next line.
pub(super) fn binding_labels(ui: &mut egui::Ui, selected: bool, source: &str, target: &str) -> (bool, bool) {
    let reserved = ui.spacing().interact_size.y + 14.0 + 4.0 * ui.spacing().item_spacing.x;
    let width = ((ui.available_width() - reserved) * 0.5).max(32.0);
    let source = ui
        .add_sized(
            egui::vec2(width, ui.spacing().interact_size.y),
            egui::Button::selectable(selected, source).truncate(),
        )
        .on_hover_text(source)
        .clicked();
    ui.label("→");
    let target = ui
        .add_sized(
            egui::vec2(width, ui.spacing().interact_size.y),
            egui::Button::new(target).truncate(),
        )
        .on_hover_text(target)
        .clicked();
    (source, target)
}

pub(super) fn weight_slider(ui: &mut egui::Ui, percent: &mut f32) -> bool {
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = 68.0;
        ui.add(egui::Slider::new(percent, 0.0..=100.0).suffix(" %").max_decimals(1))
            .on_hover_text(aurora_i18n::tr!("ik-edit-weight"))
            .changed()
    })
    .inner
}

pub(super) fn rotation_toggle(ui: &mut egui::Ui, palette: &Palette, on: bool) -> bool {
    widgets::icon_toggle(
        ui,
        palette,
        "rotate-clockwise",
        &format!(
            "{}\n{}",
            aurora_i18n::tr!("ik-edit-orient"),
            aurora_i18n::tr!("ik-edit-orient-hint")
        ),
        on,
    )
}

/// Scene target designation uses the same icon button and menu as other floaters.
pub(super) fn target_menu(ui: &mut egui::Ui, palette: &Palette, reveal: bool) -> Option<bool> {
    use crate::ui::menu;
    let response = widgets::action_button(
        ui,
        palette,
        "pencil-filled",
        "",
        widgets::ActionKind::Flat,
        egui::Vec2::splat(ui.spacing().interact_size.y),
    )
    .on_hover_text(aurora_i18n::tr!("ik-edit-edit-target"));
    if reveal {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    let mut choice = None;
    menu::dropdown(&response, palette, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        if menu::item(ui, palette, "box", aurora_i18n::tr!("ik-graph-prim")) {
            choice = Some(true);
        }
        if menu::item(ui, palette, "bone-filled", aurora_i18n::tr!("ik-graph-bone")) {
            choice = Some(false);
        }
    });
    choice
}
