use super::*;
use crate::world::inventory::Folder;
use aurora_net::inventory::InvFolder;
use std::sync::Arc;

const NOTECARD: u128 = 77001;
const SCRIPT: u128 = 77002;
const OBJECT: u128 = 77003;
const NO_COPY: u128 = 77004;
const NO_TRANSFER: u128 = 77005;
const FOLDER: u128 = 77100;
const SUBFOLDER: u128 = 77101;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// The demo plaza with its inventory, the action object 970 (root) and
/// 971 linked to it as a child, both owned and modifiable by the agent.
fn world() -> (World, usize, usize) {
    let mut w = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
        w.apply(ev);
    }
    crate::world::inventory::demo::seed(&mut w.inventory, w.agent_id);
    let root = w.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
    let child = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
    let root_id = w.objects.get(root).unwrap().key.local_id;
    let agent = w.agent_id;
    for (idx, parent) in [(root, 0), (child, root_id)] {
        let o = w.objects.get_mut(idx).unwrap();
        o.parent_id = parent;
        o.owner_id = agent;
        o.update_flags = OBJECT_MODIFY | OBJECT_YOU_OWNER;
    }
    for (folder, parent) in [(FOLDER, w.inventory.root), (SUBFOLDER, id(FOLDER))] {
        w.inventory.folders.insert(
            id(folder),
            Folder {
                info: InvFolder {
                    id: id(folder),
                    parent,
                    name: "Folder".into(),
                    type_default: -1,
                    version: 1,
                    ..Default::default()
                },
                children: if folder == FOLDER { vec![id(SUBFOLDER)] } else { Vec::new() },
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
    }
    let all = UNRESTRICTED;
    let root_folder = w.inventory.root;
    let items = [
        (NOTECARD, 7, all, root_folder),
        (SCRIPT, 10, all, root_folder),
        (OBJECT, 6, all, root_folder),
        (NO_COPY, 7, all & !rules::COPY, root_folder),
        (NO_TRANSFER, 7, all & !rules::TRANSFER, root_folder),
        (77010, 7, all, id(FOLDER)),
        (77011, 1, all & !rules::TRANSFER, id(FOLDER)),
        (77012, 2, all, id(FOLDER)),
        (77013, 24, all, id(FOLDER)),
        (77014, 20, all, id(SUBFOLDER)),
    ]
    .into_iter()
    .map(|(n, kind, mask, parent)| InvItem {
        id: id(n),
        parent,
        asset_type: kind,
        inv_type: kind,
        asset_id: id(n + 500),
        owner: agent,
        base_mask: all,
        owner_mask: mask,
        name: format!("Item {n}"),
        ..Default::default()
    })
    .collect();
    w.inventory.add_items(items);
    (w, root, child)
}

fn sent(drops: &[ContentsDrop]) -> Vec<(u32, Uuid, Option<bool>)> {
    drops
        .iter()
        .map(|d| match &d.command {
            BuildCmd::RezScript {
                local_id, enabled, item, ..
            } => (*local_id, item.item_id, Some(*enabled)),
            BuildCmd::UpdateTaskInventory { local_id, item, .. } => (*local_id, item.item_id, None),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn a_selection_without_ctrl_goes_into_the_root_with_scripts_running_and_ctrl_stops_them() {
    let (w, root, child) = world();
    let root_id = w.objects.get(root).unwrap().key.local_id;
    let ids = [id(NOTECARD), id(SCRIPT)];
    let drops = plan(&w, &ids, child, false, Target::World).unwrap();
    assert_eq!(sent(&drops), vec![(root_id, id(NOTECARD), None), (root_id, id(SCRIPT), Some(true))]);
    assert!(drops.iter().all(|d| d.consume.is_none()));
    let drops = plan(&w, &ids, child, true, Target::World).unwrap();
    assert_eq!(sent(&drops)[1], (root_id, id(SCRIPT), Some(false)));
    // The Contents tab puts everything into the object it lists.
    let child_id = w.objects.get(child).unwrap().key.local_id;
    let drops = plan(&w, &ids, child, false, Target::Panel).unwrap();
    assert!(sent(&drops).iter().all(|(local, ..)| *local == child_id));
}

#[test]
fn objects_and_folders_need_ctrl_in_the_world_and_go_into_the_prim_under_the_pointer() {
    let (w, _, child) = world();
    let child_id = w.objects.get(child).unwrap().key.local_id;
    for ids in [vec![id(NOTECARD), id(OBJECT)], vec![id(FOLDER)]] {
        assert_eq!(plan(&w, &ids, child, false, Target::World).err(), Some("rez-hold-ctrl"));
        assert!(plan(&w, &ids, child, false, Target::Panel).is_ok());
    }
    let drops = plan(&w, &[id(NOTECARD), id(OBJECT)], child, true, Target::World).unwrap();
    assert_eq!(sent(&drops)[1], (child_id, id(OBJECT), None));
}

#[test]
fn one_refused_item_refuses_the_whole_drop() {
    let (mut w, root, child) = world();
    let other = id(88001);
    for idx in [root, child] {
        let o = w.objects.get_mut(idx).unwrap();
        o.owner_id = other;
        o.update_flags = OBJECT_MODIFY;
    }
    let ids = [id(NOTECARD), id(NO_TRANSFER), id(SCRIPT)];
    assert_eq!(plan(&w, &ids, child, true, Target::Panel).err(), Some("rez-transfer-denied"));
    assert_eq!(plan(&w, &ids[..1], child, true, Target::Panel).map(|d| d.len()), Ok(1));
    // Neither Modify nor "allow inventory drop": nothing goes in.
    w.objects.get_mut(root).unwrap().update_flags = 0;
    assert_eq!(plan(&w, &ids[..1], child, true, Target::Panel).err(), Some("rez-contents-denied"));
    // Allow inventory drop takes notecards, never scripts.
    w.objects.get_mut(child).unwrap().update_flags = ALLOW_INVENTORY_DROP;
    assert!(plan(&w, &[id(NOTECARD)], child, true, Target::Panel).is_ok());
    assert_eq!(
        plan(&w, &[id(NOTECARD), id(SCRIPT)], child, true, Target::Panel).err(),
        Some("rez-contents-denied")
    );
}

#[test]
fn a_folder_brings_its_items_and_subfolders_without_links_calling_cards_or_what_cannot_be_given() {
    let (mut w, root, child) = world();
    let drops = plan(&w, &[id(FOLDER), id(77010)], child, true, Target::World).unwrap();
    let mut items: Vec<_> = sent(&drops).into_iter().map(|(_, item, _)| item).collect();
    items.sort();
    // Owned object: the no-transfer sound goes in; the link and the calling
    // card stay out; the notecard carried twice goes in once.
    assert_eq!(items, vec![id(77010), id(77011), id(77014)]);
    for idx in [root, child] {
        let o = w.objects.get_mut(idx).unwrap();
        o.owner_id = id(88001);
        o.update_flags = OBJECT_MODIFY;
    }
    let drops = plan(&w, &[id(FOLDER)], child, true, Target::World).unwrap();
    assert!(
        !sent(&drops).iter().any(|(_, item, _)| *item == id(77011)),
        "no-transfer left out for another owner"
    );
    w.inventory.folders.get_mut(&id(SUBFOLDER)).unwrap().state = FetchState::Fetching;
    assert_eq!(plan(&w, &[id(FOLDER)], child, true, Target::World).err(), Some("rez-incomplete"));
}

#[test]
fn no_copy_items_leave_the_inventory_and_links_or_calling_cards_alone_are_refused() {
    let (w, _, child) = world();
    let drops = plan(&w, &[id(NO_COPY), id(NOTECARD)], child, false, Target::World).unwrap();
    assert_eq!(drops[0].consume, Some(id(NO_COPY)));
    assert_eq!(drops[1].consume, None);
    for single in [77012, 77013] {
        assert_eq!(
            plan(&w, &[id(single)], child, true, Target::Panel).err(),
            Some("rez-contents-denied")
        );
    }
}

#[test]
fn a_worn_wearable_is_refused_and_attachments_take_only_unrestricted_items() {
    let (mut w, root, _) = world();
    let shirt = id(77020);
    let cof = crate::world::appearance::cof(&w.inventory).expect("demo current outfit");
    w.inventory.add_items(vec![
        InvItem {
            id: shirt,
            parent: w.inventory.root,
            asset_type: AT_CLOTHING,
            owner: w.agent_id,
            base_mask: UNRESTRICTED,
            owner_mask: UNRESTRICTED,
            ..Default::default()
        },
        InvItem {
            id: id(77021),
            parent: cof,
            asset_type: AT_LINK,
            asset_id: shirt,
            ..Default::default()
        },
    ]);
    assert_eq!(plan(&w, &[shirt], root, true, Target::Panel).err(), Some("rez-worn-contents"));
    // An attachment, edited from the Contents tab.
    let avatar = w.objects.index_of_uuid(&w.agent_id).unwrap();
    let avatar_id = w.objects.get(avatar).unwrap().key.local_id;
    w.objects.get_mut(root).unwrap().parent_id = avatar_id;
    assert!(plan(&w, &[id(NOTECARD)], root, true, Target::Panel).is_ok());
    assert_eq!(plan(&w, &[id(NOTECARD)], root, true, Target::World).err(), Some("rez-no-surface"));
    w.inventory.items.get_mut(&id(NOTECARD)).unwrap().base_mask = rules::COPY;
    assert_eq!(
        plan(&w, &[id(NOTECARD)], root, true, Target::Panel).err(),
        Some("rez-contents-denied")
    );
}

#[test]
fn the_check_is_cached_until_ctrl_the_inventory_or_the_rights_change() {
    let (mut w, root, child) = world();
    let mut check = PlanCheck::default();
    let ids = [id(OBJECT)];
    assert_eq!(check.check(&w, &ids, child, false, Target::World), Err("rez-hold-ctrl"));
    assert_eq!(check.check(&w, &ids, child, true, Target::World), Ok(1));
    w.objects.get_mut(root).unwrap().update_flags = 0;
    assert_eq!(check.check(&w, &ids, child, true, Target::World), Err("rez-contents-denied"));
    w.objects.get_mut(root).unwrap().update_flags = OBJECT_MODIFY | OBJECT_YOU_OWNER;
    w.inventory.consume_item(id(OBJECT));
    assert_eq!(check.check(&w, &ids, child, true, Target::World), Err("rez-incomplete"));
}

#[test]
fn dropped_scripts_send_the_active_group_without_replacing_the_items_permission_group() {
    let (mut w, _, child) = world();
    for event in crate::demo::demo_reply(&aurora_net::NetCommand::RequestGroups) {
        w.apply(event);
    }
    let land = w.groups.groups[0].id;
    w.parcel = Some(Arc::new(aurora_net::ParcelInfo {
        group_id: land,
        ..Default::default()
    }));
    let source_group = id(8802);
    w.inventory.items.get_mut(&id(SCRIPT)).unwrap().group_id = source_group;
    for active in [id(8800), id(8801), Uuid::nil()] {
        w.apply(aurora_net::NetEvent::ActiveGroup {
            id: active,
            name: "Active group".into(),
            title: "Builder".into(),
        });
        assert_eq!(w.group_for_rez(true), land);
        for target in [Target::World, Target::Panel] {
            for ctrl in [false, true] {
                let drops = plan(&w, &[id(SCRIPT)], child, ctrl, target).unwrap();
                assert_eq!(drops.len(), 1);
                let BuildCmd::RezScript {
                    local_id,
                    enabled,
                    group_id,
                    item,
                    ..
                } = &drops[0].command
                else {
                    panic!("expected a script drop");
                };
                assert_eq!(*group_id, active);
                assert_eq!(*enabled, !ctrl);
                let into = if matches!(target, Target::World) {
                    super::linkset(&w, child).unwrap().0
                } else {
                    child
                };
                let object = w.objects.get(into).unwrap();
                assert_eq!((*local_id, item.parent_id), (object.key.local_id, object.full_id));
                let source = &w.inventory.items[&id(SCRIPT)];
                assert_eq!(
                    (item.item_id, item.asset_id, item.group_id, item.owner_mask),
                    (source.id, source.asset_id, source_group, source.owner_mask)
                );
                assert!(drops[0].consume.is_none());
            }
        }
    }
}
