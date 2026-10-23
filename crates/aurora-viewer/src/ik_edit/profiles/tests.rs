use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
use std::sync::Arc;

pub(in crate::ik_edit) fn fixture() -> (World, Frame, Editor) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(aurora_net::NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.0, true),
    });
    let matrices = lib
        .rig
        .world_with(&lib.rig.local_rot, Vec3::ZERO, &lib.rig.local_pos, &lib.rig.scale);
    world.avatar_poses.insert(
        world.agent_id,
        matrices.iter().zip(&lib.rig.default_world_inv).map(|(m, inv)| *m * *inv).collect(),
    );
    let frame = Frame::new(&world).unwrap();
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(lib.rig.names["mWristLeft"], &lib.rig, &frame);
    editor.graph_demo(&world, &frame, "graph");
    editor.ensure_profiles(&world, &frame);
    (world, frame, editor)
}

#[test]
fn profiles_duplicate_independent_bindings_animation_frames_and_smoothing() {
    let (world, frame, mut editor) = fixture();
    editor.animations.demo(&world.avatar_lib.rig, "animations");
    let original = editor.graph.definition.bindings[0].clone();
    assert!(editor.add_profile(true, &world, &frame));
    assert_eq!(editor.graph.definition.profiles.len(), 2);
    assert_eq!(editor.animations.rows.len(), 2);
    editor.animations.rows[0].frame = 55;
    let slot = editor.graph.definition.profiles[1].bindings.as_ref().unwrap()[0];
    editor.graph.definition.bindings[slot].offset.position[0] += 0.1;
    editor.graph.definition.profiles[1].smoothing = Some([0.5, 1.0]);
    editor.switch_profile(0, &world, &frame);
    assert_eq!(editor.animations.rows[0].frame, 15);
    assert_eq!(editor.graph.definition.bindings[0], original);
    assert!(!editor.graph.visible_binding(slot));
    assert!(editor.graph.visible_binding(0));
    editor.switch_profile(1, &world, &frame);
    assert_eq!(editor.animations.rows[0].frame, 55);
    assert_eq!(editor.global_smoothing().0, [0.5, 1.0]);
    editor.delete_profile(&world, &frame);
    assert_eq!(editor.graph.definition.bindings.len(), 1);
    assert_eq!(editor.animations.rows[0].frame, 15);
    assert!(editor.graph.definition.validate().is_ok());
}

#[test]
fn copied_binding_keeps_virtual_parent_chain_without_aliasing_or_partial_mutation() {
    let (world, frame, mut editor) = fixture();
    let clipboard = editor.binding_clipboard(0).unwrap();
    let old_node = editor.graph.definition.nodes[0].id;
    assert!(editor.add_profile(false, &world, &frame));
    assert!(editor.animations.rows.is_empty());
    assert!(!editor.graph.visible_binding(0));
    assert!(editor.paste_binding_text(&world, &frame, &clipboard));
    assert!(editor.graph.visible_binding(1));
    assert_eq!(editor.graph.definition.bindings[0].bone, editor.graph.definition.bindings[1].bone);
    let Target::Node { id } = editor.graph.definition.bindings[1].target else {
        panic!("virtual target")
    };
    assert_ne!(id, old_node);
    assert!(editor.graph.definition.nodes.iter().any(|n| n.id == id));
    let before = editor.graph.definition.clone();
    assert!(!editor.paste_binding_text(&world, &frame, "invalid"));
    assert_eq!(editor.graph.definition, before);
    let encoded = editor.graph_payload().unwrap();
    assert!(!encoded.contains("suppress_live"));
    assert!(!encoded.contains("frame"));
    editor.graph.definition.remove_binding(0);
    assert_eq!(editor.graph.definition.profiles[1].bindings, Some(vec![0]));
    assert_eq!(editor.graph.definition.profiles[0].bindings, Some(Vec::new()));
}

#[test]
fn shared_imported_binding_keeps_other_profile_smoothing_and_hidden_rotations_out_of_selection() {
    let (world, frame, mut editor) = fixture();
    let original = editor.graph.definition.bindings[0].activation;
    let mut shared = editor.graph.definition.profiles[0].clone();
    shared.name = "Shared".into();
    shared.smoothing = None;
    editor.graph.definition.profiles.push(shared);
    editor.set_global_smoothing([0.6, 0.9]);
    assert_eq!(editor.graph.definition.bindings[0].activation, original);
    assert_eq!(editor.global_smoothing().0, [0.6, 0.9]);
    editor.switch_profile(1, &world, &frame);
    assert_eq!(editor.global_smoothing().0, [original.ease_in, original.ease_out]);
    assert!(editor.add_profile(false, &world, &frame));
    assert!(editor.graph.binding_rotation("mWristLeft").is_none());
    assert!(editor.graph.goals(&world.avatar_lib.rig).is_empty());
}
