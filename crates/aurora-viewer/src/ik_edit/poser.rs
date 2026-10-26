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
mod controls;
mod data;
mod document;
mod kinematics;
mod overlay;
mod scale_gizmo;
mod storage;
mod subjects;
mod trackball;
mod ui;
pub(crate) use data::Pose as SubjectPose;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub stop_on_close: bool,
    pub markers: bool,
    pub natural: bool,
    pub dof: bool,
    pub sensitivity: f32,
    pub also_bvh: bool,
    pub unlock_pelvis: bool,
    pub confirm_overwrite: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            stop_on_close: true,
            markers: true,
            natural: true,
            dof: false,
            sensitivity: 0.5,
            also_bvh: false,
            unlock_pelvis: false,
            confirm_overwrite: false,
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
    requests: Vec<Request>,
    /// The consent requests travel through the worn bridge's query service
    /// (query.lsl): Ask waits for `Ready`, and the menu says why it waits.
    bridge: crate::world::bridge::QueryState,
    /// A joint, shaft or gizmo handle was under the pointer, or dragged, in
    /// the last frame: the next press is the poser's, not a HUD's.
    pointer_owned: bool,
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
    fn close_poser(&mut self) {
        self.open = false;
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
            self.pose_layers(world);
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
