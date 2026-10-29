//! Joint-local scale handles: axis cubes and a central uniform-scale cube.
//! Like LLManipScale, handle sizes stay constant on screen; only pose channels change.
use super::*;
use crate::build::{draw::Painter3d, geom::Cam};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Axis(usize),
    Uniform,
}
#[derive(Clone, Copy)]
struct Geometry {
    cam: Cam,
    pivot: Vec3,
    rotation: Quat,
    length: f32,
}
impl Geometry {
    fn new(proj: &Projector, matrix: Mat4) -> Option<Self> {
        let cam = gizmo::camera(proj)?;
        let (_, rotation, pivot) = matrix.to_scale_rotation_translation();
        let depth = cam.depth(pivot);
        (depth > 0.05).then_some(Self {
            cam,
            pivot,
            rotation,
            length: depth * (65.0 * proj.ppp / cam.height * cam.fov_y).tan(),
        })
    }
    fn end(&self, axis: usize) -> Vec3 {
        self.pivot + self.rotation * Vec3::AXES[axis] * self.length
    }
    fn pick(&self, pointer: Pos2) -> Option<Part> {
        let center = self.cam.project(self.pivot)?;
        if center.distance(pointer) <= 9.0 {
            return Some(Part::Uniform);
        }
        (0..3)
            .filter_map(|axis| {
                let end = self.cam.project(self.end(axis))?;
                let distance = end.distance(pointer);
                (center.distance(end) > 16.0 && distance <= 10.0).then_some((axis, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(axis, _)| Part::Axis(axis))
    }
    fn draw(&self, ctx: &egui::Context, palette: &Palette, active: Option<Part>) {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("poser-scale")));
        let mut p = Painter3d::new(painter, &self.cam, *palette);
        for axis in 0..3 {
            let color = if active == Some(Part::Axis(axis)) {
                palette.violet_light
            } else {
                crate::build::draw::axis_color(palette, axis)
            };
            let end = self.end(axis);
            p.line_cased(self.pivot, end, color, 3.0);
            p.cube(end, self.rotation, Vec3::splat(self.length * 0.075), color);
            p.text(self.pivot.lerp(end, 1.3), ["X", "Y", "Z"][axis], 12.0, color);
        }
        p.cube(
            self.pivot,
            self.rotation,
            Vec3::splat(self.length * 0.08),
            if active == Some(Part::Uniform) {
                palette.violet_light
            } else {
                palette.ink
            },
        );
        p.flush();
    }
}
struct Drag {
    geometry: Geometry,
    part: Part,
    origin: Pos2,
    scale: Vec3,
}
impl Drag {
    fn update(&self, pointer: Pos2) -> Option<Vec3> {
        let delta = pointer - self.origin;
        let mut factor = Vec3::ONE;
        match self.part {
            Part::Axis(axis) => {
                let span = self.geometry.cam.project(self.geometry.end(axis))? - self.geometry.cam.project(self.geometry.pivot)?;
                factor[axis] += delta.dot(span) / span.length_sq().max(1.0);
            }
            Part::Uniform => factor = Vec3::splat(1.0 + (delta.x - delta.y) / 100.0),
        }
        // a drag past the pivot shrinks to the floor, never turns the joint over
        Some(data::bounded_scale(self.scale * factor.max(Vec3::splat(1e-6))))
    }
}
#[derive(Default)]
pub(super) struct Gizmo {
    drag: Option<Drag>,
}
impl Gizmo {
    pub(super) fn cancel(&mut self) {
        self.drag = None;
    }
    pub(super) fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    fn show(&mut self, ctx: &egui::Context, palette: &Palette, geometry: Geometry, scale: Vec3, allow_hover: bool) -> (bool, Option<Vec3>) {
        let (pointer, pressed, down, released, alt) = ctx.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.modifiers.alt,
            )
        });
        if alt {
            self.cancel();
        }
        let hover = pointer
            .filter(|p| allow_hover && !alt && ctx.layer_id_at(*p).is_none_or(|l| l.order <= egui::Order::Background))
            .and_then(|p| geometry.pick(p));
        if pressed && let (Some(part), Some(origin)) = (hover, pointer) {
            self.drag = Some(Drag {
                geometry,
                part,
                origin,
                scale,
            });
        }
        let mut change = None;
        let owns = hover.is_some() || self.drag.is_some();
        if let Some(drag) = &self.drag {
            if (down || released)
                && let Some(pointer) = pointer
            {
                change = drag.update(pointer);
            }
            drag.geometry.draw(ctx, palette, Some(drag.part));
            if released || !down {
                self.cancel();
            }
        } else {
            geometry.draw(ctx, palette, hover);
        }
        if owns {
            ctx.set_cursor_icon(if down { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        (owns, change)
    }
}
impl Editor {
    pub(super) fn set_pose_scale(&mut self, rig: &Rig, bone: usize, scale: Vec3) {
        if !scale.is_finite() {
            return;
        }
        self.mirrors.initialize(rig);
        let scale = data::bounded_scale(scale);
        let Some(session) = self
            .poser
            .sessions
            .get_mut(&self.poser.subject)
            .filter(|s| s.enabled && s.pose.scales[bone] != scale)
        else {
            return;
        };
        if !self.poser.drag_checkpoint {
            session.checkpoint();
            self.poser.drag_checkpoint = true;
        }
        let pose = Arc::make_mut(&mut session.pose);
        pose.scales[bone] = scale;
        pose.locks.remove(&bone);
        if self.symmetry
            && let Some(other) = self.mirrors.counterpart(bone)
        {
            pose.scales[other] = data::bounded_scale(session.base.scales[other] * scale / session.base.scales[bone]);
            pose.locks.remove(&other);
        }
    }
    pub(super) fn show_pose_scale(&mut self, ctx: &egui::Context, palette: &Palette, proj: &Projector, frame: &Frame, rig: &Rig) -> bool {
        let Some(bone) = self.selected.filter(|b| self.bone_visible(&rig.canonical_names[*b])) else {
            self.poser.scale_gizmo.cancel();
            return false;
        };
        let Some(session) = self.poser.sessions.get(&self.poser.subject).filter(|s| s.enabled) else {
            self.poser.scale_gizmo.cancel();
            return false;
        };
        let Some(geometry) = Geometry::new(proj, frame.joints[bone]) else {
            self.poser.scale_gizmo.cancel();
            return false;
        };
        let allow_hover = ctx.input(|i| i.pointer.interact_pos()).is_none_or(|p| {
            geometry.cam.project(geometry.pivot).is_some_and(|c| c.distance(p) <= 9.0) || !self.point_owns_pointer(ctx, rig, p)
        });
        let (owns, change) = self
            .poser
            .scale_gizmo
            .show(ctx, palette, geometry, session.pose.scales[bone], allow_hover);
        if let Some(scale) = change {
            self.set_pose_scale(rig, bone, scale);
        }
        owns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ik_edit::poser::tests::{projector, render, selected_fixture},
        ui::widgets::elide_tests::Frames,
    };
    #[test]
    fn axis_and_uniform_drags_scale_only_pose_channels_with_one_undo_step() {
        for part in [Part::Axis(0), Part::Uniform] {
            let (mut world, mut editor) = selected_fixture();
            let bone = editor.selected.unwrap();
            editor.poser.transform = 2;
            let frame = editor.editor_frame(&world).unwrap();
            let proj = projector(frame.joints[bone].w_axis.truncate());
            let geometry = Geometry::new(&proj, frame.joints[bone]).unwrap();
            let (start, end) = match part {
                Part::Axis(axis) => {
                    let start = geometry.cam.project(geometry.end(axis)).unwrap();
                    let center = geometry.cam.project(geometry.pivot).unwrap();
                    (start, start + (start - center) * 0.4)
                }
                Part::Uniform => {
                    let start = geometry.cam.project(geometry.pivot).unwrap();
                    (start, start + egui::vec2(25.0, -15.0))
                }
            };
            let mut frames = Frames::new();
            // the window minimized to its bar: the joint is in the middle of a
            // screen narrower than the window and the drag happens in the world
            crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
            let p = crate::theme::Theme::default().palette();
            let initial = editor.poser.sessions[&world.agent_id].pose.clone();
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            };
            for events in [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![egui::Event::PointerMoved(start), button(start, true)],
                vec![egui::Event::PointerMoved(start.lerp(end, 0.5))],
                vec![egui::Event::PointerMoved(end)],
                vec![button(end, false)],
            ] {
                frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
                render(&mut world, &editor);
            }
            let session = &editor.poser.sessions[&world.agent_id];
            let expected = match part {
                Part::Axis(axis) => {
                    let mut s = initial.scales[bone];
                    s[axis] *= 1.4;
                    s
                }
                Part::Uniform => initial.scales[bone] * 1.4,
            };
            assert!(
                session.pose.scales[bone].abs_diff_eq(expected, 1e-4),
                "{part:?}: {:?} != {expected:?}",
                session.pose.scales[bone]
            );
            assert_eq!(session.pose.positions, initial.positions);
            assert_eq!(session.pose.rotations, initial.rotations);
            assert_eq!(session.undo.len(), 1);
            assert!(!editor.poser.scale_gizmo.dragging());
            editor.poser.sessions.get_mut(&world.agent_id).unwrap().history(false);
            assert_eq!(editor.poser.sessions[&world.agent_id].pose.scales, initial.scales);
        }
    }
}
