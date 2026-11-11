use super::*;
use crate::world::inventory::{Folder, actions::demo_mutate};
use aurora_net::inventory::{InvFolder, InvItem};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

const AGENT: u128 = 2;

fn folder(inv: &mut Inventory, id: Uuid, parent: Uuid, name: &str, library: bool) {
    inv.folders.insert(
        id,
        Folder {
            info: InvFolder {
                id,
                parent,
                name: name.into(),
                type_default: -1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library,
        },
    );
    if let Some(p) = inv.folders.get_mut(&parent) {
        p.children.push(id);
    }
}

fn item(inv: &mut Inventory, n: u128, parent: Uuid, name: &str, asset_type: i32, flags: u32) {
    inv.add_items(vec![InvItem {
        id: id(n),
        parent,
        name: name.into(),
        asset_type,
        inv_type: if asset_type == 6 { 17 } else { 18 },
        flags,
        ..Default::default()
    }]);
}

fn link(inv: &mut Inventory, n: u128, parent: Uuid, target: u128, created_at: i64) {
    inv.add_items(vec![InvItem {
        id: id(n),
        parent,
        name: format!("link {target}"),
        asset_type: 24,
        asset_id: id(target),
        created_at,
        ..Default::default()
    }]);
}

/// A root, a Clothing folder of wearables (a body part, clothing, objects
/// with names that sort apart from their types), a notecard and a Library
/// shirt.
fn fixture() -> Inventory {
    let mut inv = Inventory {
        root: id(1),
        owner: id(AGENT),
        ..Default::default()
    };
    folder(&mut inv, id(1), Uuid::nil(), "My Inventory", false);
    folder(&mut inv, id(10), id(1), "Clothing", false);
    item(&mut inv, 100, id(10), "Zest skin", 13, 1);
    item(&mut inv, 101, id(10), "Blue shirt", 5, 4);
    item(&mut inv, 102, id(10), "Anklet", 6, 7);
    item(&mut inv, 103, id(10), "Coat", 5, 6);
    item(&mut inv, 104, id(10), "Notes", 7, 0);
    folder(&mut inv, id(20), Uuid::nil(), "Library", true);
    item(&mut inv, 105, id(20), "Library shirt", 5, 4);
    inv
}

fn with_favorites() -> (Inventory, Uuid) {
    let mut inv = fixture();
    let change = create(&inv).expect("create");
    demo_mutate(&mut inv, id(AGENT), change).expect("created");
    let root = root(&inv).expect("root");
    (inv, root)
}

#[test]
fn the_folder_is_made_once_under_aurora_beside_an_existing_ao() {
    let mut inv = fixture();
    let change = create(&inv).expect("missing folder");
    let names: Vec<_> = change
        .operations
        .iter()
        .map(|op| match op {
            Operation::Folder { name, .. } => name.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(names, [BRAND, FOLDER]);
    demo_mutate(&mut inv, id(AGENT), change).expect("created");
    let made = root(&inv).expect("root");
    assert_eq!(inv.folders[&made].info.name, FOLDER);
    assert!(create(&inv).is_none(), "never a second folder");

    // The AO's `#Aurora` already there: only the favorites folder, in it.
    let mut inv = fixture();
    folder(&mut inv, id(30), id(1), BRAND, false);
    folder(&mut inv, id(31), id(30), "Aurora AO", false);
    let change = create(&inv).expect("missing folder");
    assert!(
        matches!(change.operations.as_slice(), [Operation::Folder { parent, name, .. }] if *parent == id(30) && name == FOLDER),
        "{:?}",
        change.operations
    );

    // A favorites folder in the Library or under another root is not ours.
    let mut inv = fixture();
    folder(&mut inv, id(40), id(20), BRAND, true);
    folder(&mut inv, id(41), id(40), FOLDER, true);
    assert!(root(&inv).is_none());

    // No root yet: nothing is created that the skeleton could duplicate.
    let inv = Inventory::default();
    assert!(create(&inv).is_none());
}

#[test]
fn the_root_lists_each_wearable_once_and_a_tab_only_its_own() {
    let (mut inv, root) = with_favorites();
    folder(&mut inv, id(50), root, "Work", false);
    folder(&mut inv, id(51), root, "date night", false);
    link(&mut inv, 200, root, 101, 10);
    link(&mut inv, 201, id(50), 101, 11);
    link(&mut inv, 202, id(50), 103, 12);
    link(&mut inv, 203, id(51), 102, 13);
    link(&mut inv, 204, root, 104, 14); // a notecard: not a wearable
    link(&mut inv, 205, root, 999, 15); // broken
    let mut all: Vec<Uuid> = entries(&inv, root).iter().map(|e| e.item).collect();
    all.sort();
    assert_eq!(all, [id(101), id(102), id(103)]);
    let work: Vec<Entry> = entries(&inv, id(50));
    assert_eq!(
        work.iter().map(|e| (e.link, e.item)).collect::<Vec<_>>(),
        [(id(201), id(101)), (id(202), id(103))]
    );
    let names: Vec<_> = folders(&inv, root).into_iter().map(|(_, n)| n).collect();
    assert_eq!(names, ["date night", "Work"], "by name, case ignored");
}

#[test]
fn each_sort_follows_its_own_key() {
    let (mut inv, root) = with_favorites();
    // Added in an order that matches none of the sorts.
    link(&mut inv, 200, root, 103, 30); // Coat, clothing
    link(&mut inv, 201, root, 100, 10); // Zest skin, body part
    link(&mut inv, 202, root, 102, 40); // Anklet, object
    link(&mut inv, 203, root, 101, 20); // Blue shirt, clothing
    let order = |sort_by: Sort| {
        let mut list = entries(&inv, root);
        sort(&inv, &mut list, sort_by);
        list.into_iter().map(|e| inv.items[&e.item].name.clone()).collect::<Vec<_>>()
    };
    assert_eq!(order(Sort::Name), ["Anklet", "Blue shirt", "Coat", "Zest skin"]);
    assert_eq!(order(Sort::Recent), ["Anklet", "Coat", "Blue shirt", "Zest skin"]);
    // body parts, clothing by wearable type (shirt 4 before jacket 6), objects
    assert_eq!(order(Sort::Type), ["Zest skin", "Blue shirt", "Coat", "Anklet"]);
}

#[test]
fn the_sort_keeps_the_legacy_setting_values() {
    for (value, sort) in [(0, Sort::Name), (1, Sort::Recent), (3, Sort::Type)] {
        assert_eq!(Sort::from(value), sort);
        assert_eq!(u8::from(sort), value);
    }
    // E_SORT_BY_TYPE_LAYER (2) has no menu entry: the default instead.
    assert_eq!(Sort::from(2), Sort::Type);
    assert_eq!(Sort::default(), Sort::Type);
}

#[test]
fn a_drop_adds_wearable_originals_once_and_nothing_else() {
    let (mut inv, root) = with_favorites();
    link(&mut inv, 200, root, 101, 1);
    // an inventory link to the coat, as dragged from an outfit
    link(&mut inv, 300, id(10), 103, 2);
    let dropped = [id(101), id(300), id(103), id(102), id(104), id(105), id(999)];
    assert_eq!(
        addable(&inv, root, &dropped),
        [id(103), id(102)],
        "the shirt is already there, the coat link is the coat once, no notecard, no Library item"
    );
    let change = add(&inv, root, &addable(&inv, root, &dropped)).expect("links");
    demo_mutate(&mut inv, id(AGENT), change).expect("linked");
    let mut held: Vec<Uuid> = entries(&inv, root).iter().map(|e| e.item).collect();
    held.sort();
    assert_eq!(held, [id(101), id(102), id(103)]);
    assert!(add(&inv, root, &[]).is_none());
}

#[test]
fn removing_takes_the_link_and_never_the_wearable() {
    let (mut inv, root) = with_favorites();
    link(&mut inv, 200, root, 101, 1);
    assert!(remove(&inv, &[id(101)]).is_none(), "an original is never deleted");
    let change = remove(&inv, &[id(200), id(101)]).expect("the link");
    assert!(matches!(change.operations.as_slice(), [Operation::Delete { id: gone, folder: false }] if *gone == id(200)));
    assert_eq!(change.refresh, [root]);
    demo_mutate(&mut inv, id(AGENT), change).expect("removed");
    assert!(entries(&inv, root).is_empty());
    assert!(inv.items.contains_key(&id(101)));
}

#[test]
fn folders_are_named_renamed_and_deleted_like_the_legacy_window() {
    let (mut inv, root) = with_favorites();
    assert_eq!(folder_name("  Work  ").as_deref(), Ok("Work"));
    assert!(folder_name("a/b").is_err());
    assert!(folder_name("a\\b").is_err());
    assert!(folder_name("   ").is_err());
    let (made, change) = new_folder(root, " Work ").expect("folder");
    demo_mutate(&mut inv, id(AGENT), change).expect("created");
    assert_eq!(folders(&inv, root), [(made, "Work".to_string())]);

    assert!(rename_folder(&inv, root, root, "Other").is_err(), "the root keeps its name");
    let change = rename_folder(&inv, root, made, "Office").expect("rename");
    demo_mutate(&mut inv, id(AGENT), change).expect("renamed");
    assert_eq!(inv.folders[&made].info.name, "Office");

    assert!(delete_folder(&inv, root, root).is_none(), "never the root");
    link(&mut inv, 200, made, 101, 1);
    let change = delete_folder(&inv, root, made).expect("links only");
    assert!(matches!(change.operations.as_slice(), [Operation::Delete { id: gone, folder: true }] if *gone == made));
    // A real item put there from the inventory would be lost with it: refused.
    item(&mut inv, 400, made, "My own shirt", 5, 4);
    assert!(delete_folder(&inv, root, made).is_none());
}

#[test]
fn the_search_needs_every_word() {
    let (mut inv, root) = with_favorites();
    link(&mut inv, 200, root, 101, 1);
    let entry = entries(&inv, root)[0];
    assert!(matches(&inv, &entry, "SHIRT blue"));
    assert!(matches(&inv, &entry, ""));
    assert!(!matches(&inv, &entry, "blue coat"));
}
