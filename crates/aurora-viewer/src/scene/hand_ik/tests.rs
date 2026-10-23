use super::*;
use crate::{
    demo,
    scene::{anim::Controller, avatar::AvatarLibrary},
};
use aurora_net::NetEvent;
use glam::Mat4;
use std::{sync::Arc, time::Duration};

#[test]
fn hand_ik_two_segment_solution_reaches_both_sides_and_matches_orientation() {
    for sign in [-1.0, 1.0] {
        let target = Target {
            position: Vec3::new(0.3, sign * 0.2, -0.1),
            rotation: Some(Quat::from_rotation_x(0.6)),
            contact: Vec3::ZERO,
        };
        let bones = [Vec3::ZERO, Vec3::Y * sign * 0.3, Vec3::Y * sign * 0.55];
        let [a, b, c] = solve(bones, [Quat::IDENTITY; 3], Quat::IDENTITY, target, Vec3::new(0.0, sign, -0.5)).expect("solve");
        let elbow = a * (bones[1] - bones[0]);
        let wrist = elbow + a * b * (bones[2] - bones[1]);
        assert!(wrist.distance(target.position) < 1e-5);
        assert!((a * b * c).abs_diff_eq(target.rotation.expect("orientation"), 1e-5));
        assert!((elbow.length() - 0.3).abs() < 1e-5);
        assert!((wrist.distance(elbow) - 0.25).abs() < 1e-5);
    }
}

#[test]
fn hand_ik_position_only_preserves_animated_wrist_articulation_for_both_arms() {
    for sign in [-1.0, 1.0] {
        let parent = Quat::from_rotation_x(0.4);
        let shoulder = parent * Quat::from_rotation_z(-sign * 1.2);
        let elbow = shoulder * Quat::from_rotation_x(0.3) * Quat::from_rotation_z(sign * 0.5);
        let animated_wrist = Quat::from_rotation_y(-0.2) * Quat::from_rotation_x(0.15);
        let points = [Vec3::ZERO, shoulder * Vec3::Y * sign * 0.3, Vec3::ZERO];
        let points = [points[0], points[1], points[1] + elbow * Vec3::Y * sign * 0.25];
        for position in [Vec3::new(0.35, sign * 0.15, -0.1), Vec3::new(0.2, sign * 0.25, 0.2)] {
            let [a, b, c] = solve(
                points,
                [shoulder, elbow, elbow * animated_wrist],
                parent,
                Target {
                    position,
                    rotation: None,
                    contact: Vec3::ZERO,
                },
                Vec3::new(0.0, sign, -0.5),
            )
            .expect("position-only solve");
            let solved_shoulder = parent * a;
            let solved_elbow = solved_shoulder * b;
            let wrist = solved_shoulder * (shoulder.inverse() * (points[1] - points[0]))
                + solved_elbow * (elbow.inverse() * (points[2] - points[1]));
            assert!(wrist.distance(position) < 1e-5);
            assert!(c.dot(animated_wrist).abs() > 0.99999, "no extra wrist bend or roll");
        }
    }
}

#[test]
fn hand_ik_position_only_does_not_change_local_wrist_under_nonuniform_shape_scales() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let mut base = rig.local_rot.clone();
    let mut scale = rig.scale.clone();
    let now = Instant::now();
    let mut pose = Pose::default();
    for hand in [Hand::Left, Hand::Right] {
        let [_, elbow, wrist] = hand.joints().map(|name| rig.names[name]);
        scale[elbow] = Vec3::new(0.8, 1.2, 0.9);
        base[elbow] = Quat::from_rotation_z(0.5);
        base[wrist] = Quat::from_rotation_x(0.2) * Quat::from_rotation_z(-0.1);
        pose.targets[hand.index()] = Some(Target {
            position: Vec3::new(0.3, if hand == Hand::Left { 0.2 } else { -0.2 }, 1.4),
            rotation: None,
            contact: Vec3::ZERO,
        });
    }
    for frame in 0..8 {
        let output = pose
            .rotations(
                rig,
                &base,
                Vec3::ZERO,
                &rig.local_pos,
                &scale,
                now + Duration::from_millis(frame * 50),
                false,
            )
            .expect("contacts");
        for hand in [Hand::Left, Hand::Right] {
            let wrist = rig.names[hand.joints()[2]];
            assert_eq!(output[wrist], base[wrist], "animated local wrist must remain exact during the fade");
        }
    }
}

#[test]
fn hand_ik_unreachable_folded_and_singular_goals_remain_finite_without_stretch() {
    for position in [
        Vec3::ZERO,
        Vec3::Y * 0.55,
        Vec3::Y * 100.0,
        -Vec3::Y * 100.0,
        Vec3::new(0.0, 1.0, -0.5),
    ] {
        let points = [Vec3::ZERO, Vec3::Y * 0.3, Vec3::Y * 0.55];
        let goal = Target {
            position,
            rotation: Some(Quat::IDENTITY),
            contact: Vec3::ZERO,
        };
        let [a, b, c] = solve(points, [Quat::IDENTITY; 3], Quat::IDENTITY, goal, Vec3::new(0.0, 1.0, -0.5)).expect("finite solve");
        let elbow = a * Vec3::Y * 0.3;
        let wrist = elbow + a * b * Vec3::Y * 0.25;
        assert!(a.is_finite() && b.is_finite() && c.is_finite() && wrist.is_finite());
        assert!(wrist.length() <= 0.55001);
        assert!((wrist.distance(elbow) - 0.25).abs() < 1e-5);
    }
    assert!(
        solve(
            [Vec3::ZERO; 3],
            [Quat::IDENTITY; 3],
            Quat::IDENTITY,
            Target {
                position: Vec3::X,
                rotation: Some(Quat::IDENTITY),
                contact: Vec3::ZERO,
            },
            Vec3::Z
        )
        .is_none()
    );
}

#[test]
fn hand_ik_uses_shaped_and_mesh_overridden_lengths_and_preserves_base_rotations() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let now = Instant::now();
    for arm in [-1.0, 0.0, 1.0] {
        let values = lib.shape_params.values_with(&[(693, arm), (675, arm)]);
        let mut shape = lib.shape_params.shape(&values, rig);
        // A mesh override moves an elbow independently of the shape sliders.
        shape.local_pos[rig.names["mElbowLeft"]] += Vec3::new(0.015, 0.02, 0.0);
        let base = rig.local_rot.clone();
        let initial = rig.world_with(&base, Vec3::ZERO, &shape.local_pos, &shape.scale);
        let shoulder = initial[rig.names["mShoulderLeft"]].w_axis.truncate();
        let goal = Target {
            position: shoulder + Vec3::new(0.22, 0.12, -0.1),
            rotation: Some(Quat::from_rotation_z(-0.6)),
            contact: Vec3::ZERO,
        };
        let mut pose = Pose {
            targets: [Some(goal), None],
            ..Default::default()
        };
        let mut result = None;
        for i in 0..6 {
            result = pose.rotations(
                rig,
                &base,
                Vec3::ZERO,
                &shape.local_pos,
                &shape.scale,
                now + Duration::from_millis(i * 50),
                false,
            );
        }
        let solved = rig.world_with(&result.expect("rotations"), Vec3::ZERO, &shape.local_pos, &shape.scale);
        let wrist = rig.names["mWristLeft"];
        assert!(solved[wrist].w_axis.truncate().distance(goal.position) < 1e-4, "shape {arm}");
        let (_, q, _) = solved[wrist].to_scale_rotation_translation();
        assert!(q.abs_diff_eq(goal.rotation.expect("orientation"), 1e-4));
        assert_eq!(base, rig.local_rot, "IK must not feed its corrections back into the animation pose");
        assert!(solved[rig.names["mHipLeft"]].abs_diff_eq(initial[rig.names["mHipLeft"]], 1e-6));
        assert!(solved[rig.names["mWristRight"]].abs_diff_eq(initial[rig.names["mWristRight"]], 1e-6));
    }
}

#[test]
fn hand_ik_fades_out_to_the_unmodified_animation_and_freezes_during_editing() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let now = Instant::now();
    let mut controller = Controller::for_control_avatar();
    let mut output = vec![[[0.0; 4]; 4]; rig.len()];
    let target = Target {
        position: Vec3::new(0.25, 0.3, 1.4),
        rotation: None,
        contact: Vec3::new(0.0, 0.055, -0.015),
    };
    controller.ik.targets = vec![crate::scene::ik::Goal {
        follow: None,
        bone: rig.names["mWristLeft"],
        root: rig.names["mShoulderLeft"],
        position: target.position,
        rotation: target.rotation,
        contact: target.contact,
        pole: None,
        mode: crate::scene::ik::Mode::Ik,
        weight: 1.0,
    }];
    for i in 0..8 {
        controller.evaluate(rig, now + Duration::from_millis(i * 40), None, &mut output);
    }
    let wrist = rig.names["mWristLeft"];
    let posed = Mat4::from_cols_array_2d(&output[wrist]);
    controller.set_paused(true, now + Duration::from_millis(300));
    controller.ik.targets.clear();
    controller.evaluate(rig, now + Duration::from_millis(400), None, &mut output);
    assert!(Mat4::from_cols_array_2d(&output[wrist]).abs_diff_eq(posed, 1e-5));
    controller.set_paused(false, now + Duration::from_millis(500));
    for i in 0..8 {
        controller.evaluate(rig, now + Duration::from_millis(500 + i * 50), None, &mut output);
    }
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    assert!(Mat4::from_cols_array_2d(&output[wrist]).abs_diff_eq(rest[wrist], 1e-5));
}

#[test]
fn hand_ik_palm_contacts_follow_actual_hand_scales_and_keep_bone_lengths_in_both_modes() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let now = Instant::now();
    for authored in [false, true] {
        for hand_size in [-0.3, 0.0, 0.3] {
            let mut shape = lib
                .shape_params
                .shape(&lib.shape_params.values_with(&[(675, hand_size), (693, hand_size)]), rig);
            let mut base = rig.local_rot.clone();
            let mut contacts = [Vec3::ZERO; 2];
            for hand in [Hand::Left, Hand::Right] {
                let sign = if hand == Hand::Left { 1.0 } else { -1.0 };
                let [shoulder, elbow, wrist] = hand.joints().map(|name| rig.names[name]);
                base[shoulder] = Quat::from_rotation_z(-sign * 0.9);
                base[elbow] = Quat::from_rotation_z(sign * 0.4);
                base[wrist] = Quat::from_rotation_x(0.12) * Quat::from_rotation_z(sign * 0.1);
                shape.scale[wrist] *= Vec3::new(0.9, 1.15, 0.8);
                shape.local_pos[elbow] += Vec3::new(0.015, sign * 0.02, -0.005);
                contacts[hand.index()] = Vec3::new(sign * 0.007, sign * 0.055, -0.015);
            }
            let initial = rig.world_with(&base, Vec3::ZERO, &shape.local_pos, &shape.scale);
            let mut pose = Pose::default();
            for hand in [Hand::Left, Hand::Right] {
                let sign = if hand == Hand::Left { 1.0 } else { -1.0 };
                let shoulder = initial[rig.names[hand.joints()[0]]].w_axis.truncate();
                pose.targets[hand.index()] = Some(Target {
                    position: shoulder + Vec3::new(0.24, sign * 0.15, -0.08),
                    rotation: authored.then(|| Quat::from_rotation_y(-0.4) * Quat::from_rotation_z(sign * 0.7)),
                    contact: contacts[hand.index()],
                });
            }
            let mut output = base.clone();
            for frame in 0..8 {
                output = pose
                    .rotations(
                        rig,
                        &base,
                        Vec3::ZERO,
                        &shape.local_pos,
                        &shape.scale,
                        now + Duration::from_millis(frame * 50),
                        false,
                    )
                    .expect("palm contacts");
            }
            let solved = rig.world_with(&output, Vec3::ZERO, &shape.local_pos, &shape.scale);
            for hand in [Hand::Left, Hand::Right] {
                let [shoulder, elbow, wrist] = hand.joints().map(|name| rig.names[name]);
                let target = pose.targets[hand.index()].expect("palm goal");
                let palm = solved[wrist].transform_point3(contacts[hand.index()]);
                assert!(
                    palm.distance(target.position) < 1e-4,
                    "{hand:?}, size {hand_size}, authored {authored}"
                );
                assert!(
                    solved[wrist].w_axis.truncate().distance(target.position) > 0.02,
                    "the palm, not the wrist pivot, must reach the grip"
                );
                for (a, b) in [(shoulder, elbow), (elbow, wrist)] {
                    let before = initial[a].w_axis.truncate().distance(initial[b].w_axis.truncate());
                    let after = solved[a].w_axis.truncate().distance(solved[b].w_axis.truncate());
                    assert!((before - after).abs() < 1e-5, "bone lengths stay unchanged");
                }
                if let Some(q) = target.rotation {
                    assert!(solved[wrist].to_scale_rotation_translation().1.dot(q).abs() > 0.99999);
                } else {
                    assert_eq!(output[wrist], base[wrist], "no extra wrist twist to reach the palm point");
                }
            }
        }
    }
}

#[test]
fn hand_ik_unreachable_palm_goals_do_not_stretch_the_real_forearm() {
    for sign in [-1.0, 1.0] {
        let bones = [Vec3::ZERO, Vec3::Y * sign * 0.3, Vec3::Y * sign * 0.55];
        let contact = Vec3::new(0.0, sign * 0.055, -0.015);
        for rotation in [None, Some(Quat::from_rotation_x(0.6))] {
            for position in [Vec3::ZERO, Vec3::X * 10.0, -Vec3::Y * sign * 10.0] {
                let [a, b, c] = solve(
                    bones,
                    [Quat::IDENTITY; 3],
                    Quat::IDENTITY,
                    Target {
                        position,
                        rotation,
                        contact,
                    },
                    Vec3::new(0.0, sign, -0.5),
                )
                .expect("finite palm solve");
                let elbow = a * bones[1];
                let wrist = elbow + a * b * (bones[2] - bones[1]);
                let palm = wrist + a * b * c * contact;
                assert!(palm.is_finite());
                assert!((elbow.length() - 0.3).abs() < 1e-5);
                assert!((wrist.distance(elbow) - 0.25).abs() < 1e-5);
                assert!(palm.length() <= 0.55 + contact.length() + 1e-5);
            }
        }
    }
    let bad = solve(
        [Vec3::ZERO, Vec3::Y * 0.3, Vec3::Y * 0.55],
        [Quat::IDENTITY, Quat::IDENTITY, Quat::from_xyzw(f32::NAN, 0.0, 0.0, 1.0)],
        Quat::IDENTITY,
        Target {
            position: Vec3::X * 0.3,
            rotation: None,
            contact: Vec3::Y * 0.055,
        },
        Vec3::Z,
    );
    assert!(bad.is_none(), "an invalid source wrist must not enter the virtual-segment clamp");
}

#[test]
fn hand_ik_rendered_wrist_contacts_track_a_tilted_moving_vehicle_and_live_shape() {
    check_rendered_contacts([Vec3::ZERO; 2]);
}

#[test]
fn hand_ik_rendered_palm_contacts_track_a_tilted_moving_vehicle_and_live_hand_size() {
    check_rendered_contacts([Vec3::new(0.0, 0.055, -0.015), Vec3::new(0.005, -0.05, -0.02)]);
}

fn check_rendered_contacts(contacts: [Vec3; 2]) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.2, true),
    });
    let now = Instant::now();
    let command = world.update_hand_contacts(now).expect("hello");
    let offsets = [Vec3::new(0.01, 0.0, 0.02), Vec3::new(-0.02, 0.01, 0.0)];
    let turns = [Quat::from_rotation_x(0.3), Quat::from_rotation_y(-0.4)];
    for event in demo::hand_ik::reply(&command) {
        if let NetEvent::Chat(mut chat) = event {
            use base64::{Engine, engine::general_purpose::STANDARD};
            let mut body: serde_json::Value =
                serde_json::from_str(chat.message.strip_prefix(hand_ik::PREFIX).expect("prefix")).expect("JSON");
            let bytes = STANDARD.decode(body["data"].as_str().unwrap()).unwrap();
            let mut definition: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            for i in 0..2 {
                definition["bindings"][i]["offset"] = serde_json::json!({"position":offsets[i].to_array(),"rotation":turns[i].to_array()});
                definition["bindings"][i]["contact"] = serde_json::json!(contacts[i].to_array());
                definition["bindings"][i]["orient"] = serde_json::json!(i == 0);
            }
            let bytes = serde_json::to_vec(&definition).unwrap();
            let chunks: Vec<_> = bytes.chunks(480).collect();
            for (part, data) in chunks.iter().enumerate() {
                body["part"] = part.into();
                body["count"] = chunks.len().into();
                body["data"] = STANDARD.encode(data).into();
                chat.message = format!("{}{body}", hand_ik::PREFIX);
                world.apply(NetEvent::Chat(chat.clone()));
            }
        }
    }
    world.update_ik_targets(None, &demo::hand_ik::properties(), now);
    let mut scene = Scene::new(std::path::PathBuf::new(), lib.clone());
    scene.palette_base(world.agent_id);
    for (step, arm) in [0.0, -1.0, 1.0].into_iter().enumerate() {
        world.preview_shape(Some(lib.shape_params.values_with(&[(693, arm), (675, arm * 0.3)])));
        let mut objects = demo::hand_ik::objects(0.2 + step as f32 * 0.1, true);
        objects[1].position.x += step as f32 * 0.02;
        objects[1].rotation *= Quat::from_rotation_x(step as f32 * 0.15);
        objects[2].scale = Vec3::new(0.02, 0.6, 0.3);
        world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
        for frame in 0..8 {
            scene.update_poses(
                &mut world,
                Vec3::ZERO,
                now + Duration::from_millis((step * 500 + frame * 50) as u64),
            );
        }
        let time = now + Duration::from_millis((step * 500 + 350) as u64);
        let avatar = world.objects.index_of_uuid(&world.agent_id).expect("avatar");
        let (p, r, _) = Scene::object_transform(&world, avatar, time, 0).expect("root transform");
        let model = Mat4::from_rotation_translation(r, p) * Mat4::from_translation(-lib.pelvis);
        for (i, (bone, id)) in [("mWristLeft", demo::hand_ik::LEFT), ("mWristRight", demo::hand_ik::RIGHT)]
            .into_iter()
            .enumerate()
        {
            let joint = lib.rig.names[bone];
            let wrist = model * world.avatar_poses[&world.agent_id][joint] * lib.rig.default_world_inv[joint].inverse();
            let link = world.objects.index_of_uuid(&demo::action_id(id)).expect("handle");
            let (target_p, target_r, _) = Scene::object_transform(&world, link, time, 0).expect("handle transform");
            let target_p = target_p + target_r * offsets[i];
            let target_r = target_r * turns[i];
            let (_, wrist_r, wrist_p) = wrist.to_scale_rotation_translation();
            let joints = [
                if i == 0 { "mShoulderLeft" } else { "mShoulderRight" },
                if i == 0 { "mElbowLeft" } else { "mElbowRight" },
            ];
            let [shoulder, elbow] = joints.map(|name| {
                let j = lib.rig.names[name];
                (model * world.avatar_poses[&world.agent_id][j] * lib.rig.default_world_inv[j].inverse())
                    .w_axis
                    .truncate()
            });
            assert!(
                wrist.transform_point3(contacts[i]).distance(target_p) < 0.0005,
                "{bone}, shape {arm}, error {}, goal distance {}, reach {}",
                wrist.transform_point3(contacts[i]).distance(target_p),
                shoulder.distance(target_p),
                shoulder.distance(elbow) + elbow.distance(wrist_p)
            );
            if i == 0 {
                assert!(
                    wrist_r.dot(target_r).abs() > 0.99999,
                    "authored orientation follows the target prim"
                );
            } else {
                let elbow_joint = lib.rig.names["mElbowRight"];
                let elbow = model * world.avatar_poses[&world.agent_id][elbow_joint] * lib.rig.default_world_inv[elbow_joint].inverse();
                let (_, elbow_r, _) = elbow.to_scale_rotation_translation();
                assert!(
                    (elbow_r.inverse() * wrist_r).dot(lib.rig.local_rot[joint]).abs() > 0.99999,
                    "position-only hand follows the forearm without inheriting the grip's arbitrary axes"
                );
            }
        }
    }
}
