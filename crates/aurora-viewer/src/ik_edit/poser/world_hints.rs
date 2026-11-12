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

/// A joint that is a part of its own: a bone of a group, or a wing, the
/// tail or a hind limb. The others (collision volumes and attachment points,
/// named without the "m", and the spine, groin and skull inside the body)
/// follow the part of the bone they hang from.
fn own_part(name: &str) -> bool {
    name.starts_with('m') && !(group(name) == OTHER && !optional_bone(name))
}

/// The bone whose part `joint` is: itself when it is a part's own, else the
/// nearest such bone up the skeleton (`own_part`).
fn part_owner(parent: &[Option<usize>], names: &[String], mut joint: usize) -> Option<usize> {
    loop {
        if own_part(&names[joint]) {
            return Some(joint);
        }
        joint = parent[joint]?;
    }
}

/// A body bone a part takes for its own on the meshes: the head the
/// face's, a wrist its hand's. A Bento face shares its weights with mHead
/// and a hand's palm is the wrist's: without them a face is patchy and
/// fingers are a thin target. They stay Body bones to pose.
fn lends_to_part(name: &str) -> bool {
    name == "mHead" || name.starts_with("mWrist")
}

/// The GPU's part class for a canonical joint. Collision volumes and
/// inner bones follow their owner; the face borrows the head, each hand
/// its wrist. Zero still occludes, but is not an offered part.
fn pick_class(joint: usize, parent: &[Option<usize>], names: &[String]) -> u32 {
    let Some(owner) = part_owner(parent, names, joint) else { return 0 };
    let name = &names[owner];
    match part_key(name) {
        0 if name == "mHead" => 2,
        0 if name.starts_with("mWrist") => {
            if name.ends_with("Left") {
                3
            } else {
                4
            }
        }
        0 => 1,
        1 => 2,
        2 => {
            if name.ends_with("Left") {
                3
            } else {
                4
            }
        }
        WINGS => 5,
        TAIL => 6,
        _ => 0,
    }
}

/// Four bits per palette joint, matching `poser_pick_class` in object.wgsl.
/// Interpolating each class's summed weights lets the fragment choose the
/// visible part independently of the triangle's vertex/index order.
pub(super) fn pick_classes(parent: &[Option<usize>], names: &[String]) -> [u32; 32] {
    let mut classes = [0u32; 32];
    for j in 0..names.len().min(256) {
        classes[j >> 3] |= pick_class(j, parent, names) << ((j & 7) * 4);
    }
    // The system body's identity slot belongs to the pelvis/body.
    let root = names.len();
    if root < 256 {
        classes[root >> 3] |= 1 << ((root & 7) * 4);
    }
    classes
}

/// A mesh-picked part offers its nearest projected, offerable bone. The
/// current tab is never offered, even though it lends bones to another
/// part's surface (the head and wrists).
pub(super) fn picked_part(
    class: u32,
    joints: &[(usize, egui::Pos2)],
    parent: &[Option<usize>],
    names: &[String],
    offer: &[bool],
    current: usize,
    pointer: egui::Pos2,
) -> Option<usize> {
    let tab = match class {
        1 => 0,
        2 => 1,
        3 | 4 => 2,
        5 | 6 => OTHER,
        _ => return None,
    };
    if tab == current {
        return None;
    }
    joints
        .iter()
        .filter(|(j, _)| offer[*j] && group(&names[*j]) == tab && pick_class(*j, parent, names) == class)
        .min_by(|a, b| a.1.distance_sq(pointer).total_cmp(&b.1.distance_sq(pointer)))
        .map(|(j, _)| *j)
}

/// How far, in window pixels, the cursor may be from where the mesh pick
/// was read for its answer to stand: it comes a frame or two late.
const PICK_REACH: f32 = 4.0;

/// The mesh pick's answer that stands for the cursor at `pointer` (window
/// pixels): the one asked for this `subject` and read within `PICK_REACH`
/// of it. Else None, and the bones decide at once: a flick to another
/// avatar, the subject switched, a pixel no answer came for.
pub(super) fn fresh_pick(stored: Option<(Uuid, u32, [f32; 2])>, subject: Uuid, pointer: egui::Pos2) -> Option<u32> {
    let (asked, joint, [x, y]) = stored?;
    (asked == subject && pointer.distance(egui::pos2(x, y)) <= PICK_REACH).then_some(joint)
}

/// The renderer's mask of `members` (`FrameParams::part_joints`): bit j for
/// each palette joint whose part they are, the joints that follow a bone
/// included. A fitted mesh body weights its vertices to the collision
/// volumes (BELLY, PEC…) and a Bento body to the spine: they tint with the
/// body. Joints past 256 have no bit (the skeleton has 206).
pub(super) fn mask_joints(parent: &[Option<usize>], names: &[String], members: &[usize]) -> [u32; 8] {
    let mut mask = [0u32; 8];
    for j in 0..names.len().min(256) {
        if part_owner(parent, names, j).is_some_and(|o| members.contains(&o)) {
            mask[j >> 5] |= 1 << (j & 31);
        }
    }
    mask
}

/// The joints the renderer keeps for a part on the subject's meshes (the
/// tint fades where the weights pass from them to others):
/// - the body without the bones it lends (`lends_to_part`: the head, the
///   wrists) and what follows them, so it meets the face and the hands in a
///   fade at the neck and the wrists instead of lighting them;
/// - the face and the hands with the bone they hang from (the head, the
///   wrist): the whole head, the whole hand;
/// - another part, its own joints when a worn mesh weights them (rigged
///   wings, a tail), else also the bone it hangs from.
pub(super) fn mask_members(
    key: usize,
    members: &[usize],
    parent: &[Option<usize>],
    keys: &[usize],
    names: &[String],
    rigged: &std::collections::HashSet<usize>,
) -> Vec<usize> {
    if key == 0 {
        return members.iter().copied().filter(|j| !lends_to_part(&names[*j])).collect();
    }
    let mut joints = members.to_vec();
    if key != 1 && key != 2 && members.iter().any(|j| rigged.contains(j)) {
        return joints;
    }
    for &m in members {
        if let Some(up) = parent[m].filter(|up| !members.contains(up) && keys[*up] != key)
            && !joints.contains(&up)
        {
            joints.push(up);
        }
    }
    joints
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
        // an avatar's part is drawn by the renderer from its meshes, and
        // picked there (part_pick.rs); an animesh's keeps the bones (its
        // skeleton is not an avatar's)
        let avatar = world
            .objects
            .index_of_uuid(&self.poser.subject)
            .and_then(|i| world.objects.get(i))
            .is_some_and(|o| o.is_avatar());
        // the pick is asked for an avatar; its meshes draw the part when the
        // scene drew any of them (an impostor, a jelly doll, your own avatar
        // in first person: the bone outline instead)
        // ... and only while the pointer is over the subject (its skeleton's
        // area on screen): Ctrl over the rest of the world costs nothing
        let over_subject = compact::body_area(
            joints
                .iter()
                .filter(|(j, _)| !optional_bone(&rig.canonical_names[*j]) || self.poser.rigged.contains(j))
                .map(|(_, at)| *at),
        )
        .is_some_and(|area| area.contains(pointer));
        self.poser.pick_wanted = ctrl && self.poser.part_hints && avatar && over_subject;
        let meshes = avatar && self.poser_meshes_drawn();
        // the mesh pick, once one came back: on the subject's surface, the
        // part of its joint; off it, none. Until then, the bones.
        let pointer_px = pointer * ctx.pixels_per_point();
        let picked = fresh_pick(self.poser.mesh_pick, self.poser.subject, pointer_px).filter(|_| self.poser.pick_wanted && meshes);
        let candidate = match picked {
            Some(class) => picked_part(class, &joints, &rig.parent, &rig.canonical_names, &offer, self.poser.group, pointer),
            None => offered_joint(&joints, &groups, &offer, self.poser.group, pointer),
        };
        if let Some(joint) = candidate.filter(|_| self.poser.part_hints && ctrl) {
            let members = part(&rig.parent, &keys, joint);
            let world_at: Vec<glam::Vec3> = frame.joints.iter().map(|m| m.w_axis.truncate()).collect();
            let project = |at| proj.project(at).map(|(p, _)| p);
            // the part's own shape, see-through in the accent: the face an
            // oval around the head (one smooth outline), the others their
            // bones' union (an animesh's: an avatar's is the renderer's)
            let fill = p.violet.gamma_multiply(0.32);
            let stroke = Stroke::new(1.5, p.violet_light.gamma_multiply(0.7));
            // the mesh pick already found the pointer on the part; the bones
            // decide only without it
            let inside = if picked.is_some() {
                true
            } else if keys[joint] == 1 {
                let head = frame.joints[rig.names.get("mHead").copied().unwrap_or(rig.pelvis)];
                let oval = silhouette::face(&rig.canonical_names, &world_at, &members, head, project)
                    .filter(|oval| silhouette::depth(&[*oval], pointer) > 0.0);
                if let Some(oval) = oval.filter(|_| !meshes) {
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
                if inside && !meshes {
                    silhouette::paint(&painter, &silhouette::trace(&capsules), fill, stroke);
                }
                inside
            };
            if inside {
                if meshes {
                    let joints = mask_members(keys[joint], &members, &rig.parent, &keys, &rig.canonical_names, &self.poser.rigged);
                    let mut mask = mask_joints(&rig.parent, &rig.canonical_names, &joints);
                    // the identity slot (the system body's root vertices) is
                    // the body's
                    let root = rig.len();
                    if keys[joint] == 0 && root < 256 {
                        mask[root >> 5] |= 1 << (root & 31);
                    }
                    self.poser.part_overlay = Some((self.poser.subject, mask));
                }
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
