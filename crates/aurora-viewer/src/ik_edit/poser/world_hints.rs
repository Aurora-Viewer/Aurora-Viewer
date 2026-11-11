//! What the pointer offers in the world besides the joints and the gizmo,
//! in both poser windows (Aurora's own: the Legacy Viewer's poser changes
//! joint group and subject from its window only):
//! - with Ctrl held over a part of the subject that another joint group
//!   governs (a hand while posing the Body), that part is tinted in the
//!   accent (the face as an oval) with "Ctrl + click: Hands" by the cursor;
//!   the click selects its nearest joint, which opens its group;
//! - with Ctrl held over another avatar posed, or who accepted, their
//!   skeleton shows faintly with "Ctrl + click to pose …"; the click poses
//!   them (`overlay::click_beside`).
//!
//! Physics is never offered: its volumes lie over the body. Of the Other
//! bones, only the wings and the tail are, each its own part, and only when
//! a rigged mesh of the subject uses them (`Scene::rigged_joints`): the
//! hind limbs, the groin and the skull lie over the body.
use super::*;

/// What the pointer is over this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Hint {
    None,
    /// A part another group governs: its nearest joint.
    Group(usize),
    /// Another avatar a click poses.
    Avatar(Uuid),
}

/// How much nearer another group's joint must be than the current group's
/// nearest for its part to be offered: the hind limbs and the folded wings
/// lie near the legs and the back, and must not take them over.
const NEARER: f32 = 0.6;
/// The physics group, never offered.
const PHYSICS: usize = 4;
/// The Other group, and the parts its wings and tail make.
const OTHER: usize = 3;
pub(super) const WINGS: usize = 10;
pub(super) const TAIL: usize = 11;
/// The rest of the Other bones, never offered.
const OTHER_REST: usize = 12;

/// The part a joint belongs to: its group, the Other bones split into the
/// wings, the tail and the rest.
pub(super) fn part_key(name: &str) -> usize {
    match group(name) {
        OTHER if name.contains("Wing") => WINGS,
        OTHER if name.contains("Tail") => TAIL,
        OTHER => OTHER_REST,
        g => g,
    }
}

/// Whether a joint's part may be offered: not physics nor the rest of the
/// Other bones, and the wings and the tail only when a mesh uses them.
pub(super) fn offerable(key: usize, rigged: bool) -> bool {
    match key {
        PHYSICS | OTHER_REST => false,
        WINGS | TAIL => rigged,
        _ => true,
    }
}

/// The joint whose part to offer: the nearest joint to the pointer among
/// the current group's and the offerable ones, when it is not the current
/// group's and is clearly nearer than the current group's nearest.
/// `joints` are (joint, where on screen); `groups` and `offer` say each
/// joint's group and whether it may be offered.
pub(super) fn offered_joint(
    joints: &[(usize, egui::Pos2)],
    groups: &[usize],
    offer: &[bool],
    current: usize,
    pointer: egui::Pos2,
) -> Option<usize> {
    let distance = |at: egui::Pos2| at.distance(pointer);
    let nearest = joints
        .iter()
        .filter(|(j, _)| groups[*j] == current || offer[*j])
        .min_by(|a, b| distance(a.1).total_cmp(&distance(b.1)))?;
    if groups[nearest.0] == current {
        return None;
    }
    let own = joints
        .iter()
        .filter(|(j, _)| groups[*j] == current)
        .map(|(_, at)| distance(*at))
        .fold(f32::INFINITY, f32::min);
    (distance(nearest.1) < own * NEARER).then_some(nearest.0)
}

/// The joints of `start`'s part (`keys`, as `part_key`) that hang from the
/// same joint outside it: one hand's fingers (each from the wrist), the
/// face, the wings. Joined links alone would give a single finger.
pub(super) fn part(parent: &[Option<usize>], keys: &[usize], start: usize) -> Vec<usize> {
    let key = keys[start];
    // the body is one part: the Bento spine joints between the pelvis, the
    // torso and the chest are Other's, and would cut it in three
    if key == 0 {
        return (0..parent.len()).filter(|j| keys[*j] == 0).collect();
    }
    // the first joint up the skeleton outside the part, or the part's top
    let entry = |mut joint: usize| {
        while let Some(up) = parent[joint] {
            if keys[up] != key {
                return Some(up);
            }
            joint = up;
        }
        None
    };
    let from = entry(start);
    (0..parent.len()).filter(|j| keys[*j] == key && entry(*j) == from).collect()
}

/// The joints a part's outline is drawn from: the body without the head,
/// which is the face's to show (its oval), so the body stops at the neck and
/// a Ctrl over the head does not offer it.
pub(super) fn outline_members(key: usize, members: &[usize], names: &[String]) -> Vec<usize> {
    if key == 0 {
        return members.iter().copied().filter(|j| names[*j] != "mHead").collect();
    }
    members.to_vec()
}

/// The part's name for the hint: its group's tab, or Wings / Tail.
fn part_label(key: usize) -> &'static str {
    match key {
        WINGS => aurora_i18n::text("poser-part-wings"),
        TAIL => aurora_i18n::text("poser-part-tail"),
        g => aurora_i18n::text(["poser-body", "poser-face", "poser-hands", "poser-misc", "poser-physics"][g.min(4)]),
    }
}

impl Editor {
    /// Find, paint and announce what the pointer is over (`Hint`). Nothing
    /// while the markers are turned off, over a window, on a joint, shaft or
    /// gizmo, or during a drag.
    pub(super) fn world_hint(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        proj: &Projector,
        frame: &Frame,
        markers_on: bool,
    ) -> Hint {
        let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) else {
            return Hint::None;
        };
        let over_ui = ctx.layer_id_at(pointer).is_some_and(|l| l.order > egui::Order::Background);
        // with Ctrl, it looks under the selected joint's handles (they give
        // way next frame if a part or avatar is there), never under a marker
        let ctrl_held = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
        let under_handles = ctrl_held && self.poser.gizmo_hover && !self.poser.marker_hover;
        if !markers_on
            || !(self.poser.part_hints || self.poser.avatar_hints)
            || over_ui
            || (self.poser.pointer_owned && !under_handles)
            || self.drag.is_some()
            || self.gizmo.dragging()
            || self.poser.scale_gizmo.dragging()
            // none while a button is held, but on the press's own frame: the
            // handles' way (`ctrl_hint`) is decided where the press lands
            || ctx.input(|i| (i.pointer.any_down() && !i.pointer.primary_pressed()) || i.modifiers.alt)
        {
            return Hint::None;
        }
        let rig = &world.avatar_lib.rig;
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("poser-world-hints")));
        let tip = |text: String| {
            egui::Tooltip::always_open(
                ctx.clone(),
                egui::LayerId::background(),
                egui::Id::unique("poser-world-hint-tip"),
                egui::PopupAnchor::Position(pointer + egui::vec2(16.0, 14.0)),
            )
            .show(|ui| {
                ui.label(text);
            });
        };
        // a part of the subject another group governs
        let groups: Vec<usize> = rig.canonical_names.iter().map(|n| group(n)).collect();
        let keys: Vec<usize> = rig.canonical_names.iter().map(|n| part_key(n)).collect();
        let offer: Vec<bool> = (0..rig.len()).map(|j| offerable(keys[j], self.poser.rigged.contains(&j))).collect();
        // the open tab's joints count as its own only when shown: the hidden
        // spine sits exactly on the torso and the chest, and would hold the
        // body back from the Other tab
        let joints: Vec<(usize, egui::Pos2)> = (0..rig.len())
            .filter(|j| groups[*j] != self.poser.group || self.poser_bone_visible(&rig.canonical_names[*j]))
            .filter_map(|j| proj.project(frame.joints[j].w_axis.truncate()).map(|(at, _)| (j, at)))
            .collect();
        // only while Ctrl is held: without it the part neither shows nor
        // takes the click, so a plain click on the face still focuses the
        // photo (a modifier's affordance shows with the modifier, as the
        // Mac's Cmd + hover link)
        let ctrl = ctx.input(|i| i.modifiers.ctrl || i.modifiers.command);
        if let Some(joint) = offered_joint(&joints, &groups, &offer, self.poser.group, pointer).filter(|_| self.poser.part_hints && ctrl) {
            let members = part(&rig.parent, &keys, joint);
            let world_at: Vec<glam::Vec3> = frame.joints.iter().map(|m| m.w_axis.truncate()).collect();
            let project = |at| proj.project(at).map(|(p, _)| p);
            // the part's own shape, see-through in the accent: the face an
            // oval around the head (one smooth outline), the others their
            // bones' union
            let fill = p.violet.gamma_multiply(0.32);
            let stroke = Stroke::new(1.5, p.violet_light.gamma_multiply(0.7));
            let inside = if keys[joint] == 1 {
                let head = frame.joints[rig.names.get("mHead").copied().unwrap_or(rig.pelvis)];
                let oval = silhouette::face(&rig.canonical_names, &world_at, &members, head, project)
                    .filter(|oval| silhouette::depth(&[*oval], pointer) > 0.0);
                if let Some(oval) = oval {
                    painter.add(egui::Shape::convex_polygon(oval.outline(64), fill, stroke));
                }
                oval.is_some()
            } else {
                let outline = outline_members(keys[joint], &members, &rig.canonical_names);
                let capsules = silhouette::capsules(
                    &rig.parent,
                    &rig.canonical_names,
                    &world_at,
                    &outline,
                    silhouette::links_of(keys[joint]),
                    project,
                );
                let inside = silhouette::depth(&capsules, pointer) > 0.0;
                if inside {
                    silhouette::paint(&painter, &silhouette::trace(&capsules), fill, stroke);
                }
                inside
            };
            if inside {
                tip(aurora_i18n::tr!("poser-ctrl-click-group", group = part_label(keys[joint])).to_string());
                return Hint::Group(joint);
            }
        }
        // another avatar to pose, with Ctrl as the parts: a plain click on
        // them stays the world's (a photo's focus, a HUD drawn over them)
        if !self.poser.avatar_hints || !ctrl {
            return Hint::None;
        }
        let others: Vec<(Uuid, egui::Rect, f32, Frame)> = self
            .poseable_others(world)
            .into_iter()
            .filter_map(|id| Frame::subject(world, id).map(|f| (id, f)))
            .filter_map(|(id, f)| {
                // their body, not the wings, tail and hind limbs every
                // skeleton carries at rest beside it, unseen unless a mesh
                // uses them
                let (points, depth): (Vec<egui::Pos2>, Vec<f32>) = f
                    .joints
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| !optional_bone(&rig.canonical_names[*j]))
                    .filter_map(|(_, m)| proj.project(m.w_axis.truncate()))
                    .unzip();
                let near = depth.iter().copied().fold(f32::INFINITY, f32::min);
                compact::body_area(points).map(|area| (id, area, near, f))
            })
            .collect();
        // under the pointer means on their body, not in the box around it,
        // and never behind the subject's own body there: Ctrl over the
        // subject stays the subject's (its handles, Ctrl's rotation) when a
        // partner stands close behind
        let nearest = |f: &Frame| {
            f.joints
                .iter()
                .filter_map(|m| proj.project(m.w_axis.truncate()))
                .map(|(_, d)| d)
                .fold(f32::INFINITY, f32::min)
        };
        let subject_near = (self.body_depth(rig, proj, frame, pointer) > 0.0).then(|| nearest(frame));
        let body: Vec<usize> = (0..rig.len()).filter(|j| !optional_bone(&rig.canonical_names[*j])).collect();
        let areas: Vec<(Uuid, egui::Rect, f32)> = others
            .iter()
            .filter(|(_, _, near, _)| subject_near.is_none_or(|own| *near < own))
            .filter(|(_, _, _, f)| overlay::other_body_depth(rig, proj, f, &body, pointer) > 0.0)
            .map(|(id, area, near, _)| (*id, *area, *near))
            .collect();
        let Some(id) = compact::clicked_subject(&areas, pointer) else {
            return Hint::None;
        };
        if let Some((_, _, _, other)) = others.iter().find(|o| o.0 == id) {
            // their Body skeleton, faint, whatever group is open for the
            // subject: what a click would pose (they open on the pelvis)
            let open = std::mem::replace(&mut self.poser.group, 0);
            let segments = self.skeleton_segments(world, proj, other);
            self.poser.group = open;
            for segment in segments {
                painter.line_segment([segment.a.0, segment.b.0], Stroke::new(2.0, p.teal.gamma_multiply(0.6)));
            }
        }
        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        tip(aurora_i18n::tr!("poser-click-to-pose", name = subjects::subject_short_name(world, id)).to_string());
        Hint::Avatar(id)
    }
}
