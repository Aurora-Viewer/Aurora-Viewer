//! Photo posing over the current shaped Bento rig, independent of simulator motions.
//! Reference: fsfloaterposer.cpp / fsposeranimator.cpp, local joint frames and consent.
use super::*;
use crate::scene::{anim::BoundAnim, jobs::Jobs};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};

mod bones;
mod bvh;
mod compact;
mod controls;
mod data;
mod document;
mod kinematics;
mod overlay;
mod scale_gizmo;
mod silhouette;
mod storage;
mod subjects;
mod together;
mod trackball;
mod ui;
mod world_hints;
pub use compact::COMPACT_SIZE;
pub(crate) use data::Pose as SubjectPose;
#[cfg(test)]
mod tests;

/// The outfit portrait's standing pose (`snapshot::portrait`): the
/// skeleton's T, as the poser's T-pose sets it
/// (FSPoserAnimator::setAllAvatarStartingRotationsToZero), with both
/// shoulders lowered by `lowered` radians; the shape's own joint positions
/// and scales are kept. A poser layer like the others: drawn on our screen
/// only, nothing is sent.
pub(crate) fn standing_pose(world: &World, subject: Uuid, lowered: f32) -> Option<Arc<SubjectPose>> {
    let rig = &world.avatar_lib.rig;
    let frame = Frame::subject(world, subject)?;
    let mut pose = SubjectPose::capture(rig, &frame)?;
    pose.rotations.clone_from(&rig.local_rot);
    pose.origin.clone_from(&rig.local_rot);
    // the arms reach along ±Y in the T, the avatar facing +X: a turn about
    // X brings them down on both sides
    for (name, sign) in [("mShoulderLeft", -1.0), ("mShoulderRight", 1.0)] {
        if let Some(&joint) = rig.names.get(name) {
            pose.rotations[joint] = Quat::from_rotation_x(sign * lowered) * pose.rotations[joint];
        }
    }
    pose.valid(rig).then(|| Arc::new(pose))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub stop_on_close: bool,
    pub markers: bool,
    /// The joint markers show only while the pointer is over the avatar
    /// being posed (the selected joint's gizmo stays): an Aurora addition,
    /// the Legacy Viewer's markers are on or off.
    pub markers_hover: bool,
    /// The world's hints (`world_hints`): the tint of another group's part
    /// with Ctrl + click (`part_hints`), and the skeleton of another avatar
    /// to pose (`avatar_hints`), each with its own button.
    pub part_hints: bool,
    pub avatar_hints: bool,
    /// The selected joint's gizmo (rings, arrows, scale handles); off, the
    /// sliders and the markers' drags still pose it. The Legacy Viewer's
    /// visual manipulators toggle (FSFloaterPoser::onToggleVisualManipulators).
    pub gizmo: bool,
    /// The small window (`compact`) instead of the full one, switched by
    /// the title bar's button and remembered. The full window stays the
    /// default, as the Legacy Viewer's poser (FSFloaterPoser).
    pub compact: bool,
    pub natural: bool,
    pub dof: bool,
    pub sensitivity: f32,
    pub also_bvh: bool,
    pub unlock_pelvis: bool,
    pub confirm_overwrite: bool,
    /// Ask before a close that stops the pose and loses its edited joints.
    pub confirm_close: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            stop_on_close: true,
            markers: true,
            markers_hover: false,
            part_hints: true,
            avatar_hints: true,
            gizmo: true,
            compact: false,
            natural: true,
            dof: false,
            sensitivity: 0.5,
            also_bvh: false,
            unlock_pelvis: false,
            confirm_overwrite: false,
            confirm_close: true,
        }
    }
}
/// What the poser window asks of the app, which owns the consent state and
/// the Preferences: the subject menu lists the nearby avatars with their
/// consent action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Ask(Uuid),
    Cancel(Uuid),
    OpenPrivacy,
    /// The bridge's Repair, offered when its query service is silent or failed.
    RepairBridge,
}
/// Which side of the paired joints the window edits: the Côté selector. In
/// `Mirror` the left joint is edited and its counterpart follows (the
/// editor's symmetry).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Side {
    #[default]
    Left,
    Right,
    Mirror,
}
#[derive(Default)]
pub(super) struct State {
    pub active: bool,
    config: Config,
    settings_loaded: bool,
    subject: Uuid,
    sessions: HashMap<Uuid, data::Session>,
    pub(super) transform: usize,
    delta: bool,
    settings_open: bool,
    /// The compact window's options popover (its title bar's gear).
    options_open: bool,
    /// Where it opens: the compact body's upper right corner, last frame.
    options_anchor: Option<egui::Pos2>,
    library_open: bool,
    storage: storage::Service,
    preset: usize,
    drag_checkpoint: bool,
    name: String,
    priority: i32,
    pub(super) group: usize,
    filter: String,
    camera: Option<crate::build::geom::Cam>,
    scale_gizmo: scale_gizmo::Gizmo,
    side: Side,
    /// The joints added to the selected one with Shift + click: they follow
    /// its changes (`together`).
    together: Vec<usize>,
    requests: Vec<Request>,
    /// The consent requests travel through the worn bridge's query service
    /// (query.lsl): Ask waits for `Ready`, and the menu says why it waits.
    bridge: crate::world::bridge::QueryState,
    /// A joint, shaft or gizmo handle was under the pointer, or dragged, in
    /// the last frame: the next press is the poser's, not a HUD's.
    pointer_owned: bool,
    /// This frame, the selected joint's handles were under the pointer (no
    /// drag), and a marker or shaft was: what `world_hint` may look under
    /// with Ctrl (the handles, never a marker).
    gizmo_hover: bool,
    marker_hover: bool,
    /// Last frame, Ctrl showed another part or avatar under the pointer:
    /// the handles let it through this frame (`poser_ctrl_picks`).
    ctrl_hint: bool,
    /// The part the renderer draws on the subject's meshes this frame (its
    /// joints as `world_hints::mask_joints`), when Ctrl hovers one the
    /// meshes carry; None draws nothing (the bone outline, if any, is the
    /// poser's own painting).
    part_overlay: Option<(Uuid, [u32; 8])>,
    /// Ctrl hovers the subject (an avatar) with the parts on: the renderer
    /// is asked for its part under the cursor (part_pick.rs).
    pick_wanted: bool,
    /// Its newest answer: the subject it was asked for, the dominant joint
    /// plus one (0 off the subject) and the window pixel it was read at,
    /// trusted only there (`world_hints::fresh_pick`). None before the first
    /// since Ctrl came.
    mesh_pick: Option<(Uuid, u32, [f32; 2])>,
    /// Whether the scene drew any of the subject's meshes last frame, when it
    /// listed them (`set_poser_meshes_drawn`): an impostored, jelly-dolled or
    /// first-person subject has none, and keeps the bone outline. None:
    /// not known yet (taken as drawn).
    meshes_drawn: Option<(Uuid, bool)>,
    /// The subject the pick answers were last taken for, and how many
    /// answers still in flight to drop after a switch (they were drawn from
    /// the subject before).
    pick_for: Uuid,
    pick_drop: u8,
    /// The joints edited over every subject, counted as the window shows:
    /// what a close that stops the pose would lose.
    edited_total: usize,
    /// The window's close waits for the confirmation (Minimize or Close
    /// anyway), and its "Don't ask again" ticked.
    close_asked: bool,
    close_dont_ask: bool,
    /// How the compact window's Load list loads a pose.
    load_mode: document::LoadMode,
    /// A demo scenario's window mode, for the session only: the saved
    /// `compact` is never written by a scenario. The title bar's button
    /// drops it.
    window_override: Option<bool>,
    /// A click beside the joints and the gizmo left no joint selected: the
    /// pelvis is not picked again by itself (an Aurora addition).
    unselected: bool,
    /// Profile pictures for the subject menu and the head: the app's, for
    /// the residents drawn last frame (`pictures_drawn`); the missing ones
    /// are asked for a few a second (`want_picture`).
    pictures: HashMap<Uuid, egui::TextureHandle>,
    pictures_drawn: std::collections::HashSet<Uuid>,
    pictures_wanted: std::collections::HashSet<Uuid>,
    pictures_asked: std::collections::HashSet<Uuid>,
    picture_budget: crate::ui::nameplate::PictureBudget,
    /// The joints the subject's rigged meshes use (`Scene::rigged_joints`),
    /// refreshed by the app: the wings and the tail are offered only then.
    rigged: std::collections::HashSet<usize>,
    /// The Other bones shown (markers, shafts, the full window's list): those
    /// a rigged mesh uses or the pose moved, set each frame
    /// (`refresh_other_bones`); the rest of the wings, tail and hind limbs
    /// stay out of the way.
    other_used: std::collections::HashSet<String>,
    /// The used Other bones inside the body (spine, groin, skull): in the
    /// full window's list only, never in the world nor the compact window.
    other_inner_used: std::collections::HashSet<String>,
    /// "On hover": the pointer was near the silhouette last frame, and when
    /// it last was (`overlay::markers_reveal`).
    markers_near: bool,
    markers_seen_at: f64,
    /// The compact window's joint row shows its sliders.
    joint_open: bool,
    /// The angles the sliders last set (`controls::apply_joint_value`):
    /// shown while the joint keeps that rotation. Read back from the
    /// rotation, a Y past ±90° comes out as X and Z turned half a turn with
    /// Y on the other side, and the sliders jumped every frame.
    angles: Option<Angles>,
}
/// The angles the sliders set on a joint, and the rotation they made.
#[derive(Clone, Copy, Debug)]
pub(super) struct Angles {
    subject: Uuid,
    bone: usize,
    delta: bool,
    rotation: Quat,
    degrees: Vec3,
}
impl Deref for State {
    type Target = Config;
    fn deref(&self) -> &Config {
        &self.config
    }
}
impl DerefMut for State {
    fn deref_mut(&mut self) -> &mut Config {
        &mut self.config
    }
}
pub(super) fn group(name: &str) -> usize {
    if !name.starts_with('m') {
        return 4;
    }
    if name == "mSkull" {
        return 3;
    }
    match visibility::Group::of(name) {
        visibility::Group::Face => 1,
        visibility::Group::Fingers => 2,
        visibility::Group::Body | visibility::Group::Arms | visibility::Group::Legs => 0,
        _ => 3,
    }
}
/// The tab of the Other bones (wings, tail, hind limbs, groin, skull).
pub(super) const OTHER_GROUP: usize = 3;
/// An Other bone inside the body: the spine, the groin, the skull (the
/// Other bones that are not `optional_bone`). Full window's list only.
pub(super) fn inner_bone(name: &str) -> bool {
    group(name) == OTHER_GROUP && !optional_bone(name)
}
/// An Other bone that rests away from the body when nothing uses it: the
/// wings, the tail and the hind limbs (the spine, groin and skull lie inside
/// it). They do not widen a body's click or hover area.
pub(super) fn optional_bone(name: &str) -> bool {
    ["Wing", "Tail", "HindLimb"].iter().any(|part| name.contains(part))
}
pub(super) const FULL_WINDOW: &str = "poser";
pub(super) const COMPACT_WINDOW: &str = "poser-compact";
pub(super) fn window_id(compact: bool) -> &'static str {
    if compact { COMPACT_WINDOW } else { FULL_WINDOW }
}
impl Editor {
    pub(super) fn poser_edited(&self, bone: usize) -> bool {
        self.poser.active && self.poser.sessions.get(&self.poser.subject).is_some_and(|s| s.edited(bone))
    }
    pub fn poser_dof(&self) -> bool {
        self.poser.active && self.poser.dof
    }
    pub fn poser_focus(&self, world: &World, bone: &str) -> Option<Vec3> {
        self.editor_frame(world).map(|f| {
            f.joints[world.avatar_lib.rig.names.get(bone).copied().unwrap_or(world.avatar_lib.rig.pelvis)]
                .w_axis
                .truncate()
        })
    }
    pub fn posing(&self) -> bool {
        self.poser.active
    }
    /// The floater id of the poser's window in the mode it shows: each mode
    /// keeps its own place and size. The saved setting decides until the
    /// window has loaded it (the toolbar opens it before that).
    pub fn poser_window(&self, saved: &Config) -> &'static str {
        window_id(if self.poser.settings_loaded {
            self.compact_mode()
        } else {
            self.poser.window_override.unwrap_or(saved.compact)
        })
    }
    /// The app's profile pictures of the residents the poser drew last
    /// frame (a few clones of shared handles).
    pub fn set_poser_pictures(&mut self, all: &HashMap<Uuid, egui::TextureHandle>) {
        let drawn = std::mem::take(&mut self.poser.pictures_drawn);
        self.poser.pictures = drawn.iter().filter_map(|id| all.get(id).map(|t| (*id, t.clone()))).collect();
    }
    /// Whether the poser shows the joint `name`: its group is the open tab,
    /// and an Other bone only when it is used (`State::other_used`).
    pub(in crate::ik_edit) fn poser_bone_visible(&self, name: &str) -> bool {
        // the open tab's joints; of the Other bones, the used ones
        // (`refresh_other_bones`)
        let shown = group(name);
        shown == self.poser.group && (shown != OTHER_GROUP || self.poser.other_used.contains(name))
    }
    /// The Other bones listed this frame: weighted by a rigged mesh of the
    /// subject, or moved by its pose (a loaded pose that bends a wing keeps
    /// it reachable to reset). Aurora's own: the Legacy Viewer lists every
    /// Other bone. The ones inside the body (`inner_bone`: the spine, the
    /// groin, the skull) go to the full window's list alone, never to the
    /// world nor the compact window, even used: the spine's are collapsed
    /// pairs (mSpine2 back down on mSpine1, mSpine4 on mSpine3) whose markers
    /// sit upside down over the torso and whose moves tear the body apart;
    /// the body's own bones bend it. Unused, the wings, the tail and the
    /// hind limbs rest unseen.
    pub(super) fn refresh_other_bones(&mut self, rig: &Rig) {
        let session = self.poser.sessions.get(&self.poser.subject);
        let (inner, outer): (std::collections::HashSet<String>, _) = (0..rig.len())
            .filter(|b| group(&rig.canonical_names[*b]) == OTHER_GROUP)
            .filter(|b| self.poser.rigged.contains(b) || session.is_some_and(|s| s.edited(*b)))
            .map(|b| rig.canonical_names[b].clone())
            .partition(|name| inner_bone(name));
        self.poser.other_used = outer;
        self.poser.other_inner_used = inner;
    }
    /// Ctrl held in the poser over another part or avatar it shows (the
    /// last frame's hint, `ctrl_hint`): the Ctrl + click picks it, so the
    /// selected joint's handles let the pointer through. Elsewhere they keep
    /// Ctrl (its temporary rotation, a handle's click in the photo mode),
    /// and a drag already begun keeps them.
    pub(in crate::ik_edit) fn poser_ctrl_picks(&self, modifiers: egui::Modifiers) -> bool {
        self.poser.active && (modifiers.ctrl || modifiers.command) && self.poser.ctrl_hint
    }
    /// The part the renderer draws on the subject's meshes (the avatar and
    /// the joints mask of `aurora_render::FrameParams::part_joints`).
    pub fn poser_part_overlay(&self) -> Option<(Uuid, [u32; 8])> {
        self.poser.part_overlay.filter(|_| self.poser.active && self.open)
    }
    /// The subject whose part under the cursor the renderer reads this
    /// frame (part_pick.rs), while Ctrl hovers it.
    pub fn poser_pick_subject(&self) -> Option<Uuid> {
        (self.poser.active && self.open && self.poser.pick_wanted).then_some(self.poser.subject)
    }
    /// The renderer's part pick (`Renderer::take_part_pick`): the newest
    /// answer is kept while Ctrl is held, forgotten once it is let go.
    pub fn set_poser_mesh_pick(&mut self, answer: Option<(u32, [u32; 2])>) {
        // a switch of subject: the answers still in flight (at most one per
        // readback slot) are the former subject's
        if self.poser.pick_for != self.poser.subject {
            self.poser.pick_for = self.poser.subject;
            self.poser.mesh_pick = None;
            self.poser.pick_drop = 3;
        }
        if !self.poser.pick_wanted {
            self.poser.mesh_pick = None;
        } else if let Some((joint, [x, y])) = answer {
            if self.poser.pick_drop > 0 {
                self.poser.pick_drop -= 1;
            } else {
                self.poser.mesh_pick = Some((self.poser.subject, joint, [x as f32, y as f32]));
            }
        }
    }
    /// Whether the scene listed any of the subject's meshes for the part
    /// (None: it was not asked to).
    pub fn set_poser_meshes_drawn(&mut self, drawn: Option<bool>) {
        // kept while the scene is not asked (no Ctrl): the last known answer
        // for that subject, so its outline does not flicker at an edge
        if let Some(drawn) = drawn {
            self.poser.meshes_drawn = Some((self.poser.subject, drawn));
        }
    }
    /// Whether the subject's meshes are drawn, as last known (unknown:
    /// taken as drawn).
    pub(super) fn poser_meshes_drawn(&self) -> bool {
        self.poser
            .meshes_drawn
            .filter(|(s, _)| *s == self.poser.subject)
            .is_none_or(|(_, d)| d)
    }
    /// The subject posed now (you, an animesh or a resident).
    pub fn poser_subject(&self) -> Uuid {
        self.poser.subject
    }
    /// The joints the subject's rigged meshes use, from the scene.
    pub fn set_poser_rigged_joints(&mut self, joints: std::collections::HashSet<usize>) {
        self.poser.rigged = joints;
    }
    /// The profile pictures the poser waits for, for the app to ask.
    pub fn take_poser_wanted_pictures(&mut self) -> impl Iterator<Item = Uuid> + '_ {
        self.poser.pictures_wanted.drain()
    }
    /// The picture of `id` when the app has it; asked for otherwise, a few
    /// new residents a second as People does (`PeopleUi::want_pic`).
    pub(super) fn picture(&mut self, id: Uuid, now: f64) -> Option<egui::TextureHandle> {
        self.poser.pictures_drawn.insert(id);
        if let Some(t) = self.poser.pictures.get(&id) {
            return Some(t.clone());
        }
        if self.poser.pictures_asked.contains(&id) || self.poser.picture_budget.take(now) {
            self.poser.pictures_asked.insert(id);
            self.poser.pictures_wanted.insert(id);
        }
        None
    }
    pub(super) fn compact_mode(&self) -> bool {
        self.poser.window_override.unwrap_or(self.poser.compact)
    }
    /// A press on the scene now goes to a poser handle. The handles are
    /// painted in egui's Background layer, which the app does not count as
    /// UI, so the app asks before giving the press to a worn HUD.
    pub fn poser_owns_pointer(&self) -> bool {
        self.poser.active && self.poser.pointer_owned
    }
    /// The consent and Preferences actions chosen in the window since the
    /// last call.
    pub fn take_poser_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.poser.requests)
    }
    pub fn set_poser_bridge(&mut self, state: crate::world::bridge::QueryState) {
        self.poser.bridge = state;
    }
    pub fn select_pose_subject(&mut self, world: &World, id: Uuid) {
        self.pick_subject(world, id);
    }
    pub fn sync_poser_settings(&mut self, config: &mut Config) -> bool {
        if !self.poser.active && !self.poser.settings_loaded {
            return false;
        }
        if !self.poser.settings_loaded {
            self.poser.config = config.clone();
            self.poser.config.sensitivity = if config.sensitivity.is_finite() {
                config.sensitivity.clamp(0.05, 2.0)
            } else {
                0.5
            };
            self.poser.settings_loaded = true;
            self.space = if self.poser.natural { Space::Local } else { Space::World };
        }
        if *config != self.poser.config {
            *config = self.poser.config.clone();
            true
        } else {
            false
        }
    }
    pub fn open_poser(&mut self) {
        if self.poser.active {
            self.open = true;
            return;
        }
        self.leave_clip();
        let clip = std::mem::take(&mut self.clip);
        *self = Self {
            clip,
            open: true,
            initialized: true,
            ignore_script_ik: true,
            poser: State {
                active: true,
                name: aurora_i18n::tr!("poser-new-name").to_string(),
                priority: 4,
                ..Default::default()
            },
            ..Default::default()
        };
    }
    /// The photo viewfinder's Poser button: closes the window as its own
    /// close button does when it shows, else opens it. True when opened.
    pub fn toggle_poser(&mut self) -> bool {
        if self.open && self.poser.active {
            self.ask_close();
            false
        } else {
            self.open_poser();
            true
        }
    }
    /// The window's close, its cross, Ctrl+W or the viewfinder's button:
    /// when it would stop the pose and lose edited joints, it asks first.
    /// The Legacy Viewer closes at once (FSFloaterPoser::onClose).
    fn ask_close(&mut self) {
        if self.poser.stop_on_close && self.poser.confirm_close && self.poser.edited_total > 0 {
            self.poser.close_asked = true;
        } else {
            self.close_poser();
        }
    }
    fn close_poser(&mut self) {
        self.poser.close_asked = false;
        self.open = false;
        // the window no longer runs its hints: nothing stays lit nor picked
        // (a pose kept by "stop when closed" off goes on without them)
        self.poser.part_overlay = None;
        self.poser.pick_wanted = false;
        self.poser.mesh_pick = None;
        self.poser.ctrl_hint = false;
        if self.poser.stop_on_close {
            self.poser.active = false;
        }
        self.drag = None;
        self.gizmo.cancel();
        self.poser.scale_gizmo.cancel();
    }
    pub fn open_ik(&mut self) {
        if self.poser.active || !self.open {
            self.toggle();
        }
    }
    pub fn stop_editing(&mut self) {
        self.poser.active = false;
        self.leave_clip();
        self.open = false;
        self.cancel_link_pick();
        self.drag = None;
        self.gizmo.cancel();
    }
    pub fn tick_pose_storage(&mut self, jobs: &Jobs, ctx: &egui::Context, world: &World) {
        if !self.poser.active {
            return;
        }
        let loaded = self.poser.storage.tick(
            jobs,
            ctx,
            world.agent_id,
            world.avatar_lib.rig.clone(),
            world.avatar_lib.skeleton.as_ref(),
        );
        for (subject, loaded) in loaded {
            let model = Frame::subject(world, subject).map_or(Mat4::IDENTITY, |f| f.model);
            let Some(session) = self.poser.sessions.get_mut(&subject) else {
                continue;
            };
            match loaded {
                storage::Loaded::Pose(document, mode) => {
                    if document.apply(&world.avatar_lib.rig, session, mode, model) {
                        self.poser.name = document.name;
                        self.poser.priority = document.priority;
                    }
                }
                storage::Loaded::Animation(animation, mode) => {
                    let mut pose = ik::LocalPose {
                        rotations: session.pose.rotations.clone(),
                        positions: session.pose.positions.clone(),
                    };
                    let sample = ik::animations::Sample {
                        animation: Arc::new(BoundAnim::bind(animation, &world.avatar_lib.rig)),
                        frame: 0,
                    };
                    ik::animations::apply(&world.avatar_lib.rig, &mut pose, Vec3::ZERO, &[sample]);
                    let selective = matches!(mode, document::LoadMode::Selective | document::LoadMode::SelectiveRotations);
                    let rotations = matches!(mode, document::LoadMode::Rotations | document::LoadMode::SelectiveRotations);
                    let edited: Vec<_> = (0..world.avatar_lib.rig.len()).map(|b| session.edited(b)).collect();
                    session.checkpoint();
                    let current = Arc::make_mut(&mut session.pose);
                    for (bone, edited) in edited.iter().enumerate() {
                        if selective && *edited {
                            continue;
                        }
                        // a joint the file drives is the file's, not the capture's
                        if current.rotations[bone].dot(pose.rotations[bone]).abs() < 0.9999999 {
                            current.origin[bone] = world.avatar_lib.rig.local_rot[bone];
                        }
                        current.rotations[bone] = pose.rotations[bone];
                        if !rotations {
                            current.positions[bone] = pose.positions[bone];
                        }
                        current.locks.remove(&bone);
                    }
                    current.keep_offsets(&world.avatar_lib.rig, &session.base);
                }
            }
            self.refresh = true;
        }
        self.clip.prepared.append(&mut self.poser.storage.prepared);
    }
    fn poser_transform(&mut self, rig: &Rig, frame: &Frame, position: Vec3, rotation: Quat, rotated: bool) {
        let Some(bone) = self.selected else { return };
        let (scale, q, p) = rig.parent[bone].map_or((rig.parent_scale[bone], Quat::IDENTITY, Vec3::ZERO), |parent| {
            (frame.model.inverse() * frame.joints[parent]).to_scale_rotation_translation()
        });
        self.position = p + q * (scale * position);
        self.set_rotation(q * rotation);
        self.mode = Mode::Direct;
        self.root = bone;
        self.commit_manipulation(rig, frame, rotated);
    }
    pub(super) fn poser_demo(&mut self, world: &World, frame: u64, mode: &str) {
        if frame == 420 {
            self.open_poser();
            // the scenarios made before the compact window show the full one
            // (after the open, which starts the poser's state anew)
            self.poser.window_override = Some(mode.starts_with("poser-compact"));
            // compact-sliders: the joint row's sliders open
            self.poser.joint_open = mode == "poser-compact-sliders";
            // compact-options: the gear's popover open
            self.poser.options_open = mode == "poser-compact-options";
            self.pose_layers(world);
        }
        // compact-save: a name typed for Save's form (opened by the app)
        if mode == "poser-compact-save" && frame == 480 {
            self.poser.name = "Demo pose".into();
        }
        // compact-face: no joint selected, so that no gizmo covers the face;
        // compact-body: the Hands posed, so the body is another group's part
        if mode == "poser-compact-face" && frame == 500 {
            // as a click beside: the pelvis is not picked again
            self.selected = None;
            self.poser.unselected = true;
            self.clear_together();
        }
        if mode == "poser-compact-body"
            && frame == 500
            && let Some(pose) = self.editor_frame(world)
        {
            self.poser.group = 2;
            self.select_pose_group(2, &world.avatar_lib.rig, &pose);
        }
        // the head edited at 450, then the window's cross: the confirmation
        if mode == "poser-close" && frame == 700 {
            self.ask_close();
        }
        if frame == 450
            && let Some(pose) = self.editor_frame(world)
        {
            let rig = &world.avatar_lib.rig;
            self.select(rig.names["mHead"], rig, &pose);
            if let Some((local, _)) = authoring::local_pose(rig, &pose) {
                self.poser_transform(rig, &pose, local.positions[rig.names["mHead"]], Quat::from_rotation_z(0.35), true);
            }
            if mode == "poser-connected" {
                self.pose_operation(rig, &pose, controls::Operation::TPose);
            } else if mode == "poser-scale" {
                self.poser.transform = 2;
            } else if mode == "poser-hands" {
                self.poser.group = 2;
                if let Some(&bone) = rig.names.get("mHandIndex1Right") {
                    self.select(bone, rig, &pose);
                }
            } else if mode == "poser-face" {
                self.poser.group = 1;
                if let Some(&bone) = rig.names.get("mFaceLipUpperCenter") {
                    self.select(bone, rig, &pose);
                }
            } else if mode == "poser-together" || mode == "poser-compact-together" {
                // both eyes selected (the right one with Shift + click), the
                // left one turned: the right one takes the same turn
                self.poser.group = 1;
                if let (Some(&left), Some(&right)) = (rig.names.get("mEyeLeft"), rig.names.get("mEyeRight")) {
                    self.click_pose_bone(left, rig, &pose, false);
                    self.click_pose_bone(right, rig, &pose, true);
                    if let Some((local, _)) = authoring::local_pose(rig, &pose) {
                        self.poser_transform(rig, &pose, local.positions[left], Quat::from_rotation_z(0.45), true);
                    }
                }
            } else if mode == "poser-physics" {
                self.poser.group = 4;
                self.select(rig.names["CHEST"], rig, &pose);
                self.poser.transform = 2;
            } else if mode == "poser-misc" {
                self.poser.group = 3;
                self.select(rig.names["mWing1Left"], rig, &pose);
            } else if mode == "poser-library" {
                self.poser.library_open = true;
            } else if mode == "poser-settings" {
                self.poser.settings_open = true;
            }
        }
        if frame == 480
            && mode == "poser-connected"
            && let Some(frame) = self.editor_frame(world)
        {
            let rig = &world.avatar_lib.rig;
            self.poser.transform = 1;
            self.select(rig.names["mWristLeft"], rig, &frame);
            self.position += Vec3::new(0.15, -0.15, -0.22);
            self.commit_poser_manipulation(rig, &frame, false);
        }
        if frame == 510
            && (mode == "poser-animesh" || mode == "poser-consent-accept" || mode == "poser-consent-withdraw")
            && let Some(frame) = self.editor_frame(world)
        {
            let rig = &world.avatar_lib.rig;
            self.select(rig.names["mHead"], rig, &frame);
            if let Some((local, _)) = authoring::local_pose(rig, &frame) {
                self.poser_transform(rig, &frame, local.positions[rig.names["mHead"]], Quat::from_rotation_z(0.55), true);
            }
        }
    }
}
