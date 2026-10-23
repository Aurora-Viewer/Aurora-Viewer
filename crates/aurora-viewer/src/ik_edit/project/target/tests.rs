use super::*;
use crate::{
    build::{BuildSettings, Mods},
    demo,
    ik_edit::profiles::tests::fixture,
};
use aurora_net::{NetEvent, build::ObjectProps};

const DELTA: u32 = 1000;
fn replacement(worn: bool) -> (World, Editor, BuildTool) {
    let (mut world, frame, mut editor) = fixture();
    editor.library_demo(&world, "");
    let original = demo::hand_ik::ROOT;
    editor.graph.source = Some(demo::action_id(original));
    editor.graph.definition.nodes[0].parent = Target::Prim {
        id: demo::action_id(original),
        name: String::new(),
    };
    editor.graph.definition.bindings[0].target = Target::Prim {
        id: demo::action_id(demo::hand_ik::LEFT),
        name: "Left".into(),
    };
    assert!(editor.add_profile(true, &world, &frame));
    editor.graph.definition.bindings[1].target = Target::Prim {
        id: demo::action_id(demo::hand_ik::RIGHT),
        name: "Right".into(),
    };
    let objects = demo::hand_ik::objects(0.0, true);
    let mut avatar = objects.last().unwrap().clone();
    avatar.parent_id = 0;
    let avatar_local = avatar.local_id;
    let mut updates = vec![avatar];
    let mut build = BuildTool::default();
    for mut object in objects.into_iter().take(3) {
        let local = object.local_id;
        object.local_id += DELTA;
        object.full_id = demo::action_id(object.local_id);
        if local == original {
            object.parent_id = if worn { avatar_local } else { 0 };
            object.state = if worn { 1_u8.rotate_left(4) } else { 0 };
        } else {
            object.parent_id = original + DELTA;
        }
        build.props.insert(
            object.full_id,
            ObjectProps {
                object_id: object.full_id,
                name: if local == original {
                    "Replacement"
                } else if local == demo::hand_ik::LEFT {
                    "Left"
                } else {
                    "Right"
                }
                .into(),
                ..Default::default()
            },
        );
        updates.push(object);
    }
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: updates,
    });
    (world, editor, build)
}
fn index(world: &World, local: u32) -> usize {
    world.objects.index_of_uuid(&demo::action_id(local + DELTA)).unwrap()
}

#[test]
fn a_profile_can_pick_two_objects_save_root_link_pairs_and_replace_only_its_project_object() {
    let (mut world, frame, _) = fixture();
    let mut build = BuildTool::default();
    build.props.extend(demo::hand_ik::properties());
    let own = demo::action_id(demo::hand_ik::ROOT);
    let external = demo::action_id(demo::hand_ik::ROOT + DELTA);
    let mut objects = demo::hand_ik::objects(0.0, true);
    objects.truncate(3);
    for object in &mut objects {
        object.local_id += DELTA;
        object.full_id = demo::action_id(object.local_id);
        if object.parent_id != 0 {
            object.parent_id += DELTA;
        }
        let name = if object.parent_id == 0 {
            "External object"
        } else if object.local_id == demo::hand_ik::LEFT + DELTA {
            "Left"
        } else {
            "Right"
        };
        build.props.insert(
            object.full_id,
            ObjectProps {
                object_id: object.full_id,
                name: name.into(),
                ..Default::default()
            },
        );
    }
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    let mut editor = Editor::default();
    editor.toggle();
    editor.library_demo(&world, "");
    assert!(editor.add_profile(false, &world, &frame));
    assert!(editor.picked_project_target(&world, &build, world.objects.index_of_uuid(&own).unwrap()));
    let bone = world.avatar_lib.rig.names["mWristLeft"];
    for id in [demo::action_id(demo::hand_ik::LEFT), demo::action_id(demo::hand_ik::RIGHT + DELTA)] {
        editor.begin_link(&world, &frame, bone, true);
        editor.links.adding = true;
        assert!(
            editor.picked_prim(&world, &build, world.objects.index_of_uuid(&id).unwrap()),
            "target {id}"
        );
    }
    editor.graph.definition.nodes.push(crate::world::rig_graph::Node {
        id: Uuid::from_u128(12345),
        name: "Root control".into(),
        parent: Target::Link {
            object: own,
            link: crate::world::rig_graph::ROOT_LINK.into(),
        },
        offset: Default::default(),
        motion: None,
    });
    let foreign = Target::Prim {
        id: demo::action_id(demo::hand_ik::RIGHT + DELTA),
        name: "Right".into(),
    };
    editor.graph.definition.nodes.push(crate::world::rig_graph::Node {
        id: Uuid::from_u128(12346),
        name: "External control".into(),
        parent: foreign.clone(),
        offset: Default::default(),
        motion: None,
    });
    editor.sync_build(&world, &build, &BuildSettings::default(), (0.0, 0.0), Mods::default());
    assert_eq!(
        editor.graph.definition.nodes[1].parent, foreign,
        "a pending external parent must never rebind to the project's identically named link"
    );
    world.update_ik_targets(Some((&editor.graph.definition, editor.graph.source)), &build.props, frame.at);
    build.props.clear();
    editor.begin_link(&world, &frame, bone, true);
    editor.graph.binding = Some(1);
    editor.links.adding = false;
    assert!(editor.picked_prim(
        &world,
        &build,
        world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::RIGHT + DELTA)).unwrap()
    ));
    editor.sync_build(&world, &build, &BuildSettings::default(), (0.0, 0.0), Mods::default());
    assert_eq!(
        editor.graph.definition.bindings.len(),
        2,
        "reuse IK metadata after the build property cache is evicted"
    );
    let stored = editor.stored_definition();
    let clipboard: serde_json::Value = serde_json::from_str(&editor.binding_clipboard(1).unwrap()).unwrap();
    assert_eq!(clipboard["binding"]["target"]["kind"], "link");
    assert_eq!(clipboard["binding"]["target"]["object"], external.to_string());
    assert_eq!(
        stored.bindings[0].target,
        Target::Link {
            object: own,
            link: "Left".into()
        }
    );
    assert_eq!(
        stored.bindings[1].target,
        Target::Link {
            object: external,
            link: "Right".into()
        }
    );
    let data: crate::world::rig_graph::Definition = serde_json::from_str(&editor.graph_payload().unwrap()).unwrap();
    assert_eq!(
        data.bindings[0].target,
        Target::Link {
            object: Uuid::nil(),
            link: "Left".into()
        }
    );
    assert_eq!(data.bindings[1].target, stored.bindings[1].target);
    assert_eq!(
        data.nodes[1].parent,
        Target::Link {
            object: external,
            link: "Right".into()
        }
    );
    assert_eq!(
        data.nodes[0].parent,
        Target::Link {
            object: Uuid::nil(),
            link: crate::world::rig_graph::ROOT_LINK.into()
        }
    );
    let bytes = editor.project_document().encode().unwrap();
    let mut loaded = Editor::default();
    loaded.apply_project(&world, &bytes).unwrap();
    assert_eq!(loaded.graph.definition, stored);
    let mut objects = demo::hand_ik::objects(0.0, true);
    objects.truncate(3);
    for object in &mut objects {
        object.local_id += DELTA * 2;
        object.full_id = demo::action_id(object.local_id);
        if object.parent_id != 0 {
            object.parent_id += DELTA * 2;
        }
        let name = if object.parent_id == 0 {
            "Another product"
        } else if object.local_id == demo::hand_ik::LEFT + DELTA * 2 {
            "Left"
        } else {
            "Right"
        };
        build.props.insert(
            object.full_id,
            ObjectProps {
                object_id: object.full_id,
                name: name.into(),
                ..Default::default()
            },
        );
    }
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    let next = demo::action_id(demo::hand_ik::ROOT + DELTA * 2);
    assert!(loaded.picked_project_target(&world, &build, world.objects.index_of_uuid(&next).unwrap()));
    let rebound = loaded.stored_definition();
    let mut expected = stored;
    expected.bindings[0].target = Target::Link {
        object: next,
        link: "Left".into(),
    };
    expected.nodes[0].parent = Target::Link {
        object: next,
        link: crate::world::rig_graph::ROOT_LINK.into(),
    };
    assert_eq!(
        rebound, expected,
        "replace only project-local roots, preserving external identity and all calibration"
    );
}

#[test]
fn late_properties_promote_only_the_exact_picked_child_to_a_reusable_project_name() {
    for worn in [false, true] {
        let (world, mut editor, mut build) = replacement(worn);
        assert!(editor.picked_project_target(&world, &build, index(&world, demo::hand_ik::ROOT)));
        let frame = Frame::new(&world).unwrap();
        let right = demo::action_id(demo::hand_ik::RIGHT + DELTA);
        let properties = build.props.remove(&right).unwrap();
        let source = world.avatar_lib.rig.names["mWristLeft"];
        editor.begin_link(&world, &frame, source, true);
        assert!(editor.picked_prim(&world, &build, index(&world, demo::hand_ik::RIGHT)));
        let slot = editor.graph.binding.unwrap();
        assert_eq!(
            editor.graph.definition.bindings[slot].target,
            Target::Prim {
                id: right,
                name: String::new()
            }
        );
        let mut expected = editor.graph.definition.clone();
        editor.sync_build(&world, &build, &BuildSettings::default(), (0.0, 0.0), Mods::default());
        assert_eq!(
            editor.graph.definition, expected,
            "incomplete properties cannot establish a unique link name"
        );
        build.props.insert(right, properties);
        editor.sync_build(&world, &build, &BuildSettings::default(), (0.0, 0.0), Mods::default());
        expected.bindings[slot].target = Target::Prim {
            id: right,
            name: "Right".into(),
        };
        assert_eq!(
            editor.graph.definition, expected,
            "a late name must never recapture offsets, poles or activation"
        );
        let exported: crate::world::rig_graph::Definition = serde_json::from_str(&editor.graph_payload().unwrap()).unwrap();
        assert_eq!(
            exported.bindings[slot].target,
            Target::Link {
                object: Uuid::nil(),
                link: "Right".into()
            }
        );
        assert!(
            exported.nodes.iter().any(|n| n.parent
                == Target::Link {
                    object: Uuid::nil(),
                    link: crate::world::rig_graph::ROOT_LINK.into()
                }),
            "the source root retains its own portable frame"
        );
        let saved = editor.project_document().encode().unwrap();
        let mut restored = Editor::default();
        restored.apply_project(&world, &saved).unwrap();
        assert_eq!(restored.graph.definition, editor.stored_definition());
    }
}

#[test]
fn late_project_names_never_promote_ambiguous_children_or_explicit_external_targets() {
    let (world, mut editor, mut build) = replacement(false);
    assert!(editor.picked_project_target(&world, &build, index(&world, demo::hand_ik::ROOT)));
    let left = demo::action_id(demo::hand_ik::LEFT + DELTA);
    let right = demo::action_id(demo::hand_ik::RIGHT + DELTA);
    let external = demo::action_id(demo::hand_ik::RIGHT);
    editor.graph.definition.bindings[0].target = Target::Prim {
        id: left,
        name: String::new(),
    };
    editor.graph.definition.bindings[1].target = Target::Prim {
        id: external,
        name: String::new(),
    };
    build.props.insert(
        external,
        ObjectProps {
            object_id: external,
            name: "Right".into(),
            ..Default::default()
        },
    );
    build.props.get_mut(&right).unwrap().name = "Left".into();
    let mut expected = editor.graph.definition.clone();
    editor.sync_project_target(&world, &build);
    assert_eq!(
        editor.graph.definition, expected,
        "duplicate local names retain the exact UUID target"
    );
    build.props.get_mut(&right).unwrap().name = "Right".into();
    editor.sync_project_target(&world, &build);
    expected.bindings[0].target = Target::Prim {
        id: left,
        name: "Left".into(),
    };
    assert_eq!(
        editor.graph.definition, expected,
        "a named external object must remain an explicit external UUID"
    );
}

#[test]
fn replacing_a_live_or_deleted_motorcycle_with_a_world_object_or_attachment_preserves_all_calibration() {
    for worn in [false, true] {
        for remove_original in [false, true] {
            let (mut world, mut editor, build) = replacement(worn);
            if remove_original {
                let key = world
                    .objects
                    .get(world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap())
                    .unwrap()
                    .key;
                world.objects.remove(&key);
            }
            let mut expected = editor.graph.definition.clone();
            let new_root = demo::action_id(demo::hand_ik::ROOT + DELTA);
            expected.nodes[0].parent = Target::Prim {
                id: new_root,
                name: String::new(),
            };
            for (slot, local) in [demo::hand_ik::LEFT, demo::hand_ik::RIGHT].into_iter().enumerate() {
                if let Target::Prim { id, .. } = &mut expected.bindings[slot].target {
                    *id = demo::action_id(local + DELTA);
                }
            }
            let selection = build.selection.clone();
            editor.begin_project_target_pick();
            assert!(editor.picked_prim(&world, &build, index(&world, demo::hand_ik::LEFT)));
            assert!(!editor.picking && !editor.project_target_picking());
            assert_eq!(editor.graph.source, Some(new_root));
            assert_eq!(editor.graph.definition, expected);
            assert_eq!(build.selection, selection);
            let bytes = editor.project_document().encode().unwrap();
            let mut restored = Editor::default();
            restored.apply_project(&world, &bytes).unwrap();
            assert_eq!(restored.graph.source, Some(new_root));
            assert_eq!(restored.project.target.name, "Replacement");
            assert_eq!(restored.graph.definition, editor.stored_definition());
            assert!(editor.graph_definition().nodes.iter().any(|n| n.parent
                == Target::Link {
                    object: Uuid::nil(),
                    link: crate::world::rig_graph::ROOT_LINK.into()
                }));
        }
    }
}

#[test]
fn duplicate_missing_and_late_names_never_keep_using_the_original_object() {
    let (world, mut editor, mut build) = replacement(false);
    let right = demo::action_id(demo::hand_ik::RIGHT + DELTA);
    build.props.get_mut(&right).unwrap().name = "Left".into();
    assert!(editor.picked_project_target(&world, &build, index(&world, demo::hand_ik::ROOT)));
    assert_eq!(editor.project.target.unresolved, 2);
    assert!(
        editor
            .graph
            .definition
            .bindings
            .iter()
            .all(|b| matches!(&b.target, Target::Prim { id, .. } if id.is_nil()))
    );
    for slot in 0..editor.graph.definition.bindings.len() {
        assert!(editor.graph.binding_goal(slot, &world.avatar_lib.rig).is_none());
    }
    assert!(editor.project_document().encode().is_ok());
    build.props.remove(&right);
    editor.sync_project_target(&world, &build);
    assert_eq!(editor.project.target.unresolved, 2);
    build.props.insert(
        right,
        ObjectProps {
            object_id: right,
            name: "Right".into(),
            ..Default::default()
        },
    );
    editor.sync_project_target(&world, &build);
    assert_eq!(editor.project.target.unresolved, 0);
    for (slot, local) in [demo::hand_ik::LEFT, demo::hand_ik::RIGHT].into_iter().enumerate() {
        assert!(matches!(&editor.graph.definition.bindings[slot].target, Target::Prim { id, .. } if *id == demo::action_id(local + DELTA)));
    }
}

#[test]
fn cancelling_project_designation_keeps_the_previous_object_and_links() {
    let (world, mut editor, build) = replacement(false);
    let before = editor.project_document().encode().unwrap();
    editor.begin_project_target_pick();
    let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
    assert!(!editor.picked_prim(&world, &build, avatar));
    assert!(editor.project_target_picking());
    editor.cancel_link_pick();
    assert!(!editor.picking && !editor.project_target_picking());
    assert_eq!(editor.project_document().encode().unwrap(), before);
}
