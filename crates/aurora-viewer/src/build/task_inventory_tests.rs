use super::*;
use crate::world::inventory::{FetchState, Folder};
use aurora_net::inventory::InvFolder;
use std::sync::Arc;

fn fixture() -> (World, Uuid, TaskItem) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let object = world.objects.iter().find(|(_, o)| o.key.local_id == 118).unwrap().1.full_id;
    let item = TaskItem {
        item_id: Uuid::from_u128(99001),
        parent_id: object,
        owner_id: world.agent_id,
        owner_mask: perm::COPY | perm::TRANSFER,
        asset_type: 0,
        ..Default::default()
    };
    (world, object, item)
}

#[test]
fn notecard_description_updates_task_metadata_without_replacing_the_asset_or_permissions() {
    let (world, object, mut item) = fixture();
    item.asset_type = 7;
    item.inv_type = 7;
    item.asset_id = Uuid::from_u128(99002);
    item.name = "Instructions".into();
    item.owner_mask |= perm::MODIFY;
    let idx = world.objects.index_of_uuid(&object).expect("object");
    let mut tool = BuildTool::default();
    tool.contents.objects.entry(object).or_default().items.push(item.clone());
    tool.describe_item(&world, idx, item.item_id, "Updated description");
    assert_eq!(tool.contents.objects[&object].items[0].description, "Updated description");
    assert!(
        matches!(&tool.out[..], [aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::UpdateTaskInventory { item: updated, .. })]
        if updated.item_id == item.item_id && updated.asset_id == item.asset_id
            && updated.name == item.name && updated.owner_mask == item.owner_mask
            && updated.description == "Updated description")
    );
}

#[test]
fn owner_can_copy_without_transfer_and_move_without_copy_but_foreign_items_need_both() {
    let (world, object, mut item) = fixture();
    item.owner_mask = perm::COPY;
    assert_eq!(kind(&world, object, &item, true), Ok(TransferKind::Copy));
    item.owner_mask = 0;
    assert!(matches!(kind(&world, object, &item, true), Ok(TransferKind::Move { .. })));
    assert!(kind(&world, object, &item, false).is_err());
    item.owner_id = Uuid::from_u128(99100);
    item.owner_mask = perm::COPY | perm::TRANSFER;
    assert!(kind(&world, object, &item, false).is_err());
    item.everyone_mask = perm::COPY;
    assert_eq!(kind(&world, object, &item, false), Ok(TransferKind::Copy));
    item.owner_mask = perm::COPY;
    assert!(kind(&world, object, &item, false).is_err());
}

#[test]
fn copy_uses_active_group_and_group_owned_items_still_need_transfer() {
    let (mut world, object, mut item) = fixture();
    world.groups.active = Uuid::from_u128(99101);
    item.group_id = world.groups.active;
    item.group_owned = true;
    item.owner_mask = perm::COPY | perm::TRANSFER;
    assert!(kind(&world, object, &item, false).is_err());
    item.group_mask = perm::COPY;
    assert_eq!(kind(&world, object, &item, false), Ok(TransferKind::Copy));
    world.groups.active = Uuid::nil();
    assert!(kind(&world, object, &item, false).is_err());
    item.everyone_mask = perm::COPY;
    assert_eq!(kind(&world, object, &item, false), Ok(TransferKind::Copy));
    item.owner_mask = 0;
    assert!(kind(&world, object, &item, false).is_err());
}

#[test]
fn no_copy_attachments_and_anonymous_or_mismatched_sources_are_rejected() {
    let (mut world, object, mut item) = fixture();
    let idx = world.objects.index_of_uuid(&object).unwrap();
    let o = world.objects.get_mut(idx).unwrap();
    o.parent_id = 1;
    o.state = 0x10;
    item.owner_mask = 0;
    assert_eq!(
        kind(&world, object, &item, true),
        Err(aurora_i18n::tr!("task-transfer-attachment-no-copy"))
    );
    item.owner_mask = perm::COPY;
    assert_eq!(kind(&world, object, &item, true), Ok(TransferKind::Copy));
    assert!(kind(&world, Uuid::from_u128(99102), &item, true).is_err());
    item.parent_id = Uuid::nil();
    assert!(kind(&world, object, &item, true).is_err());
    item.parent_id = object;
    item.is_folder = true;
    assert!(kind(&world, object, &item, true).is_err());
}

#[test]
fn script_presence_strengthens_confirmation_and_preferences_keep_separate_choices() {
    let (world, object, mut item) = fixture();
    item.owner_mask = 0;
    let mut tool = BuildTool::default();
    tool.props.insert(
        object,
        ObjectProps {
            owner_id: world.agent_id,
            ..Default::default()
        },
    );
    assert_eq!(
        tool.task_transfer_kind(&world, object, &item),
        Ok(TransferKind::Move { scripted: false })
    );
    tool.contents.objects.entry(object).or_default().items.push(TaskItem {
        asset_type: 10,
        ..Default::default()
    });
    assert_eq!(
        tool.task_transfer_kind(&world, object, &item),
        Ok(TransferKind::Move { scripted: true })
    );
    assert!(!approved(None, false));
    assert!(approved(Some(false), false));
    assert!(!approved(Some(false), true));
    assert!(approved(Some(true), true));
    assert!(approved(Some(true), false));
    let mut prefs: crate::ui::inventory::InventoryPreferences = serde_json::from_str("{}").unwrap();
    assert!(prefs.warn_task_move && prefs.warn_scripted_task_move);
    prefs.warn_task_move = false;
    let prefs: crate::ui::inventory::InventoryPreferences = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
    assert!(!prefs.warn_task_move && prefs.warn_scripted_task_move);
}

fn folder(inv: &mut Inventory, n: u128, parent: Uuid, kind: i32, library: bool) -> Uuid {
    let id = Uuid::from_u128(n);
    inv.folders.insert(
        id,
        Folder {
            info: InvFolder {
                id,
                parent,
                type_default: kind,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library,
        },
    );
    id
}

#[test]
fn destinations_accept_regular_and_trash_folders_and_reject_protected_library_outfit_and_cycles() {
    let (_, _, mut item) = fixture();
    let mut inv = Inventory {
        root: Uuid::from_u128(1),
        ..Default::default()
    };
    folder(&mut inv, 1, Uuid::nil(), 8, false);
    let root = inv.root;
    let protected = HashSet::new();
    assert!(destination(&inv, root, &item, &protected));
    for (n, kind) in [(2, -1), (3, 14), (4, 3)] {
        let id = folder(&mut inv, n, root, kind, false);
        assert!(destination(&inv, id, &item, &protected));
        assert!(!destination(&inv, id, &item, &HashSet::from([id])));
    }
    for (n, kind) in [(10, 46), (11, 47), (12, 48), (13, 50), (14, 53), (15, 54)] {
        let id = folder(&mut inv, n, root, kind, false);
        let child = folder(&mut inv, n + 100, id, -1, false);
        assert!(!destination(&inv, child, &item, &protected));
    }
    let lib = folder(&mut inv, 20, root, -1, true);
    assert!(!destination(&inv, lib, &item, &protected));
    let cyclic = folder(&mut inv, 21, Uuid::from_u128(21), -1, false);
    assert!(!destination(&inv, cyclic, &item, &protected));
    let fav = folder(&mut inv, 22, root, 23, false);
    assert!(!destination(&inv, fav, &item, &protected));
    item.asset_type = 3;
    assert!(destination(&inv, fav, &item, &protected));
    item.asset_type = 24;
    item.inv_type = 3;
    assert!(destination(&inv, fav, &item, &protected));
    item.inv_type = 0;
    assert!(!destination(&inv, fav, &item, &protected));
}

#[test]
fn contents_menu_queues_the_same_reviewable_request_without_sending_a_transfer() {
    let (world, object, item) = fixture();
    let idx = world.objects.index_of_uuid(&object).unwrap();
    let mut tool = BuildTool::default();
    tool.copy_item_to_inventory(&world, idx, &item, world.inventory.root);
    assert!(tool.out.is_empty());
    assert!(matches!(&tool.ui.requests[..], [super::super::ui::Request::TransferTask(req)]
        if req.object == object && req.item.item_id == item.item_id && req.approved.is_none()));
}

#[test]
fn demo_delivery_copies_a_copyable_item_and_removes_only_the_no_copy_original() {
    use aurora_net::{NetEvent, build::BuildCmd};
    let (mut world, object, _) = fixture();
    let key = world.objects.get(world.objects.index_of_uuid(&object).unwrap()).unwrap().key;
    let mut sim = super::super::demo_sim::DemoSim::default();
    let list = sim.reply(
        &world,
        &BuildCmd::RequestTaskInventory {
            handle: key.region,
            local_id: key.local_id,
            object_id: object,
        },
    );
    let items = list
        .iter()
        .find_map(|e| match e {
            NetEvent::TaskInventory { result: Ok(items), .. } => Some(items.clone()),
            _ => None,
        })
        .unwrap();
    for copy in [true, false] {
        let task = items
            .iter()
            .find(|i| !i.is_folder && (i.owner_mask & perm::COPY != 0) == copy)
            .unwrap();
        let events = sim.reply(
            &world,
            &BuildCmd::MoveTaskInventory {
                handle: key.region,
                local_id: key.local_id,
                item_id: task.item_id,
                folder_id: world.inventory.root,
            },
        );
        for event in events {
            world.apply(event);
        }
        assert!(
            world
                .inventory
                .items
                .values()
                .any(|i| i.name == task.name && i.asset_type == task.asset_type as i32)
        );
        let list = sim.reply(
            &world,
            &BuildCmd::RequestTaskInventory {
                handle: key.region,
                local_id: key.local_id,
                object_id: object,
            },
        );
        let current = list
            .iter()
            .find_map(|e| match e {
                NetEvent::TaskInventory { result: Ok(items), .. } => Some(items),
                _ => None,
            })
            .unwrap();
        assert_eq!(current.iter().any(|i| i.item_id == task.item_id), copy);
    }
}

#[test]
fn contents_copy_copies_moves_or_refuses_the_whole_object() {
    let (world, object, item) = fixture();
    let card = TaskItem {
        item_id: Uuid::from_u128(99010),
        name: "Card".into(),
        ..item.clone()
    };
    let cube = TaskItem {
        item_id: Uuid::from_u128(99011),
        name: "Cube".into(),
        owner_id: Uuid::from_u128(99012),
        owner_mask: perm::TRANSFER,
        ..item.clone()
    };
    let script = TaskItem {
        item_id: Uuid::from_u128(99013),
        asset_type: 10,
        inv_type: 10,
        ..item.clone()
    };
    let all = contents_transfer(&world, object, &[card.clone(), script.clone()], false).expect("copyable");
    assert!(all.moved.is_empty());
    assert!(all.scripted);
    // move_inv_category_world_to_agent: a no-copy item moves out of our own object
    let ours = contents_transfer(&world, object, &[card.clone(), cube.clone()], true).expect("ours");
    assert_eq!(ours.moved, vec![cube.item_id]);
    assert!(!ours.scripted);
    // ...and refuses the whole copy from someone else's
    let refused = contents_transfer(&world, object, &[card.clone(), cube], false).expect_err("refused");
    assert!(refused.contains("Cube"), "{refused}");
    let folder = TaskItem { is_folder: true, ..card };
    assert_eq!(
        contents_transfer(&world, object, &[folder], true),
        Err(aurora_i18n::tr!("contents-copy-nothing").to_owned())
    );
}

#[test]
fn contents_folder_name_drops_what_an_inventory_name_cannot_hold() {
    assert_eq!(
        contents_folder_name(
            "  Boutique | Gift
 box "
        ),
        "Boutique  Gift box"
    );
    assert_eq!(contents_folder_name(" 	"), aurora_i18n::tr!("common-object"));
    assert_eq!(contents_folder_name(&"x".repeat(80)).chars().count(), 63);
}
