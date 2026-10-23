use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
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
    editor.select(lib.rig.names["mWristRight"], &lib.rig, &Frame::new(&world).unwrap());
    (world, editor)
}
fn projector(point: Vec3) -> Projector {
    let eye = point + Vec3::new(0.9, -1.2, 0.5);
    Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05)
            * glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z),
        width: 1600.0,
        height: 900.0,
        ppp: 1.0,
    }
}
fn click(editor: &mut Editor, world: &World, proj: &Projector, pointer: Pos2, modifiers: egui::Modifiers, cover: bool) {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let palette = theme.palette();
    for (step, pressed) in [None, None, Some(true), Some(false)].into_iter().enumerate() {
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
                events,
                time: Some(step as f64 / 60.0),
                ..Default::default()
            },
            |ui| {
                editor.show(ui.ctx(), &palette, world, proj);
                if cover {
                    egui::Area::new(egui::Id::unique("cover-segment"))
                        .fixed_pos(pointer - egui::vec2(30.0, 30.0))
                        .order(egui::Order::Middle)
                        .show(ui.ctx(), |ui| {
                            ui.allocate_exact_size(egui::vec2(60.0, 60.0), Sense::click());
                        });
                }
            },
        );
        output.textures_delta.clear();
    }
}

#[test]
fn narrow_segments_pick_nearest_front_crossings_and_never_their_bounding_box() {
    let lines = [
        Segment {
            source: Source::Bone(1),
            a: (egui::pos2(0.0, 0.0), 3.0),
            b: (egui::pos2(100.0, 100.0), 3.0),
        },
        Segment {
            source: Source::Bone(2),
            a: (egui::pos2(0.0, 0.0), 1.0),
            b: (egui::pos2(100.0, 100.0), 1.0),
        },
    ];
    assert_eq!(pick(&lines, egui::pos2(50.0, 50.0)).unwrap().source, Source::Bone(2));
    assert!(pick(&lines, egui::pos2(10.0, 90.0)).is_none());
    assert!(pick(&lines, egui::pos2(50.0, 55.0)).is_some());
    assert!(pick(&lines, egui::pos2(50.0, 65.0)).is_none());
}

#[test]
fn actual_shaft_clicks_select_the_origin_bone_and_keep_pose_and_child_points_intact() {
    for (name, child) in [
        ("mShoulderLeft", "mElbowLeft"),
        ("mElbowLeft", "mWristLeft"),
        ("mNeck", "mHead"),
        ("mFootLeft", "mToeLeft"),
    ] {
        let (world, mut editor) = fixture();
        let rig = &world.avatar_lib.rig;
        let frame = Frame::new(&world).unwrap();
        let bone = rig.names[name];
        let child = rig.names[child];
        let middle = (frame.joints[bone].w_axis.truncate() + frame.joints[child].w_axis.truncate()) * 0.5;
        let proj = projector(middle);
        let at = proj.project(middle).unwrap().0;
        click(&mut editor, &world, &proj, at, egui::Modifiers::NONE, false);
        assert_eq!(editor.selected, Some(bone), "shaft belongs to {name}");
        assert!(editor.goals.is_empty(), "selection must not create a pin or bake pose edits");
        assert!(editor.request().joints.is_empty());
        let origin = proj.project(frame.joints[child].w_axis.truncate()).unwrap().0;
        click(&mut editor, &world, &proj, origin, egui::Modifiers::NONE, false);
        assert_eq!(editor.selected, Some(child), "point handles keep priority over shafts");
    }
}

#[test]
fn segment_selection_respects_windows_visibility_camera_and_prim_edit_tools() {
    for mode in 0..5 {
        let (world, mut editor) = fixture();
        let rig = &world.avatar_lib.rig;
        let frame = Frame::new(&world).unwrap();
        let bone = rig.names["mShoulderLeft"];
        let child = rig.names["mElbowLeft"];
        let source = editor.selected;
        let middle = (frame.joints[bone].w_axis.truncate() + frame.joints[child].w_axis.truncate()) * 0.5;
        let proj = projector(middle);
        let at = proj.project(middle).unwrap().0;
        match mode {
            1 => editor.visibility.only(visibility::Group::Body),
            2 => editor.picking = true,
            3 => editor.prims.object_handles = true,
            _ => {}
        }
        click(
            &mut editor,
            &world,
            &proj,
            at,
            if mode == 4 { egui::Modifiers::ALT } else { egui::Modifiers::NONE },
            mode == 0,
        );
        assert_eq!(editor.selected, source, "blocked selection case {mode}");
    }
}

#[test]
fn shaft_target_picking_keeps_the_locked_source_and_terminal_bones_have_a_segment() {
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let source = editor.selected.unwrap();
    let bone = rig.names["mShoulderLeft"];
    let child = rig.names["mElbowLeft"];
    let middle = (frame.joints[bone].w_axis.truncate() + frame.joints[child].w_axis.truncate()) * 0.5;
    let proj = projector(middle);
    editor.begin_link(&world, &frame, source, false);
    click(
        &mut editor,
        &world,
        &proj,
        proj.project(middle).unwrap().0,
        egui::Modifiers::NONE,
        false,
    );
    assert_eq!(editor.selected, Some(source));
    assert!(!editor.picking);
    assert!(matches!(&editor.graph.definition.bindings[0].target, Target::Bone { bone, .. } if bone == "mShoulderLeft"));
    let toe = rig.names["mToeLeft"];
    let proj = projector(frame.joints[toe].w_axis.truncate());
    assert!(
        editor
            .skeleton_segments(&world, &proj, &frame)
            .iter()
            .any(|s| s.source == Source::Bone(toe) && s.a.0.distance(s.b.0) > 1.0)
    );
}

#[test]
fn virtual_parent_connectors_select_their_node() {
    use crate::world::rig_graph::{Node, Transform};
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let head = rig.names["mHead"];
    let id = Uuid::from_u128(0x419933);
    editor.graph.definition.nodes.push(Node {
        id,
        name: "Test control".into(),
        parent: Target::Bone {
            avatar: Uuid::nil(),
            bone: "mHead".into(),
        },
        offset: Transform::new(Vec3::new(0.25, 0.0, 0.12), Quat::IDENTITY),
        motion: None,
    });
    editor.update_graph(&world, &frame);
    let (_, q, p) = frame.joints[head].to_scale_rotation_translation();
    let proj = projector(p + q * Vec3::new(0.125, 0.0, 0.06));
    let lines = editor.virtual_segments(&proj, &frame);
    let line = lines.iter().find(|s| s.source == Source::Node(id)).unwrap();
    let at = line.a.0.lerp(line.b.0, 0.5);
    click(&mut editor, &world, &proj, at, egui::Modifiers::NONE, false);
    assert_eq!(editor.graph.selected, Some(id));
    assert_eq!(editor.tab, 4);
}
