use super::*;
use crate::{
    demo,
    scene::avatar::AvatarLibrary,
    world::rig_graph::{Definition, Target},
};
use aurora_net::{NetCommand, NetEvent};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{sync::Arc, time::Duration};

fn fixture() -> (World, Frame, Editor) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.2, true),
    });
    let matrices = lib
        .rig
        .world_with(&lib.rig.local_rot, Vec3::ZERO, &lib.rig.local_pos, &lib.rig.scale);
    world.avatar_poses.insert(
        world.agent_id,
        matrices.iter().zip(&lib.rig.default_world_inv).map(|(m, i)| *m * *i).collect(),
    );
    let frame = Frame::new(&world).unwrap();
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(lib.rig.names["mWristLeft"], &lib.rig, &frame);
    editor.commit();
    (world, frame, editor)
}

fn packets(command: &NetCommand, rig: &Definition, enabled: bool, mask: u32) -> Vec<NetEvent> {
    let NetEvent::Chat(chat) = demo::hand_ik::reply(command).remove(0) else {
        panic!("chat")
    };
    let body: serde_json::Value = serde_json::from_str(chat.message.strip_prefix(crate::world::hand_ik::PREFIX).unwrap()).unwrap();
    let bytes = serde_json::to_vec(rig).unwrap();
    let chunks: Vec<_> = bytes.chunks(200).collect();
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut body = body.clone();
            body["enabled"] = enabled.into();
            body["mask"] = mask.into();
            body["part"] = i.into();
            body["count"] = chunks.len().into();
            body["data"] = STANDARD.encode(chunk).into();
            let mut chat = chat.clone();
            chat.message = format!("{}{body}", crate::world::hand_ik::PREFIX);
            NetEvent::Chat(chat)
        })
        .collect()
}

fn preview(editor: &Editor, frame: &Frame, bone: usize) -> Goal {
    let goal = editor.request().goals.into_iter().find(|g| g.bone == bone).unwrap();
    let matrices: Vec<_> = frame.joints.iter().map(|m| frame.model.inverse() * *m).collect();
    goal.follow.as_ref().unwrap().context.preview(&goal, &matrices).unwrap()
}

#[test]
fn object_import_replaces_stale_edits_only_after_full_data_and_rebinds_virtual_source_frames() {
    let (mut world, mut frame, mut editor) = fixture();
    editor.graph.definition = serde_json::from_value(serde_json::json!({
        "nodes":[{"id":Uuid::new_v4(),"name":"Stale","parent":{"kind":"source"}}],
        "bindings":[{"bone":"mWristLeft","target":{"kind":"bone","bone":"mHead"}}]
    }))
    .unwrap();
    editor.ensure_profiles(&world, &frame);
    let previous = editor.graph.definition.clone();
    editor
        .animations
        .rows
        .push(animations::Row::new(Uuid::new_v4(), "Keep this layer".into()));
    editor.animations.rows[0].frame = 17;
    editor.profiles.animations[0].rows.clear();
    editor.object_data.requested = Some(demo::action_id(demo::hand_ik::ROOT));
    editor.request_object_data(&mut world, frame.at);
    let command = world.update_hand_contacts(frame.at).unwrap();
    let node = Uuid::new_v4();
    let rig: Definition = serde_json::from_value(serde_json::json!({
        "nodes":[{"id":node,"name":"Guide","parent":{"kind":"link","object":"source","link":"_ROOT_"},
            "offset":{"position":[0.1,0.2,1.0],"rotation":Quat::from_rotation_z(0.3).to_array()}}],
        "bindings":[{"bone":"mWristLeft","target":{"kind":"node","id":node},"root":"mShoulderLeft",
            "offset":{"position":[0.02,0.03,0.04],"rotation":Quat::from_rotation_y(0.2).to_array()},"weight":0.65},
            {"bone":"mWristRight","target":{"kind":"link","object":"source","link":"Right"}}]
    }))
    .unwrap();
    rig.validate().unwrap();
    let mut replies = packets(&command, &rig, false, 2);
    world.apply(replies.pop().unwrap());
    frame.at = Instant::now();
    editor.poll_object_data(&world, &frame);
    assert_eq!(editor.graph.definition, previous, "partial data cannot replace the draft");
    assert!(matches!(editor.object_data.status, Status::Receiving));
    for reply in replies.into_iter().rev() {
        world.apply(reply);
    }
    world.update_ik_targets(None, &demo::hand_ik::properties(), frame.at);
    frame.at = Instant::now();
    editor.poll_object_data(&world, &frame);
    editor.update_graph(&world, &frame);
    editor.ensure_profiles(&world, &frame);
    assert_eq!(editor.animations.rows[0].frame, 17);
    assert_eq!(editor.graph.definition, previous);
    assert!(matches!(
        editor.object_data.status,
        Status::Applied {
            bindings: 2,
            nodes: 1,
            active: false
        }
    ));
    editor.apply_pending_project_import_for_test(&world);
    editor.update_graph(&world, &frame);
    editor.ensure_profiles(&world, &frame);
    assert!(
        editor.animations.rows.is_empty(),
        "object declarations contain no authoring animation stack"
    );
    assert_eq!(editor.graph.definition.nodes[0].name, "Guide");
    assert_eq!(
        editor.graph.definition.nodes[0].parent,
        Target::Link {
            object: demo::action_id(demo::hand_ik::ROOT),
            link: crate::world::rig_graph::ROOT_LINK.into()
        }
    );
    assert_eq!(
        editor.graph.definition.bindings[0].offset, rig.bindings[0].offset,
        "do not recapture the displayed pose"
    );
    assert!(
        editor.graph.definition.bindings[1].enabled,
        "import the declaration rather than transient runtime masks"
    );
    assert!(editor.bindings.is_empty());
    assert!(!editor.disabled.contains(&world.avatar_lib.rig.names["mWristRight"]));
    assert!(
        world.hand_contacts.graphs().is_empty(),
        "preview does not enable the object's runtime"
    );
    let wrist = world.avatar_lib.rig.names["mWristLeft"];
    let goal = preview(&editor, &frame, wrist);
    let source = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
    let (p, q, _) = Scene::object_transform(&world, source, frame.at, 0).unwrap();
    let expected = rig.nodes[0].offset.compose(rig.bindings[0].offset);
    assert!(
        goal.position
            .abs_diff_eq(frame.model.inverse().transform_point3(p + q * expected.p()), 0.0001)
    );
    assert_eq!(goal.weight, 0.65);
    let rotation = frame.model.to_scale_rotation_translation().1.inverse() * q * expected.q();
    assert!(goal.rotation.unwrap().dot(rotation).abs() > 0.99999);
    // Subsequent prim editing follows the reloaded source, rather than baking
    // the previous rendered contact back into a new offset.
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.5, true),
    });
    // Advance past the normal simulator update interpolation interval.
    frame.at = Instant::now() + Duration::from_secs(1);
    editor.update_graph(&world, &frame);
    let moved = preview(&editor, &frame, wrist);
    assert!(moved.position.distance(goal.position) > 0.01);
    assert_eq!(editor.graph.definition.bindings[0].offset, rig.bindings[0].offset);
    let exported: Definition = serde_json::from_str(&editor.graph_payload().unwrap()).unwrap();
    assert_eq!(
        exported.nodes[0].parent,
        Target::Link {
            object: Uuid::nil(),
            link: crate::world::rig_graph::ROOT_LINK.into()
        },
        "copies must resolve their own publisher root"
    );
    assert_eq!(exported.bindings[0].offset, rig.bindings[0].offset);
}

#[test]
fn authored_hand_bindings_export_and_drive_the_runtime_after_closing_the_editor_with_missing_owner_metadata() {
    let (mut world, frame, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let mut desired = Vec::new();
    for (name, shoulder, prim, label) in [
        ("mWristLeft", "mShoulderLeft", demo::hand_ik::LEFT, "Left"),
        ("mWristRight", "mShoulderRight", demo::hand_ik::RIGHT, "Right"),
    ] {
        let bone = rig.names[name];
        let root = rig.names[shoulder];
        editor.select(bone, &rig, &frame);
        let inverse = frame.model.inverse();
        let start = inverse.transform_point3(frame.joints[root].w_axis.truncate());
        let end = inverse.transform_point3(frame.joints[bone].w_axis.truncate());
        editor.position = start.lerp(end, 0.85) + Vec3::X * 0.03;
        editor.contact = Vec3::ZERO;
        editor.mode = Mode::Ik;
        editor.root = root;
        editor.weight = 1.0;
        let q = editor.rotation();
        desired.push((bone, editor.position, q));
        assert!(editor.bind_target(
            &world,
            &frame,
            bone,
            Target::Prim {
                id: demo::action_id(prim),
                name: label.into()
            }
        ));
    }
    let script = editor.graph_script().unwrap();
    let encoded = script
        .lines()
        .find_map(|line| line.strip_prefix("string RIG = "))
        .unwrap()
        .strip_suffix(';')
        .unwrap();
    let raw: String = serde_json::from_str(encoded).unwrap();
    let exported: Definition = serde_json::from_str(&raw).unwrap();
    assert_eq!(exported.bindings.len(), 2);
    editor.toggle();
    assert!(
        !editor.request().active,
        "script testing uses the live runtime, without the editor mask or authored goals"
    );
    let source = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
    world.objects.get_mut(source).unwrap().owner_id = Uuid::nil();
    let command = world.update_hand_contacts(frame.at).unwrap();
    for event in packets(&command, &exported, true, 0) {
        world.apply(event);
    }
    world.update_ik_targets(None, &demo::hand_ik::properties(), frame.at);
    let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
    let now = Instant::now();
    let mut solver = ik::Pose::default();
    solver.targets = ik::targets(&world, avatar, 0.0, now);
    assert_eq!(solver.targets.len(), 2);
    let mut solved = None;
    for i in 0..10 {
        solved = solver.apply(
            &rig,
            &rig.local_rot,
            &rig.local_pos,
            Vec3::ZERO,
            &rig.scale,
            now + Duration::from_millis(50 * i),
            false,
        );
    }
    let solved = solved.unwrap();
    let joints = rig.world_with(&solved.rotations, Vec3::ZERO, &solved.positions, &rig.scale);
    for (bone, position, rotation) in desired {
        assert!(
            joints[bone].w_axis.truncate().distance(position) < 0.001,
            "{} must reach its exported position",
            rig.canonical_names[bone]
        );
        assert!(
            joints[bone].to_scale_rotation_translation().1.dot(rotation).abs() > 0.99999,
            "exported wrist orientation"
        );
    }
}

#[test]
fn object_import_failure_preserves_the_draft_and_retry_and_close_do_not_import_old_tickets() {
    let (mut world, mut frame, mut editor) = fixture();
    let previous = editor.goals[0].position;
    editor.object_data.requested = Some(demo::action_id(demo::hand_ik::ROOT));
    editor.request_object_data(&mut world, frame.at);
    let old = world.update_hand_contacts(frame.at).unwrap();
    frame.at += Duration::from_secs(46);
    editor.poll_object_data(&world, &frame);
    assert!(matches!(editor.object_data.status, Status::TimedOut));
    assert_eq!(editor.goals[0].position, previous);
    editor.object_data.requested = Some(demo::action_id(demo::hand_ik::ROOT));
    editor.request_object_data(&mut world, frame.at);
    for reply in demo::hand_ik::reply(&old) {
        world.apply(reply);
    }
    editor.poll_object_data(&world, &frame);
    assert!(matches!(editor.object_data.status, Status::Waiting));
    let root = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
    world.objects.remove_idx(root);
    editor.poll_object_data(&world, &frame);
    assert!(matches!(editor.object_data.status, Status::Cancelled));
    assert_eq!(editor.goals[0].position, previous);
    editor.toggle();
    editor.toggle();
    assert!(editor.object_data.pending.is_none());
}

#[test]
fn explicit_object_designation_normalizes_a_child_without_modifying_the_current_project() {
    let (mut world, frame, mut editor) = fixture();
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![demo::action_avatar(false)],
    });
    let before = editor.graph.definition.clone();
    editor.begin_object_import();
    let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
    assert!(!editor.picked_prim(&world, &BuildTool::default(), avatar));
    assert!(editor.object_import_picking());
    editor.cancel_link_pick();
    assert_eq!(editor.graph.definition, before);
    editor.begin_object_import();
    let child = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::LEFT)).unwrap();
    assert!(editor.picked_prim(&world, &BuildTool::default(), child));
    assert!(!editor.object_import_picking());
    assert_eq!(editor.object_data.requested, Some(demo::action_id(demo::hand_ik::ROOT)));
    editor.request_object_data(&mut world, frame.at);
    assert!(editor.object_data.pending.is_some());
    assert_eq!(editor.graph.definition, before);
}
