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
            let mut drag = Drag::new(g, Part::Axis(0), pointer, pose).unwrap();
            let moved = drag.update(g.cam.project(end).unwrap()).unwrap();
            assert!((moved.position - pose.position).distance(basis * Vec3::X * 0.12) < 0.001);
            assert!(moved.rotation.dot(pose.rotation).abs() > 0.99999);

            let quad = g.plane(2);
            let start = quad.iter().sum::<Vec3>() * 0.25;
            let delta = (g.axis(0) * 0.07 + g.axis(1) * 0.05) * g.length;
            let mut drag = Drag::new(g, Part::Plane(2), g.cam.project(start).unwrap(), pose).unwrap();
            let moved = drag.update(g.cam.project(start + delta).unwrap()).unwrap();
            assert!((moved.position - pose.position).distance(delta) < 0.001);
            assert!((moved.position - pose.position).dot(g.axis(2)).abs() < 0.001);
        }
    }
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
        let mut drag = Drag::new(g, Part::Ring(2), g.cam.project(g.ring(2, start)).unwrap(), pose).unwrap();
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
    let mut drag = Drag::new(g, Part::Ring(0), start, pose).unwrap();
    let Motion::Edge { tangent, radius, .. } = drag.motion else {
        panic!("edge-on ring fallback");
    };
    let moved = drag.update(start + tangent * radius * 0.4).unwrap();
    assert_eq!(moved.position, pose.position);
    assert!(moved.rotation.dot(Quat::from_rotation_x(0.4) * pose.rotation).abs() > 0.99999);
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
                let (owns, edit) = gizmo.show(ui.ctx(), &p, g, pose, true);
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
