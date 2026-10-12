//! Selection regressions for LLToolSelect::handleObjectSelection.

use super::*;
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
