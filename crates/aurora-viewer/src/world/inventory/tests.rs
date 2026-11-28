use super::{actions::*, *};

fn fixture() -> Inventory {
    let mut inv = Inventory {
        root: Uuid::from_u128(1),
        ..Default::default()
    };
    inv.folders.insert(
        inv.root,
        Folder {
            info: InvFolder {
                id: inv.root,
                type_default: 8,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    demo::seed(&mut inv, Uuid::from_u128(2));
    inv
}
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn an_open_folder_whose_read_failed_waits_before_it_is_asked_again() {
    let mut inv = fixture();
    let root = inv.root;
    let now = std::time::Instant::now();
    inv.folders.get_mut(&root).expect("root").state = FetchState::Unknown;
    assert!(inv.shown_needs_fetch(root, now), "never read: asked at once");
    inv.request(root);
    assert!(!inv.shown_needs_fetch(root, now), "being read");
    inv.failed_at_time(&[root], now);
    assert_eq!(inv.folders[&root].state, FetchState::Failed);
    // not every frame: only once the pause is over
    assert!(!inv.shown_needs_fetch(root, now));
    assert!(!inv.shown_needs_fetch(root, now + SHOWN_RETRY / 2));
    assert!(inv.shown_needs_fetch(root, now + SHOWN_RETRY));
    // a folder that failed with no time recorded is not left behind
    inv.failed_at.clear();
    assert!(inv.shown_needs_fetch(root, now));
    // a read folder and an unknown id are never asked for
    inv.folders.get_mut(&root).expect("root").state = FetchState::Fetched;
    assert!(!inv.shown_needs_fetch(root, now + SHOWN_RETRY));
    assert!(!inv.shown_needs_fetch(id(999_999), now));
}

#[test]
fn shown_folder_retries_keep_the_background_owner_and_the_open_view_pause() {
    let mut inv = fixture();
    let root = inv.root;
    let now = std::time::Instant::now();
    inv.folders.get_mut(&root).expect("root").state = FetchState::Fetching;
    inv.failed_at_time(&[root], now);
    assert_eq!(inv.folders[&root].state, FetchState::Failed);
    inv.request_shown(root, now + SHOWN_RETRY / 2);
    assert!(inv.queue.is_empty(), "no retry each frame before the pause");
    inv.request_shown(root, now + SHOWN_RETRY);
    inv.request_shown(root, now + SHOWN_RETRY);
    assert_eq!(inv.queue, [(root, false)], "one read after the pause");
    assert_eq!(inv.folders[&root].state, FetchState::Fetching);
    inv.queue.clear();

    inv.failed_at_time(&[root], now);
    inv.retrying_folders.insert(root);
    assert!(!inv.shown_needs_fetch(root, now + SHOWN_RETRY * 100));
    inv.request_shown(root, now + SHOWN_RETRY * 100);
    inv.retry(root);
    assert!(inv.queue.is_empty(), "neither open views nor clicks steal a background retry");
    assert_eq!(inv.folders[&root].state, FetchState::Failed);
    inv.retrying_folders.remove(&root);
    inv.too_big.insert(root);
    assert!(!inv.shown_needs_fetch(root, now + SHOWN_RETRY * 100));
    inv.request_shown(root, now + SHOWN_RETRY * 100);
    assert!(inv.queue.is_empty(), "an over-limit folder is not polled");

    inv.too_big.remove(&root);
    inv.request_shown(root, now + SHOWN_RETRY * 100);
    assert_eq!(inv.queue, [(root, false)], "the view resumes once recovery releases ownership");
}

#[test]
fn explicit_folder_open_retries_without_waiting_for_the_view_poll() {
    let mut inv = fixture();
    let root = inv.root;
    inv.folders.get_mut(&root).expect("root").state = FetchState::Fetching;
    inv.failed_at_time(&[root], std::time::Instant::now());
    assert_eq!(inv.folders[&root].state, FetchState::Failed);
    inv.request(root);
    assert!(inv.queue.is_empty(), "ordinary automatic demand does not restart failures");
    inv.retrying_folders.insert(root);
    inv.open_folder(root);
    inv.open_folder(root);
    assert_eq!(inv.queue, [(root, false)], "an explicit open keeps main's immediate retry");
    assert!(inv.retrying_folders.contains(&root), "the walker reconciles the interactive read");
}

#[test]
fn only_an_accepted_snapshot_can_acknowledge_a_previously_unknown_item() {
    let mut inv = fixture();
    let root = inv.root;
    let known = id(987_654);
    let unknown = id(987_655);
    let never_requested = id(987_656);
    let failed = id(987_657);
    inv.add_items(vec![InvItem {
        id: known,
        parent: root,
        ..Default::default()
    }]);
    inv.claim_item_reads(&mut vec![unknown], false, std::time::Instant::now());
    inv.items_failed(&[failed]);
    let version = inv.folders[&root].info.version;
    let snapshot = |version| FolderContents {
        folder_id: root,
        owner_id: Uuid::nil(),
        version,
        folders: vec![],
        items: [known, unknown, never_requested, failed]
            .into_iter()
            .map(|id| InvItem {
                id,
                parent: root,
                ..Default::default()
            })
            .collect(),
    };
    let refetches = |inv: &Inventory| inv.queue.iter().filter(|(id, _)| *id == root).count();
    assert_eq!(inv.apply_snapshots(vec![snapshot(version - 1)], true), vec![root]);
    assert!(
        inv.take_accepted_snapshot_items().is_empty(),
        "a rejected snapshot acknowledges nothing"
    );
    assert!(inv.items.contains_key(&known));
    assert_eq!(inv.folders[&root].state, FetchState::Fetching);
    assert_eq!(refetches(&inv), 1, "the stale folder is read again");
    // The refetch's answer carries the server's version, even below the
    // local one (AISUpdate::parseCategory): taken, and not read again, or a
    // folder whose local version ran ahead would be read forever.
    assert!(inv.apply_snapshots(vec![snapshot(version - 1)], true).is_empty());
    assert_eq!(inv.folders[&root].state, FetchState::Fetched);
    assert_eq!(inv.folders[&root].info.version, version - 1);
    assert_eq!(refetches(&inv), 1, "no second refetch");
    assert_eq!(
        inv.take_accepted_snapshot_items(),
        vec![unknown, failed],
        "acknowledge outstanding reads and cooldowns without canceling refreshes or forwarding unrelated items"
    );
}

#[test]
fn multiple_inventory_favorites_keep_independent_flags_and_batch_originals() {
    let mut inv = fixture();
    let mut selected = vec![id(8000), id(8001)];
    for (offset, kind) in [1, 3, 5, 6, 10].into_iter().enumerate() {
        for n in 0..2 {
            let target = id(9000 + offset as u128 * 2 + n);
            inv.add_items(vec![InvItem {
                id: target,
                parent: id(8000),
                asset_type: kind,
                ..Default::default()
            }]);
            selected.push(target);
        }
    }
    inv.add_items(vec![InvItem {
        id: id(9999),
        parent: id(8001),
        asset_type: 24,
        asset_id: selected[2],
        ..Default::default()
    }]);
    selected.push(id(9999));
    // Duplicate aliases produce one PATCH of the original.
    let change = set_favorites(&inv, &selected, true).expect("batch favorites");
    assert_eq!(change.operations.len(), 12);
    demo_mutate(&mut inv, id(2), change).expect("add all");
    assert!(selected.iter().all(|id| is_favorite(&inv, *id)));
    let remove = toggle_favorite(&inv, selected[2]).expect("remove just one");
    demo_mutate(&mut inv, id(2), remove).expect("remove");
    assert!(!is_favorite(&inv, selected[2]));
    assert!(is_favorite(&inv, selected[3]), "same asset type remains a favorite");
    // A mixed selection sets a common state, rather than toggling each row.
    let change = set_favorites(&inv, &selected, true).expect("mixed add");
    assert_eq!(change.operations.len(), 1);
    demo_mutate(&mut inv, id(2), change).expect("restore");
    inv.items.get_mut(&id(9999)).expect("link").favorite = true;
    let change = set_favorites(&inv, &selected, false).expect("batch remove");
    assert_eq!(change.operations.len(), 13, "also clean the stale link flag");
    demo_mutate(&mut inv, id(2), change).expect("remove all");
    assert!(selected.iter().all(|id| !is_favorite(&inv, *id)));
    assert!(!inv.items[&id(9999)].favorite);
}

#[test]
fn viewer_made_calling_cards_take_their_resident_from_the_description() {
    let mut inv = fixture();
    let me = id(2);
    let friend = Uuid::parse_str("8f3b2c1a-0d4e-4b5f-9a6c-7e8d9f0a1b2c").expect("key");
    let card = |n: u128, desc: &str, creator: Uuid| InvItem {
        id: id(n),
        parent: id(8000),
        asset_type: 2,
        desc: desc.into(),
        creator,
        owner: me,
        ..Default::default()
    };
    // A card made by a viewer: created by us, the resident's key as description.
    inv.add_items(vec![card(9100, &friend.to_string(), me)]);
    assert_eq!(inv.items[&id(9100)].creator, friend);
    // A friendship card made by the server keeps its creator.
    inv.add_items(vec![card(9101, "", friend)]);
    assert_eq!(inv.items[&id(9101)].creator, friend);
    // Text that is not a key, and a key in another kind of item, change nothing.
    inv.add_items(vec![card(9102, "my best friend", me)]);
    assert_eq!(inv.items[&id(9102)].creator, me);
    let mut note = card(9103, &friend.to_string(), me);
    note.asset_type = 7;
    inv.add_items(vec![note]);
    assert_eq!(inv.items[&id(9103)].creator, me);
    // The simulator resends the card as created by us: still the friend's,
    // and the record is not seen as changed.
    let version = inv.folders[&id(8000)].info.version;
    inv.server_updated(Vec::new(), vec![card(9100, &friend.to_string(), me)]);
    assert_eq!(inv.items[&id(9100)].creator, friend);
    assert_eq!(inv.folders[&id(8000)].info.version, version);
}

#[test]
fn partial_inventory_updates_preserve_favorites_until_an_authoritative_removal() {
    let mut inv = fixture();
    inv.items.insert(
        id(9000),
        InvItem {
            id: id(9000),
            parent: id(8000),
            favorite: true,
            favorite_known: true,
            ..Default::default()
        },
    );
    inv.folders.get_mut(&id(8001)).expect("folder").info.favorite = true;
    // Older inventory services and UDP notifications carry ordinary item
    // fields without AIS metadata; a rename/move must not clear a favorite.
    let legacy = aurora_net::inventory::parse_fetch_response(&aurora_llsd::llsd_map! {
        "folders" => Llsd::Array(vec![aurora_llsd::llsd_map! {
            "folder_id" => id(8000), "version" => 20,
            "items" => Llsd::Array(vec![aurora_llsd::llsd_map! { "item_id" => id(9000), "parent_id" => id(8000), "name" => "Renamed", "favorite" => Llsd::Undef }]),
            "categories" => Llsd::Array(vec![aurora_llsd::llsd_map! { "category_id" => id(8001), "parent_id" => id(8000), "name" => "Folder" }])
        }])
    });
    inv.apply(legacy);
    assert!(is_favorite(&inv, id(9000)));
    assert!(is_favorite(&inv, id(8001)));
    inv.server_updated(
        Vec::new(),
        vec![InvItem {
            id: id(9000),
            parent: id(8001),
            name: "Moved".into(),
            ..Default::default()
        }],
    );
    assert!(is_favorite(&inv, id(9000)));
    // A complete AIS record with no favorite is an explicit removal.
    inv.add_items(vec![InvItem {
        id: id(9000),
        parent: id(8001),
        favorite_known: true,
        ..Default::default()
    }]);
    assert!(!is_favorite(&inv, id(9000)));
    let mut folder = inv.folders[&id(8001)].info.clone();
    folder.favorite = false;
    folder.favorite_known = true;
    inv.server_updated(vec![folder], Vec::new());
    assert!(!is_favorite(&inv, id(8001)));
}

#[test]
fn inventory_favorites_toggle_originals_through_item_and_folder_links() {
    for kind in [1, 3, 5, 6, 10] {
        let mut inv = fixture();
        inv.add_items(vec![
            InvItem {
                id: id(9000),
                parent: id(8000),
                asset_type: kind,
                ..Default::default()
            },
            InvItem {
                id: id(9001),
                parent: id(8001),
                asset_type: 24,
                asset_id: id(9000),
                ..Default::default()
            },
        ]);
        for expected in [true, false] {
            let change = toggle_favorite(&inv, id(9001)).expect("original favorite");
            assert_eq!(change.refresh, vec![id(8000)], "refresh the original's parent");
            assert!(
                matches!(&change.operations[..], [aurora_net::inventory::operations::Operation::Patch { id: target, folder: false, .. }] if *target == id(9000))
            );
            demo_mutate(&mut inv, id(2), change).expect("confirmed change");
            assert_eq!(is_favorite(&inv, id(9000)), expected);
            assert_eq!(is_favorite(&inv, id(9001)), expected);
            assert!(!inv.items[&id(9001)].favorite, "the link must not get its own favorite");
        }
    }
    let mut inv = fixture();
    inv.add_items(vec![InvItem {
        id: id(9002),
        parent: id(8000),
        asset_type: 25,
        asset_id: id(8001),
        ..Default::default()
    }]);
    let change = toggle_favorite(&inv, id(9002)).expect("folder original");
    assert!(
        matches!(&change.operations[..], [aurora_net::inventory::operations::Operation::Patch { id: target, folder: true, .. }] if *target == id(8001))
    );
    demo_mutate(&mut inv, id(2), change).expect("favorite folder");
    assert!(is_favorite(&inv, id(9002)));
    inv.items.get_mut(&id(9002)).expect("link").asset_id = id(9999);
    assert!(!is_favorite(&inv, id(9002)));
    assert!(toggle_favorite(&inv, id(9002)).is_err());
}

fn nested_folder_fixture() -> Inventory {
    let mut inv = fixture();
    for (number, parent) in [(8200, id(8001)), (8201, id(8200)), (8300, inv.root)] {
        let folder = Folder {
            info: InvFolder {
                id: id(number),
                parent,
                name: format!("Folder {number}"),
                type_default: -1,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        };
        inv.folders.insert(folder.info.id, folder);
        inv.folders.get_mut(&parent).expect("parent").children.push(id(number));
    }
    let items = [(8210, 8001), (8211, 8200), (8212, 8201)]
        .into_iter()
        .map(|(number, parent)| {
            let mut item = inv.items[&id(8104)].clone();
            item.id = id(number);
            item.parent = id(parent);
            item.name = format!("Item {number}");
            item
        })
        .collect();
    inv.add_items(items);
    inv
}

#[test]
fn copying_a_selected_folder_keeps_all_three_levels_and_their_contents() {
    use aurora_net::inventory::operations::Operation;
    let inv = nested_folder_fixture();
    let change = paste(&inv, &[id(8001)], id(8300), false, false, &HashSet::new(), &HashSet::new()).expect("recursive copy");
    assert_eq!(change.operations.len(), 3);
    assert_eq!(change.copies.len(), 3);
    let mut parent = id(8300);
    for (operation, item) in change.operations.iter().zip([8210, 8211, 8212]) {
        let Operation::Folder {
            id: new_folder,
            parent: destination,
            ..
        } = operation
        else {
            panic!("folder creation")
        };
        assert_eq!(*destination, parent);
        assert_ne!(*new_folder, id(8001));
        assert!(change.copies.iter().any(|copy| copy.item == id(item) && copy.parent == *new_folder));
        parent = *new_folder;
    }
    assert_eq!(inv.items[&id(8212)].parent, id(8201));
}

#[test]
fn cutting_a_selected_folder_moves_its_complete_hierarchy_without_copying_items() {
    use aurora_net::inventory::operations::Operation;
    let mut inv = nested_folder_fixture();
    inv.items.get_mut(&id(8212)).expect("deep item").owner_mask &= !COPY;
    let change = paste(&inv, &[id(8001)], id(8300), true, false, &HashSet::new(), &HashSet::new()).expect("whole folder move");
    assert!(
        matches!(&change.operations[..], [Operation::Patch { id: folder, folder: true, body }] if *folder == id(8001) && body["parent_id"].as_uuid() == id(8300))
    );
    assert!(change.copies.is_empty());
    demo_mutate(&mut inv, id(2), change).expect("moved hierarchy");
    for item in [8210, 8211, 8212] {
        assert!(under(&inv, id(item), id(8300)));
    }
    assert_eq!(inv.items[&id(8212)].parent, id(8201));
    assert_eq!(inv.folders[&id(8201)].info.parent, id(8200));
}

#[test]
fn copying_a_folder_refuses_an_unloaded_descendant_instead_of_creating_an_empty_copy() {
    let mut inv = nested_folder_fixture();
    inv.folders.get_mut(&id(8201)).expect("descendant").state = FetchState::Unknown;
    assert!(!paste_allowed(
        &inv,
        &[id(8001)],
        id(8300),
        false,
        false,
        &HashSet::new(),
        &HashSet::new()
    ));
    assert!(paste(&inv, &[id(8001)], id(8300), false, false, &HashSet::new(), &HashSet::new()).is_err());
    assert!(!request_tree(&mut inv, id(8001)));
    assert!(inv.queue.iter().any(|(folder, _)| *folder == id(8201)));
}

#[test]
fn clipboard_refuses_no_copy_but_allows_a_link_without_changing_the_original() {
    let mut inv = fixture();
    let original = inv.items[&id(8109)].clone();
    assert!(!paste_allowed(
        &inv,
        &[original.id],
        id(8001),
        false,
        false,
        &HashSet::new(),
        &HashSet::new()
    ));
    assert!(paste_allowed(
        &inv,
        &[original.id],
        id(8001),
        false,
        true,
        &HashSet::new(),
        &HashSet::new()
    ));
    assert!(paste(&inv, &[original.id], id(8001), false, false, &HashSet::new(), &HashSet::new()).is_err());
    let plan = paste(&inv, &[original.id], id(8001), false, true, &HashSet::new(), &HashSet::new()).expect("link");
    demo_mutate(&mut inv, id(2), plan).expect("demo link");
    assert_eq!(inv.items[&original.id].parent, original.parent);
    assert!(
        inv.items
            .values()
            .any(|it| it.parent == id(8001) && it.asset_type == 24 && it.asset_id == original.id)
    );
}

#[test]
fn protected_system_worn_and_unknown_elements_cannot_be_deleted() {
    let inv = fixture();
    let trash = system(&inv, 14).expect("trash");
    assert!(!movable(&inv, id(99999), &HashSet::new()));
    assert!(move_selection(&inv, &[inv.root], trash, &HashSet::new(), &HashSet::new()).is_err());
    assert!(move_selection(&inv, &[id(8000)], trash, &HashSet::from([id(8000)]), &HashSet::new()).is_err());
    assert!(move_selection(&inv, &[id(8000)], trash, &HashSet::new(), &HashSet::from([id(8100)])).is_err());
    assert!(!destination(&inv, id(701)));
}

#[test]
fn protected_folder_cannot_move_but_its_contents_can_move_in_and_out() {
    let inv = fixture();
    let locked = id(8000);
    let protected = HashSet::from([locked]);
    let trash = system(&inv, 14).expect("trash");

    let (sub, item) = (id(8001), id(8100));
    let none = HashSet::new();

    // the folder itself (LLFolderBridge::isProtected): held in place and out
    // of the Trash, but renamed and filled like any folder
    assert!(!movable(&inv, locked, &protected));
    assert!(!relocatable(&inv, locked, &protected));
    assert!(!removable(&inv, locked, &protected));
    assert!(renameable(&inv, locked));
    assert!(destination(&inv, locked));
    assert!(move_selection(&inv, &[locked], inv.root, &protected, &none).is_err());
    assert!(move_selection(&inv, &[locked], trash, &protected, &none).is_err());

    // its contents, a subfolder's included, are ordinary entries: they move
    // out, in and to the Trash
    for entry in [item, sub] {
        assert!(relocatable(&inv, entry, &protected), "{entry}");
        assert!(removable(&inv, entry, &protected), "{entry}");
        for to in [inv.root, trash] {
            assert!(move_selection(&inv, &[entry], to, &protected, &none).is_ok(), "{entry} to {to}");
        }
    }
    assert!(renameable(&inv, sub));
    assert!(move_selection(&inv, &[item], locked, &protected, &none).is_ok());
    assert!(move_selection(&inv, &[item], sub, &protected, &none).is_ok());
}

#[test]
fn a_folder_holding_a_locked_folder_goes_to_the_trash_with_it() {
    let inv = fixture();
    let (outer, locked) = (id(8000), id(8001));
    let protected = HashSet::from([locked]);
    let none = HashSet::new();
    let trash = system(&inv, 14).expect("trash");
    // get_is_category_and_children_removable only refuses system folders below
    assert!(removable(&inv, outer, &protected));
    assert!(move_selection(&inv, &[outer], trash, &protected, &none).is_ok());
    assert!(move_selection(&inv, &[outer], inv.root, &protected, &none).is_ok());
}

#[test]
fn cut_and_paste_move_a_locked_folders_contents_out_and_in() {
    use aurora_net::inventory::operations::Operation;
    let mut inv = fixture();
    let (locked, sub, inside) = (id(8000), id(8001), id(8100));
    let protected = HashSet::from([locked]);
    let none = HashSet::new();
    let outside = id(8900);
    inv.add_items(vec![InvItem {
        id: outside,
        parent: inv.root,
        asset_type: 6,
        owner_mask: COPY | MODIFY | TRANSFER,
        ..Default::default()
    }]);
    let moves = |ops: &[Operation], what: Uuid, to: Uuid| matches!(ops, [Operation::Patch { id, body, .. }] if *id == what && body["parent_id"].as_uuid() == to);

    // Cut in the locked folder, Paste elsewhere
    assert!(paste_allowed(&inv, &[inside], inv.root, true, false, &protected, &none));
    let change = paste(&inv, &[inside], inv.root, true, false, &protected, &none).expect("cut out");
    assert!(moves(&change.operations, inside, inv.root));

    // Cut elsewhere, Paste into the locked folder and into its subfolder
    for to in [locked, sub] {
        assert!(paste_allowed(&inv, &[outside], to, true, false, &protected, &none), "{to}");
        let change = paste(&inv, &[outside], to, true, false, &protected, &none).expect("cut in");
        assert!(moves(&change.operations, outside, to), "{to}");
        // a copy may come in too
        assert!(paste_allowed(&inv, &[outside], to, false, false, &protected, &none), "{to}");
    }

    // the locked folder itself still cannot be cut away
    assert!(!paste_allowed(&inv, &[locked], inv.root, true, false, &protected, &none));
    assert!(paste(&inv, &[locked], inv.root, true, false, &protected, &none).is_err());
}

#[test]
fn root_support_folders_stay_in_place_but_can_still_be_deleted() {
    let mut inv = fixture();
    let trash = system(&inv, 14).expect("trash");
    let none = HashSet::new();
    let folder = |id: Uuid, parent: Uuid, name: &str| InvFolder {
        id,
        parent,
        name: name.into(),
        type_default: -1,
        ..Default::default()
    };
    inv.created_folders(vec![
        folder(id(9100), inv.root, "#RLV"),
        folder(id(9101), id(8001), "#RLV"),
        folder(id(9102), inv.root, "#Firestorm"),
        folder(id(9103), inv.root, "#rlv"),
        InvFolder {
            type_default: 48,
            ..folder(id(9104), inv.root, "My Outfits")
        },
        folder(id(9105), inv.root, "Outfit"),
    ]);
    assert!(move_selection(&inv, &[id(9105)], id(9104), &none, &none).is_ok());
    for pinned_id in [id(9100), id(9102)] {
        assert!(pinned(&inv, pinned_id));
        assert!(!relocatable(&inv, pinned_id, &none));
        assert!(move_selection(&inv, &[pinned_id], id(8001), &none, &none).is_err());
        assert!(move_selection(&inv, &[pinned_id], id(9104), &none, &none).is_err());
        assert!(ungroup(&inv, pinned_id, &none, &none).is_err());
        assert!(!paste_allowed(&inv, &[pinned_id], id(8001), true, false, &none, &none));
        assert!(
            move_selection(&inv, &[pinned_id], trash, &none, &none).is_ok(),
            "delete stays allowed"
        );
        assert!(renameable(&inv, pinned_id));
    }
    // A misplaced #RLV goes back to the root; names match exactly, as RLVa's.
    assert!(move_selection(&inv, &[id(9101)], inv.root, &none, &none).is_ok());
    assert!(relocatable(&inv, id(9103), &none));
}

#[test]
fn selected_parent_and_child_are_moved_once_and_cycles_are_refused() {
    let inv = fixture();
    assert_eq!(roots(&inv, &[id(8100), id(8000), id(8100)]), vec![id(8000)]);
    assert!(paste(&inv, &[id(8000)], id(8001), true, false, &HashSet::new(), &HashSet::new()).is_err());
    assert!(paste(&inv, &[id(8000)], id(8001), false, false, &HashSet::new(), &HashSet::new()).is_err());
}

#[test]
fn selection_roots_skip_any_selected_ancestor_and_survive_a_folder_cycle() {
    let mut inv = fixture();
    // 8100 sits in 8000, in the root: a selected grandparent hides it too.
    assert_eq!(roots(&inv, &[id(8100), inv.root]), vec![inv.root]);
    assert_eq!(roots(&inv, &[id(8100), id(8101)]), vec![id(8100), id(8101)]);
    // A corrupt parent cycle (8000 and its subfolder 8001 holding each
    // other) ends the walk instead of looping.
    inv.folders.get_mut(&id(8000)).expect("folder").info.parent = id(8001);
    assert!(!under(&inv, id(8100), inv.root));
    assert_eq!(roots(&inv, &[id(8100), id(8001)]), vec![id(8001)]);
}

#[test]
fn worn_ancestors_hold_the_worn_item_and_every_folder_above_it() {
    let inv = fixture();
    let holds = worn_ancestors(&inv, &HashSet::from([id(8100)]));
    assert!(holds.contains(&id(8100)) && holds.contains(&id(8000)) && holds.contains(&inv.root));
    assert!(!holds.contains(&id(8101)) && !holds.contains(&id(8001)));
}

#[test]
fn a_system_folder_is_the_roots_child_with_the_smallest_id() {
    let mut inv = fixture();
    let trash = system(&inv, 14).expect("trash");
    assert!(trash > id(4));
    // A Trash-typed folder deeper down is not the Trash, even with the
    // smallest id; of two at the root, the smallest wins.
    for (stray, parent) in [(id(3), id(8000)), (id(4), inv.root)] {
        inv.folders.insert(
            stray,
            Folder {
                info: InvFolder {
                    id: stray,
                    parent,
                    type_default: 14,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&parent).expect("parent").children.push(stray);
    }
    assert_eq!(system(&inv, 14), Some(id(4)));
    inv.folders.remove(&id(4));
    assert_eq!(system(&inv, 14), Some(trash));
}

#[test]
fn moved_and_renamed_items_take_their_sorted_place() {
    let mut inv = fixture();
    let to = id(8001);
    let before = inv.folders[&to].items.clone();
    let named = |inv: &Inventory| -> Vec<String> { inv.folders[&to].items.iter().map(|i| inv.items[i].name.to_lowercase()).collect() };
    let mut expected = named(&inv);
    expected.sort();
    assert_eq!(named(&inv), expected, "fixture folder starts sorted");
    // Moves arrive one by one, in no particular order of names.
    for (n, name) in [(8100u128, "zebra"), (8101, "Apple"), (8102, "mango")] {
        let mut moved = inv.items[&id(n)].clone();
        moved.parent = to;
        moved.name = name.into();
        inv.server_updated(Vec::new(), vec![moved]);
    }
    let mut expected = named(&inv);
    expected.sort();
    assert_eq!(named(&inv), expected);
    assert_eq!(inv.folders[&to].items.len(), before.len() + 3);
    assert!(
        inv.folders[&id(8000)]
            .items
            .iter()
            .all(|i| ![8100, 8101, 8102].contains(&i.as_u128()))
    );
    // A rename in place moves the entry too.
    let mut renamed = inv.items[&id(8100)].clone();
    renamed.name = "aardvark".into();
    inv.server_updated(Vec::new(), vec![renamed]);
    let mut expected = named(&inv);
    expected.sort();
    assert_eq!(named(&inv), expected);
    assert_eq!(inv.folders[&to].items.len(), before.len() + 3);
}

#[test]
fn snapshot_sorts_renamed_members_and_keeps_origins_in_their_own_folders() {
    let mut inv = fixture();
    let listed = id(8000);
    let another = id(8001);
    let mut renamed = inv.items[&id(8100)].clone();
    renamed.name = "Zebra".into();
    let mut sibling = inv.items[&id(8101)].clone();
    sibling.name = "Apple".into();
    let mut original = renamed.clone();
    original.id = id(99999);
    original.parent = another;
    inv.apply(vec![FolderContents {
        folder_id: listed,
        owner_id: inv.owner,
        version: inv.folders[&listed].info.version + 1,
        folders: Vec::new(),
        items: vec![renamed, original, sibling],
    }]);
    assert_eq!(inv.folders[&listed].items, [id(8101), id(8100)]);
    assert!(inv.folders[&another].items.contains(&id(99999)));
    assert_eq!(inv.items[&id(99999)].parent, another);
}

#[test]
fn broken_and_cyclic_links_are_not_followed() {
    let mut inv = fixture();
    inv.items.get_mut(&id(8113)).expect("link").asset_id = id(8113);
    assert_eq!(original(&inv, id(8113)), None);
    assert!(!paste_allowed(
        &inv,
        &[id(8113)],
        id(8001),
        false,
        true,
        &HashSet::new(),
        &HashSet::new()
    ));
    inv.items.get_mut(&id(8113)).expect("link").asset_id = id(99999);
    assert_eq!(original(&inv, id(8113)), None);
}

#[test]
fn restore_uses_the_root_for_a_folder_and_the_original_type_for_a_link() {
    let mut inv = fixture();
    assert_eq!(default_folder(&inv, id(8001)), Some(inv.root));
    let mut objects = inv.folders[&id(8001)].clone();
    objects.info.id = id(12);
    objects.info.type_default = 6;
    inv.folders.insert(id(12), objects);
    assert_eq!(default_folder(&inv, id(8113)), Some(id(12)));
}

#[test]
fn ungroup_moves_contents_before_the_empty_folder_to_trash() {
    let mut inv = fixture();
    let old = inv.items[&id(8100)].asset_id;
    let group = group(&inv, &[id(8100), id(8103)], &HashSet::new(), &HashSet::new()).expect("group");
    let reply = demo_mutate(&mut inv, id(2), group).expect("group reply");
    let grouped = *reply.created.values().next().expect("folder");
    let ungroup = ungroup(&inv, grouped, &HashSet::new(), &HashSet::new()).expect("ungroup");
    demo_mutate(&mut inv, id(2), ungroup).expect("ungroup reply");
    assert_eq!(inv.items[&id(8100)].parent, id(8000));
    assert_eq!(inv.items[&id(8100)].asset_id, old);
    assert_eq!(inv.folders[&grouped].info.parent, system(&inv, 14).expect("trash"));
}

#[test]
fn recursive_copy_requires_complete_contents_and_keeps_the_library_owner() {
    let mut inv = fixture();
    inv.folders.get_mut(&id(8001)).expect("folder").state = FetchState::Unknown;
    assert!(!paste_allowed(
        &inv,
        &[id(8001)],
        inv.root,
        false,
        false,
        &HashSet::new(),
        &HashSet::new()
    ));
    inv.folders.get_mut(&id(8001)).expect("folder").library = true;
    inv.lib_owner = id(99);
    inv.items.get_mut(&id(8109)).expect("item").parent = id(8001);
    let plan = paste(&inv, &[id(8109)], inv.root, false, false, &HashSet::new(), &HashSet::new()).expect("library copy");
    assert_eq!(plan.copies[0].owner, id(99));
}

#[test]
fn marketplace_requires_transfer_and_stages_no_copy_objects_in_stock_folders() {
    let mut inv = fixture();
    assert!(marketplace(&inv, &[id(8110)], false, &HashSet::new(), &HashSet::new()).is_err());
    assert!(marketplace(&inv, &[id(8109)], true, &HashSet::new(), &HashSet::new()).is_err());
    let plan = marketplace(&inv, &[id(8109)], false, &HashSet::new(), &HashSet::new()).expect("stock move");
    demo_mutate(&mut inv, id(2), plan).expect("stock reply");
    let parent = inv.items[&id(8109)].parent;
    assert_eq!(inv.folders[&parent].info.type_default, 54);
    assert!(in_type(&inv, parent, 53));
}

#[test]
fn offers_are_bounded_and_include_the_folder_as_the_first_tuple() {
    let mut inv = fixture();
    assert!(offer(&inv, id(8110), &HashSet::new()).is_err());
    inv.remove(&[id(8110), id(8113)]);
    let bucket = offer(&inv, id(8000), &HashSet::new()).expect("offer folder");
    assert_eq!(bucket[0], 8);
    assert_eq!(&bucket[1..17], id(8000).as_bytes());
    assert_eq!(bucket.len() % 17, 0);
    assert!(offer_has_no_copy(&inv, &bucket));
    assert!(!offer_has_no_copy(
        &inv,
        &offer(&inv, id(8100), &HashSet::new()).expect("copyable item")
    ));
    assert!(offer(&inv, id(8100), &HashSet::from([id(8100)])).is_err());
}

#[test]
fn a_multi_resident_gift_rechecks_permissions_when_it_is_sent() {
    let mut inv = fixture();
    let worn = HashSet::new();
    let item = id(8100);
    assert!(offer_for_residents(&inv, item, &worn, 2).is_ok());
    let first_mask = inv.items[&item].owner_mask;
    inv.items.get_mut(&item).expect("gift").owner_mask = first_mask & !COPY;
    assert!(offer_for_residents(&inv, item, &worn, 2).is_err());
    assert!(offer_for_residents(&inv, item, &worn, 1).is_ok());
    inv.items.get_mut(&item).expect("gift").owner_mask = first_mask & !TRANSFER;
    for count in [1, 2] {
        assert!(offer_for_residents(&inv, item, &worn, count).is_err());
        assert!(offer_for_residents(&inv, id(8000), &worn, count).is_err());
    }
    inv.items.get_mut(&item).expect("gift").owner_mask = first_mask;
    inv.remove(&[id(8110), id(8113)]);
    assert!(offer_for_residents(&inv, id(8000), &worn, 1).is_ok());
    assert!(offer_for_residents(&inv, id(8000), &worn, 2).is_err());
}

#[test]
fn folder_wear_filters_documents_and_preserves_the_required_body() {
    use crate::world::appearance::{self, Action, WearMode};
    let inv = fixture();
    let (change, sync) = appearance::plan(
        &inv,
        Action::WearContents {
            folder: id(8000),
            mode: WearMode::ReplaceOutfit,
        },
        &HashMap::new(),
    )
    .expect("wear folder");
    assert!(sync);
    assert!(!change.links.iter().any(|l| l.target == id(8103)));
    assert!(change.links.iter().any(|l| l.target == id(8112)));
    assert!(change.links.iter().any(|l| {
        inv.items
            .get(&l.target)
            .is_some_and(|it| it.asset_type == 13 && it.flags & 0xff == 1)
    }));
    assert!(!change.links.iter().any(|l| l.folder));
}

#[test]
fn names_are_trimmed_bounded_and_cannot_inject_legacy_asset_headers() {
    assert_eq!(clean_name("  Chemise  ").expect("name"), "Chemise");
    assert!(clean_name("Chemise\ntype 0").is_err());
    assert!(clean_name("A|B").is_err());
    assert_eq!(clean_name(&"a".repeat(90)).expect("bounded name").len(), 63);
    for name in ["Café", "桜", "龙", "Name 桜龙"] {
        assert_eq!(clean_name(name).expect("received name"), name);
    }
}

#[test]
fn a_link_can_be_pasted_into_an_outfit_folder_but_a_copy_cannot() {
    let mut inv = fixture();
    let original = id(8109);
    inv.created_folders(vec![
        InvFolder {
            id: id(9700),
            parent: inv.root,
            name: "My Outfits".into(),
            type_default: 48,
            ..Default::default()
        },
        InvFolder {
            id: id(9701),
            parent: id(9700),
            name: "Outfit".into(),
            type_default: 47,
            ..Default::default()
        },
    ]);
    let none = HashSet::new();
    for outfit in [id(9700), id(9701)] {
        assert!(paste_allowed(&inv, &[original], outfit, false, true, &none, &none), "{outfit}");
        assert!(!paste_allowed(&inv, &[original], outfit, false, false, &none, &none), "{outfit}");
    }
    assert!(paste(&inv, &[original], id(9701), false, true, &none, &none).is_ok());
    assert!(paste(&inv, &[original], id(9701), false, false, &none, &none).is_err());

    // can_move_to_outfit: only what can be worn is linked into an outfit;
    // a notecard, a texture, a coalesced object and a folder are skipped
    let item = |n: u128, inv_type: i32, asset_type: i32, flags: u32| InvItem {
        id: id(n),
        parent: inv.root,
        inv_type,
        asset_type,
        flags,
        ..Default::default()
    };
    inv.add_items(vec![
        item(9710, 18, 13, 0),
        item(9711, 7, 7, 0),
        item(9712, 0, 0, 0),
        item(9713, 6, 6, crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS),
    ]);
    let folder = id(9720);
    inv.created_folders(vec![InvFolder {
        id: folder,
        parent: inv.root,
        name: "Loose".into(),
        type_default: -1,
        ..Default::default()
    }]);
    for refused in [id(9711), id(9712), id(9713), folder] {
        assert!(!paste_allowed(&inv, &[refused], id(9701), false, true, &none, &none), "{refused}");
        assert!(paste(&inv, &[refused], id(9701), false, true, &none, &none).is_err(), "{refused}");
        // elsewhere the link is still allowed
        assert!(paste_allowed(&inv, &[refused], id(8000), false, true, &none, &none), "{refused}");
    }
    let mixed = [id(9710), id(9711), folder];
    assert!(paste_allowed(&inv, &mixed, id(9701), false, true, &none, &none));
    let change = paste(&inv, &mixed, id(9701), false, true, &none, &none).expect("the wearable is linked");
    assert_eq!(change.operations.len(), 1, "only the wearable");
}

#[test]
fn an_asset_uuid_shows_for_full_permissions_or_a_built_in_texture() {
    let mut inv = Inventory::default();
    let asset = id(9801);
    assert!(!asset_uuid_shown(&inv, asset), "not in the inventory");
    assert!(asset_uuid_shown_to(&inv, asset, true), "a god may see any asset UUID");
    assert!(asset_uuid_shown(&inv, aurora_prim::te::BLANK_TEXTURE));
    assert!(asset_uuid_shown(&inv, aurora_prim::te::TRANSPARENT_TEXTURE));
    for missing in [COPY, MODIFY, TRANSFER] {
        inv.items.insert(
            id(9802),
            InvItem {
                id: id(9802),
                asset_id: asset,
                owner_mask: FULL & !missing,
                ..Default::default()
            },
        );
        assert!(!asset_uuid_shown(&inv, asset), "missing permission {missing:x}");
    }
    inv.items.insert(
        id(9803),
        InvItem {
            id: id(9803),
            asset_id: asset,
            owner_mask: FULL,
            ..Default::default()
        },
    );
    assert!(asset_uuid_shown(&inv, asset), "a second item gives full permissions");
}

#[test]
fn the_agent_holds_the_owner_mask_of_its_items_and_the_everyone_mask_of_others() {
    let (agent, library) = (id(9901), id(9902));
    let item = |owner, group_owned| InvItem {
        owner,
        group_owned,
        owner_mask: FULL,
        everyone_mask: COPY,
        ..Default::default()
    };
    assert_eq!(agent_mask(&item(agent, false), agent), FULL, "its own item");
    assert_eq!(agent_mask(&item(library, false), agent), COPY, "a Library item");
    assert_eq!(agent_mask(&item(agent, true), agent), COPY, "group owned");
    assert_eq!(agent_mask(&item(Uuid::nil(), false), agent), FULL, "built here, no owner yet");
    assert_eq!(agent_mask(&item(library, false), Uuid::nil()), FULL, "before the login");
    let inv = Inventory {
        owner: agent,
        ..Default::default()
    };
    assert_eq!(held(&inv, &item(agent, false)), FULL);
    assert_eq!(held(&inv, &item(library, false)), COPY);
}

#[test]
fn an_appearance_xml_export_needs_modify_rights_on_the_wearable() {
    let (agent, other, library) = (id(9911), id(9912), id(9913));
    let item = |owner, group_owned, owner_mask, everyone_mask| InvItem {
        owner,
        group_owned,
        owner_mask,
        everyone_mask,
        ..Default::default()
    };
    let cases = [
        ("its own item, full permissions", item(agent, false, FULL, 0), true),
        ("no copy", item(agent, false, MODIFY | TRANSFER, 0), true),
        ("no transfer", item(agent, false, COPY | MODIFY, 0), true),
        ("no modify", item(agent, false, COPY | TRANSFER, 0), false),
        ("group owned", item(agent, true, FULL, COPY), false),
        ("someone else's", item(other, false, FULL, COPY), false),
        ("a Library item", item(library, false, FULL, COPY | TRANSFER), false),
    ];
    for (what, item, allowed) in cases {
        assert_eq!(appearance_xml_export_allowed(&item, agent), allowed, "{what}");
    }
}

#[test]
fn an_asset_export_needs_the_items_creator_or_its_export_bit_on_opensim() {
    let (agent, other, library) = (id(9931), id(9932), id(9933));
    let item = |owner, creator, group_owned, owner_mask, everyone_mask| InvItem {
        owner,
        creator,
        group_owned,
        owner_mask,
        everyone_mask,
        ..Default::default()
    };
    let sl = ExportGrid::SecondLife;
    let denied = ExportGrid::OpenSim { export_supported: false };
    let allowed = ExportGrid::OpenSim { export_supported: true };
    // (what, item, Second Life and OpenSim without export, OpenSim with export)
    let cases = [
        (
            "its own upload, full permissions",
            item(agent, agent, false, FULL | EXPORT, 0),
            true,
            true,
        ),
        ("its own upload, no Export bit", item(agent, agent, false, FULL, 0), true, false),
        (
            "its own upload, no copy",
            item(agent, agent, false, MODIFY | TRANSFER, 0),
            true,
            false,
        ),
        (
            "its own upload, no transfer",
            item(agent, agent, false, COPY | MODIFY, 0),
            true,
            false,
        ),
        (
            "its own upload, no modify",
            item(agent, agent, false, COPY | TRANSFER, 0),
            true,
            false,
        ),
        ("bought, full permissions", item(agent, other, false, FULL, 0), false, false),
        ("bought, Export allowed", item(agent, other, false, FULL | EXPORT, 0), false, true),
        (
            "group owned, its own upload",
            item(agent, agent, true, FULL | EXPORT, 0),
            true,
            false,
        ),
        (
            "group owned, Export to everyone",
            item(agent, other, true, FULL | EXPORT, EXPORT),
            false,
            true,
        ),
        ("someone else's", item(other, other, false, FULL | EXPORT, 0), false, false),
        (
            "a Library item",
            item(library, library, false, FULL | EXPORT, COPY | TRANSFER),
            false,
            false,
        ),
    ];
    for (what, item, creator_rule, export_rule) in cases {
        assert_eq!(asset_export_allowed(&item, agent, sl), creator_rule, "Second Life: {what}");
        assert_eq!(
            asset_export_allowed(&item, agent, denied),
            creator_rule,
            "OpenSim without export: {what}"
        );
        assert_eq!(
            asset_export_allowed(&item, agent, allowed),
            export_rule,
            "OpenSim with export: {what}"
        );
    }
    assert!(
        !asset_export_allowed(&item(agent, Uuid::nil(), false, FULL, 0), Uuid::nil(), sl),
        "no agent"
    );
}

#[test]
fn the_agent_holds_the_everyone_mask_of_contents_in_someone_elses_object() {
    let (agent, friend) = (id(9921), id(9922));
    let item = |owner_id, group_owned| aurora_net::build::TaskItem {
        owner_id,
        group_owned,
        owner_mask: FULL,
        everyone_mask: COPY,
        ..Default::default()
    };
    assert_eq!(task_agent_mask(&item(agent, false), agent), FULL, "its own object's contents");
    assert_eq!(task_agent_mask(&item(friend, false), agent), COPY, "an object it may only modify");
    assert_eq!(task_agent_mask(&item(agent, true), agent), COPY, "group owned");
}

#[test]
fn an_object_rezzes_from_contents_when_the_agent_may_copy_it_and_its_owner_transfer_it() {
    let (agent, other, library, group) = (id(9931), id(9932), id(9933), id(9934));
    let item = |owner_id, group_owned, owner_mask, group_mask, everyone_mask| aurora_net::build::TaskItem {
        owner_id,
        group_owned,
        group_id: group,
        owner_mask,
        group_mask,
        everyone_mask,
        asset_type: 6,
        inv_type: 6,
        ..Default::default()
    };
    let cases = [
        ("its own object, full permissions", item(agent, false, FULL, 0, 0), group, true),
        ("no copy", item(agent, false, MODIFY | TRANSFER, 0, 0), group, false),
        ("no transfer, even its own", item(agent, false, COPY | MODIFY, 0, 0), group, false),
        ("no modify", item(agent, false, COPY | TRANSFER, 0, 0), group, true),
        ("group owned, active group may copy", item(group, true, FULL, COPY, 0), group, true),
        (
            "group owned, another active group",
            item(group, true, FULL, COPY, 0),
            Uuid::nil(),
            false,
        ),
        (
            "group owned, owner may not transfer",
            item(group, true, COPY | MODIFY, COPY, COPY),
            group,
            false,
        ),
        (
            "someone else's, everyone may copy",
            item(other, false, FULL, 0, COPY),
            Uuid::nil(),
            true,
        ),
        (
            "someone else's, no copy for everyone",
            item(other, false, FULL, 0, 0),
            Uuid::nil(),
            false,
        ),
        ("a Library item", item(library, false, FULL, 0, COPY | TRANSFER), Uuid::nil(), true),
    ];
    for (what, item, active, allowed) in cases {
        assert_eq!(task_rez_allowed(&item, agent, active), allowed, "{what}");
    }
}

#[test]
fn a_library_item_shows_its_asset_uuid_by_what_it_allows_everyone() {
    let (agent, library, asset) = (id(9911), id(9912), id(9913));
    let mut inv = Inventory {
        owner: agent,
        ..Default::default()
    };
    let mut lib_item = InvItem {
        id: id(9914),
        owner: library,
        asset_id: asset,
        owner_mask: FULL,
        everyone_mask: COPY,
        ..Default::default()
    };
    inv.items.insert(lib_item.id, lib_item.clone());
    assert!(!asset_uuid_shown(&inv, asset), "full for the Library's account only");
    lib_item.everyone_mask = FULL;
    inv.items.insert(lib_item.id, lib_item);
    assert!(asset_uuid_shown(&inv, asset), "full for everyone");
}

#[test]
fn next_owner_rights_need_the_owners_transfer() {
    let item = |owner_mask: u32, asset_type: i32, inv_type: i32| InvItem {
        owner_mask,
        asset_type,
        inv_type,
        ..Default::default()
    };
    let full = COPY | MODIFY | TRANSFER;
    assert!(next_owner_editable(&item(full, 6, 6)));
    assert!(!next_owner_editable(&item(COPY | MODIFY, 6, 6)), "no transfer for the owner");
    // LLInventoryType::cannotRestrictPermissions
    assert!(!next_owner_editable(&item(full, 2, 2)), "calling card");
    assert!(!next_owner_editable(&item(full, 3, 3)), "landmark");
}

#[test]
fn a_no_copy_object_is_not_restored_to_its_last_position_in_second_life() {
    let object = |owner_mask: u32| InvItem {
        owner_mask,
        asset_type: 6,
        ..Default::default()
    };
    assert!(restorable(&object(COPY | MODIFY | TRANSFER), false));
    assert!(!restorable(&object(MODIFY | TRANSFER), false));
    assert!(restorable(&object(MODIFY | TRANSFER), true), "OpenSim keeps it");
}

#[test]
fn god_permission_table_keeps_ordinary_agent_rights_and_grants_only_asset_bits() {
    let (agent, other, asset) = (id(9951), id(9952), id(9953));
    for (name, owner, group_owned, owner_mask, everyone_mask, ordinary) in [
        ("owner", agent, false, FULL, 0, FULL),
        ("group owned", agent, true, FULL, COPY, COPY),
        ("other", other, false, FULL, 0, 0),
        ("Library", other, false, 0, FULL, FULL),
        ("no copy", agent, false, FULL & !COPY, 0, FULL & !COPY),
        ("no transfer", agent, false, FULL & !TRANSFER, 0, FULL & !TRANSFER),
        ("no modify", agent, false, FULL & !MODIFY, 0, FULL & !MODIFY),
    ] {
        let item = InvItem {
            id: id(9954),
            asset_id: asset,
            owner,
            group_owned,
            owner_mask,
            everyone_mask,
            ..Default::default()
        };
        let mut inv = Inventory {
            owner: agent,
            ..Default::default()
        };
        inv.items.insert(item.id, item.clone());
        // OpenSim's usual god levels (200 to 249) keep the ordinary rights
        // (LLAgent::allowOperation's default god_minimum is GOD_MAINTENANCE)
        // but see every asset UUID (isGodlikeWithoutAdminMenuFakery)
        for level in [0u8, 1, 225, GOD_MAINTENANCE - 1, GOD_MAINTENANCE, 255] {
            let expected = if level >= GOD_MAINTENANCE { ordinary | FULL } else { ordinary };
            assert_eq!(held_for(&inv, &item, level), expected, "{name}, level {level}");
            assert_eq!(
                asset_uuid_shown_to(&inv, asset, level > 0),
                level > 0 || ordinary & FULL == FULL,
                "{name}, level {level}"
            );
        }
    }
    assert_eq!(with_god(MOVE, 0), MOVE);
    assert_eq!(with_god(MOVE, 225), MOVE, "under GOD_MAINTENANCE: no right added");
    assert_eq!(with_god(MOVE, GOD_MAINTENANCE), MOVE | FULL);
    assert_eq!(with_god(0, 255) & MOVE, 0, "asset rights do not grant other powers");
}

#[test]
fn contents_previews_open_textures_copyable_sounds_and_animations_of_modifiable_objects() {
    let (agent, other, group) = (id(9971), id(9972), id(9973));
    let sound = InvItem {
        owner: agent,
        owner_mask: FULL,
        asset_type: 1,
        inv_type: 1,
        ..Default::default()
    };
    let open = |item: &InvItem, member: bool, manipulate: bool| task_preview_allowed(item, agent, member, manipulate, false, 0);
    // the owner's, and full permissions
    assert!(open(&sound, false, false));
    // group-owned: the owner's rights with object manipulation, the
    // group's for another member, the everyone mask outside the group
    let shared = InvItem {
        owner: group,
        group_id: group,
        group_owned: true,
        owner_mask: FULL,
        group_mask: 0,
        everyone_mask: 0,
        ..sound.clone()
    };
    assert!(open(&shared, true, true));
    assert!(!open(&shared, true, false));
    assert!(open(
        &InvItem {
            group_mask: COPY,
            ..shared.clone()
        },
        true,
        false
    ));
    assert!(!open(&shared, false, false));
    // someone else's: their everyone mask
    let theirs = InvItem {
        owner: other,
        everyone_mask: 0,
        ..sound.clone()
    };
    assert!(!open(&theirs, false, false));
    assert!(open(
        &InvItem {
            everyone_mask: COPY,
            ..theirs.clone()
        },
        false,
        false
    ));
    // a Library item: the Library's account allows what it allows everyone
    let library = InvItem {
        owner: id(9974),
        everyone_mask: COPY | TRANSFER,
        ..sound.clone()
    };
    assert!(open(&library, false, false));
    // no copy refuses; no transfer and no modify do not matter
    assert!(!open(
        &InvItem {
            owner_mask: MODIFY | TRANSFER,
            ..sound.clone()
        },
        false,
        false
    ));
    assert!(open(
        &InvItem {
            owner_mask: COPY | MODIFY,
            ..sound.clone()
        },
        false,
        false
    ));
    assert!(open(
        &InvItem {
            owner_mask: COPY | TRANSFER,
            ..sound.clone()
        },
        false,
        false
    ));
    // a maintenance god copies anything; a lesser god level does not
    assert!(task_preview_allowed(&theirs, agent, false, false, false, GOD_MAINTENANCE));
    assert!(!task_preview_allowed(&theirs, agent, false, false, false, 1));
    // a texture opens whatever its rights; an animation follows the object
    for owner_mask in [FULL, 0, COPY, MODIFY, TRANSFER] {
        let texture = InvItem {
            owner: other,
            owner_mask,
            everyone_mask: 0,
            asset_type: 0,
            inv_type: 0,
            ..Default::default()
        };
        assert!(open(&texture, false, false));
        let animation = InvItem {
            asset_type: 20,
            inv_type: 19,
            ..texture
        };
        assert!(!open(&animation, false, false));
        assert!(task_preview_allowed(&animation, agent, false, false, true, 0));
        assert!(task_preview_allowed(&animation, agent, false, false, false, 1));
    }
    // other types keep their own windows
    for asset_type in [6, 7, 10, 21] {
        assert!(!task_preview_allowed(
            &InvItem {
                asset_type,
                ..sound.clone()
            },
            agent,
            true,
            true,
            true,
            GOD_MAINTENANCE
        ));
    }
}

#[test]
fn task_asset_group_rules_keep_script_membership_and_notecard_manipulation_distinct() {
    let (agent, other) = (id(9961), id(9962));
    let mut script = aurora_net::build::TaskItem {
        owner_id: other,
        owner_mask: FULL,
        everyone_mask: TRANSFER,
        group_mask: COPY,
        ..Default::default()
    };
    let mut note = InvItem {
        owner: other,
        owner_mask: FULL,
        everyone_mask: TRANSFER,
        group_mask: COPY,
        ..Default::default()
    };
    assert_eq!(task_script_mask(&script, agent, false), TRANSFER);
    assert_eq!(task_script_mask(&script, agent, true), TRANSFER | COPY);
    assert_eq!(task_notecard_mask(&note, agent, false, false), TRANSFER);
    assert_eq!(task_notecard_mask(&note, agent, true, false), TRANSFER | COPY);
    script.group_owned = true;
    note.group_owned = true;
    assert_eq!(task_script_mask(&script, agent, true), FULL);
    assert_eq!(task_notecard_mask(&note, agent, true, false), TRANSFER | COPY);
    assert_eq!(task_notecard_mask(&note, agent, true, true), FULL);
    script.owner_id = agent;
    note.owner = agent;
    assert_eq!(task_script_mask(&script, agent, false), FULL);
    assert_eq!(task_notecard_mask(&note, agent, false, false), FULL);
}

#[test]
fn an_experience_logo_is_a_full_permission_texture_or_none() {
    let (agent, other, group, library, asset) = (id(9981), id(9982), id(9983), id(9984), id(9985));
    let mut inv = Inventory {
        owner: agent,
        ..Default::default()
    };
    assert!(experience_logo_allowed(&inv, Uuid::nil()), "no logo");
    assert!(!experience_logo_allowed(&inv, asset), "not in the inventory");
    let mut logo = |item: InvItem| {
        inv.items.clear();
        inv.items.insert(item.id, item);
        experience_logo_allowed(&inv, asset)
    };
    // the owner's, full permissions
    let mine = InvItem {
        id: id(9986),
        owner: agent,
        asset_id: asset,
        asset_type: 0,
        inv_type: 0,
        owner_mask: FULL,
        ..Default::default()
    };
    assert!(logo(mine.clone()));
    // no copy, no modify, no transfer
    for missing in [COPY, MODIFY, TRANSFER] {
        assert!(!logo(InvItem {
            owner_mask: FULL & !missing,
            ..mine.clone()
        }));
    }
    // group-owned: what it allows everyone (LLViewerInventoryItem::getPermissionMask)
    let shared = InvItem {
        owner: group,
        group_id: group,
        group_owned: true,
        group_mask: FULL,
        everyone_mask: 0,
        ..mine.clone()
    };
    assert!(!logo(shared.clone()));
    assert!(logo(InvItem {
        everyone_mask: FULL,
        ..shared
    }));
    // someone else's: their everyone mask
    let theirs = InvItem {
        owner: other,
        everyone_mask: COPY | TRANSFER,
        ..mine.clone()
    };
    assert!(!logo(theirs.clone()));
    assert!(logo(InvItem {
        everyone_mask: FULL,
        ..theirs
    }));
    // a Library texture: what the Library allows everyone
    let lib = InvItem {
        owner: library,
        everyone_mask: COPY,
        ..mine.clone()
    };
    assert!(!logo(lib.clone()));
    assert!(logo(InvItem {
        everyone_mask: FULL,
        ..lib
    }));
    // only a texture
    assert!(!logo(InvItem {
        asset_type: 7,
        inv_type: 7,
        ..mine
    }));
}

#[cfg(test)]
mod outgoing_offers {
    use super::*;
    use crate::world::inventory::outgoing::{Effect, Feedback};
    use aurora_net::{InventoryOfferStatus, NetCommand, inventory::ais::ItemReads};

    fn start(inv: &mut Inventory, item: Uuid) -> Result<NetCommand, Feedback> {
        let bucket = offer(inv, item, &HashSet::new()).expect("transferable synthetic original");
        inv.begin_offer(Some(id(41)), id(42), item, bucket)
    }

    fn prepare() -> (Inventory, Uuid, crossbeam_channel::Sender<InventoryOfferStatus>) {
        let mut inv = fixture();
        inv.owner = id(2);
        let item = id(8109);
        let NetCommand::OfferInventory { receipt, .. } = start(&mut inv, item).expect("offer") else {
            panic!("expected offer")
        };
        (inv, item, receipt)
    }

    fn cache(inv: &Inventory) -> CachedInventory {
        let path = std::env::temp_dir().join(format!("aurora-offer-{}.bin", Uuid::new_v4()));
        inv.save_cache(&path);
        let cached = read_cache(&path).expect("synthetic inventory snapshot");
        std::fs::remove_file(path).expect("remove synthetic snapshot");
        cached
    }

    #[test]
    fn closed_channel_neither_logs_a_gift_nor_removes_or_hides_the_original_at_next_login() {
        let (mut inv, item, receipt) = prepare();
        let folder = inv.items[&item].parent;
        // Dropping the command unsent, as a closed queue or the offline worker does.
        drop(receipt);
        assert!(matches!(&inv.poll_offers(Some(id(41)))[..], [Effect::Feedback(Feedback::Unsent)]));
        assert!(inv.items.contains_key(&item));
        let mut login = fixture();
        login.items.clear();
        for target in login.folders.values_mut() {
            target.items.clear();
            target.state = FetchState::Unknown;
        }
        login.apply_cache(cache(&inv));
        assert_eq!(login.folders[&folder].state, FetchState::Fetched);
        assert!(login.items.contains_key(&item));
        assert!(matches!(start(&mut inv, item), Ok(NetCommand::OfferInventory { .. })));
    }

    #[test]
    fn one_simulator_ack_emits_one_offer_and_protects_against_a_stale_cached_original() {
        let (mut inv, item, receipt) = prepare();
        let original = inv.items[&item].clone();
        receipt.send(InventoryOfferStatus::Transmitted).expect("status");
        assert!(inv.poll_offers(Some(id(41))).is_empty());
        assert!(inv.items.contains_key(&item));
        assert!(matches!(start(&mut inv, item), Err(Feedback::Pending)));
        assert!(!cache(&inv).folders.iter().any(|f| f.id == original.parent));
        receipt.send(InventoryOfferStatus::Acknowledged).expect("ack");
        assert!(matches!(&inv.poll_offers(Some(id(41)))[..], [Effect::Sent { recipient, folder: false, .. }] if *recipient == id(42)));
        assert!(inv.poll_offers(Some(id(41))).is_empty());
        assert!(!inv.items.contains_key(&item));
        assert!(!cache(&inv).folders.iter().any(|f| f.id == original.parent));
        inv.add_items(vec![original]); // a pre-offer inventory snapshot arrived late
        assert!(matches!(start(&mut inv, item), Ok(NetCommand::CheckInventoryOffer { .. })));
    }

    fn lost_ack(inv: &mut Inventory, receipt: crossbeam_channel::Sender<InventoryOfferStatus>) -> crossbeam_channel::Sender<ItemReads> {
        receipt.send(InventoryOfferStatus::Transmitted).expect("status");
        drop(receipt);
        let mut effects = inv.poll_offers(Some(id(41)));
        assert!(matches!(&effects[..], [Effect::Feedback(Feedback::Uncertain), Effect::Check(_)]));
        let Some(Effect::Check(cmd)) = effects.pop() else {
            panic!("expected check")
        };
        let NetCommand::CheckInventoryOffer { session, items, reply } = *cmd else {
            panic!("expected item read")
        };
        assert_eq!(session, id(41));
        assert_eq!(items, vec![id(8109)]);
        reply
    }

    #[test]
    fn lost_ack_and_failed_read_never_release_an_original_or_repeat_a_gift() {
        let (mut inv, item, receipt) = prepare();
        let read = lost_ack(&mut inv, receipt);
        read.send(ItemReads {
            failed: vec![item],
            ..Default::default()
        })
        .expect("failure");
        assert!(matches!(
            &inv.poll_offers(Some(id(41)))[..],
            [Effect::Feedback(Feedback::CheckFailed)]
        ));
        assert!(inv.items.contains_key(&item));
        let NetCommand::CheckInventoryOffer { reply, .. } = start(&mut inv, item).expect("retry the check") else {
            panic!("sent a second gift")
        };
        assert!(matches!(start(&mut inv, item), Err(Feedback::Checking)));
        reply
            .send(ItemReads {
                missing: vec![item],
                ..Default::default()
            })
            .expect("absent");
        assert!(matches!(&inv.poll_offers(Some(id(41)))[..], [Effect::Feedback(Feedback::Checked)]));
        assert!(!inv.items.contains_key(&item));
        assert!(offer(&inv, item, &HashSet::new()).is_err());
    }

    #[test]
    fn a_timed_out_private_read_leaves_only_the_check_available_to_retry() {
        let (mut inv, item, receipt) = prepare();
        drop(lost_ack(&mut inv, receipt));
        assert!(matches!(
            &inv.poll_offers(Some(id(41)))[..],
            [Effect::Feedback(Feedback::CheckFailed)]
        ));
        assert!(inv.items.contains_key(&item));
        assert!(matches!(start(&mut inv, item), Ok(NetCommand::CheckInventoryOffer { .. })));
    }

    #[test]
    fn an_authoritative_original_uses_its_current_rights_without_resending() {
        let (mut inv, item, receipt) = prepare();
        let read = lost_ack(&mut inv, receipt);
        let mut changed = inv.items[&item].clone();
        changed.owner_mask &= !TRANSFER;
        read.send(ItemReads {
            items: vec![changed],
            ..Default::default()
        })
        .expect("response");
        assert!(matches!(&inv.poll_offers(Some(id(41)))[..], [Effect::Feedback(Feedback::Checked)]));
        assert!(inv.items.contains_key(&item));
        assert!(offer(&inv, item, &HashSet::new()).is_err());
    }

    #[test]
    fn a_copyable_offer_to_two_residents_confirms_each_once_and_keeps_the_item() {
        let mut inv = fixture();
        let item = id(8100);
        let mut receipts = Vec::new();
        for recipient in [id(42), id(43)] {
            let bucket = offer(&inv, item, &HashSet::new()).expect("copyable object");
            let NetCommand::OfferInventory { receipt, .. } =
                inv.begin_offer(Some(id(41)), recipient, item, bucket).expect("independent offer")
            else {
                panic!("expected an offer");
            };
            receipts.push(receipt);
        }
        for receipt in receipts {
            receipt.send(InventoryOfferStatus::Acknowledged).expect("ack");
        }
        let effects = inv.poll_offers(Some(id(41)));
        assert!(
            matches!(&effects[..], [Effect::Sent { recipient: a, .. }, Effect::Sent { recipient: b, .. }] if *a == id(42) && *b == id(43))
        );
        assert!(inv.items.contains_key(&item));
        assert!(inv.outgoing.uncached_folders(&inv.items).is_empty());
    }

    #[test]
    fn a_read_issued_before_a_local_move_cannot_remove_the_moved_item_or_an_unrelated_item() {
        let (mut inv, item, receipt) = prepare();
        let read = lost_ack(&mut inv, receipt);
        let mut moved = inv.items[&item].clone();
        moved.parent = id(8001);
        inv.add_items(vec![moved.clone()]);
        let other = id(8100);
        let original_name = inv.items[&other].name.clone();
        let mut unrelated = inv.items[&other].clone();
        unrelated.name = "Unrequested".into();
        read.send(ItemReads {
            items: vec![unrelated],
            missing: vec![item, other],
            ..Default::default()
        })
        .expect("stale read");
        let reply = next_check(inv.poll_offers(Some(id(41))));
        assert_eq!(inv.items[&item].parent, moved.parent);
        assert_eq!(inv.items[&other].name, original_name);
        let uncached = inv.outgoing.uncached_folders(&inv.items);
        assert!(uncached.contains(&id(8000)) && uncached.contains(&moved.parent));
        reply
            .send(ItemReads {
                items: vec![moved],
                ..Default::default()
            })
            .expect("fresh read");
        assert!(matches!(&inv.poll_offers(Some(id(41)))[..], [Effect::Feedback(Feedback::Checked)]));
        assert!(inv.items.contains_key(&item));
        assert!(matches!(start(&mut inv, item), Ok(NetCommand::OfferInventory { .. })));
    }

    fn next_check(effects: Vec<Effect>) -> crossbeam_channel::Sender<ItemReads> {
        assert_eq!(effects.len(), 1);
        let Effect::Check(cmd) = effects.into_iter().next().expect("one follow-up") else {
            panic!("expected an inventory check")
        };
        let NetCommand::CheckInventoryOffer { reply, .. } = *cmd else {
            panic!("expected a private read")
        };
        reply
    }

    #[test]
    fn stale_reads_cannot_resurrect_deleted_originals_or_overwrite_current_rights() {
        for deleted in [true, false] {
            let (mut inv, item, receipt) = prepare();
            let read = lost_ack(&mut inv, receipt);
            let old = inv.items[&item].clone();
            if deleted {
                inv.remove(&[item]);
            } else {
                let mut changed = old.clone();
                changed.owner_mask &= !TRANSFER;
                inv.add_items(vec![changed]);
            }
            read.send(ItemReads {
                items: vec![old],
                ..Default::default()
            })
            .expect("stale reply");
            let retry = next_check(inv.poll_offers(Some(id(41))));
            assert!(offer(&inv, item, &HashSet::new()).is_err());
            let mut fresh = ItemReads::default();
            if deleted {
                fresh.missing.push(item);
            } else {
                fresh.items.push(inv.items[&item].clone());
            }
            retry.send(fresh).expect("fresh reply");
            assert!(matches!(&inv.poll_offers(Some(id(41)))[..], [Effect::Feedback(Feedback::Checked)]));
            assert!(offer(&inv, item, &HashSet::new()).is_err());
        }
    }

    #[test]
    fn a_pending_logout_write_does_not_restore_the_previous_inventory_snapshot() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("aurora-offer-pending-{}", Uuid::new_v4()));
        let path = crate::cache::path(&dir, "inventory", &id(2), "inv");
        let other_path = crate::cache::path(&dir, "inventory", &id(3), "inv");
        let mut inventory = fixture();
        let item = id(8109);
        let parent = inventory.items[&item].parent;
        inventory.save_cache(&path);
        let old = read_cache(&path).expect("snapshot from before the give");
        assert!(old.folders.iter().any(|f| f.items.iter().any(|it| it.id == item)));
        let (release, blocked) = crossbeam_channel::bounded(1);
        let (started, writing) = crossbeam_channel::bounded(1);
        let before = crate::cache::read(&path).expect("first cache");
        let in_flight = path.clone();
        crate::saves::change(path.clone(), move || {
            started.send(()).expect("writer started");
            if blocked.recv().is_ok() {
                crate::cache::write(&in_flight, &before).expect("write late snapshot");
            }
        });
        writing
            .recv_timeout(Duration::from_secs(10))
            .expect("writer holds this synthetic path");
        inventory.remove(&[item]);
        inventory.save_cache(&path);
        let stale = read_cache_with_timeout(&path, Duration::ZERO);
        let mut other = fixture();
        other.owner = id(3);
        let mut other_item = other.items[&item].clone();
        other_item.id = id(9801);
        other_item.owner = other.owner;
        other.items.clear();
        for folder in other.folders.values_mut() {
            folder.items.clear();
        }
        other.add_items(vec![other_item.clone()]);
        other.save_cache(&other_path);
        let independent = read_cache(&other_path).expect("second account reads its own cache while the first account is blocked");
        release.send(()).expect("release the writer");
        assert!(
            independent
                .folders
                .iter()
                .flat_map(|f| &f.items)
                .any(|it| it.id == other_item.id && it.owner == other.owner)
        );
        assert!(independent.folders.iter().all(|f| f.items.iter().all(|it| it.id != item)));
        assert!(stale.is_none(), "the old snapshot cannot stand for the pending one");
        let fresh = read_cache(&path).expect("new snapshot");
        assert!(fresh.folders.iter().any(|f| f.id == parent));
        assert!(
            fresh
                .folders
                .iter()
                .all(|f| f.items.iter().all(|it| it.id != item && it.id != other_item.id))
        );
        std::fs::remove_file(path).expect("remove first cache");
        std::fs::remove_file(other_path).expect("remove second cache");
        std::fs::remove_dir(dir.join("inventory")).expect("empty inventory directory");
        std::fs::remove_dir(dir).expect("empty synthetic cache");
    }
    #[test]
    fn a_v3_cache_missing_a_no_copy_original_refetches_once_and_reuses_the_rebuilt_folder() {
        use aurora_llsd::llsd_map;
        let old = fixture();
        let item = old.items[&id(8109)].clone();
        let parent = item.parent;
        let version = old.folders[&parent].info.version;
        let row = llsd_map! { "id" => parent, "version" => version };
        let snapshot = llsd_map! { "version" => 3, "folders" => Llsd::Array(vec![row]), "items" => Llsd::Array(vec![]) };
        let path = std::env::temp_dir().join(format!("aurora-offer-v3-{}.bin", Uuid::new_v4()));
        crate::cache::write(&path, &aurora_llsd::to_binary(&snapshot)).expect("synthetic v3 snapshot");
        let skeleton = || {
            let mut inv = fixture();
            inv.items.clear();
            inv.queue.clear();
            for folder in inv.folders.values_mut() {
                folder.items.clear();
                folder.state = FetchState::Unknown;
            }
            inv
        };
        let mut first = skeleton();
        assert!(read_cache(&path).is_none(), "v3 could omit this original at the same version");
        first.request(parent);
        assert_eq!(first.queue, [(parent, false)]);
        first.apply(vec![FolderContents {
            folder_id: parent,
            owner_id: id(2),
            version,
            folders: vec![],
            items: vec![item.clone()],
        }]);
        assert!(first.items.contains_key(&item.id));
        first.save_cache(&path);
        let mut second = skeleton();
        assert_eq!(second.apply_cache(read_cache(&path).expect("current snapshot")), 1);
        second.request(parent);
        assert!(second.queue.is_empty(), "the rebuilt folder is reused at the next login");
        assert_eq!(second.items[&item.id].id, item.id);
        assert_eq!(second.items[&item.id].owner_mask, item.owner_mask);
        std::fs::remove_file(path).expect("remove synthetic cache");
    }
    #[test]
    fn late_receipts_and_private_reads_cannot_mutate_a_new_login() {
        let (mut inv, item, receipt) = prepare();
        receipt.send(InventoryOfferStatus::Acknowledged).expect("late ack");
        assert!(inv.poll_offers(Some(id(43))).is_empty());
        assert!(inv.items.contains_key(&item));
        let (mut inv, item, receipt) = prepare();
        let read = lost_ack(&mut inv, receipt);
        read.send(ItemReads {
            missing: vec![item],
            ..Default::default()
        })
        .expect("late read");
        assert!(inv.poll_offers(Some(id(43))).is_empty());
        assert!(inv.items.contains_key(&item));
    }
}

/// A link's original found unreadable after its attempts is never read
/// again: on a huge inventory, dead links read over and over took requests
/// from the folder walk.
#[test]
fn a_broken_links_original_is_never_read_again() {
    let mut inv = Inventory::default();
    let dead = Uuid::from_u128(77);
    let other = Uuid::from_u128(78);
    for _ in 0..ITEM_FETCH_ATTEMPTS {
        inv.items_failed(&[dead]);
    }
    let mut asked = vec![dead, other];
    inv.drop_broken(&mut asked);
    assert_eq!(asked, vec![other]);
}

/// An item already on its way is never asked twice: copies of the same
/// read counted their failures as attempts, twenty at once. Its answer, its
/// failure or the deadline frees it.
#[test]
fn an_item_on_its_way_is_never_read_twice() {
    let mut inv = Inventory::default();
    let (a, b) = (Uuid::from_u128(77), Uuid::from_u128(78));
    let now = std::time::Instant::now();
    let mut first = vec![a, b, a];
    inv.claim_item_reads(&mut first, false, now);
    assert_eq!(first, vec![a, b], "once each");
    let mut again = vec![a, b];
    inv.claim_item_reads(&mut again, false, now);
    assert!(again.is_empty(), "both still on their way");
    // (a worn attachment's read is filed differently: it still goes)
    let mut worn = vec![a];
    inv.claim_item_reads(&mut worn, true, now);
    assert_eq!(worn, vec![a]);
    assert!(inv.item_reading(a, true, now));
    // a failure counts once, then the item may be asked again
    inv.items_answered(&[a], false);
    inv.items_failed(&[a]);
    assert_eq!(inv.item_failures[&a], 1);
    let mut retry = vec![a, b];
    inv.claim_item_reads(&mut retry, false, now);
    assert_eq!(retry, vec![a]);
    // a read never answered frees its item after the deadline
    let mut late = vec![b];
    inv.claim_item_reads(&mut late, false, now + ITEM_READ_DEADLINE);
    assert_eq!(late, vec![b]);
    // a late copy of a broken link's read counts nothing more
    for _ in 0..ITEM_FETCH_ATTEMPTS {
        inv.items_failed(&[a]);
    }
    let attempts = inv.item_failures[&a];
    inv.items_failed(&[a]);
    assert_eq!(inv.item_failures[&a], attempts);
    let mut dead = vec![a];
    inv.claim_item_reads(&mut dead, false, now + ITEM_READ_DEADLINE);
    assert!(dead.is_empty(), "a broken link is never read again");
}

/// A refresh of an item already in the inventory (a script saved twice, a
/// gesture or an AO item changed) is always sent, as on the Legacy Viewer:
/// the newest read writes last.
#[test]
fn a_refresh_of_a_known_item_is_never_swallowed() {
    let mut inv = Inventory::default();
    let script = Uuid::from_u128(77);
    inv.items.insert(
        script,
        InvItem {
            id: script,
            ..Default::default()
        },
    );
    let now = std::time::Instant::now();
    for _ in 0..2 {
        let mut refresh = vec![script];
        inv.claim_item_reads(&mut refresh, false, now);
        assert_eq!(refresh, vec![script], "each save's refresh is sent");
    }
    assert!(!inv.item_reading(script, false, now), "no claim on a known item");
}

/// An answer releases only its own kind's claim: an ordinary read's
/// failure never lets a second attachment read of the same item go while
/// the first is still on its way.
#[test]
fn an_answer_frees_only_its_own_kind_of_read() {
    let mut inv = Inventory::default();
    let a = Uuid::from_u128(77);
    let now = std::time::Instant::now();
    let (mut plain, mut worn) = (vec![a], vec![a]);
    inv.claim_item_reads(&mut plain, false, now);
    inv.claim_item_reads(&mut worn, true, now);
    inv.items_answered(&[a], false);
    inv.items_failed(&[a]);
    assert!(inv.item_reading(a, true, now), "the attachment read is still on its way");
    let mut again = vec![a];
    inv.claim_item_reads(&mut again, true, now);
    assert!(again.is_empty());
}

#[test]
fn a_coalesced_failure_releases_both_claims_and_spends_one_attempt() {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let id = Uuid::from_u128(77);
    let now = std::time::Instant::now();
    for attachment in [false, true] {
        world.inventory.claim_item_reads(&mut vec![id], attachment, now);
    }
    world.apply(aurora_net::NetEvent::InventoryItemsFailed {
        items: vec![id],
        attachment: false,
        coalesced: true,
    });
    assert!(!world.inventory.item_reading(id, false, now));
    assert!(!world.inventory.item_reading(id, true, now));
    assert_eq!(world.inventory.item_failures[&id], 1);
}

/// The later chunks of a large item read are claimed for as long as the
/// chunks before them can take: never asked again while still queued.
#[test]
fn a_large_item_reads_later_chunks_keep_their_claim() {
    let mut inv = Inventory::default();
    let chunk = aurora_net::inventory::ais::ITEM_CHUNK;
    let ids: Vec<Uuid> = (0..chunk as u128 * 3).map(|n| Uuid::from_u128(1000 + n)).collect();
    let now = std::time::Instant::now();
    let mut read = ids.clone();
    inv.claim_item_reads(&mut read, false, now);
    let after_one = now + ITEM_READ_DEADLINE;
    assert!(!inv.item_reading(ids[0], false, after_one), "the first chunk's deadline passed");
    assert!(
        inv.item_reading(ids[chunk * 2], false, after_one),
        "the third chunk may still be queued"
    );
    assert!(!inv.item_reading(ids[chunk * 2], false, now + ITEM_READ_DEADLINE * 3));
}

/// An unreadable item is given up and read again a few rounds a session,
/// then stays unreadable without being marked broken.
#[test]
fn an_unreadable_item_is_read_again_only_a_few_rounds() {
    let mut inv = Inventory::default();
    let busy = Uuid::from_u128(77);
    for round in 0..=ITEM_GIVE_UP_ROUNDS {
        for _ in 0..ITEM_FETCH_ATTEMPTS {
            inv.items_failed(&[busy]);
        }
        assert!(inv.read_exhausted(&busy));
        assert!(!inv.broken(&busy));
        if round == ITEM_GIVE_UP_ROUNDS {
            assert!(!inv.gave_up_at.contains_key(&busy), "the last round stays exhausted");
            break;
        }
        *inv.gave_up_at.get_mut(&busy).expect("given up") -= ITEM_GIVE_UP;
        inv.expire_given_up(std::time::Instant::now());
        assert!(!inv.read_exhausted(&busy), "round {round}: read again");
    }
    inv.expire_given_up(std::time::Instant::now() + ITEM_GIVE_UP * 2);
    assert!(inv.read_exhausted(&busy));
    assert!(!inv.broken(&busy));
    assert_eq!(inv.unreadable_items(), 1);
}

/// A server busy for a while never marks a real item broken for the
/// session: one only unreadable is read again after the pause; one the
/// server answered missing never.
#[test]
fn an_unreadable_item_is_read_again_after_a_pause() {
    let mut inv = Inventory::default();
    let (busy, gone) = (Uuid::from_u128(77), Uuid::from_u128(78));
    for _ in 0..ITEM_FETCH_ATTEMPTS {
        inv.items_failed(&[busy]);
    }
    inv.items_missing(vec![gone]);
    let mut asked = vec![busy, gone];
    inv.drop_broken(&mut asked);
    assert!(asked.is_empty(), "both left out for now");
    // past the pause
    *inv.gave_up_at.get_mut(&busy).expect("given up") -= ITEM_GIVE_UP;
    let mut asked = vec![busy, gone];
    inv.drop_broken(&mut asked);
    assert_eq!(asked, vec![busy], "only the unreadable one is read again");
}

/// A folder read the user starts changes the generation: the background
/// walk's idle sweep looks again and times it, even when the walk's pump
/// did not run before the app took the queue (no capabilities yet).
#[test]
fn a_users_folder_read_changes_the_generation() {
    let mut inv = fixture();
    let folder = Uuid::from_u128(500);
    inv.folders.insert(
        folder,
        Folder {
            info: InvFolder {
                id: folder,
                parent: inv.root,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Unknown,
            library: false,
        },
    );
    let before = inv.generation;
    inv.request(folder);
    assert_eq!(inv.folders[&folder].state, FetchState::Fetching);
    assert!(inv.generation > before);
    let after = inv.generation;
    inv.request(folder);
    assert_eq!(inv.generation, after, "already on its way: nothing changes");
}

#[test]
fn folders_set_back_to_unread_change_the_generation() {
    let mut inv = fixture();
    let folder = Uuid::from_u128(600);
    inv.folders.insert(
        folder,
        Folder {
            info: InvFolder {
                id: folder,
                parent: inv.root,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    let before = inv.generation;
    inv.mark_unread([folder]);
    assert_eq!(inv.folders[&folder].state, FetchState::Unknown);
    assert!(inv.generation > before, "the background walk's sweep looks again");
    let after = inv.generation;
    inv.mark_unread([folder, Uuid::from_u128(601)]);
    assert_eq!(inv.generation, after, "nothing changed: no new generation");
}
