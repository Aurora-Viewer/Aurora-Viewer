//! Selection regressions for LLToolSelect::handleObjectSelection.

use super::*;
use aurora_net::NetEvent;
use std::sync::Arc;

fn fixture() -> (World, BuildTool, BuildSettings) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    let mut tool = BuildTool::default();
    tool.open_build(Tool::Edit);
    (world, tool, BuildSettings::default())
}

fn object(world: &World, local_id: u32) -> usize {
    world.objects.index_of_uuid(&crate::demo::action_id(local_id)).unwrap()
}

fn selected(tool: &BuildTool) -> Vec<u32> {
    tool.selection.iter().map(|key| key.local_id).collect()
}

#[test]
fn first_plain_click_after_enabling_linked_edit_selects_only_the_clicked_part() {
    let (mut world, mut tool, mut settings) = fixture();
    let root = object(&world, 99610);
    let child = object(&world, 99611);
    tool.click_select(&mut world, &settings, Some(root), false);
    tool.set_edit_linked(&mut world, &mut settings, true);
    assert_eq!(tool.selection.len(), 2);
    tool.out.clear();
    tool.click_select(&mut world, &settings, Some(child), false);
    assert_eq!(selected(&tool), [99611]);
    assert!(tool.selection_linked);
    assert!(matches!(&tool.out[..],
        [NetCommand::Build(BuildCmd::Deselect { local_ids: old, .. }),
         NetCommand::Build(BuildCmd::Select { local_ids: new, .. })]
         if old.len() == 2 && old.contains(&99610) && old.contains(&99611) && new == &[99611]));
    tool.set_edit_linked(&mut world, &mut settings, false);
    assert_eq!(selected(&tool), [99610]);
    assert_eq!(tool.highlighted(&world).len(), 2);
}

#[test]
fn linked_extension_toggles_parts_and_plain_click_reduces_a_multiple_selection() {
    let (mut world, mut tool, mut settings) = fixture();
    let root = object(&world, 99610);
    let child = object(&world, 99611);
    tool.set_edit_linked(&mut world, &mut settings, true);
    tool.click_select(&mut world, &settings, Some(root), false);
    tool.click_select(&mut world, &settings, Some(child), true);
    assert_eq!(selected(&tool), [99611, 99610]);
    tool.click_select(&mut world, &settings, Some(child), true);
    assert_eq!(selected(&tool), [99610]);
    tool.click_select(&mut world, &settings, Some(child), true);
    tool.click_select(&mut world, &settings, Some(root), false);
    assert_eq!(selected(&tool), [99610]);
}

#[test]
fn plain_click_on_a_selected_whole_object_keeps_the_other_selected_objects() {
    let (mut world, mut tool, settings) = fixture();
    let first = object(&world, 99610);
    let other = object(&world, 99612);
    tool.click_select(&mut world, &settings, Some(first), false);
    tool.click_select(&mut world, &settings, Some(other), true);
    tool.out.clear();
    tool.click_select(&mut world, &settings, Some(first), false);
    assert_eq!(selected(&tool), [99610, 99612]);
    assert!(tool.out.is_empty());
    assert!(!tool.selection_linked);
}

#[test]
fn face_click_and_shift_toggle_faces_across_prims_and_objects() {
    let (mut world, mut tool, mut settings) = fixture();
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    let a = object(&world, 99610);
    let child = object(&world, 99611);
    let b = object(&world, 99612);
    tool.face_click(&mut world, Some(a), Some(1), false);
    tool.face_click(&mut world, Some(a), Some(3), true);
    tool.face_click(&mut world, Some(child), Some(2), true);
    tool.face_click(&mut world, Some(b), Some(4), true);
    let faces = |tool: &BuildTool, idx| tool.sel_faces(&world, idx);
    assert_eq!(faces(&tool, a), [1, 3]);
    assert_eq!(faces(&tool, child), [2]);
    assert_eq!(faces(&tool, b), [4]);
    assert_eq!(tool.sel_te(&world).len(), 4);
    tool.face_click(&mut world, Some(a), Some(3), true);
    tool.out.clear();
    tool.face_click(&mut world, Some(child), Some(2), true);
    assert_eq!(tool.sel_faces(&world, a), [1]);
    assert_eq!(tool.sel_faces(&world, b), [4]);
    assert!(!tool.is_selected(&world.objects.get(child).unwrap().key));
    assert!(matches!(&tool.out[..], [NetCommand::Build(BuildCmd::Deselect { local_ids, .. })] if local_ids == &[99611]));
    tool.face_click(&mut world, Some(b), Some(4), false);
    assert_eq!(selected(&tool), [99612]);
    assert_eq!(tool.sel_te(&world), [(b, 4)]);
}

#[test]
fn first_shift_face_click_preserves_the_other_faces_of_whole_linksets() {
    let (mut world, mut tool, mut settings) = fixture();
    let root = object(&world, 99610);
    let child = object(&world, 99611);
    let other = object(&world, 99612);
    tool.click_select(&mut world, &settings, Some(root), false);
    tool.click_select(&mut world, &settings, Some(other), true);
    let prims = tool.sel_prims(&world);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(child), Some(2), true);
    assert_eq!(tool.sel_prims(&world).len(), prims.len());
    assert!(!tool.sel_faces(&world, child).contains(&2));
    assert_eq!(tool.sel_faces(&world, root).len(), tool.num_faces(&world, root));
    assert_eq!(tool.sel_faces(&world, other).len(), tool.num_faces(&world, other));
    tool.face_click(&mut world, Some(child), Some(2), true);
    assert_eq!(tool.sel_faces(&world, child).len(), tool.num_faces(&world, child));
    assert!(!settings.edit_linked);
}

#[test]
fn removing_the_last_face_clears_the_selection_and_its_highlight() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(idx), Some(1), false);
    assert_eq!(tool.face_selection(&world, &settings)[&idx], 1 << 1);
    tool.face_click(&mut world, Some(idx), Some(1), true);
    assert!(tool.selection.is_empty());
    assert!(tool.faces.is_empty());
    assert!(tool.face_selection(&world, &settings).is_empty());
    assert!(tool.sel_te(&world).is_empty());
}

#[test]
fn face_highlights_replace_whole_object_outlines_only_in_face_mode() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(idx), Some(1), false);
    tool.face_click(&mut world, Some(idx), Some(3), true);
    assert_eq!(tool.face_selection(&world, &settings)[&idx], (1 << 1) | (1 << 3));
    assert!(tool.wire_selection(&world, &settings).is_empty());
    assert!(tool.silhouette_selection(&world, &settings).is_empty());
    settings.show_highlight = false;
    assert!(tool.face_selection(&world, &settings).is_empty());
    settings.show_highlight = true;
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
    assert!(tool.face_selection(&world, &settings).is_empty());
    assert!(tool.faces.is_empty());
}

#[test]
fn invalid_face_and_shift_click_on_empty_space_preserve_selection() {
    let (mut world, mut tool, mut settings) = fixture();
    let idx = object(&world, 99610);
    tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
    tool.face_click(&mut world, Some(idx), Some(1), false);
    tool.face_click(&mut world, Some(idx), Some(255), false);
    tool.face_click(&mut world, None, None, true);
    assert_eq!(tool.sel_te(&world), [(idx, 1)]);
    tool.face_click(&mut world, None, None, false);
    assert!(tool.sel_te(&world).is_empty());
}

#[test]
fn new_prim_is_selected_once_without_update_owner_id_and_deleted_without_false_warning() {
    let (mut world, mut tool, settings) = fixture();
    tool.expect_created(&world);
    let mut update = crate::demo::action_avatar(false);
    update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
    update.local_id = 99940;
    update.full_id = crate::demo::action_id(99940);
    update.parent_id = 0;
    update.owner_id = Uuid::nil();
    update.update_flags = FLAGS_CREATE_SELECTED | crate::ui::context::flags::OBJECT_YOU_OWNER;
    let handle = world.objects.get(object(&world, 99610)).unwrap().key.region;
    world.apply(NetEvent::ObjectUpdates {
        handle,
        objects: vec![update],
    });
    tool.pick_up_created(&world, &settings);
    assert_eq!(selected(&tool), [99940]);
    let sends = tool.out.len();
    tool.pick_up_created(&world, &settings);
    assert_eq!(tool.out.len(), sends);
    tool.delete(&mut world);
    assert_eq!(tool.status, aurora_i18n::tr!("build-mod-object-s-sent-to-trash", value0 = "1"));
    assert!(tool.selection.is_empty());
}

#[test]
fn preexisting_create_flags_are_never_selected_after_ownership_arrives() {
    let (mut world, mut tool, settings) = fixture();
    let idx = object(&world, 99610);
    let o = world.objects.get_mut(idx).unwrap();
    o.owner_id = Uuid::nil();
    o.update_flags |= FLAGS_CREATE_SELECTED;
    tool.expect_created(&world);
    world.objects.get_mut(idx).unwrap().owner_id = world.agent_id;
    tool.pick_up_created(&world, &settings);
    assert!(tool.selection.is_empty());
}

#[test]
fn primitive_placement_and_deletion_round_trip_through_the_demo_simulator() {
    let (mut world, mut tool, mut settings) = fixture();
    tool.open_build(Tool::Create);
    settings.copy_selection = false;
    let x = world.agent.position.x + 5.0;
    let y = world.agent.position.y + 3.0;
    let point = Vec3::new(x, y, crate::demo::floor_at(x, y));
    tool.mouse_down(&mut world, &mut settings, Some(point), None, None, (0.0, 0.0), Mods::default());
    let mut sim = demo_sim::DemoSim::default();
    for cmd in std::mem::take(&mut tool.out) {
        if let NetCommand::Build(cmd) = cmd {
            for event in sim.reply(&world, &cmd) {
                world.apply(event);
            }
        }
    }
    tool.pick_up_created(&world, &settings);
    assert_eq!(tool.selection.len(), 1);
    let key = tool.selection[0];
    assert!(world.objects.get(world.objects.index_of(&key).unwrap()).unwrap().owner_id.is_nil());
    tool.out.clear();
    tool.delete(&mut world);
    assert_eq!(tool.status, aurora_i18n::tr!("build-mod-object-s-sent-to-trash", value0 = "1"));
    for cmd in std::mem::take(&mut tool.out) {
        if let NetCommand::Build(cmd) = cmd {
            for event in sim.reply(&world, &cmd) {
                world.apply(event);
            }
        }
    }
    assert!(world.objects.index_of(&key).is_none());
}
