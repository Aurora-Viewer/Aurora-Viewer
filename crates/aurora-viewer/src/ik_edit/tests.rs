use super::*;
use crate::scene::avatar::AvatarLibrary;

pub(super) trait HeadlessContext {
    fn run_headless(&self, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput;
}

impl HeadlessContext for egui::Context {
    fn run_headless(&self, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let mut output = self.run_ui(input, draw);
        // These frames inspect geometry and commands without a GPU backend.
        output.textures_delta.clear();
        output
    }
}

#[test]
fn each_limb_enables_and_releases_independently_even_with_symmetry() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = Frame {
        model: Mat4::IDENTITY,
        joints: rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale),
        at: Instant::now(),
    };
    let mut editor = Editor::default();
    editor.toggle();
    editor.symmetry = true;
    for (i, limb) in limbs(rig).into_iter().enumerate() {
        editor.enable(limb.end, true, rig, &frame);
        assert_eq!(editor.request().goals.len(), i + 1);
        assert_eq!(
            editor.goals.iter().find(|g| g.bone == limb.end).unwrap().root,
            ik::extended_root(rig, limb.end).unwrap()
        );
    }
    for (i, limb) in limbs(rig).into_iter().enumerate() {
        editor.enable(limb.end, false, rig, &frame);
        assert_eq!(editor.request().goals.len(), 3 - i);
        assert!(editor.request().released.contains(&limb.end));
        assert!(!editor.enabled(limb.end));
    }
    assert_eq!(editor.goals.len(), 4, "turning off preserves calibration");
    let limb = limbs(rig)[0];
    editor.enable(limb.end, true, rig, &frame);
    assert_eq!(editor.request().goals.len(), 1);
    assert!(!editor.request().released.contains(&limb.end));
}

#[test]
fn multiple_prim_bindings_follow_their_own_targets_without_baking_weighted_offsets() {
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
    let palette = rig
        .world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)
        .into_iter()
        .zip(&rig.default_world_inv)
        .map(|(pose, inv)| pose * *inv)
        .collect();
    world.avatar_poses.insert(world.agent_id, palette);
    let frame = Frame::new(&world).unwrap();
    let mut editor = Editor::default();
    editor.toggle();
    for (name, target, label, weight) in [
        ("mWristLeft", demo::hand_ik::LEFT, "Left", 0.25),
        ("mWristRight", demo::hand_ik::RIGHT, "Right", 0.75),
    ] {
        editor.select(rig.names[name], rig, &frame);
        editor.target = Some(demo::action_id(target));
        editor.target_name = label.into();
        editor.weight = weight;
        editor.orient = false;
        editor.pole = Some(editor.position + Vec3::new(-0.1, 0.2, -0.1));
        editor.attach(&world, &frame);
    }
    assert_eq!(editor.bindings.len(), 2);
    assert!(
        editor.bindings.iter().all(|b| b.definition.rotation.is_some()),
        "new bindings follow orientation by default"
    );
    let definitions: Vec<_> = editor
        .bindings
        .iter()
        .map(|b| serde_json::to_string(&b.definition).unwrap())
        .collect();
    // Move and tilt the whole motorcycle; each binding uses its own child prim.
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.9, true),
    });
    let moved = Frame::new(&world).unwrap();
    for _ in 0..20 {
        editor.update_bindings(&world, &moved);
    }
    for (binding, saved) in editor.bindings.iter().zip(&definitions) {
        assert_eq!(&serde_json::to_string(&binding.definition).unwrap(), saved);
        let goal = editor.request().goals.into_iter().find(|g| g.bone == binding.bone).unwrap();
        let index = world.objects.index_of_uuid(&binding.definition.target).unwrap();
        let (p, q, _) = Scene::object_transform(&world, index, moved.at, 0).unwrap();
        let error = moved
            .model
            .transform_point3(goal.position)
            .distance(p + q * Vec3::from_array(binding.definition.position));
        assert!(error < 3e-5, "{}: target error {error} m", binding.definition.bone);
        assert_eq!(goal.weight, binding.definition.weight);
        let world_rotation = moved.model.to_scale_rotation_translation().1 * goal.rotation.unwrap();
        assert!(world_rotation.dot(q * Quat::from_array(binding.definition.rotation.unwrap())).abs() > 0.99999);
        let world_pole = moved.model.transform_point3(goal.pole.unwrap());
        assert!(world_pole.distance(p + q * Vec3::from_array(binding.definition.pole.unwrap())) < 3e-5);
    }
    // Editing a bound bone rewrites that prim's local offset, never the prim.
    let bone = rig.names["mWristLeft"];
    editor.select(bone, rig, &moved);
    editor.position += Vec3::new(0.03, -0.02, 0.01);
    let desired = editor.position;
    editor.set_rotation(Quat::from_rotation_z(0.7));
    editor.commit();
    editor.update_bindings(&world, &moved);
    let edited = editor.goals.iter().find(|g| g.bone == bone).unwrap();
    assert!(edited.position.distance(desired) < 1e-5);
    assert!(edited.rotation.unwrap().dot(Quat::from_rotation_z(0.7)).abs() > 0.99999);
    editor.enable(bone, false, rig, &moved);
    editor.bindings[0].name = "Left \"grip\" \\ 背景 🏍".into();
    editor.bindings[0].definition.activation = crate::world::rig_graph::Activation {
        distance: 0.4,
        ease_in: 0.6,
        ease_out: 0.8,
    };
    let script = editor.script().unwrap();
    assert!(script.contains("auroraIKObserve"), "simple contacts also export observer support");
    let exported = editor.graph_definition();
    assert!(!exported.bindings[0].enabled);
    assert_eq!(exported.bindings[0].weight, 0.25);
    assert_eq!(exported.bindings[1].weight, 0.75);
    assert_eq!(exported.bindings[0].activation, editor.bindings[0].definition.activation);
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let declaration = editor.graph_rig_declaration().unwrap();
    assert_eq!(
        script.lines().find(|line| line.starts_with("string RIG = ")),
        Some(declaration.as_str()),
        "both clipboard exports use the same escaped LSL declaration"
    );
    let payload: String = serde_json::from_str(declaration.strip_prefix("string RIG = ").unwrap().strip_suffix(';').unwrap()).unwrap();
    assert_eq!(Some(payload.clone()), editor.graph_payload());
    assert!(payload.is_ascii());
    let exported: crate::world::rig_graph::Definition = serde_json::from_str(&payload).unwrap();
    assert_eq!(exported, editor.graph_definition());
    assert!(exported.nodes.is_empty());
    assert_eq!(exported.bindings.len(), 2);
    assert!(!exported.bindings[0].enabled);
    assert_eq!(exported.bindings[0].weight, 0.25);
    for (key, expected) in [("ik-edit-copy-config", script), ("ik-edit-copy-data", declaration)] {
        let mut pointer = Pos2::ZERO;
        let mut copied = None;
        for step in 0..6 {
            let mut events = vec![egui::Event::PointerMoved(pointer)];
            if step == 3 || step == 4 {
                events.push(egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed: step == 3,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.bindings_ui(ui, &palette, &world, &moved);
                    editor.export_ui(ui, &palette);
                },
            );
            if step < 2 {
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == aurora_i18n::text(key)
                    {
                        pointer = text.pos + text.galley.size() * 0.5;
                    }
                }
            }
            for command in &output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    copied = Some(text.clone());
                }
            }
            output.textures_delta.clear();
        }
        assert_eq!(
            copied.as_deref(),
            Some(expected.as_str()),
            "one click copies the requested export: {key}"
        );
    }
    let lost = editor.bindings[0].definition.target;
    let bone = editor.bindings[0].bone;
    world.objects.remove_idx(world.objects.index_of_uuid(&lost).unwrap());
    editor.update_bindings(&world, &moved);
    assert_eq!(editor.request().goals.len(), 1);
    assert!(editor.request().released.contains(&bone));
}

#[test]
fn opening_is_independent_and_script_contacts_are_imported_only_on_request() {
    use crate::demo;
    use aurora_net::NetEvent;
    use std::sync::Arc;
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.0, true),
    });
    let probe = world.update_hand_contacts(Instant::now()).unwrap();
    for event in demo::hand_ik::reply(&probe) {
        world.apply(event);
    }
    let rig = &lib.rig;
    world.avatar_poses.insert(
        world.agent_id,
        rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)
            .into_iter()
            .zip(&rig.default_world_inv)
            .map(|(p, inv)| p * *inv)
            .collect(),
    );
    let frame = Frame::new(&world).unwrap();
    let mut editor = Editor::default();
    editor.toggle();
    editor.update_bindings(&world, &frame);
    assert!(editor.bindings.is_empty());
    assert!(editor.request().goals.is_empty());
    assert!(editor.request().suppress_script_ik);
    editor.ensure_profiles(&world, &frame);
    editor.import_script_bindings(&world, &frame);
    assert_eq!(editor.graph.definition.bindings.len(), 2);
    assert_eq!(editor.graph.definition.profiles[0].bindings, Some(vec![0, 1]));
    assert_eq!(editor.request().goals.len(), 2);
    editor.enable(rig.names["mWristLeft"], false, rig, &frame);
    assert_eq!(editor.request().goals.len(), 1);
    assert_eq!(editor.request().goals[0].bone, rig.names["mWristRight"]);
    editor.update_bindings(&world, &frame);
    assert_eq!(
        editor.request().goals.len(),
        1,
        "refresh never re-enables a disabled imported binding"
    );
}

#[test]
fn symmetry_commit_updates_only_the_counterpart_and_ctrl_temporarily_rotates_gizmos() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = Frame {
        model: Mat4::IDENTITY,
        joints: rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale),
        at: Instant::now(),
    };
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(rig.names["mWristLeft"], rig, &frame);
    editor.symmetry = true;
    editor.position = Vec3::new(0.1, 0.3, 1.2);
    editor.pole = Some(Vec3::new(-0.1, 0.6, 1.1));
    editor.commit();
    assert_eq!(editor.goals.len(), 2);
    let right = editor.goals.iter().find(|g| g.bone == rig.names["mWristRight"]).unwrap();
    assert_eq!(right.position, Vec3::new(0.1, -0.3, 1.2));
    assert_eq!(right.pole, Some(Vec3::new(-0.1, -0.6, 1.1)));
    assert_eq!(right.root, rig.names["mShoulderRight"]);
    editor.tool = Tool::Move;
    assert!(editor.active_tool(egui::Modifiers::NONE) == Tool::Move);
    assert!(editor.gizmo_visible(egui::Modifiers::NONE));
    assert!(!editor.gizmo_visible(egui::Modifiers::SHIFT));
    assert!(editor.gizmo_visible(egui::Modifiers::CTRL | egui::Modifiers::SHIFT));
    assert!(editor.active_tool(egui::Modifiers::CTRL) == Tool::Rotate);
    assert!(editor.active_tool(egui::Modifiers::NONE) == Tool::Move);
    editor.tool = Tool::Rotate;
    assert!(editor.active_tool(egui::Modifiers::SHIFT) == Tool::Move);
    assert!(editor.active_tool(egui::Modifiers::NONE) == Tool::Rotate);
    let hand = editor.position;
    editor.control = Control::Pole;
    editor.move_control(Vec3::new(0.2, 0.6, 1.2));
    editor.commit();
    assert_eq!(editor.position, hand, "moving the pole never moves the hand target");
    assert_eq!(editor.goals[0].position, hand);
    assert!(
        editor.active_tool(egui::Modifiers::CTRL) == Tool::Rotate,
        "rotation steers the pole around its joint without orienting the end effector"
    );
    editor.symmetry = false;
    let right = editor.goals[1].pole;
    editor.move_control(Vec3::new(0.3, 0.5, 1.0));
    editor.commit();
    assert_eq!(editor.goals[1].pole, right, "symmetry off leaves the other side alone");
}

#[test]
fn ik_edit_pointer_ray_roundtrips_the_projected_contact_at_scaled_ui_resolution() {
    let eye = Vec3::new(4.0, -3.0, 2.0);
    let point = Vec3::new(0.2, 0.1, 0.8);
    let view = glam::camera::rh::view::look_to_mat4(eye, (Vec3::Z - eye).normalize(), Vec3::Z);
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05);
    let projector = Projector {
        view_proj: projection * view,
        width: 1600.0,
        height: 900.0,
        ppp: 1.5,
    };
    let (screen, _) = projector.project(point).expect("projected point");
    let (origin, direction) = ray(&projector, screen).expect("pointer ray");
    let normal = (Vec3::Z - eye).normalize();
    let hit = crate::build::geom::ray_plane(origin, direction, point, normal).expect("plane intersection");
    assert!(hit.distance(point) < 1e-4, "{}", hit.distance(point));
}

#[test]
fn ik_edit_contact_grip_gestures_preserve_selection_and_stay_behind_windows() {
    use crate::{demo, theme::Theme};
    use aurora_net::NetEvent;
    use std::sync::Arc;
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::hand_ik::objects(0.0, true),
    });
    let palette = lib
        .rig
        .world_with(&lib.rig.local_rot, Vec3::ZERO, &lib.rig.local_pos, &lib.rig.scale)
        .into_iter()
        .zip(&lib.rig.default_world_inv)
        .map(|(posed, inverse)| posed * *inverse)
        .collect();
    world.avatar_poses.insert(world.agent_id, palette);
    let frame = Frame::new(&world).expect("rendered skeleton");
    let bone = lib.rig.names["mWristLeft"];
    assert_eq!(lib.rig.canonical_names[bone], "mWristLeft");
    assert_eq!(
        lib.rig.names["avatar_mWristLeft"], bone,
        "alias remains valid for animation binding"
    );
    let contact = Vec3::new(0.0, 0.055, -0.015);
    let point = frame.joints[bone].transform_point3(contact);
    let eye = point + Vec3::new(3.0, -3.0, 2.0);
    let view = glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z);
    let proj = Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05) * view,
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    };
    let (screen, _) = proj.project(point).expect("contact");
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(bone, &lib.rig, &frame);
    editor.library_demo(&world, "");
    assert!(editor.add_profile(false, &world, &frame));
    editor.contact = contact;
    editor.position = frame.model.inverse().transform_point3(point);
    editor.commit();
    let before = editor.position;
    let ctx = egui::Context::default();
    let theme = Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let moved = screen + egui::vec2(30.0, 10.0);
    let mut observed_handle = None;
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::SHIFT,
    };
    for (step, mut events) in [
        vec![egui::Event::PointerMoved(screen)],
        vec![],
        vec![button(screen, true)],
        vec![egui::Event::PointerMoved(moved)],
        vec![button(moved, false)],
    ]
    .into_iter()
    .enumerate()
    {
        events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
            time: Some(step as f64 / 60.0),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| editor.show(ui.ctx(), &palette, &world, &proj));
        observed_handle = output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Circle(c) if (c.radius - 4.5).abs() < 0.01 && c.fill == palette.teal => Some(c.center),
            _ => None,
        });
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text().contains("avatar_mWristLeft"))),
            "authoring uses the canonical bone name"
        );
        output.textures_delta.clear();
    }
    assert_eq!(editor.selected, Some(bone));
    assert_eq!(editor.contact, contact);
    assert!(
        editor.position.distance(before) > 0.005,
        "selected contact must move; pole control={}, pole={:?}, handle={observed_handle:?}, pointer={screen:?}",
        editor.control == Control::Pole,
        editor.pole
    );
    assert_eq!(editor.goals.len(), 1);

    let before = editor.position;
    let rotation = editor.rotation();
    let modifiers = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    let end = screen + egui::vec2(24.0, 17.0);
    for (step, mut events) in [
        vec![egui::Event::PointerMoved(screen)],
        vec![egui::Event::PointerButton {
            pos: screen,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        }],
        vec![egui::Event::PointerMoved(end)],
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers,
        }],
    ]
    .into_iter()
    .enumerate()
    {
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
            time: Some((step + 5) as f64 / 60.0),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| editor.show(ui.ctx(), &palette, &world, &proj));
        output.textures_delta.clear();
    }
    assert_eq!(editor.selected, Some(bone));
    assert_eq!(editor.position, before, "Ctrl rotates without moving the contact target");
    assert!(editor.orient);
    let turned = editor.goals[0].rotation.expect("Ctrl enables authored orientation");
    assert!(turned.is_finite() && (turned.length_squared() - 1.0).abs() < 1e-5);
    assert!(turned.dot(rotation).abs() < 0.999, "Ctrl must rotate the selected bone");
    assert!(
        turned.dot(editor.rotation()).abs() > 0.99999,
        "numeric angles follow the mouse rotation"
    );

    // A window opened over a previously dragged handle owns both the paint
    // order and the next click, regardless of which UI is submitted first.
    let before = editor.position;
    let mut clicked = false;
    let mut button_rect = egui::Rect::NOTHING;
    for (step, events) in [
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![button(screen, true)],
        vec![button(screen, false)],
    ]
    .into_iter()
    .enumerate()
    {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
            time: Some((step + 9) as f64 / 60.0),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let mut cover = |ctx: &egui::Context| {
                Floater::new(
                    "ik-cover-test",
                    "Front window",
                    screen - egui::vec2(110.0, 75.0),
                    egui::vec2(240.0, 180.0),
                )
                .show(ctx, &palette, &mut true, |ui| {
                    let response = ui.add_sized(egui::vec2(180.0, 110.0), egui::Button::new("front action"));
                    button_rect = response.rect;
                    clicked |= response.clicked();
                });
            };
            if step % 2 == 0 {
                cover(ui.ctx());
            }
            editor.show(ui.ctx(), &palette, &world, &proj);
            if step % 2 != 0 {
                cover(ui.ctx());
            }
        });
        if step < 4 {
            output.textures_delta.clear();
            continue;
        }
        assert!(
            button_rect.contains(screen),
            "the window covers the handle: {button_rect:?}, pointer {screen:?}"
        );
        let text = output
            .shapes
            .iter()
            .position(|shape| {
                matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.text() == "front action")
            })
            .expect("front button text");
        let line = output
            .shapes
            .iter()
            .rposition(|shape| {
                matches!(&shape.shape,
            egui::epaint::Shape::LineSegment { stroke, .. } if stroke.color == palette.indigo_light.gamma_multiply(0.8))
            })
            .expect("skeleton lines");
        let point = output
            .shapes
            .iter()
            .rposition(|shape| {
                matches!(&shape.shape,
            egui::epaint::Shape::Circle(circle) if circle.center.distance(screen) < 1.0 && (circle.radius - 4.5).abs() < 0.01)
            })
            .expect("palm handle");
        assert!(
            line < text && point < text,
            "both skeleton lines and handles are painted behind the window"
        );
        output.textures_delta.clear();
    }
    assert!(clicked, "the window button takes the click");
    assert_eq!(editor.position, before, "a covered handle cannot edit the pose");
}

#[test]
fn ik_edit_export_roundtrips_a_scaled_bone_point_on_a_rotated_prim() {
    let q = Quat::from_euler(EulerRot::ZYX, -0.5, 0.3, 1.2);
    let joint = Mat4::from_scale_rotation_translation(Vec3::new(1.2, 0.8, 1.1), q, Vec3::new(124.2, 126.3, 22.5));
    let contact = Vec3::new(0.0, 0.055, -0.015);
    let target_q = Quat::from_euler(EulerRot::ZYX, 0.7, -0.4, 0.1);
    let target_p = Vec3::new(124.0, 126.1, 22.4);
    let (p, r) = offsets(joint, contact, target_p, target_q).expect("offsets");
    assert!((target_p + target_q * p).distance(joint.transform_point3(contact)) < 2e-5);
    assert!((target_q * r).dot(q).abs() > 0.99999);
    // Different prim sizes must have exactly the same metric offsets.
    assert!(p.length() < 1.0);
    assert!(offsets(Mat4::from_translation(Vec3::NAN), contact, target_p, target_q).is_none());
}

#[test]
fn ik_edit_constraint_json_keeps_orientation_optional() {
    let definition = crate::world::hand_ik::Constraint {
        activation: Default::default(),
        weight: 1.0,
        bone: "mHead".into(),
        target: Uuid::from_u128(9),
        position: [0.1, 0.2, 0.3],
        rotation: None,
        contact: [0.0; 3],
        root: Some("mNeck".into()),
        pole: None,
        mode: Mode::Direct,
    };
    let json = serde_json::to_string(&definition).expect("JSON");
    let decoded: crate::world::hand_ik::Constraint = serde_json::from_str(&json).expect("constraint");
    assert_eq!(decoded.bone, "mHead");
    assert_eq!(decoded.rotation, None);
}

#[test]
fn ik_edit_commit_is_reversible_and_updates_one_goal_per_selected_bone() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = Frame {
        model: Mat4::IDENTITY,
        joints: rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale),
        at: Instant::now(),
    };
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(rig.names["mWristLeft"], rig, &frame);
    editor.position += Vec3::X * 0.1;
    editor.commit();
    editor.position += Vec3::Z * 0.1;
    editor.commit();
    assert_eq!(editor.request().goals.len(), 1);
    editor.toggle();
    assert!(!editor.request().active);
    editor.toggle();
    assert!(editor.request().goals.is_empty());
    editor.select(rig.pelvis, rig, &frame);
    assert_eq!(editor.mode, Mode::Direct);
}
