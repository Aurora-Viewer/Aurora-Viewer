//! Offline simulated hand input; stateless arm corrections reuse the production IK solver.

use crate::scene::{anim::Rig, ik};
use crate::theme::Palette;
use crate::ui::widgets::{self, Floater};
use crate::world::World;
use aurora_i18n::tr;
use glam::{Quat, Vec3};
pub mod live;
mod xr;

pub type Targets = [Option<Vec3>; 2];
pub(crate) const CHAINS: [[&str; 3]; 2] = [
    ["mShoulderLeft", "mElbowLeft", "mWristLeft"],
    ["mShoulderRight", "mElbowRight", "mWristRight"],
];

/// Corrections are output-only: no animation clock, held pose, or release cache is changed.
pub fn apply(rig: &Rig, rotations: &[Quat], positions: &[Vec3], pelvis: Vec3, scale: &[Vec3], targets: Targets) -> Option<ik::LocalPose> {
    if targets.iter().all(Option::is_none) {
        return None;
    }
    let mut pose = ik::LocalPose {
        rotations: rotations.to_vec(),
        positions: positions.to_vec(),
    };
    let mut changed = false;
    for (names, target) in CHAINS.iter().zip(targets) {
        let Some(position) = target.filter(|p| p.is_finite()) else {
            continue;
        };
        let [Some(root), Some(elbow), Some(bone)] = names.map(|name| rig.names.get(name).copied()) else {
            continue;
        };
        if rig.parent[elbow] != Some(root) || rig.parent[bone] != Some(elbow) {
            continue;
        }
        // Exactly the shoulder/elbow/wrist chain: never recruit the torso or stretch joints.
        changed |= ik::apply_goal(
            rig,
            &mut pose,
            pelvis,
            scale,
            &ik::Goal {
                follow: None,
                weight: 1.0,
                bone,
                root,
                position,
                rotation: None,
                contact: Vec3::ZERO,
                pole: None,
                mode: ik::Mode::Ik,
            },
        );
    }
    changed.then_some(pose)
}

pub struct Panel {
    live: bool,
    live_state: Option<live::State>,
    open: bool,
    enabled: bool,
    arms: [bool; 2],
    positions: [Vec3; 2],
    defaults: [Vec3; 2],
    avatar: Option<uuid::Uuid>,
    test: bool,
    use_xr: bool,
    xr: xr::Tracking,
    xr_targets: Targets,
    motion_scale: f32,
    calibrate: bool,
}

impl Panel {
    pub fn from_env() -> Option<Self> {
        let offline = std::env::var("AURORA_DEMO").as_deref() == Ok("1");
        let live = !offline && std::env::var("AURORA_LIVE_VR_ARMS").as_deref() == Ok("1");
        let mode = if live {
            "xr".to_owned()
        } else {
            std::env::var("AURORA_DEMO_VR_ARMS").ok()?
        };
        if !live && (!offline || !matches!(mode.as_str(), "1" | "test" | "xr")) {
            return None;
        }
        Some(Self {
            live,
            live_state: None,
            open: true,
            enabled: false,
            arms: [true; 2],
            positions: [Vec3::ZERO; 2],
            defaults: [Vec3::ZERO; 2],
            avatar: None,
            test: mode == "test",
            use_xr: mode == "xr",
            xr: xr::Tracking::default(),
            xr_targets: [None; 2],
            motion_scale: 1.0,
            calibrate: false,
        })
    }

    /// Shape, region, identity and pose-editor changes require a new explicit
    /// activation/capture. Never reuse measurements from a previous appearance.
    pub fn live_scope(&mut self, offline: bool, state: Option<live::State>) -> bool {
        if !self.live {
            return offline;
        }
        if self.live_state != state {
            self.live_state = state;
            self.enabled = false;
            self.avatar = None;
            self.calibrate = false;
            self.xr.reset();
            self.xr_targets = [None; 2];
        }
        state.is_some()
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    pub fn tick(&mut self, world: &World, frame: u64) {
        if world.objects.index_of_uuid(&world.agent_id).is_none() {
            self.avatar = None;
            self.enabled = false;
            return;
        }
        if self.avatar != Some(world.agent_id) {
            self.enabled = false;
            self.avatar = None;
            let Some(pose) = crate::ik_edit::Frame::new(world) else { return };
            let model = pose.model.inverse();
            for (i, names) in CHAINS.iter().enumerate() {
                let Some(joint) = world.avatar_lib.rig.names.get(names[2]).and_then(|j| pose.joints.get(*j)) else {
                    return;
                };
                self.defaults[i] = model.transform_point3(joint.w_axis.truncate());
            }
            if !self.defaults.iter().all(|p| p.is_finite()) {
                return;
            }
            self.positions = self.defaults;
            self.avatar = Some(world.agent_id);
            self.xr.reset();
        }
        // One opt-in, deterministic visual scenario; ordinary launches never auto-enable.
        if self.test {
            match frame {
                420 => {
                    self.enabled = true;
                    self.arms = [true, false];
                    self.positions[0] += Vec3::new(0.25, -0.15, 0.15);
                }
                520 => {
                    self.arms[1] = true;
                    self.positions[1] += Vec3::new(0.35, 0.15, -0.15);
                }
                700 => self.arms[0] = false,
                850 => self.enabled = false,
                _ => {}
            }
        }
    }

    pub fn targets(&self, offline: bool) -> Targets {
        std::array::from_fn(|i| {
            if !(offline && self.open && self.enabled && self.arms[i] && self.avatar.is_some()) {
                return None;
            }
            if self.use_xr {
                self.xr_targets[i]
            } else {
                self.positions[i].is_finite().then_some(self.positions[i])
            }
        })
    }

    pub fn update_xr(&mut self, world: &World, poses: Option<aurora_render::ControllerPoses>, revision: u64, offline: bool) {
        self.xr_targets = [None; 2];
        if !offline || !self.open || !self.use_xr {
            self.xr.reset();
            return;
        }
        let Some(model) = world.avatar_models.get(&world.agent_id).copied() else {
            self.xr.reset();
            return;
        };
        // Same palette reconstruction as IK Edit's Frame, but only the two wrists.
        // Capture the displayed pose at calibration, never feed solved wrists back continuously.
        let wrists = std::array::from_fn(|i| {
            let rig = &world.avatar_lib.rig;
            let joint = *rig.names.get(CHAINS[i][2])?;
            let pose = world.avatar_poses.get(&world.agent_id)?.get(joint)?;
            let p = (*pose * rig.default_world_inv[joint].inverse()).w_axis.truncate();
            p.is_finite().then_some(p)
        });
        self.xr_targets = self.xr.step(
            poses,
            xr::AvatarFrame {
                model,
                origin: world.main_region,
                revision,
            },
            wrists,
            &mut self.enabled,
            std::mem::take(&mut self.calibrate),
            self.motion_scale,
        );
        for i in 0..2 {
            if !self.arms[i] {
                self.xr.release_hand(i);
                self.xr_targets[i] = None;
            }
        }
    }

    pub fn xr_hint(&self) -> Option<String> {
        (self.open && self.use_xr).then(|| {
            tr!(
                "vr-arms-xr-hint",
                state = aurora_i18n::text(if self.enabled { "vr-arms-xr-on" } else { "vr-arms-xr-off" }).to_owned(),
                left = aurora_i18n::text(self.xr.status[0].label()).to_owned(),
                right = aurora_i18n::text(self.xr.status[1].label()).to_owned()
            )
            .to_owned()
        })
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: &Palette, offline: bool, blocked: bool) {
        if !offline {
            return;
        }
        let mut open = self.open;
        Floater::new(
            "demo-vr-arms",
            tr!("vr-arms-title"),
            egui::pos2(24.0, 75.0),
            egui::vec2(410.0, 590.0),
        )
        .show(ctx, palette, &mut open, |ui| {
            ui.label(tr!("vr-arms-help"));
            if self.live {
                ui.label(tr!("vr-arms-live-help"));
            }
            let source = self.use_xr;
            ui.add_enabled_ui(!self.live, |ui| {
                ui.horizontal(|ui| {
                    widgets::radio_value(ui, palette, &mut self.use_xr, false, tr!("vr-arms-source-desktop"));
                    widgets::radio_value(ui, palette, &mut self.use_xr, true, tr!("vr-arms-source-xr"));
                });
            });
            if source != self.use_xr {
                self.enabled = false;
                self.xr.reset();
                self.xr_targets = [None; 2];
                self.calibrate = false;
            }
            if blocked {
                ui.label(tr!("vr-arms-paused"));
            }
            ui.add_enabled_ui(self.avatar.is_some(), |ui| {
                widgets::switch_labeled(ui, palette, &mut self.enabled, tr!("vr-arms-enable"));
                if self.use_xr {
                    ui.label(tr!("vr-arms-xr-help"));
                    if ui
                        .add_enabled(self.enabled, egui::Button::new(tr!("vr-arms-xr-calibrate")))
                        .clicked()
                    {
                        self.calibrate = true;
                    }
                    ui.horizontal(|ui| {
                        ui.label(tr!("vr-arms-xr-scale"));
                        if ui
                            .add(egui::DragValue::new(&mut self.motion_scale).speed(0.01).range(0.25..=2.0))
                            .changed()
                        {
                            self.xr.reset();
                        }
                    });
                }
                for (i, label) in [tr!("vr-arms-left"), tr!("vr-arms-right")].into_iter().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            widgets::switch_labeled(ui, palette, &mut self.arms[i], label);
                            if ui.button(tr!("vr-arms-reset")).clicked() {
                                self.positions[i] = self.defaults[i];
                                self.xr.release_hand(i);
                            }
                        });
                        if self.use_xr {
                            ui.label(aurora_i18n::text(self.xr.status[i].label()));
                        }
                        let mut point = if self.use_xr {
                            self.xr_targets[i].unwrap_or(self.defaults[i])
                        } else {
                            self.positions[i]
                        };
                        ui.add_enabled_ui(!self.use_xr, |ui| {
                            egui::Grid::new("coordinates").num_columns(2).show(ui, |ui| {
                                for (index, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                                    ui.label(axis);
                                    ui.add(egui::DragValue::new(&mut point[index]).speed(0.01).range(-5.0..=5.0).suffix(" m"));
                                    ui.end_row();
                                }
                            });
                        });
                        if !self.use_xr {
                            self.positions[i] = point;
                        }
                    });
                }
            });
        });
        self.open = open;
        if !open {
            self.enabled = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;

    fn panel() -> Panel {
        Panel {
            live: false,
            live_state: None,
            open: true,
            enabled: false,
            arms: [true; 2],
            positions: [Vec3::X, Vec3::Y],
            defaults: [Vec3::ZERO; 2],
            avatar: Some(uuid::Uuid::from_u128(1)),
            test: false,
            use_xr: false,
            xr: xr::Tracking::default(),
            xr_targets: [None; 2],
            motion_scale: 1.0,
            calibrate: false,
        }
    }

    #[test]
    fn arm_tracking_targets_require_offline_open_enabled_valid_hands() {
        let mut panel = panel();
        assert_eq!(panel.targets(true), [None; 2]);
        panel.enabled = true;
        assert_eq!(panel.targets(false), [None; 2]);
        assert_eq!(panel.targets(true), [Some(Vec3::X), Some(Vec3::Y)]);
        panel.arms[0] = false;
        assert_eq!(panel.targets(true), [None, Some(Vec3::Y)]);
        panel.positions[1].x = f32::NAN;
        assert_eq!(panel.targets(true), [None; 2]);
        panel.positions[1] = Vec3::Y;
        panel.open = false;
        assert_eq!(panel.targets(true), [None; 2]);
    }

    #[test]
    fn arm_tracking_live_changes_release_and_require_explicit_activation() {
        let mut pilot = panel();
        pilot.live = true;
        pilot.use_xr = true;
        let state = live::State {
            avatar: uuid::Uuid::from_u128(1),
            region: 7,
            appearance: 1,
            skeleton: 2,
        };
        assert!(!pilot.live_scope(false, None));
        assert!(pilot.live_scope(false, Some(state)));
        assert!(!pilot.enabled);
        pilot.enabled = true;
        pilot.avatar = Some(state.avatar);
        pilot.xr_targets = [Some(Vec3::Z); 2];
        assert!(pilot.live_scope(false, Some(state)));
        assert_eq!(pilot.targets(true), [Some(Vec3::Z); 2]);
        for changed in [
            live::State {
                avatar: uuid::Uuid::from_u128(2),
                ..state
            },
            live::State { region: 8, ..state },
            live::State { appearance: 2, ..state },
            live::State { skeleton: 4, ..state },
        ] {
            pilot.enabled = true;
            pilot.calibrate = true;
            assert!(pilot.live_scope(false, Some(changed)));
            assert!(!pilot.enabled && !pilot.calibrate);
            assert_eq!(pilot.targets(true), [None; 2]);
            assert_eq!(pilot.avatar, None);
        }
        pilot.enabled = true;
        assert!(!pilot.live_scope(false, None));
        assert!(!pilot.enabled);
        assert!(pilot.live_scope(false, Some(state)));
        assert!(!pilot.enabled, "recovery cannot reactivate a previous session");
        let mut offline = panel();
        assert!(offline.live_scope(true, None));
        assert!(!offline.live_scope(false, Some(state)), "desktop fallback stays offline");
    }

    #[test]
    fn arm_tracking_preserves_shaped_lengths_unrelated_joints_and_inputs() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let mut positions = rig.local_pos.clone();
        let mut scale = rig.scale.clone();
        let mut authored = rig.local_rot.clone();
        for names in CHAINS {
            positions[rig.names[names[1]]] *= 1.15;
            positions[rig.names[names[2]]] *= 0.9;
            scale[rig.names[names[2]]] *= Vec3::new(0.8, 1.2, 1.1);
            authored[rig.names[names[0]]] *= Quat::from_rotation_z(0.25);
            authored[rig.names[names[1]]] *= Quat::from_rotation_x(0.4);
        }
        let pelvis = Vec3::new(0.02, -0.03, 0.1);
        let expected = rig.world_with(&authored, pelvis, &positions, &scale);
        let targets = CHAINS.map(|names| Some(expected[rig.names[names[2]]].w_axis.truncate()));
        let base = rig.world_with(&rig.local_rot, pelvis, &positions, &scale);
        let result = apply(rig, &rig.local_rot, &positions, pelvis, &scale, targets).expect("two valid arms");
        let solved = rig.world_with(&result.rotations, pelvis, &result.positions, &scale);
        assert_eq!(result.positions, positions, "IK must never translate or stretch a joint");
        for (i, names) in CHAINS.iter().enumerate() {
            let [a, b, c] = names.map(|n| rig.names[n]);
            assert!(solved[c].w_axis.truncate().distance(targets[i].expect("target")) < 0.004);
            for (u, v) in [(a, b), (b, c)] {
                assert!(
                    (solved[u].w_axis.truncate().distance(solved[v].w_axis.truncate())
                        - base[u].w_axis.truncate().distance(base[v].w_axis.truncate()))
                    .abs()
                        < 0.0001
                );
            }
            assert_eq!(
                result.rotations[c], rig.local_rot[c],
                "position-only wrist retains its animated articulation"
            );
        }
        for j in 0..rig.len() {
            if !CHAINS.iter().any(|names| names[..2].iter().any(|n| rig.names[*n] == j)) {
                assert_eq!(result.rotations[j], rig.local_rot[j]);
            }
        }
        let right = apply(rig, &rig.local_rot, &positions, pelvis, &scale, [Some(Vec3::NAN), targets[1]]).expect("valid right arm");
        for n in CHAINS[0] {
            assert_eq!(right.rotations[rig.names[n]], rig.local_rot[rig.names[n]]);
        }
        let far = apply(rig, &rig.local_rot, &positions, pelvis, &scale, [Some(Vec3::splat(50.0)), None]).expect("bounded unreachable arm");
        assert_eq!(far.positions, positions);
        assert!(far.rotations.iter().all(|q| q.is_finite()));
        assert!(apply(rig, &rig.local_rot, &positions, pelvis, &scale, [None; 2]).is_none());
    }
}
