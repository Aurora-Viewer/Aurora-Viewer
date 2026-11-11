//! Shared numeric pose controls for the selected source and native clip editor.
use super::*;
impl Editor {
    /// A released constraint retains calibration, but its next manual edit
    /// starts from the displayed joint without changing the authored chain mode.
    pub(super) fn released_pose(&mut self, frame: &Frame) {
        let Some(bone) = self.selected else { return };
        let local = frame.model.inverse() * frame.joints[bone];
        self.position = local.transform_point3(self.contact);
        self.set_rotation(local.to_scale_rotation_translation().1);
        self.control = Control::Bone;
        self.pole = None;
        self.pole_orbit = None;
    }
    pub(super) fn pose_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let rig = &world.avatar_lib.rig;
        if self.graph.selected.is_some()
            && let Some(bone) = self.selected
        {
            self.select(bone, rig, frame);
        }

        let names: Vec<_> = rig.canonical_names.iter().enumerate().map(|(i, n)| (i, n.as_str())).collect();
        if self.control == Control::Pole {
            ui.colored_label(palette.violet_light, aurora_i18n::tr!("ik-edit-pole-hint"));
        }
        let Some(bone) = self.selected else {
            return;
        };
        let name = names.iter().find(|n| n.0 == bone).map_or("", |n| n.1);
        if self.control == Control::Pole {
            ui.strong(self.pole_label(rig, bone, name));
        }
        let mut changed = false;
        let mut rotate = false;
        if self.control == Control::Pole {
            ui.label(aurora_i18n::tr!("ik-edit-pole-position"));
            changed |= vector_fields(ui, self.pole.get_or_insert(self.position), 0.005);
        } else {
            let (moved, rotated) = widgets::collapsing(
                ui,
                palette,
                ("ik-source-avatar", bone),
                aurora_i18n::tr!("ik-inspector-avatar-frame"),
                false,
                |ui| transform_fields(ui, "ik-source-pose", &mut self.position, Some(&mut self.degrees), 0.005),
            )
            .body
            .unwrap_or((false, false));
            changed |= moved;
            if rotated {
                self.orient = true;
                changed = true;
                rotate = true;
            }
        }
        if changed {
            self.commit_manipulation(rig, frame, rotate);
        }
        let bound = self.graph.binding.and_then(|index| self.graph.binding_goal(index, rig));
        self.goal_error_ui(ui, palette, frame, bone, bound.as_ref());
    }
    fn pole_label<'a>(&self, rig: &Rig, bone: usize, name: &'a str) -> &'a str {
        limbs(rig)
            .into_iter()
            .find(|l| l.end == bone)
            .map_or(name, |l| aurora_i18n::text(l.pole_label))
    }
    /// How far the solved joint stays from its goal, when it cannot reach it.
    fn goal_error_ui(&self, ui: &mut egui::Ui, palette: &Palette, frame: &Frame, bone: usize, bound: Option<&Goal>) {
        if self.enabled(bone)
            && let Some(goal) = bound.or_else(|| self.goals.iter().find(|g| g.bone == bone))
        {
            let actual = frame
                .model
                .inverse()
                .transform_point3(frame.joints[bone].transform_point3(self.contact));
            let error = actual.distance(goal.position);
            if error > 0.005 {
                ui.colored_label(
                    palette.muted,
                    aurora_i18n::tr!("ik-edit-error", distance = format!("{:.1}", error * 1000.0)),
                );
            }
        }
    }
    /// The animation editor's Bone page: the picker, then the selected bone's
    /// chain mode, pose, track and resets. The rarely used controls wait in
    /// one folded Advanced section.
    pub(super) fn clip_bone_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let rig = &world.avatar_lib.rig;
        self.bone_picker_ui(ui, palette, world, frame);
        let Some(bone) = self.selected else { return };
        let name = rig.canonical_names.get(bone).map_or("", String::as_str);
        ui.add_space(ui.spacing().item_spacing.y);
        let pole = self.control == Control::Pole;
        ui.strong(if pole { self.pole_label(rig, bone, name) } else { name });
        self.chain_mode_ui(ui, palette, rig, frame, bone);
        if self.control == Control::Pole {
            ui.colored_label(palette.violet_light, aurora_i18n::tr!("ik-edit-pole-hint"));
            ui.label(aurora_i18n::tr!("ik-edit-pole-position"));
            if vector_fields(ui, self.pole.get_or_insert(self.position), 0.005) {
                self.commit_manipulation(rig, frame, false);
            }
        } else {
            let (moved, rotated) = transform_fields(ui, "animation-bone-pose", &mut self.position, Some(&mut self.degrees), 0.005);
            let mut changed = moved;
            if rotated {
                self.orient = true;
                changed = true;
            }
            if bone == rig.pelvis {
                let mut lift = self.position.z - rig.local_pos[bone].z;
                ui.label(aurora_i18n::tr!("anim-edit-root-height"));
                if ui
                    .add(egui::Slider::new(&mut lift, -1.5..=1.5).suffix(" m"))
                    .on_hover_text(aurora_i18n::tr!("anim-edit-root-height-hint"))
                    .changed()
                {
                    self.position.z = rig.local_pos[bone].z + lift;
                    changed = true;
                }
            }
            if changed {
                self.commit_manipulation(rig, frame, rotated);
            }
        }
        ui.add_space(ui.spacing().item_spacing.y);
        self.clip_bone_track_ui(ui, palette, bone);
        ui.horizontal(|ui| {
            if ui.button(aurora_i18n::tr!("ik-edit-reset-bone")).clicked() {
                self.reset_bone(bone, rig);
            }
            if ui.button(aurora_i18n::tr!("ik-edit-reset-all")).clicked() {
                self.reset_pose();
            }
        });
        widgets::collapsing(
            ui,
            palette,
            "animation-bone-advanced",
            aurora_i18n::tr!("anim-panel-advanced"),
            false,
            |ui| self.clip_bone_advanced_ui(ui, palette, world, frame, bone),
        );
        self.goal_error_ui(ui, palette, frame, bone, None);
    }
    /// IK moves the selected bone with its parent chain, whose root follows;
    /// FK moves or rotates this bone alone. The skeleton root has no chain.
    fn chain_mode_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, rig: &Rig, frame: &Frame, bone: usize) {
        let has_parent = rig.parent[bone].is_some();
        ui.horizontal_wrapped(|ui| {
            let mut mode = usize::from(!has_parent || self.mode == Mode::Direct);
            let choice = ui.add_enabled_ui(has_parent, |ui| {
                widgets::segmented(
                    ui,
                    palette,
                    &mut mode,
                    &[(aurora_i18n::tr!("anim-panel-ik"), true), (aurora_i18n::tr!("anim-panel-fk"), true)],
                )
            });
            choice.response.on_hover_text(aurora_i18n::tr!("anim-panel-ik-hint"));
            if choice.inner {
                self.set_chain_ik(mode == 0, rig, frame);
            }
            if self.mode == Mode::Direct {
                self.pole = None;
                self.control = Control::Bone;
            } else {
                self.chain_root_ui(ui, rig, frame, bone);
            }
        });
    }
    /// The bone that ends the selected IK chain, from the bone's own ancestors.
    fn chain_root_ui(&mut self, ui: &mut egui::Ui, rig: &Rig, frame: &Frame, bone: usize) {
        let mut ancestors = vec![bone];
        while ancestors.len() < 12 {
            let Some(parent) = rig.parent[*ancestors.last().unwrap_or(&bone)] else {
                break;
            };
            ancestors.push(parent);
        }
        let label = rig.canonical_names.get(self.root).map_or("", String::as_str);
        let mut changed = false;
        ui.label(aurora_i18n::tr!("ik-edit-root"));
        egui::ComboBox::from_id_salt("ik-root").selected_text(label).show_ui(ui, |ui| {
            for root in ancestors {
                if let Some(name) = rig.canonical_names.get(root) {
                    changed |= ui.selectable_value(&mut self.root, root, name).changed();
                }
            }
        });
        if !ik::supports_pole(rig, bone, self.root) {
            self.pole = None;
            self.control = Control::Bone;
        }
        if changed {
            self.drag = None;
            self.gizmo.cancel();
            self.commit_manipulation(rig, frame, false);
        }
    }
    /// Constraint switch and strength, orientation, contact point, the limb
    /// constraints, gizmo space and symmetry: set once, rarely changed.
    fn clip_bone_advanced_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame, bone: usize) {
        let rig = &world.avatar_lib.rig;
        ui.label(egui::RichText::new(aurora_i18n::tr!("ik-edit-gizmo-hint")).color(palette.muted));
        let mut enabled = self.enabled(bone);
        if widgets::switch_labeled(ui, palette, &mut enabled, aurora_i18n::tr!("ik-edit-enabled")).changed() {
            self.enable(bone, enabled, rig, frame);
        }
        let mut percent = self.weight * 100.0;
        ui.label(aurora_i18n::tr!("ik-edit-weight"));
        if ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).suffix(" %")).changed() {
            self.weight = percent / 100.0;
            self.commit();
        }
        if self.control != Control::Pole {
            let mut changed = widgets::switch_labeled(ui, palette, &mut self.orient, aurora_i18n::tr!("ik-edit-orient"))
                .on_hover_text(aurora_i18n::tr!("ik-edit-orient-hint"))
                .changed();
            changed |= self.contact_ui(ui, palette, world, frame, bone);
            if changed {
                self.commit_manipulation(rig, frame, false);
            }
        }
        ui.strong(aurora_i18n::tr!("ik-edit-enable-limbs"));
        self.limb_options_ui(ui, palette, world, frame, 2);
        ui.horizontal_wrapped(|ui| {
            self.space_ui(ui, palette);
            self.manipulation_options_ui(ui, palette);
        });
    }
    fn contact_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame, bone: usize) -> bool {
        let mut changed = if self.clip.active {
            ui.label(aurora_i18n::tr!("ik-edit-contact"));
            vector_fields(ui, &mut self.contact, 0.001)
        } else {
            ui.add_space(ui.spacing().item_spacing.y);
            coordinate_heading(
                ui,
                palette,
                aurora_i18n::tr!("ik-inspector-contact"),
                aurora_i18n::tr!("ik-inspector-bone-frame"),
                aurora_i18n::tr!("ik-edit-contact"),
            );
            transform_fields(ui, "ik-source-contact-point", &mut self.contact, None, 0.001).0
        };
        if ["mAnkleLeft", "mAnkleRight"].contains(&world.avatar_lib.rig.canonical_names[bone].as_str())
            && ui.button(aurora_i18n::tr!("ik-edit-sole")).clicked()
        {
            self.contact = Vec3::new(0.08, 0.0, -0.05);
            changed = true;
        }
        if changed {
            self.position = frame
                .model
                .inverse()
                .transform_point3(frame.joints[bone].transform_point3(self.contact));
        }
        changed
    }
    /// Switch the selected bone between its IK chain and a direct (FK) joint
    /// edit, shared by the Pose page and the timeline's posing toggle.
    pub(super) fn set_chain_ik(&mut self, ik: bool, rig: &Rig, frame: &Frame) {
        let Some(bone) = self.selected else { return };
        if rig.parent[bone].is_none() || (self.mode == Mode::Ik) == ik {
            return;
        }
        self.mode = if ik { Mode::Ik } else { Mode::Direct };
        if ik {
            // Direct bindings resolve with root == bone. Returning to IK must
            // restore an actual chain, otherwise translation has no joints.
            if self.root == bone {
                self.root = ik::extended_root(rig, bone).unwrap_or_else(|| ik::default_root(rig, bone));
            }
            if let Some(limb) = limbs(rig).into_iter().find(|l| l.end == bone)
                && ik::supports_pole(rig, bone, self.root)
            {
                self.pole = Some(limb.pole(frame));
            }
        } else {
            self.pole = None;
            self.control = Control::Bone;
        }
        self.drag = None;
        self.gizmo.cancel();
        self.commit_manipulation(rig, frame, false);
    }
    pub(super) fn chain_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame, bone: usize) {
        ui.horizontal_wrapped(|ui| {
            let rig = &world.avatar_lib.rig;
            let has_parent = rig.parent[bone].is_some();
            let mut detached = has_parent && self.mode == Mode::Direct;
            let toggled = ui
                .add_enabled_ui(has_parent, |ui| {
                    widgets::checkbox(ui, palette, &mut detached, aurora_i18n::tr!("ik-edit-detach"))
                        .on_hover_text(aurora_i18n::tr!("ik-edit-detach-hint"))
                        .changed()
                })
                .inner;
            if toggled {
                self.set_chain_ik(!detached, rig, frame);
            }
            if self.mode == Mode::Direct {
                self.pole = None;
                self.control = Control::Bone;
            }
            if self.mode == Mode::Ik {
                self.chain_root_ui(ui, rig, frame, bone);
            }
        });
    }
    pub(super) fn reset_bone(&mut self, bone: usize, rig: &Rig) {
        let counterpart = self.symmetry.then(|| self.mirrors.counterpart(bone)).flatten();
        self.authoring.remove(bone);
        if let Some(other) = counterpart {
            self.authoring.remove(other);
        }
        self.goals.retain(|g| g.bone != bone && Some(g.bone) != counterpart);
        if !self.clip.active {
            self.bindings.retain(|b| b.bone != bone && Some(b.bone) != counterpart);
            let removed: Vec<_> = self
                .graph
                .definition
                .bindings
                .iter()
                .enumerate()
                .filter_map(|(i, b)| {
                    (self.graph.visible_binding(i) && rig.names.get(&b.bone).is_some_and(|j| *j == bone || Some(*j) == counterpart))
                        .then_some(i)
                })
                .collect();
            for index in removed.into_iter().rev() {
                self.graph.definition.remove_binding(index);
            }
            self.graph.binding = None;
        }
        self.disabled.push(bone);
        if let Some(bone) = counterpart {
            self.disabled.push(bone);
        }
        self.refresh = true;
    }
    pub(super) fn reset_pose(&mut self) {
        self.authoring = authoring::State::default();
        self.disabled.extend(self.goals.iter().map(|g| g.bone));
        self.goals.clear();
        if !self.clip.active {
            self.bindings.clear();
            let removed: Vec<_> = (0..self.graph.definition.bindings.len())
                .filter(|&i| self.graph.visible_binding(i))
                .collect();
            for index in removed.into_iter().rev() {
                self.graph.definition.remove_binding(index);
            }
            self.graph.binding = None;
        }
        self.refresh = true;
    }
    pub(super) fn manipulation_ui(&self, ui: &mut egui::Ui) {
        ui.label(aurora_i18n::tr!("ik-edit-gizmo-hint"));
    }
    pub(super) fn space_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let mut space = usize::from(self.space == Space::Local);
        if widgets::segmented(
            ui,
            palette,
            &mut space,
            &[
                (aurora_i18n::tr!("ik-edit-world"), false),
                (aurora_i18n::tr!("ik-edit-bone-local"), false),
            ],
        ) {
            self.space = if space == 0 { Space::World } else { Space::Local };
            self.drag = None;
            self.gizmo.cancel();
        }
    }
    pub(super) fn manipulation_options_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        widgets::switch_labeled(ui, palette, &mut self.symmetry, aurora_i18n::tr!("ik-edit-symmetry"));
    }
    pub(super) fn limb_options_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame, columns: usize) {
        let rig = &world.avatar_lib.rig;
        egui::Grid::new("ik-limb-options")
            .num_columns(columns)
            .min_col_width(0.0)
            .show(ui, |ui| {
                for row in limbs(rig).chunks(columns) {
                    for limb in row {
                        let mut enabled = self.limb_enabled(limb.end);
                        if widgets::checkbox(ui, palette, &mut enabled, aurora_i18n::text(limb.label)).changed() {
                            self.enable_limb(limb.end, enabled, rig, frame);
                        }
                    }
                    ui.end_row();
                }
            });
    }
    pub(super) fn visibility_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        if self.visibility.show(ui, palette) {
            self.drag = None;
            self.gizmo.cancel();
        }
        widgets::switch_labeled(ui, palette, &mut self.limb_controls, aurora_i18n::tr!("ik-edit-limbs"));
        crate::ui::widgets::search_text_edit(ui, palette, &mut self.filter, |edit| {
            edit.hint_text(aurora_i18n::tr!("ik-edit-filter"))
        });
        let filter = self.filter.to_ascii_lowercase();
        for (bone, name) in world.avatar_lib.rig.canonical_names.iter().enumerate() {
            if self.visibility.visible(name)
                && name.to_ascii_lowercase().contains(&filter)
                && widgets::search_list_item(ui, self.selected == Some(bone), name).clicked()
            {
                if self.links.bone_pick {
                    self.picked_bone(
                        world,
                        frame,
                        crate::world::rig_graph::Target::Bone {
                            avatar: Uuid::nil(),
                            bone: name.clone(),
                        },
                    );
                } else {
                    self.select(bone, &world.avatar_lib.rig, frame);
                }
            }
        }
    }
}

pub(super) fn coordinate_heading(ui: &mut egui::Ui, palette: &Palette, title: &str, frame: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.strong(title);
        ui.label(egui::RichText::new(frame).color(palette.muted)).on_hover_text(hint);
    });
}

/// Equal XYZ columns keep position and rotation readable even when values have
/// different signs or precision. Numeric edits retain the existing pose paths.
pub(super) fn transform_fields(
    ui: &mut egui::Ui,
    id: &str,
    position: &mut Vec3,
    rotation: Option<&mut Vec3>,
    position_speed: f64,
) -> (bool, bool) {
    let labels = [aurora_i18n::tr!("ik-inspector-position"), aurora_i18n::tr!("ik-inspector-rotation")];
    let font = egui::TextStyle::Body.resolve(ui.style());
    let label_width = labels
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap((*label).into(), font.clone(), ui.visuals().text_color())
                .size()
                .x
        })
        .fold(0.0_f32, f32::max);
    let spacing = ui.spacing().item_spacing;
    let width = ((ui.available_width() - label_width - 3.0 * spacing.x) / 3.0).clamp(36.0, 104.0);
    let height = ui.spacing().interact_size.y;
    egui::Grid::new(id)
        .num_columns(4)
        .min_col_width(0.0)
        .show(ui, |ui| {
            let mut row = |label: &str, value: &mut Vec3, speed| {
                ui.label(label);
                let mut changed = false;
                for axis in 0..3 {
                    changed |= ui
                        .add_sized(
                            egui::vec2(width, height),
                            egui::DragValue::new(&mut value[axis])
                                .prefix(["X ", "Y ", "Z "][axis])
                                .speed(speed)
                                .max_decimals(6),
                        )
                        .changed();
                }
                ui.end_row();
                changed
            };
            (
                row(labels[0], position, position_speed),
                rotation.is_some_and(|rotation| row(labels[1], rotation, 0.5)),
            )
        })
        .inner
}

#[cfg(test)]
mod tests;
