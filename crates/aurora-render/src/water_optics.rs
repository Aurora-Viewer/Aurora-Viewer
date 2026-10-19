//! Reduced-resolution exterior capture for the underwater Snell window.
//! The main screen cannot contain all geometry reached by refracted rays.

use crate::{FrameParams, WaterEffects};
use glam::{Mat4, Vec3};
use std::sync::OnceLock;

fn demo_mode() -> Option<&'static str> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var_os("AURORA_DEMO")?;
        std::env::var("AURORA_DEMO_WATER").ok()
    })
    .as_deref()
}

pub(crate) fn scale(effects: WaterEffects) -> f32 {
    if !effects.enabled || !effects.fresnel {
        return 0.0;
    }
    if demo_mode().is_some_and(|mode| mode.contains("air-quarter")) {
        return 0.25;
    }
    [0.25, 0.375, 0.5, 0.75][effects.quality.min(3) as usize]
}

const ETA: f32 = 1.333;

pub(crate) fn view(f: &FrameParams, level: f32, effects: WaterEffects, aspect: f32) -> Option<(Mat4, Mat4)> {
    if scale(effects) == 0.0 || demo_mode().is_some_and(|mode| mode.contains("screen-only")) {
        return None;
    }
    capture_view(f.camera_pos, f.view, f.fov_y, level, effects, aspect)
}

fn capture_view(camera: Vec3, camera_view: Mat4, fov_y: f32, level: f32, effects: WaterEffects, aspect: f32) -> Option<(Mat4, Mat4)> {
    if !effects.enabled || !effects.fresnel {
        return None;
    }
    let margin = if effects.interactions { 0.16 } else { 0.0 };
    if camera.z >= level + margin {
        return None;
    }
    let inv = camera_view.inverse();
    let forward = inv.transform_vector3(-Vec3::Z).normalize_or(Vec3::Y);
    let pitch = forward.z.clamp(-1.0, 1.0).asin();
    // A downward-only view has no Snell window. Include a conservative
    // micro/macro normal margin when scheduling the exterior render.
    if pitch + fov_y * 0.5 + 20f32.to_radians() < 41.4f32.to_radians() {
        return None;
    }
    let horizontal = forward.truncate().normalize_or(glam::Vec2::Y);
    let k = 1.0 - ETA * ETA * (1.0 - forward.z * forward.z);
    let direction = if forward.z > 0.0 && k > 0.0 {
        forward * ETA + Vec3::Z * (k.sqrt() - ETA * forward.z)
    } else {
        (horizontal * 0.94).extend(0.342).normalize()
    };
    // Align the central refracted ray with its actual surface exit. Apparent
    // depth is d*cos(theta_air)/(eta*cos(theta_water)), not d/eta except at
    // normal incidence; the latter misprojects nearby geometry at oblique views.
    let apparent = direction.z / (ETA * ETA - 1.0 + direction.z * direction.z).sqrt();
    let eye = camera + Vec3::Z * ((level - camera.z).max(0.0) * (1.0 - apparent));
    let up = if direction.z.abs() > 0.99 {
        Vec3::new(-horizontal.x, -horizontal.y, 0.0)
    } else {
        Vec3::Z
    };
    let view = glam::camera::rh::view::look_at_mat4(eye, eye + direction, up);
    let fov = (fov_y * 1.8).clamp(100f32.to_radians(), 130f32.to_radians());
    let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, aspect, 0.01);
    Some((view, proj))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn camera(pitch: f32) -> (Vec3, Mat4) {
        let eye = Vec3::new(48.0, 120.0, 18.9);
        let angle = pitch.to_radians();
        let dir = (glam::Vec2::new(12.0, 10.0).normalize() * angle.cos()).extend(angle.sin());
        (eye, glam::camera::rh::view::look_at_mat4(eye, eye + dir, Vec3::Z))
    }
    #[test]
    fn exterior_capture_contains_geometry_missing_from_main_view() {
        let (eye, view) = camera(60.0);
        let fov = 55f32.to_radians();
        let main = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 1.6, 0.1) * view;
        let point = Vec3::new(57.0, 130.0, 25.5);
        assert!(main.project_point3(point).y < -1.0, "fixture must lie below the main image");
        let (air_view, air_proj) = capture_view(eye, view, fov, 20.0, WaterEffects::default(), 1.6).expect("underwater exterior view");
        let ndc = (air_proj * air_view).project_point3(point);
        assert!(
            ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0 && ndc.z > 0.0,
            "exterior must include offscreen reference: {ndc:?}"
        );
    }
    #[test]
    fn exterior_is_skipped_above_water_downward_and_when_disabled() {
        let (eye, view) = camera(60.0);
        let effects = WaterEffects::default();
        let call = |eye, view, effects| capture_view(eye, view, 55f32.to_radians(), 20.0, effects, 1.6);
        assert!(call(eye + Vec3::Z * 3.0, view, effects).is_none());
        assert!(call(eye, camera(-60.0).1, effects).is_none());
        assert!(call(eye, view, WaterEffects { enabled: false, ..effects }).is_none());
        assert!(call(eye, view, WaterEffects { fresnel: false, ..effects }).is_none());
    }

    #[test]
    fn central_interface_exit_aligns_with_the_refracted_capture_axis() {
        for pitch in [45.0, 60.0, 89.0] {
            let (eye, view) = camera(pitch);
            let forward = view.inverse().transform_vector3(-Vec3::Z);
            let exit = eye + forward * ((20.0 - eye.z) / forward.z);
            let (air_view, air_proj) = capture_view(eye, view, 55f32.to_radians(), 20.0, WaterEffects::default(), 1.6).expect("capture");
            let ndc = (air_proj * air_view).project_point3(exit);
            assert!(ndc.x.abs() < 0.0001 && ndc.y.abs() < 0.0001, "pitch={pitch}: {ndc:?}");
        }
    }
}
