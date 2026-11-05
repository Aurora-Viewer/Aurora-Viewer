//! Take destination: the folder the objects were rezzed from, like
//! handle_take (indra/newview/llviewermenu.cpp).

use super::*;
use crate::world::inventory::{FetchState, Folder, Inventory};
use aurora_net::inventory::InvFolder;
use std::sync::Arc;

const TRASH: u128 = 3;

fn folder(inv: &mut Inventory, id: u128, parent: u128, t: i32, library: bool) {
    inv.folders.insert(
        Uuid::from_u128(id),
        Folder {
            info: InvFolder {
                id: Uuid::from_u128(id),
                parent: Uuid::from_u128(parent),
                name: format!("Folder {id}"),
                type_default: t,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Unknown,
            library,
        },
    );
}

fn inventory() -> Inventory {
    let mut inv = Inventory::default();
    folder(&mut inv, 1, 0, 8, false);
    folder(&mut inv, 2, 1, -1, false); // Builds
    folder(&mut inv, TRASH, 1, 14, false);
    folder(&mut inv, 4, TRASH, -1, false); // trashed
    folder(&mut inv, 5, 1, -1, false); // Other builds
    folder(&mut inv, 6, 0, -1, true); // Library
    inv
}

fn ids(ids: &[u128]) -> Vec<Uuid> {
    ids.iter().map(|&i| Uuid::from_u128(i)).collect()
}

fn take(folders: &[u128]) -> Option<Uuid> {
    take_folder(&inventory(), ids(folders), Uuid::from_u128(TRASH))
}

#[test]
fn objects_rezzed_from_one_folder_go_back_to_it() {
    assert_eq!(take(&[2]), Some(Uuid::from_u128(2)));
    assert_eq!(take(&[2, 2, 2]), Some(Uuid::from_u128(2)));
}

#[test]
fn an_unknown_rez_folder_does_not_count() {
    // handle_take skips a nil FolderID (an object rezzed by a script, or
    // whose properties have not arrived).
    assert_eq!(take(&[0, 2, 0]), Some(Uuid::from_u128(2)));
    assert_eq!(take(&[0, 0]), None);
    assert_eq!(take(&[]), None);
}

#[test]
fn objects_from_different_folders_go_to_objects() {
    assert_eq!(take(&[2, 5]), None);
}

#[test]
fn a_missing_trashed_or_library_folder_is_not_used() {
    assert_eq!(take(&[99]), None);
    assert_eq!(take(&[TRASH]), None);
    assert_eq!(take(&[4]), None);
    assert_eq!(take(&[6]), None);
}

#[test]
fn take_uses_the_rez_folder_and_take_copy_the_objects_folder() {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    let objects = BuildTool::folder_of_type(&world, 6);
    // Shopping, a user folder under Landmarks
    let rezzed_from = crate::demo::u(crate::demo::place::SHOPS_FOLDER);
    assert!(world.inventory.folders.contains_key(&rezzed_from));
    let idx = world.objects.index_of_uuid(&crate::demo::action_id(99614)).unwrap();
    let full_id = world.objects.get(idx).unwrap().full_id;
    let mut tool = BuildTool::default();
    assert_eq!(tool.take_destination(&world, &[idx], false), objects);
    tool.props.insert(
        full_id,
        ObjectProps {
            folder_id: rezzed_from,
            ..Default::default()
        },
    );
    assert_eq!(tool.take_destination(&world, &[idx], false), rezzed_from);
    assert_eq!(tool.take_destination(&world, &[idx], true), objects);
}

#[test]
fn the_demo_coalesces_a_take_of_several_objects_into_one_item() {
    use crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS;
    use aurora_net::NetEvent;
    use aurora_net::build::derez;
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    let key = |id| {
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(id)).unwrap();
        world.objects.get(idx).unwrap().key
    };
    let (a, b) = (key(99610), key(99614));
    assert_eq!(a.region, b.region);
    let items = |local_ids: Vec<u32>, destination| {
        let mut sim = super::demo_sim::DemoSim::default();
        let events = sim.reply(
            &world,
            &BuildCmd::DeRez {
                handle: a.region,
                local_ids,
                destination,
                destination_id: Uuid::from_u128(30),
                group_id: Uuid::nil(),
            },
        );
        events
            .into_iter()
            .find_map(|e| match e {
                NetEvent::InventoryUpdated { items, .. } => Some(items),
                _ => None,
            })
            .expect("a take answers with an inventory item")
    };
    for destination in [derez::TAKE_INTO_AGENT_INVENTORY, derez::ACQUIRE_TO_AGENT_INVENTORY] {
        let both = items(vec![a.local_id, b.local_id], destination);
        assert_eq!(both.len(), 1);
        assert_ne!(both[0].flags & OBJECT_HAS_MULTIPLE_ITEMS, 0);
        assert_eq!(both[0].parent, Uuid::from_u128(30));
        let one = items(vec![b.local_id], destination);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].flags & OBJECT_HAS_MULTIPLE_ITEMS, 0);
    }
}

/// The demo's selection cycle roots A (99610, with its child), B (99612)
/// and C (99614), copyable, with full properties: A ours and rezzed by
/// `REZZER`, B and C someone else's, transferable.
const REZZER: u128 = 0x7E22;

fn save_fixture() -> (World, BuildTool, Vec<usize>) {
    use aurora_net::NetEvent;
    use aurora_net::build::perm;
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    world.apply(NetEvent::ActiveGroup {
        id: Uuid::from_u128(8800),
        name: "Active group".into(),
        title: "Builder".into(),
    });
    let mut tool = BuildTool::default();
    let roots: Vec<usize> = [99610, 99612, 99614]
        .into_iter()
        .map(|id| world.objects.index_of_uuid(&crate::demo::action_id(id)).unwrap())
        .collect();
    for (n, &idx) in roots.iter().enumerate() {
        let o = world.objects.get_mut(idx).unwrap();
        o.update_flags |= crate::ui::context::flags::OBJECT_COPY;
        let props = ObjectProps {
            object_id: o.full_id,
            full: true,
            owner_id: if n == 0 { world.agent_id } else { Uuid::from_u128(0xB0B) },
            owner_mask: if n == 0 { 0 } else { perm::TRANSFER | perm::COPY },
            from_task_id: if n == 0 { Uuid::from_u128(REZZER) } else { Uuid::nil() },
            ..Default::default()
        };
        tool.props.insert(o.full_id, props);
    }
    tool.selection = roots.iter().map(|&idx| world.objects.get(idx).unwrap().key).collect();
    world.ui_sounds.clear();
    (world, tool, roots)
}

fn derezzes(tool: &BuildTool) -> Vec<(aurora_net::RegionHandle, Vec<u32>, u8, Uuid, Uuid)> {
    tool.out
        .iter()
        .filter_map(|command| match command {
            NetCommand::Build(BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                destination_id,
                group_id,
            }) => Some((*handle, local_ids.clone(), *destination, *destination_id, *group_id)),
            _ => None,
        })
        .collect()
}

#[test]
fn save_to_object_contents_derezzes_into_the_rezzing_object() {
    let (mut world, mut tool, roots) = save_fixture();
    let keys: Vec<_> = roots.iter().map(|&idx| world.objects.get(idx).unwrap().key).collect();
    // C may not be copied, and is left out (get_derezzable_objects)
    world.objects.get_mut(roots[2]).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_COPY;
    assert_eq!(tool.save_to_task_inventory_target(&world), Some(Uuid::from_u128(REZZER)));
    tool.save_to_task_inventory(&mut world);
    let sent = derezzes(&tool);
    assert_eq!(sent.len(), 1, "one DeRezObject transaction");
    let (handle, mut local_ids, destination, destination_id, group) = sent[0].clone();
    local_ids.sort_unstable();
    assert_eq!(handle, keys[0].region);
    assert_eq!(local_ids, vec![keys[0].local_id, keys[1].local_id]);
    assert_eq!(destination, derez::SAVE_INTO_TASK_INVENTORY);
    assert_eq!(destination, 2);
    assert_eq!(destination_id, Uuid::from_u128(REZZER));
    assert_eq!(group, Uuid::from_u128(8800), "the active group");
    assert_eq!(world.ui_sounds.last(), Some(&crate::ui_sound::UiSound::ObjectRezOut));
    assert_eq!(tool.selection.len(), 3, "a copy is saved: the selection stays");

    // B no longer transferable by its owner, C copyable again but
    // group-owned without transfer: only A goes
    for (idx, owner, group) in [
        (roots[1], Uuid::from_u128(0xB0B), Uuid::nil()),
        (roots[2], Uuid::nil(), Uuid::from_u128(0x6A)),
    ] {
        world.objects.get_mut(idx).unwrap().update_flags |= crate::ui::context::flags::OBJECT_COPY;
        let p = tool.props.get_mut(&world.objects.get(idx).unwrap().full_id).unwrap();
        (p.owner_id, p.group_id, p.owner_mask) = (owner, group, aurora_net::build::perm::COPY);
    }
    tool.out.clear();
    tool.save_to_task_inventory(&mut world);
    let sent = derezzes(&tool);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, vec![keys[0].local_id]);
}

#[test]
fn save_to_object_contents_is_disabled_without_a_rezzer_or_on_attachments() {
    // the first root names no rezzer: B selected first
    let (mut world, mut tool, roots) = save_fixture();
    tool.selection.rotate_left(1);
    assert_eq!(tool.save_to_task_inventory_target(&world), None);
    tool.save_to_task_inventory(&mut world);
    assert!(derezzes(&tool).is_empty());
    assert!(world.ui_sounds.is_empty());

    // a child of A selected first stands for A
    let child = world.objects.index_of_uuid(&crate::demo::action_id(99611)).unwrap();
    tool.selection = vec![world.objects.get(child).unwrap().key];
    assert_eq!(tool.save_to_task_inventory_target(&world), Some(Uuid::from_u128(REZZER)));

    // properties from the hover (ObjectPropertiesFamily) do not count
    let a = world.objects.get(roots[0]).unwrap().full_id;
    tool.props.get_mut(&a).unwrap().full = false;
    assert_eq!(tool.save_to_task_inventory_target(&world), None);
    tool.props.get_mut(&a).unwrap().full = true;

    // worn: an attachment of ours
    for event in crate::demo::hud::events() {
        world.apply(event);
    }
    let worn = crate::demo::hud::object(31, false).full_id;
    let worn = world.objects.index_of_uuid(&worn).unwrap();
    tool.props.insert(
        world.objects.get(worn).unwrap().full_id,
        ObjectProps {
            full: true,
            owner_id: world.agent_id,
            from_task_id: Uuid::from_u128(REZZER),
            ..Default::default()
        },
    );
    tool.selection = vec![world.objects.get(worn).unwrap().key];
    assert!(tool.attachment_selected(&world));
    assert_eq!(tool.save_to_task_inventory_target(&world), None);
    tool.save_to_task_inventory(&mut world);
    assert!(derezzes(&tool).is_empty());
}

#[test]
fn save_to_object_contents_refuses_roots_in_several_regions() {
    let (mut world, mut tool, roots) = save_fixture();
    // a copy of C in the neighbouring region, selected after A
    let mut far = match crate::demo::selection_cycle_events().pop() {
        Some(aurora_net::NetEvent::ObjectUpdates { mut objects, .. }) => objects.pop().unwrap(),
        _ => unreachable!(),
    };
    let here = world.objects.get(roots[0]).unwrap().key.region;
    let there = here + (256u64 << 32);
    (far.local_id, far.full_id) = (99619, crate::demo::action_id(99619));
    world.apply(aurora_net::NetEvent::ObjectUpdates {
        handle: there,
        objects: vec![far],
    });
    let far = world.objects.index_of_uuid(&crate::demo::action_id(99619)).unwrap();
    assert_eq!(world.objects.get(far).unwrap().key.region, there);
    tool.selection.push(world.objects.get(far).unwrap().key);
    tool.save_to_task_inventory(&mut world);
    assert!(derezzes(&tool).is_empty(), "AcquireErrorObjectSpan, as take");
    assert!(world.ui_sounds.is_empty());
    assert!(!tool.status.is_empty());
}
