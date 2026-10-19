//! Offline standing on a slope or two steps, and landing recovery (#667).
use super::*;
use crate::ao::State;
use crate::scene::{Scene, anim::BoundAnim};
use aurora_assets::anim::{JointMotion, PosKey, RotKey};

pub fn animation(landing: bool) -> aurora_assets::Animation {
    let mut anim = super::idle_animation();
    for name in ["mAnkleLeft", "mAnkleRight", "mPelvis"] {
        anim.joints.push(JointMotion {
            joint_name: name.into(),
            priority: -1,
            rot_keys: vec![RotKey {
                time: 0.0,
                rotation: Quat::IDENTITY,
            }],
            pos_keys: Vec::new(),
        });
    }
    if landing {
        anim.duration = 2.0;
        anim.loop_out = 2.0;
        anim.looping = false;
        for joint in &mut anim.joints {
            let bend = if joint.joint_name.starts_with("mHip") {
                -0.45
            } else if joint.joint_name.starts_with("mKnee") {
                0.9
            } else {
                continue;
            };
            joint.rot_keys = vec![
                RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_y(bend),
                },
                RotKey {
                    time: 2.0,
                    rotation: Quat::IDENTITY,
                },
            ];
        }
        if let Some(pelvis) = anim.joints.iter_mut().find(|joint| joint.joint_name == "mPelvis") {
            pelvis.pos_keys = vec![
                PosKey {
                    time: 0.0,
                    position: Vec3::new(0.0, 0.0, -0.12),
                },
                PosKey {
                    time: 2.0,
                    position: Vec3::ZERO,
                },
            ];
        }
    }
    anim
}

pub fn tick(world: &mut crate::world::World, scene: &mut Scene, frame: u64, mode: &str) {
    if frame < 600 {
        return;
    }
    let landing = mode.starts_with("landing");
    let p = Vec3::new(128.0, 128.0, super::floor_at(128.0, 128.0));
    if frame == 600 {
        let rig = scene.avatar_lib.rig.clone();
        for state in [State::Standing, State::StandingUp] {
            scene.anims.insert(
                state.motion(),
                Arc::new(BoundAnim::bind(animation(state == State::StandingUp), &rig)),
            );
        }
        let top = |id, y, height, rotation| {
            prim(
                id,
                p + Vec3::new(0.0, y, height * 0.5),
                rotation,
                Vec3::new(3.0, 1.5, height),
                VolumeParams::default(),
                te([0.4, 0.5, 0.6, 1.0], 0, false, 0.0),
                ExtraParams::default(),
                "",
            )
        };
        let objects = if mode.starts_with("steps") {
            vec![top(9380, -0.75, 0.18, Quat::IDENTITY), top(9381, 0.75, 0.4, Quat::IDENTITY)]
        } else {
            let mut slope = top(9380, 0.0, 0.3, Quat::from_rotation_x(0.3));
            slope.scale.y = 3.0;
            vec![slope]
        };
        world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
    }
    let position = p + Vec3::Z * 1.18;
    world
        .agent
        .on_server_update_at(position, Vec3::ZERO, Vec3::ZERO, false, true, std::time::Instant::now());
    world.agent.position = position;
    world.agent.yaw = 0.0;
    world.agent.velocity = Vec3::ZERO;
    if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get_mut(i)) {
        avatar.position = world.agent.position;
        avatar.rotation = Quat::IDENTITY;
        avatar.velocity = Vec3::ZERO;
        avatar.foot_plane = None;
    }
    if frame == 600 || (landing && frame.is_multiple_of(180)) {
        let state = if landing { State::StandingUp } else { State::Standing };
        world.apply(NetEvent::AvatarAnimations {
            avatar: world.agent_id,
            anims: vec![(state.motion(), frame as i32)],
            sources: Vec::new(),
        });
    }
}
