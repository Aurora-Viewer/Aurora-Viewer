//! Category framing from LLEditWearableDictionary::Subparts
//! (llpaneleditwearable.cpp, Firestorm, originally LGPL 2.1).

use crate::{scene::Scene, world::World};
use glam::{Quat, Vec3};
use std::time::Instant;

pub fn view(world: &World, scene: &Scene, group: usize, fov: f32) -> Option<(Vec3, Vec3)> {
    let now = Instant::now();
    let id = world.agent_id;
    let pelvis = scene.joint_world(world, &id, "mPelvis", now)?;
    let head = scene.joint_world(world, &id, "mHead", now)?;
    let foot = scene.joint_world(world, &id, "mFootLeft", now)?;
    let (target, distance) = match group {
        1..=6 => (head + Vec3::Z * 0.05, 0.75),
        7 => (scene.joint_world(world, &id, "mTorso", now)? + Vec3::Z * 0.25, 1.3),
        8 => ((pelvis + foot) * 0.5, 1.55),
        _ => {
            let height = (head.z - foot.z + 0.35).max(1.0);
            (
                (head + foot) * 0.5 + Vec3::Z * 0.07,
                (height * 0.65 / (fov * 0.5).tan().max(0.1)).max(2.4),
            )
        }
    };
    let facing = Quat::from_rotation_z(world.agent.yaw);
    Some((target + facing * Vec3::new(distance, 0.08 * distance, 0.04 * distance), target))
}

/// Keep the edited part in the larger free area beside the editor, including
/// narrow viewports. Shift eye and focus together to retain the front angle.
pub fn beside_editor((eye, target): (Vec3, Vec3), fov: f32, viewport: egui::Rect, editor: egui::Rect) -> (Vec3, Vec3) {
    let left = (editor.left() - viewport.left()).max(0.0);
    let right = (viewport.right() - editor.right()).max(0.0);
    if viewport.width() < 1.0 || viewport.height() < 1.0 || left.max(right) < 180.0 {
        return (eye, target);
    }
    let x = if right >= left {
        (editor.right() + viewport.right()) * 0.5
    } else {
        (viewport.left() + editor.left()) * 0.5
    };
    let ndc = (x - viewport.center().x) * 2.0 / viewport.width();
    let screen_right = (target - eye).cross(Vec3::Z).normalize_or_zero();
    let shift = -screen_right * ndc * eye.distance(target) * (fov * 0.5).tan() * viewport.aspect_ratio();
    (eye + shift, target + shift)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn categories_frame_full_body_head_torso_and_legs() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let scene = Scene::new(std::env::temp_dir().join("aurora-shape-camera"), world.avatar_lib.clone());
        let body = view(&world, &scene, 0, 1.05).expect("body");
        let head = view(&world, &scene, 1, 1.05).expect("head");
        let torso = view(&world, &scene, 7, 1.05).expect("torso");
        let legs = view(&world, &scene, 8, 1.05).expect("legs");
        assert!(head.1.z > torso.1.z && torso.1.z > legs.1.z);
        assert!(body.0.distance(body.1) > torso.0.distance(torso.1));
        assert!(head.0.distance(head.1) < torso.0.distance(torso.1));
        for category in 2..=6 {
            assert_eq!(view(&world, &scene, category, 1.05), Some(head));
        }
        world.avatar_poses.insert(
            world.agent_id,
            vec![glam::Mat4::from_translation(Vec3::Z * 2.0); world.avatar_lib.rig.len()],
        );
        world.avatar_shape_dz.insert(world.agent_id, 0.25);
        let posed = view(&world, &scene, 1, 1.05).expect("posed head");
        assert!((posed.1.z - head.1.z - 2.25).abs() < 1e-5, "use rendered pose and root height");
    }
    #[test]
    fn edited_head_stays_visible_beside_a_narrow_or_right_hand_editor() {
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 560.0));
        let eye = Vec3::new(0.75, 0.0, 0.0);
        let project = |eye: Vec3, target: Vec3| {
            let camera = glam::camera::rh::proj::directx::perspective(1.05, viewport.aspect_ratio(), 0.01, 10.0)
                * glam::camera::rh::view::look_at_mat4(eye, target, Vec3::Z);
            let clip = camera * Vec3::ZERO.extend(1.0);
            viewport.left() + (clip.x / clip.w + 1.0) * 0.5 * viewport.width()
        };
        let left = egui::Rect::from_min_size(egui::pos2(28.0, 64.0), egui::vec2(440.0, 430.0));
        let shifted = beside_editor((eye, Vec3::ZERO), 1.05, viewport, left);
        assert!(project(shifted.0, shifted.1) > left.right());
        let right = left.translate(egui::vec2(320.0, 0.0));
        let shifted = beside_editor((eye, Vec3::ZERO), 1.05, viewport, right);
        assert!(project(shifted.0, shifted.1) < right.left());
    }
}
