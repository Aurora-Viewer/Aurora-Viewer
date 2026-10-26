//! Real-joint selection and manipulation. No project, binding or virtual-node pass.
use super::*;
use crate::ik_edit::segments::{self, Source};

impl Editor {
    pub(in crate::ik_edit) fn select_pose_bone(&mut self, bone: usize, rig: &Rig, frame: &Frame) {
        self.mirrors.initialize(rig);
        self.selected = Some(bone);
        self.control = Control::Bone;
        self.mode = Mode::Direct;
        self.root = bone;
        self.contact = Vec3::ZERO;
        self.pole = None;
        self.pole_orbit = None;
        self.drag = None;
        self.gizmo.cancel();
        self.poser.scale_gizmo.cancel();
        self.tool = if self.poser.transform == 0 { Tool::Rotate } else { Tool::Move };
        self.poser.group = group(&rig.canonical_names[bone]);
        self.sync_pose_bone(bone, frame);
    }
    pub(super) fn select_pose_group(&mut self, group: usize, rig: &Rig, frame: &Frame) {
        self.poser.group = group;
        if let Some(bone) = self
            .selected
            .filter(|b| self.bone_visible(&rig.canonical_names[*b]))
            .or_else(|| rig.canonical_names.iter().position(|name| self.bone_visible(name)))
        {
            self.select_pose_bone(bone, rig, frame);
        } else {
            self.selected = None;
            self.drag = None;
            self.gizmo.cancel();
            self.poser.scale_gizmo.cancel();
        }
    }
    fn sync_pose_bone(&mut self, bone: usize, frame: &Frame) {
        let (_, rotation, position) = (frame.model.inverse() * frame.joints[bone]).to_scale_rotation_translation();
        self.position = position;
        self.set_rotation(rotation);
    }
    pub(in crate::ik_edit) fn show_poser(&mut self, ctx: &egui::Context, p: &Palette, world: &World, proj: &Projector) {
        self.poser.pointer_owned = false;
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.drag = None;
            if !ctx.input(|i| i.pointer.primary_released()) {
                self.poser.drag_checkpoint = false;
            }
        }
        let Some(frame) = self.editor_frame(world) else { return };
        let rig = &world.avatar_lib.rig;
        self.poser.camera = gizmo::camera(proj);
        if self.selected.is_none() || self.refresh {
            self.select_pose_bone(self.selected.unwrap_or(rig.pelvis), rig, &frame);
            self.refresh = false;
        } else if self.drag.is_none() && !self.gizmo.dragging() {
            self.sync_pose_bone(self.selected.unwrap_or(rig.pelvis), &frame);
        }
        // Process tabs first: hidden groups must lose their handles this frame.
        self.poser_ui(ctx, p, world, &frame);
        if self.poser.active && self.poser.markers {
            self.pose_overlay(ctx, p, world, proj, &frame);
        } else {
            self.gizmo.cancel();
            self.poser.scale_gizmo.cancel();
        }
    }
    fn pose_overlay(&mut self, ctx: &egui::Context, p: &Palette, world: &World, proj: &Projector, frame: &Frame) {
        let rig = &world.avatar_lib.rig;
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("poser-skeleton"));
        let painter = ctx.layer_painter(layer);
        let segments = self.skeleton_segments(world, proj, frame);
        self.draw_segments(&painter, p, &segments);
        let gizmo_owns = if self.poser.transform == 2 {
            self.gizmo.cancel();
            self.show_pose_scale(ctx, p, proj, frame, rig)
        } else {
            self.poser.scale_gizmo.cancel();
            self.show_gizmo(ctx, p, proj, frame, rig)
        };
        if ctx.input(|i| i.modifiers.alt) {
            return;
        }
        // Paint the selected joint last so it wins over coincident markers.
        let chosen = self.selected;
        for joint in (0..rig.len()).filter(|b| chosen != Some(*b)).chain(chosen) {
            let name = &rig.canonical_names[joint];
            if !self.bone_visible(name) {
                continue;
            }
            let Some((at, _)) = proj.project(frame.joints[joint].w_axis.truncate()) else {
                continue;
            };
            let selected = chosen == Some(joint);
            let response = egui::Area::new(egui::Id::unique(("poser-joint", joint)))
                .fixed_pos(at - egui::vec2(6.0, 6.0))
                .order(egui::Order::Background)
                .movable(false)
                .interactable(!gizmo_owns)
                .show(ctx, |ui| {
                    let sense = if gizmo_owns {
                        Sense::hover()
                    } else if self.poser.transform == 2 {
                        Sense::click()
                    } else {
                        Sense::click_and_drag()
                    };
                    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(12.0), sense);
                    if selected {
                        ui.painter().circle_stroke(rect.center(), 7.0, Stroke::new(2.0, p.teal));
                    }
                    ui.painter().circle_filled(
                        rect.center(),
                        if selected { 4.5 } else { 2.5 },
                        if self.poser_edited(joint) {
                            p.violet_light
                        } else if selected {
                            p.teal
                        } else {
                            p.ink
                        },
                    );
                    response
                })
                .inner;
            self.poser.pointer_owned |= response.contains_pointer();
            if response.clicked() || response.drag_started() {
                self.select_pose_bone(joint, rig, frame);
            }
            if response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let rotate = self.active_tool(ctx.input(|i| i.modifiers)) == Tool::Rotate;
                if self.drag.is_none() || self.drag.is_some_and(|d| matches!(d, Drag::Rotate { .. }) != rotate) {
                    let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
                    self.start_drag(proj, frame, origin, rotate);
                }
                self.drag_to(proj, frame, rig, pointer);
            }
            response.on_hover_text(name);
        }
        if gizmo_owns || self.drag.is_some() || self.gizmo.dragging() || self.poser.scale_gizmo.dragging() {
            self.poser.pointer_owned = true;
            return;
        }
        let Some(pointer) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        if ctx.layer_id_at(pointer).is_some_and(|l| l.order > egui::Order::Background) || self.point_owns_pointer(ctx, rig, pointer) {
            return;
        }
        let Some(segment) = segments::pick(&segments, pointer) else {
            return;
        };
        let Source::Bone(bone) = segment.source else { return };
        // From the geometry: the shaft's area only exists once hovered.
        self.poser.pointer_owned = true;
        let (_, t) = crate::build::geom::seg_distance(pointer, segment.a.0, segment.b.0);
        let at = segment.a.0.lerp(segment.b.0, t);
        let response = egui::Area::new(egui::Id::unique(("poser-segment", bone)))
            .order(egui::Order::Background)
            .fixed_pos(at - egui::vec2(6.0, 6.0))
            .movable(false)
            .show(ctx, |ui| ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::click()).1)
            .inner;
        if response.hovered() {
            painter.line_segment([segment.a.0, segment.b.0], Stroke::new(3.0, p.violet_light));
        }
        if response.clicked() {
            self.select_pose_bone(bone, rig, frame);
        }
        response.on_hover_text(&rig.canonical_names[bone]);
    }
}
