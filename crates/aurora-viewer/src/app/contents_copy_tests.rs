use super::*;
use crate::world::inventory::{FetchState, Folder, Inventory};
use aurora_net::build::TaskItem;
use aurora_net::inventory::{InvFolder, InvItem};

#[test]
fn folder_copies_refresh_permissions_and_refuse_missing_items_and_no_copy_worn_contents() {
    use crate::build::{BuildTool, task_inventory};
    use aurora_net::build::perm;
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let idx = world.objects.index_of_uuid(&crate::demo::action_id(118)).unwrap();
    let agent = world.agent_id;
    let object = world.objects.get_mut(idx).unwrap();
    object.owner_id = agent;
    let object = object.full_id;
    let item = TaskItem {
        item_id: Uuid::from_u128(99001),
        parent_id: object,
        owner_id: agent,
        name: "Outfit".into(),
        asset_type: 5,
        inv_type: 18,
        owner_mask: perm::COPY | perm::TRANSFER,
        ..Default::default()
    };
    let mut build = BuildTool::default();
    build.on_task_inventory(&world, object, Some(1), vec![item.clone()]);
    let request = CopyRequest {
        object,
        name: "Packed outfit".into(),
        items: vec![item.clone()],
        transfer: task_inventory::contents_transfer(&world, object, std::slice::from_ref(&item), true).unwrap(),
        wear: None,
    };
    assert!(refresh_copy(request.clone(), &build, &world).unwrap().transfer.moved.is_empty());
    let mut current = item;
    current.owner_mask &= !perm::COPY;
    let script = TaskItem {
        item_id: Uuid::from_u128(99002),
        asset_type: 10,
        ..current.clone()
    };
    build.on_task_inventory(&world, object, Some(2), vec![current.clone(), script]);
    let updated = refresh_copy(request.clone(), &build, &world).unwrap();
    assert_eq!(updated.items, [current]);
    assert_eq!(updated.name, request.name);
    assert_eq!(updated.transfer.moved, [Uuid::from_u128(99001)]);
    assert!(updated.transfer.scripted, "an unselected script requires the stronger move warning");
    assert!(
        !copy_approval_covers(&request.transfer, &updated.transfer),
        "a copy cannot turn into an unapproved move"
    );
    let object = world.objects.get_mut(idx).unwrap();
    object.parent_id = 1;
    object.state = 0x10;
    assert!(refresh_copy(request.clone(), &build, &world).is_err());
    world.objects.get_mut(idx).unwrap().parent_id = 0;
    build.on_task_inventory(&world, request.object, Some(3), Vec::new());
    assert!(
        refresh_copy(request, &build, &world).is_err(),
        "a removed item must never be copied from its stale row"
    );
}

#[test]
fn folder_copy_approval_never_covers_a_new_move_or_a_stronger_script_warning() {
    use crate::build::task_inventory::ContentsTransfer;
    let before = ContentsTransfer {
        moved: vec![Uuid::from_u128(1)],
        scripted: false,
    };
    assert!(copy_approval_covers(&before, &before));
    assert!(copy_approval_covers(
        &before,
        &ContentsTransfer {
            moved: Vec::new(),
            scripted: true
        }
    ));
    assert!(!copy_approval_covers(
        &before,
        &ContentsTransfer {
            moved: before.moved.clone(),
            scripted: true
        }
    ));
    assert!(!copy_approval_covers(
        &before,
        &ContentsTransfer {
            moved: vec![Uuid::from_u128(2)],
            scripted: false
        }
    ));
}

fn fixture(wear: bool) -> (PendingCopy, Inventory, Instant) {
    let now = Instant::now();
    let folder = Uuid::from_u128(100);
    let request = CopyRequest {
        object: Uuid::from_u128(101),
        name: "Box".into(),
        items: vec![
            TaskItem {
                name: "Instructions".into(),
                asset_type: 7,
                inv_type: 7,
                ..Default::default()
            },
            TaskItem {
                name: "Instructions".into(),
                asset_type: 7,
                inv_type: 7,
                ..Default::default()
            },
            TaskItem {
                name: "Outfit".into(),
                asset_type: 5,
                inv_type: 18,
                ..Default::default()
            },
        ],
        transfer: crate::build::task_inventory::ContentsTransfer {
            moved: Vec::new(),
            scripted: false,
        },
        wear: wear.then_some(WearMode::Append),
    };
    let copy = PendingCopy::new(Uuid::from_u128(102), folder, &request, now);
    let mut inv = Inventory::default();
    inv.folders.insert(
        folder,
        Folder {
            info: InvFolder {
                id: folder,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    (copy, inv, now)
}

fn receive(inv: &mut Inventory, folder: Uuid, id: u128, name: &str, asset_type: i32, inv_type: i32) {
    inv.add_items(vec![InvItem {
        id: Uuid::from_u128(id),
        parent: folder,
        name: name.into(),
        asset_type,
        inv_type,
        ..Default::default()
    }]);
}

#[test]
fn contents_copy_waits_for_actual_records_and_matches_types_and_duplicate_names() {
    for wear in [false, true] {
        let (mut copy, mut inv, now) = fixture(wear);
        let folder = copy.folder;
        assert_eq!(copy.poll(&inv, now), CopyState::Pending);
        receive(&mut inv, folder, 201, "Instructions", 7, 7);
        receive(&mut inv, folder, 202, "Outfit", 5, 18);
        receive(&mut inv, folder, 203, "Unrelated", 7, 7);
        receive(&mut inv, folder, 204, "Instructions", 10, 10);
        // A duplicated row/UUID is not a second received copy.
        inv.folders.get_mut(&folder).unwrap().items.push(Uuid::from_u128(201));
        assert_eq!(copy.poll(&inv, now), CopyState::Pending);
        assert_eq!(copy.arrived, 2);
        receive(&mut inv, folder, 205, "Instructions", 7, 7);
        assert_eq!(copy.poll(&inv, now), CopyState::Complete);
        assert_eq!(copy.arrived, 3);
        assert_eq!(copy.wear.is_some(), wear);
    }
}

#[test]
fn contents_copy_never_confirms_unloaded_missing_or_failed_folders_and_times_out_once() {
    let (mut copy, mut inv, now) = fixture(false);
    let folder = copy.folder;
    receive(&mut inv, folder, 201, "Instructions", 7, 7);
    inv.folders
        .get_mut(&folder)
        .unwrap()
        .items
        .extend([Uuid::from_u128(202), Uuid::from_u128(203)]);
    inv.generation += 1;
    assert_eq!(copy.poll(&inv, now + COPY_TIMEOUT - Duration::from_millis(1)), CopyState::Pending);
    let mut copies = ContentsCopies::default();
    copies.copies.push(copy.clone());
    let outcomes = copies.poll(&inv, now + COPY_TIMEOUT);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].1, CopyState::Timeout);
    assert_eq!(outcomes[0].0.arrived, 1);
    assert!(copies.poll(&inv, now + COPY_TIMEOUT).is_empty());
    inv.folders.get_mut(&folder).unwrap().state = FetchState::Fetching;
    assert_eq!(copy.poll(&inv, now), CopyState::Pending);
    inv.failed(&[folder]);
    assert_eq!(copy.poll(&inv, now), CopyState::Unavailable);
    inv.folders.remove(&folder);
    assert_eq!(copy.poll(&inv, now), CopyState::Unavailable);
}

#[test]
fn contents_copy_completion_is_emitted_once_and_keeps_independent_requests_separate() {
    let (first, mut inv, now) = fixture(false);
    let folder = first.folder;
    let mut second = first.clone();
    second.request = Uuid::from_u128(103);
    second.folder = Uuid::from_u128(104);
    inv.folders.insert(
        second.folder,
        Folder {
            info: InvFolder {
                id: second.folder,
                ..Default::default()
            },
            items: Vec::new(),
            children: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    let mut copies = ContentsCopies {
        copies: vec![first, second],
        ..Default::default()
    };
    receive(&mut inv, folder, 201, "Instructions", 7, 7);
    receive(&mut inv, folder, 202, "Instructions", 7, 7);
    receive(&mut inv, folder, 203, "Outfit", 5, 18);
    let outcomes = copies.poll(&inv, now);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].0.request, Uuid::from_u128(102));
    assert_eq!(outcomes[0].1, CopyState::Complete);
    assert_eq!(copies.copies.len(), 1);
    assert!(copies.poll(&inv, now).is_empty());
}

#[test]
fn an_unanswered_folder_creation_reports_timeout_once_and_retains_ownership_of_a_late_reply() {
    let now = Instant::now();
    let id = Uuid::from_u128(301);
    let mut copies = ContentsCopies::default();
    copies.folders.insert(
        id,
        FolderCreation {
            request: CopyRequest {
                object: Uuid::from_u128(302),
                name: "Box".into(),
                items: Vec::new(),
                transfer: crate::build::task_inventory::ContentsTransfer {
                    moved: Vec::new(),
                    scripted: false,
                },
                wear: None,
            },
            folder: Uuid::from_u128(303),
            since: now,
            timed_out: false,
        },
    );
    assert!(copies.expired_folders(now + COPY_TIMEOUT - Duration::from_millis(1)).is_empty());
    assert_eq!(copies.expired_folders(now + COPY_TIMEOUT), [(id, "Box".into())]);
    assert!(copies.expired_folders(now + COPY_TIMEOUT).is_empty());
    assert!(copies.owns(id));
    assert!(
        copies.folders.remove(&id).unwrap().timed_out,
        "a late folder reply must not start transferring contents"
    );
}
