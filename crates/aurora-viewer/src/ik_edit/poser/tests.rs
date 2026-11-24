use super::*;

#[test]
fn poser_sensitivity_is_persisted_and_scales_the_shared_gizmo_setting() {
    let mut editor = Editor {
        poser: State {
            active: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut config = crate::settings::Settings::default().poser;
    assert!(!editor.sync_poser_settings(&mut config));
    editor.set_poser_sensitivity(0.8);
    assert!(editor.sync_poser_settings(&mut config));
    assert_eq!(config.sensitivity, 0.8);
    assert_eq!(editor.poser.sensitivity, 0.8);
    assert!(!editor.sync_poser_settings(&mut config));

    editor.set_poser_sensitivity(f32::NAN);
    assert_eq!(editor.poser.sensitivity, super::super::gizmo::DEFAULT_SENSITIVITY);
    assert_eq!(editor.poser.config.sensitivity, super::super::gizmo::DEFAULT_SENSITIVITY);
}
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::NetEvent;

fn fixture() -> (World, Editor) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![demo::action_avatar(false)],
    });
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![demo::poser::peer()],
    });
    let rig = &lib.rig;
    world.avatar_poses.insert(
        world.agent_id,
        rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)
            .iter()
            .zip(&rig.default_world_inv)
            .map(|(p, i)| *p * *i)
            .collect(),
    );
    let mut editor = Editor::default();
    editor.open_poser();
    editor.pose_layers(&world);
    (world, editor)
}
pub(super) fn render(world: &mut World, editor: &Editor) {
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(world).unwrap();
    let pose = &editor.poser.sessions[&world.agent_id].pose;
    world.avatar_poses.insert(
        world.agent_id,
        pose.matrices(rig, frame.model)
            .iter()
            .zip(&rig.default_world_inv)
            .map(|(p, i)| *p * *i)
            .collect(),
    );
}
pub(super) fn selected_fixture() -> (World, Editor) {
    let (world, mut editor) = fixture();
    let frame = Frame::new(&world).unwrap();
    let rig = &world.avatar_lib.rig;
    editor.select(rig.names["mHead"], rig, &frame);
    (world, editor)
}
#[test]
fn poser_joint_toggle_falls_back_tracks_undo_and_survives_saved_pose_loads() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let bone = rig.names["mHead"];
    let mut session = data::Session::new(SubjectPose::capture(rig, &frame).unwrap());
    let animated_rotation = (rig.local_rot[bone] * Quat::from_rotation_z(0.42)).normalize();
    let animated_position = rig.local_pos[bone] + Vec3::new(0.03, 0.0, 0.0);
    let mut rotations = rig.local_rot.clone();
    let mut positions = rig.local_pos.clone();
    rotations[bone] = animated_rotation;
    positions[bone] = animated_position;
    session.toggle_joint(bone, None);
    assert!(!session.pose.enabled[bone]);
    let actual = session
        .pose
        .matrices_with_animation(rig, frame.model, &rotations, &positions, Vec3::ZERO);
    let mut expected_rotations = session.pose.rotations.clone();
    let mut expected_positions = session.pose.positions.clone();
    expected_rotations[bone] = animated_rotation;
    expected_positions[bone] = animated_position;
    let expected = rig.world_with(&expected_rotations, Vec3::ZERO, &expected_positions, &session.pose.scales);
    assert!(
        actual[bone].abs_diff_eq(expected[bone], 1e-5),
        "disabled joint follows its live animation"
    );
    let pelvis_offset = Vec3::new(0.08, -0.03, 0.02);
    let mut pelvis_pose = (*session.pose).clone();
    pelvis_pose.enabled[rig.pelvis] = false;
    let actual = pelvis_pose.matrices_with_animation(rig, frame.model, &rotations, &positions, pelvis_offset);
    let expected = rig.world_with(&rotations, pelvis_offset, &positions, &pelvis_pose.scales);
    assert!(
        actual[rig.pelvis].abs_diff_eq(expected[rig.pelvis], 1e-5),
        "disabled pelvis keeps the live offset"
    );

    session.history(false);
    assert!(session.pose.enabled[bone], "undo restores the enabled state");
    session.history(true);
    assert!(!session.pose.enabled[bone], "redo restores the disabled state");

    let document = document::Document::capture(rig, &session, "Disabled head".into(), 4);
    let encoded = serde_json::to_string(&document).unwrap();
    assert!(encoded.contains("\"enabled\":false"));
    let decoded: document::Document = serde_json::from_str(&encoded).unwrap();
    let mut loaded = data::Session::new(SubjectPose::capture(rig, &frame).unwrap());
    assert!(decoded.apply(rig, &mut loaded, document::LoadMode::All, frame.model));
    assert!(!loaded.pose.enabled[bone], "saved pose keeps a disabled joint");
    loaded = data::Session::new(SubjectPose::capture(rig, &frame).unwrap());
    assert!(decoded.apply(rig, &mut loaded, document::LoadMode::Selective, frame.model));
    assert!(loaded.pose.enabled[bone], "selective load enables joints it applies");

    let authored = session.authored(rig, frame.model).unwrap();
    assert!(
        !storage::native(rig, &authored, "Disabled head".into(), 4)
            .unwrap()
            .animation
            .joints
            .iter()
            .any(|joint| joint.joint_name == "mHead")
    );
    let bvh = bvh::encode(rig, &authored, false).unwrap();
    let mut neutral_head = authored.clone();
    neutral_head.enabled[bone] = true;
    neutral_head.rotations[bone] = Quat::IDENTITY;
    assert_eq!(
        bvh,
        bvh::encode(rig, &neutral_head, false).unwrap(),
        "disabled head exports neutral rotation"
    );
}
#[test]
fn poser_joint_rework_selective_load_reenables_an_already_off_edited_joint() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let head = rig.names["mHead"];
    let neck = rig.names["mNeck"];
    let mut source = data::Session::new(SubjectPose::capture(rig, &frame).unwrap());
    let saved = Arc::make_mut(&mut source.pose);
    saved.rotations[head] = (rig.local_rot[head] * Quat::from_rotation_z(0.35)).normalize();
    saved.origin[head] = rig.local_rot[head];
    saved.enabled[head] = false;
    let expected = saved.rotations[head];
    let document = document::Document::capture(rig, &source, "Selective disabled joint".into(), 4);
    for mode in [document::LoadMode::Selective, document::LoadMode::SelectiveRotations] {
        let mut destination = data::Session::new(SubjectPose::capture(rig, &frame).unwrap());
        destination.toggle_joint(head, None);
        let pose = Arc::make_mut(&mut destination.pose);
        pose.rotations[head] = (rig.local_rot[head] * Quat::from_rotation_x(-0.5)).normalize();
        pose.rotations[neck] = (rig.local_rot[neck] * Quat::from_rotation_y(0.2)).normalize();
        let keep = pose.rotations[neck];
        assert!(destination.edited(head));
        assert!(document.apply(rig, &mut destination, mode, frame.model));
        assert!(destination.pose.enabled[head], "selective load must re-enable an already off joint");
        assert!(destination.pose.rotations[head].abs_diff_eq(expected, 1e-6));
        assert!(
            destination.pose.rotations[neck].abs_diff_eq(keep, 1e-6),
            "edited posed joints stay untouched"
        );
    }
}

#[test]
fn poser_joint_rework_disabled_pelvis_exports_no_bvh_translation_when_unlocked() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let mut pose = SubjectPose::capture(rig, &frame).unwrap();
    pose.enabled[rig.pelvis] = false;
    pose.positions[rig.pelvis] += Vec3::new(0.4, -0.2, 0.3);
    pose.rotations[rig.pelvis] = Quat::from_rotation_z(0.7);
    let actual = bvh::encode(rig, &pose, true).unwrap();
    pose.positions[rig.pelvis] = rig.local_pos[rig.pelvis];
    pose.rotations[rig.pelvis] = Quat::IDENTITY;
    assert_eq!(actual, bvh::encode(rig, &pose, true).unwrap());
}

#[test]
fn poser_joint_rework_row_and_finger_switches_toggle_without_selecting_and_mirror_in_one_undo() {
    use crate::ui::widgets::elide_tests::Frames;
    for hands in [false, true] {
        for side in [Side::Left, Side::Right, Side::Mirror] {
            let (world, mut editor) = selected_fixture();
            let rig = &world.avatar_lib.rig;
            let frame = editor.editor_frame(&world).unwrap();
            editor.set_pose_side(side, rig, &frame);
            editor.poser.filter = if hands { "mHandIndex" } else { "mElbow" }.into();
            let name = match (hands, side) {
                (false, Side::Right) => "mElbowRight",
                (false, _) => "mElbowLeft",
                (true, Side::Right) => "mHandIndex1Right",
                (true, _) => "mHandIndex1Left",
            };
            let bone = rig.names[name];
            let other = editor.mirrors.counterpart(bone).unwrap();
            let selected = editor.selected;
            let before = editor.poser.sessions[&world.agent_id].pose.clone();
            let mut frames = Frames::new();
            let p = crate::theme::Theme::default().palette();
            let right_edge = std::cell::Cell::new(0.0);
            let mut draw = |ui: &mut egui::Ui| {
                ui.set_width(240.0);
                right_edge.set(ui.max_rect().right());
                if hands {
                    editor.hands_panel(ui, &p, rig, &frame);
                } else {
                    editor.bone_list(ui, &p, rig, &frame);
                }
            };
            for _ in 0..3 {
                frames.run(Vec::new(), &mut draw);
            }
            let shapes = frames.shapes(Vec::new(), &mut draw);
            let switches: Vec<_> = shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(r) if r.rect.size() == widgets::SWITCH_SIZE => Some(r.rect),
                    _ => None,
                })
                .collect();
            assert_eq!(switches.len(), if hands { 3 } else { 1 });
            assert!(
                switches.iter().all(|r| r.right() <= right_edge.get() + 0.01),
                "finger switches fit the narrow joint column"
            );
            frames.run(click(switches[0].center()), &mut draw);
            assert_eq!(editor.selected, selected, "the switch must not select its row");
            let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
            assert!(!session.pose.enabled[bone]);
            assert_eq!(session.pose.enabled[other], side != Side::Mirror);
            assert_eq!(session.undo.len(), 1, "Mirror toggles both in one history entry");
            assert_eq!(session.pose.rotations, before.rotations, "the stored pose is retained");
            session.history(false);
            assert!(session.pose.enabled[bone] && session.pose.enabled[other]);
            session.history(true);
            assert!(!session.pose.enabled[bone]);
            assert_eq!(session.pose.enabled[other], side != Side::Mirror);
            let mut draw = |ui: &mut egui::Ui| {
                ui.set_width(240.0);
                if hands {
                    editor.hands_panel(ui, &p, rig, &frame);
                } else {
                    editor.bone_list(ui, &p, rig, &frame);
                }
            };
            frames.run(click(egui::pos2(switches[0].left() - 12.0, switches[0].center().y)), &mut draw);
            assert_eq!(editor.selected, Some(bone), "the rest of the cell still selects");
            assert!(!editor.poser.sessions[&world.agent_id].pose.enabled[bone]);
        }
    }
}

#[test]
fn poser_joint_rework_duplicate_pelvis_rows_have_independent_switch_hit_targets() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let p = crate::theme::Theme::default().palette();
    let mut frames = Frames::new();
    let mut draw = |ui: &mut egui::Ui| {
        ui.set_width(300.0);
        editor.bone_list(ui, &p, rig, &frame);
    };
    for _ in 0..3 {
        frames.run(Vec::new(), &mut draw);
    }
    let shapes = frames.shapes(Vec::new(), &mut draw);
    let first = shapes
        .iter()
        .find_map(|shape| match shape {
            egui::Shape::Rect(r) if r.rect.size() == widgets::SWITCH_SIZE => Some(r.rect),
            _ => None,
        })
        .unwrap();
    frames.run(click(first.center()), &mut draw);
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(
        !session.pose.enabled[rig.pelvis],
        "the avatar-position row changes the pelvis exactly once"
    );
    assert_eq!(session.undo.len(), 1);
}

#[test]
fn poser_joint_rework_off_joint_slider_ignores_pointer_edits() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    for enabled in [true, false] {
        let (world, mut editor) = selected_fixture();
        let rig = &world.avatar_lib.rig;
        let frame = editor.editor_frame(&world).unwrap();
        let bone = editor.selected.unwrap();
        if !enabled {
            editor.toggle_pose_joint(bone);
        }
        let before = editor.poser.sessions[&world.agent_id].pose.rotations[bone];
        let undo = editor.poser.sessions[&world.agent_id].undo.len();
        let mut frames = Frames::new();
        let p = crate::theme::Theme::default().palette();
        let mut draw = |ui: &mut egui::Ui| {
            ui.set_width(650.0);
            editor.joint_controls(ui, &p, rig, &frame);
        };
        for _ in 0..3 {
            frames.run(Vec::new(), &mut draw);
        }
        let texts = frames.run(Vec::new(), &mut draw);
        let x = painted(&texts, "X").rect;
        // Sliders change while the pointer is held, unlike a button's release click.
        let events = click(egui::pos2(x.right() + 20.0, x.center().y));
        frames.run(events[..2].to_vec(), &mut draw);
        frames.run(events[2..].to_vec(), &mut draw);
        let session = &editor.poser.sessions[&world.agent_id];
        assert_eq!(!session.pose.rotations[bone].abs_diff_eq(before, 1e-6), enabled);
        assert_eq!(session.undo.len(), undo + usize::from(enabled));
    }
}

#[test]
fn poser_joint_rework_live_ancestor_keeps_animation_priority_over_child_pin() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let neck = rig.names["mNeck"];
    let head = rig.names["mHead"];
    let mut pose = SubjectPose::capture(rig, &frame).unwrap();
    pose.locks.insert(head, frame.joints[head]);
    pose.enabled[neck] = false;
    let mut animated = rig.local_rot.clone();
    animated[neck] = (animated[neck] * Quat::from_rotation_z(0.3)).normalize();
    let mut positions = rig.local_pos.clone();
    positions[neck] += Vec3::new(0.03, -0.02, 0.01);
    let actual = pose.matrices_with_animation(rig, frame.model, &animated, &positions, Vec3::ZERO);
    let parent = rig.parent[neck].unwrap();
    let expected = actual[parent] * Mat4::from_scale_rotation_translation(pose.scales[neck], animated[neck], positions[neck]);
    assert!(
        actual[neck].abs_diff_eq(expected, 1e-5),
        "a descendant pin cannot pose an off ancestor"
    );
    assert!(actual[head].is_finite());
}

pub(super) fn projector(point: Vec3) -> Projector {
    let eye = point + Vec3::new(-2.0, -3.0, 0.3);
    Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 800.0 / 900.0, 0.05)
            * glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z),
        width: 800.0,
        height: 900.0,
        ppp: 1.0,
    }
}
#[test]
fn poser_face_position_press_keeps_the_value_and_drag_moves_in_tenths_of_a_millimetre() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let mut editor = Editor::default();
    editor.poser.transform = 1;
    let p = crate::theme::Theme::default().palette();
    for (delta, initial) in [(false, 1.637), (true, -0.016)] {
        editor.poser.delta = delta;
        let mut frames = Frames::new();
        let mut value = Vec3::splat(initial);
        let draw = |ui: &mut egui::Ui, value: &mut Vec3| {
            ui.set_width(320.0);
            editor.axis_sliders(ui, &p, value, Some(Vec3::splat(1.63)), "mFaceLipCornerLeft");
        };
        let rows = frames.run(Vec::new(), |ui| draw(ui, &mut value));
        let at = painted(&rows, "X").rect.right_center() + egui::vec2(60.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        frames.run(vec![egui::Event::PointerMoved(at), button(at, true)], |ui| draw(ui, &mut value));
        assert_eq!(value, Vec3::splat(initial), "pressing away from the center leaves the pose alone");
        let end = at + egui::vec2(20.0, 0.0);
        frames.run(vec![egui::Event::PointerMoved(end)], |ui| draw(ui, &mut value));
        frames.run(vec![button(end, false)], |ui| draw(ui, &mut value));
        assert!((value.x - initial - 0.002).abs() < 0.00011, "{value:?}");
        assert_eq!((value.y, value.z), (initial, initial));
    }
}

#[test]
fn poser_tabs_replace_visible_joints_and_selection_without_editing_the_pose() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    let initial = editor.poser.sessions[&world.agent_id].pose.clone();
    // as if a worn mesh used every Other bone: the tab shows them all but
    // the spine, the groin and the skull (the full window's list alone)
    editor.poser.rigged = (0..rig.len()).filter(|b| group(&rig.canonical_names[*b]) == OTHER_GROUP).collect();
    let mut texts = Vec::new();
    for _ in 0..5 {
        texts = frames.run(Vec::new(), |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    for (tab, key) in [
        (1, "poser-face"),
        (2, "poser-hands"),
        (3, "poser-misc"),
        (4, "poser-physics"),
        (0, "poser-body"),
    ] {
        let pointer = painted(&texts, aurora_i18n::text(key)).rect.center();
        for pressed in [true, false] {
            texts = frames.run(
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::PointerButton {
                        pos: pointer,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                |ui| editor.show(ui.ctx(), &p, &world, &proj),
            );
        }
        assert_eq!(editor.poser.group, tab);
        assert_eq!(group(&rig.canonical_names[editor.selected.unwrap()]), tab);
        let shapes = frames.shapes(Vec::new(), |ui| editor.show(ui.ctx(), &p, &world, &proj));
        let markers = shapes
            .iter()
            .filter(|s| matches!(s, egui::Shape::Circle(c) if c.radius == 2.5 || c.radius == 4.5))
            .count();
        assert_eq!(
            markers,
            (0..rig.len())
                .filter(|b| group(&rig.canonical_names[*b]) == tab && proj.project(frame.joints[*b].w_axis.truncate()).is_some())
                .filter(|b| !inner_bone(&rig.canonical_names[*b]))
                .count(),
            "only active tab markers are painted"
        );
        for name in &rig.canonical_names {
            assert_eq!(editor.bone_visible(name), group(name) == tab && !inner_bone(name), "{name}");
        }
        for segment in editor.skeleton_segments(&world, &proj, &frame) {
            assert_eq!(group(&rig.canonical_names[segment.bone]), tab);
        }
        assert!(editor.poser.sessions[&world.agent_id].undo.is_empty());
        assert_eq!(editor.poser.sessions[&world.agent_id].pose.rotations, initial.rotations);
        assert_eq!(editor.poser.sessions[&world.agent_id].pose.positions, initial.positions);
    }
}
#[test]
fn poser_ignores_stale_ik_edit_state() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let bone = rig.names["mHead"];
    editor.object_handles = true;
    editor.limb_controls = true;
    let frame = editor.editor_frame(&world).unwrap();
    editor.select(bone, rig, &frame);
    assert_eq!(editor.root, bone);
    assert!(editor.mode == Mode::Direct);
    assert_eq!(editor.contact, Vec3::ZERO);
    let proj = projector(frame.joints[bone].w_axis.truncate());
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    for _ in 0..3 {
        frames.run(Vec::new(), |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    assert!(
        frames
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(("poser-joint", bone))))
            .is_some()
    );
    assert!(!editor.request().active);
    // Losing a subject during a drag must never fall through to IK authoring.
    editor.poser.sessions.clear();
    editor.commit_manipulation(rig, &frame, true);
    assert!(editor.goals.is_empty());
}
#[test]
fn poser_bone_marker_drag_rotates_the_pose_without_creating_a_binding_or_ik_goal() {
    use crate::ui::widgets::elide_tests::Frames;
    for mode in [0, 1] {
        let (mut world, mut editor) = selected_fixture();
        let bone = world.avatar_lib.rig.names[if mode == 0 { "mHead" } else { "mWristLeft" }];
        let frame = editor.editor_frame(&world).unwrap();
        editor.poser.transform = mode;
        editor.select(bone, &world.avatar_lib.rig, &frame);
        let proj = projector(frame.joints[bone].w_axis.truncate());
        let pointer = proj.project(frame.joints[bone].w_axis.truncate()).unwrap().0;
        let mut frames = Frames::new();
        // the window minimized to its bar: the joint is in the middle of a
        // screen narrower than the window and the drag happens in the world
        crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
        let p = crate::theme::Theme::default().palette();
        let initial = editor.poser.sessions[&world.agent_id].pose.rotations[bone];
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        let end = pointer + egui::vec2(35.0, -20.0);
        for (step, events) in [
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![egui::Event::PointerMoved(pointer), button(pointer, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ]
        .into_iter()
        .enumerate()
        {
            frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
            render(&mut world, &editor);
            if step < 3 {
                assert_eq!(editor.poser.sessions[&world.agent_id].pose.rotations[bone], initial);
                assert!(editor.poser.sessions[&world.agent_id].undo.is_empty());
            }
        }
        assert_eq!(editor.selected, Some(bone));
        if mode == 0 {
            assert!(initial.dot(editor.poser.sessions[&world.agent_id].pose.rotations[bone]).abs() < 0.999);
        } else {
            let session = &editor.poser.sessions[&world.agent_id];
            let moved = editor.editor_frame(&world).unwrap();
            assert!(moved.joints[bone].w_axis.distance(frame.joints[bone].w_axis) > 0.001);
            assert_attached(&world.avatar_lib.rig, &session.pose, &session.base, frame.model);
        }
        assert_eq!(editor.poser.sessions[&world.agent_id].undo.len(), 1);
        assert!(editor.goals.is_empty());
        editor.poser.sessions.get_mut(&world.agent_id).unwrap().history(false);
        assert_eq!(editor.poser.sessions[&world.agent_id].pose.rotations[bone], initial);
    }
}

#[test]
fn poser_pelvis_marker_drag_rotates_the_root_without_breaking_its_children() {
    use crate::ui::widgets::elide_tests::Frames;
    let (mut world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let pelvis = rig.pelvis;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[pelvis].w_axis.truncate());
    let pointer = proj.project(frame.joints[pelvis].w_axis.truncate()).unwrap().0;
    let initial = editor.poser.sessions[&world.agent_id].pose.rotations[pelvis];
    let base = editor.poser.sessions[&world.agent_id].base.clone();
    let mut frames = Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    let p = crate::theme::Theme::default().palette();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    let end = pointer + egui::vec2(35.0, -20.0);
    for events in [
        Vec::new(),
        Vec::new(),
        vec![egui::Event::PointerMoved(pointer), button(pointer, true)],
        vec![egui::Event::PointerMoved(end)],
        vec![button(end, false)],
    ] {
        frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
        render(&mut world, &editor);
    }

    assert_eq!(editor.selected, Some(pelvis));
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(initial.dot(session.pose.rotations[pelvis]).abs() < 0.999);
    assert_attached(&rig, &session.pose, &base, frame.model);
    assert_eq!(session.undo.len(), 1);
    assert!(editor.goals.is_empty());
}

#[test]
fn poser_static_layer_keeps_fk_ancestors_scales_and_world_locks() {
    let (mut world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let head = rig.names["mHead"];
    let frame = Frame::new(&world).unwrap();
    let base = editor.poser.sessions[&world.agent_id].pose.clone();
    editor.select(head, &rig, &frame);
    editor.poser_transform(&rig, &frame, base.positions[head], Quat::from_rotation_z(0.4), true);
    render(&mut world, &editor);
    let result = &editor.poser.sessions[&world.agent_id].pose;
    assert!(result.rotations[head].dot(Quat::from_rotation_z(0.4)).abs() > 0.99999);
    assert_eq!(
        result.rotations[rig.parent[head].unwrap()],
        base.rotations[rig.parent[head].unwrap()]
    );
    let frame = Frame::new(&world).unwrap();
    editor.pose_operation(&rig, &frame, controls::Operation::WorldLock);
    let pose = editor.poser.sessions[&world.agent_id].pose.clone();
    let shifted = Mat4::from_translation(Vec3::new(2.0, 3.0, 4.0)) * frame.model;
    let locked = shifted * pose.matrices(&rig, shifted)[head];
    assert!(
        !locked.abs_diff_eq(frame.joints[head], 1e-4),
        "an unreachable lock must yield instead of detaching the head"
    );
    let resolved = pose.resolved(&rig, shifted);
    assert_eq!(resolved.positions, base.positions);
    let snapshot = pose.snapshot(&rig, shifted).unwrap();
    assert!(snapshot.matrices(&rig, Mat4::IDENTITY)[head].abs_diff_eq(pose.matrices(&rig, shifted)[head], 1e-4));
    let chest = rig.names["CHEST"];
    let mut scaled = (*pose).clone();
    scaled.scales[chest] *= 1.4;
    assert!(scaled.valid(&rig));
    assert!(
        scaled.matrices(&rig, frame.model)[chest]
            .to_scale_rotation_translation()
            .0
            .abs_diff_eq(scaled.scales[chest], 1e-5)
    );
}

#[test]
fn poser_shape_changes_rebase_pose_and_undo_onto_the_new_rest() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let head = rig.names["mHead"];
    let chest = rig.names["CHEST"];
    let hip = rig.pelvis;
    let initial_rest = Arc::new(crate::scene::anim::SkeletonBase {
        local_pos: rig.local_pos.clone(),
        scale: rig.scale.clone(),
    });
    editor.poser.sessions.get_mut(&world.agent_id).unwrap().rebase(initial_rest.clone());
    let frame = Frame::new(&world).unwrap();
    editor.select(head, &rig, &frame);
    let head_position = editor.poser.sessions[&world.agent_id].pose.positions[head];
    editor.poser_transform(&rig, &frame, head_position, Quat::from_rotation_z(0.4), true);
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    session.checkpoint();
    let posed_head = session.pose.rotations[head];
    let old_hip = session.pose.positions[hip];
    let old_hip_world = session.pose.matrices(&rig, frame.model)[hip].w_axis.z;
    let mut changed_rest = (*initial_rest).clone();
    changed_rest.scale[chest] *= 1.3;
    changed_rest.local_pos[hip].z += 0.12;
    session.rebase(Arc::new(changed_rest));

    let session = &editor.poser.sessions[&world.agent_id];
    assert_eq!(session.pose.rotations[head], posed_head);
    assert!((session.pose.scales[chest] - rig.scale[chest] * 1.3).length() < 1e-5);
    assert!((session.pose.positions[hip] - (old_hip + Vec3::new(0.0, 0.0, 0.12))).length() < 1e-5);
    assert!(session.edited(head));
    assert!(!session.edited(chest));
    let matrices = session.pose.matrices(&rig, frame.model);
    assert!((matrices[hip].w_axis.z - old_hip_world - 0.12).abs() < 1e-4);

    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    session.history(false);
    assert!((session.pose.scales[chest] - rig.scale[chest] * 1.3).length() < 1e-5);
    assert!((session.pose.positions[hip] - (rig.local_pos[hip] + Vec3::new(0.0, 0.0, 0.12))).length() < 1e-5);
    session.history(true);
    assert_eq!(session.pose.rotations[head], posed_head);
    assert!((session.pose.scales[chest] - rig.scale[chest] * 1.3).length() < 1e-5);
    assert!((session.pose.positions[hip] - (old_hip + Vec3::new(0.0, 0.0, 0.12))).length() < 1e-5);

    // A redo entry follows the rest too: undo, return to the first shape, redo.
    session.history(false);
    session.rebase(initial_rest);
    session.history(true);
    assert_eq!(session.pose.rotations[head], posed_head);
    assert!((session.pose.scales[chest] - rig.scale[chest]).length() < 1e-5);
    assert!((session.pose.positions[hip] - old_hip).length() < 1e-5);
}

#[test]
fn poser_rebase_from_a_flat_rest_axis_takes_the_new_rest_scale() {
    // Shape sliders can drive a joint's rest scale to zero on one axis
    // (#1249); the pose's ratio to it must not blow the joint up.
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let chest = rig.names["CHEST"];
    let rest = crate::scene::anim::SkeletonBase {
        local_pos: rig.local_pos.clone(),
        scale: rig.scale.clone(),
    };
    let mut flat = rest.clone();
    flat.scale[chest].x = 0.0;
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    session.rebase(Arc::new(flat));
    session.rebase(Arc::new(rest));
    assert!(session.pose.scales[chest].is_finite());
    assert!((session.pose.scales[chest] - rig.scale[chest]).length() < 1e-5);
}

#[test]
fn poser_a_rest_axis_still_flat_after_another_shape_change_keeps_the_pose_valid() {
    // Pose, a slider flattens the chest's X (#1249), then another slider
    // changes the rest while that axis stays flat: the pose and its undo
    // steps must stay usable, not be dropped for a zero scale.
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let chest = rig.names["CHEST"];
    let neck = rig.names["NECK"];
    let rest = crate::scene::anim::SkeletonBase {
        local_pos: rig.local_pos.clone(),
        scale: rig.scale.clone(),
    };
    let mut flat = rest.clone();
    flat.scale[chest].x = 0.0;
    let mut flat_taller = flat.clone();
    flat_taller.scale[neck] *= 1.2;
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    session.rebase(Arc::new(flat));
    session.rebase(Arc::new(flat_taller));
    let scale = session.pose.scales[chest];
    assert!(super::data::scale_valid(scale), "chest scale {scale} is out of the poser's bounds");
    assert!(session.pose.scales.iter().all(|s| super::data::scale_valid(*s)));
}
fn assert_attached(rig: &Rig, pose: &SubjectPose, reference: &SubjectPose, model: Mat4) {
    let matrices = pose.matrices(rig, model);
    for (bone, parent) in rig.parent.iter().enumerate() {
        if let Some(parent) = parent {
            assert_eq!(
                pose.positions[bone], reference.positions[bone],
                "{} offset changed",
                rig.canonical_names[bone]
            );
            let offset = matrices[*parent].inverse().transform_point3(matrices[bone].w_axis.truncate());
            assert!(
                offset.abs_diff_eq(reference.positions[bone], 0.0001),
                "{} detached",
                rig.canonical_names[bone]
            );
        }
    }
}
#[test]
fn poser_moving_wrists_fingers_and_ankles_preserves_the_complete_shaped_hierarchy() {
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let parents = rig.parent.clone();
    let base = editor.poser.sessions[&world.agent_id].base.clone();
    let frame = editor.editor_frame(&world).unwrap();
    editor.symmetry = true;
    for name in ["mWristLeft", "mHandIndex3Left", "mAnkleRight"] {
        let bone = rig.names[name];
        editor.select(bone, rig, &frame);
        editor.position = frame.model.inverse().transform_point3(frame.joints[bone].w_axis.truncate()) + Vec3::new(0.1, -0.15, -0.2);
        editor.commit_manipulation(rig, &frame, false);
        assert_attached(rig, &editor.poser.sessions[&world.agent_id].pose, &base, frame.model);
    }
    let wrist = rig.names["mWristLeft"];
    editor.select(wrist, rig, &frame);
    editor.position = Vec3::splat(20.0);
    editor.commit_manipulation(rig, &frame, false);
    let pose = &editor.poser.sessions[&world.agent_id].pose;
    assert_attached(rig, pose, &base, frame.model);
    assert!(pose.matrices(rig, frame.model)[wrist].w_axis.truncate().distance(editor.position) > 10.0);
    assert_eq!(rig.parent, parents);
    assert!(editor.request().goals.is_empty());
    editor.select(rig.pelvis, rig, &frame);
    editor.position += Vec3::new(0.2, 0.0, 0.0);
    editor.commit_manipulation(rig, &frame, false);
    assert_attached(rig, &editor.poser.sessions[&world.agent_id].pose, &base, frame.model);
}

#[test]
fn poser_rotating_the_pelvis_moves_the_body_as_one_attached_hierarchy() {
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let base = editor.poser.sessions[&world.agent_id].base.clone();
    let frame = editor.editor_frame(&world).unwrap();
    let pelvis = rig.pelvis;
    let torso = rig.names["mTorso"];
    let before = editor.poser.sessions[&world.agent_id].pose.matrices(rig, frame.model);
    let rotation = Quat::from_rotation_x(0.45);

    editor.select(pelvis, rig, &frame);
    editor.set_rotation(rotation);
    editor.commit_manipulation(rig, &frame, true);

    let session = &editor.poser.sessions[&world.agent_id];
    let after = session.pose.matrices(rig, frame.model);
    assert!(session.pose.rotations[pelvis].dot(rotation).abs() > 0.99999);
    assert!(
        after[torso].w_axis.truncate().distance(before[torso].w_axis.truncate()) > 0.01,
        "the torso follows the root rotation"
    );
    assert_attached(rig, &session.pose, &base, frame.model);
}

#[test]
fn poser_selects_and_rotates_every_animation_joint_without_breaking_the_hierarchy() {
    let (mut world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let base = editor.poser.sessions[&world.agent_id].base.clone();
    let animated: Vec<_> = rig
        .canonical_names
        .iter()
        .enumerate()
        .filter_map(|(bone, name)| name.starts_with('m').then_some(bone))
        .collect();
    assert!(animated.len() > 100, "the rig includes the full Bento skeleton");

    for bone in animated {
        let frame = editor.editor_frame(&world).unwrap();
        editor.select_pose_bone(bone, &rig, &frame);
        assert_eq!(editor.selected, Some(bone), "select {}", rig.canonical_names[bone]);
        let before = editor.poser.sessions[&world.agent_id].pose.rotations[bone];
        let world_rotation = (frame.model.inverse() * frame.joints[bone]).to_scale_rotation_translation().1;
        editor.set_rotation(Quat::from_rotation_z(0.02) * world_rotation);
        editor.poser.drag_checkpoint = false;
        assert!(
            editor.commit_poser_manipulation(&rig, &frame, true),
            "edit {}",
            rig.canonical_names[bone]
        );
        render(&mut world, &editor);

        let pose = &editor.poser.sessions[&world.agent_id].pose;
        assert!(pose.rotations[bone].is_finite(), "rotation of {}", rig.canonical_names[bone]);
        assert!(
            before.dot(pose.rotations[bone]).abs() < 0.999999,
            "rotation changed for {}",
            rig.canonical_names[bone]
        );
        assert!(pose.valid(&rig), "valid pose after {}", rig.canonical_names[bone]);
        assert_attached(&rig, pose, &base, frame.model);
    }

    assert!(editor.goals.is_empty());
}

#[test]
fn poser_world_pins_and_imported_translations_cannot_detach_children() {
    let (world, editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let base = editor.poser.sessions[&world.agent_id].base.clone();
    let mut pose = (*base).clone();
    let wrist = rig.names["mWristLeft"];
    let before = pose.matrices(rig, Mat4::IDENTITY);
    let target = before[wrist].w_axis.truncate() + Vec3::new(0.1, -0.15, -0.2);
    assert!(pose.move_joint(rig, wrist, target));
    let pinned = pose.matrices(rig, Mat4::IDENTITY)[wrist];
    pose.locks.insert(wrist, pinned);
    for model in [Mat4::IDENTITY, Mat4::from_translation(Vec3::splat(10.0))] {
        assert_attached(rig, &pose, &base, model);
        let snapshot = pose.snapshot(rig, model).unwrap();
        assert_attached(rig, &snapshot, &base, Mat4::IDENTITY);
    }
    let mut session = data::Session::new((*base).clone());
    pose.locks.clear();
    pose.positions[wrist] += Vec3::new(0.0, 0.0, -0.5);
    session.pose = Arc::new(pose);
    let document = document::Document::capture(rig, &session, "Old translated wrist".into(), 4);
    let mut destination = data::Session::new((*base).clone());
    assert!(document.apply(rig, &mut destination, document::LoadMode::All, Mat4::IDENTITY));
    assert_attached(rig, &destination.pose, &base, Mat4::IDENTITY);
}
#[test]
fn poser_documents_are_identity_free_and_load_modes_preserve_previously_edited_joints() {
    let (world, editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let mut session = editor.poser.sessions[&world.agent_id].clone();
    let head = rig.names["mHead"];
    let hand = rig.names["mHandIndex1Right"];
    Arc::make_mut(&mut session.pose).rotations[head] = Quat::from_rotation_z(0.5);
    Arc::make_mut(&mut session.pose).positions[hand] += Vec3::new(0.01, 0.0, 0.0);
    Arc::make_mut(&mut session.pose).scales[hand] *= 1.2;
    let document = document::Document::capture(rig, &session, "Portrait".into(), 4);
    let bytes = serde_json::to_vec(&document).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(!text.contains(&world.agent_id.to_string()));
    assert!(document.valid(rig));
    let decoded: document::Document = serde_json::from_slice(&bytes).unwrap();
    let mut destination = data::Session::new((*session.base).clone());
    Arc::make_mut(&mut destination.pose).rotations[head] = Quat::from_rotation_x(0.7);
    assert!(decoded.apply(rig, &mut destination, document::LoadMode::Selective, Mat4::IDENTITY));
    assert!(destination.pose.rotations[head].dot(Quat::from_rotation_x(0.7)).abs() > 0.99999);
    assert!(destination.pose.scales[hand].abs_diff_eq(session.pose.scales[hand], 1e-5));
    let mut rotations = data::Session::new((*session.base).clone());
    assert!(decoded.apply(rig, &mut rotations, document::LoadMode::Rotations, Mat4::IDENTITY));
    assert_eq!(rotations.pose.positions[hand], session.base.positions[hand]);
    assert_eq!(rotations.pose.scales[hand], session.base.scales[hand]);
    let mut bad = decoded;
    bad.joints[0].name = "unknown".into();
    assert!(!bad.valid(rig));
}
#[test]
fn poser_mirror_undo_hand_presets_reset_and_native_export_use_real_bento_channels() {
    let (mut world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let frame = Frame::new(&world).unwrap();
    let left = rig.names["mElbowLeft"];
    let right = rig.names["mElbowRight"];
    editor.select(left, &rig, &frame);
    let base = editor.poser.sessions[&world.agent_id].pose.clone();
    editor.poser_transform(&rig, &frame, base.positions[left], Quat::from_rotation_z(0.6), true);
    render(&mut world, &editor);
    editor.pose_operation(&rig, &Frame::new(&world).unwrap(), controls::Operation::Mirror);
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    assert!(session.edited(right));
    session.history(false);
    assert!(!session.edited(right));
    session.history(true);
    assert!(session.edited(right));
    editor.poser.preset = 1;
    editor.hand_preset(&rig, Some(true));
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(session.edited(rig.names["mHandIndex1Left"]));
    let native = storage::native(&rig, &session.authored(&rig, frame.model).unwrap(), "Pose".into(), 4).unwrap();
    assert!(native.animation.looping);
    assert_eq!(native.animation.base_priority, 4);
    assert!(native.animation.joints.iter().any(|j| j.joint_name == "mHandIndex1Left"));
    editor.pose_operation(&rig, &frame, controls::Operation::ResetAll);
    assert!((0..rig.len()).all(|b| !editor.poser.sessions[&world.agent_id].edited(b)));
}
#[test]
fn poser_native_export_only_keys_the_channels_the_user_changed() {
    let (world, mut editor, left, _, _) = captured_animation();
    let rig = &world.avatar_lib.rig;
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    let pose = Arc::make_mut(&mut session.pose);
    pose.rotations[left] *= Quat::from_rotation_y(0.07);
    pose.positions[rig.pelvis].z += 0.125;
    let lip = rig.names["mFaceLipCornerLeft"];
    pose.positions[lip].y += 0.002;
    let authored = session.authored(rig, Mat4::IDENTITY).unwrap();
    let upload = storage::native(rig, &authored, "Pose".into(), 4).unwrap();
    let tracks = &upload.animation.joints;
    assert_eq!(tracks.len(), 3, "untouched joints are available to attachments");
    let elbow = tracks.iter().find(|j| j.joint_name == "mElbowLeft").unwrap();
    assert!(near(elbow.rot_keys[0].rotation, authored.rotations[left]));
    assert!(elbow.pos_keys.is_empty());
    for (name, expected) in [("mPelvis", Vec3::Z * 0.125), ("mFaceLipCornerLeft", authored.positions[lip])] {
        let joint = tracks.iter().find(|j| j.joint_name == name).unwrap();
        assert!(joint.rot_keys.is_empty());
        assert!(joint.pos_keys[0].position.abs_diff_eq(expected, 0.0002));
    }
}
#[test]
fn poser_bvh_export_round_trips_axis_order_and_pelvis_unlock_through_the_real_importer() {
    let (world, editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let mut pose = (*editor.poser.sessions[&world.agent_id].pose).clone();
    let head = rig.names["mHead"];
    pose.rotations[head] = Quat::from_euler(EulerRot::ZYX, 0.3, -0.2, 0.4);
    pose.positions[rig.pelvis] += Vec3::new(0.1, 0.2, 0.3);
    pose.rotations[rig.pelvis] = Quat::from_rotation_z(0.2);
    let bvh = bvh::encode(rig, &pose, true).unwrap();
    let imported = aurora_assets::import_animation::parse_bvh(bvh.as_bytes(), world.avatar_lib.skeleton.as_ref().unwrap()).unwrap();
    let head_track = imported.animation.joints.iter().find(|j| j.joint_name == "mHead").unwrap();
    assert!(head_track.rot_keys[0].rotation.dot(pose.rotations[head]).abs() > 0.99999);
    let pelvis = imported.animation.joints.iter().find(|j| j.joint_name == "mPelvis").unwrap();
    assert!(pelvis.pos_keys[0].position.abs_diff_eq(Vec3::new(0.1, 0.2, 0.3), 1e-5));
    let locked = bvh::encode(rig, &pose, false).unwrap();
    let imported = aurora_assets::import_animation::parse_bvh(locked.as_bytes(), world.avatar_lib.skeleton.as_ref().unwrap()).unwrap();
    assert!(
        imported
            .animation
            .joints
            .iter()
            .find(|j| j.joint_name == "mPelvis")
            .is_none_or(|j| j.pos_keys.is_empty())
    );
}
#[test]
fn poser_closing_and_ik_switch_release_every_subject_unless_keep_pose_is_explicit() {
    let (world, mut editor) = fixture();
    assert!(!editor.pose_layers(&world).is_empty());
    editor.close_poser();
    assert!(editor.pose_layers(&world).is_empty());
    editor.open_poser();
    editor.pose_layers(&world);
    editor.poser.stop_on_close = false;
    editor.close_poser();
    assert!(!editor.pose_layers(&world).is_empty());
    editor.open_ik();
    assert!(!editor.poser.active);
    assert!(editor.pose_layers(&world).is_empty());
}
/// The demo animesh without DemoSim's "you own this" update flag, so only
/// its OwnerID decides whose it is.
fn animesh_not_flagged_ours() -> Vec<aurora_net::objects::ObjectUpdate> {
    demo::animesh_objects(demo::ANIMESH_MESH)
        .into_iter()
        .map(|mut o| {
            o.update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
            o
        })
        .collect()
}
#[test]
fn an_animesh_whose_owner_id_is_not_sent_is_ours_by_its_update_flags() {
    // the simulator sends OwnerID only with ObjectProperties: a rezzed
    // animesh not selected yet is known as ours by OBJECT_YOU_OWNER alone
    let (mut world, _) = fixture();
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::animesh_objects(demo::ANIMESH_MESH),
    });
    let (index, id) = world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == 9200)
        .map(|(i, o)| (i, o.full_id))
        .unwrap();
    assert!(world.objects.get(index).unwrap().owner_id.is_nil());
    assert_eq!(subjects::eligibility_reason(&world, id), None);
    world.objects.get_mut(index).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
    assert_eq!(subjects::eligibility_reason(&world, id), Some("not-owned"));
    world.objects.get_mut(index).unwrap().owner_id = Uuid::from_u128(77);
    world.objects.get_mut(index).unwrap().update_flags |= crate::ui::context::flags::OBJECT_YOU_OWNER;
    assert_eq!(
        subjects::eligibility_reason(&world, id),
        Some("not-owned"),
        "a known other owner wins"
    );
}
#[test]
fn poser_subjects_require_ownership_or_consent_and_losing_ownership_drops_only_that_layer() {
    let (mut world, mut editor) = fixture();
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: animesh_not_flagged_ours(),
    });
    let (index, id) = world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == 9200)
        .map(|(i, o)| (i, o.full_id))
        .unwrap();
    assert!(!subjects::allowed(&world, id));
    world.objects.get_mut(index).unwrap().owner_id = world.agent_id;
    assert!(
        !world.avatar_poses.contains_key(&id) && subjects::allowed(&world, id),
        "a still owned animesh, never posed by an animation yet, is offered"
    );
    world.avatar_poses.insert(id, world.avatar_poses[&world.agent_id].clone());
    assert!(subjects::allowed(&world, id));
    editor.pick_subject(&world, id);
    assert_eq!(editor.pose_layers(&world).len(), 2);
    let head = world.avatar_lib.rig.names["mHead"];
    Arc::make_mut(&mut editor.poser.sessions.get_mut(&id).unwrap().pose).rotations[head] = Quat::from_rotation_z(0.7);
    assert!(!editor.poser.sessions[&world.agent_id].edited(head));
    let stranger = world
        .objects
        .iter()
        .find(|(_, o)| o.is_avatar() && o.full_id != world.agent_id)
        .unwrap()
        .1
        .full_id;
    editor.pick_subject(&world, stranger);
    assert_eq!(editor.poser.subject, id);
    world.objects.get_mut(index).unwrap().owner_id = Uuid::nil();
    let layers = editor.pose_layers(&world);
    assert_eq!(layers.len(), 1);
    assert!(layers.contains_key(&world.agent_id));
    assert_eq!(editor.poser.subject, world.agent_id);
}
#[test]
fn poser_animesh_menu_has_a_name_instead_of_a_local_id_when_properties_are_missing() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (mut world, mut editor) = fixture();
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: animesh_not_flagged_ours(),
    });
    let (index, id) = world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == 9200)
        .map(|(i, o)| (i, o.full_id))
        .unwrap();
    world.objects.get_mut(index).unwrap().owner_id = world.agent_id;
    world.avatar_poses.insert(id, world.avatar_poses[&world.agent_id].clone());
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    let frame = editor.editor_frame(&world).unwrap();
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    for _ in 0..3 {
        frames.run(Vec::new(), &mut draw);
    }
    frames
        .ctx
        .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = frames.run(Vec::new(), &mut draw);
    }
    assert!(painted(&texts, aurora_i18n::tr!("poser-animesh")).rect.is_positive());
}
/// An owned demo animesh (local id 9200) and its index and id; posed
/// like you when `posed`, else as one whose rigged mesh is still loading.
fn owned_animesh(world: &mut World, posed: bool) -> (usize, Uuid) {
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: animesh_not_flagged_ours(),
    });
    let (index, id) = world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == 9200)
        .map(|(i, o)| (i, o.full_id))
        .unwrap();
    world.objects.get_mut(index).unwrap().owner_id = world.agent_id;
    if posed {
        world.avatar_poses.insert(id, world.avatar_poses[&world.agent_id].clone());
    }
    (index, id)
}
/// Draws the poser window with its subject menu open, and the texts painted.
fn draw_subject_menu(editor: &mut Editor, world: &World) -> Vec<crate::ui::widgets::elide_tests::Text> {
    use crate::ui::widgets::elide_tests::Frames;
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    let frame = editor.editor_frame(world).unwrap();
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, world, &frame);
    for _ in 0..3 {
        frames.run(Vec::new(), &mut draw);
    }
    frames
        .ctx
        .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = frames.run(Vec::new(), &mut draw);
    }
    texts
}
#[test]
fn an_owned_animesh_with_no_pose_yet_is_neither_offered_nor_picked() {
    // its rigged mesh still loading: no skeleton pose, so no frame, and
    // the poser window would vanish on it
    let (mut world, mut editor) = fixture();
    let (_, id) = owned_animesh(&mut world, false);
    assert!(
        subjects::allowed(&world, id),
        "it stays allowed: a session is not dropped while its mesh reloads"
    );
    let texts = draw_subject_menu(&mut editor, &world);
    assert!(
        !texts.iter().any(|t| t.source == aurora_i18n::tr!("poser-animesh")),
        "not in the subject menu yet"
    );
    assert!(editor.take_poser_requests().is_empty(), "nor its name asked for");
    editor.pick_subject(&world, id);
    assert_eq!(editor.poser.subject, world.agent_id, "a pick without a frame is refused");
    assert!(editor.editor_frame(&world).is_some(), "the poser keeps a frame to draw");
    world.avatar_poses.insert(id, world.avatar_poses[&world.agent_id].clone());
    let texts = draw_subject_menu(&mut editor, &world);
    assert!(
        texts.iter().any(|t| t.source == aurora_i18n::tr!("poser-animesh")),
        "offered once posed"
    );
    editor.pick_subject(&world, id);
    assert_eq!(editor.poser.subject, id);
    assert!(editor.editor_frame(&world).is_some());
}
#[test]
fn the_subject_name_cache_keeps_only_requested_names_of_objects_still_there() {
    let (mut world, mut editor) = fixture();
    let (index, id) = owned_animesh(&mut world, true);
    let props = |object_id: Uuid, name: &str| aurora_net::build::ObjectProps {
        object_id,
        name: name.to_owned(),
        ..Default::default()
    };
    // hover tips, area search, the build selection: not asked by the poser
    let other = Uuid::from_u128(0x5eed);
    editor.update_poser_subject_names(&[props(other, "A chair"), props(id, "Early name")]);
    assert!(editor.poser.subject_names.is_empty(), "names nobody asked for are not kept");
    draw_subject_menu(&mut editor, &world);
    assert_eq!(editor.take_poser_requests(), vec![Request::ObjectName(id)]);
    editor.update_poser_subject_names(&[props(other, "A chair"), props(id, "My Animesh Subject")]);
    assert_eq!(editor.poser.subject_names.len(), 1);
    assert_eq!(editor.poser.subject_names[&id], "My Animesh Subject");
    let local = world.objects.get(index).unwrap().key.local_id;
    world.apply(NetEvent::ObjectsKilled {
        handle: 1,
        local_ids: vec![local],
    });
    assert!(world.objects.index_of_uuid(&id).is_none());
    draw_subject_menu(&mut editor, &world);
    assert!(editor.poser.subject_names.is_empty(), "the name of an object gone is dropped");
}
#[test]
fn a_pick_in_the_world_asks_for_the_animesh_name_without_the_menu() {
    // Ctrl + click on a rezzed animesh: the compact window's head and the
    // tip name it without the subject menu ever being opened
    let (mut world, mut editor) = fixture();
    let (_, id) = owned_animesh(&mut world, true);
    editor.pick_subject(&world, id);
    assert_eq!(editor.poser.subject, id);
    assert_eq!(editor.take_poser_requests(), vec![Request::ObjectName(id)]);
    editor.pick_subject(&world, world.agent_id);
    editor.pick_subject(&world, id);
    assert!(editor.take_poser_requests().is_empty(), "asked once every 30 seconds");
    editor.update_poser_subject_names(&[aurora_net::build::ObjectProps {
        object_id: id,
        name: "Picked Animesh".to_owned(),
        ..Default::default()
    }]);
    assert_eq!(
        subjects::subject_short_name(&world, &editor.poser.subject_names, id),
        "Picked Animesh"
    );
    editor.pick_subject(&world, world.agent_id);
    editor.pick_subject(&world, id);
    assert!(editor.take_poser_requests().is_empty(), "a known name is not asked again");
}
#[test]
fn an_animesh_row_shows_its_distance() {
    // several animesh not named yet all read "Animesh": the distance tells
    // them apart, as on the nearby residents' rows
    let (mut world, mut editor) = fixture();
    let (index, _) = owned_animesh(&mut world, true);
    let at = |i: usize| Scene::object_transform(&world, i, Instant::now(), 0).unwrap().0;
    let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
    let distance = at(index).distance(at(own));
    let away = aurora_i18n::tr!("poser-distance", distance = aurora_i18n::number(f64::from(distance), 1)).to_string();
    let texts = draw_subject_menu(&mut editor, &world);
    assert!(texts.iter().any(|t| t.source == away), "{away} on the animesh row");
}
#[test]
fn a_renamed_animesh_takes_its_new_name_from_any_incoming_properties() {
    let (mut world, mut editor) = fixture();
    let (_, id) = owned_animesh(&mut world, true);
    let props = |object_id: Uuid, name: &str| aurora_net::build::ObjectProps {
        object_id,
        name: name.to_owned(),
        ..Default::default()
    };
    editor.pick_subject(&world, id);
    assert_eq!(editor.take_poser_requests(), vec![Request::ObjectName(id)]);
    editor.update_poser_subject_names(&[props(id, "Old Name")]);
    // the build window's properties after a rename, not asked by the poser
    let other = Uuid::from_u128(0x5eed);
    editor.update_poser_subject_names(&[props(other, "A chair"), props(id, "New Name")]);
    assert_eq!(subjects::subject_name(&world, &editor.poser.subject_names, id), "New Name");
    assert!(!editor.poser.subject_names.contains_key(&other), "other objects are still not kept");
    editor.update_poser_subject_names(&[props(id, "  ")]);
    assert_eq!(editor.poser.subject_names[&id], "New Name", "an empty name changes nothing");
}
#[test]
fn a_group_owned_animesh_flagged_yours_stays_offered_once_its_owner_is_known() {
    let (mut world, mut editor) = fixture();
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: demo::animesh_objects(demo::ANIMESH_MESH),
    });
    let (index, id) = world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == 9200)
        .map(|(i, o)| (i, o.full_id))
        .unwrap();
    world.avatar_poses.insert(id, world.avatar_poses[&world.agent_id].clone());
    let group = Uuid::from_u128(0x6409);
    world.groups.groups.push(aurora_net::GroupMembership {
        id: group,
        name: "Builders".to_owned(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    assert_eq!(subjects::eligibility_reason(&world, id), None, "flagged yours, owner not sent yet");
    // the simulator then names its group as owner (deeded): the flag is
    // still trusted, so the row does not appear and then disappear
    world.objects.get_mut(index).unwrap().owner_id = group;
    assert_eq!(subjects::eligibility_reason(&world, id), None);
    editor.pick_subject(&world, id);
    assert_eq!(editor.poser.subject, id);
    world.objects.get_mut(index).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
    assert_eq!(
        subjects::eligibility_reason(&world, id),
        Some("not-owned"),
        "a group's animesh you may not manage carries no flag"
    );
    world.objects.get_mut(index).unwrap().update_flags |= crate::ui::context::flags::OBJECT_YOU_OWNER;
    world.objects.get_mut(index).unwrap().owner_id = Uuid::from_u128(0x6410);
    assert_eq!(
        subjects::eligibility_reason(&world, id),
        Some("not-owned"),
        "an owner neither you nor one of your groups still wins"
    );
}
#[test]
fn poser_animesh_subject_uses_object_name_in_menu_and_requests_missing_properties_once() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (mut world, mut editor) = fixture();
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: animesh_not_flagged_ours(),
    });
    let (index, id) = world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == 9200)
        .map(|(i, o)| (i, o.full_id))
        .unwrap();
    world.objects.get_mut(index).unwrap().owner_id = world.agent_id;
    world.avatar_poses.insert(id, world.avatar_poses[&world.agent_id].clone());
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    let frame = editor.editor_frame(&world).unwrap();
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    for _ in 0..3 {
        frames.run(Vec::new(), &mut draw);
    }
    frames
        .ctx
        .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = frames.run(Vec::new(), &mut draw);
    }
    assert!(painted(&texts, aurora_i18n::tr!("poser-animesh")).rect.is_positive());
    drop(draw);
    assert_eq!(editor.take_poser_requests(), vec![Request::ObjectName(id)]);
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    for _ in 0..3 {
        texts = frames.run(Vec::new(), &mut draw);
    }
    drop(draw);
    assert!(
        editor.take_poser_requests().is_empty(),
        "the outstanding property request is rate-limited"
    );
    let props = aurora_net::build::ObjectProps {
        object_id: id,
        name: "My Animesh Subject".to_owned(),
        ..Default::default()
    };
    editor.update_poser_subject_names(&[props]);
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    for _ in 0..3 {
        texts = frames.run(Vec::new(), &mut draw);
    }
    assert!(painted(&texts, "My Animesh Subject").rect.is_positive());
    assert!(
        !texts
            .iter()
            .any(|t| t.source == format!("Animesh {}", world.objects.get(index).unwrap().key.local_id))
    );
    assert!(
        editor.take_poser_requests().is_empty(),
        "the property request is rate-limited while the response is pending"
    );
}
fn click(pos: egui::Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}
#[test]
fn poser_hand_tab_and_preset_apply_through_actual_pointer_events() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (world, mut editor) = fixture();
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    let frame = editor.editor_frame(&world).unwrap();
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = frames.run(Vec::new(), &mut draw);
    }
    let hands = painted(&texts, aurora_i18n::text("poser-hands")).rect.center();
    frames.run(click(hands), &mut draw);
    texts = frames.run(Vec::new(), &mut draw);
    // the left side is the default one: the preset reaches the left hand only
    let relaxed = painted(&texts, aurora_i18n::text("poser-hand-relaxed")).rect.center();
    frames.run(click(relaxed), &mut draw);
    assert_eq!(editor.poser.group, 2);
    let rig = &world.avatar_lib.rig;
    assert!(editor.poser.sessions[&world.agent_id].edited(rig.names["mHandIndex1Left"]));
    assert!(!editor.poser.sessions[&world.agent_id].edited(rig.names["mHandIndex1Right"]));
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    let texts = frames.run(Vec::new(), &mut draw);
    let right = painted(&texts, aurora_i18n::text("poser-side-right")).rect.center();
    frames.run(click(right), &mut draw);
    let texts = frames.run(Vec::new(), &mut draw);
    let fist = painted(&texts, aurora_i18n::text("poser-hand-fist")).rect.center();
    frames.run(click(fist), &mut draw);
    assert_eq!(editor.poser.side, Side::Right);
    assert!(editor.poser.sessions[&world.agent_id].edited(rig.names["mHandIndex1Right"]));
    // Mirror: one preset on both hands, one undo step
    editor.poser.side = Side::Mirror;
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    session.history(false);
    session.history(false);
    assert!(!session.edited(rig.names["mHandIndex1Left"]));
    let steps = session.undo.len();
    editor.poser.preset = 2;
    editor.hand_preset(rig, None);
    let session = &editor.poser.sessions[&world.agent_id];
    assert_eq!(session.undo.len(), steps + 1);
    assert!(session.edited(rig.names["mHandIndex1Left"]) && session.edited(rig.names["mHandIndex1Right"]));
}
#[test]
fn the_poser_frames_only_on_an_explicit_click_select() {
    use crate::camera::PoserFraming;
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let frame = Frame::new(&world).unwrap();
    let (head, neck) = (rig.names["mHead"], rig.names["mNeck"]);
    let (left, right) = (rig.names["mElbowLeft"], rig.names["mElbowRight"]);
    // a selection the code makes (the first pick, the Avatar position row)
    editor.select_pose_bone(left, &rig, &frame);
    assert_eq!(editor.take_poser_focus_request(), None);
    // a click that selects another joint asks once
    editor.click_pose_bone(head, &rig, &frame, false);
    assert_eq!(editor.take_poser_focus_request(), Some((head, PoserFraming::Joint)));
    editor.click_pose_bone(head, &rig, &frame, false);
    assert_eq!(editor.take_poser_focus_request(), None, "the joint already selected");
    // Shift + click adds a follower: the lead and the camera stay
    editor.click_pose_bone(neck, &rig, &frame, true);
    assert_eq!(editor.take_poser_focus_request(), None);
    // a drag begun on another joint selects it, the camera stays
    editor.grab_pose_bone(left, &rig, &frame);
    assert_eq!(editor.selected, Some(left));
    assert_eq!(editor.take_poser_focus_request(), None, "never under a drag");
    // a side switch
    editor.set_pose_side(Side::Right, &rig, &frame);
    assert_eq!(editor.selected, Some(right));
    assert_eq!(editor.take_poser_focus_request(), None, "never on a side switch");
    editor.set_pose_side(Side::Mirror, &rig, &frame);
    assert_eq!(editor.take_poser_focus_request(), None, "never on Mirror");
    // a tab change that picks the tab's first joint
    editor.select_pose_group(1, &rig, &frame);
    assert_ne!(editor.selected, Some(right));
    assert_eq!(editor.take_poser_focus_request(), None, "never on a tab change");
    // in the photo viewfinder, the shot is the user's
    editor.over_viewfinder = true;
    let chest = rig.names["mChest"];
    editor.click_pose_bone(chest, &rig, &frame, false);
    assert_eq!(editor.selected, Some(chest));
    assert_eq!(editor.take_poser_focus_request(), None, "never in the viewfinder");
}
#[test]
fn poser_side_selector_chooses_the_edited_bone_and_mirror_is_the_symmetry() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let frame = Frame::new(&world).unwrap();
    let left = rig.names["mElbowLeft"];
    let right = rig.names["mElbowRight"];
    editor.select_pose_bone(left, &rig, &frame);
    editor.set_pose_side(Side::Right, &rig, &frame);
    assert_eq!(editor.selected, Some(right), "the selected pair moves to the chosen side");
    assert!(!editor.symmetry);
    editor.set_pose_side(Side::Left, &rig, &frame);
    assert_eq!(editor.selected, Some(left));
    // a marker picked in the world shows its side in the selector
    editor.select_pose_bone(right, &rig, &frame);
    editor.follow_selected_side(&rig);
    assert_eq!(editor.poser.side, Side::Right);
    // Mirror keeps the selection and edits both sides
    editor.select_pose_bone(left, &rig, &frame);
    editor.set_pose_side(Side::Mirror, &rig, &frame);
    assert_eq!(editor.selected, Some(left));
    assert!(editor.symmetry);
    editor.follow_selected_side(&rig);
    assert_eq!(editor.poser.side, Side::Mirror, "Mirror stays as chosen");
    let base = editor.poser.sessions[&world.agent_id].pose.clone();
    editor.poser_transform(&rig, &frame, base.positions[left], Quat::from_rotation_z(0.5), true);
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(session.edited(left) && session.edited(right), "Mirror moves the counterpart too");
    // Left alone: the counterpart stays
    let (world, mut editor) = fixture();
    let frame = Frame::new(&world).unwrap();
    editor.select_pose_bone(left, &rig, &frame);
    editor.set_pose_side(Side::Left, &rig, &frame);
    let base = editor.poser.sessions[&world.agent_id].pose.clone();
    editor.poser_transform(&rig, &frame, base.positions[left], Quat::from_rotation_z(0.5), true);
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(session.edited(left) && !session.edited(right));
}
#[test]
fn poser_finger_grid_maps_each_cell_to_the_chosen_side() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    for (stem, _) in bones::FINGERS {
        for joint in 1..=3 {
            for (side, suffix) in [(Side::Left, "Left"), (Side::Right, "Right"), (Side::Mirror, "Left")] {
                let bone = bones::finger_bone(rig, stem, joint, side).unwrap();
                assert_eq!(rig.canonical_names[bone], format!("{stem}{joint}{suffix}"));
                assert_eq!(group(&rig.canonical_names[bone]), 2, "every cell is a Hands joint");
            }
        }
    }
}
#[test]
fn poser_whole_pose_actions_work_from_their_new_group() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (world, mut editor) = selected_fixture();
    let rig = world.avatar_lib.rig.clone();
    let frame = editor.editor_frame(&world).unwrap();
    let base = editor.poser.sessions[&world.agent_id].pose.clone();
    editor.poser_transform(&rig, &frame, base.positions[rig.names["mHead"]], Quat::from_rotation_z(0.4), true);
    assert!(editor.poser.sessions[&world.agent_id].edited(rig.names["mHead"]));
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = frames.run(Vec::new(), &mut draw);
    }
    let reset = painted(&texts, aurora_i18n::text("ik-edit-reset-all")).rect.center();
    frames.run(click(reset), &mut draw);
    assert!(
        (0..rig.len()).all(|b| !editor.poser.sessions[&world.agent_id].edited(b)),
        "Reset all is in the Whole pose group"
    );
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    let texts = frames.run(Vec::new(), &mut draw);
    let t_pose = painted(&texts, aurora_i18n::text("poser-t-pose")).rect.center();
    frames.run(click(t_pose), &mut draw);
    assert_eq!(editor.poser.sessions[&world.agent_id].pose.rotations, rig.local_rot);
    // the settings no longer hold Reset all, and open over the joint
    editor.poser.settings_open = true;
    let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
    let texts = frames.run(Vec::new(), &mut draw);
    assert!(
        texts
            .iter()
            .any(|t| t.source == aurora_i18n::text("poser-settings-files").to_uppercase())
    );
    assert_eq!(
        texts.iter().filter(|t| t.source == aurora_i18n::text("ik-edit-reset-all")).count(),
        1
    );
}
#[test]
fn poser_subject_menu_queues_consent_actions_for_the_app_and_waits_for_the_bridge() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (world, mut editor) = fixture();
    let peer = demo::poser::PEER;
    assert!(
        crate::world::poser_consent::nearby(&world, peer),
        "the fixture's peer stands nearby"
    );
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    let frame = editor.editor_frame(&world).unwrap();
    use crate::world::bridge::QueryState;
    for (bridge, hint) in [
        (QueryState::Starting, Some("poser-bridge-starting")),
        (QueryState::Silent, Some("poser-bridge-silent")),
        (QueryState::Failed, Some("poser-bridge-failed")),
        (QueryState::Detached, Some("poser-bridge-needed")),
        (QueryState::Off, Some("poser-bridge-disabled")),
        (QueryState::Ready, None),
    ] {
        editor.poser.requests.clear();
        editor.set_poser_bridge(bridge);
        let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
        for _ in 0..3 {
            frames.run(Vec::new(), &mut draw);
        }
        frames
            .ctx
            .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = frames.run(Vec::new(), &mut draw);
        }
        for key in [
            "poser-bridge-starting",
            "poser-bridge-silent",
            "poser-bridge-failed",
            "poser-bridge-needed",
            "poser-bridge-disabled",
        ] {
            assert_eq!(
                texts.iter().any(|t| t.source == aurora_i18n::text(key)),
                hint == Some(key),
                "{bridge:?}: the menu says why Ask waits ({key})"
            );
        }
        let repair = texts
            .iter()
            .find(|t| t.source == aurora_i18n::text("bridge-repair"))
            .map(|t| t.rect.center());
        assert_eq!(
            repair.is_some(),
            matches!(bridge, QueryState::Silent | QueryState::Failed),
            "{bridge:?}: Repair only when a worn bridge does not answer"
        );
        let ask = painted(&texts, aurora_i18n::text("poser-consent-ask")).rect.center();
        frames.run(click(ask), &mut draw);
        let requests = editor.take_poser_requests();
        if bridge == QueryState::Ready {
            assert_eq!(requests, vec![Request::Ask(peer)]);
        } else {
            assert!(requests.is_empty(), "{bridge:?}: no request without the bridge");
        }
        if let Some(repair) = repair {
            let mut draw = |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame);
            frames
                .ctx
                .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
            for _ in 0..3 {
                frames.run(Vec::new(), &mut draw);
            }
            frames.run(click(repair), &mut draw);
            assert_eq!(editor.take_poser_requests(), vec![Request::RepairBridge]);
        }
    }
}
#[test]
fn poser_window_stays_steady_at_scaled_fonts_and_keeps_the_upload_footer_visible() {
    for library in [false, true] {
        for settings in [false, true] {
            crate::ui::widgets::steady_tests::assert_window_steady(
                "poser",
                &[egui::vec2(880.0, 760.0), egui::vec2(720.0, 480.0)],
                aurora_i18n::text("poser-upload"),
                || {
                    let (world, mut editor) = selected_fixture();
                    editor.poser.library_open = library;
                    editor.poser.settings_open = settings;
                    editor.poser.storage.message = Some(
                        aurora_i18n::tr!(
                            "poser-saved-to",
                            path = format!(
                                "/a-long-local-profile-directory/{}/Portrait.aurora-pose.json",
                                "nested-directory/".repeat(12)
                            )
                        )
                        .to_string(),
                    );
                    (world, editor)
                },
                |(world, editor), ctx, p| {
                    let frame = editor.editor_frame(world).unwrap();
                    editor.poser_ui(ctx, p, world, &frame);
                },
            );
        }
    }
}
#[test]
fn poser_bone_rows_and_xyz_controls_are_painted_inside_their_scroll_clips() {
    fn visible(shape: &egui::Shape, clip: egui::Rect, source: &str) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| visible(s, clip, source)),
            egui::Shape::Text(text) => {
                text.galley.text() == source
                    && clip
                        .expand(1.0)
                        .contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => false,
        }
    }
    for (library, hands, font) in [
        (false, false, 1.0),
        (true, false, 1.0),
        (false, true, 1.0),
        (true, true, 1.0),
        (false, true, 1.5),
    ] {
        let (world, mut editor) = selected_fixture();
        editor.poser.library_open = library;
        if hands {
            editor.poser.group = 2;
            let frame = Frame::new(&world).unwrap();
            let rig = &world.avatar_lib.rig;
            editor.select(rig.names["mHandIndex1Right"], rig, &frame);
        }
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, font);
        let p = theme.palette();
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        widgets::place_floater(&ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
        let frame = Frame::new(&world).unwrap();
        let mut shapes = Vec::new();
        for tick in 0..20 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))),
                    time: Some(tick as f64 / 60.0),
                    ..Default::default()
                },
                |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame),
            );
            output.textures_delta.clear();
            shapes = output.shapes;
        }
        let mut labels = if font == 1.0 {
            vec!["X", "Y", "Z", aurora_i18n::text("poser-delta")]
        } else {
            Vec::new()
        };
        if library {
            labels.push(aurora_i18n::text("poser-library"));
        } else if hands {
            labels.extend([
                aurora_i18n::text("poser-hand-relaxed"),
                aurora_i18n::text("poser-side-right"),
                aurora_i18n::text("poser-finger-index"),
            ]);
        } else {
            labels.push(aurora_i18n::text("poser-bone-chest"));
        }
        for label in labels {
            assert!(
                shapes.iter().any(|s| visible(&s.shape, s.clip_rect, label)),
                "library {library}, hands {hands}, font {font}: {label} is visible inside the body clip"
            );
        }
    }
}
#[test]
fn poser_name_can_be_replaced_and_survives_idle_frames() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (world, mut editor) = selected_fixture();
    let frame = editor.editor_frame(&world).unwrap();
    let p = crate::theme::Theme::default().palette();
    let mut frames = Frames::new();
    let mut texts = Vec::new();
    for _ in 0..5 {
        texts = frames.run(Vec::new(), |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame));
    }
    let pointer = painted(&texts, &editor.poser.name).rect.center();
    for pressed in [true, false] {
        frames.run(
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::PointerButton {
                    pos: pointer,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame),
        );
    }
    let modifiers = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    frames.run(
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }],
        |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame),
    );
    frames.run(
        vec![
            egui::Event::Text("Portrait".into()),
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            }),
        ],
        |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame),
    );
    assert_eq!(editor.poser.name, "Portrait");
    for _ in 0..30 {
        frames.run(Vec::new(), |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame));
        assert_eq!(editor.poser.name, "Portrait");
    }
}
#[test]
fn poser_load_menu_opens_the_existing_library_without_a_file_picker() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (world, mut editor) = selected_fixture();
    let frame = editor.editor_frame(&world).unwrap();
    let p = crate::theme::Theme::default().palette();
    editor.poser.storage.ready = true;
    let mut frames = Frames::new();
    let mut texts = Vec::new();
    for _ in 0..5 {
        texts = frames.run(Vec::new(), |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame));
    }
    for label in ["poser-load-menu", "poser-library"] {
        let pointer = painted(&texts, aurora_i18n::text(label)).rect.center();
        for pressed in [true, false] {
            frames.run(
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::PointerButton {
                        pos: pointer,
                        pressed,
                        button: egui::PointerButton::Primary,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame),
            );
        }
        texts = frames.run(Vec::new(), |ui| editor.poser_ui(ui.ctx(), &p, &world, &frame));
    }
    assert!(editor.poser.library_open);
    assert!(!editor.poser.storage.ready);
    assert!(
        editor.poser.storage.job.is_none(),
        "opening the library must not launch a file picker"
    );
}
#[test]
fn poser_library_saves_bvh_confirms_overwrite_and_correlates_loads_to_the_original_subject() {
    let (world, editor) = fixture();
    let owner = Uuid::new_v4();
    let subject = Uuid::new_v4();
    let root = crate::settings::account_dir(&owner);
    assert!(root.starts_with(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/unit-test-user-data")));
    let rig = world.avatar_lib.rig.clone();
    let session = &editor.poser.sessions[&world.agent_id];
    let document = document::Document::capture(&rig, session, "Portrait".into(), 4);
    let (jobs, _) = Jobs::new();
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let ctx = egui::Context::default();
    let mut service = storage::Service::default();
    let tick = |service: &mut storage::Service| service.tick(&jobs, &ctx, owner, rig.clone(), world.avatar_lib.skeleton.as_ref());
    service.filter = "does not match Portrait".into();
    service.job = Some(storage::Job::Save(storage::Save {
        document: document.clone(),
        pose: session.pose.clone(),
        bvh: true,
        unlock: true,
        confirm: false,
        pick: false,
    }));
    for _ in 0..3 {
        tick(&mut service);
    }
    assert_eq!(service.entries.len(), 1);
    let path = service.entries[0].path.clone();
    assert_eq!(service.saved_path, None);
    assert_eq!(service.selected, Some(0));
    assert!(service.filter.is_empty(), "a search that hides the saved pose is cleared");
    assert!(service.scroll_to_selected, "the list scrolls to the saved pose once");
    assert!(service.message.as_ref().unwrap().contains(path.to_string_lossy().as_ref()));

    // A search the saved pose matches stays.
    service.filter = "PORT".into();
    service.scroll_to_selected = false;
    service.job = Some(storage::Job::Save(storage::Save {
        document: document.clone(),
        pose: session.pose.clone(),
        bvh: true,
        unlock: true,
        confirm: false,
        pick: false,
    }));
    for _ in 0..3 {
        tick(&mut service);
    }
    if let Some((document, pose, bvh, unlock)) = service.overwrite.take() {
        // saving the same name again asks to overwrite: confirm it
        service.job = Some(storage::Job::Save(storage::Save {
            document,
            pose,
            bvh,
            unlock,
            confirm: true,
            pick: false,
        }));
        for _ in 0..3 {
            tick(&mut service);
        }
    }
    assert_eq!(service.selected, Some(0));
    assert_eq!(service.filter, "PORT", "a search the saved pose matches is kept");
    assert!(service.scroll_to_selected);
    service.scroll_to_selected = false;

    // A user can select an existing row while the post-save refresh is pending.
    service.saved_path = Some(path.clone());
    service.ready = false;
    for _ in 0..3 {
        tick(&mut service);
    }
    assert_eq!(service.selected, Some(0));
    assert_eq!(service.saved_path, None, "a current selection must consume the pending saved path");

    service.selected = None;
    service.ready = false;
    for _ in 0..3 {
        tick(&mut service);
    }
    assert_eq!(service.selected, None, "a later library refresh must not reuse the saved path");
    let bvh = std::fs::read(root.join("poses/Portrait.bvh")).unwrap();
    let imported = aurora_assets::import_animation::parse_bvh_pose(&bvh, world.avatar_lib.skeleton.as_ref().unwrap()).unwrap();
    assert!(
        imported
            .animation
            .joints
            .iter()
            .any(|j| j.joint_name == "mHead" && !j.rot_keys.is_empty())
    );
    service.job = Some(storage::Job::Save(storage::Save {
        document,
        pose: session.pose.clone(),
        bvh: false,
        unlock: false,
        confirm: true,
        pick: false,
    }));
    tick(&mut service);
    tick(&mut service);
    assert!(service.overwrite.is_some());
    service.overwrite = None;
    service.job = Some(storage::Job::Load(Some(path), document::LoadMode::All, subject));
    tick(&mut service);
    let loaded = tick(&mut service);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].0, subject);
    assert!(matches!(loaded[0].1, storage::Loaded::Pose(_, document::LoadMode::All)));
    assert!(root.file_name().is_some_and(|n| n == owner.to_string().as_str()));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn poser_handles_own_the_pointer_and_leave_the_rest_of_the_scene_to_huds() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let at = |name: &str| proj.project(frame.joints[rig.names[name]].w_axis.truncate()).unwrap().0;
    let mut frames = Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser-compact");
    let p = crate::theme::Theme::default().palette();
    let mut hover = |editor: &mut Editor, pointer: egui::Pos2| {
        for _ in 0..3 {
            frames.run(vec![egui::Event::PointerMoved(pointer)], |ui| {
                editor.show(ui.ctx(), &p, &world, &proj)
            });
        }
        editor.poser_owns_pointer()
    };
    // mHead is selected: its gizmo is away from the leg.
    assert!(hover(&mut editor, at("mKneeLeft")), "a joint marker takes the press");
    let shaft = at("mKneeLeft").lerp(at("mAnkleLeft"), 0.5);
    assert!(hover(&mut editor, shaft), "a bone shaft takes the press");
    assert!(
        !hover(&mut editor, egui::pos2(790.0, 890.0)),
        "empty scene: a HUD there gets the press"
    );
    editor.poser.markers = false;
    assert!(!hover(&mut editor, at("mKneeLeft")), "hidden markers own nothing");
}

#[test]
fn poser_poses_a_shape_whose_sliders_mirror_face_joints() {
    // #1249: shape sliders at their ends give eyelids, nose, forehead and
    // pectorals a negative scale on one axis; the poser refused the whole
    // pose of such an avatar, which then neither froze nor moved.
    let (mut world, mut editor) = fixture();
    let lib = world.avatar_lib.clone();
    let rig = &lib.rig;
    let shape = lib.shape_params.shape(&[0; 260], rig);
    let mirrored: Vec<usize> = (0..rig.len()).filter(|b| shape.scale[*b].min_element() < 0.0).collect();
    assert!(!mirrored.is_empty(), "the fixture shape mirrors no joint");
    let joints = rig.world_with(&rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
    world.avatar_poses.insert(
        world.agent_id,
        joints.iter().zip(&rig.default_world_inv).map(|(p, i)| *p * *i).collect(),
    );
    editor.poser.sessions.clear();
    let layers = editor.pose_layers(&world);
    let pose = layers.get(&world.agent_id).expect("the mirrored shape is posed");
    // the shape's own axis keeps the sign, so the rest rotation stays the
    // rest rotation and the T-pose or a preset does not turn the joint over
    for &bone in &mirrored {
        assert!(
            pose.scales[bone].abs_diff_eq(shape.scale[bone], 1e-4),
            "scale of {}",
            rig.canonical_names[bone]
        );
        assert!(
            pose.rotations[bone].dot(rig.local_rot[bone]).abs() > 0.9999,
            "rotation of {}",
            rig.canonical_names[bone]
        );
    }
    // the captured channels rebuild the shaped skeleton (skeleton space)
    let model = Frame::new(&world).expect("frame").model;
    for (bone, (posed, shaped)) in pose.matrices(rig, model).iter().zip(&joints).enumerate() {
        assert!(shaped.abs_diff_eq(*posed, 1e-4), "{} moved", rig.canonical_names[bone]);
    }
    // an edit reaches the layer, and a scale edit keeps the mirrored axis
    let frame = Frame::new(&world).expect("frame");
    editor.select(rig.names["mHead"], rig, &frame);
    editor.set_rotation(Quat::from_rotation_z(0.4));
    assert!(editor.commit_poser_manipulation(rig, &frame, true));
    assert!(
        editor.pose_layers(&world)[&world.agent_id].rotations[rig.names["mHead"]]
            .dot(pose.rotations[rig.names["mHead"]])
            .abs()
            < 0.999
    );
    editor.set_pose_scale(rig, mirrored[0], shape.scale[mirrored[0]] * 2.0);
    let scaled = editor.pose_layers(&world)[&world.agent_id].scales[mirrored[0]];
    assert!(
        scaled.abs_diff_eq(data::bounded_scale(shape.scale[mirrored[0]] * 2.0), 1e-5),
        "{scaled:?}"
    );
    assert_eq!(data::bounded_scale(Vec3::new(-20.0, 0.0, -1e-6)), Vec3::new(-10.0, 0.001, -0.001));
}

#[test]
fn poser_poses_and_exports_a_shape_that_leaves_a_joint_almost_flat() {
    // A slider crossing zero left both pectorals at this scale on a real
    // avatar: the capture took it, the check behind 0.001 refused the pose,
    // and neither edits nor Save and Upload did anything.
    let (mut world, mut editor) = fixture();
    let lib = world.avatar_lib.clone();
    let rig = &lib.rig;
    let mut shape = lib.shape_params.shape(&[0; 260], rig);
    for name in ["LEFT_PEC", "RIGHT_PEC"] {
        shape.scale[rig.names[name]].x = -0.00019;
    }
    let joints = rig.world_with(&rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
    world.avatar_poses.insert(
        world.agent_id,
        joints.iter().zip(&rig.default_world_inv).map(|(p, i)| *p * *i).collect(),
    );
    editor.poser.sessions.clear();
    assert!(editor.pose_layers(&world).contains_key(&world.agent_id), "the avatar is posed");
    let frame = Frame::new(&world).expect("frame");
    editor.select(rig.names["mHead"], rig, &frame);
    editor.set_rotation(Quat::from_rotation_z(0.4));
    assert!(editor.commit_poser_manipulation(rig, &frame, true));
    let session = &editor.poser.sessions[&world.agent_id];
    let authored = session.authored(rig, frame.model).expect("the export has a pose");
    let moved = authored.rotations[rig.names["mHead"]].dot(rig.local_rot[rig.names["mHead"]]).abs();
    assert!(moved < 0.999, "the edit is exported");
    assert!(storage::native(rig, &authored, "Flat".into(), 4).is_ok(), "the upload is prepared");
    assert!(
        document::Document::capture(rig, session, "Flat".into(), 4).valid(rig),
        "the pose can be saved"
    );
}

#[test]
fn poser_upload_keeps_both_subjects_and_prepares_the_next_without_restarting() {
    let (mut world, mut editor) = fixture();
    let peer = demo::poser::PEER;
    world.avatar_poses.insert(peer, world.avatar_poses[&world.agent_id].clone());
    let mut consent = std::mem::take(&mut world.poser_consent);
    consent.request(&world, peer, Instant::now());
    consent.demo_accept(&world, peer, std::time::Duration::ZERO);
    world.poser_consent = consent;
    let rig = world.avatar_lib.rig.clone();
    let head = rig.names["mHead"];
    let mut expected = Vec::new();
    for (subject, angle) in [(world.agent_id, 0.4), (peer, -0.6)] {
        editor.pick_subject(&world, subject);
        let session = editor.poser.sessions.get_mut(&subject).unwrap();
        session.checkpoint();
        Arc::make_mut(&mut session.pose).rotations[head] = Quat::from_rotation_z(angle) * rig.local_rot[head];
        expected.push((subject, session.pose.clone(), session.undo.len()));
    }
    let (jobs, _) = Jobs::new();
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let ctx = egui::Context::default();
    editor.poser.storage.ready = true;
    // The import preview may be cancelled, or stay open while the second
    // subject is prepared. Neither case should release either pose.
    for (subject, name) in [(world.agent_id, "Our pose"), (peer, "Their pose")] {
        editor.pick_subject(&world, subject);
        editor.poser.name = name.into();
        let frame = editor.editor_frame(&world).unwrap();
        editor.import_pose(&rig, &frame);
        editor.tick_pose_storage(&jobs, &ctx, &world);
        editor.tick_pose_storage(&jobs, &ctx, &world);
        let prepared = editor.take_clip_imports();
        assert_eq!(prepared.len(), 1, "the selected subject reaches the import preview");
        assert_eq!(prepared[0].name, name);
        assert!(!prepared[0].bytes.is_empty());
        assert!(editor.open && editor.poser.active, "the Poser stays open and active");
        assert!(!editor.animation_editor_return_pending(), "a pose export needs no Studio return");
        assert_eq!(editor.poser.subject, subject);
        let layers = editor.pose_layers(&world);
        assert_eq!(layers.len(), 2, "both avatars keep their poses");
        for (id, pose, undo_len) in &expected {
            assert!(Arc::ptr_eq(&layers[id], pose), "the displayed pose is unchanged");
            assert_eq!(editor.poser.sessions[id].undo.len(), *undo_len);
        }
        assert!(editor.take_clip_imports().is_empty(), "each prepared pose is handed over once");
        assert!(editor.open && editor.poser.active);
    }
}

/// The fixture on a pose it did not make: an animation plays on the avatar
/// when the poser captures it.
fn captured_animation() -> (World, Editor, usize, usize, Quat) {
    let (mut world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let (left, right) = (rig.names["mElbowLeft"], rig.names["mElbowRight"]);
    let played = Quat::from_euler(EulerRot::ZYX, 0.9, -0.4, 0.7);
    let mut rotations = rig.local_rot.clone();
    rotations[left] = played;
    rotations[rig.names["mHead"]] = Quat::from_rotation_x(0.8);
    let mut positions = rig.local_pos.clone();
    positions[rig.pelvis] += Vec3::new(0.1, -0.2, 0.3);
    world.avatar_poses.insert(
        world.agent_id,
        rig.world_with(&rotations, Vec3::ZERO, &positions, &rig.scale)
            .iter()
            .zip(&rig.default_world_inv)
            .map(|(p, i)| *p * *i)
            .collect(),
    );
    editor.poser.sessions.clear();
    editor.pose_layers(&world);
    (world, editor, left, right, played)
}
fn near(a: Quat, b: Quat) -> bool {
    a.dot(b).abs() > 0.9999
}
#[test]
fn poser_upload_exports_an_explicit_t_pose_in_full_and_undo_restores_sparse_export() {
    let (world, mut editor, left, _, _) = captured_animation();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let export = |session: &data::Session| storage::native(rig, &session.authored(rig, frame.model).unwrap(), "Pose".into(), 4);
    assert!(export(&editor.poser.sessions[&world.agent_id]).is_err());
    editor.pose_operation(rig, &frame, controls::Operation::TPose);
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    let expected = rig.canonical_names.iter().filter(|name| name.starts_with('m')).count();
    let neutral = export(session).expect("an explicit neutral pose has rotation tracks");
    assert_eq!(neutral.animation.joints.len(), expected);
    for track in &neutral.animation.joints {
        assert!(near(track.rot_keys[0].rotation, rig.local_rot[rig.names[&track.joint_name]]));
        if track.joint_name == "mPelvis" {
            assert!(track.pos_keys[0].position.abs_diff_eq(Vec3::ZERO, 0.0002));
        } else {
            assert!(track.pos_keys.is_empty());
        }
    }
    session.checkpoint();
    Arc::make_mut(&mut session.pose).rotations[left] = Quat::from_rotation_y(0.5);
    let built = export(session).unwrap();
    assert_eq!(built.animation.joints.len(), expected, "the other bones keep the T-pose");
    let elbow = built
        .animation
        .joints
        .iter()
        .find(|track| track.joint_name == "mElbowLeft")
        .unwrap();
    assert!(near(elbow.rot_keys[0].rotation, Quat::from_rotation_y(0.5)));
    session.history(false);
    session.history(false);
    assert!(export(session).is_err(), "undoing T-pose restores the captured baseline");
    session.history(true);
    assert_eq!(export(session).unwrap().animation.joints.len(), expected);
    editor.pose_operation(rig, &frame, controls::Operation::ResetAll);
    assert!(export(&editor.poser.sessions[&world.agent_id]).is_err());
}

#[test]
fn poser_saved_t_pose_round_trips_neutral_keys_and_released_joints() {
    let (world, mut editor, left, _, _) = captured_animation();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    editor.select(left, rig, &frame);
    editor.pose_operation(rig, &frame, controls::Operation::TPose);
    editor.pose_operation(rig, &frame, controls::Operation::ResetJoint);
    let session = &editor.poser.sessions[&world.agent_id];
    let document = document::Document::capture(rig, session, "T-pose".into(), 4);
    let saved = serde_json::to_vec(&document).unwrap();
    let reopened: document::Document = serde_json::from_slice(&saved).unwrap();
    let mut fresh = data::Session::new((*session.base).clone());
    assert!(reopened.apply(rig, &mut fresh, document::LoadMode::All, frame.model));
    let upload = storage::native(rig, &fresh.authored(rig, frame.model).unwrap(), "T-pose".into(), 4).unwrap();
    let names: std::collections::HashSet<_> = upload.animation.joints.iter().map(|j| j.joint_name.as_str()).collect();
    for (bone, name) in rig.canonical_names.iter().enumerate().filter(|(_, name)| name.starts_with('m')) {
        assert_eq!(names.contains(name.as_str()), bone != left, "{name}");
    }
}

#[test]
fn poser_exports_nothing_of_a_captured_animation_through_any_format() {
    let (world, editor, left, _, played) = captured_animation();
    let rig = &world.avatar_lib.rig;
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(near(session.pose.rotations[left], played), "the avatar keeps showing its animation");
    let authored = session.authored(rig, Mat4::IDENTITY).unwrap();
    for bone in 0..rig.len() {
        assert!(near(authored.rotations[bone], rig.local_rot[bone]), "{}", rig.canonical_names[bone]);
        assert!(authored.positions[bone].abs_diff_eq(rig.local_pos[bone], 1e-5));
    }
    assert!(
        storage::native(rig, &authored, "Pose".into(), 4).is_err(),
        "there is no authored channel to upload"
    );
    let bvh = bvh::encode(rig, &authored, true).unwrap();
    assert!(
        aurora_assets::import_animation::parse_bvh(bvh.as_bytes(), world.avatar_lib.skeleton.as_ref().unwrap()).is_err(),
        "nothing moves in the file"
    );
    let document = document::Document::capture(rig, session, "Pose".into(), 4);
    let text = serde_json::to_string(&document).unwrap();
    let decoded: document::Document = serde_json::from_str(&text).unwrap();
    let mut other = data::Session::new(SubjectPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
        scales: rig.scale.clone(),
        locks: Default::default(),
        origin: rig.local_rot.clone(),
        enabled: vec![true; rig.len()],
        hold_rotation: vec![false; rig.len()],
    });
    assert!(decoded.apply(rig, &mut other, document::LoadMode::All, Mat4::IDENTITY));
    assert!(
        (0..rig.len()).all(|b| near(other.pose.rotations[b], rig.local_rot[b])),
        "a saved file brings none of it to another avatar"
    );
    assert!(storage::native(rig, &other.authored(rig, Mat4::IDENTITY).unwrap(), "Pose".into(), 4).is_err());
}
#[test]
fn poser_touching_or_mirroring_joints_exports_the_change_and_not_the_captured_rotation() {
    let (world, mut editor, left, right, played) = captured_animation();
    let rig = world.avatar_lib.rig.clone();
    let frame = Frame::new(&world).unwrap();
    let change = Quat::from_rotation_z(0.2);
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    Arc::make_mut(&mut session.pose).rotations[left] = played * change;
    let authored = session.authored(&rig, frame.model).unwrap();
    assert!(near(authored.rotations[left], rig.local_rot[left] * change));
    editor.select(left, &rig, &frame);
    editor.pose_operation(&rig, &frame, controls::Operation::Mirror);
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(session.edited(right), "the right arm shows the mirrored pose");
    let authored = session.authored(&rig, frame.model).unwrap();
    let mirrored = data::mirrored(&rig, rig.local_rot[left] * change, left, right);
    assert!(near(authored.rotations[right], mirrored), "the mirror carries the change only");
    editor.pose_operation(&rig, &frame, controls::Operation::Flip);
    let authored = editor.poser.sessions[&world.agent_id].authored(&rig, frame.model).unwrap();
    assert!(near(authored.rotations[rig.names["mHead"]], rig.local_rot[rig.names["mHead"]]));
    assert!(near(authored.rotations[left], rig.local_rot[left] * change));
    editor.poser.preset = 1;
    editor.hand_preset(&rig, Some(true));
    let session = &editor.poser.sessions[&world.agent_id];
    let authored = session.authored(&rig, frame.model).unwrap();
    let finger = rig.names["mHandIndex1Left"];
    assert!(
        near(authored.rotations[finger], session.pose.rotations[finger]),
        "a preset is the user's"
    );
}
#[test]
fn poser_t_pose_makes_the_whole_pose_the_users_until_undo_or_reset() {
    let (world, mut editor, left, _, played) = captured_animation();
    let rig = world.avatar_lib.rig.clone();
    let frame = Frame::new(&world).unwrap();
    editor.select(left, &rig, &frame);
    editor.pose_operation(&rig, &frame, controls::Operation::TPose);
    let built = Quat::from_rotation_y(0.5);
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    session.checkpoint();
    Arc::make_mut(&mut session.pose).rotations[left] = built;
    let authored = session.authored(&rig, frame.model).unwrap();
    assert!(near(authored.rotations[left], built), "built from the T-pose, it exports in full");
    let document = document::Document::capture(&rig, session, "Built".into(), 4);
    let mut other = data::Session::new((*session.base).clone());
    assert!(document.apply(&rig, &mut other, document::LoadMode::All, Mat4::IDENTITY));
    assert!(near(other.pose.rotations[left], built), "a complete file reloads as it was");
    assert!(
        near(other.authored(&rig, frame.model).unwrap().rotations[left], built),
        "and stays the user's on the avatar it is loaded on"
    );
    session.history(false);
    session.history(false);
    assert!(near(session.pose.rotations[left], played));
    assert!(
        near(session.authored(&rig, frame.model).unwrap().rotations[left], rig.local_rot[left]),
        "undo restores the origin"
    );
    session.history(true);
    editor.pose_operation(&rig, &frame, controls::Operation::ResetJoint);
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(near(session.pose.rotations[left], played));
    assert!(near(
        session.authored(&rig, frame.model).unwrap().rotations[left],
        rig.local_rot[left]
    ));
    editor.pose_operation(&rig, &frame, controls::Operation::ResetAll);
    assert!(near(editor.poser.sessions[&world.agent_id].pose.origin[left], played));
}
#[test]
fn poser_a_saved_change_applies_on_the_destinations_own_pose_and_older_files_still_load() {
    let (world, editor, left, _, played) = captured_animation();
    let rig = &world.avatar_lib.rig;
    let change = Quat::from_rotation_z(0.2);
    let mut session = editor.poser.sessions[&world.agent_id].clone();
    Arc::make_mut(&mut session.pose).rotations[left] = played * change;
    let document = document::Document::capture(rig, &session, "Change".into(), 4);
    let mut same = data::Session::new((*session.base).clone());
    assert!(document.apply(rig, &mut same, document::LoadMode::All, Mat4::IDENTITY));
    assert!(near(same.pose.rotations[left], played * change), "reloaded over the same pose");
    let older = br#"{"name":"mElbowLeft","rotation":[0.0,0.0,0.0,1.0],"translation":[0.0,0.0,0.0],"scale":[1.0,1.0,1.0],"locked":false}"#;
    let older: document::Joint = serde_json::from_slice(older).unwrap();
    assert_eq!(older.name, "mElbowLeft");
}
#[test]
fn poser_title_eye_hides_and_shows_the_markers_and_gizmos() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let frame = editor.editor_frame(&world).unwrap();
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    // the full window's title (the compact one has its markers button in its
    // last row)
    editor.poser.window_override = Some(false);
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    assert!(editor.poser.markers);
    // the eye shows what is on screen: open, then struck through once hidden
    for (icon, after) in [("eye-filled", false), ("eye-off", true)] {
        for _ in 0..3 {
            frames.run(Vec::new(), |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame));
        }
        let eye = frames
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("floater_action", "poser", icon))))
            .unwrap_or_else(|| panic!("the title shows {icon}"))
            .center();
        frames.run(click(eye), |ui: &mut egui::Ui| editor.poser_ui(ui.ctx(), &p, &world, &frame));
        assert_eq!(editor.poser.markers, after, "{icon} toggles the settings' switch");
        assert!(!editor.poser.settings_open, "no detour through the settings");
    }
}

#[test]
fn hover_markers_follow_the_silhouette_with_a_wider_margin_to_leave() {
    use compact::{REVEAL_ENTER, REVEAL_LEAVE, markers_shown};
    assert!(markers_shown(false, None, false, false, false), "Always mode shows them");
    assert!(markers_shown(true, Some(5.0), false, false, false), "on the body");
    assert!(
        markers_shown(true, Some(-REVEAL_ENTER + 1.0), false, false, false),
        "just beside it"
    );
    let between = -(REVEAL_ENTER + REVEAL_LEAVE) / 2.0;
    assert!(!markers_shown(true, Some(between), false, false, false), "too far to come");
    assert!(
        markers_shown(true, Some(between), true, false, false),
        "but near enough to stay: no flicker at the edge"
    );
    assert!(
        !markers_shown(true, Some(-REVEAL_LEAVE - 1.0), true, false, false),
        "gone past the wider margin"
    );
    assert!(
        !markers_shown(true, Some(5.0), false, true, false),
        "a window over the avatar hides them"
    );
    assert!(!markers_shown(true, None, true, false, false), "pointer outside the viewer");
    assert!(
        markers_shown(true, Some(-500.0), false, false, true),
        "a drag that leaves keeps them"
    );
}

#[test]
fn a_click_picks_the_nearest_posed_avatar_under_the_pointer() {
    let (near, far) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let near_area = compact::body_area([egui::pos2(100.0, 100.0), egui::pos2(140.0, 300.0)]).unwrap();
    let far_area = compact::body_area([egui::pos2(110.0, 120.0), egui::pos2(130.0, 220.0)]).unwrap();
    let others = [(far, far_area, 9.0), (near, near_area, 3.0)];
    assert_eq!(
        compact::clicked_subject(&others, egui::pos2(120.0, 150.0)),
        Some(near),
        "the nearer one in front"
    );
    assert_eq!(compact::clicked_subject(&others, egui::pos2(120.0, 290.0)), Some(near));
    assert_eq!(
        compact::clicked_subject(&others, egui::pos2(600.0, 150.0)),
        None,
        "the ground beside"
    );
    assert_eq!(compact::clicked_subject(&[], egui::pos2(120.0, 150.0)), None);
}

#[test]
fn a_click_beside_the_joints_unselects_and_the_pelvis_is_not_picked_again() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let at = |name: &str| proj.project(frame.joints[rig.names[name]].w_axis.truncate()).unwrap().0;
    let mut frames = Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser-compact");
    let p = crate::theme::Theme::default().palette();
    let mut click = |editor: &mut Editor, pointer: egui::Pos2| {
        let button = |pressed| egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(pointer)],
            vec![button(true)],
            vec![button(false)],
            vec![],
            vec![],
        ] {
            frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
        }
    };
    assert!(editor.selected.is_some());
    click(&mut editor, egui::pos2(790.0, 890.0));
    assert_eq!(editor.selected, None, "the ground beside unselects");
    assert!(editor.poser.unselected);
    click(&mut editor, egui::pos2(790.0, 880.0));
    assert_eq!(editor.selected, None, "the pelvis is not chosen again by itself");
    click(&mut editor, at("mKneeLeft"));
    assert_eq!(editor.selected, Some(rig.names["mKneeLeft"]), "a joint's marker selects it");
    assert!(!editor.poser.unselected);
}

#[test]
fn reset_joint_in_mirror_resets_both_sides_and_one_side_otherwise() {
    let (mut world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let left = rig.names["mElbowLeft"];
    let right = rig.names["mElbowRight"];
    let bend_both = |world: &mut World, editor: &mut Editor| {
        let frame = Frame::new(world).unwrap();
        editor.select(left, &rig, &frame);
        let base = editor.poser.sessions[&world.agent_id].pose.clone();
        editor.poser_transform(&rig, &frame, base.positions[left], Quat::from_rotation_z(0.6), true);
        render(world, editor);
        editor.pose_operation(&rig, &Frame::new(world).unwrap(), controls::Operation::Mirror);
        let session = &editor.poser.sessions[&world.agent_id];
        assert!(session.edited(left) && session.edited(right));
    };
    // Left: the reset leaves the right elbow as it is
    bend_both(&mut world, &mut editor);
    editor.poser.side = Side::Left;
    editor.pose_operation(&rig, &Frame::new(&world).unwrap(), controls::Operation::ResetJoint);
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(!session.edited(left) && session.edited(right));
    // Mirror: both elbows go back (FSPoserAnimator::resetJoint)
    bend_both(&mut world, &mut editor);
    editor.poser.side = Side::Mirror;
    editor.select(left, &rig, &Frame::new(&world).unwrap());
    editor.pose_operation(&rig, &Frame::new(&world).unwrap(), controls::Operation::ResetJoint);
    let session = &editor.poser.sessions[&world.agent_id];
    assert!(!session.edited(left) && !session.edited(right));
}

#[test]
fn an_avatar_who_accepted_can_be_posed_by_a_click_before_any_menu_choice() {
    let (mut world, editor) = fixture();
    let peer = demo::poser::PEER;
    assert!(
        !editor.poseable_others(&world).contains(&peer),
        "not asked yet: a click only unselects"
    );
    let mut consent = std::mem::take(&mut world.poser_consent);
    consent.request(&world, peer, Instant::now());
    assert!(!editor.poseable_others(&world).contains(&peer), "asked, no answer yet");
    consent.demo_accept(&world, peer, std::time::Duration::ZERO);
    world.poser_consent = consent;
    assert!(editor.poseable_others(&world).contains(&peer), "accepted: a click poses them");
    assert!(
        !editor.poseable_others(&world).contains(&world.agent_id),
        "the subject itself is not listed"
    );
}

#[test]
fn another_groups_part_is_offered_only_when_clearly_nearer() {
    use world_hints::offered_joint;
    // joints 0 and 1 are Body, 2 Hands, 3 Physics (never offered)
    let groups = [0, 0, 2, 4];
    let offer = [true, true, true, false];
    let joints = [
        (0, egui::pos2(0.0, 0.0)),
        (1, egui::pos2(100.0, 0.0)),
        (2, egui::pos2(100.0, 40.0)),
        (3, egui::pos2(100.0, 41.0)),
    ];
    assert_eq!(
        offered_joint(&joints, &groups, &offer, 0, egui::pos2(100.0, 45.0)),
        Some(2),
        "on the hand: Hands"
    );
    assert_eq!(
        offered_joint(&joints, &groups, &offer, 0, egui::pos2(100.0, 22.0)),
        None,
        "between wrist and hand: Body keeps it"
    );
    assert_eq!(
        offered_joint(&joints, &groups, &offer, 2, egui::pos2(100.0, 45.0)),
        None,
        "already in Hands"
    );
    assert_eq!(
        offered_joint(&joints, &groups, &offer, 2, egui::pos2(2.0, 0.0)),
        Some(0),
        "from Hands, the body offers Body"
    );
    // a joint that may not be offered does not hide the current group's
    let none = [true, true, false, false];
    assert_eq!(offered_joint(&joints, &groups, &none, 0, egui::pos2(100.0, 45.0)), None);
}

#[test]
fn wings_and_tail_are_their_own_parts_offered_only_when_a_mesh_uses_them() {
    use world_hints::{TAIL, WINGS, offerable, part, part_key};
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let keys: Vec<usize> = rig.canonical_names.iter().map(|n| part_key(n)).collect();
    let wing = rig.names["mWing1Left"];
    let tail = rig.names["mTail1"];
    assert_eq!(keys[wing], WINGS);
    assert_eq!(keys[tail], TAIL);
    let wings = part(&rig.parent, &keys, wing);
    assert!(!wings.contains(&tail), "the tail is apart from the wings");
    assert!(wings.iter().all(|j| keys[*j] == WINGS));
    assert!(part(&rig.parent, &keys, tail).iter().all(|j| keys[*j] == TAIL));
    assert!(
        !offerable(WINGS, false) && offerable(WINGS, true),
        "wings only when a mesh uses them"
    );
    assert!(!offerable(TAIL, false) && offerable(TAIL, true));
    let hind = keys[rig.names["mHindLimb1Left"]];
    assert!(!offerable(hind, true), "the hind limbs lie over the legs: never offered");
    assert!(!offerable(keys[rig.names["mSkull"]], true));
    assert!(!offerable(4, true), "physics is never offered");
    assert!(offerable(keys[rig.names["mHandIndex1Left"]], false), "the hands always");
}

#[test]
fn a_part_is_one_hand_one_face_and_stops_at_other_groups() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let groups: Vec<usize> = rig.canonical_names.iter().map(|n| world_hints::part_key(n)).collect();
    let left = world_hints::part(&rig.parent, &groups, rig.names["mHandIndex1Left"]);
    assert!(left.contains(&rig.names["mHandThumb3Left"]), "the whole left hand");
    assert!(!left.contains(&rig.names["mHandIndex1Right"]), "not the other hand");
    assert!(!left.contains(&rig.names["mWristLeft"]), "the wrist is Body's");
    assert!(left.iter().all(|j| groups[*j] == 2));
    let face = world_hints::part(&rig.parent, &groups, rig.names["mFaceLipUpperCenter"]);
    assert!(
        face.contains(&rig.names["mFaceEyeAltLeft"]) || face.len() > 20,
        "the face is one part"
    );
    assert!(face.iter().all(|j| groups[*j] == 1));
}

#[test]
fn the_tint_is_the_union_of_the_bones_capsules_painted_once() {
    use silhouette::{Capsule, depth, trace};
    // an upper arm and a forearm, thinner at the wrist
    let arm = [
        Capsule {
            a: egui::pos2(0.0, 0.0),
            b: egui::pos2(100.0, 0.0),
            ra: 12.0,
            rb: 9.0,
        },
        Capsule {
            a: egui::pos2(100.0, 0.0),
            b: egui::pos2(100.0, 90.0),
            ra: 9.0,
            rb: 6.0,
        },
    ];
    assert!(depth(&arm, egui::pos2(50.0, 5.0)) > 0.0, "on the upper arm");
    assert!(depth(&arm, egui::pos2(103.0, 85.0)) > 0.0, "on the forearm near the wrist");
    assert!(
        depth(&arm, egui::pos2(50.0, 40.0)) < 0.0,
        "in the elbow's crook: not a hull's triangle"
    );
    assert!(depth(&arm, egui::pos2(100.0, 92.0 + 8.0)) < 0.0, "past the wrist");
    let shape = trace(&arm);
    assert!(!shape.triangles.is_empty() && !shape.edge.is_empty());
    // every triangle lies on the arm: the inside is clipped, nothing doubles
    let area: f32 = shape
        .triangles
        .iter()
        .map(|[a, b, c]| ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs() / 2.0)
        .sum();
    let expected = 100.0 * 21.0 + 90.0 * 15.0 + std::f32::consts::PI * 9.0 * 9.0 / 2.0;
    assert!(
        (area - expected).abs() / expected < 0.2,
        "area {area} near {expected}: painted once"
    );
    assert!(trace::<Capsule>(&[]).triangles.is_empty());
}

#[test]
fn the_face_part_has_no_links_and_a_hand_reaches_its_wrist() {
    use silhouette::{Links, capsules, links_of};
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let at: Vec<glam::Vec3> = frame.joints.iter().map(|m| m.w_axis.truncate()).collect();
    let project = |w: glam::Vec3| proj.project(w).map(|(p, _)| p);
    let keys: Vec<usize> = rig.canonical_names.iter().map(|n| world_hints::part_key(n)).collect();
    assert_eq!(links_of(1), Links::None);
    let face = world_hints::part(&rig.parent, &keys, rig.names["mFaceLipUpperCenter"]);
    let face_caps = capsules(&rig.parent, &rig.canonical_names, &at, &face, links_of(1), project);
    assert!(face_caps.iter().all(|c| c.a == c.b), "the face: joints only, no star from the head");
    let hand = world_hints::part(&rig.parent, &keys, rig.names["mHandIndex1Left"]);
    let hand_caps = capsules(&rig.parent, &rig.canonical_names, &at, &hand, links_of(2), project);
    let wrist = project(at[rig.names["mWristLeft"]]).unwrap();
    assert!(hand_caps.iter().any(|c| c.a == wrist), "the fingers reach the wrist: the palm");
}

#[test]
fn the_other_tab_shows_only_the_bones_a_mesh_uses_or_the_pose_moved() {
    let (mut world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let tail = rig.names["mTail1"];
    let wing = rig.names["mWing1Left"];
    let hind = rig.names["mHindLimb1Left"];
    editor.poser.group = OTHER_GROUP;
    editor.refresh_other_bones(&rig);
    assert!(editor.poser.other_used.is_empty(), "nothing worn: the tab is empty");
    // the spine's collapsed pairs too (the human's choice, 2026-11-10: their
    // markers sit upside down and moving them tears the body apart)
    assert!(!editor.poser_bone_visible("mSpine1") && !editor.poser_bone_visible("mSpine4"));
    // a worn tail
    editor.poser.rigged = [tail].into();
    editor.refresh_other_bones(&rig);
    assert!(editor.poser_bone_visible("mTail1"));
    assert!(!editor.poser_bone_visible("mWing1Left") && !editor.poser_bone_visible("mHindLimb1Left"));
    // a pose that bends a wing keeps it reachable
    let frame = Frame::new(&world).unwrap();
    editor.select(wing, &rig, &frame);
    let base = editor.poser.sessions[&world.agent_id].pose.clone();
    editor.poser_transform(&rig, &frame, base.positions[wing], Quat::from_rotation_z(0.4), true);
    render(&mut world, &editor);
    editor.refresh_other_bones(&rig);
    assert!(editor.poser_bone_visible("mWing1Left"), "an edited wing stays shown");
    assert!(!editor.poser_bone_visible(&rig.canonical_names[hind]));
    // the other tabs are untouched
    editor.poser.group = 0;
    assert!(editor.poser_bone_visible("mElbowLeft"));
}

#[test]
fn an_empty_other_tab_stays_open_instead_of_going_back_to_body() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    editor.refresh_other_bones(rig);
    editor.select_pose_group(OTHER_GROUP, rig, &frame);
    assert_eq!(editor.selected, None, "nothing worn: no Other joint to select");
    for _ in 0..3 {
        frames.run(Vec::new(), |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    assert_eq!(editor.poser.group, OTHER_GROUP, "the tab stays open");
    assert_eq!(editor.selected, None, "the pelvis is not picked again");
}

#[test]
fn a_click_on_the_ground_unselects_with_the_markers_hidden_on_hover() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let mut frames = Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser-compact");
    let p = crate::theme::Theme::default().palette();
    editor.poser.markers_hover = true;
    let ground = egui::pos2(790.0, 890.0);
    // away from the avatar for a second: the markers fade out
    for _ in 0..60 {
        frames.run(vec![egui::Event::PointerMoved(ground)], |ui| {
            editor.show(ui.ctx(), &p, &world, &proj)
        });
    }
    assert!(editor.selected.is_some());
    let button = |pressed| egui::Event::PointerButton {
        pos: ground,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [vec![button(true)], vec![button(false)], vec![], vec![]] {
        frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    assert_eq!(editor.selected, None, "the ground unselects, markers hidden or not");
}

#[test]
fn the_body_is_one_part_from_the_head_to_the_feet() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let keys: Vec<usize> = rig.canonical_names.iter().map(|n| world_hints::part_key(n)).collect();
    for start in ["mChest", "mKneeLeft", "mPelvis", "mHead"] {
        let body = world_hints::part(&rig.parent, &keys, rig.names[start]);
        for joint in ["mPelvis", "mTorso", "mChest", "mHead", "mAnkleLeft", "mAnkleRight", "mWristLeft"] {
            assert!(body.contains(&rig.names[joint]), "hovering {start} shows {joint}");
        }
    }
    // the torso joins the pelvis across the spine joints, which are Other's
    let frame = Frame::new(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let at: Vec<glam::Vec3> = frame.joints.iter().map(|m| m.w_axis.truncate()).collect();
    let project = |w: glam::Vec3| proj.project(w).map(|(p, _)| p);
    let body = world_hints::part(&rig.parent, &keys, rig.pelvis);
    let caps = silhouette::capsules(&rig.parent, &rig.canonical_names, &at, &body, silhouette::Links::Inside, project);
    let waist = project(at[rig.pelvis])
        .unwrap()
        .lerp(project(at[rig.names["mTorso"]]).unwrap(), 0.5);
    assert!(silhouette::depth(&caps, waist) > 0.0, "no gap at the waist");
}

#[test]
fn a_ctrl_click_on_an_avatar_who_accepted_poses_them_and_a_plain_click_does_not() {
    use crate::ui::widgets::elide_tests::Frames;
    let (mut world, mut editor) = selected_fixture();
    let peer = demo::poser::PEER;
    // the peer stands posed beside us, and has accepted
    let own = world.avatar_poses[&world.agent_id].clone();
    world.avatar_poses.insert(peer, own);
    let mut consent = std::mem::take(&mut world.poser_consent);
    consent.request(&world, peer, Instant::now());
    consent.demo_accept(&world, peer, std::time::Duration::ZERO);
    world.poser_consent = consent;
    let peer_frame = Frame::subject(&world, peer).expect("the peer's skeleton");
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let on_peer = proj.project(peer_frame.joints[rig.names["mChest"]].w_axis.truncate()).unwrap().0;
    let mut frames = Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser-compact");
    let p = crate::theme::Theme::default().palette();
    editor.poser.markers_hover = true;
    let me = editor.poser.subject;
    for (keys, expected, why) in [
        (
            egui::Modifiers::NONE,
            me,
            "a plain click stays the world's (a photo's focus, a HUD)",
        ),
        (egui::Modifiers::CTRL, peer, "Ctrl + click poses them"),
    ] {
        // a second over them: our own markers fade out, their preview
        // (with Ctrl) stays
        for _ in 0..60 {
            frames.run(
                vec![egui::Event::ModifiersChanged(keys), egui::Event::PointerMoved(on_peer)],
                |ui| editor.show(ui.ctx(), &p, &world, &proj),
            );
        }
        let button = |pressed| egui::Event::PointerButton {
            pos: on_peer,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: keys,
        };
        for events in [vec![button(true)], vec![button(false)], vec![], vec![]] {
            frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
        }
        assert_eq!(editor.poser.subject, expected, "{why}");
    }
}

#[test]
fn a_y_angle_past_ninety_degrees_stays_as_set() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let frame = Frame::new(&world).unwrap();
    let elbow = rig.names["mElbowLeft"];
    editor.select(elbow, &rig, &frame);
    editor.poser.transform = 0;
    // a slider dragged from 85° to 120°, one frame at a time
    for y in [85.0, 95.0, 105.0, 120.0] {
        let set = Vec3::new(10.0, y, -20.0);
        editor.apply_joint_value(&rig, &frame, elbow, set);
        let shown = editor.joint_value(&rig, &frame, elbow).unwrap();
        assert!(
            (shown - set).length() < 1e-3,
            "Y {y}: shows {shown:?}, not the same turn read the other way"
        );
    }
    // the same rotation still: Y 120° with X and Z as set
    let q = editor.poser.sessions[&world.agent_id].pose.rotations[elbow];
    let expected = Quat::from_euler(EulerRot::ZYX, (-20f32).to_radians(), 120f32.to_radians(), 10f32.to_radians());
    assert!(q.dot(expected).abs() > 0.9999);
    // once something else turns the joint, the angles are read from it again
    editor.poser.sessions.get_mut(&world.agent_id).unwrap().history(false);
    let after = editor.joint_value(&rig, &frame, elbow).unwrap();
    assert!(after.y.abs() <= 90.0 + 1e-3, "read back from the rotation: {after:?}");
}

#[test]
fn compact_reset_greys_out_when_it_would_change_nothing() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let left = rig.names["mElbowLeft"];
    let right = rig.names["mElbowRight"];
    let frame = Frame::new(&world).unwrap();
    editor.select(left, &rig, &frame);
    editor.poser.side = Side::Left;
    assert!(!editor.reset_available(false), "an untouched joint has nothing to reset");
    assert!(!editor.reset_available(true), "nor an untouched pose");
    // only the counterpart bent: one side has nothing, Mirror resets it too
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    Arc::make_mut(&mut session.pose).rotations[right] = Quat::from_rotation_z(0.6) * session.base.rotations[right];
    assert!(!editor.reset_available(false));
    editor.poser.side = Side::Mirror;
    assert!(
        editor.reset_available(false),
        "Mirror resets both sides (FSPoserAnimator::resetJoint)"
    );
    assert!(editor.reset_available(true), "Shift: the whole pose has an edit");
    // a lock alone is something the reset removes
    editor.poser.side = Side::Left;
    editor.pose_operation(&rig, &frame, controls::Operation::ResetAll);
    assert!(!editor.reset_available(true));
    editor.pose_operation(&rig, &frame, controls::Operation::WorldLock);
    assert!(editor.reset_available(false) && editor.reset_available(true));
}

#[test]
fn an_ellipse_is_inside_to_its_edge_and_a_tilted_one_follows_its_axes() {
    use silhouette::{Ellipse, Field};
    let c = egui::pos2(100.0, 100.0);
    let round = Ellipse::of_axes(c, [egui::vec2(20.0, 0.0), egui::vec2(0.0, 20.0), egui::Vec2::ZERO]).unwrap();
    assert!((round.depth(c) - 20.0).abs() < 1e-3, "its centre is a radius deep");
    assert!(round.depth(c + egui::vec2(19.5, 0.0)) > 0.0);
    assert!(round.depth(c + egui::vec2(20.5, 0.0)) < 0.0);
    // tall and thin: inside up the long axis, outside as far to the side
    let tall = Ellipse::of_axes(c, [egui::vec2(10.0, 0.0), egui::vec2(0.0, 30.0), egui::Vec2::ZERO]).unwrap();
    assert!(tall.depth(c + egui::vec2(0.0, 25.0)) > 0.0);
    assert!(tall.depth(c + egui::vec2(25.0, 0.0)) < 0.0);
    assert_eq!(tall.bounds(), egui::Rect::from_center_size(c, egui::vec2(20.0, 60.0)));
    // seen edge on, there is no outline
    assert!(Ellipse::of_axes(c, [egui::vec2(10.0, 0.0), egui::vec2(20.0, 0.0), egui::Vec2::ZERO]).is_none());
}

#[test]
fn the_face_tint_is_an_oval_over_the_face_and_not_the_neck() {
    use silhouette::Field;
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let head = frame.joints[rig.names["mHead"]];
    let proj = projector(head.w_axis.truncate());
    let at: Vec<glam::Vec3> = frame.joints.iter().map(|m| m.w_axis.truncate()).collect();
    let project = |w: glam::Vec3| proj.project(w).map(|(p, _)| p);
    let keys: Vec<usize> = rig.canonical_names.iter().map(|n| world_hints::part_key(n)).collect();
    let face = world_hints::part(&rig.parent, &keys, rig.names["mFaceLipUpperCenter"]);
    let oval = silhouette::face(&rig.canonical_names, &at, &face, head, project).expect("the face faces the camera");
    for joint in [
        "mFaceLipUpperCenter",
        "mFaceEyeAltLeft",
        "mFaceEyeAltRight",
        "mFaceNoseBase",
        "mFaceForeheadCenter",
    ] {
        let point = project(at[rig.names[joint]]).unwrap();
        assert!(oval.depth(point) > 0.0, "{joint} is inside the oval");
    }
    for joint in ["mNeck", "mChest"] {
        let point = project(at[rig.names[joint]]).unwrap();
        assert!(oval.depth(point) < 0.0, "{joint} is not");
    }
    // one smooth closed outline on its edge, not a cloud of dots
    let outline = oval.outline(64);
    assert_eq!(outline.len(), 64);
    assert!(outline.iter().all(|p| oval.depth(*p).abs() < 0.05), "the outline lies on the edge");
}

#[test]
fn another_part_shows_and_takes_the_click_only_while_ctrl_is_held() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let p = crate::theme::Theme::default().palette();
    editor.poser.group = 0;
    // a fingertip while posing the Body: the Hands part
    let finger = proj
        .project(frame.joints[rig.names["mHandMiddle3Left"]].w_axis.truncate())
        .unwrap()
        .0;
    let mut frames = Frames::new();
    let hint = |frames: &mut Frames, editor: &mut Editor, keys: egui::Modifiers| -> world_hints::Hint {
        let mut out = world_hints::Hint::None;
        let events = vec![egui::Event::ModifiersChanged(keys), egui::Event::PointerMoved(finger)];
        frames.run(events, |ui| {
            out = editor.world_hint(ui.ctx(), &p, &world, &proj, &frame, true);
        });
        out
    };
    assert_eq!(
        hint(&mut frames, &mut editor, egui::Modifiers::NONE),
        world_hints::Hint::None,
        "no Ctrl: no tint, the click stays the world's"
    );
    assert!(
        matches!(
            hint(&mut frames, &mut editor, egui::Modifiers::CTRL),
            world_hints::Hint::Group(j) if group(&rig.canonical_names[j]) == 2
        ),
        "Ctrl: the hand is offered"
    );
}

#[test]
fn a_click_a_worn_hud_took_keeps_the_joint_and_the_next_one_on_the_ground_unselects() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let mut frames = Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser-compact");
    let p = crate::theme::Theme::default().palette();
    let ground = egui::pos2(790.0, 890.0);
    let button = |pressed| egui::Event::PointerButton {
        pos: ground,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let click = |editor: &mut Editor, frames: &mut Frames| {
        for events in [
            vec![egui::Event::PointerMoved(ground)],
            vec![button(true)],
            vec![button(false)],
            vec![],
        ] {
            frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
        }
    };
    assert!(editor.selected.is_some());
    // the app's press path gave this press to a HUD button
    editor.press_on_hud = true;
    click(&mut editor, &mut frames);
    assert!(editor.selected.is_some(), "the HUD's click leaves the joint selected");
    assert!(!editor.press_on_hud, "read once");
    // a drag on a HUD (no click) does not leave its press to the next click
    editor.press_on_hud = true;
    for events in [
        vec![egui::Event::PointerMoved(ground)],
        vec![button(true)],
        vec![egui::Event::PointerMoved(ground + egui::vec2(80.0, 40.0))],
        vec![egui::Event::PointerButton {
            pos: ground + egui::vec2(80.0, 40.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
        vec![],
    ] {
        frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    assert!(!editor.press_on_hud, "the HUD's press ends with its release");
    assert!(editor.selected.is_some(), "a drag is no click beside");
    click(&mut editor, &mut frames);
    assert_eq!(editor.selected, None, "a click on the ground still unselects");
}

#[test]
fn the_reset_greys_out_once_posing_is_stopped() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let elbow = rig.names["mElbowLeft"];
    let frame = Frame::new(&world).unwrap();
    editor.select(elbow, &rig, &frame);
    editor.poser.side = Side::Left;
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    session.enabled = true;
    Arc::make_mut(&mut session.pose).rotations[elbow] = Quat::from_rotation_z(0.6) * session.base.rotations[elbow];
    assert!(editor.reset_available(false) && editor.reset_available(true), "posing, edited: lit");
    editor.poser.sessions.get_mut(&world.agent_id).unwrap().enabled = false;
    assert!(
        !editor.reset_available(false) && !editor.reset_available(true),
        "stopped: the reset would change nothing (pose_operation leaves a stopped pose)"
    );
}

#[test]
fn from_the_other_tab_ctrl_over_the_chest_offers_the_body_past_the_hidden_spine() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let p = crate::theme::Theme::default().palette();
    // the Other tab, nothing worn: the spine is hidden, but mSpine3 sits on
    // mChest and would count as the tab's own, nearer than the body's
    editor.poser.group = OTHER_GROUP;
    editor.refresh_other_bones(rig);
    assert!(!editor.poser_bone_visible("mSpine3"));
    let chest = proj.project(frame.joints[rig.names["mChest"]].w_axis.truncate()).unwrap().0;
    let mut frames = Frames::new();
    let mut hint = world_hints::Hint::None;
    frames.run(
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
            egui::Event::PointerMoved(chest),
        ],
        |ui| {
            hint = editor.world_hint(ui.ctx(), &p, &world, &proj, &frame, true);
        },
    );
    assert!(
        matches!(hint, world_hints::Hint::Group(j) if group(&rig.canonical_names[j]) == 0),
        "the Body is offered: {hint:?}"
    );
}

#[test]
fn the_handles_give_way_to_ctrl_only_over_a_part_or_avatar_it_shows() {
    let (_world, mut editor) = fixture();
    editor.poser.active = true;
    // Ctrl with nothing to pick under the pointer: the handles keep it (its
    // temporary rotation, a handle's click in the photo mode)
    editor.poser.ctrl_hint = false;
    assert!(!editor.poser_ctrl_picks(egui::Modifiers::CTRL));
    // a part or an avatar shown under the pointer last frame: they give way
    editor.poser.ctrl_hint = true;
    assert!(editor.poser_ctrl_picks(egui::Modifiers::CTRL));
    assert!(editor.poser_ctrl_picks(egui::Modifiers::COMMAND));
    // no Ctrl: the handles, always
    assert!(!editor.poser_ctrl_picks(egui::Modifiers::NONE));
}

#[test]
fn a_bone_inside_the_body_is_listed_in_the_full_window_and_dropped_by_the_compact_one() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let p = crate::theme::Theme::default().palette();
    let skull = rig.names["mSkull"];
    // hair rigged to the skull: in the full window's Other list
    editor.poser.rigged = [skull].into();
    editor.poser.window_override = Some(false);
    editor.refresh_other_bones(rig);
    editor.select_pose_group(OTHER_GROUP, rig, &frame);
    assert!(editor.poser.other_inner_used.contains("mSkull") && !editor.poser_bone_visible("mSkull"));
    let mut frames = Frames::new();
    crate::ui::widgets::place_floater(&frames.ctx, "poser", egui::pos2(30.0, 30.0), egui::vec2(880.0, 760.0));
    let mut texts = Vec::new();
    for _ in 0..3 {
        texts = frames.run(Vec::new(), |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    assert!(
        texts.iter().any(|t| t.source == bones::label("mSkull")),
        "the full window lists the skull"
    );
    // chosen there, then the compact window: dropped, as a click beside
    editor.selected = Some(skull);
    editor.poser.window_override = Some(true);
    for _ in 0..2 {
        frames.run(Vec::new(), |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    assert_eq!(editor.selected, None, "no bone inside the body in the compact window");
}

/// One frame of the poser with `keys` held and the pointer at `at`, the
/// primary button pressed or let go there when `button` says so.
fn pose_frame(
    frames: &mut crate::ui::widgets::elide_tests::Frames,
    editor: &mut Editor,
    world: &World,
    proj: &Projector,
    keys: egui::Modifiers,
    at: egui::Pos2,
    button: Option<bool>,
) {
    let p = crate::theme::Theme::default().palette();
    let mut events = vec![egui::Event::ModifiersChanged(keys), egui::Event::PointerMoved(at)];
    if let Some(pressed) = button {
        events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: keys,
        });
    }
    frames.run(events, |ui| editor.show(ui.ctx(), &p, world, proj));
}

/// A point around `center` (rings out to 120 points, every 15 degrees)
/// where, with Ctrl newly held, the selected joint's handles take the
/// pointer and `want` holds after that frame.
fn point_on_handles(
    frames: &mut crate::ui::widgets::elide_tests::Frames,
    editor: &mut Editor,
    world: &World,
    proj: &Projector,
    center: egui::Pos2,
    want: impl Fn(&Editor, egui::Pos2) -> bool,
) -> Option<egui::Pos2> {
    for radius in (4..=120).step_by(4) {
        for k in 0..24 {
            let angle = k as f32 * std::f32::consts::TAU / 24.0;
            let at = center + egui::vec2(angle.cos(), angle.sin()) * radius as f32;
            pose_frame(frames, editor, world, proj, egui::Modifiers::NONE, at, None);
            pose_frame(frames, editor, world, proj, egui::Modifiers::CTRL, at, None);
            if editor.poser.gizmo_hover && want(editor, at) {
                return Some(at);
            }
        }
    }
    None
}

/// Nothing of the open tab's skeleton (marker or shaft) under `at`, read
/// from the segments themselves, whatever the poser reports.
fn bare(editor: &Editor, world: &World, proj: &Projector, at: egui::Pos2) -> bool {
    let frame = Frame::new(world).unwrap();
    !editor.poser.marker_hover && segments::pick(&editor.skeleton_segments(world, proj, &frame), at).is_none()
}

/// The left wrist chosen in the Body tab: its handles around it, the
/// fingers (Hands) under some of them.
fn wrist_fixture() -> (World, Editor, usize, Projector, crate::ui::widgets::elide_tests::Frames) {
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let wrist = rig.names["mWristLeft"];
    editor.select(wrist, rig, &frame);
    editor.poser.group = 0;
    let frames = crate::ui::widgets::elide_tests::Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser-compact");
    (world, editor, wrist, proj, frames)
}

#[test]
fn a_ctrl_click_through_the_handles_reaches_the_part_under_them_across_frames() {
    let (world, mut editor, wrist, proj, mut frames) = wrist_fixture();
    let rig = &world.avatar_lib.rig;
    let center = proj.project(Frame::new(&world).unwrap().joints[wrist].w_axis.truncate()).unwrap().0;
    let at = point_on_handles(&mut frames, &mut editor, &world, &proj, center, |e, _| e.poser.ctrl_hint)
        .expect("a point of the wrist's handles over the fingers");
    // hovered, then pressed, held and let go on separate frames
    let ctrl = egui::Modifiers::CTRL;
    for button in [None, Some(true), None, None, Some(false), None] {
        pose_frame(&mut frames, &mut editor, &world, &proj, ctrl, at, button);
    }
    assert!(
        editor.selected.is_some_and(|b| group(&rig.canonical_names[b]) == 2),
        "the Ctrl + click picks the hand: {:?}",
        editor.selected.map(|b| rig.canonical_names[b].clone())
    );
}

#[test]
fn ctrl_on_the_handles_with_nothing_under_them_keeps_them_and_the_joint() {
    let (world, mut editor, wrist, proj, mut frames) = wrist_fixture();
    let center = proj.project(Frame::new(&world).unwrap().joints[wrist].w_axis.truncate()).unwrap().0;
    let at = point_on_handles(&mut frames, &mut editor, &world, &proj, center, |e, at| {
        !e.poser.ctrl_hint && bare(e, &world, &proj, at)
    })
    .expect("a point of the wrist's handles with nothing to pick");
    // the handles keep the pointer with Ctrl: in the photo mode the press
    // stays the poser's (`poser_owns_pointer`), not the world's
    assert!(editor.poser_owns_pointer(), "the handles own the pointer");
    for button in [Some(true), None, Some(false), None] {
        pose_frame(&mut frames, &mut editor, &world, &proj, egui::Modifiers::CTRL, at, button);
    }
    assert_eq!(editor.selected, Some(wrist), "the Ctrl + click on the handles does not unselect");
}

#[test]
fn a_press_moved_from_a_part_onto_the_handles_alone_keeps_the_joint() {
    let ctrl = egui::Modifiers::CTRL;
    // points of the handles with nothing to pick, the nearest first; one
    // where no marker takes the pointer during the press either (a marker
    // would take the click whatever the handles do)
    let (world, mut editor, wrist, proj, mut frames) = wrist_fixture();
    let center = proj.project(Frame::new(&world).unwrap().joints[wrist].w_axis.truncate()).unwrap().0;
    let on_part = point_on_handles(&mut frames, &mut editor, &world, &proj, center, |e, _| e.poser.ctrl_hint)
        .expect("a point of the handles over the fingers");
    let mut tried = 0;
    for radius in (4..=120).step_by(4) {
        for k in 0..24 {
            let angle = k as f32 * std::f32::consts::TAU / 24.0;
            let alone = center + egui::vec2(angle.cos(), angle.sin()) * radius as f32;
            let (world, mut editor, wrist, proj, mut frames) = wrist_fixture();
            pose_frame(&mut frames, &mut editor, &world, &proj, egui::Modifiers::NONE, alone, None);
            pose_frame(&mut frames, &mut editor, &world, &proj, ctrl, alone, None);
            if !(editor.poser.gizmo_hover && !editor.poser.ctrl_hint && bare(&editor, &world, &proj, alone)) {
                continue;
            }
            tried += 1;
            // Ctrl over the hand (the handles give way), then in one frame
            // the pointer moves onto the handles alone and presses; held;
            // let go
            pose_frame(&mut frames, &mut editor, &world, &proj, ctrl, on_part, None);
            pose_frame(&mut frames, &mut editor, &world, &proj, ctrl, on_part, None);
            assert!(editor.poser.ctrl_hint);
            let mut marker = false;
            for button in [Some(true), None, None, Some(false), None] {
                pose_frame(&mut frames, &mut editor, &world, &proj, ctrl, alone, button);
                marker |= editor.poser.marker_hover;
            }
            if marker {
                continue;
            }
            assert_eq!(
                editor.selected,
                Some(wrist),
                "decided where the press lands: the handles, no unselect"
            );
            return;
        }
    }
    panic!("no point of the handles free of markers during a press ({tried} tried)");
}

#[test]
fn the_body_outline_stops_at_the_neck_and_leaves_the_head_to_the_face() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let proj = projector(frame.joints[rig.names["mHead"]].w_axis.truncate());
    let at: Vec<glam::Vec3> = frame.joints.iter().map(|m| m.w_axis.truncate()).collect();
    let project = |w: glam::Vec3| proj.project(w).map(|(p, _)| p);
    let keys: Vec<usize> = rig.canonical_names.iter().map(|n| world_hints::part_key(n)).collect();
    let body = world_hints::part(&rig.parent, &keys, rig.pelvis);
    assert!(body.contains(&rig.names["mHead"]), "the head is still a Body bone to pose");
    let outline = world_hints::outline_members(0, &body, &rig.canonical_names);
    let caps = silhouette::capsules(&rig.parent, &rig.canonical_names, &at, &outline, silhouette::Links::Inside, project);
    // inside the head (its sphere is 9.5 cm): outside the body's outline;
    // the neck: inside
    let crown = project(at[rig.names["mHead"]] + glam::Vec3::Z * 0.05).unwrap();
    assert!(silhouette::depth(&caps, crown) < 0.0, "the head is not the body's outline");
    assert!(
        silhouette::depth(&caps, project(at[rig.names["mNeck"]]).unwrap()) > 0.0,
        "the neck is"
    );
}

#[test]
fn compact_reset_counts_the_joints_selected_together() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let (left, right) = (rig.names["mEyeLeft"], rig.names["mEyeRight"]);
    let frame = Frame::new(&world).unwrap();
    editor.poser.group = 1;
    editor.click_pose_bone(left, &rig, &frame, false);
    editor.click_pose_bone(right, &rig, &frame, true);
    editor.poser.side = Side::Left;
    assert_eq!(editor.reset_joints(), vec![left, right]);
    assert!(!editor.reset_available(false), "nothing edited yet");
    // only the follower turned: Reset joint resets it with the lead
    let session = editor.poser.sessions.get_mut(&world.agent_id).unwrap();
    Arc::make_mut(&mut session.pose).rotations[right] = Quat::from_rotation_z(0.4) * session.base.rotations[right];
    assert!(editor.reset_available(false), "a follower's edit is the reset's");
    editor.pose_operation(&rig, &frame, controls::Operation::ResetJoint);
    assert!(!editor.reset_available(false), "both reset in one step");
}

#[test]
fn a_joint_picked_alone_leaves_no_joint_following() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let (left, right) = (rig.names["mEyeLeft"], rig.names["mEyeRight"]);
    let frame = Frame::new(&world).unwrap();
    editor.poser.group = 1;
    editor.click_pose_bone(left, &rig, &frame, false);
    editor.click_pose_bone(right, &rig, &frame, true);
    assert_eq!(editor.followers(), vec![right]);
    // Ctrl + click on another group's part selects that joint alone
    editor.click_pose_bone(rig.pelvis, &rig, &frame, false);
    assert!(editor.followers().is_empty());
    assert_eq!(editor.selected, Some(rig.pelvis));
}

#[test]
fn the_compact_window_drops_inner_followers_and_hands_an_inner_lead_over() {
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    let (skull, spine, eye) = (rig.names["mSkull"], rig.names["mSpine1"], rig.names["mEyeLeft"]);
    // an inner follower goes, the lead stays
    editor.selected = Some(eye);
    editor.poser.together = vec![skull];
    editor.drop_inner_selection(&rig);
    assert_eq!((editor.selected, editor.followers()), (Some(eye), vec![]));
    // an inner lead hands over to a follower the window shows
    editor.selected = Some(spine);
    editor.poser.together = vec![skull, eye];
    editor.drop_inner_selection(&rig);
    assert_eq!((editor.selected, editor.followers()), (Some(eye), vec![]));
    assert!(!editor.poser.unselected);
    // alone, it is unselected as before
    editor.selected = Some(skull);
    editor.drop_inner_selection(&rig);
    assert_eq!(editor.selected, None);
    assert!(editor.poser.unselected);
}

#[test]
fn ctrl_over_the_subject_stays_the_subjects_with_a_partner_close_behind() {
    use crate::ui::widgets::elide_tests::Frames;
    let (mut world, mut editor) = selected_fixture();
    let peer = demo::poser::PEER;
    let own = world.avatar_poses[&world.agent_id].clone();
    world.avatar_poses.insert(peer, own);
    let mut consent = std::mem::take(&mut world.poser_consent);
    consent.request(&world, peer, Instant::now());
    consent.demo_accept(&world, peer, std::time::Duration::ZERO);
    world.poser_consent = consent;
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let peer_frame = Frame::subject(&world, peer).expect("the peer's skeleton");
    // the camera in front of us, the peer straight behind us on screen
    let chest = frame.joints[rig.names["mChest"]].w_axis.truncate();
    let away = peer_frame.joints[rig.names["mChest"]].w_axis.truncate() - chest;
    let eye = chest - away.normalize() * 3.0;
    let proj = Projector {
        view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 800.0 / 900.0, 0.05)
            * glam::camera::rh::view::look_to_mat4(eye, (chest - eye).normalize(), Vec3::Z),
        width: 800.0,
        height: 900.0,
        ppp: 1.0,
    };
    // on the torso, beside the chest's marker (a marker takes its own click)
    let on_me = proj.project(chest).unwrap().0 + egui::vec2(14.0, 18.0);
    let mut frames = Frames::new();
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser");
    crate::ui::widgets::minimize_floater(&frames.ctx, "poser-compact");
    let p = crate::theme::Theme::default().palette();
    let me = editor.poser.subject;
    for _ in 0..3 {
        frames.run(
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                egui::Event::PointerMoved(on_me),
            ],
            |ui| editor.show(ui.ctx(), &p, &world, &proj),
        );
    }
    let button = |pressed| egui::Event::PointerButton {
        pos: on_me,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::CTRL,
    };
    for events in [vec![button(true)], vec![button(false)], vec![], vec![]] {
        frames.run(events, |ui| editor.show(ui.ctx(), &p, &world, &proj));
    }
    assert_eq!(editor.poser.subject, me, "the partner behind is not picked through us");
}

/// Bit `joint` of a renderer part mask (`world_hints::mask_joints`).
fn in_mask(mask: &[u32; 8], joint: usize) -> bool {
    joint < 256 && mask[joint >> 5] & (1 << (joint & 31)) != 0
}

#[test]
fn the_body_mask_carries_its_volumes_and_spine_and_leaves_the_head_and_wrists() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let keys: Vec<usize> = rig.canonical_names.iter().map(|n| world_hints::part_key(n)).collect();
    let body = world_hints::part(&rig.parent, &keys, rig.pelvis);
    let joints = world_hints::mask_members(0, &body, &rig.parent, &keys, &rig.canonical_names, &Default::default());
    let mask = world_hints::mask_joints(&rig.parent, &rig.canonical_names, &joints);
    // a fitted body's volumes and a Bento body's spine tint with the body
    for name in [
        "mTorso",
        "mChest",
        "mElbowLeft",
        "BELLY",
        "LEFT_PEC",
        "UPPER_BACK",
        "L_UPPER_ARM",
        "mSpine1",
        "mSpine3",
        "NECK",
    ] {
        assert!(in_mask(&mask, rig.names[name]), "{name} is the body's");
    }
    // the head (its volume and the skull follow it) is the face's, a wrist
    // (its hand's volume) its hand's; the fingers and the face's own bones
    for name in ["mHead", "HEAD", "mSkull", "mWristLeft", "L_HAND", "mHandMiddle1Left", "mFaceRoot"] {
        assert!(!in_mask(&mask, rig.names[name]), "{name} is not the body's");
    }
}

#[test]
fn the_face_takes_the_head_and_a_hand_its_wrist_rigged_or_not() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let keys: Vec<usize> = rig.canonical_names.iter().map(|n| world_hints::part_key(n)).collect();
    let mask_of = |start: &str, rigged: &std::collections::HashSet<usize>| {
        let members = world_hints::part(&rig.parent, &keys, rig.names[start]);
        let key = keys[rig.names[start]];
        let joints = world_hints::mask_members(key, &members, &rig.parent, &keys, &rig.canonical_names, rigged);
        world_hints::mask_joints(&rig.parent, &rig.canonical_names, &joints)
    };
    let none = std::collections::HashSet::new();
    // a Bento head weights the face's bones; the system head mHead alone:
    // either way the whole head lights, the skull and its volume with it
    let bento: std::collections::HashSet<usize> = (0..rig.len()).filter(|j| keys[*j] == 1).collect();
    for rigged in [&none, &bento] {
        let face = mask_of("mFaceLipUpperCenter", rigged);
        for name in ["mFaceLipUpperCenter", "mHead", "HEAD", "mSkull"] {
            assert!(in_mask(&face, rig.names[name]), "{name} lights with the face");
        }
        assert!(!in_mask(&face, rig.names["mNeck"]), "the neck is the body's");
    }
    // a hand: its fingers and its wrist (the palm), not the other hand
    let fingers: std::collections::HashSet<usize> = (0..rig.len()).filter(|j| keys[*j] == 2).collect();
    for rigged in [&none, &fingers] {
        let hand = mask_of("mHandIndex1Left", rigged);
        for name in ["mHandIndex1Left", "mHandThumb1Left", "mWristLeft", "L_HAND"] {
            assert!(in_mask(&hand, rig.names[name]), "{name} lights with the left hand");
        }
        for name in ["mWristRight", "mHandIndex1Right", "mElbowLeft"] {
            assert!(!in_mask(&hand, rig.names[name]), "{name} is not the left hand's");
        }
    }
}

#[test]
fn the_mesh_pick_offers_the_part_its_joint_follows() {
    let (world, _) = fixture();
    let rig = &world.avatar_lib.rig;
    let names = &rig.canonical_names;
    let keys: Vec<usize> = names.iter().map(|n| world_hints::part_key(n)).collect();
    let offer: Vec<bool> = (0..rig.len()).map(|j| world_hints::offerable(keys[j], false)).collect();
    let classes = world_hints::pick_classes(&rig.parent, names);
    let class = |j: usize| (classes[j >> 3] >> ((j & 7) * 4)) & 15;
    let joints: Vec<_> = (0..rig.len()).map(|j| (j, egui::pos2(j as f32, 0.0))).collect();
    let picked = |joint: &str, current: usize| {
        world_hints::picked_part(
            class(rig.names[joint]),
            &joints,
            &rig.parent,
            names,
            &offer,
            current,
            egui::Pos2::ZERO,
        )
        .map(|j| group(&names[j]))
    };
    // posing the Hands: a fitted body's volume, a Bento spine: the body
    assert_eq!(picked("BELLY", 2), Some(0));
    assert_eq!(picked("mSpine2", 2), Some(0));
    // posing the Body: the head is the face's, a wrist its hand's
    assert_eq!(picked("mHead", 0), Some(1));
    assert_eq!(picked("HEAD", 0), Some(1), "the head's volume follows the head");
    assert_eq!(picked("mWristLeft", 0), Some(2));
    // the open group's own: nothing to offer
    assert_eq!(picked("mChest", 0), None);
    assert_eq!(picked("mHead", 1), None, "posing the face, the head is its own");
    // a wing nobody wears: not offered
    assert_eq!(picked("mWing1Left", 0), None);
    assert_eq!(class(rig.len()), 1, "the identity slot is the body's");
    for name in ["mChest", "BELLY", "mSpine2"] {
        assert_eq!(class(rig.names[name]), 1);
    }
    for name in ["mHead", "HEAD", "mSkull", "mEyeLeft"] {
        assert_eq!(class(rig.names[name]), 2);
    }
    for name in ["mWristLeft", "L_HAND", "mHandIndex1Left"] {
        assert_eq!(class(rig.names[name]), 3);
    }
    assert_eq!(class(rig.names["mWristRight"]), 4);
    assert_eq!(class(rig.names["mWing1Left"]), 5);
    assert_eq!(class(rig.names["mTail1"]), 6);
    assert_eq!(class(rig.names["mHindLimb1Left"]), 0);
    // Within the mesh-picked left hand choose the bone at the cursor,
    // never the first finger in the skeleton nor a right-hand finger.
    let finger = rig.names["mHandIndex2Left"];
    let near = world_hints::picked_part(3, &joints, &rig.parent, names, &offer, 0, joints[finger].1);
    assert_eq!(near, Some(finger));
    assert_eq!(
        world_hints::picked_part(0, &joints, &rig.parent, names, &offer, 1, egui::Pos2::ZERO),
        None
    );
    assert_eq!(
        world_hints::picked_part(99, &joints, &rig.parent, names, &offer, 1, egui::Pos2::ZERO),
        None
    );
}

#[test]
fn a_mesh_pick_stands_only_for_its_subject_near_where_it_was_read() {
    let (me, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let stored = Some((me, 13, [400.0, 300.0]));
    let at = |x: f32, y: f32| egui::pos2(x, y);
    assert_eq!(
        world_hints::fresh_pick(stored, me, at(402.0, 301.0)),
        Some(13),
        "a frame or two late, still there"
    );
    assert_eq!(
        world_hints::fresh_pick(stored, me, at(440.0, 300.0)),
        None,
        "the pointer moved on: the bones decide"
    );
    assert_eq!(
        world_hints::fresh_pick(stored, other, at(400.0, 300.0)),
        None,
        "another subject since"
    );
    assert_eq!(world_hints::fresh_pick(None, me, at(400.0, 300.0)), None);
}

#[test]
fn closing_the_window_clears_the_part_and_the_pick() {
    let (world, mut editor) = fixture();
    editor.poser.active = true;
    editor.open = true;
    editor.poser.part_overlay = Some((world.agent_id, [1; 8]));
    editor.poser.pick_wanted = true;
    editor.poser.mesh_pick = Some((world.agent_id, 13, [1.0, 1.0]));
    assert!(editor.poser_part_overlay().is_some() && editor.poser_pick_subject().is_some());
    // "stop posing when closed" off: the pose goes on, the part does not
    editor.poser.stop_on_close = false;
    editor.close_poser();
    assert!(editor.poser.active, "the pose goes on");
    assert_eq!(editor.poser_part_overlay(), None);
    assert_eq!(editor.poser_pick_subject(), None);
    assert_eq!(editor.poser.mesh_pick, None);
}

#[test]
fn an_avatar_not_drawn_as_meshes_keeps_the_bone_outline() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let frame = Frame::new(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let p = crate::theme::Theme::default().palette();
    editor.poser.active = true;
    editor.open = true;
    editor.poser.group = 2;
    // the chest while posing the Hands: the Body
    let chest = proj.project(frame.joints[rig.names["mChest"]].w_axis.truncate()).unwrap().0;
    let mut frames = Frames::new();
    let mut hint_with = |drawn: Option<bool>, editor: &mut Editor| {
        editor.set_poser_meshes_drawn(drawn);
        editor.poser.part_overlay = None;
        let mut hint = world_hints::Hint::None;
        frames.run(
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                egui::Event::PointerMoved(chest),
            ],
            |ui| {
                hint = editor.world_hint(ui.ctx(), &p, &world, &proj, &frame, true);
            },
        );
        hint
    };
    // drawn as meshes: the renderer gets the part, the identity slot (the
    // system body's root vertices) with the body
    assert!(matches!(hint_with(Some(true), &mut editor), world_hints::Hint::Group(_)));
    let (_, mask) = editor.poser_part_overlay().expect("the renderer draws the body");
    let root = rig.len();
    assert!(mask[root >> 5] & (1 << (root & 31)) != 0, "the identity slot is the body's");
    // an impostor, a jelly doll, first person: no meshes drawn, the bones'
    // outline instead (the renderer gets nothing)
    assert!(matches!(hint_with(Some(false), &mut editor), world_hints::Hint::Group(_)));
    assert_eq!(editor.poser_part_overlay(), None, "the bone outline, painted by the poser");
}

#[test]
fn the_drawn_state_and_the_pick_follow_the_subject() {
    let (world, mut editor) = fixture();
    let other = Uuid::from_u128(77);
    editor.poser.subject = world.agent_id;
    // not drawn as meshes: kept while the scene is not asked, so the outline
    // does not flicker; another subject starts unknown (taken as drawn)
    editor.set_poser_meshes_drawn(Some(false));
    editor.set_poser_meshes_drawn(None);
    assert!(!editor.poser_meshes_drawn());
    editor.poser.subject = other;
    assert!(editor.poser_meshes_drawn());
    // the pick: after a switch, the answers still in flight (drawn from the
    // former subject) are dropped, then the new subject's are kept
    editor.poser.pick_wanted = true;
    editor.set_poser_mesh_pick(None);
    for _ in 0..3 {
        editor.set_poser_mesh_pick(Some((13, [10, 10])));
        assert_eq!(editor.poser.mesh_pick, None, "the former subject's answer");
    }
    editor.set_poser_mesh_pick(Some((21, [10, 10])));
    assert_eq!(editor.poser.mesh_pick, Some((other, 21, [10.0, 10.0])));
}
#[test]
fn hand_presets_turn_each_finger_as_the_legacy_viewer_does() {
    // #3602: LLMatrix3(roll, pitch, yaw) (indra/llmath/m3math.cpp, setRot),
    // as FSPoserAnimator::translateRotationToQuaternion builds a preset
    // joint's rotation, turns X, then Y, then Z: Rx · Ry · Rz in column form.
    let presets: HashMap<String, HashMap<String, [f32; 3]>> = serde_json::from_str(include_str!("presets.json")).unwrap();
    let (world, mut editor) = fixture();
    let rig = world.avatar_lib.rig.clone();
    for (index, name) in ["Relaxed", "Tight_Grip", "Flat", "Pointing", "Okay"].into_iter().enumerate() {
        editor.poser.preset = index;
        editor.hand_preset(&rig, None);
        let pose = &editor.poser.sessions[&world.agent_id].pose;
        for (joint, [x, y, z]) in &presets[name] {
            let Some(&bone) = rig.names.get(joint) else { continue };
            let legacy = Quat::from_rotation_x(*x) * Quat::from_rotation_y(*y) * Quat::from_rotation_z(*z);
            // the angle of the difference, read with atan2: acos near 1
            // (angle_between) cannot tell 0.1° from 0 in f32
            let diff = pose.rotations[bone].inverse() * legacy;
            let off = (2.0 * diff.xyz().length().atan2(diff.w.abs())).to_degrees();
            assert!(off < 0.01, "{name} {joint}: {off:.1}° away from the Legacy Viewer's rotation");
        }
    }
}
