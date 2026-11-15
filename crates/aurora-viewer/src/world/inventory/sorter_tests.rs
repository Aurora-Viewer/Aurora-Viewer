use super::*;
use crate::world::inventory::Folder;
use aurora_net::inventory::{InvFolder, InvItem};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

const AGENT: u128 = 2;
const OBJECTS: u128 = 10;

fn folder(id: Uuid, parent: Uuid, name: &str, kind: i32) -> Folder {
    Folder {
        info: InvFolder {
            id,
            parent,
            name: name.into(),
            type_default: kind,
            version: 1,
            ..Default::default()
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    }
}

/// My Inventory with an Objects folder, its contents named as creators do.
fn fixture(names: &[&str]) -> Inventory {
    let mut inv = Inventory {
        root: id(1),
        ..Default::default()
    };
    inv.folders.insert(id(1), folder(id(1), Uuid::nil(), "My Inventory", 8));
    inv.folders.insert(id(OBJECTS), folder(id(OBJECTS), id(1), "Objects", 6));
    inv.folders.insert(id(11), folder(id(11), id(1), "Trash", 14));
    inv.folders.get_mut(&id(1)).expect("root").children = vec![id(OBJECTS), id(11)];
    inv.add_items(
        names
            .iter()
            .enumerate()
            .map(|(n, name)| InvItem {
                id: id(100 + n as u128),
                parent: id(OBJECTS),
                name: (*name).into(),
                asset_type: 6,
                inv_type: 6,
                owner: id(AGENT),
                ..Default::default()
            })
            .collect(),
    );
    inv
}

fn path(name: &str) -> Option<String> {
    destination(name, 6, 0, &[]).map(|p| p.join("/"))
}

#[test]
fn creator_names_find_their_body_and_category() {
    for (name, expected) in [
        ("FemAvg_Dress_Cut_Long_Reborn_F", "Reborn/Dresses"),
        ("MAZE.mods - Soft Thighs V4.2 - eBody Reborn", "Reborn/Body add-ons"),
        (
            ". i i c i n g . // duo toned stockings - REBORN (add me)",
            "Reborn/Stockings & socks",
        ),
        ("(Enfer Sombre*) Reborn+Maze Stockings - Lamb", "Reborn/Stockings & socks"),
        ("VUDU TATTOO - Mother of Dragons - BoM/EVO-X [FATPACK] [REBORN]", "Reborn/Tattoos"),
        ("18.Loki - Knife Play GACHA - Black Ribbon Garter - Reborn", "Reborn/Lingerie"),
        ("2. Dhoma - Alyssa Lingerie Set Reborn Red RARE", "Reborn/Lingerie"),
        ("4. Dhoma Cosette Corset Reborn+Juicy Black", "Reborn/Lingerie"),
        ("18. Dhoma Cosette Robe Reborn+Juicy Black", "Reborn/Robes & sleepwear"),
        ("3. N-Uno - Kristina Shirt // Reborn", "Reborn/Tops"),
        ("REBORN - Waifu Boobs - Wear to unpack", "Reborn/Body add-ons"),
        ("Violent Seduction - Maia - Reborn (Black)", "Reborn"),
        (".SB. Angel Set - Reborn Pack [Unpacker]", "Reborn/Outfits & sets"),
        ("Velvet skirt [Legacy F]", "Legacy/Skirts"),
        ("Cargo pants - Legacy M", "Legacy Male/Pants"),
        ("Lara X Petite bikini", "Maitreya/Swimwear"),
        ("Bodysuit for Maitreya, Legacy and Reborn", "Several bodies/Bodysuits"),
        ("eBODY Classic sweater", "eBODY/Tops"),
        ("Black leather chair", "Furniture"),
    ] {
        assert_eq!(path(name).as_deref(), Some(expected), "{name}");
    }
}

#[test]
fn ebody_reborn_is_reborn_and_weak_words_need_no_other_body() {
    assert_eq!(classify("eBODY - REBORN - Wear to unpack", 6, 0).body, Some(Body::Named("Reborn")));
    assert_eq!(classify("eBODY Curvy top", 6, 0).body, Some(Body::Named("eBODY")));
    // "Juicy" alone names the Reborn add-on; beside Legacy it is a word
    assert_eq!(classify("Juicy rolls", 6, 0).body, Some(Body::Named("Reborn")));
    assert_eq!(classify("Juicy Muse top - Legacy", 6, 0).body, Some(Body::Named("Legacy")));
    // whole words only: "Unreborn" or "stop" name nothing
    assert_eq!(classify("Unreborn stop sign", 6, 0), Classified::default());
}

#[test]
fn unpackers_are_recognised_and_sorted_with_their_product() {
    for name in [
        "eBODY - REBORN - Wear to unpack",
        ".SB. Vixen Dress - Reborn Pack [Unpacker]",
        "rvn - my body reflection - reborn version (packed)",
    ] {
        assert!(classify(name, 6, 0).unpacker, "{name}");
    }
    assert!(!classify("Packer jacket", 6, 0).unpacker);
    assert_eq!(path(".SB. Vixen Dress - Reborn Pack [Unpacker]").as_deref(), Some("Reborn/Dresses"));
}

#[test]
fn the_asset_type_decides_when_the_name_says_nothing() {
    assert_eq!(destination("Welcome", 7, 0, &[]), Some(vec!["Notecards".to_owned()]));
    assert_eq!(destination("Store", 3, 0, &[]), Some(vec!["Landmarks".to_owned()]));
    // a clothing layer by its wearable type (skirt = 12), a body part (eyes = 3)
    assert_eq!(destination("Nyx", 5, 12, &[]), Some(vec!["Skirts".to_owned()]));
    assert_eq!(destination("Nyx", 13, 3, &[]), Some(vec!["Eyes".to_owned()]));
    assert_eq!(destination("Mystery box", 6, 0, &[]), None);
}

#[test]
fn user_rules_come_first_and_need_every_word() {
    let rules = vec![
        Rule {
            words: "chair black".into(),
            folder: "Home / Black chairs".into(),
        },
        Rule {
            words: "".into(),
            folder: "Ignored".into(),
        },
    ];
    assert_eq!(
        destination("Black leather CHAIR", 6, 0, &rules),
        Some(vec!["Home".to_owned(), "Black chairs".to_owned()])
    );
    assert_eq!(destination("White chair", 6, 0, &rules), Some(vec!["Furniture".to_owned()]));
}

#[test]
fn the_plan_keeps_worn_links_locked_and_sorted_folders_in_place() {
    let mut inv = fixture(&["Velvet skirt Reborn", "Lace bra Reborn", "Mystery box", "Reborn top link"]);
    // a link stays with its folder
    if let Some(link) = inv.items.get_mut(&id(103)) {
        link.asset_type = 24;
    }
    inv.generation += 1;
    // an unpacked folder moves as a whole, an already sorted name does not
    let unpacked = id(200);
    inv.folders
        .insert(unpacked, folder(unpacked, id(OBJECTS), ".SB. Vixen Dress Reborn", -1));
    inv.folders.insert(id(201), folder(id(201), id(OBJECTS), "Reborn", -1));
    inv.folders.get_mut(&id(OBJECTS)).expect("objects").children = vec![unpacked, id(201)];
    let worn: HashSet<Uuid> = [id(101)].into();
    let plan = plan(&inv, id(OBJECTS), &Preferences::default(), &HashSet::new(), &worn);
    let moved: Vec<_> = plan.moves.iter().map(|m| (m.name.as_str(), m.path.join("/"))).collect();
    assert_eq!(
        moved,
        [
            (".SB. Vixen Dress Reborn", "Reborn/Dresses".to_owned()),
            ("Velvet skirt Reborn", "Reborn/Skirts".to_owned())
        ]
    );
    assert_eq!(plan.unknown, ["Mystery box"]);
    // the worn bra and the link
    assert_eq!(plan.kept, 2);
    // a locked folder stays too
    let locked: HashSet<Uuid> = [unpacked].into();
    let plan = super::plan(&inv, id(OBJECTS), &Preferences::default(), &locked, &HashSet::new());
    assert!(plan.moves.iter().all(|m| m.id != unpacked));
}

#[test]
fn a_sort_creates_each_folder_once_reuses_existing_ones_and_is_idempotent() {
    let mut inv = fixture(&[
        "Velvet skirt Reborn",
        "Denim skirt REBORN",
        "Lace bra Reborn",
        "Silk dress Legacy",
        "Oak table",
    ]);
    // "sorted" exists already, in another case: it is reused
    inv.folders.insert(id(50), folder(id(50), id(1), "sorted", -1));
    inv.folders.get_mut(&id(1)).expect("root").children.push(id(50));
    let prefs = Preferences::default();
    let plan = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new());
    assert_eq!(plan.moves.len(), 5);
    let mut queue: VecDeque<Move> = plan.moves.into();
    let Step::Change(change, taken) = step(
        &mut inv,
        id(OBJECTS),
        &prefs.base_name(),
        &mut queue,
        &HashSet::new(),
        &HashSet::new(),
    ) else {
        panic!("a mutation")
    };
    assert_eq!(taken, 5);
    let made: Vec<_> = change
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Folder { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    // Sorted is reused; Reborn once for its two categories
    assert_eq!(made, ["Furniture", "Legacy", "Dresses", "Reborn", "Lingerie", "Skirts"]);
    actions::demo_mutate(&mut inv, id(AGENT), *change).expect("demo server");
    let at = |path: &[&str]| path.iter().try_fold(id(50), |parent, part| child(&inv, parent, part));
    let skirts = at(&["Reborn", "Skirts"]).expect("Sorted/Reborn/Skirts");
    assert_eq!(inv.folders[&skirts].items.len(), 2);
    assert!(at(&["Furniture"]).is_some_and(|f| inv.folders[&f].items == [id(104)]));
    assert!(inv.folders[&id(OBJECTS)].items.is_empty());
    assert!(matches!(
        step(
            &mut inv,
            id(OBJECTS),
            &prefs.base_name(),
            &mut queue,
            &HashSet::new(),
            &HashSet::new()
        ),
        Step::Done
    ));
    // sorting Sorted/Reborn/Skirts itself moves nothing
    assert!(super::plan(&inv, skirts, &prefs, &HashSet::new(), &HashSet::new()).moves.is_empty());
}

#[test]
fn a_large_sort_goes_in_bounded_batches() {
    let names: Vec<String> = (0..450).map(|n| format!("Skirt {n} Reborn")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut inv = fixture(&refs);
    let prefs = Preferences::default();
    let mut queue: VecDeque<Move> = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new()).moves.into();
    let mut batches = Vec::new();
    while let Step::Change(change, taken) = step(
        &mut inv,
        id(OBJECTS),
        &prefs.base_name(),
        &mut queue,
        &HashSet::new(),
        &HashSet::new(),
    ) {
        batches.push(taken);
        actions::demo_mutate(&mut inv, id(AGENT), *change).expect("demo server");
    }
    assert_eq!(batches, [BATCH, BATCH, 50]);
    assert!(inv.folders[&id(OBJECTS)].items.is_empty());
}

#[test]
fn an_unloaded_destination_waits_without_sending_anything() {
    let mut inv = fixture(&["Velvet skirt Reborn"]);
    inv.folders.insert(id(50), folder(id(50), id(1), "Sorted", -1));
    inv.folders.get_mut(&id(1)).expect("root").children.push(id(50));
    inv.folders.get_mut(&id(50)).expect("sorted").state = FetchState::Unknown;
    let prefs = Preferences::default();
    let mut queue: VecDeque<Move> = plan(&inv, id(OBJECTS), &prefs, &HashSet::new(), &HashSet::new()).moves.into();
    assert!(matches!(
        step(
            &mut inv,
            id(OBJECTS),
            &prefs.base_name(),
            &mut queue,
            &HashSet::new(),
            &HashSet::new()
        ),
        Step::Wait
    ));
    assert_eq!(queue.len(), 1);
}

#[test]
fn the_trash_and_the_library_are_not_sources() {
    let mut inv = fixture(&[]);
    assert!(source_allowed(&inv, id(OBJECTS)));
    assert!(!source_allowed(&inv, id(11)));
    inv.folders.insert(id(60), folder(id(60), Uuid::nil(), "Library", 8));
    inv.folders.get_mut(&id(60)).expect("library").library = true;
    assert!(!source_allowed(&inv, id(60)));
}

#[test]
fn the_demo_inventory_sorts_into_reused_folders_and_keeps_what_it_cannot_read() {
    let mut inv = Inventory {
        root: id(1),
        ..Default::default()
    };
    inv.folders.insert(id(1), folder(id(1), Uuid::nil(), "My Inventory", 8));
    demo::seed(&mut inv, id(AGENT), 1);
    let prefs = Preferences::default();
    let plan = plan(&inv, demo::OBJECTS, &prefs, &HashSet::new(), &HashSet::new());
    let unknown = plan.unknown.clone();
    assert!(plan.moves.len() > 500, "{}", plan.moves.len());
    assert!(plan.unpackers() > 50);
    assert!(unknown.iter().any(|n| n == "Gift box"));
    let mut queue: VecDeque<Move> = plan.moves.into();
    while let Step::Change(change, _) = step(
        &mut inv,
        demo::OBJECTS,
        &prefs.base_name(),
        &mut queue,
        &HashSet::new(),
        &HashSet::new(),
    ) {
        actions::demo_mutate(&mut inv, id(AGENT), *change).expect("demo server");
    }
    // the earlier run's Sorted and Sorted/Reborn are reused, not doubled
    let named = |parent: Uuid, name: &str| {
        inv.folders[&parent]
            .children
            .iter()
            .filter(|c| inv.folders[*c].info.name.eq_ignore_ascii_case(name))
            .count()
    };
    assert_eq!(named(id(1), "Sorted"), 1);
    let sorted = child(&inv, id(1), "Sorted").expect("Sorted");
    assert_eq!(named(sorted, "Reborn"), 1);
    assert!(child(&inv, sorted, "Reborn").and_then(|r| child(&inv, r, "Skirts")).is_some());
    // Objects keeps only what nothing could read
    let left: Vec<String> = inv.folders[&demo::OBJECTS]
        .items
        .iter()
        .chain(&inv.folders[&demo::OBJECTS].children)
        .map(|id| actions::name(&inv, *id))
        .collect();
    let mut expected = unknown;
    let mut left_sorted = left;
    expected.sort();
    left_sorted.sort();
    assert_eq!(left_sorted, expected);
}
