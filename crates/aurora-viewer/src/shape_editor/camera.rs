//! Category framing from LLEditWearableDictionary::Subparts
//! (llpaneleditwearable.cpp, Firestorm, originally LGPL 2.1).

use crate::{scene::Scene, world::World};
use glam::{Quat, Vec3};
use std::time::Instant;

/// Where the camera looks from, in the frame of the avatar's face (+X the
/// way it faces): the height of the subparts' camera offset over their
/// target offset, the face being on the negative side of their root frame
/// (the customise animation turns the avatar around). The whole body is
/// seen from above, the head nearly level. Straight in front: the subparts'
/// small sideways offsets are taken up, on the Legacy Viewer's screen, by
/// the end of the turn, which leaves the avatar facing the camera.
fn direction(group: usize) -> Vec3 {
    match group {
        1..=6 => Vec3::new(0.5, 0.0, 0.02),
        7 => Vec3::X,
        8 => Vec3::X,
        _ => Vec3::new(2.5, 0.0, 0.7),
    }
    .normalize()
}

/// `turned`: the customise animation plays, which turns our avatar half a
/// turn around its pelvis (ANIM_AGENT_CUSTOMIZE ends at 180°): its face is
/// then behind the root, where the Legacy Viewer puts the camera
/// (LLAgentCamera::changeCameraToCustomizeAvatar, LLMorphView::updateCamera
/// with the root joint's rotation). The root is the one drawn, not the
/// agent's heading: the body of our avatar follows that heading late.
pub fn view(world: &World, scene: &Scene, group: usize, fov: f32, turned: bool) -> Option<(Vec3, Vec3)> {
    let now = Instant::now();
    let id = world.agent_id;
    let pelvis = scene.joint_world(world, &id, "mPelvis", now)?;
    let head = scene.joint_world(world, &id, "mHead", now)?;
    let foot = scene.joint_world(world, &id, "mFootLeft", now)?;
    let (target, distance) = match group {
        1..=6 => (head + Vec3::Z * 0.05, 0.75),
        7 => (scene.joint_world(world, &id, "mTorso", now)? + Vec3::Z * 0.25, 1.3),
        8 => (pelvis.with_z((pelvis.z + foot.z) * 0.5), 1.55),
        _ => {
            let height = (head.z - foot.z + 0.35).max(1.0);
            (
                pelvis.with_z((head.z + foot.z) * 0.5 + 0.07),
                (height * 0.6 / (fov * 0.5).tan().max(0.1)).max(2.2),
            )
        }
    };
    let root = world
        .objects
        .index_of_uuid(&id)
        .and_then(|index| Scene::object_transform(world, index, now, 0))
        .map(|(_, rotation, _)| rotation * Vec3::X)
        .filter(|at| at.truncate().length_squared() > 1e-6)
        .map_or(world.agent.yaw, |at| at.y.atan2(at.x));
    let face = Quat::from_rotation_z(root + if turned { std::f32::consts::PI } else { 0.0 });
    Some((target + face * direction(group) * distance, target))
}

/// The avatar stays in the middle of the window, as the Legacy Viewer shows
/// it; only when the editor covers that middle (a narrow window, an editor
/// moved there) is the edited part moved to the larger free area beside
/// it. Eye and focus shift together to retain the front angle.
pub fn beside_editor((eye, target): (Vec3, Vec3), fov: f32, viewport: egui::Rect, editor: egui::Rect) -> (Vec3, Vec3) {
    let left = (editor.left() - viewport.left()).max(0.0);
    let right = (viewport.right() - editor.right()).max(0.0);
    let middle = viewport.width() * 0.15;
    let covered = editor.right() > viewport.center().x - middle && editor.left() < viewport.center().x + middle;
    if viewport.width() < 1.0 || viewport.height() < 1.0 || left.max(right) < 180.0 || !covered {
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
        let body = view(&world, &scene, 0, 1.05, false).expect("body");
        let head = view(&world, &scene, 1, 1.05, false).expect("head");
        let torso = view(&world, &scene, 7, 1.05, false).expect("torso");
        let legs = view(&world, &scene, 8, 1.05, false).expect("legs");
        assert!(head.1.z > torso.1.z && torso.1.z > legs.1.z);
        assert!(body.0.distance(body.1) > torso.0.distance(torso.1));
        assert!(head.0.distance(head.1) < torso.0.distance(torso.1));
        for category in 2..=6 {
            assert_eq!(view(&world, &scene, category, 1.05, false), Some(head));
        }
        world.avatar_poses.insert(
            world.agent_id,
            vec![glam::Mat4::from_translation(Vec3::Z * 2.0); world.avatar_lib.rig.len()],
        );
        world.avatar_shape_dz.insert(world.agent_id, 0.25);
        let posed = view(&world, &scene, 1, 1.05, false).expect("posed head");
        assert!((posed.1.z - head.1.z - 2.25).abs() < 1e-5, "use rendered pose and root height");
    }
    #[test]
    fn the_customise_turn_puts_the_camera_behind_the_root() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        // the heading the body has not caught up with yet does not count
        world.agent.yaw += 2.0;
        let scene = Scene::new(std::env::temp_dir().join("aurora-shape-camera-turn"), world.avatar_lib.clone());
        let index = world.objects.index_of_uuid(&world.agent_id).expect("our avatar");
        let (_, rotation, _) = Scene::object_transform(&world, index, Instant::now(), 0).expect("drawn");
        let (at, left) = (rotation * Vec3::X, rotation * Vec3::Y);
        for group in [0, 1, 7, 8] {
            let (eye, target) = view(&world, &scene, group, 1.05, false).expect("front");
            assert!((eye - target).normalize().dot(at) > 0.9, "in front of the root");
            assert!((eye - target).dot(left).abs() < 1e-3, "straight in front");
            let (eye, turned) = view(&world, &scene, group, 1.05, true).expect("turned");
            assert_eq!(turned, target);
            assert!((eye - target).normalize().dot(at) < -0.9, "behind the root, before the turned face");
            assert!((eye - target).dot(left).abs() < 1e-3, "straight in front of the turned face");
        }
        let (eye, target) = view(&world, &scene, 0, 1.05, true).expect("body");
        assert!(eye.z - target.z > 0.2 * eye.distance(target), "the whole body seen from above");
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
        // a wide window: the editor leaves the middle free, the avatar stays there
        let wide = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2000.0, 1080.0));
        let editor = egui::Rect::from_min_size(egui::pos2(65.0, 300.0), egui::vec2(340.0, 520.0));
        assert_eq!(beside_editor((eye, Vec3::ZERO), 1.05, wide, editor), (eye, Vec3::ZERO));
    }
}
