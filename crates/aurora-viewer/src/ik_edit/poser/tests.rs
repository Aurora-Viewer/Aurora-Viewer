use super::*;
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
fn poser_tabs_replace_visible_joints_and_selection_without_editing_the_pose() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let (world, mut editor) = selected_fixture();
    let rig = &world.avatar_lib.rig;
    let frame = editor.editor_frame(&world).unwrap();
    let proj = projector(frame.joints[rig.pelvis].w_axis.truncate());
    let mut frames = Frames::new();
    let p = crate::theme::Theme::default().palette();
    let initial = editor.poser.sessions[&world.agent_id].pose.clone();
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
                .count(),
            "only active tab markers are painted"
        );
        for name in &rig.canonical_names {
            assert_eq!(editor.bone_visible(name), group(name) == tab);
        }
        for segment in editor.skeleton_segments(&world, &proj, &frame) {
            let segments::Source::Bone(bone) = segment.source else {
                panic!("unexpected virtual node");
            };
            assert_eq!(group(&rig.canonical_names[bone]), tab);
        }
        assert!(editor.poser.sessions[&world.agent_id].undo.is_empty());
        assert_eq!(editor.poser.sessions[&world.agent_id].pose.rotations, initial.rotations);
        assert_eq!(editor.poser.sessions[&world.agent_id].pose.positions, initial.positions);
    }
}
#[test]
fn poser_ignores_ik_bindings_virtual_bones_and_prim_picking_even_with_stale_editor_state() {
    use crate::ui::widgets::elide_tests::Frames;
    let (world, mut editor) = fixture();
    let rig = &world.avatar_lib.rig;
    let bone = rig.names["mHead"];
    let node = Uuid::from_u128(0x727_cccc);
    editor.graph.definition = serde_json::from_value(serde_json::json!({
        "nodes": [{"id": node, "name": "Unrelated control", "parent": {"kind": "bone", "bone": "mPelvis"}}],
        "bindings": [{"bone": "mHead", "target": {"kind": "prim", "id": Uuid::from_u128(0x727_dddd)},
                      "contact": [0.1, 0.0, 0.0], "root": "mPelvis"}]
    }))
    .unwrap();
    editor.graph.selected = Some(node);
    editor.graph.error = true;
    editor.links.bone_pick = true;
    editor.picking = true;
    editor.prims.object_handles = true;
    editor.limb_controls = true;
    let definition = editor.graph.definition.clone();
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
    assert!(editor.graph.error, "the poser must not evaluate or replace the unrelated graph");
    assert_eq!(editor.graph.selected, Some(node));
    assert_eq!(editor.graph.definition, definition);
    assert!(
        editor
            .skeleton_segments(&world, &proj, &frame)
            .iter()
            .all(|s| matches!(s.source, segments::Source::Bone(_)))
    );
    assert!(
        frames
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(("poser-joint", bone))))
            .is_some()
    );
    assert!(frames.ctx.memory(|m| m.area_rect(egui::Id::unique(("ik-virtual", node)))).is_none());
    assert!(editor.link_target_definition().is_none());
    assert!(!editor.request().active);
    // Losing a subject during a drag must never fall through to IK authoring.
    editor.poser.sessions.clear();
    editor.commit_manipulation(rig, &frame, true);
    assert!(editor.goals.is_empty());
    assert_eq!(editor.graph.definition, definition);
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
        assert!(editor.graph.definition.bindings.is_empty());
        assert!(editor.bindings.is_empty());
        editor.poser.sessions.get_mut(&world.agent_id).unwrap().history(false);
        assert_eq!(editor.poser.sessions[&world.agent_id].pose.rotations[bone], initial);
    }
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
    assert!(editor.graph.definition.bindings.is_empty());
    editor.select(rig.pelvis, rig, &frame);
    editor.position += Vec3::new(0.2, 0.0, 0.0);
    editor.commit_manipulation(rig, &frame, false);
    assert_attached(rig, &editor.poser.sessions[&world.agent_id].pose, &base, frame.model);
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
    let native = storage::native(&rig, &session.pose, &[], "Pose".into(), 4).unwrap();
    assert!(native.animation.looping);
    assert_eq!(native.animation.base_priority, 4);
    assert!(native.animation.joints.iter().any(|j| j.joint_name == "mHandIndex1Left"));
    editor.pose_operation(&rig, &frame, controls::Operation::ResetAll);
    assert!((0..rig.len()).all(|b| !editor.poser.sessions[&world.agent_id].edited(b)));
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
#[test]
fn poser_subjects_require_ownership_or_consent_and_losing_ownership_drops_only_that_layer() {
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
    assert!(!subjects::allowed(&world, id));
    world.objects.get_mut(index).unwrap().owner_id = world.agent_id;
    assert!(subjects::allowed(&world, id));
    world.avatar_poses.insert(id, world.avatar_poses[&world.agent_id].clone());
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
    service.job = Some(storage::Job::Save(document.clone(), session.pose.clone(), true, true, false));
    for _ in 0..3 {
        tick(&mut service);
    }
    assert_eq!(service.entries.len(), 1);
    let path = service.entries[0].path.clone();
    let bvh = std::fs::read(root.join("poses/Portrait.bvh")).unwrap();
    let imported = aurora_assets::import_animation::parse_bvh_pose(&bvh, world.avatar_lib.skeleton.as_ref().unwrap()).unwrap();
    assert!(
        imported
            .animation
            .joints
            .iter()
            .any(|j| j.joint_name == "mHead" && !j.rot_keys.is_empty())
    );
    service.job = Some(storage::Job::Save(document, session.pose.clone(), false, false, true));
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
