use super::*;
use crate::scene::avatar::AvatarLibrary;

fn local(rig: &Rig) -> LocalPose {
    LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    }
}

#[test]
fn finger_straight_chain_fallback_is_joint_local_and_finite() {
    let rotation = Quat::from_euler(glam::EulerRot::XYZ, 1.1, -0.9, 2.0);
    let shift = Vec3::new(0.2, -0.3, 0.4);
    for (axis, side) in [(Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::Z, Vec3::X)] {
        for length in [0.03, 0.3] {
            let points = [Vec3::ZERO, axis * length, axis * length * 1.8];
            let target = axis * length * 1.5 + side * length * 0.1;
            let pole = animated_pole(points, Quat::IDENTITY, target);
            assert!(pole.is_finite());
            assert!(pole.dot(target.normalize()).abs() < 0.00001);
            let rotated = animated_pole(points.map(|p| rotation * p + shift), rotation, rotation * target + shift);
            assert!(
                rotated.distance(rotation * pole) < 0.0001,
                "a straight finger must not choose avatar-space axes"
            );
        }
    }
    assert_eq!(animated_pole([Vec3::ZERO; 3], Quat::IDENTITY, Vec3::X), Vec3::ZERO);
}

fn goal(rig: &Rig, bone: usize, matrices: &[glam::Mat4]) -> Goal {
    let middle = rig.parent[bone].unwrap();
    Goal {
        follow: None,
        weight: 1.0,
        bone,
        root: extended_root(rig, bone).unwrap(),
        position: matrices[bone].transform_point3(Vec3::new(0.02, 0.03, -0.01)),
        rotation: Some(matrices[bone].to_scale_rotation_translation().1),
        contact: Vec3::new(0.02, 0.03, -0.01),
        pole: Some(
            matrices[middle].w_axis.truncate()
                + if rig.canonical_names[bone].starts_with("mWrist") {
                    -Vec3::Z * 0.3
                } else {
                    Vec3::X * 0.3
                },
        ),
        mode: Mode::Ik,
    }
}

#[test]
fn extended_limbs_recruit_the_body_to_reach_beyond_the_short_chain_without_stretching() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    for factor in [0.8, 1.3] {
        let scale: Vec<_> = rig.scale.iter().map(|s| *s * factor).collect();
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &scale);
        for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
            let bone = rig.names[name];
            let mut author = local(rig);
            let root = extended_root(rig, bone).unwrap();
            author.rotations[root] *= if name.starts_with("mWrist") {
                Quat::from_rotation_y(0.6)
            } else {
                Quat::from_rotation_x(if name.ends_with("Left") { -0.5 } else { 0.5 })
            };
            let target = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &scale);
            let desired = goal(rig, bone, &target);
            let mut short = local(rig);
            apply_goal(
                rig,
                &mut short,
                Vec3::ZERO,
                &scale,
                &Goal {
                    root: default_root(rig, bone),
                    ..desired.clone()
                },
            );
            let short_world = rig.world_with(&short.rotations, Vec3::ZERO, &short.positions, &scale);
            assert!(
                short_world[bone].transform_point3(desired.contact).distance(desired.position) > 0.001,
                "{name}: requires body assistance"
            );
            let mut result = local(rig);
            assert!(apply_goal(rig, &mut result, Vec3::ZERO, &scale, &desired));
            let solved = rig.world_with(&result.rotations, Vec3::ZERO, &result.positions, &scale);
            let error = solved[bone].transform_point3(desired.contact).distance(desired.position);
            assert!(error < 0.003, "{name} scale={factor}: error={error}");
            assert!(solved[bone].to_scale_rotation_translation().1.dot(desired.rotation.unwrap()).abs() > 0.99999);
            assert_eq!(result.positions, rig.local_pos);
            let path = chain(rig, bone, root).unwrap();
            assert!(
                path[..path.len() - 3]
                    .iter()
                    .any(|&j| result.rotations[j].dot(rig.local_rot[j]).abs() < 0.99999)
            );
            for pair in path.windows(2) {
                let length = |m: &[glam::Mat4]| m[pair[0]].w_axis.truncate().distance(m[pair[1]].w_axis.truncate());
                assert!((length(&solved) - length(&rest)).abs() < 0.00001, "{name}: segment length");
            }
        }
    }
}

#[test]
fn reachable_limb_contacts_keep_the_torso_and_pelvis_at_the_animation_pose() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
        for fraction in [0.8, 1.0] {
            let bone = rig.names[name];
            let mut desired = goal(rig, bone, &rest);
            let limb_root = default_root(rig, bone);
            let root = rest[limb_root].w_axis.truncate();
            desired.position = root + (desired.position - root) * fraction;
            let mut result = local(rig);
            assert!(apply_goal(rig, &mut result, Vec3::ZERO, &rig.scale, &desired));
            let path = chain(rig, bone, desired.root).unwrap();
            for &j in &path[..path.len() - 3] {
                assert_eq!(result.rotations[j], rig.local_rot[j], "{name}: preserve body");
            }
        }
    }
}

#[test]
fn shared_body_contacts_converge_together_and_releasing_all_restores_the_frozen_base() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let mut author = local(rig);
    author.rotations[rig.names["mTorso"]] *= Quat::from_rotation_y(0.45);
    let targets = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
    let goals: Vec<_> = ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"]
        .iter()
        .map(|n| goal(rig, rig.names[*n], &targets))
        .collect();
    let mut state = Pose::default();
    state.edit.active = true;
    state.edit.goals = goals.clone();
    let now = Instant::now();
    let result = state
        .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, now, true)
        .unwrap();
    let solved = rig.world_with(&result.rotations, Vec3::ZERO, &result.positions, &rig.scale);
    let errors: Vec<_> = goals
        .iter()
        .map(|g| {
            (
                rig.canonical_names[g.bone].clone(),
                solved[g.bone].transform_point3(g.contact).distance(g.position),
            )
        })
        .collect();
    assert!(
        errors.iter().all(|(_, e)| *e < 0.004),
        "{errors:?}; pelvis={:?}, torso={:?}",
        result.rotations[rig.pelvis],
        result.rotations[rig.names["mTorso"]]
    );
    state.edit.goals.clear();
    let released = state
        .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, now, true)
        .unwrap();
    assert_eq!(released.rotations, rig.local_rot);
    assert_eq!(released.positions, rig.local_pos);
}

#[test]
fn partial_body_weights_are_blended_once_even_with_multiple_contacts() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let mut author = local(rig);
    author.rotations[rig.names["mTorso"]] *= Quat::from_rotation_y(0.5);
    let targets = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
    let goals: Vec<_> = ["mWristLeft", "mWristRight"]
        .iter()
        .map(|n| Goal {
            weight: 0.5,
            ..goal(rig, rig.names[*n], &targets)
        })
        .collect();
    let mut expected = local(rig);
    for g in &goals {
        assert!(apply_goal(rig, &mut expected, Vec3::ZERO, &rig.scale, g));
    }
    let mut state = Pose::default();
    state.edit.active = true;
    state.edit.goals = goals;
    let result = state
        .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, Instant::now(), true)
        .unwrap();
    assert_eq!(result.rotations, expected.rotations);
    assert_eq!(result.positions, expected.positions);
}

#[test]
fn two_hand_motion_is_continuous_across_shared_torso_reach_transitions() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let torso = rig.names["mTorso"];
    let bones = ["mWristLeft", "mWristRight"].map(|n| rig.names[n]);
    let mut author = local(rig);
    author.rotations[torso] *= Quat::from_rotation_y(0.45);
    let targets = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
    let now = Instant::now();
    for extended in [false, true] {
        let mut state = Pose {
            targets: bones
                .iter()
                .map(|&bone| {
                    let mut g = goal(rig, bone, &targets);
                    if !extended {
                        g.root = default_root(rig, bone);
                    }
                    g
                })
                .collect(),
            ..Default::default()
        };
        let mut previous: Option<LocalPose> = None;
        let mut worst = (0.0_f32, 0, String::new());
        let mut worst_contact = 0.0_f32;
        for frame in 0..1200 {
            let mut source = local(rig);
            source.rotations[torso] *= Quat::from_rotation_y(0.45 + 0.4 * (frame as f32 * 0.006).sin());
            let pose = state
                .apply(
                    rig,
                    &source.rotations,
                    &source.positions,
                    Vec3::ZERO,
                    &rig.scale,
                    now + std::time::Duration::from_secs_f32(frame as f32 / 120.0),
                    false,
                )
                .unwrap();
            if frame > 60
                && let Some(previous) = &previous
            {
                assert_eq!(pose.positions, rig.local_pos, "continuity cannot stretch the skeleton");
                if extended {
                    let matrices = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &rig.scale);
                    for g in &state.targets {
                        worst_contact = worst_contact.max(matrices[g.bone].transform_point3(g.contact).distance(g.position));
                        assert!(matrices[g.bone].to_scale_rotation_translation().1.dot(g.rotation.unwrap()).abs() > 0.99999);
                    }
                }
                for name in [
                    "mTorso",
                    "mChest",
                    "mCollarLeft",
                    "mCollarRight",
                    "mShoulderLeft",
                    "mShoulderRight",
                    "mElbowLeft",
                    "mElbowRight",
                ] {
                    let j = rig.names[name];
                    let angle = 2.0 * previous.rotations[j].dot(pose.rotations[j]).abs().min(1.0).acos();
                    if angle > worst.0 {
                        worst = (angle, frame, name.into());
                    }
                }
            }
            previous = Some(pose);
        }
        eprintln!("two-hand continuity extended={extended}: worst inter-frame rotation {worst:?}");
        assert!(worst.0 < 0.04, "extended={extended}: worst inter-frame rotation {worst:?}");
        if extended {
            eprintln!("two-hand continuity: worst contact error {worst_contact} m");
            assert!(
                worst_contact < 0.003,
                "continuity must retain both hand contacts: {worst_contact} m"
            );
        }
        // A creator can shorten the chains while contacts remain active.
        // The old shared-body correction must immediately stop participating.
        for g in &mut state.targets {
            g.root = default_root(rig, g.bone);
        }
        let released = state
            .apply(
                rig,
                &rig.local_rot,
                &rig.local_pos,
                Vec3::ZERO,
                &rig.scale,
                now + std::time::Duration::from_secs(20),
                false,
            )
            .unwrap();
        for name in BODY_JOINTS {
            let j = rig.names[name];
            assert_eq!(released.rotations[j], rig.local_rot[j], "short chains must release {name}");
        }
        state.targets.clear();
        let mut released = None;
        for frame in 0..8 {
            released = state.apply(
                rig,
                &rig.local_rot,
                &rig.local_pos,
                Vec3::ZERO,
                &rig.scale,
                now + std::time::Duration::from_millis(21000 + frame * 50),
                false,
            );
        }
        assert!(released.is_none(), "removing contacts must restore the uncorrected animation");
    }
}

#[test]
fn repeated_two_hand_activation_restores_the_same_body_pose_without_reseating() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let torso = rig.names["mTorso"];
    let chest = rig.names["mChest"];
    let mut author = local(rig);
    author.rotations[torso] *= Quat::from_rotation_y(0.45);
    let targets = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
    let goals: Vec<_> = [
        (
            "mWristLeft",
            [0.2703108, -0.6718586, 0.19940919, 0.66013193],
            [-0.1627158, -0.7245891, -0.3275151],
        ),
        (
            "mWristRight",
            [-0.6817447, 0.19802968, 0.68146825, 0.17779069],
            [-0.7278296, -0.29464805, 0.26294714],
        ),
    ]
    .into_iter()
    .map(|(name, rotation, pole)| {
        let mut g = goal(rig, rig.names[name], &targets);
        g.contact = Vec3::ZERO;
        g.position = targets[g.bone].w_axis.truncate();
        g.rotation = Some(Quat::from_array(rotation).normalize());
        g.pole = Some(g.position + Vec3::from_array(pole));
        g
    })
    .collect();
    let now = Instant::now();
    for idle_frames in [60, 1] {
        let mut state = Pose::default();
        let mut first: Option<LocalPose> = None;
        let mut frame = 0;
        for cycle in 0..8 {
            state.targets = goals.clone();
            let mut result = None;
            for step in 0..240 {
                let mut source = local(rig);
                source.rotations[torso] *= Quat::from_rotation_y(0.25 + 0.2 * (step as f32 * 0.025).sin());
                source.rotations[chest] *= Quat::from_rotation_y(0.01 * (frame as f32 * 0.02).sin());
                result = state.apply(
                    rig,
                    &source.rotations,
                    &source.positions,
                    Vec3::ZERO,
                    &rig.scale,
                    now + std::time::Duration::from_secs_f32(frame as f32 / 120.0),
                    false,
                );
                frame += 1;
            }
            let result = result.unwrap();
            assert_eq!(result.positions, rig.local_pos, "restarting must not stretch the skeleton");
            let matrices = rig.world_with(&result.rotations, Vec3::ZERO, &result.positions, &rig.scale);
            for g in &goals {
                assert!(matrices[g.bone].transform_point3(g.contact).distance(g.position) < 0.003);
                assert!(matrices[g.bone].to_scale_rotation_translation().1.dot(g.rotation.unwrap()).abs() > 0.99999);
            }
            if let Some(first) = &first {
                let drift = rotation_delta(first.rotations[torso], result.rotations[torso]).length();
                eprintln!("repeated two-hand activation: idle={idle_frames}, cycle={cycle}, torso drift={drift}");
                assert!(drift < 0.02, "idle={idle_frames}, cycle={cycle}, torso drift={drift}");
            } else {
                first = Some(result);
            }
            state.targets.clear();
            let mut released = None;
            for _ in 0..idle_frames {
                released = state.apply(
                    rig,
                    &rig.local_rot,
                    &rig.local_pos,
                    Vec3::ZERO,
                    &rig.scale,
                    now + std::time::Duration::from_secs_f32(frame as f32 / 120.0),
                    false,
                );
                frame += 1;
            }
            if idle_frames == 60 {
                assert!(released.is_none(), "stopping must fully release the shared body correction");
            }
        }
    }
}

#[test]
fn body_assistance_recovers_when_the_current_animation_can_reach_both_hands() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let torso = rig.names["mTorso"];
    let mut author = local(rig);
    author.rotations[torso] *= Quat::from_rotation_y(0.5);
    author.rotations[rig.names["mShoulderLeft"]] *= Quat::from_rotation_z(-0.8);
    author.rotations[rig.names["mShoulderRight"]] *= Quat::from_rotation_z(0.8);
    let targets = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
    let now = Instant::now();
    for fps in [30, 60, 120] {
        let mut state = Pose {
            targets: ["mWristLeft", "mWristRight"]
                .map(|name| goal(rig, rig.names[name], &targets))
                .into(),
            ..Default::default()
        };
        let mut result = None;
        for frame in 0..fps * 4 {
            let mut source = local(rig);
            if frame >= fps {
                source.rotations[torso] = author.rotations[torso];
            }
            result = state.apply(
                rig,
                &source.rotations,
                &source.positions,
                Vec3::ZERO,
                &rig.scale,
                now + std::time::Duration::from_secs_f32(frame as f32 / fps as f32),
                false,
            );
            let pose = result.as_ref().unwrap();
            if frame == fps - 1 {
                assert!(
                    rotation_delta(rig.local_rot[torso], pose.rotations[torso]).length() > 0.05,
                    "the initial animation must require real body assistance"
                );
            }
            if frame >= fps {
                let matrices = rig.world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &rig.scale);
                for g in &state.targets {
                    assert!(
                        matrices[g.bone].transform_point3(g.contact).distance(g.position) < 0.003,
                        "recovery must retain {} contact at {fps} Hz, frame {frame}",
                        rig.canonical_names[g.bone]
                    );
                }
            }
        }
        let result = result.unwrap();
        let error = rotation_delta(author.rotations[torso], result.rotations[torso]).length();
        eprintln!("body assistance recovery: {fps} Hz, torso error={error}");
        assert!(
            error < 0.01,
            "unneeded body assistance must not remain over the new animation at {fps} Hz: {error}"
        );
    }
}
