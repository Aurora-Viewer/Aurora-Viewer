//! Six-degree-of-freedom camera pose, following LLViewerJoystick::moveFlycam
//! (indra/newview/llviewerjoystick.cpp, originally LGPL 2.1).

use glam::{Mat3, Quat, Vec3};

/// Physical headset orientation rotates travel only; artificial yaw stays on the rig.
pub fn vr_axes(mut axes: [f32; 7], head_rotation: Quat) -> [f32; 7] {
    // Aurora: +X forward, +Y left, +Z up. OpenXR: +X right, +Y up, -Z forward.
    let travel = head_rotation * Vec3::new(-axes[1], axes[2], -axes[0]);
    axes[0] = -travel.z;
    axes[1] = -travel.x;
    axes[2] = travel.y;
    axes[3] = 0.0;
    axes[4] = 0.0;
    axes
}

#[derive(Debug, Clone, Copy)]
pub struct Flycam {
    pub position: Vec3,
    /// Local +X forward, +Y left and +Z up, as in the agent frame.
    pub rotation: Quat,
    pub fov: f32,
    reference_fov: f32,
}

impl Flycam {
    pub fn from_view(position: Vec3, target: Vec3, up: Vec3, fov: f32) -> Self {
        Self::from_direction(position, target - position, up, fov)
    }

    /// Construct from a direction without adding/subtracting world coordinates.
    pub fn from_direction(position: Vec3, forward: Vec3, up: Vec3, fov: f32) -> Self {
        let forward = forward.normalize_or(Vec3::X);
        let left = up.cross(forward).normalize_or(Vec3::Y);
        let up = forward.cross(left).normalize_or(Vec3::Z);
        Self {
            position,
            rotation: Quat::from_mat3(&Mat3::from_cols(forward, left, up)).normalize(),
            fov,
            reference_fov: fov,
        }
    }

    pub fn set_fov(&mut self, fov: f32) {
        self.fov = fov;
        self.reference_fov = fov;
    }

    pub fn reset_zoom(&mut self) {
        self.fov = self.reference_fov;
    }

    /// Keep position, heading and lens while removing artificial pitch/roll.
    pub fn reset_orientation(&mut self) {
        let forward = self.forward().with_z(0.0).normalize_or(Vec3::X);
        self.rotation = Self::from_direction(self.position, forward, Vec3::Z, self.fov).rotation;
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::X
    }
    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Z
    }

    pub fn step(&mut self, axes: [f32; 7], settings: &super::Settings, dt: f32) {
        self.step_with_speed(axes, settings, dt, 1.0);
    }

    pub fn step_with_speed(&mut self, axes: [f32; 7], settings: &super::Settings, dt: f32, speed: f32) {
        let dt = super::finite(dt, 0.0).clamp(0.0, 0.2);
        let a = axes.map(|v| super::finite(v, 0.0).clamp(-20.0, 20.0));
        self.position += self.rotation * Vec3::new(a[0], a[1], a[2]) * dt * speed;
        self.rotation =
            (self.rotation * Quat::from_rotation_z(a[5] * dt) * Quat::from_rotation_y(-a[4] * dt) * Quat::from_rotation_x(a[3] * dt))
                .normalize();
        if settings.auto_level {
            let leveled = Self::from_view(self.position, self.position + self.forward(), Vec3::Z, self.fov);
            let feather = super::finite(settings.flycam.feather, 5.0).clamp(0.0, 32.0);
            let blend = if feather == 0.0 { 1.0 } else { (dt * feather).min(1.0) };
            self.rotation = self.rotation.slerp(leveled.rotation, blend);
        }
        if settings.absolute_zoom() {
            if settings.zoom_mode == super::ZoomMode::Buttons || (settings.mapping[6] >= 0 && settings.flycam.gain[6] != 0.0) {
                self.fov = self.reference_fov + a[6];
            }
        } else {
            self.fov += a[6] * dt;
        }
        self.fov = self.fov.clamp(*crate::camera::FOV_RANGE.start(), *crate::camera::FOV_RANGE.end());
    }

    pub fn roll(&self) -> f32 {
        let left = crate::camera::view_reference_up(self.forward())
            .cross(self.forward())
            .normalize_or(Vec3::Y);
        let above = self.forward().cross(left);
        (-self.up().dot(left)).atan2(self.up().dot(above))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_view_preserves_position_heading_and_zoom() {
        let mut cam = Flycam::from_direction(Vec3::new(3.0, 5.0, 7.0), Vec3::new(1.0, 2.0, 3.0), Vec3::Z, 0.8);
        cam.rotation *= Quat::from_rotation_x(0.7);
        let heading = cam.forward().with_z(0.0).normalize();
        cam.reset_orientation();
        assert_eq!(cam.position, Vec3::new(3.0, 5.0, 7.0));
        assert_eq!(cam.fov, 0.8);
        assert!(cam.forward().abs_diff_eq(heading, 1e-5));
        assert!(cam.up().abs_diff_eq(Vec3::Z, 1e-5));
    }

    #[test]
    fn vr_flycam_forward_follows_full_rendered_head_pose_without_tilting_rig() {
        use aurora_render::stereo_view::{Anchor, Fov, Pose};
        for rig_yaw in [0.0_f32, 0.73] {
            for initial_yaw in [0.0, 1.2] {
                for head_yaw in [0.0, std::f32::consts::FRAC_PI_2, -std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
                    for pitch in [-0.6, 0.0, 0.6] {
                        let direction = Vec3::new(rig_yaw.cos(), rig_yaw.sin(), 0.0);
                        let mut camera = Flycam::from_view(Vec3::ZERO, direction, Vec3::Z, 1.0);
                        let view = glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), camera.up());
                        let mut anchor = Anchor::new(
                            view,
                            Pose {
                                position: Vec3::ZERO,
                                orientation: Quat::from_rotation_y(initial_yaw),
                            },
                        )
                        .expect("entry");
                        let head = Pose {
                            position: Vec3::ZERO,
                            orientation: Quat::from_rotation_y(initial_yaw + head_yaw) * Quat::from_rotation_x(pitch),
                        };
                        let rotation = anchor.relative_head_rotation(head).expect("head");
                        let axes = vr_axes([1.0, 0.0, 0.0, 0.9, -0.7, 0.0, 0.0], rotation);
                        for _ in 0..10 {
                            let before = camera.position;
                            camera.step(axes, &super::super::Settings::default(), 0.016);
                            let view = glam::camera::rh::view::look_to_mat4(camera.position, camera.forward(), camera.up());
                            anchor.follow(view).expect("follow");
                            let eye = anchor
                                .eye(
                                    head,
                                    Fov {
                                        left: -0.6,
                                        right: 0.6,
                                        down: -0.6,
                                        up: 0.6,
                                    },
                                    0.1,
                                )
                                .expect("eye");
                            let gaze = eye.view.inverse().transform_vector3(Vec3::NEG_Z).normalize();
                            assert!((camera.position - before).normalize().dot(gaze) > 0.999);
                            assert!(camera.forward().abs_diff_eq(direction, 1e-5));
                            assert!(camera.up().abs_diff_eq(Vec3::Z, 1e-5));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn joystick_flycam_travel_endpoints_survive_axis_limits() {
        let s = super::super::Settings::default();
        for endpoint in [0.05, 0.1, 2.5, 20.0, 50.0] {
            let mut cam = Flycam::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, 1.0);
            cam.step_with_speed([2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], &s, 0.1, endpoint);
            assert!((cam.position.x - 0.2 * endpoint).abs() < 1e-5, "endpoint {endpoint}");
        }
    }

    #[test]
    fn joystick_flycam_leveling_is_gradual_and_off_preserves_roll() {
        let mut s = super::super::Settings {
            auto_level: false,
            ..Default::default()
        };
        let mut cam = Flycam::from_view(Vec3::ZERO, Vec3::X, Quat::from_rotation_x(0.7) * Vec3::Z, 1.0);
        cam.step([0.0; 7], &s, 0.1);
        assert!((cam.roll().abs() - 0.7).abs() < 1e-5);
        s.auto_level = true;
        cam.step([0.0; 7], &s, 0.1);
        assert!((cam.roll().abs() - 0.35).abs() < 1e-5);
    }

    #[test]
    fn joystick_flycam_moves_in_its_own_frame_and_caps_long_frames() {
        let s = super::super::Settings {
            auto_level: false,
            ..Default::default()
        };
        let mut cam = Flycam::from_view(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 3.0, 3.0), Vec3::Z, 1.0);
        cam.step([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], &s, 5.0);
        assert!(cam.position.distance(Vec3::new(1.0, 2.2, 3.0)) < 1e-5);
        cam.step([0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], &s, 0.2);
        assert!(cam.roll().abs() > 0.19);
        let before = cam.position;
        let up = cam.up();
        cam.step([0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0], &s, 0.1);
        assert!((cam.position - before).distance(up * 0.1) < 1e-5);
        cam.step([f32::NAN; 7], &s, f32::INFINITY);
        assert!(cam.position.is_finite() && cam.rotation.is_finite());
    }

    #[test]
    fn joystick_flycam_levels_and_bounds_zoom() {
        let s = super::super::Settings::default();
        let mut cam = Flycam::from_view(Vec3::ZERO, Vec3::X, Quat::from_rotation_x(0.7) * Vec3::Z, 1.0);
        for _ in 0..100 {
            cam.step([0.0; 7], &s, 0.1);
        }
        assert!(cam.roll().abs() < 1e-4);
        cam.step([20.0; 7], &s, 0.2);
        assert!(crate::camera::FOV_RANGE.contains(&cam.fov));
        for forward in [Vec3::Z, -Vec3::Z] {
            let pole = Flycam::from_view(Vec3::ZERO, forward, Vec3::X, 1.0);
            let up = Quat::from_axis_angle(forward, pole.roll()) * crate::camera::view_reference_up(forward);
            assert!(up.distance(pole.up()) < 1e-4, "a vertical handoff keeps the camera's orientation");
        }
    }
}
