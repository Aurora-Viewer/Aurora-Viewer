//! Guided association creation: lock the source while a scene target is being
//! picked, then apply it atomically without moving the authored source pose.
use super::*;
use crate::world::rig_graph::{Activation, Target};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Copy {
    Script,
    Data,
    Binding(usize),
}

#[derive(Default)]
pub(super) struct State {
    pub source: Option<usize>,
    pub bone_pick: bool,
    pub reveal_menu: bool,
    pub error: bool,
    pub smoothing: Option<[f32; 2]>,
    pub footer_height: f32,
    pub reveal_offsets: bool,
    pub adding: bool,
    pub filter: String,
    pub copied: Option<(Copy, f64)>,
}
impl Editor {
    pub(super) fn binding_matches(&self, source: &str, target: &str) -> bool {
        if self.links.filter.trim().is_empty() {
            return true;
        }
        let label = format!("{source} {target}").to_lowercase();
        self.links
            .filter
            .split_whitespace()
            .all(|term| label.contains(&term.to_lowercase()))
    }

    pub(super) fn copied(&self, ctx: &egui::Context, action: Copy) -> bool {
        self.links.copied.is_some_and(|(kind, at)| {
            let elapsed = ctx.input(|i| i.time) - at;
            kind == action && (0.0..2.0).contains(&elapsed)
        })
    }

    pub(super) fn acknowledge_copy(&mut self, ctx: &egui::Context, action: Copy) {
        self.links.copied = Some((action, ctx.input(|i| i.time)));
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
    }

    fn begin_new_link(&mut self, world: &World, frame: &Frame, bone: usize, prim: bool) {
        self.begin_link(world, frame, bone, prim);
        self.links.adding = true;
    }
    pub(super) fn begin_link(&mut self, world: &World, frame: &Frame, bone: usize, prim: bool) {
        self.select(bone, &world.avatar_lib.rig, frame);
        self.links.source = Some(bone);
        self.links.adding = false;
        self.links.error = false;
        self.prim_picking(true, None);
        self.links.bone_pick = !prim;
    }
    pub(super) fn cancel_link_pick(&mut self) {
        self.cancel_object_import_pick();
        self.cancel_project_target_pick();
        self.prim_picking(false, None);
        self.links.bone_pick = false;
        self.links.error = false;
        self.links.source = None;
        self.links.adding = false;
    }
    pub(super) fn picked_bone(&mut self, world: &World, frame: &Frame, target: Target) -> bool {
        if !self.picking || !self.links.bone_pick {
            return false;
        }
        self.finish_link(world, frame, target)
    }
    pub(super) fn finish_link(&mut self, world: &World, frame: &Frame, target: Target) -> bool {
        let Some(source) = self.links.source else { return false };
        if !self.bind_target(world, frame, source, target) {
            self.links.error = true;
            return false;
        }
        self.links.filter.clear();
        self.cancel_link_pick();
        true
    }
    pub(super) fn link_pick_status_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World) {
        if !self.picking {
            return;
        }
        if self.project_target_picking() || self.object_import_picking() {
            return;
        }
        if let Some(source) = self.links.source.and_then(|b| world.avatar_lib.rig.canonical_names.get(b)) {
            selected_source_ui(ui, palette, source, |_| {});
        }
        ui.label(if self.links.bone_pick {
            aurora_i18n::tr!("ik-links-pick-bone-hint")
        } else {
            aurora_i18n::tr!("ik-links-pick-prim-hint")
        });
        if self.links.error {
            ui.label(aurora_i18n::tr!("ik-links-invalid-target"));
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.cancel_link_pick();
        }
        if !ui.ctx().is_pointer_over_egui() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
    pub(super) fn links_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        // The source is locked until designation ends. Keep competing source
        // selectors and pose controls out of this focused target-picking step.
        if self.picking {
            return;
        }
        self.ensure_profiles(world, frame);
        if self.graph.definition.profiles.is_empty() {
            self.profiles_ui(ui, palette, world, frame);
            return;
        }
        let rig = &world.avatar_lib.rig;
        if self.graph.selected.is_some()
            && let Some(bone) = self.selected
        {
            self.select(bone, rig, frame);
        }
        if self.links.source != self.selected {
            self.links.source = self.selected;
            self.links.error = false;
        }
        if let Some(bone) = self.selected {
            let name = &rig.canonical_names[bone];
            selected_source_ui(ui, palette, name, |ui| {
                if !self.picking {
                    self.bind_controls_ui(ui, world, frame);
                    ui.horizontal_wrapped(|ui| {
                        self.activation_ui(ui, rig, bone);
                        if ui.button(aurora_i18n::tr!("ik-edit-reset-bone")).clicked() {
                            self.reset_bone(bone, rig);
                        }
                    });
                    widgets::collapsing(
                        ui,
                        palette,
                        ("ik-selected-chain", bone),
                        aurora_i18n::tr!("ik-inspector-chain"),
                        false,
                        |ui| {
                            self.pose_ui(ui, palette, world, frame);
                            self.chain_ui(ui, palette, world, frame, bone);
                        },
                    );
                }
            });
        }
        if self.links.error && !self.picking {
            ui.colored_label(palette.muted, aurora_i18n::tr!("ik-links-invalid-target"));
        }
        self.profiles_ui(ui, palette, world, frame);
    }
    fn bind_controls_ui(&mut self, ui: &mut egui::Ui, world: &World, frame: &Frame) {
        let Some(bone) = self.selected else { return };
        ui.horizontal_wrapped(|ui| {
            let mut profile = self.graph.profile.unwrap_or(0);
            let width = (ui.available_width() * 0.38).clamp(100.0, 190.0);
            self.profile_selector_ui(ui, &mut profile, "ik-bind-profile", width);
            if self.graph.profile != Some(profile) {
                self.switch_profile(profile, world, frame);
            }
            if ui.button(aurora_i18n::tr!("ik-links-bind-prim")).clicked() {
                self.begin_new_link(world, frame, bone, true);
            }
            if ui.button(aurora_i18n::tr!("ik-links-bind-bone")).clicked() {
                self.begin_new_link(world, frame, bone, false);
            }
            if let Some((id, name)) = self.prims.edited.clone() {
                let label = aurora_i18n::tr!(
                    "ik-links-use-edited",
                    name = if name.is_empty() { id.to_string() } else { name.clone() }
                );
                let size = egui::vec2(ui.available_width().clamp(120.0, 210.0), ui.spacing().interact_size.y);
                if ui
                    .add_sized(size, egui::Button::new(&label).truncate())
                    .on_hover_text(label)
                    .clicked()
                {
                    self.links.source = Some(bone);
                    self.links.adding = true;
                    self.finish_link(world, frame, Target::Prim { id, name });
                }
            }
        });
    }
    pub(super) fn footer_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        widgets::surface(ui, palette, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add_enabled_ui(!self.picking, |ui| {
                ui.horizontal(|ui| {
                    widgets::section_header(ui, palette, "settings-filled", aurora_i18n::tr!("ik-links-options"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, palette, "file-import", aurora_i18n::tr!("ik-edit-import-scripts"), true) {
                            self.import_script_bindings(world, frame);
                        }
                    });
                });
                let font = egui::TextStyle::Button.resolve(ui.style());
                let ppp = ui.pixels_per_point();
                let limb_width = limbs(&world.avatar_lib.rig)
                    .iter()
                    .map(|limb| {
                        ui.painter()
                            .layout_no_wrap(aurora_i18n::text(limb.label).to_owned(), font.clone(), palette.ink)
                            .size()
                            .x
                            + ui.spacing().icon_width
                            + ui.spacing().icon_spacing
                    })
                    .fold(ui.spacing().interact_size.y, f32::max);
                let limb_width = (limb_width * ppp).ceil() / ppp;
                let right_width = 2.0 * limb_width + ui.spacing().item_spacing.x;
                // Round each checkbox column up and the flexible column down:
                // pixel rounding must not feed a wider footer into the floater.
                let left_width = ((ui.available_width() - right_width - ui.spacing().item_spacing.x) * ppp)
                    .floor()
                    .max(1.0)
                    / ppp;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(egui::vec2(left_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(left_width);
                        ui.horizontal_wrapped(|ui| {
                            self.space_ui(ui, palette);
                            self.manipulation_options_ui(ui, palette);
                            if ui.button(aurora_i18n::tr!("ik-edit-reset-all")).clicked() {
                                self.reset_pose();
                            }
                        });
                        widgets::switch_labeled(ui, palette, &mut self.ignore_script_ik, aurora_i18n::tr!("ik-edit-ignore-scripts"))
                            .on_hover_text(aurora_i18n::tr!("ik-edit-ignore-scripts-hint"));
                    });
                    ui.allocate_ui_with_layout(egui::vec2(right_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        self.limb_options_ui(ui, palette, world, frame, 2);
                    });
                });
            });
            self.export_ui(ui, palette);
        });
    }
    pub(super) fn binding_activation(&self, bone: usize, rig: &Rig) -> Option<crate::world::rig_graph::Activation> {
        self.graph
            .binding_index(bone, &rig.canonical_names)
            .and_then(|index| self.graph.definition.bindings.get(index))
            .map(|b| b.activation)
            .or_else(|| self.bindings.iter().find(|b| b.bone == bone).map(|b| b.definition.activation))
    }
    /// The authoring control is global; the existing wire format stores the
    /// durations with every binding so compatible observers need no new schema.
    /// Imported differences stay intact until the creator edits the control.
    pub(super) fn global_smoothing(&self) -> ([f32; 2], bool) {
        if let Some(values) = self
            .graph
            .profile
            .and_then(|i| self.graph.definition.profiles.get(i))
            .and_then(|p| p.smoothing)
        {
            return (values, false);
        }
        let mut durations = self
            .graph
            .definition
            .bindings
            .iter()
            .enumerate()
            .filter(|(i, _)| self.graph.visible_binding(*i))
            .map(|(_, b)| b.activation)
            .chain(self.bindings.iter().map(|b| b.definition.activation))
            .map(|a| [a.ease_in, a.ease_out]);
        let defaults = Activation::default();
        let first = durations
            .next()
            .unwrap_or(self.links.smoothing.unwrap_or([defaults.ease_in, defaults.ease_out]));
        (first, durations.any(|d| d != first))
    }
    pub(super) fn new_binding_activation(&self) -> Activation {
        let ([ease_in, ease_out], _) = self.global_smoothing();
        Activation {
            ease_in,
            ease_out,
            ..Default::default()
        }
    }
    pub(super) fn set_global_smoothing(&mut self, durations: [f32; 2]) {
        self.links.smoothing = Some(durations);
        let visible: Vec<_> = self
            .graph
            .definition
            .bindings
            .iter()
            .enumerate()
            .map(|(i, b)| {
                self.graph.visible_binding(i)
                    && !self
                        .graph
                        .definition
                        .profiles
                        .iter()
                        .enumerate()
                        .any(|(p, profile)| Some(p) != self.graph.profile && profile.contains(i, &b.bone))
            })
            .collect();
        if let Some(profile) = self.graph.profile.and_then(|i| self.graph.definition.profiles.get_mut(i)) {
            profile.smoothing = Some(durations);
        }
        for settings in self
            .graph
            .definition
            .bindings
            .iter_mut()
            .enumerate()
            .filter(|(i, _)| visible[*i])
            .map(|(_, b)| &mut b.activation)
            .chain(self.bindings.iter_mut().map(|b| &mut b.definition.activation))
        {
            settings.ease_in = durations[0];
            settings.ease_out = durations[1];
        }
    }
    pub(super) fn smoothing_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let (mut durations, mixed) = self.global_smoothing();
        let before = durations;
        self.links.smoothing = Some(durations);
        ui.horizontal_wrapped(|ui| {
            ui.strong(aurora_i18n::tr!("ik-smoothing-global"));
            let [ease_in, ease_out] = &mut durations;
            for (label, value) in [
                (aurora_i18n::tr!("ik-activation-in"), ease_in),
                (aurora_i18n::tr!("ik-activation-out"), ease_out),
            ] {
                ui.label(label).on_hover_text(aurora_i18n::tr!("ik-smoothing-global-hint"));
                timing_value(ui, value, 60.0, " s").on_hover_text(aurora_i18n::tr!("ik-activation-smooth-hint"));
            }
        });
        if durations != before {
            self.set_global_smoothing(durations);
        } else if mixed {
            ui.colored_label(palette.muted, aurora_i18n::tr!("ik-smoothing-mixed"));
        }
        ui.add_space(ui.spacing().item_spacing.y);
    }
    fn activation_ui(&mut self, ui: &mut egui::Ui, rig: &Rig, bone: usize) {
        let Some(mut settings) = self.binding_activation(bone, rig) else {
            return;
        };
        let before = settings;
        ui.push_id(("ik-binding-activation", bone), |ui| {
            egui::Grid::new("ik-activation-fields").num_columns(2).show(ui, |ui| {
                ui.label(aurora_i18n::tr!("ik-activation-distance"))
                    .on_hover_text(aurora_i18n::tr!("ik-activation-distance-hint"));
                timing_value(ui, &mut settings.distance, 64.0, " m");
                ui.end_row();
            });
        });
        if settings != before {
            for binding in self.bindings.iter_mut().filter(|b| b.bone == bone) {
                binding.definition.activation = settings;
            }
            if let Some(index) = self.graph.binding_index(bone, &rig.canonical_names)
                && let Some(binding) = self.graph.definition.bindings.get_mut(index)
            {
                binding.activation = settings;
            }
        }
    }
    pub(super) fn binding_details_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame, index: usize) {
        let reveal = if self.graph.binding == Some(index) {
            std::mem::take(&mut self.links.reveal_offsets).then_some(true)
        } else {
            None
        };
        widgets::collapsing_forced(
            ui,
            palette,
            ("ik-binding-contact", index),
            aurora_i18n::tr!("ik-edit-contact-settings"),
            false,
            reveal,
            |ui| {
                self.binding_calibration_ui(ui, palette, world, frame, index);
            },
        );
    }
    fn binding_calibration_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame, index: usize) {
        let Some(mut binding) = self.graph.definition.bindings.get(index).cloned() else {
            return;
        };
        pose::coordinate_heading(
            ui,
            palette,
            aurora_i18n::tr!("ik-inspector-target-offset"),
            aurora_i18n::tr!("ik-inspector-target-frame"),
            aurora_i18n::tr!("ik-graph-bound-offset"),
        );
        let mut p = binding.offset.p();
        let (z, y, x) = binding.offset.q().to_euler(EulerRot::ZYX);
        let mut degrees = Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI);
        let (moved, rotated) = pose::transform_fields(ui, "ik-bound-offset", &mut p, Some(&mut degrees), 0.005);
        let mut changed = moved;
        if rotated {
            binding.orient = true;
            changed = true;
        }
        if changed {
            binding.offset = crate::world::rig_graph::Transform::new(
                p,
                Quat::from_euler(
                    EulerRot::ZYX,
                    degrees.z.to_radians(),
                    degrees.y.to_radians(),
                    degrees.x.to_radians(),
                ),
            );
        }
        pose::coordinate_heading(
            ui,
            palette,
            aurora_i18n::tr!("ik-inspector-contact"),
            aurora_i18n::tr!("ik-inspector-bone-frame"),
            aurora_i18n::tr!("ik-edit-contact"),
        );
        let mut contact = Vec3::from_array(binding.contact);
        let mut calibrated = pose::transform_fields(ui, "ik-binding-contact-point", &mut contact, None, 0.001).0;
        if ["mAnkleLeft", "mAnkleRight"].contains(&binding.bone.as_str()) && ui.button(aurora_i18n::tr!("ik-edit-sole")).clicked() {
            contact = Vec3::new(0.08, 0.0, -0.05);
            calibrated = true;
        }
        if calibrated
            && contact.is_finite()
            && contact.length() <= 0.3
            && let Some(&bone) = world.avatar_lib.rig.names.get(&binding.bone)
            && let Some(target) = self.graph.binding_target(index)
        {
            // Calibration preserves the displayed joint. Its new material
            // point is measured against this association's target only.
            let local = frame.model.inverse() * frame.joints[bone];
            binding.contact = contact.to_array();
            binding.offset.position = (target.q().inverse() * (local.transform_point3(contact) - target.p())).to_array();
            changed = true;
        }
        if changed && binding.valid() {
            self.graph.definition.bindings[index] = binding;
            if self.graph.binding == Some(index) {
                self.select_binding(index, world, frame);
            }
        }
    }
}

fn timing_value(ui: &mut egui::Ui, value: &mut f32, max: f32, suffix: &str) -> egui::Response {
    ui.add(
        egui::DragValue::new(value)
            .range(0.0..=max)
            .speed(0.01)
            .min_decimals(2)
            .max_decimals(3)
            .suffix(suffix),
    )
}

fn selected_source_ui(ui: &mut egui::Ui, palette: &Palette, name: &str, body: impl FnOnce(&mut egui::Ui)) {
    widgets::surface(ui, palette, |ui| {
        ui.set_min_width(ui.available_width());
        widgets::section_header(ui, palette, "bone-filled", &aurora_i18n::tr!("ik-links-selected", bone = name));
        body(ui);
    });
    ui.add_space(ui.spacing().item_spacing.y);
}

#[cfg(test)]
mod tests;
