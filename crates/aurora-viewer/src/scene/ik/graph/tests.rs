use super::*;
use crate::{
    demo,
    scene::avatar::AvatarLibrary,
    world::rig_graph::{Binding, Curve, Function, Motion, Node, Playback},
};
use aurora_net::NetEvent;
use std::time::Duration;

fn glasses_grip(world: &World, distance: f32, now: Instant) -> (Vec<Goal>, Vec<Mat4>) {
    let rig = &world.avatar_lib.rig;
    let mut author = LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    };
    author.rotations[rig.names["mShoulderLeft"]] *= Quat::from_euler(glam::EulerRot::XYZ, 0.3, -0.4, 1.0);
    author.rotations[rig.names["mElbowLeft"]] *= Quat::from_rotation_z(-0.9);
    author.rotations[rig.names["mHandIndex1Left"]] *= Quat::from_rotation_z(-0.7);
    author.rotations[rig.names["mHandIndex2Left"]] *= Quat::from_rotation_z(-0.8);
    author.rotations[rig.names["mHandThumb1Left"]] *= Quat::from_rotation_y(0.4);
    author.rotations[rig.names["mHandThumb2Left"]] *= Quat::from_rotation_z(-0.5);
    let matrices = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
    let anchor = rigid(matrices[rig.names["mHead"]]).unwrap();
    let mut bindings = Vec::new();
    // Deliberately retain the reported export order: index, thumb, then wrist.
    for (bone, root) in [
        ("mHandIndex3Left", "mWristLeft"),
        ("mHandThumb3Left", "mHandThumb1Left"),
        ("mWristLeft", "mShoulderLeft"),
    ] {
        let offset = anchor.inverse().compose(rigid(matrices[rig.names[bone]]).unwrap());
        let mut item = serde_json::json!({"bone":bone,"root":root,"mode":"ik","orient":true,
            "target":{"kind":"bone","bone":"mHead"},"offset":offset,
            "activation":{"distance":if bone == "mHandIndex3Left" { distance } else { 0.0 },"ease_in":0.2,"ease_out":0.25}});
        if bone == "mWristLeft" {
            item["pole"] =
                serde_json::json!((anchor.q().inverse() * (matrices[rig.names["mElbowLeft"]].w_axis.truncate() - anchor.p())).to_array());
        }
        bindings.push(item);
    }
    let definition: Definition = serde_json::from_value(serde_json::json!({"bindings":bindings})).unwrap();
    assert!(definition.validate().is_ok());
    let context = Arc::new(
        Context::new(world, Mat4::IDENTITY, Arc::new(definition), None, 0.0, now).activated(Some(crate::world::hand_ik::ActivationStamp {
            session: Uuid::from_u128(1),
            epoch: Uuid::from_u128(2),
            bundle: Uuid::from_u128(3),
            cycles: [0; 16],
        })),
    );
    (context.goals(rig), matrices)
}

#[test]
fn nested_glasses_finger_contacts_keep_the_authored_index_and_thumb_knuckles() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let now = Instant::now();
    let (goals, expected) = glasses_grip(&world, 0.0, now);
    let mut base = rig.local_rot.clone();
    let fingers: Vec<_> = ["Index", "Thumb"]
        .into_iter()
        .flat_map(|digit| [1, 2, 3].map(|i| rig.names[&format!("mHand{digit}{i}Left")]))
        .collect();
    // Keep the authored finger animation, but let IK bring the arm from a
    // different base pose. A wrist pin must not choose a new knuckle plane.
    for &joint in &fingers {
        let parent = rig.parent[joint].unwrap();
        base[joint] = (rigid(expected[parent]).unwrap().q().inverse() * rigid(expected[joint]).unwrap().q()).normalize();
    }
    for edited in [false, true] {
        let mut state = Pose::default();
        if edited {
            state.edit = EditRequest {
                active: true,
                goals: goals.clone(),
                ..Default::default()
            };
        } else {
            state.targets = goals.clone();
        }
        let mut solved = None;
        for frame in 0..50 {
            solved = state.apply(
                rig,
                &base,
                &rig.local_pos,
                Vec3::ZERO,
                &rig.scale,
                now + Duration::from_millis(frame * 20),
                false,
            );
        }
        let solved = solved.unwrap();
        let actual = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
        for &joint in &fingers {
            assert!(
                rigid(actual[joint]).unwrap().p().distance(rigid(expected[joint]).unwrap().p()) < 0.0001,
                "edited={edited}, joint={}: preserve the authored knuckle position",
                rig.canonical_names[joint]
            );
            assert!(
                rigid(actual[joint]).unwrap().q().dot(rigid(expected[joint]).unwrap().q()).abs() > 0.99999,
                "edited={edited}, joint={}: preserve the authored phalanx orientation",
                rig.canonical_names[joint]
            );
        }
    }
}

#[test]
fn nested_glasses_finger_contacts_preserve_the_bound_wrist_in_live_and_editor_poses() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let now = Instant::now();
    let (goals, expected) = glasses_grip(&world, 0.0, now);
    for edited in [false, true] {
        for curl in [-0.8, 0.5] {
            let mut base = rig.local_rot.clone();
            base[rig.names["mHandIndex1Left"]] *= Quat::from_rotation_z(curl);
            base[rig.names["mHandThumb2Left"]] *= Quat::from_rotation_y(-curl);
            let mut state = Pose::default();
            if edited {
                state.edit = EditRequest {
                    active: true,
                    goals: goals.clone(),
                    ..Default::default()
                };
            } else {
                state.targets = goals.clone();
            }
            let mut solved = None;
            for frame in 0..50 {
                solved = state.apply(
                    rig,
                    &base,
                    &rig.local_pos,
                    Vec3::ZERO,
                    &rig.scale,
                    now + Duration::from_millis(frame * 20),
                    false,
                );
            }
            let solved = solved.unwrap();
            let actual = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
            for bone in ["mWristLeft", "mHandIndex3Left", "mHandThumb3Left"] {
                let bone = rig.names[bone];
                let a = rigid(actual[bone]).unwrap();
                let e = rigid(expected[bone]).unwrap();
                assert!(
                    a.p().distance(e.p()) < 0.003,
                    "edited={edited}, curl={curl}, bone={}, distance={}",
                    rig.canonical_names[bone],
                    a.p().distance(e.p())
                );
                assert!(
                    a.q().dot(e.q()).abs() > 0.9999,
                    "edited={edited}, curl={curl}, bone={}, orientation={}",
                    rig.canonical_names[bone],
                    a.q().dot(e.q()).abs()
                );
            }
            assert_eq!(solved.positions, rig.local_pos, "contacts must not stretch the fingers or arm");
        }
    }
}

#[test]
fn glasses_index_distance_gate_can_arrive_after_the_wrist_brings_it_near() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let now = Instant::now();
    let (goals, expected) = glasses_grip(&world, 0.15, now);
    let bone = rig.names["mHandIndex3Left"];
    let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    assert!(rest[bone].w_axis.truncate().distance(expected[bone].w_axis.truncate()) > 0.15);
    let mut state = Pose {
        targets: goals,
        ..Default::default()
    };
    let mut solved = None;
    for frame in 0..80 {
        solved = state.apply(
            rig,
            &rig.local_rot,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_millis(frame * 20),
            false,
        );
    }
    let solved = solved.unwrap();
    let actual = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
    assert!(
        actual[bone].w_axis.truncate().distance(expected[bone].w_axis.truncate()) < 0.003,
        "a hand already brought to the glasses must allow its fingertip contact to engage"
    );
}

#[test]
fn nested_glasses_contacts_keep_partial_and_position_only_wrist_rotation_free() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let now = Instant::now();
    for position_only in [false, true] {
        let (mut goals, expected) = glasses_grip(&world, 0.0, now);
        if position_only {
            let follow = goals[2].follow.as_mut().unwrap();
            Arc::make_mut(&mut Arc::make_mut(&mut follow.context).definition).bindings[follow.index].orient = false;
        } else {
            goals[2].weight = 0.5;
        }
        let mut base = rig.local_rot.clone();
        base[rig.names["mHandIndex1Left"]] *= Quat::from_rotation_z(0.5);
        let mut state = Pose {
            targets: goals,
            ..Default::default()
        };
        let mut solved = None;
        for frame in 0..50 {
            solved = state.apply(
                rig,
                &base,
                &rig.local_pos,
                Vec3::ZERO,
                &rig.scale,
                now + Duration::from_millis(frame * 20),
                false,
            );
        }
        let solved = solved.unwrap();
        let actual = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
        let wrist = rig.names["mWristLeft"];
        assert!(
            rigid(actual[wrist]).unwrap().q().dot(rigid(expected[wrist]).unwrap().q()).abs() < 0.9999,
            "a partial or position-only wrist must retain rotation freedom for its children"
        );
    }
}

#[test]
fn nested_glasses_contacts_release_and_restart_without_carrying_the_previous_grip() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let now = Instant::now();
    let (goals, expected) = glasses_grip(&world, 0.15, now);
    let mut state = Pose::default();
    for cycle in 0..3 {
        state.targets = goals.clone();
        for frame in 0..100 {
            let at = now + Duration::from_millis((cycle * 130 + frame) * 20);
            let solved = state
                .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, at, false)
                .unwrap();
            if frame > 70 {
                let actual = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
                for name in ["mWristLeft", "mHandIndex3Left", "mHandThumb3Left"] {
                    let bone = rig.names[name];
                    assert!(actual[bone].w_axis.truncate().distance(expected[bone].w_axis.truncate()) < 0.003);
                    assert!(rigid(actual[bone]).unwrap().q().dot(rigid(expected[bone]).unwrap().q()).abs() > 0.9999);
                }
            }
        }
        state.targets.clear();
        for frame in 100..130 {
            let released = state.apply(
                rig,
                &rig.local_rot,
                &rig.local_pos,
                Vec3::ZERO,
                &rig.scale,
                now + Duration::from_millis((cycle * 130 + frame) * 20),
                false,
            );
            if frame == 129 {
                assert!(
                    released.is_none(),
                    "the completed release must not leave frozen finger/arm channels"
                );
            }
        }
    }
}

#[test]
fn glasses_fingers_never_change_the_wrists_own_smooth_entry_or_release() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let now = Instant::now();
    let (goals, _) = glasses_grip(&world, 0.0, now);
    let wrist = rig.names["mWristLeft"];
    let mut complete = Pose {
        targets: goals.clone(),
        ..Default::default()
    };
    let mut only_wrist = Pose {
        targets: vec![goals[2].clone()],
        ..Default::default()
    };
    let mut base = rig.local_rot.clone();
    base[rig.names["mHandIndex1Left"]] *= Quat::from_rotation_z(0.5);
    for frame in 0..80 {
        if frame == 50 {
            complete.targets.clear();
            only_wrist.targets.clear();
        }
        let at = now + Duration::from_millis(frame * 20);
        let a = complete.apply(rig, &base, &rig.local_pos, Vec3::ZERO, &rig.scale, at, false);
        let b = only_wrist.apply(rig, &base, &rig.local_pos, Vec3::ZERO, &rig.scale, at, false);
        let a = a.unwrap_or_else(|| LocalPose {
            rotations: base.clone(),
            positions: rig.local_pos.clone(),
        });
        let b = b.unwrap_or_else(|| LocalPose {
            rotations: base.clone(),
            positions: rig.local_pos.clone(),
        });
        let ma = rig.world_with(&a.rotations, Vec3::ZERO, &a.positions, &rig.scale);
        let mb = rig.world_with(&b.rotations, Vec3::ZERO, &b.positions, &rig.scale);
        let a = rigid(ma[wrist]).unwrap();
        let b = rigid(mb[wrist]).unwrap();
        assert!(
            a.p().distance(b.p()) < 1e-5,
            "fingers changed the wrist position during fade, frame={frame}"
        );
        assert!(
            a.q().dot(b.q()).abs() > 0.99999,
            "fingers changed the wrist orientation during fade, frame={frame}"
        );
    }
}

fn setup() -> (World, Mat4, Vec<Mat4>) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.0, false),
    });
    let matrices = lib
        .rig
        .world_with(&lib.rig.local_rot, Vec3::ZERO, &lib.rig.local_pos, &lib.rig.scale);
    world.avatar_poses.insert(
        world.agent_id,
        matrices.iter().zip(&lib.rig.default_world_inv).map(|(m, inv)| *m * *inv).collect(),
    );
    (world, Mat4::IDENTITY, matrices)
}

#[test]
fn observed_avatar_follows_this_frames_other_avatar_pose_while_only_the_local_editor_is_frozen() {
    use crate::scene::ik::{EditRequest, JointEdit};
    use aurora_net::{ChatMessage, ChatSourceType, ChatType};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::time::Duration;
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    let rider = demo::hand_ik::RIDER;
    let mut objects = demo::hand_ik::objects(0.2, true);
    for o in objects.iter_mut().take(3) {
        o.owner_id = rider;
    }
    objects[3].full_id = rider;
    let mut local = demo::action_avatar(false);
    local.local_id = 9999;
    objects.push(local);
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    let now = Instant::now();
    let command = world.update_hand_contacts(now).unwrap();
    let aurora_net::NetCommand::ScriptDialogReply { label, .. } = command else {
        panic!("probe")
    };
    let fields: Vec<_> = label.split('|').collect();
    let definition = serde_json::json!({"nodes":[],"bindings":[{
        "bone":"mHead","mode":"direct","orient":true,
        "target":{"kind":"bone","avatar":world.agent_id,"bone":"mHead"}
    }]});
    world.apply(NetEvent::Chat(ChatMessage {
        from_name: "Observed rig".into(),
        source_id: demo::action_id(demo::hand_ik::ROOT),
        owner_id: rider,
        source_type: ChatSourceType::Object,
        chat_type: ChatType::Other(9),
        position: Vec3::ZERO,
        message: format!(
            "{}{}",
            crate::world::hand_ik::PREFIX,
            serde_json::json!({"avatar":rider,"nonce":fields[3],
            "revision":1,"bundle":Uuid::new_v4(),"epoch":Uuid::new_v4(),"enabled":true,"time":0.0,"part":0,"count":1,
            "data":STANDARD.encode(definition.to_string())})
        ),
    }));
    let mut scene = Scene::new(std::path::PathBuf::new(), lib.clone());
    scene.palette_base(world.agent_id);
    scene.palette_base(rider);
    let head = lib.rig.names["mHead"];
    scene.ik_edit = EditRequest {
        active: true,
        joints: vec![JointEdit {
            bone: head,
            rotation: Some(Quat::from_rotation_z(0.6)),
            translation: None,
            weight: 1.0,
        }],
        ..Default::default()
    };
    for frame in 0..8 {
        scene.update_poses(&mut world, Vec3::ZERO, now + Duration::from_millis(frame * 50));
    }
    scene.ik_edit.joints[0].rotation = Some(Quat::from_rotation_z(-0.4));
    let at = now + Duration::from_millis(500);
    scene.update_poses(&mut world, Vec3::ZERO, at);
    let frame = |id: Uuid| {
        let index = world.objects.index_of_uuid(&id).unwrap();
        let (p, q, _) = Scene::object_transform(&world, index, at, 0).unwrap();
        Mat4::from_rotation_translation(q, p)
            * Mat4::from_translation(-lib.pelvis)
            * world.avatar_poses[&id][head]
            * lib.rig.default_world_inv[head].inverse()
    };
    let local = rigid(frame(world.agent_id)).unwrap();
    let remote = rigid(frame(rider)).unwrap();
    assert!(local.p().distance(remote.p()) < 5e-4);
    assert!(
        local.q().dot(remote.q()).abs() > 0.99999,
        "remote targets refresh from this frame, including the local editor's new rotation"
    );
}
fn binding(bone: &str, target: Target) -> Binding {
    Binding {
        activation: Default::default(),
        bone: bone.into(),
        target,
        offset: Transform::default(),
        orient: true,
        contact: [0.0; 3],
        root: None,
        mode: Mode::Direct,
        pole: None,
        weight: 1.0,
        enabled: true,
    }
}

#[test]
fn virtual_hierarchy_and_own_bone_targets_read_the_working_pose_and_block_feedback() {
    let (world, model, mut matrices) = setup();
    let rig = &world.avatar_lib.rig;
    let id = Uuid::new_v4();
    let definition = Arc::new(Definition {
        profiles: Vec::new(),
        nodes: vec![Node {
            id,
            name: "Helper".into(),
            parent: Target::default(),
            offset: Transform::new(Vec3::X, Quat::IDENTITY),
            motion: Some(Motion::Function {
                duration: 1.0,
                playback: Playback::Once,
                function: Function::Ramp,
                position: [0.0, 0.0, 0.2],
                degrees: [0.0; 3],
                phase: 0.0,
                curve: Curve::Linear,
            }),
        }],
        bindings: vec![binding("mHead", Target::Node { id })],
    });
    let context = Arc::new(Context::new(&world, model, definition, None, 0.5, Instant::now()));
    let goal = context.goals(rig).remove(0);
    let expected = rigid(matrices[rig.pelvis])
        .unwrap()
        .compose(Transform::new(Vec3::new(1.0, 0.0, 0.1), Quat::IDENTITY));
    assert!(context.preview(&goal, &matrices).unwrap().position.abs_diff_eq(expected.p(), 1e-5));
    matrices[rig.pelvis] = Mat4::from_rotation_translation(Quat::from_rotation_z(0.7), Vec3::new(0.1, 0.3, 1.0));
    let expected = rigid(matrices[rig.pelvis])
        .unwrap()
        .compose(Transform::new(Vec3::new(1.0, 0.0, 0.1), Quat::IDENTITY));
    assert!(context.preview(&goal, &matrices).unwrap().position.abs_diff_eq(expected.p(), 1e-5));
    assert_eq!(order(rig, &[&goal]), vec![0]);
    let cyclic = Arc::new(Context::new(
        &world,
        model,
        Arc::new(Definition {
            profiles: Vec::new(),
            nodes: Vec::new(),
            bindings: vec![binding(
                "mHead",
                Target::Bone {
                    avatar: Uuid::nil(),
                    bone: "mHead".into(),
                },
            )],
        }),
        None,
        0.0,
        Instant::now(),
    ));
    let goal = cyclic.goals(rig).remove(0);
    assert!(order(rig, &[&goal]).is_empty());
}

#[test]
fn targets_that_depend_on_an_edited_parent_solve_after_it() {
    let (world, model, _) = setup();
    let rig = &world.avatar_lib.rig;
    let context = Arc::new(Context::new(
        &world,
        model,
        Arc::new(Definition {
            profiles: Vec::new(),
            nodes: Vec::new(),
            bindings: vec![binding("mHead", Target::default())],
        }),
        None,
        0.0,
        Instant::now(),
    ));
    let head = context.goals(rig).remove(0);
    let mut pelvis = head.clone();
    pelvis.bone = rig.pelvis;
    pelvis.root = rig.pelvis;
    pelvis.follow = None;
    assert_eq!(order(rig, &[&head, &pelvis]), vec![1, 0]);
    let unknown = Target::Bone {
        avatar: Uuid::new_v4(),
        bone: "mWristLeft".into(),
    };
    let context = Context::new(
        &world,
        model,
        Arc::new(Definition {
            profiles: Vec::new(),
            nodes: Vec::new(),
            bindings: vec![binding("mHead", unknown.clone())],
        }),
        None,
        0.0,
        Instant::now(),
    );
    assert!(context.target(&unknown, &[]).is_none());
}

#[test]
fn own_hand_attachment_targets_use_the_working_joint_and_reject_a_hand_following_its_own_attachment() {
    let (mut world, model, matrices) = setup();
    let rig = world.avatar_lib.rig.clone();
    let wrist = rig.names["mWristLeft"];
    let point = *world
        .avatar_lib
        .attach_points
        .iter()
        .find(|(_, a)| a.joint == Some(wrist) && !a.hud)
        .unwrap()
        .0;
    let mut objects = demo::hand_ik::objects(0.0, false);
    objects[0].parent_id = objects[3].local_id;
    objects[0].state = ((point & 15) << 4) | ((point & 240) >> 4);
    objects[0].position = Vec3::new(0.1, 0.0, 0.0);
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    let target = Target::Prim {
        id: demo::action_id(demo::hand_ik::LEFT),
        name: String::new(),
    };
    let context = Arc::new(Context::new(
        &world,
        model,
        Arc::new(Definition {
            profiles: Vec::new(),
            nodes: Vec::new(),
            bindings: vec![binding("mWristLeft", target.clone())],
        }),
        None,
        0.0,
        Instant::now(),
    ));
    let goal = context.goals(&rig).remove(0);
    assert!(order(&rig, &[&goal]).is_empty());
    let before = context.target(&target, &matrices).unwrap();
    let mut moved = matrices.clone();
    moved[wrist].w_axis.z += 0.2;
    let after = context.target(&target, &moved).unwrap();
    assert!((after.p().z - before.p().z - 0.2).abs() < 1e-5);
}

#[test]
fn named_prim_targets_release_when_unlinked_while_explicit_external_uuid_targets_still_resolve() {
    let (mut world, model, matrices) = setup();
    let source = Seat {
        root: demo::action_id(demo::hand_ik::ROOT),
        region: 1,
    };
    let named = Target::Prim {
        id: demo::action_id(demo::hand_ik::LEFT),
        name: "Left".into(),
    };
    let definition = Arc::new(Definition {
        profiles: Vec::new(),
        nodes: Vec::new(),
        bindings: vec![binding("mWristLeft", named.clone())],
    });
    assert!(
        Context::new(&world, model, definition.clone(), Some(source), 0.0, Instant::now())
            .target(&named, &matrices)
            .is_some()
    );
    let mut objects = demo::hand_ik::objects(0.0, false);
    objects[1].parent_id = 0;
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    assert!(
        Context::new(&world, model, definition, Some(source), 0.0, Instant::now())
            .target(&named, &matrices)
            .is_none()
    );
    let external = Target::Prim {
        id: demo::action_id(demo::hand_ik::LEFT),
        name: String::new(),
    };
    let definition = Arc::new(Definition {
        profiles: Vec::new(),
        nodes: Vec::new(),
        bindings: vec![binding("mWristLeft", external.clone())],
    });
    assert!(
        Context::new(&world, model, definition, Some(source), 0.0, Instant::now())
            .target(&external, &matrices)
            .is_some()
    );
    let missing = Target::Prim {
        id: Uuid::nil(),
        name: "Missing link".into(),
    };
    assert!(missing.valid());
    assert!(
        !Target::Prim {
            id: Uuid::nil(),
            name: String::new()
        }
        .valid()
    );
}
