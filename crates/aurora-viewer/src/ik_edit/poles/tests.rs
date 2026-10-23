use super::*;
use crate::scene::avatar::AvatarLibrary;

fn frame(rig: &Rig) -> Frame {
    Frame {
        model: Mat4::IDENTITY,
        joints: rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale),
        at: Instant::now(),
    }
}

fn projector(point: Vec3) -> Projector {
    let eye = point + Vec3::new(3.0, -4.0, 3.0);
    Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05)
            * glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z),
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    }
}

#[test]
fn bend_handles_follow_individual_activation_and_reveal_the_limb_without_touching_other_goals() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = frame(rig);
    let proj = projector(Vec3::ZERO);
    let mut editor = Editor::default();
    editor.toggle();
    editor.limb_controls = false;
    editor.visibility.only(visibility::Group::Face);
    let ctx = egui::Context::default();
    let palette = crate::theme::Theme::default().palette();
    let controls = limbs(rig);
    for count in 0..=4 {
        if count > 0 {
            editor.enable(controls[count - 1].end, true, rig, &frame);
        }
        for warmup in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                    ..Default::default()
                },
                |ui| editor.show_poles(ui.ctx(), &palette, &proj, &frame, rig, false),
            );
            // Newly created Areas measure themselves before becoming visible.
            if warmup == 0 {
                output.textures_delta.clear();
                continue;
            }
            for (index, limb) in controls.iter().enumerate() {
                let shown = output
                    .shapes
                    .iter()
                    .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == aurora_i18n::text(limb.pole_label)));
                assert_eq!(shown, index < count, "active limbs: {count}, handle: {}", limb.pole_label);
                if index < count {
                    assert!(editor.visibility.visible(&rig.canonical_names[limb.end]));
                    let anchor = proj
                        .project(frame.model.transform_point3(editor.goals[index].pole.unwrap()))
                        .unwrap()
                        .0;
                    // Areas snap their placement to device pixels.
                    assert!(
                        output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Circle(c)
                        if c.radius == 3.0 && c.center.distance(anchor) < 1.0)),
                        "bend handle must stay at its real target"
                    );
                }
            }
            output.textures_delta.clear();
        }
    }
    let saved = editor.goals.clone();
    for limb in controls {
        editor.enable(limb.end, false, rig, &frame);
    }
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        editor.show_poles(ui.ctx(), &palette, &proj, &frame, rig, false)
    });
    assert!(output.shapes.iter().all(|s| !matches!(&s.shape, egui::Shape::Text(_))));
    assert_eq!(editor.goals.len(), saved.len());
    assert!(editor.request().goals.is_empty());
    output.textures_delta.clear();
}

#[test]
fn joint_centred_pole_rotation_changes_the_bend_without_moving_or_orienting_the_effector() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = frame(rig);
    for limb in limbs(rig) {
        let mut editor = Editor::default();
        editor.toggle();
        editor.enable(limb.end, true, rig, &frame);
        editor.select(limb.end, rig, &frame);
        let root = frame.joints[limb.root].w_axis.truncate();
        editor.position = root + (editor.position - root) * 0.8;
        editor.orient = true;
        editor.control = Control::Pole;
        editor.commit();
        let before = editor.goals[0].clone();
        let middle = frame.joints[limb.middle].w_axis.truncate();
        editor.begin_pole_orbit(&frame, rig);
        let axis = (before.position - root).normalize();
        editor.rotate_pole(Quat::from_axis_angle(axis, 0.3));
        let rotation = Quat::from_axis_angle(axis, 0.6);
        editor.rotate_pole(rotation);
        editor.commit();
        let after = &editor.goals[0];
        assert_eq!(after.position, before.position);
        assert_eq!(after.rotation, before.rotation);
        assert_eq!(after.contact, before.contact);
        assert!(
            after
                .pole
                .unwrap()
                .abs_diff_eq(middle + rotation * (before.pole.unwrap() - middle), 1e-5)
        );
        let rest = ik::LocalPose {
            rotations: rig.local_rot.clone(),
            positions: rig.local_pos.clone(),
        };
        let mut first = rest.clone();
        let mut second = rest;
        assert!(ik::apply_goal(rig, &mut first, Vec3::ZERO, &rig.scale, &before));
        assert!(ik::apply_goal(rig, &mut second, Vec3::ZERO, &rig.scale, after));
        let a = rig.world_with(&first.rotations, Vec3::ZERO, &first.positions, &rig.scale);
        let b = rig.world_with(&second.rotations, Vec3::ZERO, &second.positions, &rig.scale);
        assert!(a[limb.end].w_axis.truncate().distance(before.position) < 0.003);
        assert!(b[limb.end].w_axis.truncate().distance(before.position) < 0.003);
        assert!(a[limb.middle].w_axis.truncate().distance(b[limb.middle].w_axis.truncate()) > 0.005);
    }
}

#[test]
fn default_translation_gizmo_keeps_the_grabbed_axis_when_shift_is_pressed() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = frame(rig);
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(rig.names["mWristLeft"], rig, &frame);
    editor.orient = true;
    editor.commit();
    let before = editor.goals[0].clone();
    let point = frame.joints[before.bone].w_axis.truncate();
    let proj = projector(point);
    let length = Vec3::new(3.0, -4.0, 3.0).length() * (50.0_f32 / 900.0 * 0.8).tan();
    let start = proj.project(point + Vec3::X * length * 0.85).unwrap().0;
    let end = proj.project(point + Vec3::X * (length * 0.85 + 0.12)).unwrap().0;
    let ctx = egui::Context::default();
    let palette = crate::theme::Theme::default().palette();
    for (step, (modifiers, position, press)) in [
        (egui::Modifiers::NONE, start, None),
        (egui::Modifiers::NONE, start, None),
        (egui::Modifiers::NONE, start, Some(true)),
        (egui::Modifiers::NONE, end, None),
        (egui::Modifiers::SHIFT, end, None),
        (egui::Modifiers::SHIFT, end, Some(false)),
        (egui::Modifiers::SHIFT, end, None),
    ]
    .into_iter()
    .enumerate()
    {
        let mut events = vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(position)];
        if let Some(pressed) = press {
            events.push(egui::Event::PointerButton {
                pos: position,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers,
            });
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(step as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                editor.show_gizmo(ui.ctx(), &palette, &proj, &frame, rig);
            },
        );
        output.textures_delta.clear();
    }
    assert!(editor.goals[0].position.distance(before.position) > 0.1);
    assert_eq!(editor.goals[0].rotation, before.rotation);
    assert!(!editor.gizmo_visible(egui::Modifiers::SHIFT));
    assert!(editor.gizmo_visible(egui::Modifiers::NONE));
    assert!(editor.tool == Tool::Move);
}

#[test]
fn ctrl_exposes_rotation_rings_and_keeps_a_grab_after_release() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = frame(rig);
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(rig.names["mWristLeft"], rig, &frame);
    editor.orient = true;
    editor.commit();
    let before = editor.goals[0].clone();
    let point = frame.joints[before.bone].w_axis.truncate();
    let proj = projector(point);
    let radius = Vec3::new(3.0, -4.0, 3.0).length() * (100.0_f32 / 900.0 * 0.8).tan();
    let ring = |t: f32| proj.project(point + Vec3::new(t.cos(), t.sin(), 0.0) * radius).unwrap().0;
    let ctx = egui::Context::default();
    let palette = crate::theme::Theme::default().palette();
    for (step, (modifiers, position, press)) in [
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
                pressed,
                button: egui::PointerButton::Primary,
                modifiers,
            });
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(step as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                editor.show_gizmo(ui.ctx(), &palette, &proj, &frame, rig);
            },
        );
        output.textures_delta.clear();
    }
    assert_eq!(editor.goals[0].position, before.position);
    assert_eq!(editor.goals[0].contact, before.contact);
    let actual = editor.goals[0].rotation.unwrap();
    let expected = Quat::from_rotation_z(0.5) * before.rotation.unwrap();
    assert!(actual.dot(expected).abs() > 0.99999);
    assert!(editor.gizmo_visible(egui::Modifiers::NONE));
    assert!(editor.tool == Tool::Move);
}

#[test]
fn shift_freely_drags_effector_grips_by_the_pointer_delta_without_moving_their_pivots() {
    use crate::demo;
    use aurora_net::NetEvent;
    use std::sync::Arc;

    let lib = Arc::new(AvatarLibrary::load());
    let rig = &lib.rig;
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.0, true),
    });
    world.avatar_poses.insert(
        world.agent_id,
        rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)
            .iter()
            .zip(&rig.default_world_inv)
            .map(|(m, inv)| *m * *inv)
            .collect(),
    );
    let frame = Frame::new(&world).unwrap();
    for limb in limbs(rig) {
        let mut editor = Editor::default();
        editor.toggle();
        editor.library_demo(&world, "");
        assert!(editor.add_profile(false, &world, &frame));
        editor.enable(limb.end, true, rig, &frame);
        editor.orient = true;
        editor.commit();
        let before = editor.goals[0].clone();
        let point = frame.model.transform_point3(before.position);
        let proj = projector(point);
        let anchor = proj.project(point).unwrap().0;
        let grip = anchor;
        let delta = egui::vec2(18.0, -11.0);
        let ctx = egui::Context::default();
        let palette = crate::theme::Theme::default().palette();
        let mut time = 0.0;
        let mut draw = |editor: &mut Editor, position, press| {
            time += 1.0 / 60.0;
            let mut events = vec![
                egui::Event::ModifiersChanged(egui::Modifiers::SHIFT),
                egui::Event::PointerMoved(position),
            ];
            if let Some(pressed) = press {
                events.push(egui::Event::PointerButton {
                    pos: position,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: egui::Modifiers::SHIFT,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| editor.show(ui.ctx(), &palette, &world, &proj),
            );
            output.textures_delta.clear();
            output
        };
        for _ in 0..3 {
            draw(&mut editor, grip, None);
        }
        let output = draw(&mut editor, grip, None);
        assert!(
            output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Circle(c)
            if c.radius == 10.0 && c.center.distance(grip) < 0.1)),
            "{} grip must stay on the joint",
            limb.label
        );
        draw(&mut editor, grip, Some(true));
        draw(&mut editor, grip, Some(false));
        assert_eq!(editor.selected, Some(limb.end));
        assert_eq!(editor.goals[0].position, before.position);
        draw(&mut editor, grip, Some(true));
        draw(&mut editor, grip + delta, None);
        draw(&mut editor, grip + delta, Some(false));
        let after = &editor.goals[0];
        let moved = proj.project(frame.model.transform_point3(after.position)).unwrap().0;
        assert!(moved.distance(anchor + delta) < 0.2, "{} target follows the drag delta", limb.label);
        assert_eq!(after.rotation, before.rotation);
        assert_eq!(after.contact, before.contact);
        assert_eq!(after.pole, before.pole);
    }
}
