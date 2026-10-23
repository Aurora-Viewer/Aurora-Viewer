use super::*;
use crate::{demo, scene::avatar::AvatarLibrary, world::rig_graph::Target};
use aurora_net::NetEvent;
use std::sync::Arc;

fn fixture() -> (World, Editor, ik::Pose) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.0, true),
    });
    let mut editor = Editor::default();
    editor.toggle();
    let mut pose = ik::Pose::default();
    evaluate(&mut world, &editor, &mut pose);
    (world, editor, pose)
}
fn evaluate(world: &mut World, editor: &Editor, state: &mut ik::Pose) -> ik::LocalPose {
    let rig = &world.avatar_lib.rig;
    state.edit = editor.request();
    let local = state
        .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, Instant::now(), true)
        .unwrap();
    let matrices = rig.world_with(&local.rotations, Vec3::ZERO, &local.positions, &rig.scale);
    world.avatar_poses.insert(
        world.agent_id,
        matrices.iter().zip(&rig.default_world_inv).map(|(m, i)| *m * *i).collect(),
    );
    local
}
fn rotate(world: &World, editor: &mut Editor, name: &str, local_rotation: Quat) {
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(world).unwrap();
    let bone = rig.names[name];
    editor.select(bone, rig, &frame);
    let parent = rig.parent[bone].map_or(Quat::IDENTITY, |p| {
        (frame.model.inverse() * frame.joints[p]).to_scale_rotation_translation().1
    });
    editor.set_rotation(parent * local_rotation);
    editor.commit_manipulation(rig, &frame, true);
}

#[test]
fn shoulders_upper_arms_neck_and_toes_rotate_locally_without_moving_ancestors() {
    for (parent, child) in [
        ("mCollarLeft", "mShoulderLeft"),
        ("mShoulderLeft", "mWristLeft"),
        ("mCollarRight", "mShoulderRight"),
        ("mShoulderRight", "mWristRight"),
        ("mNeck", "mHead"),
        ("mFootLeft", "mToeLeft"),
        ("mFootRight", "mToeRight"),
    ] {
        let (mut world, mut editor, mut state) = fixture();
        let lib = world.avatar_lib.clone();
        let rig = &lib.rig;
        let bone = rig.names[parent];
        let child = rig.names[child];
        let child_rotation = Quat::from_rotation_x(0.25) * rig.local_rot[child];
        rotate(&world, &mut editor, &rig.canonical_names[child], child_rotation);
        evaluate(&mut world, &editor, &mut state);
        for angle in [0.4, 1.2, 2.8, -0.3, 0.4] {
            let q = Quat::from_rotation_y(angle) * rig.local_rot[bone];
            rotate(&world, &mut editor, parent, q);
            let local = evaluate(&mut world, &editor, &mut state);
            assert!(editor.request().goals.is_empty(), "{parent}: posing must not pin joints");
            assert_eq!(editor.mode, Mode::Ik, "FK does not detach the bone");
            assert_eq!(local.positions, rig.local_pos);
            for j in 0..rig.len() {
                let expected = if j == bone {
                    q
                } else if j == child {
                    child_rotation
                } else {
                    rig.local_rot[j]
                };
                assert!(
                    local.rotations[j].dot(expected).abs() > 0.99999,
                    "{parent} unexpectedly changes {}",
                    rig.canonical_names[j]
                );
            }
            let frame = Frame::new(&world).unwrap();
            let actual_child = (frame.model.inverse() * frame.joints[child]).to_scale_rotation_translation().1;
            let mut expected = child_rotation;
            let mut ancestor = rig.parent[child];
            while let Some(j) = ancestor {
                expected = (if j == bone { q } else { rig.local_rot[j] }) * expected;
                ancestor = rig.parent[j];
            }
            assert!(
                actual_child.dot(expected).abs() > 0.99999,
                "children follow {parent}, including prior child edits"
            );
            for _ in 0..8 {
                let stable = evaluate(&mut world, &editor, &mut state);
                assert_eq!(stable.rotations, local.rotations, "{parent}: no frame accumulation");
            }
        }
        editor.reset_bone(bone, rig);
        let local = evaluate(&mut world, &editor, &mut state);
        assert!(local.rotations[bone].dot(rig.local_rot[bone]).abs() > 0.99999);
        assert!(local.rotations[child].dot(child_rotation).abs() > 0.99999);
        editor.reset_pose();
        let local = evaluate(&mut world, &editor, &mut state);
        assert_eq!(local.rotations, rig.local_rot);
    }
}

#[test]
fn an_unbound_ik_move_becomes_local_pose_and_follows_later_parent_rotations() {
    let (mut world, mut editor, mut state) = fixture();
    let lib = world.avatar_lib.clone();
    let rig = &lib.rig;
    let wrist = rig.names["mWristLeft"];
    let frame = Frame::new(&world).unwrap();
    editor.select(wrist, rig, &frame);
    editor.position = editor.position.lerp(
        frame.model.inverse().transform_point3(frame.joints[editor.root].w_axis.truncate()),
        0.15,
    ) + Vec3::X * 0.04;
    editor.commit_manipulation(rig, &frame, false);
    let before = evaluate(&mut world, &editor, &mut state);
    assert!(!editor.limb_enabled(wrist));
    assert!(editor.request().goals.is_empty());
    assert_eq!(before.positions, rig.local_pos);
    let mut captured = editor.authoring.clone();
    let channels: Vec<_> = editor.request().joints.iter().map(|j| j.bone).collect();
    assert!(
        captured.baked(&channels).contains(&wrist),
        "recording the chain releases its owner even if the wrist itself did not change"
    );
    assert!(!captured.owns(wrist));
    let chest = rig.names["mChest"];
    let q = Quat::from_rotation_z(0.5) * before.rotations[chest];
    rotate(&world, &mut editor, "mChest", q);
    let after = evaluate(&mut world, &editor, &mut state);
    for j in 0..rig.len() {
        assert!(after.rotations[j].dot(if j == chest { q } else { before.rotations[j] }).abs() > 0.99999);
    }
    editor.enable_limb(wrist, true, rig, &Frame::new(&world).unwrap());
    assert!(editor.limb_enabled(wrist));
    assert_eq!(editor.request().goals.len(), 1, "only an explicit activation creates a pin");
    editor.enable_limb(wrist, false, rig, &Frame::new(&world).unwrap());
    assert!(editor.request().goals.is_empty());
}

#[test]
fn a_bound_joint_keeps_target_offsets_and_rotations_instead_of_becoming_fk() {
    let (mut world, mut editor, mut state) = fixture();
    let lib = world.avatar_lib.clone();
    let rig = &lib.rig;
    let bone = rig.names["mHead"];
    rotate(&world, &mut editor, "mHead", Quat::from_rotation_y(0.3));
    evaluate(&mut world, &editor, &mut state);
    let frame = Frame::new(&world).unwrap();
    assert!(editor.bind_target(
        &world,
        &frame,
        bone,
        Target::Prim {
            id: demo::action_id(demo::hand_ik::LEFT),
            name: "Left".into()
        }
    ));
    assert!(!editor.authoring.owns(bone));
    editor.update_graph(&world, &frame);
    let before = editor.graph.definition.bindings[0].offset;
    editor.set_rotation(Quat::from_rotation_z(0.2) * editor.rotation());
    editor.commit_manipulation(rig, &frame, true);
    editor.update_graph(&world, &frame);
    assert_eq!(editor.request().goals.len(), 1);
    assert!(editor.request().joints.is_empty());
    assert!(!editor.graph.definition.bindings[0].offset.q().abs_diff_eq(before.q(), 1e-4));
}

#[test]
fn root_translation_is_a_displacement_and_does_not_duplicate_pelvis_or_shape_offsets() {
    let (mut world, mut editor, _) = fixture();
    let lib = world.avatar_lib.clone();
    let rig = &lib.rig;
    let offset = Vec3::new(0.12, -0.08, 0.32);
    let mut scale = rig.scale.clone();
    scale[rig.pelvis] *= Vec3::new(1.2, 0.8, 1.4);
    let matrices = rig.world_with(&rig.local_rot, offset, &rig.local_pos, &scale);
    world.avatar_poses.insert(
        world.agent_id,
        matrices.iter().zip(&rig.default_world_inv).map(|(m, i)| *m * *i).collect(),
    );
    let frame = Frame::new(&world).unwrap();
    editor.select(rig.pelvis, rig, &frame);
    let delta = Vec3::new(0.08, 0.04, -0.03);
    let desired = editor.position + delta;
    editor.position = desired;
    editor.commit_manipulation(rig, &frame, false);
    let mut state = ik::Pose::default();
    state.edit = editor.request();
    assert!(state.edit.goals.is_empty());
    for _ in 0..8 {
        let local = state
            .apply(rig, &rig.local_rot, &rig.local_pos, offset, &scale, Instant::now(), true)
            .unwrap();
        let matrices = rig.world_with(&local.rotations, offset, &local.positions, &scale);
        assert!(matrices[rig.pelvis].w_axis.truncate().distance(desired) < 0.0001);
    }
}

#[test]
fn symmetry_keeps_an_explicit_opposite_hand_pin_enabled() {
    let (world, mut editor, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let right = rig.names["mWristRight"];
    let left = rig.names["mWristLeft"];
    editor.enable_limb(right, true, rig, &Frame::new(&world).unwrap());
    editor.symmetry = true;
    rotate(&world, &mut editor, "mWristLeft", Quat::from_rotation_x(0.4));
    assert!(editor.authoring.owns(left));
    assert!(!editor.authoring.owns(right));
    assert!(editor.limb_enabled(right));
    assert!(!editor.limb_enabled(left));
    assert!(editor.request().goals.iter().any(|g| g.bone == right));
}
