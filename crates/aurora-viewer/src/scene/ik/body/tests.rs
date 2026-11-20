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
