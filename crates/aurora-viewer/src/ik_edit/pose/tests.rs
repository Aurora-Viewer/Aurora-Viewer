use super::*;
use crate::{demo, scene::avatar::AvatarLibrary, world::rig_graph::Target};
use aurora_net::NetEvent;
use std::sync::Arc;

fn fixture() -> (World, Editor) {
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
    let mut editor = Editor::default();
    editor.toggle();
    editor.library_demo(&world, "");
    assert!(editor.add_profile(false, &world, &Frame::new(&world).unwrap()));
    (world, editor)
}

fn evaluate(world: &mut World, state: &mut ik::Pose, editor: &Editor) -> ik::LocalPose {
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

#[test]
fn toggling_limb_ik_keeps_the_selected_chain_root_and_exported_binding() {
    for format in 0..3 {
        for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
            let (world, mut editor) = fixture();
            let rig = &world.avatar_lib.rig;
            let frame = Frame::new(&world).unwrap();
            let bone = rig.names[name];
            let root = ik::default_root(rig, bone);
            editor.enable_limb(bone, true, rig, &frame);
            editor.root = root;
            editor.commit();
            if format == 1 {
                editor.target = Some(demo::action_id(demo::hand_ik::LEFT));
                editor.target_name = "Left".into();
                editor.attach(&world, &frame);
            } else if format == 2 {
                assert!(editor.bind_target(
                    &world,
                    &frame,
                    bone,
                    Target::Prim {
                        id: demo::action_id(demo::hand_ik::LEFT),
                        name: "Left".into(),
                    }
                ));
            }
            for _ in 0..3 {
                editor.enable_limb(bone, false, rig, &frame);
                editor.enable_limb(bone, true, rig, &frame);
                editor.update_bindings(&world, &frame);
                editor.update_graph(&world, &frame);
                assert_eq!(editor.root, root, "format={format}, bone={name}");
                assert_eq!(editor.goals.iter().find(|g| g.bone == bone).unwrap().root, root);
                if format == 1 {
                    assert_eq!(
                        editor.bindings[0].definition.root.as_deref(),
                        Some(rig.canonical_names[root].as_str())
                    );
                } else if format == 2 {
                    let export = editor.graph_definition();
                    assert_eq!(export.bindings[0].root.as_deref(), Some(rig.canonical_names[root].as_str()));
                }
            }
        }
    }
}

#[test]
fn released_limbs_accept_direct_edits_without_reenabling_ik_or_losing_bindings() {
    for format in 0..3 {
        for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
            let (mut world, mut editor) = fixture();
            let lib = world.avatar_lib.clone();
            let rig = &lib.rig;
            let bone = rig.names[name];
            let base = Frame::new(&world).unwrap();
            editor.enable_limb(bone, true, rig, &base);
            editor.position += Vec3::new(0.08, 0.02, 0.04);
            editor.set_rotation(Quat::from_rotation_y(0.2) * editor.rotation());
            editor.commit();
            if format == 1 {
                editor.target = Some(demo::action_id(demo::hand_ik::LEFT));
                editor.target_name = "Left".into();
                editor.attach(&world, &base);
            } else if format == 2 {
                assert!(editor.bind_target(
                    &world,
                    &base,
                    bone,
                    Target::Prim {
                        id: demo::action_id(demo::hand_ik::LEFT),
                        name: "Left".into(),
                    }
                ));
            }
            editor.update_bindings(&world, &base);
            editor.update_graph(&world, &base);
            let mut state = ik::Pose::default();
            evaluate(&mut world, &mut state, &editor);
            let active = Frame::new(&world).unwrap();
            let saved_position = editor.goals.iter().find(|g| g.bone == bone).unwrap().position;
            editor.enable_limb(bone, false, rig, &active);
            let released = evaluate(&mut world, &mut state, &editor);
            assert_eq!(released.rotations, rig.local_rot);
            assert_eq!(released.positions, rig.local_pos);
            let frame = Frame::new(&world).unwrap();
            editor.update_bindings(&world, &frame);
            editor.update_graph(&world, &frame);
            editor.select(bone, rig, &frame);
            editor.set_binding_weight(bone, rig, 0.7);
            editor.set_binding_weight(bone, rig, 1.0);
            assert!(
                !editor.enabled(bone),
                "passive refresh/selection/strength must not reactivate {name}"
            );
            assert!(editor.request().goals.is_empty());
            editor.refresh = false;
            editor.enable_limb(bone, true, rig, &frame);
            assert_eq!(
                editor.mode,
                Mode::Ik,
                "restoring an existing pin must synchronize the selected editor"
            );
            assert_eq!(editor.root, editor.goals.iter().find(|g| g.bone == bone).unwrap().root);
            assert!(editor.position.distance(saved_position) < 0.0003);
            editor.enable_limb(bone, false, rig, &frame);
            editor.select(bone, rig, &frame);
            assert!(
                editor
                    .goals
                    .iter()
                    .find(|g| g.bone == bone)
                    .unwrap()
                    .position
                    .distance(saved_position)
                    < 0.0003
            );
            let displayed = frame.model.inverse() * frame.joints[bone];
            assert!(editor.position.distance(displayed.transform_point3(editor.contact)) < 0.0003);
            assert_eq!(editor.mode, Mode::Ik, "release must not detach {name}");
            let root = frame.model.inverse().transform_point3(frame.joints[editor.root].w_axis.truncate());
            editor.position = editor.position.lerp(root, 0.1) + Vec3::X * 0.03;
            editor.set_rotation(Quat::from_rotation_z(0.15) * editor.rotation());
            editor.commit();
            editor.update_bindings(&world, &frame);
            editor.update_graph(&world, &frame);
            assert!(editor.limb_enabled(bone), "an explicit parented edit reactivates its chain");
            assert_eq!(editor.request().goals[0].mode, Mode::Ik);
            let local = evaluate(&mut world, &mut state, &editor);
            assert_eq!(
                local.positions, rig.local_pos,
                "format={format} {name}: release must not stretch segments"
            );
            let matrices = rig.world_with(&local.rotations, Vec3::ZERO, &local.positions, &rig.scale);
            assert!(
                matrices[bone].w_axis.truncate().distance(editor.position) < 0.003,
                "format={format} {name}: released edit must move"
            );
            editor.enable_limb(bone, false, rig, &Frame::new(&world).unwrap());
            evaluate(&mut world, &mut state, &editor);
            editor.select(bone, rig, &Frame::new(&world).unwrap());
            assert_eq!(editor.mode, Mode::Ik);
            // Free translation is an explicit creator choice, never a release default.
            editor.mode = Mode::Direct;
            editor.position += Vec3::new(0.09, -0.02, 0.04);
            editor.set_rotation(Quat::from_rotation_z(0.3) * editor.rotation());
            let desired = (editor.position, editor.rotation());
            editor.commit();
            editor.update_bindings(&world, &frame);
            editor.update_graph(&world, &frame);
            assert!(editor.enabled(bone));
            assert!(!editor.limb_enabled(bone), "manual posing must not turn the limb IK switch back on");
            let request = editor.request();
            assert_eq!(request.goals.len(), 1, "format={format} {name}");
            assert_eq!(request.goals[0].mode, Mode::Direct);
            let local = evaluate(&mut world, &mut state, &editor);
            let matrices = rig.world_with(&local.rotations, Vec3::ZERO, &local.positions, &rig.scale);
            assert!(
                matrices[bone].transform_point3(editor.contact).distance(desired.0) < 0.0003,
                "format={format} {name}"
            );
            assert!(matrices[bone].to_scale_rotation_translation().1.dot(desired.1).abs() > 0.99999);
            for (j, q) in local.rotations.iter().enumerate().filter(|(j, _)| *j != bone) {
                assert!(q.dot(rig.local_rot[j]).abs() > 0.99999, "direct posing only changes its own joint");
            }
            if format == 2 {
                let binding = &editor.graph.definition.bindings[0];
                assert!(binding.enabled && binding.mode == Mode::Direct);
            }
            let frame = Frame::new(&world).unwrap();
            editor.enable(bone, false, rig, &frame);
            editor.select(bone, rig, &frame);
            assert_eq!(editor.mode, Mode::Direct, "an explicit Detach choice survives release/selection");
            editor.enable_limb(bone, true, rig, &frame);
            editor.update_bindings(&world, &frame);
            editor.update_graph(&world, &frame);
            assert!(editor.limb_enabled(bone));
            assert_eq!(
                editor.goals.iter().find(|g| g.bone == bone).unwrap().root,
                ik::extended_root(rig, bone).unwrap()
            );
            assert_eq!(editor.request().goals[0].mode, Mode::Ik);
            editor.enable_limb(bone, false, rig, &frame);
            let released = evaluate(&mut world, &mut state, &editor);
            assert_eq!(released.rotations, rig.local_rot);
            assert_eq!(released.positions, rig.local_pos);
        }
    }
}

#[test]
fn unchecking_detach_restores_a_real_limb_chain_after_direct_binding_refresh() {
    for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
        let (world, mut editor) = fixture();
        let rig = &world.avatar_lib.rig;
        let bone = rig.names[name];
        let frame = Frame::new(&world).unwrap();
        editor.select(bone, rig, &frame);
        editor.mode = Mode::Direct;
        editor.position += Vec3::new(0.03, 0.0, 0.02);
        editor.target = Some(demo::action_id(demo::hand_ik::LEFT));
        editor.attach(&world, &frame);
        editor.update_bindings(&world, &frame);
        editor.select(bone, rig, &frame);
        assert_eq!(editor.root, bone);
        let desired = editor.position;
        let ctx = egui::Context::default();
        let palette = crate::theme::Theme::default().palette();
        let draw = |editor: &mut Editor, events| {
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| editor.chain_ui(ui, &palette, &world, &frame, bone),
            )
        };
        let mut output = draw(&mut editor, vec![]);
        let at = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text("ik-edit-detach") => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .unwrap();
        output.textures_delta.clear();
        for pressed in [true, false] {
            output = draw(
                &mut editor,
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            output.textures_delta.clear();
        }
        editor.update_bindings(&world, &frame);
        let goal = editor.goals.iter().find(|g| g.bone == bone).unwrap();
        assert_eq!(goal.mode, Mode::Ik);
        assert_eq!(goal.root, ik::extended_root(rig, bone).unwrap());
        assert!(goal.pole.is_some());
        assert!(goal.position.distance(desired) < 0.0003);
    }
}

#[test]
fn every_parented_joint_defaults_to_a_length_preserving_chain() {
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    for (bone, name) in rig.canonical_names.iter().enumerate() {
        editor.goals.clear();
        editor.select(bone, rig, &frame);
        assert_eq!(
            editor.mode,
            if rig.parent[bone].is_some() { Mode::Ik } else { Mode::Direct },
            "{name}: only the parentless root can translate freely by default"
        );
        editor.position += Vec3::new(0.03, 0.02, 0.01);
        editor.set_rotation(Quat::from_rotation_x(0.2) * editor.rotation());
        editor.commit();
        let mut state = ik::Pose::default();
        state.edit = editor.request();
        let local = state
            .apply(rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, Instant::now(), true)
            .unwrap();
        let matrices = rig.world_with(&local.rotations, Vec3::ZERO, &local.positions, &rig.scale);
        if rig.parent[bone].is_some() {
            assert_eq!(local.positions, rig.local_pos, "{name}: keep every parent segment");
        } else {
            assert!(matrices[bone].w_axis.truncate().distance(editor.position) < 0.0003, "{name}");
        }
        assert!(
            matrices[bone].to_scale_rotation_translation().1.dot(editor.rotation()).abs() > 0.99999,
            "{name}"
        );
    }
}

#[test]
fn actual_pointer_can_select_the_human_torso_and_move_rotate_a_released_foot() {
    for (name, mode) in [
        ("mTorso", Mode::Ik),
        ("mTorso", Mode::Direct),
        ("mAnkleRight", Mode::Ik),
        ("mAnkleRight", Mode::Direct),
    ] {
        let (mut world, mut editor) = fixture();
        let lib = world.avatar_lib.clone();
        let rig = &lib.rig;
        let bone = rig.names[name];
        let mut state = ik::Pose::default();
        if name == "mAnkleRight" {
            let frame = Frame::new(&world).unwrap();
            editor.enable_limb(bone, true, rig, &frame);
            editor.position += Vec3::X * 0.12;
            editor.commit();
            evaluate(&mut world, &mut state, &editor);
            editor.enable_limb(bone, false, rig, &Frame::new(&world).unwrap());
            evaluate(&mut world, &mut state, &editor);
        }
        editor.select(rig.names["mWristLeft"], rig, &Frame::new(&world).unwrap());
        editor.tab = 0;
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let palette = theme.palette();
        let mut time = 0.0;
        let projector = |point: Vec3| {
            let eye = point + Vec3::new(3.0, -4.0, 3.0);
            Projector {
                view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05)
                    * glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z),
                width: 1600.0,
                height: 900.0,
                ppp: 1.0,
            }
        };
        let mut draw = |editor: &mut Editor, world: &mut World, proj: &Projector, modifiers, pointer, pressed| {
            time += 1.0 / 60.0;
            let mut events = vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(pointer)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| editor.show(ui.ctx(), &palette, world, proj),
            );
            output.textures_delta.clear();
            evaluate(world, &mut state, editor);
        };
        let point = Frame::new(&world).unwrap().joints[bone].w_axis.truncate();
        let proj = projector(point);
        let at = proj.project(point).unwrap().0;
        for press in [None, None, Some(true), Some(false)] {
            draw(&mut editor, &mut world, &proj, egui::Modifiers::NONE, at, press);
        }
        assert_eq!(
            editor.selected,
            Some(bone),
            "overlapping extended spines must not steal the torso click"
        );
        assert_eq!(editor.mode, Mode::Ik, "selection/release must not detach {name}");
        editor.mode = mode;
        for modifiers in [egui::Modifiers::NONE, egui::Modifiers::CTRL] {
            let frame = Frame::new(&world).unwrap();
            let point = frame.joints[bone].transform_point3(editor.contact);
            let proj = projector(point);
            let length = Vec3::new(3.0, -4.0, 3.0).length() * ((if modifiers.ctrl { 100.0 } else { 50.0 }) / 900.0_f32 * 0.8).tan();
            let ring = |t: f32| proj.project(point + Vec3::new(t.cos(), t.sin(), 0.0) * length).unwrap().0;
            let (start, end) = if modifiers.ctrl {
                (ring(0.4), ring(0.9))
            } else {
                (
                    proj.project(point + Vec3::X * length * 0.85).unwrap().0,
                    proj.project(point + Vec3::X * (length * 0.85 + 0.12)).unwrap().0,
                )
            };
            let before = (editor.position, editor.rotation());
            for (mods, pointer, press) in [
                (modifiers, start, None),
                (modifiers, start, Some(true)),
                (modifiers, end, None),
                (egui::Modifiers::NONE, end, None),
                (egui::Modifiers::NONE, end, Some(false)),
                (egui::Modifiers::NONE, end, None),
            ] {
                draw(&mut editor, &mut world, &proj, mods, pointer, press);
            }
            assert_eq!(editor.mode, mode);
            assert!(!editor.limb_enabled(bone));
            if modifiers.ctrl {
                let (_, model_q, _) = frame.model.to_scale_rotation_translation();
                let expected = model_q.inverse() * Quat::from_rotation_z(0.5) * model_q * before.1;
                assert!(editor.rotation().dot(expected).abs() > 0.99999, "{name}: Ctrl rotation");
                assert!(editor.position.distance(before.0) < 0.0003);
            } else {
                assert!(editor.position.distance(before.0) > 0.1, "{name}: default translation gizmo");
            }
        }
        for name in ["mSpine1", "mSpine2", "mSpine3", "mSpine4"] {
            assert!(
                ctx.memory(|m| m.area_rect(egui::Id::unique(("ik-joint", rig.names[name]))))
                    .is_none()
            );
        }
    }
}
