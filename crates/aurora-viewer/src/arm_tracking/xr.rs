//! Position-only neutral calibration. Lost samples never retain an IK target.
use super::Targets;
use aurora_render::{ControllerPoses, GripState, stereo_view::RigMode};
use glam::{Mat4, Vec3};

#[derive(Clone, Copy)]
pub struct AvatarFrame {
    pub model: Mat4,
    pub origin: Option<u64>,
    pub revision: u64,
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub enum Status {
    #[default]
    Unavailable,
    Inactive,
    Lost,
    NeedsCalibration,
    Ready,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "vr-arms-xr-unavailable",
            Self::Inactive => "vr-arms-xr-inactive",
            Self::Lost => "vr-arms-xr-lost",
            Self::NeedsCalibration => "vr-arms-xr-needs-calibration",
            Self::Ready => "vr-arms-xr-ready",
        }
    }
}

#[derive(Clone, Copy)]
struct Neutral {
    // Both points are in the same stable LOCAL tracking space, never avatar space.
    grip: Vec3,
    wrist: Vec3,
}
#[derive(Default)]
pub struct Tracking {
    neutral: [Option<Neutral>; 2],
    key: Option<(u64, Option<u64>, u64, RigMode)>,
    buttons: [u64; 2],
    was_enabled: bool,
    pub status: [Status; 2],
}
impl Tracking {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn release_hand(&mut self, i: usize) {
        self.neutral[i] = None;
        if self.status[i] == Status::Ready {
            self.status[i] = Status::NeedsCalibration;
        }
    }
    pub fn step(
        &mut self,
        sample: Option<ControllerPoses>,
        avatar: AvatarFrame,
        wrists: [Option<Vec3>; 2],
        enabled: &mut bool,
        calibrate: bool,
        scale: f32,
    ) -> Targets {
        let sample = sample.filter(|p| p.matches(avatar.origin, avatar.revision));
        let Some(sample) = sample.filter(|p| {
            avatar.model.is_finite()
                && avatar.model.determinant().abs() > 1e-8
                && p.tracking_to_world.is_finite()
                && p.tracking_to_world.determinant().abs() > 1e-8
                && scale.is_finite()
                && (0.25..=2.0).contains(&scale)
        }) else {
            self.reset();
            return [None; 2];
        };
        let key = (sample.generation, sample.origin, sample.revision, sample.mode);
        let mut requested = calibrate;
        if self.key != Some(key) {
            self.reset();
            self.key = Some(key);
        } else {
            if sample.buttons[1].wrapping_sub(self.buttons[1]) % 2 == 1 {
                *enabled = !*enabled;
            }
            requested |= sample.buttons[0] != self.buttons[0];
        }
        self.buttons = sample.buttons;
        if self.was_enabled != *enabled || !*enabled {
            self.neutral = [None; 2];
        }
        self.was_enabled = *enabled;
        let inverse = avatar.model.inverse();
        let world_to_tracking = sample.tracking_to_world.inverse();
        let avatar_to_tracking = world_to_tracking * avatar.model;
        let tracking_to_avatar = inverse * sample.tracking_to_world;
        std::array::from_fn(|i| {
            let grip = match sample.hands[i] {
                GripState::Tracked(p) if p.is_finite() => world_to_tracking.transform_point3(p),
                state => {
                    self.neutral[i] = None;
                    self.status[i] = match state {
                        GripState::Inactive => Status::Inactive,
                        GripState::Unavailable => Status::Unavailable,
                        _ => Status::Lost,
                    };
                    return None;
                }
            };
            if !grip.is_finite() {
                self.release_hand(i);
                self.status[i] = Status::Lost;
                return None;
            }
            if *enabled && requested {
                self.neutral[i] = wrists[i].filter(|p| p.is_finite()).map(|wrist| Neutral {
                    grip,
                    wrist: avatar_to_tracking.transform_point3(wrist),
                });
            }
            self.status[i] = Status::NeedsCalibration;
            let neutral = self.neutral[i]?;
            // Convert only the final point into the current root frame. Comparing
            // grips in two different avatar frames rotates the calibration offset
            // when the body turns independently of the headset/camera rig.
            let target = tracking_to_avatar.transform_point3(neutral.wrist + (grip - neutral.grip) * scale);
            if !target.is_finite() {
                self.release_hand(i);
                return None;
            }
            self.status[i] = Status::Ready;
            Some(target)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn assert_targets(actual: Targets, expected: Targets) {
        for (a, b) in actual.into_iter().zip(expected) {
            match (a, b) {
                (Some(a), Some(b)) => assert!(a.abs_diff_eq(b, 2e-5), "{a:?} != {b:?}"),
                (a, b) => assert_eq!(a, b),
            }
        }
    }
    fn sample() -> ControllerPoses {
        ControllerPoses {
            hands: [GripState::Tracked(Vec3::new(1.0, 2.0, 3.0)); 2],
            tracking_to_world: Mat4::IDENTITY,
            buttons: [0; 2],
            generation: 1,
            origin: Some(7),
            revision: 3,
            mode: RigMode::FirstPerson,
            sampled: Instant::now(),
        }
    }
    fn avatar() -> AvatarFrame {
        AvatarFrame {
            model: Mat4::from_rotation_translation(glam::Quat::from_rotation_z(0.7), Vec3::new(10.0, 20.0, 1.0)),
            origin: Some(7),
            revision: 3,
        }
    }
    #[test]
    fn xr_arms_neutral_survives_independent_root_turn_and_common_rig_motion() {
        let capture = avatar();
        let wrists = [Some(Vec3::new(0.2, 0.3, 1.0)), Some(Vec3::new(0.2, -0.3, 1.0))];
        for angle in [0.0, 1.57, -1.57, std::f32::consts::PI] {
            let mut tracking = Tracking::default();
            let mut enabled = true;
            let mut frame = sample();
            assert_targets(tracking.step(Some(frame), capture, wrists, &mut enabled, true, 1.5), wrists);
            let changed = AvatarFrame {
                model: Mat4::from_rotation_translation(glam::Quat::from_rotation_z(angle), Vec3::new(10.1, 20.0, 1.0)),
                ..capture
            };
            let delta = Vec3::new(0.0, 0.0, 0.03);
            frame.hands[0] = GripState::Tracked(Vec3::new(1.0, 2.0, 3.0) + delta);
            let expected = std::array::from_fn(|i| {
                wrists[i].map(|p| {
                    changed
                        .model
                        .inverse()
                        .transform_point3(capture.model.transform_point3(p) + if i == 0 { delta * 1.5 } else { Vec3::ZERO })
                })
            });
            assert_targets(tracking.step(Some(frame), changed, wrists, &mut enabled, false, 1.5), expected);

            // Artificial rig motion carries its calibration with it, even at non-unit scale.
            let motion = Mat4::from_rotation_translation(glam::Quat::from_rotation_z(-0.4), Vec3::new(0.4, -0.2, 0.1));
            frame.tracking_to_world = motion;
            for hand in &mut frame.hands {
                if let GripState::Tracked(p) = hand {
                    *p = motion.transform_point3(*p);
                }
            }
            let moved = AvatarFrame {
                model: motion * changed.model,
                ..changed
            };
            assert_targets(tracking.step(Some(frame), moved, wrists, &mut enabled, false, 1.5), expected);
            frame.tracking_to_world = Mat4::ZERO;
            assert_eq!(tracking.step(Some(frame), moved, wrists, &mut enabled, false, 1.5), [None; 2]);
        }
    }

    #[test]
    fn xr_arms_local_grips_through_eye_anchor_and_bento_ik_preserve_axes_after_head_turn() {
        use super::super::{CHAINS, apply};
        use crate::scene::avatar::AvatarLibrary;
        use aurora_render::stereo_view::{Anchor, Pose};
        use glam::Quat;
        let library = AvatarLibrary::load();
        let rig = &library.rig;
        let mut rotations = rig.local_rot.clone();
        for names in CHAINS {
            rotations[rig.names[names[1]]] *= Quat::from_rotation_x(0.7);
        }
        let rest = rig.world_with(&rotations, Vec3::ZERO, &rig.local_pos, &rig.scale);
        let wrists = CHAINS.map(|names| Some(rest[rig.names[names[2]]].w_axis.truncate()));
        for yaw in [0.0, std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
            let body = AvatarFrame {
                model: Mat4::from_rotation_translation(Quat::from_rotation_z(yaw), Vec3::new(2.0, 3.0, 0.0)),
                ..avatar()
            };
            let view = glam::camera::rh::view::look_to_mat4(Vec3::new(0.0, 0.0, 2.0), Quat::from_rotation_z(yaw) * Vec3::X, Vec3::Z);
            let head = Pose {
                position: Vec3::new(0.0, 1.6, 0.0),
                orientation: Quat::IDENTITY,
            };
            for hand in 0..2 {
                for (delta, expected_delta) in [
                    (Vec3::Y, Vec3::Z),
                    (Vec3::NEG_Z, Vec3::X),
                    (
                        if hand == 0 { Vec3::NEG_X } else { Vec3::X },
                        if hand == 0 { Vec3::Y } else { Vec3::NEG_Y },
                    ),
                ] {
                    let mut anchor = Anchor::new(view, head).expect("anchor");
                    anchor.follow_head(view, head, RigMode::Avatar).expect("initial mode");
                    let grips = [Vec3::new(-0.2, 1.1, -0.3), Vec3::new(0.2, 1.1, -0.3)];
                    let mut frame = ControllerPoses {
                        hands: grips.map(|p| GripState::Tracked(anchor.tracking_position(p).expect("grip"))),
                        tracking_to_world: anchor.tracking_transform(),
                        mode: RigMode::Avatar,
                        ..sample()
                    };
                    let mut tracking = Tracking::default();
                    let mut enabled = true;
                    assert_targets(tracking.step(Some(frame), body, wrists, &mut enabled, true, 1.0), wrists);
                    anchor
                        .follow_head(
                            view,
                            Pose {
                                orientation: Quat::from_rotation_y(1.9) * Quat::from_rotation_x(0.4),
                                ..head
                            },
                            RigMode::Avatar,
                        )
                        .expect("head turn");
                    frame.tracking_to_world = anchor.tracking_transform();
                    frame.hands[hand] = GripState::Tracked(anchor.tracking_position(grips[hand] + delta * 0.015).expect("moved grip"));
                    let targets = tracking.step(Some(frame), body, wrists, &mut enabled, false, 1.0);
                    let mut expected = wrists;
                    expected[hand] = expected[hand].map(|p| p + expected_delta * 0.015);
                    assert_targets(targets, expected);
                    let solved = apply(rig, &rotations, &rig.local_pos, Vec3::ZERO, &rig.scale, targets).expect("IK");
                    let matrices = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
                    let wrist = rig.names[CHAINS[hand][2]];
                    assert!(matrices[wrist].w_axis.truncate().distance(expected[hand].expect("target")) < 0.004);
                }
            }
        }
    }
    #[test]
    fn xr_arms_neutral_conversion_is_independent_scaled_and_does_not_jump_on_recovery() {
        let mut tracking = Tracking::default();
        let mut enabled = true;
        let mut frame = sample();
        let wrists = [Some(Vec3::new(0.2, 0.3, 1.0)), Some(Vec3::new(0.2, -0.3, 1.0))];
        assert_eq!(tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.0), [None; 2]);
        assert_targets(tracking.step(Some(frame), avatar(), wrists, &mut enabled, true, 1.5), wrists);
        let delta = avatar().model.transform_vector3(Vec3::new(0.1, 0.0, 0.2));
        if let GripState::Tracked(p) = &mut frame.hands[0] {
            *p += delta;
        }
        let targets = tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.5);
        assert!(
            targets[0]
                .expect("left")
                .abs_diff_eq(wrists[0].expect("wrist") + Vec3::new(0.15, 0.0, 0.3), 1e-5)
        );
        assert_targets([None, targets[1]], [None, wrists[1]]);
        frame.hands[0] = GripState::Untracked;
        assert_targets(
            tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.5),
            [None, wrists[1]],
        );
        frame.hands[0] = GripState::Tracked(Vec3::splat(50.0));
        assert_targets(
            tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.5),
            [None, wrists[1]],
        );
        assert_targets(tracking.step(Some(frame), avatar(), wrists, &mut enabled, true, 1.5), wrists);
    }
    #[test]
    fn xr_arms_reject_stale_origin_revision_recenter_and_inactive_samples() {
        for invalid in [0, 1, 2, 3, 4] {
            let mut tracking = Tracking::default();
            let mut enabled = true;
            let mut frame = sample();
            let wrists = [Some(Vec3::Z); 2];
            tracking.step(Some(frame), avatar(), wrists, &mut enabled, true, 1.0);
            match invalid {
                0 => frame.sampled -= Duration::from_secs(1),
                1 => frame.origin = Some(8),
                2 => frame.revision += 1,
                3 => frame.generation += 1,
                _ => frame.hands = [GripState::Inactive; 2],
            }
            assert_eq!(tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.0), [None; 2]);
        }
    }
    #[test]
    fn xr_arms_button_counters_toggle_once_and_disable_releases_immediately() {
        let mut tracking = Tracking::default();
        let mut enabled = false;
        let mut frame = sample();
        let wrists = [Some(Vec3::Z); 2];
        tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.0);
        frame.buttons = [1, 1];
        assert_targets(tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.0), wrists);
        assert!(enabled);
        assert_targets(tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.0), wrists);
        assert!(enabled, "same frame cannot toggle twice");
        frame.buttons[1] = 2;
        assert_eq!(tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.0), [None; 2]);
        assert!(!enabled);
        frame.buttons[1] = 3;
        assert_eq!(
            tracking.step(Some(frame), avatar(), wrists, &mut enabled, false, 1.0),
            [None; 2],
            "enable needs new neutral calibration"
        );
        assert_eq!(tracking.step(None, avatar(), wrists, &mut enabled, false, 1.0), [None; 2]);
    }
}
