use super::*;

fn geometry(tool: Tool, basis: Quat, ppp: f32, edge_on: bool) -> Geometry {
    let pivot = Vec3::new(128.0, 132.0, 20.0);
    let eye = pivot
        + if edge_on {
            Vec3::new(0.0, -5.0, 2.0)
        } else {
            Vec3::new(3.0, -4.0, 3.0)
        };
    let view = glam::camera::rh::view::look_to_mat4(eye, (pivot - eye).normalize(), Vec3::Z);
    let proj = Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05) * view,
        width: 1600.0,
        height: 900.0,
        ppp,
    };
    let g = Geometry::new(&proj, pivot, basis, tool).expect("perspective geometry");
    assert!(g.cam.eye.distance(eye) < 0.001);
    assert!((g.cam.fov_y - 0.8).abs() < 0.001);
    g
}

/// A camera rolled by `roll` about its view line, looking `aside` metres to
/// the side of the joint, which then sits away from the centre of the screen.
fn rolled_geometry(basis: Quat, roll: f32, aside: Vec3) -> Geometry {
    let pivot = Vec3::new(128.0, 132.0, 20.0);
    let eye = pivot + Vec3::new(3.0, -4.0, 3.0);
    let direction = (pivot + aside - eye).normalize();
    let up = Quat::from_axis_angle(direction, roll) * Vec3::Z;
    let view = glam::camera::rh::view::look_to_mat4(eye, direction, up);
    let proj = Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05) * view,
        width: 1600.0,
        height: 900.0,
        ppp: 1.5,
    };
    Geometry::new(&proj, pivot, basis, Tool::Rotate).expect("rolled perspective geometry")
}

#[test]
fn bone_gizmo_axes_and_planes_follow_world_or_rotated_bone_axes_at_every_ui_scale() {
    let local = Quat::from_rotation_z(0.65) * Quat::from_rotation_x(0.35);
    for ppp in [1.0, 1.5, 2.0] {
        for basis in [Quat::IDENTITY, local] {
            let g = geometry(Tool::Move, basis, ppp, false);
            let pose = Pose {
                position: g.pivot + Vec3::splat(0.04),
                rotation: local,
            };
            let start = g.pivot + g.axis(0) * g.length * 0.85;
            let end = start + g.axis(0) * 0.12;
            let pointer = g.cam.project(start).unwrap();
            assert!(g.pick(pointer) == Some(Part::Axis(0)));
            let mut drag = Drag::new(g, Part::Axis(0), pointer, pose, 0.5).unwrap();
            let moved = drag.update(g.cam.project(end).unwrap()).unwrap();
            assert!((moved.position - pose.position).distance(basis * Vec3::X * 0.12) < 0.001);
            assert!(moved.rotation.dot(pose.rotation).abs() > 0.99999);

            let quad = g.plane(2);
            let start = quad.iter().sum::<Vec3>() * 0.25;
            let delta = (g.axis(0) * 0.07 + g.axis(1) * 0.05) * g.length;
            let mut drag = Drag::new(g, Part::Plane(2), g.cam.project(start).unwrap(), pose, 0.5).unwrap();
            let moved = drag.update(g.cam.project(start + delta).unwrap()).unwrap();
            assert!((moved.position - pose.position).distance(delta) < 0.001);
            assert!((moved.position - pose.position).dot(g.axis(2)).abs() < 0.001);
        }
    }
}

#[test]
fn bone_gizmo_sensitivity_scales_move_and_rotation_from_the_existing_default() {
    let g = geometry(Tool::Move, Quat::IDENTITY, 1.0, false);
    let pose = Pose {
        position: g.pivot,
        rotation: Quat::IDENTITY,
    };
    let start_world = g.pivot + g.axis(0) * g.length * 0.85;
    let end_world = start_world + g.axis(0) * 0.12;
    let mut drag = Drag::new(g, Part::Axis(0), g.cam.project(start_world).unwrap(), pose, 0.25).unwrap();
    let moved = drag.update(g.cam.project(end_world).unwrap()).unwrap();
    assert!((moved.position - pose.position).distance(g.axis(0) * 0.06) < 0.001);

    let g = geometry(Tool::Rotate, Quat::IDENTITY, 1.0, false);
    let pose = Pose {
        position: g.pivot,
        rotation: Quat::IDENTITY,
    };
    let start = 0.3;
    let drag_start = g.cam.project(g.ring(2, start)).unwrap();
    let drag_end = g.cam.project(g.ring(2, start + 0.4)).unwrap();
    let mut drag = Drag::new(g, Part::Ring(2), drag_start, pose, 0.25).unwrap();
    let rotated = drag.update(drag_end).unwrap();
    assert!(rotated.rotation.dot(Quat::from_rotation_z(0.2)).abs() > 0.99999);

    let mut default_drag = Drag::new(g, Part::Ring(2), drag_start, pose, DEFAULT_SENSITIVITY).unwrap();
    let default_rotation = default_drag.update(drag_end).unwrap();
    assert!(default_rotation.rotation.dot(Quat::from_rotation_z(0.4)).abs() > 0.99999);
}

#[test]
fn poser_sensitivity_mapping_preserves_default_and_bounds_invalid_values() {
    assert_eq!(sensitivity_scale(DEFAULT_SENSITIVITY), 1.0);
    assert_eq!(sensitivity_scale(0.0), 0.1);
    assert_eq!(sensitivity_scale(3.0), 4.0);
    assert_eq!(sensitivity_scale(f32::NAN), 1.0);
    assert_eq!(sensitivity_scale(f32::INFINITY), 1.0);
}

#[test]
fn world_gizmo_drag_captures_the_pointer_without_a_press_jump() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    let geometry = geometry(Tool::Move, Quat::IDENTITY, 1.0, false);
    let pose = Pose {
        position: geometry.pivot,
        rotation: Quat::from_rotation_y(0.2),
    };
    let start_world = geometry.pivot + geometry.axis(0) * geometry.length * 0.85;
    let end_world = start_world + geometry.axis(0) * 0.12;
    let start = geometry.cam.project(start_world).unwrap();
    let end = geometry.cam.project(end_world).unwrap();
    let mut gizmo = Gizmo::default();
    let show_frame = |gizmo: &mut Gizmo, time: f64, events: Vec<egui::Event>| {
        let mut result = (false, None);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| result = gizmo.show(ui.ctx(), &palette, geometry, pose, true, 0.5),
        );
        output.textures_delta.clear();
        result
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };

    let (owns, change) = show_frame(&mut gizmo, 0.0, vec![egui::Event::PointerMoved(start), button(start, true)]);
    assert!(owns && gizmo.dragging(), "the gizmo captures the press");
    let captured = change.expect("the initial drag sample").0;
    assert!(
        captured.position.abs_diff_eq(pose.position, 1e-4),
        "pressing the handle does not move it"
    );
    assert!(captured.rotation.dot(pose.rotation).abs() > 0.99999);

    let (owns, change) = show_frame(&mut gizmo, 1.0 / 60.0, vec![egui::Event::PointerMoved(end)]);
    assert!(owns && gizmo.dragging(), "capture continues while the pointer moves");
    let moved = change.expect("the drag update").0;
    assert!((moved.position - pose.position).distance(geometry.axis(0) * 0.12) < 0.001);
    assert!(moved.rotation.dot(pose.rotation).abs() > 0.99999);

    let (owns, _) = show_frame(&mut gizmo, 2.0 / 60.0, vec![button(end, false)]);
    assert!(owns, "release remains owned by the active gizmo drag");
    assert!(!gizmo.dragging(), "release drops pointer capture");
}

#[test]
fn bone_gizmo_rings_preserve_the_contact_and_continue_across_complete_turns() {
    for basis in [Quat::IDENTITY, Quat::from_rotation_y(0.4) * Quat::from_rotation_z(0.25)] {
        let g = geometry(Tool::Rotate, basis, 1.5, false);
        let pose = Pose {
            position: g.pivot,
            rotation: basis * Quat::from_rotation_x(0.35),
        };
        let start = 0.3;
        let mut drag = Drag::new(g, Part::Ring(2), g.cam.project(g.ring(2, start)).unwrap(), pose, 0.5).unwrap();
        assert!(matches!(drag.motion, Motion::Ring { .. }));
        for step in 1..50 {
            let angle = step as f32 * 0.2;
            let moved = drag.update(g.cam.project(g.ring(2, start + angle)).unwrap()).unwrap();
            let expected = Quat::from_axis_angle(basis * Vec3::Z, angle) * pose.rotation;
            assert_eq!(moved.position, pose.position);
            assert!(moved.rotation.is_finite());
            assert!(moved.rotation.dot(expected).abs() > 0.99999);
        }
    }
}

#[test]
fn bone_gizmo_edge_on_ring_uses_a_visible_tangent_without_moving_the_contact() {
    let g = geometry(Tool::Rotate, Quat::IDENTITY, 1.0, true);
    let pose = Pose {
        position: g.pivot,
        rotation: Quat::from_rotation_z(0.5),
    };
    let start = g.cam.project(g.ring(0, 0.6)).unwrap();
    let mut drag = Drag::new(g, Part::Ring(0), start, pose, 0.5).unwrap();
    let Motion::Edge { tangent, radius, .. } = drag.motion else {
        panic!("edge-on ring fallback");
    };
    let moved = drag.update(start + tangent * radius * 0.4).unwrap();
    assert_eq!(moved.position, pose.position);
    assert!(moved.rotation.dot(Quat::from_rotation_x(0.4) * pose.rotation).abs() > 0.99999);
}

#[test]
fn bone_gizmo_camera_roll_ring_picks_after_axis_rings_and_turns_in_its_drawn_plane() {
    let basis = Quat::from_rotation_y(0.4) * Quat::from_rotation_z(0.25);
    let off_centre = rolled_geometry(basis, 0.0, Vec3::new(1.2, 0.4, -0.9));
    let screen = egui::vec2(off_centre.cam.width, off_centre.cam.height) / off_centre.cam.ppp;
    let centre = off_centre.cam.project(off_centre.pivot).unwrap() - (screen * 0.5).to_pos2();
    assert!(
        centre.length() > screen.y * 0.25,
        "the joint is well away from the centre: {centre:?}"
    );
    for g in [
        geometry(Tool::Rotate, basis, 1.5, false),
        rolled_geometry(basis, 0.65, Vec3::ZERO),
        off_centre,
        rolled_geometry(basis, 0.65, Vec3::new(-1.0, -0.3, 1.1)),
    ] {
        let pose = Pose {
            position: g.pivot,
            rotation: basis * Quat::from_rotation_x(0.35),
        };
        let start = g.cam.project(g.view_ring(0.0)).unwrap();
        let sweep = g.cam.project(g.view_ring(std::f32::consts::FRAC_PI_2)).unwrap();
        assert_eq!(g.pick(start), Some(Part::Roll));
        let inside = g.cam.project(g.pivot + g.cam.left * g.length * 0.8).unwrap();
        assert_ne!(g.pick(inside), Some(Part::Roll));
        for axis in 0..3 {
            let on_ring = g.cam.project(g.ring(axis, 0.3)).unwrap();
            assert!(matches!(g.pick(on_ring), Some(Part::Ring(_))));
        }
        let mut moving = g;
        moving.tool = Tool::Move;
        assert_ne!(moving.pick(start), Some(Part::Roll));

        // The axis is the camera-to-joint line, in world space, and the ring
        // is drawn in the plane normal to it, like renderCenterCircle.
        let axis = g.part_axis(Part::Roll);
        assert!(axis.cross(g.pivot - g.cam.eye).length() < 1e-4);
        for k in 0..8 {
            let t = k as f32 * std::f32::consts::FRAC_PI_4;
            assert!((g.view_ring(t) - g.pivot).dot(axis).abs() < g.length * 1e-4);
            assert!(((g.view_ring(t) - g.pivot).length() - g.length * VIEW_RING).abs() < g.length * 1e-4);
        }
        let mut drag = Drag::new(g, Part::Roll, start, pose, DEFAULT_SENSITIVITY).expect("view-facing roll drag");
        let moved = drag.update(sweep).expect("screen-plane rotation");
        assert_eq!(moved.position, pose.position);
        // A quarter turn about that axis, in the pointer's direction: the
        // joint's point under the start of the drag ends under the pointer.
        let turn = moved.rotation * pose.rotation.inverse();
        assert!((turn.to_axis_angle().1.abs() - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
        let carried = turn * (g.view_ring(0.0) - g.pivot);
        assert!(carried.distance(g.view_ring(std::f32::consts::FRAC_PI_2) - g.pivot) < g.length * 1e-3);
        assert!(g.cam.project(g.pivot + carried).unwrap().distance(sweep) < 0.5);
        let back = drag.update(start).expect("back to the start");
        assert!(back.rotation.dot(pose.rotation).abs() > 0.99999);
    }
}

#[test]
fn bone_gizmo_roll_ring_shows_only_while_hovered_or_dragged() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let p = theme.palette();
    let g = geometry(Tool::Rotate, Quat::IDENTITY, 1.0, false);
    let pose = Pose {
        position: g.pivot,
        rotation: Quat::IDENTITY,
    };
    let on_ring = g.cam.project(g.view_ring(0.0)).unwrap();
    let away = g.cam.project(g.view_ring(0.0)).unwrap() + egui::vec2(0.0, -200.0);
    assert_eq!(g.pick(away), None);
    let mut gizmo = Gizmo::default();
    // Away, on the ring, then pressed on it and dragged a little.
    let steps = [
        (away, None),
        (on_ring, None),
        (on_ring, Some(true)),
        (on_ring + egui::vec2(0.0, 20.0), None),
    ];
    let mut shown = Vec::new();
    for (step, (pointer, press)) in steps.into_iter().enumerate() {
        let mut events = vec![egui::Event::PointerMoved(pointer)];
        if let Some(pressed) = press {
            events.push(egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(step as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                gizmo.show(ui.ctx(), &p, g, pose, true, DEFAULT_SENSITIVITY);
            },
        );
        shown.push(
            out.shapes
                .iter()
                .filter(|s| matches!(&s.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == p.amber))
                .count(),
        );
        out.textures_delta.clear();
    }
    assert_eq!(shown[0], 0, "hidden while the pointer is away");
    assert!(shown[1..].iter().all(|&n| n > 48), "shown while hovered and dragged: {shown:?}");
}

#[test]
fn a_window_over_the_bone_gizmo_owns_the_click_and_the_paint_order() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let p = theme.palette();
    let g = geometry(Tool::Move, Quat::IDENTITY, 1.0, false);
    let pose = Pose {
        position: g.pivot,
        rotation: Quat::IDENTITY,
    };
    let pointer = g.cam.project(g.pivot + g.axis(0) * g.length * 0.85).unwrap();
    let mut gizmo = Gizmo::default();
    let mut clicked = false;
    for step in 0..8 {
        let mut events = vec![egui::Event::PointerMoved(pointer)];
        if step >= 6 {
            events.push(egui::Event::PointerButton {
                pos: pointer,
                button: egui::PointerButton::Primary,
                pressed: step == 6,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                time: Some(step as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                let (owns, edit) = gizmo.show(ui.ctx(), &p, g, pose, true, 0.5);
                if step >= 4 {
                    assert!(!owns && edit.is_none());
                }
                crate::ui::widgets::Floater::new("gizmo-cover", "Window", pointer - egui::vec2(100.0, 75.0), egui::vec2(230.0, 180.0))
                    .show(ui.ctx(), &p, &mut true, |ui| {
                        clicked |= ui.add_sized(egui::vec2(170.0, 110.0), egui::Button::new("cover action")).clicked();
                    });
            },
        );
        if step >= 4 {
            let button = out
                .shapes
                .iter()
                .position(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "cover action"))
                .unwrap();
            let axis = out
                .shapes
                .iter()
                .rposition(|s| matches!(&s.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == p.danger))
                .unwrap();
            assert!(axis < button);
        }
        out.textures_delta.clear();
    }
    assert!(clicked);
}

#[test]
fn shift_during_a_handle_drag_moves_and_turns_ten_times_finer_without_a_jump() {
    let g = geometry(Tool::Move, Quat::IDENTITY, 1.0, false);
    let pose = Pose {
        position: g.pivot,
        rotation: Quat::IDENTITY,
    };
    let on_axis = |d: f32| g.cam.project(g.pivot + g.axis(0) * (g.length * 0.85 + d)).unwrap();
    let mut drag = Drag::new(g, Part::Axis(0), on_axis(0.0), pose, DEFAULT_SENSITIVITY).unwrap();
    let coarse = drag.update_fine(on_axis(0.1), false).unwrap();
    assert!((coarse.position - pose.position).distance(g.axis(0) * 0.1) < 0.001);
    // Shift pressed: the handle does not jump, then follows a tenth as far
    let held = drag.update_fine(on_axis(0.1), true).unwrap();
    assert!(held.position.distance(coarse.position) < 0.0005);
    let fine = drag.update_fine(on_axis(0.3), true).unwrap();
    assert!(
        (fine.position - coarse.position).distance(g.axis(0) * 0.02) < 0.001,
        "{}",
        fine.position
    );
    // Shift released: back to one to one from there
    let released = drag.update_fine(on_axis(0.3), false).unwrap();
    assert!(released.position.distance(fine.position) < 0.0005);

    let g = geometry(Tool::Rotate, Quat::IDENTITY, 1.0, false);
    let pose = Pose {
        position: g.pivot,
        rotation: Quat::IDENTITY,
    };
    let ring = |a: f32| g.cam.project(g.ring(2, a)).unwrap();
    let mut drag = Drag::new(g, Part::Ring(2), ring(0.3), pose, DEFAULT_SENSITIVITY).unwrap();
    drag.update_fine(ring(0.3), true).unwrap();
    let turned = drag.update_fine(ring(0.7), true).unwrap();
    assert!(turned.rotation.dot(Quat::from_rotation_z(0.04)).abs() > 0.99999);
}
