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
    assert!(move_selection(&inv, &[id(8100)], trash, &HashSet::from([id(8000)]), &HashSet::new()).is_err());
    assert!(move_selection(&inv, &[id(8000)], trash, &HashSet::new(), &HashSet::from([id(8100)])).is_err());
    assert!(!destination(&inv, id(701), &HashSet::new()));
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
        assert!(renameable(&inv, pinned_id, &none));
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
    assert!(offer(&inv, id(8110), &HashSet::new(), &HashSet::new()).is_err());
    inv.remove(&[id(8110), id(8113)]);
    let bucket = offer(&inv, id(8000), &HashSet::new(), &HashSet::new()).expect("offer folder");
    assert_eq!(bucket[0], 8);
    assert_eq!(&bucket[1..17], id(8000).as_bytes());
    assert_eq!(bucket.len() % 17, 0);
    assert!(offer_has_no_copy(&inv, &bucket));
    assert!(!offer_has_no_copy(
        &inv,
        &offer(&inv, id(8100), &HashSet::new(), &HashSet::new()).expect("copyable item")
    ));
    assert!(offer(&inv, id(8100), &HashSet::new(), &HashSet::from([id(8100)])).is_err());
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
    assert_eq!(clean_name(&"é".repeat(90)).expect("bounded name").chars().count(), 63);
}
