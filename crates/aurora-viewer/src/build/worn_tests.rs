//! Editing what our avatar wears on its body: the selection space of
//! attachments and their coordinates relative to the attachment point
//! (demo_worn.rs fixture).
use super::*;
use crate::scene::Scene;

fn world() -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for ev in crate::demo::events().into_iter().chain(crate::demo::worn::events()) {
        world.apply(ev);
    }
    world
}

fn local(world: &World, local_id: u32) -> usize {
    world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == local_id)
        .map(|(i, _)| i)
        .expect("demo object")
}

fn worn(world: &World) -> (usize, usize) {
    (local(world, crate::demo::worn::ROOT), local(world, crate::demo::worn::CHILD))
}

#[test]
fn what_we_wear_is_selectable_as_an_attachment_and_other_avatars_things_are_not() {
    let world = world();
    let (root, child) = worn(&world);
    assert_eq!(select_type(&world, root), SelectType::Attachment);
    assert_eq!(select_type(&world, child), SelectType::Attachment);
    assert!(selectable(&world, root) && selectable(&world, child));
    // a click on a linked prim takes the whole worn object
    assert_eq!(BuildTool::unit_of(&world, child, false), Some(world.objects.get(root).unwrap().key));
    // Violet Wolf's hat: an attachment too, but not ours to edit
    let hat = local(&world, 9100);
    assert_eq!(select_type(&world, hat), SelectType::Attachment);
    assert!(!selectable(&world, hat));
}

#[test]
fn the_edit_frame_of_a_worn_object_is_its_attachment_point() {
    let world = world();
    let (root, _) = worn(&world);
    let now = Instant::now();
    let (point, frame) = edit_parent(&world, root, now).expect("attachment point");
    let (pos, rot, hud) = Scene::object_transform(&world, root, now, 0).unwrap();
    assert!(!hud);
    let o = world.objects.get(root).unwrap();
    assert!((point + frame * o.position).abs_diff_eq(pos, 1e-4));
    assert!((frame * o.rotation).abs_diff_eq(rot, 1e-4));
    // LLSelectMgr::getGrid: the world grid of a worn selection is that frame
    let f = manip::frame(&world, &BuildSettings::default(), &[o.key], false, now, None).unwrap();
    assert!(f.grid.world);
    assert!(f.grid.origin.abs_diff_eq(point, 1e-4));
    assert!(f.grid.rotation.abs_diff_eq(frame, 1e-4));
}

#[test]
fn typed_position_of_a_worn_object_is_local_to_its_point_and_within_reach() {
    let mut world = world();
    let (root, child) = worn(&world);
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.click_select(&mut world, &s, Some(root), false);
    tool.out.clear();
    let state = world.objects.get(root).unwrap().state;
    let child_before = world.objects.get(child).unwrap().position;
    let pos = Vec3::new(-0.3, 0.05, 0.2);
    let rotation = Quat::from_rotation_y(0.4);
    tool.set_transform(&mut world, &s, Some(pos), None, Some(rotation), true);
    let o = world.objects.get(root).unwrap();
    assert!(o.position.abs_diff_eq(pos, 1e-5));
    assert!(o.rotation.abs_diff_eq(rotation, 1e-5));
    // still worn on the same point, its linked prim where it was on it
    assert_eq!(o.state, state);
    assert_eq!(world.objects.get(child).unwrap().position, child_before);
    assert!(matches!(&tool.out[..], [NetCommand::Build(BuildCmd::Transform { updates, .. })]
        if updates[0].linked && updates[0].position.unwrap().abs_diff_eq(pos, 1e-5)));
    // MAX_ATTACHMENT_DIST from the point
    tool.set_transform(&mut world, &s, Some(Vec3::new(10.0, 0.0, 0.0)), None, None, true);
    let o = world.objects.get(root).unwrap();
    assert!(o.position.abs_diff_eq(Vec3::new(MAX_ATTACHMENT_DIST, 0.0, 0.0), 1e-4));
}

#[test]
fn a_worn_object_moves_without_the_modify_right_but_its_parts_need_it() {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};
    let mut world = world();
    let (root, child) = worn(&world);
    for idx in [root, child] {
        world.objects.get_mut(idx).unwrap().update_flags = OBJECT_MOVE;
    }
    let mut s = BuildSettings::default();
    let mut tool = BuildTool::default();
    tool.click_select(&mut world, &s, Some(root), false);
    tool.out.clear();
    let pos = Vec3::new(-0.2, 0.0, 0.3);
    tool.set_transform(&mut world, &s, Some(pos), None, None, true);
    assert!(world.objects.get(root).unwrap().position.abs_diff_eq(pos, 1e-5));
    assert_eq!(tool.out.len(), 1);
    s.edit_linked = true;
    tool.click_select(&mut world, &s, Some(child), false);
    tool.out.clear();
    let before = world.objects.get(child).unwrap().position;
    tool.set_transform(&mut world, &s, Some(Vec3::new(0.0, 0.1, 0.2)), None, None, true);
    assert_eq!(world.objects.get(child).unwrap().position, before);
    assert!(tool.out.is_empty());
    world.objects.get_mut(root).unwrap().update_flags |= OBJECT_MODIFY;
    tool.set_transform(&mut world, &s, Some(Vec3::new(0.0, 0.1, 0.2)), None, None, true);
    assert!(
        world
            .objects
            .get(child)
            .unwrap()
            .position
            .abs_diff_eq(Vec3::new(0.0, 0.1, 0.2), 1e-5)
    );
}

#[test]
fn a_worn_selection_is_kept_apart_and_cannot_be_deleted_taken_copied_or_linked() {
    let mut world = world();
    let (root, _) = worn(&world);
    let key = world.objects.get(root).unwrap().key;
    let ground = world
        .objects
        .iter()
        .find(|(idx, _)| selectable(&world, *idx) && select_type(&world, *idx) == SelectType::World)
        .map(|(i, _)| i)
        .unwrap();
    let s = BuildSettings::default();
    let mut tool = BuildTool::default();
    // Shift + click on a world object does not add it to a worn selection
    tool.click_select(&mut world, &s, Some(root), false);
    assert!(tool.attachment_selected(&world) && !tool.hud_selected(&world));
    tool.click_select(&mut world, &s, Some(ground), true);
    assert_eq!(tool.selection, vec![world.objects.get(ground).unwrap().key]);
    assert!(!tool.attachment_selected(&world));
    tool.click_select(&mut world, &s, Some(root), false);
    tool.out.clear();
    tool.delete(&mut world);
    tool.take(&mut world, false);
    tool.take(&mut world, true);
    tool.duplicate(&mut world, Vec3::X, true);
    tool.link(&mut world);
    tool.unlink(&mut world);
    assert!(tool.out.is_empty(), "{:?}", tool.out);
    assert_eq!(tool.selection, vec![key]);
    // nor left behind when the avatar walks away with it
    tool.open_build(Tool::Edit);
    world.agent.position += Vec3::splat(1000.0);
    let limited = BuildSettings {
        limit_select_distance: true,
        ..Default::default()
    };
    tool.prune(&world, &limited, Instant::now());
    assert_eq!(tool.selection, vec![key]);
}
