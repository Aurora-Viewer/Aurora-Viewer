//! Real-joint selection and manipulation. No project, binding or virtual-node pass.
use super::*;
use crate::ik_edit::segments::{self, Source};

impl Editor {
    pub(in crate::ik_edit) fn select_pose_bone(&mut self, bone: usize, rig: &Rig, frame: &Frame) {
        self.mirrors.initialize(rig);
        self.selected = Some(bone);
        self.poser.unselected = false;
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
            // an empty tab (Other with nothing worn) stays open: the pelvis
            // is not picked again, which would go back to Body
            self.selected = None;
            self.poser.unselected = true;
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
        self.poser.gizmo_hover = false;
        self.poser.marker_hover = false;
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.drag = None;
            if !ctx.input(|i| i.pointer.primary_released()) {
                self.poser.drag_checkpoint = false;
            }
        }
        let Some(frame) = self.editor_frame(world) else { return };
        let rig = &world.avatar_lib.rig;
        self.refresh_other_bones(rig);
        self.poser.camera = gizmo::camera(proj);
        if (self.selected.is_none() && !self.poser.unselected) || self.refresh {
            if let Some(bone) = self.selected.or((!self.poser.unselected).then_some(rig.pelvis)) {
                self.select_pose_bone(bone, rig, &frame);
            }
            self.refresh = false;
        } else if let Some(bone) = self.selected
            && self.drag.is_none()
            && !self.gizmo.dragging()
        {
            self.sync_pose_bone(bone, &frame);
        }
        // Process tabs first: hidden groups must lose their handles this frame.
        self.poser_ui(ctx, p, world, &frame);
        if self.poser.active && self.poser.markers {
            let shown = self.markers_reveal(ctx, world, proj, &frame);
            let fade = ctx.animate_bool_with_time(egui::Id::unique("poser-markers-fade"), shown, 0.15);
            self.pose_overlay(ctx, p, world, proj, &frame, fade);
            // markers on, shown or not: in "On hover", pointing at another
            // avatar hides the subject's own markers, and its preview must
            // stay for the click that poses it
            let hint = self.world_hint(ctx, p, world, proj, &frame, self.poser.markers);
            // the handles give way to it next frame (`poser_ctrl_picks`)
            self.poser.ctrl_hint = hint != world_hints::Hint::None;
            // markers on, even hidden "On hover" away from the avatar: a
            // click on the ground still unselects
            self.click_beside(ctx, world, hint, self.poser.markers);
            // a hint's part or avatar takes the next press, not a HUD's
            self.poser.pointer_owned |= hint != world_hints::Hint::None;
        } else {
            self.gizmo.cancel();
            self.poser.scale_gizmo.cancel();
        }
    }
    /// A click in the world that no joint, shaft or gizmo took
    /// (`world_hints`): Ctrl + click on a part another group governs selects
    /// its joint and opens that group; Ctrl + click on another avatar posed,
    /// or who accepted, poses that one at once; anywhere else, unless a worn
    /// HUD took the click, it unselects the joint. Aurora's own: the Legacy
    /// Viewer's poser keeps its joint until another is chosen in its list.
    fn click_beside(&mut self, ctx: &egui::Context, world: &World, hint: world_hints::Hint, markers_on: bool) {
        let (clicked, ctrl) = ctx.input(|i| {
            (
                i.pointer.primary_clicked() && !i.modifiers.alt,
                i.modifiers.ctrl || i.modifiers.command,
            )
        });
        let Some(pointer) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let over_ui = ctx.layer_id_at(pointer).is_some_and(|l| l.order > egui::Order::Background);
        // a click a worn HUD took (`press_on_hud`, read once) is the HUD's
        let on_hud = clicked && std::mem::take(&mut self.press_on_hud);
        if !clicked || on_hud || over_ui || self.poser.pointer_owned || self.drag.is_some() || self.gizmo.dragging() {
            return;
        }
        let rig = &world.avatar_lib.rig;
        match hint {
            world_hints::Hint::Group(joint) if ctrl => {
                if let Some(frame) = self.editor_frame(world) {
                    self.select_pose_bone(joint, rig, &frame);
                }
            }
            world_hints::Hint::Avatar(id) if ctrl => self.pick_subject(world, id),
            _ if markers_on && self.selected.is_some() => {
                self.selected = None;
                self.poser.unselected = true;
                self.gizmo.cancel();
                self.poser.scale_gizmo.cancel();
            }
            _ => {}
        }
    }
    /// Whether the markers show (`compact::markers_shown`): in the "On
    /// hover" mode, the pointer over the subject's silhouette, with a wider
    /// margin to leave than to enter and a short linger once it left, so
    /// that they neither flicker at the edge nor drop between two joints.
    fn markers_reveal(&mut self, ctx: &egui::Context, world: &World, proj: &Projector, frame: &Frame) -> bool {
        if !self.poser.markers_hover {
            return true;
        }
        let rig = &world.avatar_lib.rig;
        let held = self.drag.is_some() || self.gizmo.dragging() || self.poser.scale_gizmo.dragging();
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        // a window over the avatar hides them; a tooltip (a joint's name,
        // a hint) is not a window
        let over_ui = pointer.is_some_and(|at| {
            ctx.layer_id_at(at)
                .is_some_and(|l| l.order > egui::Order::Background && l.order != egui::Order::Tooltip)
        });
        let depth = pointer.map(|at| {
            // the body, and the wings, tail and hind limbs only when used (a
            // mesh, or the pose moved them): unused, they rest beside it in
            // empty space
            let body: Vec<usize> = (0..rig.len())
                .filter(|j| {
                    let name = &rig.canonical_names[*j];
                    group(name) != 4 && (!optional_bone(name) || self.poser.other_used.contains(name))
                })
                .collect();
            let world_at: Vec<glam::Vec3> = frame.joints.iter().map(|m| m.w_axis.truncate()).collect();
            let capsules = silhouette::capsules(
                &rig.parent,
                &rig.canonical_names,
                &world_at,
                &body,
                silhouette::Links::Inside,
                |w| proj.project(w).map(|(p, _)| p),
            );
            silhouette::depth(&capsules, at)
        });
        let near = compact::markers_shown(true, depth, self.poser.markers_near, over_ui, held);
        let now = ctx.input(|i| i.time);
        if near {
            self.poser.markers_seen_at = now;
        }
        self.poser.markers_near = near;
        near || now - self.poser.markers_seen_at < compact::REVEAL_LINGER
    }
    /// The skeleton, the joints and the gizmo; `fade` is the markers'
    /// opacity (0 hides them, the selected joint's gizmo stays).
    fn pose_overlay(&mut self, ctx: &egui::Context, p: &Palette, world: &World, proj: &Projector, frame: &Frame, fade: f32) {
        let rig = &world.avatar_lib.rig;
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("poser-skeleton"));
        let mut painter = ctx.layer_painter(layer);
        let segments = self.skeleton_segments(world, proj, frame);
        if fade > 0.0 {
            painter.multiply_opacity(fade);
            self.draw_segments(&painter, p, &segments);
        }
        let gizmo_owns = if !self.poser.gizmo {
            // hidden: the sliders and the markers' drags still pose
            self.gizmo.cancel();
            self.poser.scale_gizmo.cancel();
            false
        } else if self.poser.transform == 2 {
            self.gizmo.cancel();
            self.show_pose_scale(ctx, p, proj, frame, rig)
        } else {
            self.poser.scale_gizmo.cancel();
            self.show_gizmo(ctx, p, proj, frame, rig)
        };
        self.poser.gizmo_hover = gizmo_owns;
        if ctx.input(|i| i.modifiers.alt) {
            return;
        }
        if fade <= 0.0 {
            self.poser.pointer_owned |= gizmo_owns || self.drag.is_some() || self.gizmo.dragging() || self.poser.scale_gizmo.dragging();
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
                    ui.multiply_opacity(fade);
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
            self.poser.marker_hover |= response.contains_pointer();
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
        self.poser.marker_hover = true;
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
