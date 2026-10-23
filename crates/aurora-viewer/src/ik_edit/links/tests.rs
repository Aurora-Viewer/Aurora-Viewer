use super::*;
use crate::ik_edit::tests::HeadlessContext as _;
use crate::{build::BuildTool, demo, scene::avatar::AvatarLibrary};
use aurora_net::NetEvent;
use std::sync::Arc;
fn fixture() -> (World, Frame, Editor, BuildTool) {
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
        matrices.iter().zip(&lib.rig.default_world_inv).map(|(m, i)| *m * *i).collect(),
    );
    let frame = Frame::new(&world).unwrap();
    let mut editor = Editor::default();
    editor.toggle();
    editor.library_demo(&world, "");
    assert!(editor.add_profile(false, &world, &frame));
    editor.select(lib.rig.names["mWristLeft"], &lib.rig, &frame);
    editor.commit();
    let mut build = BuildTool::default();
    build.props.extend(demo::hand_ik::properties());
    (world, frame, editor, build)
}

#[test]
fn compact_default_ik_window_keeps_two_bindings_and_smoothing_visible_without_scrolling() {
    let (world, frame, mut editor, _) = fixture();
    let source = editor.selected.unwrap();
    for (local, name) in [(demo::hand_ik::LEFT, "Left"), (demo::hand_ik::RIGHT, "Right")] {
        editor.links.adding = true;
        assert!(editor.bind_target(
            &world,
            &frame,
            source,
            Target::Prim {
                id: demo::action_id(local),
                name: name.into()
            }
        ));
    }
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let proj = Projector {
        view_proj: Mat4::IDENTITY,
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    };
    let mut output = egui::FullOutput::default();
    for i in 0..4 {
        output.textures_delta.clear();
        output = ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(i as f64 / 60.0),
                ..Default::default()
            },
            |ui| editor.show(ui.ctx(), &palette, &world, &proj),
        );
    }
    let visible = |text: &str| {
        output
            .shapes
            .iter()
            .filter(|s| {
                matches!(&s.shape,
        egui::Shape::Text(t) if t.galley.text() == text && s.clip_rect.contains(t.pos + t.galley.size() * 0.5))
            })
            .count()
    };
    for key in [
        "ik-activation-in",
        "ik-activation-out",
        "ik-binding-filter",
        "ik-edit-copy-data",
        "ik-edit-reset-bone",
    ] {
        assert!(visible(aurora_i18n::text(key)) > 0, "default window clips {key}");
    }
    for name in ["Left", "Right"] {
        assert_eq!(visible(name), 1, "both target rows must be visible without scrolling");
    }
    assert_eq!(visible(aurora_i18n::text("ik-edit-weight")), 2, "both influence rows must fit too");
    assert_eq!(visible(aurora_i18n::text("ik-edit-root")), 0, "advanced chain starts collapsed");
}

#[test]
fn binding_search_and_clipboard_feedback_preserve_calibration_and_selection() {
    let (world, frame, mut editor, _) = fixture();
    let source = editor.selected.unwrap();
    for (local, name) in [(demo::hand_ik::LEFT, "Left"), (demo::hand_ik::RIGHT, "Right")] {
        editor.links.adding = true;
        assert!(editor.bind_target(
            &world,
            &frame,
            source,
            Target::Prim {
                id: demo::action_id(local),
                name: name.into(),
            }
        ));
    }
    editor.graph.definition.bindings[0].weight = 0.25;
    editor.graph.definition.bindings[0].enabled = false;
    let before = editor.graph.definition.clone();
    let selection = (editor.selected, editor.graph.binding, editor.position);
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let draw = |editor: &mut Editor, events: Vec<egui::Event>, time: f64| {
        ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_max_width(510.0);
                editor.bindings_ui(ui, &palette, &world, &frame);
                editor.export_ui(ui, &palette);
            },
        )
    };
    let centre = |output: &egui::FullOutput, text: &str| {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text && s.clip_rect.contains(t.pos + t.galley.size() * 0.5) => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
    };
    let click = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let mut output = draw(&mut editor, vec![], 0.0);
    output.textures_delta.clear();
    output = draw(&mut editor, vec![], 0.1);
    for text in ["mWristLeft", "Left", "Right"] {
        assert!(centre(&output, text).is_some(), "full binding identity is readable: {text}");
    }
    assert_eq!(
        output
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape,
        egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text("ik-edit-contact-settings")))
            .count(),
        1,
        "only the selected association exposes its contact fold"
    );
    let search = centre(&output, aurora_i18n::text("ik-binding-filter")).unwrap();
    output.textures_delta.clear();
    for (at, events) in [
        (0.2, click(search, true)),
        (0.3, click(search, false)),
        (0.4, vec![egui::Event::Text("WRIST Right".into())]),
    ] {
        output = draw(&mut editor, events, at);
        output.textures_delta.clear();
    }
    output = draw(&mut editor, vec![], 0.5);
    assert_eq!(editor.links.filter, "WRIST Right");
    assert!(centre(&output, "Right").is_some());
    assert!(centre(&output, "Left").is_none());
    assert_eq!(editor.graph.definition, before);
    assert_eq!((editor.selected, editor.graph.binding, editor.position), selection);
    for (i, key) in ["ik-edit-copy-config", "ik-edit-copy-data"].into_iter().enumerate() {
        let pos = centre(&output, aurora_i18n::text(key)).unwrap();
        let time = 4.0 * i as f64 + 1.0;
        let expected = if i == 0 {
            editor.script().unwrap()
        } else {
            editor.graph_rig_declaration().unwrap()
        };
        output.textures_delta.clear();
        output = draw(&mut editor, click(pos, true), time);
        output.textures_delta.clear();
        output = draw(&mut editor, click(pos, false), time + 0.1);
        assert!(output.platform_output.commands.iter().any(|c| matches!(c,
            egui::OutputCommand::CopyText(text) if *text == expected)));
        output.textures_delta.clear();
        output = draw(&mut editor, vec![], time + 0.2);
        assert!(centre(&output, aurora_i18n::text("ik-copy-done")).is_some());
        output.textures_delta.clear();
        output = draw(&mut editor, vec![], time + 2.2);
        assert!(centre(&output, aurora_i18n::text("ik-copy-done")).is_none());
    }
    editor.links.filter = "absent".into();
    output.textures_delta.clear();
    output = draw(&mut editor, vec![], 9.0);
    assert!(centre(&output, aurora_i18n::text("ik-binding-no-match")).is_some());
    assert_eq!(editor.graph.definition, before);
    assert!(editor.add_profile(false, &world, &frame));
    assert!(
        editor.links.filter.is_empty(),
        "changing profile must not hide its contacts behind an old search"
    );
}

#[test]
fn guided_pick_binds_immediately_and_reselects_either_type_without_moving_or_changing_the_source() {
    let (world, frame, mut editor, build) = fixture();
    let source = editor.selected.unwrap();
    let desired = editor.goals[0].position;
    let activation = crate::world::rig_graph::Activation {
        distance: 0.3,
        ease_in: 0.4,
        ease_out: 0.9,
    };
    for local in [demo::hand_ik::LEFT, demo::hand_ik::RIGHT] {
        editor.begin_link(&world, &frame, source, true);
        let index = world.objects.index_of_uuid(&demo::action_id(local)).unwrap();
        assert!(editor.picked_prim(&world, &build, index));
        assert!(!editor.picking);
        assert_eq!(editor.selected, Some(source));
        assert_eq!(editor.graph.definition.bindings.len(), 1);
        if local == demo::hand_ik::LEFT {
            editor.graph.definition.bindings[0].activation = activation;
        }
        assert_eq!(editor.graph.definition.bindings[0].activation, activation);
        assert!(matches!(editor.graph.definition.bindings[0].target,Target::Prim { id,.. } if id==demo::action_id(local)));
        editor.update_graph(&world, &frame);
        assert!(
            editor
                .goals
                .iter()
                .find(|g| g.bone == source)
                .unwrap()
                .position
                .abs_diff_eq(desired, 1e-5)
        );
    }
    editor.begin_link(&world, &frame, source, false);
    let target = Target::Bone {
        avatar: Uuid::nil(),
        bone: "mHead".into(),
    };
    assert!(editor.picked_bone(&world, &frame, target.clone()));
    assert_eq!(editor.selected, Some(source));
    assert_eq!(editor.graph.definition.bindings[0].target, target);
    assert_eq!(editor.graph.definition.bindings[0].activation, activation);
    editor.update_graph(&world, &frame);
    assert!(
        editor
            .goals
            .iter()
            .find(|g| g.bone == source)
            .unwrap()
            .position
            .abs_diff_eq(desired, 1e-5)
    );
}

#[test]
fn same_bone_associations_keep_independent_calibration_selection_retargeting_and_exports() {
    let (world, frame, mut editor, build) = fixture();
    let rig = &world.avatar_lib.rig;
    let bone = editor.selected.unwrap();
    for local in [demo::hand_ik::LEFT, demo::hand_ik::RIGHT] {
        editor.begin_new_link(&world, &frame, bone, true);
        assert!(editor.picked_prim(&world, &build, world.objects.index_of_uuid(&demo::action_id(local)).unwrap()));
    }
    assert_eq!(editor.graph.definition.bindings.len(), 2);
    let second = editor.graph.definition.bindings[1].clone();
    editor.select_binding(0, &world, &frame);
    editor.contact = Vec3::new(0.01, 0.02, -0.03);
    editor.position += Vec3::Z * 0.05;
    editor.commit();
    assert_eq!(editor.graph.definition.bindings[0].contact, editor.contact.to_array());
    assert_eq!(editor.graph.definition.bindings[1], second);
    editor.set_binding_weight(bone, rig, 0.4);
    editor.set_binding_orientation(bone, rig, false);
    assert_eq!(editor.graph.definition.bindings[0].weight, 0.4);
    assert!(!editor.graph.definition.bindings[0].orient);
    assert_eq!(editor.graph.definition.bindings[1], second);
    editor.update_graph(&world, &frame);
    editor.select_binding(1, &world, &frame);
    assert_eq!(editor.contact.to_array(), second.contact);
    assert_eq!(editor.weight, second.weight);
    assert_eq!(editor.graph.binding, Some(1));
    editor.graph.definition.bindings[0].enabled = false;
    editor.sync_graph_activation(bone, rig);
    assert!(!editor.disabled.contains(&bone));
    assert_eq!(editor.request().goals.len(), 1);
    let exported: crate::world::rig_graph::Definition = serde_json::from_str(&editor.graph_payload().unwrap()).unwrap();
    let mut expected = editor.graph.definition.bindings.clone();
    expected[0].target = Target::Link {
        object: demo::action_id(demo::hand_ik::ROOT),
        link: "Left".into(),
    };
    expected[1].target = Target::Link {
        object: demo::action_id(demo::hand_ik::ROOT),
        link: "Right".into(),
    };
    assert_eq!(exported.bindings, expected);
    assert!(exported.validate().is_ok());
    // The row pencil replaces its selected slot; adding never overwrites it.
    let first = editor.graph.definition.bindings[0].clone();
    editor.begin_link(&world, &frame, bone, false);
    assert!(editor.picked_bone(
        &world,
        &frame,
        Target::Bone {
            avatar: Uuid::nil(),
            bone: "mHead".into()
        }
    ));
    assert_eq!(editor.graph.definition.bindings.len(), 2);
    assert_eq!(editor.graph.definition.bindings[0], first);
    assert!(matches!(editor.graph.definition.bindings[1].target, Target::Bone { .. }));
    editor.remove_graph_binding(0, &world, &frame);
    assert_eq!(editor.graph.definition.bindings.len(), 1);
    assert_eq!(editor.graph.binding, Some(0));
    assert!(!editor.disabled.contains(&bone));
    assert_eq!(editor.request().goals.len(), 1);
    // The native clip editor keeps its own pose even when the IK association
    // slot is selected in the saved contact editor.
    let mut native = editor.graph.binding_goal(0, rig).unwrap();
    native.position += Vec3::X * 0.17;
    native.contact = Vec3::ZERO;
    editor.goals = vec![native.clone()];
    editor.clip.active = true;
    editor.select(bone, rig, &frame);
    assert!(editor.position.abs_diff_eq(native.position, 1e-6));
    assert_eq!(editor.contact, native.contact);
}

#[test]
fn inspector_separates_coordinate_frames_and_target_numeric_edits_preserve_other_binding_settings() {
    let (world, frame, mut editor, _) = fixture();
    let bone = editor.selected.unwrap();
    assert!(editor.bind_target(
        &world,
        &frame,
        bone,
        Target::Prim {
            id: demo::action_id(demo::hand_ik::RIGHT),
            name: "Right".into()
        }
    ));
    editor.begin_new_link(&world, &frame, bone, true);
    assert!(editor.finish_link(
        &world,
        &frame,
        Target::Prim {
            id: demo::action_id(demo::hand_ik::LEFT),
            name: "Left".into()
        }
    ));
    editor.ensure_profiles(&world, &frame);
    editor.select_binding(0, &world, &frame);
    let before = editor.graph.definition.clone();
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let mut time = 0.0;
    let mut draw = |editor: &mut Editor, events| {
        time += 1.0 / 60.0;
        ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 1400.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_max_width(510.0);
                editor.links_ui(ui, &palette, &world, &frame);
            },
        )
    };
    let text_centres = |output: &egui::FullOutput, key: &'static str| {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(key) => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let mut output = draw(&mut editor, vec![]);
    output.textures_delta.clear();
    output = draw(&mut editor, vec![]);
    assert!(text_centres(&output, "ik-inspector-target-frame").is_empty());
    assert!(
        text_centres(&output, "ik-inspector-position").is_empty(),
        "avatar-frame controls start collapsed"
    );
    for key in ["ik-inspector-chain", "ik-inspector-avatar-frame", "ik-edit-contact-settings"] {
        let header = text_centres(&output, key)[0];
        for pressed in [true, false] {
            output.textures_delta.clear();
            output = draw(
                &mut editor,
                vec![
                    egui::Event::PointerMoved(header),
                    egui::Event::PointerButton {
                        pos: header,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        for _ in 0..20 {
            output.textures_delta.clear();
            output = draw(&mut editor, vec![]);
        }
    }
    assert_eq!(
        editor.graph.definition, before,
        "expanding the inspector must not alter calibration"
    );
    let captions = ["ik-inspector-avatar-frame", "ik-inspector-target-frame", "ik-inspector-bone-frame"].map(|key| {
        let centres = text_centres(&output, key);
        assert_eq!(centres.len(), 1);
        centres[0]
    });
    assert!(captions.windows(2).all(|pair| pair[0].y < pair[1].y));
    let positions = text_centres(&output, "ik-inspector-position");
    let rotations = text_centres(&output, "ik-inspector-rotation");
    assert_eq!((positions.len(), rotations.len()), (3, 2));
    let values = |row: Pos2| {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t)
                    if t.pos.x > row.x
                        && !["X", "Y", "Z"].contains(&t.galley.text().trim())
                        && (t.pos.y + t.galley.size().y * 0.5 - row.y).abs() < 2.0 =>
                {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for index in 0..2 {
        let p = values(positions[index]);
        let r = values(rotations[index]);
        assert_eq!((p.len(), r.len()), (3, 3));
        assert!(
            p.iter().zip(&r).all(|(p, r)| (p.x - r.x).abs() < 1.0),
            "position and rotation columns must align"
        );
    }
    let target_x = values(positions[1])[0];
    output.textures_delta.clear();
    for (pressed, dx) in [(Some(true), 0.0), (None, 16.0), (Some(false), 16.0)] {
        let pos = target_x + egui::vec2(dx, 0.0);
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        output = draw(&mut editor, events);
        output.textures_delta.clear();
    }
    let after = &editor.graph.definition.bindings[0];
    assert!((after.offset.p().x - before.bindings[0].offset.p().x).abs() > 0.001);
    assert!((after.offset.p().y - before.bindings[0].offset.p().y).abs() < 1e-6);
    assert!((after.offset.p().z - before.bindings[0].offset.p().z).abs() < 1e-6);
    let mut expected = before;
    expected.bindings[0].offset = after.offset;
    assert_eq!(editor.graph.definition, expected);
}

#[test]
fn activation_controls_are_in_the_selected_binding_card_and_editing_ignores_runtime_gates() {
    let (world, frame, mut editor, build) = fixture();
    let source = editor.selected.unwrap();
    editor.begin_link(&world, &frame, source, true);
    assert!(editor.picked_prim(
        &world,
        &build,
        world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::LEFT)).unwrap()
    ));
    editor.graph.definition.bindings[0].activation = crate::world::rig_graph::Activation {
        distance: 0.001,
        ease_in: 10.0,
        ease_out: 20.0,
    };
    editor.update_graph(&world, &frame);
    assert_eq!(editor.request().goals.iter().find(|g| g.bone == source).unwrap().weight, 1.0);
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let mut labels = Vec::new();
    for _ in 0..3 {
        let mut output = ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 1200.0))),
                ..Default::default()
            },
            |ui| editor.links_ui(ui, &palette, &world, &frame),
        );
        labels = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        output.textures_delta.clear();
    }
    for key in [
        "ik-smoothing-global",
        "ik-activation-distance",
        "ik-activation-in",
        "ik-activation-out",
    ] {
        assert!(
            labels.iter().any(|label| label == aurora_i18n::text(key)),
            "missing activation control {key}"
        );
    }
    let declaration = editor.graph_rig_declaration().unwrap();
    let raw: String = serde_json::from_str(declaration.strip_prefix("string RIG = ").unwrap().strip_suffix(';').unwrap()).unwrap();
    let exported: crate::world::rig_graph::Definition = serde_json::from_str(&raw).unwrap();
    assert_eq!(exported.bindings[0].activation, editor.graph.definition.bindings[0].activation);
}

#[test]
fn visible_timing_fields_edit_global_fades_but_only_the_selected_bones_distance_and_export_both_formats() {
    let (world, frame, mut editor, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let left = rig.names["mWristLeft"];
    let right = rig.names["mWristRight"];
    assert!(editor.bind_target(
        &world,
        &frame,
        left,
        Target::Bone {
            avatar: Uuid::nil(),
            bone: "mHead".into()
        }
    ));
    editor.graph.definition.bindings[0].activation.distance = 0.5;
    editor.select(right, rig, &frame);
    editor.commit();
    editor.target = Some(demo::action_id(demo::hand_ik::RIGHT));
    editor.target_name = "Right".into();
    editor.attach(&world, &frame);
    editor.bindings[0].definition.activation.distance = 0.9;
    editor.set_global_smoothing([0.6, 0.8]);
    editor.enable(right, false, rig, &frame);
    editor.select(left, rig, &frame);
    let before = editor.graph_definition();

    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let mut time = 0.0;
    let mut draw = |editor: &mut Editor, events| {
        time += 1.0 / 60.0;
        ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 1400.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_max_width(510.0);
                editor.links_ui(ui, &palette, &world, &frame);
            },
        )
    };
    for _ in 0..3 {
        draw(&mut editor, vec![]).textures_delta.clear();
    }
    // Drag the actual visible numeric widgets, rather than calling setters.
    for key in ["ik-activation-in", "ik-activation-out", "ik-activation-distance"] {
        let mut output = draw(&mut editor, vec![]);
        let label = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(key) => {
                    Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
                }
                _ => None,
            })
            .unwrap();
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.pos.x > label.right() && (t.pos.y + t.galley.size().y * 0.5 - label.center().y).abs() < 2.0 => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing numeric timing field beside {key}"));
        output.textures_delta.clear();
        output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
        let end = pos + egui::vec2(25.0, 0.0);
        output = draw(&mut editor, vec![egui::Event::PointerMoved(end)]);
        output.textures_delta.clear();
        output = draw(
            &mut editor,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        output.textures_delta.clear();
    }
    let definition = editor.graph_definition();
    let a = definition.bindings[0].activation;
    let b = definition.bindings[1].activation;
    assert!(a.ease_in > 0.6 && a.ease_out > 0.8 && a.distance > 0.5);
    assert_eq!([b.ease_in, b.ease_out], [a.ease_in, a.ease_out]);
    assert_eq!(b.distance, 0.9);
    for (binding, old) in definition.bindings.iter().zip(&before.bindings) {
        let mut expected = old.clone();
        expected.activation = binding.activation;
        assert_eq!(*binding, expected, "timing edits must not change calibration or re-enable bindings");
    }
    let declaration = editor.graph_rig_declaration().unwrap();
    assert!(editor.graph_script().unwrap().contains(&declaration));
    let raw: String = serde_json::from_str(declaration.strip_prefix("string RIG = ").unwrap().strip_suffix(';').unwrap()).unwrap();
    assert_eq!(
        serde_json::from_str::<crate::world::rig_graph::Definition>(&raw).unwrap(),
        definition
    );

    // New associations inherit the global fades, but have their own gate defaults.
    let foot = rig.names["mAnkleLeft"];
    assert!(editor.bind_target(
        &world,
        &frame,
        foot,
        Target::Bone {
            avatar: Uuid::nil(),
            bone: "mHead".into()
        }
    ));
    let added = editor.binding_activation(foot, rig).unwrap();
    assert_eq!([added.ease_in, added.ease_out], [a.ease_in, a.ease_out]);
    assert_eq!(added.distance, 0.0);
    assert!(editor.bind_target(
        &world,
        &frame,
        left,
        Target::Bone {
            avatar: Uuid::nil(),
            bone: "mNeck".into()
        }
    ));
    assert_eq!(editor.binding_activation(left, rig), Some(a));
}

#[test]
fn global_smoothing_can_be_set_before_binding_and_imported_mixed_durations_are_not_silently_rewritten() {
    let (world, frame, mut editor, _) = fixture();
    // This regression exercises raw imported timings before profile grouping.
    editor.graph.definition.profiles.clear();
    editor.graph.profile = None;
    let rig = &world.avatar_lib.rig;
    assert_eq!(editor.global_smoothing(), ([0.2, 0.2], false));
    editor.set_global_smoothing([0.4, 0.9]);
    for (bone, prim) in [("mWristLeft", demo::hand_ik::LEFT), ("mWristRight", demo::hand_ik::RIGHT)] {
        editor.select(rig.names[bone], rig, &frame);
        editor.commit();
        editor.target = Some(demo::action_id(prim));
        editor.attach(&world, &frame);
    }
    assert!(
        editor
            .bindings
            .iter()
            .all(|b| [b.definition.activation.ease_in, b.definition.activation.ease_out] == [0.4, 0.9])
    );
    editor.bindings[1].definition.activation.ease_out = 1.4;
    let before = editor.graph_definition();
    let ctx = egui::Context::default();
    let palette = crate::theme::Theme::default().palette();
    let output = ctx.run_headless(egui::RawInput::default(), |ui| editor.smoothing_ui(ui, &palette));
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text("ik-smoothing-mixed")))
    );
    assert_eq!(editor.graph_definition(), before);
    assert_eq!(editor.global_smoothing(), ([0.4, 0.9], true));
    editor.set_global_smoothing([0.0, 0.0]);
    assert_eq!(editor.global_smoothing(), ([0.0, 0.0], false));
    editor.reset_pose();
    assert_eq!(editor.global_smoothing(), ([0.0, 0.0], false));
}
#[test]
fn invalid_and_cancelled_reselection_leave_existing_associations_intact_and_remote_targets_are_not_authored() {
    let (world, frame, mut editor, build) = fixture();
    let source = editor.selected.unwrap();
    editor.begin_link(&world, &frame, source, true);
    assert!(editor.picked_prim(
        &world,
        &build,
        world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::LEFT)).unwrap()
    ));
    let before = editor.graph.definition.clone();
    editor.begin_link(&world, &frame, source, false);
    for target in [
        Target::Bone {
            avatar: Uuid::nil(),
            bone: "mWristLeft".into(),
        },
        Target::Bone {
            avatar: Uuid::nil(),
            bone: "mElbowLeft".into(),
        },
        Target::Bone {
            avatar: demo::DEMO_LOUP,
            bone: "mHead".into(),
        },
    ] {
        assert!(!editor.picked_bone(&world, &frame, target));
        assert_eq!(editor.graph.definition, before);
        assert_eq!(editor.selected, Some(source));
    }
    assert!(editor.links.error && editor.picking);
    editor.cancel_link_pick();
    assert!(!editor.picking && !editor.links.bone_pick);
    assert_eq!(editor.graph.definition, before);
}
#[test]
fn visible_tabs_and_real_button_clicks_keep_the_source_selected_when_picking_an_own_bone() {
    let (mut world, frame, mut editor, _) = fixture();
    editor.graph.definition.profiles[0].name = "Drive".into();
    assert!(editor.add_profile(false, &world, &frame));
    editor.graph.definition.profiles[1].name = "Spare".into();
    editor.switch_profile(0, &world, &frame);
    let source = editor.selected.unwrap();
    let target = world.avatar_lib.rig.names["mElbowRight"];
    let point = frame.joints[target].w_axis.truncate();
    let eye = point + Vec3::new(3.0, -3.0, 2.0);
    let view = glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z);
    let proj = Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05) * view,
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    };
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let mut time = 0.0;
    let mut draw = |editor: &mut Editor, events: Vec<egui::Event>| {
        time += 1.0 / 60.0;
        ctx.run_headless(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| editor.show(ui.ctx(), &palette, &world, &proj),
        )
    };
    let text_pos = |output: &egui::FullOutput, key: &'static str| {
        output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t)
                if t.galley.text() == aurora_i18n::text(key)
                    && s.clip_rect.contains(t.pos + t.galley.size() * 0.5)
                    && ctx
                        .memory(|m| m.area_rect(egui::Id::unique("ik-edit")))
                        .is_none_or(|r| r.contains(t.pos + t.galley.size() * 0.5)) =>
            {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
    };
    let mut output = draw(&mut editor, vec![]);
    output.textures_delta.clear();
    output = draw(&mut editor, vec![]);
    let tabs: Vec<_> = [
        "ik-edit-tab-bindings",
        "ik-edit-tab-virtual",
        "ik-edit-tab-animations",
        "ik-edit-tab-visible",
    ]
    .into_iter()
    .map(|key| text_pos(&output, key).unwrap())
    .collect();
    assert!(tabs.windows(2).all(|pair| pair[0].x < pair[1].x));
    assert!(
        text_pos(&output, "ik-edit-tab-pose").is_none_or(|pose| pose.y > tabs[0].y + 20.0),
        "the selected-bone Pose heading is not a separate tab"
    );
    for key in ["ik-inspector-position", "ik-inspector-rotation"] {
        assert!(text_pos(&output, key).is_none(), "avatar-frame fields start collapsed");
    }
    for key in [
        "ik-edit-local",
        "ik-links-source-hint",
        "ik-edit-enabled",
        "ik-links-repick-prim",
        "ik-links-repick-bone",
        "ik-graph-external-control",
        "ik-links-add",
        "ik-edit-chain",
        "ik-edit-direct",
        "ik-edit-bindings-hint",
        "common-cancel",
    ] {
        assert!(text_pos(&output, key).is_none());
    }
    for key in ["ik-edit-detach", "ik-edit-root"] {
        assert!(text_pos(&output, key).is_none(), "advanced chain controls start collapsed");
    }
    let advanced = text_pos(&output, "ik-inspector-chain").unwrap();
    for pressed in [true, false] {
        output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(advanced),
                egui::Event::PointerButton {
                    pos: advanced,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
    }
    for _ in 0..20 {
        output = draw(&mut editor, vec![]);
        output.textures_delta.clear();
    }
    for key in [
        "ik-edit-detach",
        "ik-edit-root",
        "ik-edit-reset-bone",
        "ik-smoothing-global",
        "ik-activation-in",
        "ik-activation-out",
    ] {
        assert!(text_pos(&output, key).is_some(), "missing binding control {key}");
    }
    let copy = text_pos(&output, "ik-edit-copy-config").unwrap();
    assert!(text_pos(&output, "ik-edit-copy-data").unwrap().x > copy.x);
    let window = ctx.memory(|m| m.area_rect(egui::Id::unique("ik-edit"))).unwrap();
    assert!(
        copy.y > window.center().y,
        "script export should stay below the short association list"
    );
    for name in ["Drive", "Spare"] {
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == name && s.clip_rect.contains(t.pos + t.galley.size() * 0.5) => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap();
        for pressed in [true, false] {
            output = draw(
                &mut editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            output.textures_delta.clear();
        }
        output = draw(&mut editor, vec![]);
        output.textures_delta.clear();
    }
    assert_eq!(editor.graph.profile, Some(1));
    for key in ["ik-links-bind-prim", "ik-links-bind-bone"] {
        let pos = text_pos(&output, key).unwrap();
        for pressed in [true, false] {
            output = draw(
                &mut editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            output.textures_delta.clear();
        }
        assert!(editor.picking, "direct target action did not start picking: {key}");
        assert!(text_pos(&output, "common-cancel").is_none());
        if key == "ik-links-bind-prim" {
            output = draw(
                &mut editor,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            output.textures_delta.clear();
            assert!(!editor.picking);
            assert_eq!(editor.selected, Some(source));
            assert!(editor.graph.definition.bindings.is_empty());
            output = draw(&mut editor, vec![]);
            output.textures_delta.clear();
        }
    }
    assert!(editor.picking && editor.links.bone_pick);
    let pos = proj.project(point).unwrap().0;
    output = draw(&mut editor, vec![egui::Event::PointerMoved(pos)]);
    output.textures_delta.clear();
    for pressed in [true, false] {
        output = draw(
            &mut editor,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        output.textures_delta.clear();
    }
    assert_eq!(editor.selected, Some(source));
    assert!(!editor.picking);
    assert_eq!(editor.graph.definition.profiles[0].bindings, Some(vec![]));
    assert_eq!(editor.graph.definition.profiles[1].bindings, Some(vec![0]));
    assert_eq!(
        editor.graph.definition.bindings[0].target,
        Target::Bone {
            avatar: Uuid::nil(),
            bone: "mElbowRight".into()
        }
    );
    editor.links.reveal_menu = true;
    output = draw(&mut editor, vec![]);
    output.textures_delta.clear();
    output = draw(&mut editor, vec![]);
    let pos = text_pos(&output, "ik-graph-prim").unwrap();
    output.textures_delta.clear();
    for pressed in [true, false] {
        output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
    }
    assert!(editor.picking && !editor.links.bone_pick);
    assert_eq!(editor.links.source, Some(source));
    let target_id = demo::action_id(demo::hand_ik::RIGHT);
    let mut build = BuildTool::default();
    build.props.insert(
        target_id,
        aurora_net::build::ObjectProps {
            object_id: target_id,
            name: "Right".into(),
            ..Default::default()
        },
    );
    assert!(editor.picked_prim(&world, &build, world.objects.index_of_uuid(&target_id).unwrap()));
    assert_eq!(editor.selected, Some(source));
    let width = ctx.memory(|m| m.area_rect(egui::Id::unique("ik-edit"))).unwrap().width();
    for _ in 0..32 {
        let mut output = draw(&mut editor, vec![]);
        output.textures_delta.clear();
    }
    let after = ctx.memory(|m| m.area_rect(egui::Id::unique("ik-edit"))).unwrap().width();
    assert!(after <= width + 1.0, "association row expanded the floater: {width} -> {after}");
    let mut output = draw(&mut editor, vec![]);
    output.textures_delta.clear();
    let after_copy = text_pos(&output, "ik-edit-copy-config").unwrap();
    let after_window = ctx.memory(|m| m.area_rect(egui::Id::unique("ik-edit"))).unwrap();
    assert!(
        ((after_copy.y - after_window.bottom()) - (copy.y - window.bottom())).abs() < 1.0,
        "export must remain anchored to the floater while it is fitted into the work area"
    );
    if !output.shapes.iter().any(|s| {
        matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Right"
            && s.clip_rect.contains(t.pos + t.galley.size() * 0.5))
    }) {
        let pointer = window.center();
        output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::MouseWheel {
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -220.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
        for _ in 0..16 {
            output = draw(&mut editor, vec![]);
            output.textures_delta.clear();
        }
    }
    let target = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "Right" => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .unwrap();
    let options = text_pos(&output, "ik-links-options").unwrap();
    assert!(options.y > target.y);
    assert!(text_pos(&output, "ik-edit-ignore-scripts").unwrap().y > options.y);
    for key in [
        "ik-edit-hand-left",
        "ik-edit-hand-right",
        "ik-edit-foot-left",
        "ik-edit-foot-right",
        "ik-edit-symmetry",
    ] {
        assert!(text_pos(&output, key).unwrap().y > options.y);
    }
    let [hand_left, hand_right, foot_left, foot_right] =
        ["ik-edit-hand-left", "ik-edit-hand-right", "ik-edit-foot-left", "ik-edit-foot-right"].map(|key| text_pos(&output, key).unwrap());
    let symmetry = text_pos(&output, "ik-edit-symmetry").unwrap();
    assert!(hand_left.x > symmetry.x && foot_left.x > symmetry.x);
    assert!(hand_right.x > hand_left.x && foot_right.x > foot_left.x);
    assert!((hand_left.y - hand_right.y).abs() < 1.0);
    assert!((foot_left.y - foot_right.y).abs() < 1.0);
    assert!(foot_left.y > hand_left.y);
    assert!(
        !output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Gizmo"))
    );
    for pressed in [true, false] {
        let mut output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(target),
                egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
    }
    assert_eq!(editor.prims.edit, Some(target_id));
    for limb in limbs(&world.avatar_lib.rig).into_iter().filter(|l| l.end != source) {
        editor.select(limb.end, &world.avatar_lib.rig, &frame);
        editor.commit();
        assert!(editor.bind_target(
            &world,
            &frame,
            limb.end,
            Target::Prim {
                id: target_id,
                name: "Right".into()
            }
        ));
    }
    editor.select(source, &world.avatar_lib.rig, &frame);
    for _ in 0..2 {
        output = draw(&mut editor, vec![]);
        output.textures_delta.clear();
    }
    assert_eq!(editor.graph.definition.bindings.len(), 4);
    if text_pos(&output, "ik-edit-reset-all").is_none() {
        output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(window.center()),
                egui::Event::MouseWheel {
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -650.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
        for _ in 0..32 {
            output = draw(&mut editor, vec![]);
            output.textures_delta.clear();
        }
    }
    for key in ["ik-edit-reset-all", "ik-edit-world", "ik-edit-bone-local"] {
        assert!(
            text_pos(&output, key).is_some(),
            "option {key} remains reachable with four bindings"
        );
    }
    assert!(editor.open && editor.selected == Some(source));
    let footer_keys = [
        "ik-links-options",
        "ik-edit-hand-left",
        "ik-edit-hand-right",
        "ik-edit-foot-left",
        "ik-edit-foot-right",
        "ik-edit-symmetry",
        "ik-edit-ignore-scripts",
        "ik-edit-reset-all",
        "ik-edit-world",
        "ik-edit-bone-local",
        "ik-edit-copy-config",
        "ik-edit-copy-data",
    ];
    let baseline: Vec<_> = footer_keys.iter().map(|key| text_pos(&output, key).unwrap()).collect();
    // Long Bindings and Display lists scroll independently of this shared area.
    // Short Virtual bones and Animations bodies must keep it in the same place.
    for tab in [4, 2, 3, 1] {
        editor.tab = tab;
        for events in [
            vec![
                egui::Event::PointerMoved(window.center()),
                egui::Event::MouseWheel {
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(0.0, -1200.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![],
        ] {
            output = draw(&mut editor, events);
            output.textures_delta.clear();
        }
        for (key, before) in footer_keys.iter().zip(&baseline) {
            let after = text_pos(&output, key).unwrap_or_else(|| panic!("missing shared option {key} in tab {tab}"));
            assert!(
                after.distance(*before) < 1.0,
                "footer moved for {key} in tab {tab}: {before:?} -> {after:?}"
            );
        }
    }
    editor.tab = 3;
    output = draw(&mut editor, vec![]);
    let symmetry = text_pos(&output, "ik-edit-symmetry").unwrap();
    output.textures_delta.clear();
    let before = editor.symmetry;
    for pressed in [true, false] {
        output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(symmetry),
                egui::Event::PointerButton {
                    pos: symmetry,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
    }
    assert_ne!(editor.symmetry, before, "shared controls remain interactive outside Bindings");
    let mut settings = crate::build::BuildSettings::default();
    assert!(editor.apply_prim_edit(&mut world, &mut build, &mut settings));
    assert!(build.open && settings.edit_linked);
    assert_eq!(
        build.selection,
        vec![world.objects.get(world.objects.index_of_uuid(&target_id).unwrap()).unwrap().key]
    );
}

#[test]
fn binding_controls_detach_preserve_offsets_and_reset_selected_or_all_contacts() {
    let (world, frame, mut editor, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let source = editor.selected.unwrap();
    assert!(editor.bind_target(
        &world,
        &frame,
        source,
        Target::Prim {
            id: demo::action_id(demo::hand_ik::LEFT),
            name: "Left".into(),
        }
    ));
    let before = editor.graph.definition.bindings[0].clone();
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let mut time = 0.0;
    let mut draw = |editor: &mut Editor, events| {
        time += 1.0 / 60.0;
        ctx.run_headless(
            egui::RawInput {
                events,
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                ui.set_max_width(510.0);
                editor.links_ui(ui, &palette, &world, &frame);
                editor.footer_ui(ui, &palette, &world, &frame);
            },
        )
    };
    let mut output = draw(&mut editor, vec![]);
    output.textures_delta.clear();
    let advanced = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text("ik-inspector-chain") => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .unwrap();
    for pressed in [true, false] {
        output = draw(
            &mut editor,
            vec![
                egui::Event::PointerMoved(advanced),
                egui::Event::PointerButton {
                    pos: advanced,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        output.textures_delta.clear();
    }
    for _ in 0..20 {
        output = draw(&mut editor, vec![]);
        output.textures_delta.clear();
    }
    for (key, direct) in [("ik-edit-detach", true), ("ik-edit-detach", false), ("ik-edit-bone-local", false)] {
        output = draw(&mut editor, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(key) => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .unwrap();
        output.textures_delta.clear();
        for pressed in [true, false] {
            output = draw(
                &mut editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            output.textures_delta.clear();
        }
        assert_eq!(
            editor.graph.definition.bindings[0].mode,
            if direct { Mode::Direct } else { Mode::Ik }
        );
        assert_eq!(editor.graph.definition.bindings[0].root, before.root);
        assert!(editor.graph.definition.bindings[0].offset.p().abs_diff_eq(before.offset.p(), 1e-5));
        assert!(editor.graph.definition.bindings[0].offset.q().dot(before.offset.q()).abs() > 0.99999);
    }
    assert!(editor.space == Space::Local);
    let other = rig.names["mWristRight"];
    editor.select(other, rig, &frame);
    editor.commit();
    editor.select(source, rig, &frame);
    for key in ["ik-edit-reset-bone", "ik-edit-reset-all"] {
        output = draw(&mut editor, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(key) => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .unwrap();
        output.textures_delta.clear();
        for pressed in [true, false] {
            output = draw(
                &mut editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            output.textures_delta.clear();
        }
        assert!(!editor.enabled(source));
        assert_eq!(editor.goals.len(), usize::from(key == "ik-edit-reset-bone"));
        assert!(editor.graph.definition.bindings.is_empty());
    }
    assert!(!editor.enabled(other));
}

#[test]
fn association_strength_edits_preserve_calibration_activation_and_selection_for_both_formats() {
    for advanced in [false, true] {
        let (world, frame, mut editor, _) = fixture();
        let rig = &world.avatar_lib.rig;
        let source = editor.selected.unwrap();
        let target = Target::Prim {
            id: demo::action_id(demo::hand_ik::LEFT),
            name: "Left".into(),
        };
        if advanced {
            assert!(editor.bind_target(&world, &frame, source, target));
        } else {
            editor.target = Some(demo::action_id(demo::hand_ik::LEFT));
            editor.attach(&world, &frame);
        }
        editor.enable(source, false, rig, &frame);
        let other = rig.names["mWristRight"];
        editor.select(other, rig, &frame);
        editor.commit();
        editor.symmetry = true;
        let graph_before = editor.graph.definition.clone();
        let legacy_before = editor.bindings.first().map(|b| serde_json::to_value(&b.definition).unwrap());
        let position_before = editor.position;
        editor.set_binding_weight(source, rig, 0.25);
        assert_eq!(editor.selected, Some(other));
        assert_eq!(editor.position, position_before);
        assert_eq!(editor.weight, 1.0);
        assert!(!editor.enabled(source));
        assert_eq!(editor.goals.iter().find(|g| g.bone == other).unwrap().weight, 1.0);
        if advanced {
            let mut expected = graph_before;
            expected.bindings[0].weight = 0.25;
            assert_eq!(editor.graph.definition, expected);
            editor.update_graph(&world, &frame);
        } else {
            let mut expected = legacy_before.unwrap();
            expected["weight"] = serde_json::json!(0.25);
            assert_eq!(serde_json::to_value(&editor.bindings[0].definition).unwrap(), expected);
            editor.update_bindings(&world, &frame);
        }
        assert_eq!(editor.goals.iter().find(|g| g.bone == source).unwrap().weight, 0.25);
        editor.select(source, rig, &frame);
        assert_eq!(editor.weight, 0.25);
        for weight in [0.0, 1.0] {
            editor.set_binding_weight(source, rig, weight);
            assert_eq!(editor.weight, weight);
            assert!(!editor.enabled(source));
        }
    }
}

#[test]
fn association_rotation_edits_preserve_offsets_disabled_state_and_other_bones() {
    for kind in 0..3 {
        let (world, frame, mut editor, _) = fixture();
        let rig = &world.avatar_lib.rig;
        let source = editor.selected.unwrap();
        if kind == 0 {
            editor.target = Some(demo::action_id(demo::hand_ik::LEFT));
            editor.attach(&world, &frame);
        } else {
            let target = if kind == 1 {
                Target::Prim {
                    id: demo::action_id(demo::hand_ik::LEFT),
                    name: "Left".into(),
                }
            } else {
                Target::Bone {
                    avatar: Uuid::nil(),
                    bone: "mHead".into(),
                }
            };
            assert!(editor.bind_target(&world, &frame, source, target));
        }
        let rotation = editor.goals.iter().find(|g| g.bone == source).unwrap().rotation.unwrap();
        let graph_before = editor.graph.definition.clone();
        let legacy_before = editor.bindings.first().map(|b| serde_json::to_value(&b.definition).unwrap());
        editor.enable(source, false, rig, &frame);
        let other = rig.names["mWristRight"];
        editor.select(other, rig, &frame);
        editor.commit();
        let other_goal = editor.goals.iter().find(|g| g.bone == other).unwrap().clone();
        let selected_pose = (editor.position, editor.rotation(), editor.orient);
        editor.symmetry = true;
        for orient in [false, true, false, true] {
            editor.set_binding_orientation(source, rig, orient);
            assert!(!editor.enabled(source));
            assert_eq!(editor.selected, Some(other));
            assert_eq!((editor.position, editor.rotation(), editor.orient), selected_pose);
            let goal = editor.goals.iter().find(|g| g.bone == source).unwrap();
            assert_eq!(goal.rotation.is_some(), orient);
            if let Some(actual) = goal.rotation {
                assert!(actual.dot(rotation).abs() > 0.99999);
            }
            let actual_other = editor.goals.iter().find(|g| g.bone == other).unwrap();
            assert_eq!(actual_other.position, other_goal.position);
            assert_eq!(actual_other.rotation, other_goal.rotation);
            if kind == 0 {
                let mut expected = legacy_before.clone().unwrap();
                if !orient {
                    expected["rotation"] = serde_json::Value::Null;
                }
                assert_eq!(serde_json::to_value(&editor.bindings[0].definition).unwrap(), expected);
            } else {
                let mut expected = graph_before.clone();
                expected.bindings[0].orient = orient;
                expected.bindings[0].enabled = false;
                assert_eq!(editor.graph.definition, expected);
            }
        }
        editor.select(source, rig, &frame);
        editor.set_binding_orientation(source, rig, false);
        assert!(!editor.orient && !editor.enabled(source));
        editor.set_binding_orientation(source, rig, true);
        assert!(editor.orient && editor.rotation().dot(rotation).abs() > 0.99999);
        assert!(!editor.enabled(source));
    }
}
