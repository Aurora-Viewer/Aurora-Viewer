use super::*;
use crate::scene::avatar::AvatarLibrary;
use aurora_net::inventory::{InvFolder, InvItem};
use std::sync::Arc;

const GIVER: Uuid = Uuid::from_u128(0xabcdef);
const ITEM: Uuid = Uuid::from_u128(0x123456);

fn world() -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(1);
    world.inventory.root = Uuid::from_u128(90_000);
    world.inventory.created_folders(vec![InvFolder {
        id: world.inventory.root,
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    crate::world::inventory::demo::seed(&mut world.inventory, world.agent_id);
    world.received_items.enabled = true;
    world
}

fn offer(task: bool, kind: u8) -> InstantMessage {
    let mut bucket = vec![kind];
    if !task {
        bucket.extend_from_slice(ITEM.as_bytes());
    }
    InstantMessage {
        from_agent_id: GIVER,
        from_name: if task { "Gift dispenser" } else { "Test Resident" }.into(),
        to_agent_id: Uuid::from_u128(1),
        dialog: if task { im::TASK_INVENTORY_OFFERED } else { im::INVENTORY_OFFERED },
        session_id: Uuid::from_u128(42),
        message: "Received gift".into(),
        offline: false,
        from_group: false,
        binary_bucket: bucket,
        timestamp: 0,
    }
}

fn item(world: &mut World, kind: i32) {
    let parent = actions::system(&world.inventory, kind).unwrap_or(world.inventory.root);
    world.inventory.add_items(vec![InvItem {
        id: ITEM,
        parent,
        asset_type: kind,
        inv_type: kind,
        owner: world.agent_id,
        name: "Received gift".into(),
        ..Default::default()
    }]);
}

fn deliver(world: &mut World, im: InstantMessage) -> u64 {
    world.apply(NetEvent::InstantMessage(im));
    world.notifications.list.last().expect("offer notification").id
}

fn run(world: &mut World) -> Vec<NetCommand> {
    let mut other = Vec::new();
    for _ in 0..10 {
        for command in world.file_received_items(Instant::now()) {
            match command {
                NetCommand::EditInventory { request, change } => {
                    let result = actions::demo_mutate(&mut world.inventory, world.agent_id, change);
                    assert!(result.is_ok(), "{result:?}");
                    assert!(world.received_items_event(&NetEvent::InventoryEdited { request, result }));
                }
                command => other.push(command),
            }
        }
    }
    other
}

#[test]
fn received_filing_defaults_and_persistence() {
    let mut settings = crate::settings::Settings::default();
    assert!(!settings.sort_received_items);
    let old: crate::settings::Settings = serde_json::from_str("{}").unwrap();
    assert!(!old.sort_received_items);
    settings.sort_received_items = true;
    settings.received_items_since = 123;
    let saved: crate::settings::Settings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert!(saved.sort_received_items);
    assert_eq!(saved.received_items_since, 123);
}

#[test]
fn received_filing_does_not_scan_old_items_or_replay_disabled_offers() {
    let mut w = world();
    item(&mut w, 6);
    let old_parent = w.inventory.items[&ITEM].parent;
    assert!(run(&mut w).is_empty());
    assert!(root(&w.inventory).is_none());
    w.received_items.enabled = false;
    deliver(&mut w, offer(false, 6));
    w.received_items.enabled = true;
    run(&mut w);
    assert_eq!(w.inventory.items[&ITEM].parent, old_parent);
    assert!(root(&w.inventory).is_none());
}

#[test]
fn received_filing_moves_all_direct_asset_types_and_reuses_sender_folders() {
    for kind in [0, 1, 3, 5, 6, 7, 10, 13, 20, 21, 49, 56, 57] {
        let mut w = world();
        item(&mut w, kind);
        let before = w.inventory.items[&ITEM].clone();
        let notification = deliver(&mut w, offer(false, kind as u8));
        w.respond_notification(notification, super::super::notifications::Response::Accept);
        run(&mut w);
        let received = root(&w.inventory).expect("received folder");
        let destination = child(&w.inventory, received, "Test Resident").expect("sender folder");
        let mut expected = before;
        expected.parent = destination;
        assert_eq!(w.inventory.items[&ITEM], expected, "only parent changes, kind {kind}");
        let count = w.inventory.folders.len();
        deliver(&mut w, offer(false, kind as u8));
        run(&mut w);
        assert_eq!(w.inventory.folders.len(), count);
    }
}

#[test]
fn received_filing_preserves_received_folder_descendants() {
    let mut w = world();
    w.inventory.created_folders(vec![InvFolder {
        id: ITEM,
        parent: w.inventory.root,
        name: "Gift folder".into(),
        type_default: -1,
        ..Default::default()
    }]);
    let nested = Uuid::from_u128(0x99999);
    w.inventory.add_items(vec![InvItem {
        id: nested,
        parent: ITEM,
        name: "Child".into(),
        ..Default::default()
    }]);
    deliver(&mut w, offer(false, 8));
    run(&mut w);
    assert!(actions::under(&w.inventory, ITEM, root(&w.inventory).unwrap()));
    assert_eq!(w.inventory.items[&nested].parent, ITEM);
}

#[test]
fn received_filing_handles_offline_offers_but_excludes_pre_activation_backlog() {
    let mut w = world();
    w.received_items.enabled_since = 100;
    item(&mut w, 0);
    let before = w.inventory.items[&ITEM].parent;
    let mut im = offer(false, 0);
    im.offline = true;
    im.timestamp = 99;
    deliver(&mut w, im.clone());
    run(&mut w);
    assert_eq!(w.inventory.items[&ITEM].parent, before);
    im.timestamp = 101;
    deliver(&mut w, im);
    run(&mut w);
    assert!(actions::under(&w.inventory, ITEM, root(&w.inventory).unwrap()));
}

#[test]
fn received_filing_waits_for_item_metadata_and_never_resurrects_rejected_items() {
    let mut w = world();
    let notification = deliver(&mut w, offer(false, 6));
    assert!(run(&mut w).is_empty());
    assert!(root(&w.inventory).is_none());
    w.respond_notification(notification, super::super::notifications::Response::Decline);
    item(&mut w, 6);
    let before = w.inventory.items[&ITEM].parent;
    run(&mut w);
    assert_eq!(w.inventory.items[&ITEM].parent, before);
    assert!(root(&w.inventory).is_none());
}

#[test]
fn received_filing_scripted_accept_uses_owner_legacy_name_and_exact_destination() {
    let mut w = world();
    let notification = deliver(&mut w, offer(true, 6));
    assert!(run(&mut w).is_empty(), "an unaccepted object offer creates no folder");
    let (commands, _) = w.respond_notification(notification, super::super::notifications::Response::Accept);
    assert!(commands.is_empty(), "acceptance waits for the destination");
    assert!(run(&mut w).is_empty(), "the object name is never used as an avatar name");
    w.social.avatar_names.on_legacy(GIVER, "Owner", "Resident");
    let commands = run(&mut w);
    let destination = child(&w.inventory, root(&w.inventory).unwrap(), "Owner Resident").unwrap();
    assert!(
        matches!(&commands[..], [NetCommand::SendImDialog { to: GIVER, dialog: im::TASK_INVENTORY_ACCEPTED, bucket, .. }] if bucket == destination.as_bytes())
    );
    assert!(child(&w.inventory, root(&w.inventory).unwrap(), "Gift dispenser").is_none());
}

#[test]
fn auto_accept_inventory_keeps_received_filing_and_its_delayed_scripted_reply() {
    for task in [false, true] {
        let mut w = world();
        w.inventory_offers.auto_accept = true;
        w.social.avatar_names.on_legacy(GIVER, "Test", "Resident");
        if !task {
            item(&mut w, 6);
        }
        w.apply(NetEvent::InstantMessage(offer(task, 6)));
        assert!(w.notifications.list.is_empty());
        assert!(w.inventory_offers.list.is_empty());
        let mut commands = w.take_social_commands();
        assert!(!commands.iter().any(|c| matches!(c, NetCommand::SendImDialog { .. })));
        commands.extend(run(&mut w));
        let destination = child(&w.inventory, root(&w.inventory).expect("received root"), "Test Resident").expect("sender folder");
        let expected = if task {
            im::TASK_INVENTORY_ACCEPTED
        } else {
            im::INVENTORY_ACCEPTED
        };
        assert!(commands.iter().any(|c| matches!(c,
            NetCommand::SendImDialog { to: GIVER, dialog, bucket, .. }
            if *dialog == expected && bucket == destination.as_bytes()
        )));
        if !task {
            assert_eq!(w.inventory.items[&ITEM].parent, destination);
        }
    }
}

#[test]
fn received_filing_disable_and_timeout_preserve_scripted_acceptance() {
    for disable in [false, true] {
        let mut w = world();
        let id = deliver(&mut w, offer(true, 6));
        w.respond_notification(id, super::super::notifications::Response::Accept);
        let expected_folder = actions::system(&w.inventory, 6).unwrap_or(w.inventory.root);
        w.received_items.enabled = !disable;
        let commands = w.file_received_items(Instant::now() + TIMEOUT + Duration::from_secs(1));
        assert!(
            matches!(&commands[..], [NetCommand::SendImDialog { dialog: im::TASK_INVENTORY_ACCEPTED, bucket, .. }] if bucket == expected_folder.as_bytes())
        );
        assert!(w.file_received_items(Instant::now()).is_empty());
    }
}

#[test]
fn received_filing_failed_move_does_not_drop_the_next_delivery() {
    let mut w = world();
    item(&mut w, 6);
    deliver(&mut w, offer(false, 6));
    run(&mut w);
    w.received_items.seen.clear();
    w.inventory.items.get_mut(&ITEM).unwrap().parent = w.inventory.root;
    deliver(&mut w, offer(false, 6));
    let command = w.file_received_items(Instant::now()).pop().unwrap();
    let NetCommand::EditInventory { request, .. } = command else {
        panic!("move")
    };
    let next = deliver(&mut w, offer(true, 6));
    w.respond_notification(next, super::super::notifications::Response::Accept);
    assert_eq!(w.received_items.pending.len(), 1);
    w.received_items_event(&NetEvent::InventoryEdited {
        request,
        result: Err("simulated failure".into()),
    });
    assert_eq!(w.received_items.pending.len(), 1);
}

#[test]
fn received_filing_failed_folder_creation_preserves_the_original_acceptance() {
    let mut w = world();
    w.social.avatar_names.on_legacy(GIVER, "Owner", "Resident");
    let id = deliver(&mut w, offer(true, 6));
    w.respond_notification(id, super::super::notifications::Response::Accept);
    let command = w.file_received_items(Instant::now()).pop().unwrap();
    let NetCommand::EditInventory { request, .. } = command else {
        panic!("folder creation")
    };
    w.received_items_event(&NetEvent::InventoryEdited {
        request,
        result: Err("simulated failure".into()),
    });
    assert!(matches!(
        w.groups.out.last(),
        Some(NetCommand::SendImDialog {
            dialog: im::TASK_INVENTORY_ACCEPTED,
            ..
        })
    ));
    assert!(w.received_items.pending.is_empty());
}

#[test]
fn received_filing_slow_owner_lookup_does_not_hold_up_another_delivery() {
    let mut w = world();
    let id = deliver(&mut w, offer(true, 6));
    w.respond_notification(id, super::super::notifications::Response::Accept);
    item(&mut w, 0);
    let mut direct = offer(false, 0);
    direct.from_agent_id = Uuid::from_u128(0xabcdee);
    deliver(&mut w, direct);
    run(&mut w);
    assert!(actions::under(&w.inventory, ITEM, root(&w.inventory).unwrap()));
    assert_eq!(w.received_items.pending.len(), 1, "only the unresolved scripted owner remains");
}

#[test]
fn received_filing_rejection_wins_over_a_late_in_flight_move() {
    let mut w = world();
    item(&mut w, 6);
    let notification = deliver(&mut w, offer(false, 6));
    // Acknowledge only the three destination folders; hold the actual move.
    let mut flight = None;
    for _ in 0..5 {
        for command in w.file_received_items(Instant::now()) {
            if let NetCommand::EditInventory { request, change } = command {
                if change.move_only() {
                    flight = Some((request, change));
                    break;
                }
                let result = actions::demo_mutate(&mut w.inventory, w.agent_id, change);
                w.received_items_event(&NetEvent::InventoryEdited { request, result });
            }
        }
        if flight.is_some() {
            break;
        }
    }
    let (request, change) = flight.expect("in-flight gift move");
    w.respond_notification(notification, super::super::notifications::Response::Decline);
    w.take_inventory_edits();
    // Even after a local timeout, a late success must restore the rejection.
    w.file_received_items(Instant::now() + TIMEOUT + Duration::from_secs(1));
    let result = actions::demo_mutate(&mut w.inventory, w.agent_id, change);
    assert!(w.received_items_event(&NetEvent::InventoryEdited { request, result }));
    let trash = actions::system(&w.inventory, 14).expect("Trash");
    let edits = w.take_inventory_edits();
    assert!(matches!(&edits[0].operations[..], [Operation::Patch { id: ITEM, body, .. }] if body["parent_id"].as_uuid() == trash));
}

#[test]
fn received_filing_retires_only_empty_sender_folders_to_trash() {
    let mut w = world();
    let marketplace = Uuid::from_u128(50);
    w.inventory.created_folders(vec![InvFolder {
        id: marketplace,
        parent: w.inventory.root,
        type_default: 50,
        name: "Inbox".into(),
        ..Default::default()
    }]);
    item(&mut w, 6);
    deliver(&mut w, offer(false, 6));
    run(&mut w);
    let received = root(&w.inventory).unwrap();
    let sender = child(&w.inventory, received, "Test Resident").unwrap();
    let trash = actions::system(&w.inventory, 14).unwrap();
    assert_eq!(w.inventory.items[&ITEM].parent, sender);
    let mut moved = w.inventory.items[&ITEM].clone();
    moved.parent = w.inventory.root;
    w.inventory.add_items(vec![moved]);
    let commands = w.file_received_items(Instant::now() + Duration::from_secs(3));
    assert_eq!(commands.len(), 1);
    let NetCommand::EditInventory { request, change } = commands.into_iter().next().unwrap() else {
        panic!("empty sender cleanup")
    };
    assert!(
        matches!(&change.operations[..], [Operation::Patch { id, folder: true, body }] if *id == sender && body["parent_id"].as_uuid() == trash)
    );
    let result = actions::demo_mutate(&mut w.inventory, w.agent_id, change);
    w.received_items_event(&NetEvent::InventoryEdited { request, result });
    assert_eq!(w.inventory.folders[&sender].info.parent, trash);
    assert_eq!(root(&w.inventory), Some(received), "the Received container stays");
    assert_eq!(
        w.inventory.folders[&marketplace].info.parent, w.inventory.root,
        "empty Marketplace stays"
    );
    assert_eq!(w.inventory.items[&ITEM].parent, w.inventory.root);
    // A future delivery recreates the sender destination outside the Trash.
    let mut next = offer(false, 6);
    next.binary_bucket = [vec![6], Uuid::from_u128(100_000).as_bytes().to_vec()].concat();
    let mut next_item = w.inventory.items[&ITEM].clone();
    next_item.id = Uuid::from_u128(100_000);
    w.inventory.add_items(vec![next_item]);
    deliver(&mut w, next);
    run(&mut w);
    assert_ne!(w.inventory.items[&Uuid::from_u128(100_000)].parent, sender);
    assert!(actions::under(&w.inventory, Uuid::from_u128(100_000), received));
}

#[test]
fn received_filing_never_cleans_unloaded_or_disabled_sender_folders() {
    let mut w = world();
    item(&mut w, 6);
    deliver(&mut w, offer(false, 6));
    run(&mut w);
    let received = root(&w.inventory).unwrap();
    let unknown = Uuid::from_u128(200_000);
    w.inventory.created_folders(vec![InvFolder {
        id: unknown,
        parent: received,
        name: "Another Resident".into(),
        type_default: -1,
        ..Default::default()
    }]);
    w.inventory.folders.get_mut(&unknown).unwrap().state = FetchState::Unknown;
    assert!(w.file_received_items(Instant::now() + Duration::from_secs(3)).is_empty());
    assert!(w.inventory.queue.iter().any(|(id, _)| *id == unknown));
    w.inventory.folders.get_mut(&unknown).unwrap().state = FetchState::Fetched;
    w.received_items.enabled = false;
    assert!(w.file_received_items(Instant::now() + Duration::from_secs(6)).is_empty());
    assert_eq!(w.inventory.folders[&unknown].info.parent, received);
}

#[test]
fn received_filing_keeps_a_scripted_destination_until_delivery_arrives() {
    let mut w = world();
    w.social.avatar_names.on_legacy(GIVER, "Owner", "Resident");
    let notification = deliver(&mut w, offer(true, 6));
    w.respond_notification(notification, super::super::notifications::Response::Accept);
    assert_eq!(run(&mut w).len(), 1, "the acceptance was sent");
    let received = root(&w.inventory).unwrap();
    let sender = child(&w.inventory, received, "Owner Resident").unwrap();
    assert!(w.file_received_items(Instant::now() + Duration::from_secs(3)).is_empty());
    assert_eq!(w.inventory.folders[&sender].info.parent, received);
    w.inventory.add_items(vec![InvItem {
        id: ITEM,
        parent: sender,
        ..Default::default()
    }]);
    assert!(w.file_received_items(Instant::now() + Duration::from_secs(6)).is_empty());
    w.inventory.remove(&[ITEM]);
    assert_eq!(w.file_received_items(Instant::now() + Duration::from_secs(9)).len(), 1);
}

#[test]
fn received_filing_overrides_type_destinations_when_kept_before_or_after_the_move() {
    for keep_first in [false, true] {
        let mut w = world();
        item(&mut w, 6);
        let notification = deliver(&mut w, offer(false, 6));
        if keep_first {
            assert!(
                w.respond_notification(notification, super::super::notifications::Response::Accept)
                    .0
                    .is_empty()
            );
        }
        let mut commands = run(&mut w);
        let destination = child(&w.inventory, root(&w.inventory).unwrap(), "Test Resident").unwrap();
        if !keep_first {
            commands = w
                .respond_notification(notification, super::super::notifications::Response::Accept)
                .0;
        }
        assert!(
            matches!(&commands[..], [NetCommand::SendImDialog { dialog: im::INVENTORY_ACCEPTED, bucket, .. }] if bucket == destination.as_bytes())
        );
        assert_eq!(w.inventory.items[&ITEM].parent, destination);
    }
}

#[test]
fn received_filing_disabled_while_keep_waits_preserves_the_original_answer() {
    let mut w = world();
    let notification = deliver(&mut w, offer(false, 6));
    assert!(
        w.respond_notification(notification, super::super::notifications::Response::Accept)
            .0
            .is_empty()
    );
    w.received_items.enabled = false;
    let expected = actions::system(&w.inventory, 6).unwrap_or(w.inventory.root);
    let commands = w.file_received_items(Instant::now());
    assert!(
        matches!(&commands[..], [NetCommand::SendImDialog { dialog: im::INVENTORY_ACCEPTED, bucket, .. }] if bucket == expected.as_bytes())
    );
}
