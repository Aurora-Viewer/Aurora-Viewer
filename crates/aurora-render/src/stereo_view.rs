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
        (self.position.is_finite() && self.orientation.is_finite() && length > 1e-8)
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

/// Level heading and position captured once on VR entry. A later desktop-camera change
/// cannot move the VR anchor. Runtime recentering requires a new anchor.
#[derive(Debug, Clone, Copy)]
pub struct Anchor(Mat4);
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
        Some(Self(level_view.inverse() * initial_level_view))
    }
    pub fn eye(self, pose: Pose, fov: Fov, near: f32) -> Option<EyeView> {
        let world = self.0 * pose.matrix()?;
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
