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
fn manipulator_pointer_ownership_includes_a_hovered_world_gizmo() {
    let mut editor = Editor::default();
    assert!(!editor.manipulator_owns_pointer());

    editor.manipulator_hovered = true;
    assert!(editor.manipulator_owns_pointer(), "a hovered world gizmo blocks camera clicks");
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
        assert!(!editor.enabled(limb.end));
    }
    assert_eq!(editor.goals.len(), 4, "turning off preserves calibration");
    let limb = limbs(rig)[0];
    editor.enable(limb.end, true, rig, &frame);
    assert_eq!(editor.request().goals.len(), 1);
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
        objects: vec![demo::action_avatar(false)],
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
