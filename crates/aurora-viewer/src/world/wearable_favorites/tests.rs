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

fn with_legacy() -> (Inventory, Uuid) {
    let mut inv = seeded();
    seed_demo_legacy(&mut inv, AGENT);
    let root = folder(&inv).expect("favorites");
    (inv, root)
}

fn link_targets(inv: &Inventory, parent: Uuid) -> Vec<u128> {
    let mut ids: Vec<u128> = inv.folders[&parent]
        .items
        .iter()
        .filter_map(|i| inv.items.get(i))
        .filter(|it| it.parent == parent)
        .map(|it| if it.asset_type == 24 { it.asset_id } else { it.id }.as_u128())
        .collect();
    ids.sort();
    ids
}

#[test]
fn the_legacy_folder_is_found_by_its_own_names() {
    let (inv, root) = with_legacy();
    let legacy = legacy_folder(&inv).expect("legacy favorites");
    assert_ne!(legacy, root);
    assert_eq!(inv.folders[&legacy].info.name, FOLDER);
    assert_eq!(LEGACY_ROOT, "#Firestorm");
    assert!(legacy_folder(&seeded()).is_none());
    assert!(import_plan(&seeded(), root).is_none(), "no Legacy folder, no offer");
}

#[test]
fn the_import_links_what_ours_lacks_at_the_same_place() {
    let (mut inv, root) = with_legacy();
    let before = inv.folders.len();
    let plan = import_plan(&inv, root).expect("something to import");
    // 713 at the top, 716 in Evening (715 is linked there already), 718 and
    // 721 in the new Party folder; the broken link and 712 (linked) are left
    assert_eq!(plan.links, 4);
    assert_eq!(plan.folders, 1);
    demo_mutate(&mut inv, AGENT, plan.change).expect("imported");
    assert_eq!(inv.folders.len(), before + 1, "only Party is created");
    let names: Vec<String> = subfolders(&inv, root).into_iter().map(|(_, n)| n).collect();
    assert_eq!(names, ["Evening", "Party"], "Evening is reused");
    let (evening, party) = (subfolders(&inv, root)[0].0, subfolders(&inv, root)[1].0);
    assert!(link_targets(&inv, root).contains(&713));
    assert_eq!(link_targets(&inv, evening), [715, 716, 719]);
    assert_eq!(link_targets(&inv, party), [718, 721]);
    // the Legacy folder and every original are untouched
    let legacy = legacy_folder(&inv).expect("legacy");
    assert_eq!(link_targets(&inv, legacy).len(), 3);
    assert!(inv.items.contains_key(&id(713)) && inv.items[&id(713)].parent != root);
    // running it again changes nothing
    assert!(import_plan(&inv, root).is_none());
}

#[test]
fn the_import_starts_from_an_empty_favorites_folder() {
    let (mut inv, root) = with_legacy();
    let rows: Vec<Uuid> = inv.folders[&root].items.clone();
    let change = remove(&inv, &rows).expect("removal");
    demo_mutate(&mut inv, AGENT, change).expect("emptied");
    let evening = subfolders(&inv, root)[0].0;
    let change = delete_folder(&inv, evening).expect("delete");
    demo_mutate(&mut inv, AGENT, change).expect("deleted");
    let plan = import_plan(&inv, root).expect("everything is new");
    // 712 and 713 at the top; Evening (715, 716) and Party (718, 721) are made
    assert_eq!((plan.links, plan.folders), (6, 2));
    demo_mutate(&mut inv, AGENT, plan.change).expect("imported");
    assert_eq!(link_targets(&inv, root), [712, 713]);
    assert_eq!(subfolders(&inv, root).len(), 2);
    assert!(import_plan(&inv, root).is_none());
}

#[test]
fn the_import_skips_library_items_and_waits_for_a_complete_tree() {
    let (mut inv, root) = with_legacy();
    let legacy = legacy_folder(&inv).expect("legacy");
    // 713 and 716 become Library items: nothing of them is linked
    for n in [713, 716] {
        let lib = Uuid::from_u128(0x11b0 + n);
        let mut folder = inv.folders[&legacy].clone();
        folder.info.id = lib;
        folder.children.clear();
        folder.items.clear();
        folder.library = true;
        inv.folders.insert(lib, folder);
        inv.items.get_mut(&id(n)).expect("original").parent = lib;
        inv.folders.get_mut(&lib).expect("lib").items.push(id(n));
    }
    let plan = import_plan(&inv, root).expect("718 and 721 remain");
    assert_eq!(plan.links, 2);
    assert!(plan.change.operations.iter().all(|op| match op {
        Operation::Links { links, .. } => links.iter().all(|l| ![id(713), id(716)].contains(&l.target)),
        _ => true,
    }));
    // a folder still being fetched: no offer
    inv.folders.get_mut(&legacy).expect("legacy").state = FetchState::Fetching;
    assert!(import_plan(&inv, root).is_none());
}

/// An original (not a link) put by hand into `parent`.
fn put_original(inv: &mut Inventory, parent: Uuid, n: u128) {
    inv.items.insert(
        id(n),
        InvItem {
            id: id(n),
            parent,
            name: "Hand-made".into(),
            asset_type: 5,
            owner: AGENT,
            creator: AGENT,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            ..Default::default()
        },
    );
    inv.folders.get_mut(&parent).expect("folder").items.push(id(n));
}

#[test]
fn remove_never_deletes_an_original_put_in_the_folder() {
    let mut inv = seeded();
    let root = folder(&inv).expect("favorites");
    put_original(&mut inv, root, 9001);
    let rows = entries(&inv, root, root);
    let original = rows.iter().find(|e| e.target == id(9001)).copied().expect("listed");
    assert_eq!(original.link, original.target, "an original stands for itself");
    assert!(!is_link(&inv, original.link));
    assert!(remove(&inv, &[original.link]).is_none(), "refused");
    // mixed with a link: only the link goes
    let link_row = rows.iter().find(|e| e.target == id(720)).copied().expect("boots");
    let change = remove(&inv, &[original.link, link_row.link]).expect("the link");
    assert_eq!(change.removed, [link_row.link]);
    assert!(
        change
            .operations
            .iter()
            .all(|op| matches!(op, Operation::Delete { id, .. } if *id == link_row.link))
    );
}

#[test]
fn deleting_a_folder_is_refused_while_its_tree_holds_an_original() {
    let mut inv = seeded();
    let root = folder(&inv).expect("favorites");
    let (evening, _) = subfolders(&inv, root)[0].clone();
    assert!(!holds_originals(&inv, evening));
    // an original in a subfolder of the subfolder
    let (change, deep) = create_folder(evening, "Deep");
    demo_mutate(&mut inv, AGENT, change).expect("created");
    let deep = inv.folders[&evening]
        .children
        .iter()
        .copied()
        .find(|c| inv.folders[c].info.name == "Deep")
        .unwrap_or(deep);
    put_original(&mut inv, deep, 9002);
    assert!(holds_originals(&inv, evening));
    assert!(delete_folder(&inv, evening).is_none());
    assert!(delete_folder(&inv, deep).is_none());
    assert!(inv.items.contains_key(&id(9002)));
    // once it is moved out, the folder can go
    inv.folders.get_mut(&deep).expect("deep").items.clear();
    assert!(delete_folder(&inv, evening).is_some());
}

#[test]
fn the_import_plan_is_linear_on_a_deep_chain_of_missing_folders() {
    let (mut inv, root) = with_legacy();
    let legacy = legacy_folder(&inv).expect("legacy");
    let mut parent = legacy;
    let depth = 40;
    for n in 0..depth {
        let fid = id(50_000 + n);
        inv.folders.insert(
            fid,
            crate::world::inventory::Folder {
                info: InvFolder {
                    id: fid,
                    parent,
                    name: format!("d{n}"),
                    type_default: -1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&parent).expect("parent").children.push(fid);
        parent = fid;
    }
    let link = id(60_000);
    inv.items.insert(
        link,
        InvItem {
            id: link,
            parent,
            name: "Deep link".into(),
            asset_type: 24,
            asset_id: id(719),
            owner: AGENT,
            ..Default::default()
        },
    );
    inv.folders.get_mut(&parent).expect("bottom").items.push(link);
    let started = std::time::Instant::now();
    let plan = import_plan(&inv, root).expect("something to import");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    // the earlier four links and one folder (Party), plus the whole chain
    assert_eq!(plan.folders, 1 + depth as usize);
    assert_eq!(plan.links, 5);
    let folder_ops = plan
        .change
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::Folder { .. }))
        .count();
    assert_eq!(folder_ops, plan.folders);
    // an empty chain creates nothing
    inv.folders.get_mut(&parent).expect("bottom").items.clear();
    let plan = import_plan(&inv, root).expect("the rest");
    assert_eq!(plan.folders, 1);
}

#[test]
fn deleting_a_folder_is_refused_while_a_subfolder_is_not_fetched() {
    let mut inv = seeded();
    let root = folder(&inv).expect("favorites");
    let (evening, _) = subfolders(&inv, root)[0].clone();
    assert!(delete_folder(&inv, evening).is_some());
    // a subfolder still being fetched: an original may be in it, unknown yet
    let (change, _) = create_folder(evening, "Later");
    demo_mutate(&mut inv, AGENT, change).expect("created");
    let later = inv.folders[&evening]
        .children
        .iter()
        .copied()
        .find(|c| inv.folders[c].info.name == "Later")
        .expect("later");
    inv.folders.get_mut(&later).expect("later").state = FetchState::Fetching;
    assert!(!holds_originals(&inv, evening));
    assert!(delete_folder(&inv, evening).is_none());
    inv.folders.get_mut(&later).expect("later").state = FetchState::Fetched;
    assert!(delete_folder(&inv, evening).is_some());
}
