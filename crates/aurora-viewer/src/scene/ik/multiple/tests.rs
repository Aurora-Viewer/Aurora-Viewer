use super::*;
use crate::{
    scene::avatar::AvatarLibrary,
    world::{World, rig_graph::Definition},
};
use std::sync::Arc;

#[test]
fn multiple_targets_blend_independent_material_points_and_rotations_without_row_order_bias() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bone = rig.names["mWristRight"];
    let pose = LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    };
    let q = Quat::from_rotation_z(0.4);
    let origin = Vec3::new(0.2, -0.4, 1.1);
    let mut a = Goal {
        follow: None,
        weight: 0.25,
        bone,
        root: rig.names["mShoulderRight"],
        position: origin,
        rotation: Some(q),
        contact: Vec3::new(0.1, 0.02, 0.0),
        pole: None,
        mode: Mode::Ik,
    };
    a.position += q * (a.contact * rig.scale[bone]);
    let mut b = a.clone();
    b.weight = 0.75;
    b.rotation = Some(-q);
    b.contact = Vec3::new(-0.05, 0.0, 0.06);
    b.position = origin + Vec3::X * 0.2 + q * (b.contact * rig.scale[bone]);
    let expected = origin + Vec3::X * 0.15;
    for peers in [[(&a, 1.0), (&b, 1.0)], [(&b, 1.0), (&a, 1.0)]] {
        let merged = combine(rig, &pose, Vec3::ZERO, &rig.scale, &peers).unwrap();
        assert!(merged.position.abs_diff_eq(expected, 1e-6));
        assert_eq!(merged.contact, Vec3::ZERO);
        assert!(merged.rotation.unwrap().dot(q).abs() > 0.999999);
        assert_eq!(merged.weight, 1.0);
    }
    let merged = combine(rig, &pose, Vec3::ZERO, &rig.scale, &[(&a, 0.5), (&b, 0.0)]).unwrap();
    assert!((merged.weight - 0.125).abs() < 1e-6);
    assert!(merged.position.abs_diff_eq(origin, 1e-6));
}

#[test]
fn runtime_same_bone_bindings_keep_separate_fades_cycles_and_release_without_removing_the_peer() {
    use crate::world::hand_ik::ActivationStamp;
    use glam::Mat4;
    use std::time::Duration;
    use uuid::Uuid;
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let definition: Definition = serde_json::from_value(serde_json::json!({"bindings":[
        {"bone":"mHead","mode":"direct","target":{"kind":"bone","bone":"mPelvis"},"offset":{"position":[0.0,0.0,0.7],"rotation":[0,0,0,1]},"activation":{"ease_in":0.0,"ease_out":0.3}},
        {"bone":"mHead","mode":"direct","target":{"kind":"bone","bone":"mPelvis"},"offset":{"position":[0.2,0.0,0.7],"rotation":[0,0,0,1]},"activation":{"ease_in":0.4,"ease_out":0.3}}
    ]})).unwrap();
    assert_eq!(definition.validate(), Ok(()));
    let stamp = ActivationStamp {
        session: Uuid::from_u128(1),
        epoch: Uuid::from_u128(2),
        bundle: Uuid::from_u128(3),
        cycles: [0; 16],
    };
    let context =
        Arc::new(graph::Context::new(&world, Mat4::IDENTITY, Arc::new(definition), None, 0.0, Instant::now()).activated(Some(stamp)));
    let mut state = Pose {
        targets: context.goals(rig),
        ..Default::default()
    };
    let apply = |state: &mut Pose, at| {
        state
            .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, at, false)
            .unwrap()
    };
    let now = Instant::now();
    apply(&mut state, now);
    assert_eq!(state.held.len(), 2);
    assert_eq!((state.held[0].weight, state.held[1].weight), (1.0, 0.0));
    apply(&mut state, now + Duration::from_millis(200));
    assert!((state.held[1].weight - 0.5).abs() < 1e-5);
    let solved = apply(&mut state, now + Duration::from_millis(400));
    let head = rig.names["mHead"];
    let frames = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
    assert!((frames[head].w_axis.x - frames[rig.pelvis].w_axis.x - 0.1).abs() < 1e-5);
    state.targets.remove(0);
    let released = apply(&mut state, now + Duration::from_millis(450));
    assert!(
        (released.positions[head].x - solved.positions[head].x).abs() < 1e-5,
        "release begins without a target jump"
    );
    assert_eq!(state.held.len(), 2);
    assert_eq!(state.held[1].weight, 1.0);
    let midway = apply(&mut state, now + Duration::from_millis(600));
    let frames = rig.world_with(&midway.rotations, Vec3::ZERO, &midway.positions, &rig.scale);
    assert!((frames[head].w_axis.x - frames[rig.pelvis].w_axis.x - 0.2 / 1.5).abs() < 1e-5);
    apply(&mut state, now + Duration::from_millis(800));
    assert_eq!(state.held.len(), 1);
    assert_eq!(state.held[0].goal.follow.as_ref().unwrap().index, 1);
}

#[test]
fn same_bone_peers_wait_for_every_target_dependency_before_the_group_is_solved() {
    use glam::Mat4;
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let definition: Definition = serde_json::from_value(serde_json::json!({"bindings":[
        {"bone":"mHead","mode":"direct","target":{"kind":"bone","bone":"mPelvis"}},
        {"bone":"mWristRight","mode":"direct","target":{"kind":"bone","bone":"mPelvis"}},
        {"bone":"mHead","mode":"direct","target":{"kind":"bone","bone":"mWristRight"}}
    ]}))
    .unwrap();
    let context = Arc::new(graph::Context::new(
        &world,
        Mat4::IDENTITY,
        Arc::new(definition),
        None,
        0.0,
        Instant::now(),
    ));
    let goals = context.goals(rig);
    let order = graph::order(rig, &goals.iter().collect::<Vec<_>>());
    assert_eq!(order.len(), 3);
    assert_eq!(order[0], 1, "the apparently independent peer also waits for the wrist writer");
}
