//! Limb bend handles, separate from the joint-centred rotation pivot.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Orbit {
    pivot: Vec3,
    point: Vec3,
}

impl Editor {
    pub(super) fn begin_pole_orbit(&mut self, frame: &Frame, rig: &Rig) {
        if self.pole_orbit.is_none()
            && let Some(limb) = limbs(rig).into_iter().find(|l| Some(l.end) == self.selected)
            && let Some(point) = self.pole
        {
            self.pole_orbit = Some(Orbit {
                pivot: frame.model.inverse().transform_point3(frame.joints[limb.middle].w_axis.truncate()),
                point,
            });
        }
    }
    pub(super) fn rotate_pole(&mut self, rotation: Quat) {
        if let Some(orbit) = self.pole_orbit
            && rotation.is_finite()
        {
            self.pole = Some(orbit.pivot + rotation * (orbit.point - orbit.pivot));
        }
    }
    pub(super) fn show_poles(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        proj: &Projector,
        frame: &Frame,
        rig: &Rig,
        gizmo_owns: bool,
    ) {
        if self.clip.waiting_source() || !self.limb_controls || self.picking || self.prims.object_handles || ctx.input(|i| i.modifiers.alt)
        {
            return;
        }
        for limb in limbs(rig) {
            if !self.limb_enabled(limb.end) || !self.visibility.visible(&rig.canonical_names[limb.end]) {
                continue;
            }
            let pole = self
                .goals
                .iter()
                .find(|g| g.bone == limb.end)
                .and_then(|g| g.pole)
                .unwrap_or_else(|| limb.pole(frame));
            let point = frame.model.transform_point3(pole);
            let Some((at, _)) = proj.project(point) else {
                continue;
            };
            let selected = self.selected == Some(limb.end) && self.control == Control::Pole;
            let response = egui::Area::new(egui::Id::unique(("ik-pole", limb.end)))
                .fixed_pos(at - egui::vec2(14.0, 14.0))
                .order(egui::Order::Background)
                .movable(false)
                .interactable(!gizmo_owns)
                .show(ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(28.0, 28.0),
                        if gizmo_owns { Sense::hover() } else { Sense::click_and_drag() },
                    );
                    let p = ui.painter();
                    let center = rect.center();
                    if let Some((middle, _)) = proj.project(frame.joints[limb.middle].w_axis.truncate()) {
                        p.line_segment([middle, center], Stroke::new(4.0, palette.navy));
                        p.line_segment([middle, center], Stroke::new(1.8, palette.violet_light));
                        p.circle_stroke(middle, 5.0, Stroke::new(2.0, palette.violet_light));
                    }
                    let corners = [
                        center + egui::vec2(0.0, -12.0),
                        center + egui::vec2(12.0, 0.0),
                        center + egui::vec2(0.0, 12.0),
                        center + egui::vec2(-12.0, 0.0),
                    ];
                    p.add(egui::Shape::convex_polygon(
                        corners.to_vec(),
                        if selected || response.hovered() {
                            palette.violet
                        } else {
                            palette.navy
                        },
                        Stroke::new(3.0, palette.violet_light),
                    ));
                    p.circle_filled(center, 3.0, if selected { palette.on_accent } else { palette.violet_light });
                    let label = p.layout_no_wrap(
                        aurora_i18n::text(limb.pole_label).to_owned(),
                        egui::FontId::proportional(12.0),
                        palette.violet_light,
                    );
                    let label_rect = egui::Rect::from_min_size(center + egui::vec2(17.0, -label.size().y * 0.5), label.size());
                    p.rect_filled(label_rect.expand2(egui::vec2(3.0, 1.0)), 3.0, palette.navy);
                    p.galley(label_rect.min, label, palette.violet_light);
                    response
                })
                .inner;
            if response.clicked() || response.drag_started() {
                if !selected {
                    self.select(limb.end, rig, frame);
                }
                self.control = Control::Pole;
                self.mode = Mode::Ik;
                if !ik::supports_pole(rig, limb.end, self.root) {
                    self.root = ik::extended_root(rig, limb.end).unwrap_or(limb.root);
                }
                self.tool = Tool::Move;
                self.pole = Some(pole);
                self.commit();
            }
            if self.point_drag_enabled(ctx.input(|i| i.modifiers))
                && response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let rotate = ctx.input(|i| i.modifiers.ctrl);
                let changed = self.drag.is_some_and(|d| matches!(d, Drag::Rotate { .. }) != rotate);
                if self.drag.is_none() || changed {
                    self.pole_orbit = None;
                    if rotate {
                        self.begin_pole_orbit(frame, rig);
                    }
                    let origin = if response.drag_started() {
                        ctx.input(|i| i.pointer.press_origin()).unwrap_or(pointer)
                    } else {
                        pointer
                    };
                    self.start_drag(proj, frame, origin, rotate);
                }
                self.drag_to(proj, frame, rig, pointer);
            }
            if response.drag_stopped() {
                self.drag = None;
            }
            response.on_hover_text(aurora_i18n::tr!("ik-edit-pole-hint"));
        }
    }
}

#[cfg(test)]
mod tests;
