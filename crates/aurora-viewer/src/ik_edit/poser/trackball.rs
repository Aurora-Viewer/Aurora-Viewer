//! LLVirtualTrackball-style dragging, expressed in the current camera frame.
//! Axes show the joint's actual world orientation, also in delta mode.
use crate::build::geom::Cam;
use glam::{Quat, Vec3};

pub(super) fn project(cam: &Cam, direction: Vec3) -> egui::Vec2 {
    egui::vec2(direction.dot(-cam.left), -direction.dot(cam.up))
}

pub(super) fn rotate(cam: &Cam, parent: Quat, local: Quat, delta: egui::Vec2) -> Quat {
    let world_delta = Quat::from_axis_angle(cam.up, delta.x) * Quat::from_axis_angle(-cam.left, delta.y);
    (parent.inverse() * world_delta * parent * local).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ik_edit::{Projector, gizmo};
    #[test]
    fn camera_orbit_and_roll_keep_axes_and_drag_in_the_rendered_frame() {
        let parent = Quat::from_euler(glam::EulerRot::ZYX, 0.6, -0.3, 0.2);
        let local = Quat::from_rotation_x(-0.7);
        for (eye, up) in [
            (Vec3::new(-3.0, -4.0, 2.0), Vec3::Z),
            (Vec3::new(4.0, -3.0, 2.0), Vec3::Z),
            (Vec3::new(-3.0, -4.0, 2.0), Vec3::Y),
        ] {
            let proj = Projector {
                view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 1.0, 0.05)
                    * glam::camera::rh::view::look_to_mat4(eye, -eye.normalize(), up),
                width: 900.0,
                height: 900.0,
                ppp: 1.5,
            };
            let cam = gizmo::camera(&proj).unwrap();
            let center = cam.project(Vec3::ZERO).unwrap();
            for axis in Vec3::AXES {
                let direction = parent * local * axis;
                let screen = cam.project(direction * 0.01).unwrap() - center;
                assert!(project(&cam, direction).normalized().dot(screen.normalized()) > 0.9999);
            }
            for delta in [
                egui::vec2(0.3, 0.0),
                egui::vec2(-0.3, 0.0),
                egui::vec2(0.0, -0.2),
                egui::vec2(0.0, 0.2),
            ] {
                let result = parent * rotate(&cam, parent, local, delta);
                // A point facing the camera must move with the pointer on
                // screen, regardless of the parent's orientation or view roll.
                let front = (parent * local).inverse() * -cam.at;
                let movement = project(&cam, result * front);
                assert!(movement.normalized().dot(delta.normalized()) > 0.9999);
            }
        }
    }
}
