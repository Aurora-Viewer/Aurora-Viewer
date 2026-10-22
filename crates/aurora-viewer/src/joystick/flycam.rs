//! Six-degree-of-freedom camera pose, following LLViewerJoystick::moveFlycam
//! (indra/newview/llviewerjoystick.cpp, originally LGPL 2.1).

use glam::{Mat3, Quat, Vec3};

#[derive(Debug, Clone, Copy)]
pub struct Flycam {
    pub position: Vec3,
    /// Local +X forward, +Y left and +Z up, as in the agent frame.
    pub rotation: Quat,
    pub fov: f32,
}

impl Flycam {
    pub fn from_view(position: Vec3, target: Vec3, up: Vec3, fov: f32) -> Self {
        let forward = (target - position).normalize_or(Vec3::X);
        let left = up.cross(forward).normalize_or(Vec3::Y);
        let up = forward.cross(left).normalize_or(Vec3::Z);
        Self {
            position,
            rotation: Quat::from_mat3(&Mat3::from_cols(forward, left, up)).normalize(),
            fov,
        }
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::X
    }
    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Z
    }

    pub fn step(&mut self, axes: [f32; 7], settings: &super::Settings, dt: f32) {
        let dt = super::finite(dt, 0.0).clamp(0.0, 0.2);
        let a = axes.map(|v| super::finite(v, 0.0).clamp(-20.0, 20.0));
        self.position += self.rotation * Vec3::new(a[0], a[1], a[2]) * dt;
        self.rotation =
            (self.rotation * Quat::from_rotation_z(a[5] * dt) * Quat::from_rotation_y(-a[4] * dt) * Quat::from_rotation_x(a[3] * dt))
                .normalize();
        if settings.auto_level {
            let leveled = Self::from_view(self.position, self.position + self.forward(), Vec3::Z, self.fov);
            let feather = super::finite(settings.flycam.feather, 5.0).clamp(0.0, 32.0);
            let blend = if feather == 0.0 { 1.0 } else { (dt * feather).min(1.0) };
            self.rotation = self.rotation.slerp(leveled.rotation, blend);
        }
        if settings.direct_zoom {
            if settings.mapping[6] >= 0 && settings.flycam.gain[6] != 0.0 {
                self.fov = crate::camera::DEFAULT_FOV + a[6];
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
