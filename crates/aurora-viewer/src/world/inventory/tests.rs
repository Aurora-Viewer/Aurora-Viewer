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
fn selected_parent_and_child_are_moved_once_and_cycles_are_refused() {
    let inv = fixture();
    assert_eq!(roots(&inv, &[id(8100), id(8000), id(8100)]), vec![id(8000)]);
    assert!(paste(&inv, &[id(8000)], id(8001), true, false, &HashSet::new(), &HashSet::new()).is_err());
    assert!(paste(&inv, &[id(8000)], id(8001), false, false, &HashSet::new(), &HashSet::new()).is_err());
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
