use super::*;
use crate::world::inventory::{FetchState, Folder, actions::demo_mutate};
use aurora_net::inventory::{InvFolder, InvItem};

const AGENT: Uuid = Uuid::from_u128(2);

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn bare() -> Inventory {
    let mut inv = Inventory {
        root: id(1),
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
    crate::world::appearance::seed_demo(&mut inv, AGENT);
    inv
}

fn seeded() -> Inventory {
    let mut inv = bare();
    seed_demo(&mut inv, AGENT);
    inv
}

fn targets(entries: &[Entry]) -> Vec<u128> {
    entries.iter().map(|e| e.target.as_u128()).collect()
}

#[test]
fn the_folder_is_created_once_in_the_viewer_folder() {
    let mut inv = bare();
    assert!(folder(&inv).is_none());
    let change = create_plan(&inv).expect("both folders to create");
    // #Aurora, then #Wearable Favorites inside it
    let names: Vec<_> = change
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Folder { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, [ROOT, FOLDER]);
    demo_mutate(&mut inv, AGENT, change).expect("created");
    let favorites = folder(&inv).expect("found by name");
    assert!(create_plan(&inv).is_none(), "nothing more to create");
    // an existing #Aurora only gets the subfolder
    let mut inv2 = bare();
    let (brand, _) = create_folder(inv2.root, ROOT);
    demo_mutate(&mut inv2, AGENT, brand).expect("brand");
    let change = create_plan(&inv2).expect("subfolder");
    assert_eq!(change.operations.len(), 1);
    assert!(entries(&inv, favorites, favorites).is_empty());
}

#[test]
fn the_whole_list_shows_each_original_once_and_a_folder_its_own() {
    let inv = seeded();
    let root = folder(&inv).expect("favorites");
    let all = entries(&inv, root, root);
    // the Evening folder first (subfolders before items), each original once
    let listed = targets(&all);
    assert_eq!(listed[..2], [715, 719]);
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(sorted, [712, 714, 715, 717, 719, 720, 722, 723]);
    let (evening, name) = subfolders(&inv, root).into_iter().next().expect("Evening");
    assert_eq!(name, "Evening");
    assert_eq!(targets(&entries(&inv, root, evening)), [715, 719]);
    // a link the root view drops is still the folder's own row
    assert!(entries(&inv, root, evening).iter().all(|e| inv.items[&e.link].parent == evening));
    assert!(loaded(&inv, root));
}

#[test]
fn broken_links_and_other_types_are_not_listed() {
    let mut inv = seeded();
    let root = folder(&inv).expect("favorites");
    inv.add_items(vec![
        InvItem {
            id: id(900),
            parent: root,
            name: "gone".into(),
            asset_type: 24,
            asset_id: id(901),
            ..Default::default()
        },
        InvItem {
            id: id(902),
            parent: root,
            name: "a notecard".into(),
            asset_type: 7,
            ..Default::default()
        },
        InvItem {
            id: id(903),
            parent: root,
            name: "a gesture".into(),
            asset_type: 21,
            ..Default::default()
        },
    ]);
    let all = entries(&inv, root, root);
    assert!(!all.iter().any(|e| [900, 901, 902].contains(&e.target.as_u128())));
    // LLFindOutfitItems keeps gestures
    assert!(all.iter().any(|e| e.target == id(903) && e.link == id(903)));
}

#[test]
fn the_three_orders_follow_the_legacy_comparators() {
    let inv = seeded();
    let root = folder(&inv).expect("favorites");
    let mut all = entries(&inv, root, root);
    let names = |all: &[Entry]| -> Vec<String> { all.iter().map(|e| inv.items[&e.target].name.clone()).collect() };
    sort(&inv, &mut all, SortOrder::Name);
    let by_name = names(&all);
    let mut expected = by_name.clone();
    expected.sort_by_key(|n| n.to_uppercase());
    assert_eq!(by_name, expected);
    sort(&inv, &mut all, SortOrder::MostRecent);
    // the newest original first (seed_demo dates them one day apart)
    let dates: Vec<i64> = all.iter().map(|e| inv.items[&e.target].created_at).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "{dates:?}");
    sort(&inv, &mut all, SortOrder::TypeName);
    // clothing (by wearable type), objects (by name), body parts
    let kinds: Vec<i32> = all.iter().map(|e| inv.items[&e.target].asset_type).collect();
    assert_eq!(kinds, [5, 5, 5, 6, 6, 6, 6, 13]);
    let clothing: Vec<u32> = all.iter().take(3).map(|e| inv.items[&e.target].flags & 0xff).collect();
    assert!(clothing.windows(2).all(|w| w[0] <= w[1]), "{clothing:?}");
    let objects = names(&all[3..7]);
    let mut sorted = objects.clone();
    sorted.sort_by_key(|n| n.to_uppercase());
    assert_eq!(objects, sorted);
}

#[test]
fn the_filter_reads_the_title_and_its_worn_tag() {
    assert!(matches("", "Boots", None));
    assert!(matches("  boo ", "Boots", None));
    assert!(!matches("hair", "Boots", None));
    assert!(matches("worn", "Boots", Some("worn on Left Foot")));
    assert!(!matches("worn", "Boots", None));
}

#[test]
fn a_drop_links_wearables_once_and_resolves_links() {
    let mut inv = seeded();
    let root = folder(&inv).expect("favorites");
    let (evening, _) = subfolders(&inv, root)[0].clone();
    let cof_link = inv
        .items
        .values()
        .find(|it| it.asset_type == 24 && it.asset_id == id(716))
        .map(|it| it.id)
        .expect("a COF link to the pants");
    inv.add_items(vec![InvItem {
        id: id(950),
        parent: inv.root,
        name: "a notecard".into(),
        asset_type: 7,
        ..Default::default()
    }]);
    // 715 is already in Evening; the notecard is no wearable; the link stands for 716
    let dropped = droppable(&inv, &[id(715), id(950), cof_link, id(716), id(713)], evening);
    assert_eq!(dropped, [id(716), id(713)]);
    let change = link(&inv, &dropped, evening).expect("links");
    demo_mutate(&mut inv, AGENT, change).expect("linked");
    assert_eq!(targets(&entries(&inv, root, evening)).len(), 4);
    assert!(link(&inv, &[], evening).is_none());
}

#[test]
fn remove_deletes_only_the_links() {
    let mut inv = seeded();
    let root = folder(&inv).expect("favorites");
    let all = entries(&inv, root, root);
    let row = all.iter().find(|e| e.target == id(720)).copied().expect("boots");
    let change = remove(&inv, &[row.link]).expect("removal");
    demo_mutate(&mut inv, AGENT, change).expect("removed");
    assert!(!entries(&inv, root, root).iter().any(|e| e.target == id(720)));
    assert!(inv.items.contains_key(&id(720)), "the original stays");
}

#[test]
fn folder_names_refuse_slashes_and_folders_rename_and_delete() {
    assert_eq!(folder_name("  Party  "), Ok("Party".into()));
    assert_eq!(folder_name("   "), Err(NameError::Empty));
    assert_eq!(folder_name("a/b"), Err(NameError::Slash("a/b".into())));
    assert_eq!(folder_name("a\\b"), Err(NameError::Slash("a\\b".into())));
    let mut inv = seeded();
    let root = folder(&inv).expect("favorites");
    let (change, _) = create_folder(root, "Party");
    demo_mutate(&mut inv, AGENT, change).expect("created");
    let names: Vec<String> = subfolders(&inv, root).into_iter().map(|(_, n)| n).collect();
    assert_eq!(names, ["Evening", "Party"]);
    let (evening, _) = subfolders(&inv, root)[0].clone();
    assert_eq!(entries(&inv, root, evening).len(), 2);
    let change = rename_folder(&inv, evening, "Zzz").expect("rename");
    demo_mutate(&mut inv, AGENT, change).expect("renamed");
    let names: Vec<String> = subfolders(&inv, root).into_iter().map(|(_, n)| n).collect();
    assert_eq!(names, ["Party", "Zzz"]);
    let change = delete_folder(&inv, evening).expect("delete");
    demo_mutate(&mut inv, AGENT, change).expect("deleted");
    assert_eq!(subfolders(&inv, root).len(), 1);
    assert!(inv.items.contains_key(&id(715)) && inv.items.contains_key(&id(719)));
}
