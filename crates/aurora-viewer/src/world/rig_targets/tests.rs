use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::NetEvent;
use std::sync::Arc;

#[test]
fn cold_prim_names_resolve_when_family_replies_are_root_only_or_absent() {
    use crate::build::BuildTool;
    use aurora_net::{NetCommand, build::BuildCmd};

    for worn in [false, true] {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = demo::DEMO_AGENT;
        world.main_region = Some(1);
        let mut objects = demo::hand_ik::objects(0.0, !worn);
        if worn {
            objects[0].parent_id = objects[3].local_id;
        }
        for part in 0..3 {
            let mut child = objects[1].clone();
            child.local_id += 10 + part;
            child.full_id = demo::action_id(child.local_id);
            objects.push(child);
        }
        let props: HashMap<_, _> = objects
            .iter()
            .filter(|o| o.pcode != 47)
            .map(|o| {
                let name = match o.local_id {
                    demo::hand_ik::ROOT => "Glasses".into(),
                    demo::hand_ik::LEFT => "IK Left".into(),
                    demo::hand_ik::RIGHT => "IK Right".into(),
                    _ => format!("Decoration {}", o.local_id),
                };
                (
                    o.full_id,
                    ObjectProps {
                        object_id: o.full_id,
                        name,
                        ..Default::default()
                    },
                )
            })
            .collect();
        world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
        let root = demo::action_id(demo::hand_ik::ROOT);
        let wanted = [(
            root,
            Requested {
                names: ["IK Left".into(), "IK Right".into()].into(),
                ..Default::default()
            },
        )]
        .into();
        let now = Instant::now();
        let build = BuildTool::default();
        let reply = |command: &NetCommand| -> Vec<ObjectProps> {
            match command {
                // Reproduce the real report: family reads return only root
                // metadata, even when the request names a child UUID.
                NetCommand::RequestObjectProperties { .. } => vec![props[&root].clone()],
                NetCommand::Build(BuildCmd::Select { local_ids, .. }) => local_ids
                    .iter()
                    .filter_map(|id| props.get(&demo::action_id(*id)).cloned())
                    .collect(),
                _ => Vec::new(),
            }
        };
        let mut old = Targets::default();
        for tick in 0..200 {
            if let Some(key) = old.sync(&world, &wanted, &HashMap::new(), now + SPACING * tick) {
                let object = world.objects.get(world.objects.index_of(&key).unwrap()).unwrap();
                old.on_properties(&reply(&NetCommand::RequestObjectProperties {
                    handle: key.region,
                    object: object.full_id,
                }));
            }
        }
        assert_eq!(old.prims.values().filter(|p| p.name.is_some()).count(), 1);
        assert_eq!(old.prims.values().map(|p| usize::from(p.attempts)).sum::<usize>(), 16);
        assert!(old.index(&world, None, root, "IK Left").is_none());
        let individual_reply = |command: &NetCommand| -> Vec<ObjectProps> {
            if matches!(command, NetCommand::RequestObjectProperties { .. }) {
                Vec::new()
            } else {
                reply(command)
            }
        };
        // The child-only repair still depends on a root hover response.
        // A missing root response reproduces the next report: 5/6, 8 attempts.
        let mut partial = Targets::default();
        for tick in 0..200 {
            if let Some(key) = partial.sync(&world, &wanted, &HashMap::new(), now + SPACING * tick) {
                let object = world.objects.get(world.objects.index_of(&key).unwrap()).unwrap();
                if object.full_id == root {
                    partial.on_properties(&individual_reply(&NetCommand::RequestObjectProperties {
                        handle: key.region,
                        object: root,
                    }));
                } else {
                    for command in build.prim_properties_query(&world, key) {
                        partial.on_properties(&individual_reply(&command));
                    }
                }
            }
        }
        assert_eq!(partial.prims.values().filter(|p| p.name.is_some()).count(), 5);
        assert_eq!(partial.prims.values().map(|p| usize::from(p.attempts)).sum::<usize>(), 8);
        assert!(
            partial.index(&world, None, root, "IK Left").is_none(),
            "unknown root can duplicate a child name"
        );
        let mut targets = Targets::default();
        for frame in 0..200 {
            if let Some(key) = targets.sync(&world, &wanted, &HashMap::new(), now + Duration::from_millis(frame * 8)) {
                for command in build.prim_properties_query(&world, key) {
                    targets.on_properties(&individual_reply(&command));
                }
            }
        }
        for (name, id) in [("IK Left", demo::hand_ik::LEFT), ("IK Right", demo::hand_ik::RIGHT)] {
            let index = targets.index(&world, None, root, name).expect("all six individual names received");
            assert_eq!(world.objects.get(index).unwrap().full_id, demo::action_id(id));
        }
        assert!(
            targets
                .sync(&world, &wanted, &HashMap::new(), now + Duration::from_secs(60))
                .is_none()
        );
        assert_eq!(targets.prims.values().map(|p| usize::from(p.attempts)).sum::<usize>(), 6);
        assert!(build.selection.is_empty() && build.out.is_empty());
    }
}

const SPACING: Duration = Duration::from_millis(250);

#[test]
fn missing_name_diagnostics_identify_roots_and_children_without_object_ids() {
    let (mut world, mut props, root, _) = fixture();
    let now = Instant::now();
    props.remove(&root);
    sync(&mut world, &[root].into(), &props, now);
    let report = world.ik_targets.describe_link(&world, Seat { root, region: 1 }, root, "Grip");
    assert!(
        report.contains("waiting for names 2/3") && report.contains("missing=root:1"),
        "{report}"
    );
    assert!(!report.contains(&root.to_string()));
    let (mut world, mut props, root, _) = fixture();
    let child = demo::action_id(demo::hand_ik::RIGHT);
    props.remove(&child);
    sync(&mut world, &[root].into(), &props, now);
    let report = world.ik_targets.describe_link(&world, Seat { root, region: 1 }, root, "Grip");
    assert!(
        report.contains("waiting for names 2/3") && report.contains("missing=child["),
        "{report}"
    );
    assert!(!report.contains(&child.to_string()));
    assert!(world.ik_targets.index(&world, None, root, "Grip").is_none());
}

#[test]
fn target_diagnostics_distinguish_waiting_resolved_duplicate_and_absent_links() {
    let (mut world, mut props, root, _) = fixture();
    let now = Instant::now();
    let source = Seat { root, region: 1 };
    assert_eq!(world.ik_targets.describe_link(&world, source, root, "Grip"), "not requested");
    sync(&mut world, &[root].into(), &HashMap::new(), now);
    assert!(
        world
            .ik_targets
            .describe_link(&world, source, root, "Grip")
            .starts_with("waiting for names 0/3")
    );
    sync(&mut world, &[root].into(), &props, now);
    assert_eq!(world.ik_targets.describe_link(&world, source, root, "Grip"), "resolved");
    props.get_mut(&demo::action_id(demo::hand_ik::RIGHT)).unwrap().name = "Grip".into();
    sync(&mut world, &[root].into(), &props, now);
    assert_eq!(world.ik_targets.describe_link(&world, source, root, "Grip"), "duplicate name");
    assert_eq!(
        world.ik_targets.describe_link(&world, source, Uuid::new_v4(), "Grip"),
        "object absent"
    );
}

#[test]
fn cold_names_resolve_on_consecutive_frames_without_waiting_for_each_quarter_second() {
    let (mut world, props, root, _) = fixture();
    let now = Instant::now();
    let mut received = HashMap::new();
    for frame in 0..3 {
        let at = now + Duration::from_millis(frame * 8);
        let command = sync(&mut world, &[root].into(), &received, at).expect("cold property request");
        let object = world.objects.get(world.objects.index_of(&command).unwrap()).unwrap().full_id;
        received.insert(object, props[&object].clone());
        if frame < 2 {
            assert!(
                world.ik_targets.index(&world, None, root, "Grip").is_none(),
                "uniqueness still needs all names"
            );
        }
    }
    assert!(sync(&mut world, &[root].into(), &received, now + Duration::from_millis(24)).is_none());
    assert!(world.ik_targets.index(&world, None, root, "Grip").is_some());
    for second in 1..60 {
        assert!(sync(&mut world, &[root].into(), &received, now + Duration::from_secs(second)).is_none());
    }
}

#[test]
fn immediate_queries_share_a_rolling_four_per_second_budget_across_roots_and_retirement() {
    let (mut world, _, root, other) = fixture();
    let now = Instant::now();
    let roots = [root, other].into();
    let mut sent = Vec::new();
    for millis in 0..3000 {
        let at = now + Duration::from_millis(millis);
        if millis == 100 {
            assert!(sync(&mut world, &BTreeSet::new(), &HashMap::new(), at).is_none());
        }
        if sync(&mut world, &roots, &HashMap::new(), at).is_some() {
            sent.push(millis);
            assert!(sent.iter().filter(|previous| millis - **previous < 1000).count() <= 4);
        }
    }
    assert_eq!(&sent[..4], &[0, 1, 2, 3]);
    assert_eq!(sent[4], 1000, "retiring/redeclaring roots cannot refill the budget");
    assert!(world.ik_targets.queries.len() <= MAX_QUERIES);
}

fn fixture() -> (World, HashMap<Uuid, ObjectProps>, Uuid, Uuid) {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    let mut objects = demo::hand_ik::objects(0.0, true);
    let root = demo::action_id(demo::hand_ik::ROOT);
    let other = demo::action_id(demo::hand_ik::ROOT + 100);
    for mut object in objects.clone().into_iter().take(3) {
        object.local_id += 100;
        object.full_id = demo::action_id(object.local_id);
        if object.parent_id != 0 {
            object.parent_id += 100;
        }
        objects.push(object);
    }
    let props = objects
        .iter()
        .filter(|o| o.pcode != 47)
        .map(|o| {
            (
                o.full_id,
                ObjectProps {
                    object_id: o.full_id,
                    name: if o.parent_id == 0 {
                        "Object"
                    } else if o.local_id % 100 == demo::hand_ik::LEFT % 100 {
                        "Grip"
                    } else {
                        ROOT_LINK
                    }
                    .into(),
                    ..Default::default()
                },
            )
        })
        .collect();
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    (world, props, root, other)
}

fn sync(world: &mut World, roots: &BTreeSet<Uuid>, props: &HashMap<Uuid, ObjectProps>, now: Instant) -> Option<ObjKey> {
    let mut targets = std::mem::take(&mut world.ik_targets);
    targets.on_properties(&props.values().cloned().collect::<Vec<_>>());
    let requested = roots
        .iter()
        .map(|id| {
            (
                *id,
                Requested {
                    names: ["Grip".into()].into(),
                    ..Default::default()
                },
            )
        })
        .collect();
    let result = targets.sync(world, &requested, props, now);
    world.ik_targets = targets;
    result
}

#[test]
fn names_are_scoped_to_each_declared_root_and_root_token_never_searches_names() {
    let (mut world, props, root, other) = fixture();
    let now = Instant::now();
    assert!(sync(&mut world, &[root, other].into(), &props, now).is_none());
    let source = Some(Seat { root, region: 1 });
    for (object, expected) in [(Uuid::nil(), root), (root, root), (other, other)] {
        let index = world.ik_targets.index(&world, source, object, ROOT_LINK).unwrap();
        assert_eq!(world.objects.get(index).unwrap().full_id, expected);
        let index = world.ik_targets.index(&world, source, object, "Grip").unwrap();
        assert_eq!(
            world.objects.get(index).unwrap().parent_id,
            if expected == root {
                demo::hand_ik::ROOT
            } else {
                demo::hand_ik::ROOT + 100
            }
        );
    }
    assert!(world.ik_targets.index(&world, None, Uuid::nil(), ROOT_LINK).is_none());
    assert!(
        world
            .ik_targets
            .index(&world, source, demo::action_id(demo::hand_ik::LEFT), ROOT_LINK)
            .is_none()
    );
    assert!(world.ik_targets.index(&world, source, world.agent_id, ROOT_LINK).is_none());
    assert!(
        world
            .ik_targets
            .index(&world, Some(Seat { root, region: 2 }), other, "Grip")
            .is_none()
    );
}

#[test]
fn incomplete_duplicate_and_unlinked_names_release_without_affecting_other_objects() {
    let (mut world, mut props, root, other) = fixture();
    let now = Instant::now();
    let child = demo::action_id(demo::hand_ik::RIGHT);
    let missing = props.remove(&child).unwrap();
    sync(&mut world, &[root, other].into(), &props, now);
    assert!(world.ik_targets.index(&world, None, root, "Grip").is_none());
    assert!(world.ik_targets.index(&world, None, other, "Grip").is_some());
    props.insert(
        child,
        ObjectProps {
            name: "Grip".into(),
            ..missing
        },
    );
    sync(&mut world, &[root, other].into(), &props, now);
    assert!(world.ik_targets.index(&world, None, root, "Grip").is_none());
    props.get_mut(&child).unwrap().name = "Different".into();
    sync(&mut world, &[root, other].into(), &props, now);
    let selected = world.ik_targets.index(&world, None, root, "Grip").unwrap();
    world.objects.get_mut(selected).unwrap().parent_id = 0;
    assert!(world.ik_targets.index(&world, None, root, "Grip").is_none());
    assert!(world.ik_targets.index(&world, None, other, "Grip").is_some());
}

#[test]
fn property_queries_are_shared_finite_and_stop_after_success_or_source_retirement() {
    let (mut world, props, root, other) = fixture();
    let now = Instant::now();
    let roots = [root, other].into();
    let mut count = 0;
    for tick in 0..200 {
        count += usize::from(sync(&mut world, &roots, &HashMap::new(), now + SPACING * tick).is_some());
    }
    assert_eq!(count, 18, "three finite attempts for each of six prims");
    assert!(sync(&mut world, &roots, &props, now + Duration::from_secs(60)).is_none());
    for tick in 0..10 {
        assert!(sync(&mut world, &roots, &props, now + Duration::from_secs(100 + tick)).is_none());
    }
    sync(&mut world, &BTreeSet::new(), &props, now);
    assert!(world.ik_targets.prims.is_empty() && world.ik_targets.roots.is_empty());
    assert!(
        world.ik_targets.index(&world, None, root, ROOT_LINK).is_some(),
        "root needs no property cache"
    );
}

#[test]
fn region_changes_clear_names_and_teleports_release_all_targets_including_roots() {
    let (mut world, props, root, _) = fixture();
    let now = Instant::now();
    sync(&mut world, &[root].into(), &props, now);
    world.teleporting = true;
    sync(&mut world, &[root].into(), &props, now);
    assert!(world.ik_targets.prims.is_empty());
    assert!(world.ik_targets.index(&world, None, root, ROOT_LINK).is_none());
    world.teleporting = false;
    world.main_region = Some(2);
    sync(&mut world, &[root].into(), &props, now);
    assert!(world.ik_targets.roots.is_empty());
}

#[test]
fn two_object_links_and_virtual_root_parents_keep_the_same_metric_frames_as_concrete_picks() {
    use crate::{scene::ik::graph::Context, world::rig_graph::Transform};
    use glam::{Mat4, Quat, Vec3};
    let (mut world, props, root, other) = fixture();
    let now = Instant::now();
    sync(&mut world, &[root, other].into(), &props, now);
    let source = Some(Seat { root, region: 1 });
    let model = Mat4::from_rotation_translation(Quat::from_rotation_z(0.7), Vec3::new(130.0, 126.0, 0.0));
    let rig = &world.avatar_lib.rig;
    let matrices = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    for (object, id, link) in [
        (Uuid::nil(), demo::action_id(demo::hand_ik::LEFT), "Grip"),
        (other, demo::action_id(demo::hand_ik::LEFT + 100), "Grip"),
        (other, other, ROOT_LINK),
    ] {
        let target = Target::Link { object, link: link.into() };
        let concrete = Target::Prim { id, name: String::new() };
        let node = Uuid::from_u128(42);
        let offset = Transform::new(Vec3::new(0.04, -0.02, 0.03), Quat::from_rotation_y(0.4));
        let definition: Definition = serde_json::from_value(serde_json::json!({
            "nodes":[{"id":node,"name":"Control","parent":target,"offset":offset}],
            "bindings":[{"bone":"mWristLeft","target":target},{"bone":"mWristRight","target":concrete}]
        }))
        .unwrap();
        let context = Context::new(&world, model, Arc::new(definition), source, 0.0, now);
        let named = context.target(&target, &matrices).unwrap();
        let exact = context.target(&concrete, &matrices).unwrap();
        assert!(named.p().abs_diff_eq(exact.p(), 1e-5));
        assert!(named.q().abs_diff_eq(exact.q(), 1e-5));
        let control = context.target(&Target::Node { id: node }, &matrices).unwrap();
        assert!(control.p().abs_diff_eq(exact.compose(offset).p(), 1e-5));
        assert_eq!(context.definition.bindings.len(), 2);
    }
}

#[test]
fn metadata_storage_is_bounded_even_when_definitions_reference_many_large_linksets() {
    let (mut world, _, root, _) = fixture();
    let base = demo::hand_ik::objects(0.0, true)[0].clone();
    let mut objects = Vec::new();
    let mut roots = BTreeSet::new();
    for group in 0..5 {
        let id = 20_000 + group * 300;
        roots.insert(demo::action_id(id));
        for part in 0..256 {
            let mut object = base.clone();
            object.local_id = id + part;
            object.full_id = demo::action_id(id + part);
            object.parent_id = if part == 0 { 0 } else { id };
            objects.push(object);
        }
    }
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    sync(&mut world, &roots, &HashMap::new(), Instant::now());
    assert_eq!(world.ik_targets.prims.len(), MAX_PRIMS);
    assert_eq!(world.ik_targets.roots.len(), 4);
    assert!(!world.ik_targets.roots.contains_key(&root));
    let mut objects = Vec::new();
    let roots: BTreeSet<_> = (0..80)
        .map(|part| {
            let mut object = base.clone();
            object.local_id = 30_000 + part;
            object.full_id = demo::action_id(object.local_id);
            let id = object.full_id;
            objects.push(object);
            id
        })
        .collect();
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    sync(&mut world, &roots, &HashMap::new(), Instant::now());
    assert_eq!(world.ik_targets.roots.len(), MAX_ROOTS);
    assert_eq!(world.ik_targets.prims.len(), MAX_ROOTS);
    let highest = *roots.last().unwrap();
    let requests: BTreeMap<_, _> = roots
        .iter()
        .map(|id| {
            (
                *id,
                Requested {
                    names: ["Grip".into()].into(),
                    priority: if *id == highest { 0 } else { 3 },
                    ..Default::default()
                },
            )
        })
        .collect();
    let mut targets = std::mem::take(&mut world.ik_targets);
    let command = targets
        .sync(&world, &requests, &HashMap::new(), Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert!(
        targets.roots.contains_key(&highest),
        "local/editor metadata wins admission over observers"
    );
    assert!(
        world.objects.get(world.objects.index_of(&command).unwrap()).unwrap().full_id == highest,
        "local/editor queries have priority too"
    );
    assert_eq!(targets.prims.len(), MAX_ROOTS);
}

#[test]
fn a_new_missing_name_refreshes_once_without_reusing_stale_properties_or_polling_known_names() {
    let (world, mut props, root, _) = fixture();
    let now = Instant::now();
    let mut targets = Targets::default();
    let mut request = Requested {
        names: ["Grip".into()].into(),
        bundles: [(root, Uuid::from_u128(1))].into(),
        ..Default::default()
    };
    assert!(targets.sync(&world, &[(root, request.clone())].into(), &props, now).is_none());
    assert!(targets.index(&world, None, root, "Grip").is_some());
    request.names = ["Updated grip".into()].into();
    request.bundles = [(root, Uuid::from_u128(2))].into();
    let wanted = [(root, request.clone())].into();
    assert!(targets.sync(&world, &wanted, &props, now).is_some());
    assert!(
        targets.index(&world, None, root, "Grip").is_some(),
        "known contacts remain usable during a new-name lookup"
    );
    for tick in 1..200 {
        targets.sync(&world, &wanted, &props, now + SPACING * tick);
    }
    assert_eq!(targets.prims.values().map(|p| usize::from(p.attempts)).sum::<usize>(), 9);
    assert!(targets.sync(&world, &wanted, &props, now + Duration::from_secs(70)).is_none());
    props.get_mut(&demo::action_id(demo::hand_ik::LEFT)).unwrap().name = "Updated grip".into();
    targets.on_properties(&props.values().cloned().collect::<Vec<_>>());
    assert!(targets.sync(&world, &wanted, &props, now + Duration::from_secs(71)).is_none());
    assert!(targets.index(&world, None, root, "Updated grip").is_some());
    assert!(targets.index(&world, None, root, "Grip").is_none());
    request.bundles = [(root, Uuid::from_u128(3))].into();
    assert!(
        targets
            .sync(&world, &[(root, request)].into(), &props, now + Duration::from_secs(72))
            .is_none()
    );
    assert!(targets.index(&world, None, root, "Updated grip").is_some());
}

#[test]
fn a_declared_root_target_never_needs_the_linkset_names() {
    let (mut world, _, root, _) = fixture();
    let definition: Definition = serde_json::from_value(serde_json::json!({
        "bindings":[{"bone":"mWristLeft","target":{"kind":"link","object":"source","link":"_ROOT_"}}]
    }))
    .unwrap();
    assert!(
        world
            .update_ik_targets(Some((&definition, Some(root))), &HashMap::new(), Instant::now())
            .is_none()
    );
    assert!(world.ik_targets.prims.is_empty());
    assert!(
        world
            .ik_targets
            .index(&world, Some(Seat { root, region: 1 }), Uuid::nil(), ROOT_LINK)
            .is_some()
    );
}
