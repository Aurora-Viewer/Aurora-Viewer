//! Pose operations and hand presets from app_settings/poses/hand_presets/*.xml.
use super::*;
use std::collections::HashMap;

fn reflected(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, q.y, -q.z, q.w)
}
#[derive(Clone, Copy)]
pub(super) enum Operation {
    Mirror,
    Flip,
    LeftToRight,
    RightToLeft,
    ResetJoint,
    ResetAll,
    TPose,
    WorldLock,
}
impl Editor {
    pub(super) fn pose_operation(&mut self, rig: &Rig, frame: &Frame, operation: Operation) {
        self.mirrors.initialize(rig);
        let Some(session) = self.poser.sessions.get_mut(&self.poser.subject) else {
            return;
        };
        if !session.enabled {
            return;
        }
        let before = session.pose.clone();
        session.checkpoint();
        let pose = Arc::make_mut(&mut session.pose);
        let copy = |pose: &mut SubjectPose, from: usize, to: usize| {
            pose.rotations[to] = (rig.local_rot[to] * reflected(rig.local_rot[from].inverse() * before.rotations[from])).normalize();
            pose.positions[to] = session.base.positions[to] + rig_controls::reflect(before.positions[from] - session.base.positions[from]);
            pose.scales[to] = session.base.scales[to] * (before.scales[from] / session.base.scales[from]);
            pose.locks.remove(&to);
        };
        match operation {
            Operation::Mirror => {
                if let Some(b) = self.selected
                    && let Some(other) = self.mirrors.counterpart(b)
                {
                    copy(pose, b, other);
                }
            }
            Operation::Flip | Operation::LeftToRight | Operation::RightToLeft => {
                for b in 0..rig.len() {
                    if let Some(other) = self.mirrors.counterpart(b) {
                        let name = &rig.canonical_names[b];
                        let left = name.ends_with("Left") || name.starts_with("L_") || name.starts_with("LEFT_");
                        if matches!(operation, Operation::Flip)
                            || matches!(operation, Operation::LeftToRight) && left
                            || matches!(operation, Operation::RightToLeft) && !left
                        {
                            copy(pose, b, other);
                        }
                    } else if matches!(operation, Operation::Flip) {
                        pose.rotations[b] = reflected(before.rotations[b]);
                        pose.positions[b] =
                            session.base.positions[b] + rig_controls::reflect(before.positions[b] - session.base.positions[b]);
                        pose.locks.remove(&b);
                    }
                }
            }
            Operation::ResetJoint => {
                if let Some(b) = self.selected {
                    pose.rotations[b] = session.base.rotations[b];
                    pose.positions[b] = session.base.positions[b];
                    pose.scales[b] = session.base.scales[b];
                    pose.locks.remove(&b);
                }
            }
            Operation::ResetAll => *pose = (*session.base).clone(),
            Operation::TPose => {
                *pose = (*session.base).clone();
                pose.rotations.clone_from(&rig.local_rot);
                pose.locks.clear();
            }
            Operation::WorldLock => {
                if let Some(b) = self.selected
                    && pose.locks.remove(&b).is_none()
                {
                    pose.locks.insert(b, frame.joints[b]);
                }
            }
        }
        self.refresh = true;
    }
    /// A hand preset on one hand (`Some(true)`: the left), or both in one
    /// undo step.
    pub(super) fn hand_preset(&mut self, rig: &Rig, left: Option<bool>) {
        static PRESETS: std::sync::OnceLock<HashMap<String, HashMap<String, [f32; 3]>>> = std::sync::OnceLock::new();
        let presets = PRESETS.get_or_init(|| serde_json::from_str(include_str!("presets.json")).unwrap_or_default());
        let name = ["Relaxed", "Tight_Grip", "Flat", "Pointing", "Okay"][self.poser.preset.min(4)];
        let Some(preset) = presets.get(name) else { return };
        let Some(session) = self.poser.sessions.get_mut(&self.poser.subject) else {
            return;
        };
        session.checkpoint();
        let pose = Arc::make_mut(&mut session.pose);
        for (name, xyz) in preset {
            let applies = match left {
                Some(true) => name.ends_with("Left"),
                Some(false) => name.ends_with("Right"),
                None => name.ends_with("Left") || name.ends_with("Right"),
            };
            if applies && let Some(&bone) = rig.names.get(name) {
                pose.rotations[bone] = Quat::from_euler(EulerRot::ZYX, xyz[2], xyz[1], xyz[0]);
                pose.locks.remove(&bone);
            }
        }
        self.refresh = true;
    }
    /// The selected joint: its name and Bento code, its own actions (world
    /// pin, mirror, reset), the transform, the trackball and the X / Y / Z
    /// values, then the actions of the whole pose.
    pub(super) fn joint_controls(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        let Some(bone) = self.selected else { return };
        let name = &rig.canonical_names[bone];
        let locked = self
            .poser
            .sessions
            .get(&self.poser.subject)
            .is_some_and(|s| s.pose.locks.contains_key(&bone));
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let title = if bones::side_of(name).is_some() {
                    aurora_i18n::tr!("poser-joint-side", joint = bones::label(name), side = self.side_word()).to_string()
                } else {
                    bones::label(name)
                };
                ui.label(egui::RichText::new(title).size(16.0).strong().color(p.ink));
                ui.label(egui::RichText::new(name).size(11.5).monospace().color(p.muted_dim));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, p, "restore", aurora_i18n::text("ik-edit-reset-bone"), true) {
                    self.pose_operation(rig, frame, Operation::ResetJoint);
                }
                let paired = self.mirrors.counterpart(bone).is_some();
                if widgets::icon_button(ui, p, "arrows-horizontal", aurora_i18n::text("poser-mirror"), paired) {
                    self.pose_operation(rig, frame, Operation::Mirror);
                }
                if widgets::icon_toggle(ui, p, "pin-filled", aurora_i18n::text("poser-world-lock"), locked) {
                    self.pose_operation(rig, frame, Operation::WorldLock);
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let changed = widgets::segmented(
                ui,
                p,
                &mut self.poser.transform,
                &[
                    (aurora_i18n::text("poser-mode-rotation"), false),
                    (aurora_i18n::text("poser-mode-position"), false),
                    (aurora_i18n::text("poser-mode-scale"), false),
                ],
            );
            if changed {
                self.tool = if self.poser.transform == 0 { Tool::Rotate } else { Tool::Move };
                self.gizmo.cancel();
                self.poser.scale_gizmo.cancel();
            }
            ui.label(
                egui::RichText::new(aurora_i18n::text(
                    ["poser-hint-rotation", "poser-hint-position", "poser-hint-scale"][self.poser.transform.min(2)],
                ))
                .size(12.0)
                .color(p.muted),
            );
        });
        ui.add_space(6.0);
        let Some(session) = self.poser.sessions.get(&self.poser.subject) else {
            return;
        };
        let pose = session.pose.clone();
        let base = session.base.clone();
        let q = if self.poser.delta {
            base.rotations[bone].inverse() * pose.rotations[bone]
        } else {
            pose.rotations[bone]
        };
        let (z, y, x) = q.to_euler(EulerRot::ZYX);
        let base_point = (self.poser.transform == 1).then(|| base.matrices(rig, frame.model)[bone].w_axis.truncate());
        let mut value = match self.poser.transform {
            0 => Vec3::new(x, y, z) * 180.0 / std::f32::consts::PI,
            1 => {
                let point = pose.matrices(rig, frame.model)[bone].w_axis.truncate();
                point
                    - if self.poser.delta {
                        base_point.unwrap_or(Vec3::ZERO)
                    } else {
                        Vec3::ZERO
                    }
            }
            _ => {
                if self.poser.delta {
                    pose.scales[bone] / base.scales[bone]
                } else {
                    pose.scales[bone]
                }
            }
        };
        let mut edited = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.vertical(|ui| {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(TRACKBALL, TRACKBALL), Sense::drag());
                let painter = ui.painter();
                let center = rect.center();
                let radius = TRACKBALL / 2.0 - 14.0;
                painter.circle_filled(center, TRACKBALL / 2.0, p.bar);
                painter.circle_stroke(center, TRACKBALL / 2.0, Stroke::new(1.0, p.raised));
                if let Some(cam) = self.poser.camera {
                    let parent = rig.parent[bone]
                        .map_or(frame.model, |b| frame.joints[b])
                        .to_scale_rotation_translation()
                        .1;
                    // the joint's own axes for rotation and scale, the
                    // avatar's for position (its values' frame)
                    let axes = match self.poser.transform {
                        1 => frame.model.to_scale_rotation_translation().1,
                        _ => (parent * pose.rotations[bone]).normalize(),
                    };
                    if self.poser.transform == 0 {
                        painter.circle_stroke(center, radius, Stroke::new(1.5, p.muted_dim));
                    }
                    for axis in 0..3 {
                        let dir = axes * Vec3::AXES[axis];
                        let end = center + trackball::project(&cam, dir) * radius;
                        let color = crate::build::draw::fade(
                            crate::build::draw::axis_color(p, axis),
                            if dir.dot(cam.at) > 0.0 { 0.45 } else { 1.0 },
                        );
                        painter.line_segment([center, end], Stroke::new(2.5, color));
                        if self.poser.transform == 2 {
                            painter.rect_filled(
                                egui::Rect::from_center_size(end, egui::Vec2::splat(10.0)),
                                0.0, // theme-guard: content — the scale gizmo's square handles
                                color,
                            );
                        } else if self.poser.transform == 1 {
                            painter.circle_filled(end, 3.5, color);
                        }
                    }
                    painter.circle_filled(center, 4.0, p.ink);
                    if self.poser.transform == 0 && response.dragged() {
                        let d = ui.input(|i| i.pointer.delta()) * self.poser.sensitivity * 0.025;
                        let local = trackball::rotate(&cam, parent, pose.rotations[bone], d);
                        let q = if self.poser.delta {
                            base.rotations[bone].inverse() * local
                        } else {
                            local
                        };
                        let (z, y, x) = q.to_euler(EulerRot::ZYX);
                        value = Vec3::new(x, y, z) * 180.0 / std::f32::consts::PI;
                        edited = true;
                    }
                }
                ui.allocate_ui_with_layout(egui::vec2(TRACKBALL, 30.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_width(TRACKBALL);
                    ui.label(
                        egui::RichText::new(aurora_i18n::text(
                            ["poser-ball-rotation", "poser-ball-position", "poser-ball-scale"][self.poser.transform.min(2)],
                        ))
                        .size(11.0)
                        .color(p.muted_dim),
                    );
                });
            });
            ui.vertical(|ui| {
                ui.add_space(TRACKBALL / 2.0 - 60.0);
                let unit = ["°", " m", " ×"][self.poser.transform.min(2)];
                for axis in 0..3 {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(["X", "Y", "Z"][axis])
                                .strong()
                                .color(crate::build::draw::axis_color(p, axis)),
                        );
                        let range = match self.poser.transform {
                            0 => -180.0..=180.0,
                            1 => -5.0..=5.0,
                            _ => 0.001..=10.0,
                        };
                        ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(60.0);
                        // Display precision must not round the captured pose on idle
                        // frames or add another undo step when a gizmo drag ends.
                        let response = ui.add(
                            egui::Slider::new(&mut value[axis], range)
                                .clamping(egui::SliderClamping::Edits)
                                .suffix(unit)
                                .max_decimals(if self.poser.transform == 0 { 2 } else { 4 }),
                        );
                        edited |= response.changed();
                        if response.secondary_clicked() {
                            value[axis] = match self.poser.transform {
                                1 if !self.poser.delta => base_point.unwrap_or(Vec3::ZERO)[axis],
                                2 => 1.0,
                                _ => 0.0,
                            };
                            edited = true;
                        }
                    });
                }
                ui.add_space(4.0);
                widgets::switch_labeled(ui, p, &mut self.poser.delta, aurora_i18n::tr!("poser-delta"));
            });
        });
        if edited && self.poser.transform == 1 {
            self.position = value
                + if self.poser.delta {
                    base_point.unwrap_or(Vec3::ZERO)
                } else {
                    Vec3::ZERO
                };
            self.commit_poser_manipulation(rig, frame, false);
            self.refresh = true;
        } else if edited && self.poser.transform == 2 {
            self.set_pose_scale(rig, bone, if self.poser.delta { base.scales[bone] * value } else { value });
        } else if edited && let Some(session) = self.poser.sessions.get_mut(&self.poser.subject) {
            if !self.poser.drag_checkpoint {
                session.checkpoint();
                self.poser.drag_checkpoint = true;
            }
            let pose = Arc::make_mut(&mut session.pose);
            match self.poser.transform {
                0 => {
                    let q = Quat::from_euler(EulerRot::ZYX, value.z.to_radians(), value.y.to_radians(), value.x.to_radians());
                    pose.rotations[bone] = if self.poser.delta { base.rotations[bone] * q } else { q };
                }
                _ => pose.scales[bone] = if self.poser.delta { base.scales[bone] * value } else { value },
            }
            pose.locks.remove(&bone);
            if self.symmetry
                && let Some(other) = self.mirrors.counterpart(bone)
            {
                pose.rotations[other] =
                    (rig.local_rot[other] * reflected(rig.local_rot[bone].inverse() * pose.rotations[bone])).normalize();
                pose.scales[other] = base.scales[other] * (pose.scales[bone] / base.scales[bone]);
                pose.locks.remove(&other);
            }
            self.refresh = true;
        }
        ui.add_space(14.0);
        bones::caption(ui, p, aurora_i18n::text("poser-whole-pose"));
        ui.horizontal_wrapped(|ui| {
            for (operation, key) in [
                (Operation::Flip, "poser-flip"),
                (Operation::LeftToRight, "poser-symmetry-lr"),
                (Operation::RightToLeft, "poser-symmetry-rl"),
                (Operation::TPose, "poser-t-pose"),
                (Operation::ResetAll, "ik-edit-reset-all"),
            ] {
                if ui.button(aurora_i18n::text(key)).clicked() {
                    self.pose_operation(rig, frame, operation);
                }
            }
        });
    }
}
/// The trackball's side, its caption under it.
const TRACKBALL: f32 = 200.0;
