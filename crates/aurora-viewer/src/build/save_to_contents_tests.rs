//! Build › Save back to object contents: enable_save_into_task_inventory,
//! LLToolsSaveToObjectInventory and save_object_back_to_contents
//! (indra/newview/llviewermenu.cpp).

use super::*;
use aurora_net::NetEvent;
use aurora_net::build::derez;
use std::sync::Arc;

fn demo_world() -> World {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
        world.apply(event);
    }
    world
}

fn idx(world: &World, id: u32) -> usize {
    world.objects.index_of_uuid(&crate::demo::action_id(id)).expect("demo object")
}

/// The tool with the demo objects `ids` selected, latest first.
fn selected(world: &World, ids: &[u32]) -> BuildTool {
    let mut tool = BuildTool::default();
    for &id in ids {
        tool.selection.push(world.objects.get(idx(world, id)).expect("object").key);
    }
    tool
}

fn rezzed_from(tool: &mut BuildTool, world: &World, id: u32, from: Uuid, full: bool) {
    let object = world.objects.get(idx(world, id)).expect("object").full_id;
    tool.props.insert(
        object,
        ObjectProps {
            object_id: object,
            full,
            from_task_id: from,
            ..Default::default()
        },
    );
}

#[test]
fn the_latest_linkset_selected_names_the_object_it_was_rezzed_from() {
    let world = demo_world();
    let from = Uuid::from_u128(0x5A5E);
    // A's child selected: the linkset's root stands for it
    let mut tool = selected(&world, &[99611]);
    assert_eq!(tool.save_to_contents_target(&world), None, "no properties yet");
    rezzed_from(&mut tool, &world, 99610, from, false);
    assert_eq!(tool.save_to_contents_target(&world), None, "family properties only");
    rezzed_from(&mut tool, &world, 99610, Uuid::nil(), true);
    assert_eq!(tool.save_to_contents_target(&world), None, "not rezzed from contents");
    rezzed_from(&mut tool, &world, 99610, from, true);
    assert_eq!(tool.save_to_contents_target(&world), Some((idx(&world, 99610), from)));
    // the first root node only: C, selected last, was not rezzed from contents
    let mut both = selected(&world, &[99614, 99610]);
    rezzed_from(&mut both, &world, 99610, from, true);
    assert_eq!(both.save_to_contents_target(&world), None);
    let mut both = selected(&world, &[99610, 99614]);
    rezzed_from(&mut both, &world, 99610, from, true);
    assert_eq!(both.save_to_contents_target(&world), Some((idx(&world, 99610), from)));
    // Edit linked parts: individual prims, no root node
    tool.selection_linked = true;
    assert_eq!(tool.save_to_contents_target(&world), None);
}

#[test]
fn an_attachment_is_never_saved_back() {
    let mut world = demo_world();
    let item = Uuid::from_u128(0xA77A);
    world.apply(NetEvent::ObjectUpdates {
        handle: world.main_region.expect("region"),
        objects: vec![crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: item,
            owner_id: world.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: "Worn".into(),
            desc: String::new(),
        })],
    });
    let (worn, _) = world
        .objects
        .iter()
        .find(|(_, o)| o.attachment_item_id() == Some(item))
        .expect("worn object");
    let mut tool = BuildTool::default();
    let o = world.objects.get(worn).expect("object");
    tool.selection.push(o.key);
    tool.props.insert(
        o.full_id,
        ObjectProps {
            full: true,
            from_task_id: Uuid::from_u128(0x5A5E),
            ..Default::default()
        },
    );
    assert_eq!(tool.save_to_contents_target(&world), None);
}

#[test]
fn one_derez_saves_the_first_root_alone_and_keeps_the_selection() {
    let mut world = demo_world();
    world.groups.active = Uuid::from_u128(0x6A0F);
    let from = Uuid::from_u128(0x5A5E);
    let mut tool = selected(&world, &[99611, 99614]);
    rezzed_from(&mut tool, &world, 99610, from, true);
    rezzed_from(&mut tool, &world, 99614, from, true);
    let selection = tool.selection.clone();
    tool.save_to_contents(&mut world);
    let root = world.objects.get(idx(&world, 99610)).expect("root").key;
    match &tool.out[..] {
        [
            NetCommand::Build(BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                destination_id,
                group_id,
            }),
        ] => {
            assert_eq!((*handle, local_ids.as_slice()), (root.region, &[root.local_id][..]));
            assert_eq!(*destination, derez::SAVE_INTO_TASK_INVENTORY);
            assert_eq!(*destination_id, from);
            assert_eq!(*group_id, world.groups.active);
        }
        other => panic!("expected one DeRezObject, got {} commands", other.len()),
    }
    assert_eq!(tool.selection, selection);
    // nothing is sent when the entry is greyed out
    let mut none = selected(&world, &[99612]);
    none.save_to_contents(&mut world);
    assert!(none.out.is_empty());
}

#[test]
fn the_demo_names_the_rezzing_object_and_lists_the_saved_copy_in_its_contents() {
    let world = demo_world();
    let source = world.objects.get(idx(&world, 99612)).expect("source").full_id;
    let rezzed = world.objects.get(idx(&world, 99614)).expect("rezzed").key;
    let mut sim = super::demo_sim::DemoSim::default();
    let sent = sim.reply(
        &world,
        &BuildCmd::DeRez {
            handle: rezzed.region,
            local_ids: vec![rezzed.local_id],
            destination: derez::SAVE_INTO_TASK_INVENTORY,
            destination_id: source,
            group_id: Uuid::nil(),
        },
    );
    let items = sent
        .into_iter()
        .find_map(|e| match e {
            NetEvent::TaskInventory {
                object, result: Ok(items), ..
            } if object == source => Some(items),
            _ => None,
        })
        .expect("the destination's contents again");
    let saved = items.last().expect("the saved copy");
    assert_eq!((saved.asset_type, saved.inv_type, saved.parent_id), (6, 6, source));
    assert!(!saved.name.is_empty());
}

#[test]
fn the_demo_reports_from_task_id_for_an_object_rezzed_from_contents() {
    let mut world = demo_world();
    let source = world.objects.get(idx(&world, 99612)).expect("source").full_id;
    let handle = world.main_region.expect("region");
    let mut sim = super::demo_sim::DemoSim::default();
    let rez = BuildCmd::RezInventory(Box::new(aurora_net::inventory::rez::RezInventory {
        handle,
        group: Uuid::nil(),
        item: aurora_net::inventory::InvItem {
            id: Uuid::from_u128(0xC4A1),
            parent: source,
            name: "Chair".into(),
            asset_type: 6,
            inv_type: 6,
            ..Default::default()
        },
        ray_start: glam::Vec3::new(128.0, 128.0, 40.0),
        ray_end: glam::Vec3::new(128.0, 128.0, 25.0),
        target: Uuid::nil(),
        selected: true,
        remove: false,
        source_folder: source,
        from_task: source,
    }));
    let mut rezzed = None;
    for event in sim.reply(&world, &rez) {
        if let NetEvent::ObjectUpdates { objects, .. } = &event {
            rezzed = objects.first().map(|o| o.full_id);
        }
        world.apply(event);
    }
    let rezzed = rezzed.expect("the rezzed object");
    let key = world
        .objects
        .get(world.objects.index_of_uuid(&rezzed).expect("in the world"))
        .expect("object")
        .key;
    let props = sim
        .reply(
            &world,
            &BuildCmd::Select {
                handle,
                local_ids: vec![key.local_id],
            },
        )
        .into_iter()
        .find_map(|e| match e {
            NetEvent::ObjectProperties(list) => list.into_iter().next(),
            _ => None,
        })
        .expect("properties");
    assert_eq!(props.from_task_id, source);
    // the contents folder is no inventory folder for Take
    assert!(props.folder_id.is_nil());
}
