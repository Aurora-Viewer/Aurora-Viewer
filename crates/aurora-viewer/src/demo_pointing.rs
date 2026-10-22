//! Offline point-at traffic and real build selection for #669.

use super::*;
use crate::build::{BuildSettings, BuildTool, Tool};
use crate::scene::{Scene, anim::BoundAnim};
use crate::world::{World, body::Body};

pub fn tick(world: &mut World, scene: &mut Scene, build: &mut BuildTool, settings: &mut BuildSettings, frame: u64, mode: &str) {
    if frame == 600 {
        let p = world.agent.position;
        world.agent.yaw = 0.0;
        world.bodies.insert(world.agent_id, Body::new(Vec3::X));
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(crate::ao::State::Standing.motion(), frame as i32)],
            sources: Vec::new(),
        });
        let target = p + Vec3::new(3.0, 1.0, 0.2);
        let mut object = prim(
            9390,
            target,
            Quat::IDENTITY,
            Vec3::splat(0.5),
            shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
            te([0.7, 0.4, 0.9, 1.0], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        object.owner_id = world.agent_id;
        let id = object.full_id;
        world.apply(NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![object],
        });
        if mode.starts_with("remote") {
            world.apply(NetEvent::PointAt {
                effect: Uuid::from_u128(9390),
                source: DEMO_LOUP,
                target: id,
                offset: [0.0; 3],
                kind: 1,
                duration: f32::MAX / 4.0,
            });
        } else {
            build.open_build(Tool::Edit);
            build.click_select(world, settings, world.objects.index_of_uuid(&id), false);
        }
        if mode == "aim" {
            let rig = scene.avatar_lib.rig.clone();
            let mut clip = link::animations().remove(0).1;
            clip.base_priority = 2;
            clip.joints
                .retain(|j| !matches!(j.joint_name.as_str(), "mShoulderRight" | "mElbowRight" | "mWristRight"));
            for (joint, rotation) in [
                ("mShoulderRight", Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                ("mElbowRight", Quat::IDENTITY),
                ("mWristRight", Quat::IDENTITY),
            ] {
                clip.joints.push(aurora_assets::JointMotion {
                    joint_name: joint.into(),
                    priority: 2,
                    rot_keys: vec![aurora_assets::anim::RotKey { time: 0.0, rotation }],
                    pos_keys: Vec::new(),
                });
            }
            let id = crate::scene::default_motions::GUN_AIM_ANIMS[0];
            scene.anims.insert(id, Arc::new(BoundAnim::bind(clip, &rig)));
            world.apply(NetEvent::AvatarAnimations {
                avatar: world.agent_id,
                anims: vec![(id, 1)],
                sources: Vec::new(),
            });
        }
    }
    if frame == 840 && mode == "release" {
        build.close(world, settings);
    }
    if frame == 840 && mode == "remote-release" {
        world.apply(NetEvent::PointAt {
            effect: Uuid::from_u128(9390),
            source: DEMO_LOUP,
            target: Uuid::nil(),
            offset: [0.0; 3],
            kind: 0,
            duration: 0.0,
        });
    }
    if frame == 760 || frame == 940 {
        log::info!(
            "demo pointing: mode {mode}, selected {}, own kind {}, remote {}",
            build.selection.len(),
            world.point_at.own.target.kind,
            world.point_at.remote.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_pointing_demo_uses_a_present_avatar_and_clears_its_target() {
        let library = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(library.clone());
        for event in events() {
            world.apply(event);
        }
        let mut scene = Scene::new(std::env::temp_dir().join("aurora-pointing-demo"), library);
        let mut build = BuildTool::default();
        let mut settings = BuildSettings::default();
        tick(&mut world, &mut scene, &mut build, &mut settings, 600, "remote-release");
        assert_eq!(world.point_at.remote.len(), 1);
        assert!(world.point_at.remote.contains_key(&DEMO_LOUP));
        tick(&mut world, &mut scene, &mut build, &mut settings, 840, "remote-release");
        assert!(world.point_at.remote.is_empty());
    }
}
