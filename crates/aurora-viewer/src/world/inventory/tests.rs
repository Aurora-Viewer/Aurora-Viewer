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
