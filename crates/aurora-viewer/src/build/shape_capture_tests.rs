//! The Aurora Create window: its Center and Rotate on every prim
//! (`BuildSettings::creation_pose`), and its shape picker
//! (`BuildTool::capture_shape`): the rule
//! of LLPanelObject::onCopyParams (a prim we may modify, not a mesh), the
//! captured shapes kept in the settings and rezzed with their own size.

use super::*;
use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
use aurora_prim::params::{LL_PCODE_PATH_FLEXIBLE, LL_PCODE_PATH_LINE, LL_PCODE_VOLUME, SculptParams};
use std::sync::Arc;

fn fixture() -> (World, BuildTool, BuildSettings, usize) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let mut tool = BuildTool::default();
    tool.open_build(Tool::Create);
    // a plain prim of the demo, standing alone, made ours to modify
    let idx = world
        .objects
        .iter()
        .find(|(idx, o)| {
            o.pcode == LL_PCODE_VOLUME
                && o.volume.sculpt.is_none()
                && world.objects.parent_of(o).is_none()
                && super::root_of(&world, *idx) == *idx
        })
        .map(|(idx, _)| idx)
        .expect("a demo prim");
    let o = world.objects.get_mut(idx).unwrap();
    o.update_flags = OBJECT_MOVE | OBJECT_MODIFY;
    o.scale = Vec3::new(0.4, 1.2, 3.0);
    o.volume = shapes::shapes()[10].params();
    (world, tool, BuildSettings::default(), idx)
}

#[test]
fn a_prim_we_may_modify_gives_its_shape_and_size_as_the_chosen_shape() {
    let (world, mut tool, mut s, idx) = fixture();
    s.copy_selection = true;
    assert!(tool.capture_shape(&world, &mut s, idx));
    assert_eq!(s.captured_shapes.len(), 1);
    let captured = s.captured_shapes[0];
    assert_eq!(captured.shape, shapes::shapes()[10].raw(), "the torus' own quantized shape");
    assert_eq!(captured.scale, [0.4, 1.2, 3.0]);
    assert_eq!(s.create_shape, shapes::shapes().len() + 2);
    assert!(!s.copy_selection, "a shape chosen unticks Copy selection, as setObjectType does");
    let (shape, rotation, size) = s.create_volume().expect("a prim to rez");
    assert_eq!(
        (shape, rotation, size),
        (captured.shape, Quat::IDENTITY, Some(Vec3::new(0.4, 1.2, 3.0)))
    );
    // the same prim again: the shape already kept is chosen, not doubled
    s.create_shape = 0;
    assert!(tool.capture_shape(&world, &mut s, idx));
    assert_eq!(s.captured_shapes.len(), 1);
    assert_eq!(s.create_shape, shapes::shapes().len() + 2);
}

#[test]
fn the_picker_refuses_what_the_legacy_copy_of_a_shape_refuses() {
    let (mut world, mut tool, mut s, idx) = fixture();
    // no modify right (LLPanelObject::getState's enable_modify)
    world.objects.get_mut(idx).unwrap().update_flags = OBJECT_MOVE;
    assert!(!tool.capture_shape(&world, &mut s, idx));
    assert_eq!(tool.status, aurora_i18n::tr!("build-capture-shape-no-modify"));
    // a mesh (onCopyParams returns) or a sculpted prim (stricter here)
    world.objects.get_mut(idx).unwrap().update_flags = OBJECT_MOVE | OBJECT_MODIFY;
    world.objects.get_mut(idx).unwrap().volume.sculpt = Some(SculptParams::default());
    tool.status.clear();
    assert!(!tool.capture_shape(&world, &mut s, idx));
    assert_eq!(tool.status, aurora_i18n::tr!("build-capture-shape-mesh-or-sculpt"));
    // a tree, grass or an avatar has no prim shape
    world.objects.get_mut(idx).unwrap().volume.sculpt = None;
    world.objects.get_mut(idx).unwrap().pcode = aurora_prim::params::LL_PCODE_LEGACY_TREE;
    tool.status.clear();
    assert!(!tool.capture_shape(&world, &mut s, idx));
    assert_eq!(tool.status, aurora_i18n::tr!("build-capture-shape-not-a-prim"));
    assert!(s.captured_shapes.is_empty());
    assert_eq!(s.create_shape, 0, "a refusal keeps the shape chosen");
}

#[test]
fn a_flexible_prim_gives_its_shape_on_a_line_path() {
    let (mut world, mut tool, mut s, idx) = fixture();
    let mut volume = shapes::shapes()[0].params();
    volume.path.curve_type = LL_PCODE_PATH_FLEXIBLE;
    world.objects.get_mut(idx).unwrap().volume = volume;
    assert!(tool.capture_shape(&world, &mut s, idx));
    assert_eq!(s.captured_shapes[0].shape.path_curve, LL_PCODE_PATH_LINE);
}

#[test]
fn the_window_keeps_twenty_shapes_and_removing_one_keeps_the_choice_on_its_shape() {
    let (mut world, mut tool, mut s, idx) = fixture();
    for k in 0..shapes::MAX_CAPTURED {
        world.objects.get_mut(idx).unwrap().scale = Vec3::splat(0.1 + k as f32 * 0.1);
        assert!(tool.capture_shape(&world, &mut s, idx));
    }
    world.objects.get_mut(idx).unwrap().scale = Vec3::splat(9.0);
    assert!(!tool.capture_shape(&world, &mut s, idx));
    assert_eq!(tool.status, aurora_i18n::tr!("build-capture-shape-full"));
    assert_eq!(s.captured_shapes.len(), shapes::MAX_CAPTURED);
    let first = shapes::shapes().len() + 2;
    // the last one chosen; one removed before it: the choice follows it
    let last = s.captured_shapes[shapes::MAX_CAPTURED - 1];
    s.remove_captured(3);
    assert_eq!(s.create_shape, first + shapes::MAX_CAPTURED - 2);
    assert_eq!(s.create_volume().map(|v| v.0), Some(last.shape));
    // the chosen one removed: back to the cube
    s.remove_captured(shapes::MAX_CAPTURED - 2);
    assert_eq!(s.create_shape, 0);
    // removed after the chosen one, or out of range: the choice stays
    s.create_shape = first;
    s.remove_captured(5);
    s.remove_captured(99);
    assert_eq!(s.create_shape, first);
}

#[test]
fn a_captured_shape_is_rezzed_with_its_shape_and_size() {
    let (world, mut tool, mut s, idx) = fixture();
    let mut world = world;
    assert!(tool.capture_shape(&world, &mut s, idx));
    let point = world.agent.position + Vec3::X * 5.0;
    tool.place(&mut world, &s, Some(point), None, (0.0, 0.0));
    let prims: Vec<_> = tool
        .out
        .iter()
        .filter_map(|command| match command {
            NetCommand::Build(BuildCmd::Add(prim)) => Some(prim),
            _ => None,
        })
        .collect();
    assert_eq!(prims.len(), 1);
    assert_eq!(prims[0].shape, shapes::shapes()[10].raw());
    assert_eq!(prims[0].scale, Vec3::new(0.4, 1.2, 3.0));
    assert_eq!(prims[0].rotation, Quat::IDENTITY);
    assert_eq!(prims[0].pcode, LL_PCODE_VOLUME);
}

#[test]
fn the_captured_shapes_are_saved_with_the_build_settings() {
    let (world, mut tool, mut s, idx) = fixture();
    assert!(tool.capture_shape(&world, &mut s, idx));
    let text = serde_json::to_string(&s).expect("saved");
    let back: BuildSettings = serde_json::from_str(&text).expect("read back");
    assert_eq!(back.captured_shapes, s.captured_shapes);
    assert_eq!(back.create_shape, s.create_shape);
    // settings saved before the picker existed read as no shape captured
    let old: BuildSettings = serde_json::from_str("{}").expect("older settings");
    assert!(old.captured_shapes.is_empty());
}

#[test]
fn the_picker_is_put_down_with_the_create_window() {
    let (mut world, mut tool, mut s, _) = fixture();
    tool.shape_capture = true;
    assert!(tool.capturing_shape());
    tool.close_tool_window(Tool::Create, &mut world, &mut s);
    assert!(!tool.shape_capture);
    assert!(!tool.capturing_shape());
}

#[test]
fn center_and_rotate_place_the_prim_on_the_surface_clicked() {
    let scale = Vec3::new(1.0, 2.0, 3.0);
    let point = Vec3::new(10.0, 20.0, 30.0);
    let wall = Vec3::X;
    let mut s = BuildSettings::default();
    // off: upright, resting on the point, whatever the surface
    let (center, rotation) = s.creation_pose(Quat::IDENTITY, scale, point, wall);
    assert_eq!(rotation, Quat::IDENTITY);
    assert!(center.abs_diff_eq(point + Vec3::Z * 1.5, 1e-5));
    // Center: its centre on the point
    s.create_centers = true;
    let (center, _) = s.creation_pose(Quat::IDENTITY, scale, point, wall);
    assert!(center.abs_diff_eq(point, 1e-5));
    // Rotate: standing on the wall, its top toward the camera
    s.create_centers = false;
    s.create_rotates = true;
    let (center, rotation) = s.creation_pose(Quat::IDENTITY, scale, point, wall);
    assert!((rotation * Vec3::Z).abs_diff_eq(wall, 1e-5));
    assert!(center.abs_diff_eq(point + wall * 1.5, 1e-5), "{center:?}");
    // a shape with its own turn (the sphere's) keeps it on the surface
    let turned = shapes::shapes()[8].rotation();
    let (center, rotation) = s.creation_pose(turned, scale, point, Vec3::Z);
    assert!(rotation.abs_diff_eq(turned, 1e-5));
    assert!(
        center.abs_diff_eq(point + Vec3::Z * 0.5, 1e-5),
        "the turned shape's own height: {center:?}"
    );
}

#[test]
fn a_prim_placed_by_center_and_rotate_is_put_there_once_it_arrives() {
    let (mut world, mut tool, mut s, _) = fixture();
    s.create_centers = true;
    s.create_rotates = true;
    s.new_prim_size = [0.5, 0.5, 0.5];
    let point = world.agent.position + Vec3::X * 5.0;
    let wall = Vec3::Y;
    tool.creation_normal = Some(wall);
    tool.place(&mut world, &s, Some(point), None, (0.0, 0.0));
    assert!(tool.creation_normal.is_none(), "taken by the click");
    let commands = std::mem::take(&mut tool.out);
    let prim = commands
        .iter()
        .find_map(|command| match command {
            NetCommand::Build(BuildCmd::Add(prim)) => Some(prim.clone()),
            _ => None,
        })
        .expect("ObjectAdd");
    assert!((prim.rotation * Vec3::Z).abs_diff_eq(wall, 1e-5), "sent turned to the surface");
    let mut sim = demo_sim::DemoSim::default();
    for event in sim.reply(&world, &BuildCmd::Add(prim.clone())) {
        world.apply(event);
    }
    tool.pick_up_created(&mut world, &s);
    let (handle, update) = tool
        .out
        .iter()
        .find_map(|command| match command {
            NetCommand::Build(BuildCmd::Transform { handle, updates }) => Some((*handle, updates[0])),
            _ => None,
        })
        .expect("the place sent again once the prim arrived");
    assert_eq!(handle, prim.handle);
    let off = world.region_offset(handle).expect("the region");
    assert!(update.position.expect("a position").abs_diff_eq(point - off, 1e-4));
    assert!(update.rotation.expect("a rotation").abs_diff_eq(prim.rotation, 1e-5));
}

#[test]
fn a_prim_placed_as_the_legacy_viewer_does_is_left_where_the_region_puts_it() {
    let (mut world, mut tool, s, _) = fixture();
    let point = world.agent.position + Vec3::X * 5.0;
    tool.creation_normal = Some(Vec3::Y);
    tool.place(&mut world, &s, Some(point), None, (0.0, 0.0));
    let prim = tool
        .out
        .iter()
        .find_map(|command| match command {
            NetCommand::Build(BuildCmd::Add(prim)) => Some(prim.clone()),
            _ => None,
        })
        .expect("ObjectAdd");
    assert_eq!(prim.rotation, Quat::IDENTITY);
    tool.out.clear();
    let mut sim = demo_sim::DemoSim::default();
    for event in sim.reply(&world, &BuildCmd::Add(prim)) {
        world.apply(event);
    }
    tool.pick_up_created(&mut world, &s);
    assert!(
        !tool.out.iter().any(|c| matches!(c, NetCommand::Build(BuildCmd::Transform { .. }))),
        "no place of our own"
    );
}
