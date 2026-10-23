use super::*;
use crate::scene::avatar::AvatarLibrary;
use std::time::Duration;

fn local(rig: &Rig) -> LocalPose {
    LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    }
}

#[test]
fn finger_contacts_keep_the_animated_knuckle_plane_when_already_on_target() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    for side in ["Left", "Right"] {
        for digit in ["Index", "Middle", "Ring", "Pinky", "Thumb"] {
            let names = [1, 2, 3].map(|i| format!("mHand{digit}{i}{side}"));
            let [root, middle, bone] = names.each_ref().map(|n| rig.names[n]);
            for (axis, shape) in [(Vec3::X, Vec3::ONE), (Vec3::Z, Vec3::ONE), (Vec3::X, Vec3::new(0.7, 1.3, 1.1))] {
                let scale: Vec<_> = rig.scale.iter().map(|s| *s * shape).collect();
                let mut pose = local(rig);
                pose.rotations[rig.names[&format!("mWrist{side}")]] *= Quat::from_euler(glam::EulerRot::XYZ, 0.8, -0.4, 1.2);
                pose.rotations[root] *= Quat::from_axis_angle(axis, -0.7);
                pose.rotations[middle] *= Quat::from_axis_angle(axis, -0.8);
                let expected = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &scale);
                let (_, q, p) = expected[bone].to_scale_rotation_translation();
                let goal = Goal {
                    follow: None,
                    bone,
                    root,
                    position: p,
                    rotation: Some(q),
                    contact: Vec3::ZERO,
                    pole: None,
                    mode: Mode::Ik,
                    weight: 1.0,
                };
                assert!(apply_goal(rig, &mut pose, Vec3::ZERO, &scale, &goal));
                let actual = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &scale);
                for joint in [root, middle, bone] {
                    assert!(
                        actual[joint].w_axis.truncate().distance(expected[joint].w_axis.truncate()) < 0.0001,
                        "{}: an unchanged contact must not swivel the middle knuckle",
                        rig.canonical_names[joint]
                    );
                    assert!(
                        actual[joint]
                            .to_scale_rotation_translation()
                            .1
                            .dot(expected[joint].to_scale_rotation_translation().1)
                            .abs()
                            > 0.99999,
                        "{}: an unchanged contact must not twist the phalanges",
                        rig.canonical_names[joint]
                    );
                }
            }
        }
    }
}

#[test]
fn finger_bend_planes_follow_the_wrist_instead_of_avatar_axes() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    for side in ["Left", "Right"] {
        for digit in ["Index", "Thumb"] {
            let names = [1, 2, 3].map(|i| format!("mHand{digit}{i}{side}"));
            let joints = names.each_ref().map(|n| rig.names[n]);
            let [root, middle, bone] = joints;
            let wrist = rig.names[&format!("mWrist{side}")];
            for oriented in [false, true] {
                let mut reference = None;
                for turn in [Quat::IDENTITY, Quat::from_euler(glam::EulerRot::XYZ, 1.1, -0.9, 2.0)] {
                    let mut pose = local(rig);
                    pose.rotations[wrist] *= turn;
                    pose.rotations[root] *= Quat::from_rotation_x(-0.6);
                    pose.rotations[middle] *= Quat::from_rotation_x(-0.9);
                    let matrices = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &rig.scale);
                    let (_, wq, wp) = matrices[wrist].to_scale_rotation_translation();
                    let (_, q, p) = matrices[bone].to_scale_rotation_translation();
                    let contact = Vec3::new(0.002, 0.005, -0.001);
                    // Pull inward on either hand: a fixed negative-Y step
                    // would push the right finger outside its reachable shell.
                    let inward = (p - matrices[root].w_axis.truncate()).normalize() * -0.004;
                    let position = p + q * contact + inward + wq * Vec3::X * 0.001;
                    let goal = Goal {
                        follow: None,
                        bone,
                        root,
                        position,
                        rotation: oriented.then_some(q),
                        contact,
                        pole: None,
                        mode: Mode::Ik,
                        weight: 1.0,
                    };
                    assert!(apply_goal(rig, &mut pose, Vec3::ZERO, &rig.scale, &goal));
                    let actual = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &rig.scale);
                    assert!(
                        (actual[bone].w_axis.truncate() + actual[bone].to_scale_rotation_translation().1 * contact).distance(position)
                            < 0.0001,
                        "{digit}{side}, oriented={oriented}: the contact must remain on its reachable target"
                    );
                    let relative = joints.map(|j| {
                        let (_, q, p) = actual[j].to_scale_rotation_translation();
                        (wq.inverse() * (p - wp), (wq.inverse() * q).normalize())
                    });
                    if let Some(expected) = reference {
                        for ((p, q), (ep, eq)) in relative.into_iter().zip(expected) {
                            assert!(
                                p.distance(ep) < 0.0001,
                                "{digit}{side}: rotating the hand must rotate its whole bend plane"
                            );
                            assert!(
                                q.dot(eq).abs() > 0.99999,
                                "{digit}{side}: rotating the hand must not add finger twist"
                            );
                        }
                    } else {
                        reference = Some(relative);
                    }
                }
            }
        }
    }
}

#[test]
fn authored_strength_blends_each_limb_without_feedback_or_stretching() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
        let bone = rig.names[name];
        let root = default_root(rig, bone);
        let mut goal = Goal {
            follow: None,
            bone,
            root,
            position: rest[bone].w_axis.truncate() + Vec3::new(0.06, 0.01, 0.02),
            rotation: Some(Quat::from_rotation_z(0.6)),
            contact: Vec3::ZERO,
            pole: None,
            mode: Mode::Ik,
            weight: 1.0,
        };
        let mut full = local(rig);
        assert!(apply_goal(rig, &mut full, Vec3::ZERO, &rig.scale, &goal));
        goal.weight = 0.0;
        let mut zero = local(rig);
        assert!(apply_goal(rig, &mut zero, Vec3::ZERO, &rig.scale, &goal));
        assert_eq!(zero.rotations, rig.local_rot);
        assert_eq!(zero.positions, rig.local_pos);
        goal.weight = 0.5;
        let mut half = local(rig);
        assert!(apply_goal(rig, &mut half, Vec3::ZERO, &rig.scale, &goal));
        for j in 0..rig.len() {
            assert!(half.rotations[j].dot(rig.local_rot[j].slerp(full.rotations[j], 0.5)).abs() > 0.99999);
        }
        assert_eq!(half.positions, rig.local_pos, "weight cannot stretch segments");
        for invalid in [f32::NAN, -0.01, 1.01] {
            goal.weight = invalid;
            assert!(!apply_goal(rig, &mut half, Vec3::ZERO, &rig.scale, &goal));
        }
    }
    let bone = rig.names["mHead"];
    let mut goal = Goal {
        follow: None,
        bone,
        root: bone,
        position: rest[bone].w_axis.truncate() + Vec3::X * 0.1,
        rotation: Some(Quat::from_rotation_z(0.8)),
        contact: Vec3::ZERO,
        pole: None,
        mode: Mode::Direct,
        weight: 1.0,
    };
    let mut full = local(rig);
    apply_goal(rig, &mut full, Vec3::ZERO, &rig.scale, &goal);
    goal.weight = 0.5;
    let mut half = local(rig);
    apply_goal(rig, &mut half, Vec3::ZERO, &rig.scale, &goal);
    assert!(half.positions[bone].distance(rig.local_pos[bone].lerp(full.positions[bone], 0.5)) < 1e-6);
}

#[test]
fn disabling_each_limb_unbakes_live_ik_from_the_frozen_animation_base() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let now = Instant::now();
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    let goals: Vec<_> = ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"]
        .map(|name| {
            let bone = rig.names[name];
            Goal {
                follow: None,
                bone,
                root: default_root(rig, bone),
                position: rest[bone].w_axis.truncate() + Vec3::X * 0.08,
                rotation: None,
                contact: Vec3::ZERO,
                pole: None,
                mode: Mode::Ik,
                weight: 1.0,
            }
        })
        .to_vec();
    let mut pose = Pose {
        targets: goals.clone(),
        ..Default::default()
    };
    for step in 0..8 {
        pose.apply(
            rig,
            &rig.local_rot,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_millis(step * 50),
            false,
        );
    }
    pose.edit = EditRequest {
        active: true,
        goals: goals.clone(),
        released: Vec::new(),
        ..Default::default()
    };
    for (i, released) in goals.iter().enumerate() {
        pose.edit.goals.retain(|g| g.bone != released.bone);
        pose.edit.released.push(released.bone);
        let output = pose
            .apply(
                rig,
                &rig.local_rot,
                &rig.local_pos,
                Vec3::ZERO,
                &rig.scale,
                now + Duration::from_secs(1 + i as u64),
                true,
            )
            .unwrap();
        for goal in &goals[..=i] {
            for j in chain(rig, goal.bone, goal.root).unwrap() {
                assert_eq!(output.rotations[j], rig.local_rot[j], "released {}", rig.canonical_names[j]);
            }
        }
    }
    let output = pose
        .apply(
            rig,
            &rig.local_rot,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_secs(8),
            true,
        )
        .unwrap();
    assert_eq!(output.rotations, rig.local_rot);
    assert_eq!(output.positions, rig.local_pos);
}

#[test]
fn editor_release_bypasses_shared_script_constraints_before_snapshotting() {
    use crate::scene::anim::Controller;
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let now = Instant::now();
    let mut controller = Controller::for_control_avatar();
    controller.ik.targets = vec![Goal {
        follow: None,
        bone: rig.names["mWristLeft"],
        root: rig.names["mShoulderLeft"],
        position: Vec3::new(0.25, 0.3, 1.4),
        rotation: None,
        contact: Vec3::ZERO,
        pole: None,
        mode: Mode::Ik,
        weight: 1.0,
    }];
    let mut output = vec![[[0.0; 4]; 4]; rig.len()];
    for i in 0..8 {
        controller.evaluate(rig, now + Duration::from_millis(i * 40), None, &mut output);
    }
    let wrist = rig.names["mWristLeft"];
    let active = glam::Mat4::from_cols_array_2d(&output[wrist]);
    controller.set_paused(true, now + Duration::from_millis(300));
    controller.ik.edit = EditRequest {
        active: true,
        released: vec![wrist],
        goals: Vec::new(),
        ..Default::default()
    };
    controller.evaluate(rig, now + Duration::from_millis(400), None, &mut output);
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    assert!(glam::Mat4::from_cols_array_2d(&output[wrist]).abs_diff_eq(rest[wrist], 1e-5));
    assert!(!active.abs_diff_eq(rest[wrist], 0.01));
    controller.ik.edit = EditRequest::default();
    controller.set_paused(false, now + Duration::from_millis(500));
    controller.evaluate(rig, now + Duration::from_millis(600), None, &mut output);
    assert!(
        glam::Mat4::from_cols_array_2d(&output[wrist]).abs_diff_eq(active, 1e-5),
        "closing returns to the existing live contact"
    );
}

#[test]
fn limb_poles_swivel_all_four_limbs_without_moving_contacts_or_changing_lengths() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let scale: Vec<_> = rig.scale.iter().map(|s| *s * Vec3::new(0.8, 1.15, 1.2)).collect();
    let pelvis = Vec3::new(0.02, -0.01, 0.08);
    for (root_name, middle_name, end_name) in [
        ("mShoulderLeft", "mElbowLeft", "mWristLeft"),
        ("mShoulderRight", "mElbowRight", "mWristRight"),
        ("mHipLeft", "mKneeLeft", "mAnkleLeft"),
        ("mHipRight", "mKneeRight", "mAnkleRight"),
    ] {
        let [root, middle, end] = [root_name, middle_name, end_name].map(|n| rig.names[n]);
        let mut author = local(rig);
        author.rotations[root] *= Quat::from_rotation_z(0.3);
        author.rotations[middle] *= Quat::from_euler(glam::EulerRot::XYZ, 0.75, 0.6, 0.3);
        let world = rig.world_with(&author.rotations, pelvis, &author.positions, &scale);
        let [a, b, c] = [root, middle, end].map(|j| world[j].w_axis.truncate());
        let orientation = world[end].to_scale_rotation_translation().1.normalize();
        for contact in [Vec3::ZERO, Vec3::new(0.025, 0.035, -0.015)] {
            let target = world[end].transform_point3(contact);
            for rotation in [None, Some(orientation)] {
                let axis = (if rotation.is_some() { c } else { target } - a).normalize();
                let start = (b - a - axis * axis.dot(b - a)).normalize();
                for angle in [-1.2, 0.0, 0.8, 2.6] {
                    let bend = Quat::from_axis_angle(axis, angle) * start;
                    let goal = Goal {
                        follow: None,
                        weight: 1.0,
                        bone: end,
                        root,
                        position: target,
                        rotation,
                        contact,
                        pole: Some(a + bend * 1.2),
                        mode: Mode::Ik,
                    };
                    let mut result = author.clone();
                    assert!(apply_goal(rig, &mut result, pelvis, &scale, &goal));
                    let solved = rig.world_with(&result.rotations, pelvis, &result.positions, &scale);
                    let [sa, sb, sc] = [root, middle, end].map(|j| solved[j].w_axis.truncate());
                    assert!(solved[end].transform_point3(contact).distance(target) < 0.0002, "{end_name}");
                    assert!((sa.distance(sb) - a.distance(b)).abs() < 0.0001);
                    assert!((sb.distance(sc) - b.distance(c)).abs() < 0.0001);
                    let actual = (sb - sa - axis * axis.dot(sb - sa)).normalize();
                    assert!(actual.dot(bend) > 0.999, "{end_name}: pole plane");
                    assert_eq!(result.positions, author.positions);
                }
            }
        }
    }
}

#[test]
fn limb_poles_handle_collinear_targets_and_unreachable_effectors_without_stretching() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let world = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    let [root, middle, end] = ["mShoulderLeft", "mElbowLeft", "mWristLeft"].map(|n| rig.names[n]);
    let [a, b, c] = [root, middle, end].map(|j| world[j].w_axis.truncate());
    for position in [a, c, a + (c - a) * 100.0] {
        for pole in [a, position, a + (c - a) * 2.0] {
            let mut result = local(rig);
            let goal = Goal {
                follow: None,
                weight: 1.0,
                bone: end,
                root,
                position,
                rotation: None,
                contact: Vec3::ZERO,
                pole: Some(pole),
                mode: Mode::Ik,
            };
            assert!(apply_goal(rig, &mut result, Vec3::ZERO, &rig.scale, &goal));
            let solved = rig.world_with(&result.rotations, Vec3::ZERO, &result.positions, &rig.scale);
            let [sa, sb, sc] = [root, middle, end].map(|j| solved[j].w_axis.truncate());
            assert!((sa.distance(sb) - a.distance(b)).abs() < 0.0001);
            assert!((sb.distance(sc) - b.distance(c)).abs() < 0.0001);
            assert!(result.rotations.iter().all(|q| q.is_finite()));
            assert_eq!(result.positions, rig.local_pos);
        }
    }
}

#[test]
fn bone_ik_direct_reaches_every_named_bone_with_shape_scale_contact_and_pelvis_offset() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let pelvis = Vec3::new(0.02, -0.01, 0.1);
    let scale: Vec<_> = rig.scale.iter().map(|s| *s * Vec3::new(0.8, 1.2, 1.1)).collect();
    let rest = rig.world_with(&rig.local_rot, pelvis, &rig.local_pos, &scale);
    for (name, &bone) in &rig.names {
        let mut pose = local(rig);
        let goal = Goal {
            follow: None,
            weight: 1.0,
            bone,
            root: bone,
            position: rest[bone].w_axis.truncate() + Vec3::new(0.1, -0.05, 0.03),
            rotation: Some(Quat::from_euler(glam::EulerRot::ZYX, 0.3, -0.4, 0.2)),
            contact: Vec3::new(0.01, 0.02, -0.01),
            pole: None,
            mode: Mode::Direct,
        };
        assert!(apply_goal(rig, &mut pose, pelvis, &scale, &goal), "{name}");
        let result = rig.world_with(&pose.rotations, pelvis, &pose.positions, &scale);
        assert!(result[bone].transform_point3(goal.contact).distance(goal.position) < 2e-5, "{name}");
        let (_, q, _) = result[bone].to_scale_rotation_translation();
        assert!(q.dot(goal.rotation.expect("rotation")).abs() > 0.99999, "{name}");
        assert_eq!(
            pose.positions
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != bone)
                .map(|(_, p)| p)
                .collect::<Vec<_>>(),
            rig.local_pos
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != bone)
                .map(|(_, p)| p)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn bone_ik_analytic_arms_legs_and_long_finger_chain_reach_without_translation_edits() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    for (bone, root) in [
        ("mWristLeft", "mShoulderLeft"),
        ("mWristRight", "mShoulderRight"),
        ("mAnkleLeft", "mHipLeft"),
        ("mAnkleRight", "mHipRight"),
        ("mHandIndex3Left", "mElbowLeft"),
    ] {
        let bone = rig.names[bone];
        let root = rig.names[root];
        // Generate a reachable goal with forward kinematics, independently of IK.
        let mut author = local(rig);
        author.rotations[root] *= Quat::from_rotation_z(0.25);
        if let Some(&joint) = chain(rig, bone, root).expect("chain").get(1) {
            author.rotations[joint] *= Quat::from_rotation_x(0.4);
        }
        let expected = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
        let mut pose = local(rig);
        let goal = Goal {
            follow: None,
            weight: 1.0,
            bone,
            root,
            position: expected[bone].w_axis.truncate(),
            rotation: None,
            contact: Vec3::ZERO,
            pole: None,
            mode: Mode::Ik,
        };
        assert!(apply_goal(rig, &mut pose, Vec3::ZERO, &rig.scale, &goal));
        let result = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &rig.scale);
        assert!(
            result[bone].w_axis.truncate().distance(goal.position) < 0.004,
            "{}",
            result[bone].w_axis.truncate().distance(goal.position)
        );
        assert_eq!(pose.positions, rig.local_pos);
        assert_eq!(pose.rotations[bone], rig.local_rot[bone], "position-only effector articulation");
    }
}

#[test]
fn bone_ik_freezes_a_copy_orders_parent_before_child_and_restores_animation_input() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let now = Instant::now();
    let bone = rig.names["mHead"];
    let parent = rig.names["mNeck"];
    let mut pose = Pose::default();
    pose.edit.active = true;
    let first = pose
        .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, now, true)
        .expect("edit");
    let mut moving = rig.local_rot.clone();
    moving[rig.pelvis] = Quat::from_rotation_z(1.0);
    let frozen = pose
        .apply(
            rig,
            &moving,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_secs(2),
            true,
        )
        .expect("frozen");
    assert_eq!(first.rotations, frozen.rotations);
    let child = Goal {
        follow: None,
        weight: 1.0,
        bone,
        root: bone,
        position: Vec3::new(0.1, 0.05, 1.7),
        rotation: Some(Quat::IDENTITY),
        contact: Vec3::ZERO,
        pole: None,
        mode: Mode::Direct,
    };
    let parent_goal = Goal {
        weight: 1.0,
        bone: parent,
        root: parent,
        position: Vec3::new(0.03, 0.05, 1.5),
        ..child.clone()
    };
    pose.edit.goals = vec![child.clone(), parent_goal];
    let edited = pose
        .apply(
            rig,
            &moving,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_secs(3),
            true,
        )
        .expect("edited");
    let result = rig.world_with(&edited.rotations, Vec3::ZERO, &edited.positions, &rig.scale);
    assert!(result[bone].w_axis.truncate().distance(child.position) < 1e-5);
    assert_eq!(moving[rig.pelvis], Quat::from_rotation_z(1.0));
    pose.edit.active = false;
    assert!(
        pose.apply(
            rig,
            &moving,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_secs(4),
            false
        )
        .is_none()
    );
    assert!(pose.frozen.is_none());
}

#[test]
fn bone_ik_bounds_bad_chains_unreachable_targets_and_releases_runtime_contacts() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bone = rig.names["mWristLeft"];
    let goal = Goal {
        follow: None,
        weight: 1.0,
        bone,
        root: rig.names["mShoulderLeft"],
        position: Vec3::splat(100.0),
        rotation: None,
        contact: Vec3::ZERO,
        pole: None,
        mode: Mode::Ik,
    };
    let mut output = local(rig);
    assert!(apply_goal(rig, &mut output, Vec3::ZERO, &rig.scale, &goal));
    assert_eq!(output.positions, rig.local_pos);
    assert!(output.rotations.iter().all(|q| q.is_finite()));
    for invalid in [
        Goal {
            root: rig.names["mAnkleRight"],
            ..goal.clone()
        },
        Goal {
            position: Vec3::NAN,
            ..goal.clone()
        },
        Goal {
            rotation: Some(Quat::from_xyzw(0.0, 0.0, 0.0, 100.0)),
            ..goal.clone()
        },
    ] {
        assert!(!apply_goal(rig, &mut output, Vec3::ZERO, &rig.scale, &invalid));
    }
    let now = Instant::now();
    let mut runtime = Pose {
        targets: vec![goal],
        ..Default::default()
    };
    for frame in 0..8 {
        assert!(
            runtime
                .apply(
                    rig,
                    &rig.local_rot,
                    &rig.local_pos,
                    Vec3::ZERO,
                    &rig.scale,
                    now + Duration::from_millis(frame * 50),
                    false
                )
                .is_some()
        );
    }
    runtime.targets.clear();
    for frame in 8..16 {
        runtime.apply(
            rig,
            &rig.local_rot,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_millis(frame * 50),
            false,
        );
    }
    assert!(
        runtime
            .apply(
                rig,
                &rig.local_rot,
                &rig.local_pos,
                Vec3::ZERO,
                &rig.scale,
                now + Duration::from_secs(1),
                false
            )
            .is_none()
    );
}

#[test]
fn editor_can_hide_script_ik_without_stopping_its_updates_or_local_authoring() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let now = Instant::now();
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    let head = rig.names["mHead"];
    let hand = rig.names["mWristLeft"];
    let goal = |bone: usize, shift: f32| Goal {
        follow: None,
        weight: 1.0,
        bone,
        root: bone,
        position: rest[bone].w_axis.truncate() + Vec3::X * shift,
        rotation: None,
        contact: Vec3::ZERO,
        pole: None,
        mode: Mode::Direct,
    };
    let mut pose = Pose {
        targets: vec![goal(head, 0.1)],
        ..Default::default()
    };
    for step in 0..8 {
        pose.apply(
            rig,
            &rig.local_rot,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_millis(step * 50),
            false,
        );
    }
    pose.edit = EditRequest {
        active: true,
        suppress_script_ik: true,
        goals: vec![goal(hand, 0.05)],
        ..Default::default()
    };
    let sample = |pose: &mut Pose, ms: u64| {
        pose.apply(
            rig,
            &rig.local_rot,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_millis(ms),
            true,
        )
        .unwrap()
    };
    let hidden = sample(&mut pose, 500);
    assert!(hidden.positions[head].abs_diff_eq(rig.local_pos[head], 1e-6));
    assert!(hidden.positions[hand].distance(rig.local_pos[hand]) > 0.04);
    pose.targets[0] = goal(head, 0.2);
    sample(&mut pose, 550);
    pose.edit.suppress_script_ik = false;
    let shown = sample(&mut pose, 600);
    assert!(
        shown.positions[head].distance(rig.local_pos[head]) > 0.19,
        "runtime target kept updating under the local mask"
    );
    assert!(shown.positions[hand].abs_diff_eq(hidden.positions[hand], 1e-6));
    pose.edit.suppress_script_ik = true;
    assert!(sample(&mut pose, 650).positions[head].abs_diff_eq(rig.local_pos[head], 1e-6));
    pose.edit = EditRequest::default();
    assert!(sample(&mut pose, 700).positions[head].distance(rig.local_pos[head]) > 0.19);
}
