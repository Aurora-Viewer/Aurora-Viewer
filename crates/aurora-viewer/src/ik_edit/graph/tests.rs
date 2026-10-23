use super::*;
use crate::ik_edit::tests::HeadlessContext as _;
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::NetEvent;
use base64::{Engine, engine::general_purpose::STANDARD};

fn fixture() -> (World, Frame) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
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
    (world, frame)
}

#[test]
fn named_profiles_survive_calibration_exports_retargeting_and_binding_deletion() {
    let (world, frame) = fixture();
    let mut editor = Editor::default();
    editor.toggle();
    let wrist = world.avatar_lib.rig.names["mWristLeft"];
    editor.select(wrist, &world.avatar_lib.rig, &frame);
    editor.graph_demo(&world, &frame, "graph");
    let profile = data::Profile {
        bindings: None,
        smoothing: None,
        name: "Handlebars".into(),
        bones: vec!["mWristLeft".into()],
    };
    editor.graph.definition.profiles = vec![profile.clone()];
    assert!(editor.bind_target(
        &world,
        &frame,
        wrist,
        Target::Prim {
            id: demo::action_id(demo::hand_ik::LEFT),
            name: "Left".into()
        }
    ));
    let exported: Definition = serde_json::from_str(&editor.graph_payload().unwrap()).unwrap();
    assert_eq!(exported.profiles, vec![profile]);
    assert!(editor.graph_script().unwrap().contains("integer EXTERNAL_CONTROL = TRUE;"));
    editor.graph.definition.bindings.clear();
    editor.graph.definition.prune_profiles();
    assert!(editor.graph.definition.profiles.is_empty());
    assert!(editor.graph.definition.validate().is_ok());
    assert!(editor.graph_script().unwrap().contains("integer EXTERNAL_CONTROL = TRUE;"));
}

#[test]
fn importing_a_partial_named_profile_preserves_the_full_declaration() {
    let (mut world, frame) = fixture();
    let command = world.update_hand_contacts(frame.at).unwrap();
    let NetEvent::Chat(chat) = demo::hand_ik::reply(&command).remove(0) else {
        panic!("chat")
    };
    let mut packet: serde_json::Value = serde_json::from_str(chat.message.strip_prefix(crate::world::hand_ik::PREFIX).unwrap()).unwrap();
    let bytes = STANDARD.decode(packet["data"].as_str().unwrap()).unwrap();
    let mut definition: Definition = serde_json::from_slice(&bytes).unwrap();
    definition.profiles = vec![
        data::Profile {
            bindings: None,
            smoothing: None,
            name: "drive".into(),
            bones: vec!["mWristLeft".into(), "mWristRight".into()],
        },
        data::Profile {
            bindings: None,
            smoothing: None,
            name: "gas_only".into(),
            bones: vec!["mWristRight".into()],
        },
    ];
    definition.validate().unwrap();
    packet["profile"] = "gas_only".into();
    packet["cycles"] = serde_json::json!([1, 1]);
    packet["mask"] = 1.into();
    let bytes = serde_json::to_vec(&definition).unwrap();
    let parts: Vec<_> = bytes.chunks(160).collect();
    for (index, part) in parts.iter().enumerate() {
        packet["part"] = index.into();
        packet["count"] = parts.len().into();
        packet["data"] = STANDARD.encode(part).into();
        let mut chat = chat.clone();
        chat.message = format!("{}{packet}", crate::world::hand_ik::PREFIX);
        world.apply(NetEvent::Chat(chat));
    }
    let runtime = world.hand_contacts.graphs().first().unwrap();
    assert_eq!(runtime.bones, ["mWristRight"]);
    for explicit_reload in [false, true] {
        let mut editor = Editor::default();
        editor.toggle();
        if explicit_reload {
            editor.graph.replace_runtime(runtime, frame.at);
        } else {
            editor.graph.imported = false;
            editor.graph.refresh(&world, &frame);
        }
        let exported: Definition = serde_json::from_str(&editor.graph_payload().unwrap()).unwrap();
        assert_eq!(exported.profiles, definition.profiles);
        let mut expected = definition.bindings.clone();
        if !explicit_reload {
            for binding in &mut expected {
                if let Target::Link { object, .. } = &mut binding.target {
                    *object = runtime.source.root;
                }
            }
        }
        assert_eq!(exported.bindings, expected);
        assert!(exported.bindings.iter().all(|b| b.enabled));
    }
}

#[test]
fn virtual_list_shows_parents_and_ctrl_rotation_survives_modifier_release() {
    let (world, frame) = fixture();
    let mut editor = Editor::default();
    editor.toggle();
    editor.library_demo(&world, "");
    assert!(editor.add_profile(false, &world, &frame));
    editor.select(world.avatar_lib.rig.names["mWristLeft"], &world.avatar_lib.rig, &frame);
    editor.graph_demo(&world, &frame, "graph");
    editor.assign_profile_binding(0);
    let id = editor.graph.selected.unwrap();
    editor.select_virtual_node(id);
    let before = editor.graph.node_transform(id).unwrap();
    let point = frame.model.transform_point3(before.p());
    let offset = Vec3::new(3.0, -4.0, 3.0);
    let eye = point + offset;
    let proj = Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05)
            * glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z),
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    };
    let radius = offset.length() * (100.0_f32 / 900.0 * 0.8).tan();
    let ring = |t: f32| proj.project(point + Vec3::new(t.cos(), t.sin(), 0.0) * radius).unwrap().0;
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    for (step, (modifiers, position, press)) in [
        (egui::Modifiers::NONE, ring(0.4), None),
        (egui::Modifiers::CTRL, ring(0.4), None),
        (egui::Modifiers::CTRL, ring(0.4), Some(true)),
        (egui::Modifiers::CTRL, ring(0.7), None),
        (egui::Modifiers::NONE, ring(0.9), None),
        (egui::Modifiers::NONE, ring(0.9), Some(false)),
        (egui::Modifiers::NONE, ring(0.9), None),
    ]
    .into_iter()
    .enumerate()
    {
        let mut events = vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(position)];
        if let Some(pressed) = press {
            events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            });
        }
        let mut output = ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(step as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| editor.show(ui.ctx(), &palette, &world, &proj),
        );
        if step == 1 {
            assert_eq!(editor.graph.selected, Some(id));
            let has_text = |label: &str| {
                output.shapes.iter().any(|s| {
                    matches!(&s.shape,
                egui::Shape::Text(t) if t.galley.text() == label)
                })
            };
            for node in &editor.graph.definition.nodes {
                assert!(has_text(&node.name));
                assert!(has_text(&label(&node.parent, &world, &editor.graph.definition.nodes)));
            }
            for key in ["ik-edit-move", "ik-edit-turn", "ik-graph-link-name"] {
                assert!(!has_text(aurora_i18n::text(key)), "virtual controls should not duplicate {key}");
            }
            for key in ["ik-edit-world", "ik-edit-bone-local"] {
                let count = output
                    .shapes
                    .iter()
                    .filter(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(key)))
                    .count();
                assert_eq!(count, 1, "space {key} appears once in the shared footer");
            }
        }
        output.textures_delta.clear();
    }
    let after = editor.graph.node_transform(id).unwrap();
    let (_, model_q, _) = frame.model.to_scale_rotation_translation();
    let expected = model_q.inverse() * Quat::from_rotation_z(0.5) * model_q * before.q();
    assert!(after.p().abs_diff_eq(before.p(), 1e-5));
    assert!(
        after.q().dot(expected).abs() > 0.99999,
        "virtual drag must not restart or cancel each frame"
    );
    assert!(editor.gizmo_visible(egui::Modifiers::NONE));
}

#[test]
fn shift_drags_a_virtual_point_freely_while_default_mode_reserves_translation_for_axes() {
    let (world, frame) = fixture();
    let mut editor = Editor::default();
    editor.toggle();
    editor.library_demo(&world, "");
    assert!(editor.add_profile(false, &world, &frame));
    editor.select(world.avatar_lib.rig.names["mWristLeft"], &world.avatar_lib.rig, &frame);
    editor.graph_demo(&world, &frame, "graph");
    editor.assign_profile_binding(0);
    let id = editor.graph.selected.unwrap();
    let before = editor.graph.node_transform(id).unwrap();
    let point = frame.model.transform_point3(before.p());
    let eye = point + Vec3::new(3.0, -4.0, 3.0);
    let proj = Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05)
            * glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z),
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    };
    let at = proj.project(point).unwrap().0;
    let delta = egui::vec2(32.0, -19.0);
    let ctx = egui::Context::default();
    let palette = crate::theme::Theme::default().palette();
    let mut time = 0.0;
    for modifiers in [egui::Modifiers::NONE, egui::Modifiers::SHIFT] {
        for (pointer, press) in [
            (at, None),
            (at, None),
            (at, Some(true)),
            (at + delta, None),
            (at + delta, Some(false)),
        ] {
            time += 1.0 / 60.0;
            let mut events = vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(pointer)];
            if let Some(pressed) = press {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                });
            }
            let mut output = ctx.run_headless(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| editor.show(ui.ctx(), &palette, &world, &proj),
            );
            output.textures_delta.clear();
        }
        let after = editor.graph.node_transform(id).unwrap();
        assert!(after.q().dot(before.q()).abs() > 0.99999);
        if modifiers.shift {
            let moved = proj.project(frame.model.transform_point3(after.p())).unwrap().0;
            assert!(
                moved.distance(at + delta) < 0.2,
                "Shift must move the node in the camera plane: moved={moved:?}, expected={:?}, selected={:?}, drag={}, before={:?}, after={:?}",
                at + delta,
                editor.graph.selected,
                editor.drag.is_some(),
                before.p(),
                after.p()
            );
        } else {
            assert!(
                after.p().abs_diff_eq(before.p(), 1e-5),
                "a default point drag must not bypass the arrows"
            );
        }
    }
}

#[test]
fn virtual_visual_edits_update_parent_offsets_or_delta_keys_and_export_the_same_definition() {
    let (world, frame) = fixture();
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(world.avatar_lib.rig.names["mWristLeft"], &world.avatar_lib.rig, &frame);
    editor.graph_demo(&world, &frame, "graph-keys");
    let id = editor.graph.selected.unwrap();
    editor.graph.definition.nodes[1].name = "Control \"test\" \\ / 背景 🏍".into();
    editor.graph.time = 0.5;
    editor.graph.refresh(&world, &frame);
    let before = editor.graph.node_transform(id).unwrap();
    let desired = Transform::new(before.p() + Vec3::new(0.03, -0.01, 0.04), Quat::from_rotation_x(0.25) * before.q());
    editor.graph.capture_node(desired);
    let actual = editor.graph.node_transform(id).unwrap();
    assert!(actual.p().abs_diff_eq(desired.p(), 1e-5));
    assert!(actual.q().dot(desired.q()).abs() > 0.99999);
    let base = editor.graph.definition.nodes[1].offset;
    editor.graph.auto_key = true;
    let keyed = Transform::new(desired.p() + Vec3::Y * 0.02, desired.q());
    editor.graph.capture_node(keyed);
    assert_eq!(editor.graph.definition.nodes[1].offset, base);
    let Some(data::Motion::Keys { keys, .. }) = &editor.graph.definition.nodes[1].motion else {
        panic!("keys")
    };
    assert_eq!(keys.len(), 3);
    assert!(editor.graph.node_transform(id).unwrap().p().abs_diff_eq(keyed.p(), 1e-5));
    let script = editor.graph_script().unwrap();
    let line = script.lines().find(|l| l.starts_with("string RIG = ")).unwrap();
    assert_eq!(Some(line.to_owned()), editor.graph_rig_declaration());
    let raw: String = serde_json::from_str(line.strip_prefix("string RIG = ").unwrap().strip_suffix(';').unwrap()).unwrap();
    assert!(raw.is_ascii());
    let exported: Definition = serde_json::from_str(&raw).unwrap();
    let mut expected = editor.graph.definition.clone();
    expected.nodes[0].parent = Target::Link {
        object: demo::action_id(demo::hand_ik::ROOT),
        link: "Left".into(),
    };
    assert_eq!(exported, expected);
    assert_eq!(
        Some(raw),
        editor.graph_payload(),
        "raw rig payload is the complete script's exact RIG definition"
    );
    assert!(
        script.contains("integer EXTERNAL_CONTROL = TRUE;"),
        "export preserves the companion template policy"
    );
    assert!(exported.validate().is_ok());
    editor.graph.definition.bindings.clear();
    let nodes_only: Definition = serde_json::from_str(&editor.graph_payload().unwrap()).unwrap();
    assert_eq!(nodes_only.nodes, exported.nodes);
    assert!(nodes_only.bindings.is_empty());
    assert!(
        editor.graph_script().is_some(),
        "virtual controls can be exported before assigning an effector"
    );
}

#[test]
fn advanced_binding_switches_release_without_erasing_calibration_and_new_target_offsets_are_stable() {
    let (world, frame) = fixture();
    let rig = &world.avatar_lib.rig;
    let bone = rig.names["mWristLeft"];
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(bone, rig, &frame);
    editor.graph_demo(&world, &frame, "graph");
    editor.select(bone, rig, &frame);
    editor.update_graph(&world, &frame);
    let mut goal = editor.goals.iter().find(|g| g.bone == bone).unwrap().clone();
    goal.position += Vec3::Z * 0.03;
    editor.graph.capture(&goal, &rig.canonical_names);
    let offset = editor.graph.definition.bindings[0].offset;
    for _ in 0..20 {
        editor.update_graph(&world, &frame);
    }
    assert_eq!(editor.graph.definition.bindings[0].offset, offset);
    assert_eq!(editor.request().goals.iter().filter(|g| g.bone == bone).count(), 1);
    editor.enable(bone, false, rig, &frame);
    assert!(editor.request().goals.iter().all(|g| g.bone != bone));
    assert!(editor.request().released.contains(&bone));
    assert_eq!(editor.graph.definition.bindings[0].offset, offset);
    editor.enable(bone, true, rig, &frame);
    assert_eq!(editor.request().goals.iter().filter(|g| g.bone == bone).count(), 1);
}

#[test]
fn control_timeline_displays_loop_phase_without_stopping_a_long_running_preview() {
    let (world, frame) = fixture();
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(world.avatar_lib.rig.names["mWristLeft"], &world.avatar_lib.rig, &frame);
    editor.graph_demo(&world, &frame, "graph");
    editor.graph.playing = true;
    editor.graph.time = 100_000_001.25;
    let before = editor.graph.time;
    let ctx = egui::Context::default();
    let palette = crate::theme::Theme::default().palette();
    let _ = ctx.run_headless(egui::RawInput::default(), |ui| editor.graph.motion_ui(ui, &palette, 1));
    assert!(editor.graph.playing);
    assert_eq!(editor.graph.time, before);
}
