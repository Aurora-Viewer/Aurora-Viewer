use super::*;
use crate::{
    scene::avatar::AvatarLibrary,
    world::{World, hand_ik::ActivationStamp, rig_graph::Definition},
};
use glam::Mat4;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

fn source(rig: &Rig) -> LocalPose {
    LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    }
}

fn stamp(epoch: u128) -> ActivationStamp {
    ActivationStamp {
        session: Uuid::from_u128(1),
        epoch: Uuid::from_u128(epoch),
        bundle: Uuid::from_u128(2),
        cycles: [0; 16],
    }
}

fn goals(world: &World, names: &[&str], activation: Activation, epoch: u128) -> Vec<Goal> {
    let rig = &world.avatar_lib.rig;
    let mut author = source(rig);
    author.rotations[rig.names["mTorso"]] *= Quat::from_rotation_y(0.45);
    let matrices = rig.world_with(&author.rotations, Vec3::ZERO, &author.positions, &rig.scale);
    let pelvis = matrices[rig.pelvis];
    let bindings: Vec<_> = names
        .iter()
        .map(|name| {
            let bone = rig.names[*name];
            let relative = graph::rigid(pelvis.inverse() * matrices[bone]).unwrap();
            serde_json::json!({"bone":name,"root":"mTorso","mode":if name.starts_with("mWrist") {"ik"} else {"direct"},
            "target":{"kind":"bone","bone":"mPelvis"},"offset":relative,"activation":activation})
        })
        .collect();
    let definition: Definition = serde_json::from_value(serde_json::json!({"bindings":bindings})).unwrap();
    assert!(definition.validate().is_ok());
    Arc::new(graph::Context::new(world, Mat4::IDENTITY, Arc::new(definition), None, 0.0, Instant::now()).activated(Some(stamp(epoch))))
        .goals(rig)
}

fn apply(state: &mut Pose, rig: &Rig, base: &LocalPose, at: Instant) -> Option<LocalPose> {
    state.apply(rig, &base.rotations, &base.positions, Vec3::ZERO, &rig.scale, at, false)
}

#[test]
fn activation_distance_starts_on_first_contact_without_a_viewer_delay_and_stays_latched() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let mut settings = Activation {
        distance: 0.05,
        ease_in: 0.0,
        ease_out: 0.0,
    };
    let goal = goals(&world, &["mHead"], settings, 3).remove(0);
    let mut base = source(rig);
    let desired = goal
        .follow
        .as_ref()
        .unwrap()
        .resolve(&goal, rig, &base, Vec3::ZERO, &rig.scale)
        .unwrap();
    let original = rig.world_with(&base.rotations, Vec3::ZERO, &base.positions, &rig.scale)[goal.bone]
        .w_axis
        .truncate();
    let parent = rig.parent[goal.bone].unwrap();
    let parent_q = rig.world_with(&base.rotations, Vec3::ZERO, &base.positions, &rig.scale)[parent]
        .to_scale_rotation_translation()
        .1;
    let near = parent_q.inverse() * (desired.position - original);
    let mut held = Held::new(&goal);
    for _ in 0..10 {
        held.update(Some(&goal), rig, &base, Vec3::ZERO, &rig.scale, 0.05);
        assert_eq!(held.weight, 0.0);
    }
    base.positions[goal.bone] += near;
    held.update(Some(&goal), rig, &base, Vec3::ZERO, &rig.scale, 0.0);
    assert_eq!(held.weight, 1.0);
    base.positions[goal.bone] -= near;
    held.update(Some(&goal), rig, &base, Vec3::ZERO, &rig.scale, 0.05);
    assert_eq!(held.weight, 1.0, "distance is an entry gate, not a break distance");
    settings.distance = 0.0;
    let goal = goals(&world, &["mHead"], settings, 4).remove(0);
    held.update(Some(&goal), rig, &base, Vec3::ZERO, &rig.scale, 0.0);
    assert_eq!(held.weight, 1.0, "zero distance/fade starts immediately");
    // An installed declaration must not preserve an obsolete viewer delay.
    let settings: Activation = serde_json::from_str("{\"delay\":3600,\"ease_in\":0}").unwrap();
    let goal = goals(&world, &["mHead"], settings, 5).remove(0);
    held.update(Some(&goal), rig, &base, Vec3::ZERO, &rig.scale, 0.0);
    assert_eq!(held.weight, 1.0);
}

#[test]
fn independent_activation_and_release_fades_follow_the_current_animation_without_a_release_jump() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let base = source(rig);
    let mut state = Pose {
        targets: goals(
            &world,
            &["mHead"],
            Activation {
                ease_in: 0.4,
                ease_out: 0.8,
                ..Default::default()
            },
            3,
        ),
        ..Default::default()
    };
    let bone = state.targets[0].bone;
    let now = Instant::now();
    let mut solved = None;
    for frame in 0..=4 {
        solved = apply(&mut state, rig, &base, now + Duration::from_millis(frame * 100));
        if frame == 2 {
            assert!((state.held[0].weight - 0.5).abs() < 1e-5);
        }
    }
    let solved = solved.unwrap();
    let position = solved.positions[bone] - base.positions[bone];
    assert!(position.length() > 0.05);
    state.targets.clear();
    let mut animated = base.clone();
    animated.rotations[bone] *= Quat::from_rotation_x(0.1);
    animated.positions[bone] += Vec3::Y * 0.02;
    let released = apply(&mut state, rig, &animated, now + Duration::from_millis(500)).unwrap();
    assert!(released.rotations[bone].dot(solved.rotations[bone]).abs() > 0.999999);
    assert!(
        released.positions[bone].abs_diff_eq(solved.positions[bone], 1e-6),
        "an animation cut at release must still start from the displayed pose"
    );
    for frame in 1..=4 {
        let released = apply(&mut state, rig, &animated, now + Duration::from_millis(500 + frame * 100)).unwrap();
        if frame == 4 {
            assert!(released.positions[bone].abs_diff_eq(animated.positions[bone].lerp(solved.positions[bone], 0.5), 1e-5));
            let expected = animated.rotations[bone].slerp(solved.rotations[bone], 0.5);
            assert!(released.rotations[bone].dot(expected).abs() > 0.999999);
        }
    }
    let mut last = Some(released);
    for frame in 5..=9 {
        last = apply(&mut state, rig, &animated, now + Duration::from_millis(500 + frame * 100));
    }
    assert!(last.is_none(), "the independent release duration must fully restore animation");
}

#[test]
fn restarted_body_contacts_match_a_fresh_pose_even_without_rendering_the_off_packet() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let base = source(rig);
    let names = ["mWristLeft", "mWristRight"];
    let now = Instant::now();
    for render_off in [false, true] {
        let mut state = Pose {
            targets: goals(&world, &names, Activation::default(), 3),
            ..Default::default()
        };
        let mut last = None;
        for frame in 0..120 {
            last = apply(&mut state, rig, &base, now + Duration::from_secs_f32(frame as f32 / 120.0));
        }
        let torso = rig.names["mTorso"];
        assert!(2.0 * base.rotations[torso].dot(last.unwrap().rotations[torso]).abs().min(1.0).acos() > 0.05);
        if render_off {
            state.targets.clear();
            assert!(apply(&mut state, rig, &base, now + Duration::from_secs(1)).is_some());
        }
        state.targets = goals(&world, &names, Activation::default(), 4);
        let mut fresh = Pose {
            targets: state.targets.clone(),
            ..Default::default()
        };
        for frame in 0..60 {
            let at = now + Duration::from_secs_f32(1.1 + frame as f32 / 120.0);
            let reused = apply(&mut state, rig, &base, at).unwrap();
            let expected = apply(&mut fresh, rig, &base, at).unwrap();
            for (actual, expected) in reused.rotations.iter().zip(&expected.rotations) {
                assert!(
                    actual.dot(*expected).abs() > 0.999999,
                    "fresh activation differs after off-render={render_off}, frame={frame}"
                );
            }
            assert_eq!(reused.positions, expected.positions);
        }
        let heartbeat = state.targets[0].follow.as_ref().unwrap().context.refreshed(&world, now);
        state.targets = heartbeat.goals(rig);
        apply(&mut state, rig, &base, now + Duration::from_secs(2));
        assert!(state.held.iter().all(|h| h.weight == 1.0), "heartbeats preserve the activation");
        let mut masked = stamp(4);
        masked.cycles[0] = 1;
        let context = (*heartbeat).clone().activated(Some(masked));
        state.targets = Arc::new(context).goals(rig);
        apply(&mut state, rig, &base, now + Duration::from_millis(2010));
        assert_eq!(state.held[0].weight, 0.0);
        assert_eq!(state.held[1].weight, 1.0, "a per-bone restart preserves other activation clocks");
    }
}

#[test]
fn bindings_keep_independent_fade_durations_and_zero_release_drops_the_correction_immediately() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let base = source(rig);
    let mut targets = goals(
        &world,
        &["mHead", "mWristLeft"],
        Activation {
            ease_in: 0.0,
            ease_out: 0.8,
            ..Default::default()
        },
        3,
    );
    let mut context = (*targets[0].follow.as_ref().unwrap().context).clone();
    Arc::make_mut(&mut context.definition).bindings[1].activation.ease_out = 0.2;
    targets = Arc::new(context).goals(rig);
    let mut state = Pose {
        targets,
        ..Default::default()
    };
    let now = Instant::now();
    apply(&mut state, rig, &base, now).unwrap();
    assert!(state.held.iter().all(|held| held.weight == 1.0));
    state.targets.clear();
    apply(&mut state, rig, &base, now + Duration::from_millis(100)).unwrap();
    for frame in 2..=3 {
        apply(&mut state, rig, &base, now + Duration::from_millis(frame * 100)).unwrap();
    }
    assert_eq!(state.held.len(), 1);
    assert_eq!(state.held[0].goal.bone, rig.names["mHead"]);
    for frame in 4..=10 {
        apply(&mut state, rig, &base, now + Duration::from_millis(frame * 100));
    }
    assert!(state.held.is_empty());
    state.targets = goals(
        &world,
        &["mHead"],
        Activation {
            ease_in: 0.0,
            ease_out: 0.0,
            ..Default::default()
        },
        4,
    );
    apply(&mut state, rig, &base, now + Duration::from_secs(2)).unwrap();
    state.targets.clear();
    assert!(apply(&mut state, rig, &base, now + Duration::from_millis(2010)).is_none());
}

#[test]
fn activation_timing_uses_elapsed_seconds_and_suspends_during_an_animation_pause() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let base = source(rig);
    let mut state = Pose {
        targets: goals(
            &world,
            &["mHead"],
            Activation {
                ease_in: 0.5,
                ease_out: 0.6,
                ..Default::default()
            },
            3,
        ),
        ..Default::default()
    };
    let now = Instant::now();
    apply(&mut state, rig, &base, now).unwrap();
    apply(&mut state, rig, &base, now + Duration::from_secs(1)).unwrap();
    assert_eq!(
        state.held[0].weight, 1.0,
        "a slow frame must not extend the fade measured in seconds"
    );
    state.targets.clear();
    apply(&mut state, rig, &base, now + Duration::from_secs(2)).unwrap();
    assert!(
        state
            .apply(
                rig,
                &base.rotations,
                &base.positions,
                Vec3::ZERO,
                &rig.scale,
                now + Duration::from_secs(3),
                true
            )
            .is_some()
    );
    assert!(
        apply(&mut state, rig, &base, now + Duration::from_secs(10)).is_some(),
        "resuming must not count the paused duration"
    );
    assert!(apply(&mut state, rig, &base, now + Duration::from_millis(10601)).is_none());
}

#[test]
fn closing_the_editor_does_not_bake_manual_pose_channels_into_a_script_release() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let base = source(rig);
    let mut state = Pose {
        targets: goals(
            &world,
            &["mHead"],
            Activation {
                ease_in: 0.0,
                ease_out: 0.8,
                ..Default::default()
            },
            3,
        ),
        ..Default::default()
    };
    let now = Instant::now();
    let live = apply(&mut state, rig, &base, now).unwrap();
    state.edit = EditRequest {
        active: true,
        joints: vec![JointEdit {
            bone: rig.names["mTorso"],
            rotation: Some(Quat::from_rotation_y(0.8)),
            translation: None,
            weight: 1.0,
        }],
        ..Default::default()
    };
    apply(&mut state, rig, &base, now + Duration::from_millis(100)).unwrap();
    state.edit = EditRequest::default();
    state.targets.clear();
    let released = apply(&mut state, rig, &base, now + Duration::from_millis(200)).unwrap();
    let head = rig.names["mHead"];
    assert!(released.rotations[head].dot(live.rotations[head]).abs() > 0.999999);
    assert!(released.positions[head].abs_diff_eq(live.positions[head], 1e-5));
    assert_eq!(released.rotations[rig.names["mTorso"]], rig.local_rot[rig.names["mTorso"]]);
}

#[test]
fn a_new_activation_with_an_unavailable_target_discards_the_previous_release() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let base = source(rig);
    let settings = Activation {
        ease_in: 0.0,
        ease_out: 1.0,
        ..Default::default()
    };
    let mut state = Pose {
        targets: goals(&world, &["mHead"], settings, 3),
        ..Default::default()
    };
    let now = Instant::now();
    apply(&mut state, rig, &base, now).unwrap();
    let definition: Definition = serde_json::from_value(serde_json::json!({"bindings":[{
        "bone":"mHead","mode":"direct","activation":settings,
        "target":{"kind":"prim","id":Uuid::from_u128(0xdead)}
    }]}))
    .unwrap();
    let context = Arc::new(graph::Context::new(&world, Mat4::IDENTITY, Arc::new(definition), None, 0.0, now).activated(Some(stamp(4))));
    state.targets = context.goals(rig);
    assert!(apply(&mut state, rig, &base, now + Duration::from_millis(10)).is_none());
    assert!(
        state.held.is_empty(),
        "a missing new target must not retain the old activation's correction"
    );
}

#[test]
fn rejected_bone_cycles_cannot_cache_or_extend_another_contacts_body_correction() {
    let world = World::new(Arc::new(AvatarLibrary::load()));
    let rig = &world.avatar_lib.rig;
    let base = source(rig);
    let targets = goals(
        &world,
        &["mHead", "mWristLeft"],
        Activation {
            ease_in: 0.0,
            ease_out: 1.0,
            ..Default::default()
        },
        3,
    );
    let mut definition = (*targets[0].follow.as_ref().unwrap().context.definition).clone();
    definition.bindings[0].mode = Mode::Ik;
    definition.bindings[0].target = crate::world::rig_graph::Target::Bone {
        avatar: Uuid::nil(),
        bone: "mHead".into(),
    };
    definition.bindings[1].activation.ease_out = 0.2;
    let now = Instant::now();
    let context = Arc::new(graph::Context::new(&world, Mat4::IDENTITY, Arc::new(definition), None, 0.0, now).activated(Some(stamp(3))));
    let mut state = Pose {
        targets: context.goals(rig),
        ..Default::default()
    };
    apply(&mut state, rig, &base, now).unwrap();
    assert!(
        state
            .held
            .iter()
            .find(|h| h.goal.bone == rig.names["mHead"])
            .unwrap()
            .correction
            .is_empty()
    );
    assert!(
        !state
            .held
            .iter()
            .find(|h| h.goal.bone == rig.names["mWristLeft"])
            .unwrap()
            .correction
            .is_empty()
    );
    state.targets.clear();
    apply(&mut state, rig, &base, now + Duration::from_millis(100)).unwrap();
    assert!(apply(&mut state, rig, &base, now + Duration::from_millis(400)).is_none());
}
