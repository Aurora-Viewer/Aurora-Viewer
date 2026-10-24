//! Narrow screen-space bone picking. A real shaft belongs to its origin
//! joint, not its child; virtual parent connectors belong to the virtual node.
use super::*;
use crate::{build::geom::seg_distance, world::rig_graph::Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Source {
    Bone(usize),
    Node(Uuid),
}
pub(super) struct Segment {
    pub source: Source,
    pub a: (Pos2, f32),
    pub b: (Pos2, f32),
}
pub(super) fn pick(segments: &[Segment], pointer: Pos2) -> Option<&Segment> {
    segments
        .iter()
        .filter_map(|s| {
            let (distance, t) = seg_distance(pointer, s.a.0, s.b.0);
            let depth = ((1.0 - t) / s.a.1 + t / s.b.1).recip();
            (distance <= 6.0 && s.a.0.distance_sq(s.b.0) > 1.0).then_some((s, distance, depth))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.2.total_cmp(&b.2)))
        .map(|v| v.0)
}

impl Editor {
    pub(super) fn bone_visible(&self, name: &str) -> bool {
        if self.poser.active {
            poser::group(name) == self.poser.group
        } else {
            self.visibility.visible(name)
        }
    }
    pub(super) fn skeleton_segments(&self, world: &World, proj: &Projector, frame: &Frame) -> Vec<Segment> {
        let rig = &world.avatar_lib.rig;
        let mut segments = Vec::new();
        let mut children = vec![false; rig.len()];
        for (joint, _) in rig.canonical_names.iter().enumerate().filter(|(_, n)| self.bone_visible(n)) {
            let parent = if self.poser.active {
                rig.parent[joint].filter(|b| self.bone_visible(&rig.canonical_names[*b]))
            } else {
                self.visibility.parent(rig, joint)
            };
            if let Some(parent) = parent {
                children[parent] = true;
                if let (Some(a), Some(b)) = (
                    proj.project(frame.joints[parent].w_axis.truncate()),
                    proj.project(frame.joints[joint].w_axis.truncate()),
                ) {
                    segments.push(Segment {
                        source: Source::Bone(parent),
                        a,
                        b,
                    });
                }
            }
        }
        // Leaves also have a shaft: use the canonical local end marker, scaled
        // and rotated by the displayed joint (fingers, toes, tail tips, etc.).
        if let Some(skeleton) = &world.avatar_lib.skeleton {
            for (bone, _) in rig
                .canonical_names
                .iter()
                .enumerate()
                .filter(|(j, n)| !children[*j] && self.bone_visible(n))
            {
                let Some(joint) = skeleton.joints.get(bone) else { continue };
                if joint.end.is_finite()
                    && joint.end.length_squared() > 1e-8
                    && let (Some(a), Some(b)) = (
                        proj.project(frame.joints[bone].w_axis.truncate()),
                        proj.project(frame.joints[bone].transform_point3(joint.end)),
                    )
                {
                    segments.push(Segment {
                        source: Source::Bone(bone),
                        a,
                        b,
                    });
                }
            }
        }
        if !self.poser.active && !self.clip.active {
            segments.extend(self.virtual_segments(proj, frame));
        }
        segments
    }
    pub(super) fn draw_segments(&self, painter: &egui::Painter, palette: &Palette, segments: &[Segment]) {
        for s in segments {
            let selected = match s.source {
                Source::Bone(b) => {
                    (self.poser.active || self.graph.selected.is_none()) && self.selected == Some(b) && self.control == Control::Bone
                }
                Source::Node(id) => self.graph.selected == Some(id),
            };
            let color = if selected {
                palette.teal
            } else if matches!(s.source, Source::Node(_)) {
                palette.teal.gamma_multiply(0.6)
            } else {
                palette.indigo_light.gamma_multiply(0.8)
            };
            painter.line_segment([s.a.0, s.b.0], Stroke::new(if selected { 2.4 } else { 1.4 }, color));
        }
    }
    pub(super) fn point_owns_pointer(&self, ctx: &egui::Context, rig: &Rig, pointer: Pos2) -> bool {
        if self.poser.active {
            return ctx.memory(|m| {
                (0..rig.len()).filter(|b| self.bone_visible(&rig.canonical_names[*b])).any(|b| {
                    m.area_rect(egui::Id::unique(("poser-joint", b)))
                        .is_some_and(|r| r.contains(pointer))
                })
            });
        }
        ctx.memory(|m| {
            rig.canonical_names
                .iter()
                .enumerate()
                .filter(|(_, n)| self.visibility.visible(n))
                .any(|(b, _)| m.area_rect(egui::Id::unique(("ik-joint", b))).is_some_and(|r| r.contains(pointer)))
                || (self.limb_controls
                    && limbs(rig)
                        .iter()
                        .filter(|l| self.limb_enabled(l.end) && self.visibility.visible(&rig.canonical_names[l.end]))
                        .any(|l| {
                            m.area_rect(egui::Id::unique(("ik-pole", l.end)))
                                .is_some_and(|r| r.contains(pointer))
                        }))
                || (!self.clip.active
                    && self.graph.definition.nodes.iter().any(|n| {
                        m.area_rect(egui::Id::unique(("ik-virtual", n.id)))
                            .is_some_and(|r| r.contains(pointer))
                    }))
        })
    }
    pub(super) fn pick_segment(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        world: &World,
        frame: &Frame,
        segments: &[Segment],
        gizmo_owns: bool,
    ) {
        if gizmo_owns
            || self.drag.is_some()
            || self.gizmo.dragging()
            || self.clip.waiting_source()
            || (self.picking && !self.links.bone_pick)
            || (self.prims.object_handles && !self.links.bone_pick)
            || ctx.input(|i| i.modifiers.alt)
        {
            return;
        }
        let Some(pointer) = ctx.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        if ctx.layer_id_at(pointer).is_some_and(|l| l.order > egui::Order::Background) {
            return;
        }
        let rig = &world.avatar_lib.rig;
        // Existing point, effector, bend and virtual controls keep their exact
        // hit rectangles. Never put a segment widget across those controls.
        if self.point_owns_pointer(ctx, rig, pointer) {
            return;
        }
        let Some(s) = pick(segments, pointer) else { return };
        let name = match s.source {
            Source::Bone(b) => rig.canonical_names[b].clone(),
            Source::Node(id) => self
                .graph
                .definition
                .nodes
                .iter()
                .find(|n| n.id == id)
                .map_or_else(String::new, |n| n.name.clone()),
        };
        let (_, t) = seg_distance(pointer, s.a.0, s.b.0);
        let at = s.a.0.lerp(s.b.0, t);
        let response = egui::Area::new(egui::Id::unique(("ik-segment", s.source)))
            .order(egui::Order::Background)
            .fixed_pos(at - egui::vec2(6.0, 6.0))
            .movable(false)
            .show(ctx, |ui| ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::click()).1)
            .inner;
        if response.hovered() {
            ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("ik-skeleton")))
                .line_segment([s.a.0, s.b.0], Stroke::new(3.0, palette.violet_light));
        }
        if response.clicked() {
            if self.links.bone_pick {
                let target = match s.source {
                    Source::Bone(b) => Target::Bone {
                        avatar: Uuid::nil(),
                        bone: rig.canonical_names[b].clone(),
                    },
                    Source::Node(id) => Target::Node { id },
                };
                self.picked_bone(world, frame, target);
            } else {
                match s.source {
                    Source::Bone(b) => self.select(b, rig, frame),
                    Source::Node(id) => self.select_virtual_node(id),
                }
            }
        }
        response.on_hover_text(name);
    }
}

#[cfg(test)]
mod tests;
