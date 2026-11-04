//! Camera anchoring and asymmetric infinite reverse-Z projection for stereo output.
//! Runtime poses use OpenXR's right-handed, Y-up camera coordinates. The captured
//! desktop inverse-view matrix maps these into Aurora's world coordinates.
use glam::{Mat4, Quat, Vec3, Vec4};

#[derive(Debug, Clone, Copy)]
pub struct Pose {
    pub position: Vec3,
    pub orientation: Quat,
}
impl Pose {
    fn matrix(self) -> Option<Mat4> {
        let length = self.orientation.length_squared();
        (self.position.is_finite() && self.orientation.is_finite() && length.is_finite() && length > 1e-8)
            .then(|| Mat4::from_rotation_translation(self.orientation.normalize(), self.position))
    }
}

/// Per-eye angular bounds in radians, supplied by the runtime without symmetrizing.
#[derive(Debug, Clone, Copy)]
pub struct Fov {
    pub left: f32,
    pub right: f32,
    pub down: f32,
    pub up: f32,
}
impl Fov {
    pub fn projection(self, near: f32) -> Option<Mat4> {
        let angles = [self.left, self.right, self.down, self.up];
        if !near.is_finite()
            || near <= 0.0
            || angles.iter().any(|a| !a.is_finite() || a.abs() >= std::f32::consts::FRAC_PI_2)
            || self.left >= self.right
            || self.down >= self.up
        {
            return None;
        }
        let [l, r, d, u] = angles.map(f32::tan);
        Some(Mat4::from_cols(
            Vec4::new(2.0 / (r - l), 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 / (u - d), 0.0, 0.0),
            Vec4::new((r + l) / (r - l), (u + d) / (u - d), 0.0, -1.0),
            Vec4::new(0.0, 0.0, near, 0.0),
        ))
    }
}

/// Artificial camera motion differs between a head pivot and an avatar orbit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RigMode {
    FirstPerson,
    Avatar,
    Flycam,
}

/// Headset reference captured on entry, with a level world rig updated for locomotion.
/// Runtime recentering requires a new headset reference.
#[derive(Debug, Clone, Copy)]
pub struct Anchor {
    tracking: Mat4,
    rig: Mat4,
    /// World translation accumulated only by artificial rig changes.
    correction: Vec3,
    mode: Option<RigMode>,
}
#[derive(Debug, Clone, Copy)]
pub struct EyeView {
    pub view: Mat4,
    pub projection: Mat4,
    pub position: Vec3,
}
impl Anchor {
    pub fn new(desktop_view: Mat4, initial_head: Pose) -> Option<Self> {
        let det = desktop_view.determinant();
        if !desktop_view.is_finite() || !det.is_finite() || det.abs() < 1e-8 {
            return None;
        }
        let desktop_world = desktop_view.inverse();
        let forward = desktop_world.transform_vector3(Vec3::NEG_Z);
        let right = desktop_world.transform_vector3(Vec3::X);
        // Preserve heading and position, but start level regardless of desktop
        // pitch/roll. At a vertical look direction, the right axis retains yaw.
        let horizontal = Vec3::new(forward.x, forward.y, 0.0);
        let fallback = Vec3::Z.cross(Vec3::new(right.x, right.y, 0.0)).normalize_or(Vec3::Y);
        let heading = if horizontal.length_squared() > 1e-8 {
            horizontal.normalize()
        } else {
            fallback
        };
        let level_view = glam::camera::rh::view::look_to_mat4(desktop_world.w_axis.truncate(), heading, Vec3::Z);
        // Do not cancel the entry pose's pitch/roll: the headset may be resting
        // tilted when VR starts. Gravity defines level even after it is put on.
        let initial = initial_head.matrix()?;
        let initial_forward = initial.transform_vector3(Vec3::NEG_Z);
        let initial_right = initial.transform_vector3(Vec3::X);
        let flat = Vec3::new(initial_forward.x, 0.0, initial_forward.z);
        let fallback = Vec3::Y
            .cross(Vec3::new(initial_right.x, 0.0, initial_right.z))
            .normalize_or(Vec3::NEG_Z);
        let yaw = if flat.length_squared() > 1e-8 { flat.normalize() } else { fallback };
        let initial_level_view = glam::camera::rh::view::look_to_mat4(initial_head.position, yaw, Vec3::Y);
        Some(Self {
            tracking: initial_level_view,
            rig: level_view.inverse(),
            correction: Vec3::ZERO,
            mode: None,
        })
    }
    /// Follow artificial translation/yaw without cancelling physical head motion.
    pub fn follow(&mut self, view: Mat4) -> Option<()> {
        let determinant = view.determinant();
        if !view.is_finite() || !determinant.is_finite() || determinant.abs() < 1e-8 {
            return None;
        }
        let world = view.inverse();
        let forward = world.transform_vector3(Vec3::NEG_Z);
        let heading = Vec3::new(forward.x, forward.y, 0.0).normalize_or(Vec3::Y);
        let position = world.w_axis.truncate();
        if !position.is_finite() || !heading.is_finite() {
            return None;
        }
        self.rig = glam::camera::rh::view::look_to_mat4(position, heading, Vec3::Z).inverse();
        Some(())
    }

    /// Relocate the cyclopean head, then apply each eye's unchanged relative pose.
    /// Free rigs turn at the tracked head; Avatar offsets turn with its rig.
    pub fn follow_head(&mut self, view: Mat4, head: Pose, mode: RigMode) -> Option<()> {
        head.matrix()?;
        let tracked_head = self.tracking.transform_point3(head.position);
        if !tracked_head.is_finite() {
            return None;
        }
        let old_rig = self.rig;
        let mut next = *self;
        next.follow(view)?;
        let changed_mode = self.mode.is_some_and(|previous| previous != mode);
        if !changed_mode && next.rig == old_rig {
            // No artificial move: retain the correction bit-for-bit. Recomputing
            // it from a moving tracked head would introduce cancellation jitter.
            next.mode = Some(mode);
            *self = next;
            return Some(());
        }
        if changed_mode && !matches!(mode, RigMode::Flycam) {
            // Returning to the avatar/eyes chooses a new viewpoint. A previous
            // dolly/turn correction belongs to the old rig, not to room-scale lean.
            // Use the tracked height at the mode switch. The startup
            // reference may have been sampled before the headset was lifted.
            next.correction = Vec3::Z * -next.rig.transform_vector3(tracked_head).z;
        } else if matches!(mode, RigMode::Avatar) {
            // The tracked head and its offset orbit with the Avatar rig.
            next.correction = next.rig.transform_vector3(old_rig.inverse().transform_vector3(self.correction));
        } else {
            next.correction = self.correction + (old_rig.transform_vector3(tracked_head) - next.rig.transform_vector3(tracked_head));
        }
        if !next.correction.is_finite() {
            return None;
        }
        next.mode = Some(mode);
        *self = next;
        Some(())
    }

    pub fn head_position(&self, pose: Pose) -> Option<Vec3> {
        pose.matrix()?;
        let position = (self.rig * self.tracking).transform_point3(pose.position) + self.correction;
        position.is_finite().then_some(position)
    }

    /// Physical VIEW-space head orientation after entry-yaw cancellation.
    pub fn relative_head_rotation(self, pose: Pose) -> Option<Quat> {
        Some(Quat::from_mat4(&(self.tracking * pose.matrix()?)).normalize())
    }

    pub fn eye(self, pose: Pose, fov: Fov, near: f32) -> Option<EyeView> {
        let world = Mat4::from_translation(self.correction) * self.rig * self.tracking * pose.matrix()?;
        Some(EyeView {
            view: world.inverse(),
            projection: fov.projection(near)?,
            position: world.w_axis.truncate(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pose(position: Vec3) -> Pose {
        Pose {
            position,
            orientation: Quat::IDENTITY,
        }
    }
    fn fov() -> Fov {
        Fov {
            left: -0.6,
            right: 0.8,
            down: -0.5,
            up: 0.7,
        }
    }
    fn rig_view(position: Vec3, yaw: f32) -> Mat4 {
        glam::camera::rh::view::look_to_mat4(position, Vec3::new(yaw.cos(), yaw.sin(), 0.0), Vec3::Z)
    }
    fn eye_pose(head: Pose, offset: f32, cant: f32) -> Pose {
        Pose {
            position: head.position + head.orientation * Vec3::X * offset,
            orientation: head.orientation * Quat::from_rotation_y(cant),
        }
    }

    #[test]
    fn vr_artificial_yaw_pivots_at_current_head_preserving_lean_and_ipd() {
        let initial = Pose {
            position: Vec3::new(0.8, 1.65, -0.45),
            orientation: Quat::from_rotation_y(0.85),
        };
        let head = Pose {
            position: initial.position + initial.orientation * Vec3::new(0.28, -0.07, -0.12),
            orientation: initial.orientation * Quat::from_rotation_x(0.2),
        };
        let position = Vec3::new(20.0, 30.0, 2.0);
        let mut anchor = Anchor::new(rig_view(position, 0.3), initial).expect("anchor");
        anchor
            .follow_head(rig_view(position, 0.3), head, RigMode::FirstPerson)
            .expect("lean");
        let before = anchor.eye(head, fov(), 0.1).expect("head").position;
        for i in 1..=30 {
            anchor
                .follow_head(rig_view(position, 0.3 + i as f32 * 0.04), head, RigMode::FirstPerson)
                .expect("turn");
            let left = anchor.eye(eye_pose(head, -0.032, 0.09), fov(), 0.1).expect("left");
            let right = anchor.eye(eye_pose(head, 0.032, -0.09), fov(), 0.1).expect("right");
            assert!(
                ((left.position + right.position) * 0.5).distance(before) < 2e-5,
                "artificial turn translated the current head"
            );
            assert!((left.position.distance(right.position) - 0.064).abs() < 1e-5);
        }
        let delta = Vec3::new(0.4, -0.2, 0.1);
        anchor
            .follow_head(rig_view(position + delta, 1.9), head, RigMode::FirstPerson)
            .expect("turn and translate");
        let translated = anchor.eye(head, fov(), 0.1).expect("head").position;
        assert!(translated.abs_diff_eq(before + delta, 2e-5));
        let leaned = Pose {
            position: head.position + Vec3::X * 0.1,
            ..head
        };
        anchor
            .follow_head(rig_view(position + delta, 1.9), leaned, RigMode::FirstPerson)
            .expect("physical lean");
        let after = anchor.eye(leaned, fov(), 0.1).expect("lean").position;
        assert!(
            (after.distance(translated) - 0.1).abs() < 2e-5,
            "physical lean must remain one-to-one"
        );
    }

    #[test]
    fn vr_first_person_entry_uses_height_after_the_headset_is_picked_up() {
        let floor = pose(Vec3::ZERO);
        let raised = pose(Vec3::Y * 1.6);
        let view = rig_view(Vec3::new(20.0, 30.0, 2.0), 0.3);
        let mut anchor = Anchor::new(view, floor).expect("anchor");
        anchor.follow_head(view, floor, RigMode::Avatar).expect("startup");
        anchor.follow_head(view, raised, RigMode::FirstPerson).expect("first person");
        assert!((anchor.head_position(raised).expect("head").z - 2.0).abs() < 1e-5);
        let leaned = pose(Vec3::Y * 1.8);
        anchor.follow_head(view, leaned, RigMode::FirstPerson).expect("tracking");
        assert!((anchor.head_position(leaned).expect("head").z - 2.2).abs() < 1e-5);
    }

    #[test]
    fn vr_avatar_entry_matches_normal_height_then_tracks_physically_one_to_one() {
        let initial = Pose {
            position: Vec3::new(0.8, 1.65, -0.4),
            orientation: Quat::from_rotation_y(0.9),
        };
        let head = Pose {
            position: initial.position + initial.orientation * Vec3::new(0.3, 0.4, -0.1),
            orientation: initial.orientation,
        };
        let first = rig_view(Vec3::new(20.0, 30.0, 2.0), 0.3);
        let mut anchor = Anchor::new(first, initial).expect("anchor");
        anchor.follow_head(first, head, RigMode::FirstPerson).expect("first");
        let avatar_position = Vec3::new(17.0, 29.0, 4.0);
        let avatar_view = rig_view(avatar_position, 0.3);
        let mode = RigMode::Avatar;
        anchor.follow_head(avatar_view, head, mode).expect("avatar");
        let before = anchor.head_position(head).expect("head");
        assert!(
            (before.z - avatar_position.z).abs() < 1e-5,
            "actual head starts at normal camera height"
        );
        let moved = Pose {
            position: head.position + Vec3::new(0.1, 0.2, -0.3),
            ..head
        };
        anchor.follow_head(avatar_view, moved, mode).expect("physical");
        let actual = anchor.head_position(moved).expect("head") - before;
        assert!((actual.length() - Vec3::new(0.1, 0.2, -0.3).length()).abs() < 1e-5);
        let shift = Vec3::new(0.4, 0.2, 0.0);
        let before = anchor.head_position(moved).expect("head");
        anchor
            .follow_head(rig_view(avatar_position + shift, 0.3), moved, mode)
            .expect("horizontal zoom/pan");
        assert!((anchor.head_position(moved).expect("head") - before).abs_diff_eq(shift, 2e-5));
        let before = anchor.head_position(moved).expect("head");
        anchor
            .follow_head(rig_view(avatar_position + shift, 0.3), moved, RigMode::Flycam)
            .expect("flycam");
        assert!(anchor.head_position(moved).expect("head").abs_diff_eq(before, 2e-5));
    }

    #[test]
    fn vr_pivot_rejects_invalid_frames_without_poisoning_the_previous_anchor() {
        let view = rig_view(Vec3::new(20.0, 30.0, 2.0), 0.0);
        let head = pose(Vec3::new(0.2, 1.6, -0.3));
        let mut anchor = Anchor::new(view, head).expect("anchor");
        anchor.follow_head(view, head, RigMode::FirstPerson).expect("start");
        let before = anchor.eye(head, fov(), 0.1).expect("eye");
        assert!(anchor.follow_head(Mat4::ZERO, head, RigMode::FirstPerson).is_none());
        let invalid_head = Pose {
            position: Vec3::splat(f32::NAN),
            ..head
        };
        assert!(anchor.follow_head(view, invalid_head, RigMode::Avatar).is_none());
        assert_eq!(anchor.eye(head, fov(), 0.1).expect("eye").position, before.position);
        assert_eq!(anchor.eye(head, fov(), 0.1).expect("eye").view, before.view);
        anchor.follow_head(view, head, RigMode::Avatar).expect("mode change is safe");
        anchor.follow_head(view, head, RigMode::Avatar).expect("repeated mode is safe");
        assert!(anchor.eye(head, fov(), 0.1).expect("eye").view.is_finite());
    }

    #[test]
    fn stereo_rejects_overflowing_runtime_quaternion_norm() {
        let pose = Pose {
            position: Vec3::ZERO,
            orientation: Quat::from_xyzw(f32::MAX, 0.0, 0.0, 1.0),
        };
        assert!(pose.matrix().is_none());
    }
    #[test]
    fn stereo_locomotion_preserves_physical_lean_and_ignores_artificial_pitch() {
        let initial = glam::camera::rh::view::look_to_mat4(Vec3::ZERO, Vec3::X, Vec3::Z);
        let mut anchor = Anchor::new(initial, pose(Vec3::ZERO)).expect("anchor");
        let position = Vec3::new(4.0, 2.0, 1.5);
        let moved = glam::camera::rh::view::look_to_mat4(position, Vec3::Y + Vec3::Z, Vec3::Z);
        anchor.follow(moved).expect("follow");
        let eye = anchor.eye(pose(Vec3::ZERO), fov(), 0.1).expect("eye");
        assert!(eye.position.distance(position) < 1e-5);
        let direction = eye.view.inverse().transform_vector3(Vec3::NEG_Z);
        assert!(direction.distance(Vec3::Y) < 1e-5);
        let leaned = anchor.eye(pose(Vec3::new(0.1, 0.0, 0.0)), fov(), 0.1).expect("lean");
        assert!((leaned.position.distance(position) - 0.1).abs() < 1e-5);
    }

    #[test]
    fn stereo_asymmetric_edges_and_reverse_depth() {
        let f = fov();
        let n = 0.08;
        let p = f.projection(n).expect("valid FOV");
        for (x, y, expected) in [
            (f.left.tan(), 0.0, (-1.0, None)),
            (f.right.tan(), 0.0, (1.0, None)),
            (0.0, f.down.tan(), (0.0, Some(-1.0))),
            (0.0, f.up.tan(), (0.0, Some(1.0))),
        ] {
            let q = p * Vec4::new(x * n, y * n, -n, 1.0);
            let ndc = q / q.w;
            assert!((ndc.z - 1.0).abs() < 1e-5);
            if let Some(y) = expected.1 {
                assert!((ndc.y - y).abs() < 1e-5);
            } else {
                assert!((ndc.x - expected.0).abs() < 1e-5);
            }
        }
        let distant = p * Vec4::new(0.0, 0.0, -10000.0, 1.0);
        assert!(distant.z / distant.w < 0.00001);
    }
    #[test]
    fn stereo_anchor_cancels_initial_height_preserves_ipd_and_lean() {
        let desktop = glam::camera::rh::view::look_to_mat4(Vec3::new(10., 20., 30.), Vec3::Y, Vec3::Z);
        let head = pose(Vec3::new(2., 1.6, 3.));
        let a = Anchor::new(desktop, head).expect("anchor");
        let left = a.eye(pose(head.position - Vec3::X * 0.032), fov(), 0.1).expect("left");
        let right = a.eye(pose(head.position + Vec3::X * 0.032), fov(), 0.1).expect("right");
        assert!(((left.position + right.position) * 0.5 - Vec3::new(10., 20., 30.)).length() < 1e-4);
        assert!(((right.position - left.position).length() - 0.064).abs() < 1e-5);
        let leaned = a.eye(pose(head.position + Vec3::X * 0.2), fov(), 0.1).expect("lean");
        assert!((leaned.position - Vec3::new(10.2, 20., 30.)).length() < 1e-4);
    }
    #[test]
    fn stereo_anchor_starts_on_horizon_for_pitched_and_vertical_desktop_views() {
        let position = Vec3::new(10.0, 20.0, 30.0);
        for direction in [Vec3::new(1.0, 1.0, -3.0).normalize(), Vec3::new(1.0, 1.0, 3.0).normalize()] {
            let view = glam::camera::rh::view::look_to_mat4(position, direction, Vec3::Z);
            let anchor = Anchor::new(view, pose(Vec3::ZERO)).expect("anchor");
            let eye = anchor.eye(pose(Vec3::ZERO), fov(), 0.1).expect("eye");
            let world = eye.view.inverse();
            let at = world.transform_vector3(Vec3::NEG_Z);
            assert!(at.z.abs() < 1e-5);
            assert!(at.normalize().dot(Vec3::new(1.0, 1.0, 0.0).normalize()) > 0.9999);
            assert!((eye.position - position).length() < 1e-4);
            assert!(world.transform_vector3(Vec3::Y).dot(Vec3::Z) > 0.9999);
            let head = Pose {
                position: Vec3::ZERO,
                orientation: Quat::from_rotation_x(0.3),
            };
            let moved = anchor.eye(head, fov(), 0.1).expect("head pitch");
            assert!(moved.view.inverse().transform_vector3(Vec3::NEG_Z).z.abs() > 0.1);
        }
        let vertical = glam::camera::rh::view::look_to_mat4(position, Vec3::NEG_Z, Vec3::Y);
        let eye = Anchor::new(vertical, pose(Vec3::ZERO))
            .expect("vertical anchor")
            .eye(pose(Vec3::ZERO), fov(), 0.1)
            .expect("vertical eye");
        assert!(eye.view.is_finite());
        assert!(eye.view.inverse().transform_vector3(Vec3::NEG_Z).z.abs() < 1e-5);
    }

    #[test]
    fn stereo_headset_resting_tilt_does_not_offset_the_horizon_when_worn() {
        let desktop = glam::camera::rh::view::look_to_mat4(Vec3::new(5., 6., 7.), Vec3::new(0., 1., -0.4).normalize(), Vec3::Z);
        let yaw = Quat::from_rotation_y(0.7);
        for tilt in [-0.6, 0.6] {
            let resting = Pose {
                position: Vec3::new(1., 1.2, 2.),
                orientation: yaw * Quat::from_rotation_x(tilt),
            };
            let anchor = Anchor::new(desktop, resting).expect("resting anchor");
            let worn = Pose {
                position: resting.position,
                orientation: yaw,
            };
            let eye = anchor.eye(worn, fov(), 0.1).expect("worn eye");
            let world = eye.view.inverse();
            let at = world.transform_vector3(Vec3::NEG_Z);
            assert!(at.z.abs() < 1e-5, "resting tilt {tilt}: {at:?}");
            assert!(world.transform_vector3(Vec3::Y).dot(Vec3::Z) > 0.9999);
        }
    }

    #[test]
    fn stereo_rejects_invalid_runtime_inputs() {
        assert!(fov().projection(0.0).is_none());
        assert!(Fov { left: f32::NAN, ..fov() }.projection(0.1).is_none());
        assert!(Anchor::new(Mat4::ZERO, pose(Vec3::ZERO)).is_none());
        assert!(
            Anchor::new(
                Mat4::IDENTITY,
                Pose {
                    position: Vec3::ZERO,
                    orientation: Quat::from_xyzw(0., 0., 0., 0.)
                }
            )
            .is_none()
        );
    }
}
