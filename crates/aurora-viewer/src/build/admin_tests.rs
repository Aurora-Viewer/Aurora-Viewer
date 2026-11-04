//! Advanced › Admin › Object on the selection: roots only, by region
//! (force_take_copy, handle_object_lock in llviewermenu.cpp).

use super::*;
use aurora_net::build::{derez, perm, perm_field};
use std::sync::Arc;

fn demo_world() -> World {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    world
}

/// The tool with the demo objects `ids` selected, and their root keys.
fn selected(world: &World, ids: &[u32]) -> (BuildTool, Vec<ObjKey>) {
    let mut tool = BuildTool::default();
    let mut roots = Vec::new();
    for &id in ids {
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(id)).expect("demo object");
        tool.selection.push(world.objects.get(idx).expect("object").key);
        let root = world.objects.get(root_of(world, idx)).expect("root").key;
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    (tool, roots)
}

#[test]
fn admin_requests_name_the_selected_roots_of_each_region() {
    let world = demo_world();
    let (tool, roots) = selected(&world, &[99610, 99614]);
    let groups = tool.admin_selection(&world);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, roots[0].region);
    let mut want: Vec<u32> = roots.iter().map(|k| k.local_id).collect();
    let mut got = groups[0].1.clone();
    want.sort_unstable();
    got.sort_unstable();
    assert_eq!(got, want);
    assert!(BuildTool::default().admin_selection(&world).is_empty());
}

#[test]
fn take_copy_goes_to_god_inventory_and_lock_takes_the_owner_modify_right() {
    let world = demo_world();
    let (mut tool, roots) = selected(&world, &[99614]);
    tool.force_take_copy(&world);
    tool.lock_selection(&world);
    let objects = BuildTool::folder_of_type(&world, 6);
    let sent = std::mem::take(&mut tool.out);
    match &sent[..] {
        [
            NetCommand::Build(BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                destination_id,
                ..
            }),
            NetCommand::Build(BuildCmd::SetPermissions {
                local_ids: locked,
                field,
                set,
                mask,
                ..
            }),
        ] => {
            assert_eq!(
                (*handle, local_ids, *destination, *destination_id),
                (roots[0].region, &vec![roots[0].local_id], derez::FORCE_TO_GOD_INVENTORY, objects)
            );
            assert_eq!(
                (locked, *field, *set, *mask),
                (&vec![roots[0].local_id], perm_field::OWNER, false, perm::MODIFY)
            );
        }
        other => panic!("unexpected commands {other:?}"),
    }
}
