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
    pub(super) fn hand_preset(&mut self, rig: &Rig, left: bool) {
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
            if name.ends_with(if left { "Left" } else { "Right" })
                && let Some(&bone) = rig.names.get(name)
            {
                pose.rotations[bone] = Quat::from_euler(EulerRot::ZYX, xyz[2], xyz[1], xyz[0]);
                pose.locks.remove(&bone);
            }
        }
        self.refresh = true;
    }
    pub(super) fn joint_controls(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        let Some(bone) = self.selected else { return };
        ui.strong(bones::label(&rig.canonical_names[bone]));
        ui.colored_label(p.muted, &rig.canonical_names[bone]);
        let changed = widgets::segmented(
            ui,
            p,
            &mut self.poser.transform,
            &[
                (aurora_i18n::tr!("poser-mode-rotation"), false),
                (aurora_i18n::tr!("poser-mode-position"), false),
                (aurora_i18n::tr!("poser-mode-scale"), false),
            ],
        );
        if changed {
            self.tool = if self.poser.transform == 0 { Tool::Rotate } else { Tool::Move };
            self.gizmo.cancel();
            self.poser.scale_gizmo.cancel();
        }
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
        if self.poser.transform == 0 {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(128.0, 128.0), Sense::drag());
            let painter = ui.painter();
            let center = rect.center();
            let radius = 52.0;
            painter.circle_stroke(center, radius, Stroke::new(1.5, p.muted_dim));
            if let Some(cam) = self.poser.camera {
                let parent = rig.parent[bone]
                    .map_or(frame.model, |b| frame.joints[b])
                    .to_scale_rotation_translation()
                    .1;
                let world = (parent * pose.rotations[bone]).normalize();
                for axis in 0..3 {
                    let dir = world * Vec3::AXES[axis];
                    let end = center + trackball::project(&cam, dir) * radius;
                    let color = crate::build::draw::fade(
                        crate::build::draw::axis_color(p, axis),
                        if dir.dot(cam.at) > 0.0 { 0.45 } else { 1.0 },
                    );
                    painter.line_segment([center, end], Stroke::new(2.0, color));
                }
                if response.dragged() {
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
        }
        for axis in 0..3 {
            ui.horizontal(|ui| {
                ui.colored_label(crate::build::draw::axis_color(p, axis), ["X", "Y", "Z"][axis]);
                let range = match self.poser.transform {
                    0 => -180.0..=180.0,
                    1 => -5.0..=5.0,
                    _ => 0.001..=10.0,
                };
                // Display precision must not round the captured pose on idle
                // frames or add another undo step when a gizmo drag ends.
                let response = ui.add(
                    egui::Slider::new(&mut value[axis], range)
                        .clamping(egui::SliderClamping::Edits)
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
        widgets::checkbox(ui, p, &mut self.poser.delta, aurora_i18n::tr!("poser-delta"));
        ui.horizontal_wrapped(|ui| {
            for (operation, key) in [
                (Operation::Mirror, "poser-mirror"),
                (Operation::Flip, "poser-flip"),
                (Operation::LeftToRight, "poser-symmetry-lr"),
                (Operation::RightToLeft, "poser-symmetry-rl"),
                (Operation::WorldLock, "poser-world-lock"),
                (Operation::ResetJoint, "ik-edit-reset-bone"),
            ] {
                if ui.button(aurora_i18n::text(key)).clicked() {
                    self.pose_operation(rig, frame, operation);
                }
            }
        });
        self.manipulation_options_ui(ui, p);
    }
}
