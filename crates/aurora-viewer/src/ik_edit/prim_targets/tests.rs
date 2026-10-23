use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::{NetEvent, build::ObjectProps};
use std::sync::Arc;

fn fixture() -> (World, Frame, BuildTool, BuildSettings) {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    let mut objects = demo::hand_ik::objects(0.0, true);
    for object in &mut objects {
        object.update_flags |= crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY;
    }
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    let matrices = lib
        .rig
        .world_with(&lib.rig.local_rot, Vec3::ZERO, &lib.rig.local_pos, &lib.rig.scale);
    world.avatar_poses.insert(
        world.agent_id,
        matrices.iter().zip(&lib.rig.default_world_inv).map(|(m, inv)| *m * *inv).collect(),
    );
    let frame = Frame::new(&world).unwrap();
    let mut build = BuildTool::default();
    build.open_build(BuildMode::Edit);
    for (local, name) in [(demo::hand_ik::LEFT, "Left"), (demo::hand_ik::RIGHT, "Right")] {
        let id = demo::action_id(local);
        build.props.insert(
            id,
            ObjectProps {
                object_id: id,
                name: name.into(),
                ..Default::default()
            },
        );
    }
    (
        world,
        frame,
        build,
        BuildSettings {
            edit_linked: true,
            ..Default::default()
        },
    )
}

fn index(world: &World, local: u32) -> usize {
    world.objects.index_of_uuid(&demo::action_id(local)).unwrap()
}

#[test]
fn target_names_load_without_changing_selection_and_display_renames_without_rewriting_link_resolution() {
    let (world, frame, mut build, settings) = fixture();
    let mut editor = Editor::default();
    editor.toggle();
    let bone = world.avatar_lib.rig.names["mWristLeft"];
    editor.select(bone, &world.avatar_lib.rig, &frame);
    build.props.clear();
    editor.begin_link(&world, &frame, bone, true);
    let id = demo::action_id(demo::hand_ik::LEFT);
    assert!(editor.picked_prim(&world, &build, index(&world, demo::hand_ik::LEFT)));
    let selection = build.selection.clone();
    editor.sync_build(&world, &build, &settings, (0.0, 0.0), Mods::default());
    let requests = editor.take_prim_requests();
    assert!(matches!(&requests[..], [
        aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Select { local_ids: select, .. }),
        aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Deselect { local_ids: deselect, .. }),
    ] if select == &[demo::hand_ik::LEFT] && select == deselect));
    assert_ne!(editor.prim_label(id, ""), id.to_string());
    editor.sync_build(&world, &build, &settings, (0.0, 0.0), Mods::default());
    assert!(editor.take_prim_requests().is_empty());
    for name in ["Left handle", "Renamed handle"] {
        build.props.insert(
            id,
            ObjectProps {
                object_id: id,
                name: name.into(),
                ..Default::default()
            },
        );
        editor.sync_build(&world, &build, &settings, (0.0, 0.0), Mods::default());
        assert_eq!(editor.prim_label(id, ""), name);
        assert_eq!(editor.target_label(&Target::Prim { id, name: String::new() }, &world), name);
        assert_eq!(build.selection, selection);
        assert!(matches!(&editor.graph.definition.bindings[0].target, Target::Prim { name, .. } if name.is_empty()));
    }
}
fn sync(editor: &mut Editor, world: &World, build: &BuildTool, settings: &BuildSettings) {
    editor.sync_build(world, build, settings, (0.0, 0.0), Mods::default());
    editor.sync_prim_draft();
}

#[test]
fn a_single_edited_prim_is_proposed_without_rebinding_and_explicit_picks_win_until_selection_changes() {
    let (mut world, frame, mut build, settings) = fixture();
    let left = index(&world, demo::hand_ik::LEFT);
    let right = index(&world, demo::hand_ik::RIGHT);
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(world.avatar_lib.rig.names["mWristLeft"], &world.avatar_lib.rig, &frame);
    editor.target = Some(demo::action_id(demo::hand_ik::RIGHT));
    editor.attach(&world, &frame);
    let calibration = serde_json::to_value(&editor.bindings[0].definition).unwrap();
    build.click_select(&mut world, &settings, Some(left), false);
    let mut fresh = Editor::default();
    fresh.toggle();
    fresh.prim_picking(true, None);
    assert!(fresh.picked_prim(&world, &build, right));
    sync(&mut fresh, &world, &build, &settings);
    assert_eq!(
        fresh.target,
        Some(demo::action_id(demo::hand_ik::RIGHT)),
        "a pick before the first paint still wins"
    );
    let commands = build.out.len();
    sync(&mut editor, &world, &build, &settings);
    assert_eq!(build.out.len(), commands, "designation adds no object selection or edit command");
    assert_eq!(
        editor.graph.target,
        Target::Prim {
            id: demo::action_id(demo::hand_ik::LEFT),
            name: "Left".into()
        }
    );
    assert_eq!(
        serde_json::to_value(&editor.bindings[0].definition).unwrap(),
        calibration,
        "only Bind changes a saved association"
    );
    let before = build.selection.clone();
    editor.prim_picking(true, None);
    assert!(editor.picked_prim(&world, &build, right));
    assert_eq!(build.selection, before, "read-only picking does not select the linkset root");
    sync(&mut editor, &world, &build, &settings);
    assert_eq!(editor.target, Some(demo::action_id(demo::hand_ik::RIGHT)));
    assert!(!editor.picking);
    build.click_select(&mut world, &settings, Some(right), true);
    sync(&mut editor, &world, &build, &settings);
    assert!(
        editor.prims.edited.is_none(),
        "a multiple selection never chooses its first prim arbitrarily"
    );
    build.click_select(&mut world, &settings, Some(left), false);
    sync(&mut editor, &world, &build, &settings);
    assert_eq!(editor.target, Some(demo::action_id(demo::hand_ik::LEFT)));
}

#[test]
fn object_manipulator_geometry_reserves_overlapping_pose_input_and_respects_permissions() {
    let (mut world, _, mut build, settings) = fixture();
    let left = index(&world, demo::hand_ik::LEFT);
    build.click_select(&mut world, &settings, Some(left), false);
    let point = Scene::object_transform(&world, left, Instant::now(), 0).unwrap().0;
    let eye = point + Vec3::new(3.0, -3.0, 2.0);
    let view = glam::camera::rh::view::look_to_mat4(eye, (point - eye).normalize(), Vec3::Z);
    let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(0.8, 16.0 / 9.0, 0.05);
    build.cam = crate::build::geom::Cam::new(eye, point, projection * view, 0.8, 1600.0, 900.0, 1.0);
    let cursor = (350..550)
        .step_by(4)
        .find_map(|y| {
            (700..900)
                .step_by(4)
                .map(|x| (x as f32, y as f32))
                .find(|p| build.manipulator_at(&world, &settings, *p, Mods::default()))
        })
        .expect("the object manipulator is pickable");
    let mut editor = Editor::default();
    editor.toggle();
    editor.sync_build(&world, &build, &settings, cursor, Mods::default());
    assert!(editor.prims.object_handles);
    let root = index(&world, demo::hand_ik::ROOT);
    world.objects.get_mut(root).unwrap().update_flags = 0;
    editor.sync_build(&world, &build, &settings, cursor, Mods::default());
    assert!(
        !editor.prims.object_handles,
        "a read-only object's inactive handles cannot hide the pose tools"
    );
}

#[test]
fn scene_picking_sets_the_requested_virtual_parent_and_rejects_avatars_and_deleted_destinations() {
    let (world, frame, build, settings) = fixture();
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(world.avatar_lib.rig.pelvis, &world.avatar_lib.rig, &frame);
    let id = Uuid::new_v4();
    editor.graph.definition.nodes.push(crate::world::rig_graph::Node {
        id,
        name: "Control".into(),
        parent: Target::default(),
        offset: Default::default(),
        motion: None,
    });
    let draft = editor.graph.target.clone();
    sync(&mut editor, &world, &build, &settings);
    editor.prim_picking(true, Some(id));
    let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
    assert!(!editor.picked_prim(&world, &build, avatar));
    assert!(editor.picking);
    assert!(editor.picked_prim(&world, &build, index(&world, demo::hand_ik::RIGHT)));
    assert_eq!(
        editor.graph.definition.nodes[0].parent,
        Target::Prim {
            id: demo::action_id(demo::hand_ik::RIGHT),
            name: "Right".into()
        }
    );
    assert_eq!(editor.graph.target, draft, "parent picking must not overwrite the bone target");
    editor.prim_picking(true, Some(id));
    editor.graph.definition.nodes.clear();
    assert!(!editor.picked_prim(&world, &build, index(&world, demo::hand_ik::LEFT)));
    assert!(editor.picking);
}

#[test]
fn editing_a_target_moves_the_live_ik_goal_and_each_editor_closes_independently() {
    let (mut world, frame, mut build, mut settings) = fixture();
    let left = index(&world, demo::hand_ik::LEFT);
    let mut editor = Editor::default();
    editor.toggle();
    editor.select(world.avatar_lib.rig.names["mWristLeft"], &world.avatar_lib.rig, &frame);
    editor.target = Some(demo::action_id(demo::hand_ik::LEFT));
    editor.attach(&world, &frame);
    editor.update_bindings(&world, &frame);
    let before = editor.goals[0].position;
    let calibration = serde_json::to_value(&editor.bindings[0].definition).unwrap();
    build.click_select(&mut world, &settings, Some(left), false);
    let p = Scene::object_transform(&world, left, Instant::now(), 0).unwrap().0;
    build.set_transform(&mut world, &settings, Some(p + Vec3::Z * 0.08), None, None, true);
    editor.update_bindings(&world, &frame);
    assert!(build.open && editor.request().active);
    assert!(editor.goals[0].position.distance(before) > 0.07);
    assert_eq!(serde_json::to_value(&editor.bindings[0].definition).unwrap(), calibration);
    let selection = build.selection.clone();
    editor.toggle();
    assert!(!editor.request().active);
    assert!(build.open && build.selection == selection);
    editor.toggle();
    build.close(&mut world, &mut settings);
    assert!(editor.request().active);
    assert!(!build.open && build.selection.is_empty());
}
