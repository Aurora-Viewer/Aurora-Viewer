//! Local IK authoring: frozen rendered skeleton, joint handles, read-only
//! object picking and offsets in the target prim's unscaled local axes.
use crate::{
    scene::{
        Scene,
        anim::Rig,
        ik::{self, Goal, Mode},
    },
    theme::Palette,
    ui::{
        hud::Projector,
        widgets::{self, Floater},
    },
    world::World,
};
use egui::{Pos2, Sense, Stroke};
use glam::{EulerRot, Mat4, Quat, Vec3, Vec4};
use std::time::Instant;
use uuid::Uuid;

mod animations;
mod authoring;
mod bindings;
mod clip;
mod gizmo;
mod graph;
mod links;
mod object_data;
mod poles;
mod pose;
pub(crate) mod poser;
mod prim_targets;
mod profiles;
mod project;
mod rig_controls;
mod segments;
mod visibility;
use gizmo::{Geometry, Gizmo, Pose, Space, Tool};
use rig_controls::{Mirrors, limbs};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Control {
    #[default]
    Bone,
    Pole,
}

#[derive(Default)]
pub struct Editor {
    pub open: bool,
    /// The poser's window shows over the photo viewfinder, whose controls
    /// sit above the windows: it goes up with them (#1389).
    pub over_viewfinder: bool,
    pub picking: bool,
    pub target: Option<Uuid>,
    pub target_name: String,
    pub goals: Vec<Goal>,
    authoring: authoring::State,
    bindings: Vec<bindings::Binding>,
    disabled: Vec<usize>,
    initialized: bool,
    ignore_script_ik: bool,
    canonical_names: Vec<String>,
    weight: f32,
    visibility: visibility::Visibility,
    animations: animations::Stack,
    tab: u8,
    graph: graph::State,
    clip: clip::State,
    poser: poser::State,
    prims: prim_targets::State,
    links: links::State,
    profiles: profiles::State,
    project: project::State,
    object_data: object_data::State,
    selected: Option<usize>,
    root: usize,
    mode: Mode,
    position: Vec3,
    degrees: Vec3,
    contact: Vec3,
    orient: bool,
    filter: String,
    drag: Option<Drag>,
    refresh: bool,
    tool: Tool,
    space: Space,
    gizmo: Gizmo,
    symmetry: bool,
    mirrors: Mirrors,
    limb_controls: bool,
    control: Control,
    pole: Option<Vec3>,
    pole_orbit: Option<poles::Orbit>,
}

#[derive(Clone, Copy)]
enum Drag {
    Move {
        point: Vec3,
        normal: Vec3,
        offset: Vec3,
    },
    Rotate {
        origin: Pos2,
        rotation: Quat,
        right: Vec3,
        up: Vec3,
    },
}

pub struct Frame {
    pub model: Mat4,
    pub joints: Vec<Mat4>,
    at: Instant,
}

impl Frame {
    pub fn new(world: &World) -> Option<Self> {
        Self::subject(world, world.agent_id)
    }
    pub fn subject(world: &World, subject: Uuid) -> Option<Self> {
        let index = world.objects.index_of_uuid(&subject)?;
        let at = Instant::now();
        let (p, q, hud) = Scene::object_transform(world, index, at, 0)?;
        if hud {
            return None;
        }
        let model = world
            .avatar_models
            .get(&subject)
            .copied()
            .unwrap_or_else(|| Mat4::from_rotation_translation(q, p) * Mat4::from_translation(-world.avatar_lib.pelvis));
        let poses = world.avatar_poses.get(&subject)?;
        let rig = &world.avatar_lib.rig;
        if poses.len() < rig.len() {
            return None;
        }
        let joints = poses
            .iter()
            .zip(&rig.default_world_inv)
            .map(|(p, inverse)| model * *p * inverse.inverse())
            .collect();
        Some(Self { model, joints, at })
    }
}

/// Prim scale deliberately does not enter: LSL's position offset is metric.
#[cfg(test)]
pub fn offsets(joint: Mat4, contact: Vec3, target_p: Vec3, target_q: Quat) -> Option<(Vec3, Quat)> {
    let (_, q, _) = joint.to_scale_rotation_translation();
    let p = target_q.inverse() * (joint.transform_point3(contact) - target_p);
    let q = (target_q.inverse() * q).normalize();
    (p.is_finite() && q.is_finite()).then_some((p, q))
}

fn ray(proj: &Projector, at: Pos2) -> Option<(Vec3, Vec3)> {
    let inverse = proj.view_proj.inverse();
    let point = |z| {
        let p = inverse
            * Vec4::new(
                at.x * proj.ppp / proj.width * 2.0 - 1.0,
                1.0 - at.y * proj.ppp / proj.height * 2.0,
                z,
                1.0,
            );
        p.truncate() / p.w
    };
    let near = point(1.0);
    Some((near, (point(0.0001) - near).try_normalize()?))
}

impl Editor {
    fn active_tool(&self, modifiers: egui::Modifiers) -> Tool {
        if modifiers.ctrl {
            Tool::Rotate
        } else if modifiers.shift {
            Tool::Move
        } else {
            self.tool
        }
    }
    fn gizmo_visible(&self, modifiers: egui::Modifiers) -> bool {
        self.gizmo.dragging() || modifiers.ctrl || !modifiers.shift
    }

    fn point_drag_enabled(&self, modifiers: egui::Modifiers) -> bool {
        self.drag.is_some() || modifiers.shift || modifiers.ctrl
    }

    fn control_position(&self) -> Vec3 {
        if self.control == Control::Pole {
            self.pole.unwrap_or(self.position)
        } else {
            self.position
        }
    }

    fn move_control(&mut self, point: Vec3) {
        if self.control == Control::Pole {
            self.pole = Some(point);
        } else {
            self.position = point;
        }
    }
    fn set_rotation(&mut self, rotation: Quat) {
        let (z, y, x) = rotation.to_euler(EulerRot::ZYX);
        self.degrees = Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI);
        self.orient = true;
    }

    fn rotation(&self) -> Quat {
        Quat::from_euler(
            EulerRot::ZYX,
            self.degrees.z.to_radians(),
            self.degrees.y.to_radians(),
            self.degrees.x.to_radians(),
        )
    }

    fn start_drag(&mut self, proj: &Projector, frame: &Frame, pointer: Pos2, rotate: bool) {
        self.drag = if rotate {
            // Unproject an entire near-plane span: a one-pixel difference
            // loses precision at region coordinates far from the origin.
            let inverse = proj.view_proj.inverse();
            let near = |x, y| {
                let p = inverse * Vec4::new(x, y, 1.0, 1.0);
                p.truncate() / p.w
            };
            let model_inverse = frame.model.inverse();
            let right = model_inverse.transform_vector3(near(1.0, 0.0) - near(-1.0, 0.0)).try_normalize();
            let up = model_inverse.transform_vector3(near(0.0, 1.0) - near(0.0, -1.0)).try_normalize();
            right.zip(up).map(|(right, up)| Drag::Rotate {
                origin: pointer,
                rotation: if self.control == Control::Pole {
                    Quat::IDENTITY
                } else {
                    self.rotation()
                },
                right,
                up,
            })
        } else {
            let center = egui::pos2(proj.width / proj.ppp * 0.5, proj.height / proj.ppp * 0.5);
            ray(proj, pointer).zip(ray(proj, center)).and_then(|((o, d), (_, normal))| {
                let point = frame.model.transform_point3(self.control_position());
                crate::build::geom::ray_plane(o, d, point, normal).map(|hit| Drag::Move {
                    point,
                    normal,
                    offset: point - hit,
                })
            })
        };
    }

    fn drag_to(&mut self, proj: &Projector, frame: &Frame, rig: &Rig, pointer: Pos2) {
        match self.drag {
            Some(Drag::Move { point, normal, offset }) => {
                if let Some((o, d)) = ray(proj, pointer)
                    && let Some(hit) = crate::build::geom::ray_plane(o, d, point, normal)
                {
                    self.move_control(frame.model.inverse().transform_point3(hit + offset));
                    self.commit_manipulation(rig, frame, false);
                }
            }
            Some(Drag::Rotate {
                origin,
                rotation,
                right,
                up,
            }) => {
                let delta = pointer - origin;
                let rotation =
                    (Quat::from_axis_angle(right, delta.y * 0.01) * Quat::from_axis_angle(up, delta.x * 0.01) * rotation).normalize();
                if self.control == Control::Pole {
                    self.rotate_pole(rotation);
                } else {
                    self.set_rotation(rotation);
                }
                self.commit_manipulation(rig, frame, true);
            }
            None => {}
        }
    }

    pub fn demo_tick(&mut self, world: &World, frame: u64, mode: &str) {
        if mode.starts_with("poser") {
            self.poser_demo(world, frame, mode);
            return;
        }
        if frame == 420 {
            self.toggle();
            if ["library-empty", "library-new", "library-profile", "library-profile-name"].contains(&mode) {
                self.library_creation_demo(world, mode);
                return;
            }
            if let Some(pose) = Frame::new(world) {
                self.library_demo(world, "");
                let rig = &world.avatar_lib.rig;
                self.select(rig.names["mWristLeft"], rig, &pose);
                self.contact = Vec3::new(0.0, 0.055, -0.015);
                self.position = pose
                    .model
                    .inverse()
                    .transform_point3(pose.joints[rig.names["mWristLeft"]].transform_point3(self.contact));
                self.target = Some(crate::demo::action_id(crate::demo::hand_ik::LEFT));
                self.target_name = "Left".into();
                self.filter = "mWrist".into();
                self.tool = if mode == "gizmo-rotate" { Tool::Rotate } else { Tool::Move };
                self.space = if mode == "gizmo-local" { Space::Local } else { Space::World };
                if mode != "1" {
                    self.position += Vec3::new(-0.03, 0.02, -0.015);
                    self.degrees.z += 10.0;
                    self.orient = true;
                }
                self.commit();
                if mode.starts_with("links-") {
                    self.initialized = true;
                    self.visibility.body();
                    self.tab = if mode == "links-visible" { 3 } else { 1 };
                    if mode == "links-offsets" {
                        self.select(rig.names["mWristRight"], rig, &pose);
                        self.links.reveal_offsets = true;
                    }
                    let source = self.selected.unwrap_or(rig.pelvis);
                    if mode == "links-add" {
                        self.links.source = Some(source);
                    } else if mode == "links-bone-pick" {
                        self.begin_link(world, &pose, source, false);
                    } else if mode == "links-bone" {
                        self.begin_link(world, &pose, source, false);
                        self.picked_bone(
                            world,
                            &pose,
                            crate::world::rig_graph::Target::Bone {
                                avatar: Uuid::nil(),
                                bone: "mHead".into(),
                            },
                        );
                    } else if [
                        "links-prim",
                        "links-menu",
                        "links-pose",
                        "links-offsets",
                        "links-multiple",
                        "links-search",
                    ]
                    .contains(&mode)
                    {
                        self.links.source = Some(source);
                        self.finish_link(
                            world,
                            &pose,
                            crate::world::rig_graph::Target::Prim {
                                id: crate::demo::action_id(if mode == "links-offsets" {
                                    crate::demo::hand_ik::RIGHT
                                } else {
                                    crate::demo::hand_ik::LEFT
                                }),
                                name: if mode == "links-offsets" { "Right" } else { "Left" }.into(),
                            },
                        );
                        self.links.reveal_menu = mode == "links-menu";
                        if ["links-multiple", "links-search"].contains(&mode) {
                            self.graph.definition.bindings[0].weight = 0.4;
                            self.links.adding = true;
                            self.bind_target(
                                world,
                                &pose,
                                source,
                                crate::world::rig_graph::Target::Prim {
                                    id: crate::demo::action_id(crate::demo::hand_ik::RIGHT),
                                    name: "Right".into(),
                                },
                            );
                            self.graph.definition.bindings[1].weight = 0.6;
                            if mode != "links-search" {
                                self.links.reveal_offsets = true;
                            }
                        }
                        if mode == "links-pose" {
                            self.tab = 0;
                        }
                    }
                }
                if mode.starts_with("build-") {
                    self.attach(world, &pose);
                    self.tab = 1;
                    self.visibility.body();
                }
                if mode.starts_with("profiles") || mode.starts_with("library") {
                    self.graph_demo(world, &pose, "graph");
                    self.graph.selected = None;
                    self.ensure_profiles(world, &pose);
                    self.animations.demo(rig, "animations");
                    self.add_profile(true, world, &pose);
                    self.graph.definition.profiles[1].name = "Gas only".into();
                    self.tab = if mode == "profiles-animations" { 2 } else { 1 };
                    if mode.starts_with("library") {
                        self.library_demo(world, mode);
                        if mode.starts_with("library-target") {
                            self.project_target_demo(mode);
                        }
                    }
                }
                if mode.starts_with("animations") {
                    self.attach(world, &pose);
                    self.animations.demo(rig, mode);
                    self.tab = 2;
                    self.visibility.body();
                }
                if mode.starts_with("graph") {
                    self.tab = 4;
                    if mode != "graph-empty" {
                        self.graph_demo(world, &pose, mode);
                    }
                }
                if mode.starts_with("clip") {
                    self.clip_demo(world, &pose, mode);
                }
                if mode.starts_with("rig") {
                    for limb in limbs(rig) {
                        self.select(limb.end, rig, &pose);
                        self.root = ik::extended_root(rig, limb.end).unwrap_or(limb.root);
                        self.mode = Mode::Ik;
                        let mut pole = limb.pole(&pose);
                        if mode == "rig-poles" {
                            let root = pose.model.inverse().transform_point3(pose.joints[limb.root].w_axis.truncate());
                            if let Some(axis) = (self.position - root).try_normalize() {
                                pole = root + Quat::from_axis_angle(axis, 0.65) * (pole - root);
                            }
                        }
                        self.pole = Some(pole);
                        if mode == "rig-body" && !limb.leg {
                            self.position += Vec3::new(0.5, 0.0, 0.0);
                        }
                        self.commit();
                    }
                    self.select(rig.names["mWristLeft"], rig, &pose);
                    if mode == "rig-symmetry" {
                        self.symmetry = true;
                        self.position += Vec3::new(0.04, 0.03, -0.02);
                        self.commit();
                    }
                    if ["rig-elbow", "rig-knee"].contains(&mode) {
                        let limb = limbs(rig)[if mode == "rig-knee" { 3 } else { 0 }];
                        self.select(limb.end, rig, &pose);
                        self.control = Control::Pole;
                        self.tool = Tool::Rotate;
                        self.tab = 0;
                    }
                    self.filter = "mWrist".into();
                    if mode == "rig-groups" {
                        self.visibility.only(visibility::Group::Arms);
                        self.tab = 3;
                    }
                    if ["rig-bindings", "rig-activation", "rig-weight", "rig-disabled", "rig-released"].contains(&mode) {
                        self.tab = 1;
                        for limb in limbs(rig) {
                            self.select(limb.end, rig, &pose);
                            self.target = Some(crate::demo::action_id(if limb.right {
                                crate::demo::hand_ik::RIGHT
                            } else {
                                crate::demo::hand_ik::LEFT
                            }));
                            self.target_name = if limb.right { "Right".into() } else { "Left".into() };
                            self.weight = if mode == "rig-weight" { 0.5 } else { 1.0 };
                            self.position += Vec3::X * 0.08;
                            self.attach(world, &pose);
                        }
                        self.select(rig.names["mWristLeft"], rig, &pose);
                        if mode == "rig-activation" {
                            for binding in &mut self.bindings {
                                binding.definition.activation = crate::world::rig_graph::Activation {
                                    distance: 0.5,
                                    ease_in: 0.6,
                                    ease_out: 0.8,
                                };
                            }
                        }
                    }
                }
                let virtual_node = self.graph.selected;
                self.ensure_profiles(world, &pose);
                if self.graph.definition.profiles.is_empty() {
                    self.add_profile(false, world, &pose);
                }
                if mode == "links-search" {
                    self.links.filter = "Right".into();
                }
                if let Some(id) = virtual_node {
                    self.select_virtual_node(id);
                }
            }
        }
        if (mode.starts_with("seat-reload") || matches!(mode, "library-object-import" | "library-object-waiting")) && frame == 460 {
            self.tab = 5;
            self.object_data.requested = Some(crate::demo::action_id(crate::demo::hand_ik::ROOT));
        }
        if ["pose-hierarchy", "pose-toes"].contains(&mode)
            && let Some(pose) = Frame::new(world)
        {
            let rig = &world.avatar_lib.rig;
            if frame == 460 {
                self.goals.clear();
                self.authoring = authoring::State::default();
                self.filter.clear();
                self.tab = 1;
            }
            let edit = match frame {
                460 => Some(("mHead", Vec3::X, 0.25)),
                480 => Some(("mNeck", Vec3::Y, 0.45)),
                500 => Some(("mCollarLeft", Vec3::X, -0.3)),
                520 => Some(("mShoulderLeft", Vec3::X, 0.65)),
                540 => Some(("mFootLeft", Vec3::Y, -0.3)),
                560 => Some(("mToeLeft", Vec3::Y, 0.6)),
                _ => None,
            };
            if let Some((name, axis, angle)) = edit {
                let bone = rig.names[name];
                self.select(bone, rig, &pose);
                let parent = rig.parent[bone].map_or(Quat::IDENTITY, |p| {
                    (pose.model.inverse() * pose.joints[p]).to_scale_rotation_translation().1
                });
                self.set_rotation(parent * Quat::from_axis_angle(axis, angle) * rig.local_rot[bone]);
                self.commit_manipulation(rig, &pose, true);
            }
            if frame == 580 {
                self.select(rig.names[if mode == "pose-toes" { "mToeLeft" } else { "mNeck" }], rig, &pose);
                self.tool = Tool::Rotate;
            }
        }
        if ["segments", "segments-toe"].contains(&mode)
            && frame == 460
            && let Some(pose) = Frame::new(world)
        {
            self.goals.clear();
            self.filter.clear();
            self.tab = 1;
            let rig = &world.avatar_lib.rig;
            self.select(
                rig.names[if mode == "segments-toe" { "mToeLeft" } else { "mShoulderLeft" }],
                rig,
                &pose,
            );
        }
        if ["human", "torso"].contains(&mode)
            && frame == 460
            && let Some(pose) = Frame::new(world)
        {
            self.goals.clear();
            self.select(world.avatar_lib.rig.names["mTorso"], &world.avatar_lib.rig, &pose);
            self.filter.clear();
            self.tab = if mode == "human" { 3 } else { 0 };
            if mode == "torso" {
                self.position += Vec3::new(0.05, 0.0, 0.02);
                self.set_rotation(Quat::from_rotation_y(0.3) * self.rotation());
                self.commit_manipulation(&world.avatar_lib.rig, &pose, false);
            }
        }
        if ["rig-disabled", "rig-released"].contains(&mode)
            && frame == 520
            && let Some(pose) = Frame::new(world)
        {
            for limb in limbs(&world.avatar_lib.rig) {
                self.enable(limb.end, false, &world.avatar_lib.rig, &pose);
            }
        }
        if mode == "rig-released"
            && frame == 560
            && let Some(pose) = Frame::new(world)
        {
            self.select(world.avatar_lib.rig.names["mAnkleRight"], &world.avatar_lib.rig, &pose);
            self.position += Vec3::new(0.12, -0.03, 0.08);
            self.set_rotation(Quat::from_rotation_y(0.3) * self.rotation());
            self.commit();
        }
        if ["release", "animations-release", "clip-release"].contains(&mode) && frame == 720 {
            self.toggle();
        }
    }

    pub fn toggle(&mut self) {
        if self.poser.active {
            self.poser.active = false;
            self.open = false;
        }
        if self.open {
            self.leave_clip();
            self.open = false;
            self.cancel_link_pick();
            self.drag = None;
            self.gizmo.cancel();
        } else {
            let clip = std::mem::take(&mut self.clip);
            *self = Self {
                clip,
                open: true,
                limb_controls: true,
                initialized: true,
                ignore_script_ik: true,
                tab: 1,
                ..Default::default()
            };
        }
    }

    pub fn request(&self) -> ik::EditRequest {
        if !self.open && !(self.poser.active && !self.poser.stop_on_close) {
            return ik::EditRequest::default();
        }
        if self.poser.active {
            return ik::EditRequest::default();
        }
        if self.clip.active {
            return ik::EditRequest {
                active: true,
                suppress_script_ik: true,
                animations: if self.clip.waiting_source() {
                    self.animations.samples()
                } else {
                    self.clip.sample()
                },
                suppress_live: self.clip.waiting_source() && self.animations.suppress_live,
                goals: self
                    .goals
                    .iter()
                    .filter(|g| !self.disabled.contains(&g.bone) && !self.authoring.owns(g.bone))
                    .cloned()
                    .collect(),
                joints: self.authoring.request(&self.goals, &self.disabled),
                released: (0..self.canonical_names.len()).collect(),
            };
        }
        ik::EditRequest {
            active: self.open,
            suppress_script_ik: self.ignore_script_ik,
            animations: self.animations.samples(),
            suppress_live: self.animations.suppress_live,
            joints: self.authoring.request(&self.goals, &self.disabled),
            goals: self
                .goals
                .iter()
                .filter(|g| {
                    !self.disabled.contains(&g.bone)
                        && !self.authoring.owns(g.bone)
                        && !self.bindings.iter().any(|b| b.bone == g.bone && !b.available)
                        && !self
                            .graph
                            .definition
                            .bindings
                            .iter()
                            .any(|b| self.canonical_names.get(g.bone) == Some(&b.bone))
                })
                .cloned()
                .chain(self.graph.request_goals().into_iter().filter(|g| !self.disabled.contains(&g.bone)))
                .collect(),
            released: self
                .disabled
                .iter()
                .copied()
                .chain(self.bindings.iter().filter(|b| !b.available).map(|b| b.bone))
                .chain(
                    self.graph
                        .definition
                        .bindings
                        .iter()
                        .filter_map(|b| self.canonical_names.iter().position(|n| n == &b.bone)),
                )
                .collect(),
        }
    }

    fn select(&mut self, bone: usize, rig: &Rig, frame: &Frame) {
        if self.poser.active {
            self.select_pose_bone(bone, rig, frame);
            return;
        }
        self.graph.selected = None;
        self.graph.binding = self.graph.binding_index(bone, &rig.canonical_names);
        self.mirrors.initialize(rig);
        if self.canonical_names.is_empty() {
            self.canonical_names.clone_from(&rig.canonical_names);
        }
        self.weight = 1.0;
        if let Some(binding) = self.bindings.iter().find(|b| b.bone == bone) {
            self.target = Some(binding.definition.target);
            self.target_name.clone_from(&binding.name);
            self.graph.target = crate::world::rig_graph::Target::Prim {
                id: binding.definition.target,
                name: binding.name.clone(),
            };
        }
        if let Some(binding) = self.graph.binding.and_then(|index| self.graph.definition.bindings.get(index)) {
            self.graph.target = binding.target.clone();
        }
        self.control = Control::Bone;
        self.tool = Tool::Move;
        self.pole_orbit = None;
        self.pole = None;
        self.drag = None;
        self.gizmo.cancel();
        self.selected = Some(bone);
        self.root = ik::default_root(rig, bone);
        self.mode = if rig.parent[bone].is_some() {
            Mode::Ik
        } else {
            // The skeleton root has no parent or segment to preserve.
            Mode::Direct
        };
        self.contact = Vec3::ZERO;
        self.orient = false;
        let local = frame.model.inverse() * frame.joints[bone];
        let (_, q, p) = local.to_scale_rotation_translation();
        self.position = p;
        let (z, y, x) = q.to_euler(EulerRot::ZYX);
        self.degrees = Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI);
        let bound = (!self.clip.active)
            .then(|| self.graph.binding.and_then(|index| self.graph.binding_goal(index, rig)))
            .flatten();
        if let Some(goal) = bound.as_ref().or_else(|| self.goals.iter().find(|g| g.bone == bone)) {
            self.weight = goal.weight;
            self.position = goal.position;
            self.root = goal.root;
            self.mode = goal.mode;
            self.contact = goal.contact;
            self.orient = goal.rotation.is_some();
            self.pole = goal.pole;
            if let Some(q) = goal.rotation {
                let (z, y, x) = q.to_euler(EulerRot::ZYX);
                self.degrees = Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI);
            }
        }
        if self.disabled.contains(&bone)
            || (!self.clip.active
                && self
                    .graph
                    .binding
                    .and_then(|i| self.graph.definition.bindings.get(i))
                    .is_some_and(|b| !b.enabled))
        {
            self.released_pose(frame);
        } else if self.authoring.owns(bone) {
            self.position = local.transform_point3(self.contact);
            let (z, y, x) = q.to_euler(EulerRot::ZYX);
            self.degrees = Vec3::new(x, y, z) * (180.0 / std::f32::consts::PI);
        }
    }

    fn commit(&mut self) {
        if self.clip.waiting_source() {
            return;
        }
        if self.graph.selected.is_some() {
            self.graph
                .capture_node(crate::world::rig_graph::Transform::new(self.position, self.rotation()));
            return;
        }
        let Some(bone) = self.selected else {
            return;
        };
        if !self.weight.is_finite()
            || !(0.0..=1.0).contains(&self.weight)
            || !self.position.is_finite()
            || !self.contact.is_finite()
            || !self.degrees.is_finite()
            || self.contact.length() > 0.3
            || self.pole.is_some_and(|p| !p.is_finite())
        {
            return;
        }
        let goal = Goal {
            follow: None,
            weight: self.weight,
            bone,
            root: self.root,
            position: self.position,
            contact: self.contact,
            mode: self.mode,
            pole: if self.mode == Mode::Ik { self.pole } else { None },
            rotation: self.orient.then(|| self.rotation()),
        };
        let mirrored = self.symmetry.then(|| self.mirrors.goal(&goal)).flatten();
        for goal in std::iter::once(goal).chain(mirrored) {
            // An explicit pose edit starts a new correction. Merely selecting,
            // refreshing or adjusting a row's strength never reactivates it.
            self.disabled.retain(|b| *b != goal.bone);
            if self.clip.active {
                self.clip.mark(&goal);
            } else {
                self.graph.capture(&goal, &self.canonical_names);
                if let Some(index) = self.graph.binding_index(goal.bone, &self.canonical_names)
                    && let Some(binding) = self.graph.definition.bindings.get_mut(index)
                {
                    binding.enabled = true;
                }
                if let Some(binding) = self.bindings.iter_mut().find(|b| b.bone == goal.bone) {
                    binding.capture(&goal, self.canonical_names.get(goal.root).map(String::as_str));
                }
            }
            if let Some(old) = self.goals.iter_mut().find(|g| g.bone == goal.bone) {
                *old = goal;
            } else {
                self.goals.push(goal);
            }
        }
    }

    fn show_gizmo(&mut self, ctx: &egui::Context, palette: &Palette, proj: &Projector, frame: &Frame, rig: &Rig) -> bool {
        let modifiers = ctx.input(|i| i.modifiers);
        if (!self.poser.active
            && (self.clip.waiting_source() || self.graph.selected.is_some() || self.picking || self.prims.object_handles))
            || !self.gizmo_visible(modifiers)
            || modifiers.alt
            || self.drag.is_some()
            || self.selected.is_some_and(|bone| !self.bone_visible(&rig.canonical_names[bone]))
        {
            self.gizmo.cancel();
            return false;
        }
        let Some(bone) = self.selected else {
            return false;
        };
        let pole_limb = (self.control == Control::Pole)
            .then(|| limbs(rig).into_iter().find(|l| l.end == bone))
            .flatten();
        if self.control == Control::Pole && !self.limb_enabled(bone) {
            self.gizmo.cancel();
            return false;
        }
        let tool = self.active_tool(modifiers);
        let (_, rotation, _) = frame.joints[pole_limb.map_or(bone, |l| l.middle)].to_scale_rotation_translation();
        let pivot = if self.control == Control::Pole {
            if tool == Tool::Rotate {
                frame.joints[pole_limb.map_or(bone, |l| l.middle)].w_axis.truncate()
            } else {
                frame.model.transform_point3(self.control_position())
            }
        } else if tool == Tool::Rotate && self.mode == Mode::Ik && !self.pose_constrained(rig, bone) {
            frame.joints[bone].w_axis.truncate()
        } else {
            frame.joints[bone].transform_point3(self.contact)
        };
        let basis = if self.space == Space::World { Quat::IDENTITY } else { rotation };
        let Some(geometry) = Geometry::new(proj, pivot, basis, tool) else {
            return false;
        };
        let (_, model_rotation, _) = frame.model.to_scale_rotation_translation();
        let pose = Pose {
            position: frame.model.transform_point3(self.control_position()),
            // The rendered orientation may be only a fraction of the authored
            // goal at partial strength. Rotate the full goal, without a jump.
            rotation: if self.control == Control::Pole {
                Quat::IDENTITY
            } else {
                (model_rotation * self.rotation()).normalize()
            },
        };
        let point_hit = ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| self.point_owns_pointer(ctx, rig, p));
        let (owns, change) = self.gizmo.show(ctx, palette, geometry, pose, !point_hit);
        if let Some((pose, tool)) = change {
            if self.limb_controls
                && self.mode == Mode::Ik
                && self.pole.is_none()
                && let Some(limb) = limbs(rig).into_iter().find(|l| l.end == bone)
            {
                self.pole = Some(limb.pole(frame));
            }
            if tool == Tool::Move {
                self.move_control(frame.model.inverse().transform_point3(pose.position));
            } else if self.control == Control::Pole {
                self.begin_pole_orbit(frame, rig);
                self.rotate_pole(model_rotation.inverse() * pose.rotation * model_rotation);
            } else {
                self.set_rotation(model_rotation.inverse() * pose.rotation);
            }
            self.commit_manipulation(rig, frame, tool == Tool::Rotate);
        }
        owns
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: &Palette, world: &World, proj: &Projector) {
        if !self.open {
            return;
        }
        if self.poser.active {
            if world.teleporting {
                self.stop_editing();
            } else {
                self.show_poser(ctx, palette, world, proj);
            }
            return;
        }
        if let Some(frame) = self.editor_frame(world) {
            self.poll_object_data(world, &frame);
        }
        if self.library_loading_ui(ctx, palette) {
            return;
        }
        if self.library_start_ui(ctx, palette, world) {
            return;
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.drag = None;
            if !ctx.input(|i| i.pointer.primary_released()) {
                self.pole_orbit = None;
            }
        }
        if world.teleporting {
            self.poser.active = false;
            self.leave_clip();
            self.open = false;
            return;
        }
        let Some(frame) = self.editor_frame(world) else {
            return;
        };
        let rig = &world.avatar_lib.rig;
        if self.clip.active {
            self.capture_clip(world, &frame);
        } else {
            self.update_bindings(world, &frame);
            self.update_graph(world, &frame);
        }
        let names: Vec<_> = rig
            .canonical_names
            .iter()
            .enumerate()
            .map(|(joint, name)| (joint, name.as_str()))
            .collect();
        if self.selected.is_none() || self.refresh {
            self.select(
                self.selected
                    .unwrap_or_else(|| rig.names.get("mWristLeft").copied().unwrap_or(rig.pelvis)),
                rig,
                &frame,
            );
            self.refresh = false;
        }
        if self.graph.selected.is_none()
            && self.selected.is_some_and(|b| self.disabled.contains(&b))
            && self.drag.is_none()
            && !self.gizmo.dragging()
        {
            self.released_pose(&frame);
        }
        self.sync_clip_selection(&frame);
        self.sync_authored_selection(&frame);
        self.sync_prim_draft();
        // World overlays stay in Background, like the build gizmos. Middle
        // joint areas can rise above a floater after being clicked.
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("ik-skeleton")));
        let segments = self.skeleton_segments(world, proj, &frame);
        self.draw_segments(&painter, palette, &segments);
        if let Some(bone) = self.selected
            && self.visibility.visible(&rig.canonical_names[bone])
            && let Some((point, _)) = proj.project(frame.joints[bone].transform_point3(self.contact))
        {
            painter.circle_stroke(point, 7.0, Stroke::new(2.0, palette.teal));
        }
        let graph_owns = !self.clip.active && self.graph_overlay(ctx, palette, world, proj, &frame);
        // A selected virtual bone owns the shared manipulator. Do not let the
        // real-bone renderer cancel its in-progress modifier drag afterwards.
        let gizmo_owns = if self.graph.selected.is_some() {
            graph_owns
        } else {
            self.show_gizmo(ctx, palette, proj, &frame, rig) || graph_owns
        };
        let selected_pole = self.control == Control::Pole;
        if !selected_pole {
            self.show_poles(ctx, palette, proj, &frame, rig, gizmo_owns);
        }
        let limb_list = if self.limb_controls {
            limbs(rig).into_iter().filter(|l| self.limb_enabled(l.end)).collect()
        } else {
            Vec::new()
        };
        if (!self.picking || self.links.bone_pick)
            && (!self.prims.object_handles || self.links.bone_pick)
            && !ctx.input(|i| i.modifiers.alt)
        {
            let chosen = self.selected;
            let handles = names
                .iter()
                .filter(|n| chosen != Some(n.0) && !limb_list.iter().any(|l| l.end == n.0))
                .chain(
                    names
                        .iter()
                        .filter(|n| chosen != Some(n.0) && limb_list.iter().any(|l| l.end == n.0)),
                )
                .chain(names.iter().filter(|n| chosen == Some(n.0)));
            for &(joint, name) in handles {
                if !self.visibility.visible(name) {
                    continue;
                }
                let selected = self.selected == Some(joint);
                let limb = limb_list.iter().find(|l| l.end == joint);
                let point = if self.links.bone_pick {
                    frame.joints[joint].w_axis.truncate()
                } else if selected {
                    frame.joints[joint].transform_point3(self.contact)
                } else if limb.is_some() {
                    let contact = self.goals.iter().find(|g| g.bone == joint).map_or(Vec3::ZERO, |g| g.contact);
                    frame.joints[joint].transform_point3(contact)
                } else {
                    frame.joints[joint].w_axis.truncate()
                };
                let Some((anchor, _)) = proj.project(point) else {
                    continue;
                };
                let at = anchor;
                let half = if limb.is_some() { 12.0 } else { 6.0 };
                let response = egui::Area::new(egui::Id::unique(("ik-joint", joint)))
                    .fixed_pos(at - egui::vec2(half, half))
                    .order(egui::Order::Background)
                    .movable(false)
                    .interactable(!gizmo_owns && !self.clip.waiting_source())
                    .show(ctx, |ui| {
                        let sense = if self.links.bone_pick {
                            Sense::click()
                        } else if gizmo_owns || self.clip.waiting_source() {
                            Sense::hover()
                        } else {
                            Sense::click_and_drag()
                        };
                        let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(half * 2.0), sense);
                        if let Some(limb) = limb {
                            ui.painter().circle_stroke(rect.center(), 10.0, Stroke::new(3.0, palette.teal));
                            ui.painter().text(
                                rect.center() + egui::vec2(11.0, -10.0),
                                egui::Align2::LEFT_BOTTOM,
                                aurora_i18n::text(limb.label),
                                egui::FontId::proportional(12.0),
                                palette.teal,
                            );
                        }
                        ui.painter().circle_filled(
                            rect.center(),
                            if selected { 4.5 } else { 2.5 },
                            if selected { palette.teal } else { palette.ink },
                        );
                        response
                    })
                    .inner;
                if self.links.bone_pick && response.clicked() {
                    self.picked_bone(
                        world,
                        &frame,
                        crate::world::rig_graph::Target::Bone {
                            avatar: Uuid::nil(),
                            bone: name.to_owned(),
                        },
                    );
                    response.on_hover_text(name);
                    continue;
                }
                if response.clicked() || response.drag_started() {
                    self.select(joint, rig, &frame);
                    if self.mode == Mode::Ik
                        && self.pole.is_none()
                        && let Some(limb) = limb
                    {
                        self.pole = Some(limb.pole(&frame));
                    }
                }
                if self.point_drag_enabled(ctx.input(|i| i.modifiers))
                    && response.dragged()
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    let rotate = self.active_tool(ctx.input(|i| i.modifiers)) == Tool::Rotate;
                    let changed_mode = self.drag.is_some_and(|drag| matches!(drag, Drag::Rotate { .. }) != rotate);
                    if self.drag.is_none() || changed_mode {
                        let origin = if response.drag_started() {
                            ctx.input(|i| i.pointer.press_origin()).unwrap_or(pointer)
                        } else {
                            pointer
                        };
                        self.start_drag(proj, &frame, origin, rotate);
                    }
                    self.drag_to(proj, &frame, rig, pointer);
                }
                if response.drag_stopped() {
                    self.drag = None;
                }
                response.on_hover_text(name);
            }
        }
        if selected_pole {
            self.show_poles(ctx, palette, proj, &frame, rig, gizmo_owns);
        }
        self.pick_segment(ctx, palette, world, &frame, &segments, gizmo_owns);
        let mut open = self.open;
        let mut floater = Floater::new(
            "ik-edit",
            if self.clip.active {
                aurora_i18n::tr!("anim-edit-title")
            } else {
                aurora_i18n::tr!("ik-edit-title")
            },
            egui::pos2(24.0, 85.0),
            egui::vec2(530.0, 785.0),
        );
        floater.min_size.x = 530.0;
        floater.show(ctx, palette, &mut open, |ui| {
            if self.clip.active && ui.button(aurora_i18n::tr!("anim-edit-return")).clicked() {
                self.leave_clip();
            }
            if !self.clip.active {
                self.library_title_ui(ui);
            }
            self.project_tabs_ui(ui, palette);
            self.link_pick_status_ui(ui, palette, world);
            let footer = if self.clip.active {
                150.0
            } else if self.links.footer_height > 0.0 {
                self.links.footer_height + ui.spacing().item_spacing.y
            } else {
                190.0
            };
            let mut body = egui::ScrollArea::vertical()
                .id_salt(("ik-editor-body", self.tab))
                .auto_shrink([false, self.clip.active])
                .max_height((ui.available_height() - footer).max(60.0));
            if self.links.reveal_offsets {
                body = body.vertical_scroll_offset(110.0);
            }
            body.show(ui, |ui| {
                if self.clip.waiting_source() {
                    ui.label(aurora_i18n::tr!("anim-edit-building"));
                    return;
                }
                match self.tab {
                    5 => self.library_browser_ui(ui, palette, world),
                    3 => self.visibility_ui(ui, palette, world, &frame),
                    2 => {
                        if !self.clip.active {
                            self.clip_entry_ui(ui, palette, world, &frame);
                        }
                        if self.clip.active {
                            self.clip_ui(ui, palette, world, &frame);
                        } else {
                            self.ensure_profiles(world, &frame);
                            let mut profile = self.graph.profile.unwrap_or(0);
                            ui.horizontal(|ui| {
                                ui.strong(aurora_i18n::tr!("ik-profile-title"));
                                let width = ui.available_width().max(80.0);
                                self.profile_selector_ui(ui, &mut profile, "ik-animation-profile", width);
                            });
                            if self.graph.profile != Some(profile) {
                                self.switch_profile(profile, world, &frame);
                            }
                            self.animations.show(ui, palette, &world.inventory);
                        }
                    }
                    0 => {
                        if !self.picking {
                            self.manipulation_ui(ui, palette);
                            self.pose_ui(ui, palette, world, &frame);
                        }
                    }
                    4 => {
                        if !self.picking {
                            self.nodes_ui(ui, palette, world, &frame);
                        }
                    }
                    _ => self.links_ui(ui, palette, world, &frame),
                }
            });
            if self.clip.active {
                self.clip_export_ui(ui, palette);
            } else {
                // Measure once per frame so wrapping, diagnostics and font/window
                // changes reserve their real height without replaying actions.
                let footer = ui.scope(|ui| self.footer_ui(ui, palette, world, &frame));
                if (self.links.footer_height - footer.response.rect.height()).abs() > 0.5 {
                    self.links.footer_height = footer.response.rect.height();
                    ctx.request_repaint();
                }
            }
        });
        self.open = open;
        if !self.open {
            self.leave_clip();
            self.cancel_link_pick();
            self.drag = None;
            self.gizmo.cancel();
        }
    }
}

fn vector_fields(ui: &mut egui::Ui, value: &mut Vec3, speed: f64) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for axis in 0..3 {
            ui.label(["X", "Y", "Z"][axis]);
            changed |= ui
                .add(egui::DragValue::new(&mut value[axis]).speed(speed).max_decimals(6))
                .changed();
        }
    });
    changed
}

#[cfg(test)]
mod tests;
